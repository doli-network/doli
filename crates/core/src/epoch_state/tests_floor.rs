//! INC-I-190 F3 / D2 (REQ-AUTH-012.*): bound the `MIN_PRODUCERS_FLOOR` fallback.
//!
//! TDD RED. These tests are written against the NEW `compute_live_producer_list`
//! signature (2 extra params: the activation height and `prev`; returns
//! `(Vec<PublicKey>, FloorOutcome)`) and do not compile against the current 8-arg
//! signature. That compile failure IS the required red evidence.
//!
//! BEHAVIORAL RED (pre-fix, existing 8-arg signature, throwaway probe):
//!   plain fallback branch  — observed len = 84, expected <= 50
//!   ghost-filtered branch  — observed len = 80, expected <= 50
//!
//! Implementation files the developer edits to turn these green:
//!   - `floor.rs`        — NEW. `compute_live_producer_list` + `FloorOutcome` live here.
//!   - `mod.rs`          — thin re-export; `EpochDerivationInput` gains the AH
//!     field; `derive_at_boundary` passes `Some(&prev.producer_list)`.
//!   - `defaults.rs`     — `inc_i_190_floor_bound_activation_height` per-network arms.
//!   - `env_loader.rs`   — non-mainnet env override for the same field.
//!   - `rewards.rs`      — rebuild caller: passes `prev = None`, reads `FloorOutcome`.
//!   - `post_commit.rs`  — populates the new `EpochDerivationInput` field.
//!   - `fork_recovery.rs`— populates the new `EpochDerivationInput` field.
//!   - `tests.rs`        — existing `EpochDerivationInput` literals need the new field.
//!   - `tests_m2.rs`     — existing `compute_live_producer_list` call sites need 2 args.
//
// OUTPUT CONTRACT: fn compute_live_producer_list(...)
// O1: the returned producer list (Vec<PublicKey>) — membership, length, and order
// O2: the returned FloorOutcome branch discriminator (NotTriggered | PreviousEpochList | BoundedActiveSet | LegacyUnbounded)
// O3: the observability log line on an AH-gated fallback exit — NOT unit-assertable from a pure fn; covered by the live GS-009 log grep (REQ-AUTH-012.13)
// PATHS: P1 no-fallback | P2 pref-(a)-accepted | P3 pref-(a)-below-floor->(b) | P4 (b)-bounded | P5 below-AH-ghost-filtered | P6 below-AH-plain | P7 pre-epoch_prune-proportional
// INPUT PARTITIONS:
//   IP1 (P1): attested >= MIN_PRODUCERS_FLOOR, height >= AH — filtered list kept verbatim, no fallback
//   IP2 (P4): attested == 0 (blackout), prev = None, 84-producer registry, ghosts = 0 — bounded to cap
//   IP3 (P4): attested == 0, prev = None, 84 registry with 4 ghosts — ghost arm, bounded to cap
//   IP4 (P4): 0 < attested < MIN_PRODUCERS_FLOOR, prev = None — same bounded exit as IP2
//   IP5 (P2): prev present, |prev ∩ active| = 25 >= floor and <= cap — result is prev, seniority-ordered
//   IP6 (P2): prev present, |prev ∩ active| = 84 > cap (prev is itself uncapped) — must re-truncate to cap
//   IP7 (P2): prev contains a ghost — ghost dropped, remainder still >= floor
//   IP8 (P3): |prev ∩ active| ∈ {0, 2} (small prev / stale prev / empty prev) — falls through to (b)
//   IP9 (P6): height < AH, blackout, ghosts = 0 — byte-identical to `active_producers.to_vec()`
//   IP10 (P5): height < AH, blackout, ghosts > 0 — byte-identical to today's ghost-filtered collect
//   IP11 (P7): height < epoch_prune AH, attested 12 < effective*2/3 — proportional floor fires, uncapped, unchanged
//   IP12 (P7): height < epoch_prune AH, attested 60 >= effective*2/3 — proportional floor does not fire
//   IP13 (P4/P2): 3-producer registry, AH = 0 — result non-empty and >= MIN_PRODUCERS_FLOOR (no deadlock)
//   IP14 (P4 then P2): two consecutive blackout boundaries, registry unchanged — cap holds on both
//   IP15 (P2/P4): adversary-chosen attested subsets {∅, 2 most-junior, 1 mid-registry} — cap holds for each
// MATRIX: O1 asserted on IP1-IP15 (membership + length + order where the order is specified);
//   O2 asserted on IP1 (NotTriggered), IP5/IP6/IP7/IP14b (PreviousEpochList), IP2/IP3/IP4/IP8/IP13/IP14a (BoundedActiveSet),
//   IP9/IP10/IP11/IP12 (LegacyUnbounded / NotTriggered); O3 out of unit scope, see REQ-AUTH-012.13.

