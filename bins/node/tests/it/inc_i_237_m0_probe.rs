//! INC-I-237 M0 (C9) — `scripts/inc-i-237-m0-latency-probe.sh` reads the M0
//! series correctly (the REQ-GRV-016 reader). Fixtures: tests/fixtures/inc_i_237_m0.
//!
//! OUTPUT CONTRACT: inc-i-237-m0-latency-probe.sh [--series-count] [--file PATH | URL]
//!   O1: none   O2: none
//!   O3: stdout lines (topic=, block_recv, attestation_coverage, tx_batch, GATE)
//!       and the exit status
//!   O4: none (reads only)   O5: none   O6: none
//! PATHS: P1 --file with data, gate PASS (p99 exactly 2 s), P2 gate FAIL,
//!   P3 all-zero histograms -> NO_DATA, P4 --series-count, P5 unreachable URL,
//!   P6 missing file
//! INPUT PARTITIONS: first-bucket interpolation (votes), mid-bucket (blocks p50),
//!   boundary p99 == 2 s (blocks), count 0 topic (producers), unrelated
//!   histogram with a `topic` label (noise)
//! MATRIX: O3 x P1-P6 — every cell asserted. Values are compared as floats
//!   (>= 4 decimals printed); expected numbers are hand-computed with
//!   histogram_quantile linear interpolation.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn script() -> PathBuf {
    root().join("../../scripts/inc-i-237-m0-latency-probe.sh")
}

fn fixture(name: &str) -> String {
    root()
        .join("tests/fixtures/inc_i_237_m0")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn run(args: &[&str]) -> (i32, String) {
    let out = Command::new("bash")
        .arg(script())
        .args(args)
        .output()
        .expect("run bash");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// `key=value` pairs of the first stdout line whose first token is `head`.
fn line(stdout: &str, head: &str) -> HashMap<String, String> {
    let l = stdout
        .lines()
        .find(|l| l.split_whitespace().next() == Some(head))
        .unwrap_or_else(|| panic!("no `{head}` line in:\n{stdout}"));
    l.split_whitespace()
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn topic(stdout: &str, t: &str) -> HashMap<String, String> {
    line(stdout, &format!("topic={t}"))
}

fn num(m: &HashMap<String, String>, k: &str, want: f64) {
    let got: f64 = m
        .get(k)
        .unwrap_or_else(|| panic!("missing {k} in {m:?}"))
        .parse()
        .expect("float");
    assert!(
        (got - want).abs() <= 1e-4 + want.abs() * 1e-3,
        "{k}: got {got}, want {want} ({m:?})"
    );
}

fn gate(stdout: &str) -> String {
    stdout
        .lines()
        .find(|l| l.starts_with("GATE "))
        .unwrap_or_else(|| panic!("no GATE line:\n{stdout}"))
        .to_string()
}

// REQ-GRV-016 — Decision: a failure means the probe misreads the histograms (wrong interpolation, wrong family, wrong bucket for le=2), so the operator's M0 gate verdict is wrong.
#[test]
fn req_grv_016_probe_reports_hand_computed_quantiles_and_pass() {
    let (code, out) = run(&["--file", &fixture("pass.prom")]);
    assert_eq!(code, 0, "{out}");
    for (t, count, p50, p99, le2s) in [
        ("attestations", 4.0, 0.005, 0.0099, 1.0),
        ("blocks", 100.0, 0.0375, 2.0, 0.99),
        ("txs", 10.0, 0.5, 0.99, 1.0),
        ("votes", 2.0, 0.0005, 0.00099, 1.0),
    ] {
        let m = topic(&out, t);
        num(&m, "count", count);
        num(&m, "p50", p50);
        num(&m, "p99", p99);
        num(&m, "le2s", le2s);
    }
    num(&topic(&out, "producers"), "count", 0.0);
    let br = line(&out, "block_recv");
    num(&br, "count", 10.0);
    num(&br, "p50", 1.0);
    num(&br, "p99", 2.95);
    let ac = line(&out, "attestation_coverage");
    num(&ac, "count", 4.0);
    num(&ac, "mean", 0.85);
    num(&ac, "p50", 0.9);
    let tb = line(&out, "tx_batch");
    for (k, v) in [
        ("msgs", 11.0),
        ("txs", 30.0),
        ("1", 7.0),
        ("2_5", 3.0),
        ("6_10", 1.0),
        ("11_25", 0.0),
        ("26_50", 0.0),
        ("51_100", 0.0),
        ("101_plus", 0.0),
    ] {
        num(&tb, k, v);
    }
    let g = gate(&out);
    assert!(
        g.contains(" PASS"),
        "p99 max exactly 2 s is a PASS (<= 2s): {g}"
    );
    num(&line(&out, "GATE"), "verdict_p99_max", 2.0);
}

// REQ-GRV-016 — Decision: a failure means a topic over the 2 s SLO is reported as passing, so M2/M3 ship past the F-8 gate.
#[test]
fn req_grv_016_probe_fails_the_gate_when_any_topic_p99_exceeds_two_seconds() {
    let (code, out) = run(&["--file", &fixture("fail.prom")]);
    assert_eq!(code, 0, "a FAIL verdict is a result, not an error: {out}");
    let m = topic(&out, "txs");
    num(&m, "count", 200.0);
    num(&m, "p50", 0.25);
    num(&m, "p99", 3.2);
    num(&m, "le2s", 0.95);
    let g = gate(&out);
    assert!(g.contains(" FAIL") && !g.contains(" PASS"), "{g}");
    num(&line(&out, "GATE"), "verdict_p99_max", 3.2);
}

// REQ-GRV-016 — Decision: a failure means an idle node (no gossip samples) reads as PASS, so a dead measurement passes the gate.
#[test]
fn req_grv_016_probe_reports_no_data_when_every_histogram_is_empty() {
    let (code, out) = run(&["--file", &fixture("nodata.prom")]);
    assert_eq!(code, 0, "{out}");
    let g = gate(&out);
    assert!(
        g.contains("NO_DATA") && !g.contains(" PASS") && !g.contains(" FAIL"),
        "{g}"
    );
}

// REQ-GRV-016 — Decision: a failure means the series-served check (milestone outcome) counts the wrong family, so a node without the M0 binary looks instrumented.
#[test]
fn req_grv_016_probe_series_count_counts_only_verdict_count_series() {
    let (code, out) = run(&["--series-count", "--file", &fixture("pass.prom")]);
    assert_eq!((code, out.trim()), (0, "5"), "{out}");
    let (code, out) = run(&["--series-count", "--file", &fixture("nodata.prom")]);
    assert_eq!(
        (code, out.trim()),
        (0, "5"),
        "zero-initialised series still count"
    );
}

// REQ-GRV-016 — Decision: a failure means an unreachable node or a typo'd path exits 0, so a cron/alert wrapper records "no data" as healthy.
#[test]
fn req_grv_016_probe_exits_two_when_the_source_is_unreachable() {
    let (code, out) = run(&["http://127.0.0.1:1/metrics"]);
    assert_eq!(code, 2, "unreachable URL: {out}");
    let (code, out) = run(&["--file", "/nonexistent/inc-i-237-m0.prom"]);
    assert_eq!(code, 2, "missing file: {out}");
}
