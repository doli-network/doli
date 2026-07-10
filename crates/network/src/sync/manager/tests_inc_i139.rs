//! INC-I-139 M1 — TESTS-ONLY milestone (run_id 455).
//!
//! Regression classes 1-8 for the snap-admission redesign (INV-SYNC-011,
//! REQ-SNAP-001/002/003/007/010). This is a `redesign` workflow milestone:
//! some tests LOCK current good behavior (must PASS), others reproduce the
//! defects and MUST FAIL against current code — those are committed
//! `#[ignore]`-annotated so the tree stays green. M2/M3/M5 runners de-ignore
//! them and verify FAIL→PASS.
//!
//! DC map (deletions/companions the ignored tests target):
//!   DC-1 (decision.rs:168 `|| gap > self.snap.threshold`)  → Class 2
//!   DC-2 (production_gate.rs Gate 1 forward-large-gap exempt) → Class 4
//!   DC-4 (dispatch.rs:84 unconditional counter reset)         → Class 3
//!
//! ZERO source changes: every assertion uses only the crate-internal API the
//! INC-I-138 tests already drive (fields, `start_sync`, `next_request`,
//! `request_genesis_resync`, `needs_genesis_resync`, `RecoveryCoordinator::classify`).
//! Where an ideal observable (the SnapCollecting transition) is not reachable
//! from a coordinator-only path, we assert on the nearest reachable observable
//! (the `request_genesis_resync` bool that gates it) and note it in the contract.

use crypto::Hash;
use libp2p::PeerId;
use std::time::Instant;

use crate::sync::manager::recovery::{
    thresholds, RecoveryAction, RecoveryContext, RecoveryCoordinator, RecoveryEvidence,
};
use crate::sync::manager::{
    RecoveryPhase, RecoveryReason, SyncConfig, SyncManager, SyncPhase, SyncPipelineData, SyncState,
};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// A SyncManager with `n` peers all at `peer_height` sharing `peer_hash`.
///
/// Identical (height, hash) across ≥2 peers is required so `consensus_target_hash()`
/// returns Some — otherwise `start_sync` falls back to header-first even when
/// `should_snap` is true, which would mask the Route-A admission we are probing.
fn mgr_with_agreeing_peers(local_height: u64, peer_height: u64, n: usize) -> SyncManager {
    let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    mgr.local_height = local_height;
    mgr.local_slot = local_height as u32;
    mgr.local_hash = crypto::hash::hash(format!("local_{}", local_height).as_bytes());
    let peer_hash = crypto::hash::hash(b"canonical_agreed_tip");
    for _ in 0..n {
        // add_peer sets network_tip_height from the peer claim and defaults
        // pending_request=None (INC-I-138 harness convention).
        mgr.add_peer(PeerId::random(), peer_height, peer_hash, peer_height as u32);
    }
    mgr
}

/// The INC-I-138 Phase-2 coordinator context: gap=28, stale tip, 5 peers.
/// Reused by classes 1/5/7 that assert on `RecoveryCoordinator::classify`.
fn ctx(local_height: u64, network_tip_height: u64, last_applied_secs: u64) -> RecoveryContext {
    RecoveryContext {
        local_height,
        network_tip_height,
        peer_count: 5,
        last_applied_secs,
        shallow_rollback_count: 0,
        snap_attempts: 0,
        last_rollback_local_height: None,
        last_rollback_time: None,
        in_grace_period: false,
        last_finality_height: None,
    }
}

// ===========================================================================
// CLASS 1 — N4 wedge (REQ-SNAP-002 / INV-SYNC-010). MUST PASS. NOT ignored.
//
// finality == local_tip == fork-tip, node self-producing on the minority branch:
//   (a) no snap below gap 50 — the wedge parks (finality guard) instead;
//   (b) evidence-gated snap fires ONLY at gap≥50 + empties≥10 + stale.
//
// OUTPUT CONTRACT: fn RecoveryCoordinator::classify(&self, ctx) -> RecoveryAction
//   O1: RecoveryAction
//   PATHS: P1 wedge (gap<50, finality==local_tip)  P2 evidence-gated (gap≥50)
//   INPUT PARTITIONS: IP1 gap=10 finality-guarded ; IP2 gap=55 empties=10 stale
//   MATRIX (O1): IP1 → None (≠SnapSync) ; IP2 → SnapSync
// ===========================================================================

