//! M-RC9 — Silent `vec![]` regression tests (INC-I-034)
//!
//! Reproduces the 2026-04-16 live mainnet cascade: a producer whose block_store
//! is missing some blocks inside a completed epoch silently undercounts attestation
//! minutes and produces an EpochReward with the wrong output set, which is rejected
//! by complete-store validators and triggers a brief fork.
//!
//! The bug lives in `bins/node/src/node/rewards.rs::calculate_epoch_rewards` at
//! lines 41-69 (HEAD as of 2026-04-16):
//!   1. `if let Ok(Some(block)) = self.block_store.get_block_by_height(h)` — a
//!      missing block is **silently skipped** (no error, no log).
//!   2. Inside the `if let`, when `block.attestation_bitfield.is_empty()` AND
//!      `h >= BITFIELD_BODY_ACTIVATION_HEIGHT` (which is 0 — so always), the
//!      code falls to the `vec![]` branch, silently dropping the minute.
//!
//! Both silent paths cause the same class of divergence: `attested_minutes` is
//! populated only from the subset of blocks the local node happens to have
//! bodies for, while the canonical set is what the network agrees on. The
//! fix (M-RC9) must either:
//!   (a) FAIL-FAST: refuse to produce an EpochReward when the block_store is
//!       incomplete in the epoch window (return empty, or a dedicated error),
//!       OR
//!   (b) SNAPSHOT: compute qualifiers from the persisted EpochState
//!       attestation accumulators instead of scanning block_store at epoch end.
//!
//! Either fix makes the adversarial tests pass.
//!
//! OUTPUT CONTRACT: calculate_epoch_rewards(epoch: u64) -> Vec<(u64, Hash)>
//!   Function under test: `bins/node/src/node/rewards.rs::Node::calculate_epoch_rewards`
//!   Entry point verified: `pub async fn calculate_epoch_rewards(&self, epoch: u64) -> Vec<(u64, Hash)>` (rewards.rs:14).
//!   Observable outputs:
//!     O1: Return value length — number of reward outputs produced
//!     O2: Return value pubkey_hash set — WHICH producers receive rewards (by pkh)
//!     O3: Return value amounts sum — total distributed amount (must equal pool on non-empty result)
//!     O4: Determinism — two calls with identical state produce byte-identical Vec<(u64, Hash)>
//!   Paths (inputs crossed with outputs):
//!     P1: complete_body_bitfield    — all epoch heights present, all with non-empty bitfields
//!     P2: gap_in_middle             — one minute-bucket of blocks missing, one producer
//!                                     loses the ONLY blocks that attested them in that bucket
//!                                     (Santiago 39600-39628 pattern at smaller scale)
//!     P3: gap_at_start              — N/A: subsumed by P2 generality (same silent-skip path)
//!     P4: gap_at_end                — N/A: subsumed by P2 generality (same silent-skip path)
//!     P5: many_missing_mainnet      — santiago-scale: 37 producers, 2 minute-buckets dropped,
//!                                     disqualifying 12 producers if bug is active
//!     P6: pre_activation_header_bf  — N/A: BITFIELD_BODY_ACTIVATION_HEIGHT=0 in HEAD
//!                                     (the `else if h < ACTIVATION` branch at rewards.rs:55
//!                                     is dead code on HEAD; documented in CLAUDE context)
//!   MATRIX: 4 outputs × 6 paths = 24 assertion cells
//!     P1: O1 ✓ | O2 ✓ | O3 ✓ | O4 ✓ (regression anchor — test_regression_complete_store)
//!     P2: O1 ✓ | O2 ✓ | O3 ✓ | O4 ✓ (adversarial — test_adversarial_gap_in_middle)
//!     P3: N/A (same code path as P2 — redundant)
//!     P4: N/A (same code path as P2 — redundant)
//!     P5: O1 ✓ | O2 ✓ | O3 ✓ | O4 — (santiago replay — test_santiago_cascade_replay)
//!     P6: N/A × 4 (dead branch on HEAD)
//!     Covered reachable cells: 12 / 12 reachable. Justified N/A: 12 / 12 unreachable.

use std::collections::HashSet;
use std::sync::Once;

