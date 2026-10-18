// INC-I-235 M1 — a snap-disabled sync manager (the archive / relay role) must never
// be told to snap by the recovery coordinator, and must keep the Wedged terminal
// reachable past SNAP_SYNC_GAP_MIN. Snap-enabled behaviour must stay unchanged.
//
// covers: recovery.rs, block_lifecycle.rs, mod.rs (sync manager); node role wiring that feeds the flag: config.rs, run.rs, cli.rs
//
// OUTPUT CONTRACT: fn SyncManager::classify_and_dispatch(&mut self, shallow_rollback_count: u32) -> (RecoveryAction, Option<RecoveryContext>)
//   O1: mutable params — none (shallow_rollback_count is by value).
//   O2: self.recovery.last_action — armed for every non-None action (observed: an
//       immediate second tick returns None); self.snap.threshold, self.snap.attempts,
//       self.fork.needs_genesis_resync — never written by the tick.
//   O3: return.0 — the action; return.1 — Some(ctx), ctx.gap() and ctx.in_grace_period
//       are the inputs the action was derived from.
// OUTPUT CONTRACT: fn SyncManager::disable_snap_sync(&mut self) / enable_snap_sync(&mut self)
//   O2: self.snap.threshold (u64::MAX / < u64::MAX) AND the coordinator's snap-disabled
//       flag (new in M1; observed only through classify_and_dispatch's O3).
// OUTPUT CONTRACT: fn SyncManager::start_sync(&mut self) — downstream of the tick (T3)
//   O2: self.state phase, self.pipeline_data variant, self.fork.needs_genesis_resync,
//       self.snap.attempts.
// PATHS (rule ladder in recovery.rs once Rule 2 reads the snap-disabled flag):
//   P1: snap disabled, gap >= 500, no evidence        -> Rule 2 skipped, Rule 3 has no
//       stale tip, no wedge shape, Rule 4 has no apply failures -> None
//   P2: snap disabled, gap >= 500, StaleTip evidence  -> Rule 3 -> HeaderFirstSync
//   P3: snap disabled, gap 600, StuckFork + no apply for >= 300 s -> wedge shape; Rule 3
//       is closed by the wedge, Rule 4 has no apply failures, no finality and the rollback
//       budget unspent -> Wedged { NoActionableRung }
//   P4: snap enabled (default), gap == 500             -> Rule 2 -> SnapSync (unchanged)
//   P5: disable then enable, gap >= 500                -> Rule 2 -> SnapSync (symmetry)
//   P6: P1 tick, then the periodic SnapSync arm replayed, then start_sync -> header-first
// INPUT PARTITIONS:
//   P1a: incident shape, local 731_553 vs 5 agreeing peers at 734_838 (gap 3_285), fresh apply
//   P2a: same gap, StaleTip reported and no apply for 400 s (stale but not wedged)
//   P3a: gap 600 (just past the snap floor, far past the minor band), StuckFork, 400 s
//   P4a: gap exactly 500 (the >= boundary of SNAP_SYNC_GAP_MIN) on a default manager
//   P5a: incident gap after a disable/enable round trip
//   P6a: P1a manager; base code emits SnapSync, so the replayed arm hits Gate 4
// MATRIX: 3 outputs x 6 partitions
//   P1a: O3(action=None)✓ O3(ctx gap/grace)✓ O2(snap+fork untouched)✓
//   P2a: O3(action=HeaderFirstSync)✓ O2(cooldown armed)✓ O2(snap+fork untouched)✓
//   P3a: O3(action=Wedged NoActionableRung)✓ O3(ctx gap/stale/finality)✓ O2(snap untouched)✓
//   P4a: O3(action=SnapSync)✓ O3(ctx gap=500)✓ O2(threshold enabled)✓
//   P5a: O3(action=SnapSync)✓ O2(threshold enabled)✓
//   P6a: O2(phase DownloadingHeaders)✓ O2(pipeline Headers, not SnapCollecting)✓
//        O2(needs_genesis_resync false)✓ O2(snap.attempts unchanged)✓
//   O1 has no cells (no mutable parameters).

use std::time::{Duration, Instant};

