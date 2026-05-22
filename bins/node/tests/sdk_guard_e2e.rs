// OUTPUT CONTRACT: validate_transaction_with_utxos(&tx, &ctx, &utxo_set) -> Result<(), ValidationError>
//
// Observable outputs:
//   1. Return value: Ok(()) when all conditions satisfied, Err(ValidationError) when any fails
//   2. No mutations: utxo_set is read-only, tx and ctx are immutable references
//
// Code paths:
//   P1: Guard condition evaluation PASSES → Ok(())
//   P2: Guard condition evaluation FAILS (RecipientGuard mismatch) → Err(InvalidSignature)
//   P3: Guard condition evaluation FAILS (AmountGuard mismatch) → Err(InvalidSignature)
//   P4: Guard condition evaluation FAILS (correct data at wrong output index) → Err(InvalidSignature)
//   P5: Guard-only condition with empty witness → Ok(()) (proves guards need no witness)
//
// INPUT PARTITIONS:
//   P1: {tx.outputs[0].amount >= min_amount AND tx.outputs[0].pubkey_hash == expected} → Ok
//   P2: {tx.outputs[0].pubkey_hash != expected, amount correct} → Err(InvalidSignature)
//   P3: {tx.outputs[0].amount < min_amount, pubkey_hash correct} → Err(InvalidSignature)
//   P4: {correct data at outputs[1] not outputs[0], guards reference idx=0} → Err(InvalidSignature)
//   P5: {guard-only cond, Witness::default(), matching outputs} → Ok (REQ-SDK-008)

//! SDK Guard Condition E2E Integration Tests
//!
//! Tests guard conditions (AmountGuard, RecipientGuard) through the real
//! `validate_transaction_with_utxos` pipeline. No mocks on the validation
//! logic — the same code path that runs on every node is exercised here.

use crypto::hash::hash_with_domain;
use crypto::{Hash, KeyPair};
use doli_core::conditions::{Condition, Witness};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::transaction::{Input, Output, OutputType, Transaction};
use doli_core::validation::{
    validate_transaction_with_utxos, UtxoInfo, UtxoProvider, ValidationContext,
};

// ============================================================
// TEST UTXO PROVIDER
// ============================================================

/// A minimal UtxoProvider that holds UTXOs for testing.
/// Keyed by (tx_hash, output_index).
struct TestUtxoSet {
    entries: Vec<(Hash, u32, UtxoInfo)>,
}

impl TestUtxoSet {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    fn insert(&mut self, tx_hash: Hash, output_index: u32, utxo: UtxoInfo) {
        self.entries.push((tx_hash, output_index, utxo));
    }
}

impl UtxoProvider for TestUtxoSet {
    fn get_utxo(&self, tx_hash: &Hash, output_index: u32) -> Option<UtxoInfo> {
        self.entries
            .iter()
            .find(|(h, idx, _)| h == tx_hash && *idx == output_index)
            .map(|(_, _, info)| info.clone())
    }
}

// ============================================================
// HELPERS
// ============================================================

/// DOLI amount unit: 1 DOLI = 100_000_000 base units (8 decimal places).
const UNITS_PER_DOLI: u64 = 100_000_000;

/// Fee buffer: generous allowance subtracted from change output so
/// total_output < total_input, satisfying the minimum fee check.
/// Actual minimum fee is ~BASE_FEE(1) + tx_size * FEE_PER_BYTE(1),
/// typically under 1000 units. 100_000 units provides ample margin.
const FEE_BUFFER: u64 = 100_000;

/// Build a devnet ValidationContext at the given height.
fn devnet_ctx(height: u64) -> ValidationContext {
    let params = ConsensusParams::devnet();
    let mut ctx = ValidationContext::new(params, Network::Devnet, 1_000_000, height);
    // sig_verification_height=0: require public_key on all inputs
    ctx.sig_verification_height = 0;
    ctx
}

/// Create a deterministic test address hash from a seed string.
fn test_addr(seed: &str) -> Hash {
    hash_with_domain(crypto::ADDRESS_DOMAIN, seed.as_bytes())
}

/// Build a conditioned UTXO output with a guard condition in extra_data.
/// Uses OutputType::Multisig (the standard conditioned type for guards).
fn make_guard_utxo(amount: u64, owner_hash: Hash, condition: &Condition) -> Output {
    Output::conditioned(OutputType::Multisig, amount, owner_hash, condition)
        .expect("condition encoding should succeed")
}

/// Build a spending transaction that references a single UTXO and produces
/// the given outputs. Sets an empty covenant witness (no signatures needed
/// for guard-only conditions).
fn build_spend_tx(prev_tx_hash: Hash, prev_output_index: u32, outputs: Vec<Output>) -> Transaction {
    let input = Input::new(prev_tx_hash, prev_output_index);
    let mut tx = Transaction::new_transfer(vec![input], outputs);

    // Set empty covenant witness for the single input.
    // Guard conditions need no witness data — they evaluate against tx.outputs.
    let empty_witness = Witness::default().encode();
    tx.set_covenant_witnesses(&[empty_witness]);

    // Set public_key on input (required post-sig-verification activation).
    // For guard-only conditions the pubkey is not checked, but the field
    // must be present to pass structural validation.
    let dummy_kp = KeyPair::generate();
    tx.inputs[0].public_key = Some(*dummy_kp.public_key());

    tx
}

