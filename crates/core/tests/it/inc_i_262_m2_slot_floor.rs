//! INC-I-262 M2 / REQ-DEF-003 — the registration floor in slots: the smallest
//! stamped slot of the accused's Bond UTXOs, waived while locked collateral
//! exists; the gate bound against the gate only.
//!
// covers: slash_window slash_state
//
// OUTPUT CONTRACT: fn validate_slash_target(&Transaction, &ValidationContext) -> Result<(), ValidationError>
//   O1 mutable params: none (both shared refs)
//   O2 receiver: none (free fn); no store writes, no globals
//   O3 return: Ok | InvalidSlash(EXPIRED) | InvalidSlash(PREDATES)
// PATHS: P-X age >= unbonding_period | P-T0 height == max(registered_at, gate)
//        | P-G age > height - gate | P-F no locked collateral and (oldest_bond_slot None or offence < it)
//        | P-W locked collateral waives P-F | P-OK | P-BELOW height < gate
// INPUT PARTITIONS:
//   chain: registration block 100 at slot 1000, dense to slot 1010, 20 blocks over 40 slots
//          (tip 130 at slot 1050, judged 131 at slot 1051), gate 50
//   offence vs oldest_bond_slot: below | equal | above; oldest_bond_slot None | Some
//   locked_collateral: false | true; age vs height - gate: equal | +1; age vs period: -1 | equal
// MATRIX:
//   O3 x P-OK (spec example)        -> inc_i_262_m2_slot_floor_spec_example_admitted
//   O3 x P-OK (offence == floor)    -> inc_i_262_m2_slot_floor_offence_at_oldest_bond_slot_admitted
//   O3 x P-F (offence < floor)      -> inc_i_262_m2_slot_floor_offence_before_oldest_bond_slot_refused
//   O3 x P-F (floor moved by a later bond) -> inc_i_262_m2_slot_floor_remaining_bond_floor_refuses_without_collateral
//   O3 x P-W                        -> inc_i_262_m2_slot_floor_locked_collateral_waives_floor
//   O3 x P-F (None)                 -> inc_i_262_m2_slot_floor_no_bond_slot_without_collateral_refused
//   O3 x P-T0                       -> inc_i_262_m2_slot_floor_zero_tenure_refused
//   O3 x P-G                        -> inc_i_262_m2_slot_floor_gate_bound_genesis_producer
//                                      inc_i_262_m2_slot_floor_gate_bound_uses_gate_not_registration
//   O3 x P-X                        -> inc_i_262_m2_slot_floor_expiry_unchanged
//   O3 x P-BELOW                    -> inc_i_262_m2_slot_floor_below_gate_unchanged

use doli_core::transaction::{SlashData, Transaction};
use doli_core::validation::{validate_slash_target, SlashTargetState, ValidationError};

use super::inc_i_255_slash_evidence::{accused, ctx_at, header, sign};

const GATE: u64 = 50;
const REG: u64 = 100;
const FLOOR: u32 = 1_000;
const HEIGHT: u64 = 131;
const PREV: u32 = 1_050;
const WIDE: u64 = 1_000_000;
const EXPIRED: &str = "slash evidence expired";
const PREDATES: &str = "predates the accused's current registration";

fn pair_tx(offence_slot: u32) -> Transaction {
    let kp = accused();
    let pk = *kp.public_key();
    let mut a = header(0x61, offence_slot, pk);
    let mut b = header(0x62, offence_slot, pk);
    sign(&mut a, &kp);
    sign(&mut b, &kp);
    Transaction::new_slash_producer(SlashData::double_production(pk, a, b))
}

fn target(registered_at: u64, oldest_bond_slot: Option<u32>, locked: bool) -> SlashTargetState {
    SlashTargetState {
        active: true,
        locked_collateral: locked,
        registered_at,
        oldest_bond_slot,
        ..SlashTargetState::default()
    }
}

