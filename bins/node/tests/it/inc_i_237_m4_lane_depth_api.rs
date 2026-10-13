//! INC-I-237 M4 (REQ-GRV-016) — the node maps the lane snapshot's depth and
//! per-topic shed onto /metrics, label by label.
//!
//! covers: metrics_gossip event_sink
//!
//! OUTPUT CONTRACT: fn observe_event_lanes(&LaneSnapshot, &LaneBudgets)
//!   O1-O3: none (returns ())   O4: none   O6: none
//!   O5: REGISTRY doli_event_lane_depth{lane} = consensus_len / bulk_len /
//!       backlog_len; doli_event_shed_by_topic_total{topic} = shed_by_topic
//!       (process-global: writers hold `inc_i_237_m0_common::lock()`)
//! PATHS: P1 first refresh with distinct non-zero values; P2 depth refresh to 0
//! INPUT PARTITIONS: every lane and every topic carries a DISTINCT value, so a
//!   swapped label fails; depth {non-zero, zero}
//! MATRIX: O5 x P1 (3 lanes + 5 topics) + O5 x P2 (3 lanes) — all asserted.

use super::inc_i_204_m0_common::exported_value;
use super::inc_i_237_m0_common as c;
use doli_node::metrics_gossip::observe_event_lanes;
use network::service::{LaneBudgets, LaneSnapshot, TopicCounts};

const DEPTH: &str = "doli_event_lane_depth";
const SHED: &str = "doli_event_shed_by_topic_total";

// REQ-GRV-016 — Decision: a failure means the depth gauge shows another lane's count (or none), so an operator reads the wrong lane as backed up.
#[test]
fn req_grv_016_depth_gauge_reports_each_lane_count() {
    let _g = c::lock();
    let budgets = LaneBudgets::for_max_peers(8);
    let snap = LaneSnapshot {
        consensus_len: 11,
        bulk_len: 22,
        backlog_len: 33,
        ..LaneSnapshot::default()
    };
    observe_event_lanes(&snap, &budgets);
    for (lane, want) in [("consensus", 11.0), ("bulk", 22.0), ("backlog", 33.0)] {
        assert_eq!(
            exported_value(DEPTH, &[("lane", lane)]),
            Some(want),
            "{lane}"
        );
    }
    observe_event_lanes(&LaneSnapshot::default(), &budgets);
    for lane in ["consensus", "bulk", "backlog"] {
        assert_eq!(
            exported_value(DEPTH, &[("lane", lane)]),
            Some(0.0),
            "{lane} resets"
        );
    }
}

// REQ-GRV-016 — Decision: a failure means a topic's shed count lands on another topic's series, so the dashboard blames the wrong topic for drops.
#[test]
fn req_grv_016_shed_by_topic_reports_each_topic_count() {
    let _g = c::lock();
    let snap = LaneSnapshot {
        shed_by_topic: TopicCounts {
            blocks: 101,
            txs: 102,
            attestations: 103,
            votes: 104,
            producers: 105,
        },
        ..LaneSnapshot::default()
    };
    observe_event_lanes(&snap, &LaneBudgets::for_max_peers(8));
    for (topic, want) in [
        ("blocks", 101.0),
        ("txs", 102.0),
        ("attestations", 103.0),
        ("votes", 104.0),
        ("producers", 105.0),
    ] {
        assert_eq!(
            exported_value(SHED, &[("topic", topic)]),
            Some(want),
            "{topic}"
        );
    }
}
