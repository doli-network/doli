//! INC-I-204 M4.1 / REQ-FORK-012 — the rescue. TESTS-FIRST (RED).
//!
//! REQ-FORK-012 — Decision: a failure here means LB-4 has no replacement, so
//! closing the poison bypass in M4.2 would convert every poison event into a
//! fleet-wide wedge whose only exit is history-destroying snap sync.
//! REQ-FORK-011 — Decision: a failure means the escape bought recovery with
//! archival history, recreating INC-I-190's permanent 314592-314640 body hole.
//! REQ-FORK-003 — Decision: a failure in the `FinalityTracker` assertion means the
//! escape mutated the MONOTONE truth store instead of the erasable `ReorgHandler`
//! mirror, so the node's own record of what it finalized is now rewritable by RPC
//! (INV-FINALITY-001).
//!
//! OUTPUT CONTRACT — `async fn Node::try_consume_force_reorg(&mut self)
//!                       -> Option<ForceReorgOutcome>`
//!   O1 mutable params: none (`&mut self` only).
//!   O2 receiver mutation:
//!        O2a `chain_state` (best_hash / best_height)  — the point of the call
//!        O2b `utxo_set` + `producer_set`              — moved by `execute_reorg`
//!        O2c `sync_manager.force_reorg` slot          — consumed or retained
//!        O2d `sync_manager.reorg_handler.last_finality_height` (the MIRROR)
//!        O2e `cumulative_rollback_depth`              — MUST stay 0 (no poison arm)
//!   O3 return value: `Option<ForceReorgOutcome>` — `None` iff nothing was armed.
//!   O4 persistent store: `block_store` canonical index + bodies; the LOSING
//!        branch's bodies MUST survive (REQ-FORK-011).
//!   O5 statics: `doli_force_reorg_outcomes_total{outcome}` (asserted in
//!        `inc_i_204_m41_metrics`).
//!   O6 events: tracing only.
//!   PATHS asserted here: P-EXEC (landed), P-NOLAND (`execute_reorg` no-op).
//!   INPUT PARTITIONS: IP-3BLOCK full branch present; IP-2BLOCK exactly-2/3
//!   corroboration; IP-HOLE an intermediate body missing from cache AND store.
//!   MATRIX: {O2a, O2c, O2d, O2e, O3, O4} x {P-EXEC, P-NOLAND} — all cells asserted.

use doli_node::node::ForceReorgOutcome;

use super::inc_i_204_m41_common::{feed_branch, wedged_node, WEDGE_TIP_HEIGHT};

// ---------------------------------------------------------------------------
// A / K — the incident-shape rescue. One test, not two: the C-12 deterministic
// half IS this scenario (`tip == finality`, `0 < gap < 50`, sibling exhausted),
// and duplicating it would add a second name over one falsifier.
// ---------------------------------------------------------------------------

/// REQ-FORK-012 / REQ-FORK-011 — Decision: a failure means the audited operator door
/// does not open in the recorded wedge cell, so the fleet keeps only the unaudited
/// poison bypass and history-destroying snap sync as escapes.
#[tokio::test]
async fn c12_force_reorg_rescues_the_recorded_wedge_with_full_history() {
    let mut w = wedged_node().await;
    feed_branch(&mut w, 3).await;
    let winner = w.branch[2].hash();
    let loser = w.block_a.hash();

    // Wedge preconditions, stated so a later fixture drift cannot hide a false pass.
    {
        let sync = w.node.sync_manager.read().await;
        assert_eq!(
            sync.last_finalized_height(),
            Some(WEDGE_TIP_HEIGHT),
            "precondition: tip == finality == 6"
        );
        assert_eq!(
            sync.reorg_handler().last_finality_height(),
            Some(WEDGE_TIP_HEIGHT),
            "precondition: the reorg finality MARKER is armed — every common ancestor \
             of the competing branch is below it, so the automatic path cannot act"
        );
        assert!(
            !sync.is_snap_syncing(),
            "precondition: not snap syncing before the escape"
        );
    }
    assert_eq!(
        w.node.cumulative_rollback_depth, 0,
        "precondition: the BLOCK_POISON arm (rollback_one_block) has not run"
    );

    // The operator names the winning branch.
    w.node.sync_manager.write().await.arm_force_reorg(winner);

    let outcome = w.node.try_consume_force_reorg().await;
    assert_eq!(
        outcome,
        Some(ForceReorgOutcome::Executed),
        "O3: the escape must report Executed, and only when the tip actually moved"
    );

    // O2a — the node landed on the branch the operator named.
    {
        let cs = w.node.chain_state.read().await;
        assert_eq!(cs.best_hash, winner, "O2a: tip is the operator's target");
        assert_eq!(cs.best_height, 8, "O2a: and the branch's full height");
    }

    // O4 / REQ-FORK-011 — FULL history. The losing branch's body is still there,
    // and the canonical height index has no new hole.
    assert!(
        w.node
            .block_store
            .get_block(&loser)
            .expect("block_store read")
            .is_some(),
        "REQ-FORK-011: the LOSING branch's body must remain retrievable by hash \
         after the escape — recovery may never cost archival history"
    );
    assert!(
        w.node.block_store.has_contiguous_bodies(1, 8),
        "REQ-FORK-011: no NEW height-index gap in 1..=8 after the escape"
    );

    // Hard list item 5 — snap sync never invoked, never armed.
    assert!(
        !w.node.sync_manager.read().await.is_snap_syncing(),
        "the escape must never reach for snap sync"
    );

    // Hard list item 6 — the BLOCK_POISON arm was not the mechanism. `execute_reorg`
    // runs its own undo loop, so a non-zero counter can only mean rollback_one_block
    // (the poison door) ran.
    assert_eq!(
        w.node.cumulative_rollback_depth, 0,
        "the escape must not route through rollback_one_block — that is LB-4, the \
         thing being REPLACED"
    );

    // O2d — the mirror reconciled itself (execute_reorg clears below the new tip);
    // O2c — the directive is spent.
    assert_eq!(
        w.node.sync_manager.read().await.force_reorg_target(),
        None,
        "O2c: an executed directive is consumed"
    );
}