fn judge_at(
    gate: u64,
    offence: u32,
    height: u64,
    prev_slot: u32,
    period: u64,
    t: SlashTargetState,
) -> Result<(), ValidationError> {
    let mut ctx = ctx_at(height, gate, 0).with_slash_targets([(*accused().public_key(), t)]);
    ctx.prev_slot = prev_slot;
    ctx.unbonding_period = period;
    validate_slash_target(&pair_tx(offence), &ctx)
}

/// The spec chain: judged at height 131, slot 1051, gate 50.
fn judge(offence: u32, t: SlashTargetState) -> Result<(), ValidationError> {
    judge_at(GATE, offence, HEIGHT, PREV, WIDE, t)
}

fn msg(r: &Result<(), ValidationError>) -> String {
    match r {
        Err(ValidationError::InvalidSlash(m)) => m.clone(),
        other => format!("{other:?}"),
    }
}

fn assert_predates(r: &Result<(), ValidationError>, case: &str) {
    let m = msg(r);
    assert!(
        m.contains(PREDATES) && !m.contains(EXPIRED),
        "{case}: expected only the predates refusal; got {r:?}"
    );
}

fn assert_expired(r: &Result<(), ValidationError>, case: &str) {
    let m = msg(r);
    assert!(
        m.contains(EXPIRED) && !m.contains(PREDATES),
        "{case}: expected only the expiry refusal; got {r:?}"
    );
}

// REQ-DEF-003 — Decision: a failure means an offence inside the registration is refused as "predates" once missed slots follow it (blocks compared with slots), so a valid slash is lost.
#[test]
fn inc_i_262_m2_slot_floor_spec_example_admitted() {
    let r = judge(1_010, target(REG, Some(FLOOR), false));
    assert_eq!(
        r,
        Ok(()),
        "offence slot 1010 >= floor slot 1000; age 41 <= height - gate 81; blocks since registration 31"
    );
}

// REQ-DEF-003 — Decision: a failure means the floor is off by one: an equivocation in the registration block's own slot is refused, or the comparison is not `offence < oldest_bond_slot`.
#[test]
fn inc_i_262_m2_slot_floor_offence_at_oldest_bond_slot_admitted() {
    let r = judge(FLOOR, target(REG, Some(FLOOR), false));
    assert_eq!(r, Ok(()), "offence slot == oldest bond slot 1000, age 51");
}

// REQ-DEF-003 — Decision: a failure means evidence from before the accused's registration slot burns its bond (a wrongful burn).
#[test]
fn inc_i_262_m2_slot_floor_offence_before_oldest_bond_slot_refused() {
    let r = judge(FLOOR - 1, target(REG, Some(FLOOR), false));
    assert_predates(
        &r,
        "offence slot 999 < oldest bond slot 1000, no locked collateral",
    );
}

// REQ-DEF-003 — Decision: a failure means the floor is not read from the Bond UTXOs: with the registration bonds gone and no locked collateral, an offence older than every remaining Bond still burns them.
#[test]
fn inc_i_262_m2_slot_floor_remaining_bond_floor_refuses_without_collateral() {
    let r = judge(1_040, target(REG, Some(1_045), false));
    assert_predates(
        &r,
        "offence 1040 < oldest remaining bond slot 1045, no locked collateral",
    );
    let after = judge(1_045, target(REG, Some(1_045), false));
    assert_eq!(after, Ok(()), "offence at the remaining bond's slot");
}

// REQ-DEF-003 — Decision: a failure means a RequestWithdrawal of the older bonds moves the floor past the offence while its Unbonding collateral is still locked, so the equivocator escapes the slash.
#[test]
fn inc_i_262_m2_slot_floor_locked_collateral_waives_floor() {
    let below = judge(FLOOR - 1, target(REG, Some(FLOOR), true));
    assert_eq!(
        below,
        Ok(()),
        "offence before the floor slot, locked collateral"
    );
    let moved = judge(1_010, target(REG, Some(1_045), true));
    assert_eq!(
        moved,
        Ok(()),
        "floor moved past the offence, locked collateral"
    );
    let none = judge(1_010, target(REG, None, true));
    assert_eq!(none, Ok(()), "no Bond UTXO left, locked collateral");
}

