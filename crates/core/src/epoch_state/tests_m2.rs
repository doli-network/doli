//! INC-I-116 M2: extraction equivalence + decode-list correctness tests.
//!
//! Tests that `compute_live_producer_list()` produces IDENTICAL output to
//! the current inline `derive_at_boundary()` logic. Also tests the decode-list
//! bug fix (FILTER-02/FILTER-08) at rewards.rs:777.
//!
//! Workflow: redesign. Tests calling `compute_live_producer_list` FAIL until
//! the function is extracted (TDD red).

use super::*;
use crate::consensus::{ACTIVE_PRODUCERS_CAP, MIN_PRODUCERS_FLOOR};
use std::collections::{HashMap, HashSet};

fn make_pubkey(seed: u8) -> PublicKey {
    let mut bytes = [0u8; 32];
    bytes[0] = seed;
    PublicKey::from_bytes(bytes)
}

/// Derive producer_list via derive_at_boundary for equivalence reference.
#[allow(clippy::too_many_arguments)]
fn derive_producer_list(
    active: Vec<PublicKey>,
    attested: &[PublicKey],
    registered_at: HashMap<PublicKey, u64>,
    bpe: u64,
    epoch: u64,
    height: u64,
    ghost_ah: u64,
    prune_ah: u64,
) -> Vec<PublicKey> {
    let mut prev = EpochState::genesis();
    prev.epoch = epoch - 1;
    for pk in attested {
        prev.attested_sets[0].insert(*pk);
    }
    let input = EpochDerivationInput {
        active_producers: active,
        bond_counts: HashMap::new(),
        blocks_per_epoch: bpe,
        snap_attestation_skip_height: u64::MAX,
        height,
        epoch,
        registered_at,
        ghost_exclusion_activation_height: ghost_ah,
        epoch_prune_activation_height: prune_ah,
        inc_i_190_floor_bound_activation_height: u64::MAX,
    };
    EpochState::derive_at_boundary(&prev, &input).producer_list
}

fn reg_at_zero(pks: &[PublicKey]) -> HashMap<PublicKey, u64> {
    pks.iter().map(|pk| (*pk, 0u64)).collect()
}

/// Assert compute_live_producer_list matches derive_at_boundary reference.
#[allow(clippy::too_many_arguments)]
fn assert_extraction_matches(
    active: &[PublicKey],
    attested: &[PublicKey],
    registered_at: &HashMap<PublicKey, u64>,
    bpe: u64,
    epoch: u64,
    height: u64,
    ghost_ah: u64,
    prune_ah: u64,
) {
    let expected = derive_producer_list(
        active.to_vec(),
        attested,
        registered_at.clone(),
        bpe,
        epoch,
        height,
        ghost_ah,
        prune_ah,
    );
    let attested_union: HashSet<&PublicKey> = attested.iter().collect();
    let (result, _) = compute_live_producer_list(
        active,
        &attested_union,
        registered_at,
        bpe,
        epoch,
        height,
        ghost_ah,
        prune_ah,
        u64::MAX,
        None,
    );
    let mut sorted = result.clone();
    sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    assert_eq!(sorted, expected, "Extraction must match derive_at_boundary");
}

// ============================================================================
// Section 1: Extraction Equivalence (7 tests)
// ============================================================================

// OUTPUT CONTRACT: fn compute_live_producer_list(...) -> Vec<PublicKey>
// Outputs:
//   O1: return -- Vec<PublicKey>, filtered/floored producer list
// Paths:
//   P1-P7: see individual tests below (pre/post activation x floor x ghost)
// INPUT PARTITIONS:
//   P1a: 12/57 attested, prune OFF -- proportional floor fires, all 57
//   P2a: 2/3 attested, prune OFF -- filter applies, 2 retained
//   P3a: 12/57, prune ON -- 12 >= 3 floor, only 12 retained
//   P4a: 2/57, prune ON -- 2 < 3, fallback to all active
//   P5a: 5 real + 5 ghosts, prune OFF -- ghost adj, 5 retained
//   P6a: 2 real + 55 ghosts, prune ON -- floor + ghost fallback
//   P7a: 60 active, 40 attested, prune OFF -- floor passes, 40 retained
// MATRIX: 1 output x 7 partitions = 7 cells

