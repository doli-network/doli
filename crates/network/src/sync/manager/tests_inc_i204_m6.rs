//! INC-I-204 M6 / REQ-FORK-004 — snap sync narrowed to bootstrap-only.
//! TESTS-FIRST (RED). Design brief: `docs/.workflow/inc-i-204-M6-design-brief.md`.
//!
//! Four behaviour deletions, one file:
//!   D1 `recovery.rs` Rule 2 — drop `rollback_exhausted` + `deep_fork_confirmed`.
//!   D2 `recovery.rs` Rule 4 — add `&& !wedged_shape`.
//!   D3 `sync_engine/dispatch.rs` — drop the `gap >= 50` genesis-resync fallthrough.
//!   D4 `production_gate.rs` Gate 4 — drop the emergency `enable_snap_sync()`.
//!
//! The cell-space census for the same property lives in
//! `tests_inc_i204_m6_census.rs`; the finality-mirror pins live in
//! `crates/network/src/sync/reorg/tests_m6_mirror_pin.rs`.
//!
//! OUTPUT CONTRACT — `fn RecoveryCoordinator::classify(&self, &RecoveryContext)
//!                       -> RecoveryAction`
//!   O1 mutable params: none.  O2 receiver mutation: none (pure).
//!   O3 return value: `RecoveryAction` — the ONLY observable output.
//!   O4 persistent store: none.  O5 statics: none.  O6 events: tracing only.
//!   PATHS: P1/P1b rollback, P2 snap, P3 header-first, P4 genesis resync,
//!          P5 `Wedged` terminal / `None`.
//!   INPUT PARTITIONS: {rollback-exhausted minor gap, deep-fork medium gap,
//!          gap 499/500 boundary, snap-budget spent, peer quorum short,
//!          wedge shape + apply failures, non-forked broken node}.
//!   MATRIX: O3 × {P2, P4} × every partition above — asserted per test.
//!
//! OUTPUT CONTRACT — `fn SyncManager::next_request(&mut self)
//!                       -> Option<(PeerId, SyncRequest)>`
//!   O1 mutable params: none.  O2 receiver mutation: `fork.needs_genesis_resync`,
//!      `fork.consecutive_empty_headers`, `state`, `peers[*].pending_request`,
//!      `pipeline.*` — read back through the public getters below.
//!   O3 return value: `Option<(PeerId, SyncRequest)>`.
//!   O4 persistent store: none.  O5 statics: none.  O6 events: tracing only.
//!   PATHS (empty-headers escalation block): E1 `gap <= 3` gossip wait,
//!          E2 `3 < gap < 50` minor-fork park, E3 `gap >= 50` genesis fallthrough.
//!   INPUT PARTITIONS: gap ∈ {2, 10, 50, 600} at `consecutive_empty_headers == 10`.
//!   MATRIX: (O2 needs_genesis_resync, O2 consecutive_empty_headers, O2 state)
//!          × {E1, E2, E3}.
//!
//! OUTPUT CONTRACT — `fn SyncManager::request_genesis_resync(&mut self, reason)
//!                       -> bool`
//!   O1 mutable params: none.  O2 receiver mutation: `fork.needs_genesis_resync`
//!      and `snap.threshold` (the D4 defect).  O3 return value: `bool`.
//!   O4 persistent store: none.  O5 statics: none.  O6 events: tracing only.
//!   PATHS: G4a `--no-snap-sync` + non-emergency → refuse.
//!          G4b `--no-snap-sync` + emergency → honor (D4: without re-enabling snap).
//!   INPUT PARTITIONS: threshold ∈ {u64::MAX, enabled} × reason ∈ {emergency,
//!          forward-large-gap}.
//!   MATRIX: (O3 honored, O2 threshold, O2 needs_genesis_resync, downstream
//!          `start_sync` snap admission) × {G4a, G4b}.
//!
//! FAIL EVIDENCE (pre-fix, all four production files unmodified):
//!   RED       — `m6_d1_*`, `m6_d2_wedge_*`, `m6_d3_*_must_park`, `m6_d4_*`
//!   PASS-LOCK — `m6_lock_*`

