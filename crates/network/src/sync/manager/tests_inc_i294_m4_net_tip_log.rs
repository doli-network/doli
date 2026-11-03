// covers: manager/peers.rs (add_peer, update_peer, remove_peer, note_orphan_gossip_block, update_network_tip_height)
//! INC-I-294 M-4: every peer-sourced write of network_tip_height that changes the value
//! logs one INFO `[NET_TIP] src=status|peer_removed|orphan_guess peer=<id|-> height= prev=`.
//
// OUTPUT CONTRACT: SyncManager::{add_peer, update_peer, remove_peer, note_orphan_gossip_block,
//   update_network_tip_height}(&mut self, ..)
//   O2 self.network.network_tip_height (read via network_tip().0)
//   O6 INFO tracing events containing `[NET_TIP]`
// PATHS: P1 add_peer | P2 update_peer | P3 remove_peer | P4 note_orphan_gossip_block | P5 applied block
// MATRIX
//   P1 raise -> 1 line src=status peer=<p> height=<h> prev=<old> | P1 equal/lower -> 0 lines
//   P2 raise -> 1 line src=status | P2 equal/lower -> 0 lines
//   P3 lowers -> 1 line src=peer_removed height=<new> prev=<old> | P3 unchanged -> 0 lines
//   P4 below gate raise -> 1 line src=orphan_guess peer=- | P4 above gate -> 0 lines, tip unchanged
//   P4 below gate equal -> 0 lines | P5 raise -> 0 lines, tip raised
// INPUT PARTITIONS: delta raise | equal | lower; gate local < gate | local >= gate

use std::sync::{Arc, Mutex};

use crypto::Hash;
use libp2p::PeerId;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

use crate::sync::manager::{SyncConfig, SyncManager};

const GATE: u64 = 1_000;
const TAG: &str = "[NET_TIP]";

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
        let mut m = Msg(String::new());
        e.record(&mut m);
        let line = format!("{} {}", e.metadata().level(), m.0);
        self.0.lock().unwrap().push(line);
    }
}

/// A second live dispatcher keeps tracing-core out of its one-dispatcher mode, where an
/// uncaptured thread's first hit of a callsite caches `never` for every thread.
fn keep_interest_rebuilt() {
    static KEEP: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
    KEEP.get_or_init(|| tracing::Dispatch::new(tracing_subscriber::registry()));
}

/// `[NET_TIP]` lines emitted while `f` runs on this thread.
fn net_tip_lines(f: impl FnOnce()) -> Vec<String> {
    let logs = Logs::default();
    {
        keep_interest_rebuilt();
        let _g =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
        f();
    }
    let all = logs.0.lock().unwrap().clone();
    all.into_iter().filter(|l| l.contains(TAG)).collect()
}

fn mgr(local: u64) -> SyncManager {
    let config = SyncConfig {
        inc_i_288_fork_choice_activation_height: GATE,
        ..SyncConfig::default()
    };
    let mut m = SyncManager::new(config, Hash::ZERO);
    m.update_local_tip(local, crypto::hash::hash(b"m4 local"), local as u32);
    m
}

fn h(tag: &[u8]) -> Hash {
    crypto::hash::hash(tag)
}

fn tip(m: &SyncManager) -> u64 {
    m.network_tip().0
}

fn one_info(lines: &[String], want: &str, case: &str) {
    assert_eq!(
        lines.len(),
        1,
        "{case}: exactly one {TAG} line expected; got {lines:?}"
    );
    assert!(
        lines[0].starts_with("INFO "),
        "{case}: {TAG} must be INFO; got {:?}",
        lines[0]
    );
    assert!(
        lines[0].contains(want),
        "{case}: want {want:?} in {:?}",
        lines[0]
    );
}

// INC-I-294 M-4 (Must) — Decision: a failure means a peer status that moves the network tip leaves no trace, so a phantom gap cannot be tied to the peer that claimed it.
#[test]
fn inc_i_294_m4_add_peer_raising_tip_logs_one_net_tip_status_line() {
    let mut m = mgr(100);
    let prev = tip(&m);
    let peer = PeerId::random();
    let lines = net_tip_lines(|| m.add_peer(peer, 150, h(b"a"), 150));
    assert_eq!(tip(&m), 150, "FIXTURE: add_peer raises the tip");
    one_info(
        &lines,
        &format!("[NET_TIP] src=status peer={peer} height=150 prev={prev}"),
        "add_peer raise",
    );
}

// INC-I-294 M-4 (Must) — Decision: a failure means every status refresh (every 30 s per peer) logs even when the tip did not move, flooding INFO.
#[test]
fn inc_i_294_m4_add_peer_equal_or_lower_logs_nothing() {
    let mut m = mgr(100);
    m.add_peer(PeerId::random(), 150, h(b"a"), 150);
    let lines = net_tip_lines(|| {
        m.add_peer(PeerId::random(), 150, h(b"b"), 150);
        m.add_peer(PeerId::random(), 120, h(b"c"), 120);
    });
    assert_eq!(tip(&m), 150, "FIXTURE: tip unchanged");
    assert!(lines.is_empty(), "no change, no {TAG}; got {lines:?}");
}

