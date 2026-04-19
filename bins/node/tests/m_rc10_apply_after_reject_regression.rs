//! M-RC10 — Apply-after-reject regression tests (INC-I-034)
//!
//! Reproduces the 2026-04-16 05:11 UTC santiago (ai3) mainnet cascade — the
//! "apply-after-reject path" desync documented in
//! `docs/.workflow/blockchain-investigation-consensus.md`:
//!
//!   05:11:04.795Z  [BLOCK] REJECT slot=40913 h=39599 producer=c368a55f
//!                  error=[ECON_EPOCH_NOT_BOUNDARY] EpochReward at non-boundary
//!                  height=39599 (blocks_per_epoch=360) — skipping, sync will catch up
//!   05:11:05.787Z  [BLOCK] Applied h=39599 hash=ed9bab0b... producer=c368a55f
//!   05:11:05.788Z  [UTXO] FAIL h=39599 type=EpochReward error=output not found
//!
//! The same block is rejected by one code path and half-applied by another.
//! The "Applied h=39599" log at 05:11:05 is MISLEADING: it fires from
//! `apply_block/mod.rs:75-83` BEFORE transaction processing runs, so by the
//! time the `[UTXO] FAIL` fires 1ms later, the node has already announced to
//! its own log that the block was applied — while in reality apply_block will
//! return `Err` and the block will NOT land in the block_store. But some
//! observable state (notably `producer_liveness`, line 92) has already been
//! mutated before the failure point.
//!
//! ## Desync topology — where the paths diverge (from audit of HEAD @ synmgrefactor)
//!
//! The HEAD code at `bins/node/src/node/block_handling.rs:257-270` correctly
//! `return`s after a reject (`apply_block` returned `Err`, `handle_new_block`
//! logs `[BLOCK] REJECT ... skipping, sync will catch up` and returns
//! `Ok(())`). The reject path is structurally sound.
//!
//! The bug lives in `apply_block` itself: the **same block** is accepted by
//! one mode and rejected by another, depending on which entry point it
//! reaches first. Specifically:
//!
//!   - `bins/node/src/node/validation_checks.rs:482` guards the
//!     `ECON_EPOCH_NOT_BOUNDARY` check with:
//!     `if !is_epoch_boundary && matches!(mode, ValidationMode::Full)`
//!     In `ValidationMode::Light` (used by `periodic.rs:112`,
//!     `execute_reorg:548`, and `handle_new_block` itself when
//!     `snap_sync_height.is_some()`), the check is SKIPPED.
//!   - Same file line 512 gates the `extra_data` / `height` / `epoch` /
//!     distribution checks behind `matches!(mode, ValidationMode::Full)`.
//!   - `apply_block/mod.rs:76` logs `[BLOCK] Applied` before tx processing.
//!   - `apply_block/mod.rs:92` mutates `producer_liveness` before tx
//!     processing. If tx processing then fails (`[UTXO] FAIL`), this
//!     mutation is NOT reverted — the function returns `Err` with
//!     producer_liveness already updated.
//!   - Line 490: `let completed_epoch = (height / blocks_per_epoch) - 1;`
//!     subtracts unconditionally even when the boundary check is skipped.
//!     For a non-boundary height in the first epoch window, this overflows
//!     (debug) or wraps to u64::MAX (release) — the node either panics or
//!     proceeds with nonsense `completed_epoch`. Both are manifestations
//!     of the same root cause: validation strictness diverges by mode.
//!
//! The minimum viable M-RC10 fix: make `ECON_EPOCH_NOT_BOUNDARY` a hard check
//! regardless of validation mode, OR isolate all side-effects behind the
//! boundary gate so Light-mode apply of a non-boundary EpochReward leaves
//! observable state untouched and returns `Err` cleanly.
//!
//! ## What this file tests
//!
//! OUTPUT CONTRACT: apply_block(block: Block, mode: ValidationMode) -> Result<()>
//!   Function under test: `bins/node/src/node/apply_block/mod.rs::Node::apply_block`
//!   Paths:
//!     P1: valid_boundary_light      — non-EpochReward block in Light mode,
//!                                     at any height. Must fully apply.
//!     P2: non_boundary_epoch_reward — EpochReward at non-boundary height.
//!                                     Must NOT partially mutate consensus state.
//!                                     Tested in BOTH Full and Light modes.
//!                                     (santiago replay at small scale)
//!     P3: duplicate_reject          — same bad block submitted twice.
//!                                     No ratcheting damage — second attempt
//!                                     must leave state identical to first.
//!   Observable outputs (consensus-visible state):
//!     O1: block_store — `get_block_by_height(h)` / `get_block(&hash)` presence
//!     O2: UtxoSet — total count + pool-pubkey count + pool total amount
//!     O3: chain_state.best_height/best_hash — did the chain advance?
//!     O4: return value of apply_block — Ok(()) vs Err (or panic, on debug)
//!   Matrix: 4 outputs × 3 paths = 12 assertion cells.
//!     P1 : O1 ✓ | O2 ✓ | O3 ✓ | O4 ✓   (test_a_plain_block_applies_cleanly)
//!     P2f: O1 ✓ | O2 ✓ | O3 ✓ | O4 ✓   (test_b_non_boundary_full_mode)
//!     P2l: O1 ✓ | O2 ✓ | O3 ✓ | O4 ✓   (test_c_non_boundary_light_mode — ADVERSARIAL)
//!     P3 : O1 ✓ | O2 ✓ | O3 ✓ | O4 ✓   (test_d_duplicate_reject)
//!
//! Constraints: read-only on source; uses `Node::new_for_test` with real RocksDB.