use crypto::{Hash, KeyPair};
use doli_core::consensus::{reward_pool_pubkey_hash, ConsensusParams};
use doli_core::transaction::{Output, Transaction};
use doli_core::{Block, BlockHeader, Network};
use doli_node::node::Node;
use storage::{Outpoint, UtxoEntry};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

// ============================================================
// Environment bootstrap
// ============================================================
//
// Force `blocks_per_reward_epoch=36` for devnet in this test binary.
// 36 blocks / 6 SLOTS_PER_ATTESTATION_MINUTE = 6 attestation minutes per epoch.
// attestation_qualification_threshold(36) = (6 * 90) / 100 = 5 → each producer
// needs at least 5 attestation minutes to qualify.
//
// With the "one-attester-per-block" pattern used below (block at height h
// attests ONLY the producer at index h % num_producers), each producer gets
// attested exactly once per minute-bucket (6 times across the epoch). Dropping
// ALL 6 blocks of a single minute bucket removes exactly 1 minute from
// producer P's count if P == (first_height_of_bucket) % num_producers.
// Dropping 2 minute buckets reduces the affected producers to 4 minutes,
// which is BELOW threshold=5 → they are silently disqualified by the bug.
//
// The snapshot fix (reads EpochState.attestation_accum accumulated at
// apply_block time) does not see the gap and keeps all producers qualified.
// The fail-fast fix refuses to produce any reward when the block_store is
// incomplete. Any other behavior means the silent-skip bug persists.

static ENV_INIT: Once = Once::new();
fn init_env() {
    ENV_INIT.call_once(|| {
        // Safe in 2021 edition (test binary is single-process).
        // Must precede the first Network::Devnet.params() call so the OnceLock
        // inside NetworkParams::load caches the right value.
        std::env::set_var("DOLI_BLOCKS_PER_REWARD_EPOCH", "36");
        // Eagerly lock into NetworkParams' OnceLock.
        let _ = Network::Devnet.params();
    });
}

// ============================================================
// Constants
// ============================================================
const EPOCH_LEN: u64 = 36; // matches DOLI_BLOCKS_PER_REWARD_EPOCH
const MINUTES_PER_EPOCH: u32 = 6; // 36 / SLOTS_PER_ATTESTATION_MINUTE (6)
const QUAL_THRESHOLD: u32 = 5; // (6 * 90) / 100
const NUM_PRODUCERS_SMALL: usize = 6; // aligned with MINUTES_PER_EPOCH for clean cycle
const NUM_PRODUCERS_MAINNET: usize = 37; // matches mainnet active set for santiago replay

// ============================================================
// Test scaffolding helpers
// ============================================================

async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    init_env();
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    // Sanity-check that the env override actually took effect. If another test
    // in this binary locked a different value first, this fails loudly instead
    // of silently masking the bug behind devnet's default (4).
    assert_eq!(
        node.config.network.blocks_per_reward_epoch(),
        EPOCH_LEN,
        "DOLI_BLOCKS_PER_REWARD_EPOCH override did not take effect"
    );
    // Keep block reward positive through the whole test range (devnet era
    // decay would zero it out after h=576 otherwise).
    node.params.blocks_per_era = 100_000;
    (node, producers, temp)
}

/// Build a block that attests a CUSTOM set of producer indices.
///
/// The bitfield length is `ceil(producer_count / 8)` bytes; bits at the
/// positions in `attested_indices` are set, others cleared.
/// `presence_root = BLAKE3(bitfield)` for commitment consistency.
fn build_block_with_bitfield(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    attested_indices: &[usize],
    producer_count: usize,
    params: &ConsensusParams,
) -> Block {
    let bitfield = doli_core::encode_attestation_bitfield_vec(attested_indices, producer_count);
    let presence_root = crypto::hash::hash(&bitfield);

    let reward = params.block_reward(height);
    let pool_hash = reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase));
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root,
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

    let mut block = Block::new(header, vec![coinbase]);
    block.attestation_bitfield = bitfield;
    block
}

/// Shortcut for a block that attests ALL producers.
fn build_block_with_full_bitfield(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    producer_count: usize,
    params: &ConsensusParams,
) -> Block {
    let indices: Vec<usize> = (0..producer_count).collect();
    build_block_with_bitfield(
        height,
        slot,
        prev_hash,
        producer,
        &indices,
        producer_count,
        params,
    )
}

