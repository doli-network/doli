//! INC-I-112: Regression test for Phase 3 stale in-memory UtxoSet pool read.
//!
//! **Root cause**: Storage Phase 3 (commit 632045f2) removed the per-tx
//! in-memory UtxoSet update calls (`spend_transaction` + `add_transaction`)
//! from `process_transaction_utxos`. All UTXO mutations now flow only through
//! `BlockBatch` (state_db WriteBatch). However, three consensus-critical sites
//! still read pool UTXOs via `self.utxo_set.read().get_by_pubkey_hash(pool_hash)`:
//!   1. `rewards.rs:332`         — `calculate_epoch_rewards` pool total
//!   2. `validation_checks.rs:677` — `validate_block_economics` conservation
//!   3. `validation_checks.rs:763` — pool input verification
//!
//! Because the in-memory UtxoSet is never refreshed during apply_block, these
//! reads return the pool snapshot from node startup (empty for a fresh node).
//! At the epoch boundary, the stale pool drives wrong reward distribution
//! (pool=0 → all rewards are 0) OR triggers `[ECON_EPOCH_OVERFLOW]`.
//!
//! **Bisect anchors** (verified empirically on local testnet N13):
//!   GOOD: cd4645a5 (Phase 2) — in-memory UtxoSet updated per-tx
//!   FIRST BAD: 632045f2 (Phase 3) — in-memory UtxoSet updates removed
//!   BAD: 28831590 (Phase 4) — preserves Phase 3 bug surface
//!
//! This test MUST pass at cd4645a5 and FAIL at 632045f2. The developer's fix
//! must restore in-memory UtxoSet liveness so the test passes on main again.

// OUTPUT CONTRACT: fn test_inc_i_112_epoch_boundary_sees_live_pool
//
// Function under test: Node::apply_block + calculate_epoch_rewards pipeline.
// The observable bug is that after N calls to apply_block, the in-memory
// UtxoSet still reports 0 pool UTXOs, causing calculate_epoch_rewards to
// compute a pool total of 0 and return empty reward outputs.
//
// Outputs:
//   O1: self.utxo_set (in-memory) — pool UTXOs after N apply_block calls
//       Should contain N entries totaling N * block_reward, not 0.
//   O2: calculate_epoch_rewards() return value — Vec<(u64, Hash)>
//       Sum of amounts should equal the pool total, not 0.
//   O3: Pool total derived from O1 matches pool total used by O2
//       (cross-consistency between the direct read and the function's internal read)
//
// PATHS:
//   P1: happy path — apply 3 blocks in epoch 0, read pool at boundary
//       Under Phase 2: pool = 3 * 100_000_000 = 300_000_000
//       Under Phase 3 (bug): pool = 0 (in-memory never updated)
//   P2: accumulated epoch path — apply 7 blocks across epochs 0+1, read pool
//       Under Phase 2: pool = 7 * 100_000_000 = 700_000_000
//       Under Phase 3 (bug): pool = 0
//
// INPUT PARTITIONS:
//   P1a: Fresh node, epoch 0, 3 blocks applied (minimum non-trivial epoch fill)
//        — pool should reflect exactly 3 coinbase deposits
//   P1b: Fresh node, epoch 0+1, 7 blocks applied (cross-epoch accumulation)
//        — pool should reflect all 7 coinbase deposits without epoch-boundary reset
//        (epoch 0 has no EpochReward because it's the genesis epoch)
//
// MATRIX: O1×O2×O3 across (P1a, P1b) — 6 cells, 6 assertions
//   P1a: O1(3 pool UTXOs, 300M total)✓  O2(rewards sum=300M)✓  O3(match)✓
//   P1b: O1(7 pool UTXOs, 700M total)✓  O2(rewards sum=700M)✓  O3(match)✓

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::Transaction;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader};
use doli_node::node::Node;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

// ============================================================
// HELPERS
// ============================================================