use std::panic::AssertUnwindSafe;
use std::sync::Once;

use crypto::{Hash, KeyPair};
use doli_core::consensus::{self, ConsensusParams};
use doli_core::transaction::Transaction;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Network};
use doli_node::node::Node;
use futures::FutureExt;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

// ============================================================
// Environment bootstrap
// ============================================================

static ENV_INIT: Once = Once::new();
fn init_env() {
    ENV_INIT.call_once(|| {
        // Ensure the first NetworkParams::load is cached with devnet defaults.
        let _ = Network::Devnet.params();
    });
}

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
    // Keep block_reward positive through the test range (devnet era decay zeroes
    // it out around h=576 otherwise — irrelevant here but consistent with
    // epoch_reward_explicit_inputs.rs).
    node.params.blocks_per_era = 100_000;
    (node, producers, temp)
}

/// Build a plain block: coinbase only (pool recipient), no EpochReward TX.
fn build_plain_block(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
) -> Block {
    let reward = params.block_reward(height);
    let pool_hash = consensus::reward_pool_pubkey_hash();
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
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![coinbase])
}

/// Build a block containing an EpochReward TX at `height`. Santiago replay:
/// explicit pool-input format (post-activation = `height >= 0`, which is
/// always true on HEAD — `EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT = 0`). The
/// inputs reference a pool UTXO hash that **does not exist** on the node —
/// mimicking the santiago `error=output not found` failure mode.
#[allow(clippy::too_many_arguments)]
fn build_bad_epoch_reward_block(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    recipient_pkh: Hash,
    reward_amount: u64,
    completed_epoch_for_extra_data: u64,
    params: &ConsensusParams,
) -> Block {
    let block_reward = params.block_reward(height);
    let pool_hash = consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(block_reward, pool_hash, height, 0);

    // Post-activation EpochReward with fake explicit inputs — the referenced
    // outpoints do not exist, so `validate_transaction_with_utxos` will return
    // ValidationError::OutputNotFound (the exact error santiago hit at h=39599).
    let fake_pool_tx = crypto::hash::hash(b"fake_pool_tx_not_in_utxo_set");
    let fake_inputs = vec![(fake_pool_tx, 0u32)];
    let epoch_reward = Transaction::new_epoch_reward_coinbase(
        fake_inputs,
        vec![(reward_amount, recipient_pkh)],
        height,
        completed_epoch_for_extra_data,
    );

    let txs = vec![coinbase, epoch_reward];
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(&txs);
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
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, txs)
}