/// Insert a conditioned UTXO into the test set and return the fake tx hash.
fn seed_utxo(
    utxo_set: &mut TestUtxoSet,
    amount: u64,
    owner_hash: Hash,
    condition: &Condition,
) -> Hash {
    let output = make_guard_utxo(amount, owner_hash, condition);
    let utxo_info = UtxoInfo {
        output,
        pubkey: None,
        spent: false,
    };
    // Use a deterministic hash so tests are reproducible
    let fake_tx_hash = hash_with_domain(b"TEST_TX", b"guard_utxo_seed");
    utxo_set.insert(fake_tx_hash, 0, utxo_info);
    fake_tx_hash
}

// ============================================================
// TEST 1: Positive round-trip — matching outputs satisfy guards
// ============================================================

/// REQ-SDK-010 positive case:
/// UTXO locked with and(amount_guard(100 DOLI, 0), recipient_guard(alice, 0)).
/// Spending tx has output[0] = normal(100 DOLI, alice).
/// Expected: validation passes.
#[test]
fn test_guard_positive_round_trip() {
    let alice = test_addr("alice");
    let min_amount = 100 * UNITS_PER_DOLI; // 100 DOLI

    // Lock condition: and(amount_guard(100 DOLI, idx=0), recipient_guard(alice, idx=0))
    let condition = Condition::And(
        Box::new(Condition::AmountGuard {
            min_amount,
            output_index: 0,
        }),
        Box::new(Condition::RecipientGuard {
            expected_pubkey_hash: alice,
            output_index: 0,
        }),
    );

    let mut utxo_set = TestUtxoSet::new();
    let utxo_amount = 200 * UNITS_PER_DOLI; // Fund with 200 DOLI
    let tx_hash = seed_utxo(&mut utxo_set, utxo_amount, alice, &condition);

    // Spend: output[0] pays 100 DOLI to alice, output[1] is change (minus fee)
    let change_addr = test_addr("change");
    let change_amount = utxo_amount - min_amount - FEE_BUFFER;
    let outputs = vec![
        Output::normal(min_amount, alice),
        Output::normal(change_amount, change_addr),
    ];
    let tx = build_spend_tx(tx_hash, 0, outputs);
    let ctx = devnet_ctx(10); // height 10, well past activation height 0

    let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_set);
    assert!(
        result.is_ok(),
        "Expected guard-conditioned spend to succeed, got: {:?}",
        result.err()
    );
}

// ============================================================
// TEST 2: Wrong recipient at index 0 → RecipientGuard fails
// ============================================================

/// REQ-SDK-010 negative case: wrong recipient.
/// UTXO locked with and(amount_guard(100 DOLI, 0), recipient_guard(alice, 0)).
/// Spending tx has output[0] = normal(100 DOLI, bob) — wrong recipient.
/// Expected: validation fails (RecipientGuard rejects).
#[test]
fn test_guard_wrong_recipient_rejected() {
    let alice = test_addr("alice");
    let bob = test_addr("bob");
    let min_amount = 100 * UNITS_PER_DOLI;

    let condition = Condition::And(
        Box::new(Condition::AmountGuard {
            min_amount,
            output_index: 0,
        }),
        Box::new(Condition::RecipientGuard {
            expected_pubkey_hash: alice,
            output_index: 0,
        }),
    );

    let mut utxo_set = TestUtxoSet::new();
    let utxo_amount = 200 * UNITS_PER_DOLI;
    let tx_hash = seed_utxo(&mut utxo_set, utxo_amount, alice, &condition);

    // Spend: output[0] pays correct amount but to BOB (wrong recipient)
    let change_amount = utxo_amount - min_amount - FEE_BUFFER;
    let outputs = vec![
        Output::normal(min_amount, bob), // <-- wrong: alice expected
        Output::normal(change_amount, alice),
    ];
    let tx = build_spend_tx(tx_hash, 0, outputs);
    let ctx = devnet_ctx(10);

    let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_set);
    assert!(result.is_err(), "Expected wrong-recipient spend to fail");
    // The error should be InvalidSignature (condition evaluation failed)
    let err = result.unwrap_err();
    let err_str = format!("{:?}", err);
    assert!(
        err_str.contains("InvalidSignature") || err_str.contains("Signature"),
        "Expected InvalidSignature error, got: {}",
        err_str
    );
}

// ============================================================
// TEST 3: Insufficient amount at index 0 → AmountGuard fails
// ============================================================

