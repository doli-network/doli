//! INC-I-204 M3 / REQ-FORK-010 — ladder-termination proof, rung by rung.
//! TESTS-FIRST (RED). Split out of `tests_inc_i204_m3.rs` to stay inside the
//! 800-line test-module budget; fixtures and the cell space live there.
//!
//! One test per rung of the classifier ladder. Each asks the same question in a
//! different place: is this rung's EXIT reachable, and does iterating
//! `classify -> record_action` from it reach a NAMED terminal instead of a cycle?
//!
//! OUTPUT CONTRACT — `fn RecoveryCoordinator::classify(&self, &RecoveryContext)
//!                       -> RecoveryAction`
//!   O1 mutable params: none.  O2 receiver mutation: none (`&self`, pure).
//!   O3 return value: `RecoveryAction` — the ONLY observable output.
//!   O4 persistent store: none.  O5 statics: none.  O6 events: tracing only.
//!   PATHS: P1 Rule 1/1b rollback, P1b' bounded sibling probe, P2 snap,
//!          P3 header-first, P4 genesis resync, P5 quiescent `None`.
//!
//!   INPUT PARTITIONS (one test each):
//!     IP-R1 minor gap, guard permits, rollback budget burned to exhaustion
//!           → `rung_rule1_...`
//!     IP-R2 minor gap, guard refuses, sibling probe budget burned, then re-armed
//!           by the fall-through rung → `rung_rule1b_...`
//!     IP-R3 gap >= 500 with snap budget (legitimate) vs a fork-sized gap that
//!           only reached snap by growing → `rung_rule2_...`
//!     IP-R4 plain behind-ness, no fork evidence vs peers rejecting our tip hash
//!           → `rung_rule3_...`
//!     IP-R5 apply failures + snap exhausted at gap=0 vs snap budget remaining
//!           → `rung_rule4_...`
//!
//! OUTPUT CONTRACT — `fn RecoveryCoordinator::record_action(&mut self, RecoveryAction)`
//!   O1 mutable params: none.  O2 receiver mutation: `last_action` and
//!      `sibling_fetch_attempts`, read back through the next `classify`.
//!   O3 return value: `()`.  O4/O5/O6: none.
//!   PATHS: R1 `None` → no change. R2 `SiblingFetch` → budget +1.
//!          R3 any other action → budget RESET (the defect IP-R2 exposes).
//!   INPUT PARTITIONS: R2/R3 interleaved → `rung_rule1b_...`; R3 → `rung_rule1_...`.
//!
//! FAIL EVIDENCE (pre-fix, `recovery.rs` classifier unmodified):
//!   RED       — `rung_rule1_*`, `rung_rule1b_*`, `rung_rule2_*`, `rung_rule3_*`
//!   PASS-LOCK — `rung_rule4_*`

use crate::sync::manager::recovery::{
    thresholds, RecoveryAction, RecoveryContext, RecoveryCoordinator, RecoveryEvidence, WedgeReason,
};
use crate::sync::manager::tests_inc_i204_m3::{
    cell_coordinator, cell_ctx, drive, is_lossy, rung_name, settled, Budget, Cell, FinalityPos,
    GapBand, COHORT_PEERS, LOCAL_H,
};

// ===========================================================================
// (3) LADDER TERMINATION, RUNG BY RUNG
// ===========================================================================

