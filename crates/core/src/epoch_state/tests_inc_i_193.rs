//! INC-I-193 M1 — the attestor-refill gate on the tier retain (`mod.rs:242-246`).
//!
//! These tests FAIL TO COMPILE until `EpochDerivationInput` gains
//! `inc_i_193_attestor_refill_activation_height` (change C4). That is the intended
//! TDD red for this milestone.
//!
//! OUTPUT CONTRACT: fn EpochState::derive_at_boundary(prev: &EpochState, input: &EpochDerivationInput) -> EpochState
//!   O1: (none) — `prev` and `input` are shared references; the function mutates neither.
//!   O2: (none) — associated function, no receiver.
//!   O3: return.active_list — the ONLY field this gate can change. Post-AH it admits
//!       candidates with `produced < min_produced`; pre-AH it does not.
//!   O3: return.producer_list — MUST be identical on both arms (the gate sits after the
//!       floor stage; a difference here means the test setup, not the gate, moved).
//!   O3: return.bond_snapshot / attested_sets / attestation_accum / blocks_produced —
//!       MUST be unaffected by the gate (rotation + clone, `mod.rs:271-292`).
//!   O4: (none) — pure function, no store writes.
//!   O5: (none) — no globals. O6: (none) — no channels.
//! PATHS:
//!   B0: `len(producer_list) <= ACTIVE_PRODUCERS_CAP` — no tier stage (not exercised here;
//!       covered by tests_m2.rs).
//!   B1: `len > CAP`, `epoch <= 1` — no retain (not exercised here).
//!   B2: `len > CAP`, `epoch > 1`, `height < AH` — legacy retain (mins AND produced).
//!   B3: `len > CAP`, `epoch > 1`, `height >= AH` — minutes-only retain.
//!   B4: post-retain fallback `len(result) < len(producer_list)/3` — every scenario below
//!       is sized to keep this branch UNTAKEN, so the assertions read the retain itself.
//! INPUT PARTITIONS (the classes where the two arms' math differs):
//!   IP-A: candidate has `mins >= 30` AND `produced >= min_produced` — kept by BOTH arms.
//!   IP-B: candidate has `mins >= 30` AND `produced == 0` — kept ONLY by B3. The gate.
//!   IP-C: candidate has `mins < 30` (any production) — dropped by BOTH arms.
//!   IP-D: candidate has no accumulator entry at all — dropped by BOTH arms.
//!   IP-E: EVERY candidate is IP-A — B2 and B3 must be bit-identical (gate-edge identity).
//!   IP-F: the IP-B population is re-seeded across three consecutive boundaries — B2 is
//!         shrink-only (stuck), B3 refills to the cap.
//! MATRIX: 3 asserted outputs (active_list, producer_list, accumulator carry-over)
//!         x 2 paths (B2, B3) x 6 partitions = the cells asserted below.

use super::*;
use crate::consensus::{ACTIVE_PRODUCERS_CAP, MIN_ATTESTATION_MINUTES};
use std::collections::{HashMap, HashSet};

/// Registry size that puts the tier stage past its cap: 80 > CAP(50).
/// `min_produced` = (360/80)*80/100 = 3; the 1/3 fallback floor = 26.
const N: usize = 80;
const BPE: u64 = 360;
const HEIGHT: u64 = 1800;
const EPOCH: u64 = 5;

fn make_pubkey(seed: u8) -> PublicKey {
    let mut bytes = [0u8; 32];
    bytes[0] = seed;
    PublicKey::from_bytes(bytes)
}

/// `n` producers, seniority `i*10` — index order IS seniority order.
fn registry(n: usize) -> (Vec<PublicKey>, HashMap<PublicKey, u64>) {
    let all: Vec<PublicKey> = (1..=n as u8).map(make_pubkey).collect();
    let reg: HashMap<PublicKey, u64> = all
        .iter()
        .enumerate()
        .map(|(i, pk)| (*pk, i as u64 * 10))
        .collect();
    (all, reg)
}

/// Per-producer state of the just-completed epoch: (pubkey, attested minutes, blocks).
type Accum = Vec<(PublicKey, usize, u32)>;

/// Everyone in `all` with `mins` minutes and `blocks` blocks.
fn all_with(all: &[PublicKey], mins: usize, blocks: u32) -> Accum {
    all.iter().map(|pk| (*pk, mins, blocks)).collect()
}

/// Write one epoch's accumulators into slot [0]. Slot [0] is what the tier retain reads.
fn seed_accum(st: &mut EpochState, accum: &Accum) {
    st.attested_sets[0].clear();
    st.attestation_accum[0].clear();
    st.blocks_produced.clear();
    for (pk, mins, blocks) in accum {
        st.attested_sets[0].insert(*pk);
        st.attestation_accum[0].insert(*pk, (0..*mins as u32).collect::<HashSet<u32>>());
        if *blocks > 0 {
            st.blocks_produced.insert(*pk, *blocks);
        }
    }
}

