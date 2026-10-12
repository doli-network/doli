//! INC-I-237 M4 (REQ-GRV-016) — the channel series exist on /metrics and
//! every gossip metric has a reader in the probe script (INC-I-232 shape).
//!
//! covers: metrics_gossip metrics event_sink event_lanes verdict inc-i-237-m0-latency-probe
//!
//! OUTPUT CONTRACT: inc-i-237-m0-latency-probe.sh --file PATH (channel section)
//!   O1: none   O2: none   O4: none (reads only)   O5: none   O6: none
//!   O3: stdout lines starting with `channel ` + exit status:
//!       `channel lane=<l> depth=<n> inflight_bytes=<n> budget_bytes=<n> shed=<n>` per lane,
//!       `channel shed topic=<t> n=<n>` per topic series present,
//!       `channel control_dropped=<n>` (= doli_event_lane_shed_total{lane="backlog"})
//! OUTPUT CONTRACT: fn register_gossip_metrics / observe_event_lanes / scrape_for_exposition
//!   O5: REGISTRY families doli_event_lane_depth{lane}, doli_event_shed_by_topic_total{topic}
//! PATHS: P1 full fixture; P2 partial topic set; P3 registry after registration;
//!   P4 metric-name census vs probe text
//! INPUT PARTITIONS: lanes {3 present, any line order}; topic series {5, 2};
//!   noise families sharing a name prefix or a lane/topic label; control drop
//!   value distinct from every other shed value
//! MATRIX: O3 x P1-P2 (exact line sets), O5 x P3, census x P4 — all asserted.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

use super::inc_i_237_m0_common as c;
use doli_node::metrics_gossip::{observe_event_lanes, scrape_for_exposition};
use network::service::{LaneBudgets, LaneSnapshot};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn script() -> PathBuf {
    root().join("../../scripts/inc-i-237-m0-latency-probe.sh")
}

fn channel_lines(fixture: &str) -> BTreeSet<String> {
    let path = root().join("tests/fixtures/inc_i_237_m4").join(fixture);
    let out = Command::new("bash")
        .arg(script())
        .arg("--file")
        .arg(&path)
        .output()
        .expect("run bash");
    assert_eq!(out.status.code(), Some(0), "probe exit status");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let lines: Vec<String> = stdout
        .lines()
        .filter(|l| l.starts_with("channel "))
        .map(str::to_string)
        .collect();
    let set: BTreeSet<String> = lines.iter().cloned().collect();
    assert_eq!(set.len(), lines.len(), "duplicate channel lines:\n{stdout}");
    set
}

fn set(lines: &[&str]) -> BTreeSet<String> {
    lines.iter().map(|s| s.to_string()).collect()
}

// REQ-GRV-016 — Decision: a failure means the lane depth, in-flight bytes, budget, shed per lane, shed per topic or control-drop series have no reader, so a saturated swarm/node channel stays invisible (INC-I-232).
#[test]
fn req_grv_016_probe_prints_channel_section_per_lane_topic_and_control_drop() {
    let want = set(&[
        "channel lane=backlog depth=4 inflight_bytes=300 budget_bytes=67108864 shed=7",
        "channel lane=bulk depth=2 inflight_bytes=200 budget_bytes=16777216 shed=6",
        "channel lane=consensus depth=1 inflight_bytes=100 budget_bytes=67108864 shed=5",
        "channel shed topic=attestations n=3",
        "channel shed topic=blocks n=5",
        "channel shed topic=producers n=0",
        "channel shed topic=txs n=4",
        "channel shed topic=votes n=1",
        "channel control_dropped=7",
    ]);
    assert_eq!(channel_lines("channel.prom"), want);
}

// REQ-GRV-016 — Decision: a failure means the probe invents topic lines for series the node did not export, or drops lanes whose shed is zero, so the outcome-metric line count lies.
#[test]
fn req_grv_016_probe_prints_one_shed_line_per_topic_series_present() {
    let want = set(&[
        "channel lane=backlog depth=0 inflight_bytes=0 budget_bytes=2000 shed=0",
        "channel lane=bulk depth=8 inflight_bytes=80 budget_bytes=500 shed=12",
        "channel lane=consensus depth=9 inflight_bytes=90 budget_bytes=1000 shed=0",
        "channel shed topic=txs n=10",
        "channel shed topic=votes n=2",
        "channel control_dropped=0",
    ]);
    assert_eq!(channel_lines("channel_partial.prom"), want);
}

// REQ-GRV-016 — Decision: a failure means /metrics lacks a per-lane depth gauge or a per-topic shed counter pre-initialised for all five topics, so a dashboard cannot tell an idle topic from a missing series.
#[test]
fn req_grv_016_registry_exports_lane_depth_and_per_topic_shed_series() {
    let _g = c::lock();
    let mut topics = c::label_values("doli_event_shed_by_topic_total", "topic");
    topics.sort();
    assert_eq!(
        topics,
        ["attestations", "blocks", "producers", "txs", "votes"],
        "per-topic shed series must exist from registration"
    );
    observe_event_lanes(&LaneSnapshot::default(), &LaneBudgets::for_max_peers(8));
    scrape_for_exposition();
    let mut lanes = c::label_values("doli_event_lane_depth", "lane");
    lanes.sort();
    assert_eq!(lanes, ["backlog", "bulk", "consensus"]);
}

fn gossip_metric_names() -> BTreeSet<String> {
    let dir = root().join("src");
    let mut names = BTreeSet::new();
    for e in std::fs::read_dir(&dir).expect("bins/node/src") {
        let p = e.expect("entry").path();
        let file = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !(file.starts_with("metrics_gossip") && file.ends_with(".rs")) {
            continue;
        }
        let src = std::fs::read_to_string(&p).expect("read");
        for lit in src.split('"').skip(1).step_by(2) {
            let ident = lit
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if lit.starts_with("doli_") && ident {
                names.insert(lit.to_string());
            }
        }
    }
    names
}

// REQ-GRV-016 — Decision: a failure means a gossip metric is exported with zero readers, the INC-I-232 shape the M1.5 lane gauges already repeated.
#[test]
fn req_grv_016_every_gossip_metric_has_a_probe_reader() {
    let names = gossip_metric_names();
    assert!(names.len() >= 10, "vacuous census: {names:?}");
    let probe = std::fs::read_to_string(script()).expect("probe script");
    let missing: Vec<&String> = names
        .iter()
        .filter(|n| !probe.contains(n.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "metrics without a probe reader: {missing:?}"
    );
}
