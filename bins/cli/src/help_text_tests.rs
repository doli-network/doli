//! Agent-help drift guards. Output contract: the rendered help string is the
//! only output; paths = long `--help` vs short `-h`; partitions = root, root
//! leaf, nested leaf. Every number is built from the code value it describes.

use clap::Parser;
use doli_core::consensus::{withdrawal_penalty_rate_with_quarter, MAX_BONDS_PER_PRODUCER};
use doli_core::types::{Slot, DECIMALS, UNITS_PER_COIN};
use doli_core::{
    attestation_minutes_per_epoch, attestation_qualification_threshold, Network, NetworkParams,
};

use crate::commands::Cli;
use crate::common::default_rpc_for_network;

fn render(args: &[&str]) -> String {
    let mut argv = vec!["doli"];
    argv.extend_from_slice(args);
    match Cli::try_parse_from(argv) {
        Ok(_) => panic!("{args:?} parsed instead of printing help"),
        Err(e) => {
            assert_eq!(e.kind(), clap::error::ErrorKind::DisplayHelp, "{args:?}");
            e.render().to_string()
        }
    }
}

fn long_help(path: &[&str]) -> String {
    let mut args = path.to_vec();
    args.push("--help");
    render(&args)
}

fn short_help(path: &[&str]) -> String {
    let mut args = path.to_vec();
    args.push("-h");
    render(&args)
}

fn root() -> String {
    long_help(&[])
}

fn mainnet() -> NetworkParams {
    NetworkParams::defaults(Network::Mainnet)
}

/// 806859 -> "806,859", the way the help text writes numbers.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn assert_has(text: &str, needle: &str, what: &str) {
    assert!(
        text.contains(needle),
        "{what}: missing {needle:?}\n--- rendered ---\n{text}"
    );
}

fn assert_lacks(text: &str, needle: &str, what: &str) {
    assert!(
        !text.contains(needle),
        "{what}: still contains {needle:?}\n--- rendered ---\n{text}"
    );
}

/// The DANGER block of a long help, from the `DANGER:` line to the end.
fn danger_block(path: &[&str]) -> String {
    let help = long_help(path);
    let start = help
        .find("\nDANGER:\n")
        .unwrap_or_else(|| panic!("{path:?}: no DANGER: line\n{help}"));
    help[start + 1..].to_string()
}

/// Penalty percent at bond ages 0, 1, 2 and 3 quarters.
fn ladder() -> [u8; 4] {
    let q = Slot::try_from(mainnet().vesting_quarter_slots).expect("quarter fits a slot");
    [0, 1, 2, 3].map(|k| withdrawal_penalty_rate_with_quarter(k * q, q))
}

fn ladder_line() -> String {
    let [p0, p1, p2, p3] = ladder();
    format!("under 1 year {p0}%, 1-2 years {p1}%, 2-3 years {p2}%, 3 years or more {p3}%")
}

const NEVER_RULES: [&str; 10] = [
    "  1. Do not withdraw bonds without checking the penalty first.",
    "  2. Do not run one producer key on two nodes at the same time.",
    "  3. Do not pass --fee to send or channel. It is in DOLI: --fee 1 pays",
    "  4. Do not put a public key where an address is expected.",
    "  5. Do not use \"doli spend\" without a change output.",
    "  6. Do not lock coins with a bare timelock(...) or hashlock(...).",
    "  7. Do not wipe or snap a data directory that holds wallet files.",
    "  8. Do not trust the 24 words as your only backup.",
    "  9. Do not run new, init, export, import-bls or bridge-swap where the output",
    " 10. Do not run \"doli upgrade\" without sudo on a host that runs the node as a",
];

#[test]
fn root_long_help_has_title_and_intro() {
    let help = root();
    assert_has(&help, "DOLI wallet and node CLI\n", "title");
    assert_has(
        &help,
        "DOLI is a proof-of-stake blockchain with a UTXO model.",
        "intro",
    );
}

#[test]
fn root_long_help_has_all_ten_never_rules() {
    let help = root();
    assert_has(&help, "NEVER DO THESE - EACH ONE CAN LOSE DOLI", "header");
    for rule in NEVER_RULES {
        assert_has(&help, rule, "NEVER rule");
    }
}

#[test]
fn root_long_help_has_sections_and_move_procedure() {
    let help = root();
    for section in [
        "FACTS\n",
        "MOVE A PRODUCER TO A NEW MACHINE\n",
        "EXIT CODES\n",
        "MORE\n",
    ] {
        assert_has(&help, section, "section");
    }
    assert_has(&help, "     doli restore\n", "move procedure");
    assert_has(
        &help,
        "     doli producer rotate-bls --yes\n",
        "move procedure",
    );
}

