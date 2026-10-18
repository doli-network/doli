// INC-I-235 R-3 (SEC-LOGIC-002) — a snap-disabled sync manager (the archive / relay role)
// that is far behind on the CANONICAL chain must not park forever after 10 empty header
// replies while peers that are ahead are present. The peers lack the requested range, so
// the replies are empty although our tip is canonical (no fork).
//
// covers: dispatch.rs, response.rs, recovery.rs, types.rs (sync manager)
//
// OUTPUT CONTRACT: fn SyncManager::next_request(&mut self) -> Option<(PeerId, SyncRequest)>
//   O1: mutable params — none.
//   O2: self.state + self.pipeline_data (the park writes Idle, which clears the pipeline);
//       self.fork.use_height_based_headers (cleared on the height-based branch);
//       self.pipeline.pending_requests + the peer's pending_request (every dispatch);
//       self.snap.threshold / self.snap.attempts — never written;
//       self.fork.parked_header_retry_at / parked_header_retries (written on P4a only).
//   O3: return — Some((peer, GetHeaders | GetHeadersByHeight)) or None.
// OUTPUT CONTRACT: fn SyncManager::handle_response(&mut self, peer, SyncResponse::Headers(vec![]))
//   (the feeder) O2: self.fork.consecutive_empty_headers += 1; use_height_based_headers at a
//   count < 3; stuck_fork_signal at a count >= 3; EmptyHeaders evidence; state Idle.
//   O3: the returned Vec<Block> is empty for a headers reply (not asserted: no decision).
// PATHS (next_request, Headers arm):
//   P1: use_height_based_headers            -> GetHeadersByHeight(local_height)
//   P2: empties < 10                         -> GetHeaders(local_hash)
//   P3: empties >= 10, gap <= 3              -> counter reset, Idle, None (gossip wait)
//   P4: empties >= 10, gap > 3               -> Idle, None (park, awaits the coordinator)
//   P4a: P4 + snap disabled, gap >= SNAP_SYNC_GAP_MIN, retry due
//                                            -> GetHeadersByHeight(local) to the next peer
//   P4b: P4a but the retry is not due (< 30 s) -> Idle, None
//   P5: the peer has a request in flight     -> None
// INPUT PARTITIONS:
//   I1: feed — local 731_553, 5 agreeing peers at 734_838 (gap 3_285), snap disabled,
//       Normal phase; every header request gets an empty reply until the count is 10.
//   I2: ticks — the I1 manager, peers still ahead, no apply for 400 s; 40 periodic ticks
//       (~1 s each), the coordinator cooldown cleared every 30 ticks (models 30 s).
//   I3: the I1 manager; retry, empty reply, same-instant calls, then the timer 31 s old.
//   I4: snap ENABLED, gap 3_285, empties 10, Headers pipeline.
//   I5: snap disabled, gap 400 (< SNAP_SYNC_GAP_MIN), empties 10, Headers pipeline.
//   I6: I5 shape at gap 3_285; the first peer in the rotation has a request in flight
//       (the fork-walk peer), the Headers pipeline peer is another, idle peer.
//   I7: I6 shape; (a) every peer ahead is header-blacklisted; (b) the pipeline peer is
//       behind us and every peer ahead has a request in flight.
// MATRIX: 3 outputs x 7 partitions
//   I1: O3(a header request on every feed step, P1/P2)✓ O2(count reaches 10)✓
//       O2(snap threshold MAX, attempts 0)✓
//   I2: O3(>= 1 GetHeaders | GetHeadersByHeight in 40 ticks: P4 may not hold forever)✓
//       O3(no snap request: GetStateRoot / Snapshot / Manifest / Chunk)✓
//       O2(snap attempts 0, threshold MAX, pipeline never SnapCollecting)✓
//       coordinator action never SnapSync✓
//   I3: O3(P4a: ByHeight(local) to peer A)✓ O3(P4b: no header request)✓
//       O3(P4a again: a peer other than A)✓ O2(empties >= 10, snap untouched, retry_at set)✓
//   I4: O3(None)✓ O2(Idle, retry_at None, no request registered)✓
//   I5: O3(None)✓ O2(Idle, retry_at None, no request registered)✓
//   I6: O3(P4a: ByHeight(local) to a peer that is not the busy one)✓ O2(retry_at set,
//       empties >= 10)✓
//   I7: O3(None, a and b)✓ O2(retry_at None, empties >= 10)✓
//   O1 has no cells (no mutable parameters). P3 (gap <= 3) and P5 (in flight) are out of
//   this partition; other suites cover them.

