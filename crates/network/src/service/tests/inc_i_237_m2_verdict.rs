//! INC-I-237 M2 (D1 / R5 / C6) — the swarm verdict arm, judged with a counting
//! fake reporter, and the node->swarm verdict send.
//!
//! OUTPUT CONTRACT: fn apply_gossip_verdict(GossipHandle, Verdict,
//!   report: FnOnce(&MessageId, &PeerId, MessageAcceptance)) -> bool;
//!   fn send_gossip_verdict(&Sender<NetworkCommand>, GossipHandle, Verdict) -> bool;
//!   fn event_bytes(&NetworkEvent) for the one-event-per-message tx shape
//!   O1: none   O2: none
//!   O3: bool (reported / sent); usize (lane bytes)
//!   O4: none
//!   O5: LATE_VERDICTS per topic (process-global: each test owns one topic)
//!   O6: `report` calls (count, id, source, acceptance); the command channel
//! PATHS: P1 on time -> exactly one report on the ORIGINAL id + source;
//!   P2 late (> VERDICT_DEADLINE) -> no report, late[topic] += 1;
//!   P3 handle never stamped by the swarm -> no report, not counted;
//!   P4 send on a free / full / closed channel -> true / false / false, never waits
//! INPUT PARTITIONS: verdict {Accept, Ignore}; age {fresh, 3.3 s, 3.7 s, 10 s};
//!   topic {txs, votes, producers, blocks}; channel {free, full, closed};
//!   tx message size {0, 1, 50}
//! MATRIX: O6 x P1-P3 x verdicts, O5 x P1-P3, O3 x P1-P4 — asserted below.
//!   Duplicate and never-sent verdicts are judged on the wire (vendor mcache)
//!   in tests/it/inc_i_237_m2_verdict_path.rs.

use std::time::{Duration, Instant};

use crypto::Hash;
use doli_core::Transaction;
use libp2p::gossipsub::{MessageAcceptance, MessageId};
use libp2p::PeerId;
use tokio::sync::mpsc;

use crate::gossip::staleness::GossipTopic;
use crate::service::event_sink::event_bytes;
use crate::service::types::{NetworkCommand, NetworkEvent};
use crate::service::verdict::{
    apply_gossip_verdict, send_gossip_verdict, Verdict, LATE_VERDICTS, VERDICT_DEADLINE,
};
use crate::service::GossipHandle;

type Calls = Vec<(MessageId, PeerId, &'static str)>;

fn label(a: &MessageAcceptance) -> &'static str {
    match a {
        MessageAcceptance::Accept => "Accept",
        MessageAcceptance::Ignore => "Ignore",
        MessageAcceptance::Reject => "Reject",
    }
}

fn handle(topic: GossipTopic, age: Duration) -> (GossipHandle, MessageId, PeerId) {
    let id = MessageId::new(&[0x37; 20]);
    let src = PeerId::random();
    let mut h = GossipHandle::new(topic, id.clone(), src);
    h.recv_at = Instant::now()
        .checked_sub(age)
        .expect("monotonic clock past age");
    (h, id, src)
}

fn apply(h: GossipHandle, v: Verdict) -> (bool, Calls) {
    let mut calls = Calls::new();
    let reported = apply_gossip_verdict(h, v, |id, src, acc| {
        calls.push((id.clone(), *src, label(&acc)))
    });
    (reported, calls)
}

fn tx(tag: u64) -> Transaction {
    Transaction::new_coinbase(tag, Hash::default(), tag, tag as u32)
}

// F-3 / REQ-GRV-008 — Decision: a failure means an on-time node verdict is dropped, reported twice, reported on another id or peer, or with the wrong acceptance, so the mesh forwards (or scores) the wrong message.
#[test]
fn f_3_on_time_verdict_reports_exactly_once_on_the_original_id_and_source() {
    let before = LATE_VERDICTS.get(GossipTopic::Producers);
    for (v, want) in [(Verdict::Accept, "Accept"), (Verdict::Ignore, "Ignore")] {
        let (h, id, src) = handle(GossipTopic::Producers, Duration::from_millis(10));
        let (reported, calls) = apply(h, v);
        assert!(reported, "{want}: on time must report");
        assert_eq!(
            calls,
            vec![(id, src, want)],
            "{want}: exactly one report, original id + source"
        );
    }
    assert_eq!(
        LATE_VERDICTS.get(GossipTopic::Producers),
        before,
        "on time is never counted late"
    );
}