/// REQ-SDK-010 negative case: insufficient amount.
/// UTXO locked with and(amount_guard(100 DOLI, 0), recipient_guard(alice, 0)).
/// Spending tx has output[0] = normal(50 DOLI, alice) — amount too low.
/// Expected: validation fails (AmountGuard rejects).
#[test]
fn test_guard_insufficient_amount_rejected() {
    let alice = test_addr("alice");
    let min_amount = 100 * UNITS_PER_DOLI;

    let condition = Condition::And(
        Box::new(Condition::AmountGuard {
            min_amount,
            output_index: 0,
        }),
        Box::new(Condition::RecipientGuard {
            expected_pubkey_hash: alice,
            output_index: 0,
        }),
    );

    let mut utxo_set = TestUtxoSet::new();
    let utxo_amount = 200 * UNITS_PER_DOLI;
    let tx_hash = seed_utxo(&mut utxo_set, utxo_amount, alice, &condition);

    // Spend: output[0] pays only 50 DOLI to alice — too low
    let low_amount = 50 * UNITS_PER_DOLI;
    let change_amount = utxo_amount - low_amount - FEE_BUFFER;
    let outputs = vec![
        Output::normal(low_amount, alice), // <-- wrong: 100 DOLI min required
        Output::normal(change_amount, alice),
    ];
    let tx = build_spend_tx(tx_hash, 0, outputs);
    let ctx = devnet_ctx(10);

    let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_set);
    assert!(
        result.is_err(),
        "Expected insufficient-amount spend to fail"
    );
    let err = result.unwrap_err();
    let err_str = format!("{:?}", err);
    assert!(
        err_str.contains("InvalidSignature") || err_str.contains("Signature"),
        "Expected InvalidSignature error, got: {}",
        err_str
    );
}

// ============================================================
// TEST 4: Right amount + recipient but at WRONG output index
// ============================================================

/// REQ-SDK-010 negative case: wrong output index.
/// UTXO locked with and(amount_guard(100 DOLI, idx=0), recipient_guard(alice, idx=0)).
/// Spending tx has:
///   output[0] = normal(50 DOLI, bob)   — wrong recipient AND amount at idx 0
///   output[1] = normal(100 DOLI, alice) — correct but at idx 1 (guards check idx 0)
/// Expected: validation fails (both guards check index 0, find wrong data).
#[test]
fn test_guard_wrong_output_index_rejected() {
    let alice = test_addr("alice");
    let bob = test_addr("bob");
    let min_amount = 100 * UNITS_PER_DOLI;

    let condition = Condition::And(
        Box::new(Condition::AmountGuard {
            min_amount,
            output_index: 0,
        }),
        Box::new(Condition::RecipientGuard {
            expected_pubkey_hash: alice,
            output_index: 0,
        }),
    );

    let mut utxo_set = TestUtxoSet::new();
    let utxo_amount = 200 * UNITS_PER_DOLI;
    let tx_hash = seed_utxo(&mut utxo_set, utxo_amount, alice, &condition);

    // Spend: correct values at index 1, but guards check index 0
    let bob_amount = 50 * UNITS_PER_DOLI;
    let change_amount = utxo_amount - min_amount - bob_amount - FEE_BUFFER;
    let outputs = vec![
        Output::normal(bob_amount, bob),   // idx 0: wrong recipient + amount
        Output::normal(min_amount, alice), // idx 1: correct but at wrong index
        Output::normal(change_amount, alice), // change
    ];
    let tx = build_spend_tx(tx_hash, 0, outputs);
    let ctx = devnet_ctx(10);

    let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_set);
    assert!(result.is_err(), "Expected wrong-index spend to fail");
    let err = result.unwrap_err();
    let err_str = format!("{:?}", err);
    assert!(
        err_str.contains("InvalidSignature") || err_str.contains("Signature"),
        "Expected InvalidSignature error, got: {}",
        err_str
    );
}

// ============================================================
// TEST 5: Witness-free satisfaction (REQ-SDK-008)
// ============================================================

/// REQ-SDK-008: Guard-only condition with empty witness.
/// UTXO locked with and(amount_guard(50 DOLI, 0), recipient_guard(carol, 0)).
/// No Signature sub-condition — guards consume NO witness fields.
/// Spending tx provides Witness::default() (empty) + matching outputs.
/// Expected: validation passes, proving guards need no witness data.
#[test]
fn test_guard_witness_free_satisfaction() {
    let carol = test_addr("carol");
    let min_amount = 50 * UNITS_PER_DOLI;

    // Guard-only condition: no Signature, no Hashlock, no Timelock
    let condition = Condition::And(
        Box::new(Condition::AmountGuard {
            min_amount,
            output_index: 0,
        }),
        Box::new(Condition::RecipientGuard {
            expected_pubkey_hash: carol,
            output_index: 0,
        }),
    );

    let mut utxo_set = TestUtxoSet::new();
    let utxo_amount = 100 * UNITS_PER_DOLI;
    let tx_hash = seed_utxo(&mut utxo_set, utxo_amount, carol, &condition);

    // Spend with matching outputs and explicitly empty witness (minus fee)
    let change_amount = utxo_amount - min_amount - FEE_BUFFER;
    let outputs = vec![
        Output::normal(min_amount, carol),
        Output::normal(change_amount, carol),
    ];
    let tx = build_spend_tx(tx_hash, 0, outputs);
    let ctx = devnet_ctx(5);

    let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_set);
    assert!(
        result.is_ok(),
        "Guard-only condition should succeed with empty witness, got: {:?}",
        result.err()
    );
}
