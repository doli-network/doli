//! Bond-weighted median tests — Phase 2.1 Oracle M6.
//!
//! Spec: `specs/oracle-structural-anchored-economics.md` §1.3 + §5
//! (manipulation surface table).
//!
//! These tests pin the economic invariants the design phase
//! converged on:
//!   - 37.3% adversary: 0% deviation possible
//!   - 50.1% adversary: full deviation (defended by sunset, M8)
//!   - Tie boundary: deterministic lower-median pick
//!   - Empty / all-zero-weight inputs return None
//!   - LATEST per attester wins when duplicates appear
//!
//! D.3 sunset gradient tests (added for DeFi L1 Foundations M3):
//!   - Warning zone entry/exit
//!   - Halt recoverable entry/exit
//!   - Halt permanent after ORACLE_RECOVERY_EPOCHS
//!   - State persistence via OracleSunsetState serde round-trip

use super::{
    bond_weighted_median, compute_structural_share_bps, dedupe_latest_per_attester,
    AttestationContribution, OracleHealthState, OracleSunsetState, ORACLE_RECOVERY_EPOCHS,
    SUNSET_THRESHOLD_BPS, SUNSET_WARNING_BPS,
};
use crypto::Hash;
use std::collections::HashMap;

fn h(seed: u8) -> Hash {
    Hash::from_bytes([seed; 32])
}

/// Build a bond_snapshot from a list of (signer_hash_seed, weight).
fn bonds(entries: &[(u8, u64)]) -> HashMap<Hash, u64> {
    entries.iter().map(|(s, w)| (h(*s), *w)).collect()
}

/// Build an attestation contribution list from
/// (signer_hash_seed, price_cents).
fn attests(entries: &[(u8, u64)]) -> Vec<AttestationContribution> {
    entries
        .iter()
        .map(|(s, p)| AttestationContribution {
            signer_hash: h(*s),
            price_cents: *p,
        })
        .collect()
}

// OUTPUT CONTRACT: fn bond_weighted_median — empty / invalid inputs
//   O1: return — None when attestations is empty
//   O2: return — None when no attester has positive bond weight
// PATHS:
//   P1: empty attestations -> None
//   P2: attesters present but all weights 0 (or missing from snapshot)
// INPUT PARTITIONS:
//   part-A (P1): zero-length slice
//   part-B (P2): attestations referencing signers not in snapshot
//   part-C (P2): attestations referencing signers with weight 0
// MATRIX:
//   P1×part-A: O1✓     P2×part-B: O2✓     P2×part-C: O2✓
#[test]
fn test_median_empty_returns_none() {
    let bs = bonds(&[(1, 100)]);
    let result = bond_weighted_median(&[], &bs);
    assert_eq!(result, None); // O1
}

#[test]
fn test_median_returns_none_when_signers_missing_from_snapshot() {
    // Attester refers to seed=7 but snapshot has seed=1.
    let a = attests(&[(7, 100)]);
    let bs = bonds(&[(1, 100)]);
    assert_eq!(bond_weighted_median(&a, &bs), None); // O2 (part-B)
}

#[test]
fn test_median_returns_none_when_all_weights_zero() {
    let a = attests(&[(1, 100), (2, 200)]);
    let bs = bonds(&[(1, 0), (2, 0)]);
    assert_eq!(bond_weighted_median(&a, &bs), None); // O2 (part-C)
}

// OUTPUT CONTRACT: fn bond_weighted_median — single attester
//   O1: return — Some((that_attester_price, 1))
// PATHS / PARTITIONS: trivial — 1 attester with positive weight.
#[test]
fn test_median_single_attester() {
    let a = attests(&[(1, 555)]);
    let bs = bonds(&[(1, 100)]);
    assert_eq!(bond_weighted_median(&a, &bs), Some((555, 1))); // O1
}

