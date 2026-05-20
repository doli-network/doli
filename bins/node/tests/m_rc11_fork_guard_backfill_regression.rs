//! M-RC11 — FORK_GUARD backfill / chain_state-completeness invariant
//! regression tests (INC-I-034, REQ-REDESIGN-011).
//!
//! Reproduces the 2026-04-16 05:11 UTC mainnet cascade documented in
//! `docs/.workflow/blockchain-investigation-consensus.md`:
//!
//!   05:11:14.470Z  [BLOCK] Applied h=39599 hash=602990400e... (canonical winner)
//!   05:11:14.874Z  [FORK_GUARD] Dropping fork block ed9bab0b at h=39599
//!                                — canonical 602990400e exists
//!   05:11:31.916Z  Empty headers from peers (new canonical chain ahead at
//!                  h=39603, local stuck at 39599 — gap=4, peers blacklisted
//!                  as fork-evidence)
//!   ... santiago accepts forward gossip but never re-fetches 39600-39628 ...
//!   Result: santiago tip catches up but block_store has permanent gap
//!           39600-39628.
//!
//! ## The defect (audit of HEAD @ synmgrefactor)
//!
//! The visible FORK_GUARD log lives at `bins/node/src/node/block_handling.rs:90`,
//! inside `Node::handle_new_block`. That branch DROPS the fork block — it does
//! NOT switch chains. The actual switch happens later (when the fork chain is
//! heavier) via `Node::execute_reorg` at the same file, lines 311-602.
//!
//! `execute_reorg`'s rollback path contains the latent invariant violation
//! that REQ-REDESIGN-011 targets. At lines 399-413:
//!
//! ```text
//!   let common_ancestor_block = if target_height == 0 {
//!       None
//!   } else {
//!       self.block_store.get_block_by_height(target_height)?   // (A)
//!   };
//!
//!   let common_ancestor_hash = common_ancestor_block
//!       .as_ref()
//!       .map(|b| b.hash())
//!       .unwrap_or(genesis_hash);                                // (B)
//! ```
//!
//! At (A) `get_block_by_height` returns `Ok(None)` if the block at
//! `target_height` is not present in `block_store` (e.g. because of a prior
//! partial reorg, snap-sync gap, archiver pruning, or RocksDB index
//! divergence). At (B) `common_ancestor_hash` then SILENTLY substitutes
//! `genesis_hash` for the missing block.
//!
//! The rollback then executes (lines 462-466 / 487-488):
//!
//! ```text
//!   state.best_height = target_height;        // e.g. 5
//!   state.best_hash = common_ancestor_hash;   // = genesis_hash (BUG)
//!   state.best_slot = common_ancestor_slot;   // = 0
//! ```
//!
//! `chain_state` is now corrupt: `best_hash != block_store[best_height].hash()`.
//! All downstream paths that read `best_hash` (gossip eligibility, validation,
//! sync status broadcasts, set_canonical_chain walks, fork-recovery anchors)
//! see a hash that has no corresponding block in the local store. From the
//! sync manager's view the local chain looks like it's on an unknowable fork
//! — every peer's `GetHeaders(best_hash)` returns empty, and every empty
//! response is interpreted as "peer is forked" → blacklist cascade. That is
//! exactly the santiago/ivan/seed3 pattern (see investigation doc, "Empty
//! headers from peers" at 05:11:31.916Z).
//!
//! Sibling violation in the legacy fallback at lines 467-517: same silent
//! substitution at line 487, plus a `utxo.clear()` + rebuild loop that walks
//! `block_store.get_block_by_height(height)` from 1..=target_height — if any
//! of those heights is missing, the rebuild silently skips with `.ok().flatten()`
//! and the UTXO set ends up structurally corrupt to match the corrupt
//! chain_state. Tested implicitly by the same scenario.
//!
//! ## REQ-REDESIGN-011 invariant (what we are testing)
//!
//! > After ANY mutation of `chain_state.best_hash`, the system MUST guarantee
//! > that `block_store.get_block_by_height(chain_state.best_height)` returns
//! > a block whose hash equals `chain_state.best_hash`.
//! > AND every height in `1..=chain_state.best_height` must be retrievable
//! > from `block_store` (no mid-chain gap).
//! > If backfill cannot complete, the switch MUST NOT occur — `chain_state`
//! > stays on the OLD canonical until backfill succeeds.
//!
//! ## What this file tests
//!
//! OUTPUT CONTRACT: Node::execute_reorg(ReorgResult, Block) -> Result<()>
//!   Function under test: `bins/node/src/node/block_handling.rs:311`
//!   Sibling under invariant: `Node::handle_new_block` at the FORK_GUARD
//!     gate (line 88-104), `apply_block` (mod.rs:230 put_block + 231
//!     set_canonical_chain).
//!
//!   Observable outputs (every chain_state-mutating call must preserve them):
//!     O1: `chain_state.best_hash` matches OR equals OLD canonical at OLD best_height
//!         (atomic — never an intermediate corrupt value)
//!     O2: `block_store.get_block_by_height(chain_state.best_height)` returns
//!         a block whose hash equals `chain_state.best_hash`
//!     O3: `verifyChainIntegrity` style scan over 1..=best_height returns
//!         complete=true (no mid-chain gap)
//!     O4: switch is atomic — pre/post snapshots of (best_hash, best_height)
//!         show either OLD or NEW state, never a mismatch where best_height
//!         and best_hash come from different chains
//!
//!   Paths covered:
//!     P1: tip_reorg_parent_present — 1-block reorg at tip, parent already in
//!         block_store. The simple regression anchor; must pass today and after
//!         the fix. (test_a_*)
//!     P2: deeper_reorg_parent_missing — reorg whose `target_height` block is
//!         absent from block_store (engineered via `delete_blocks_above`).
//!         FAILS on HEAD; PASSES after the fix. (test_b_*)
//!     P3: equal_weight_no_switch — 1-block fork at tip, same weight,
//!         deterministic tie-break should not violate completeness either way.
//!         Tested implicitly via Test A's invariant assertion.
//!     P4: peer_lacks_chain — execute_reorg's `new_blocks` references a hash
//!         that exists in neither fork_block_cache nor block_store; the
//!         function should return Ok(()) WITHOUT mutating chain_state.
//!         (test_c_*)
//!
//!   Matrix: 4 outputs × 3 paths = 12 cells (P3 is implicit in P1).
//!     P1 : O1 ✓ | O2 ✓ | O3 ✓ | O4 ✓   (test_a_simple_tip_reorg_preserves_invariant)
//!     P2 : O1 ✓ | O2 ✓ | O3 ✓ | O4 ✓   (test_b_deeper_reorg_with_missing_ancestor_preserves_invariant)
//!     P4 : O1 ✓ | O2 ✓ | O3 ✓ | O4 ✓   (test_c_reorg_with_missing_new_block_does_not_advance_chain_state)
//!
//! Constraints: read-only on source; uses `Node::new_for_test` + real RocksDB.

