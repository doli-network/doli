//! INC-I-237 M1.5 — developer path coverage for the sink guards the red tests
//! do not reach: control emit after the node side is gone with a non-empty
//! backlog, the flush-on-closed accounting reset, and the empty-lane escape
//! that keeps an oversized control event from wedging the consensus lane.
//!
//! OUTPUT CONTRACT: EventSink::{emit, try_flush} / EventLanes::try_recv
//!   O1: none   O2: lane queue + control backlog   O3: EmitOutcome / bool / Option<NetworkEvent>
//!   O4: none   O5: LaneSnapshot {consensus_bytes, backlog_bytes, backlog_len, control_dropped}
//!   O6: events delivered to the node
//! PATHS: P1 control emit, backlog non-empty, receiver dropped -> Shed;
//!   P2 try_flush on a closed lane -> backlog cleared, bytes 0, false;
//!   P3 control event larger than the consensus byte budget into an EMPTY lane -> admitted;
//!   P4 same into a NON-empty lane -> backlogged, then flushed once the lane drains
//! INPUT PARTITIONS: receiver {open, dropped}; lane bytes {0, > 0}; event size {<= budget, > budget}
//! MATRIX: O3 x P1-P4, O5 x P2-P4, O6 x P3-P4 — asserted below.

use libp2p::PeerId;

use super::inc_i_237_m15_fixtures::{label, roomy, sync_resp};
use crate::service::event_sink::{event_bytes, event_lanes, EmitOutcome, LaneBudgets};
use crate::service::types::NetworkEvent;

fn pc() -> NetworkEvent {
    NetworkEvent::PeerConnected(PeerId::random())
}

// REQ-GRV-006 / F-11 — Decision: a failure means a control event emitted after the node side is gone is reported as delivered (backlogged forever) or the flush leaves phantom backlog bytes on the lane gauges.
#[test]
fn req_grv_006_control_emit_after_close_with_backlog_sheds_and_flush_resets() {
    let b = LaneBudgets {
        consensus_cap: 1,
        ..roomy()
    };
    let (sink, lanes) = event_lanes(b);
    assert_eq!(sink.emit(pc()), EmitOutcome::Emitted, "into the lane");
    assert_eq!(sink.emit(pc()), EmitOutcome::Emitted, "into the backlog");
    drop(lanes);
    assert_eq!(
        sink.emit(pc()),
        EmitOutcome::Shed,
        "closed: no backlog growth"
    );
    let s = sink.stats().snapshot();
    assert_eq!(
        (s.backlog_len, s.control_dropped),
        (1, 0),
        "shutdown is not a drop"
    );
    assert!(!sink.try_flush(), "node side gone");
    let s = sink.stats().snapshot();
    assert_eq!(
        (s.backlog_len, s.backlog_bytes),
        (0, 0),
        "flush on close resets the backlog"
    );
}

// REQ-GRV-007 — Decision: a failure means one control event larger than the consensus byte budget can never enter the lane, so the backlog wedges behind it and every later control event is lost.
#[test]
fn req_grv_007_oversized_control_event_enters_only_an_empty_lane() {
    let big = sync_resp(1, 4096);
    let b = LaneBudgets {
        consensus_bytes: event_bytes(&big) / 2,
        ..roomy()
    };
    let (sink, mut lanes) = event_lanes(b);
    assert_eq!(sink.emit(big), EmitOutcome::Emitted, "empty lane admits it");
    assert_eq!(
        sink.emit(sync_resp(2, 4096)),
        EmitOutcome::Emitted,
        "backlogged"
    );
    assert_eq!(sink.stats().snapshot().backlog_len, 1, "lane over budget");
    assert_eq!(lanes.try_recv().map(|e| label(&e)), Some("SR:1".into()));
    assert!(lanes.try_recv().is_none(), "second waits in the backlog");
    sink.try_flush();
    assert_eq!(lanes.try_recv().map(|e| label(&e)), Some("SR:2".into()));
    let s = sink.stats().snapshot();
    assert_eq!(
        (s.consensus_bytes, s.backlog_bytes, s.backlog_len),
        (0, 0, 0)
    );
}