/// Class 1a (REQ-SNAP-002, INC-I-139 wedge): finality==local_tip → no snap, park.
#[test]
fn class1_n4_wedge_parks_no_snap_below_gap_50() {
    let mut coord = RecoveryCoordinator::new();
    // Minor-fork evidence (≥MIN_MINOR_FORK_EVIDENCE) at a small gap.
    for _ in 0..2 {
        coord.report(RecoveryEvidence::EmptyHeaders {
            peer: PeerId::random(),
            gap: 10,
        });
    }
    // finality == local_tip (100): any depth-1 rollback target (99) < finality.
    let mut c = ctx(100, 110, 5); // gap=10, recently_synced
    c.last_finality_height = Some(100);

    let action = coord.classify(&c);

    // O1/IP1: the finality guard refuses the ShallowRollback (target 99 < finality
    // 100) → None. Below gap 50 with no deep-fork evidence, snap is NEVER admitted.
    assert_ne!(
        action,
        RecoveryAction::SnapSync,
        "Class 1 (INV-SYNC-010): wedge at gap=10 must NOT admit SnapSync"
    );
    assert_eq!(
        action,
        RecoveryAction::None,
        "Class 1: finality==local_tip parks (Phase-1 764s trap) — not rollback, not snap"
    );
}

/// Class 1b (REQ-SNAP-002): evidence-gated snap only at gap≥50 + empties≥10 + stale.
#[test]
fn class1_evidence_gated_snap_only_at_gap_50_plus_empties() {
    let mut coord = RecoveryCoordinator::new();
    for _ in 0..10 {
        coord.report(RecoveryEvidence::EmptyHeaders {
            peer: PeerId::random(),
            gap: 55,
        });
    }
    coord.report(RecoveryEvidence::StaleTip {
        last_applied_secs: 325,
        gap: 55,
    });
    // gap=55 ≥ MINOR_FORK_GAP_MAX(50), stale ≥ STALE_TIP_SECS(300).
    let c = ctx(100, 155, 325);

    let action = coord.classify(&c);

    // O1/IP2: deep_fork_confirmed = empty≥10 && stale≥300 && gap≥50 → Rule 2 SnapSync.
    assert_eq!(
        action,
        RecoveryAction::SnapSync,
        "Class 1: corroborated evidence at gap={} (≥{}) must escalate to SnapSync",
        c.gap(),
        thresholds::MINOR_FORK_GAP_MAX
    );
}

// ===========================================================================
// CLASS 2 — N1 bare-gap (REQ-SNAP-002, INC-I-139 replay). DC-1 reproduction.
// MUST FAIL against current code (decision.rs:168 Route A still admits). IGNORED.
//
// OUTPUT CONTRACT: fn start_sync(&mut self) [decision.rs:117]
//   O1: self.pipeline_data — SnapCollecting iff snap admitted (proxy for SNAP_TRIGGER)
//   O2: self.state.phase   — SnapCollecting iff snap admitted
//   PATH P1: gap=51 > snap.threshold(50), no fork evidence, local_height>0
//   INPUT PARTITION: bare gap only (needs_genesis_resync=false, local_height≠0)
//   MATRIX (O1): P1 → NOT SnapCollecting (MUST). FAILS today: Route A admits snap.
// ===========================================================================