use super::*;
use crate::consensus::{ACTIVE_PRODUCERS_CAP, MIN_PRODUCERS_FLOOR};
use std::collections::{HashMap, HashSet};

const BPE: u64 = 100;
const EPOCH: u64 = 10;
/// The floor-bound activation height under test.
const FLOOR_AH: u64 = 60_000;
const AT_AH: u64 = 60_000;
const BELOW_AH: u64 = 59_999;
/// `registered_at` base that keeps a producer inside GHOST_EXCLUSION_GRACE_EPOCHS
/// at EPOCH=10 / BPE=100 (reg_epoch 7, 10-7 = 3, not > 3).
const RECENT: u64 = 700;
const GHOST_OFF: u64 = u64::MAX;
const GHOST_ON: u64 = 0;
const PRUNE_ON: u64 = 0;

fn make_pubkey(seed: u8) -> PublicKey {
    let mut bytes = [0u8; 32];
    bytes[0] = seed;
    PublicKey::from_bytes(bytes)
}

fn keys(range: std::ops::RangeInclusive<u8>) -> Vec<PublicKey> {
    range.map(make_pubkey).collect()
}

fn reg_recent(pks: &[PublicKey]) -> HashMap<PublicKey, u64> {
    pks.iter()
        .enumerate()
        .map(|(i, k)| (*k, RECENT + i as u64))
        .collect()
}

/// First `n_ghost` entries registered at epoch 0 (ghosts at EPOCH=10); rest recent.
fn reg_with_ghosts(pks: &[PublicKey], n_ghost: usize) -> HashMap<PublicKey, u64> {
    pks.iter()
        .enumerate()
        .map(|(i, k)| {
            (
                *k,
                if i < n_ghost {
                    i as u64
                } else {
                    RECENT + i as u64
                },
            )
        })
        .collect()
}

fn union(pks: &[PublicKey]) -> HashSet<&PublicKey> {
    pks.iter().collect()
}

fn no_attesters<'a>() -> HashSet<&'a PublicKey> {
    HashSet::new()
}

#[allow(clippy::too_many_arguments)]
fn call(
    active: &[PublicKey],
    attested: &HashSet<&PublicKey>,
    reg: &HashMap<PublicKey, u64>,
    height: u64,
    ghost_ah: u64,
    prune_ah: u64,
    floor_ah: u64,
    prev: Option<&[PublicKey]>,
) -> (Vec<PublicKey>, FloorOutcome) {
    compute_live_producer_list(
        active, attested, reg, BPE, EPOCH, height, ghost_ah, prune_ah, floor_ah, prev,
    )
}

// ============================================================================
// REQ-AUTH-012.6 — INV-EPOCH-005: every at/above-AH fallback exit is cap-bounded
// ============================================================================