/// The just-completed epoch state. Lookback slots [1]/[2] hold the WHOLE registry, so the
/// attestation filter keeps `producer_list` at `all.len()` and the tier stage is reached
/// even when only a minority attested in slot [0] — the mainnet shape this bug lives in.
fn prev_state(all: &[PublicKey], accum: &Accum) -> EpochState {
    let mut prev = EpochState::genesis();
    prev.epoch = EPOCH - 1;
    prev.producer_list = all.to_vec();
    prev.active_list = all.to_vec();
    for pk in all {
        prev.attested_sets[1].insert(*pk);
        prev.attested_sets[2].insert(*pk);
    }
    seed_accum(&mut prev, accum);
    prev
}

/// Every non-INC-I-193 gate is held at `u64::MAX` so the floor stage is a no-op and the
/// only variable in the assertions is `ah`.
fn input(
    all: &[PublicKey],
    reg: &HashMap<PublicKey, u64>,
    height: u64,
    epoch: u64,
    ah: u64,
) -> EpochDerivationInput {
    EpochDerivationInput {
        active_producers: all.to_vec(),
        bond_counts: HashMap::new(),
        blocks_per_epoch: BPE,
        snap_attestation_skip_height: u64::MAX,
        height,
        epoch,
        registered_at: reg.clone(),
        ghost_exclusion_activation_height: u64::MAX,
        epoch_prune_activation_height: u64::MAX,
        inc_i_190_floor_bound_activation_height: u64::MAX,
        inc_i_193_attestor_refill_activation_height: ah,
    }
}

/// Assert the tier stage was actually reached and the 1/3 fallback did NOT fire — without
/// this, an `active_list` assertion can pass for the wrong reason (B4 returns the full list).
fn assert_gated_branch_reached(st: &EpochState, expected_list_len: usize) {
    assert_eq!(
        st.producer_list.len(),
        expected_list_len,
        "precondition: the floor stage must leave the whole registry in producer_list"
    );
    assert!(
        st.producer_list.len() > ACTIVE_PRODUCERS_CAP,
        "precondition: the tier stage only runs above the cap"
    );
    assert_ne!(
        st.active_list, st.producer_list,
        "precondition: active_list == producer_list means the 1/3 fallback fired and the \
         retain under test was overwritten — the scenario is mis-sized"
    );
}

// ---------------------------------------------------------------------------
// TEST-193-01 — B2 x {IP-B, IP-A, IP-D}: the pre-AH arm stays byte-identical.
// ---------------------------------------------------------------------------

// TEST-193-01 — Decision: a failure means the gate was applied to heights BELOW the
// activation height, retroactively rewriting the schedule of sealed epochs (INC-I-054).
#[test]
fn test_193_01_pre_ah_keeps_the_produced_clause() {
    let (all, reg) = registry(N);
    let x = all[0]; // IP-B: most senior, attests, produces nothing
    let mut accum: Accum = vec![(x, MIN_ATTESTATION_MINUTES, 0)];
    for pk in &all[1..=31] {
        accum.push((*pk, MIN_ATTESTATION_MINUTES, 11)); // IP-A
    }
    // all[32..] have no entry at all — IP-D.

    let prev = prev_state(&all, &accum);
    let state = EpochState::derive_at_boundary(&prev, &input(&all, &reg, HEIGHT, EPOCH, u64::MAX));

    assert_gated_branch_reached(&state, N);
    assert!(
        !state.active_list.contains(&x),
        "O3/B2/IP-B: below the AH a 0-block attester MUST stay demoted"
    );
    assert_eq!(
        state.active_list,
        all[1..=31].to_vec(),
        "O3/B2/IP-A: the legacy retain keeps exactly the 31 producers that met BOTH clauses, \
         ordered by registered_at ascending"
    );
    assert_eq!(state.active_list.len(), 31);
}

// ---------------------------------------------------------------------------
// TEST-193-02 — B3 x IP-B: the same registry, one height later, refills.
// ---------------------------------------------------------------------------

