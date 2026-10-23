//! INC-I-182 M2 — REQ-PAUTH-005 spec sync: the public record of the
//! producer-authorization decision (specs/protocol.md, docs/cli.md) names the
//! same gate, domain tag, refusal code, signed types and exit forms as the code.
//!
// covers: protocol cli producer_auth exit unbonding_tx
//
// OUTPUT CONTRACT: the shipped decision record, read from disk (pure file I/O),
// compared with values derived from code (no second hard-coded copy of a code value).
//   O1 specs/protocol.md producer-authorization section:
//      gate field (compile-time tie: NetworkParams field) and its per-network value,
//      domain tag (PRODUCER_AUTH_DOMAIN), refusal code (prefix of a check_producer_auth Err),
//      the tx types in the rule paragraph == the types check_producer_auth refuses at the gate
//   O2 the same section: Exit (refused by validate_transaction_with_utxos at every height)
//      recorded as unmineable; the zero-Bond-input self-exit shape; the Exited post-state
//      (ProducerStatus::Exited); the slash refusal code (literal read from slash_withdrawal.rs)
//   O3 docs/cli.md exit section: exit works when the wallet owns no live Bond UTXO
// PATHS: file absent -> panic naming the path | section heading absent -> panic naming it
//   | phrase absent -> assert naming file, section and phrase | all present -> pass
// MATRIX
//   O1 -> inc_i_182_m2_req_pauth_005_spec_rule_matches_code
//   O2 -> inc_i_182_m2_req_pauth_005_spec_records_m2_exit_forms
//   O3 -> inc_i_182_m2_req_pauth_005_cli_doc_records_exit_without_bond_utxo
// INPUT PARTITIONS:
//   TxType: every decodable value of 0..=255; networks: Mainnet, Testnet, Devnet
//   text: whitespace runs collapsed to one space; phrases matched case-insensitively,
//   code names (tx types, status, codes) matched case-sensitively

use std::collections::BTreeSet;
use std::path::PathBuf;

use crypto::KeyPair;
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{Transaction, TxType, PRODUCER_AUTH_DOMAIN};
use doli_core::validation::producer_auth::check_producer_auth;
use doli_core::validation::{validate_transaction_with_utxos, ValidationContext, ValidationError};
use doli_core::{Network, NetworkParams};
use storage::{ProducerStatus, UtxoSet};

const SPEC: &str = "specs/protocol.md";
const SPEC_HEADING: &str = "producer authorization";
const CLI_DOC: &str = "docs/cli.md";
const CLI_HEADING: &str = "exit producer set";
const SLASH_SRC: &str = "bins/node/src/node/validation_checks/slash_withdrawal.rs";
const SLASH_CODE: &str = "[ECON_WITHDRAWAL_SLASHED]";

fn read_repo(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("REQ-PAUTH-005: cannot read {}: {e}", path.display()))
}

/// The paragraphs of the section whose heading line contains `heading`
/// (case-insensitive), up to the next heading of the same or a higher level.
/// Lines inside fenced code blocks are never headings (`# comment` in bash).
fn section(rel: &str, heading: &str) -> Vec<String> {
    let doc = read_repo(rel);
    let lines: Vec<&str> = doc.lines().collect();
    let level = |l: &str| l.chars().take_while(|c| *c == '#').count();
    let start = lines
        .iter()
        .position(|l| level(l) > 0 && l.to_lowercase().contains(heading))
        .unwrap_or_else(|| panic!("REQ-PAUTH-005: {rel} has no heading containing {heading:?}"));
    let depth = level(lines[start]);
    let mut fenced = false;
    let mut body = Vec::new();
    for l in &lines[start + 1..] {
        if l.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced && level(l) > 0 && level(l) <= depth {
            break;
        }
        body.push(*l);
    }
    body.join("\n")
        .split("\n\n")
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|p| !p.is_empty())
        .collect()
}

fn words(text: &str) -> BTreeSet<&str> {
    text.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .collect()
}

fn assert_phrases(rel: &str, paragraphs: &[String], phrases: &[&str]) {
    let all = paragraphs.join(" ").to_lowercase();
    let missing: Vec<&&str> = phrases
        .iter()
        .filter(|p| !all.contains(&p.to_lowercase()))
        .collect();
    assert!(
        missing.is_empty(),
        "REQ-PAUTH-005: the section of {rel} does not record {missing:?}"
    );
}

fn all_types() -> Vec<TxType> {
    (0..=255u32).filter_map(TxType::from_u32).collect()
}

fn gate_ctx(height: u64) -> ValidationContext {
    ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, height)
}

fn junk(t: TxType) -> Transaction {
    Transaction {
        version: 1,
        tx_type: t,
        inputs: Vec::new(),
        outputs: Vec::new(),
        extra_data: vec![0xAB; 200],
    }
}

/// (types the code predicate refuses at the gate, the refusal code it prints).
fn code_rule() -> (BTreeSet<String>, String) {
    let ctx = gate_ctx(1_000).with_inc_i_182_producer_auth(0);
    let mut covered = BTreeSet::new();
    let mut code = None;
    for t in all_types() {
        if let Err(ValidationError::InvalidTransaction(m)) = check_producer_auth(&junk(t), &ctx) {
            covered.insert(format!("{t:?}"));
            code = m.find(']').map(|end| m[..=end].to_string());
        }
    }
    let code = code.expect("NON-VACUITY: check_producer_auth refuses some type at the gate");
    assert!(
        code.starts_with('['),
        "the refusal starts with a bracketed code: {code}"
    );
    (covered, code)
}

