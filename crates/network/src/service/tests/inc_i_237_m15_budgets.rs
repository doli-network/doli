//! INC-I-237 M1.5 (R1/R3) — per-lane byte bounds (F-13), byte-bounded control
//! backlog (F-12 fallback), and the M1 sink contract on the two-lane receiver.
//!
//! OUTPUT CONTRACT: fn LaneBudgets::for_max_peers(usize) -> LaneBudgets /
//!   event_lanes(LaneBudgets) -> (EventSink, EventLanes) /
//!   EventSink::{emit, emit_block, try_flush} / EventLanes::try_recv
//!   O1: none   O2: lane queues + control backlog (observed through delivery order)
//!   O3: LaneBudgets / EmitOutcome {Emitted, Shed} / try_flush bool / Option<NetworkEvent>
//!   O4: none
//!   O5: LaneSnapshot {consensus_bytes, bulk_bytes, backlog_bytes, backlog_len,
//!       consensus_shed, bulk_shed, control_dropped}; GossipShedMetrics::blocks_dropped
//!   O6: events delivered to the node (count + order)
//! PATHS: P1 production budgets; P2 bulk emit under/over byte budget; P3 attestation
//!   and block emit under/over consensus byte budget; P4 payload control into the
//!   backlog under/over backlog bytes; P5 control-STATE into the backlog while backlog
//!   bytes are full; P6 backlog at backlog_cap -> drop; P7 try_flush FIFO;
//!   P8 dequeue releases bytes (full drain -> 0); P9 receiver dropped
//! INPUT PARTITIONS: flood {10x bulk bytes, 10x consensus bytes}; backlog {payload
//!   SyncResponse 64 KiB, state PeerConnected}; budget fill {below, exactly at, over};
//!   receiver {draining, idle, dropped}
//! MATRIX: O3 x P1-P9, O5 x P2-P8, O6 x P6-P7 — all asserted below.

use libp2p::PeerId;

use super::inc_i_237_m15_fixtures::{
    att_event, block_event, label, roomy, sync_resp, tx_event, variant,
};
use crate::service::backpressure::GossipShedMetrics;
use crate::service::event_lanes::EventLanes;
use crate::service::event_sink::{
    event_bytes, event_lanes, EmitOutcome, EventSink, LaneBudgets, LaneSnapshot, BACKLOG_BYTES,
    BACKLOG_CAP, BULK_LANE_BYTES, CONSENSUS_LANE_BYTES,
};
use crate::service::types::NetworkEvent;

fn assert_drained_to_zero(sink: &EventSink, lanes: &mut EventLanes) {
    while lanes.try_recv().is_some() {
        sink.try_flush();
    }
    let s: LaneSnapshot = sink.stats().snapshot();
    assert_eq!(
        (
            s.consensus_bytes,
            s.bulk_bytes,
            s.backlog_bytes,
            s.backlog_len
        ),
        (0, 0, 0, 0),
        "a full drain must return every in-flight counter to 0 (accounting leak)"
    );
}

fn pc() -> NetworkEvent {
    NetworkEvent::PeerConnected(PeerId::random())
}

fn connected(ev: NetworkEvent) -> PeerId {
    match ev {
        NetworkEvent::PeerConnected(p) => p,
        other => panic!("expected PeerConnected, got {other:?}"),
    }
}

// REQ-GRV-007 / R3 — Decision: a failure means production runs with a byte bound other than the spec R3 values (64 MiB consensus, 16 MiB bulk, 64 MiB backlog, 8192 backlog slots), e.g. an unbounded lane.
#[test]
fn req_grv_007_production_budgets_are_the_spec_r3_values() {
    assert_eq!(CONSENSUS_LANE_BYTES, 64 << 20);
    assert_eq!(BULK_LANE_BYTES, 16 << 20);
    assert_eq!(BACKLOG_BYTES, 64 << 20);
    assert_eq!(BACKLOG_CAP, 8_192);
    for peers in [1usize, 50, 1_000] {
        let b = LaneBudgets::for_max_peers(peers);
        assert_eq!(b.consensus_bytes, CONSENSUS_LANE_BYTES, "peers={peers}");
        assert_eq!(b.bulk_bytes, BULK_LANE_BYTES, "peers={peers}");
        assert_eq!(b.backlog_bytes, BACKLOG_BYTES, "peers={peers}");
        assert_eq!(b.backlog_cap, BACKLOG_CAP, "peers={peers}");
        assert!(b.consensus_cap > 0 && b.bulk_cap > 0, "peers={peers}");
    }
}