#[test]
fn root_short_help_omits_agent_rules() {
    let help = short_help(&[]);
    assert_has(&help, "DOLI wallet CLI", "short about");
    assert_lacks(&help, "NEVER DO THESE", "short help");
    assert_lacks(&help, "FACTS", "short help");
}

const DANGER_COMMANDS: &[&[&str]] = &[
    &["producer", "request-withdrawal"],
    &["producer", "exit"],
    &["send"],
    &["spend"],
    &["wipe"],
    &["snap"],
    &["restore"],
    &["new"],
    &["init"],
    &["export"],
    &["import"],
    &["import-bls"],
    &["address"],
    &["add-bls"],
    &["upgrade"],
    &["producer", "rotate-bls"],
    &["producer", "delegate"],
    &["producer", "revoke-delegation"],
    &["channel", "open"],
    &["pool", "add"],
    &["pool", "swap"],
    &["bridge-swap"],
    &["nft"],
];

#[test]
fn every_risky_command_has_a_danger_note() {
    for path in DANGER_COMMANDS {
        let block = danger_block(path);
        assert_has(&block, "Do this:", &format!("{path:?}"));
    }
}

#[test]
fn danger_notes_fit_80_columns_and_stay_out_of_short_help() {
    for path in DANGER_COMMANDS {
        for line in danger_block(path).lines() {
            assert!(line.chars().count() <= 80, "{path:?}: line > 80: {line:?}");
        }
        assert_lacks(&short_help(path), "DANGER:", &format!("{path:?} -h"));
    }
}

#[test]
fn danger_notes_carry_the_key_facts() {
    let p = mainnet();
    let gate = grouped(p.inc_i_257_unbonding_activation_height);
    let lock = grouped(p.unbonding_period);
    let fee = doli_cli::rotate_tx::rotate_bls_fee().to_string();
    let rot_fee = format!("{fee} base units");
    let minutes = format!(
        "{} of the {} minutes",
        attestation_qualification_threshold(p.blocks_per_reward_epoch),
        attestation_minutes_per_epoch(p.blocks_per_reward_epoch)
    );
    let ladder = ladder_line();
    let cases: Vec<(&[&str], Vec<&str>)> = vec![
        (
            &["producer", "request-withdrawal"],
            vec![
                &ladder,
                "does not ask for confirmation",
                "burned",
                "simulate-withdrawal --count N",
                &gate,
                &lock,
                "doli1",
                "delegate",
            ],
        ),
        (
            &["producer", "exit"],
            vec![&ladder, "ALL", "dry run", "exits 0", &gate, &lock],
        ),
        (
            &["send"],
            vec![
                "--fee 1", "vesting(", "timelock", "hashlock", "multisig", "doli1",
            ],
        ),
        (
            &["spend"],
            vec!["NO change", "--output", "--yes", "input minus", "64-hex"],
        ),
        (
            &["wipe"],
            vec![
                "ignores",
                "mainnet",
                "--network",
                "keys, .env, wallet.json, wallet.seed.txt,",
                "node_key, config.toml",
            ],
        ),
        (
            &["snap"],
            vec!["does not ask", "BEFORE", "--trust", "node_key, config.toml"],
        ),
        (
            &["restore"],
            vec!["version 3", "0 rewards", "rotate-bls --yes", &rot_fee],
        ),
        (&["new"], vec!["24 words", ".seed.txt", "logged"]),
        (&["init"], vec!["24 words", ".seed.txt", "--force"]),
        (&["export"], vec!["plain text", "--force"]),
        (&["import"], vec!["--force", "lost"]),
        (
            &["import-bls"],
            vec!["shell history", "--force", "0 rewards", "wallet.json"],
        ),
        (&["address"], vec!["RANDOM", "wallet.json"]),
        (&["add-bls"], vec!["24 words", "wallet.json"]),
        (
            &["upgrade"],
            vec![
                "sudo",
                "restarts",
                "one node at a time",
                "maintainer_state.bin",
            ],
        ),
        (
            &["producer", "rotate-bls"],
            vec![
                &rot_fee,
                "irreversible",
                "next epoch boundary",
                "wallet.json",
            ],
        ),
        (&["producer", "delegate"], vec![&minutes]),
        (&["producer", "revoke-delegation"], vec![&minutes]),
        (
            &["channel", "open"],
            vec!["NO timeout", "2-of-2", "forever", "--fee 1"],
        ),
        (&["pool", "add"], vec!["existing providers", "pool info"]),
        (&["pool", "swap"], vec!["--min-out", "0"]),
        (&["bridge-swap"], vec!["preimage", "logged"]),
        (&["nft"], vec!["--sell-sign", "wallet file", "--to"]),
    ];
    for (path, needles) in cases {
        let block = danger_block(path);
        for needle in needles {
            assert_has(&block, needle, &format!("{path:?} DANGER"));
        }
    }
}