use crypto::Hash;
use libp2p::PeerId;
use std::time::Instant;

use super::tests_inc_i139::mgr_with_agreeing_peers;
use crate::sync::manager::recovery::{
    thresholds, RecoveryAction, RecoveryContext, RecoveryCoordinator, RecoveryEvidence,
};
use crate::sync::manager::{
    RecoveryPhase, RecoveryReason, SyncConfig, SyncManager, SyncPhase, SyncPipelineData, SyncState,
};

const LOCAL_H: u64 = 1_000;
/// Past `STALE_TIP_SECS` (300) but below Rule 4's 600s `truly_stuck` bar.
const STALE_SECS: u64 = 420;
/// Past Rule 4's 600s bar.
const DEAD_SECS: u64 = 700;

fn ctx_at_gap(gap: u64) -> RecoveryContext {
    RecoveryContext {
        local_height: LOCAL_H,
        network_tip_height: LOCAL_H + gap,
        peer_count: 9,
        last_applied_secs: STALE_SECS,
        shallow_rollback_count: 0,
        snap_attempts: 0,
        last_rollback_local_height: None,
        last_rollback_time: None,
        in_grace_period: false,
        last_finality_height: None,
    }
}

fn report_empty_headers(coord: &mut RecoveryCoordinator, n: usize, gap: u64) {
    for _ in 0..n {
        coord.report(RecoveryEvidence::EmptyHeaders {
            peer: PeerId::random(),
            gap,
        });
    }
}

fn is_lossy(action: RecoveryAction) -> bool {
    matches!(
        action,
        RecoveryAction::SnapSync | RecoveryAction::GenesisResync
    )
}

// ===========================================================================
// D1 — Rule 2: fork evidence below gap 500 must not buy a snapshot
// ===========================================================================

/// REQ-FORK-004 — Decision: a failure here reveals that a node with fork evidence
/// and a spent rollback budget can still trade its block history for a snapshot at
/// a MINOR gap, so M6's "snap is bootstrap-only" property is false and the
/// INC-I-204 cohort's lossy escape survives the milestone.
///
/// The `rollback_exhausted` trigger: `minor_fork_evidence && !rollback_budget &&
/// !stuck_fork`. A spent budget is a statement about a BUDGET, not evidence of
/// behind-ness. Covers O3 on P2.
#[test]
fn m6_d1_rollback_exhausted_at_a_minor_gap_must_not_reach_snap() {
    let mut coord = RecoveryCoordinator::new();
    for _ in 0..3 {
        coord.report(RecoveryEvidence::OrphanGossip { slot: 1, gap: 3 });
    }
    let mut ctx = ctx_at_gap(3);
    ctx.shallow_rollback_count = thresholds::SHALLOW_ROLLBACK_MAX;

    let action = coord.classify(&ctx);

    assert!(
        !is_lossy(action),
        "REQ-FORK-004 (D1): a node with fork evidence at gap=3 and a spent shallow-rollback \
         budget reached the history-destroying rung {action:?}. A spent budget is not \
         behind-ness; after M6 only `gap >= SNAP_SYNC_GAP_MIN` may admit SnapSync."
    );
}

/// REQ-FORK-004 — Decision: a failure here reveals that a node can still earn a
/// snapshot by letting its fork gap GROW past MINOR_FORK_GAP_MAX — §4.3's "recovery
/// requires getting worse" — which is the precise escape M6 exists to close.
///
/// The `deep_fork_confirmed` trigger: `empty_count >= 10 && last_applied_secs >=
/// STALE_TIP_SECS && gap >= MINOR_FORK_GAP_MAX && !stuck_fork`. Covers O3 on P2.
#[test]
fn m6_d1_deep_fork_confirmed_shape_must_not_reach_snap() {
    let mut coord = RecoveryCoordinator::new();
    report_empty_headers(&mut coord, 12, 120);
    let ctx = ctx_at_gap(120);

    let action = coord.classify(&ctx);

    assert!(
        !is_lossy(action),
        "REQ-FORK-004 (D1): the deep-fork shape (12 empty-header responses, stale tip, \
         gap=120 < SNAP_SYNC_GAP_MIN) reached {action:?}. gap=120 is a FORK gap, not \
         behind-ness — after M6 it must fall through to a non-lossy rung."
    );
}

