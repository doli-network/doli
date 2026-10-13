//! INC-I-237 M4 — specs/docs describe the shipped relay rule (REQ-GRV-019) and
//! the design keeps its evolution path (REQ-GRV-018 Non-Foreclosure Check).
//!
//! covers: protocol security_model architecture troubleshooting gossip-relay-verdict-architecture
//!
//! OUTPUT CONTRACT: the five documents below (read-only scan)
//!   O1-O3: none   O4: document text   O5: none   O6: none
//! PATHS: P1 stale phrase present (drift); P2 required phrase absent (gap);
//!   P3 re-publish described without negation (regression)
//! INPUT PARTITIONS: case {upper, lower, mixed}; phrase spread {one line};
//!   document {protocol.md, security_model.md, troubleshooting.md,
//!   architecture.md, gossip-relay-verdict-architecture.md}
//! MATRIX: each document x its P1/P2/P3 checks — all asserted, every check
//!   first proves the document is non-empty (vacuous-read guard).

use std::path::PathBuf;

fn doc(rel: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p:?}: {e}"));
    assert!(text.lines().count() > 50, "{rel}: vacuous document");
    text
}

fn lines_with(text: &str, pred: impl Fn(&str) -> bool) -> Vec<(usize, &str)> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| pred(&l.to_lowercase()))
        .map(|(i, l)| (i + 1, l))
        .collect()
}