use std::sync::Once;

use crypto::{Hash, KeyPair};
use doli_core::consensus::{self, ConsensusParams};
use doli_core::transaction::Transaction;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Network};
use doli_node::node::Node;
use network::sync::ReorgResult;
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
// Test scaffolding
// ============================================================

async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    init_env();
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    // Keep block_reward positive throughout the test range (devnet era decay
    // would zero it around h=576 otherwise).
    node.params.blocks_per_era = 100_000;
    (node, producers, temp)
}

/// Build a coinbase-only block. The pair `(block_slot, coinbase_slot)` lets
/// us produce DIFFERENT blocks at the same `(height, prev_hash)` so we can
/// construct sibling fork chains without colliding on hashes. The coinbase's
/// `slot` parameter goes into its `extra_data` (see `transaction/core.rs:60`)
/// — varying it gives the coinbase a unique tx hash, hence a unique merkle
/// root, hence a unique block hash.
fn build_block(
    height: u64,
    block_slot: u32,
    coinbase_slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
) -> Block {
    let reward = params.block_reward(height);
    let pool_hash = consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, coinbase_slot);
    let timestamp = params.genesis_time + (block_slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase));
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash,
        timestamp,
        slot: block_slot,
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

/// Apply a chain of `count` plain blocks starting from the current tip.
/// Returns the final (hash, height).
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
        // Canonical chain: block_slot == coinbase_slot == h. Sibling blocks
        // (built elsewhere) use a different coinbase_slot to differ in hash.
        let block = build_block(
            h,
            h as u32, // block_slot
            h as u32, // coinbase_slot — same as block_slot for canonical
            prev,
            &producers[(h as usize) % producers.len()],
            params,
        );
        prev = block.hash();
        node.apply_block(block, ValidationMode::Light, None)
            .await
            .unwrap_or_else(|e| panic!("plain block apply failed at h={}: {}", h, e));
    }
    (prev, start_h + count)
}