/// REQ-FORK-004 / LB-5 — Decision: a failure here reveals that M6 removed snap's
/// CAPABILITY rather than its fork REACHABILITY, stranding every genuinely
/// far-behind node — the regression that would make the deletion unshippable.
///
/// Boundary: gap==499 is not snap, gap==500 (`SNAP_SYNC_GAP_MIN`) is. Covers O3 on
/// P2/P3 at the exact fencepost.
#[test]
fn m6_lock_snap_boundary_499_is_not_snap_and_500_is() {
    let coord = RecoveryCoordinator::new();

    let below = coord.classify(&ctx_at_gap(thresholds::SNAP_SYNC_GAP_MIN - 1));
    assert_eq!(
        below,
        RecoveryAction::HeaderFirstSync,
        "LB-5 fencepost: gap=499 is below SNAP_SYNC_GAP_MIN and must stay header-first"
    );

    let at = coord.classify(&ctx_at_gap(thresholds::SNAP_SYNC_GAP_MIN));
    assert_eq!(
        at,
        RecoveryAction::SnapSync,
        "LB-5: gap=500 is genuine behind-ness — snap sync MUST remain reachable. M6 \
         removes snap's fork reachability, never its bootstrap/catch-up capability."
    );
}

/// REQ-FORK-004 / LB-6 — Decision: a failure here reveals that the surviving
/// large-gap door lost its attempt limiter or its peer quorum, which would let a
/// node loop snapshot attempts against too few peers — the DC-3 defect returning
/// through the one door M6 keeps open.
#[test]
fn m6_lock_large_gap_door_still_gated_by_attempts_and_peer_quorum() {
    let coord = RecoveryCoordinator::new();

    let mut exhausted = ctx_at_gap(900);
    exhausted.snap_attempts = thresholds::SNAP_ATTEMPTS_MAX;
    assert_ne!(
        coord.classify(&exhausted),
        RecoveryAction::SnapSync,
        "LB-6: `snap_attempts < SNAP_ATTEMPTS_MAX` must still gate the surviving \
         large-gap door"
    );

    let mut lonely = ctx_at_gap(900);
    lonely.peer_count = thresholds::SNAP_MIN_PEERS - 1;
    assert_ne!(
        coord.classify(&lonely),
        RecoveryAction::SnapSync,
        "LB-6: `peer_count >= SNAP_MIN_PEERS` must still gate the surviving large-gap door"
    );
}

// ===========================================================================
// D2 — Rule 4: a corroborated fork must never buy a genesis wipe
// ===========================================================================

/// REQ-FORK-004 — Decision: a failure here reveals that a node whose fork is
/// CORROBORATED (StuckFork + stale tip) can still be routed to a full state wipe by
/// accumulating apply failures, so the ladder's named non-lossy terminal is
/// bypassed by the most destructive rung on it.
///
/// Rule 4 sits ABOVE the `Wedged` terminal; without `&& !wedged_shape` the wedge
/// shape reaches `GenesisResync`. Covers O3 on P4 → P5.
#[test]
fn m6_d2_wedge_shape_with_apply_failures_must_wedge_not_wipe() {
    let mut coord = RecoveryCoordinator::new();
    coord.report(RecoveryEvidence::StuckFork { gap: 0 });
    coord.report(RecoveryEvidence::StaleTip {
        last_applied_secs: DEAD_SECS,
        gap: 0,
    });
    for i in 0..5 {
        coord.report(RecoveryEvidence::ApplyFailure {
            height: LOCAL_H + i,
        });
    }
    let mut ctx = ctx_at_gap(0);
    ctx.last_applied_secs = DEAD_SECS;
    ctx.snap_attempts = thresholds::SNAP_ATTEMPTS_MAX;

    let action = coord.classify(&ctx);

    assert_ne!(
        action,
        RecoveryAction::GenesisResync,
        "REQ-FORK-004 (D2): a corroborated fork (StuckFork + last_applied={DEAD_SECS}s) with \
         5 apply failures and an exhausted snap budget bought a GENESIS WIPE. A fork is not \
         a broken node — Rule 4 must be closed by `!wedged_shape`."
    );
    assert!(
        matches!(action, RecoveryAction::Wedged { .. }),
        "REQ-FORK-004 (D2): the wedge shape must fall through to the ladder's one named \
         non-lossy terminal, RecoveryAction::Wedged. Observed {action:?}."
    );
}

