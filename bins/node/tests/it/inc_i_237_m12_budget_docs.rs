//! INC-I-237 M12.3 / REQ-GRV-043 — the registry, RPC and CLI references and the
//! specs name the read-budget refusal `MPTX017` and its budget, state the 1-bond
//! limit of the MPTX014 workaround, and list honest flow (f).
//!
//! OUTPUT CONTRACT: text of docs/error-codes.md, docs/rpc_reference.md, docs/cli.md,
//!   specs/protocol.md, specs/security_model.md
//!   O1: none   O2: none   O3: none   O4: file text (read-only)   O5: none   O6: none
//! PATHS: P1 MPTX017 registry row + RPC mention; P2 each spec names MPTX017 and
//!   "2 reads/s with burst 2" (not burst 2,000); P3 flow (a) row and cli.md state the
//!   1-bond limit and its workarounds; P4 flow (f) row with its workaround
//! INPUT PARTITIONS: doc {registry, rpc reference, cli reference, 2 specs};
//!   phrase case {as written, lowercased for prose}.
// covers: zero_flow_mined zero_flow_budget error_codes error-codes rpc_reference cli protocol security_model gossip-relay-verdict-status

use std::path::Path;

const BUDGET_CODE: &str = "MPTX017";
const BUDGET_PHRASE: &str = "2 reads/s with burst 2";
const BUDGET_SPECS: [&str; 2] = ["specs/protocol.md", "specs/security_model.md"];
const ONE_BOND_PHRASES: [&str; 4] = [
    "delegator with one bond",
    "same txid",
    "addbond first",
    "a different delegate",
];
const FLOW_F_PHRASES: [&str; 3] = [
    "`mptx016`",
    "will fail at the boundary",
    "revoke naming the queued delegate, then delegate",
];

fn read_repo_file(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// `phrase` occurs with no digit, `,<digit>` or `_<digit>` right after it.
fn has_exact(text: &str, phrase: &str) -> bool {
    text.match_indices(phrase).any(|(i, _)| {
        let rest = &text[i + phrase.len()..];
        let mut it = rest.chars();
        match (it.next(), it.next()) {
            (Some(c), _) if c.is_ascii_digit() => false,
            (Some(',' | '_'), Some(c)) if c.is_ascii_digit() => false,
            _ => true,
        }
    })
}

fn row<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.lines().find(|l| l.starts_with(prefix))
}

fn missing(text: &str, phrases: &[&str]) -> Vec<String> {
    let lower = text.to_lowercase();
    phrases
        .iter()
        .filter(|p| !lower.contains(*p))
        .map(|p| p.to_string())
        .collect()
}

// REQ-GRV-043 — Decision: a failure means an operator or RPC client who gets MPTX017 finds no registry row, RPC note or spec text naming it or its budget.
#[test]
fn m12_3_grv_043_docs_name_mptx017_and_the_read_budget() {
    let mut bad = Vec::new();
    let registry = read_repo_file("docs/error-codes.md");
    if row(&registry, "| `MPTX017` |").is_none() {
        bad.push("docs/error-codes.md: no `| `MPTX017` |` row".to_string());
    }
    if !read_repo_file("docs/rpc_reference.md").contains(BUDGET_CODE) {
        bad.push("docs/rpc_reference.md: no MPTX017".to_string());
    }
    for rel in BUDGET_SPECS {
        let text = read_repo_file(rel);
        if !text.contains(BUDGET_CODE) {
            bad.push(format!("{rel}: no MPTX017"));
        }
        if !has_exact(&text, BUDGET_PHRASE) {
            bad.push(format!("{rel}: no exact `{BUDGET_PHRASE}`"));
        }
    }
    assert!(bad.is_empty(), "REQ-GRV-043 MPTX017 docs: {bad:#?}");
}

// REQ-GRV-043 (REL-574-R1-02) — Decision: a failure means the registry or CLI reference still offers "use a different bond_count" to a 1-bond delegator, for whom no such tx exists.
#[test]
fn m12_3_grv_043_docs_state_the_one_bond_limit() {
    let mut bad = Vec::new();
    let registry = read_repo_file("docs/error-codes.md");
    match row(&registry, "| (a) ") {
        Some(a) => {
            let gaps = missing(a, &ONE_BOND_PHRASES);
            if !gaps.is_empty() {
                bad.push(format!(
                    "docs/error-codes.md flow (a) row lacks {gaps:?}: {a}"
                ));
            }
        }
        None => bad.push("docs/error-codes.md: no flow (a) row".to_string()),
    }
    let gaps = missing(&read_repo_file("docs/cli.md"), &ONE_BOND_PHRASES);
    if !gaps.is_empty() {
        bad.push(format!("docs/cli.md lacks {gaps:?}"));
    }
    assert!(bad.is_empty(), "REQ-GRV-043 one-bond limit: {bad:#?}");
}

// REQ-GRV-043 (REL-574-R1-03) — Decision: a failure means the registry hides the honest flow where a queued DelegateBond that fails at the boundary blocks a new one, and its workaround.
#[test]
fn m12_3_grv_043_error_codes_list_flow_f() {
    let registry = read_repo_file("docs/error-codes.md");
    let f = row(&registry, "| (f) ").expect("docs/error-codes.md has a flow (f) row");
    let gaps = missing(f, &FLOW_F_PHRASES);
    assert!(
        gaps.is_empty(),
        "REQ-GRV-043 flow (f) row lacks {gaps:?}: {f}"
    );
}