/// REQ-FORK-003 — Decision: a failure means the escape erased the monotone finality
/// record itself, so a node can be talked into forgetting what it finalized and the
/// next automatic reorg is unguarded (INV-FINALITY-001).
#[tokio::test]
async fn the_escape_never_mutates_the_finality_tracker() {
    let mut w = wedged_node().await;
    feed_branch(&mut w, 3).await;
    let before = w.node.sync_manager.read().await.last_finalized_height();
    assert_eq!(before, Some(WEDGE_TIP_HEIGHT), "precondition");

    w.node
        .sync_manager
        .write()
        .await
        .arm_force_reorg(w.branch[2].hash());
    let outcome = w.node.try_consume_force_reorg().await;
    assert_eq!(outcome, Some(ForceReorgOutcome::Executed));

    assert_eq!(
        w.node.sync_manager.read().await.last_finalized_height(),
        Some(WEDGE_TIP_HEIGHT),
        "INV-FINALITY-001: the escape touches ONLY the erasable ReorgHandler mirror. \
         FinalityTracker::last_finalized is the monotone truth store and must be \
         byte-identical across the escape"
    );
}

// ---------------------------------------------------------------------------
// H — no false green. `execute_reorg` returns Ok(()) on four silent no-op paths.
// ---------------------------------------------------------------------------

/// REQ-FORK-012 — Decision: a failure repeats the exact defect M3 fixed with
/// `RollbackOutcome` — an escape that reports success on an `Ok(())` no-op, so the
/// operator believes the node was rescued while it is still wedged and nothing
/// escalates.
#[tokio::test]
async fn a_reorg_that_does_not_land_is_never_reported_as_success() {
    let mut w = wedged_node().await;
    feed_branch(&mut w, 3).await;
    let winner = w.branch[2].hash();
    let missing = w.branch[1].hash(); // D1, h=7 — an INTERMEDIATE block

    // IP-HOLE: evict the intermediate body from the fork cache. `execute_reorg`
    // resolves new-block bodies from the fork cache, then the block store; D1 was
    // never applied, so after the eviction it is in neither and `execute_reorg`
    // takes its `Ok(())` early return (block_handling.rs:537).
    w.node.fork_block_cache.write().await.remove(&missing);
    assert!(
        w.node
            .block_store
            .get_block(&missing)
            .expect("block_store read")
            .is_none(),
        "precondition: the evicted body must be absent from the STORE too, or this \
         test proves nothing about the no-op path"
    );

    w.node.sync_manager.write().await.arm_force_reorg(winner);
    let outcome = w.node.try_consume_force_reorg().await;

    assert_ne!(
        outcome,
        Some(ForceReorgOutcome::Executed),
        "O3: `Executed` may be reported ONLY when chain_state.best_hash == target"
    );
    assert_eq!(
        outcome,
        Some(ForceReorgOutcome::ReorgDidNotLand),
        "O3: the no-op must get its OWN outcome, distinguishable from every refusal — \
         a refusal and a silent failure demand different operator actions"
    );

    // O2a — and the chain genuinely did not move.
    {
        let cs = w.node.chain_state.read().await;
        assert_eq!(cs.best_hash, w.block_a.hash(), "still on the losing branch");
        assert_eq!(cs.best_height, WEDGE_TIP_HEIGHT);
    }
    assert_eq!(
        w.node.sync_manager.read().await.force_reorg_target(),
        None,
        "O2c: a no-op is a terminal DECISION — the single shot is spent, not looped"
    );
}