// INC-I-116 M2 FILTER-08: pre-activation, proportional floor fires
#[test]
fn test_extract_equiv_pre_act_floor_fires() {
    let all: Vec<PublicKey> = (1..=57).map(make_pubkey).collect();
    let attested: Vec<PublicKey> = (1..=12).map(make_pubkey).collect();
    let reg = reg_at_zero(&all);
    let expected = derive_producer_list(
        all.clone(),
        &attested,
        reg.clone(),
        360,
        5,
        500,
        u64::MAX,
        u64::MAX,
    );
    assert_eq!(expected.len(), 57, "Sanity: floor fires, all 57");
    assert_extraction_matches(&all, &attested, &reg, 360, 5, 500, u64::MAX, u64::MAX);
}

// INC-I-116 M2 FILTER-08: pre-activation, proportional floor does NOT fire
#[test]
fn test_extract_equiv_pre_act_floor_passes() {
    let all: Vec<PublicKey> = (1..=3).map(make_pubkey).collect();
    let attested: Vec<PublicKey> = (1..=2).map(make_pubkey).collect();
    let reg = reg_at_zero(&all);
    let expected = derive_producer_list(
        all.clone(),
        &attested,
        reg.clone(),
        360,
        5,
        1800,
        u64::MAX,
        u64::MAX,
    );
    assert_eq!(expected.len(), 2, "Sanity: filter applies, 2 retained");
    assert_extraction_matches(&all, &attested, &reg, 360, 5, 1800, u64::MAX, u64::MAX);
}

// INC-I-116 M2: post-activation, absolute floor does NOT fire (12 >= 3)
#[test]
fn test_extract_equiv_post_act_floor_passes() {
    let all: Vec<PublicKey> = (1..=57).map(make_pubkey).collect();
    let attested: Vec<PublicKey> = (1..=12).map(make_pubkey).collect();
    let reg = reg_at_zero(&all);
    let expected = derive_producer_list(
        all.clone(),
        &attested,
        reg.clone(),
        360,
        5,
        500,
        u64::MAX,
        0,
    );
    assert_eq!(expected.len(), 12, "Sanity: post-activation prunes to 12");
    assert_extraction_matches(&all, &attested, &reg, 360, 5, 500, u64::MAX, 0);
}

// INC-I-116 M2: post-activation, absolute floor fires (2 < 3)
#[test]
fn test_extract_equiv_post_act_floor_fires() {
    let all: Vec<PublicKey> = (1..=57).map(make_pubkey).collect();
    let attested: Vec<PublicKey> = (1..=2).map(make_pubkey).collect();
    let reg = reg_at_zero(&all);
    let expected = derive_producer_list(
        all.clone(),
        &attested,
        reg.clone(),
        360,
        5,
        500,
        u64::MAX,
        0,
    );
    assert!(
        expected.len() >= MIN_PRODUCERS_FLOOR,
        "Sanity: fallback fires"
    );
    assert_extraction_matches(&all, &attested, &reg, 360, 5, 500, u64::MAX, 0);
}

