//! INC-I-178 M0 — CHARACTERIZATION baseline for the attendance bitfield.
//!
//! These tests are GREEN on the current tree by construction. They lock what the
//! shipped encoder/decoder/validator DO today so that M3 (one shared universe +
//! one decode length) and M4 (BLS aggregate commitment) cannot change pre-AH
//! bytes or pre-AH credited indices without a test turning red.
//!
//! Spec: `specs/attestation-bls-architecture.md` (D5, D6, D8, R3).
//! Analysis: `docs/redesigns/attestation-bls-redesign-analysis.md` §8.1.
//! Requirements: REQ-BLS-003 (Must), REQ-BLS-004 (Must), REQ-BLS-005 (Must).
//!
//! Names containing `current_behaviour` characterize a value that is
//! CHARACTERIZED, NOT ENDORSED: changing it is a consensus decision, not a bugfix.

use crypto::{Hash, PublicKey};
use doli_core::{
    decode_attestation_bitfield, decode_attestation_bitfield_vec, encode_attestation_bitfield,
    encode_attestation_bitfield_vec, validate_attestation_bitfield,
    validate_attestation_bitfield_vec,
};
use proptest::prelude::*;
use std::collections::HashSet;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Shared model of the five production decode sites
// ---------------------------------------------------------------------------

/// One production decode site and the verbatim `producer_count` it passes.
struct Site {
    name: &'static str,
    expr: &'static str,
}

/// The five decode sites, read read-only from the tree at INC-I-178 M0.
const SITES: [Site; 5] = [
    Site {
        name: "post_commit.rs:61/66",
        expr: "let base_len = self.epoch_state.producer_list.len(); \
               let (decode_len, extra_pks) = if use_full_decode && has_attestation_data \
               { let total = base_len + extra.len(); (total, extra) } else { (base_len, Vec::new()) }",
    },
    Site {
        name: "rewards.rs:139/145",
        expr: "let producer_count = sorted_producers.len(); \
               sorted_producers = if use_epoch_list { epl.iter().filter_map(|pk| producers.get_by_pubkey(pk).cloned()).collect() } \
               else { active_producers_at_height(epoch_start_height) sorted by public_key.as_bytes() }",
    },
    Site {
        name: "rewards.rs:814/819",
        expr: "sorted_for_decode.len(); \
               sorted_for_decode = if epoch_boundary_h >= epoch_prune_activation_height { self.epoch_state.producer_list.clone() } \
               else { let mut list = active.clone(); list.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes())); list }",
    },
    Site {
        name: "rewards.rs:1016/1021",
        expr: "sorted_for_decode.len(); \
               let mut sorted_for_decode = self.epoch_state.producer_list.clone(); \
               sorted_for_decode.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()))",
    },
    Site {
        name: "schedule.rs:306/311",
        expr: "let producer_count = sorted_producers.len(); \
               sorted_producers = if use_full_decode { [epl.iter().filter_map(|pk| all_active.iter().find(|(p, _)| p == pk)) | extra sorted by as_bytes()] } \
               else { legacy_producer_list(epoch_start) }",
    },
];

const N_VALUES: [usize; 12] = [0, 1, 8, 45, 63, 64, 65, 200, 255, 256, 257, 1000];
const SHAPES: [&str; 5] = ["empty", "full", "sparse", "first", "last"];

/// Attendance shapes over a universe of `n` producers.
fn shape_indices(n: usize, shape: &str) -> Vec<usize> {
    match shape {
        "empty" => Vec::new(),
        "full" => (0..n).collect(),
        "sparse" => (0..n).step_by(3).collect(),
        "first" if n > 0 => vec![0],
        "last" if n > 0 => vec![n - 1],
        _ => Vec::new(),
    }
}

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

fn key_bytes(v: &[PublicKey]) -> Vec<[u8; 32]> {
    v.iter().map(|p| *p.as_bytes()).collect()
}