// REQ-AUTH-012.6 — Decision: a failure means the uncapped registry can still reach the
// scheduler and the bitfield encoder order, i.e. the 2026-08-27 84-producer schedule is
// still reachable and the whole fix is inert.
#[test]
fn test_floor_fallback_bounded_to_cap() {
    let active = keys(1..=84);
    let reg = reg_recent(&active);

    // IP2 — plain branch: ghost exclusion off, total attestation blackout.
    let (r, outcome) = call(
        &active,
        &no_attesters(),
        &reg,
        AT_AH,
        GHOST_OFF,
        PRUNE_ON,
        FLOOR_AH,
        None,
    );
    assert!(
        r.len() <= ACTIVE_PRODUCERS_CAP,
        "plain fallback returned {} (pre-fix observed 84)",
        r.len()
    );
    assert_eq!(r.len(), ACTIVE_PRODUCERS_CAP);
    assert_eq!(outcome, FloorOutcome::BoundedActiveSet);
    // Bounded set = the 50 most senior, registered_at asc.
    assert_eq!(r, keys(1..=50));

    // IP3 — ghost arm: 4 ghosts, 80 survivors, must still cap.
    let reg_g = reg_with_ghosts(&active, 4);
    let (rg, outcome_g) = call(
        &active,
        &no_attesters(),
        &reg_g,
        AT_AH,
        GHOST_ON,
        PRUNE_ON,
        FLOOR_AH,
        None,
    );
    assert!(
        rg.len() <= ACTIVE_PRODUCERS_CAP,
        "ghost-filtered fallback returned {} (pre-fix observed 80)",
        rg.len()
    );
    assert_eq!(rg.len(), ACTIVE_PRODUCERS_CAP);
    assert_eq!(outcome_g, FloorOutcome::BoundedActiveSet);
    for ghost in keys(1..=4) {
        assert!(
            !rg.contains(&ghost),
            "ghost leaked into the bounded fallback"
        );
    }
    assert_eq!(rg, keys(5..=54));

    // IP4 — non-empty but sub-floor attested set takes the same bounded exit.
    let two = keys(1..=2);
    assert!(two.len() < MIN_PRODUCERS_FLOOR);
    let (r2, outcome2) = call(
        &active,
        &union(&two),
        &reg,
        AT_AH,
        GHOST_OFF,
        PRUNE_ON,
        FLOOR_AH,
        None,
    );
    assert!(r2.len() <= ACTIVE_PRODUCERS_CAP);
    assert_eq!(outcome2, FloorOutcome::BoundedActiveSet);
}

// ============================================================================
// REQ-AUTH-012.8 — preference (a): the previous epoch's producer_list
// ============================================================================

// REQ-AUTH-012.8 — Decision: a failure means the fallback discards the epoch's known-good
// membership and re-admits the whole registry, so a blackout still reshuffles the schedule
// instead of freezing it.
#[test]
fn test_floor_fallback_prefers_prev_epoch_list() {
    // IP5
    let active = keys(1..=84);
    let reg = reg_recent(&active);
    let prev = keys(60..=84); // 25 producers, all still in the registry

    let (r, outcome) = call(
        &active,
        &no_attesters(),
        &reg,
        AT_AH,
        GHOST_OFF,
        PRUNE_ON,
        FLOOR_AH,
        Some(&prev),
    );

    assert_eq!(outcome, FloorOutcome::PreviousEpochList);
    assert_eq!(r.len(), 25, "intersection is 25, above the floor");
    // registered_at asc, pubkey-bytes tiebreak.
    assert_eq!(r, keys(60..=84));
    assert_ne!(r, active, "must NOT be the whole active set");
    assert!(r.len() <= ACTIVE_PRODUCERS_CAP);
}

// ============================================================================
// REQ-AUTH-012.7 — below the AH the function is byte-identical to today
// ============================================================================

// REQ-AUTH-012.7 — Decision: a failure means a node running the new binary computes a
// different producer_list for already-sealed history than the fleet did, which is a
// retroactive consensus change and a replay/bitfield-index fork.
#[test]
fn test_floor_fallback_below_ah_is_byte_identical() {
    let active = keys(1..=84);
    let reg = reg_recent(&active);
    // `prev` is deliberately supplied: below the AH it MUST be ignored.
    let prev = keys(60..=84);

    // IP9 — plain branch: exactly today's `active_producers.to_vec()`, same order.
    let (r, outcome) = call(
        &active,
        &no_attesters(),
        &reg,
        BELOW_AH,
        GHOST_OFF,
        PRUNE_ON,
        FLOOR_AH,
        Some(&prev),
    );
    assert_eq!(outcome, FloorOutcome::LegacyUnbounded);
    assert_eq!(r.len(), active.len());
    for (i, (got, want)) in r.iter().zip(active.iter()).enumerate() {
        assert_eq!(got, want, "order diverged at index {i}");
    }
    assert_eq!(r, active);

    // IP10 — ghost arm: exactly today's ghost-filtered collect, in active order.
    let reg_g = reg_with_ghosts(&active, 4);
    let expected_ghost_filtered = keys(5..=84);
    let (rg, outcome_g) = call(
        &active,
        &no_attesters(),
        &reg_g,
        BELOW_AH,
        GHOST_ON,
        PRUNE_ON,
        FLOOR_AH,
        Some(&prev),
    );
    assert_eq!(outcome_g, FloorOutcome::LegacyUnbounded);
    assert_eq!(rg.len(), 80);
    for (i, (got, want)) in rg.iter().zip(expected_ghost_filtered.iter()).enumerate() {
        assert_eq!(got, want, "ghost-filtered order diverged at index {i}");
    }
    assert_eq!(rg, expected_ghost_filtered);
}