// TEST-193-02 — Decision: a failure means the refill never happens, so a producer benched
// once can never re-enter and the list stays shrink-only — the defect itself.
#[test]
fn test_193_02_post_ah_admits_the_zero_block_attester() {
    let (all, reg) = registry(N);
    let x = all[0];
    let mut accum: Accum = vec![(x, MIN_ATTESTATION_MINUTES, 0)];
    for pk in &all[1..=31] {
        accum.push((*pk, MIN_ATTESTATION_MINUTES, 11));
    }

    let prev = prev_state(&all, &accum);
    let legacy = EpochState::derive_at_boundary(&prev, &input(&all, &reg, HEIGHT, EPOCH, u64::MAX));
    let gated = EpochState::derive_at_boundary(&prev, &input(&all, &reg, HEIGHT, EPOCH, HEIGHT));

    assert_gated_branch_reached(&gated, N);
    assert_eq!(
        gated.active_list[0], x,
        "O3/B3/IP-B: at height == AH the 0-block attester is admitted, and being the most \
         senior it takes index 0"
    );
    assert_eq!(
        gated.active_list,
        all[0..=31].to_vec(),
        "O3/B3: the post-AH retain is exactly the pre-AH retain PLUS the 30-minute attesters"
    );
    assert_eq!(gated.active_list.len(), 32);
    assert_ne!(
        gated.active_list, legacy.active_list,
        "non-vacuity: the two arms must disagree on this input, or the test proves nothing"
    );
    assert_eq!(
        gated.producer_list, legacy.producer_list,
        "O3: the gate sits AFTER the floor stage — producer_list must not move"
    );
}

// ---------------------------------------------------------------------------
// TEST-193-03 — B3 x {IP-B, IP-F}: refill to the cap, and the 3-boundary sequence.
// ---------------------------------------------------------------------------

// TEST-193-03 — Decision: a failure means the refill stops short of ACTIVE_PRODUCERS_CAP,
// leaving stake-weighted producers benched even after the gate opens.
#[test]
fn test_193_03_refill_reaches_the_cap_by_seniority() {
    let (all, reg) = registry(N);
    // All 80 attest; only the 31 currently listed could produce.
    let mut accum: Accum = all_with(&all, MIN_ATTESTATION_MINUTES, 0);
    for entry in accum.iter_mut().take(31) {
        entry.2 = 11;
    }

    let prev = prev_state(&all, &accum);
    let legacy = EpochState::derive_at_boundary(&prev, &input(&all, &reg, HEIGHT, EPOCH, u64::MAX));
    let gated = EpochState::derive_at_boundary(&prev, &input(&all, &reg, HEIGHT, EPOCH, HEIGHT));

    assert_gated_branch_reached(&gated, N);
    assert_eq!(
        legacy.active_list.len(),
        31,
        "non-vacuity: below the AH this registry is stuck at the 31 producers"
    );
    assert_eq!(
        gated.active_list.len(),
        ACTIVE_PRODUCERS_CAP,
        "O3/B3: 80 attesters, cap 50 — the refill fills the list"
    );
    assert_eq!(
        gated.active_list,
        all[0..ACTIVE_PRODUCERS_CAP].to_vec(),
        "O3/B3: the admitted 50 are the 50 smallest registered_at, unchanged sort"
    );
}

/// Drive three consecutive boundaries at `ah`, feeding each output back as the next `prev`.
/// Returns the `active_list` length after each boundary plus the final list.
fn three_boundaries(
    all: &[PublicKey],
    reg: &HashMap<PublicKey, u64>,
    epochs: [&Accum; 3],
    ah: u64,
) -> ([usize; 3], Vec<PublicKey>) {
    let mut prev = prev_state(all, epochs[0]);
    let mut lens = [0usize; 3];
    let mut last = Vec::new();
    for (i, accum) in epochs.iter().enumerate() {
        if i > 0 {
            seed_accum(&mut prev, accum);
        }
        let epoch = EPOCH + i as u64;
        let height = epoch * BPE;
        let state = EpochState::derive_at_boundary(&prev, &input(all, reg, height, epoch, ah));
        lens[i] = state.active_list.len();
        last = state.active_list.clone();
        prev = state;
    }
    (lens, last)
}