/// Snapshot of consensus-relevant state for before/after comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StateSnapshot {
    best_height: u64,
    best_hash: Hash,
}

async fn snapshot_state(node: &Node) -> StateSnapshot {
    let cs = node.chain_state.read().await;
    StateSnapshot {
        best_height: cs.best_height,
        best_hash: cs.best_hash,
    }
}

/// Mirror of `verifyChainIntegrity` (RPC) at the test level: scan every
/// height from 1..=best_height and return the list of missing heights.
/// Empty Vec ⇒ chain is complete.
async fn missing_heights(node: &Node) -> Vec<u64> {
    let tip = node.chain_state.read().await.best_height;
    let mut missing = Vec::new();
    for h in 1..=tip {
        match node.block_store.get_block_by_height(h) {
            Ok(Some(_)) => {}
            Ok(None) => missing.push(h),
            Err(_) => missing.push(h),
        }
    }
    missing
}

/// Assert the REQ-REDESIGN-011 invariant — completeness contract:
///   chain_state.best_hash matches block_store[best_height].hash()
///   AND every height 1..=best_height has a retrievable block.
///
/// Returns Ok(()) if the invariant holds, Err(message) otherwise. We return
/// rather than panic so callers can format scenario-specific failure messages
/// while still capturing the underlying violation.
async fn check_completeness_invariant(node: &Node) -> Result<(), String> {
    let snap = snapshot_state(node).await;

    // Genesis is the trivially-complete case.
    if snap.best_height == 0 {
        let cs = node.chain_state.read().await;
        if snap.best_hash == cs.genesis_hash {
            return Ok(());
        } else {
            return Err(format!(
                "best_height=0 but best_hash {} != genesis_hash {}",
                snap.best_hash, cs.genesis_hash
            ));
        }
    }

    // O2: best_hash <-> block_store[best_height] consistency.
    let tip_block = match node.block_store.get_block_by_height(snap.best_height) {
        Ok(Some(b)) => b,
        Ok(None) => {
            return Err(format!(
                "O2 VIOLATION: chain_state.best_height={} but \
                 block_store.get_block_by_height({}) = None. \
                 chain_state.best_hash = {}",
                snap.best_height, snap.best_height, snap.best_hash
            ));
        }
        Err(e) => {
            return Err(format!(
                "O2 ERROR: block_store.get_block_by_height({}) failed: {}",
                snap.best_height, e
            ));
        }
    };
    if tip_block.hash() != snap.best_hash {
        return Err(format!(
            "O2 VIOLATION: chain_state.best_hash={} but \
             block_store.get_block_by_height({}).hash()={}",
            snap.best_hash,
            snap.best_height,
            tip_block.hash()
        ));
    }

    // O3: no mid-chain gap.
    let missing = missing_heights(node).await;
    if !missing.is_empty() {
        return Err(format!(
            "O3 VIOLATION: block_store has {} missing heights in 1..={}: {:?}",
            missing.len(),
            snap.best_height,
            // Print at most 20 to keep failure output manageable.
            missing.iter().take(20).collect::<Vec<_>>()
        ));
    }

    Ok(())
}

