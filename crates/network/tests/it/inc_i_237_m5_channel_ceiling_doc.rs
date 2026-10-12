//! INC-I-237 M5 (REQ-GRV-007 / F-13) — the command channel's memory ceiling is
//! documented: count bound, largest item, theoretical and realistic figures.
//!
//! covers: architecture gossip-relay-verdict-architecture
//!
//! OUTPUT CONTRACT: docs/architecture.md (lines naming the command channel,
//!   +-4 lines) and the `| F-13 |` row of specs/gossip-relay-verdict-architecture.md
//!   O1-O3: none   O4: document text   O5/O6: none
//! PATHS: P1 every ceiling token present -> pass; P2 any token missing -> fail
//!   naming it; P3 planted text missing one token -> the checker rejects it
//! INPUT PARTITIONS: case {any}; document {architecture.md window, spec F-13 row}
//! MATRIX: each document x P1/P2, checker x P3 — asserted below.

use std::path::PathBuf;

/// Stable tokens (lower-case): count bound, max item per command, the
/// theoretical figure at max_peers 50, the realistic figure.
const CEILING: [&str; 8] = [
    "(max_peers * 4).clamp(2048, 32768)",
    "sendsyncresponse",
    "16 mib",
    "broadcastblock",
    "2,000,000",
    "32 gib",
    "request-paced",
    "own block per slot",
];

fn doc(rel: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p:?}: {e}"));
    assert!(text.lines().count() > 50, "{rel}: vacuous document");
    text
}

fn missing(text: &str) -> Vec<&'static str> {
    let t = text.to_lowercase();
    CEILING.iter().copied().filter(|k| !t.contains(k)).collect()
}

/// Fewest missing tokens over the windows around each `command channel` line.
fn best_window(text: &str) -> Vec<&'static str> {
    let lines: Vec<&str> = text.lines().collect();
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.to_lowercase().contains("command channel"))
        .map(|(i, _)| missing(&lines[i.saturating_sub(4)..(i + 5).min(lines.len())].join("\n")))
        .min_by_key(Vec::len)
        .unwrap_or_else(|| CEILING.to_vec())
}

// REQ-GRV-007 (F-13) — Decision: a failure means an operator or auditor cannot bound the command channel's memory from the docs (REQ-GRV-007 requires a documented ceiling per queue), or the documented figure drifted from `(max_peers * 4).clamp(2048, 32768)` x the largest command.
#[test]
fn m5_req_grv_007_command_channel_ceiling_is_documented() {
    let planted = "command channel: (max_peers * 4).clamp(2048, 32768) x SendSyncResponse \
        16 MiB / BroadcastBlock 2,000,000 B = 32 GiB; realistic: request-paced sync \
        responses + 1 own block per slot";
    assert!(
        missing(planted).is_empty(),
        "checker rejects a complete text"
    );
    assert_eq!(
        missing(&planted.replace("32 GiB", "lots")),
        vec!["32 gib"],
        "checker must name the dropped token"
    );

    let arch = best_window(&doc("docs/architecture.md"));
    let spec_text = doc("specs/gossip-relay-verdict-architecture.md");
    let row = spec_text
        .lines()
        .find(|l| l.starts_with("| F-13 |"))
        .expect("spec keeps its F-13 row");
    let spec = missing(row);
    assert!(
        arch.is_empty() && spec.is_empty(),
        "REQ-GRV-007 command-channel ceiling: architecture.md window missing {arch:?}; spec F-13 row missing {spec:?}"
    );
}
