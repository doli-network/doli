// ── OUTPUT CONTRACT ─────────────────────────────────────────────────
//
// | Function                | Input Partition                       | Path          | Expected Output                               |
// |-------------------------|---------------------------------------|---------------|-----------------------------------------------|
// | condition_to_cli_string | vault template output                 | roundtrip     | parse(serialize(vault)) == vault              |
// | condition_to_cli_string | escrow template output                | roundtrip     | parse(serialize(escrow)) == escrow            |
// | condition_to_cli_string | htlc_payment template output          | roundtrip     | parse(serialize(htlc)) == htlc                |
// | condition_to_cli_string | subscription template output          | roundtrip     | parse(serialize(sub)) == sub                  |
// | condition_to_cli_string | agent_allowance template output       | roundtrip     | parse(serialize(aa)) == aa                    |
// | condition_to_cli_string | escrow_loan template output           | roundtrip     | parse(serialize(el)) == el                    |
// | condition_to_cli_string | And(timelock, hashlock)               | and           | "and(timelock(..), hashlock(..))"             |
// | condition_to_cli_string | Or(sig, timelock)                     | or            | "or(multisig(1,..), timelock(..))"            |
// | condition_to_cli_string | Threshold{2,[h,t,s]}                  | threshold     | "threshold(2, hashlock(..), ..)"              |
// | condition_to_cli_string | AmountGuard standalone                | guard         | "amount_guard(5.00000000, 0)"                 |
// | condition_to_cli_string | OutputTypeGuard standalone             | guard         | "output_type_guard(normal, 0)"                |
// | condition_to_cli_string | RecipientGuard standalone              | guard         | "recipient_guard(<hex>, 0)"                   |
// | Cli::try_parse_from    | vault with all required args           | happy         | parse succeeds, Vault variant                 |
// | Cli::try_parse_from    | escrow with all required args          | happy         | parse succeeds, Escrow variant                |
// | Cli::try_parse_from    | htlc-payment with all args             | happy         | parse succeeds, HtlcPayment variant           |
// | Cli::try_parse_from    | subscription with all args             | happy         | parse succeeds, Subscription variant          |
// | Cli::try_parse_from    | agent-allowance with all args          | happy         | parse succeeds, AgentAllowance variant        |
// | Cli::try_parse_from    | escrow-loan with all args              | happy         | parse succeeds, EscrowLoan variant            |
// | Cli::try_parse_from    | vault missing --owner                  | error         | Err                                           |
// | Cli::try_parse_from    | escrow missing --parties               | error         | Err                                           |
// | Cli::try_parse_from    | escrow-loan missing --lender           | error         | Err                                           |
// | Cli::try_parse_from    | template no subcommand                 | error         | Err                                           |
// | escrow_loan template   | valid args                             | structure     | Or(And(AmtGuard,RecipGuard),And(Sig,TL))      |
// | escrow_loan encode     | encode/decode                          | roundtrip     | decode(encode(cond)) == cond                  |
// | evaluate(escrow_loan)  | repay >= min to lender at deadline     | repay_pass    | true                                          |
// | evaluate(escrow_loan)  | repay < min to lender                  | repay_fail    | false                                         |
// | evaluate(escrow_loan)  | before deadline, lender sig            | reclaim_fail  | false                                         |
// | evaluate(escrow_loan)  | wrong recipient                        | wrong_recip   | false                                         |
// | escrow_loan golden     | golden vector JSON                     | golden        | condition_hex + cli_string match golden       |
//
// ── INPUT PARTITIONS ───────────────────────────────────────────────
//
// Partition 1 — Template round-trips (6 templates x 1 test each)
// Partition 2 — Composition operators (3 tests: and, or, threshold)
// Partition 3 — Standalone guard variants (3 tests: amount, output_type, recipient)
// Partition 4 — Clap positive parsing (6 templates)
// Partition 5 — Clap error cases (7 tests)
// Partition 6 — Edge cases (3 tests: nested, signature, timelock_expiry)
// Partition 7 — Escrow-loan: structure, roundtrip, eval paths (6 tests)
// Partition 8 — Golden vector (1 test)
// ─────────────────────────────────────────────────────────────────────

use super::serialize::condition_to_cli_string;
use crate::commands::{Cli, Commands, TemplateCommands};
use crate::parsers::parse_condition;
use clap::Parser;
use crypto::hash::hash;

fn test_hash(val: u8) -> crypto::Hash {
    hash(&[val])
}

// ── Round-trip tests ────────────────────────────────────────────────

