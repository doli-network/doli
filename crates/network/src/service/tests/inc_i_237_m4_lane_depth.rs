//! INC-I-237 M4 (REQ-GRV-016) — per-lane queued-event depth and per-topic shed
//! are counted where events enter and leave the lanes.
//!
//! covers: event_sink event_lanes verdict mod
//!
//! OUTPUT CONTRACT: EventSink::{emit, emit_block}(..) -> EmitOutcome;
//!   EventLanes::{try_recv, recv} -> Option<NetworkEvent>
//!   O1: none   O3: EmitOutcome / Option<NetworkEvent>   O4: none   O6: lane queues
//!   O2: LaneStats::snapshot() {consensus_len, bulk_len, backlog_len,
//!       shed_by_topic: TopicCounts} (per sink instance, no global state)   O5: none
//! PATHS: P1 emitted -> depth +1; P2 dequeued -> depth -1; P3 refused by count
//!   or by bytes -> depth unchanged, topic shed +1; P4 control overflow ->
//!   backlog, no topic shed
//! INPUT PARTITIONS: lane {consensus gossip, consensus control, bulk};
//!   refusal {count full, bytes full}; topic {blocks, txs, attestations, votes,
//!   producers}; drain {partial try_recv, full}
//! MATRIX: depth x P1-P3 per lane, shed_by_topic x P3 per topic + P4 — all asserted.

use libp2p::PeerId;

use super::inc_i_237_m15_fixtures::{att_event, block_event, roomy, tx_event};
use crate::gossip::staleness::GossipTopic;
use crate::service::backpressure::GossipShedMetrics;
use crate::service::event_sink::{event_lanes, EmitOutcome, LaneBudgets, TopicCounts};
use crate::service::types::NetworkEvent;
use crate::service::GossipHandle;

const TOPICS: [GossipTopic; 5] = [
    GossipTopic::Blocks,
    GossipTopic::Transactions,
    GossipTopic::Attestations,
    GossipTopic::Votes,
    GossipTopic::Producers,
];

fn vote() -> NetworkEvent {
    NetworkEvent::NewVote(vec![7; 16], GossipHandle::now())
}

fn producers() -> NetworkEvent {
    NetworkEvent::ProducersAnnounced(Vec::new(), GossipHandle::now())
}

fn shed_counts(c: &TopicCounts) -> [u64; 5] {
    TOPICS.map(|t| c.get(t))
}

// REQ-GRV-016 — Decision: a failure means lane depth does not follow enqueue/dequeue per lane, so the depth gauge cannot show which lane is backing up.
#[tokio::test]
async fn req_grv_016_depth_follows_enqueue_and_dequeue_per_lane() {
    let (sink, mut lanes) = event_lanes(roomy());
    let stats = sink.stats();
    for ev in [
        att_event(8),
        att_event(8),
        NetworkEvent::PeerConnected(PeerId::random()),
    ] {
        assert_eq!(sink.emit(ev), EmitOutcome::Emitted);
    }
    for ev in [tx_event(1), tx_event(2), vote(), producers()] {
        assert_eq!(sink.emit(ev), EmitOutcome::Emitted);
    }
    let s = stats.snapshot();
    assert_eq!(
        (s.consensus_len, s.bulk_len, s.backlog_len),
        (3, 4, 0),
        "after enqueue"
    );
    assert!(lanes.try_recv().is_some());
    let s = stats.snapshot();
    assert_eq!(
        (s.consensus_len, s.bulk_len),
        (2, 4),
        "consensus dequeued first"
    );
    for _ in 0..6 {
        assert!(lanes.recv().await.is_some());
    }
    let s = stats.snapshot();
    assert_eq!(
        (s.consensus_len, s.bulk_len, s.backlog_len),
        (0, 0, 0),
        "drained"
    );
}

// REQ-GRV-016 — Decision: a failure means a refused event is counted as queued, so depth over-reports exactly when the lane is shedding.
#[test]
fn req_grv_016_refused_events_do_not_count_as_depth() {
    let by_count = LaneBudgets {
        bulk_cap: 1,
        ..roomy()
    };
    let (sink, _lanes) = event_lanes(by_count);
    assert_eq!(sink.emit(tx_event(1)), EmitOutcome::Emitted);
    assert_eq!(sink.emit(tx_event(2)), EmitOutcome::Shed);
    assert_eq!(sink.stats().snapshot().bulk_len, 1, "count-full refusal");
    let by_bytes = LaneBudgets {
        bulk_bytes: 1,
        ..roomy()
    };
    let (sink, _lanes) = event_lanes(by_bytes);
    assert_eq!(sink.emit(tx_event(3)), EmitOutcome::Shed);
    assert_eq!(sink.stats().snapshot().bulk_len, 0, "byte-full refusal");
}

// REQ-GRV-016 — Decision: a failure means a shed gossip event is not attributed to its topic, so operators cannot see which topic the lanes are dropping.
#[test]
fn req_grv_016_each_shed_gossip_event_counts_its_topic() {
    let tight = LaneBudgets {
        consensus_cap: 1,
        bulk_cap: 1,
        ..roomy()
    };
    let (sink, _lanes) = event_lanes(tight);
    let shed = || sink.stats().snapshot().shed_by_topic;
    assert_eq!(shed(), TopicCounts::default(), "fresh sink");
    assert_eq!(sink.emit(tx_event(1)), EmitOutcome::Emitted);
    assert_eq!(sink.emit(att_event(8)), EmitOutcome::Emitted);
    assert_eq!(
        shed(),
        TopicCounts::default(),
        "emitted events are not shed"
    );
    assert_eq!(sink.emit(tx_event(2)), EmitOutcome::Shed);
    for _ in 0..2 {
        assert_eq!(sink.emit(vote()), EmitOutcome::Shed);
    }
    for _ in 0..3 {
        assert_eq!(sink.emit(producers()), EmitOutcome::Shed);
    }
    for _ in 0..4 {
        assert_eq!(sink.emit(att_event(8)), EmitOutcome::Shed);
    }
    let metrics = GossipShedMetrics::new();
    for _ in 0..5 {
        assert_eq!(sink.emit_block(block_event(), &metrics), EmitOutcome::Shed);
    }
    let after = shed();
    let want = TopicCounts {
        blocks: 5,
        txs: 1,
        attestations: 4,
        votes: 2,
        producers: 3,
    };
    assert_eq!(after, want, "each drop is attributed to its own topic");
    assert_eq!(
        shed_counts(&after),
        [5, 1, 4, 2, 3],
        "get() agrees with the fields"
    );
    assert_eq!(
        sink.emit(NetworkEvent::PeerConnected(PeerId::random())),
        EmitOutcome::Emitted,
        "control overflow goes to the backlog"
    );
    assert_eq!(shed(), after, "control events are not topic shed");
    assert_eq!(sink.stats().snapshot().backlog_len, 1);
}
