//! INC-I-262 M1 — the desktop wallet builds Registration, AddBond and
//! RequestWithdrawal through the shared producer-tx builders, the producer-auth
//! digest exists once, and the CLI exit prints its summary from one pure function.
//!
// covers: lib producer_tx unbonding_tx exit withdrawal builder mod producer
// covers: crates/producer-tx/src/lib.rs crates/producer-tx/src/producer_tx.rs crates/producer-tx/src/unbonding_tx.rs
// covers: bins/cli/src/producer_tx.rs bins/cli/src/unbonding_tx.rs bins/cli/src/cmd_producer/exit.rs bins/cli/src/cmd_producer/withdrawal.rs
// covers: crates/wallet/src/tx_builder/builder.rs crates/wallet/src/tx_builder/mod.rs crates/wallet/src/lib.rs bins/gui/src/commands/producer.rs
//
// OUTPUT CONTRACT: source of bins/gui/src/commands/producer.rs (no test harness), fns register_producer, add_bonds, request_withdrawal
//   O1 register_producer calls build_signed_registration( and chain_genesis_hash(; no doli_core::Transaction literal; no inline input signing
//   O2 add_bonds / request_withdrawal never call sign_and_build( and pass the chain genesis_hash
// OUTPUT CONTRACT: source of crates/wallet/src/tx_builder/builder.rs, fns build_add_bond, build_request_withdrawal
//   O3 each calls the shared builder (build_add_bond_tx( / build_withdrawal_tx() and finalize_producer_tx(
// OUTPUT CONTRACT: every *.rs under crates/*/src and bins/*/src
//   O4 exactly one definition of PRODUCER_AUTH_DOMAIN; the tag literal only in its defining file; no digest in crates/wallet
// OUTPUT CONTRACT: source of bins/cli/src/cmd_producer/exit.rs, fn handle_exit
//   O5 prints exit_summary_lines(; no inline "You receive: {}" printer
// PATHS: P-GUI | P-WALLET | P-TREE | P-EXIT, each with a planted violation
// MATRIX
//   O1, O2 x P-GUI -> inc_i_262_m1_req_def_001_gui_builds_through_shared_builder
//   O3 x P-WALLET -> inc_i_262_m1_req_def_001_wallet_delegates_to_shared_builder
//   O4 x P-TREE -> inc_i_262_m1_req_def_001_one_producer_auth_digest
//   O5 x P-EXIT -> inc_i_262_m1_req_def_002_exit_prints_pure_summary
// INPUT PARTITIONS: source text today | planted old shape | planted second definition

use std::path::{Path, PathBuf};

const GUI: &str = include_str!("../../../gui/src/commands/producer.rs");
const WALLET_BUILDER: &str = include_str!("../../../../crates/wallet/src/tx_builder/builder.rs");
const EXIT: &str = include_str!("../../src/cmd_producer/exit.rs");
const TAG: &str = "DOLI-PRODUCER-AUTH-V1";
const DEFINER: &str = "crates/core/src/transaction/producer_auth.rs";

fn body<'a>(src: &'a str, start: &str, next: &[&str]) -> &'a str {
    let Some(at) = src.find(start) else {
        return "";
    };
    let rest = &src[at + start.len()..];
    let end = next
        .iter()
        .filter_map(|n| rest.find(n))
        .min()
        .unwrap_or(rest.len());
    &src[at..at + start.len() + end]
}

fn gui_violations(src: &str) -> Vec<String> {
    let next = ["#[tauri::command]"];
    let reg = body(src, "pub async fn register_producer", &next);
    let bond = body(src, "pub async fn add_bonds", &next);
    let wd = body(src, "pub async fn request_withdrawal", &next);
    let mut bad = Vec::new();
    let mut need = |ok: bool, what: &str| {
        if !ok {
            bad.push(what.to_string());
        }
    };
    need(
        !reg.is_empty() && !bond.is_empty() && !wd.is_empty(),
        "three commands found",
    );
    need(
        reg.contains("build_signed_registration("),
        "register: build_signed_registration(",
    );
    need(
        reg.contains("chain_genesis_hash("),
        "register: genesis parsed by chain_genesis_hash(",
    );
    need(
        !reg.contains("doli_core::Transaction {"),
        "register: no Transaction literal",
    );
    need(
        !reg.contains("signing_message_for_input"),
        "register: no inline input signing",
    );
    for (name, b) in [("add_bonds", bond), ("request_withdrawal", wd)] {
        need(
            !b.contains("sign_and_build("),
            &format!("{name}: no unsigned sign_and_build("),
        );
        need(
            b.contains("genesis_hash"),
            &format!("{name}: passes the chain genesis_hash"),
        );
    }
    bad
}

// REQ-DEF-001 — Decision: a failure names a GUI producer command that still builds its tx outside the shared builder, so it is refused once producer signatures are required.
#[test]
fn inc_i_262_m1_req_def_001_gui_builds_through_shared_builder() {
    let planted = "#[tauri::command]\npub async fn register_producer() { let tx = doli_core::Transaction { }; \
                   tx.signing_message_for_input(0); }\n#[tauri::command]\npub async fn add_bonds() { b.sign_and_build(&k); }\n\
                   #[tauri::command]\npub async fn request_withdrawal() { b.sign_and_build(&k); }\n";
    assert_eq!(
        gui_violations(planted).len(),
        8,
        "NON-VACUITY: the scan flags each part of the pre-fix shape: {:?}",
        gui_violations(planted)
    );
    let bad = gui_violations(GUI);
    assert!(bad.is_empty(), "bins/gui/src/commands/producer.rs: {bad:?}");
}