#[test]
fn roundtrip_vault() {
    let cond = doli_core::conditions::templates::vault(test_hash(1), test_hash(2), 1000);
    let s = condition_to_cli_string(&cond);
    assert_eq!(cond, parse_condition(&s).unwrap(), "vault: {}", s);
}

#[test]
fn roundtrip_escrow() {
    let parties: Vec<crypto::Hash> = (1..=3).map(test_hash).collect();
    let cond = doli_core::conditions::templates::escrow(parties, 2, 50000, test_hash(10));
    let s = condition_to_cli_string(&cond);
    assert_eq!(cond, parse_condition(&s).unwrap(), "escrow: {}", s);
}

#[test]
fn roundtrip_htlc_payment() {
    let cond = doli_core::conditions::templates::htlc_payment(test_hash(1), 100, 200, test_hash(2));
    let s = condition_to_cli_string(&cond);
    assert_eq!(cond, parse_condition(&s).unwrap(), "htlc: {}", s);
}

#[test]
fn roundtrip_subscription() {
    let cond =
        doli_core::conditions::templates::subscription(test_hash(1), 500_000_000, 0, 1000, 2000);
    let s = condition_to_cli_string(&cond);
    assert_eq!(cond, parse_condition(&s).unwrap(), "sub: {}", s);
}

#[test]
fn roundtrip_agent_allowance() {
    let cond = doli_core::conditions::templates::agent_allowance(
        test_hash(1),
        test_hash(2),
        100_000_000,
        0,
    );
    let s = condition_to_cli_string(&cond);
    assert_eq!(cond, parse_condition(&s).unwrap(), "aa: {}", s);
}

#[test]
fn roundtrip_escrow_loan() {
    let cond = doli_core::conditions::templates::escrow_loan(test_hash(1), 500_000_000, 10_000);
    let s = condition_to_cli_string(&cond);
    assert_eq!(cond, parse_condition(&s).unwrap(), "el: {}", s);
}

// ── Composition operators ───────────────────────────────────────────

#[test]
fn serialize_and_roundtrip() {
    let cond = doli_core::Condition::And(
        Box::new(doli_core::Condition::Timelock(100)),
        Box::new(doli_core::Condition::Hashlock(test_hash(5))),
    );
    let s = condition_to_cli_string(&cond);
    assert!(s.starts_with("and("), "bad prefix: {}", s);
    assert_eq!(cond, parse_condition(&s).unwrap());
}

#[test]
fn serialize_or_roundtrip() {
    let cond = doli_core::Condition::Or(
        Box::new(doli_core::Condition::multisig(
            1,
            vec![test_hash(1), test_hash(2)],
        )),
        Box::new(doli_core::Condition::Timelock(500)),
    );
    let s = condition_to_cli_string(&cond);
    assert!(s.starts_with("or("), "bad prefix: {}", s);
    assert_eq!(cond, parse_condition(&s).unwrap());
}

#[test]
fn serialize_threshold_roundtrip() {
    let cond = doli_core::Condition::Threshold {
        n: 2,
        conditions: vec![
            doli_core::Condition::Hashlock(test_hash(1)),
            doli_core::Condition::Timelock(100),
            doli_core::Condition::Hashlock(test_hash(3)),
        ],
    };
    let s = condition_to_cli_string(&cond);
    assert!(s.starts_with("threshold("), "bad prefix: {}", s);
    assert_eq!(cond, parse_condition(&s).unwrap());
}

// ── Standalone guards ───────────────────────────────────────────────

#[test]
fn serialize_amount_guard() {
    let cond = doli_core::Condition::amount_guard(500_000_000, 0);
    let s = condition_to_cli_string(&cond);
    assert_eq!(s, "amount_guard(5.00000000, 0)");
    assert_eq!(cond, parse_condition(&s).unwrap());
}

#[test]
fn serialize_output_type_guard() {
    let cond = doli_core::Condition::output_type_guard(doli_core::OutputType::Normal, 0);
    let s = condition_to_cli_string(&cond);
    assert_eq!(s, "output_type_guard(normal, 0)");
    assert_eq!(cond, parse_condition(&s).unwrap());
}

#[test]
fn serialize_recipient_guard() {
    let h = test_hash(42);
    let cond = doli_core::Condition::recipient_guard(h, 1);
    let s = condition_to_cli_string(&cond);
    assert!(
        s.starts_with("recipient_guard(") && s.contains(", 1)"),
        "bad: {}",
        s
    );
    assert_eq!(cond, parse_condition(&s).unwrap());
}

