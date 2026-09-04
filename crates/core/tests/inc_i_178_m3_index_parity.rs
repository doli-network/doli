//! INC-I-178 M3 — REQ-BLS-014 (Must): the five live decode sites do NOT agree.
//!
//! These tests assert INEQUALITY, so they are GREEN on the current tree the
//! moment `attestation_universe` exists.
//!
//! M4 SPLIT. This file keeps the PRE-AH half — the recorded five-site widths
//! `[50, 44, 45, 45, 49]`, which must stay true below
//! `inc_i_178_attestation_bls_activation_height` or the rolling deploy is a
//! consensus change. The POST-AH half (encoder == post_commit == stray-bit
//! denominator, one width) lives in `bins/node/tests/it/inc_i_178_m4_gate.rs`
//! (`req_bls_014_m4_post_ah_one_width_replaces_the_recorded_disagreement`),
//! because the three gate-parameterised site helpers are node-side symbols and
//! `crates/core` tests cannot reach `doli_node`.
//!
//! OUTPUT CONTRACT
//!
//! F1: the five production decode-length expressions, modelled read-only by
//!   `site_decode_len` (copied in shape from `inc_i_178_m0_bitfield_baseline.rs`;
//!   integration-test binaries share no code). Observable output:
//!     O1 the `producer_count` each site passes to `decode_attestation_bitfield_vec`
//!     O2 the index set each site therefore decodes from ONE block's bitfield
//! F2: `attestation_universe(base, active) -> Vec<PublicKey>`
//!     O3 return value; here only its LENGTH — the one canonical width
//!     O4 mutable params / receiver / store writes / statics / channels — NONE
//!   Paths: the single epoch shape in which BOTH directions are non-empty —
//!     P1 `producer_list \ active_at(h) != {}` (a mid-epoch EXIT)
//!     P2 `active_at(h) \ producer_list != {}` (mid-epoch additions)
//!     P3 a registry miss on the `producers.get_by_pubkey` filter (rewards.rs:139/145)
//!   INPUT PARTITIONS: M0's measured fixture — base `pk(0..45)`, active
//!     `pk(0..45)` minus seed 7 chained with `pk(100..105)`, registry
//!     `base | active` minus `pk(9)`; attendance with attesters in BOTH the base
//!     region and the extras region.
//!   MATRIX 4 outputs x 3 paths: O1 and O2 asserted on the combined P1+P2+P3
//!     shape (the only shape in which all four denominators are distinct); O3
//!     asserted against every one of O1; O4 is structural.

use crypto::PublicKey;
use doli_core::{
    attestation_universe, decode_attestation_bitfield_vec, encode_attestation_bitfield_vec,
    validate_attestation_bitfield_vec,
};
use std::collections::HashSet;

const SITE_NAMES: [&str; 5] = [
    "post_commit.rs:61/66",
    "rewards.rs:139/145",
    "rewards.rs:814/819",
    "rewards.rs:1016/1021",
    "schedule.rs:306/311",
];

/// Deterministic, injective test pubkey whose byte order is NOT seed order.
fn pk(seed: u16) -> PublicKey {
    let mut b = [0u8; 32];
    let m = seed.wrapping_mul(40_503);
    b[0] = (m >> 8) as u8;
    b[1] = m as u8;
    b[2] = (seed >> 8) as u8;
    b[3] = seed as u8;
    PublicKey::from_bytes(b)
}

/// `assembly.rs:410-424` verbatim: `active \ base`, independent of the fn under test.
fn extra_count(base: &[PublicKey], active: &[PublicKey]) -> usize {
    let base_set: HashSet<[u8; 32]> = base.iter().map(|p| *p.as_bytes()).collect();
    let mut seen: HashSet<[u8; 32]> = HashSet::new();
    active
        .iter()
        .filter(|p| !base_set.contains(p.as_bytes()) && seen.insert(*p.as_bytes()))
        .count()
}

/// The decode length each site passes for the same `(base, active, registry)`.
fn site_decode_len(
    site: usize,
    base: &[PublicKey],
    active: &[PublicKey],
    registry: &HashSet<[u8; 32]>,
) -> usize {
    let extra = extra_count(base, active);
    let active_set: HashSet<[u8; 32]> = active.iter().map(|p| *p.as_bytes()).collect();
    match site {
        0 => base.len() + extra,
        1 => base
            .iter()
            .filter(|p| registry.contains(p.as_bytes()))
            .count(),
        2 | 3 => base.len(),
        4 => {
            base.iter()
                .filter(|p| active_set.contains(p.as_bytes()))
                .count()
                + extra
        }
        _ => unreachable!("only five decode sites exist"),
    }
}