// F-3 / REQ-GRV-008 — Decision: a failure means a verdict after the window is still reported (a vendor no-op WARN per message with log rotation off) or is not counted, so late relays are invisible to the operator.
#[test]
fn f_3_late_verdict_is_counted_per_topic_and_never_reported() {
    let (txs0, blocks0) = (
        LATE_VERDICTS.get(GossipTopic::Transactions),
        LATE_VERDICTS.get(GossipTopic::Blocks),
    );
    for (v, age) in [(Verdict::Accept, 3_600), (Verdict::Ignore, 10_000)] {
        let (h, _, _) = handle(GossipTopic::Transactions, Duration::from_millis(age));
        let (reported, calls) = apply(h, v);
        assert!(!reported, "{v:?} at {age} ms must not report");
        assert!(calls.is_empty(), "{v:?} at {age} ms reported {calls:?}");
    }
    assert_eq!(
        LATE_VERDICTS.get(GossipTopic::Transactions) - txs0,
        2,
        "one count per late verdict"
    );
    assert_eq!(
        LATE_VERDICTS.get(GossipTopic::Blocks),
        blocks0,
        "counter is per topic"
    );
}

// R5 / C6 — Decision: a failure means the deadline moved (a report could land after mcache eviction, or valid on-time verdicts are thrown away as late).
#[test]
fn r5_deadline_boundary_reports_at_3_3_s_and_counts_at_3_7_s() {
    let before = LATE_VERDICTS.get(GossipTopic::Votes);
    let (h, _, _) = handle(GossipTopic::Votes, Duration::from_millis(3_300));
    let (reported, calls) = apply(h, Verdict::Accept);
    assert!(reported && calls.len() == 1, "3.3 s is inside the window");
    let (h, _, _) = handle(GossipTopic::Votes, Duration::from_millis(3_700));
    let (reported, calls) = apply(h, Verdict::Accept);
    assert!(!reported && calls.is_empty(), "3.7 s is past the window");
    assert_eq!(LATE_VERDICTS.get(GossipTopic::Votes) - before, 1);
}

// R5 / F-13 — Decision: a failure means the verdict window outlives the vendor mcache (min ~4 s: history_length 5 x 1 s heartbeat) or undercuts the 2 s SLO, so pending state is no longer bounded by the vendor window.
#[test]
fn r5_verdict_deadline_is_3_5_s_inside_the_mcache_window() {
    assert_eq!(VERDICT_DEADLINE, Duration::from_millis(3_500));
    assert!(VERDICT_DEADLINE < Duration::from_secs(4));
    assert!(VERDICT_DEADLINE >= Duration::from_secs(2));
}

// F-3 — Decision: a failure means a handle the swarm never stamped (fetch path, synthetic event) reaches report_message_validation_result with a made-up id or panics on the missing origin.
#[test]
fn f_3_unstamped_handle_never_reports() {
    for v in [Verdict::Accept, Verdict::Ignore] {
        let (reported, calls) = apply(GossipHandle::now(), v);
        assert!(!reported && calls.is_empty(), "{v:?}: {calls:?}");
    }
}

// F-11 / REQ-GRV-008 — Decision: a failure means the node's verdict send can wait on (or panic at) a full or closed swarm channel, re-opening the node<->swarm await cycle C1 or hanging shutdown.
#[test]
fn f_11_send_gossip_verdict_try_sends_and_never_waits() {
    let (cmd, mut rx) = mpsc::channel(1);
    let (h, _, _) = handle(GossipTopic::Transactions, Duration::ZERO);
    let stamp = h.recv_at;
    assert!(
        send_gossip_verdict(&cmd, h, Verdict::Accept),
        "free slot -> sent"
    );
    match rx.try_recv() {
        Ok(NetworkCommand::GossipVerdict { handle, verdict }) => {
            assert_eq!(verdict, Verdict::Accept);
            assert_eq!(handle.recv_at, stamp, "the original handle travels back");
        }
        other => panic!("expected GossipVerdict, got {other:?}"),
    }
    cmd.try_send(NetworkCommand::Bootstrap)
        .expect("fill the only slot");
    let (h, _, _) = handle(GossipTopic::Transactions, Duration::ZERO);
    assert!(
        !send_gossip_verdict(&cmd, h, Verdict::Ignore),
        "full -> dropped, not awaited"
    );
    assert!(matches!(rx.try_recv(), Ok(NetworkCommand::Bootstrap)));
    assert!(
        rx.try_recv().is_err(),
        "nothing queued behind the full slot"
    );
    drop(rx);
    let (h, _, _) = handle(GossipTopic::Transactions, Duration::ZERO);
    assert!(
        !send_gossip_verdict(&cmd, h, Verdict::Accept),
        "closed -> false"
    );
}

// D1 / F-13 — Decision: a failure means a 50-tx gossip message is charged to the bulk lane as one tx, so the byte bound on deferred tx messages undercounts up to 50x.
#[test]
fn d1_one_tx_message_event_charges_every_tx_to_the_lane() {
    let ev = |txs: Vec<Transaction>| NetworkEvent::NewTransaction(txs, GossipHandle::now());
    let empty = event_bytes(&ev(Vec::new()));
    let one = event_bytes(&ev(vec![tx(1)])) - empty;
    assert!(one > 0, "a tx costs bytes");
    let fifty = event_bytes(&ev((1..=50).map(tx).collect())) - empty;
    assert!(
        fifty >= 50 * one,
        "50-tx message charged {fifty} B < 50 x {one} B"
    );
}