// OUTPUT CONTRACT: fn bond_weighted_median — equal weights, classic median
//   O1: return — Some((middle_price, n)) when N is odd and weights equal
//   O2: return — Some((lower_median_price, n)) when N is even and weights equal
// PATHS:
//   P1: 3 equal-weight attesters at prices [100, 200, 300] -> 200
//   P2: 4 equal-weight attesters at [100, 200, 300, 400] -> 200 (lower-median)
// INPUT PARTITIONS:
//   part-A (P1): 3 attesters, equal weight = 50
//   part-B (P2): 4 attesters, equal weight = 50
// MATRIX:
//   P1×part-A: O1✓     P2×part-B: O2✓
#[test]
fn test_median_odd_count_equal_weights() {
    let a = attests(&[(1, 100), (2, 200), (3, 300)]);
    let bs = bonds(&[(1, 50), (2, 50), (3, 50)]);
    // total=150, ceil_half=75. Sorted: 100(50),200(100),300(150).
    // 100 -> cum=50 (<75), 200 -> cum=100 (>=75) -> median=200.
    assert_eq!(bond_weighted_median(&a, &bs), Some((200, 3))); // O1
}

#[test]
fn test_median_even_count_equal_weights_picks_lower_median() {
    let a = attests(&[(1, 100), (2, 200), (3, 300), (4, 400)]);
    let bs = bonds(&[(1, 50), (2, 50), (3, 50), (4, 50)]);
    // total=200, ceil_half=100. Sorted: 100(50),200(100),300(150),400(200).
    // 100 -> cum=50 (<100), 200 -> cum=100 (>=100) -> median=200.
    assert_eq!(bond_weighted_median(&a, &bs), Some((200, 4))); // O2
}

// OUTPUT CONTRACT: fn bond_weighted_median — 37.3% adversary scenario
//   O1: deviation from honest median is 0 (spec §5 manipulation surface)
// PATHS / PARTITIONS:
//   3 honest attesters at $1.00 = 62.7% combined weight; 1 adversary
//   attester at $99.99 = 37.3% weight. Median MUST equal honest price.
//
// Numbers chosen to mirror real fleet: 176,650 honest bonds vs
// 105,067 adversary bonds (= 37.3% of 281,717 total).
#[test]
fn test_median_resists_37pct_adversary() {
    let a = attests(&[
        (1, 100),   // honest @ $1.00
        (2, 100),   // honest @ $1.00
        (3, 100),   // honest @ $1.00
        (4, 9_999), // adversary @ $99.99
    ]);
    let bs = bonds(&[
        (1, 58_883), // 1/3 of 176,650 honest weight
        (2, 58_883),
        (3, 58_884),  // (rounding remainder)
        (4, 105_067), // adversary 37.3%
    ]);
    // total = 281,717; ceil_half = 140,859.
    // Sorted asc: 100(58_883),100(58_883),100(58_884),9_999(105_067).
    // cum: 58_883, 117_766, 176_650 (>=140_859) -> median=100.
    assert_eq!(bond_weighted_median(&a, &bs), Some((100, 4))); // O1
}

// OUTPUT CONTRACT: fn bond_weighted_median — 50.1% adversary scenario
//   O1: adversary fully controls the median (consensus surface is
//        defended by SUNSET trigger at M8, NOT by this function)
// PATHS / PARTITIONS:
//   Adversary holds 50.1% bond weight. The sort places adversary's
//   price at the median crossing -> output is adversary's value.
#[test]
fn test_median_fully_controlled_at_50_1pct_adversary() {
    // 49.9% honest split across 3 voices @ 100, 100, 100.
    // 50.1% adversary at 9_999.
    let a = attests(&[(1, 100), (2, 100), (3, 100), (4, 9_999)]);
    let bs = bonds(&[
        (1, 33_267), // 1/3 of 49.9% of 200_000
        (2, 33_267),
        (3, 33_266),
        (4, 100_200), // 50.1% of 200_000
    ]);
    // total=200_000, ceil_half=100_000.
    // Sorted asc: 100(33_267),100(33_267),100(33_266),9_999(100_200).
    // cum: 33_267, 66_534, 99_800, 200_000.
    // At cum=99_800 (<100_000), still on honest @ 100.
    // Next: 9_999 with cum=200_000 (>=100_000) -> median=9_999.
    // Adversary won.
    assert_eq!(bond_weighted_median(&a, &bs), Some((9_999, 4))); // O1
}

