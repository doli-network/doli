//! Supply invariant tests for the channels crate.
//!
//! Verifies that all transaction types constructed by the channels crate
//! preserve the DOLI supply invariant: sum(outputs) <= sum(inputs).

use channels::close::*;
use channels::commitment::CommitmentPair;
use channels::funding::build_funding_tx_with_change;
use channels::types::{ChannelBalance, HtlcState, InFlightHtlc, PaymentDirection};
use crypto::hash::hash;
use crypto::KeyPair;

/// Helper: compute fee = input_amount - sum(outputs)
fn tx_fee(tx: &doli_core::transaction::Transaction, input_amount: u64) -> i64 {
    let output_sum: u64 = tx.outputs.iter().map(|o| o.amount).sum();
    input_amount as i64 - output_sum as i64
}

// ── Funding Transaction ─────────────────────────────────────────────

#[test]
fn funding_with_change_auto_calculated() {
    let alice = hash(b"alice");
    let bob = hash(b"bob");
    let capacity = 1_000_000u64;
    let fee = 1500u64;

    let tx = build_funding_tx_with_change(
        vec![(hash(b"utxo1"), 0, 800_000), (hash(b"utxo2"), 1, 500_000)],
        alice,
        bob,
        capacity,
        fee,
        alice,
    )
    .unwrap();

    let input_amount = 1_300_000u64;
    let f = tx_fee(&tx, input_amount);
    assert_eq!(f, fee as i64, "fee should be exactly the requested fee");
    // change = 1_300_000 - 1_000_000 - 1500 = 298_500
    assert_eq!(tx.outputs.len(), 2); // funding + change
    assert_eq!(tx.outputs[0].amount, capacity);
    assert_eq!(tx.outputs[1].amount, 298_500);
}

#[test]
fn funding_with_change_exact_amount_no_change_output() {
    let alice = hash(b"alice");
    let bob = hash(b"bob");
    let capacity = 1_000_000u64;
    let fee = 0u64;

    let tx = build_funding_tx_with_change(
        vec![(hash(b"utxo"), 0, 1_000_000)],
        alice,
        bob,
        capacity,
        fee,
        alice,
    )
    .unwrap();

    assert_eq!(tx.outputs.len(), 1); // only funding, no change
    assert_eq!(tx.outputs[0].amount, capacity);
}

#[test]
fn funding_with_change_insufficient_funds() {
    let alice = hash(b"alice");
    let bob = hash(b"bob");

    let result = build_funding_tx_with_change(
        vec![(hash(b"utxo"), 0, 500_000)],
        alice,
        bob,
        1_000_000,
        1500,
        alice,
    );
    assert!(result.is_err());
}

// ── Cooperative Close ────────────────────────────────────────────────

#[test]
fn cooperative_close_preserves_capacity() {
    let capacity = 1_000_000u64;
    let balance = ChannelBalance::new(600_000, 400_000);
    assert_eq!(balance.total(), capacity);

    let tx = build_cooperative_close(hash(b"f"), 0, hash(b"l"), hash(b"r"), &balance, capacity, 0)
        .unwrap();

    let f = tx_fee(&tx, capacity);
    assert_eq!(
        f, 0,
        "cooperative close with 0 fee should preserve capacity"
    );
}

#[test]
fn cooperative_close_with_fee() {
    let capacity = 1_000_000u64;
    let fee = 1500u64;
    // balance.total() == capacity; fee deducted from local output only
    let balance = ChannelBalance::new(600_000, 400_000);

    let tx = build_cooperative_close(
        hash(b"f"),
        0,
        hash(b"l"),
        hash(b"r"),
        &balance,
        capacity,
        fee,
    )
    .unwrap();

    let f = tx_fee(&tx, capacity);
    assert_eq!(f, fee as i64);
}

#[test]
fn cooperative_close_rejects_mismatch() {
    let capacity = 1_000_000u64;
    let balance = ChannelBalance::new(500_000, 400_000); // total = 900K
    let result =
        build_cooperative_close(hash(b"f"), 0, hash(b"l"), hash(b"r"), &balance, capacity, 0);
    assert!(
        result.is_err(),
        "should reject when balance.total() != capacity"
    );
}

// ── Commitment ───────────────────────────────────────────────────────

#[test]
fn commitment_preserves_capacity() {
    let seed = [42u8; 32];
    let capacity = 1_000_000u64;
    let pair = CommitmentPair::new(0, ChannelBalance::new(700_000, 300_000), &seed);

    let tx = pair
        .build_local_commitment(hash(b"f"), 0, hash(b"l"), hash(b"r"), 1144, capacity, 0)
        .unwrap();

    let f = tx_fee(&tx, capacity);
    assert_eq!(f, 0);
}

