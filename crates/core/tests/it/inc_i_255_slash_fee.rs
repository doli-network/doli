//! INC-I-255 — the slash fee exemption predicate and the below-gate verdict lock.
//!
// covers: slash_fee slash_evidence utxo transaction types validation
//
// OUTPUT CONTRACT: fn slash_fee_exempt(&Transaction, &ValidationContext) -> bool
//   O1 return value
//   no mutable params, no receiver writes, no store writes (pure)
// OUTPUT CONTRACT: fn validate_transaction / validate_transaction_skip_registration_vdf / validate_transaction_with_utxos
//   O2 return Result<(), ValidationError> below the gate (must equal the pre-fix verdict)
// PATHS: P-AH (h >= slash gate) | P-BELOW (h < slash gate) | P-TYPE (tx_type != SlashProducer)
// INPUT PARTITIONS:
//   tx:  IE-OK canonical slash | IE-TRAIL | IE-ORDER | IE-REP | IE-PAE | IE-RUNA (7 variants)
//        | every other TxType, 0-in/0-out, with empty extra_data and with the IE-OK bytes
//   ctx: at the gate | far above | zero-flow fee branch | gate = h + 1 | gate u64::MAX | both gates u64::MAX
// MATRIX:
//   O1 x P-AH x IE-OK                     -> inc_i_255_fee_exempt_true_for_valid_slash_at_or_above_gate
//   O1 x P-BELOW x IE-OK                  -> inc_i_255_fee_exempt_false_below_gate
//   O1 x P-AH x every invalid variant     -> inc_i_255_fee_exempt_false_for_every_invalid_variant
//   O1 x P-TYPE                           -> inc_i_255_fee_exempt_false_for_other_tx_types
//   O2 x P-BELOW x {IE-OK, TRAIL, ORDER, REP, PAE} -> inc_i_255_below_gate_verdict_is_the_fee_floor

use super::inc_i_255_slash_evidence::{
    armed_ctxs, canonical_tx, ctx_at, descending_tx, pae_tx, reporter_tx, run_a_invalid, verdicts,
    with_trailing, HEIGHT,
};
use crypto::Signature;
use doli_core::consensus::{BASE_FEE, FEE_PER_BYTE};
use doli_core::transaction::{Transaction, TxType};
use doli_core::validation::{slash_fee_exempt, ValidationContext, ValidationError};

fn below_gate_ctxs() -> Vec<ValidationContext> {
    vec![
        ctx_at(HEIGHT, HEIGHT + 1, HEIGHT),
        ctx_at(HEIGHT, u64::MAX, HEIGHT),
        ctx_at(HEIGHT, u64::MAX, u64::MAX),
    ]
}

fn invalid_variants() -> Vec<(&'static str, Transaction)> {
    let mut v = vec![
        ("trailing byte", with_trailing(&[0x00])),
        ("descending header order", descending_tx()),
        (
            "non-default reporter",
            reporter_tx(Signature::from_bytes([7u8; 64])),
        ),
        ("price attestation equivocation", pae_tx()),
    ];
    v.extend(run_a_invalid());
    v
}

fn fee_floor() -> ValidationError {
    ValidationError::InsufficientFee {
        actual: 0,
        minimum: 1,
        base: BASE_FEE,
        extra_bytes: 0,
        per_byte: FEE_PER_BYTE,
    }
}

// REQ-SLASH-002 — Decision: a failure means the one exemption predicate refuses authenticated evidence, so mempool and block validation disagree with the validator.
#[test]
fn inc_i_255_fee_exempt_true_for_valid_slash_at_or_above_gate() {
    let tx = canonical_tx();
    for ctx in armed_ctxs() {
        assert!(
            slash_fee_exempt(&tx, &ctx),
            "canonical slash at h={} must be exempt",
            ctx.current_height
        );
    }
}

// REQ-SLASH-004 — Decision: a failure means a slash becomes mineable before the activation height, splitting old and new nodes.
#[test]
fn inc_i_255_fee_exempt_false_below_gate() {
    let tx = canonical_tx();
    for ctx in below_gate_ctxs() {
        assert!(
            !slash_fee_exempt(&tx, &ctx),
            "slash gate {} above h={} must not exempt",
            ctx.inc_i_255_slashing_activation_height,
            ctx.current_height
        );
    }
}

// REQ-SLASH-002 — Decision: a failure means the exemption is keyed on the tx type and not on verified evidence, so forged slashes ride for free.
#[test]
fn inc_i_255_fee_exempt_false_for_every_invalid_variant() {
    for (what, tx) in invalid_variants() {
        for ctx in armed_ctxs() {
            assert!(!slash_fee_exempt(&tx, &ctx), "{what}: must not be exempt");
            let oracle_on = ctx.clone().with_oracle_activation_height(0);
            assert!(
                !slash_fee_exempt(&tx, &oracle_on),
                "{what}: must not be exempt with the oracle active"
            );
        }
    }
}

// REQ-SLASH-002 — Decision: a failure means the exemption leaks to another tx type (a general fee bypass), e.g. one that copies slash bytes.
#[test]
fn inc_i_255_fee_exempt_false_for_other_tx_types() {
    let slash_bytes = canonical_tx().extra_data;
    let others: Vec<TxType> = (0u32..=255)
        .filter_map(TxType::from_u32)
        .filter(|t| *t != TxType::SlashProducer)
        .collect();
    assert!(others.len() >= 10, "PRECONDITION: tx types enumerated");
    for ctx in armed_ctxs() {
        assert!(
            slash_fee_exempt(&canonical_tx(), &ctx),
            "NON-VACUITY: the slash itself is exempt in this ctx"
        );
        for tx_type in &others {
            for extra_data in [Vec::new(), slash_bytes.clone()] {
                let tx = Transaction {
                    version: 1,
                    tx_type: *tx_type,
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                    extra_data,
                };
                assert!(
                    !slash_fee_exempt(&tx, &ctx),
                    "{tx_type:?} 0-in/0-out ({} extra bytes) must not be exempt",
                    tx.extra_data.len()
                );
            }
        }
    }
}

// REQ-SLASH-004 — Decision: a failure means a pre-gate verdict changed, so a mixed fleet splits on a slash tx before the height.
#[test]
fn inc_i_255_below_gate_verdict_is_the_fee_floor() {
    let cases = [
        ("canonical", canonical_tx()),
        ("trailing byte", with_trailing(&[0x00])),
        ("descending header order", descending_tx()),
        (
            "non-default reporter",
            reporter_tx(Signature::from_bytes([7u8; 64])),
        ),
        ("price attestation equivocation", pae_tx()),
    ];
    for ctx in below_gate_ctxs() {
        let ctx = ctx.with_oracle_activation_height(0);
        for (what, tx) in &cases {
            let [(_, full), (_, skip), (_, utxo)] = verdicts(tx, &ctx);
            assert_eq!(full, Ok(()), "{what}: validate_transaction below the gate");
            assert_eq!(skip, Ok(()), "{what}: skip-vdf entry below the gate");
            assert_eq!(
                utxo,
                Err(fee_floor()),
                "{what}: below the gate the verdict stays the fee floor"
            );
            assert!(
                !slash_fee_exempt(tx, &ctx),
                "{what}: not exempt below the gate"
            );
        }
    }
}
