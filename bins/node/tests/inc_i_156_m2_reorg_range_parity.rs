//! INC-I-156 / M2 QA-iteration-1 — F1 / OBS-001: the reorg caller's dense-guard range must be
//! IDENTICAL to the range the hoisted helper checks.
//!
//! covers: bins/node/src/node/block_handling.rs (execute_reorg guard range parity)
//! covers: bins/node/src/node/rewards.rs (rebuild_producer_set_from_blocks `.max(1)` guard)
//!
//! ## Line numbers in this file
//!
//! Every `block_handling.rs` / `rewards.rs` line number quoted below is the PRE-FIX number,
//! as measured by QA on this branch — same convention as the sibling
//! `inc_i_156_m2_rebuild_guard.rs`, so the trace can be read against the code that had the
//! defect. Applying the fix shifted them; the post-fix anchors are:
//!
//! | pre-fix | post-fix | what |
//! |---|---|---|
//! | `block_handling.rs:607`     | `:620-621`  | the reorg caller's dense guard (the fix) |
//! | `block_handling.rs:814-823` | `:829-838`  | `set_rebuild_in_progress` + `rebuild_marker_armed = true` |
//! | `block_handling.rs:837`     | `:852`      | `utxo.clear()` |
//! | `block_handling.rs:844`     | `:859`      | the UTXO replay loop |
//! | `block_handling.rs:895`     | `:910`      | the `rebuild_producer_set_from_blocks` call |
//! | `block_handling.rs:923`     | `:938`      | the height-index purge |
//! | `block_handling.rs:942`     | `:958`      | `atomic_replace` |
//! | `block_handling.rs:949-953` | `:964-968`  | the conditional disarm |
//! | `rewards.rs:1145-1154`      | `:1147-1156`| the hoisted helper guard |
//! | `rewards.rs:1156`           | `:1158`     | `producers.clear()` |
//!
//! ## The defect, as traced by QA on this branch
//!
//! M2 hoisted `ensure_blocks_present(1, target_height.max(1))` to be the FIRST statement of
//! `Node::rebuild_producer_set_from_blocks` (`rewards.rs:1145-1154`). The reorg call sites are
//! fronted by `execute_reorg`'s own FORK_GUARD check, which uses the NARROWER range
//! `ensure_blocks_present(1, target_height)` (`block_handling.rs:607`).
//!
//! `ensure_blocks_present` returns `Ok(())` whenever `low > high`
//! (`crates/storage/src/block_store/queries.rs:193-196`). At `target_height == 0` the two
//! ranges therefore DIVERGE: the caller checks `1..=0` (a no-op that admits everything) while
//! the helper checks `1..=1` (which refuses when block 1 is absent from the height index).
//!
//! The reorg site's own comment declared that case "unaffected" — M2 silently falsified it.
//! The consequence, on the legacy (no-undo) branch:
//!
//!   1. `block_handling.rs:607` admits the reorg (`1..=0` is a no-op).
//!   2. `has_undo == false` -> legacy branch.
//!   3. `:814-823` arms the DURABLE `rebuild_in_progress` marker.
//!   4. `:837` `utxo.clear()` — a REAL durable clear on the RocksDb backend since M1.
//!   5. `:844` the UTXO replay loop `for height in 1..=0` does nothing.
//!   6. `:895` the helper now checks `1..=1`, block 1 is absent -> `Err`.
//!   7. `?` propagates out of `execute_reorg`; the disarm at `:949-953` is SKIPPED.
//!   8. The durable marker stays ARMED -> `init.rs` reports `[STATE_CORRUPT]` on restart and
//!      `rebuild_halt_reason()` refuses production and snapshot service until an operator
//!      resyncs. A permanent, self-inflicted halt — the same defect class as AUDIT-P3-102.
//!
//! The fix aligns the caller's range with the helper's (`target_height.max(1)`), which moves
//! the refusal AHEAD of the marker arm at `:815` and AHEAD of `utxo.clear()` at `:837`, and
//! restores the M2 premise that all three previously-guarded sites are provably unchanged.
//! Adding a disarm-on-error mechanism instead would leave the destructive path reachable and
//! merely paper over it; aligning the ranges removes the path.
//!
//! ## OUTPUT CONTRACT — `Node::execute_reorg(&mut self, reorg_result: ReorgResult, triggering_block: Block) -> Result<()>`
//!
//! OUTPUTS (every way the call can change observable state), restricted to the branch reached
//! by `target_height == 0` + `has_undo == false` + `new_blocks` empty:
//!   O1  `state_db` CF_META `rebuild_in_progress` — armed at `:815`, disarmed at `:951`.
//!       THE output this file exists for. Read back through
//!       `state_db.get_rebuild_in_progress()`, not through the writer's own view.
//!   O2  the UTXO set, in memory AND durably — `utxo.clear()` at `:837` is a real
//!       `StateDb::clear_utxos` on the RocksDb variant since M1, so a refusal that happens
//!       after it has already emptied `cf_utxo`. Asserted through BOTH the façade
//!       (`h::utxo_content`) and the store (`h::persisted_utxo_content`).
//!   O3  `chain_state` — `best_height` / `best_hash` / `best_slot` are assigned at `:828-830`
//!       INSIDE the same guarded block, so a refusal before `:815` leaves them untouched.
//!   O4  the `ProducerSet` — the helper's own REQ-I156-006 contract; asserted by count so a
//!       refusal cannot be confused with a completed rebuild-to-genesis (which empties it).
//!   O5  return value `Result<()>` — `Err` on BOTH sides of the fix (pre-fix from the helper
//!       at `:895`, post-fix from the caller at `:607`), so the variant alone is NOT the
//!       discriminator. It is asserted FIRST precisely so the RED failure below cannot be
//!       mistaken for "the reorg stopped erroring".
//!   O6  the canonical height index — purged for `target+1..=current` at `:923`, reached only
//!       on the non-refusing path. Asserted intact on the refusal.
//!
//! INPUT PARTITIONS (of `target_height` x block-1 presence, the two values the diverging
//! ranges are functions of):
//!   J1  `target_height == 0`, block 1 ABSENT  -> caller must REFUSE before any mutation. [RED]
//!   J2  `target_height == 0`, block 1 PRESENT -> caller must ADMIT; the full rollback to
//!       genesis must still COMPLETE and disarm its marker.                        [PASS-LOCK]
//!   J3  `target_height  > 0`, dense store     -> `.max(1)` is inert; unchanged.    [PASS-LOCK]
//!
//! MATRIX (outputs x partitions; `-` = not applicable):
//!   J1: O1 ✓  O2 ✓  O3 ✓  O4 ✓  O5 ✓  O6 ✓
//!   J2: O1 ✓  O2 -   O3 ✓  O4 ✓  O5 ✓  O6 -
//!   J3: O1 ✓  O2 -   O3 ✓  O4 -   O5 ✓  O6 -