// OUTPUT CONTRACT: fn bond_weighted_median — tie-break determinism
//   O1: two runs over the same inputs produce IDENTICAL output
//   O2: equal-cumulative-weight tie picks the LOWER price (the one
//        at which cumulative weight first crosses 50%)
// PATHS / PARTITIONS:
//   2 attesters at prices 100 and 200, equal weight. Median = 100
//   (the LOWER price; the spec says "first crossing", which equals
//   lower-median on a 2-attester tie).
#[test]
fn test_median_tie_picks_lower_price_deterministically() {
    let a = attests(&[(1, 100), (2, 200)]);
    let bs = bonds(&[(1, 50), (2, 50)]);
    // total=100, ceil_half=50.
    // Sorted: 100(50), 200(100).
    // cum=50 at 100 -> median=100.
    let r1 = bond_weighted_median(&a, &bs);
    let r2 = bond_weighted_median(&a, &bs);
    assert_eq!(r1, r2); // O1
    assert_eq!(r1, Some((100, 2))); // O2
}

// OUTPUT CONTRACT: fn bond_weighted_median — unequal weights bias the median
//   O1: heavily-weighted attester pulls the median to their price
// PATHS / PARTITIONS:
//   3 attesters: prices 100, 200, 300. Weights 10, 1000, 10.
//   Total = 1020, ceil_half = 510. The 200 attester has weight 1000
//   alone — they cross 50% on their own.
#[test]
fn test_median_weighted_attester_dominates() {
    let a = attests(&[(1, 100), (2, 200), (3, 300)]);
    let bs = bonds(&[(1, 10), (2, 1_000), (3, 10)]);
    // total=1020, ceil_half=510.
    // sorted: 100(10), 200(1010 cum), 300.
    // 100 cum=10, 200 cum=1010 (>=510) -> median=200.
    assert_eq!(bond_weighted_median(&a, &bs), Some((200, 3)));
}

// OUTPUT CONTRACT: fn dedupe_latest_per_attester
//   O1: same signer appearing twice -> only the LATEST is kept
//   O2: distinct signers preserved
// PATHS / PARTITIONS:
//   P1: signer 1 appears twice (first @100, then @300); signer 2 once @200
//   P1×part-A: O1✓ O2✓
#[test]
fn test_dedupe_latest_per_attester_keeps_last_occurrence() {
    let input = vec![
        AttestationContribution {
            signer_hash: h(1),
            price_cents: 100,
        },
        AttestationContribution {
            signer_hash: h(2),
            price_cents: 200,
        },
        AttestationContribution {
            signer_hash: h(1), // duplicate signer
            price_cents: 300,
        },
    ];
    let mut deduped = dedupe_latest_per_attester(&input);
    deduped.sort_by_key(|c| c.signer_hash.as_bytes()[0]);

    assert_eq!(deduped.len(), 2);
    let sig1 = deduped.iter().find(|c| c.signer_hash == h(1)).unwrap();
    assert_eq!(sig1.price_cents, 300); // O1 — latest, not first
    let sig2 = deduped.iter().find(|c| c.signer_hash == h(2)).unwrap();
    assert_eq!(sig2.price_cents, 200); // O2
}

// OUTPUT CONTRACT: contributor_count saturation
//   O1: count saturates at u16::MAX when there are more than u16::MAX
//        positive-weight attesters
// PATHS / PARTITIONS:
//   Synthetic: 70_000 attesters all weight=1 at price=100. The actual
//   limit is u16::MAX = 65_535. count should saturate.
#[test]
fn test_contributor_count_saturates_at_u16_max() {
    let n: usize = 70_000;
    let mut a: Vec<AttestationContribution> = Vec::with_capacity(n);
    let mut bs: HashMap<Hash, u64> = HashMap::with_capacity(n);
    for i in 0..n {
        let mut seed = [0u8; 32];
        seed[0..4].copy_from_slice(&(i as u32).to_le_bytes());
        let hh = Hash::from_bytes(seed);
        a.push(AttestationContribution {
            signer_hash: hh,
            price_cents: 100,
        });
        bs.insert(hh, 1);
    }
    let result = bond_weighted_median(&a, &bs).unwrap();
    assert_eq!(result.0, 100);
    assert_eq!(result.1, u16::MAX); // O1
}