// REQ-GRV-007 / REQ-GRV-SEC-003 / F-13 — Decision: a failure means a tx flood holds more bulk bytes than its budget (unbounded heap behind the swarm), or a dequeue never releases bytes so the lane wedges shut after one burst.
#[test]
fn req_grv_007_bulk_bytes_stay_within_budget_under_10x_flood_and_release_on_dequeue() {
    let sz = event_bytes(&tx_event(7));
    let b = LaneBudgets {
        bulk_bytes: 8 * sz,
        bulk_cap: 10_000,
        ..roomy()
    };
    let (sink, mut lanes) = event_lanes(b);
    let stats = sink.stats();
    let mut emitted = 0;
    for i in 0..80 {
        if sink.emit(tx_event(7)) == EmitOutcome::Emitted {
            emitted += 1;
        }
        let s = stats.snapshot();
        assert!(
            s.bulk_bytes <= b.bulk_bytes,
            "emit {i}: {} > {}",
            s.bulk_bytes,
            b.bulk_bytes
        );
    }
    let s = stats.snapshot();
    assert_eq!((emitted, s.bulk_shed, s.consensus_shed), (8, 72, 0));
    assert_eq!(s.bulk_bytes, 8 * sz);
    assert!(lanes.try_recv().is_some() && lanes.try_recv().is_some());
    assert_eq!(
        stats.snapshot().bulk_bytes,
        6 * sz,
        "dequeue releases bytes"
    );
    for _ in 0..2 {
        assert_eq!(
            sink.emit(tx_event(7)),
            EmitOutcome::Emitted,
            "freed bytes re-admit"
        );
    }
    assert_eq!(sink.emit(tx_event(7)), EmitOutcome::Shed);
    assert_drained_to_zero(&sink, &mut lanes);
}

// REQ-GRV-007 / F-13 — Decision: a failure means an attestation or block flood holds more consensus bytes than its budget, or a block shed over the byte budget is not counted where operators look.
#[test]
fn req_grv_007_consensus_bytes_stay_within_budget_under_attestation_flood() {
    let asz = event_bytes(&att_event(4096));
    let b = LaneBudgets {
        consensus_bytes: 4 * asz,
        consensus_cap: 10_000,
        ..roomy()
    };
    let (sink, mut lanes) = event_lanes(b);
    let m = GossipShedMetrics::new();
    for i in 0..40 {
        sink.emit(att_event(4096));
        let s = sink.stats().snapshot();
        assert!(s.consensus_bytes <= b.consensus_bytes, "emit {i}");
    }
    assert_eq!(sink.stats().snapshot().consensus_shed, 36);
    assert!(event_bytes(&block_event()) > 0);
    assert_eq!(
        sink.emit_block(block_event(), &m),
        EmitOutcome::Shed,
        "over byte budget"
    );
    assert_eq!(
        m.blocks_dropped(),
        1,
        "block shed keeps the GossipShedMetrics count"
    );
    let s = sink.stats().snapshot();
    assert_eq!((s.consensus_shed, s.bulk_shed), (37, 0));
    assert_drained_to_zero(&sink, &mut lanes);
}