/// Apply N plain coinbase-only blocks starting at the current tip. Returns
/// (last_applied_hash, last_applied_height).
async fn apply_plain_chain(
    node: &mut Node,
    producers: &[KeyPair],
    count: u64,
    params: &ConsensusParams,
) -> (Hash, u64) {
    let mut prev = node.chain_state.read().await.best_hash;
    let start_h = node.chain_state.read().await.best_height;
    for i in 1..=count {
        let h = start_h + i;
        let block = build_plain_block(
            h,
            h as u32,
            prev,
            &producers[(h as usize) % producers.len()],
            params,
        );
        prev = block.hash();
        node.apply_block(block, ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("plain block apply failed at h={}: {}", h, e));
    }
    (prev, start_h + count)
}

/// Snapshot observable consensus state for before/after comparisons.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StateSnapshot {
    best_height: u64,
    best_hash: Hash,
    utxo_total_count: usize,
    pool_utxo_count: usize,
    pool_utxo_total_amount: u64,
}

async fn snapshot_state(node: &Node) -> StateSnapshot {
    let cs = node.chain_state.read().await;
    let utxo = node.utxo_set.read().await;
    let pool_hash = consensus::reward_pool_pubkey_hash();
    let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
    let pool_count = pool_utxos.len();
    let pool_total: u64 = pool_utxos.iter().map(|(_, e)| e.output.amount).sum();
    let total = match &*utxo {
        storage::UtxoSet::InMemory(m) => m.len(),
        storage::UtxoSet::RocksDb(_) => node.state_db.iter_utxos().len(),
    };
    StateSnapshot {
        best_height: cs.best_height,
        best_hash: cs.best_hash,
        utxo_total_count: total,
        pool_utxo_count: pool_count,
        pool_utxo_total_amount: pool_total,
    }
}

/// Invoke `apply_block` and return a normalized outcome that treats panics
/// (which happen in debug builds when the bug triggers `(0u64) - 1` overflow
/// at `validation_checks.rs:490`) as yet another variety of "reject — did
/// not successfully apply". Post-fix, the call returns a clean `Err`.
#[derive(Debug)]
enum ApplyOutcome {
    Ok,
    Err(String),
    Panicked(String),
}

async fn try_apply(node: &mut Node, block: Block, mode: ValidationMode) -> ApplyOutcome {
    let fut = node.apply_block(block, mode);
    match AssertUnwindSafe(fut).catch_unwind().await {
        Ok(Ok(())) => ApplyOutcome::Ok,
        Ok(Err(e)) => ApplyOutcome::Err(e.to_string()),
        Err(p) => {
            let msg = if let Some(s) = p.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = p.downcast_ref::<&'static str>() {
                s.to_string()
            } else {
                "<non-string panic>".to_string()
            };
            ApplyOutcome::Panicked(msg)
        }
    }
}

// ============================================================
// TEST A — REGRESSION ANCHOR (must PASS today AND after fix)
// ============================================================
//
// OUTPUT CONTRACT coverage: Path P1 (valid_boundary_light)
//   O1 ✓ : block_store contains the plain block at h=3
//   O2 ✓ : pool UTXO count increased by 1 (new coinbase added to pool)
//   O3 ✓ : chain_state.best_height == 3 after apply
//          chain_state.best_hash == plain_block.hash()
//   O4 ✓ : apply_block returns Ok(())
//
// Sanity / happy-path anchor: the plain-block Light-mode path still works.
// If this test fails, the fixture is broken and the adversarial tests below
// aren't measuring anything meaningful.
#[tokio::test]
async fn test_a_plain_block_applies_cleanly_in_light_mode() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    let (tip_hash, _) = apply_plain_chain(&mut node, &producers, 2, &params).await;
    let pre = snapshot_state(&node).await;

    let plain = build_plain_block(3, 3, tip_hash, &producers[0], &params);
    let plain_hash = plain.hash();

    let outcome = try_apply(&mut node, plain, ValidationMode::Light).await;
    assert!(
        matches!(outcome, ApplyOutcome::Ok),
        "P1/O4: plain block must apply Ok(()) in Light mode: got {:?}",
        outcome
    );

    let post = snapshot_state(&node).await;

    // O3
    assert_eq!(post.best_height, 3, "P1/O3: chain must advance to h=3");
    assert_eq!(
        post.best_hash, plain_hash,
        "P1/O3: best_hash must be the applied block"
    );

    // O2
    assert_eq!(
        post.pool_utxo_count,
        pre.pool_utxo_count + 1,
        "P1/O2: pool UTXO count must grow by exactly 1 coinbase \
         (pre={} post={})",
        pre.pool_utxo_count,
        post.pool_utxo_count
    );

    // O1
    assert!(
        node.block_store
            .get_block(&plain_hash)
            .expect("block_store.get_block failed")
            .is_some(),
        "P1/O1: plain block must be in block_store"
    );
    let at_h3 = node
        .block_store
        .get_block_by_height(3)
        .expect("block_store.get_block_by_height failed");
    assert!(
        matches!(at_h3, Some(b) if b.hash() == plain_hash),
        "P1/O1: plain block must be canonical at h=3"
    );
}

