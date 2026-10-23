// covers: slash_state slash_window slash_evidence slash_admission pool unbonding entry error assembly tx_processing validation_checks mod lib block transaction types set_registration set_lifecycle defaults network_params
//! INC-I-256 M2 / R-4 — the slash evidence window (AUDIT-P2-539) and the
//! tenure floor (AUDIT-P2-547) inside `validate_slash_target`.
//!
// OUTPUT CONTRACT: fn validate_slash_target(&Transaction, &ValidationContext) -> Result<(), ValidationError>
//   O1 mutable params: none (both shared refs)
//   O2 receiver: none (free fn); no store writes, no globals
//   O3 return: Ok | InvalidSlash(EXPIRED) | InvalidSlash(PREDATES) | InvalidSlash(existing state refusal)
// PATHS: P-OK | P-X age >= unbonding_period | P-Y age >= tenure | P-STATE state refusal first
//        | P-BELOW current_height < slash gate | P-OTHER non-slash tx
//   age = (prev_slot +sat 1) -sat offence_slot; tenure = current_height -sat max(registered_at, gate)
// INPUT PARTITIONS:
//   age vs period: period - 1 (Ok) | period (X)
//   age vs tenure: tenure - 1 (Ok) | tenure (Y)
//   floor: gate > registered_at | registered_at > gate | current_height == gate (tenure 0)
//   saturation: prev_slot u32::MAX | offence slot > judged slot | current_height < floor
//   order: Slashed or queued + expired (state error) | expired + pre-tenure (X)
// MATRIX: O3 x 13 cells, one test per path family:
//   P-OK,P-X x period            -> inc_i_256_m2_window_age_period_minus_one_ok_period_expired
//   P-OK,P-Y x tenure            -> inc_i_256_m2_window_age_tenure_minus_one_ok_tenure_predates
//   P-Y x floor                  -> inc_i_256_m2_window_floor_is_gate_above_registration
//                                   inc_i_256_m2_window_floor_is_registration_above_gate
//   P-STATE x order              -> inc_i_256_m2_window_state_refusals_run_before_expiry
//   P-X x order                  -> inc_i_256_m2_window_expiry_checked_before_tenure
//   saturation                   -> inc_i_256_m2_window_prev_slot_max_saturates_to_expired
//                                   inc_i_256_m2_window_future_offence_slot_is_age_zero
//                                   inc_i_256_m2_window_height_below_floor_is_zero_tenure
//   P-BELOW, P-OTHER             -> inc_i_256_m2_window_below_gate_unchanged
//                                   inc_i_256_m2_window_non_slash_tx_ok

use doli_core::transaction::{SlashData, Transaction};
use doli_core::validation::{validate_slash_target, SlashTargetState, ValidationError};

use super::inc_i_255_slash_evidence::{accused, ctx_at, header, sign};

const GATE: u64 = 1_000;
const EXPIRED: &str = "slash evidence expired";
const PREDATES: &str = "predates the accused's current registration";
const WIDE: u64 = 1_000_000;

fn pair_tx(offence_slot: u32) -> Transaction {
    let kp = accused();
    let pk = *kp.public_key();
    let mut a = header(0x61, offence_slot, pk);
    let mut b = header(0x62, offence_slot, pk);
    sign(&mut a, &kp);
    sign(&mut b, &kp);
    Transaction::new_slash_producer(SlashData::double_production(pk, a, b))
}

fn active(registered_at: u64) -> SlashTargetState {
    SlashTargetState {
        active: true,
        registered_at,
        ..SlashTargetState::default()
    }
}

/// Verdict at `height` (slash gate `GATE`, signature gate 0) with parent slot `prev_slot`.
fn judge(
    tx: &Transaction,
    height: u64,
    prev_slot: u32,
    period: u64,
    target: SlashTargetState,
) -> Result<(), ValidationError> {
    let mut ctx = ctx_at(height, GATE, 0).with_slash_targets([(*accused().public_key(), target)]);
    ctx.prev_slot = prev_slot;
    ctx.unbonding_period = period;
    validate_slash_target(tx, &ctx)
}

fn slash_msg(r: &Result<(), ValidationError>) -> Option<String> {
    match r {
        Err(ValidationError::InvalidSlash(m)) => Some(m.clone()),
        _ => None,
    }
}

fn assert_expired(r: &Result<(), ValidationError>, case: &str) {
    let m = slash_msg(r).unwrap_or_default();
    assert!(
        m.contains(EXPIRED) && !m.contains(PREDATES),
        "{case}: expected only the expiry refusal; got {r:?}"
    );
}

fn assert_predates(r: &Result<(), ValidationError>, case: &str) {
    let m = slash_msg(r).unwrap_or_default();
    assert!(
        m.contains(PREDATES) && !m.contains(EXPIRED),
        "{case}: expected only the tenure refusal; got {r:?}"
    );
}

// R-4 (AUDIT-P2-539) — Decision: a failure means evidence one block inside the window is refused, or evidence exactly one window old still burns a bond whose funds have already unlocked.
#[test]
fn inc_i_256_m2_window_age_period_minus_one_ok_period_expired() {
    let (height, period, offence) = (1_500, 100, 1_300u32);
    let tx = pair_tx(offence);
    let ok = judge(&tx, height, offence + 98, period, active(0));
    assert_eq!(
        ok,
        Ok(()),
        "age 99 < period 100, tenure 500: fresh evidence"
    );
    let x = judge(&tx, height, offence + 99, period, active(0));
    assert_expired(&x, "age 100 == period 100");
}