/// Class 2 (REQ-SNAP-002, DC-1): a bare 51-block gap with no fork evidence must
/// NOT admit snap. Today decision.rs:168 `|| gap > self.snap.threshold` admits it.
#[test]
fn class2_n1_bare_gap_51_must_not_snap() {
    // local=100, 3 agreeing peers at 151 → gap=51 > snap.threshold(50).
    //
    // NOTE: `add_peer` auto-invokes `start_sync()` on the FIRST peer
    // (SyncConfig::default min_peers_for_sync=1). With only 1 peer present at that
    // moment `enough_peers` is false, so the manager commits to header-first and — being
    // `is_syncing` — never re-evaluates the snap decision when the 3rd peer arrives.
    // There is therefore NO Idle precondition to assert. To probe the DC-1 Route-A
    // snap decision with the full 3-peer quorum present — the realistic
    // Synchronized→peers-advance-51 path — reset to Idle and drive start_sync once with
    // all 3 agreeing peers connected. This reflects real current behavior: start_sync
    // evaluating a bare gap=51 against 3 peers admits snap via decision.rs:168.
    let mut mgr = mgr_with_agreeing_peers(100, 151, 3);
    mgr.fork.needs_genesis_resync = false; // no deep-fork signal
    mgr.state = SyncState::Idle;
    mgr.pipeline_data = SyncPipelineData::None;

    mgr.start_sync();

    // O1: SNAP_TRIGGER count must be 0 for a bare gap. pipeline_data is the
    // reachable proxy for the SnapCollecting (X1) transition.
    let snapped = matches!(mgr.pipeline_data, SyncPipelineData::SnapCollecting { .. });
    assert!(
        !snapped,
        "Class 2 (DC-1): bare gap=51 with no fork evidence admitted snap via Route A \
         (decision.rs:168 `|| gap > self.snap.threshold`). Post-DC-1 this must fall \
         through to header-first sync."
    );
}

// ===========================================================================
// CLASS 3 — counter-starvation co-test (REQ-SNAP-007; INC-I-012 F1 + INC-I-138
// D2 + INC-I-139 E5). DC-4 reproduction. MUST FAIL today (dispatch.rs:84 reset
// still starves the counter). IGNORED.
//
// OUTPUT CONTRACT: fn next_request(&mut self) [dispatch.rs:15]
//   (SyncPipelineData::Headers branch, use_height_based_headers=true → :72)
//   O1: self.fork.consecutive_empty_headers — the evidence counter
//   PATH P1: sustained empty headers WITH a per-cycle request-shape change
//            (use_height_based_headers re-set) firing the dispatch.rs:84 reset
//   INPUT PARTITION: 15 cycles, +1 empty per cycle, reset via :84 each cycle
//   MATRIX (O1): P1 → max counter reaches the escalation threshold (≥10) (MUST).
//                FAILS today: :84 resets to 0 every cycle → max stays 1.
// ===========================================================================

/// Class 3 (REQ-SNAP-007, DC-4): under sustained empty headers the evidence
/// counter must reach the escalation threshold despite request-shape changes.
/// Today dispatch.rs:84 resets it unconditionally on every height-based request.
#[ignore = "FAILS-BY-DESIGN until DC-4 lands (INC-I-139 M5)"]
#[test]
fn class3_counter_not_starved_by_dispatch84_reset() {
    let mut mgr = mgr_with_agreeing_peers(36, 64, 5); // gap=28
    let peer = *mgr.peers.keys().next().expect("5 peers inserted");
    mgr.state = SyncState::Syncing {
        phase: SyncPhase::DownloadingHeaders,
        started_at: Instant::now(),
    };
    mgr.pipeline_data = SyncPipelineData::Headers {
        target_slot: 664,
        peer,
        headers_count: 0,
    };
    mgr.fork.height_fallback_attempted = false;

    const CYCLES: u32 = 15;
    let mut max_counter: u32 = 0;
    for _ in 0..CYCLES {
        // Sustained empty header arrives.
        mgr.fork.consecutive_empty_headers += 1;
        max_counter = max_counter.max(mgr.fork.consecutive_empty_headers);
        // Request-shape change (INC-I-139 E5): the post-snap height-fallback flag
        // is re-armed, and dispatch.rs:72→:84 wipes the counter on the request.
        mgr.fork.use_height_based_headers = true;
        for s in mgr.peers.values_mut() {
            s.pending_request = None; // simulate the prior response being consumed
        }
        let _ = mgr.next_request();
    }

    // O1: the counter must be able to reach the dispatch.rs:96 escalation threshold
    // (≥10) so deep-fork evidence can escalate. dispatch.rs:84 starves it to 1.
    assert!(
        max_counter >= 10,
        "Class 3 (DC-4): consecutive_empty_headers maxed at {} across {} cycles. \
         dispatch.rs:84 resets it unconditionally on every height-based request, \
         starving the evidence pipeline. Post-DC-4 (reset removed) it must accumulate.",
        max_counter,
        CYCLES
    );
}