use std::time::{Duration, Instant};

use libp2p::PeerId;

use crate::protocols::{SyncRequest, SyncResponse};
use crate::sync::manager::recovery::RecoveryAction;
use crate::sync::manager::tests_inc_i139::mgr_with_agreeing_peers;
use crate::sync::manager::{
    RecoveryPhase, RecoveryReason, SyncManager, SyncPhase, SyncPipelineData, SyncRequestId,
    SyncState,
};

const LOCAL_H: u64 = 731_553;
const PEER_H: u64 = 734_838;
const GAP: u64 = PEER_H - LOCAL_H;
const AGREEING_PEERS: usize = 5;
const PARK_EMPTIES: u32 = 10;
const NO_APPLY_SECS: u64 = 400;
const TICKS: u32 = 40;
const COOLDOWN_TICKS: u32 = 30;

fn is_header_request(req: &SyncRequest) -> bool {
    matches!(
        req,
        SyncRequest::GetHeaders { .. } | SyncRequest::GetHeadersByHeight { .. }
    )
}

/// One periodic.rs tick as the sync manager sees it, in periodic.rs order: cleanup,
/// stuck-fork consume, evidence reports, coordinator, the sync-manager side of the arm,
/// start_sync, snap batch, next_request. The ShallowRollback / SiblingFetch / Wedged arms
/// return early in periodic.rs, so those ticks take no request.
fn periodic_tick(mgr: &mut SyncManager, tick: u32) -> (RecoveryAction, Vec<SyncRequest>) {
    mgr.cleanup();
    if let Some(alert) = mgr.consume_stuck_fork_signal() {
        mgr.report_stuck_fork(alert.best_peer_height.saturating_sub(alert.local_height));
    }
    let gap = mgr.network_tip_height().saturating_sub(mgr.local_tip().0);
    if mgr.consecutive_empty_headers() >= 3 {
        mgr.report_empty_headers(PeerId::random(), gap);
    }
    let last_applied = mgr.last_block_applied_secs();
    if last_applied >= 30 {
        mgr.report_stale_tip(last_applied, gap);
    }
    if tick.is_multiple_of(COOLDOWN_TICKS) {
        mgr.recovery.clear_cooldown_for_test();
    }
    let (action, _) = mgr.classify_and_dispatch(0);
    if matches!(
        action,
        RecoveryAction::SnapSync | RecoveryAction::GenesisResync
    ) {
        let _ = mgr.request_genesis_resync(RecoveryReason::CoordinatorSnapEscalation);
    }
    if matches!(
        action,
        RecoveryAction::ShallowRollback { .. }
            | RecoveryAction::SiblingFetch { .. }
            | RecoveryAction::Wedged { .. }
    ) {
        return (action, Vec::new());
    }
    if matches!(mgr.state, SyncState::Idle) && mgr.should_sync() {
        mgr.start_sync();
    }
    let mut sent: Vec<SyncRequest> = mgr
        .next_snap_requests()
        .into_iter()
        .map(|(_, r)| r)
        .collect();
    if let Some((_, req)) = mgr.next_request() {
        sent.push(req);
    }
    (action, sent)
}