use crate::sync::manager::recovery::{RecoveryAction, WedgeReason};
use crate::sync::manager::tests_inc_i139::mgr_with_agreeing_peers;
use crate::sync::manager::{
    RecoveryPhase, RecoveryReason, SyncManager, SyncPhase, SyncPipelineData, SyncState,
};

const INCIDENT_LOCAL_H: u64 = 731_553;
const INCIDENT_PEER_H: u64 = 734_838;
const INCIDENT_GAP: u64 = INCIDENT_PEER_H - INCIDENT_LOCAL_H;
const AGREEING_PEERS: usize = 5;
const NO_APPLY_SECS: u64 = 400;

fn not_in_grace(mut mgr: SyncManager) -> SyncManager {
    mgr.recovery_phase = RecoveryPhase::Normal;
    mgr.snap.attempts = 0;
    mgr
}

/// The archive / relay role as the sync manager sees it: `--no-snap-sync` in effect.
fn archive_mgr(local_h: u64, peer_h: u64) -> SyncManager {
    let mut mgr = mgr_with_agreeing_peers(local_h, peer_h, AGREEING_PEERS);
    mgr.disable_snap_sync();
    not_in_grace(mgr)
}

fn assert_snap_state_untouched(mgr: &SyncManager, label: &str) {
    assert_eq!(
        mgr.snap.threshold,
        u64::MAX,
        "{label}: the tick must not re-enable snap sync"
    );
    assert_eq!(
        mgr.snap.attempts, 0,
        "{label}: no snap attempt may be spent"
    );
    assert!(
        !mgr.fork.needs_genesis_resync,
        "{label}: the tick must not arm the snap admission flag"
    );
}

// T1 — REQ-ARCHSYNC-001 — Decision: a failure reveals the coordinator still orders a snapshot for an archive node far behind, which is the path that leaves the block-history hole.
#[test]
fn archive_far_behind_coordinator_never_emits_snapsync() {
    let mut quiet = archive_mgr(INCIDENT_LOCAL_H, INCIDENT_PEER_H);
    let (quiet_action, quiet_ctx) = quiet.classify_and_dispatch(0);
    let quiet_ctx = quiet_ctx.expect("classify_and_dispatch always returns its context");
    assert_eq!(quiet_ctx.gap(), INCIDENT_GAP, "precondition: incident gap");
    assert!(
        !quiet_ctx.in_grace_period,
        "precondition: outside the post-restart grace"
    );

    let mut stale = archive_mgr(INCIDENT_LOCAL_H, INCIDENT_PEER_H);
    stale.network.last_block_applied = Instant::now() - Duration::from_secs(NO_APPLY_SECS);
    stale.report_stale_tip(NO_APPLY_SECS, INCIDENT_GAP);
    let (stale_action, _) = stale.classify_and_dispatch(0);

    assert_eq!(
        [quiet_action, stale_action],
        [RecoveryAction::None, RecoveryAction::HeaderFirstSync],
        "INC-I-235: a snap-disabled manager at gap {INCIDENT_GAP} must fall through Rule 2. \
         [no evidence, stale tip] must yield [None, HeaderFirstSync]; SnapSync here is the \
         archive snap hole (the coordinator orders a snapshot the archive role forbids)."
    );

    let (second_tick, _) = stale.classify_and_dispatch(0);
    assert_eq!(
        second_tick,
        RecoveryAction::None,
        "HeaderFirstSync must still arm the action cooldown"
    );
    assert_snap_state_untouched(&quiet, "P1a");
    assert_snap_state_untouched(&stale, "P2a");
}

