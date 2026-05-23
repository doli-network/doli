// ── OUTPUT CONTRACT ─────────────────────────────────────────────────
//
// | Function               | Input Partition                      | Path         | Expected Output                              |
// |------------------------|--------------------------------------|--------------|----------------------------------------------|
// | condition_to_cli_string| vault template output                | roundtrip    | parse(serialize(vault)) == vault             |
// | condition_to_cli_string| escrow template output               | roundtrip    | parse(serialize(escrow)) == escrow           |
// | condition_to_cli_string| htlc_payment template output         | roundtrip    | parse(serialize(htlc)) == htlc               |
// | condition_to_cli_string| subscription template output         | roundtrip    | parse(serialize(sub)) == sub                 |
// | condition_to_cli_string| agent_allowance template output      | roundtrip    | parse(serialize(aa)) == aa                   |
// | condition_to_cli_string| And(timelock, hashlock)              | and          | "and(timelock(..), hashlock(..))"            |
// | condition_to_cli_string| Or(sig, timelock)                    | or           | "or(multisig(1,..), timelock(..))"           |
// | condition_to_cli_string| Threshold{2,[h,t,s]}                 | threshold    | "threshold(2, hashlock(..), ..)"             |
// | condition_to_cli_string| AmountGuard standalone               | guard        | "amount_guard(5.00000000, 0)"                |
// | condition_to_cli_string| OutputTypeGuard standalone            | guard        | "output_type_guard(normal, 0)"               |
// | condition_to_cli_string| RecipientGuard standalone             | guard        | "recipient_guard(<hex>, 0)"                  |
// | clap parsing           | vault with all required args         | happy        | parse succeeds                               |
// | clap parsing           | escrow with all required args        | happy        | parse succeeds                               |
// | clap parsing           | htlc-payment with all args           | happy        | parse succeeds                               |
// | clap parsing           | subscription with all args           | happy        | parse succeeds                               |
// | clap parsing           | agent-allowance with all args        | happy        | parse succeeds                               |
// | clap parsing           | vault missing --owner                | error        | clap error about required arg                |
// | clap parsing           | escrow missing --parties             | error        | clap error about required arg                |
// | clap parsing           | --send without --to                  | error        | error about missing --to                     |
// | clap parsing           | --send without --amount              | error        | error about missing --amount                 |
// | clap parsing           | template no subcommand               | error        | clap error: subcommand required              |
// | condition_to_cli_string| nested guards (subscription)         | nested       | parse(serialize(nested)) == nested           |
// | condition_to_cli_string| Signature round-trip via signature()            | asymmetric   | parse(serialize(sig)) == sig                |
// | condition_to_cli_string| TimelockExpiry standalone             | primitive    | "timelock_expiry(999)"                       |
//
// ── INPUT PARTITIONS ───────────────────────────────────────────────
//
// Partition 1 — Template round-trips (5 templates x 1 test each):
//   Each template function produces a distinct Condition tree shape.
//   Serializer must handle Or, And, Multisig, Hashlock, Timelock, TimelockExpiry,
//   AmountGuard, RecipientGuard nested in various combinations.
//
// Partition 2 — Composition operators (3 tests: and, or, threshold):
//   Each operator wraps sub-conditions. Tests verify the serializer handles
//   recursive descent and the parser handles the round-trip.
//
// Partition 3 — Standalone guard variants (3 tests: amount, output_type, recipient):
//   Each guard variant serializes differently (amount uses units_to_coins,
//   output_type uses type name mapping, recipient uses hex hash).
//
// Partition 4 — Clap positive parsing (5 templates):
//   Each subcommand has distinct required args. Tests verify clap accepts
//   well-formed invocations.
//
// Partition 5 — Clap error cases (5+ tests):
//   Missing required args, missing subcommand. Tests verify clap rejects
//   malformed invocations.
//
// Partition 6 — Edge cases (3 tests):
//   Nested guards, Signature asymmetry, TimelockExpiry standalone.
// ─────────────────────────────────────────────────────────────────────

use super::serialize::condition_to_cli_string;
use crate::commands::{Cli, Commands, TemplateCommands};
use crate::parsers::parse_condition;
use clap::Parser;
use crypto::hash::hash;

fn test_hash(val: u8) -> crypto::Hash {
    hash(&[val])
}

// ── Round-trip tests: condition_to_cli_string → parse_condition ──────

#[test]
fn roundtrip_vault() {
    let owner = test_hash(1);
    let cosigner = test_hash(2);
    let cond = doli_core::conditions::templates::vault(owner, cosigner, 1000);
    let s = condition_to_cli_string(&cond);
    let parsed = parse_condition(&s).expect("parse should succeed");
    assert_eq!(cond, parsed, "vault round-trip failed.\nSerialized: {}", s);
}