// T5 — REQ-ARCHSYNC-003 — Decision: a failure reveals a snap-disabled node far behind on the canonical chain parks forever after 10 empty header replies, so an archive node never refills its history from peers that are ahead.
#[test]
fn snap_disabled_far_behind_canonical_empty_headers_never_parks() {
    let mut mgr = mgr_with_agreeing_peers(LOCAL_H, PEER_H, AGREEING_PEERS);
    mgr.disable_snap_sync();
    mgr.recovery_phase = RecoveryPhase::Normal;
    mgr.snap.attempts = 0;
    assert_eq!(
        mgr.network_tip_height() - LOCAL_H,
        GAP,
        "precondition: incident gap"
    );

    // I1: answer every header request with an empty reply until the park threshold.
    mgr.start_sync();
    let mut fed = 0u32;
    let mut feed_kinds: Vec<&'static str> = Vec::new();
    while mgr.fork.consecutive_empty_headers < PARK_EMPTIES && fed < 3 * PARK_EMPTIES {
        if matches!(mgr.state, SyncState::Idle) && mgr.should_sync() {
            mgr.start_sync();
        }
        let (peer, req) = mgr.next_request().unwrap_or_else(|| {
            panic!(
                "precondition: a header request before empty reply #{} (state {:?}, empties {})",
                fed + 1,
                mgr.state,
                mgr.fork.consecutive_empty_headers
            )
        });
        assert!(
            is_header_request(&req),
            "precondition: feed step {} must be a header request, got {req:?}",
            fed + 1
        );
        feed_kinds.push(match req {
            SyncRequest::GetHeadersByHeight { .. } => "ByHeight",
            _ => "GetHeaders",
        });
        mgr.handle_response(peer, SyncResponse::Headers(Vec::new()));
        fed += 1;
    }
    assert!(
        mgr.fork.consecutive_empty_headers >= PARK_EMPTIES,
        "precondition: {fed} empty replies must reach the park count {PARK_EMPTIES}, got {} \
         (requests {feed_kinds:?})",
        mgr.fork.consecutive_empty_headers
    );
    assert_eq!(mgr.snap.threshold, u64::MAX, "precondition: snap disabled");
    assert_eq!(mgr.snap.attempts, 0, "precondition: no snap attempt spent");

    // I2: the node's periodic loop with peers still ahead and no apply for 400 s.
    mgr.network.last_block_applied = Instant::now() - Duration::from_secs(NO_APPLY_SECS);
    let mut actions: Vec<RecoveryAction> = Vec::new();
    let mut sent: Vec<SyncRequest> = Vec::new();
    let mut snap_collecting_seen = false;
    for tick in 0..TICKS {
        let (action, reqs) = periodic_tick(&mut mgr, tick);
        actions.push(action);
        snap_collecting_seen |=
            matches!(mgr.pipeline_data, SyncPipelineData::SnapCollecting { .. });
        sent.extend(reqs);
    }

    let snap_requests = sent
        .iter()
        .filter(|r| {
            matches!(
                r,
                SyncRequest::GetStateRoot { .. }
                    | SyncRequest::GetStateSnapshot { .. }
                    | SyncRequest::GetStateManifest { .. }
                    | SyncRequest::GetStateChunk { .. }
            )
        })
        .count();
    assert_eq!(
        snap_requests, 0,
        "a snap-disabled node must send no snap request: {sent:?}"
    );
    assert!(
        !actions.contains(&RecoveryAction::SnapSync),
        "the coordinator must not order SnapSync: {actions:?}"
    );
    assert!(
        !snap_collecting_seen,
        "the pipeline must never enter SnapCollecting"
    );
    assert_eq!(mgr.snap.attempts, 0, "no snap attempt may be spent");
    assert_eq!(mgr.snap.threshold, u64::MAX, "snap must stay disabled");

    let header_requests = sent.iter().filter(|r| is_header_request(r)).count();
    assert!(
        header_requests >= 1,
        "SEC-LOGIC-002: a snap-disabled node 3285 behind on the canonical chain parks forever \
         after 10 empty header replies; no header request is sent while peers that are ahead \
         are present. [fed {fed} empties {feed_kinds:?}; {TICKS} ticks sent {sent:?}; \
         actions {actions:?}; final state {:?}, empties {}, needs_genesis_resync {}]",
        mgr.state,
        mgr.fork.consecutive_empty_headers,
        mgr.fork.needs_genesis_resync
    );
}

/// Feed empty header replies to a snap-disabled I1 manager until the park count.
fn snap_disabled_parked_mgr() -> SyncManager {
    let mut mgr = mgr_with_agreeing_peers(LOCAL_H, PEER_H, AGREEING_PEERS);
    mgr.disable_snap_sync();
    mgr.recovery_phase = RecoveryPhase::Normal;
    mgr.snap.attempts = 0;
    mgr.start_sync();
    for _ in 0..3 * PARK_EMPTIES {
        if mgr.fork.consecutive_empty_headers >= PARK_EMPTIES {
            break;
        }
        if matches!(mgr.state, SyncState::Idle) && mgr.should_sync() {
            mgr.start_sync();
        }
        let (peer, req) = mgr.next_request().expect("precondition: a feed request");
        assert!(is_header_request(&req), "precondition: feed {req:?}");
        mgr.handle_response(peer, SyncResponse::Headers(Vec::new()));
    }
    assert!(
        mgr.fork.consecutive_empty_headers >= PARK_EMPTIES,
        "precondition: the park count is reached"
    );
    mgr
}