// ============================================================
// TEST A — REGRESSION ANCHOR (must PASS today AND after fix)
// ============================================================
//
// OUTPUT CONTRACT coverage: Path P1 (tip_reorg_parent_present)
//   O1 ✓ : chain_state.best_hash transitions atomically OLD → NEW
//   O2 ✓ : block_store.get_block_by_height(best_height).hash() == best_hash
//   O3 ✓ : no mid-chain gap after the reorg
//   O4 ✓ : pre/post snapshot consistency — never an intermediate corrupt state
//
// Setup: apply a 5-block chain A. Then drive `execute_reorg` with a 1-block
// reorg at the tip — replacing A5 with B5 (a sibling block built on A4).
//
// On HEAD: the reorg path's rollback to common_ancestor=A4 finds A4 in
// block_store (target_height=4 > 0, get_block_by_height returns Some(A4)),
// so the `unwrap_or(genesis_hash)` branch is NOT taken. The new block B5 is
// applied via apply_block, which does put_block + set_canonical_chain. After
// reorg: best_hash=B5, best_height=5, block_store[5]=B5. Invariant holds.
//
// This is the regression anchor: the common-case reorg must continue to work
// after the fix, and the fix must not introduce extra latency for it.
#[tokio::test]
async fn test_a_simple_tip_reorg_preserves_invariant() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    // Build canonical chain A: heights 1..=5, each with nonce=0.
    let (a5_hash, _) = apply_plain_chain(&mut node, &producers, 5, &params).await;
    assert_eq!(node.chain_state.read().await.best_height, 5);
    assert_eq!(node.chain_state.read().await.best_hash, a5_hash);

    // Pre-reorg snapshot — invariant must already hold.
    let pre = snapshot_state(&node).await;
    check_completeness_invariant(&node)
        .await
        .expect("P1 PRE: invariant must hold before reorg");

    // Build sibling block B5 at height=5, building on A4 (the parent of A5).
    let a4_hash = node
        .block_store
        .get_block_by_height(4)
        .expect("get A4 failed")
        .expect("A4 must exist after applying chain A")
        .hash();
    // Build B5 with a DIFFERENT coinbase_slot than A5 so the block hash
    // differs deterministically (slot extra_data → tx hash → merkle root →
    // block hash). The block_slot stays the same as A5 (5) so the timestamp
    // is identical; only the coinbase content distinguishes them. We drive
    // execute_reorg directly, bypassing the equal-weight tie-break check at
    // reorg/mod.rs:278.
    let b5 = build_block(
        5,
        5_u32,    // block_slot
        4242_u32, // coinbase_slot — distinct from A5's coinbase_slot=5
        a4_hash,
        &producers[(5_usize) % producers.len()],
        &params,
    );
    let b5_hash = b5.hash();
    assert_ne!(
        b5_hash, a5_hash,
        "P1 setup: B5 must differ from A5 (different nonce → different merkle → different hash)"
    );

    // Pre-populate fork_block_cache so execute_reorg can find B5.
    node.fork_block_cache
        .write()
        .await
        .insert(b5_hash, b5.clone());

    // Construct ReorgResult: roll back A5, common_ancestor=A4, new_blocks=[B5].
    let reorg = ReorgResult {
        rollback: vec![a5_hash],
        common_ancestor: a4_hash,
        new_blocks: vec![b5_hash],
        weight_delta: 1, // any positive value; execute_reorg only logs it
    };

    // Execute the reorg.
    node.execute_reorg(reorg, b5.clone())
        .await
        .expect("P1: execute_reorg must succeed");

    // O1: chain_state advanced to B5 (NEW), not stuck at A5.
    let post = snapshot_state(&node).await;
    assert_eq!(
        post.best_height, 5,
        "P1/O1: best_height must remain 5 after a 1-block tip reorg"
    );
    assert_eq!(
        post.best_hash, b5_hash,
        "P1/O1: best_hash must be B5 after the reorg (was {} pre-reorg)",
        pre.best_hash
    );

    // O2 + O3 + O4: completeness invariant must hold post-reorg.
    check_completeness_invariant(&node)
        .await
        .unwrap_or_else(|e| {
            panic!(
                "P1 POST: REQ-REDESIGN-011 invariant violated after a simple \
             tip reorg. This is the REGRESSION ANCHOR — if it fails, the \
             fix has broken the common case. Pre={:?} Post={:?}\n\
             Violation: {}",
                pre, post, e
            )
        });
}