// ============================================================================
// REQ-AUTH-012.4 / .15 — (a) falls through to (b) when it cannot reach the floor
// ============================================================================

// REQ-AUTH-012.4/.15 — Decision: a failure means a shrunken previous list can pin the
// schedule below MIN_PRODUCERS_FLOOR, which is the deadlock the floor exists to prevent.
#[test]
fn test_floor_fallback_prev_below_floor_falls_through() {
    // IP8
    let active = keys(1..=60);
    let reg = reg_recent(&active);

    // Shape 1: prev itself is below the floor.
    let prev_small = keys(1..=2);
    let (r1, o1) = call(
        &active,
        &no_attesters(),
        &reg,
        AT_AH,
        GHOST_OFF,
        PRUNE_ON,
        FLOOR_AH,
        Some(&prev_small),
    );
    assert_eq!(o1, FloorOutcome::BoundedActiveSet);
    assert_eq!(r1.len(), ACTIVE_PRODUCERS_CAP);

    // Shape 2: prev is large but only 2 members survive the intersection with the registry.
    let prev_stale = vec![
        make_pubkey(1),
        make_pubkey(2),
        make_pubkey(200),
        make_pubkey(201),
        make_pubkey(202),
    ];
    let (r2, o2) = call(
        &active,
        &no_attesters(),
        &reg,
        AT_AH,
        GHOST_OFF,
        PRUNE_ON,
        FLOOR_AH,
        Some(&prev_stale),
    );
    assert_eq!(o2, FloorOutcome::BoundedActiveSet);
    assert_eq!(r2.len(), ACTIVE_PRODUCERS_CAP);

    // Shape 3: prev present but empty.
    let (r3, o3) = call(
        &active,
        &no_attesters(),
        &reg,
        AT_AH,
        GHOST_OFF,
        PRUNE_ON,
        FLOOR_AH,
        Some(&[]),
    );
    assert_eq!(o3, FloorOutcome::BoundedActiveSet);
    assert_eq!(r3.len(), ACTIVE_PRODUCERS_CAP);

    // Shape 4: prev absent (the rebuild path, REQ-AUTH-012.10).
    let (r4, o4) = call(
        &active,
        &no_attesters(),
        &reg,
        AT_AH,
        GHOST_OFF,
        PRUNE_ON,
        FLOOR_AH,
        None,
    );
    assert_eq!(o4, FloorOutcome::BoundedActiveSet);
    assert_eq!(r4, r3, "None and empty prev must agree");
}

// REQ-AUTH-012.4/.15 — Decision: a failure means preference (a) re-admits producers that
// ghost exclusion already removed, silently undoing INC-I-046 at every floor trip.
#[test]
fn test_floor_fallback_ghost_excluded_from_prev() {
    // IP7
    let active = keys(1..=10);
    // Index 6 (seed 7) registered at epoch 0 => ghost at EPOCH=10; the rest are recent.
    let reg: HashMap<PublicKey, u64> = active
        .iter()
        .enumerate()
        .map(|(i, k)| (*k, if i == 6 { 0 } else { RECENT + i as u64 }))
        .collect();
    let prev = keys(1..=7); // contains the ghost

    let (r, outcome) = call(
        &active,
        &no_attesters(),
        &reg,
        AT_AH,
        GHOST_ON,
        PRUNE_ON,
        FLOOR_AH,
        Some(&prev),
    );

    assert_eq!(outcome, FloorOutcome::PreviousEpochList);
    assert!(
        !r.contains(&make_pubkey(7)),
        "ghost from prev.producer_list must be excluded by preference (a)"
    );
    assert_eq!(r.len(), 6);
    assert!(r.len() >= MIN_PRODUCERS_FLOOR);
    assert_eq!(r, keys(1..=6));
}

// ============================================================================
// REQ-AUTH-012-SEC-001 — adversarial attestation withholding
// ============================================================================

