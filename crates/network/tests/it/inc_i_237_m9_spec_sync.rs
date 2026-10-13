//! INC-I-237 M9 — REQ-GRV-032: specs, the redesign analysis and the network
//! skill describe the state shipped by M5-M8.
//!
//! OUTPUT CONTRACT: specs/{protocol,security_model,gossip-relay-verdict-status}.md,
//!   the redesign analysis, the network skill + index, docs/{architecture,
//!   troubleshooting}.md (read-only scan).  O4: document text; O1-O3, O5, O6: none
//! PATHS: P1 required token absent -> fail listing doc + token; P2 forbidden
//!   token present -> fail listing doc + token; P3 vacuous read (`doc`)
//! INPUT PARTITIONS: token source {derived from a code constant (cache caps,
//!   cap ÷ 180 s floor rate), literal (symbols, metric name)}; document {each
//!   file above}
//! MATRIX: every document x every token of its test, all asserted.

use std::path::Path;

use network::gossip::{GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES, PEER_SCORE_DELIVERIES_MAX_ENTRIES};

fn doc(rel: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../{rel}"));
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p:?}: {e}"));
    assert!(!text.trim().is_empty(), "{rel}: vacuous document");
    text
}

fn thousands(n: usize) -> String {
    match n {
        0..=999 => n.to_string(),
        _ => format!("{},{:03}", thousands(n / 1000), n % 1000),
    }
}

fn mismatches(rels: &[&str], tokens: &[&str], want: bool) -> Vec<String> {
    let mut out = Vec::new();
    for rel in rels {
        let text = doc(rel);
        for t in tokens {
            if text.contains(t) != want {
                out.push(format!("{rel}: {t:?}"));
            }
        }
    }
    out
}

// REQ-GRV-032 — Decision: a failure means a spec omits the shipped M7.2 cache caps, M8 zero_flow_auth, or the M6 AttestSeen "members × 15" bound, so audits reason about unbounded or unauthenticated paths.
#[test]
fn m9_req_grv_032_specs_state_m6_to_m8_bounds() {
    let dup = thousands(GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES);
    let del = thousands(PEER_SCORE_DELIVERIES_MAX_ENTRIES);
    let tokens = [dup.as_str(), del.as_str(), "zero_flow_auth", "× 15"];
    let rels = ["specs/protocol.md", "specs/security_model.md"];
    let gaps = mismatches(&rels, &tokens, true);
    assert!(gaps.is_empty(), "missing tokens:\n{}", gaps.join("\n"));
}

// REQ-GRV-032 — Decision: a failure means a spec still states the 180 s dedup floor as unconditional (REL-573-R-01), so a TTL or cap change reasons from a retention the M7.2 cap gives only up to cap ÷ 180 ids/s.
#[test]
fn m9_req_grv_032_specs_state_rate_dependent_retention() {
    let floor_rate = thousands(GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES / 180);
    let rels = ["specs/protocol.md", "specs/security_model.md"];
    let gaps = mismatches(&rels, &[floor_rate.as_str()], true);
    assert!(gaps.is_empty(), "missing tokens:\n{}", gaps.join("\n"));
}

// REQ-GRV-032 — Decision: a failure means an operator doc omits the dropped-verdict counter (REL-573-R-04), so verdicts refused by a full swarm command channel have no documented signal.
#[test]
fn m9_req_grv_032_docs_name_verdict_dropped_counter() {
    let rels = ["docs/architecture.md", "docs/troubleshooting.md"];
    let gaps = mismatches(&rels, &["doli_gossip_verdict_dropped_total"], true);
    assert!(gaps.is_empty(), "missing tokens:\n{}", gaps.join("\n"));
}

// REQ-GRV-032 — Decision: a failure means the status spec has no REQ-GRV-025 row, so its open/closed state is invisible to the release gate.
#[test]
fn m9_req_grv_032_status_spec_has_req_grv_025_row() {
    let rel = "specs/gossip-relay-verdict-status.md";
    let found = doc(rel).lines().any(|l| l.starts_with("| REQ-GRV-025"));
    assert!(found, "{rel}: missing line starting \"| REQ-GRV-025\"");
}

// REQ-GRV-032 — Decision: a failure means the redesign analysis does not name the M6-M8 regression tests, so a reader cannot trace those milestones to their evidence.
#[test]
fn m9_req_grv_032_analysis_names_m6_to_m8_tests() {
    let rels = ["docs/redesigns/gossip-relay-verdict-redesign-analysis.md"];
    let tokens = [
        "inc_i_237_m6_",
        "inc_i_237_m7",
        "inc_i_237_m8_",
        "tests_inc_i237_m8_admission",
    ];
    let gaps = mismatches(&rels, &tokens, true);
    assert!(gaps.is_empty(), "missing tokens:\n{}", gaps.join("\n"));
}

// REQ-GRV-032 — Decision: a failure means the network skill still routes agents to symbols deleted in M1-M4, so a lookup lands on code that no longer exists.
#[test]
fn m9_req_grv_032_skill_has_no_removed_symbols() {
    let rels = [
        ".claude/skills/network/SKILL.md",
        ".claude/skills/SKILLS-INDEX.md",
    ];
    let hits = mismatches(&rels, &["enqueue_or_shed", "broadcast_transaction"], false);
    assert!(hits.is_empty(), "forbidden tokens:\n{}", hits.join("\n"));
}