// ===========================================================================
// M8 — Sunset trigger (compute_structural_share_bps)
// ===========================================================================

/// Build a `(bond_snapshot, registered_at)` pair from a list of
/// `(seed, weight, registered_at_height)`. Returns the structural-
/// hash list (the first `structural_n` entries) for sunset metric
/// computation.
fn build_m8_inputs(
    entries: &[(u8, u64, u64)],
    structural_n: usize,
) -> (HashMap<Hash, u64>, HashMap<Hash, u64>, Vec<Hash>) {
    let mut snapshot = HashMap::new();
    let mut regd = HashMap::new();
    let mut structural = Vec::new();
    for (i, (seed, w, r)) in entries.iter().enumerate() {
        let key = h(*seed);
        snapshot.insert(key, *w);
        regd.insert(key, *r);
        if i < structural_n {
            structural.push(key);
        }
    }
    (snapshot, regd, structural)
}

// OUTPUT CONTRACT: fn compute_structural_share_bps — happy path
//   O1: returns Some(bps) where bps = structural_bonds * 10_000 /
//       total_eligible
//   O2: returned bps matches expected value within rounding
// PATHS:
//   P1: 3 structural producers + 1 non-structural, all registered
//       long ago (eligible) -> bps = 75% (7500)
// INPUT PARTITIONS:
//   part-A (P1): structural weights = [10, 20, 30] (sum 60),
//                non-structural = [20]. total = 80. bps = 7500.
#[test]
fn test_sunset_share_basic_partition() {
    let (snap, regd, struct_keys) = build_m8_inputs(
        &[
            (1, 10, 0), // structural N1
            (2, 20, 0), // structural N2
            (3, 30, 0), // structural N3
            (4, 20, 0), // non-structural (excluded from numerator,
                        //                  included in denominator)
        ],
        3,
    );
    // current_epoch_start = 720, blocks_per_epoch = 360 -> threshold
    // for eligibility = registered_at <= 360. All entries registered
    // at 0 — eligible.
    let bps = compute_structural_share_bps(&snap, &regd, 720, 360, &struct_keys);
    // structural = 60, total = 80, bps = 60*10000/80 = 7500.
    assert_eq!(bps, Some(7500)); // O1+O2
}

// OUTPUT CONTRACT: at-threshold boundary
//   O1: bps == SUNSET_THRESHOLD_BPS (5500) means sunset is NOT
//       triggered (spec §1.8 says "structural_share < 0.55",
//       strict inequality, so 55.00% exactly is still OK)
#[test]
fn test_sunset_share_exact_threshold_not_triggered() {
    // 55% structural exactly: 11 structural at 10 each + 1
    // non-structural at 90. total = 200, structural = 110.
    // bps = 110 * 10_000 / 200 = 5500.
    let mut entries: Vec<(u8, u64, u64)> = (1..=11).map(|s| (s, 10, 0)).collect();
    entries.push((20, 90, 0));
    let (snap, regd, struct_keys) = build_m8_inputs(&entries, 11);
    let bps = compute_structural_share_bps(&snap, &regd, 720, 360, &struct_keys).unwrap();
    assert_eq!(bps, 5500);
    assert!(
        bps >= SUNSET_THRESHOLD_BPS,
        "5500 bps == threshold, must NOT trigger sunset (strict < gate)"
    );
}

// OUTPUT CONTRACT: just-below threshold triggers sunset
//   O1: bps < SUNSET_THRESHOLD_BPS when structural share is 54.99%
#[test]
fn test_sunset_share_just_below_threshold_triggers() {
    // 11 structural at 10 each = 110, non-structural at 91. total =
    // 201, bps = 110*10000/201 = 5472. Below 5500 -> sunset.
    let mut entries: Vec<(u8, u64, u64)> = (1..=11).map(|s| (s, 10, 0)).collect();
    entries.push((20, 91, 0));
    let (snap, regd, struct_keys) = build_m8_inputs(&entries, 11);
    let bps = compute_structural_share_bps(&snap, &regd, 720, 360, &struct_keys).unwrap();
    assert!(
        bps < SUNSET_THRESHOLD_BPS,
        "5472 bps < 5500 threshold, MUST trigger sunset; got bps={bps}"
    );
}