/// REQ-FORK-010 — Decision: a failure here reveals that spending the shallow
/// rollback budget hands the node to snap sync at a fork-sized gap, which is trap
/// T8 — the lossy terminal becoming reachable again from the wedge band.
#[test]
fn rung_rule1_shallow_rollback_exit_must_not_fall_into_the_lossy_rung() {
    let cell = Cell {
        gap: GapBand::Minor,
        fin: FinalityPos::AboveFinality,
        rollback: Budget::Fresh,
        sibling: Budget::Fresh,
        snap: Budget::Fresh,
    };
    let seq = drive(cell);

    assert!(
        seq.iter()
            .any(|a| matches!(a, RecoveryAction::ShallowRollback { .. })),
        "fixture: Rule 1/1b must be reachable at {} with the guard permitting it",
        cell.name()
    );

    let rollbacks = seq
        .iter()
        .filter(|a| matches!(a, RecoveryAction::ShallowRollback { .. }))
        .count();
    assert!(
        rollbacks <= thresholds::SHALLOW_ROLLBACK_MAX as usize,
        "LB-7: the rollback rung must stay bounded by SHALLOW_ROLLBACK_MAX; observed \
         {rollbacks} rollbacks"
    );

    let fixpoint = settled(&seq).unwrap_or_else(|| {
        panic!(
            "{} never settled: {:?}",
            cell.name(),
            seq.iter().map(|a| rung_name(*a)).collect::<Vec<_>>()
        )
    });
    assert!(
        !is_lossy(fixpoint),
        "REQ-FORK-010 / trap T8: driving {} until the rollback budget is spent, the ladder \
         terminated on `{}` — a rung that discards block history to resolve a fork of {} \
         blocks. The successor to an exhausted rollback budget must be a named, non-lossy \
         terminal.",
        cell.name(),
        rung_name(fixpoint),
        cell.gap.gap()
    );
}