/// REQ-FORK-004 — Decision: a failure here reveals that D2 over-reached and took
/// the last-resort recovery away from a genuinely broken NON-forked node, which
/// would strand a node whose apply path is failing for reasons unrelated to a fork.
#[test]
fn m6_lock_non_forked_broken_node_still_reaches_genesis_resync() {
    let mut coord = RecoveryCoordinator::new();
    for i in 0..5 {
        coord.report(RecoveryEvidence::ApplyFailure {
            height: LOCAL_H + i,
        });
    }
    let mut ctx = ctx_at_gap(0);
    ctx.last_applied_secs = DEAD_SECS;
    ctx.snap_attempts = thresholds::SNAP_ATTEMPTS_MAX;

    assert_eq!(
        coord.classify(&ctx),
        RecoveryAction::GenesisResync,
        "REQ-FORK-004 (D2 scope): with NO StuckFork evidence the node is broken, not forked. \
         Rule 4 is its only remaining recovery and must be unchanged."
    );
}

// ===========================================================================
// D3 — dispatch.rs: the gap>=50 empty-headers funnel is gone
// ===========================================================================

/// Arms the empty-headers escalation block of `next_request()` exactly as
/// `class9_a1_does_not_reset_snap_attempts` does.
fn mgr_in_empty_headers_escalation(local: u64, peer_height: u64) -> SyncManager {
    let mut mgr = mgr_with_agreeing_peers(local, peer_height, 3);
    let peer = *mgr.peers.keys().next().expect("3 peers inserted");
    mgr.fork.use_height_based_headers = false;
    mgr.fork.height_fallback_attempted = false;
    mgr.fork.consecutive_empty_headers = 10;
    mgr.fork.needs_genesis_resync = false;
    mgr.snap.attempts = 0;
    mgr.confirmed_height_floor = 0;
    mgr.consecutive_resync_count = 0;
    mgr.recovery_phase = RecoveryPhase::Normal;
    mgr.state = SyncState::Syncing {
        phase: SyncPhase::DownloadingHeaders,
        started_at: Instant::now(),
    };
    mgr.pipeline_data = SyncPipelineData::Headers {
        target_slot: peer_height as u32,
        peer,
        headers_count: 0,
    };
    for s in mgr.peers.values_mut() {
        s.pending_request = None;
    }
    mgr
}

/// REQ-FORK-004 — Decision: a failure here reveals that the exact funnel that took
/// all six INC-I-204 nodes at gap=50 is still open, so the milestone did not remove
/// the production path that produced the incident.
///
/// Covers O2 (`fork.needs_genesis_resync`, `state`) on dispatch path E3.
#[test]
fn m6_d3_empty_headers_at_gap_50_must_park_not_resync() {
    let mut mgr = mgr_in_empty_headers_escalation(100, 150);

    let _ = mgr.next_request();

    assert!(
        !mgr.needs_genesis_resync(),
        "REQ-FORK-004 (D3): 10 empty-header responses at gap=50 (== MINOR_FORK_GAP_MAX) \
         requested a genesis resync. That fallthrough is the measured INC-I-204 funnel; \
         after M6 the park must extend to every gap > 3 and let the coordinator decide."
    );
    assert!(
        matches!(mgr.state, SyncState::Idle),
        "REQ-FORK-004 (D3): the node must PARK (Idle) for the coordinator. Observed {:?}.",
        mgr.state
    );
}