// ── Clap positive parsing ───────────────────────────────────────────

#[test]
fn clap_parse_vault_dry_run() {
    let cli = Cli::try_parse_from([
        "doli",
        "template",
        "vault",
        "--owner",
        "aaaa000000000000000000000000000000000000000000000000000000000001",
        "--cosigner",
        "bbbb000000000000000000000000000000000000000000000000000000000002",
        "--unlock-height",
        "1000",
    ])
    .unwrap();
    assert!(
        matches!(&cli.command, Commands::Template { command } if matches!(command, TemplateCommands::Vault { .. }))
    );
}

#[test]
fn clap_parse_escrow_dry_run() {
    let cli = Cli::try_parse_from(["doli", "template", "escrow",
        "--parties", "aa00000000000000000000000000000000000000000000000000000000000001,bb00000000000000000000000000000000000000000000000000000000000002",
        "--threshold", "2", "--timeout", "50000",
        "--refund", "cc00000000000000000000000000000000000000000000000000000000000003"]).unwrap();
    assert!(
        matches!(&cli.command, Commands::Template { command } if matches!(command, TemplateCommands::Escrow { .. }))
    );
}

#[test]
fn clap_parse_htlc_payment_dry_run() {
    let cli = Cli::try_parse_from([
        "doli",
        "template",
        "htlc-payment",
        "--hash",
        "abcd000000000000000000000000000000000000000000000000000000000001",
        "--lock",
        "100",
        "--expiry",
        "200",
        "--refund",
        "dd00000000000000000000000000000000000000000000000000000000000004",
    ])
    .unwrap();
    assert!(
        matches!(&cli.command, Commands::Template { command } if matches!(command, TemplateCommands::HtlcPayment { .. }))
    );
}

#[test]
fn clap_parse_subscription_dry_run() {
    let cli = Cli::try_parse_from([
        "doli",
        "template",
        "subscription",
        "--recipient",
        "ee00000000000000000000000000000000000000000000000000000000000005",
        "--amount",
        "500.0",
        "--output-index",
        "0",
        "--start",
        "1000",
        "--end",
        "2000",
    ])
    .unwrap();
    assert!(
        matches!(&cli.command, Commands::Template { command } if matches!(command, TemplateCommands::Subscription { .. }))
    );
}

#[test]
fn clap_parse_agent_allowance_dry_run() {
    let cli = Cli::try_parse_from([
        "doli",
        "template",
        "agent-allowance",
        "--agent",
        "ff00000000000000000000000000000000000000000000000000000000000006",
        "--recipient",
        "0000000000000000000000000000000000000000000000000000000000000007",
        "--amount",
        "100.0",
        "--output-index",
        "0",
    ])
    .unwrap();
    assert!(
        matches!(&cli.command, Commands::Template { command } if matches!(command, TemplateCommands::AgentAllowance { .. }))
    );
}

#[test]
fn clap_parse_escrow_loan_dry_run() {
    let cli = Cli::try_parse_from([
        "doli",
        "template",
        "escrow-loan",
        "--lender",
        "aaaa000000000000000000000000000000000000000000000000000000000001",
        "--repay-amount",
        "10.5",
        "--deadline",
        "50000",
    ])
    .unwrap();
    assert!(
        matches!(&cli.command, Commands::Template { command } if matches!(command, TemplateCommands::EscrowLoan { .. }))
    );
}

// ── Clap error cases ────────────────────────────────────────────────

#[test]
fn clap_vault_missing_owner_fails() {
    assert!(Cli::try_parse_from([
        "doli",
        "template",
        "vault",
        "--cosigner",
        "bbbb000000000000000000000000000000000000000000000000000000000002",
        "--unlock-height",
        "1000"
    ])
    .is_err());
}

#[test]
fn clap_escrow_missing_parties_fails() {
    assert!(Cli::try_parse_from([
        "doli",
        "template",
        "escrow",
        "--threshold",
        "2",
        "--timeout",
        "50000",
        "--refund",
        "cc00000000000000000000000000000000000000000000000000000000000003"
    ])
    .is_err());
}

#[test]
fn clap_htlc_missing_hash_fails() {
    assert!(Cli::try_parse_from([
        "doli",
        "template",
        "htlc-payment",
        "--lock",
        "100",
        "--expiry",
        "200",
        "--refund",
        "dd00000000000000000000000000000000000000000000000000000000000004"
    ])
    .is_err());
}