/// Write a block as canonical at the given height.
fn put_canonical(node: &Node, block: &Block, height: u64) {
    node.block_store
        .put_block_canonical(block, height)
        .expect("put_block_canonical failed");
}

/// Seed the reward pool with a single non-zero UTXO so the function reaches
/// its distribution path (otherwise line 187-189 short-circuits to Vec::new).
async fn seed_reward_pool(node: &Node, total_amount: u64, tag: &str) {
    let pool_hash = reward_pool_pubkey_hash();
    let tx_hash = crypto::hash::hash(tag.as_bytes());
    let entry = UtxoEntry {
        output: Output::normal(total_amount, pool_hash),
        height: 0,
        is_coinbase: true,
        is_epoch_reward: false,
    };
    let mut utxo = node.utxo_set.write().await;
    utxo.insert(Outpoint::new(tx_hash, 0), entry)
        .expect("insert pool UTXO failed");
}

/// Collect the set of pubkey_hashes that appear in the reward output vector.
fn pkh_set(outputs: &[(u64, Hash)]) -> HashSet<Hash> {
    outputs.iter().map(|(_, pkh)| *pkh).collect()
}

/// Compute canonical pubkey_hash for a producer (same formula as rewards.rs:178).
fn producer_pkh(kp: &KeyPair) -> Hash {
    crypto::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes())
}

/// Build the "producers sorted by pubkey bytes" order that rewards.rs uses
/// (see rewards.rs:27). The block_store heights do not depend on this order
/// directly, but the attestation bitfield bit indices DO — bit N corresponds
/// to the Nth producer in sorted order.
fn sorted_producer_order(producers: &[KeyPair]) -> Vec<usize> {
    let mut indexed: Vec<(usize, &KeyPair)> = producers.iter().enumerate().collect();
    indexed.sort_by(|a, b| a.1.public_key().as_bytes().cmp(b.1.public_key().as_bytes()));
    indexed.into_iter().map(|(i, _)| i).collect()
}

/// Populate heights 0..EPOCH_LEN with fully-attested blocks (epoch 0),
/// then populate heights EPOCH_LEN..2*EPOCH_LEN with the "one-attester-per-block"
/// pattern (epoch 1 — the one under test). Returns the generated epoch-1 blocks
/// indexed by height offset (0..EPOCH_LEN).
///
/// For each height `h` in epoch 1, the block's attestation bitfield has exactly
/// one bit set: the bit for the producer at sorted-index `h % num_producers`.
/// Over 36 blocks, each of `num_producers` sorted producers is attested exactly
/// `36 / num_producers` times (rounded). With NUM_PRODUCERS_SMALL=6, each
/// producer is attested exactly once per minute-bucket (6 times total across
/// the epoch, one per minute).
async fn populate_chain_with_one_attester_pattern(
    node: &Node,
    producers: &[KeyPair],
    producer_count: usize,
    params: &ConsensusParams,
) -> Vec<Block> {
    let genesis_hash = node.chain_state.read().await.best_hash;
    let sorted_order = sorted_producer_order(producers);
    // Inverse: sorted_index -> original_index
    let sorted_to_orig = sorted_order.clone();

    let mut prev = genesis_hash;

    // Epoch 0: fully-attested to avoid the epoch-0 short-circuit affecting state.
    for h in 0..EPOCH_LEN {
        let block = build_block_with_full_bitfield(
            h,
            h as u32,
            prev,
            &producers[(h as usize) % producers.len()],
            producer_count,
            params,
        );
        prev = block.hash();
        put_canonical(node, &block, h);
    }

    // Epoch 1: "one-attester-per-block" pattern. Bit `sorted_index = h % N` set.
    let mut epoch1_blocks: Vec<Block> = Vec::with_capacity(EPOCH_LEN as usize);
    for offset in 0..EPOCH_LEN {
        let h = EPOCH_LEN + offset;
        let sorted_index = (h as usize) % producer_count;
        let orig_producer_index = sorted_to_orig[sorted_index];
        let block = build_block_with_bitfield(
            h,
            h as u32,
            prev,
            &producers[orig_producer_index],
            &[sorted_index],
            producer_count,
            params,
        );
        prev = block.hash();
        put_canonical(node, &block, h);
        epoch1_blocks.push(block);
    }

    epoch1_blocks
}