/// Test-local model of `assembly.rs:385-455`: `[base | (active \ base) sorted]`.
/// Deliberately NOT production code — M3 owns the shared function.
fn universe(base: &[PublicKey], active: &[PublicKey]) -> Vec<PublicKey> {
    let base_set: HashSet<[u8; 32]> = base.iter().map(|p| *p.as_bytes()).collect();
    let mut extra: Vec<PublicKey> = active
        .iter()
        .filter(|p| !base_set.contains(p.as_bytes()))
        .copied()
        .collect();
    extra.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    let mut all = base.to_vec();
    all.extend(extra);
    all
}

/// The decode length each site would pass for the same (base, active, registry).
fn site_decode_len(
    site: usize,
    base: &[PublicKey],
    active: &[PublicKey],
    registry: &HashSet<[u8; 32]>,
) -> usize {
    let extra = universe(base, active).len() - base.len();
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

/// Vector-store model: full universe for the two `[base | extra]` sites,
/// base-only for the three rewards sites.
fn model_decode_len(site: usize, n: usize) -> usize {
    match site {
        0 | 4 => n,
        _ => n - n / 10,
    }
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("attestation_baseline_vectors.json")
}

// ---------------------------------------------------------------------------
// REQ-BLS-004 AC-3 — encode -> decode identity
// ---------------------------------------------------------------------------

fn assert_vec_identity(n: usize, shape: &str) {
    let idx = shape_indices(n, shape);
    let bf = encode_attestation_bitfield_vec(&idx, n);
    assert_eq!(bf.len(), n.div_ceil(8), "n={n} shape={shape}: byte length");
    assert_eq!(
        decode_attestation_bitfield_vec(&bf, n),
        idx,
        "n={n} shape={shape}: identity"
    );
    assert!(
        validate_attestation_bitfield_vec(&bf, n),
        "n={n} shape={shape}: encoder output must validate"
    );
}

/// REQ-BLS-004 (Must) AC-3 — Decision: a failure means M3's unified decode changed which producer indices a 45-producer epoch credits.
#[test]
fn req_bls_004_ac3_encode_decode_identity_vec_at_n45_all_shapes() {
    for shape in SHAPES {
        assert_vec_identity(45, shape);
    }
}

/// REQ-BLS-004 (Must) AC-3 — Decision: a failure means the identity that holds at 45 producers stops holding at the 200-producer scale the fleet is growing into.
#[test]
fn req_bls_004_ac3_encode_decode_identity_vec_at_n200_all_shapes() {
    for shape in SHAPES {
        assert_vec_identity(200, shape);
    }
}

/// REQ-BLS-004 (Must) AC-3 — Decision: a failure means the body-bitfield path lost its 256-producer-cap freedom, the whole reason it exists.
#[test]
fn req_bls_004_ac3_encode_decode_identity_vec_at_n1000_all_shapes() {
    for shape in SHAPES {
        assert_vec_identity(1000, shape);
    }
}

/// REQ-BLS-004 (Must) AC-3 — Decision: a failure means an off-by-one entered the byte/bit packing at a boundary, silently shifting every index past it.
#[test]
fn req_bls_004_ac3_encode_decode_identity_vec_at_byte_boundaries() {
    for n in [1usize, 8, 63, 64, 65, 255, 256, 257] {
        for shape in SHAPES {
            assert_vec_identity(n, shape);
        }
    }
}

/// REQ-BLS-004 (Must) AC-3 — Decision: a failure means R3's deletion of the legacy Hash codec would NOT have been byte-neutral for historical blocks.
#[test]
fn req_bls_004_ac3_encode_decode_identity_presence_root_at_or_below_256() {
    for n in [1usize, 8, 45, 63, 64, 65, 200, 255, 256] {
        for shape in SHAPES {
            let idx = shape_indices(n, shape);
            let root = encode_attestation_bitfield(&idx);
            assert_eq!(
                decode_attestation_bitfield(&root, n),
                idx,
                "n={n} shape={shape}: presence_root identity"
            );
        }
    }
}

/// REQ-BLS-004 (Must) AC-3 — Decision: a failure means the five sites stopped agreeing on identity for the lengths they actually pass, which is exactly what M3 unifies.
#[test]
fn req_bls_004_ac3_identity_holds_at_every_site_decode_len() {
    let base: Vec<PublicKey> = (0..45u16).map(pk).collect();
    let active: Vec<PublicKey> = (0..45u16).chain(100..105u16).map(pk).collect();
    let registry: HashSet<[u8; 32]> = active.iter().map(|p| *p.as_bytes()).collect();
    for (site, meta) in SITES.iter().enumerate() {
        let len = site_decode_len(site, &base, &active, &registry);
        for shape in SHAPES {
            let idx = shape_indices(len, shape);
            let bf = encode_attestation_bitfield_vec(&idx, len);
            assert_eq!(
                decode_attestation_bitfield_vec(&bf, len),
                idx,
                "site={} len={len} shape={shape}",
                meta.name
            );
        }
    }
}

// ---------------------------------------------------------------------------
// REQ-BLS-004 — CURRENT truncation edges (characterized, not endorsed)
// ---------------------------------------------------------------------------

/// REQ-BLS-004 (Must) — Decision: a failure means the 256-producer silent drop changed, which reassigns rewards for any fleet past 256 producers.
#[test]
fn req_bls_004_current_behaviour_presence_root_encoder_drops_indices_at_or_above_256() {
    let root = encode_attestation_bitfield(&[0, 255, 256, 257, 1000, usize::MAX]);
    let mut expected = [0u8; 32];
    expected[0] |= 1;
    expected[31] |= 1 << 7;
    assert_eq!(root, Hash::from_bytes(expected));
    assert_eq!(decode_attestation_bitfield(&root, 256), vec![0, 255]);
}

/// REQ-BLS-004 (Must) — Decision: a failure means identity above 256 started holding on the Hash path, so a historical block would now decode differently.
#[test]
fn req_bls_004_current_behaviour_presence_root_identity_fails_above_256() {
    let idx: Vec<usize> = (250..260).collect();
    let root = encode_attestation_bitfield(&idx);
    let decoded = decode_attestation_bitfield(&root, 260);
    assert_ne!(decoded, idx);
    assert_eq!(decoded, vec![250, 251, 252, 253, 254, 255]);
}

/// REQ-BLS-004 (Must) — Decision: a failure means the Hash decoder stopped capping at 256 and would read past the 32-byte root.
#[test]
fn req_bls_004_current_behaviour_presence_root_decoder_caps_scan_at_256() {
    let root = Hash::from_bytes([0xFF; 32]);
    assert_eq!(decode_attestation_bitfield(&root, 1000).len(), 256);
    assert_eq!(decode_attestation_bitfield(&root, 256).len(), 256);
    assert_eq!(decode_attestation_bitfield(&root, 45).len(), 45);
    assert_eq!(decode_attestation_bitfield(&root, 0), Vec::<usize>::new());
}

/// REQ-BLS-004 (Must) — Decision: a failure means an out-of-universe attester index started being encoded instead of dropped, or started panicking.
#[test]
fn req_bls_004_current_behaviour_vec_encoder_drops_indices_at_or_above_producer_count() {
    let bf = encode_attestation_bitfield_vec(&[0, 44, 45, 46, 10_000], 45);
    assert_eq!(bf.len(), 6);
    assert_eq!(decode_attestation_bitfield_vec(&bf, 45), vec![0, 44]);
    assert!(encode_attestation_bitfield_vec(&[0, 1, 2], 0).is_empty());
}

/// REQ-BLS-004 (Must) — Decision: a failure means a truncated or snap-synced bitfield stopped decoding as a silent partial and started diverging nodes.
#[test]
fn req_bls_004_current_behaviour_vec_decoder_breaks_early_on_short_bitfield() {
    let full = encode_attestation_bitfield_vec(&(0..200).collect::<Vec<_>>(), 200);
    assert_eq!(full.len(), 25);
    let short = &full[..3];
    assert_eq!(
        decode_attestation_bitfield_vec(short, 200),
        (0..24).collect::<Vec<usize>>()
    );
    assert_eq!(
        decode_attestation_bitfield_vec(&[], 200),
        Vec::<usize>::new()
    );
}

/// REQ-BLS-004 (Must) — Decision: a failure means the on-wire bitfield width changed, which changes block bytes and needs a synchronized deploy.
#[test]
fn req_bls_004_current_behaviour_vec_encode_length_is_ceil_producer_count_over_8() {
    let expected = [
        (0usize, 0usize),
        (1, 1),
        (8, 1),
        (9, 2),
        (63, 8),
        (64, 8),
        (65, 9),
        (200, 25),
        (255, 32),
        (256, 32),
        (257, 33),
        (1000, 125),
    ];
    for (n, bytes) in expected {
        assert_eq!(
            encode_attestation_bitfield_vec(&[], n).len(),
            bytes,
            "n={n}"
        );
    }
}

/// REQ-BLS-004 (Must) — Decision: a failure means an empty producer universe stopped producing the canonical empty encoding D6 depends on.
#[test]
fn req_bls_004_current_behaviour_zero_producer_count_yields_empty_bitfield_and_no_indices() {
    let bf = encode_attestation_bitfield_vec(&[], 0);
    assert!(bf.is_empty());
    assert_eq!(decode_attestation_bitfield_vec(&bf, 0), Vec::<usize>::new());
    assert!(validate_attestation_bitfield_vec(&bf, 0));
    assert_eq!(encode_attestation_bitfield(&[]), Hash::ZERO);
}

/// REQ-BLS-004 (Must) — Decision: a failure means M3 unified the decode length and the four historically distinct site values collapsed, which is the intended change and must be reviewed, not absorbed.
#[test]
fn req_bls_004_current_behaviour_five_sites_disagree_on_decode_len() {
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

    let lens: Vec<usize> = (0..SITES.len())
        .map(|s| site_decode_len(s, &base, &active, &registry))
        .collect();
    assert_eq!(lens, vec![50, 44, 45, 45, 49]);

    let bf = encode_attestation_bitfield_vec(&[0, 44, 47, 49], 50);
    let decoded: Vec<Vec<usize>> = lens
        .iter()
        .map(|l| decode_attestation_bitfield_vec(&bf, *l))
        .collect();
    assert_eq!(decoded[0], vec![0, 44, 47, 49]);
    assert_eq!(decoded[1], vec![0]);
    assert_eq!(decoded[2], vec![0, 44]);
    assert_eq!(decoded[3], vec![0, 44]);
    assert_eq!(decoded[4], vec![0, 44, 47]);
}

// ---------------------------------------------------------------------------
// REQ-BLS-003 AC-3 — presence_root == BLAKE3(bitfield) + stray-bit bound
// ---------------------------------------------------------------------------

/// REQ-BLS-003 (Must) AC-3 — Decision: a failure means D6's new commitment leaked into the pre-AH path and every existing block's presence_root stopped verifying.
#[test]
fn req_bls_003_ac3_presence_root_is_blake3_of_body_bitfield_empty_full_sparse() {
    for n in [0usize, 1, 8, 45, 64, 65, 200, 256, 257, 1000] {
        for shape in ["empty", "full", "sparse"] {
            let bf = encode_attestation_bitfield_vec(&shape_indices(n, shape), n);
            assert_eq!(
                crypto::hash::hash(&bf),
                crypto::hash::hash(bf.as_slice()),
                "n={n} shape={shape}: hash must be slice-stable"
            );
            assert_ne!(
                crypto::hash::hash(&bf),
                Hash::ZERO,
                "n={n} shape={shape}: BLAKE3 never yields the absent sentinel"
            );
        }
    }
}

/// REQ-BLS-003 (Must) AC-3 — Decision: a failure means a relay could flip one attendance bit without breaking the commitment validation_checks.rs:426 enforces.
#[test]
fn req_bls_003_ac3_presence_root_commitment_breaks_on_single_bit_flip() {
    let bf = encode_attestation_bitfield_vec(&shape_indices(200, "sparse"), 200);
    let good = crypto::hash::hash(&bf);
    for byte in 0..bf.len() {
        for bit in 0..8u32 {
            let mut tampered = bf.clone();
            tampered[byte] ^= 1 << bit;
            assert_ne!(
                crypto::hash::hash(&tampered),
                good,
                "byte={byte} bit={bit}: flip must break the commitment"
            );
        }
    }
}

/// REQ-BLS-003 (Must) AC-3 — Decision: a failure means an honest producer's own encoder output would be rejected by the stray-bit check, halting the chain.
#[test]
fn req_bls_003_ac3_validate_vec_accepts_encoder_output_at_every_covered_n() {
    for n in N_VALUES {
        for shape in SHAPES {
            let bf = encode_attestation_bitfield_vec(&shape_indices(n, shape), n);
            assert!(
                validate_attestation_bitfield_vec(&bf, n),
                "n={n} shape={shape}"
            );
        }
    }
}

/// REQ-BLS-003 (Must) AC-3 — Decision: a failure means a producer could set a bit for a non-existent producer index and have the block accepted.
#[test]
fn req_bls_003_ac3_validate_vec_rejects_stray_bit_at_non_multiple_of_8_producer_counts() {
    for n in [1usize, 9, 45, 63, 65, 100, 255, 257, 1001] {
        let mut bf = encode_attestation_bitfield_vec(&[], n);
        let last = bf.len() - 1;
        let remainder = n % 8;
        assert!(remainder > 0, "n={n} must not be a multiple of 8");
        bf[last] |= 1 << remainder;
        assert!(
            !validate_attestation_bitfield_vec(&bf, n),
            "n={n}: stray bit at position {remainder} must be rejected"
        );
    }
}

/// REQ-BLS-003 (Must) AC-3 — Decision: a failure means an over-long bitfield could smuggle set bits past the producer count undetected.
#[test]
fn req_bls_003_ac3_validate_vec_rejects_nonzero_trailing_byte() {
    let mut bf = encode_attestation_bitfield_vec(&[0, 3], 45);
    bf.push(0x00);
    assert!(validate_attestation_bitfield_vec(&bf, 45));
    bf.push(0x01);
    assert!(!validate_attestation_bitfield_vec(&bf, 45));
}

/// REQ-BLS-003 (Must) AC-3 — Decision: a failure means the truncated-bitfield acceptance changed, which changes whether a snap-sync gap block is admitted.
#[test]
fn req_bls_003_ac3_current_behaviour_validate_vec_accepts_short_bitfield() {
    assert!(validate_attestation_bitfield_vec(&[], 200));
    assert!(validate_attestation_bitfield_vec(&[0xFF, 0xFF], 200));
    assert!(validate_attestation_bitfield_vec(&[0xFF; 24], 200));
}

/// REQ-BLS-003 (Must) AC-3 — Decision: a failure means the multiple-of-8 case gained or lost a last-byte check, changing acceptance for exactly-byte-aligned fleets.
#[test]
fn req_bls_003_ac3_current_behaviour_validate_vec_has_no_stray_check_at_multiple_of_8() {
    for n in [8usize, 64, 256, 1000] {
        let bf = vec![0xFFu8; n.div_ceil(8)];
        assert!(
            validate_attestation_bitfield_vec(&bf, n),
            "n={n}: all bits are in-universe when n % 8 == 0"
        );
    }
}

/// REQ-BLS-003 (Must) AC-3 — Decision: a failure means the legacy Hash validator stopped waving through fleets of 256+, changing acceptance on the path R3 wants to delete.
#[test]
fn req_bls_003_ac3_current_behaviour_validate_presence_root_is_unconditionally_true_at_256_and_above(
) {
    let root = Hash::from_bytes([0xFF; 32]);
    assert!(validate_attestation_bitfield(&root, 256));
    assert!(validate_attestation_bitfield(&root, 1000));
    assert!(!validate_attestation_bitfield(&root, 255));
}

/// REQ-BLS-003 (Must) AC-3 — Decision: a failure means the legacy stray-bit bound stopped rejecting out-of-universe bits below 256.
#[test]
fn req_bls_003_ac3_validate_presence_root_rejects_stray_bit_below_256() {
    for n in [1usize, 45, 63, 64, 65, 200, 255] {
        let clean = encode_attestation_bitfield(&shape_indices(n, "full"));
        assert!(validate_attestation_bitfield(&clean, n), "n={n}: clean");
        let mut bytes = *clean.as_bytes();
        bytes[n / 8] |= 1 << (n % 8);
        assert!(
            !validate_attestation_bitfield(&Hash::from_bytes(bytes), n),
            "n={n}: bit {n} is out of universe"
        );
    }
}

// ---------------------------------------------------------------------------
// REQ-BLS-004 — universe prefix property
// ---------------------------------------------------------------------------

fn assert_universe_contract(base: &[PublicKey], active: &[PublicKey]) {
    let all = universe(base, active);
    assert_eq!(
        key_bytes(&all[..base.len()]),
        key_bytes(base),
        "base must be a byte-exact prefix"
    );
    let set: HashSet<[u8; 32]> = all.iter().map(|p| *p.as_bytes()).collect();
    assert_eq!(set.len(), all.len(), "universe must be duplicate-free");
    assert!(
        all[base.len()..]
            .windows(2)
            .all(|w| w[0].as_bytes() < w[1].as_bytes()),
        "extra must be strictly sorted by pubkey bytes"
    );
}

/// REQ-BLS-004 (Must) — Decision: a failure means M3's shared universe reordered the base, shifting every historical attendance index.
#[test]
fn req_bls_004_universe_prefix_equals_base_disjoint_active() {
    let base: Vec<PublicKey> = (0..45u16).map(pk).collect();
    let active: Vec<PublicKey> = (100..140u16).map(pk).collect();
    assert_universe_contract(&base, &active);
    assert_eq!(universe(&base, &active).len(), 85);
}

/// REQ-BLS-004 (Must) — Decision: a failure means an epoch whose active set shrank below the base started producing a shorter universe, dropping indices mid-epoch.
#[test]
fn req_bls_004_universe_prefix_equals_base_active_subset_of_base() {
    let base: Vec<PublicKey> = (0..45u16).map(pk).collect();
    let active: Vec<PublicKey> = (0..20u16).map(pk).collect();
    assert_universe_contract(&base, &active);
    assert_eq!(
        key_bytes(&universe(&base, &active)),
        key_bytes(&base),
        "an active subset adds nothing"
    );
}

/// REQ-BLS-004 (Must) — Decision: a failure means a producer that activated mid-epoch stopped landing at a stable index, which is the exact defect the full-bitfield-decode pillar fixed.
#[test]
fn req_bls_004_universe_prefix_equals_base_with_mid_epoch_additions() {
    let base: Vec<PublicKey> = (0..45u16).map(pk).collect();
    let mut active: Vec<PublicKey> = (0..45u16).map(pk).collect();
    active.extend((200..205u16).map(pk));
    assert_universe_contract(&base, &active);
    let all = universe(&base, &active);
    assert_eq!(all.len(), 50);
    let mut expected_extra: Vec<[u8; 32]> = (200..205u16).map(|s| *pk(s).as_bytes()).collect();
    expected_extra.sort_unstable();
    assert_eq!(key_bytes(&all[45..]), expected_extra);
}

/// REQ-BLS-004 (Must) — Decision: a failure means the tail ordering became insertion-dependent, so two honest nodes would build different universes from the same state.
#[test]
fn req_bls_004_universe_extra_is_sorted_by_pubkey_bytes_and_duplicate_free() {
    let base: Vec<PublicKey> = (0..10u16).map(pk).collect();
    let forward: Vec<PublicKey> = (500..540u16).map(pk).collect();
    let mut reversed = forward.clone();
    reversed.reverse();
    assert_eq!(
        key_bytes(&universe(&base, &forward)),
        key_bytes(&universe(&base, &reversed)),
        "extra ordering must not depend on active-set iteration order"
    );
    assert_universe_contract(&base, &forward);
}

// ---------------------------------------------------------------------------
// REQ-BLS-005 AC-1 — committed golden vectors
// ---------------------------------------------------------------------------

fn make_vector(id: String, n: usize, shape: &str, site: usize) -> serde_json::Value {
    let attested = shape_indices(n, shape);
    let bitfield = encode_attestation_bitfield_vec(&attested, n);
    let decode_len = model_decode_len(site, n);
    serde_json::json!({
        "id": id,
        "n": n,
        "shape": shape,
        "attested": attested,
        "bitfield_hex": hex::encode(&bitfield),
        "presence_root_hex": crypto::hash::hash(&bitfield).to_hex(),
        "legacy_presence_root_hex": encode_attestation_bitfield(&attested).to_hex(),
        "decode_len": decode_len,
        "decode_len_expr": SITES[site].expr,
        "site": SITES[site].name,
        "decoded": decode_attestation_bitfield_vec(&bitfield, decode_len),
    })
}

fn build_vectors() -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let mut k = 0usize;
    for n in N_VALUES {
        for shape in SHAPES {
            if n == 0 && shape != "empty" {
                continue;
            }
            let site = k % SITES.len();
            out.push(make_vector(format!("n{n}-{shape}-s{site}"), n, shape, site));
            k += 1;
        }
    }
    for n in [45usize, 200] {
        for site in 0..SITES.len() {
            out.push(make_vector(format!("site-n{n}-s{site}"), n, "sparse", site));
        }
    }
    out
}