// INC-I-116 M2: ghost exclusion + pre-activation
#[test]
fn test_extract_equiv_ghost_pre_act() {
    let real: Vec<PublicKey> = (1..=5).map(make_pubkey).collect();
    let ghosts: Vec<PublicKey> = (6..=10).map(make_pubkey).collect();
    let mut all = real.clone();
    all.extend_from_slice(&ghosts);
    let mut reg = HashMap::new();
    for pk in &real {
        reg.insert(*pk, 0);
    }
    for pk in &ghosts {
        reg.insert(*pk, 360);
    } // epoch 1, past grace at epoch 10
    let expected = derive_producer_list(
        all.clone(),
        &real,
        reg.clone(),
        360,
        10,
        10_680,
        0,
        u64::MAX,
    );
    assert_eq!(expected.len(), 5, "Sanity: ghost exclusion, 5 real");
    assert_extraction_matches(&all, &real, &reg, 360, 10, 10_680, 0, u64::MAX);
}

// INC-I-116 M2: ghost exclusion + post-activation, absolute floor fires
#[test]
fn test_extract_equiv_ghost_post_act() {
    let real: Vec<PublicKey> = (1..=2).map(make_pubkey).collect();
    let ghosts: Vec<PublicKey> = (3..=57).map(make_pubkey).collect();
    let mut all = real.clone();
    all.extend_from_slice(&ghosts);
    let mut reg = HashMap::new();
    for pk in &real {
        reg.insert(*pk, 0);
    }
    for pk in &ghosts {
        reg.insert(*pk, 360);
    }
    let expected = derive_producer_list(all.clone(), &real, reg.clone(), 360, 10, 10_680, 0, 0);
    assert!(!expected.is_empty(), "Sanity: fallback produces non-empty");
    assert_extraction_matches(&all, &real, &reg, 360, 10, 10_680, 0, 0);
}

// INC-I-116 M2 FILTER-08: >50 active exercises ACTIVE_PRODUCERS_CAP
#[test]
fn test_extract_equiv_tier_cap() {
    let all: Vec<PublicKey> = (1..=60).map(make_pubkey).collect();
    let attested: Vec<PublicKey> = (1..=40).map(make_pubkey).collect();
    let reg: HashMap<PublicKey, u64> = all
        .iter()
        .enumerate()
        .map(|(i, pk)| (*pk, i as u64 * 10))
        .collect();
    let expected = derive_producer_list(
        all.clone(),
        &attested,
        reg.clone(),
        360,
        5,
        1800,
        u64::MAX,
        u64::MAX,
    );
    assert_eq!(expected.len(), 40, "Sanity: 40 attested retained");
    assert_extraction_matches(&all, &attested, &reg, 360, 5, 1800, u64::MAX, u64::MAX);
}

// ============================================================================
// Section 2: Decode-List Correctness (FILTER-02/FILTER-08) — 3 tests
// ============================================================================

// OUTPUT CONTRACT: bitfield decode index correctness
// Outputs:
//   O1: correct_pk -- PublicKey from correct (pruned) decode list
//   O2: wrong_pk -- PublicKey from wrong (full) decode list
// Paths: P1: pruned vs full, P2: pre-act identity, P3: post-act divergence
// INPUT PARTITIONS:
//   P1a: index 5 in 30-entry pruned -> pk(12); in 60-entry full -> pk(6)
//   P2a: 12/57, floor fires -> producer_list==active, decode identical
//   P3a: 12/57, prune ON -> producer_list(12)!=active(57), list sizes differ
// MATRIX: 2 outputs x 3 partitions = 6 cells

// FILTER-02: pruned vs full decode lists produce different producers
#[test]
fn test_decode_list_correct_vs_wrong() {
    let all: Vec<PublicKey> = (1..=60).map(make_pubkey).collect();
    let mut sorted_all = all.clone();
    sorted_all.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

    let pruned: Vec<PublicKey> = (1..=60).filter(|s| s % 2 == 0).map(make_pubkey).collect();
    let mut sorted_pruned = pruned.clone();
    sorted_pruned.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

    // Index 5 in even-only list: 2,4,6,8,10,12 -> pk(12)
    // Index 5 in full list: 1,2,3,4,5,6 -> pk(6)
    assert_ne!(
        sorted_pruned[5], sorted_all[5],
        "Correct and wrong decode lists MUST produce different pubkeys"
    );
    assert_eq!(sorted_pruned[5], make_pubkey(12));
    assert_eq!(sorted_all[5], make_pubkey(6));
}