mod inc_i_156_m1_harness;
use inc_i_156_m1_harness as h;

use crypto::Hash;
use doli_core::Block;
use doli_node::node::Node;
use network::sync::ReorgResult;
use tempfile::TempDir;

// ==================== Scenario geometry ====================

/// Pre-reorg tip (`current_height`, `block_handling.rs:566`).
const CHAIN_LEN: u64 = 4;

/// A FULL-depth reorg: `target_height = current_height - rollback_count`
/// (`block_handling.rs:567`) — rolling back every block puts `target_height` at 0, the single
/// input where the caller's `1..=0` and the helper's `1..=1` disagree.
const TARGET_HEIGHT: u64 = 0;

// ==================== Fixture ====================

/// The `execute_reorg` pre-reorg shape for a full rollback to genesis.
///
/// The PRODUCTION `RocksDb` UTXO variant is load-bearing here for the same reason as in the
/// M1 files (see `inc_i_156_m1_harness`): on the `InMemory` variant `utxo.clear()` would be
/// honest either way, but only the RocksDb variant makes step 4 of the trace above a DURABLE
/// mutation that outlives the refusal.
async fn build_fixture() -> (Node, Vec<Hash>, Block, TempDir) {
    let (mut node, producers, temp) = h::make_node(3).await;
    let params = node.params.clone();
    h::install_production_utxo_backend(&node).await;
    h::apply_plain_up_to(&mut node, &producers, CHAIN_LEN, &params).await;

    assert_eq!(
        node.chain_state.read().await.best_height,
        CHAIN_LEN,
        "fixture: the chain must reach CHAIN_LEN before the reorg"
    );

    let rollback_hashes: Vec<Hash> = (1..=CHAIN_LEN)
        .map(|height| {
            node.block_store
                .get_hash_by_height(height)
                .expect("block_store read failed")
                .unwrap_or_else(|| panic!("fixture: canonical entry missing at h={height}"))
        })
        .collect();
    assert_eq!(
        rollback_hashes.len() as u64,
        CHAIN_LEN,
        "fixture: rollback depth must equal the chain length so target_height lands on 0"
    );

    let tip_block = node
        .block_store
        .get_block_by_height(CHAIN_LEN)
        .expect("block_store read failed")
        .expect("fixture: the tip block must be readable");

    // Force the LEGACY branch: `has_undo` at `block_handling.rs:665-666` must be FALSE over
    // the whole rollback range so control reaches the marker arm at `:814-823`. A snap-synced
    // node arrives in exactly this shape. `prune_undo_above(0)` is the production truncation
    // API (`state_db/undo.rs`).
    node.state_db.prune_undo_above(0);
    for height in 1..=CHAIN_LEN {
        assert!(
            node.state_db.get_undo(height).is_none(),
            "fixture: undo at h={height} must be absent so the legacy branch is taken"
        );
    }

    (node, rollback_hashes, tip_block, temp)
}