/// Create a test Node with N producers, real RocksDB, devnet config.
/// Devnet blocks_per_reward_epoch = 4.
async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    // Ensure block reward stays constant through our test range.
    // Devnet blocks_per_era = 576 → safe for heights 1-7, but be explicit.
    node.params.blocks_per_era = 100_000;
    (node, producers, temp)
}

/// Build a block with valid coinbase going to reward pool.
fn build_block(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
) -> Block {
    let reward = params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, slot);
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase));
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash,
        timestamp,
        slot,
        producer: *producer.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: crypto::Hash::ZERO,
        fork_id: crypto::Hash::ZERO,
    };

    Block::new(header, vec![coinbase])
}

/// Build a chain of N blocks starting at the given height/slot.
fn build_chain(
    start_height: u64,
    start_slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    count: usize,
    params: &ConsensusParams,
) -> Vec<Block> {
    let mut blocks = Vec::with_capacity(count);
    let mut prev = prev_hash;
    for i in 0..count {
        let h = start_height + i as u64;
        let s = start_slot + i as u32;
        let block = build_block(h, s, prev, producer, params);
        prev = block.hash();
        blocks.push(block);
    }
    blocks
}

/// Apply blocks with Light validation (bypasses producer scheduling).
async fn apply_chain(node: &mut Node, blocks: &[Block]) {
    for block in blocks {
        node.apply_block(block.clone(), ValidationMode::Light, None)
            .await
            .unwrap_or_else(|e| panic!("apply_block failed at h={}: {}", block.header.slot, e));
    }
}

// ============================================================
// TEST: INC-I-112 — in-memory UtxoSet pool must reflect applied blocks
// ============================================================

