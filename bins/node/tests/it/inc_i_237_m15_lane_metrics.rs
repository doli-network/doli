//! INC-I-237 M1.5 (R1) — the event-lane byte budgets and in-flight bytes are on /metrics.
//!
//! OUTPUT CONTRACT: fn observe_event_lanes(&LaneSnapshot, &LaneBudgets)
//!   O1: none   O2: none   O3: none (returns ())   O4: none
//!   O5: REGISTRY gauges doli_event_lane_budget_bytes{lane} and
//!       doli_event_lane_inflight_bytes{lane}, lane in {consensus, bulk, backlog}
//!       (process-global: writers hold `inc_i_237_m0_common::lock()`)
//!   O6: none
//! PATHS: P1 first refresh creates the series; P2 a later refresh replaces values
//! INPUT PARTITIONS: budgets {distinct per lane}; snapshot {non-zero, all zero}
//! MATRIX: O5 x P1-P2 — every cell asserted below.

use super::inc_i_204_m0_common::{encode_registry, exported_value};
use super::inc_i_237_m0_common as c;
use doli_node::metrics_gossip::observe_event_lanes;
use network::service::{LaneBudgets, LaneSnapshot};

const BUDGET: &str = "doli_event_lane_budget_bytes";
const INFLIGHT: &str = "doli_event_lane_inflight_bytes";
const LANES: [&str; 3] = ["consensus", "bulk", "backlog"];

fn snapshot(c: usize, b: usize, k: usize) -> LaneSnapshot {
    LaneSnapshot {
        consensus_bytes: c,
        bulk_bytes: b,
        backlog_bytes: k,
        backlog_len: 4,
        consensus_shed: 5,
        bulk_shed: 6,
        control_dropped: 7,
        ..LaneSnapshot::default()
    }
}

fn series(name: &str) -> usize {
    let prefix = format!("{name}{{");
    encode_registry()
        .lines()
        .filter(|l| l.starts_with(&prefix))
        .count()
}

// REQ-GRV-007 — Decision: a failure means operators cannot see the finite lane bounds or the live in-flight bytes (the M1.5 outcome metric), or a refresh accumulates instead of reporting the current value.
#[test]
fn req_grv_007_lane_gauges_export_three_budget_series_and_live_inflight_bytes() {
    let _g = c::lock();
    let budgets = LaneBudgets {
        consensus_cap: 7,
        bulk_cap: 5,
        consensus_bytes: 64 << 20,
        bulk_bytes: 16 << 20,
        backlog_bytes: 32 << 20,
        backlog_cap: 8_192,
    };
    observe_event_lanes(&snapshot(1_111, 2_222, 3_333), &budgets);
    assert_eq!(series(BUDGET), 3, "exactly one budget series per lane");
    assert_eq!(series(INFLIGHT), 3, "exactly one in-flight series per lane");
    let want_budget = [64u64 << 20, 16 << 20, 32 << 20];
    let want_inflight = [1_111u64, 2_222, 3_333];
    for ((lane, b), f) in LANES.into_iter().zip(want_budget).zip(want_inflight) {
        assert_eq!(
            exported_value(BUDGET, &[("lane", lane)]),
            Some(b as f64),
            "{lane}"
        );
        assert_eq!(
            exported_value(INFLIGHT, &[("lane", lane)]),
            Some(f as f64),
            "{lane}"
        );
    }
    observe_event_lanes(&snapshot(0, 0, 0), &budgets);
    for lane in LANES {
        assert_eq!(
            exported_value(INFLIGHT, &[("lane", lane)]),
            Some(0.0),
            "{lane} resets"
        );
    }
    assert_eq!(series(BUDGET), 3);
}