// REQ-AUTH-012-SEC-001 — Decision: a failure means a withholding cartel still picks the
// schedule size by choosing who attests, i.e. peer-controlled input expands a consensus
// set past its hard cap.
#[test]
fn test_floor_fallback_adversarial_withholding_cartel_bounded() {
    // IP15 (+ IP6 on shape 4)
    let active = keys(1..=84);
    let reg = reg_recent(&active);
    let full_prev = active.clone(); // prev.producer_list is NOT itself cap-bounded

    // Shape 1: total withholding, no prev.
    let (r1, _) = call(
        &active,
        &no_attesters(),
        &reg,
        AT_AH,
        GHOST_OFF,
        PRUNE_ON,
        FLOOR_AH,
        None,
    );
    assert!(r1.len() <= ACTIVE_PRODUCERS_CAP, "shape 1 => {}", r1.len());

    // Shape 2: cartel leaves exactly 2 attesters, chosen as the most junior producers.
    let junior = keys(83..=84);
    let (r2, _) = call(
        &active,
        &union(&junior),
        &reg,
        AT_AH,
        GHOST_OFF,
        PRUNE_ON,
        FLOOR_AH,
        None,
    );
    assert!(r2.len() <= ACTIVE_PRODUCERS_CAP, "shape 2 => {}", r2.len());

    // Shape 3: cartel leaves exactly 1 attester, chosen mid-registry.
    let one = vec![make_pubkey(42)];
    let (r3, _) = call(
        &active,
        &union(&one),
        &reg,
        AT_AH,
        GHOST_OFF,
        PRUNE_ON,
        FLOOR_AH,
        None,
    );
    assert!(r3.len() <= ACTIVE_PRODUCERS_CAP, "shape 3 => {}", r3.len());

    // Shape 4: total withholding while prev carries the full uncapped 84-entry list.
    // Preference (a) must re-truncate or the fix leaks the defect it removes.
    let (r4, o4) = call(
        &active,
        &no_attesters(),
        &reg,
        AT_AH,
        GHOST_OFF,
        PRUNE_ON,
        FLOOR_AH,
        Some(&full_prev),
    );
    assert_eq!(o4, FloorOutcome::PreviousEpochList);
    assert!(r4.len() <= ACTIVE_PRODUCERS_CAP, "shape 4 => {}", r4.len());
    assert_eq!(r4.len(), ACTIVE_PRODUCERS_CAP);

    // Shape 5: same, with ghosts in play so the adversary drives the ghost arm too.
    let reg_g = reg_with_ghosts(&active, 4);
    let (r5, _) = call(
        &active,
        &union(&one),
        &reg_g,
        AT_AH,
        GHOST_ON,
        PRUNE_ON,
        FLOOR_AH,
        Some(&full_prev),
    );
    assert!(r5.len() <= ACTIVE_PRODUCERS_CAP, "shape 5 => {}", r5.len());
}

// ============================================================================
// REQ-AUTH-012.7 — the pre-epoch_prune proportional branch is untouched
// ============================================================================

// REQ-AUTH-012.7 — Decision: a failure means the new AH leaked into a branch that only
// runs on pre-INC-I-116 history, retroactively changing the producer_list of blocks the
// fleet already sealed.
#[test]
fn test_floor_fallback_pre_epoch_prune_branch_untouched() {
    let active = keys(1..=84);
    let reg = reg_recent(&active);
    let prune_ah = 1_000_000; // height is BELOW epoch_prune activation
    let height = 500;
    let prev = keys(60..=84); // supplied but must be ignored on this branch

    // IP11 — proportional floor fires: 12 attested < effective_active * 2 / 3 = 56.
    let attested = keys(1..=12);
    let (r, outcome) = call(
        &active,
        &union(&attested),
        &reg,
        height,
        GHOST_OFF,
        prune_ah,
        0, // floor-bound AH already crossed: it must still not reach this branch
        Some(&prev),
    );
    assert_eq!(outcome, FloorOutcome::LegacyUnbounded);
    assert_eq!(
        r.len(),
        84,
        "proportional floor still admits the whole registry"
    );
    assert_eq!(r, active);

    // IP12 — proportional floor does NOT fire: 60 attested >= 56.
    let attested_ok = keys(1..=60);
    let (r2, outcome2) = call(
        &active,
        &union(&attested_ok),
        &reg,
        height,
        GHOST_OFF,
        prune_ah,
        0,
        Some(&prev),
    );
    assert_eq!(outcome2, FloorOutcome::NotTriggered);
    assert_eq!(r2, attested_ok);
}

// ============================================================================
// REQ-AUTH-012.15 — small networks must not deadlock under the new acceptance test
// ============================================================================