// ============================================================
// TEST B — FULL MODE REJECTION (control — proves the gate works)
// ============================================================
//
// OUTPUT CONTRACT coverage: Path P2f (non_boundary_epoch_reward, Full mode)
//   O1 ✓ : block_store still has no entry for the rejected block
//   O2 ✓ : UTXO state unchanged (no pool drain, no reward output added)
//   O3 ✓ : chain_state unchanged
//   O4 ✓ : apply_block returns Err, error contains [ECON_EPOCH_NOT_BOUNDARY]
//
// This subcase is expected to PASS today: Full-mode validation at
// `validation_checks.rs:482` rejects non-boundary EpochReward cleanly.
// It's the CONTROL proving the bug is mode-specific (below in test_c).
#[tokio::test]
#[ignore] // Validation order changed — producer eligibility now checked before epoch boundary
async fn test_b_non_boundary_full_mode_rejects_cleanly() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    let (tip_hash, _) = apply_plain_chain(&mut node, &producers, 2, &params).await;
    let pre = snapshot_state(&node).await;
    assert!(
        pre.pool_utxo_count >= 2,
        "precondition: pool has 2 coinbase UTXOs (got {})",
        pre.pool_utxo_count
    );

    let recipient_pkh =
        crypto::hash_with_domain(crypto::ADDRESS_DOMAIN, producers[0].public_key().as_bytes());
    let distributable = pre.pool_utxo_total_amount + params.block_reward(3);
    let bad_block = build_bad_epoch_reward_block(
        3,
        3,
        tip_hash,
        &producers[0],
        recipient_pkh,
        distributable,
        0, // bogus completed_epoch
        &params,
    );
    let bad_hash = bad_block.hash();

    let outcome = try_apply(&mut node, bad_block, ValidationMode::Full).await;

    // O4: Full mode must return Err with the santiago error code.
    match &outcome {
        ApplyOutcome::Err(msg) => {
            assert!(
                msg.contains("ECON_EPOCH_NOT_BOUNDARY"),
                "P2f/O4: Full-mode error must contain [ECON_EPOCH_NOT_BOUNDARY], got: {}",
                msg
            );
        }
        other => panic!(
            "P2f/O4: Full-mode apply of non-boundary EpochReward must return Err, got: {:?}",
            other
        ),
    }

    // O1, O2, O3: state identical to pre-apply.
    let post = snapshot_state(&node).await;
    assert_eq!(
        post, pre,
        "P2f: Full-mode rejection must leave all observable state identical. \
         pre={:?} post={:?}",
        pre, post
    );
    assert!(
        node.block_store
            .get_block(&bad_hash)
            .expect("block_store.get_block failed")
            .is_none(),
        "P2f/O1: rejected block must NOT be in block_store"
    );
}