fn show(hits: &[(usize, &str)]) -> String {
    hits.iter()
        .map(|(n, l)| format!("{n}: {}", l.chars().take(160).collect::<String>()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// True when `fail open` / `fails open` / `fail-open` is followed by `accept` in the same sentence.
fn fail_open_accept(l: &str) -> bool {
    ["fails open", "fail open", "fail-open", "fails-open"]
        .iter()
        .any(|p| {
            l.match_indices(p).any(|(i, _)| {
                let rest = &l[i + p.len()..];
                let sentence = rest.split(". ").next().unwrap_or(rest);
                sentence.contains("accept")
            })
        })
}

// REQ-GRV-019 — Decision: a failure means protocol.md still documents the deleted 180 s app SeenCache as the storm-closer, so operators and reviewers reason about a dedup layer that no longer exists.
#[test]
fn req_grv_019_protocol_md_has_no_app_seen_cache() {
    let text = doc("specs/protocol.md");
    let hits = lines_with(&text, |l| {
        l.contains("seencache") || l.contains("seen_cache") || l.contains("seen cache")
    });
    assert!(hits.is_empty(), "SeenCache drift:\n{}", show(&hits));
}

// REQ-GRV-019 — Decision: a failure means protocol.md still documents a fail-open Accept or a "never silently Accepted" None-topic rule for non-block topics, which the D2 table replaced with Ignore/Defer.
#[test]
fn req_grv_019_protocol_md_has_no_non_block_fail_open_accept() {
    let text = doc("specs/protocol.md");
    let hits = lines_with(&text, |l| {
        fail_open_accept(l) || l.contains("never silently")
    });
    assert!(hits.is_empty(), "fail-open Accept drift:\n{}", show(&hits));
}

// REQ-GRV-019 — Decision: a failure means protocol.md again presents a node re-publish as the tx multi-hop mechanism, the path M2 removed (vendor refuses byte-identical re-publish as Duplicate).
#[test]
fn req_grv_019_protocol_md_never_presents_republish_as_relay() {
    let text = doc("specs/protocol.md");
    let lower = text.to_lowercase();
    assert!(
        lower.contains("original message id"),
        "verdict-on-original-id rule absent"
    );
    let bad: Vec<String> = ["re-publish", "republish"]
        .iter()
        .flat_map(|p| lower.match_indices(p).map(|(i, _)| i).collect::<Vec<_>>())
        .filter(|&i| {
            let ctx = &lower[i.saturating_sub(40)..i];
            !(ctx.contains("no ") || ctx.contains("not ") || ctx.contains("never"))
        })
        .map(|i| lower[i.saturating_sub(60)..(i + 60).min(lower.len())].replace('\n', " "))
        .collect();
    assert!(bad.is_empty(), "un-negated re-publish:\n{}", bad.join("\n"));
}

// REQ-GRV-019 — Decision: a failure means security_model.md does not record INC-I-238 as an open item, so the residual gossip risk is invisible to the security review.
#[test]
fn req_grv_019_security_model_names_inc_i_238_open() {
    let text = doc("specs/security_model.md");
    let hits = lines_with(&text, |l| l.contains("inc-i-238") && l.contains("open"));
    assert!(!hits.is_empty(), "no line names INC-I-238 as open");
}

// REQ-GRV-019 — Decision: a failure means troubleshooting.md gives operators no path from a stalled relay to the late-verdict counter and its probe.
#[test]
fn req_grv_019_troubleshooting_names_late_counter_and_probe() {
    let text = doc("docs/troubleshooting.md");
    for token in [
        "doli_gossip_verdict_late_total",
        "inc-i-237-m0-latency-probe.sh",
    ] {
        assert!(text.contains(token), "troubleshooting.md lacks {token}");
    }
}

// REQ-GRV-019 — Decision: a failure means architecture.md does not describe the two event lanes and the verdict seam, so the swarm/node contract is undocumented outside the spec.
#[test]
fn req_grv_019_architecture_doc_names_lanes_and_verdict_seam() {
    let text = doc("docs/architecture.md");
    let lower = text.to_lowercase();
    for token in ["consensus lane", "bulk lane"] {
        assert!(lower.contains(token), "architecture.md lacks '{token}'");
    }
    assert!(
        text.contains("GossipVerdict") || text.contains("send_gossip_verdict"),
        "architecture.md lacks the verdict seam (GossipVerdict / send_gossip_verdict)"
    );
}

// REQ-GRV-019 — Decision: a failure means the architecture spec does not record what shipped (status, milestone SHAs) and what stays open, so the next session re-derives it from git.
#[test]
fn req_grv_019_architecture_spec_records_implementation_and_open_items() {
    let text = doc("specs/gossip-relay-verdict-architecture.md");
    assert!(text.contains("IMPLEMENTED"), "status IMPLEMENTED absent");
    let missing: Vec<&str> = [
        "4277c300",
        "07a8b122",
        "0ac1123e",
        "a5d7d4e1",
        "0e69eb5d",
        "F-12",
        "INC-I-238",
        "F2-bucket",
        "G3",
    ]
    .into_iter()
    .filter(|t| !text.contains(t))
    .collect();
    assert!(missing.is_empty(), "spec lacks {missing:?}");
}

// REQ-GRV-018 — Decision: a failure means the design no longer shows that thousands of producers, more topics, gossipsub >= 0.48 and announce-then-fetch fit the seam without rework.
#[test]
fn req_grv_018_architecture_spec_has_non_foreclosure_check() {
    let text = doc("specs/gossip-relay-verdict-architecture.md");
    let start = text
        .find("\n## Non-Foreclosure Check")
        .expect("section '## Non-Foreclosure Check' absent");
    let body = &text[start + 1..];
    let section = body[3..].find("\n## ").map_or(body, |e| &body[..e + 3]);
    let lower = section.to_lowercase();
    let need: [(&str, &[&str]); 4] = [
        ("thousands of producers", &["thousands", "producers"]),
        ("more topics", &["topic"]),
        ("gossipsub >= 0.48 / send_queue", &["0.48", "send_queue"]),
        ("announce-then-fetch", &["announce-then-fetch"]),
    ];
    for (item, tokens) in need {
        assert!(
            tokens.iter().all(|t| lower.contains(t)),
            "Non-Foreclosure Check does not name {item}"
        );
    }
}