// REQ-AUTH-012.15 — Decision: a failure means devnet (AH = 0) halts the moment attestation
// stops, because the bounded fallback returns fewer producers than the scheduler needs.
#[test]
fn test_floor_fallback_small_devnet_no_deadlock() {
    // IP13
    let active = keys(1..=3);
    let reg = reg_recent(&active);
    let devnet_ah = 0;
    let height = 500;
    let prev_two = keys(1..=2);
    let prev_three = keys(1..=3);

    for prev in [
        None,
        Some(&prev_two[..]),
        Some(&prev_three[..]),
        Some(&[][..]),
    ] {
        let (r, outcome) = call(
            &active,
            &no_attesters(),
            &reg,
            height,
            GHOST_OFF,
            PRUNE_ON,
            devnet_ah,
            prev,
        );
        assert!(!r.is_empty(), "devnet fallback returned an empty schedule");
        assert_eq!(r.len(), 3, "all 3 devnet producers must stay schedulable");
        assert!(r.len() >= MIN_PRODUCERS_FLOOR);
        assert!(r.len() <= ACTIVE_PRODUCERS_CAP);
        assert_ne!(
            outcome,
            FloorOutcome::NotTriggered,
            "blackout must take a fallback branch"
        );
    }
}

// ============================================================================
// Characterization — two consecutive floor-triggering boundaries
// (analyst "what I don't understand" #1)
// ============================================================================

// REQ-AUTH-012.6 — Decision: a failure means repeated floor trips either grow the set past
// the cap or churn membership non-deterministically across boundaries.
//
// OBSERVED (asserted below): with the registry unchanged, boundary 2 returns EXACTLY the
// boundary-1 set. Membership is pinned, not rotated, for as long as the floor keeps firing;
// the tier promotion path that normally rotates membership does not run while the bounded
// list is <= the cap. The pinning is bounded and deterministic — it never grows and never
// drops below the floor — and it clears at the first non-floor boundary.
#[test]
fn test_floor_fallback_two_consecutive_boundaries() {
    // IP14
    let active = keys(1..=60);
    let reg = reg_recent(&active);

    // Boundary 1: no prev => bounded active set.
    let (b1, o1) = call(
        &active,
        &no_attesters(),
        &reg,
        AT_AH,
        GHOST_OFF,
        PRUNE_ON,
        FLOOR_AH,
        None,
    );
    assert_eq!(o1, FloorOutcome::BoundedActiveSet);
    assert!(b1.len() <= ACTIVE_PRODUCERS_CAP);
    assert_eq!(b1.len(), ACTIVE_PRODUCERS_CAP);

    // Boundary 2: prev = boundary 1's list, registry unchanged, still blacked out.
    let (b2, o2) = call(
        &active,
        &no_attesters(),
        &reg,
        AT_AH + 1,
        GHOST_OFF,
        PRUNE_ON,
        FLOOR_AH,
        Some(&b1),
    );
    assert_eq!(o2, FloorOutcome::PreviousEpochList);
    assert!(b2.len() <= ACTIVE_PRODUCERS_CAP, "cap holds on boundary 2");
    assert_eq!(
        b2, b1,
        "membership is pinned across consecutive floor trips"
    );
    assert!(b2.len() >= MIN_PRODUCERS_FLOOR);
}

// ============================================================================
// REQ-AUTH-012.10 — the branch taken is reported, so callers never re-test the predicate
// ============================================================================

// REQ-AUTH-012.10 — Decision: a failure means `rewards.rs` has to re-derive "did the floor
// fire" with its own copy of the predicate, which is the INC-I-116 dual-implementation
// divergence shape that this function was extracted to remove.
#[test]
fn test_floor_outcome_reports_branch_taken() {
    let active = keys(1..=84);
    let reg = reg_recent(&active);
    let prev = keys(60..=84);

    // IP1 — no fallback: 40 attested is above the floor.
    let attested = keys(1..=40);
    let (r_none, o_none) = call(
        &active,
        &union(&attested),
        &reg,
        AT_AH,
        GHOST_OFF,
        PRUNE_ON,
        FLOOR_AH,
        Some(&prev),
    );
    assert_eq!(o_none, FloorOutcome::NotTriggered);
    assert_eq!(r_none, attested);

    // Preference (a).
    let (_, o_prev) = call(
        &active,
        &no_attesters(),
        &reg,
        AT_AH,
        GHOST_OFF,
        PRUNE_ON,
        FLOOR_AH,
        Some(&prev),
    );
    assert_eq!(o_prev, FloorOutcome::PreviousEpochList);

    // Fallback (b).
    let (_, o_bounded) = call(
        &active,
        &no_attesters(),
        &reg,
        AT_AH,
        GHOST_OFF,
        PRUNE_ON,
        FLOOR_AH,
        None,
    );
    assert_eq!(o_bounded, FloorOutcome::BoundedActiveSet);

    // Below the AH.
    let (_, o_legacy) = call(
        &active,
        &no_attesters(),
        &reg,
        BELOW_AH,
        GHOST_OFF,
        PRUNE_ON,
        FLOOR_AH,
        Some(&prev),
    );
    assert_eq!(o_legacy, FloorOutcome::LegacyUnbounded);

    // All four discriminate; the caller learns "a fallback fired" from O2 alone.
    for o in [o_prev, o_bounded, o_legacy] {
        assert_ne!(o, FloorOutcome::NotTriggered);
        assert_ne!(o, o_none);
    }
    assert_ne!(o_prev, o_bounded);
    assert_ne!(o_bounded, o_legacy);
}

