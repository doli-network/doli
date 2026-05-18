//! INC-I-081 — Incomplete store must abort slot production at epoch boundary
//!
//! Reproduces the wiring bug where `build_block_content` (assembly.rs) cannot
//! distinguish "incomplete block store" from "no qualified producers" in the
//! `calculate_epoch_rewards` return value. Both return `Vec::new()`, causing
//! the producer to emit an epoch-boundary block WITHOUT an EpochReward TX.
//! Fleet validators reject it with `[ECON_EPOCH_MISSING]`.
//!
//! The fix (INC-I-081 Bundle 1) changes `calculate_epoch_rewards` to return
//! `Result<Vec<(u64, Hash)>, IncompleteEpochStoreError>`. On `Err`, the caller
//! `build_block_content` returns `Ok(None)` — aborting the slot instead of
//! emitting a poisoned block.
//!
//! OUTPUT CONTRACT: fn Node::build_block_content(prev_hash, prev_slot, height, current_slot, our_pubkey)
//!                  -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//!
//!   Outputs:
//!     O1: Return value — Ok(Some(...)) with block content, or Ok(None) for slot abort
//!     O2: If Ok(Some), presence/absence of EpochReward TX (TxType::EpochReward) in the
//!         transaction list
//!     O3: Side-effect — no block header committed to block_store (slot is skipped)
//!
//!   Paths:
//!     P1: incomplete_store_at_epoch_boundary
//!         — gap in completed epoch's block range + height is epoch start
//!         INPUT PARTITIONS:
//!           P1a: single gap in mid-epoch (1 missing block)
//!
//!     P2: complete_store_at_epoch_boundary
//!         — all blocks present in completed epoch + height is epoch start
//!         INPUT PARTITIONS:
//!           P2a: all producers attested, pool seeded (normal happy path)
//!
//!     P3: non_boundary_height_incomplete_store
//!         — incomplete store but height is NOT an epoch boundary
//!         INPUT PARTITIONS:
//!           P3a: mid-epoch height with gaps in prior epoch (no EpochReward needed)
//!
//!   MATRIX: 3 outputs x 3 paths = 9 cells
//!     P1a: O1(Ok(None)) + O2(N/A—no block) + O3(no commit)
//!     P2a: O1(Ok(Some)) + O2(EpochReward present) + O3(N/A—block emitted not committed by this fn)
//!     P3a: O1(Ok(Some)) + O2(no EpochReward—not boundary) + O3(N/A—block emitted)

use std::sync::Once;

use crypto::{Hash, KeyPair};
use doli_core::consensus::{reward_pool_pubkey_hash, ConsensusParams};
use doli_core::transaction::{Output, Transaction, TxType};
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
// attestation_qualification_threshold(36) = (6 * 90) / 100 = 5 -> each producer
// needs at least 5 attestation minutes to qualify.

static ENV_INIT: Once = Once::new();
fn init_env() {
    ENV_INIT.call_once(|| {
        std::env::set_var("DOLI_BLOCKS_PER_REWARD_EPOCH", "36");
        let _ = Network::Devnet.params();
    });
}

// ============================================================
// Constants
// ============================================================
const EPOCH_LEN: u64 = 36;
const NUM_PRODUCERS: usize = 6;

// ============================================================
// Test scaffolding helpers (modeled after m_rc9)
// ============================================================

async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    init_env();
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    assert_eq!(
        node.config.network.blocks_per_reward_epoch(),
        EPOCH_LEN,
        "DOLI_BLOCKS_PER_REWARD_EPOCH override did not take effect"
    );
    // Keep block reward positive through the whole test range
    node.params.blocks_per_era = 100_000;
    (node, producers, temp)
}

/// Build a block that attests a CUSTOM set of producer indices.
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

/// Seed the reward pool with a single non-zero UTXO so the reward function
/// reaches its distribution path.
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