#[test]
fn roundtrip_escrow() {
    let parties: Vec<crypto::Hash> = (1..=3).map(test_hash).collect();
    let refund = test_hash(10);
    let cond = doli_core::conditions::templates::escrow(parties, 2, 50000, refund);
    let s = condition_to_cli_string(&cond);
    let parsed = parse_condition(&s).expect("parse should succeed");
    assert_eq!(cond, parsed, "escrow round-trip failed.\nSerialized: {}", s);
}

#[test]
fn roundtrip_htlc_payment() {
    let payment_hash = test_hash(1);
    let refund = test_hash(2);
    let cond = doli_core::conditions::templates::htlc_payment(payment_hash, 100, 200, refund);
    let s = condition_to_cli_string(&cond);
    let parsed = parse_condition(&s).expect("parse should succeed");
    assert_eq!(
        cond, parsed,
        "htlc_payment round-trip failed.\nSerialized: {}",
        s
    );
}

#[test]
fn roundtrip_subscription() {
    let recipient = test_hash(1);
    let cond =
        doli_core::conditions::templates::subscription(recipient, 500_000_000, 0, 1000, 2000);
    let s = condition_to_cli_string(&cond);
    let parsed = parse_condition(&s).expect("parse should succeed");
    assert_eq!(
        cond, parsed,
        "subscription round-trip failed.\nSerialized: {}",
        s
    );
}

#[test]
fn roundtrip_agent_allowance() {
    let agent = test_hash(1);
    let recipient = test_hash(2);
    let cond = doli_core::conditions::templates::agent_allowance(agent, recipient, 100_000_000, 0);
    let s = condition_to_cli_string(&cond);
    let parsed = parse_condition(&s).expect("parse should succeed");
    assert_eq!(
        cond, parsed,
        "agent_allowance round-trip failed.\nSerialized: {}",
        s
    );
}

// ── Composition operator serialization ──────────────────────────────

#[test]
fn serialize_and_roundtrip() {
    let cond = doli_core::Condition::And(
        Box::new(doli_core::Condition::Timelock(100)),
        Box::new(doli_core::Condition::Hashlock(test_hash(5))),
    );
    let s = condition_to_cli_string(&cond);
    assert!(s.starts_with("and("), "should start with 'and(': {}", s);
    let parsed = parse_condition(&s).expect("parse should succeed");
    assert_eq!(cond, parsed);
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
    assert!(s.starts_with("or("), "should start with 'or(': {}", s);
    let parsed = parse_condition(&s).expect("parse should succeed");
    assert_eq!(cond, parsed);
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
    assert!(
        s.starts_with("threshold("),
        "should start with 'threshold(': {}",
        s
    );
    let parsed = parse_condition(&s).expect("parse should succeed");
    assert_eq!(cond, parsed);
}

// ── Standalone guard serialization ──────────────────────────────────

#[test]
fn serialize_amount_guard() {
    let cond = doli_core::Condition::amount_guard(500_000_000, 0);
    let s = condition_to_cli_string(&cond);
    assert_eq!(s, "amount_guard(5.00000000, 0)");
    let parsed = parse_condition(&s).expect("parse should succeed");
    assert_eq!(cond, parsed);
}

#[test]
fn serialize_output_type_guard() {
    let cond = doli_core::Condition::output_type_guard(doli_core::OutputType::Normal, 0);
    let s = condition_to_cli_string(&cond);
    assert_eq!(s, "output_type_guard(normal, 0)");
    let parsed = parse_condition(&s).expect("parse should succeed");
    assert_eq!(cond, parsed);
}

#[test]
fn serialize_recipient_guard() {
    let h = test_hash(42);
    let cond = doli_core::Condition::recipient_guard(h, 1);
    let s = condition_to_cli_string(&cond);
    assert!(
        s.starts_with("recipient_guard("),
        "should start with 'recipient_guard(': {}",
        s
    );
    assert!(s.contains(", 1)"), "should contain output_index: {}", s);
    let parsed = parse_condition(&s).expect("parse should succeed");
    assert_eq!(cond, parsed);
}

// ── Clap parsing: positive cases ────────────────────────────────────

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
    ]);
    assert!(cli.is_ok(), "vault parse failed: {:?}", cli.err());
    if let Commands::Template { command } = &cli.unwrap().command {
        assert!(matches!(command, TemplateCommands::Vault { .. }));
    } else {
        panic!("Expected Template command");
    }
}