// ============================================================
// TEST B — DEEPER REORG, common_ancestor MISSING (FAILS on HEAD)
// ============================================================
//
// OUTPUT CONTRACT coverage: Path P2 (deeper_reorg_parent_missing)
//   O1 ✓ : chain_state.best_hash MUST stay on OLD canonical (A10) OR advance
//          atomically to a new tip whose block exists in block_store.
//          `best_hash = genesis_hash` while `best_height = 5` is FORBIDDEN.
//   O2 ✓ : block_store.get_block_by_height(best_height) MUST return a block
//          whose hash equals best_hash. (HEAD: returns None when best_height=5
//          because we deleted h=5; OR returns A5 while best_hash=genesis.)
//   O3 ✓ : no mid-chain gap in 1..=best_height. (HEAD: gap at h=5..10 is
//          inevitable after the rollback unless backfill ran first.)
//   O4 ✓ : the switch is atomic — either OLD or NEW, never the corrupt
//          (best_height=5, best_hash=genesis_hash) intermediate.
//
// Scenario: apply 10-block canonical chain A. Use the public
// `delete_blocks_above(4)` API to simulate a prior partial reorg / archiver
// pruning event that erased blocks 5..10 from block_store WITHOUT updating
// chain_state. (chain_state still says best_height=10.) Now drive a reorg
// with target_height=5 (rollback 5 blocks) + new_blocks=[B6 building on the
// MISSING A5].
//
// On HEAD execute_reorg's lines 399-413 fall into the `unwrap_or(genesis_hash)`
// branch because get_block_by_height(5) returns None (we just deleted it).
// The rollback then writes:
//   state.best_height = 5
//   state.best_hash   = genesis_hash      ← THE BUG
//   state.best_slot   = 0
// At this point block_store.get_block_by_height(5) is None, and best_hash
// no longer references a block in our store. The check_completeness_invariant
// helper should detect both the O2 and O3 violations and the test FAILS.
//
// Post-fix: execute_reorg MUST detect the missing common ancestor and EITHER
// (a) return without mutating chain_state, OR (b) backfill first and then
// proceed atomically. Either outcome satisfies the assertions below; the
// test passes.
#[tokio::test]
async fn test_b_deeper_reorg_with_missing_ancestor_preserves_invariant() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    // Build canonical chain A: heights 1..=10.
    let (a10_hash, _) = apply_plain_chain(&mut node, &producers, 10, &params).await;
    assert_eq!(node.chain_state.read().await.best_height, 10);
    assert_eq!(node.chain_state.read().await.best_hash, a10_hash);

    // Sanity: chain integrity holds before we engineer the gap.
    check_completeness_invariant(&node)
        .await
        .expect("P2 PRE: invariant must hold before engineering the gap");

    let pre = snapshot_state(&node).await;

    // Engineer the gap: delete blocks 5..=10 from block_store via the public
    // `delete_blocks_above(4)` API. This mimics: archiver pruning that ran
    // ahead of chain_state, OR a prior partial reorg that left block_store
    // truncated, OR a snap-sync race. The key property is:
    //   chain_state.best_height = 10
    //   block_store.get_block_by_height(5..=10) = None
    let deleted = node
        .block_store
        .delete_blocks_above(4)
        .expect("delete_blocks_above failed");
    assert_eq!(
        deleted, 6,
        "P2 setup: delete_blocks_above(4) should delete heights 5..=10 (6 blocks)"
    );

    // Verify the engineered state — block_store now diverges from chain_state.
    let mid_check_a5 = node
        .block_store
        .get_block_by_height(5)
        .expect("get_block_by_height failed");
    assert!(
        mid_check_a5.is_none(),
        "P2 setup: h=5 must be absent from block_store after delete_blocks_above(4)"
    );
    let chain_state_still_at_10 = node.chain_state.read().await.best_height;
    assert_eq!(
        chain_state_still_at_10, 10,
        "P2 setup: chain_state.best_height must still be 10 (we only mutated block_store)"
    );

    // Build NEW chain block B6 supposedly building on the now-missing A5.
    // Its prev_hash is the OLD A5 hash (which we cached in `pre`); the chain
    // can no longer be walked, but execute_reorg's chain-validation only
    // checks `first.header.prev_hash == reorg_result.common_ancestor`, so we
    // make those match.
    //
    // Recover the OLD A5 hash from the chain we built before deletion: walk
    // the headers/cache. Since block_store no longer has it, we re-derive it
    // by replaying the build function deterministically.
    //
    // Re-derive A5 from genesis by replaying apply_plain_chain's hashing
    // logic. apply_plain_chain uses nonce=0 and slot=h.
    let derived_a5_hash = derive_chain_hash_at(5, &producers, &params).await;
    let b6 = build_block(
        6,
        6_u32,           // block_slot
        9999_u32,        // coinbase_slot — distinct so B6 differs from the deleted A6
        derived_a5_hash, // builds on the NOW-MISSING A5
        &producers[(6_usize) % producers.len()],
        &params,
    );
    let b6_hash = b6.hash();

    // Pre-populate fork_block_cache with B6.
    node.fork_block_cache
        .write()
        .await
        .insert(b6_hash, b6.clone());

    // Construct ReorgResult that will exercise the buggy rollback path:
    //   rollback_count = 5  ⇒  target_height = current_height(10) - 5 = 5
    //   common_ancestor = derived A5 hash
    //   new_blocks = [B6]   (builds on common_ancestor)
    // The 5 rollback hashes are placeholder — execute_reorg only uses the
    // length (line 380), not the contents (line 431 iterates by height).
    let reorg = ReorgResult {
        rollback: vec![Hash::ZERO; 5],
        common_ancestor: derived_a5_hash,
        new_blocks: vec![b6_hash],
        weight_delta: 1,
    };

    // Execute the reorg. We do NOT assert on the return value — execute_reorg
    // returns Ok(()) in many failure modes (it logs and absorbs errors to
    // avoid crashing the node). The assertion is on observable state AFTER.
    let _ = node.execute_reorg(reorg, b6.clone()).await;

    // Post-state — what we actually require:
    let post = snapshot_state(&node).await;

    // O1+O4 (atomicity): EITHER the switch was refused (state unchanged from
    // pre) OR the switch completed and chain_state is consistent with
    // block_store. The forbidden state is "best_height moved but best_hash is
    // genesis_hash" (the silent substitution at execute_reorg lines 406-409).
    let cs_genesis = {
        let cs = node.chain_state.read().await;
        cs.genesis_hash
    };
    let switch_was_refused = post == pre;

    if !switch_was_refused {
        // Switch occurred. Now the completeness invariant MUST hold.
        if let Err(e) = check_completeness_invariant(&node).await {
            panic!(
                "P2/REQ-REDESIGN-011 VIOLATION: execute_reorg mutated \
                 chain_state in the face of a missing common ancestor.\n\
                 Pre  state: best_height={} best_hash={}\n\
                 Post state: best_height={} best_hash={}\n\
                 genesis_hash={}\n\
                 Invariant violation: {}\n\n\
                 Expected behavior (REQ-REDESIGN-011): when block_store does \
                 not contain the rollback target, the switch MUST either \
                 (a) be refused (chain_state stays on OLD tip), OR \
                 (b) backfill first and proceed atomically. The current \
                 silent `unwrap_or(genesis_hash)` at \
                 block_handling.rs:406-409 corrupts chain_state and is the \
                 root cause of the 2026-04-16 santiago/ivan/seed3 cascade.",
                pre.best_height, pre.best_hash, post.best_height, post.best_hash, cs_genesis, e
            );
        }
    } else {
        eprintln!(
            "P2 NOTE: execute_reorg refused the switch (state unchanged). \
             This is one of the two acceptable post-fix outcomes."
        );
    }

    // Defense in depth: even if the switch was refused, no observable state
    // should be in the corrupt mid-state where best_hash=genesis but
    // best_height>0. (Cheap to check; cheap to maintain.)
    assert!(
        !(post.best_height > 0 && post.best_hash == cs_genesis),
        "P2/O1: chain_state.best_hash must NEVER be genesis_hash when \
         best_height > 0. Got best_height={} best_hash=genesis_hash. \
         This is the precise corruption documented at \
         block_handling.rs:406-409 (silent `unwrap_or(genesis_hash)`).",
        post.best_height
    );
}