/// Drop the canonical entry for `height`, making it invisible to BOTH `ensure_blocks_present`
/// and `get_block_by_height` — the observable shape of a node that never fetched it (pruned
/// prefix or snap sync). Technique recorded at
/// `inc_i_152_p1_003_rollback_holed_store.rs:569`.
fn punch_hole(node: &Node, height: u64) {
    let hash = node
        .block_store
        .get_hash_by_height(height)
        .expect("block_store get_hash_by_height failed")
        .unwrap_or_else(|| panic!("setup: expected a canonical entry at h={height}"));
    node.block_store
        .remove_canonical_entry(height, hash)
        .expect("remove_canonical_entry failed");
    assert!(
        node.block_store
            .get_block_by_height(height)
            .expect("block_store read failed")
            .is_none(),
        "setup: h={height} must be invisible to get_block_by_height after the hole"
    );
}

fn reorg_result(rollback_hashes: &[Hash], common_ancestor: Hash) -> ReorgResult {
    ReorgResult {
        rollback: rollback_hashes.to_vec(),
        common_ancestor,
        // Deliberately EMPTY: with no fork blocks to apply, `execute_reorg` performs the
        // rollback half only, which is the half this file is about. It also skips the
        // `common_ancestor` linkage check at `block_handling.rs:542-553`.
        new_blocks: Vec::new(),
        weight_delta: 1, // execute_reorg only logs it
    }
}

// ==========================================================================
//  J1 — THE RED TEST. Must FAIL pre-fix on O1 (marker), not on O5 (Err).
// ==========================================================================