#[test]
fn drift_units_and_bond_unit() {
    let help = root();
    let units = format!(
        "1 DOLI = {} base units ({DECIMALS} decimals).",
        grouped(UNITS_PER_COIN)
    );
    assert_has(&help, &units, "units");
    let bond = mainnet().bond_unit;
    assert_eq!(bond % UNITS_PER_COIN, 0, "bond unit is a whole DOLI");
    let line = format!("1 bond = {} DOLI on mainnet.", bond / UNITS_PER_COIN);
    assert_has(&help, &line, "bond unit");
}

#[test]
fn drift_max_bonds() {
    let max = u64::from(MAX_BONDS_PER_PRODUCER);
    let line = format!("Maximum {} bonds per producer.", grouped(max));
    assert_has(&root(), &line, "max bonds");
    let range = format!("1-{max}");
    assert_has(&long_help(&["producer", "register"]), &range, "register");
    assert_has(&long_help(&["producer", "add-bond"]), &range, "add-bond");
    assert_has(&long_help(&["producer", "delegate"]), &range, "delegate");
}

#[test]
fn drift_slot_and_epoch() {
    let p = mainnet();
    let epoch = p.blocks_per_reward_epoch;
    assert_eq!(epoch * p.slot_duration, 3_600, "1 epoch = 1 hour");
    let line = format!(
        "1 slot = {} s. 1 epoch = {epoch} blocks = 1 hour.",
        p.slot_duration
    );
    assert_has(&root(), &line, "slot and epoch");
}

#[test]
fn drift_block_reward_and_attestation_threshold() {
    let p = mainnet();
    assert_eq!(
        p.initial_reward % UNITS_PER_COIN,
        0,
        "reward is a whole DOLI"
    );
    let reward = format!(
        "Each block adds {} DOLI to a reward pool.",
        p.initial_reward / UNITS_PER_COIN
    );
    assert_has(&root(), &reward, "block reward");
    let minutes = format!(
        "least {} of the {} minutes.",
        attestation_qualification_threshold(p.blocks_per_reward_epoch),
        attestation_minutes_per_epoch(p.blocks_per_reward_epoch)
    );
    assert_has(&root(), &minutes, "attestation threshold");
}

#[test]
fn drift_coinbase_maturity() {
    let line = format!(
        "Rewards are spendable after {} blocks.",
        mainnet().coinbase_maturity
    );
    assert_has(&root(), &line, "maturity");
}

#[test]
fn drift_penalty_ladder_from_vesting_function() {
    let p = mainnet();
    let year = 365 * 24 * 3_600;
    assert_eq!(
        p.vesting_quarter_slots * p.slot_duration,
        year,
        "1 tier = 1 year"
    );
    let [p0, p1, p2, p3] = ladder();
    let tiers = format!("younger than 1 year loses {p0}%. 1-2 years: {p1}%. 2-3 years: {p2}%.");
    assert_has(&root(), &tiers, "ladder");
    assert_has(&root(), &format!("3 years or more: {p3}%."), "ladder tail");
}

#[test]
fn drift_unbonding_period_and_activation_gate() {
    let p = mainnet();
    assert_eq!(
        p.unbonding_period * p.slot_duration,
        7 * 24 * 3_600,
        "about 7 days"
    );
    let help = root();
    let lock = format!(
        "locked for {} blocks (about 7 days).",
        grouped(p.unbonding_period)
    );
    assert_has(&help, &lock, "unbonding period");
    let unbond = format!(
        "From mainnet height {} the payout is also",
        grouped(p.inc_i_257_unbonding_activation_height)
    );
    assert_has(&help, &unbond, "unbonding gate");
    let slash = format!(
        "From mainnet height {}, this burns 100% of the bonds",
        grouped(p.effective_slashing_activation_height())
    );
    assert_has(&help, &slash, "slashing gate");
}

#[test]
fn drift_rotate_bls_fee_and_refusal_text() {
    let fee = doli_cli::rotate_tx::rotate_bls_fee();
    let help = root();
    assert_has(
        &help,
        &format!("rotate-bls costs {fee} base units."),
        "rotate fee",
    );
    let refusal = doli_cli::rotate_tx::RotateTxError::SameKey.to_string();
    let quoted = "the wallet BLS key is already the key on chain";
    assert!(
        refusal.starts_with(quoted),
        "refusal text drifted: {refusal}"
    );
    assert_has(&help, &format!("\"{quoted}\""), "rotate refusal");
}