// T2 — REQ-ARCHSYNC-002 — Decision: a failure reveals Rule 2 still shadows the Wedged terminal at gap >= 500, so a forked archive node never reaches the non-lossy fork walk.
#[test]
fn archive_wedged_past_snap_gap_reaches_wedged() {
    let local_h = INCIDENT_LOCAL_H;
    let gap = 600;
    let mut mgr = archive_mgr(local_h, local_h + gap);
    mgr.network.last_block_applied = Instant::now() - Duration::from_secs(NO_APPLY_SECS);
    mgr.report_stuck_fork(gap);

    let (action, ctx) = mgr.classify_and_dispatch(0);
    let ctx = ctx.expect("classify_and_dispatch always returns its context");
    assert_eq!(ctx.gap(), gap, "precondition: gap 600");
    assert!(
        ctx.last_applied_secs >= 300,
        "precondition: no apply for >= STALE_TIP_SECS"
    );
    assert_eq!(
        ctx.last_finality_height, None,
        "precondition: no finality, so the reason is not FinalityConflict"
    );

    assert_eq!(
        action,
        RecoveryAction::Wedged {
            reason: WedgeReason::NoActionableRung
        },
        "INC-I-235: a snap-disabled manager in the wedge shape at gap {gap} must reach the \
         Wedged terminal; SnapSync means Rule 2 still returns before the terminal."
    );
    assert_snap_state_untouched(&mgr, "P3a");
}

// T3 — REQ-ARCHSYNC-001 — Decision: a failure reveals the periodic SnapSync arm can still move a snap-disabled node into SnapCollecting, so the role flag would not close the snap door downstream.
#[test]
fn archive_far_behind_start_sync_stays_header_first() {
    let mut mgr = archive_mgr(INCIDENT_LOCAL_H, INCIDENT_PEER_H);
    let attempts_before = mgr.snap.attempts;

    let (action, _) = mgr.classify_and_dispatch(0);
    if action == RecoveryAction::SnapSync {
        let _ = mgr.request_genesis_resync(RecoveryReason::CoordinatorSnapEscalation);
    }
    mgr.state = SyncState::Idle;
    mgr.pipeline_data = SyncPipelineData::None;
    mgr.start_sync();

    assert!(
        matches!(
            mgr.state,
            SyncState::Syncing {
                phase: SyncPhase::DownloadingHeaders,
                ..
            }
        ),
        "a snap-disabled node far behind must sync header-first, got {:?}",
        mgr.state
    );
    assert!(
        matches!(mgr.pipeline_data, SyncPipelineData::Headers { .. }),
        "the pipeline must carry a header download, never SnapCollecting"
    );
    assert!(
        !mgr.needs_genesis_resync(),
        "the snap admission flag must stay unarmed"
    );
    assert_eq!(
        mgr.snap.attempts, attempts_before,
        "no snap attempt may be spent"
    );
    assert_eq!(mgr.snap.threshold, u64::MAX, "snap must stay disabled");
}

// T4a — REQ-ARCHSYNC-002 — Decision: a failure reveals the archive fix leaked into snap-enabled nodes and removed their large-gap snapshot catch-up.
#[test]
fn snap_enabled_node_still_snaps_at_gap_500() {
    let local_h = 10_000;
    let mut mgr = not_in_grace(mgr_with_agreeing_peers(
        local_h,
        local_h + 500,
        AGREEING_PEERS,
    ));
    assert!(
        mgr.snap.threshold < u64::MAX,
        "precondition: a default manager has snap enabled"
    );

    let (action, ctx) = mgr.classify_and_dispatch(0);
    let ctx = ctx.expect("classify_and_dispatch always returns its context");
    assert_eq!(
        ctx.gap(),
        500,
        "precondition: the SNAP_SYNC_GAP_MIN boundary"
    );
    assert_eq!(
        action,
        RecoveryAction::SnapSync,
        "Rule 2 must still fire for a snap-enabled node at gap 500"
    );
    assert!(
        mgr.snap.threshold < u64::MAX,
        "the tick must not disable snap"
    );
}

// T4b — REQ-ARCHSYNC-002 — Decision: a failure reveals enable_snap_sync no longer clears the coordinator flag, so a node that re-enables snap stays silently snap-blind.
#[test]
fn enable_after_disable_restores_rule_2() {
    let mut mgr = archive_mgr(INCIDENT_LOCAL_H, INCIDENT_PEER_H);
    mgr.enable_snap_sync();
    assert!(
        mgr.snap.threshold < u64::MAX,
        "precondition: enable_snap_sync re-enables snap"
    );

    let (action, _) = mgr.classify_and_dispatch(0);
    assert_eq!(
        action,
        RecoveryAction::SnapSync,
        "disable then enable must restore Rule 2 at gap {INCIDENT_GAP}"
    );
}