/// One dispatch step: start_sync when idle (as periodic.rs does), then next_request.
fn dispatch(mgr: &mut SyncManager) -> Option<(PeerId, SyncRequest)> {
    if matches!(mgr.state, SyncState::Idle) && mgr.should_sync() {
        mgr.start_sync();
    }
    mgr.next_request()
}

/// A manager with the empties count already at the park threshold, in the Headers pipeline.
fn headers_parked_mgr(peer_h: u64, snap_disabled: bool) -> SyncManager {
    let mut mgr = mgr_with_agreeing_peers(LOCAL_H, peer_h, AGREEING_PEERS);
    if snap_disabled {
        mgr.disable_snap_sync();
    }
    mgr.recovery_phase = RecoveryPhase::Normal;
    let peer = *mgr.peers.keys().next().expect("peers inserted");
    mgr.state = SyncState::Syncing {
        phase: SyncPhase::DownloadingHeaders,
        started_at: Instant::now(),
    };
    mgr.pipeline_data = SyncPipelineData::Headers {
        target_slot: peer_h as u32,
        peer,
        headers_count: 0,
    };
    mgr.fork.use_height_based_headers = false;
    mgr.fork.consecutive_empty_headers = PARK_EMPTIES;
    mgr
}

fn assert_parked(mgr: &SyncManager, got: Option<(PeerId, SyncRequest)>, case: &str) {
    assert!(got.is_none(), "{case}: the park must hold, got {got:?}");
    assert!(
        matches!(mgr.state, SyncState::Idle),
        "{case}: the park writes Idle, got {:?}",
        mgr.state
    );
    assert!(
        mgr.fork.parked_header_retry_at.is_none(),
        "{case}: no parked retry may be armed"
    );
    assert!(
        mgr.pipeline.pending_requests.is_empty(),
        "{case}: no request may be registered"
    );
    assert!(
        mgr.fork.consecutive_empty_headers >= PARK_EMPTIES,
        "{case}: the park must not reset the counter (INV-SYNC-011)"
    );
}

// T6 — REQ-ARCHSYNC-003 — Decision: a failure reveals the parked header retry of a snap-disabled node is not rate limited to one per 30 s, or keeps asking the same peer that already answered empty.
#[test]
fn parked_snap_disabled_retry_is_rate_limited_and_rotates_peers() {
    let mut mgr = snap_disabled_parked_mgr();

    let (peer_a, req) = dispatch(&mut mgr).expect("P4a: the first parked dispatch retries");
    assert!(
        matches!(req, SyncRequest::GetHeadersByHeight { start_height, .. } if start_height == LOCAL_H),
        "P4a: a GetHeadersByHeight(local tip) retry, got {req:?}"
    );
    assert!(
        mgr.peers[&peer_a].best_height > LOCAL_H,
        "P4a: the peer is ahead"
    );
    assert!(
        mgr.fork.parked_header_retry_at.is_some(),
        "P4a: retry time armed"
    );
    mgr.handle_response(peer_a, SyncResponse::Headers(Vec::new()));

    for step in 0..3 {
        let got = dispatch(&mut mgr);
        assert!(
            !got.as_ref().is_some_and(|(_, r)| is_header_request(r)),
            "P4b: no header retry inside 30 s (step {step}), got {got:?}"
        );
    }

    mgr.fork.parked_header_retry_at = Some(Instant::now() - Duration::from_secs(31));
    let (peer_b, req) = dispatch(&mut mgr).expect("P4a: the retry is due again after 31 s");
    assert!(
        matches!(req, SyncRequest::GetHeadersByHeight { .. }),
        "P4a: a height-based retry, got {req:?}"
    );
    assert_ne!(peer_b, peer_a, "the second retry must ask another peer");

    assert!(
        mgr.fork.consecutive_empty_headers >= PARK_EMPTIES,
        "the retry must not reset the counter (INV-SYNC-011)"
    );
    assert_eq!(mgr.snap.threshold, u64::MAX, "snap must stay disabled");
    assert_eq!(mgr.snap.attempts, 0, "no snap attempt may be spent");
    assert!(!matches!(
        mgr.pipeline_data,
        SyncPipelineData::SnapCollecting { .. }
    ));
}