/// REQ-FORK-004 — Decision: a failure here reveals that a genuinely-behind node is
/// still routed to a state wipe by the dispatcher instead of by the classifier,
/// leaving the behind-ness decision in the one place that cannot tell it from a fork.
#[test]
fn m6_d3_empty_headers_at_gap_600_must_park_not_resync() {
    let mut mgr = mgr_in_empty_headers_escalation(100, 700);

    let _ = mgr.next_request();

    assert!(
        !mgr.needs_genesis_resync(),
        "REQ-FORK-004 (D3): 10 empty-header responses at gap=600 requested a genesis \
         resync from the DISPATCHER. A genuinely-behind node still earns SnapSync from \
         classifier Rule 2 on the next tick; the dispatcher must park."
    );
    assert!(
        matches!(mgr.state, SyncState::Idle),
        "REQ-FORK-004 (D3): the node must PARK (Idle). Observed {:?}.",
        mgr.state
    );
}

/// INV-SYNC-011 / REQ-FORK-004 — Decision: a failure here reveals that D3 changed a
/// `consecutive_empty_headers` reset writer, which is the counter INC-I-026 and
/// INC-I-139 E5 both turned on — a third reset writer makes the escalation threshold
/// unreachable again.
///
/// Covers O2 (`fork.consecutive_empty_headers`) on dispatch paths E1 and E2.
#[test]
fn m6_lock_gap_le_3_gossip_wait_and_its_only_reset_are_unchanged() {
    let mut mgr = mgr_in_empty_headers_escalation(100, 102);

    let out = mgr.next_request();

    assert!(
        out.is_none(),
        "INV-SYNC-011 / INC-I-026: the gap<=3 arm must issue no request"
    );
    assert_eq!(
        mgr.fork.consecutive_empty_headers, 0,
        "INV-SYNC-011: the bounded gap<=3 gossip-wait is one of the TWO permitted reset \
         writers and must keep resetting the counter"
    );
    assert!(
        matches!(mgr.state, SyncState::Idle),
        "INC-I-026: the gap<=3 arm parks in Idle. Observed {:?}.",
        mgr.state
    );
    assert!(
        !mgr.needs_genesis_resync(),
        "INC-I-026: gap<=3 is gossip timing, never a resync"
    );
}

/// INV-SYNC-011 — Decision: a failure here reveals that a park arm started zeroing
/// the empty-headers evidence counter, which would hide the very fork evidence the
/// coordinator needs to raise StuckFork and pick a rung.
#[test]
fn m6_lock_park_arms_above_gap_3_never_reset_the_evidence_counter() {
    for (local, peer_height, label) in [
        (100u64, 110u64, "minor-fork park (3 < gap < 50)"),
        (100, 150, "gap == MINOR_FORK_GAP_MAX"),
        (100, 700, "gap > SNAP_SYNC_GAP_MIN"),
    ] {
        let mut mgr = mgr_in_empty_headers_escalation(local, peer_height);
        let _ = mgr.next_request();
        assert_eq!(
            mgr.fork.consecutive_empty_headers, 10,
            "INV-SYNC-011 ({label}): only genuine block application and the gap<=3 \
             gossip-wait may reset consecutive_empty_headers. A park arm must not."
        );
    }
}

// ===========================================================================
// D4 — production_gate.rs Gate 4: --no-snap-sync is honored, not overridden
// ===========================================================================