// REQ-GRV-007 / F-12 / R3 — Decision: a failure means 16 MiB SyncResponses can pile up past the backlog byte budget (unbounded heap while the node stalls), a byte-full backlog drops a peer-state event (peer table diverges), or the backlog re-orders control events.
#[test]
fn req_grv_007_backlog_is_byte_bounded_keeps_state_events_and_drains_fifo() {
    const P: usize = 64 * 1024;
    let ssz = event_bytes(&sync_resp(0, P));
    let b = LaneBudgets {
        consensus_cap: 1,
        backlog_bytes: 3 * ssz,
        backlog_cap: 64,
        ..roomy()
    };
    let (sink, mut lanes) = event_lanes(b);
    let first = PeerId::random();
    assert_eq!(
        sink.emit(NetworkEvent::PeerConnected(first)),
        EmitOutcome::Emitted
    );
    for id in 1..=5u64 {
        let want = if id <= 3 {
            EmitOutcome::Emitted
        } else {
            EmitOutcome::Shed
        };
        assert_eq!(sink.emit(sync_resp(id, P)), want, "SyncResponse {id}");
        let s = sink.stats().snapshot();
        assert!(
            s.backlog_bytes <= b.backlog_bytes,
            "after SyncResponse {id}"
        );
    }
    let s = sink.stats().snapshot();
    assert_eq!((s.control_dropped, s.backlog_len), (2, 3));
    let late = PeerId::random();
    assert_eq!(
        sink.emit(NetworkEvent::PeerConnected(late)),
        EmitOutcome::Emitted,
        "control-STATE event admitted while backlog bytes are full (count bound only)"
    );
    let s = sink.stats().snapshot();
    assert_eq!((s.control_dropped, s.backlog_len), (2, 4));
    let mut order = Vec::new();
    while let Some(ev) = lanes.try_recv() {
        order.push(label(&ev));
        sink.try_flush();
    }
    let want = [
        format!("PC:{first}"),
        "SR:1".into(),
        "SR:2".into(),
        "SR:3".into(),
        format!("PC:{late}"),
    ];
    assert_eq!(order, want, "backlog drains FIFO behind the lane");
    assert_drained_to_zero(&sink, &mut lanes);
}

// REQ-GRV-007 / R3 — Decision: a failure means a payload control event bypasses the backlog COUNT bound when its bytes still fit, so tiny SyncResponses can grow the backlog without limit.
#[test]
fn req_grv_007_payload_control_respects_the_backlog_count_bound() {
    let b = LaneBudgets {
        consensus_cap: 1,
        backlog_cap: 2,
        ..roomy()
    };
    let (sink, mut lanes) = event_lanes(b);
    assert_eq!(sink.emit(pc()), EmitOutcome::Emitted);
    assert_eq!(sink.emit(sync_resp(1, 8)), EmitOutcome::Emitted);
    assert_eq!(sink.emit(sync_resp(2, 8)), EmitOutcome::Emitted);
    assert_eq!(
        sink.emit(sync_resp(3, 8)),
        EmitOutcome::Shed,
        "backlog_cap reached"
    );
    assert_eq!(sink.stats().snapshot().control_dropped, 1);
    assert_drained_to_zero(&sink, &mut lanes);
}

// ==================== M1 sink contract, migrated to event_lanes ====================

// REQ-GRV-006 — Decision: a failure means the sync sink either blocks/loses a sendable event or stops shedding gossip on a full lane, so the swarm parks or the node queue grows without bound.
#[test]
fn inc_i_237_m1_emit_sends_with_room_and_sheds_gossip_when_full() {
    let b = LaneBudgets {
        consensus_cap: 2,
        bulk_cap: 1,
        ..roomy()
    };
    let (sink, mut lanes) = event_lanes(b);
    let vote = || NetworkEvent::NewVote(b"{}".to_vec(), crate::service::GossipHandle::now());
    assert_eq!(sink.emit(pc()), EmitOutcome::Emitted);
    assert_eq!(sink.emit(vote()), EmitOutcome::Emitted);
    assert_eq!(sink.emit(vote()), EmitOutcome::Shed, "full + gossip");
    assert!(lanes.try_recv().is_some() && lanes.try_recv().is_some());
    assert!(
        lanes.try_recv().is_none(),
        "the shed event must not be delivered"
    );
    sink.try_flush();
    assert!(
        lanes.try_recv().is_none(),
        "nothing backlogged for a shed gossip event"
    );
}