// ===========================================================================
// CLASS 4 — floor>0, gap≥500 coordinator catch-up (REQ-SNAP-003). DC-2 proof.
// MUST FAIL today: Gate 1 refuses CoordinatorSnapEscalation when floor>0, so a
// legitimate forward-large-gap coordinator snap is blocked — proving DC-2 is
// load-bearing (a Route-A deletion WITHOUT DC-2 would strand this consumer).
// IGNORED.
//
// OUTPUT CONTRACT: fn request_genesis_resync(&mut self, reason) -> bool
//   [production_gate.rs:660]
//   O1: return bool — true iff all gates pass (→ needs_genesis_resync set → X1)
//   O2: self.fork.needs_genesis_resync — set iff O1
//   PATH P1: floor>0, reason=CoordinatorSnapEscalation, gap≥500, snap enabled
//   INPUT PARTITION: forward-large-gap coordinator reason with a monotonic floor
//   MATRIX (O1): P1 → true (POST-DC-2 expectation). FAILS today: Gate 1 → false.
// ===========================================================================

/// Class 4 (REQ-SNAP-003, DC-2): a forward coordinator snap at gap≥500 with a
/// confirmed floor must pass Gate 1 and reach X1. Today Gate 1 (production_gate.rs
/// :674 `confirmed_height_floor>0 && !is_emergency`) refuses CoordinatorSnapEscalation
/// because it is NOT in the emergency set — proving the DC-2 forward-large-gap
/// exemption is load-bearing once Route A (DC-1) is deleted.
#[test]
fn class4_floor_gap500_coordinator_snap_passes_gate1() {
    let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    mgr.local_height = 100;
    mgr.network.network_tip_height = 700; // gap=600 ≥ SNAP_SYNC_GAP_MIN(500)
    mgr.confirmed_height_floor = 100; // monotonic floor set (post first sync)
    mgr.consecutive_resync_count = 0;
    mgr.recovery_phase = RecoveryPhase::Normal;
    // snap.threshold defaults to 50 (< u64::MAX) and snap.attempts=0 → Gates 4/5 pass.

    let honored = mgr.request_genesis_resync(RecoveryReason::CoordinatorSnapEscalation);

    // O1: POST-DC-2, forward-large-gap reasons (CoordinatorSnapEscalation,
    // StuckSyncLargeGap) join the Gate-1 exemption set. Today they do not, so
    // request is refused (false) — this test proves DC-2 is required.
    assert!(
        honored,
        "Class 4 (DC-2): CoordinatorSnapEscalation at gap=600 with floor=100 was \
         REFUSED by Gate 1 (production_gate.rs:674). Post-DC-2 the forward-large-gap \
         reasons must be floor-exempt so a legitimate gap≥500 catch-up reaches X1."
    );
    assert!(
        mgr.needs_genesis_resync(),
        "Class 4: honored request must set needs_genesis_resync (feeds the single X1 guard)"
    );
}

// ===========================================================================
// CLASS 5 — INC-I-138 replay (REQ-SNAP-002). MUST PASS. NOT ignored.
//
// gap=28 + counter=10 → the minor-fork regime guard (dispatch.rs:144) keeps the
// node off genesis-resync. Locks the shipped INC-I-138 D2+D4 fix.
//
// OUTPUT CONTRACT: fn next_request(&mut self) [dispatch.rs:15]
//   (Headers branch, consecutive_empty_headers≥10, use_height_based_headers=false)
//   O1: self.fork.needs_genesis_resync — MUST stay false in the 4..50 gap regime
//   PATH P1: counter=10, gap=28, floor=0 → :96 escalation → :144 regime guard → None
//   MATRIX (O1): P1 → false (regime guard intact). PASSES today.
// ===========================================================================