/// Requirement: REQ-I156-005 / REQ-I156-006 extended to the reorg call site (F1 / OBS-001).
///
/// Acceptance: `execute_reorg` with `target_height == 0` and block 1 absent must be REFUSED by
/// the caller's own guard, leaving the durable `rebuild_in_progress` marker DISARMED and every
/// piece of state byte-identical.
///
/// PRE-FIX this test FAILS on the MARKER assertion (O1): `Err` is returned either way — the
/// helper's `?` at `block_handling.rs:895` — but pre-fix the marker at `:815` has already been
/// armed and `utxo.clear()` at `:837` has already emptied `cf_utxo`, and the disarm at `:949`
/// is skipped. The `Err` assertion is deliberately FIRST so the failure cannot be mistaken for
/// "the reorg stopped erroring".
#[tokio::test]
async fn inc_i156_f1_reorg_target_zero_missing_block_one_must_not_set_rebuild_marker() {
    let (mut node, rollback_hashes, tip_block, _tmp) = build_fixture().await;
    let genesis_hash = node.chain_state.read().await.genesis_hash;

    punch_hole(&node, 1);

    // ---- Preconditions: the two ranges really do disagree on THIS store. ----
    //
    // `target_height` is recomputed here exactly as `block_handling.rs:567` does
    // (`current_height - rollback_count`) rather than read from the const, so the two guard
    // ranges below are evaluated on a runtime value — the same way the code under test sees
    // them, and not something the compiler can fold away.
    let target_height = CHAIN_LEN - rollback_hashes.len() as u64;
    assert_eq!(
        target_height, TARGET_HEIGHT,
        "precondition: a full-depth rollback must put target_height at 0"
    );
    node.block_store
        .ensure_blocks_present(1, target_height)
        .expect(
            "precondition: `ensure_blocks_present(1, 0)` must be a NO-OP (queries.rs:193-196 \
             returns Ok when low > high). That is exactly why the pre-fix caller's guard at \
             block_handling.rs:607 admits this store — if it refused here, the divergence this \
             test is about would not exist.",
        );
    let helper_range_err = node
        .block_store
        .ensure_blocks_present(1, target_height.max(1))
        .expect_err(
            "precondition: `ensure_blocks_present(1, 1)` — the range the hoisted helper checks \
             (rewards.rs:1146) and the range the fix gives the caller — must REFUSE this store. \
             If it does not, block 1 is still visible and the scenario is not constructed.",
        );
    assert!(
        helper_range_err.to_string().contains("height 1"),
        "precondition: the guard must name the FIRST missing height (1); got {helper_range_err}"
    );
    assert!(
        node.state_db.get_rebuild_in_progress().is_none(),
        "precondition: the durable rebuild marker must start DISARMED, otherwise the \
         post-condition below would be a tautology"
    );

    let utxo_before = h::utxo_content(&node).await;
    let persisted_before = h::persisted_utxo_content(&node);
    assert!(
        utxo_before.len > 0,
        "precondition: the UTXO set must be NON-EMPTY so that `utxo.clear()` at \
         block_handling.rs:837 is an OBSERVABLE mutation. Got {} entries",
        utxo_before.len
    );
    let (height_before, hash_before) = {
        let cs = node.chain_state.read().await;
        (cs.best_height, cs.best_hash)
    };
    let producers_before = node.producer_set.read().await.total_count();
    assert!(
        producers_before > 0,
        "precondition: the ProducerSet must be NON-EMPTY, otherwise O4 below is vacuous"
    );

    // ---- The call. ----
    let result = node
        .execute_reorg(reorg_result(&rollback_hashes, genesis_hash), tip_block)
        .await;

    // ---- O5 — asserted FIRST. Err on BOTH sides of the fix; not the discriminator. ----
    let err = result.expect_err(
        "O5: a reorg to target_height == 0 over a store missing block 1 must be REFUSED. \
         Pre-fix the refusal comes from the helper at block_handling.rs:895, post-fix from the \
         caller at block_handling.rs:607 — either way the call must not report success.",
    );
    assert!(
        err.to_string().contains("[FORK_GUARD_BACKFILL"),
        "O5: the refusal must carry the [FORK_GUARD_BACKFILL...] marker the fleet runbook keys \
         on. Got: {err}"
    );

    // ---- O1 — THE RED ASSERTION. ----
    assert!(
        node.state_db.get_rebuild_in_progress().is_none(),
        "F1 / OBS-001 / O1: a REFUSED reorg must leave CF_META[rebuild_in_progress] DISARMED. \
         The caller's guard at block_handling.rs:607 checks `1..=target_height`, which is a \
         no-op at target_height == 0 (queries.rs:193-196), while the helper hoisted by M2 \
         checks `1..=target_height.max(1)` (rewards.rs:1146). The reorg is therefore admitted \
         at :607, arms the DURABLE marker at :815, clears the UTXO set at :837, and only THEN \
         refuses at :895 — whose `?` skips the disarm at :949. That leaves a permanent halt \
         marker armed on a node an operator must resync. Fix: give the caller the IDENTICAL \
         range, `ensure_blocks_present(1, target_height.max(1))`, so the refusal happens BEFORE \
         :815 and BEFORE :837. Marker found armed at target_height={:?}",
        node.state_db.get_rebuild_in_progress()
    );

    // ---- O2 — the refusal must precede `utxo.clear()`, in memory AND on disk. ----
    let utxo_after = h::utxo_content(&node).await;
    assert_eq!(
        utxo_after.canonical, utxo_before.canonical,
        "F1 / O2: a REFUSED reorg must leave the in-memory UTXO set byte-identical. \
         `utxo.clear()` at block_handling.rs:837 runs BEFORE the helper's guard at :895, so a \
         refusal reached at :895 has already destroyed it (before={} entries, after={})",
        utxo_before.len, utxo_after.len
    );
    assert_eq!(
        h::persisted_utxo_content(&node).canonical,
        persisted_before.canonical,
        "F1 / O2: the PERSISTED UTXO set must also be byte-identical. Since M1, \
         `UtxoSet::clear()` on the RocksDb variant is a real `StateDb::clear_utxos`, so the \
         refusal at :895 leaves `cf_utxo` durably empty even though `atomic_replace` (:942) is \
         never reached. Read back from the store, not from the façade (Rule AQ-5)."
    );

    // ---- O3 — chain_state is assigned at :828-830, inside the same guarded block. ----
    let (height_after, hash_after) = {
        let cs = node.chain_state.read().await;
        (cs.best_height, cs.best_hash)
    };
    assert_eq!(
        (height_after, hash_after),
        (height_before, hash_before),
        "F1 / O3: a REFUSED reorg must not rewind chain_state. best_height/best_hash are \
         assigned at block_handling.rs:828-830, before the refusal point at :895."
    );

    // ---- O4 — the helper's own REQ-I156-006 contract, at this call site. ----
    assert_eq!(
        node.producer_set.read().await.total_count(),
        producers_before,
        "F1 / O4: a REFUSED reorg must leave the ProducerSet intact. `producers.clear()` sits \
         behind the hoisted guard (rewards.rs:1156), so this holds post-M2 at the helper — but \
         it is asserted here so the caller-side fix cannot regress it."
    );

    // ---- O6 — the height-index purge at :923 must not have run. ----
    for height in 2..=CHAIN_LEN {
        assert!(
            node.block_store
                .get_hash_by_height(height)
                .expect("block_store read failed")
                .is_some(),
            "F1 / O6: a REFUSED reorg must not purge the canonical height index. The purge at \
             block_handling.rs:923 covers target+1..=current and is only reachable past the \
             refusal point; h={height} went missing."
        );
    }
}