// TEST-193-03 — Decision: a failure means the shrink-only ratchet survives the fix; this
// printed sequence is the milestone's outcome-metric probe, so its numbers are the evidence.
#[test]
fn test_193_03_three_boundary_sequence() {
    let (all, reg) = registry(N);

    // Epoch 1: healthy — every producer attests and produces. Both arms give 50.
    let e1 = all_with(&all, MIN_ATTESTATION_MINUTES, 11);
    // Epoch 2: only the 31 most senior of the listed 50 are alive. Both arms give 31.
    let e2: Accum = all[0..31]
        .iter()
        .map(|pk| (*pk, MIN_ATTESTATION_MINUTES, 11))
        .collect();
    // Epoch 3: the 19 benched producers come back ATTESTING but cannot produce (IP-B),
    // because a producer outside active_list is never scheduled.
    let mut e3 = e2.clone();
    for pk in &all[31..ACTIVE_PRODUCERS_CAP] {
        e3.push((*pk, MIN_ATTESTATION_MINUTES, 0));
    }

    let (pre, _) = three_boundaries(&all, &reg, [&e1, &e2, &e3], u64::MAX);
    let (post, post_last) = three_boundaries(&all, &reg, [&e1, &e2, &e3], 0);

    let label = |seq: &[usize; 3]| if seq[2] > seq[1] { "refilled" } else { "stuck" };
    println!(
        "pre-AH active_list: {} -> {} -> {} ({})",
        pre[0],
        pre[1],
        pre[2],
        label(&pre)
    );
    println!(
        "post-AH active_list: {} -> {} -> {} ({})",
        post[0],
        post[1],
        post[2],
        label(&post)
    );

    assert_eq!(
        pre,
        [ACTIVE_PRODUCERS_CAP, 31, 31],
        "O3/B2/IP-F: below the AH the list ratchets down and never recovers"
    );
    assert_eq!(
        post,
        [ACTIVE_PRODUCERS_CAP, 31, ACTIVE_PRODUCERS_CAP],
        "O3/B3/IP-F: at/above the AH the third boundary refills to the cap"
    );
    assert_eq!(
        post_last,
        all[0..ACTIVE_PRODUCERS_CAP].to_vec(),
        "O3/B3/IP-F: the refilled list is the 50 smallest registered_at"
    );
}

// ---------------------------------------------------------------------------
// TEST-193-04 — B2 vs B3 x IP-E: bit-identity when both clauses already hold.
// ---------------------------------------------------------------------------

// TEST-193-04 — Decision: a failure means the gate changed the schedule for inputs where
// the deleted clause was never binding — an unintended consensus change at the gate edge.
#[test]
fn test_193_04_gate_edge_is_bit_identical_when_both_clauses_hold() {
    // 60 producers: expected_per_producer = 360/60 = 6, min_produced = 4. Everyone
    // produces 10, so the deleted clause is satisfied by every candidate (IP-E).
    let (all, reg) = registry(60);
    let accum = all_with(&all, MIN_ATTESTATION_MINUTES, 10);
    let prev = prev_state(&all, &accum);

    let below =
        EpochState::derive_at_boundary(&prev, &input(&all, &reg, HEIGHT, EPOCH, HEIGHT + 1));
    let at = EpochState::derive_at_boundary(&prev, &input(&all, &reg, HEIGHT, EPOCH, HEIGHT));

    assert_gated_branch_reached(&at, 60);
    assert_eq!(
        at.active_list, below.active_list,
        "O3/IP-E: when every candidate satisfies both clauses, B2 and B3 MUST agree"
    );
    assert_eq!(
        at.producer_list, below.producer_list,
        "O3/IP-E: producer_list"
    );
    assert_eq!(
        at.active_list.len(),
        ACTIVE_PRODUCERS_CAP,
        "non-vacuity: 60 candidates, cap 50 — the compared lists are truncated, not trivial"
    );
    assert_eq!(
        at.bond_snapshot.len(),
        below.bond_snapshot.len(),
        "O3: the gate must not touch the bond snapshot"
    );
    assert_eq!(
        at.attestation_accum[1].len(),
        below.attestation_accum[1].len(),
        "O3: the gate must not touch accumulator rotation"
    );
    assert!(
        at.blocks_produced.is_empty(),
        "O3: the new epoch always starts with an empty production counter"
    );
}

// ---------------------------------------------------------------------------
// TEST-193-06 — B3 x IP-C: the attestation-minutes demotion still bites.
// ---------------------------------------------------------------------------

// TEST-193-06 — Decision: a failure means deleting the produced-clause also deleted the
// liveness filter, letting silent nodes hold scheduler slots forever.
#[test]
fn test_193_06_post_ah_still_demotes_below_min_attestation_minutes() {
    let (all, reg) = registry(N);
    let y = all[0]; // IP-C: listed, productive, but one minute short
    let mut accum: Accum = vec![(y, MIN_ATTESTATION_MINUTES - 1, 11)];
    for pk in &all[1..=40] {
        accum.push((*pk, MIN_ATTESTATION_MINUTES, 0)); // IP-B
    }

    let prev = prev_state(&all, &accum);
    let state = EpochState::derive_at_boundary(&prev, &input(&all, &reg, HEIGHT, EPOCH, HEIGHT));

    assert_gated_branch_reached(&state, N);
    assert!(
        !state.active_list.contains(&y),
        "O3/B3/IP-C: 29 minutes is below MIN_ATTESTATION_MINUTES — still demoted"
    );
    assert_eq!(
        state.active_list,
        all[1..=40].to_vec(),
        "O3/B3/IP-B: exactly the 40 producers that reached 30 minutes, 0 blocks and all"
    );
    assert_eq!(state.active_list.len(), 40);
}
