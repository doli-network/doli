//! The epoch-boundary floor stage: the attestation filter plus the
//! deadlock-safety fallback that runs when too few producers attested.
//!
//! Extracted from `mod.rs` (INC-I-190 F3 / REQ-AUTH-012.12). `mod.rs` re-exports
//! both public items, so existing import paths are unchanged.

use std::collections::{HashMap, HashSet};

use crypto::PublicKey;

use crate::consensus::{ACTIVE_PRODUCERS_CAP, GHOST_EXCLUSION_GRACE_EPOCHS, MIN_PRODUCERS_FLOOR};

/// Which branch of the floor stage produced the returned list.
///
/// Returned alongside the list so callers learn "a fallback fired" without
/// re-testing `< MIN_PRODUCERS_FLOOR` themselves (the INC-I-116 duplicate-predicate
/// failure shape).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloorOutcome {
    /// No fallback fired — the attestation-filtered list was returned verbatim.
    NotTriggered,
    /// At/above the floor-bound AH: preference (a), the previous epoch's list.
    PreviousEpochList,
    /// At/above the floor-bound AH: fallback (b), the cap-bounded active set.
    BoundedActiveSet,
    /// Below the floor-bound AH (or on the pre-`epoch_prune` proportional branch):
    /// the historical uncapped fallback.
    LegacyUnbounded,
}

/// INC-I-116 M2: Pure function that computes the attestation-filtered + floor-adjusted
/// producer list. Extracted from `derive_at_boundary()` so `rewards.rs` can call the
/// same logic instead of maintaining a 100-line inline duplicate.
///
/// Callers are responsible for:
/// - Deciding whether to call this function at all (epoch<=1 → skip, use all active)
/// - Building `attested_union` from the appropriate source (epoch state lookback or block scan)
///
/// `prev` is the previous epoch's `producer_list` when the caller has one. The rebuild
/// path (`rewards.rs`) has none and passes `None`.
///
/// Returns the filtered (and possibly floor-adjusted) list plus the branch taken.
/// The list is UNSORTED except on the INC-I-190 bounded branches, which are ordered by
/// `registered_at` ascending. The caller sorts after if needed.
#[allow(clippy::too_many_arguments)]
pub fn compute_live_producer_list(
    active_producers: &[PublicKey],
    attested_union: &HashSet<&PublicKey>,
    registered_at: &HashMap<PublicKey, u64>,
    blocks_per_epoch: u64,
    epoch: u64,
    height: u64,
    ghost_exclusion_activation_height: u64,
    epoch_prune_activation_height: u64,
    inc_i_190_floor_bound_activation_height: u64,
    prev: Option<&[PublicKey]>,
) -> (Vec<PublicKey>, FloorOutcome) {
    // Step 1: Attestation filter — keep only producers present in attested_union
    let mut new_list: Vec<PublicKey> = active_producers
        .iter()
        .filter(|pk| attested_union.contains(pk))
        .copied()
        .collect();

    // Step 2: Ghost identification + counting
    let active_count = active_producers.len();
    let ghost_exclusion_active = height >= ghost_exclusion_activation_height && epoch > 1;

    let is_ghost = |pk: &PublicKey| -> bool {
        if !ghost_exclusion_active {
            return false;
        }
        if attested_union.contains(pk) {
            return false;
        }
        match registered_at.get(pk) {
            Some(&reg_height) => {
                let reg_epoch = reg_height.checked_div(blocks_per_epoch).unwrap_or(0);
                epoch.saturating_sub(reg_epoch) > GHOST_EXCLUSION_GRACE_EPOCHS
            }
            None => false, // Unknown registration: not a ghost (conservative)
        }
    };

    let ghost_count = if ghost_exclusion_active {
        active_producers.iter().filter(|pk| is_ghost(pk)).count()
    } else {
        0
    };
    let effective_active = active_count - ghost_count;

    let mut outcome = FloorOutcome::NotTriggered;

    // Step 3: Gated floor logic
    if height >= epoch_prune_activation_height {
        // Post-activation: absolute floor (MIN_PRODUCERS_FLOOR).
        if new_list.len() < MIN_PRODUCERS_FLOOR {
            if height >= inc_i_190_floor_bound_activation_height {
                // INC-I-190 F3: bounded fallback — never hand the scheduler more
                // than ACTIVE_PRODUCERS_CAP producers. Callers log off the
                // returned outcome; this function stays side-effect free.
                let (bounded, branch) =
                    bounded_fallback(active_producers, registered_at, prev, &is_ghost);
                new_list = bounded;
                outcome = branch;
            } else {
                new_list = legacy_fallback(active_producers, ghost_count > 0, &is_ghost);
                outcome = FloorOutcome::LegacyUnbounded;
            }
        }
    } else {
        // Pre-activation: VERBATIM proportional floor (byte-identical to pre-INC-I-116).
        // The INC-I-190 gate deliberately does not reach this branch: it only ever runs
        // on history sealed before epoch_prune activation.
        if new_list.len() < (effective_active * 2 / 3)
            || (new_list.is_empty() && effective_active > 0)
        {
            new_list = legacy_fallback(active_producers, ghost_count > 0, &is_ghost);
            outcome = FloorOutcome::LegacyUnbounded;
        }
    }

    (new_list, outcome)
}