// FILTER-02: pre-activation identity -- when floor fires, lists are identical
#[test]
fn test_decode_list_pre_act_identity() {
    let all: Vec<PublicKey> = (1..=57).map(make_pubkey).collect();
    let attested: Vec<PublicKey> = (1..=12).map(make_pubkey).collect();
    let reg = reg_at_zero(&all);

    let mut prev = EpochState::genesis();
    prev.epoch = 4;
    for pk in &attested {
        prev.attested_sets[0].insert(*pk);
    }

    let input = EpochDerivationInput {
        active_producers: all.clone(),
        bond_counts: HashMap::new(),
        blocks_per_epoch: 360,
        snap_attestation_skip_height: u64::MAX,
        height: 500,
        epoch: 5,
        registered_at: reg,
        ghost_exclusion_activation_height: u64::MAX,
        epoch_prune_activation_height: u64::MAX,
        inc_i_190_floor_bound_activation_height: u64::MAX,
    };
    let state = EpochState::derive_at_boundary(&prev, &input);

    let mut active_sorted = all.clone();
    active_sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    assert_eq!(
        state.producer_list, active_sorted,
        "Pre-activation floor fires: producer_list == sorted active"
    );

    // Decode from either list gives same result
    for idx in [0, 10, 30, 56] {
        assert_eq!(state.producer_list.get(idx), active_sorted.get(idx));
    }
}

// FILTER-02: post-activation divergence -- producer_list(12) != active(57)
#[test]
fn test_decode_list_post_act_divergence() {
    let all: Vec<PublicKey> = (1..=57).map(make_pubkey).collect();
    let attested: Vec<PublicKey> = (1..=12).map(make_pubkey).collect();
    let reg = reg_at_zero(&all);

    let mut prev = EpochState::genesis();
    prev.epoch = 4;
    for pk in &attested {
        prev.attested_sets[0].insert(*pk);
    }

    let input = EpochDerivationInput {
        active_producers: all.clone(),
        bond_counts: HashMap::new(),
        blocks_per_epoch: 360,
        snap_attestation_skip_height: u64::MAX,
        height: 500,
        epoch: 5,
        registered_at: reg,
        ghost_exclusion_activation_height: u64::MAX,
        epoch_prune_activation_height: 0,
        inc_i_190_floor_bound_activation_height: u64::MAX,
    };
    let state = EpochState::derive_at_boundary(&prev, &input);

    assert_eq!(state.producer_list.len(), 12);
    let mut active_sorted = all.clone();
    active_sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    assert_ne!(
        state.producer_list.len(),
        active_sorted.len(),
        "Post-activation: producer_list(12) != active(57)"
    );

    for pk in &attested {
        assert!(state.producer_list.contains(pk));
    }
    for pk in all.iter().skip(12) {
        assert!(!state.producer_list.contains(pk));
    }
}

// ============================================================================
// Section 3: FILTER-08 Regression — tier cap + floor interaction (2 tests)
// ============================================================================

// OUTPUT CONTRACT: fn derive_at_boundary with >50 producers
// Outputs:
//   O1: producer_list -- Vec<PublicKey>, O2: active_list -- Vec<PublicKey>
// Paths: P1: >50 active, floor passes; P2: >50 active, active_list diverges
// INPUT PARTITIONS:
//   P1a: 55 active, 40 attested -- producer_list=40, active_list=40 (<=50)
//   P2a: 60 active, 55 attested -- producer_list=55, active_list=50 (capped)
// MATRIX: 2 outputs x 2 partitions = 4 cells