/// **INC-I-112 Regression**: After applying N blocks, the in-memory UtxoSet
/// must contain the N coinbase pool UTXOs that were written to state_db.
///
/// Phase 3 (632045f2) broke this: apply_block writes UTXOs only through
/// BlockBatch → state_db, but never updates the in-memory UtxoSet. The three
/// consensus-critical pool reads in rewards.rs and validation_checks.rs see
/// the startup snapshot (empty for a fresh node) rather than the live state.
///
/// This test exercises two partitions:
///   P1a: 3 blocks (single epoch fill) → pool = 300_000_000
///   P1b: 7 blocks (cross-epoch accumulation) → pool = 700_000_000
///
/// Under Phase 3, both partitions yield pool = 0 (FAIL).
/// Under Phase 2 or post-fix, both yield the correct totals (PASS).
#[tokio::test]
async fn test_inc_i_112_epoch_boundary_sees_live_pool() {
    // ── Setup ──────────────────────────────────────────────────
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();
    let block_reward = params.block_reward(1); // 100_000_000 in devnet era 0
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();

    // Verify devnet epoch length is 4 (test assumption).
    let blocks_per_epoch = node.config.network.blocks_per_reward_epoch();
    assert_eq!(
        blocks_per_epoch, 4,
        "Test assumes devnet blocks_per_reward_epoch=4"
    );

    // ── Partition P1a: 3 blocks (epoch 0 fill, pre-boundary) ──
    let chain_3 = build_chain(1, 1, Hash::ZERO, &producers[0], 3, &params);
    apply_chain(&mut node, &chain_3).await;

    // O1 (P1a): In-memory UtxoSet must contain 3 pool UTXOs totaling 300M.
    let pool_total_p1a = {
        let utxo = node.utxo_set.read().await;
        let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
        let total: u64 = pool_utxos.iter().map(|(_, e)| e.output.amount).sum();
        assert_eq!(
            pool_utxos.len(),
            3,
            "O1/P1a: in-memory UtxoSet should have 3 pool UTXOs after 3 blocks, \
             got {} (Phase 3 stale read: UTXOs written to state_db but not in-memory)",
            pool_utxos.len()
        );
        total
    };
    assert_eq!(
        pool_total_p1a,
        3 * block_reward,
        "O1/P1a: pool total should be 3 * {} = {}, got {} \
         (Phase 3: in-memory UtxoSet never refreshed during apply_block)",
        block_reward,
        3 * block_reward,
        pool_total_p1a
    );

    // O2 (P1a): calculate_epoch_rewards(0) must distribute the full pool.
    // Epoch 0 auto-qualifies all producers (no attestation data needed).
    let rewards_p1a = node
        .calculate_epoch_rewards(0)
        .await
        .expect("calculate_epoch_rewards(0) should not fail");
    let rewards_sum_p1a: u64 = rewards_p1a.iter().map(|(amount, _)| *amount).sum();
    assert_eq!(
        rewards_sum_p1a,
        3 * block_reward,
        "O2/P1a: epoch 0 rewards sum should be {} (3 blocks * {}), got {} \
         (Phase 3: pool read returns 0 → all rewards are 0 → empty Vec)",
        3 * block_reward,
        block_reward,
        rewards_sum_p1a
    );

    // O3 (P1a): Cross-consistency — direct pool read matches rewards total.
    assert_eq!(
        pool_total_p1a, rewards_sum_p1a,
        "O3/P1a: direct pool read ({}) must match rewards sum ({})",
        pool_total_p1a, rewards_sum_p1a
    );

    // ── Partition P1b: 4 more blocks (epoch 1 fill, cross-epoch) ──
    let prev_hash = chain_3.last().unwrap().hash();
    let chain_4 = build_chain(4, 4, prev_hash, &producers[0], 4, &params);
    apply_chain(&mut node, &chain_4).await;

    // O1 (P1b): In-memory UtxoSet must now contain 7 pool UTXOs totaling 700M.
    // No EpochReward was distributed (epoch 0 boundary at h=4 doesn't have one
    // because ECON_EPOCH_ZERO prevents it, and epoch 1 boundary is at h=8 which
    // hasn't been reached). All 7 coinbase UTXOs should be in the pool.
    let pool_total_p1b = {
        let utxo = node.utxo_set.read().await;
        let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
        let total: u64 = pool_utxos.iter().map(|(_, e)| e.output.amount).sum();
        assert_eq!(
            pool_utxos.len(),
            7,
            "O1/P1b: in-memory UtxoSet should have 7 pool UTXOs after 7 blocks, \
             got {} (Phase 3 stale read: UTXOs written to state_db but not in-memory)",
            pool_utxos.len()
        );
        total
    };
    assert_eq!(
        pool_total_p1b,
        7 * block_reward,
        "O1/P1b: pool total should be 7 * {} = {}, got {} \
         (Phase 3: in-memory UtxoSet never refreshed during apply_block)",
        block_reward,
        7 * block_reward,
        pool_total_p1b
    );

    // O2 (P1b): calculate_epoch_rewards(0) with accumulated pool.
    // Even though we're past epoch 0 boundary, the pool still has all UTXOs
    // because no EpochReward was distributed (genesis epoch exempt).
    // The function reads the current pool state regardless of which epoch
    // we pass — it's the live pool snapshot.
    let rewards_p1b = node
        .calculate_epoch_rewards(0)
        .await
        .expect("calculate_epoch_rewards(0) should not fail");
    let rewards_sum_p1b: u64 = rewards_p1b.iter().map(|(amount, _)| *amount).sum();
    assert_eq!(
        rewards_sum_p1b,
        7 * block_reward,
        "O2/P1b: epoch 0 rewards sum should be {} (7 blocks * {}), got {} \
         (Phase 3: pool read returns 0 → all rewards are 0 → empty Vec)",
        7 * block_reward,
        block_reward,
        rewards_sum_p1b
    );

    // O3 (P1b): Cross-consistency — direct pool read matches rewards total.
    assert_eq!(
        pool_total_p1b, rewards_sum_p1b,
        "O3/P1b: direct pool read ({}) must match rewards sum ({})",
        pool_total_p1b, rewards_sum_p1b
    );
}