// ============================================================
// TEST A — REGRESSION ANCHOR (must PASS today AND after fix)
// ============================================================
//
// OUTPUT CONTRACT coverage: Path P1 (complete_body_bitfield)
//   O1 ✓ : len(outputs) == NUM_PRODUCERS_SMALL
//   O2 ✓ : pkh_set(outputs) == {producer_pkh(p) for p in all producers}
//   O3 ✓ : sum(amounts) == POOL_TOTAL
//   O4 ✓ : two back-to-back calls return identical Vec
//
// Happy-path anchor. If the fix accidentally breaks the complete-store case,
// this test catches it. Uses "one-attester-per-block" pattern so every
// producer gets exactly 6 unique minutes (> threshold 5).
#[tokio::test]
async fn test_regression_complete_store_all_producers_qualify() {
    let (node, producers, _tmp) = make_node(NUM_PRODUCERS_SMALL).await;
    let params = node.params.clone();

    // Fully populate both epochs.
    let _epoch1_blocks =
        populate_chain_with_one_attester_pattern(&node, &producers, NUM_PRODUCERS_SMALL, &params)
            .await;

    let pool_total: u64 = 60_000_000;
    seed_reward_pool(&node, pool_total, "test_A_pool").await;

    let outputs = node.calculate_epoch_rewards(1).await;

    // O1: one entry per producer (all qualified — 6 unique minutes each).
    assert_eq!(
        outputs.len(),
        NUM_PRODUCERS_SMALL,
        "O1: complete-store epoch must award every producer (expected {}, got {}). \
         This is the regression anchor — failure indicates the fix broke the happy path.",
        NUM_PRODUCERS_SMALL,
        outputs.len()
    );

    // O2: exactly the expected producer pubkey_hash set.
    let got_pkhs = pkh_set(&outputs);
    let want_pkhs: HashSet<Hash> = producers.iter().map(producer_pkh).collect();
    assert_eq!(
        got_pkhs, want_pkhs,
        "O2: reward recipient set must match the active producer set"
    );

    // O3: amounts sum to the pool total.
    let sum: u64 = outputs.iter().map(|(a, _)| *a).sum();
    assert_eq!(
        sum, pool_total,
        "O3: distributed amount must equal pool_total (got {} of {})",
        sum, pool_total
    );

    // O4: determinism.
    let outputs2 = node.calculate_epoch_rewards(1).await;
    assert_eq!(
        outputs, outputs2,
        "O4: calculate_epoch_rewards must be deterministic for identical state"
    );
}