// ==========================================================================
//  J2 — PASS-LOCK: `target_height == 0` with block 1 PRESENT is not over-refused.
// ==========================================================================

/// Requirement: REQ-I156-007 (Must) — the fix must be a range ALIGNMENT, not a new refusal.
///
/// This is the twin of J1 and the reason `.max(1)` is exactly right rather than merely strict:
/// when block 1 IS present, a legitimate full rollback to genesis must still COMPLETE. A fix
/// that refused here would convert every rollback-to-genesis into a wedge.
///
/// GREEN on both sides of the fix.
#[tokio::test]
async fn inc_i156_f1_reorg_target_zero_with_block_one_present_still_completes() {
    let (mut node, rollback_hashes, tip_block, _tmp) = build_fixture().await;
    let genesis_hash = node.chain_state.read().await.genesis_hash;

    assert!(
        node.block_store
            .get_block_by_height(1)
            .expect("block_store read failed")
            .is_some(),
        "precondition: block 1 must be PRESENT for this partition — it is the only thing that \
         separates J2 from J1"
    );

    node.execute_reorg(reorg_result(&rollback_hashes, genesis_hash), tip_block)
        .await
        .expect(
            "REQ-I156-007 / O5: a full rollback to genesis over a store that HAS block 1 must \
             be ADMITTED. `.max(1)` widens the caller's range to `1..=1`, which this store \
             satisfies; refusing here would wedge every legitimate rollback-to-genesis.",
        );

    // O3 — the reorg really landed, it was not refused-and-skipped.
    let (height_after, hash_after) = {
        let cs = node.chain_state.read().await;
        (cs.best_height, cs.best_hash)
    };
    assert_eq!(
        (height_after, hash_after),
        (TARGET_HEIGHT, genesis_hash),
        "REQ-I156-007 / O3: chain_state must land on genesis (height 0, genesis_hash). \
         `execute_reorg` returns Ok(()) on several refusal paths too, so the landed tip is the \
         only discriminator between 'completed' and 'refused'."
    );

    // O1 — a COMPLETED reorg disarms its own marker.
    h::assert_utxo_invariants(&node, "J2 target_height==0 / block 1 present").await;
}

