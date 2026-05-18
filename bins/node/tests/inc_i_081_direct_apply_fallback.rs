//! INC-I-081 Bundle 2, Bug 3 -- Direct-apply fallback in fork recovery
//!
//! Reproduces the bug where `handle_completed_fork_recovery` silently drops a
//! candidate block even when it is a direct successor of the local tip
//! (candidate.prev_hash == chain_state.best_hash). Three branches in the
//! function can reach this silent-drop state:
//!   - check_reorg_weighted returns Some but should_switch=false
//!   - plan_reorg returns Some but fork is "not heavier"
//!   - plan_reorg returns None ("could not plan reorg")
//!
//! In all three cases, if the candidate is a direct successor, it MUST be
//! applied via the standard block-apply path instead of being dropped.
//!
//! Invariant: INV-SYNC-003 -- when reorg evaluation fails or declines but the
//! candidate's parent_hash equals local_tip_hash, the candidate is a direct
//! successor and MUST be applied. Silently dropping it is a bug.
//!
//! The fix extracts a `try_apply_direct_successor` helper on Node that checks
//! the parent condition and applies the block if it matches. This test verifies
//! the helper.
//!
//! OUTPUT CONTRACT: fn Node::try_apply_direct_successor(&mut self, candidate: &Block)
//!                  -> Result<bool>
//!
//!   Outputs:
//!     O1: Return value -- Ok(true) if applied, Ok(false) if not a direct successor
//!     O2: self.chain_state.best_height -- advances by 1 on Ok(true), unchanged on Ok(false)
//!     O3: self.chain_state.best_hash -- equals candidate.hash() on Ok(true), unchanged on Ok(false)
//!     O4: self.block_store -- contains candidate at new height on Ok(true), absent on Ok(false)
//!
//!   Paths:
//!     P1: candidate_is_direct_successor (prev_hash == local tip hash)
//!         INPUT PARTITIONS:
//!           P1a: single successor after short chain (height 6, proves basic wiring)
//!     P2: candidate_is_NOT_direct_successor (prev_hash != local tip hash)
//!         INPUT PARTITIONS:
//!           P2a: candidate whose prev_hash is an ancestor (NOT current tip)
//!
//!   MATRIX: 4 outputs x 2 partitions = 8 cells
//!     P1a: O1(Ok(true)) + O2(+1) + O3(== candidate.hash()) + O4(present)
//!     P2a: O1(Ok(false)) + O2(unchanged) + O3(unchanged) + O4(absent)

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Transaction};
use doli_node::node::Node;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

// ============================================================
// HELPERS (adapted from fork_recovery.rs test patterns)
// ============================================================

/// Create a test Node with N producers, real RocksDB, real everything.
async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
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
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);
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

/// Build a chain of N blocks from a given ancestor.
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

/// Apply a chain of blocks to a node with Light validation (skip VDF).
async fn apply_chain(node: &mut Node, blocks: &[Block]) {
    for block in blocks {
        node.apply_block(block.clone(), ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("apply_block failed at h={}: {}", block.header.slot, e));
    }
}