/// Class 5 (REQ-SNAP-002): gap=28 with 10 empty headers must NOT genesis-resync —
/// the dispatch.rs:144 minor-fork regime guard (INC-I-138 fix) holds.
#[test]
fn class5_inc_i138_replay_gap28_no_genesis_resync() {
    let mut mgr = mgr_with_agreeing_peers(36, 64, 5); // gap=28
    let peer = *mgr.peers.keys().next().expect("5 peers inserted");
    mgr.fork.consecutive_empty_headers = 10; // escalation threshold reached
    mgr.fork.use_height_based_headers = false; // reach :96, not the :72 fallback
    mgr.fork.height_fallback_attempted = false;
    mgr.confirmed_height_floor = 0; // :105 snap redirect requires floor>0 → skipped
    mgr.state = SyncState::Syncing {
        phase: SyncPhase::DownloadingHeaders,
        started_at: Instant::now(),
    };
    mgr.pipeline_data = SyncPipelineData::Headers {
        target_slot: 664,
        peer,
        headers_count: 0,
    };

    let _ = mgr.next_request();

    // O1: dispatch.rs:144 (gap>3 && gap<MINOR_FORK_GAP_MAX) parks in Idle without
    // requesting genesis-resync. A 28-block fork is ShallowRollback range.
    assert!(
        !mgr.fork.needs_genesis_resync,
        "Class 5 (INC-I-138 replay): gap=28 with counter=10 must NOT genesis-resync; \
         the minor-fork regime guard (dispatch.rs:144) must park for coordinator ShallowRollback"
    );
}

// ===========================================================================
// CLASS 6 — fresh bootstrap (REQ-SNAP-003). MUST PASS. NOT ignored.
//
// local_height==0 snaps via Route C (decision.rs:167 `local_height == 0`),
// which DC-1 does NOT touch (only the bare-gap OR-term is deleted).
//
// OUTPUT CONTRACT: fn start_sync(&mut self) [decision.rs:117]
//   O1: self.pipeline_data — SnapCollecting (Route C admits at h==0)
//   PATH P1: local_height=0, ≥3 agreeing peers, large gap
//   MATRIX (O1): P1 → SnapCollecting. PASSES today AND post-DC-1.
// ===========================================================================

/// Class 6 (REQ-SNAP-003): a fresh node (h==0) still snaps via Route C.
#[test]
fn class6_fresh_bootstrap_snaps_via_route_c() {
    //
    // NOTE: `add_peer` auto-invokes `start_sync()` once the 3rd agreeing peer makes
    // the manager peer-ahead in Idle state (peers.rs:66-68). With local_height==0 that
    // admission already takes Route C (decision.rs:167), so the helper returns with the
    // snap already admitted — there is NO Idle precondition to assert. The explicit
    // `start_sync()` below is an idempotent no-op (guarded by `is_syncing`) kept for
    // determinism; the assertion reads the post-admission state either way.
    let mut mgr = mgr_with_agreeing_peers(0, 600, 3); // h==0, gap=600

    mgr.start_sync();

    // O1: local_height==0 admits snap regardless of the gap term (Route C).
    let snapped = matches!(mgr.pipeline_data, SyncPipelineData::SnapCollecting { .. });
    assert!(
        snapped,
        "Class 6 (REQ-SNAP-003): fresh node (h==0) must snap via Route C \
         (decision.rs:167 local_height==0 term); DC-1 must not touch this path"
    );
}

// ===========================================================================
// CLASS 7 — epoch-boundary replay of classes 1 & 5 (Failure Scenario 5).
// PASS-class variants → NOT ignored.
//
// CLASS 7 GAP: the SyncManager unit layer cannot simulate the true INC-I-139
// trigger — GetHeaders(canonical-start-hash) returning 0 headers AT an epoch
// boundary — because header generation / block_store lookups live at the
// node/block_store layer, not in SyncManager. What is reproducible at this layer
// is the RESULTING evidence shape (empty headers + gap) with heights aligned to
// an epoch boundary (h=36/64 = the INC-I-138 epoch-1 boundary framing). The
// FAIL-class variants (2/3/4) are NOT re-added here: their admission logic is
// gap/evidence-driven and epoch-position-invariant, so a boundary-aligned copy
// would be a redundant duplicate of the base ignored tests, not new coverage.
// ===========================================================================