#[test]
fn clap_parse_escrow_dry_run() {
    let cli = Cli::try_parse_from([
        "doli",
        "template",
        "escrow",
        "--parties",
        "aa00000000000000000000000000000000000000000000000000000000000001,bb00000000000000000000000000000000000000000000000000000000000002",
        "--threshold",
        "2",
        "--timeout",
        "50000",
        "--refund",
        "cc00000000000000000000000000000000000000000000000000000000000003",
    ]);
    assert!(cli.is_ok(), "escrow parse failed: {:?}", cli.err());
    if let Commands::Template { command } = &cli.unwrap().command {
        assert!(matches!(command, TemplateCommands::Escrow { .. }));
    } else {
        panic!("Expected Template command");
    }
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
    ]);
    assert!(cli.is_ok(), "htlc-payment parse failed: {:?}", cli.err());
    if let Commands::Template { command } = &cli.unwrap().command {
        assert!(matches!(command, TemplateCommands::HtlcPayment { .. }));
    } else {
        panic!("Expected Template command");
    }
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
    ]);
    assert!(cli.is_ok(), "subscription parse failed: {:?}", cli.err());
    if let Commands::Template { command } = &cli.unwrap().command {
        assert!(matches!(command, TemplateCommands::Subscription { .. }));
    } else {
        panic!("Expected Template command");
    }
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
    ]);
    assert!(cli.is_ok(), "agent-allowance parse failed: {:?}", cli.err());
    if let Commands::Template { command } = &cli.unwrap().command {
        assert!(matches!(command, TemplateCommands::AgentAllowance { .. }));
    } else {
        panic!("Expected Template command");
    }
}

// ── Clap parsing: error cases ───────────────────────────────────────

#[test]
fn clap_vault_missing_owner_fails() {
    let cli = Cli::try_parse_from([
        "doli",
        "template",
        "vault",
        "--cosigner",
        "bbbb000000000000000000000000000000000000000000000000000000000002",
        "--unlock-height",
        "1000",
    ]);
    assert!(cli.is_err(), "vault should fail without --owner");
}

#[test]
fn clap_escrow_missing_parties_fails() {
    let cli = Cli::try_parse_from([
        "doli",
        "template",
        "escrow",
        "--threshold",
        "2",
        "--timeout",
        "50000",
        "--refund",
        "cc00000000000000000000000000000000000000000000000000000000000003",
    ]);
    assert!(cli.is_err(), "escrow should fail without --parties");
}

#[test]
fn clap_htlc_missing_hash_fails() {
    let cli = Cli::try_parse_from([
        "doli",
        "template",
        "htlc-payment",
        "--lock",
        "100",
        "--expiry",
        "200",
        "--refund",
        "dd00000000000000000000000000000000000000000000000000000000000004",
    ]);
    assert!(cli.is_err(), "htlc-payment should fail without --hash");
}

#[test]
fn clap_subscription_missing_amount_fails() {
    let cli = Cli::try_parse_from([
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
        "2000",
    ]);
    assert!(cli.is_err(), "subscription should fail without --amount");
}

#[test]
fn clap_agent_allowance_missing_agent_fails() {
    let cli = Cli::try_parse_from([
        "doli",
        "template",
        "agent-allowance",
        "--recipient",
        "0000000000000000000000000000000000000000000000000000000000000007",
        "--amount",
        "100.0",
        "--output-index",
        "0",
    ]);
    assert!(cli.is_err(), "agent-allowance should fail without --agent");
}

// ── Template help listing ───────────────────────────────────────────

#[test]
fn clap_template_no_subcommand_fails() {
    // `doli template` without a subcommand should fail (clap requires one)
    let cli = Cli::try_parse_from(["doli", "template"]);
    assert!(cli.is_err(), "template without subcommand should fail");
}

// ── Additional edge cases ───────────────────────────────────────────

#[test]
fn serialize_nested_guards_roundtrip() {
    // Complex: And(And(RecipientGuard, AmountGuard), And(Timelock, TimelockExpiry))
    let cond =
        doli_core::conditions::templates::subscription(test_hash(1), 1_000_000, 0, 500, 1500);
    let s = condition_to_cli_string(&cond);
    // Must be parseable
    let parsed = parse_condition(&s).expect("nested guard parse should succeed");
    assert_eq!(cond, parsed);
    // Must start with and(
    assert!(
        s.starts_with("and("),
        "subscription serialization should start with 'and(': {}",
        s
    );
}

#[test]
fn serialize_signature_roundtrip() {
    // Signature(h) -> signature(h) -> Signature(h)
    let h = test_hash(99);
    let cond = doli_core::Condition::Signature(h);
    let s = condition_to_cli_string(&cond);
    assert!(
        s.starts_with("signature("),
        "Signature should serialize as signature(...): {}",
        s
    );
    let parsed = parse_condition(&s).expect("parse should succeed");
    assert_eq!(cond, parsed, "Signature should round-trip exactly");
}

#[test]
fn serialize_timelock_expiry_roundtrip() {
    let cond = doli_core::Condition::TimelockExpiry(999);
    let s = condition_to_cli_string(&cond);
    assert_eq!(s, "timelock_expiry(999)");
    let parsed = parse_condition(&s).expect("parse should succeed");
    assert_eq!(cond, parsed);
}