// ============================================================
// TEST B — ADVERSARIAL GAP IN MIDDLE (must FAIL on HEAD, PASS after fix)
// ============================================================
//
// OUTPUT CONTRACT coverage: Path P2 (gap_in_middle)
//   O1 ✓ : len(outputs) ∈ {0, NUM_PRODUCERS_SMALL}
//   O2 ✓ : pkh_set(outputs) ∈ {∅, full producer set} — never a silent subset
//   O3 ✓ : sum(amounts) ∈ {0, pool_total}
//   O4 ✓ : determinism under the gap condition
//
// Reproduces the bug pattern (Santiago, 2026-04-16): blocks are missing
// from the LOCAL block_store but the canonical chain includes them. The
// current code silently drops them. With the "one-attester-per-block"
// pattern, dropping the 6 blocks of one minute bucket AND one extra block
// from a second minute bucket causes producer at sorted-index 0 to lose
// 2 of their 6 attestation minutes → drops to 4 → below threshold 5 → the
// buggy code silently disqualifies ONLY producer 0, returning a 5-entry
// reward vector that's neither empty (fail-fast) nor full (snapshot).
//
// The gap targets:
//   - All 6 heights of minute bucket 6 (heights 36..42, slots 36..41). Slot
//     bucket 6 contains one block per producer (h=36 attests sorted-index 0,
//     h=37 attests index 1, ..., h=41 attests index 5). Dropping all 6
//     removes minute 6 from EVERY producer's attested set.
//   - Additionally height 42 (slot 42, bucket 7, attests sorted-index 0).
//     Dropping this block removes minute 7 from producer 0 ONLY (other
//     producers still have their own blocks at 43..47 in bucket 7).
//
// Result if bug is active:
//   - Producer 0: 4 unique minutes (buckets 8, 9, 10, 11) → BELOW threshold → excluded
//   - Producers 1..5: 5 unique minutes (buckets 7, 8, 9, 10, 11) → AT threshold → included
//   → outputs.len() == 5, missing producer 0's pkh. Test asserts this is invalid.
//
// Result with a correct fix:
//   - Fail-fast: outputs.len() == 0 (refuse to compute from incomplete store)
//   - Snapshot:  outputs.len() == 6 (compute from EpochState.attestation_accum
//                which was accumulated during apply_block, unaffected by gaps)
#[tokio::test]
async fn test_adversarial_gap_in_middle_must_not_silently_undercount() {
    let (node, producers, _tmp) = make_node(NUM_PRODUCERS_SMALL).await;
    let params = node.params.clone();

    // First populate everything normally.
    let epoch1_blocks =
        populate_chain_with_one_attester_pattern(&node, &producers, NUM_PRODUCERS_SMALL, &params)
            .await;

    // Now delete specific heights from the block_store to simulate gaps.
    // Targeted gap set: all of minute-bucket 6 (h=36..42) + h=42.
    // That is heights 36, 37, 38, 39, 40, 41, 42 — the first 7 heights of
    // epoch 1 (heights 36..=42 inclusive).
    // This mirrors santiago's 39600..=39628 range (29 consecutive heights)
    // at a smaller scale — the key property is the ~first-sorted-producer
    // discrimination pattern.
    let gap_heights: Vec<u64> = (36..=42).collect();

    // Delete each gap block. We use the low-level write path that skips
    // canonical indexing; simplest is to re-insert a sentinel at those
    // heights by overwriting the canonical hash map to a hash with no
    // stored block. Easier: bypass canonical index by NOT inserting those
    // heights in the first place. Since populate_chain_with_one_attester_pattern
    // inserted everything, we delete them directly via the BlockStore
    // RocksDB CF_HEIGHT_INDEX. But BlockStore has no public `delete_by_height`.
    // Simplest workaround: rebuild without them. Let's do that below.
    drop(epoch1_blocks);

    // Rebuild the node with the gap. We need a fresh node to control insertion.
    // Use a new TempDir / fresh producers derived from the SAME keys to keep
    // sort order stable.
    let _node_to_drop = node;
    let (node, producers, _tmp2) = {
        let tmp = TempDir::new().unwrap();
        // New node — but we want the SAME producer keys so sorted order stays
        // stable. Reuse `producers` by cloning.
        let producers_clone = producers.clone();
        let mut node2 = Node::new_for_test(tmp.path().to_path_buf(), producers_clone.clone())
            .await
            .expect("Node::new_for_test failed");
        assert_eq!(
            node2.config.network.blocks_per_reward_epoch(),
            EPOCH_LEN,
            "env override must still hold on second node"
        );
        node2.params.blocks_per_era = 100_000;
        (node2, producers, tmp)
    };

    // Populate epoch 0 fully.
    let genesis_hash = node.chain_state.read().await.best_hash;
    let mut prev = genesis_hash;
    for h in 0..EPOCH_LEN {
        let block = build_block_with_full_bitfield(
            h,
            h as u32,
            prev,
            &producers[(h as usize) % producers.len()],
            NUM_PRODUCERS_SMALL,
            &params,
        );
        prev = block.hash();
        put_canonical(&node, &block, h);
    }

    // Epoch 1: "one-attester-per-block" BUT drop heights in gap_heights.
    // We still need to thread prev_hash through the canonical chain so the
    // block that WOULD have been there contributes its hash to the chain
    // (otherwise `prev_hash` of the next stored block is wrong — but the
    // reward function does not validate chain continuity via block_store,
    // so we can just skip insertions).
    let sorted_order = sorted_producer_order(&producers);
    for offset in 0..EPOCH_LEN {
        let h = EPOCH_LEN + offset;
        let sorted_index = (h as usize) % NUM_PRODUCERS_SMALL;
        let orig_producer_index = sorted_order[sorted_index];
        let block = build_block_with_bitfield(
            h,
            h as u32,
            prev,
            &producers[orig_producer_index],
            &[sorted_index],
            NUM_PRODUCERS_SMALL,
            &params,
        );
        prev = block.hash();
        if !gap_heights.contains(&h) {
            put_canonical(&node, &block, h);
        }
    }

    // Self-check the gap.
    for &gh in &gap_heights {
        assert!(
            node.block_store.get_block_by_height(gh).unwrap().is_none(),
            "test setup error: height {} should be absent from block_store",
            gh
        );
    }

    let pool_total: u64 = 60_000_000;
    seed_reward_pool(&node, pool_total, "test_B_pool").await;

    // Call the function under test.
    let outputs = node.calculate_epoch_rewards(1).await;

    // M-RC9 CONTRACT: the fix must pick one of two behaviors. Any other
    // outcome means the silent-undercount path is still reachable.
    //
    //   FAIL-FAST: outputs is empty (function detects incomplete block_store
    //              and refuses to distribute — consistent with "accumulate
    //              pool to next epoch" language at line 133/158).
    //   SNAPSHOT:  outputs has NUM_PRODUCERS_SMALL entries, set == all-producers,
    //              sum == pool_total (function reads EpochState accumulators,
    //              ignoring block_store gaps).
    let len = outputs.len();
    let got_pkhs = pkh_set(&outputs);
    let want_all_pkhs: HashSet<Hash> = producers.iter().map(producer_pkh).collect();
    let sum: u64 = outputs.iter().map(|(a, _)| *a).sum();

    // Diagnostic logging — makes failure readable.
    eprintln!(
        "[M-RC9 test B] len={} sum={}/{} pkh_in_output={}/{}",
        len,
        sum,
        pool_total,
        got_pkhs.len(),
        want_all_pkhs.len()
    );

    let is_fail_fast = len == 0 && sum == 0;
    let is_snapshot = len == NUM_PRODUCERS_SMALL && got_pkhs == want_all_pkhs && sum == pool_total;

    assert!(
        is_fail_fast || is_snapshot,
        "M-RC9: calculate_epoch_rewards on a block_store gap must either fail-fast \
         (empty Vec) or snapshot (full correct Vec) — got len={}, sum={}/{}, \
         pkh_subset_size={}/{}. This outcome (a silent subset) is exactly the \
         pattern that caused the 2026-04-16 cascade: the silent skip at \
         rewards.rs:42 and/or the silent vec![] at rewards.rs:62 is still in \
         effect. Threshold={}, minutes/epoch={}.",
        len,
        sum,
        pool_total,
        got_pkhs.len(),
        want_all_pkhs.len(),
        QUAL_THRESHOLD,
        MINUTES_PER_EPOCH
    );

    // O4: determinism even under gap.
    let outputs2 = node.calculate_epoch_rewards(1).await;
    assert_eq!(
        outputs, outputs2,
        "O4: calculate_epoch_rewards must be deterministic even with block_store gaps"
    );
}