#[test]
fn drift_default_rpc_ports() {
    let port = |net| default_rpc_for_network(net).rsplit(':').next().unwrap();
    let line = format!(
        "Default {} (testnet :{}, devnet :{}).",
        default_rpc_for_network("mainnet"),
        port("testnet"),
        port("devnet")
    );
    assert_has(&root(), &line, "rpc defaults");
}

#[test]
fn drift_ladder_in_root_matches_withdrawal_note() {
    let [p0, ..] = ladder();
    assert_eq!(p0, 75, "first tier");
    assert_has(
        &danger_block(&["producer", "request-withdrawal"]),
        &ladder_line(),
        "note ladder",
    );
}

#[test]
fn wrong_help_texts_are_gone() {
    let rw = long_help(&["producer", "request-withdrawal"]);
    assert_lacks(&rw, "Withdraw bonds instantly", "request-withdrawal");
    assert_lacks(&rw, "(hex pubkey_hash)", "request-withdrawal");
    let revoke = long_help(&["producer", "revoke-delegation"]).to_lowercase();
    assert_lacks(&revoke, "unbonding delay", "revoke-delegation");
    let delegate = long_help(&["producer", "delegate"]);
    assert_lacks(&delegate, "(1-100)", "delegate");
    assert_lacks(&delegate, "public key (hex) or address", "delegate");
    assert_lacks(&long_help(&["producer", "register"]), "1-10000", "register");
    assert_lacks(&long_help(&["producer", "add-bond"]), "1-10000", "add-bond");
    let slash = long_help(&["producer", "slash"]);
    assert_lacks(&slash, "Submit slashing evidence", "slash about");
    assert_has(&slash, "submits nothing", "slash about");
    let send = long_help(&["send"]);
    assert_lacks(
        &send,
        "htlc(hex_hash, lock_height, expiry_height)",
        "send htlc",
    );
    assert_has(
        &send,
        "htlc(hex_hash, lock_height, expiry_height, refund_pubkey_hash)",
        "send htlc",
    );
    assert_lacks(&long_help(&["spend"]), "remaining goes to change", "spend");
}

#[test]
fn htlc_help_example_has_the_arity_the_parser_needs() {
    let h = "11".repeat(32);
    let three = format!("htlc({h}, 10, 20)");
    let four = format!("htlc({h}, 10, 20, {h})");
    assert!(crate::parsers::parse_condition(&three).is_err());
    assert!(crate::parsers::parse_condition(&four).is_ok());
}

#[test]
fn wrong_printed_texts_are_gone() {
    let delegation = include_str!("cmd_producer/delegation.rs");
    assert_lacks(
        delegation,
        "Unbonding delay applies after revocation",
        "delegation.rs",
    );
    let exit = include_str!("cmd_producer/exit.rs");
    assert_lacks(exit, "Can re-register like any new producer", "exit.rs");
    assert_has(exit, "This command submits nothing.", "exit.rs");
    let gate = grouped(mainnet().effective_slashing_activation_height());
    assert_has(exit, &format!("(mainnet {gate})"), "exit.rs slashing gate");
    let chain = include_str!("cmd_chain.rs");
    assert_lacks(
        chain,
        "distributed automatically via coinbase",
        "cmd_chain.rs",
    );
    assert_has(chain, "once per epoch from the reward pool", "cmd_chain.rs");
}

#[test]
fn grouped_formats_like_the_text() {
    assert_eq!(grouped(806_859), "806,859");
    assert_eq!(grouped(100_000_000), "100,000,000");
    assert_eq!(grouped(360), "360");
    assert_eq!(grouped(3_000), "3,000");
}

#[test]
fn docs_cli_md_mirrors_the_root_rules() {
    let docs = include_str!("../../../docs/cli.md");
    let rules = crate::help_text::ROOT_AFTER_LONG_HELP;
    let end = rules.find("\n\nMORE\n").expect("MORE section");
    assert_has(docs, "## Agent Safety Rules", "docs/cli.md");
    assert_has(docs, &rules[..end], "docs/cli.md mirror");
}

#[test]
fn root_exit_codes_do_not_list_a_partial_set() {
    let help = root();
    assert_has(
        &help,
        "Most commands exit 0 when you cancel their prompt.",
        "exit codes",
    );
    assert!(
        !help.contains("exits 0 for send, wipe, upgrade and init"),
        "the old partial list must be gone"
    );
}