// ============================================================
// TEST 1 -- PRIMARY FAIL TEST (P1a: direct successor)
// ============================================================
//
// Requirement: INC-I-081 Bundle 2, Bug 3 / INV-SYNC-003
// Acceptance: try_apply_direct_successor returns Ok(true) and advances
//             chain_state when candidate.prev_hash == local tip
//
// OUTPUT CONTRACT coverage: Path P1a (candidate_is_direct_successor)
//   O1: return == Ok(true)
//   O2: chain_state.best_height == prev_height + 1
//   O3: chain_state.best_hash == candidate.hash()
//   O4: block_store.get_block_by_height(new_height).is_some()
//
// Pre-fix (stub returns Ok(false)): test FAILS.
// Post-fix (helper applies block): test PASSES.
#[tokio::test]
async fn test_inc_i_081_direct_apply_when_candidate_is_direct_successor() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    // Build a short chain (5 blocks) so we have a non-genesis tip
    let base = build_chain(1, 1, Hash::ZERO, &producers[0], 5, &params);
    apply_chain(&mut node, &base).await;

    let prev_height = node.chain_state.read().await.best_height;
    let prev_hash = node.chain_state.read().await.best_hash;
    assert_eq!(prev_height, 5, "setup: chain should be at height 5");
    assert_eq!(
        prev_hash,
        base[4].hash(),
        "setup: tip should be last base block"
    );

    // Build a direct successor: prev_hash == current tip
    let candidate = build_block(6, 6, prev_hash, &producers[1], &params);
    let candidate_hash = candidate.hash();

    // Call the helper -- pre-fix this returns Ok(false), post-fix Ok(true)
    let result = node
        .try_apply_direct_successor(&candidate)
        .await
        .expect("try_apply_direct_successor should not error");

    // O1: Must return true (block was applied)
    assert!(
        result,
        "INC-I-081 Bug 3: try_apply_direct_successor returned false for a direct \
         successor (candidate.prev_hash == local tip). The block was silently \
         dropped instead of being applied. This is the bug."
    );

    // O2: Chain height must advance by 1
    let new_height = node.chain_state.read().await.best_height;
    assert_eq!(
        new_height,
        prev_height + 1,
        "chain_state.best_height should advance from {} to {}",
        prev_height,
        prev_height + 1
    );

    // O3: Best hash must equal the candidate's hash
    let new_hash = node.chain_state.read().await.best_hash;
    assert_eq!(
        new_hash, candidate_hash,
        "chain_state.best_hash should equal the applied candidate's hash"
    );

    // O4: Block should be in block_store at the new height
    let stored = node
        .block_store
        .get_block_by_height(new_height)
        .unwrap_or(None);
    assert!(
        stored.is_some(),
        "block_store should contain the candidate at height {}",
        new_height
    );
}

// ============================================================
// TEST 2 -- SANITY: non-successor is correctly declined (P2a)
// ============================================================
//
// Requirement: INC-I-081 Bundle 2, Bug 3 / INV-SYNC-003 (negative case)
// Acceptance: try_apply_direct_successor returns Ok(false) when candidate's
//             prev_hash does NOT equal local tip (it is NOT a direct successor)
//
// OUTPUT CONTRACT coverage: Path P2a (candidate_is_NOT_direct_successor)
//   O1: return == Ok(false)
//   O2: chain_state.best_height unchanged
//   O3: chain_state.best_hash unchanged
//   O4: block_store does NOT contain the candidate at height+1
//
// This test should PASS both pre-fix and post-fix (the stub already returns false).
#[tokio::test]
async fn test_inc_i_081_non_successor_is_declined() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    // Build a short chain (5 blocks)
    let base = build_chain(1, 1, Hash::ZERO, &producers[0], 5, &params);
    apply_chain(&mut node, &base).await;

    let prev_height = node.chain_state.read().await.best_height;
    let prev_hash = node.chain_state.read().await.best_hash;
    assert_eq!(prev_height, 5);

    // Build a candidate whose prev_hash is an ancestor (height 3), NOT the tip
    let ancestor_hash = base[2].hash(); // height 3
    assert_ne!(
        ancestor_hash, prev_hash,
        "setup: ancestor_hash must differ from tip"
    );
    let candidate = build_block(6, 6, ancestor_hash, &producers[1], &params);

    // Call the helper -- should return Ok(false) for a non-successor
    let result = node
        .try_apply_direct_successor(&candidate)
        .await
        .expect("try_apply_direct_successor should not error");

    // O1: Must return false (not a direct successor)
    assert!(
        !result,
        "try_apply_direct_successor should return false for a non-successor block"
    );

    // O2: Chain height must NOT change
    let new_height = node.chain_state.read().await.best_height;
    assert_eq!(
        new_height, prev_height,
        "chain_state.best_height must not change for a non-successor"
    );

    // O3: Best hash must NOT change
    let new_hash = node.chain_state.read().await.best_hash;
    assert_eq!(
        new_hash, prev_hash,
        "chain_state.best_hash must not change for a non-successor"
    );

    // O4: Block should NOT be in block_store at height+1
    let stored = node
        .block_store
        .get_block_by_height(prev_height + 1)
        .unwrap_or(None);
    assert!(
        stored.is_none(),
        "block_store should NOT contain the non-successor at height {}",
        prev_height + 1
    );
}