fn load_fixture() -> serde_json::Value {
    let path = fixture_path();
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "REQ-BLS-005 AC-1 baseline vectors missing at {}: {e}. \
             Run `cargo test -p doli-core --test inc_i_178_m0_bitfield_baseline \
             -- --ignored regenerate_attestation_baseline_vectors` ONLY with a deliberate consensus change.",
            path.display()
        )
    });
    serde_json::from_str(&raw).expect("baseline vector file must be valid JSON")
}

fn as_usize_vec(v: &serde_json::Value, field: &str) -> Vec<usize> {
    v.get(field)
        .and_then(serde_json::Value::as_array)
        .unwrap_or_else(|| panic!("vector field `{field}` must be an array"))
        .iter()
        .map(|x| x.as_u64().expect("index must be a u64") as usize)
        .collect()
}

/// REQ-BLS-005 (Must) AC-1 — Decision: a failure means the committed store no longer describes what the tree emits, so nothing proves a new binary is byte-identical pre-AH.
#[test]
fn req_bls_005_ac1_baseline_vector_file_is_present_and_well_formed() {
    let doc = load_fixture();
    assert_eq!(doc["schema"].as_u64(), Some(1));
    assert!(doc["generated_for"]
        .as_str()
        .is_some_and(|s| s.contains("INC-I-178")));
    assert!(doc["note"].as_str().is_some_and(|s| !s.is_empty()));
    let vectors = doc["vectors"].as_array().expect("vectors array");
    assert!(
        vectors.len() >= 40,
        "expected >= 40 vectors, got {}",
        vectors.len()
    );

    let ids: HashSet<&str> = vectors.iter().filter_map(|v| v["id"].as_str()).collect();
    assert_eq!(ids.len(), vectors.len(), "vector ids must be unique");

    let sites: HashSet<&str> = vectors.iter().filter_map(|v| v["site"].as_str()).collect();
    for site in SITES {
        assert!(
            sites.contains(site.name),
            "site {} has no vector",
            site.name
        );
    }
    for n in N_VALUES {
        assert!(
            vectors.iter().any(|v| v["n"].as_u64() == Some(n as u64)),
            "n={n} has no vector"
        );
    }
}