// OUTPUT CONTRACT: anti-dilution excludes young bonds
//   O1: bonds whose registered_at > (current - blocks_per_epoch)
//       are excluded from total_bonds_eligible
//   O2: a sybil attacker who flash-registers 100k bonds AT the
//       current epoch start cannot dilute structural_share
// PATHS:
//   Structural set is 62.7% of OLD-eligible bonds. Attacker
//   registers 200k fresh bonds at the current epoch start. With
//   anti-dilution, structural_share stays at 62.7%.
#[test]
fn test_sunset_anti_dilution_excludes_fresh_bonds() {
    let (snap, regd, struct_keys) = build_m8_inputs(
        &[
            (1, 176_650, 0), // structural — registered long ago
            (2, 105_067, 0), // non-structural, also long ago
            // Sybil dilution: 200k bonds registered AT current
            // epoch start = ineligible (bond_age = 0 epochs).
            (3, 200_000, 720),
        ],
        1, // only entry 0 is structural
    );
    // current_epoch_start = 720, blocks_per_epoch = 360. Eligibility
    // threshold = registered_at <= 360. Entries 0 (regd=0) and 1
    // (regd=0) are eligible. Entry 2 (regd=720) is NOT eligible.
    // structural = 176_650; total_eligible = 281_717.
    // bps = 176_650 * 10_000 / 281_717 = 6,270 (matches the real
    // fleet's structural share of 62.7%).
    let bps = compute_structural_share_bps(&snap, &regd, 720, 360, &struct_keys).unwrap();
    assert_eq!(
        bps, 6_270,
        "anti-dilution must keep structural at 62.7% bps despite the 200k sybil"
    );
    assert!(
        bps >= SUNSET_THRESHOLD_BPS,
        "sybil dilution must NOT trigger sunset; got bps={bps}"
    );
}

// OUTPUT CONTRACT: 1-epoch lag through caller (orchestrator)
//   The lag is enforced at the call site (orchestrator reads
//   self.epoch_state.bond_snapshot BEFORE rotation, which IS the
//   closing epoch's snapshot). This test pins that the FUNCTION
//   accepts an arbitrary snapshot — semantic responsibility for
//   passing the right snapshot lives with the orchestrator.
#[test]
fn test_sunset_function_is_pure_with_respect_to_snapshot() {
    let (snap_a, regd, struct_keys) = build_m8_inputs(&[(1, 100, 0), (2, 100, 0)], 1);
    let mut snap_b = snap_a.clone();
    snap_b.insert(h(2), 200); // mutate non-structural weight
    let bps_a = compute_structural_share_bps(&snap_a, &regd, 720, 360, &struct_keys);
    let bps_b = compute_structural_share_bps(&snap_b, &regd, 720, 360, &struct_keys);
    assert_ne!(
        bps_a, bps_b,
        "different snapshots must produce different bps — the function is pure but \
         the orchestrator's choice of WHICH snapshot to pass matters"
    );
}

// OUTPUT CONTRACT: empty / all-ineligible -> None
//   O1: returns None when total_bonds_eligible == 0
#[test]
fn test_sunset_returns_none_when_no_eligible_bonds() {
    // Only one entry, registered AT current epoch start =
    // ineligible.
    let (snap, regd, struct_keys) = build_m8_inputs(&[(1, 100, 720)], 1);
    let bps = compute_structural_share_bps(&snap, &regd, 720, 360, &struct_keys);
    assert_eq!(bps, None); // O1
}