/// Class 7a — epoch-boundary variant of Class 1 wedge (finality==local_tip at
/// an epoch-boundary height): still no snap below gap 50.
#[test]
fn class7_epoch_boundary_wedge_no_snap() {
    let mut coord = RecoveryCoordinator::new();
    for _ in 0..2 {
        coord.report(RecoveryEvidence::EmptyHeaders {
            peer: PeerId::random(),
            gap: 12,
        });
    }
    // local_tip=36 (epoch-1 boundary), finality pinned at the boundary.
    let mut c = ctx(36, 48, 5); // gap=12
    c.last_finality_height = Some(36);

    let action = coord.classify(&c);

    assert_ne!(
        action,
        RecoveryAction::SnapSync,
        "Class 7a: epoch-boundary wedge must NOT admit snap below gap 50"
    );
    assert_eq!(
        action,
        RecoveryAction::None,
        "Class 7a: finality==local_tip at the epoch boundary parks (no rollback/snap)"
    );
}

/// Class 7b — epoch-boundary variant of Class 5: gap=28 at the boundary still
/// refuses genesis-resync (regime guard intact across the epoch edge).
#[test]
fn class7_epoch_boundary_gap28_no_genesis_resync() {
    // local=36 is the epoch-1 boundary; network_tip=64 → gap=28.
    let mut mgr = mgr_with_agreeing_peers(36, 64, 5);
    let peer = *mgr.peers.keys().next().expect("5 peers inserted");
    mgr.fork.consecutive_empty_headers = 10;
    mgr.fork.use_height_based_headers = false;
    mgr.fork.height_fallback_attempted = false;
    mgr.confirmed_height_floor = 0;
    mgr.state = SyncState::Syncing {
        phase: SyncPhase::DownloadingHeaders,
        started_at: Instant::now(),
    };
    mgr.pipeline_data = SyncPipelineData::Headers {
        target_slot: 664,
        peer,
        headers_count: 0,
    };

    let _ = mgr.next_request();

    assert!(
        !mgr.fork.needs_genesis_resync,
        "Class 7b: gap=28 at the epoch boundary must NOT genesis-resync (regime guard intact)"
    );
}

// ===========================================================================
// CLASS 8 — B7 negative (REQ-SNAP-010). MUST PASS. NOT ignored.
//
// A minor-fork stall never satisfies HeightOffsetDetected admission. The raise
// site (cleanup.rs:668 `gap>=2 && blocks_recent`) is suppressed when no block
// applied recently (blocks_recent=false), and — as the reachable unit-level
// backstop — HeightOffsetDetected is NOT an emergency reason, so Gate 1 refuses
// it whenever a monotonic floor exists. DC-2 must NOT over-exempt it (only the
// forward-large-gap reasons are exempted).
//
// OUTPUT CONTRACT: fn request_genesis_resync(&mut self, reason) -> bool
//   O1: return bool
//   PATH P1: reason=HeightOffsetDetected, floor>0
//   MATRIX (O1): P1 → false (floor-gated). PASSES today AND must stay false post-DC-2.
// ===========================================================================

/// Class 8 (REQ-SNAP-010): HeightOffsetDetected is floor-gated — a minor-fork
/// stall with a confirmed floor never admits snap through this route.
#[test]
fn class8_b7_height_offset_floor_gated_refused() {
    let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    mgr.local_height = 100;
    mgr.network.network_tip_height = 110; // small gap (minor-fork stall)
    mgr.confirmed_height_floor = 100; // monotonic floor established
    mgr.consecutive_resync_count = 0;
    mgr.recovery_phase = RecoveryPhase::Normal;

    let honored = mgr.request_genesis_resync(RecoveryReason::HeightOffsetDetected { gap: 10 });

    // O1: HeightOffsetDetected is NOT emergency and NOT a forward-large-gap
    // reason → Gate 1 (floor>0) refuses. This must hold post-DC-2, which exempts
    // only CoordinatorSnapEscalation / StuckSyncLargeGap.
    assert!(
        !honored,
        "Class 8 (REQ-SNAP-010): HeightOffsetDetected with floor=100 must be REFUSED \
         by Gate 1 (floor-gated). DC-2 must not over-exempt it — only forward-large-gap \
         reasons are exempt."
    );
    assert!(
        !mgr.needs_genesis_resync(),
        "Class 8: refused request must not set needs_genesis_resync"
    );
}