// REQ-DEF-003 — Decision: a failure means an accused with no Bond UTXO and no locked collateral (nothing burnable, or an owner lookup past the cap) is admitted on an unknown floor.
#[test]
fn inc_i_262_m2_slot_floor_no_bond_slot_without_collateral_refused() {
    let r = judge(1_040, target(REG, None, false));
    assert_predates(&r, "oldest_bond_slot None, no locked collateral, age 11");
}

// REQ-DEF-003 — Decision: a failure means a slash is valid in the floor block itself (tenure 0), so an offence in the slots skipped just before the floor is punished.
#[test]
fn inc_i_262_m2_slot_floor_zero_tenure_refused() {
    for locked in [false, true] {
        let at_reg = judge_at(
            GATE,
            FLOOR,
            REG,
            FLOOR - 1,
            WIDE,
            target(REG, Some(FLOOR), locked),
        );
        assert_predates(
            &at_reg,
            &format!("height == registered_at, locked {locked}"),
        );
        let at_gate = judge_at(
            GATE,
            FLOOR,
            GATE,
            FLOOR - 1,
            WIDE,
            target(0, Some(0), locked),
        );
        assert_predates(&at_gate, &format!("height == gate, locked {locked}"));
    }
}

// REQ-DEF-003 — Decision: a failure means an offence from before the slashing gate is punished retroactively, with or without locked collateral.
#[test]
fn inc_i_262_m2_slot_floor_gate_bound_genesis_producer() {
    let (gate, height, prev) = (1_000, 1_031, 1_050);
    for locked in [false, true] {
        let t = target(0, Some(0), locked);
        let at_bound = judge_at(gate, 1_020, height, prev, WIDE, t);
        assert_eq!(
            at_bound,
            Ok(()),
            "age 31 == height - gate 31, locked {locked}"
        );
        let past = judge_at(gate, 1_019, height, prev, WIDE, t);
        assert_predates(
            &past,
            &format!("age 32 > height - gate 31, locked {locked}"),
        );
    }
}

// REQ-DEF-003 — Decision: a failure means the gate bound still counts blocks since the registration, so the blocks-versus-slots refusal survives for every registered producer.
#[test]
fn inc_i_262_m2_slot_floor_gate_bound_uses_gate_not_registration() {
    let t = target(REG, Some(0), true);
    let inside = judge(PREV + 1 - 81, t);
    assert_eq!(
        inside,
        Ok(()),
        "age 81 == height - gate 81, > blocks since registration 31"
    );
    let past = judge(PREV + 1 - 82, t);
    assert_predates(&past, "age 82 > height - gate 81");
}

// REQ-DEF-003 — Decision: a failure means the floor change altered the expiry window: evidence one window old must still be the expiry error, before any floor check.
#[test]
fn inc_i_262_m2_slot_floor_expiry_unchanged() {
    let t = target(REG, Some(FLOOR), false);
    let fresh = judge_at(GATE, 1_010, HEIGHT, PREV, 42, t);
    assert_eq!(fresh, Ok(()), "age 41 < period 42");
    let at_period = judge_at(GATE, 1_010, HEIGHT, PREV, 41, t);
    assert_expired(&at_period, "age 41 == period 41");
    let before_floor = judge_at(GATE, FLOOR - 1, HEIGHT, PREV, 41, t);
    assert_expired(&before_floor, "expired and before the floor: expiry first");
}

// REQ-DEF-003 — Decision: a failure means the floor facts change a verdict below the slashing gate, a consensus change on heights already mined.
#[test]
fn inc_i_262_m2_slot_floor_below_gate_unchanged() {
    for t in [target(REG, None, false), target(REG, Some(u32::MAX), false)] {
        let r = judge_at(HEIGHT + 1, 10, HEIGHT, PREV, 5, t);
        assert_eq!(
            r,
            Ok(()),
            "below the gate the window is not judged; facts {t:?}"
        );
    }
}
