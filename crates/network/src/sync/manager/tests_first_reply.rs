// covers: manager/first_reply.rs manager/peers.rs (add_peer) sync_engine/response.rs (handle_response_to)
//! The first-reply latch: one bounded hold per connected session, ended by the
//! first non-stale header reply or by 30 s. No peer input extends it.
//
// OUTPUT CONTRACT: fn SyncManager::first_reply_holds(&mut self, now: Instant) -> bool
//   (with SyncManager::add_peer and SyncManager::handle_response_to as the writers)
//   O2 self.first_reply.spent | O2 self.first_reply.armed_at | O3 return
//   O6 INFO tracing lines: one at hold start, one at release, none per tick
// PATHS: P1 spent -> false | P2 peer not taller -> false, not armed |
//   P3 armed, under 30 s -> true | P4 elapsed >= 30 s -> false, spends |
//   P5 reset at the 0-to-1 peer branch | P6 header reply spends | P7 stale reply ignored
// INPUT PARTITIONS AND MATRIX (each row asserts return, spent, armed_at):
//   no peer yet                 -> P2 false, unspent, None
//   first connect, taller       -> P3 true, unspent, Some(now)
//   gap of 6 (first taller gap) -> P3 true, unspent, Some(now)
//   reply arrived               -> P6 then P1 false, spent, stays spent
//   reply before first tick     -> P6 then P1 false, spent, None
//   stale-epoch reply           -> P7 true, unspent; current-epoch reply then spends
//   non-header reply            -> true, unspent
//   snap-descent reply          -> P6 spent, on the other header route
//   30 s elapsed                -> P4 false, spent; 29 s -> P3 true, unspent, same armed_at
//   re-connect                  -> P5 then P3 true, unspent, Some(new now)
//   churn, one of two peers     -> P1 false, spent, same armed_at
//   peer not taller (gap 5)     -> P2 false, unspent, None, also at a later tick
//   not taller, then taller     -> P2 then P3 true, Some(now of the first taller tick)
//   below gate: not this path; `below_gate_keeps_old_peer_rule` in bins/node/tests/it
//     shows through try_produce_block that the latch is not read there
//   SEC churn each tick 60 s    -> 30 true ticks, then false, same armed_at
//   SEC rising status 60 s      -> 30 true ticks, then false, same armed_at
//   SEC best_height = u64::MAX  -> same bound, no panic; local u64::MAX -> no hold
//   O6: tracing-subscriber is already a dev-dependency, so the lines are captured here.
//     The message text is free, so the release by a reply counts against a control
//     manager that takes the same replies with no armed hold.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crypto::Hash;
use doli_core::FinalityCheckpoint;
use libp2p::PeerId;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

use super::types::{PendingRequest, SyncRequestId};
use crate::protocols::{SyncRequest, SyncResponse};
use crate::sync::manager::{SyncConfig, SyncManager, SyncPhase, SyncPipelineData, SyncState};

const LOCAL: u64 = 100;
const TALL: u64 = LOCAL + 20;
const NOT_TALLER: u64 = LOCAL + 5;
const BOUND: Duration = Duration::from_secs(30);

fn mgr_at(local: u64) -> SyncManager {
    let config = SyncConfig {
        inc_i_288_fork_choice_activation_height: 0,
        ..SyncConfig::default()
    };
    let mut m = SyncManager::new(config, Hash::ZERO);
    m.update_local_tip(local, crypto::hash::hash(b"local tip"), local as u32);
    m
}

fn mgr() -> SyncManager {
    mgr_at(LOCAL)
}

fn peer_at(m: &mut SyncManager, height: u64) -> PeerId {
    let peer = PeerId::random();
    let other = crypto::hash::hash(b"other branch");
    m.add_peer(peer, height, other, height as u32);
    peer
}

fn reply(m: &mut SyncManager, peer: PeerId) {
    m.handle_response(peer, SyncResponse::Headers(vec![]));
}

fn latch(m: &SyncManager) -> (bool, Option<Instant>) {
    (m.first_reply.spent, m.first_reply.armed_at)
}