// ==========================================================================
//  J3 — PASS-LOCK: `.max(1)` is inert for every `target_height > 0`.
// ==========================================================================

/// Requirement: REQ-I156-007 (Must) — the fix must change behaviour at EXACTLY one input.
///
/// `target_height.max(1) == target_height` for every `target_height >= 1`, so a normal
/// shallow reorg over a dense store must be bit-identical before and after. This is the lock
/// that keeps the fix from being a general tightening of the FORK_GUARD check.
///
/// GREEN on both sides of the fix.
#[tokio::test]
async fn inc_i156_f1_shallow_reorg_on_dense_store_is_unaffected_by_max1() {
    let (mut node, rollback_hashes, tip_block, _tmp) = build_fixture().await;

    const DEPTH: usize = 2;
    let target = CHAIN_LEN - DEPTH as u64;
    let ancestor_hash = node
        .block_store
        .get_hash_by_height(target)
        .expect("block_store read failed")
        .expect("fixture: canonical entry must exist at the common ancestor");

    node.block_store
        .ensure_blocks_present(1, target)
        .expect("precondition: the store must be DENSE over the rebuild range");
    assert_eq!(
        target.max(1),
        target,
        "precondition: `.max(1)` must be INERT here — that is the property this partition locks"
    );

    let rollback: Vec<Hash> = rollback_hashes[rollback_hashes.len() - DEPTH..].to_vec();
    node.execute_reorg(reorg_result(&rollback, ancestor_hash), tip_block)
        .await
        .expect("REQ-I156-007 / O5: a dense shallow reorg must still succeed after the fix");

    let (height_after, hash_after) = {
        let cs = node.chain_state.read().await;
        (cs.best_height, cs.best_hash)
    };
    assert_eq!(
        (height_after, hash_after),
        (target, ancestor_hash),
        "REQ-I156-007 / O3: the shallow reorg must land on the common ancestor"
    );
    h::assert_utxo_invariants(&node, "J3 shallow dense reorg").await;
}