// OUTPUT CONTRACT: missing registered_at entry is treated as
//                  ineligible (conservative)
#[test]
fn test_sunset_missing_registered_at_is_ineligible() {
    let mut snap = HashMap::new();
    let mut regd = HashMap::new();
    let struct_key = h(1);
    snap.insert(struct_key, 100);
    snap.insert(h(2), 100);
    // Only registered_at[struct_key] is populated. The non-
    // structural entry has no registered_at -> ineligible.
    regd.insert(struct_key, 0);
    let bps = compute_structural_share_bps(&snap, &regd, 720, 360, &[struct_key]);
    // structural = 100, total_eligible = 100. bps = 10_000.
    assert_eq!(bps, Some(10_000));
}

// ===========================================================================
// D.3 — Sunset gradient state machine (OracleSunsetState)
// Spec: specs/defi-l1-foundations-architecture.md §D.3
// ===========================================================================

// OUTPUT CONTRACT: OracleSunsetState::transition — HEALTHY -> WARNING
//   O1: share drops from 6500 to 5800 -> state becomes Warning
//   O2: warning_since_epoch is set to the current epoch
// PATHS:
//   P1: fresh state, share=5800 (below WARNING threshold 6000,
//       above SUNSET threshold 5500)
#[test]
fn test_warning_zone_entry() {
    let mut state = OracleSunsetState::default();
    // Start HEALTHY: share at 6500 bps (above warning threshold).
    let health = state.transition(Some(6500), 10);
    assert_eq!(health, OracleHealthState::Healthy);
    assert_eq!(state.warning_since_epoch, None);

    // Share drops to 5800 bps (below 6000, above 5500) -> WARNING.
    let health = state.transition(Some(5800), 11);
    assert_eq!(health, OracleHealthState::Warning);
    assert_eq!(state.warning_since_epoch, Some(11));
    assert_eq!(state.halt_since_epoch, None);
    assert!(!state.halt_permanent);
}

// OUTPUT CONTRACT: OracleSunsetState::transition — WARNING -> HEALTHY
//   O1: share rises from 5800 to 6500 -> state becomes Healthy
//   O2: warning_since_epoch is cleared
#[test]
fn test_warning_zone_recovery() {
    let mut state = OracleSunsetState::default();
    // Enter warning zone.
    state.transition(Some(5800), 10);
    assert_eq!(state.warning_since_epoch, Some(10));

    // Share rises back to 6500 bps (above 6000) -> HEALTHY.
    let health = state.transition(Some(6500), 11);
    assert_eq!(health, OracleHealthState::Healthy);
    assert_eq!(state.warning_since_epoch, None);
    assert_eq!(state.halt_since_epoch, None);
}

// OUTPUT CONTRACT: OracleSunsetState::transition — WARNING -> HALT_RECOVERABLE
//   O1: share drops from 5800 to 5400 -> state becomes HaltRecoverable
//   O2: halt_since_epoch is set
#[test]
fn test_halt_recoverable_entry() {
    let mut state = OracleSunsetState::default();
    // Enter warning zone first.
    state.transition(Some(5800), 10);
    assert_eq!(state.warning_since_epoch, Some(10));

    // Share drops below 5500 -> HALT_RECOVERABLE.
    let health = state.transition(Some(5400), 11);
    assert_eq!(health, OracleHealthState::HaltRecoverable);
    assert_eq!(state.halt_since_epoch, Some(11));
    assert_eq!(state.warning_since_epoch, Some(10));
    assert!(!state.halt_permanent);
}

// OUTPUT CONTRACT: OracleSunsetState::transition — HALT_RECOVERABLE -> WARNING
//   O1: share rises from <5500 to >=5500 within recovery window -> WARNING
//   O2: halt_since_epoch is cleared, warning_since_epoch stays
#[test]
fn test_halt_recovery_within_window() {
    let mut state = OracleSunsetState::default();
    // Enter halt at epoch 10.
    state.transition(Some(5400), 10);
    assert_eq!(state.halt_since_epoch, Some(10));

    // Share stays low at epoch 11.
    let health = state.transition(Some(5300), 11);
    assert_eq!(health, OracleHealthState::HaltRecoverable);

    // Share recovers to 5600 at epoch 12 (within 4-epoch window).
    let health = state.transition(Some(5600), 12);
    assert_eq!(health, OracleHealthState::Warning);
    assert_eq!(state.halt_since_epoch, None);
    // warning_since_epoch should still be set (share is in warning zone).
    assert!(state.warning_since_epoch.is_some());
}