/// Runs one check per simulated second over 0..=60 s; returns the seconds that held.
fn seconds_held(
    m: &mut SyncManager,
    t0: Instant,
    mut each_second: impl FnMut(&mut SyncManager, u64),
) -> Vec<u64> {
    (0..=60u64)
        .filter(|s| {
            each_second(m, *s);
            m.first_reply_holds(t0 + Duration::from_secs(*s))
        })
        .collect()
}

// REQ-PASK-003 — Decision: a failure means a node with no peer starts a hold, so a lone node waits on nobody.
#[test]
fn no_peer_yet_no_hold_no_arm() {
    let mut m = mgr();
    assert!(!m.first_reply_holds(Instant::now()), "no peer: no hold");
    assert_eq!(latch(&m), (false, None), "no peer: unspent, not armed");
}

// REQ-PASK-003 — Decision: a failure means a stale restart with a taller peer builds at once, before it can learn of the taller chain.
#[test]
fn first_connect_taller_holds_and_arms() {
    let mut m = mgr();
    peer_at(&mut m, TALL);
    let t0 = Instant::now();
    assert!(m.first_reply_holds(t0), "taller peer, no reply: hold");
    assert_eq!(latch(&m), (false, Some(t0)), "armed at the first tick");
}

// REQ-PASK-003 — Decision: a failure means the taller test is off by one against the old gap of 5, which moves the start of the hold.
#[test]
fn gap_of_six_is_the_first_taller_gap() {
    let mut m = mgr();
    peer_at(&mut m, NOT_TALLER + 1);
    let t0 = Instant::now();
    assert!(m.first_reply_holds(t0), "gap 6: hold");
    assert_eq!(latch(&m), (false, Some(t0)), "gap 6: armed");
}

// REQ-PASK-003 — Decision: a failure means the reply does not end the hold, so a producer waits 30 s after it can already build.
#[test]
fn reply_arrived_ends_hold_and_stays_spent() {
    let mut m = mgr();
    let peer = peer_at(&mut m, TALL);
    let t0 = Instant::now();
    assert!(m.first_reply_holds(t0), "fixture: holding");

    reply(&mut m, peer);

    assert_eq!(
        latch(&m),
        (true, Some(t0)),
        "the empty reply spends the latch"
    );
    assert!(!m.first_reply_holds(t0), "spent: no hold");
    assert!(
        !m.first_reply_holds(t0 + Duration::from_secs(1)),
        "spent: no hold at a later tick"
    );
    assert_eq!(latch(&m), (true, Some(t0)), "stays spent");
}

// REQ-PASK-003 — Decision: a failure means a reply that lands before the first tick is lost, which adds a useless 30 s hold on a healthy connect.
#[test]
fn reply_before_first_tick_gives_no_hold() {
    let mut m = mgr();
    let peer = peer_at(&mut m, TALL);
    reply(&mut m, peer);
    assert_eq!(latch(&m), (true, None), "spent before any tick");
    assert!(!m.first_reply_holds(Instant::now()), "no hold");
    assert_eq!(latch(&m), (true, None), "a spent latch never arms");
}

// REQ-PASK-003 — Decision: a failure means a discarded answer to an old sync request ends the hold, so the node builds with no fresh word from a peer.
#[test]
fn stale_epoch_reply_still_holds() {
    let mut m = mgr();
    let peer = peer_at(&mut m, TALL);
    let t0 = Instant::now();
    assert!(m.first_reply_holds(t0), "fixture: holding");

    let track = |m: &mut SyncManager, id: u64, epoch: u64| {
        let id = SyncRequestId::new(id);
        m.peers.get_mut(&peer).unwrap().pending_request = Some(id);
        let request = SyncRequest::GetHeaders {
            start_hash: Hash::ZERO,
            max_count: 1,
        };
        let sent_at = Instant::now();
        let tracked = PendingRequest {
            peer,
            request,
            sent_at,
            epoch,
        };
        m.pipeline.pending_requests.insert(id, tracked);
    };

    let old_epoch = m.pipeline.sync_epoch;
    m.pipeline.sync_epoch += 1;
    track(&mut m, 9_001, old_epoch);
    reply(&mut m, peer);
    assert_eq!(latch(&m), (false, Some(t0)), "stale reply: not spent");
    assert!(m.first_reply_holds(t0), "stale reply: still holds");

    let current = m.pipeline.sync_epoch;
    track(&mut m, 9_002, current);
    reply(&mut m, peer);
    assert_eq!(latch(&m), (true, Some(t0)), "control: current epoch spends");
}

