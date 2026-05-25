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

use super::{bond_weighted_median, dedupe_latest_per_attester, AttestationContribution};
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