fn wallet_violations(src: &str) -> Vec<String> {
    let next = ["pub fn "];
    let mut bad = Vec::new();
    for (f, shared) in [
        ("pub fn build_add_bond", "build_add_bond_tx("),
        ("pub fn build_request_withdrawal", "build_withdrawal_tx("),
    ] {
        let b = body(src, f, &next);
        for call in [shared, "finalize_producer_tx("] {
            if !b.contains(call) {
                bad.push(format!("{f}: calls {call}"));
            }
        }
    }
    bad
}

// REQ-DEF-001 — Decision: a failure means a wallet AddBond or RequestWithdrawal entry point keeps its own payload or signing code instead of the shared builder the node accepts.
#[test]
fn inc_i_262_m1_req_def_001_wallet_delegates_to_shared_builder() {
    let planted = "pub fn build_add_bond() { builder.add_output(); }\npub fn build_request_withdrawal() { extra.push(0); }\n";
    assert_eq!(wallet_violations(planted).len(), 4, "NON-VACUITY");
    let bad = wallet_violations(WALLET_BUILDER);
    assert!(
        bad.is_empty(),
        "crates/wallet/src/tx_builder/builder.rs: {bad:?}"
    );
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            sources(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn src_files() -> Vec<(String, String)> {
    let r = root();
    let mut files = Vec::new();
    for top in ["crates", "bins"] {
        let Ok(rd) = std::fs::read_dir(r.join(top)) else {
            continue;
        };
        for e in rd.flatten() {
            sources(&e.path().join("src"), &mut files);
        }
    }
    files
        .into_iter()
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            let rel = p
                .strip_prefix(&r)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            Some((rel, text))
        })
        .collect()
}

fn digest_violations(files: &[(String, String)]) -> Vec<String> {
    let defs: Vec<&str> = files
        .iter()
        .filter(|(_, t)| {
            t.contains("const PRODUCER_AUTH_DOMAIN") || t.contains("static PRODUCER_AUTH_DOMAIN")
        })
        .map(|(p, _)| p.as_str())
        .collect();
    let mut bad = Vec::new();
    if defs != [DEFINER] {
        bad.push(format!("PRODUCER_AUTH_DOMAIN defined in {defs:?}"));
    }
    for (p, t) in files {
        if p != DEFINER && t.contains(TAG) {
            bad.push(format!("{p}: carries the producer-auth tag literal"));
        }
        if p.starts_with("crates/wallet/") && t.contains("fn producer_auth_digest") {
            bad.push(format!("{p}: second producer-auth digest"));
        }
    }
    bad
}

// REQ-DEF-001 — Decision: a failure means a second producer-auth digest or domain tag exists, which can drift from the one the node verifies.
#[test]
fn inc_i_262_m1_req_def_001_one_producer_auth_digest() {
    let files = src_files();
    assert!(
        files.iter().any(|(p, _)| p == DEFINER) && files.len() > 100,
        "NON-VACUITY: the scan reads the workspace sources ({} files)",
        files.len()
    );
    let mut planted = files.clone();
    planted.push((
        "crates/wallet/src/tx_builder/auth.rs".to_string(),
        format!(
            "pub const PRODUCER_AUTH_DOMAIN: &[u8] = b\"{TAG}\";\nfn producer_auth_digest() {{}}"
        ),
    ));
    assert_eq!(
        digest_violations(&planted).len(),
        3,
        "NON-VACUITY: a planted wallet copy is flagged"
    );
    let bad = digest_violations(&files);
    assert!(bad.is_empty(), "{bad:?}");
}

fn exit_violations(src: &str) -> Vec<&'static str> {
    let b = body(src, "async fn handle_exit", &["async fn handle_slash"]);
    let mut bad = Vec::new();
    if !b.contains("exit_summary_lines(") {
        bad.push("prints exit_summary_lines(");
    }
    if b.contains("\"You receive: {}\"") {
        bad.push("no inline You receive printer");
    }
    bad
}

// REQ-DEF-002 — Decision: a failure means the exit still prints its own summary, which reports 0 received while coins are locked.
#[test]
fn inc_i_262_m1_req_def_002_exit_prints_pure_summary() {
    let planted = "async fn handle_exit() { println!(\"You receive: {}\", x); }";
    assert_eq!(exit_violations(planted).len(), 2, "NON-VACUITY");
    let bad = exit_violations(EXIT);
    assert!(bad.is_empty(), "exit.rs handle_exit: {bad:?}");
}

const WITHDRAW: &str = include_str!("../../src/cmd_producer/withdrawal.rs");
const WITHDRAW_SUMMARY: [&str; 8] = [
    "\"Withdrawing {} bonds (FIFO — oldest first):\"",
    "\"Destination: {}\"",
    "\"You receive: {}\", format_balance(breakdown.total_net)",
    "\"Penalty burned: {}\"",
    "\"WARNING: High penalty — {}% of bond value will be burned.\"",
    "\"No penalty — all bonds fully vested.\"",
    "\"Bonds remaining: {}\"",
    "\"Unbonding: the payout is locked until height {} (unbonding period {} blocks).\"",
];

// REQ-DEF-002 — Decision: a failure means the exit fix changed the summary the withdrawal command prints, which REQ-DEF-002 keeps byte-equal.
#[test]
fn inc_i_262_m1_req_def_002_golden_withdrawal_summary_text() {
    assert!(
        !"println!(\"You get: {}\")".contains(WITHDRAW_SUMMARY[2]),
        "NON-VACUITY: a reworded line is not matched"
    );
    let missing: Vec<&str> = WITHDRAW_SUMMARY
        .iter()
        .copied()
        .filter(|s| !WITHDRAW.contains(s))
        .collect();
    assert!(
        missing.is_empty(),
        "withdrawal.rs summary lines changed: {missing:?}"
    );
}