// REQ-PASK-003 — Decision: a failure means a reply of another kind ends the hold, which the design keeps for header replies only.
#[test]
fn body_reply_does_not_end_hold() {
    let mut m = mgr();
    let peer = peer_at(&mut m, TALL);
    let t0 = Instant::now();
    assert!(m.first_reply_holds(t0), "fixture: holding");
    m.handle_response(peer, SyncResponse::Bodies(vec![]));
    assert_eq!(latch(&m), (false, Some(t0)), "bodies do not spend");
    assert!(m.first_reply_holds(t0), "still holds");
}

// REQ-PASK-003 — Decision: a failure means the release sits after the snap-descent route, so a reply on that route never ends the hold.
#[test]
fn snap_descent_reply_ends_hold() {
    let f = crypto::hash::hash(b"finalized");
    let anchor = crypto::hash::hash(b"anchor");
    let mut m = mgr_at(12);
    m.restore_finality(FinalityCheckpoint {
        block_hash: f,
        height: 10,
        slot: 10,
        attestation_weight: 1,
        total_weight: 1,
    });
    let peer = PeerId::random();
    m.add_peer(peer, 700, anchor, 700);
    m.pipeline_data = SyncPipelineData::SnapDownloading {
        target_hash: anchor,
        target_height: 700,
        quorum_root: Hash::ZERO,
        peer,
        alternate_peers: vec![],
    };
    m.state = SyncState::Syncing {
        phase: SyncPhase::SnapDownloading,
        started_at: Instant::now(),
    };
    m.peers.get_mut(&peer).unwrap().pending_request = None;
    assert!(m.next_request().is_some(), "fixture: the first proof batch");
    assert!(
        m.snap_descent_owns(peer),
        "fixture: the descent owns the peer"
    );
    let t0 = Instant::now();
    assert!(m.first_reply_holds(t0), "fixture: holding");

    reply(&mut m, peer);

    assert_eq!(latch(&m), (true, Some(t0)), "both header routes spend");
}

// REQ-PASK-003 — Decision: a failure means a peer that never replies stops this node's builds past the bound, which is the stall this fix removes.
#[test]
fn thirty_seconds_elapsed_ends_hold_and_spends() {
    let mut m = mgr();
    peer_at(&mut m, TALL);
    let now = Instant::now();
    let armed = now - BOUND;
    m.first_reply.armed_at = Some(armed);
    assert!(!m.first_reply_holds(now), "30 s: no hold");
    assert_eq!(latch(&m), (true, Some(armed)), "30 s: spent");
    assert!(!m.first_reply_holds(now), "stays released");
}

// REQ-PASK-003 — Decision: a failure means the hold ends before its bound, which cuts the time a stale restart has to hear from a peer.
#[test]
fn twenty_nine_seconds_elapsed_still_holds() {
    let mut m = mgr();
    peer_at(&mut m, TALL);
    let now = Instant::now();
    let armed = now - Duration::from_secs(29);
    m.first_reply.armed_at = Some(armed);
    assert!(m.first_reply_holds(now), "29 s: hold");
    assert_eq!(latch(&m), (false, Some(armed)), "29 s: the arm time stays");
}

// REQ-PASK-003 — Decision: a failure means a node that lost all peers builds at once on reconnect, with no chance to hear of a taller chain.
#[test]
fn reconnect_after_losing_all_peers_holds_again() {
    let mut m = mgr();
    let a = peer_at(&mut m, TALL);
    let b = peer_at(&mut m, TALL);
    reply(&mut m, a);
    assert!(latch(&m).0, "fixture: spent");

    m.remove_peer(&a);
    m.remove_peer(&b);
    assert!(latch(&m).0, "losing the peers alone does not reset");
    peer_at(&mut m, TALL);
    assert_eq!(latch(&m), (false, None), "0-to-1 peer: reset");

    let t1 = Instant::now();
    assert!(m.first_reply_holds(t1), "new session: hold");
    assert_eq!(latch(&m), (false, Some(t1)), "new session: armed anew");
}