// ============================================================================
// REQ-AUTH-012.9 — the AH is threaded through EpochDerivationInput
// ============================================================================

// REQ-AUTH-012.9 — Decision: a failure means the bounded fallback exists but the live
// epoch-boundary path never reaches it, so the fix is dead code on every node.
#[test]
fn test_derive_at_boundary_threads_floor_bound_ah() {
    let active = keys(1..=84);
    let reg = reg_recent(&active);

    let mut prev = EpochState::genesis();
    prev.epoch = EPOCH - 1;
    prev.producer_list = keys(60..=84);
    // One attester keeps have_full_history true while staying below MIN_PRODUCERS_FLOOR.
    prev.attested_sets[0].insert(make_pubkey(60));

    let input = EpochDerivationInput {
        active_producers: active.clone(),
        bond_counts: HashMap::new(),
        blocks_per_epoch: BPE,
        snap_attestation_skip_height: u64::MAX,
        height: AT_AH,
        epoch: EPOCH,
        registered_at: reg,
        ghost_exclusion_activation_height: GHOST_OFF,
        epoch_prune_activation_height: PRUNE_ON,
        inc_i_190_floor_bound_activation_height: FLOOR_AH,
    };

    let state = EpochState::derive_at_boundary(&prev, &input);

    assert!(
        state.producer_list.len() <= ACTIVE_PRODUCERS_CAP,
        "producer_list (the bitfield encoder order) escaped the cap: {}",
        state.producer_list.len()
    );
    assert_eq!(state.producer_list.len(), 25);
    assert!(state.active_list.len() <= ACTIVE_PRODUCERS_CAP);
    let mut expected = keys(60..=84);
    expected.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    assert_eq!(state.producer_list, expected);
}

// ============================================================================
// REV-I190-M4-F2 / AUDIT-P2-502 — the fallback never shrinks and never duplicates
// ============================================================================

// REV-I190-M4-F2 — Decision: a failure means a repeated pubkey both satisfies the floor
// alone and takes several scheduler slots, or the fallback returns an EMPTY schedule in
// a total blackout — the stall it exists to break.
#[test]
fn test_floor_fallback_never_shrinks_or_duplicates() {
    // IP16: prev repeats one active pubkey 6x. It dedups to 1 < MIN_PRODUCERS_FLOOR, so
    // (a) falls through to (b), a superset of deduped (a) and therefore never smaller.
    let active = keys(1..=10);
    let dup_prev = vec![make_pubkey(1); 6];
    let (r, o) = call(
        &active,
        &no_attesters(),
        &reg_recent(&active),
        AT_AH,
        GHOST_OFF,
        PRUNE_ON,
        FLOOR_AH,
        Some(&dup_prev),
    );
    assert_eq!(o, FloorOutcome::BoundedActiveSet);
    assert_eq!(r, keys(1..=10));

    // IP17: every active producer is a ghost — ghost exclusion must yield to the floor.
    let small = keys(1..=6);
    let (r2, o2) = call(
        &small,
        &no_attesters(),
        &reg_with_ghosts(&small, 6),
        AT_AH,
        GHOST_ON,
        PRUNE_ON,
        FLOOR_AH,
        None,
    );
    assert_eq!(o2, FloorOutcome::BoundedActiveSet);
    assert_eq!(r2, small, "all-ghost registry must still yield a schedule");
}