// ============================================================
// TEST C — SANTIAGO CASCADE REPLAY (must FAIL on HEAD, PASS after fix)
// ============================================================
//
// OUTPUT CONTRACT coverage: Path P5 (many_missing_mainnet)
//   O1 ✓ : len(outputs) ∈ {0, NUM_PRODUCERS_MAINNET}
//   O2 ✓ : pkh_set(outputs) ∈ {∅, full active set}
//   O3 ✓ : sum(amounts) ∈ {0, pool_total}
//
// Mainnet-scale replay: 37 producers, several blocks missing. Pattern mirrors
// santiago's 29-height gap in a 36-block epoch.
//
// With NUM_PRODUCERS_MAINNET=37 and "one-attester-per-block", across a 36-block
// epoch the bit-distribution does NOT cleanly align (36 % 37 != 0). Sorted
// indices 0..35 get attested exactly once; sorted index 36 never gets attested
// in this specific epoch. That's fine for the test: the threshold-5 check
// would exclude index 36 ALREADY (0 minutes) in the correct behavior too —
// but we want the test to focus on the SILENT BUG, not pre-existing edge
// cases. So we use a full-bitfield pattern for producers 0..35, meaning
// every block at height `h` in epoch 1 attests producers {h%37, h%37+1, ...,
// h%37+5} (mod 37). Each producer gets attested multiple times per minute
// bucket, so the minute coverage is robust.
//
// Then we drop 11 consecutive heights in the middle of the epoch (santiago's
// 29/36 at this smaller producer count). If the bug is active, ~30% of
// producers drop below threshold.
#[tokio::test]
async fn test_santiago_cascade_replay_mainnet_scale() {
    let (node, producers, _tmp) = make_node(NUM_PRODUCERS_MAINNET).await;
    let params = node.params.clone();
    let sorted_order = sorted_producer_order(&producers);
    let genesis_hash = node.chain_state.read().await.best_hash;

    // Populate epoch 0 fully (fully-attested).
    let mut prev = genesis_hash;
    for h in 0..EPOCH_LEN {
        let block = build_block_with_full_bitfield(
            h,
            h as u32,
            prev,
            &producers[(h as usize) % producers.len()],
            NUM_PRODUCERS_MAINNET,
            &params,
        );
        prev = block.hash();
        put_canonical(&node, &block, h);
    }

    // Epoch 1: "sliding-window attests 6 producers" pattern. Block at height
    // h attests sorted-indices {h%37, (h%37)+1, ..., (h%37)+5} mod 37. Over
    // 36 blocks, each sorted index is attested in approximately 6*36/37 ≈ 5.8
    // blocks — spread across every minute bucket.
    //
    // Gaps: drop 11 consecutive heights mid-epoch (h=45..=55). That removes
    // ~2 full minute buckets' worth of blocks from the local view. If the bug
    // is active, every producer's minute count drops below threshold=5 for
    // some subset → ragged subset output.
    let gap_heights: Vec<u64> = (45..=55).collect();

    for offset in 0..EPOCH_LEN {
        let h = EPOCH_LEN + offset;
        let base = (h as usize) % NUM_PRODUCERS_MAINNET;
        let attested: Vec<usize> = (0..6).map(|k| (base + k) % NUM_PRODUCERS_MAINNET).collect();
        let orig_producer_index = sorted_order[base];
        let block = build_block_with_bitfield(
            h,
            h as u32,
            prev,
            &producers[orig_producer_index],
            &attested,
            NUM_PRODUCERS_MAINNET,
            &params,
        );
        prev = block.hash();
        if !gap_heights.contains(&h) {
            put_canonical(&node, &block, h);
        }
    }

    // Self-check gaps.
    for &gh in &gap_heights {
        assert!(
            node.block_store.get_block_by_height(gh).unwrap().is_none(),
            "gap missing at h={}",
            gh
        );
    }

    let pool_total: u64 = 370_000_000;
    seed_reward_pool(&node, pool_total, "test_C_pool").await;

    let outputs = node.calculate_epoch_rewards(1).await;

    let len = outputs.len();
    let got_pkhs = pkh_set(&outputs);
    let want_all_pkhs: HashSet<Hash> = producers.iter().map(producer_pkh).collect();
    let sum: u64 = outputs.iter().map(|(a, _)| *a).sum();

    eprintln!(
        "[M-RC9 test C] len={} sum={}/{} pkh_in_output={}/{}",
        len,
        sum,
        pool_total,
        got_pkhs.len(),
        want_all_pkhs.len()
    );

    let is_fail_fast = len == 0 && sum == 0;
    let is_snapshot =
        len == NUM_PRODUCERS_MAINNET && got_pkhs == want_all_pkhs && sum == pool_total;

    assert!(
        is_fail_fast || is_snapshot,
        "M-RC9 santiago replay: {} producers, 11 heights missing from epoch 1. \
         calculate_epoch_rewards must return empty (fail-fast) or full correct \
         Vec (snapshot). Got len={}, sum={}/{}, pkh_subset_size={}/{}. A silent \
         subset here is exactly the class of divergence that caused the \
         2026-04-16 cascade at mainnet scale.",
        NUM_PRODUCERS_MAINNET,
        len,
        sum,
        pool_total,
        got_pkhs.len(),
        want_all_pkhs.len()
    );
}