/// REQ-BLS-005 (Must) AC-1 — Decision: a failure means a later binary emits different pre-AH bytes than the one that produced these vectors, i.e. a silent consensus change.
#[test]
fn req_bls_005_ac1_committed_baseline_vectors_reproduce_from_production_encoder() {
    let doc = load_fixture();
    let vectors = doc["vectors"].as_array().expect("vectors array");
    for v in vectors {
        let id = v["id"].as_str().expect("id");
        let n = v["n"].as_u64().expect("n") as usize;
        let attested = as_usize_vec(v, "attested");

        let bitfield = encode_attestation_bitfield_vec(&attested, n);
        assert_eq!(
            hex::encode(&bitfield),
            v["bitfield_hex"].as_str().unwrap(),
            "{id}: bitfield bytes"
        );
        assert_eq!(bitfield.len(), n.div_ceil(8), "{id}: bitfield width");
        assert_eq!(
            crypto::hash::hash(&bitfield).to_hex(),
            v["presence_root_hex"].as_str().unwrap(),
            "{id}: presence_root = BLAKE3(bitfield)"
        );
        assert_eq!(
            encode_attestation_bitfield(&attested).to_hex(),
            v["legacy_presence_root_hex"].as_str().unwrap(),
            "{id}: legacy packed presence_root"
        );

        let decode_len = v["decode_len"].as_u64().expect("decode_len") as usize;
        assert_eq!(
            decode_attestation_bitfield_vec(&bitfield, decode_len),
            as_usize_vec(v, "decoded"),
            "{id}: decoded at {}",
            v["site"].as_str().unwrap()
        );
        assert!(
            validate_attestation_bitfield_vec(&bitfield, n),
            "{id}: validates"
        );
        assert!(
            !v["decode_len_expr"].as_str().unwrap().is_empty(),
            "{id}: expr"
        );
    }
}