/// REQ-FORK-010 — Decision: a failure here reveals that the bounded non-destructive
/// probe never actually terminates, because the rung it falls through to resets the
/// probe budget via `record_action`'s catch-all arm — a 4-tick cycle, not a bound.
/// That is the INC-I-143 livelock with a 30s pacer bolted on.
#[test]
fn rung_rule1b_sibling_fetch_bound_must_survive_its_own_fall_through() {
    let cell = Cell {
        gap: GapBand::Minor,
        fin: FinalityPos::AtFinality,
        rollback: Budget::Fresh,
        sibling: Budget::Fresh,
        snap: Budget::Fresh,
    };
    let seq = drive(cell);

    let mut run = 0usize;
    let mut longest = 0usize;
    for action in &seq {
        if matches!(action, RecoveryAction::SiblingFetch { .. }) {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    assert!(
        longest <= thresholds::SIBLING_FETCH_MAX as usize,
        "LB-9 / trap T11: no more than SIBLING_FETCH_MAX consecutive probes; observed a \
         run of {longest} in {:?}",
        seq.iter().map(|a| rung_name(*a)).collect::<Vec<_>>()
    );

    let fixpoint = settled(&seq).unwrap_or_else(|| {
        panic!(
            "REQ-FORK-010: {} never settles — the ladder CYCLES. `record_action`'s \
             catch-all arm resets `sibling_fetch_attempts` on the very rung the probe \
             falls through to, so the budget refills every lap and the bound never \
             terminates anything. Observed: {:?}",
            cell.name(),
            seq.iter().map(|a| rung_name(*a)).collect::<Vec<_>>()
        )
    });
    assert_eq!(
        fixpoint,
        RecoveryAction::Wedged {
            reason: WedgeReason::FinalityConflict
        },
        "REQ-FORK-010: when the finality guard correctly refuses the rollback (LB-1) and \
         the bounded probe is spent, the ladder must fall through to the NAMED terminal, \
         not to `{}`.",
        rung_name(fixpoint)
    );
}

/// REQ-FORK-010 — Decision: a failure here reveals that snap sync is still reachable
/// as a fork remedy (or has stopped being reachable for genuine behind-ness), which
/// decides whether M6 can narrow snap admission at all.
#[test]
fn rung_rule2_snap_sync_terminates_only_for_genuine_behindness() {
    let far_behind = Cell {
        gap: GapBand::Large,
        fin: FinalityPos::Unfinalized,
        rollback: Budget::Fresh,
        sibling: Budget::Fresh,
        snap: Budget::Fresh,
    };
    assert_eq!(
        cell_coordinator(far_behind).classify(&cell_ctx(far_behind)),
        RecoveryAction::SnapSync,
        "LB-5: snap must remain the remedy for a genuinely far-behind node ({})",
        far_behind.name()
    );

    let forked = Cell {
        gap: GapBand::Medium,
        fin: FinalityPos::AtFinality,
        rollback: Budget::Fresh,
        sibling: Budget::Spent,
        snap: Budget::Fresh,
    };
    let action = cell_coordinator(forked).classify(&cell_ctx(forked));
    assert!(
        !is_lossy(action),
        "REQ-FORK-010 / §4.3: at {} the node is FORKED, not behind — `deep_fork_confirmed` \
         opened the snap door purely because the gap grew past MINOR_FORK_GAP_MAX. It \
         returned `{}`, destroying the block history the node was still serving.",
        forked.name(),
        rung_name(action)
    );
}

/// REQ-FORK-010 — Decision: a failure here reveals whether `HeaderFirstSync` is
/// still emitted for a node whose peers reject its tip hash — the silent 30s
/// cooldown loop against a comment-only dispatch arm.
#[test]
fn rung_rule3_header_first_is_reserved_for_nodes_that_can_actually_extend() {
    let mut behind = RecoveryCoordinator::new();
    behind.report(RecoveryEvidence::StaleTip {
        last_applied_secs: 90,
        gap: 120,
    });
    let behind_ctx = RecoveryContext {
        local_height: LOCAL_H,
        network_tip_height: LOCAL_H + 120,
        peer_count: COHORT_PEERS,
        last_applied_secs: 90,
        shallow_rollback_count: 0,
        snap_attempts: 0,
        last_rollback_local_height: None,
        last_rollback_time: None,
        in_grace_period: false,
        last_finality_height: Some(LOCAL_H - 10),
    };
    assert_eq!(
        behind.classify(&behind_ctx),
        RecoveryAction::HeaderFirstSync,
        "Rule 3 must keep serving plain behind-ness: no fork evidence, gap 120, peers \
         still extend from our tip. Deleting this rung outright is not the M3 contract."
    );

    let wedged = Cell {
        gap: GapBand::Minor,
        fin: FinalityPos::AtFinality,
        rollback: Budget::Fresh,
        sibling: Budget::Spent,
        snap: Budget::Spent,
    };
    let action = cell_coordinator(wedged).classify(&cell_ctx(wedged));
    assert_ne!(
        action,
        RecoveryAction::HeaderFirstSync,
        "REQ-FORK-010 (C3): twelve peers returned ZERO headers for our tip hash, so \
         header-first cannot extend from it. Emitting it anyway arms a 30s cooldown \
         against a dispatch arm that performs no action — the node goes silent and \
         re-arms the same silence every tick, forever, at {}.",
        wedged.name()
    );
}

/// REQ-FORK-010 — Decision: a failure here reveals that the last-resort rung either
/// became reachable without its preconditions (a state wipe on ordinary evidence)
/// or stopped terminating.
#[test]
fn rung_rule4_genesis_resync_stays_gated_behind_its_preconditions() {
    let mut coord = RecoveryCoordinator::new();
    for h in 0..6u64 {
        coord.report(RecoveryEvidence::ApplyFailure {
            height: LOCAL_H + h,
        });
    }
    let mut ctx = cell_ctx(Cell {
        gap: GapBand::AtTip,
        fin: FinalityPos::Unfinalized,
        rollback: Budget::Fresh,
        sibling: Budget::Fresh,
        snap: Budget::Spent,
    });
    ctx.last_applied_secs = 700;

    assert_eq!(
        coord.classify(&ctx),
        RecoveryAction::GenesisResync,
        "Rule 4's exit must remain reachable: 6 apply failures, 700s silent, snap spent, \
         and gap=0 with no StaleTip evidence (any gap > 0 lets Rule 3 intercept first)"
    );

    ctx.snap_attempts = thresholds::SNAP_ATTEMPTS_MAX - 1;
    assert_ne!(
        coord.classify(&ctx),
        RecoveryAction::GenesisResync,
        "Rule 4 must never wipe state while snap attempts remain — the ladder's most \
         destructive rung may not move up the ordering"
    );

    assert!(
        is_lossy(RecoveryAction::GenesisResync),
        "GenesisResync must stay classified as lossy so no wedge cell may terminate on it"
    );
}