// REQ-GRV-006 / F-12 — Decision: a failure means a control event emitted while the node is behind is lost or re-ordered, so the node's peer table diverges from the swarm's.
#[test]
fn inc_i_237_m1_control_events_backlog_in_order_and_try_flush_drains_them() {
    let b = LaneBudgets {
        consensus_cap: 2,
        ..roomy()
    };
    let (sink, mut lanes) = event_lanes(b);
    let want: Vec<PeerId> = (0..4).map(|_| PeerId::random()).collect();
    let mut evs = want.iter().map(|p| NetworkEvent::PeerConnected(*p));
    for _ in 0..3 {
        assert_eq!(sink.emit(evs.next().unwrap()), EmitOutcome::Emitted);
    }
    sink.try_flush();
    let mut got = vec![connected(lanes.try_recv().unwrap())];
    assert_eq!(sink.emit(evs.next().unwrap()), EmitOutcome::Emitted);
    sink.try_flush();
    got.push(connected(lanes.try_recv().unwrap()));
    got.push(connected(lanes.try_recv().unwrap()));
    assert!(
        lanes.try_recv().is_none(),
        "c4 must wait behind c3, not jump the backlog"
    );
    sink.try_flush();
    got.push(connected(lanes.try_recv().unwrap()));
    assert_eq!(got, want, "control events delivered in emit order");
    sink.try_flush();
    assert!(
        lanes.try_recv().is_none(),
        "empty backlog: try_flush is a no-op"
    );
}

// REQ-GRV-007 / F-12 — Decision: a failure means the control backlog is unbounded (memory) or silently drops below its cap, so a long node stall either grows the heap or loses control events it promised to keep.
#[test]
fn inc_i_237_m1_control_backlog_is_bounded_and_overflow_is_shed() {
    let b = LaneBudgets {
        consensus_cap: 1,
        backlog_cap: BACKLOG_CAP,
        backlog_bytes: BACKLOG_BYTES,
        ..roomy()
    };
    let (sink, mut lanes) = event_lanes(b);
    assert_eq!(sink.emit(pc()), EmitOutcome::Emitted);
    for i in 0..BACKLOG_CAP {
        let ev = NetworkEvent::PeerDisconnected(PeerId::random());
        assert_eq!(sink.emit(ev), EmitOutcome::Emitted, "backlog slot {i}");
    }
    let over = NetworkEvent::PeerDisconnected(PeerId::random());
    assert_eq!(sink.emit(over), EmitOutcome::Shed, "backlog at BACKLOG_CAP");
    assert_eq!(sink.stats().snapshot().control_dropped, 1);
    let mut delivered = 0usize;
    while lanes.try_recv().is_some() {
        delivered += 1;
        sink.try_flush();
    }
    assert_eq!(delivered, 1 + BACKLOG_CAP);
}

// REQ-GRV-006 / F-11 — Decision: a failure means a block shed is not counted (or a shutdown is counted as a shed), or a dropped node side makes the sync sink panic or report delivery.
#[test]
fn inc_i_237_m1_emit_block_counts_full_shed_not_closed_and_closed_is_safe() {
    let metrics = GossipShedMetrics::new();
    let b = LaneBudgets {
        consensus_cap: 1,
        ..roomy()
    };
    let (sink, mut lanes) = event_lanes(b);
    assert_eq!(
        sink.emit_block(block_event(), &metrics),
        EmitOutcome::Emitted
    );
    assert_eq!(metrics.blocks_dropped(), 0);
    assert_eq!(sink.emit_block(block_event(), &metrics), EmitOutcome::Shed);
    assert_eq!(metrics.blocks_dropped(), 1, "full lane = counted shed");
    assert_eq!(sink.stats().snapshot().consensus_shed, 1);
    assert_eq!(lanes.try_recv().map(|e| variant(&e)), Some("NewBlock"));
    assert_eq!(sink.emit(pc()), EmitOutcome::Emitted);
    assert_eq!(sink.emit(pc()), EmitOutcome::Emitted);
    drop(lanes);
    assert!(!sink.try_flush(), "try_flush reports the node side is gone");
    assert_eq!(sink.emit_block(block_event(), &metrics), EmitOutcome::Shed);
    assert_eq!(metrics.blocks_dropped(), 1, "closed = shutdown, not a shed");
    assert_eq!(sink.emit(tx_event(1)), EmitOutcome::Shed);
    assert_eq!(sink.emit(pc()), EmitOutcome::Shed);
    sink.try_flush();
}