#[test]
fn commitment_rejects_htlc_without_balance_reduction() {
    let seed = [42u8; 32];
    let capacity = 1_000_000u64;
    // Balance NOT reduced for HTLC — invariant violated
    let mut pair = CommitmentPair::new(0, ChannelBalance::new(700_000, 300_000), &seed);
    pair.htlcs.push(InFlightHtlc {
        htlc_id: 0,
        payment_hash: [11u8; 32],
        amount: 100_000,
        expiry_height: 5000,
        direction: PaymentDirection::Outgoing,
        state: HtlcState::Pending,
        preimage: None,
    });

    let result =
        pair.build_local_commitment(hash(b"f"), 0, hash(b"l"), hash(b"r"), 1144, capacity, 0);
    assert!(
        result.is_err(),
        "should reject when balance + htlcs != capacity"
    );
}

#[test]
fn commitment_with_htlcs_and_correct_balance() {
    let seed = [42u8; 32];
    let capacity = 1_000_000u64;
    let htlc_amount = 100_000u64;
    // Balance reduced by HTLC amount
    let mut pair = CommitmentPair::new(
        0,
        ChannelBalance::new(700_000 - htlc_amount, 300_000),
        &seed,
    );
    pair.htlcs.push(InFlightHtlc {
        htlc_id: 0,
        payment_hash: [11u8; 32],
        amount: htlc_amount,
        expiry_height: 5000,
        direction: PaymentDirection::Outgoing,
        state: HtlcState::Pending,
        preimage: None,
    });

    let tx = pair
        .build_local_commitment(hash(b"f"), 0, hash(b"l"), hash(b"r"), 1144, capacity, 0)
        .unwrap();

    let f = tx_fee(&tx, capacity);
    assert_eq!(f, 0, "correct balance adjustment should preserve capacity");
}

// ── Penalty / Delayed Claim ──────────────────────────────────────────

#[test]
fn penalty_tx_with_fee() {
    let keypair = KeyPair::generate();
    let amount = 700_000u64;
    let fee = 1500u64;

    let tx = build_penalty_tx(
        hash(b"revoked"),
        0,
        amount,
        hash(b"claim"),
        &keypair,
        &[42u8; 32],
        fee,
    )
    .unwrap();

    let f = tx_fee(&tx, amount);
    assert_eq!(f, fee as i64);
    assert_eq!(tx.outputs[0].amount, amount - fee);
}

#[test]
fn delayed_claim_with_fee() {
    let keypair = KeyPair::generate();
    let amount = 500_000u64;
    let fee = 1500u64;

    let tx = build_delayed_claim(
        hash(b"commitment"),
        0,
        amount,
        hash(b"claim"),
        &keypair,
        fee,
    )
    .unwrap();

    let f = tx_fee(&tx, amount);
    assert_eq!(f, fee as i64);
    assert_eq!(tx.outputs[0].amount, amount - fee);
}

// ── Full Lifecycle ───────────────────────────────────────────────────

#[test]
fn full_lifecycle_no_supply_leak() {
    let capacity = 1_000_000u64;
    let funding_fee = 1500u64;
    let close_fee = 1500u64;

    let alice = hash(b"alice");
    let bob = hash(b"bob");

    // 1. Fund channel (auto change)
    let funding_tx = build_funding_tx_with_change(
        vec![(hash(b"utxo"), 0, 1_200_000)],
        alice,
        bob,
        capacity,
        funding_fee,
        alice,
    )
    .unwrap();

    let change: u64 = funding_tx.outputs.iter().skip(1).map(|o| o.amount).sum();
    assert_eq!(change, 1_200_000 - capacity - funding_fee);

    // 2. Off-chain payments — balance.total() == capacity always
    let mut balance = ChannelBalance::new(capacity, 0);
    balance = balance.pay_local_to_remote(300_000).unwrap();
    balance = balance.pay_remote_to_local(50_000).unwrap();

    // 3. Cooperative close
    let close_tx = build_cooperative_close(
        hash(b"funding_hash"),
        0,
        alice,
        bob,
        &balance,
        capacity,
        close_fee,
    )
    .unwrap();

    let close_output_sum: u64 = close_tx.outputs.iter().map(|o| o.amount).sum();
    assert_eq!(close_output_sum, capacity - close_fee);

    // Total accounting:
    // Input: 1,200,000
    // Burned: 1,500 (funding) + 1,500 (close) = 3,000
    // Returned: change + close outputs = 198,500 + 998,500 = 1,197,000
    // 1,197,000 + 3,000 = 1,200,000 ✓
    assert_eq!(
        change + close_output_sum + funding_fee + close_fee,
        1_200_000
    );
}