/// M0's measured epoch: one mid-epoch exit (seed 7), five mid-epoch additions,
/// one registry miss (seed 9).
fn divergent_epoch() -> (Vec<PublicKey>, Vec<PublicKey>, HashSet<[u8; 32]>) {
    let base: Vec<PublicKey> = (0..45u16).map(pk).collect();
    let active: Vec<PublicKey> = (0..45u16)
        .filter(|s| *s != 7)
        .chain(100..105u16)
        .map(pk)
        .collect();
    let missing = *pk(9).as_bytes();
    let registry: HashSet<[u8; 32]> = base
        .iter()
        .chain(active.iter())
        .map(|p| *p.as_bytes())
        .filter(|b| *b != missing)
        .collect();
    (base, active, registry)
}

/// REQ-BLS-014 (Must) — Decision: a failure means the four historically distinct denominators collapsed, so the same block no longer credits different producers depending on which site reads it — the intended M4 change, which must be reviewed as a consensus change and not absorbed as a refactor.
#[test]
fn req_bls_014_m3_divergence_five_sites_disagree_while_universe_fn_yields_one_width() {
    let (base, active, registry) = divergent_epoch();

    // P1: at least one producer left the active set mid-epoch.
    let active_set: HashSet<[u8; 32]> = active.iter().map(|p| *p.as_bytes()).collect();
    let exited: Vec<&PublicKey> = base
        .iter()
        .filter(|p| !active_set.contains(p.as_bytes()))
        .collect();
    assert_eq!(exited.len(), 1, "P1: producer_list \\ active_at(h) != {{}}");

    // P2: at least one producer joined mid-epoch.
    assert_eq!(extra_count(&base, &active), 5, "P2: active_at(h) \\ base");

    // P3: the registry filter drops exactly one base entry.
    assert!(!registry.contains(pk(9).as_bytes()), "P3: registry miss");

    // O1: the five denominators, as measured at M0.
    let lens: Vec<usize> = (0..SITE_NAMES.len())
        .map(|s| site_decode_len(s, &base, &active, &registry))
        .collect();
    assert_eq!(lens, vec![50, 44, 45, 45, 49], "O1: measured denominators");

    let distinct: HashSet<usize> = lens.iter().copied().collect();
    assert!(
        distinct.len() > 1,
        "O1: the load-bearing claim is disagreement"
    );
    assert_eq!(distinct.len(), 4, "O1: four distinct denominators");

    // O3: the shared universe answers with exactly ONE width.
    let canonical = attestation_universe(&base, &active).len();
    assert_eq!(canonical, 50, "O3: base 45 + 5 additions");
    let disagreeing = lens.iter().filter(|l| **l != canonical).count();
    assert!(
        disagreeing >= 2,
        "O3: expected at least two sites to disagree with {canonical}, got {disagreeing}"
    );

    // O2: the concrete consequence — one honest block, five readers, four answers.
    let bf = encode_attestation_bitfield_vec(&[0, 44, 47, 49], canonical);
    let decoded: Vec<Vec<usize>> = lens
        .iter()
        .map(|l| decode_attestation_bitfield_vec(&bf, *l))
        .collect();
    assert_eq!(decoded[0], vec![0, 44, 47, 49], "O2: {}", SITE_NAMES[0]);
    assert_eq!(decoded[1], vec![0], "O2: {}", SITE_NAMES[1]);
    assert_eq!(decoded[2], vec![0, 44], "O2: {}", SITE_NAMES[2]);
    assert_eq!(decoded[3], vec![0, 44], "O2: {}", SITE_NAMES[3]);
    assert_eq!(decoded[4], vec![0, 44, 47], "O2: {}", SITE_NAMES[4]);

    let distinct_credits: HashSet<Vec<usize>> = decoded.iter().cloned().collect();
    assert_eq!(
        distinct_credits.len(),
        4,
        "O2: the same block credits four different producer sets"
    );
    assert_ne!(
        decoded[0], decoded[1],
        "O2: post_commit and rewards credit different producers for the same block"
    );
}

/// REQ-BLS-014 (Must) — Decision: a failure means the honest encoder width no longer exceeds the stray-bit denominator when a producer exits mid-epoch, i.e. the latent honest-block rejection this milestone documents has moved or been silently fixed elsewhere.
#[test]
fn req_bls_014_m3_universe_width_exceeds_the_stray_bit_denominator_after_a_mid_epoch_exit() {
    let (base, active, _registry) = divergent_epoch();

    let canonical = attestation_universe(&base, &active).len();
    assert_eq!(canonical, 50, "the honest encoder width");
    assert_eq!(active.len(), 49, "active_producers_at_height(h).len()");
    assert!(
        canonical > active.len(),
        "the SHRINK direction: {canonical} > {}",
        active.len()
    );

    // An honest attester sitting in the extras region sets a bit the validator's
    // denominator cannot reach.
    let bf = encode_attestation_bitfield_vec(&[0, 44, 49], canonical);
    assert!(
        validate_attestation_bitfield_vec(&bf, canonical),
        "the block validates at its OWN width"
    );
    assert!(
        !validate_attestation_bitfield_vec(&bf, active.len()),
        "validation_checks.rs:430-443 rejects the same honest block at active.len()"
    );
}