fn shown(v: u64) -> String {
    if v == u64::MAX {
        return "u64::MAX".into();
    }
    v.to_string()
}

// Requirement: REQ-PAUTH-005 (Should)
// Acceptance: the decision row (binding rule per tx type, gate) matches the code.
// REQ-PAUTH-005 — Decision: a failure means the recorded producer-auth rule drifted from the code (gate field or value, domain tag, refusal code, or the set of signed types), so the public decision record misleads anyone who builds or audits these transactions.
#[test]
fn inc_i_182_m2_req_pauth_005_spec_rule_matches_code() {
    let paras = section(SPEC, SPEC_HEADING);
    let all = paras.join(" ");
    let gate = "inc_i_182_producer_auth_activation_height";
    for (net, label) in [
        (Network::Mainnet, "mainnet"),
        (Network::Testnet, "testnet"),
        (Network::Devnet, "devnet"),
    ] {
        let v = NetworkParams::defaults(net).inc_i_182_producer_auth_activation_height;
        let want = format!("{label} `{}`", shown(v));
        assert!(
            all.contains(&want),
            "REQ-PAUTH-005: {SPEC} does not state the {gate} value {want:?}"
        );
    }
    assert!(
        all.contains(gate),
        "REQ-PAUTH-005: {SPEC} does not name {gate}"
    );
    let domain = std::str::from_utf8(PRODUCER_AUTH_DOMAIN).expect("domain tag is UTF-8");
    assert!(
        all.contains(domain),
        "REQ-PAUTH-005: {SPEC} does not name the domain tag {domain:?}"
    );
    let (covered, code) = code_rule();
    assert!(
        all.contains(&code),
        "REQ-PAUTH-005: {SPEC} does not name the refusal code {code}"
    );
    let rule = paras
        .iter()
        .find(|p| p.contains("valid only when"))
        .unwrap_or_else(|| {
            panic!("REQ-PAUTH-005: {SPEC} has no rule paragraph (\"valid only when\")")
        });
    let names: BTreeSet<String> = all_types().iter().map(|t| format!("{t:?}")).collect();
    let listed: BTreeSet<String> = words(rule)
        .into_iter()
        .filter(|w| names.contains(*w))
        .map(str::to_string)
        .collect();
    assert_eq!(
        listed, covered,
        "REQ-PAUTH-005: the tx types the spec's rule paragraph lists as signed != the types check_producer_auth refuses"
    );
}

// Requirement: REQ-PAUTH-005 (Should)
// Acceptance: the record covers M2 — Exit unchanged, the producer's own zero-Bond-input exit,
// and the post-states after a full withdrawal and after a slash.
// REQ-PAUTH-005 — Decision: a failure means the spec does not record how a producer leaves under the rule (Exit stays unmineable, the zero-Bond-input self-exit, the Exited and slashed post-states), so the decision record no longer matches the shipped behaviour.
#[test]
fn inc_i_182_m2_req_pauth_005_spec_records_m2_exit_forms() {
    let exit = Transaction::new_exit(*KeyPair::generate().public_key());
    for h in [0, 1_000, u64::MAX - 1] {
        let r = validate_transaction_with_utxos(&exit, &gate_ctx(h), &UtxoSet::new());
        assert!(r.is_err(), "PRECONDITION: Exit is refused at height {h}");
    }
    let (covered, _) = code_rule();
    let exit_name = format!("{:?}", TxType::Exit);
    assert!(
        !covered.contains(&exit_name),
        "PRECONDITION: Exit is outside the producer-auth rule"
    );
    let slash_src = read_repo(SLASH_SRC);
    assert!(
        slash_src.contains(SLASH_CODE),
        "PRECONDITION: {SLASH_SRC} still refuses with {SLASH_CODE}"
    );

    let paras = section(SPEC, SPEC_HEADING);
    let all = paras.join(" ");
    let exited = format!("{:?}", ProducerStatus::Exited);
    let mut missing = Vec::new();
    if !paras
        .iter()
        .any(|p| words(p).contains(exit_name.as_str()) && p.to_lowercase().contains("unmineable"))
    {
        missing.push(format!("{exit_name} + \"unmineable\" in one paragraph"));
    }
    if !words(&all).contains(exited.as_str()) {
        missing.push(format!("post-withdrawal status {exited}"));
    }
    if !all.contains(SLASH_CODE) {
        missing.push(format!("slash refusal {SLASH_CODE}"));
    }
    let lower = all.to_lowercase();
    for p in ["zero Bond inputs", "Normal inputs", "Unbonding output"] {
        if !lower.contains(&p.to_lowercase()) {
            missing.push(format!("phrase {p:?}"));
        }
    }
    assert!(
        missing.is_empty(),
        "REQ-PAUTH-005: the producer-authorization section of {SPEC} does not record: {missing:?}"
    );
}

// Requirement: REQ-PAUTH-005 (Should)
// Acceptance: docs/cli.md says `exit` works when the wallet owns no live Bond UTXO.
// REQ-PAUTH-005 — Decision: a failure means docs/cli.md still implies exit needs a live Bond UTXO, so an operator whose ledger shows bonds but whose wallet holds none believes the producer cannot leave.
#[test]
fn inc_i_182_m2_req_pauth_005_cli_doc_records_exit_without_bond_utxo() {
    let paras = section(CLI_DOC, CLI_HEADING);
    assert_phrases(CLI_DOC, &paras, &["no live Bond UTXO"]);
}
