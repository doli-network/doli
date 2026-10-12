//! INC-I-237 M7.1 (REQ-GRV-026) — `scripts/inc-i-237-m0-latency-probe.sh`
//! reads `doli_gossip_duplicate_cache_entries` (the gauge's reader).
//!
//! covers: scripts/inc-i-237-m0-latency-probe.sh bins/node/src/metrics_gossip.rs
//!
//! OUTPUT CONTRACT: inc-i-237-m0-latency-probe.sh --file PATH (dup_cache line)
//!   O1: none   O2: none   O4: none (reads only)   O5: none   O6: none
//!   O3: exactly one stdout line starting with `dup_cache `:
//!       `dup_cache entries=<n>` (series present) or `dup_cache entries=absent`
//! PATHS: P1 series present; P2 series absent
//! INPUT PARTITIONS: HELP / TYPE lines; a family sharing the series' name
//!   prefix listed before it (noise)
//! MATRIX: O3 x P1-P2 — every cell asserted.

use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn dup_cache_lines(fixture: &str) -> Vec<String> {
    let out = Command::new("bash")
        .arg(root().join("../../scripts/inc-i-237-m0-latency-probe.sh"))
        .arg("--file")
        .arg(root().join("tests/fixtures").join(fixture))
        .output()
        .expect("run bash");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("dup_cache "))
        .map(str::to_string)
        .collect()
}

// REQ-GRV-026 — Decision: a failure means the duplicate-cache gauge has no reader, or the reader takes a prefix-sharing family's value, so an operator reads the wrong C1 size.
#[test]
fn req_grv_026_probe_prints_duplicate_cache_entries() {
    assert_eq!(
        dup_cache_lines("inc_i_237_m7/dup_cache.prom"),
        ["dup_cache entries=123456"]
    );
}

// REQ-GRV-026 — Decision: a failure means a node without the gauge (pre-M7 binary) prints nothing or a fake 0, so an operator cannot tell "not exported" from "empty cache".
#[test]
fn req_grv_026_probe_reports_absent_duplicate_cache_series() {
    assert_eq!(
        dup_cache_lines("inc_i_237_m4/channel.prom"),
        ["dup_cache entries=absent"]
    );
}