#[test]
fn clap_subscription_missing_amount_fails() {
    assert!(Cli::try_parse_from([
        "doli",
        "template",
        "subscription",
        "--recipient",
        "ee00000000000000000000000000000000000000000000000000000000000005",
        "--output-index",
        "0",
        "--start",
        "1000",
        "--end",
        "2000"
    ])
    .is_err());
}

#[test]
fn clap_agent_allowance_missing_agent_fails() {
    assert!(Cli::try_parse_from([
        "doli",
        "template",
        "agent-allowance",
        "--recipient",
        "0000000000000000000000000000000000000000000000000000000000000007",
        "--amount",
        "100.0",
        "--output-index",
        "0"
    ])
    .is_err());
}

#[test]
fn clap_escrow_loan_missing_lender_fails() {
    assert!(Cli::try_parse_from([
        "doli",
        "template",
        "escrow-loan",
        "--repay-amount",
        "10.5",
        "--deadline",
        "50000"
    ])
    .is_err());
}

#[test]
fn clap_template_no_subcommand_fails() {
    assert!(Cli::try_parse_from(["doli", "template"]).is_err());
}

// ── Edge cases ──────────────────────────────────────────────────────

#[test]
fn serialize_nested_guards_roundtrip() {
    let cond =
        doli_core::conditions::templates::subscription(test_hash(1), 1_000_000, 0, 500, 1500);
    let s = condition_to_cli_string(&cond);
    assert!(s.starts_with("and("), "sub should start with 'and(': {}", s);
    assert_eq!(cond, parse_condition(&s).unwrap());
}

#[test]
fn serialize_signature_roundtrip() {
    let cond = doli_core::Condition::Signature(test_hash(99));
    let s = condition_to_cli_string(&cond);
    assert!(s.starts_with("signature("), "bad prefix: {}", s);
    assert_eq!(cond, parse_condition(&s).unwrap());
}

#[test]
fn serialize_timelock_expiry_roundtrip() {
    let cond = doli_core::Condition::TimelockExpiry(999);
    assert_eq!(condition_to_cli_string(&cond), "timelock_expiry(999)");
    assert_eq!(cond, parse_condition("timelock_expiry(999)").unwrap());
}

// ═══════════════════════════════════════════════════════════════════════
// M4: Escrow-Loan (P7) — structure, roundtrip, eval, golden
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn escrow_loan_template_constructs_valid_condition_tree() {
    let lender = test_hash(1);
    let cond = doli_core::conditions::templates::escrow_loan(lender, 1_050_000_000, 50_000);
    match &cond {
        doli_core::Condition::Or(repay, reclaim) => {
            match repay.as_ref() {
                doli_core::Condition::And(ag, rg) => {
                    assert!(matches!(ag.as_ref(), doli_core::Condition::AmountGuard {
                        min_amount, output_index: 0 } if *min_amount == 1_050_000_000));
                    assert!(matches!(rg.as_ref(), doli_core::Condition::RecipientGuard {
                        expected_pubkey_hash, output_index: 0 } if *expected_pubkey_hash == lender));
                }
                _ => panic!("repay should be And(AmountGuard, RecipientGuard)"),
            }
            match reclaim.as_ref() {
                doli_core::Condition::And(sig, tl) => {
                    assert!(
                        matches!(sig.as_ref(), doli_core::Condition::Signature(h) if *h == lender)
                    );
                    assert!(matches!(
                        tl.as_ref(),
                        doli_core::Condition::Timelock(50_000)
                    ));
                }
                _ => panic!("reclaim should be And(Signature, Timelock)"),
            }
        }
        _ => panic!("escrow_loan should be Or"),
    }
    cond.validate().unwrap();
}

#[test]
fn escrow_loan_condition_round_trip() {
    let cond = doli_core::conditions::templates::escrow_loan(test_hash(1), 500_000_000, 10_000);
    let encoded = cond.encode().unwrap();
    assert_eq!(cond, doli_core::Condition::decode(&encoded).unwrap());
    let s = condition_to_cli_string(&cond);
    assert_eq!(cond, parse_condition(&s).unwrap(), "CLI rt: {}", s);
}

#[test]
fn escrow_loan_below_min_repayment_rejected() {
    let lender = test_hash(1);
    let cond = doli_core::conditions::templates::escrow_loan(lender, 1_000_000_000, 50_000);
    let tx = doli_core::Transaction::new_transfer(
        vec![],
        vec![doli_core::Output::normal(500_000_000, lender)],
    );
    let h = test_hash(99);
    let ctx = doli_core::conditions::EvalContext {
        current_height: 50_001,
        signing_hash: &h,
        transaction: Some(&tx),
    };
    let w = doli_core::conditions::Witness {
        or_branches: vec![false],
        ..Default::default()
    };
    assert!(!doli_core::conditions::evaluate(&cond, &w, &ctx, &mut 0));
}