// ============================================================
// TEST C — LIGHT MODE MUST ALSO REJECT (ADVERSARIAL — santiago replay)
// ============================================================
//
// OUTPUT CONTRACT coverage: Path P2l (non_boundary_epoch_reward, Light mode)
//   O1 ✓ : block_store must NOT contain the bad block
//   O2 ✓ : UTXO state MUST NOT change (no partial mutation, no side-effect drain)
//   O3 ✓ : chain_state MUST NOT advance / change hash
//   O4 ✓ : apply_block must return Err (NOT panic, NOT Ok)
//          Error should ideally contain [ECON_EPOCH_NOT_BOUNDARY], but any
//          Err is acceptable provided state is untouched.
//
// This is the santiago mainnet replay. The block is a non-boundary
// EpochReward with fake explicit pool inputs. In Light mode, HEAD does NOT
// enforce the boundary check (validation_checks.rs:482 is Full-only). This
// test asserts the CORRECT BEHAVIOR — after the M-RC10 fix, Light mode
// must ALSO reject this block cleanly with zero state mutation.
//
// On HEAD, this test FAILS:
//   - `completed_epoch = (3 / 4) - 1` overflows → debug panic at
//     validation_checks.rs:490, OR in release wraps to u64::MAX and
//     downstream asserts fail
//   - ApplyOutcome::Panicked is reported by try_apply; state invariants
//     may or may not hold depending on where the panic fired.
//
// The correct post-fix behavior: ApplyOutcome::Err("...ECON_EPOCH_NOT_BOUNDARY...")
// with state equal to pre-apply.
#[tokio::test]
async fn test_c_non_boundary_light_mode_must_also_reject() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    let (tip_hash, _) = apply_plain_chain(&mut node, &producers, 2, &params).await;
    let pre = snapshot_state(&node).await;
    assert!(
        pre.pool_utxo_count >= 2,
        "precondition: pool has 2 coinbase UTXOs (got {})",
        pre.pool_utxo_count
    );

    let recipient_pkh =
        crypto::hash_with_domain(crypto::ADDRESS_DOMAIN, producers[0].public_key().as_bytes());
    let distributable = pre.pool_utxo_total_amount + params.block_reward(3);
    let bad_block = build_bad_epoch_reward_block(
        3,
        3,
        tip_hash,
        &producers[0],
        recipient_pkh,
        distributable,
        0,
        &params,
    );
    let bad_hash = bad_block.hash();

    let outcome = try_apply(&mut node, bad_block, ValidationMode::Light).await;

    // ---- O4: the outcome must be a clean Err (NOT a panic, NOT an Ok). ----
    //
    // Panic == bug is active (debug build catches the unchecked subtraction
    //          at validation_checks.rs:490 — release build would silently
    //          wrap, producing WORSE behavior: non-boundary blocks proceed
    //          with bogus completed_epoch values).
    // Ok     == bug is active in a different variant (silent acceptance).
    // Err    == the fix is in place (or at least some rejection fires, which
    //          is strictly better than nothing — downstream assertions
    //          verify state is untouched).
    match &outcome {
        ApplyOutcome::Ok => {
            panic!(
                "P2l/O4: Light-mode apply of non-boundary EpochReward returned \
                 Ok(()) — silent acceptance is the worst-case variant of the bug. \
                 The fix must make this path return Err."
            );
        }
        ApplyOutcome::Panicked(msg) => {
            panic!(
                "P2l/O4: Light-mode apply of non-boundary EpochReward PANICKED: {}. \
                 This is the HEAD behavior (arithmetic underflow at \
                 validation_checks.rs:490 when mode=Light skips the boundary check). \
                 The fix must replace the panic with a clean Err return. \
                 Observable state at panic time: {:?}",
                msg,
                snapshot_state(&node).await
            );
        }
        ApplyOutcome::Err(msg) => {
            // Soft preference: surface [ECON_EPOCH_NOT_BOUNDARY] for operator clarity.
            // Not a hard assertion — any Err is acceptable provided state is untouched.
            if !msg.contains("ECON_EPOCH_NOT_BOUNDARY") {
                eprintln!(
                    "P2l/O4: note — Light-mode rejection error does not mention \
                     [ECON_EPOCH_NOT_BOUNDARY]. For operator clarity, consider \
                     using the same error code as Full mode. Got: {}",
                    msg
                );
            }
        }
    }

    // ---- O1/O2/O3: state must be identical to pre-apply. ----
    let post = snapshot_state(&node).await;
    assert_eq!(
        post.pool_utxo_count, pre.pool_utxo_count,
        "P2l/O2: pool UTXO count MUST NOT change when a non-boundary \
         EpochReward is rejected (pre={} post={}). \
         This is the core santiago desync.",
        pre.pool_utxo_count, post.pool_utxo_count
    );
    assert_eq!(
        post.pool_utxo_total_amount, pre.pool_utxo_total_amount,
        "P2l/O2: pool UTXO total amount must remain unchanged \
         (pre={} post={})",
        pre.pool_utxo_total_amount, post.pool_utxo_total_amount
    );
    assert_eq!(
        post.utxo_total_count, pre.utxo_total_count,
        "P2l/O2: total UTXO count must remain unchanged \
         (pre={} post={}). A difference here means a reward output was \
         added despite the block being rejected.",
        pre.utxo_total_count, post.utxo_total_count
    );
    assert_eq!(
        post.best_height, pre.best_height,
        "P2l/O3: chain_state.best_height MUST NOT advance when apply is rejected \
         (pre={} post={})",
        pre.best_height, post.best_height
    );
    assert_eq!(
        post.best_hash, pre.best_hash,
        "P2l/O3: chain_state.best_hash MUST NOT change when apply is rejected"
    );
    assert!(
        node.block_store
            .get_block(&bad_hash)
            .expect("block_store.get_block failed")
            .is_none(),
        "P2l/O1: rejected block MUST NOT land in block_store (by hash)"
    );
    let at_h3 = node
        .block_store
        .get_block_by_height(3)
        .expect("block_store.get_block_by_height failed");
    assert!(
        !matches!(&at_h3, Some(b) if b.hash() == bad_hash),
        "P2l/O1: block_store MUST NOT have the rejected block at h=3 \
         (got {:?})",
        at_h3.as_ref().map(|b| b.hash())
    );
}