// ============================================================
// TEST C — REORG WITH MISSING NEW BLOCK (must NOT advance state)
// ============================================================
//
// OUTPUT CONTRACT coverage: Path P4 (peer_lacks_chain)
//   O1 ✓ : chain_state.best_hash unchanged
//   O2 ✓ : invariant holds (it held pre, and nothing mutated)
//   O3 ✓ : no new gaps
//   O4 ✓ : atomic — no half-mutated state
//
// Scenario: drive execute_reorg with a `new_blocks` hash that exists neither
// in fork_block_cache nor in block_store. Lines 340-346 in execute_reorg
// SHOULD return Ok(()) without touching chain_state. This test verifies that
// the early-exit path is wired correctly: it's the closest-existing analogue
// of the REQ-REDESIGN-011 "switch refused" branch, and the fix should not
// regress it.
#[tokio::test]
async fn test_c_reorg_with_missing_new_block_does_not_advance_chain_state() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    let (a5_hash, _) = apply_plain_chain(&mut node, &producers, 5, &params).await;
    assert_eq!(node.chain_state.read().await.best_height, 5);

    let pre = snapshot_state(&node).await;
    check_completeness_invariant(&node)
        .await
        .expect("P4 PRE: invariant must hold");

    // Construct a reorg whose new_blocks references a hash that is NOT in
    // fork_block_cache and NOT in block_store.
    let phantom_hash = crypto::hash::hash(b"this-block-does-not-exist-anywhere");
    let a4_hash = node
        .block_store
        .get_block_by_height(4)
        .expect("get A4 failed")
        .expect("A4 must exist")
        .hash();

    // Triggering block: its hash must NOT match phantom_hash so the cache
    // lookup is the only place new_blocks[0] can be found — and it isn't.
    let triggering = build_block(5, 5_u32, 7777_u32, a4_hash, &producers[2], &params);
    assert_ne!(triggering.hash(), phantom_hash, "P4 setup");

    let reorg = ReorgResult {
        rollback: vec![a5_hash],
        common_ancestor: a4_hash,
        new_blocks: vec![phantom_hash],
        weight_delta: 1,
    };

    // execute_reorg should return Ok(()) and leave chain_state alone.
    let r = node.execute_reorg(reorg, triggering).await;
    assert!(
        r.is_ok(),
        "P4/O4: execute_reorg should return Ok(()) when new_blocks references \
         a missing block (it logs `Cannot execute reorg: missing block ...` \
         at block_handling.rs:341-345). Got: {:?}",
        r
    );

    // O1+O4: chain_state unchanged.
    let post = snapshot_state(&node).await;
    assert_eq!(
        post, pre,
        "P4/O1+O4: chain_state must NOT change when execute_reorg cannot \
         find a `new_blocks` hash. pre={:?} post={:?}",
        pre, post
    );

    // O2+O3: invariant still holds.
    check_completeness_invariant(&node)
        .await
        .unwrap_or_else(|e| {
            panic!(
                "P4 POST: invariant must hold after a no-op execute_reorg. \
             Violation: {}",
                e
            )
        });
}

// ============================================================
// Helpers
// ============================================================

/// Re-derive the canonical-chain block hash at height `target_h`. Mirrors the
/// hashing performed by `apply_plain_chain` (block_slot=coinbase_slot=h,
/// producer index = h % producers.len()) so we can reference a hash whose
/// underlying block has been deleted from block_store.
async fn derive_chain_hash_at(
    target_h: u64,
    producers: &[KeyPair],
    params: &ConsensusParams,
) -> Hash {
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();
    let mut prev = genesis_hash;
    for h in 1..=target_h {
        let block = build_block(
            h,
            h as u32, // block_slot
            h as u32, // coinbase_slot — same as canonical chain
            prev,
            &producers[(h as usize) % producers.len()],
            params,
        );
        prev = block.hash();
    }
    prev
}