#[test]
fn escrow_loan_correct_recipient_correct_amount_after_deadline_accepted() {
    let lender = test_hash(1);
    let cond = doli_core::conditions::templates::escrow_loan(lender, 1_000_000_000, 50_000);
    let tx = doli_core::Transaction::new_transfer(
        vec![],
        vec![doli_core::Output::normal(1_000_000_000, lender)],
    );
    let h = test_hash(99);
    let ctx = doli_core::conditions::EvalContext {
        current_height: 50_001,
        signing_hash: &h,
        transaction: Some(&tx),
    };
    let w = doli_core::conditions::Witness {
        or_branches: vec![false],
        ..Default::default()
    };
    assert!(doli_core::conditions::evaluate(&cond, &w, &ctx, &mut 0));
}

#[test]
fn escrow_loan_before_deadline_rejected() {
    let lender = test_hash(1);
    let cond = doli_core::conditions::templates::escrow_loan(lender, 1_000_000_000, 50_000);
    let tx = doli_core::Transaction::new_transfer(vec![], vec![]);
    let h = test_hash(99);
    let ctx = doli_core::conditions::EvalContext {
        current_height: 49_999,
        signing_hash: &h,
        transaction: Some(&tx),
    };
    let w = doli_core::conditions::Witness {
        or_branches: vec![true],
        ..Default::default()
    };
    assert!(!doli_core::conditions::evaluate(&cond, &w, &ctx, &mut 0));
}

#[test]
fn escrow_loan_wrong_recipient_rejected() {
    let lender = test_hash(1);
    let cond = doli_core::conditions::templates::escrow_loan(lender, 1_000_000_000, 50_000);
    let tx = doli_core::Transaction::new_transfer(
        vec![],
        vec![doli_core::Output::normal(1_000_000_000, test_hash(2))],
    );
    let h = test_hash(99);
    let ctx = doli_core::conditions::EvalContext {
        current_height: 50_001,
        signing_hash: &h,
        transaction: Some(&tx),
    };
    let w = doli_core::conditions::Witness {
        or_branches: vec![false],
        ..Default::default()
    };
    assert!(!doli_core::conditions::evaluate(&cond, &w, &ctx, &mut 0));
}

/// Golden vector — catches accidental wire-format changes.
/// If this test fails, you changed the condition encoding or CLI serializer.
#[test]
fn escrow_loan_golden_vector() {
    let cond = doli_core::conditions::templates::escrow_loan(test_hash(1), 1_050_000_000, 50_000);
    let encoded = cond.encode().unwrap();
    let condition_hex = hex::encode(&encoded);
    let cli_string = condition_to_cli_string(&cond);

    let golden_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/cmd_template/testdata/escrow_loan_golden.json"
    );
    // Generate golden file if missing (first run only)
    if !std::path::Path::new(golden_path).exists() {
        let golden_json = serde_json::json!({
            "lender_hash": test_hash(1).to_hex(),
            "repay_amount": 1_050_000_000u64,
            "deadline": 50_000u64,
            "condition_hex": condition_hex,
            "cli_string": cli_string,
        });
        std::fs::create_dir_all(std::path::Path::new(golden_path).parent().unwrap()).unwrap();
        std::fs::write(
            golden_path,
            serde_json::to_string_pretty(&golden_json).unwrap(),
        )
        .unwrap();
        eprintln!("Generated golden vector at {}", golden_path);
        return; // First run: generate only
    }
    let golden_content = std::fs::read_to_string(golden_path)
        .unwrap_or_else(|e| panic!("Missing golden vector at {}: {}", golden_path, e));
    let golden: serde_json::Value =
        serde_json::from_str(&golden_content).unwrap_or_else(|e| panic!("Bad golden JSON: {}", e));

    assert_eq!(
        golden["condition_hex"].as_str().unwrap(),
        condition_hex,
        "wire format break"
    );
    assert_eq!(
        golden["cli_string"].as_str().unwrap(),
        cli_string,
        "CLI format break"
    );
    assert_eq!(
        cond,
        doli_core::Condition::decode(
            &hex::decode(golden["condition_hex"].as_str().unwrap()).unwrap()
        )
        .unwrap()
    );
}