// T7 — REQ-ARCHSYNC-003 — Decision: a failure reveals the parked header retry also fires on a snap-enabled node, whose exit from the park is coordinator Rule 2 (snap sync).
#[test]
fn parked_snap_enabled_far_behind_still_parks() {
    let mut mgr = headers_parked_mgr(PEER_H, false);
    assert!(!mgr.recovery.snap_disabled(), "precondition: snap enabled");
    let got = mgr.next_request();
    assert_parked(&mgr, got, "snap enabled, gap 3285");
}

// T8 — REQ-ARCHSYNC-003 — Decision: a failure reveals the parked header retry fires below SNAP_SYNC_GAP_MIN, widening R-3 beyond the far-behind regime it is scoped to.
#[test]
fn parked_snap_disabled_below_snap_gap_still_parks() {
    let mut mgr = headers_parked_mgr(LOCAL_H + 400, true);
    assert!(mgr.recovery.snap_disabled(), "precondition: snap disabled");
    let got = mgr.next_request();
    assert_parked(&mgr, got, "snap disabled, gap 400");
}

// T9 — REQ-ARCHSYNC-003 — Decision: a failure reveals the parked header retry waits on a peer that already has a request in flight (the fork-walk peer keeps one GetBlockByHash outstanding, dispatch.rs:21-27), so the retry stalls for the whole walk.
#[test]
fn parked_retry_skips_a_peer_busy_with_the_fork_walk() {
    let mut mgr = headers_parked_mgr(PEER_H, true);
    let mut ids: Vec<PeerId> = mgr.peers.keys().copied().collect();
    ids.sort_unstable();
    let busy = ids[0]; // the first peer in the retry rotation
    let header_peer = ids[ids.len() - 1];
    if let SyncPipelineData::Headers { ref mut peer, .. } = mgr.pipeline_data {
        *peer = header_peer;
    }
    mgr.peers.get_mut(&busy).expect("peer").pending_request = Some(SyncRequestId::new(9_999));

    let got = mgr.next_request();
    let (peer, req) = got.expect(
        "I6: the retry must go to another peer when the first peer in the rotation is busy; \
         None means it waits on the busy (fork-walk) peer",
    );
    assert_ne!(peer, busy, "I6: never the busy peer");
    assert!(
        matches!(req, SyncRequest::GetHeadersByHeight { start_height, .. } if start_height == LOCAL_H),
        "I6: a GetHeadersByHeight(local tip) retry, got {req:?}"
    );
    assert!(
        mgr.fork.parked_header_retry_at.is_some(),
        "I6: retry time armed"
    );
    assert!(
        mgr.fork.consecutive_empty_headers >= PARK_EMPTIES,
        "I6: the retry must not reset the counter (INV-SYNC-011)"
    );
}

// T10 — REQ-ARCHSYNC-003 — Decision: a failure reveals the parked retry sends to a header-blacklisted peer, or to a peer that already has a request in flight, when no other peer ahead can take it.
#[test]
fn parked_retry_holds_when_no_peer_ahead_can_take_it() {
    // I7a: every peer ahead is header-blacklisted -> no eligible peer.
    let mut mgr = headers_parked_mgr(PEER_H, true);
    let ids: Vec<PeerId> = mgr.peers.keys().copied().collect();
    for pid in &ids {
        mgr.fork
            .header_blacklisted_peers
            .insert(*pid, Instant::now());
    }
    let got = mgr.next_request();
    assert_parked(&mgr, got, "I7a: all peers ahead blacklisted");

    // I7b: the Headers pipeline peer is behind us (idle); every peer ahead is busy.
    let mut mgr = headers_parked_mgr(PEER_H, true);
    let mut ids: Vec<PeerId> = mgr.peers.keys().copied().collect();
    ids.sort_unstable();
    let behind = ids[0];
    mgr.peers.get_mut(&behind).expect("peer").best_height = LOCAL_H - 1;
    if let SyncPipelineData::Headers { ref mut peer, .. } = mgr.pipeline_data {
        *peer = behind;
    }
    for (n, pid) in ids.iter().skip(1).enumerate() {
        mgr.peers.get_mut(pid).expect("peer").pending_request =
            Some(SyncRequestId::new(9_000 + n as u64));
    }
    let got = mgr.next_request();
    assert!(got.is_none(), "I7b: no idle peer ahead, got {got:?}");
    assert!(
        mgr.fork.parked_header_retry_at.is_none(),
        "I7b: no parked retry may be armed"
    );
    assert!(
        mgr.fork.consecutive_empty_headers >= PARK_EMPTIES,
        "I7b: the park must not reset the counter (INV-SYNC-011)"
    );
}