// FILTER-08: 55 active, 40 attested, floor passes, no tier cap hit
#[test]
fn test_filter08_tier_cap_floor_passes() {
    let all: Vec<PublicKey> = (1..=55).map(make_pubkey).collect();
    let attested: Vec<PublicKey> = (1..=40).map(make_pubkey).collect();
    let reg: HashMap<PublicKey, u64> = all
        .iter()
        .enumerate()
        .map(|(i, pk)| (*pk, i as u64 * 10))
        .collect();

    let mut prev = EpochState::genesis();
    prev.epoch = 4;
    for pk in &attested {
        prev.attested_sets[0].insert(*pk);
    }

    let input = EpochDerivationInput {
        active_producers: all,
        bond_counts: HashMap::new(),
        blocks_per_epoch: 360,
        snap_attestation_skip_height: u64::MAX,
        height: 1800,
        epoch: 5,
        registered_at: reg,
        ghost_exclusion_activation_height: u64::MAX,
        epoch_prune_activation_height: u64::MAX,
        inc_i_190_floor_bound_activation_height: u64::MAX,
    };
    let state = EpochState::derive_at_boundary(&prev, &input);

    // 40 >= 55*2/3=36 -> floor does NOT fire
    assert_eq!(state.producer_list.len(), 40);
    assert!(state
        .producer_list
        .windows(2)
        .all(|w| w[0].as_bytes() <= w[1].as_bytes()));
    let mut att_sorted = attested.clone();
    att_sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    assert_eq!(state.producer_list, att_sorted);
    assert_eq!(state.active_list.len(), 40); // 40 <= CAP(50)
}

// FILTER-08: 60 active, 55 attested, active_list capped at 50
#[test]
fn test_filter08_tier_cap_active_diverges() {
    let all: Vec<PublicKey> = (1..=60).map(make_pubkey).collect();
    let attested: Vec<PublicKey> = (1..=55).map(make_pubkey).collect();
    let reg: HashMap<PublicKey, u64> = all
        .iter()
        .enumerate()
        .map(|(i, pk)| (*pk, i as u64 * 10))
        .collect();

    let mut prev = EpochState::genesis();
    prev.epoch = 4;
    for pk in &attested {
        prev.attested_sets[0].insert(*pk);
    }
    // Populate prev attestation_accum and blocks_produced so the tier
    // promotion filter retains the 55 attested producers (rather than
    // zeroing them all and triggering the tier deadlock safety fallback).
    // MIN_ATTESTATION_MINUTES=30, min_produced=max(360/55*80/100,1)=4.
    for pk in &attested {
        let mins: HashSet<u32> = (0..30).collect();
        prev.attestation_accum[0].insert(*pk, mins);
        prev.blocks_produced.insert(*pk, 10);
    }

    let input = EpochDerivationInput {
        active_producers: all,
        bond_counts: HashMap::new(),
        blocks_per_epoch: 360,
        snap_attestation_skip_height: u64::MAX,
        height: 1800,
        epoch: 5,
        registered_at: reg,
        ghost_exclusion_activation_height: u64::MAX,
        epoch_prune_activation_height: u64::MAX,
        inc_i_190_floor_bound_activation_height: u64::MAX,
    };
    let state = EpochState::derive_at_boundary(&prev, &input);

    assert_eq!(state.producer_list.len(), 55);
    assert_eq!(state.active_list.len(), ACTIVE_PRODUCERS_CAP);
    assert_ne!(
        state.producer_list.len(),
        state.active_list.len(),
        "producer_list > CAP => active_list MUST differ"
    );
}

// ============================================================================
// Section 4: Edge cases (5 tests)
// ============================================================================

// OUTPUT CONTRACT: fn compute_live_producer_list edge cases
// Outputs: O1: return -- Vec<PublicKey>
// Paths: P1-P5: empty, all-attested, at-floor, single, activation boundary
// INPUT PARTITIONS:
//   P1a: 0 active -- empty
//   P2a: 20/20 attested, post-act -- all 20
//   P3a: 3/30 attested, post-act -- exactly at floor
//   P4a: 1/1 attested, post-act -- 1 < 3 floor, fallback = [1]
//   P5a: height=AH vs AH-1 -- behavior differs at boundary
// MATRIX: 1 output x 5 partitions = 5 cells