// REQ-PASK-003 — Decision: a failure means one peer that reconnects can stop this node's builds again, which hands a stop back to a peer.
#[test]
fn churn_of_one_of_two_peers_gives_no_hold() {
    let mut m = mgr();
    let a = peer_at(&mut m, TALL);
    let b = peer_at(&mut m, TALL);
    let t0 = Instant::now();
    assert!(m.first_reply_holds(t0), "fixture: holding");
    reply(&mut m, a);

    m.remove_peer(&b);
    m.add_peer(b, TALL, crypto::hash::hash(b"other branch"), TALL as u32);

    assert!(!m.first_reply_holds(t0), "churn after spend: no hold");
    assert_eq!(latch(&m), (true, Some(t0)), "churn: no reset, no re-arm");
}

// REQ-PASK-003 — Decision: a failure means a level peer spends or arms the latch, so it is gone when the real taller peer connects.
#[test]
fn peer_not_taller_gives_no_hold_and_no_spend() {
    let mut m = mgr();
    peer_at(&mut m, NOT_TALLER);
    let t0 = Instant::now();
    assert!(!m.first_reply_holds(t0), "gap 5: no hold");
    assert_eq!(latch(&m), (false, None), "gap 5: unspent, not armed");
    assert!(!m.first_reply_holds(t0 + BOUND), "gap 5, later: no hold");
    assert_eq!(latch(&m), (false, None), "gap 5: the bound does not count");
}

// REQ-PASK-003 — Decision: a failure means the wait for a taller peer is lost after a level peer connects first, which is the co-located stale-peer shape.
#[test]
fn not_taller_then_taller_holds() {
    let mut m = mgr();
    peer_at(&mut m, NOT_TALLER);
    let t0 = Instant::now();
    assert!(!m.first_reply_holds(t0), "fixture: gap 5");
    peer_at(&mut m, TALL);
    let t1 = t0 + BOUND;
    assert!(m.first_reply_holds(t1), "a taller peer now: hold");
    assert_eq!(
        latch(&m),
        (false, Some(t1)),
        "armed at the first taller tick"
    );
}

// REQ-PASK-SEC-001 — Decision: a failure means a peer that reconnects on each tick keeps this node from building, which is a stop any peer can drive.
#[test]
fn churn_never_extends_hold() {
    let mut m = mgr();
    peer_at(&mut m, TALL);
    let churner = peer_at(&mut m, TALL);
    let t0 = Instant::now();

    let held = seconds_held(&mut m, t0, |m, _| {
        m.remove_peer(&churner);
        m.add_peer(churner, TALL, Hash::ZERO, TALL as u32);
    });

    assert_eq!(held, (0..30).collect::<Vec<u64>>(), "one hold of 30 s");
    assert_eq!(latch(&m), (true, Some(t0)), "spent, armed once");
}

// REQ-PASK-SEC-001 — Decision: a failure means a peer that keeps claiming a higher height keeps this node from building past the bound.
#[test]
fn rising_status_never_extends_hold() {
    let mut m = mgr();
    let peer = peer_at(&mut m, TALL);
    let t0 = Instant::now();

    let held = seconds_held(&mut m, t0, |m, s| {
        let rising = TALL + s;
        if s % 2 == 0 {
            m.update_peer(peer, rising, Hash::ZERO, rising as u32);
        } else {
            m.add_peer(peer, rising, Hash::ZERO, rising as u32);
        }
    });

    assert_eq!(held, (0..30).collect::<Vec<u64>>(), "one hold of 30 s");
    assert_eq!(latch(&m), (true, Some(t0)), "spent, armed once");
}