// OUTPUT CONTRACT: OracleSunsetState::transition — HALT_PERMANENT after 4 epochs
//   O1: halt_since_epoch=N, current_epoch=N+4, share still <5500 -> HaltPermanent
//   O2: halt_permanent flag set sticky
#[test]
fn test_halt_permanent_after_4_epochs() {
    let mut state = OracleSunsetState::default();
    // Enter halt at epoch 10.
    state.transition(Some(5400), 10);
    assert_eq!(state.halt_since_epoch, Some(10));

    // Epoch 11, 12, 13: share stays below 5500.
    for ep in 11..14 {
        let health = state.transition(Some(5300), ep);
        assert_eq!(
            health,
            OracleHealthState::HaltRecoverable,
            "at epoch {ep}, should still be recoverable"
        );
    }

    // Epoch 14 (= 10 + 4): 4 epochs have elapsed -> PERMANENT.
    let health = state.transition(Some(5300), 14);
    assert_eq!(health, OracleHealthState::HaltPermanent);
    assert!(state.halt_permanent);

    // Once permanent, even recovery of share does not help.
    let health = state.transition(Some(9000), 15);
    assert_eq!(health, OracleHealthState::HaltPermanent);
    assert!(state.halt_permanent);
}

// OUTPUT CONTRACT: OracleSunsetState serde round-trip (persistence)
//   O1: write state with halt_since_epoch set, deserialize, verify
#[test]
fn test_state_persistence_across_restart() {
    let mut state = OracleSunsetState::default();
    state.transition(Some(5800), 5); // enter warning
    state.transition(Some(5400), 7); // enter halt

    // Simulate persistence via bincode round-trip (same as StateDB).
    let bytes = bincode::serialize(&state).expect("serialize");
    let restored: OracleSunsetState = bincode::deserialize(&bytes).expect("deserialize");

    assert_eq!(restored, state);
    assert_eq!(restored.warning_since_epoch, Some(5));
    assert_eq!(restored.halt_since_epoch, Some(7));
    assert!(!restored.halt_permanent);

    // After restore, the health query must return the same result.
    assert_eq!(restored.health(8), OracleHealthState::HaltRecoverable);
}

// OUTPUT CONTRACT: OracleHealthState helper methods
//   O1: should_aggregate() is true for Healthy and Warning only
//   O2: is_sunset_triggered() is true for HaltRecoverable and HaltPermanent
#[test]
fn test_health_state_helper_methods() {
    assert!(OracleHealthState::Healthy.should_aggregate());
    assert!(OracleHealthState::Warning.should_aggregate());
    assert!(!OracleHealthState::HaltRecoverable.should_aggregate());
    assert!(!OracleHealthState::HaltPermanent.should_aggregate());

    assert!(!OracleHealthState::Healthy.is_sunset_triggered());
    assert!(!OracleHealthState::Warning.is_sunset_triggered());
    assert!(OracleHealthState::HaltRecoverable.is_sunset_triggered());
    assert!(OracleHealthState::HaltPermanent.is_sunset_triggered());
}

// OUTPUT CONTRACT: OracleHealthState::as_rpc_str
//   O1: 4 states produce their documented RPC strings
#[test]
fn test_health_state_rpc_strings() {
    assert_eq!(OracleHealthState::Healthy.as_rpc_str(), "healthy");
    assert_eq!(OracleHealthState::Warning.as_rpc_str(), "warning");
    assert_eq!(
        OracleHealthState::HaltRecoverable.as_rpc_str(),
        "halted_recoverable"
    );
    assert_eq!(
        OracleHealthState::HaltPermanent.as_rpc_str(),
        "halted_permanent"
    );
}

// OUTPUT CONTRACT: direct HEALTHY -> HALT (skip warning zone)
//   O1: share drops from 6500 to 5400 in one step -> HaltRecoverable
//   O2: both warning_since_epoch and halt_since_epoch set
#[test]
fn test_direct_healthy_to_halt() {
    let mut state = OracleSunsetState::default();
    let health = state.transition(Some(5400), 10);
    assert_eq!(health, OracleHealthState::HaltRecoverable);
    assert_eq!(state.warning_since_epoch, Some(10));
    assert_eq!(state.halt_since_epoch, Some(10));
}