/// The historical (uncapped) fallback. Byte-identical to the pre-INC-I-190 code.
fn legacy_fallback(
    active_producers: &[PublicKey],
    has_ghosts: bool,
    is_ghost: &dyn Fn(&PublicKey) -> bool,
) -> Vec<PublicKey> {
    if has_ghosts {
        active_producers
            .iter()
            .filter(|pk| !is_ghost(pk))
            .copied()
            .collect()
    } else {
        active_producers.to_vec()
    }
}

/// INC-I-190 F3: the cap-bounded fallback (REQ-AUTH-012.4/.5/.6).
///
/// Preference (a) freezes the previous epoch's membership; (b) is the seniority-ordered
/// active set. Both are truncated to `ACTIVE_PRODUCERS_CAP` — `prev` is NOT itself
/// cap-bounded, so (a) must re-truncate.
///
/// (a)'s candidates are deduplicated, which makes them a subset of (b)'s, so falling
/// through from (a) to (b) can never shrink the schedule. `prev` is peer-supplied on
/// the snap-sync path and is not guaranteed distinct (AUDIT-P2-502): without the dedup
/// one repeated pubkey could both satisfy the floor and take several scheduler slots.
fn bounded_fallback(
    active_producers: &[PublicKey],
    registered_at: &HashMap<PublicKey, u64>,
    prev: Option<&[PublicKey]>,
    is_ghost: &dyn Fn(&PublicKey) -> bool,
) -> (Vec<PublicKey>, FloorOutcome) {
    let from_prev: Vec<PublicKey> = match prev {
        Some(prev_list) if !prev_list.is_empty() => {
            let active_set: HashSet<&PublicKey> = active_producers.iter().collect();
            let mut seen: HashSet<PublicKey> = HashSet::new();
            let candidates = prev_list
                .iter()
                .filter(|pk| active_set.contains(*pk) && !is_ghost(pk) && seen.insert(**pk))
                .copied();
            seniority_sorted_capped(candidates, registered_at)
        }
        _ => Vec::new(),
    };

    if from_prev.len() >= MIN_PRODUCERS_FLOOR {
        debug_assert!(from_prev.len() <= ACTIVE_PRODUCERS_CAP);
        return (from_prev, FloorOutcome::PreviousEpochList);
    }

    let candidates = active_producers.iter().filter(|pk| !is_ghost(pk)).copied();
    let from_active = seniority_sorted_capped(candidates, registered_at);
    debug_assert!(from_active.len() >= from_prev.len());

    if from_active.is_empty() && !active_producers.is_empty() {
        // Every active producer is a ghost. Returning nothing schedules nobody and
        // stalls the chain permanently — the exact deadlock this fallback exists to
        // break — so ghost exclusion yields to the floor here (REV-I190-M4-F2).
        let all = seniority_sorted_capped(active_producers.iter().copied(), registered_at);
        return (all, FloorOutcome::BoundedActiveSet);
    }

    debug_assert!(from_active.len() <= ACTIVE_PRODUCERS_CAP);
    (from_active, FloorOutcome::BoundedActiveSet)
}

/// Seniority ordering: `registered_at` ascending, pubkey-bytes tiebreak, truncated
/// to the cap. The comparator is the tier sort of `derive_at_boundary`; the candidate
/// set is NOT — an unknown `registered_at` sorts last (most junior) here instead of
/// being dropped, because dropping would let the safety net return an empty schedule.
fn seniority_sorted_capped(
    candidates: impl Iterator<Item = PublicKey>,
    registered_at: &HashMap<PublicKey, u64>,
) -> Vec<PublicKey> {
    let mut with_reg: Vec<(PublicKey, u64)> = candidates
        .map(|pk| {
            let reg = registered_at.get(&pk).copied().unwrap_or(u64::MAX);
            (pk, reg)
        })
        .collect();

    with_reg.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
    });

    with_reg
        .into_iter()
        .take(ACTIVE_PRODUCERS_CAP)
        .map(|(pk, _)| pk)
        .collect()
}