// REQ-PASK-SEC-002 — Decision: a failure means one forged status height panics the node or removes the bound on the hold.
#[test]
fn peer_height_u64_max_is_bounded() {
    let mut m = mgr();
    peer_at(&mut m, u64::MAX);
    let t0 = Instant::now();
    assert!(m.first_reply_holds(t0), "u64::MAX: taller, hold");
    assert!(
        m.first_reply_holds(t0 + Duration::from_secs(29)),
        "29 s: hold"
    );
    assert!(!m.first_reply_holds(t0 + BOUND), "30 s: no hold");
    assert_eq!(latch(&m), (true, Some(t0)), "u64::MAX: same bound");

    let mut top = mgr_at(u64::MAX);
    peer_at(&mut top, u64::MAX);
    assert!(
        !top.first_reply_holds(t0),
        "local u64::MAX: nothing is taller"
    );
    assert_eq!(
        latch(&top),
        (false, None),
        "local u64::MAX: no arm, no overflow"
    );
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<String>>>);

struct Msg(String);

impl tracing::field::Visit for Msg {
    fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
        if f.name() == "message" {
            self.0 = format!("{v:?}");
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for Logs {
    fn on_event(&self, e: &tracing::Event<'_>, _: Context<'_, S>) {
        if *e.metadata().level() == tracing::Level::INFO {
            let mut m = Msg(String::new());
            e.record(&mut m);
            self.0.lock().unwrap().push(m.0);
        }
    }
}

/// INFO lines emitted while `f` runs on this thread.
fn info_lines(f: impl FnOnce()) -> Vec<String> {
    // A second live dispatcher keeps a callsite from caching `never` on other threads.
    static KEEP: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
    KEEP.get_or_init(|| tracing::Dispatch::new(tracing_subscriber::registry()));
    let logs = Logs::default();
    {
        let _g =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
        f();
    }
    let lines = logs.0.lock().unwrap().clone();
    lines
}

// REQ-PASK-009 — Decision: a failure means the hold logs per tick and floods INFO, or an operator cannot see why a build waited and why it resumed.
#[test]
fn hold_logs_once_at_start_and_once_at_bound_release() {
    let mut m = mgr();
    peer_at(&mut m, TALL);
    let t0 = Instant::now();

    let start = info_lines(|| assert!(m.first_reply_holds(t0)));
    assert_eq!(start.len(), 1, "hold start: one line, got {start:?}");

    let ticks = info_lines(|| {
        for s in 1..30 {
            assert!(m.first_reply_holds(t0 + Duration::from_secs(s)));
        }
    });
    assert_eq!(ticks, Vec::<String>::new(), "no line per tick");

    let release = info_lines(|| assert!(!m.first_reply_holds(t0 + BOUND)));
    assert_eq!(release.len(), 1, "release: one line, got {release:?}");

    let after = info_lines(|| assert!(!m.first_reply_holds(t0 + BOUND)));
    assert_eq!(after, Vec::<String>::new(), "after release: no line");
}

// REQ-PASK-009 — Decision: a failure means the release by a reply is silent or repeats, so the log does not show what ended the hold.
#[test]
fn reply_release_logs_once_and_only_when_armed() {
    let replies = |arm: bool| {
        let mut m = mgr();
        let peer = peer_at(&mut m, TALL);
        if arm {
            assert!(m.first_reply_holds(Instant::now()), "fixture: holding");
        }
        let first = info_lines(|| reply(&mut m, peer)).len();
        let second = info_lines(|| reply(&mut m, peer)).len();
        (first, second)
    };
    let (armed_first, armed_second) = replies(true);
    let (idle_first, idle_second) = replies(false);
    assert_eq!(armed_first, idle_first + 1, "armed: one release line");
    assert_eq!(armed_second, idle_second, "second reply: no release line");
}

// REQ-PASK-009 — Decision: a failure means a tick with no hold logs a line, which adds an INFO line each second on a healthy node.
#[test]
fn tick_without_hold_logs_nothing() {
    let mut m = mgr();
    peer_at(&mut m, NOT_TALLER);
    let t0 = Instant::now();
    let lines = info_lines(|| assert!(!m.first_reply_holds(t0)));
    assert_eq!(lines, Vec::<String>::new(), "peer not taller: no line");
}
