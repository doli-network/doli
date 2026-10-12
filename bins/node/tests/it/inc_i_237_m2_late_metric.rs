//! INC-I-237 M2 (R5 / C6) — the per-topic late-verdict counter is a node
//! metric WITH a reader: /metrics exports it and the M0 probe script reads it.
//!
//! OUTPUT CONTRACT: fn scrape_for_exposition() over network::service::LATE_VERDICTS;
//!   inc-i-237-m0-latency-probe.sh --file PATH
//!   O1: none   O2: none   O3: probe stdout `verdict_late total=N <topic>=N...`
//!   O4: none
//!   O5: REGISTRY series doli_gossip_verdict_late_total{topic} (process-global:
//!       writers hold `inc_i_237_m0_common::lock()`)
//!   O6: none
//! PATHS: P1 scrape after late verdicts on two topics -> every topic series ==
//!   the network counter; P2 probe over a late family; P3 probe without it
//! INPUT PARTITIONS: topics {txs (2 more), attestations (1 more), others
//!   unchanged}; probe source {late family present + a look-alike family,
//!   absent (pass.prom)}
//! MATRIX: O5 x P1 (5 topics), O3 x P2-P3 — asserted below.
//! M5 (AUDIT-P3-406): P4 a verdict refused by a full command channel -> the
//!   doli_gossip_verdict_dropped_total{topic} series counts it; all 5 topic
//!   series exist. Contract: docs/.workflow/inc-i-237-M5-test-contract.md §1g.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use super::inc_i_204_m0_common::exported_value;
use super::inc_i_237_m0_common as c;
use doli_node::metrics_gossip::scrape_for_exposition;
use network::gossip::staleness::GossipTopic;
use network::service::{send_gossip_verdict, Verdict, LATE_VERDICTS};
use network::GossipHandle;

const LATE: &str = "doli_gossip_verdict_late_total";
const DROPPED: &str = "doli_gossip_verdict_dropped_total";
const TOPICS: [GossipTopic; 5] = [
    GossipTopic::Blocks,
    GossipTopic::Transactions,
    GossipTopic::Attestations,
    GossipTopic::Votes,
    GossipTopic::Producers,
];

fn probe(fixture: &str) -> HashMap<String, f64> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = Command::new("bash")
        .arg(root.join("../../scripts/inc-i-237-m0-latency-probe.sh"))
        .arg("--file")
        .arg(root.join("tests/fixtures/inc_i_237_m0").join(fixture))
        .output()
        .expect("run bash");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let line = stdout
        .lines()
        .find(|l| l.split_whitespace().next() == Some("verdict_late"))
        .unwrap_or_else(|| panic!("no `verdict_late` line in:\n{stdout}"));
    line.split_whitespace()
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), v.parse().expect("number")))
        .collect()
}

// REQ-GRV-008 / R5 — Decision: a failure means late verdicts are counted in the swarm but never reach /metrics (or under the wrong topic), so the operator cannot see that a backlogged node stopped relaying.
#[test]
fn r5_late_verdict_counter_is_exported_per_topic() {
    let _g = c::lock();
    LATE_VERDICTS.record(GossipTopic::Transactions);
    LATE_VERDICTS.record(GossipTopic::Transactions);
    LATE_VERDICTS.record(GossipTopic::Attestations);
    scrape_for_exposition();
    for t in TOPICS {
        assert_eq!(
            exported_value(LATE, &[("topic", t.metric_label())]),
            Some(LATE_VERDICTS.get(t) as f64),
            "{} series must equal the swarm counter",
            t.metric_label()
        );
    }
    assert!(LATE_VERDICTS.get(GossipTopic::Transactions) >= 2);
}

// REQ-GRV-016 / R5 — Decision: a failure means the late counter has no reader (the probe ignores it, sums a look-alike family, or drops a topic), so the M2 late-verdict signal is invisible in the operator's gate run.
#[test]
fn r5_probe_reads_the_late_verdict_counter() {
    let m = probe("late.prom");
    for (k, want) in [
        ("total", 4.0),
        ("txs", 3.0),
        ("attestations", 1.0),
        ("votes", 0.0),
    ] {
        assert_eq!(m.get(k), Some(&want), "{k} in {m:?}");
    }
    let none = probe("pass.prom");
    assert_eq!(
        none.get("total"),
        Some(&0.0),
        "absent family reads as 0: {none:?}"
    );
}

// REQ-GRV-016 / REQ-GRV-SEC-003 (AUDIT-P3-406) — Decision: a failure means verdicts refused by a full swarm command channel never reach /metrics, so messages silently expiring unreported (valid content not relayed) stay invisible.
#[test]
fn m5_audit_p3_406_dropped_verdict_counter_is_exported_per_topic() {
    let _g = c::lock();
    let (cmd, _rx) = tokio::sync::mpsc::channel(1);
    let _held = cmd.try_reserve().expect("hold the only slot");
    let h = GossipHandle::new(
        GossipTopic::Votes,
        vec![0x40u8; 20].into(),
        network::PeerId::random(),
    );
    assert!(
        !send_gossip_verdict(&cmd, h, Verdict::Ignore),
        "setup: a full channel refuses"
    );
    scrape_for_exposition();
    let votes = exported_value(DROPPED, &[("topic", "votes")]);
    assert!(
        votes.is_some_and(|v| v >= 1.0),
        "AUDIT-P3-406: {DROPPED}{{topic=votes}} = {votes:?}"
    );
    for t in TOPICS {
        let v = exported_value(DROPPED, &[("topic", t.metric_label())]);
        assert!(
            v.is_some(),
            "{DROPPED} series missing for {}",
            t.metric_label()
        );
    }
}