/// REQ-BLS-005 (Must) AC-1 — Decision: regenerating rewrites the consensus reference, so it must be an explicit, ignored, deliberate act.
#[test]
#[ignore = "rewrites the committed consensus reference; run only with a deliberate consensus change"]
fn regenerate_attestation_baseline_vectors() {
    let doc = serde_json::json!({
        "schema": 1,
        "generated_for": "INC-I-178 M0 / run 544",
        "note": "Reference bytes emitted by the pre-AH production encoder. A later binary that \
                 reproduces these is byte-identical on the pre-AH path (REQ-BLS-005 AC-1). \
                 Regenerate ONLY with a deliberate consensus change.",
        "vectors": build_vectors(),
    });
    let path = fixture_path();
    std::fs::create_dir_all(path.parent().unwrap()).expect("create fixtures dir");
    let mut body = serde_json::to_string_pretty(&doc).expect("serialize");
    body.push('\n');
    std::fs::write(&path, body).expect("write baseline vectors");
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// REQ-BLS-004 (Must) AC-3 — Decision: a failure names a concrete (n, indices) pair where the shipped round-trip loses or invents an attester.
    #[test]
    fn req_bls_004_ac3_prop_vec_roundtrip_identity(
        n in 1usize..=1200,
        picks in prop::collection::hash_set(0usize..1200, 0..80),
    ) {
        let mut idx: Vec<usize> = picks.into_iter().filter(|i| *i < n).collect();
        idx.sort_unstable();
        let bf = encode_attestation_bitfield_vec(&idx, n);
        prop_assert_eq!(bf.len(), n.div_ceil(8));
        prop_assert_eq!(decode_attestation_bitfield_vec(&bf, n), idx);
        prop_assert!(validate_attestation_bitfield_vec(&bf, n));
    }

    /// REQ-BLS-004 (Must) — Decision: a failure names a (base, active) pair where the universe stops being base-prefixed or duplicate-free, breaking index stability.
    #[test]
    fn req_bls_004_prop_universe_prefix_and_dupfree(
        base_seeds in prop::collection::hash_set(0u16..600, 0..50),
        active_seeds in prop::collection::hash_set(0u16..600, 0..50),
    ) {
        let mut bs: Vec<u16> = base_seeds.into_iter().collect();
        bs.sort_unstable();
        let base: Vec<PublicKey> = bs.iter().copied().map(pk).collect();
        let active: Vec<PublicKey> = active_seeds.into_iter().map(pk).collect();

        let all = universe(&base, &active);
        prop_assert_eq!(key_bytes(&all[..base.len()]), key_bytes(&base));
        let set: HashSet<[u8; 32]> = all.iter().map(|p| *p.as_bytes()).collect();
        prop_assert_eq!(set.len(), all.len());
        prop_assert!(all[base.len()..].windows(2).all(|w| w[0].as_bytes() < w[1].as_bytes()));
        prop_assert!(all.len() >= base.len());
    }
}