// ===========================================================================
// CLASS 9 — DC-3 A1 attempts-limiter bypass (REQ-SNAP-001 / REQ-SNAP-002).
// FAILS-BY-DESIGN on HEAD; the M4 deletion of A1 lands in THIS milestone, so the
// final state must be green → NOT #[ignore].
//
// The A1 deep-fork snap redirect (dispatch.rs:105-117) zeroes `snap.attempts`
// (dispatch.rs:112) as a side effect. That reset defeats the 3-attempt limiter
// (production_gate.rs Gate 5, :745 refuses when attempts>=3): a node that has
// already burned 2 snap attempts has the counter wiped back to 0 every time it
// re-enters this block, so the limiter can never fire. After DC-3 deletes A1, the
// deep-fork empty-headers path (gap>50, empties>=10, floor>0) falls through to the
// gated B3 emergency funnel `request_genesis_resync(GenesisFallbackEmptyHeaders)`
// (dispatch.rs:157), which READS `snap.attempts` (Gate 5) but never RESETS it.
//
// OUTPUT CONTRACT: fn next_request(&mut self) [dispatch.rs:15]
//   (Headers branch, consecutive_empty_headers>=10, use_height_based_headers=false)
//   O1: self.snap.attempts — MUST be preserved across the deep-fork escalation
//   PATH P1: deep-fork empty-headers, gap>50, floor>0, empties>=10 → :96 → A1(:105)
//   INPUT PARTITION: gap=100 (>threshold 50), floor=100, empties=10, attempts=2
//   MATRIX (O1): P1 → attempts unchanged (2). FAILS today: A1 (:112) resets to 0.
// ===========================================================================

/// Class 9 (REQ-SNAP-001/002, DC-3): the deep-fork empty-headers escalation must
/// NOT zero `snap.attempts`. Today A1 (dispatch.rs:112) resets it to 0, defeating
/// the 3-attempt limiter (production_gate.rs Gate 5). Post-DC-3 the path falls
/// through to the gated B3 funnel which reads but never resets the counter.
#[test]
fn class9_a1_does_not_reset_snap_attempts() {
    // gap = 200 - 100 = 100 > snap.threshold(50); 3 peers → enough_peers=true.
    let mut mgr = mgr_with_agreeing_peers(100, 200, 3);
    let peer = *mgr.peers.keys().next().expect("3 peers inserted");
    mgr.confirmed_height_floor = 100; // floor>0 → A1's third condition holds.
    mgr.fork.use_height_based_headers = false; // reach :96, not the :72 fallback.
    mgr.fork.height_fallback_attempted = false;
    mgr.fork.consecutive_empty_headers = 10; // >=10 triggers the escalation block.
    mgr.snap.attempts = 2; // approaching the 3-attempt limit — the value under test.
    mgr.state = SyncState::Syncing {
        phase: SyncPhase::DownloadingHeaders,
        started_at: Instant::now(),
    };
    mgr.pipeline_data = SyncPipelineData::Headers {
        target_slot: 264,
        peer,
        headers_count: 0,
    };
    for s in mgr.peers.values_mut() {
        s.pending_request = None; // no in-flight request → reach the escalation.
    }

    let _ = mgr.next_request();

    // O1: snap.attempts must be preserved. A1 (dispatch.rs deep_fork_snap_redirect,
    // :112) zeroes it, defeating the 3-attempt limiter. Post-DC-3 the deep-fork
    // empty-headers path falls through to the gated B3 emergency funnel
    // request_genesis_resync(GenesisFallbackEmptyHeaders), which reads snap.attempts
    // (Gate 5) but never resets it.
    assert_eq!(
        mgr.snap.attempts, 2,
        "Class 9 (DC-3): deep-fork escalation zeroed snap.attempts (observed {}, expected 2). \
         A1 (dispatch.rs:112) resets the counter, defeating the 3-attempt limiter \
         (production_gate.rs Gate 5). Post-DC-3 the path must fall through to the gated \
         B3 funnel which reads but never resets snap.attempts.",
        mgr.snap.attempts
    );
}