/// REQ-FORK-004 / REQ-SNAP-008 — Decision: a failure here reveals that an emergency
/// still silently re-enables snap sync for an operator who passed `--no-snap-sync`,
/// converting a non-lossy header-first recovery into a state-replacing one against
/// an explicit operator instruction.
///
/// Covers O3 (honored) + O2 (`snap.threshold`, `fork.needs_genesis_resync`) on G4b.
#[test]
fn m6_d4_emergency_under_no_snap_sync_is_honored_without_re_enabling_snap() {
    let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    mgr.disable_snap_sync();
    assert_eq!(
        mgr.snap.threshold,
        u64::MAX,
        "precondition: --no-snap-sync in effect"
    );
    mgr.confirmed_height_floor = 0;
    mgr.snap.attempts = 0;
    mgr.consecutive_resync_count = 0;
    mgr.recovery_phase = RecoveryPhase::Normal;

    let honored = mgr.request_genesis_resync(RecoveryReason::GenesisFallbackEmptyHeaders);

    assert!(
        honored,
        "INC-I-139 anti-deadlock (REQ-SNAP-008): an emergency under --no-snap-sync must \
         still be HONORED. M6 deletes the re-enable, never the exemption from refusal."
    );
    assert!(
        mgr.needs_genesis_resync(),
        "INC-I-139: the honored emergency must arm the single X1 guard"
    );
    assert_eq!(
        mgr.snap.threshold,
        u64::MAX,
        "REQ-FORK-004 (D4): Gate 4 re-enabled snap sync (threshold={}) for an operator who \
         passed --no-snap-sync. The recovery must run the header-first reset_state_only \
         path that PRESERVES block data — exactly what needs_genesis_resync()'s own \
         doc-comment promises.",
        mgr.snap.threshold
    );
}

/// REQ-FORK-004 — Decision: a failure here reveals that the honored emergency still
/// opens the snap admission door downstream, so `--no-snap-sync` is defeated at the
/// consumer even if the threshold field were restored afterwards.
///
/// `decision.rs`'s `snap_allowed = self.snap.threshold < u64::MAX` is not directly
/// observable; its only effect is snap admission in `start_sync()`.
#[test]
fn m6_d4_honored_emergency_does_not_open_the_snap_admission_door() {
    let mut mgr = mgr_with_agreeing_peers(100, 700, 3);
    mgr.disable_snap_sync();
    mgr.confirmed_height_floor = 0;
    mgr.snap.attempts = 0;
    mgr.consecutive_resync_count = 0;
    mgr.recovery_phase = RecoveryPhase::Normal;

    let honored = mgr.request_genesis_resync(RecoveryReason::GenesisFallbackEmptyHeaders);
    assert!(honored, "precondition: the emergency is honored");

    mgr.state = SyncState::Idle;
    mgr.pipeline_data = SyncPipelineData::None;
    mgr.start_sync();

    assert!(
        !matches!(mgr.pipeline_data, SyncPipelineData::SnapCollecting { .. }),
        "REQ-FORK-004 (D4): after an honored emergency under --no-snap-sync the node \
         entered SnapCollecting. `snap_allowed` (decision.rs) must stay false — the \
         operator's flag survives the emergency."
    );
}

/// REQ-SNAP-008 — Decision: a failure here reveals that the D4 deletion also removed
/// the emergency exemption from Gate 4's REFUSAL, which would restore INC-I-139's
/// permanent deadlock for a forked `--no-snap-sync` node.
#[test]
fn m6_lock_non_emergency_under_no_snap_sync_is_still_refused() {
    let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    mgr.local_height = 100;
    mgr.network.network_tip_height = 700;
    mgr.disable_snap_sync();
    mgr.snap.attempts = 0;
    mgr.confirmed_height_floor = 100;
    mgr.consecutive_resync_count = 0;
    mgr.recovery_phase = RecoveryPhase::Normal;

    let honored = mgr.request_genesis_resync(RecoveryReason::CoordinatorSnapEscalation);

    assert!(
        !honored,
        "REQ-SNAP-008 (capability ii): a non-emergency reason must still be REFUSED under \
         --no-snap-sync. D4 deletes the re-enable block only, never the refusal gate."
    );
    assert!(
        !mgr.needs_genesis_resync(),
        "REQ-SNAP-008: a refused request must not arm needs_genesis_resync"
    );
    assert_eq!(
        mgr.snap.threshold,
        u64::MAX,
        "REQ-SNAP-008: a refused request must not touch snap.threshold"
    );
}