// OUTPUT CONTRACT: None share_bps treated as 0 (halt zone)
//   O1: no eligible bonds -> treated as share=0 -> enters halt
#[test]
fn test_none_share_enters_halt() {
    let mut state = OracleSunsetState::default();
    let health = state.transition(None, 10);
    assert_eq!(health, OracleHealthState::HaltRecoverable);
    assert_eq!(state.halt_since_epoch, Some(10));
}

// OUTPUT CONTRACT: recovery at exact boundary of recovery window
//   O1: at epoch halt+3 (< 4), recovery is still possible
//   O2: at epoch halt+4 (= 4), recovery is NOT possible
#[test]
fn test_recovery_window_boundary() {
    // Case 1: recover at halt+3 (should succeed)
    let mut state1 = OracleSunsetState::default();
    state1.transition(Some(5400), 10); // halt at epoch 10
    let health = state1.transition(Some(5600), 13); // epoch 13 = 10+3 < 10+4
    assert_eq!(health, OracleHealthState::Warning);
    assert_eq!(state1.halt_since_epoch, None);

    // Case 2: try to recover at halt+4 (should be permanent)
    let mut state2 = OracleSunsetState::default();
    state2.transition(Some(5400), 10); // halt at epoch 10
                                       // Keep share low through epochs 11-13.
    for ep in 11..14 {
        state2.transition(Some(5300), ep);
    }
    // At epoch 14, share goes up but it's too late.
    let health = state2.transition(Some(5600), 14);
    assert_eq!(health, OracleHealthState::HaltPermanent);
}

// OUTPUT CONTRACT: SUNSET_WARNING_BPS and ORACLE_RECOVERY_EPOCHS constants
//   O1: values match spec
#[test]
fn test_constants_match_spec() {
    assert_eq!(SUNSET_THRESHOLD_BPS, 5500);
    assert_eq!(SUNSET_WARNING_BPS, 6000);
    assert_eq!(ORACLE_RECOVERY_EPOCHS, 4);
}

// OUTPUT CONTRACT: fn dedupe_latest_per_attester — deterministic order
//   O1: output is sorted by signer_hash (BTreeMap iteration order, NOT
//       HashMap hash order). Guarantees bit-identical iteration across
//       all nodes regardless of hasher seed.
//   O2: dedup semantics preserved (last-write-wins per signer)
// PATHS / PARTITIONS:
//   P1: 3 distinct signers in non-sorted input order — verify output
//       comes out sorted by signer_hash byte-order
// MATRIX:
//   P1×O1✓  P1×O2✓
// AUDIT-P3-001: HashMap iteration in consensus path replaced with
// BTreeMap. The downstream median is order-independent today but the
// class — "future maintenance introduces an order-sensitive secondary
// effect → silent consensus fork" — is eliminated by deterministic
// iteration order.
#[test]
fn test_dedupe_latest_per_attester_output_is_sorted_by_signer_hash() {
    let input = vec![
        AttestationContribution {
            signer_hash: h(7), // out-of-order on purpose
            price_cents: 700,
        },
        AttestationContribution {
            signer_hash: h(2),
            price_cents: 200,
        },
        AttestationContribution {
            signer_hash: h(5),
            price_cents: 500,
        },
    ];
    let deduped = dedupe_latest_per_attester(&input);
    let signer_seeds: Vec<u8> = deduped.iter().map(|c| c.signer_hash.as_bytes()[0]).collect();
    let mut expected = signer_seeds.clone();
    expected.sort();
    assert_eq!(
        signer_seeds, expected,
        "AUDIT-P3-001: dedupe output must be sorted by signer_hash \
         (BTreeMap iteration order). HashMap order is non-deterministic \
         across hasher seeds and would diverge if any future code path \
         became iteration-order-sensitive."
    ); // O1
    assert_eq!(deduped.len(), 3); // O2 (no duplicates here, all preserved)
}