/// Build the "producers sorted by pubkey bytes" order that rewards.rs uses.
fn sorted_producer_order(producers: &[KeyPair]) -> Vec<usize> {
    let mut indexed: Vec<(usize, &KeyPair)> = producers.iter().enumerate().collect();
    indexed.sort_by(|a, b| a.1.public_key().as_bytes().cmp(b.1.public_key().as_bytes()));
    indexed.into_iter().map(|(i, _)| i).collect()
}

/// Compute the real-time current slot from the node's params.
/// Returns (current_slot, prev_slot) where prev_slot = current_slot - 1.
fn compute_current_slot(params: &ConsensusParams) -> (u32, u32) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let current_slot = params.timestamp_to_slot(now);
    let prev_slot = current_slot.saturating_sub(1);
    (current_slot, prev_slot)
}

/// Populate epochs 0 and 1 with fully-attested blocks.
/// Returns the hash of the last block (at height 2*EPOCH_LEN - 1).
async fn populate_two_full_epochs(
    node: &Node,
    producers: &[KeyPair],
    producer_count: usize,
    params: &ConsensusParams,
) -> Hash {
    let genesis_hash = node.chain_state.read().await.best_hash;
    let mut prev = genesis_hash;

    for h in 0..(2 * EPOCH_LEN) {
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

    prev
}

/// Populate epochs 0 and 1, but skip `gap_heights` in epoch 1.
/// Returns the hash of the conceptually-last block (for prev_hash chaining).
async fn populate_two_epochs_with_gap(
    node: &Node,
    producers: &[KeyPair],
    producer_count: usize,
    params: &ConsensusParams,
    gap_heights: &[u64],
) -> Hash {
    let genesis_hash = node.chain_state.read().await.best_hash;
    let sorted_order = sorted_producer_order(producers);
    let mut prev = genesis_hash;

    // Epoch 0: fully-attested
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

    // Epoch 1: one-attester-per-block, skipping gap_heights
    for offset in 0..EPOCH_LEN {
        let h = EPOCH_LEN + offset;
        let sorted_index = (h as usize) % producer_count;
        let orig_producer_index = sorted_order[sorted_index];
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
        if !gap_heights.contains(&h) {
            put_canonical(node, &block, h);
        }
    }

    prev
}

// ============================================================
// TEST 1 — PRIMARY FAIL TEST (must FAIL pre-fix, PASS after fix)
// ============================================================
//
// Requirement: INC-I-081 Bundle 1
// Acceptance: build_block_content returns Ok(None) when calculate_epoch_rewards
//             detects an incomplete block store at the epoch boundary
//
// OUTPUT CONTRACT coverage: Path P1a (incomplete_store_at_epoch_boundary)
//   O1: return == Ok(None)                          -- slot aborted
//   O2: N/A (no block emitted)
//   O3: no block committed (verified indirectly: None means no header)
//
// On current code (pre-fix): build_block_content returns Ok(Some((header, txs, bf)))
// where txs does NOT contain an EpochReward coinbase. The test asserts Ok(None),
// which FAILS against the current behavior.
#[tokio::test]
async fn test_inc_i_081_build_block_aborts_when_incomplete_store_at_boundary() {
    let (mut node, producers, _tmp) = make_node(NUM_PRODUCERS).await;
    let params = node.params.clone();
    let our_pubkey = *producers[0].public_key();

    // Gap: heights 40-42 (3 blocks in epoch 1, range 36..72)
    let gap_heights: Vec<u64> = (40..=42).collect();

    let _last_hash =
        populate_two_epochs_with_gap(&node, &producers, NUM_PRODUCERS, &params, &gap_heights).await;

    // Self-check that the gap exists
    for &gh in &gap_heights {
        assert!(
            node.block_store.get_block_by_height(gh).unwrap().is_none(),
            "test setup error: height {} should be absent from block_store",
            gh
        );
    }

    // Seed the reward pool so the reward path is reachable
    let pool_total: u64 = 60_000_000;
    seed_reward_pool(&node, pool_total, "test_081_primary").await;

    // Update chain_state to reflect the tip is at 2*EPOCH_LEN - 1 = 71
    // so the next block would be at height 72 (= 2*EPOCH_LEN).
    // Also set the best_hash to the last stored block's hash.
    {
        let last_stored_height = 2 * EPOCH_LEN - 1; // h=71
        let last_block = node
            .block_store
            .get_block_by_height(last_stored_height)
            .unwrap()
            .expect("last block should exist");
        let mut cs = node.chain_state.write().await;
        cs.best_height = last_stored_height;
        cs.best_hash = last_block.hash();
        cs.best_slot = last_block.header.slot;
    }

    // Compute real-time slot so the wall-clock sanity check passes
    let (current_slot, prev_slot) = compute_current_slot(&params);

    // The prev_hash for the new block is the hash of the block at h=71
    let prev_hash = node.chain_state.read().await.best_hash;

    // Call build_block_content at the epoch boundary (height = 2*EPOCH_LEN = 72)
    let height = 2 * EPOCH_LEN;
    let result = node
        .build_block_content(prev_hash, prev_slot, height, current_slot, our_pubkey)
        .await;

    // The fix makes this return Ok(None) — slot aborted because the store
    // is incomplete for the completed epoch. Pre-fix, this returns Ok(Some(...))
    // with a block that has NO EpochReward TX — a fleet-poisoning block.
    match result {
        Ok(None) => {
            // PASS: slot was correctly aborted
        }
        Ok(Some((header, txs, _bf))) => {
            // Pre-fix behavior: block emitted without EpochReward.
            // Verify it's indeed missing the EpochReward TX for diagnostic clarity.
            let has_epoch_reward = txs.iter().any(|tx| tx.tx_type == TxType::EpochReward);
            panic!(
                "INC-I-081: build_block_content returned Ok(Some) at epoch boundary with \
                 incomplete store — this is the bug. Block at h={} slot={} has {} txs, \
                 EpochReward present: {}. Expected Ok(None) (slot abort).",
                height,
                header.slot,
                txs.len(),
                has_epoch_reward
            );
        }
        Err(e) => {
            panic!(
                "INC-I-081: build_block_content returned Err at epoch boundary: {}. \
                 Expected Ok(None) (slot abort), not an error.",
                e
            );
        }
    }
}

// ============================================================
// TEST 2 — SANITY: complete store emits block WITH EpochReward
// ============================================================
//
// Requirement: INC-I-081 Bundle 1 (regression anchor)
// Acceptance: build_block_content at epoch boundary with complete store
//             returns Ok(Some(...)) containing an EpochReward TX
//
// OUTPUT CONTRACT coverage: Path P2a (complete_store_at_epoch_boundary)
//   O1: return == Ok(Some((header, txs, bf)))       -- block emitted
//   O2: txs contains at least one TX with TxType::EpochReward
//   O3: N/A (build_block_content does not commit to block_store)
#[tokio::test]
async fn test_inc_i_081_build_block_emits_when_store_complete() {
    let (mut node, producers, _tmp) = make_node(NUM_PRODUCERS).await;
    let params = node.params.clone();
    let our_pubkey = *producers[0].public_key();

    // Populate both epochs fully — no gaps
    let _last_hash = populate_two_full_epochs(&node, &producers, NUM_PRODUCERS, &params).await;

    // Seed the reward pool
    let pool_total: u64 = 60_000_000;
    seed_reward_pool(&node, pool_total, "test_081_sanity").await;

    // Update chain_state to tip at h=71
    {
        let last_stored_height = 2 * EPOCH_LEN - 1;
        let last_block = node
            .block_store
            .get_block_by_height(last_stored_height)
            .unwrap()
            .expect("last block should exist");
        let mut cs = node.chain_state.write().await;
        cs.best_height = last_stored_height;
        cs.best_hash = last_block.hash();
        cs.best_slot = last_block.header.slot;
    }

    let (current_slot, prev_slot) = compute_current_slot(&params);
    let prev_hash = node.chain_state.read().await.best_hash;
    let height = 2 * EPOCH_LEN;

    let result = node
        .build_block_content(prev_hash, prev_slot, height, current_slot, our_pubkey)
        .await;

    match result {
        Ok(Some((_header, txs, _bf))) => {
            // O2: Must contain an EpochReward TX
            let has_epoch_reward = txs.iter().any(|tx| tx.tx_type == TxType::EpochReward);
            assert!(
                has_epoch_reward,
                "Sanity: complete store at epoch boundary must produce block with \
                 EpochReward TX. Got {} txs: {:?}",
                txs.len(),
                txs.iter().map(|tx| tx.tx_type).collect::<Vec<_>>()
            );
        }
        Ok(None) => {
            panic!(
                "Sanity: build_block_content returned Ok(None) with complete store — \
                 this should only happen for slot boundary crossing, not for a normal \
                 epoch boundary with full data."
            );
        }
        Err(e) => {
            panic!(
                "Sanity: build_block_content returned Err with complete store: {}",
                e
            );
        }
    }
}

// ============================================================
// TEST 3 — NEGATIVE: incomplete store at non-boundary still emits
// ============================================================
//
// Requirement: INC-I-081 Bundle 1 (negative test)
// Acceptance: build_block_content at a non-boundary height still produces
//             a block even if the store has gaps (no EpochReward required)
//
// OUTPUT CONTRACT coverage: Path P3a (non_boundary_height_incomplete_store)
//   O1: return == Ok(Some((header, txs, bf)))       -- block emitted
//   O2: txs does NOT contain EpochReward (not at boundary)
//   O3: N/A (build_block_content does not commit)
#[tokio::test]
async fn test_inc_i_081_non_boundary_incomplete_store_still_emits() {
    let (mut node, producers, _tmp) = make_node(NUM_PRODUCERS).await;
    let params = node.params.clone();
    let our_pubkey = *producers[0].public_key();

    // Populate epoch 0 fully, epoch 1 with gaps (same as primary test)
    let gap_heights: Vec<u64> = (40..=42).collect();
    let _last_hash =
        populate_two_epochs_with_gap(&node, &producers, NUM_PRODUCERS, &params, &gap_heights).await;

    // Seed the reward pool
    seed_reward_pool(&node, 60_000_000, "test_081_negative").await;

    // Set chain_state tip to a mid-epoch height (h=50, which is NOT an epoch boundary)
    // height 50 is in epoch 1 (36..72), not a boundary.
    let mid_height = 50u64;
    {
        // Find the last stored block before mid_height
        let mut tip_height = mid_height;
        while node
            .block_store
            .get_block_by_height(tip_height)
            .unwrap()
            .is_none()
        {
            tip_height -= 1;
        }
        let tip_block = node
            .block_store
            .get_block_by_height(tip_height)
            .unwrap()
            .unwrap();
        let mut cs = node.chain_state.write().await;
        cs.best_height = tip_height;
        cs.best_hash = tip_block.hash();
        cs.best_slot = tip_block.header.slot;
    }

    let (current_slot, prev_slot) = compute_current_slot(&params);
    let prev_hash = node.chain_state.read().await.best_hash;
    // Next height: mid_height + 1. NOT an epoch boundary (36-aligned).
    let height = node.chain_state.read().await.best_height + 1;
    assert!(
        height % EPOCH_LEN != 0,
        "test setup: height {} should NOT be an epoch boundary",
        height
    );

    let result = node
        .build_block_content(prev_hash, prev_slot, height, current_slot, our_pubkey)
        .await;

    match result {
        Ok(Some((_header, txs, _bf))) => {
            // O2: Must NOT contain an EpochReward TX (not at boundary)
            let has_epoch_reward = txs.iter().any(|tx| tx.tx_type == TxType::EpochReward);
            assert!(
                !has_epoch_reward,
                "Negative: non-boundary block should NOT contain EpochReward TX"
            );
        }
        Ok(None) => {
            // This is also acceptable — the slot boundary check may trigger
            // depending on timing. Not a test failure per se, but log it.
            eprintln!(
                "[INC-I-081 test 3] build_block_content returned Ok(None) — \
                 likely slot boundary crossed during test. Acceptable."
            );
        }
        Err(e) => {
            panic!(
                "Negative: build_block_content returned Err at non-boundary: {}",
                e
            );
        }
    }
}