// R-4 (AUDIT-P2-547) — Decision: a failure means evidence inside the current registration is refused, or an offence that is not after the registration floor still slashes.
#[test]
fn inc_i_256_m2_window_age_tenure_minus_one_ok_tenure_predates() {
    let (height, registered_at, offence) = (1_300, 1_200, 1_250u32);
    let tx = pair_tx(offence);
    let ok = judge(&tx, height, offence + 98, WIDE, active(registered_at));
    assert_eq!(ok, Ok(()), "age 99 < tenure 100");
    let y = judge(&tx, height, offence + 99, WIDE, active(registered_at));
    assert_predates(&y, "age 100 == tenure 100");
}

// R-4 (AUDIT-P2-547) — Decision: a failure means an offence before the activation height (the floor for a genesis producer) can be punished retroactively.
#[test]
fn inc_i_256_m2_window_floor_is_gate_above_registration() {
    let height = GATE + 50;
    let before_gate = judge(&pair_tx(990), height, 1_049, WIDE, active(0));
    assert_predates(
        &before_gate,
        "offence slot 990 < gate 1000, age 60 >= tenure 50",
    );
    let after_gate = judge(&pair_tx(1_010), height, 1_049, WIDE, active(0));
    assert_eq!(after_gate, Ok(()), "age 40 < tenure 50");
    let at_gate = judge(&pair_tx(990), GATE, 999, WIDE, active(0));
    assert_predates(&at_gate, "current_height == gate: tenure 0");
}

// R-4 (AUDIT-P2-546) — Decision: a failure means evidence from an earlier registration is reused against the producer after it re-registers.
#[test]
fn inc_i_256_m2_window_floor_is_registration_above_gate() {
    let (height, registered_at) = (GATE + 50, GATE + 40);
    let old_tenure = judge(&pair_tx(1_030), height, 1_049, WIDE, active(registered_at));
    assert_predates(
        &old_tenure,
        "offence 1030 before registration 1040, age 20 >= tenure 10",
    );
    let new_tenure = judge(&pair_tx(1_045), height, 1_049, WIDE, active(registered_at));
    assert_eq!(new_tenure, Ok(()), "age 5 < tenure 10");
}

// R-4 (AUDIT-P2-546) — Decision: a failure means the window replaces the existing one-slash-per-offence refusals, so reused evidence gets a different error on different paths.
#[test]
fn inc_i_256_m2_window_state_refusals_run_before_expiry() {
    let tx = pair_tx(1_000);
    let (height, prev, period) = (1_500, 1_400, 10);
    let slashed = SlashTargetState {
        slashed: true,
        ..SlashTargetState::default()
    };
    let r = judge(&tx, height, prev, period, slashed);
    let m = slash_msg(&r).unwrap_or_default();
    assert!(
        m.contains("slash target is already slashed") && !m.contains(EXPIRED),
        "Slashed + expired: the state refusal comes first; got {r:?}"
    );
    let queued = SlashTargetState {
        active: true,
        slash_queued: true,
        ..SlashTargetState::default()
    };
    let r = judge(&tx, height, prev, period, queued);
    let m = slash_msg(&r).unwrap_or_default();
    assert!(
        m.contains("slash target already has a pending slash") && !m.contains(EXPIRED),
        "queued + expired: the state refusal comes first; got {r:?}"
    );
}

// R-4 (AUDIT-P2-539) — Decision: a failure means one stale pair gets different errors on different paths, depending on which window check runs first.
#[test]
fn inc_i_256_m2_window_expiry_checked_before_tenure() {
    let r = judge(&pair_tx(1_000), 1_500, 1_499, 100, active(1_450));
    assert_expired(&r, "age 500 >= period 100 and >= tenure 50");
}

// R-4 (AUDIT-P2-539) — Decision: a failure means a parent at slot u32::MAX wraps the judged slot to 0 (or panics in debug), so expired evidence reads as fresh.
#[test]
fn inc_i_256_m2_window_prev_slot_max_saturates_to_expired() {
    let tx = pair_tx(1_300);
    let first = judge(&tx, 1_500, u32::MAX, 100, active(0));
    let again = judge(&tx, 1_500, u32::MAX, 100, active(0));
    assert_eq!(first, again, "the verdict is deterministic");
    assert_expired(&first, "judged slot saturates at u32::MAX");
}

// R-4 (AUDIT-P2-539) — Decision: a failure means an offence slot above the judged slot underflows the age (panic or a huge age), so a valid pair is refused.
#[test]
fn inc_i_256_m2_window_future_offence_slot_is_age_zero() {
    let r = judge(&pair_tx(5_000), 1_500, 100, 100, active(0));
    assert_eq!(r, Ok(()), "age saturates to 0 < period, 0 < tenure 500");
}

// R-4 (AUDIT-P2-547) — Decision: a failure means a registration floor above the judged height underflows the tenure and admits evidence against a registration that has not started.
#[test]
fn inc_i_256_m2_window_height_below_floor_is_zero_tenure() {
    let r = judge(&pair_tx(1_400), 1_500, 1_400, 100, active(2_000));
    assert_predates(&r, "floor 2000 > height 1500: tenure 0, age 1");
}

// R-4 — Decision: a failure means the window changes the verdict below the slashing gate, a consensus change on heights already mined.
#[test]
fn inc_i_256_m2_window_below_gate_unchanged() {
    let r = judge(&pair_tx(10), GATE - 1, 998, 100, active(998));
    assert_eq!(
        r,
        Ok(()),
        "below the gate an expired, pre-tenure pair is Ok as today"
    );
}

// R-4 — Decision: a failure means the window check runs on a transaction that is not a slash.
#[test]
fn inc_i_256_m2_window_non_slash_tx_ok() {
    let tx = Transaction::new_transfer(Vec::new(), Vec::new());
    let r = judge(&tx, 1_500, u32::MAX, 0, active(2_000));
    assert_eq!(r, Ok(()));
}