// INC-I-294 M-4 (Must) — Decision: a failure means the on_peer_status path (update_peer, the live refresh) still raises the tip silently.
#[test]
fn inc_i_294_m4_update_peer_raise_logs_once_and_equal_or_lower_logs_nothing() {
    let mut m = mgr(100);
    let peer = PeerId::random();
    m.add_peer(peer, 150, h(b"a"), 150);
    let lines = net_tip_lines(|| m.update_peer(peer, 200, h(b"a2"), 200));
    assert_eq!(tip(&m), 200, "FIXTURE: update_peer raises the tip");
    one_info(
        &lines,
        &format!("[NET_TIP] src=status peer={peer} height=200 prev=150"),
        "update_peer raise",
    );
    let quiet = net_tip_lines(|| {
        m.update_peer(peer, 200, h(b"a2"), 200);
        m.update_peer(peer, 180, h(b"a3"), 180);
    });
    assert!(quiet.is_empty(), "update_peer no change: got {quiet:?}");
}

// INC-I-294 M-4 (Must) — Decision: a failure means a disconnect that lowers the network tip leaves no trace, so a vanished gap cannot be explained.
#[test]
fn inc_i_294_m4_remove_peer_lowering_tip_logs_peer_removed() {
    let mut m = mgr(100);
    let (hi, lo) = (PeerId::random(), PeerId::random());
    m.add_peer(hi, 200, h(b"hi"), 200);
    m.add_peer(lo, 150, h(b"lo"), 150);
    let quiet = net_tip_lines(|| m.remove_peer(&lo));
    assert_eq!(
        tip(&m),
        200,
        "FIXTURE: removing the lower peer keeps the tip"
    );
    assert!(quiet.is_empty(), "remove_peer no change: got {quiet:?}");
    m.add_peer(lo, 150, h(b"lo"), 150);
    let lines = net_tip_lines(|| m.remove_peer(&hi));
    assert_eq!(
        tip(&m),
        150,
        "FIXTURE: removing the top peer lowers the tip"
    );
    one_info(
        &lines,
        "[NET_TIP] src=peer_removed peer=",
        "remove_peer lowers",
    );
    assert!(
        lines[0].contains("height=150 prev=200"),
        "remove_peer lowers: want height=150 prev=200 in {:?}",
        lines[0]
    );
}

// INC-I-294 M-4 (Must) — Decision: a failure means the below-gate orphan guess (best+1, a phantom gap of one) still moves the tip silently.
#[test]
fn inc_i_294_m4_orphan_guess_below_gate_logs_orphan_guess() {
    let mut m = mgr(500);
    m.add_peer(PeerId::random(), 500, h(b"p"), 500);
    let prev = tip(&m);
    assert_eq!(prev, 500, "FIXTURE: tip at the local height");
    let lines = net_tip_lines(|| m.note_orphan_gossip_block(501, 501));
    assert_eq!(tip(&m), 501, "FIXTURE: the guess is written below the gate");
    one_info(
        &lines,
        "[NET_TIP] src=orphan_guess peer=- height=501 prev=500",
        "orphan guess below gate",
    );
    let quiet = net_tip_lines(|| m.note_orphan_gossip_block(501, 501));
    assert!(quiet.is_empty(), "orphan guess equal: got {quiet:?}");
}

// INC-I-294 M-4 (Must) — Decision: a failure means a line claims an orphan guess moved the tip at/above the gate, where no write happens.
#[test]
fn inc_i_294_m4_orphan_guess_above_gate_logs_nothing() {
    let mut m = mgr(GATE);
    m.add_peer(PeerId::random(), GATE, h(b"p"), GATE as u32);
    let lines = net_tip_lines(|| m.note_orphan_gossip_block(GATE + 1, GATE as u32 + 1));
    assert_eq!(tip(&m), GATE, "FIXTURE: no write at/above the gate");
    assert!(lines.is_empty(), "above gate no {TAG}; got {lines:?}");
}

// INC-I-294 M-4 (Should) — Decision: a failure means every applied block (one per slot) adds a [NET_TIP] INFO line, which the spec excludes.
#[test]
fn inc_i_294_m4_applied_block_writer_logs_nothing() {
    let mut m = mgr(100);
    let lines = net_tip_lines(|| m.update_network_tip_height(300));
    assert_eq!(tip(&m), 300, "FIXTURE: applied height raises the tip");
    assert!(lines.is_empty(), "applied writer no {TAG}; got {lines:?}");
}
