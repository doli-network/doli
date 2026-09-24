//! INC-I-234 M5 — `doli token issue` must emit an input-anchored asset_id.
//!
//! OUTPUT CONTRACT: fn build_issue_tx(selected, issuer, ticker, supply, cond, fee)
//!   -> Result<Transaction>. Pure, no mutable params, no stores.
//!   O1 tx_type · O2 inputs (order + outpoints) · O3 outputs[0] FA (asset_id, amount,
//!   total_supply, ticker, owner, type) · O4 change output (present?, amount, owner)
//!   · O5 Err on insufficient/empty inputs · O6 determinism (hash stable).
//! Paths: P1 Σin > fee (change) · P2 Σin == fee (no change) · P3 Σin < fee or no inputs.
//! INPUT PARTITIONS:
//!   P1a multi-input, anchor ≠ later input  → O1,O2,O3,O4 (test anchored_on_first_input)
//!   P1b single input, authority rule above AH → O3 via check_value_authority
//!   P1c same inputs twice                   → O6
//!   P2  exact fee                           → O4 absent + authority
//!   P3a Σin < fee, P3b empty                → O5
//! Matrix: every (O, P) cell above has an assertion in the named test.

use super::build_issue_tx;
use crypto::Hash;
use doli_core::consensus::ConsensusParams;
use doli_core::validation::{check_value_authority, ValidationContext};
use doli_core::{Condition, Network, Output, OutputType, TxType};

fn h(b: u8) -> Hash {
    Hash::from_bytes([b; 32])
}

fn ctx_above_ah() -> ValidationContext {
    ValidationContext::new(ConsensusParams::mainnet(), Network::Mainnet, 0, 10)
        .with_inc_i_234_activation_height(0)
}

fn consumed(issuer: Hash, selected: &[(Hash, u32, u64)]) -> Vec<Output> {
    selected
        .iter()
        .map(|(_, _, amt)| Output::normal(*amt, issuer))
        .collect()
}

// REQ-234-014 (Should) — Decision: a failure means the CLI still emits a placeholder asset_id that every node rejects above the AH.
#[test]
fn req_234_014_issue_asset_id_is_anchored_on_first_input() {
    let issuer = h(7);
    let selected = [(h(0xA1), 3u32, 5_000u64), (h(0xB2), 0, 5_000)];
    let tx = build_issue_tx(
        &selected,
        issuer,
        "GOLD",
        1_000_000,
        &Condition::signature(issuer),
        1_000,
    )
    .expect("build");

    let (asset_id, total_supply, ticker) = tx.outputs[0]
        .fungible_asset_metadata()
        .expect("FA output 0");
    assert_eq!(asset_id, Output::compute_asset_id(&h(0xA1), 3));
    assert_ne!(
        asset_id,
        Output::compute_asset_id(&h(0xB2), 0),
        "anchor must be inputs[0], not a later input"
    );
    assert_eq!(total_supply, 1_000_000);
    assert_eq!(ticker, "GOLD");
    assert_eq!(tx.outputs[0].amount, 1_000_000);
    assert_eq!(tx.outputs[0].output_type, OutputType::FungibleAsset);
    assert_eq!(tx.outputs[0].pubkey_hash, issuer);
    assert_eq!(tx.tx_type, TxType::Transfer);
    assert_eq!(tx.inputs.len(), 2);
    assert_eq!(
        (tx.inputs[0].prev_tx_hash, tx.inputs[0].output_index),
        (h(0xA1), 3)
    );
    assert_eq!(
        (tx.inputs[1].prev_tx_hash, tx.inputs[1].output_index),
        (h(0xB2), 0)
    );
    assert_eq!(tx.outputs.len(), 2);
    assert_eq!(tx.outputs[1].output_type, OutputType::Normal);
    assert_eq!(tx.outputs[1].amount, 9_000);
    assert_eq!(tx.outputs[1].pubkey_hash, issuer);
}

// REQ-234-014 (Should) — Decision: a failure means `doli token issue` output is rejected by the REQ-234-SEC-008 consensus rule it must ship with.
#[test]
fn req_234_014_issue_tx_passes_value_authority_above_ah() {
    let issuer = h(9);
    let selected = [(h(0xC3), 1u32, 2_000u64)];
    let tx = build_issue_tx(
        &selected,
        issuer,
        "SILVER",
        21_000_000,
        &Condition::signature(issuer),
        500,
    )
    .expect("build");
    check_value_authority(&tx, &consumed(issuer, &selected), &ctx_above_ah())
        .expect("CLI issuance must satisfy check_value_authority above the AH");
}

// REQ-234-014 (Should) — Decision: a failure means the time-nonce placeholder survived, so the id cannot be predicted from the tx.
#[test]
fn req_234_014_issue_is_deterministic_for_same_inputs() {
    let issuer = h(4);
    let selected = [(h(0xD4), 0u32, 10_000u64)];
    let cond = Condition::signature(issuer);
    let a = build_issue_tx(&selected, issuer, "T", 5, &cond, 1_000).expect("a");
    let b = build_issue_tx(&selected, issuer, "T", 5, &cond, 1_000).expect("b");
    assert_eq!(a.hash(), b.hash());
}

// REQ-234-014 (Should) — Decision: a failure means an exact-fee input emits a zero-value change output or mis-sizes the tx.
#[test]
fn req_234_014_issue_exact_fee_has_no_change_and_still_passes() {
    let issuer = h(5);
    let selected = [(h(0xE5), 2u32, 1_000u64)];
    let tx = build_issue_tx(
        &selected,
        issuer,
        "EXACT",
        77,
        &Condition::signature(issuer),
        1_000,
    )
    .expect("build");
    assert_eq!(tx.outputs.len(), 1);
    check_value_authority(&tx, &consumed(issuer, &selected), &ctx_above_ah()).expect("authority");
}

// REQ-234-014 (Should) — Decision: a failure means the extracted builder dropped the insufficient-balance guard of the old inline flow.
#[test]
fn req_234_014_issue_rejects_insufficient_or_empty_inputs() {
    let issuer = h(6);
    let cond = Condition::signature(issuer);
    assert!(build_issue_tx(&[(h(1), 0, 999)], issuer, "X", 1, &cond, 1_000).is_err());
    assert!(build_issue_tx(&[], issuer, "X", 1, &cond, 0).is_err());
}
