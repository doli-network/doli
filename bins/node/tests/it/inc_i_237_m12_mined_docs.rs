//! INC-I-237 M12.2 / AUDIT-P1-516 — the registry and specs state the real cost of
//! the mined check, and every admission site calls the one shared admission path.
//!
//! OUTPUT CONTRACT: text of docs/error-codes.md, docs/rpc_reference.md,
//!   specs/gossip-relay-verdict-status.md, specs/protocol.md, specs/security_model.md;
//!   non-comment code of validation_checks/mod.rs, rpc methods/transaction.rs and
//!   rpc methods/governance.rs
//!   O1: none   O2: none   O3: none   O4: file text (read-only)   O5: none   O6: none
//! PATHS: P1 each of the five files carries the cost phrase; P2 the MPTX014 row
//!   names DelegateBond as the mined subject and lists no maintainer change as one
//!   (its do-not-reach exclusion aside); P3 each of the three sites calls
//!   admit_zero_flow in code, not only in a comment
//! INPUT PARTITIONS: doc {registry, rpc reference, 3 specs}; site {gossip,
//!   sendTransaction, submitMaintainerChange}; line {code, comment}.
// covers: zero_flow_mined zero_flow_auth zero_flow_policy zero_flow_standing zero_flow_newness pool lib mod gossip_txs validation_checks network_events transaction governance error-codes rpc_reference gossip-relay-verdict-status protocol security_model

use std::path::Path;

const COST_PHRASE: &str = "checked after every other refusal; repeated copies hit a bounded cache";

const COST_DOCS: [&str; 5] = [
    "docs/error-codes.md",
    "docs/rpc_reference.md",
    "specs/gossip-relay-verdict-status.md",
    "specs/protocol.md",
    "specs/security_model.md",
];

const ADMISSION_SITES: [&str; 3] = [
    "bins/node/src/node/validation_checks/mod.rs",
    "crates/rpc/src/methods/transaction.rs",
    "crates/rpc/src/methods/governance.rs",
];

const MAINTAINER_EXCLUSION: &str = "AddMaintainer / RemoveMaintainer do not reach this check";

fn read_repo_file(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn code_occurrences(src: &str, needle: &str) -> usize {
    src.lines()
        .map(|line| {
            line.split("//")
                .next()
                .unwrap_or("")
                .matches(needle)
                .count()
        })
        .sum()
}

// REQ-GRV-041 — Decision: a failure means an operator reading the registry or specs is told a stale or cheaper cost model than the node actually runs.
#[test]
fn m12_mined_docs_state_the_real_cost_and_mptx014_scope() {
    for rel in COST_DOCS {
        assert!(
            read_repo_file(rel).contains(COST_PHRASE),
            "{rel} must state: {COST_PHRASE}"
        );
    }
    let registry = read_repo_file("docs/error-codes.md");
    let row = registry
        .lines()
        .find(|line| line.starts_with("| `MPTX014` |"))
        .expect("docs/error-codes.md has an MPTX014 row");
    assert!(
        row.contains("DelegateBond is already mined"),
        "the MPTX014 row must name DelegateBond as the mined subject: {row}"
    );
    let subject_text = row.replace(MAINTAINER_EXCLUSION, "");
    assert!(
        !subject_text.contains("AddMaintainer") && !subject_text.contains("RemoveMaintainer"),
        "the MPTX014 row must not list a maintainer change as mined: {row}"
    );
}

// REQ-GRV-SEC-016 — Decision: a failure means one admission site has its own check order again, so the mined body read can drift back in front of the cheaper refusals there (INV-VALIDATION-001).
#[test]
fn m12_mined_docs_every_admission_site_calls_admit_zero_flow() {
    for rel in ADMISSION_SITES {
        assert!(
            code_occurrences(&read_repo_file(rel), "admit_zero_flow(") >= 1,
            "{rel} must call admit_zero_flow in code"
        );
    }
}