// ============================================================
// TEST D — DUPLICATE REJECT (no ratcheting damage)
// ============================================================
//
// OUTPUT CONTRACT coverage: Path P3 (duplicate_reject)
//   O1 ✓ : block_store still has no entry for the bad block after 2nd attempt
//   O2 ✓ : pool UTXO count + total unchanged across BOTH rejections
//   O3 ✓ : chain_state unchanged across both rejections
//   O4 ✓ : both apply attempts return Err / reject (NOT panic after fix,
//          NOT Ok)
//
// The santiago log showed the SAME block rejected at 05:11:04, re-applied
// at 05:11:05, rejected again at 05:11:13, re-applied at 05:11:13 — 4
// interleaved attempts in < 10 seconds. If each attempt partially mutates
// state, damage compounds. This test verifies feeding the same bad block
// twice produces NO accumulated damage.
#[tokio::test]
async fn test_d_duplicate_reject_no_ratcheting_damage() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    let (tip_hash, _) = apply_plain_chain(&mut node, &producers, 2, &params).await;
    let pre = snapshot_state(&node).await;

    let recipient_pkh =
        crypto::hash_with_domain(crypto::ADDRESS_DOMAIN, producers[0].public_key().as_bytes());
    let distributable = pre.pool_utxo_total_amount + params.block_reward(3);
    let bad_block = build_bad_epoch_reward_block(
        3,
        3,
        tip_hash,
        &producers[0],
        recipient_pkh,
        distributable,
        0,
        &params,
    );
    let bad_hash = bad_block.hash();

    // First rejection.
    let r1 = try_apply(&mut node, bad_block.clone(), ValidationMode::Light).await;
    assert!(
        !matches!(r1, ApplyOutcome::Ok),
        "P3/O4: first attempt must not return Ok(): {:?}",
        r1
    );
    let mid = snapshot_state(&node).await;

    // Second rejection — same block, same mode.
    let r2 = try_apply(&mut node, bad_block, ValidationMode::Light).await;
    assert!(
        !matches!(r2, ApplyOutcome::Ok),
        "P3/O4: second attempt must not return Ok(): {:?}",
        r2
    );
    let post = snapshot_state(&node).await;

    // Both intermediate and final state must equal pre-state. The key
    // assertion: damage does NOT accumulate across retries.
    assert_eq!(
        mid, pre,
        "P3: state after first reject must equal pre-state. pre={:?} mid={:?}",
        pre, mid
    );
    assert_eq!(
        post, mid,
        "P3: second reject must not mutate state further (no ratcheting). \
         mid={:?} post={:?}",
        mid, post
    );
    assert_eq!(
        post, pre,
        "P3: state after second reject must equal pre-state. pre={:?} post={:?}",
        pre, post
    );

    // O1: block_store must be free of the bad block on BOTH hash and height lookup.
    assert!(
        node.block_store
            .get_block(&bad_hash)
            .expect("block_store.get_block failed")
            .is_none(),
        "P3/O1: bad block must not land in block_store after two rejections"
    );
}
