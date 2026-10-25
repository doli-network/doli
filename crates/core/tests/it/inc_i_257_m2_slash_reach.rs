//! INC-I-257 M2 — a slash reaches the locked funds: the burnable predicate,
//! and the target rule that admits an inactive accused holding locked collateral.
//!
// covers: validation slash_state
//
// OUTPUT CONTRACT: fn is_slash_burnable(&Output, u64) -> bool
//   O1 return; pure (no mutable params, no receiver, no store)
// OUTPUT CONTRACT: fn validate_slash_target(&Transaction, &ValidationContext) -> Result<(), ValidationError>
//   O1 return: Ok | Err(InvalidSlash(message)); pure
// PATHS: P-BOND | P-UNB | P-OTHER (is_slash_burnable)
//        | P-ACTIVE | P-COLLATERAL | P-NONE | P-SLASHED | P-QUEUED (validate_slash_target)
// INPUT PARTITIONS:
//   P-BOND: lock {0, L, MAX} x h {0, L-1, L, L+1, MAX} -> true
//   P-UNB: L = 100: h = L-1 true | h = L false | h = L+1 false; L = MAX: h = MAX-1 true | h = MAX false; L = 0 false
//   P-OTHER: every other decodable type x the same lock/h grid -> false
//   target state (active, locked_collateral): FF refused | FT admitted | TF admitted | TT admitted
//   slashed or slash_queued, with locked collateral -> the existing refusal text
// MATRIX:
//   O1 x {P-BOND, P-UNB, P-OTHER} -> inc_i_257_m2_req_unbond_010_burnable_truth_table
//   O1 x {FF, FT, TF, TT} -> inc_i_257_m2_req_unbond_011_target_admits_inactive_with_locked_collateral
//   O1 x {P-SLASHED, P-QUEUED} -> inc_i_257_m2_req_unbond_011_slashed_or_queued_refusals_unchanged

use crypto::Hash;
use doli_core::transaction::{Output, OutputType};
use doli_core::validation::{
    is_slash_burnable, validate_slash_target, SlashTargetState, ValidationError,
};

use super::inc_i_255_slash_evidence::{accused, armed_ctxs, armed_target_ctxs, canonical_tx};

const L: u64 = 100;
const M: u64 = u64::MAX;

fn out(output_type: OutputType, lock_until: u64) -> Output {
    Output {
        output_type,
        amount: 5_000,
        pubkey_hash: Hash::from_bytes([0x33; 32]),
        lock_until,
        extra_data: Vec::new(),
    }
}

fn all_output_types() -> Vec<OutputType> {
    (0..=255u8).filter_map(OutputType::from_u8).collect()
}

// REQ-UNBOND-010 — Decision: a failure means the burn misses a locked Unbonding output (offender keeps withdrawn value), burns an unlocked one (races a legal spend), or touches a type it must not (Normal, NFT, ...).
#[test]
fn inc_i_257_m2_req_unbond_010_burnable_truth_table() {
    let types = all_output_types();
    assert!(
        types.contains(&OutputType::Unbonding) && types.len() >= 15,
        "NON-VACUITY: type enumeration"
    );
    let grid = [0, L - 1, L, L + 1, M];
    for lock in [0, L, M] {
        for h in grid {
            assert!(
                is_slash_burnable(&out(OutputType::Bond, lock), h),
                "Bond lock={lock} h={h} must be burnable"
            );
        }
    }
    let unb = |lock, h| is_slash_burnable(&out(OutputType::Unbonding, lock), h);
    assert!(
        unb(L, L - 1),
        "Unbonding at lock_until - 1 is locked, so burnable"
    );
    assert!(
        !unb(L, L),
        "Unbonding at lock_until is spendable, so not burnable"
    );
    assert!(!unb(L, L + 1), "Unbonding after lock_until is not burnable");
    assert!(
        unb(M, M - 1),
        "Unbonding lock u64::MAX at u64::MAX - 1 is burnable"
    );
    assert!(
        !unb(M, M),
        "Unbonding lock u64::MAX at u64::MAX is not burnable"
    );
    assert!(!unb(0, 0), "Unbonding with lock 0 is never burnable");
    for ty in types
        .into_iter()
        .filter(|t| !matches!(t, OutputType::Bond | OutputType::Unbonding))
    {
        for lock in [0, L, M] {
            for h in grid {
                assert!(
                    !is_slash_burnable(&out(ty, lock), h),
                    "{ty:?} lock={lock} h={h} must not be burnable"
                );
            }
        }
    }
}

/// An active accused holds its genesis Bond UTXOs (slot 0); an exited one holds none.
fn state(active: bool, locked_collateral: bool) -> SlashTargetState {
    SlashTargetState {
        active,
        locked_collateral,
        oldest_bond_slot: active.then_some(0),
        ..SlashTargetState::default()
    }
}

fn target_msg(r: &Result<(), ValidationError>) -> Option<String> {
    match r {
        Err(ValidationError::InvalidSlash(m)) => Some(m.clone()),
        _ => None,
    }
}

// REQ-UNBOND-011 / REQ-UNBOND-012 — Decision: a failure means a slash against an exited accused that still holds locked funds is refused (the funds escape), or one with nothing left to burn is admitted (a mineable no-op).
#[test]
fn inc_i_257_m2_req_unbond_011_target_admits_inactive_with_locked_collateral() {
    let tx = canonical_tx();
    let pk = *accused().public_key();
    for ctx in armed_target_ctxs() {
        let h = ctx.current_height;
        let judge = |s| validate_slash_target(&tx, &ctx.clone().with_slash_targets([(pk, s)]));
        let none = judge(state(false, false));
        assert!(
            target_msg(&none).is_some_and(|m| m.contains("not an active or unbonding producer")),
            "h={h}: inactive without locked collateral keeps the existing refusal; got {none:?}"
        );
        for (active, collateral) in [(false, true), (true, false), (true, true)] {
            let r = judge(state(active, collateral));
            assert_eq!(
                r,
                Ok(()),
                "h={h}: active={active} locked_collateral={collateral} must be admitted"
            );
        }
    }
}

// REQ-UNBOND-011 — Decision: a failure means locked collateral lets a second slash through for an accused that is already Slashed or has a queued Slash (double burn accounting).
#[test]
fn inc_i_257_m2_req_unbond_011_slashed_or_queued_refusals_unchanged() {
    let tx = canonical_tx();
    let pk = *accused().public_key();
    let slashed = SlashTargetState {
        slashed: true,
        ..state(false, true)
    };
    let queued = SlashTargetState {
        slash_queued: true,
        ..state(true, true)
    };
    for ctx in armed_ctxs() {
        let h = ctx.current_height;
        let r = validate_slash_target(&tx, &ctx.clone().with_slash_targets([(pk, slashed)]));
        assert!(
            target_msg(&r).is_some_and(|m| m.contains("already slashed")),
            "h={h}: Slashed accused with collateral; got {r:?}"
        );
        let r = validate_slash_target(&tx, &ctx.clone().with_slash_targets([(pk, queued)]));
        assert!(
            target_msg(&r).is_some_and(|m| m.contains("pending slash")),
            "h={h}: queued accused with collateral; got {r:?}"
        );
    }
}