// Edge: empty active -> empty result
#[test]
fn test_edge_empty_active() {
    let au: HashSet<&PublicKey> = HashSet::new();
    let reg: HashMap<PublicKey, u64> = HashMap::new();
    let expected = derive_producer_list(vec![], &[], reg.clone(), 360, 5, 1800, u64::MAX, u64::MAX);
    assert!(expected.is_empty());
    let (result, _) = compute_live_producer_list(
        &[],
        &au,
        &reg,
        360,
        5,
        1800,
        u64::MAX,
        u64::MAX,
        u64::MAX,
        None,
    );
    assert!(result.is_empty());
}

// Edge: 100% attestation -> all retained
#[test]
fn test_edge_all_attested() {
    let all: Vec<PublicKey> = (1..=20).map(make_pubkey).collect();
    let reg = reg_at_zero(&all);
    let expected = derive_producer_list(all.clone(), &all, reg.clone(), 360, 5, 1800, u64::MAX, 0);
    assert_eq!(expected.len(), 20);
    assert_extraction_matches(&all, &all, &reg, 360, 5, 1800, u64::MAX, 0);
}

// Edge: exactly MIN_PRODUCERS_FLOOR attested -> no fallback
#[test]
fn test_edge_exactly_at_floor() {
    let all: Vec<PublicKey> = (1..=30).map(make_pubkey).collect();
    let attested: Vec<PublicKey> = (1..=3).map(make_pubkey).collect();
    let reg = reg_at_zero(&all);
    let expected = derive_producer_list(
        all.clone(),
        &attested,
        reg.clone(),
        360,
        5,
        1800,
        u64::MAX,
        0,
    );
    assert_eq!(expected.len(), MIN_PRODUCERS_FLOOR);
    assert_extraction_matches(&all, &attested, &reg, 360, 5, 1800, u64::MAX, 0);
}

// Edge: single producer, post-activation -> floor fires, fallback = [pk]
#[test]
fn test_edge_single_producer() {
    let pk = make_pubkey(1);
    let reg: HashMap<PublicKey, u64> = [(pk, 0)].into();
    let expected = derive_producer_list(vec![pk], &[pk], reg.clone(), 360, 5, 1800, u64::MAX, 0);
    assert!(expected.contains(&pk));
    let au: HashSet<&PublicKey> = [&pk].into();
    let (result, _) =
        compute_live_producer_list(&[pk], &au, &reg, 360, 5, 1800, u64::MAX, 0, u64::MAX, None);
    assert!(result.contains(&pk));
}

// Edge: activation height boundary (height==AH vs AH-1)
#[test]
fn test_edge_activation_boundary() {
    let all: Vec<PublicKey> = (1..=20).map(make_pubkey).collect();
    let attested: Vec<PublicKey> = (1..=10).map(make_pubkey).collect();
    let reg = reg_at_zero(&all);
    let ah = 1800_u64;

    // At AH: post-activation, 10 >= 3, prunes to 10
    let at = derive_producer_list(
        all.clone(),
        &attested,
        reg.clone(),
        360,
        5,
        ah,
        u64::MAX,
        ah,
    );
    assert_eq!(at.len(), 10);

    // Below AH: pre-activation, 10 < 20*2/3=13, floor fires, all 20
    let below = derive_producer_list(
        all.clone(),
        &attested,
        reg.clone(),
        360,
        5,
        ah - 1,
        u64::MAX,
        ah,
    );
    assert_eq!(below.len(), 20);

    assert_ne!(at.len(), below.len(), "Behavior must differ at AH boundary");
    assert_extraction_matches(&all, &attested, &reg, 360, 5, ah, u64::MAX, ah);
}
