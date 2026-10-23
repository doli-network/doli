//! INC-I-182 M2.5 — Replay-mode block validation refuses a wrong-key or replayed
//! producer signature with the same code as Full and Light, for all three types.
//!
// covers: producer_auth transaction types validation core block
//
// OUTPUT CONTRACT: fn validate_block_with_mode(&Block, &ValidationContext, ValidationMode) -> Result<(), ValidationError>
//   O1 return: Ok | Err; Err text holds [ERRTX-PAUTH001] for an unauthorized producer tx; pure
// PATHS: P-REPLAY | P-LIGHT | P-FULL; height gate - 1 | gate
// MATRIX
//   O1 x 3 kinds x {wrong key, replayed} x {P-REPLAY, P-LIGHT, P-FULL} x gate -> coded
//   O1 x the same forms x {P-REPLAY, P-LIGHT} x gate - 1 -> Ok; P-FULL x gate - 1 -> not coded (control)
//   O1 x 3 kinds x signed x gate -> Replay/Light Ok, Full not coded (control)
//   one test fn per kind x form: inc_i_182_m25_replay_<kind>_<form>_refused
// INPUT PARTITIONS:
//   signer: stranger key | producer key over a tx with another input
//   block: one producer tx, header valid for the time checks, fake VDF (Full stops after the tx loop)

use doli_core::transaction::TxType;
use doli_core::validation::{validate_block_with_mode, ValidationMode};

use super::inc_i_182_m1_block_modes::{block_ctx, block_of};
use super::inc_i_182_m1_core_fx::*;

const TAG: u8 = 0x50;

fn refused_in_every_mode(kind: TxType, shape: Shape) {
    let forged = block_of(shaped(kind, TAG, shape));
    let replay = validate_block_with_mode(&forged, &block_ctx(GATE), ValidationMode::Replay);
    assert!(
        coded(&replay),
        "{kind:?} {shape:?} at the gate, Replay: {replay:?}"
    );
    let signed_block = block_of(signed(kind, TAG));
    let mut bad = Vec::new();
    for mode in [ValidationMode::Light, ValidationMode::Full] {
        let at = validate_block_with_mode(&forged, &block_ctx(GATE), mode);
        if !coded(&at) {
            bad.push(format!("{kind:?} {shape:?} at the gate, {mode:?}: {at:?}"));
        }
    }
    for mode in [
        ValidationMode::Replay,
        ValidationMode::Light,
        ValidationMode::Full,
    ] {
        let lax = !matches!(mode, ValidationMode::Full);
        let below = validate_block_with_mode(&forged, &block_ctx(GATE - 1), mode);
        if coded(&below) || (lax && below.is_err()) {
            bad.push(format!(
                "CONTROL {kind:?} {shape:?} below the gate, {mode:?}: {below:?}"
            ));
        }
        let ok = validate_block_with_mode(&signed_block, &block_ctx(GATE), mode);
        if coded(&ok) || (lax && ok.is_err()) {
            bad.push(format!(
                "CONTROL {kind:?} signed at the gate, {mode:?}: {ok:?}"
            ));
        }
    }
    assert!(bad.is_empty(), "block verdicts:\n{}", bad.join("\n"));
}

// REQ-PAUTH-002 — Decision: a failure means a syncing node (Replay) accepts a withdrawal signed by another key that live nodes refuse, so sync and live validation fork on that block.
#[test]
fn inc_i_182_m25_replay_withdrawal_wrong_key_refused() {
    refused_in_every_mode(TxType::RequestWithdrawal, Shape::WrongKey);
}

// REQ-PAUTH-002 — Decision: a failure means Replay accepts a withdrawal whose producer signature was lifted from another tx, so a synced node holds a forced exit live nodes refused.
#[test]
fn inc_i_182_m25_replay_withdrawal_replayed_refused() {
    refused_in_every_mode(TxType::RequestWithdrawal, Shape::Replayed);
}

// REQ-PAUTH-002 — Decision: a failure means Replay accepts an AddBond signed by another key, so Replay and live validation disagree on a producer's allowance.
#[test]
fn inc_i_182_m25_replay_add_bond_wrong_key_refused() {
    refused_in_every_mode(TxType::AddBond, Shape::WrongKey);
}

// REQ-PAUTH-002 — Decision: a failure means Replay accepts an AddBond with a replayed producer signature, so Replay and live validation disagree on a producer's allowance.
#[test]
fn inc_i_182_m25_replay_add_bond_replayed_refused() {
    refused_in_every_mode(TxType::AddBond, Shape::Replayed);
}

// REQ-PAUTH-002 — Decision: a failure means Replay accepts a Registration signed by a key other than the registered one, so a synced node lists a producer live nodes refused.
#[test]
fn inc_i_182_m25_replay_registration_wrong_key_refused() {
    refused_in_every_mode(TxType::Registration, Shape::WrongKey);
}

// REQ-PAUTH-002 — Decision: a failure means Replay accepts a Registration with a replayed producer signature, so a synced node lists a producer live nodes refused.
#[test]
fn inc_i_182_m25_replay_registration_replayed_refused() {
    refused_in_every_mode(TxType::Registration, Shape::Replayed);
}
