//! INC-I-178 M3 — REQ-BLS-004 (Must): the ONE canonical attestation universe.
//!
//! RED until the developer lands `doli_core::attestation_universe`.
//!
//! OUTPUT CONTRACT
//!
//! F1: `attestation_universe(base: &[PublicKey], active: &[PublicKey]) -> Vec<PublicKey>`
//!   O1 mutable parameters — NONE; both are shared slices. Asserted negatively:
//!      `base` and `active` are byte-identical after the call.
//!   O2 receiver/self — N/A, free function.
//!   O3 return value — THE contract, four separable properties:
//!      O3a base is a byte-exact PREFIX, order preserved, never re-sorted
//!      O3b extras are exactly `active \ base`, strictly ascending by `as_bytes()`
//!      O3c the whole vector is duplicate-free (C5/F14)
//!      O3d width is `|dedup(base)| + |dedup(active) \ dedup(base)|`
//!   O4 persistent store writes — NONE; the signature admits no height and no
//!      store, which is what keeps `doli-core` free of `doli-storage`.
//!   O5 global/static state — NONE.  O6 channel/event emissions — NONE.
//!   Paths (input-shape classes):
//!     P1 base and active disjoint, both non-empty
//!     P2 active subset of base            P3 base subset of active
//!     P4 base empty                       P5 active empty
//!     P6 both empty                       P7 duplicate inside base
//!     P8 duplicate inside active          P9 same key in base AND active
//!   INPUT PARTITIONS:
//!     base ordering: seed order (not byte order), byte-DESCENDING, byte-ascending
//!     active iteration order: forward and reversed
//!     N in {0, 1, 45, 200, 1000}
//!     all 66 committed golden vectors x splits b in {n, n/2, n/3, 0}
//!   MATRIX 6 outputs x 9 paths: O3 is asserted on every one of P1-P9; O1 on P1
//!     and P7; O4/O5/O6 are structural — the signature carries no store, no
//!     height and no receiver, so the crate boundary is the assertion.

use crypto::PublicKey;
use doli_core::{
    attestation_universe, decode_attestation_bitfield_vec, encode_attestation_bitfield_vec,
    validate_attestation_bitfield_vec,
};
use std::collections::HashSet;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Shared model (mirrors `inc_i_178_m0_bitfield_baseline.rs`)
// ---------------------------------------------------------------------------

const SHAPES: [&str; 5] = ["empty", "full", "sparse", "first", "last"];

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

/// A `(base, active)` pair whose canonical universe is exactly `n` wide:
/// `b` frozen base entries plus `n - b` mid-epoch additions disjoint from them.
fn split(n: usize, b: usize) -> (Vec<PublicKey>, Vec<PublicKey>) {
    let b = b.min(n);
    let base: Vec<PublicKey> = (0..b as u16).map(pk).collect();
    let mut active = base.clone();
    active.extend((0..(n - b) as u16).map(|i| pk(10_000 + i)));
    (base, active)
}

/// O3a + O3b + O3c on one call, for any `(base, active)` with a duplicate-free base.
fn assert_universe_contract(base: &[PublicKey], active: &[PublicKey]) {
    let all = attestation_universe(base, active);
    assert_eq!(
        key_bytes(&all[..base.len()]),
        key_bytes(base),
        "O3a: base must be a byte-exact prefix in its own order"
    );
    let set: HashSet<[u8; 32]> = all.iter().map(|p| *p.as_bytes()).collect();
    assert_eq!(set.len(), all.len(), "O3c: universe must be duplicate-free");
    assert!(
        all[base.len()..]
            .windows(2)
            .all(|w| w[0].as_bytes() < w[1].as_bytes()),
        "O3b: extras must be strictly ascending by pubkey bytes"
    );
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("attestation_baseline_vectors.json")
}

fn load_fixture() -> serde_json::Value {
    let path = fixture_path();
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("M0 golden store missing at {}: {e}", path.display()));
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

// ---------------------------------------------------------------------------
// O3a — base prefix preservation (P1, P2, P3)
// ---------------------------------------------------------------------------

/// REQ-BLS-004 (Must) — Decision: a failure means the shared universe re-sorted the frozen base, so every historical attendance index shifts and the block that credited producer i now credits someone else — the exact contradiction M0 measured at `rewards.rs:1016`.
#[test]
fn req_bls_004_base_prefix_is_preserved_verbatim_at_n45_n200_n1000() {
    for n in [45usize, 200, 1000] {
        for b in [n, n / 2, n / 3, 1] {
            let (base, active) = split(n, b);
            assert_universe_contract(&base, &active);
            assert_eq!(
                attestation_universe(&base, &active).len(),
                n,
                "n={n} b={b}: O3d width"
            );
        }
    }
}

/// REQ-BLS-004 (Must) — Decision: a failure means the function sorts its base, which passes silently whenever the base happens to arrive sorted and corrupts every index the moment it does not.
#[test]
fn req_bls_004_a_byte_descending_base_is_never_re_sorted() {
    for n in [45usize, 200, 1000] {
        let mut base: Vec<PublicKey> = (0..n as u16).map(pk).collect();
        base.sort_by(|a, b| b.as_bytes().cmp(a.as_bytes()));
        assert!(
            base.windows(2).all(|w| w[0].as_bytes() > w[1].as_bytes()),
            "n={n}: fixture must be strictly DESCENDING for this test to falsify"
        );
        let mut active = base.clone();
        active.extend((0..5u16).map(|i| pk(10_000 + i)));

        let all = attestation_universe(&base, &active);
        assert_eq!(
            key_bytes(&all[..n]),
            key_bytes(&base),
            "n={n}: O3a on a descending base"
        );
        assert_eq!(all.len(), n + 5, "n={n}: O3d");
        assert_universe_contract(&base, &active);
    }
}

// ---------------------------------------------------------------------------
// O3b — extras are exactly `active \ base`, sorted, disjoint (P1, P3)
// ---------------------------------------------------------------------------

/// REQ-BLS-004 (Must) — Decision: a failure means two honest nodes with the same state build different universes, so the same block decodes to different attesters on each — the death spiral the Full Bitfield Decode pillar closed.
#[test]
fn req_bls_004_extras_are_exactly_active_minus_base_sorted_and_disjoint() {
    let base: Vec<PublicKey> = (0..45u16).map(pk).collect();
    let mut active: Vec<PublicKey> = (0..45u16).map(pk).collect();
    active.extend((200..240u16).map(pk));

    let all = attestation_universe(&base, &active);
    let extras = &all[base.len()..];

    let mut expected: Vec<[u8; 32]> = (200..240u16).map(|s| *pk(s).as_bytes()).collect();
    expected.sort_unstable();
    assert_eq!(key_bytes(extras), expected, "O3b: extras = active \\ base");

    let base_set: HashSet<[u8; 32]> = base.iter().map(|p| *p.as_bytes()).collect();
    assert!(
        extras.iter().all(|p| !base_set.contains(p.as_bytes())),
        "O3b: extras must be disjoint from base"
    );

    let mut reversed = active.clone();
    reversed.reverse();
    assert_eq!(
        key_bytes(&attestation_universe(&base, &reversed)),
        key_bytes(&all),
        "O3b: extras must not depend on active-set iteration order"
    );
}

// ---------------------------------------------------------------------------
// O3c — duplicate-free under adversarial input (P7, P8, P9)
// ---------------------------------------------------------------------------

/// REQ-BLS-004 (Must) — Decision: a failure means a repeated base entry reserves two bitfield slots, so one attester owns two bits and every index after it is off by one for the rest of the epoch.
#[test]
fn req_bls_004_duplicate_inside_base_collapses_to_its_first_position() {
    let mut base: Vec<PublicKey> = (0..10u16).map(pk).collect();
    base.insert(7, pk(3));
    assert_eq!(base.len(), 11);

    let all = attestation_universe(&base, &[]);
    assert_eq!(all.len(), 10, "O3d: the repeat must not widen the universe");
    assert_eq!(
        key_bytes(&all),
        key_bytes(&(0..10u16).map(pk).collect::<Vec<_>>()),
        "O3a+O3c: the survivor keeps its FIRST base position"
    );
    assert_eq!(
        all.iter()
            .filter(|p| *p.as_bytes() == *pk(3).as_bytes())
            .count(),
        1,
        "O3c: exactly once"
    );
}

/// REQ-BLS-004 (Must) — Decision: a failure means a producer already frozen into the base gets a second index appended in the extras region, and a block crediting the base index leaves the extras bit dark for every node that reads it the other way.
#[test]
fn req_bls_004_key_present_in_both_base_and_active_is_never_appended_as_an_extra() {
    let base: Vec<PublicKey> = (0..45u16).map(pk).collect();
    let mut active: Vec<PublicKey> = (0..45u16).map(pk).collect();
    active.extend((300..305u16).map(pk));

    let all = attestation_universe(&base, &active);
    assert_eq!(all.len(), 50, "O3d: 45 base + 5 genuinely new");
    for s in 0..45u16 {
        let hits: Vec<usize> = all
            .iter()
            .enumerate()
            .filter(|(_, p)| *p.as_bytes() == *pk(s).as_bytes())
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            hits,
            vec![s as usize],
            "O3c: seed {s} appears once, in base"
        );
    }
    assert_universe_contract(&base, &active);
}

/// REQ-BLS-004 (Must) — Decision: a failure means a duplicated entry in the caller's `active_at(h)` widens the bitfield past what the stray-bit validator allows, and honest blocks start being rejected.
#[test]
fn req_bls_004_duplicate_extra_inside_active_is_appended_once() {
    let base: Vec<PublicKey> = (0..10u16).map(pk).collect();
    let mut active: Vec<PublicKey> = (0..10u16).map(pk).collect();
    active.extend([pk(500), pk(501), pk(500), pk(501), pk(500)]);

    let all = attestation_universe(&base, &active);
    assert_eq!(all.len(), 12, "O3d: two distinct extras, not five");
    let mut expected: Vec<[u8; 32]> = vec![*pk(500).as_bytes(), *pk(501).as_bytes()];
    expected.sort_unstable();
    assert_eq!(key_bytes(&all[10..]), expected, "O3b+O3c");
    assert_universe_contract(&base, &active);
}

// ---------------------------------------------------------------------------
// O3 — empty inputs (P4, P5, P6)
// ---------------------------------------------------------------------------

/// REQ-BLS-004 (Must) — Decision: a failure means the universe panics or fabricates entries at an epoch edge where one side is empty, which is a node crash on a live block rather than a rejection.
#[test]
fn req_bls_004_empty_inputs_yield_empty_base_only_and_sorted_active() {
    assert!(
        attestation_universe(&[], &[]).is_empty(),
        "P6: nothing in, nothing out"
    );

    let mut active: Vec<PublicKey> = (0..45u16).map(pk).collect();
    active.reverse();
    let from_empty_base = attestation_universe(&[], &active);
    let mut expected: Vec<[u8; 32]> = (0..45u16).map(|s| *pk(s).as_bytes()).collect();
    expected.sort_unstable();
    assert_eq!(key_bytes(&from_empty_base), expected, "P4: active, sorted");

    let base: Vec<PublicKey> = (0..45u16).map(pk).collect();
    assert_eq!(
        key_bytes(&attestation_universe(&base, &[])),
        key_bytes(&base),
        "P5: base unchanged, still in base order"
    );
}

// ---------------------------------------------------------------------------
// O1 + O3 — determinism, idempotence, input immutability
// ---------------------------------------------------------------------------

/// REQ-BLS-004 (Must) — Decision: a failure means the universe is not a pure function of its two inputs, so the encoder and the validator can disagree on the same state within one process.
#[test]
fn req_bls_004_is_deterministic_idempotent_and_leaves_its_inputs_untouched() {
    let (base, active) = split(200, 120);
    let base_before = key_bytes(&base);
    let active_before = key_bytes(&active);

    let a = attestation_universe(&base, &active);
    let b = attestation_universe(&base, &active);
    assert_eq!(key_bytes(&a), key_bytes(&b), "O3: two calls, one answer");

    // O1: shared slices must survive the call byte-identical.
    assert_eq!(key_bytes(&base), base_before, "O1: base untouched");
    assert_eq!(key_bytes(&active), active_before, "O1: active untouched");

    // Idempotence: feeding a duplicate-free universe back in is a no-op.
    assert_eq!(
        key_bytes(&attestation_universe(&a, &a)),
        key_bytes(&a),
        "O3: universe(u, u) == u"
    );
}

// ---------------------------------------------------------------------------
// O3 — round-trip identity against the CURRENT encoder/decoder/validator
// ---------------------------------------------------------------------------

/// REQ-BLS-004 (Must) AC-3 — Decision: a failure means a bitfield sized by the shared universe no longer round-trips through the shipped encoder, so M4 would ship a width the live decoders cannot read.
#[test]
fn req_bls_004_round_trip_identity_over_the_universe_width_at_n45_n200_n1000() {
    for n in [45usize, 200, 1000] {
        for b in [n, n / 2, n / 3, 0] {
            let (base, active) = split(n, b);
            let u = attestation_universe(&base, &active);
            assert_eq!(u.len(), n, "n={n} b={b}: O3d");
            for shape in SHAPES {
                let idx = shape_indices(u.len(), shape);
                let bf = encode_attestation_bitfield_vec(&idx, u.len());
                assert_eq!(
                    decode_attestation_bitfield_vec(&bf, u.len()),
                    idx,
                    "n={n} b={b} shape={shape}: identity"
                );
                assert!(
                    validate_attestation_bitfield_vec(&bf, u.len()),
                    "n={n} b={b} shape={shape}: must validate at its own width"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// O3 — golden bit-identity against the M0 committed store (read-only)
// ---------------------------------------------------------------------------

/// REQ-BLS-004 (Must) — Decision: a failure means the shared universe produces a different width or a different byte string than the pre-AH encoder did for the same epoch, so M4's switch would NOT be bit-identical below the activation height and every node that upgraded would fork.
#[test]
fn req_bls_004_golden_vectors_stay_bit_identical_under_the_shared_universe() {
    let doc = load_fixture();
    let vectors = doc["vectors"].as_array().expect("vectors array");
    assert_eq!(
        vectors.len(),
        66,
        "the M0 store is byte-frozen at 66 vectors"
    );

    for v in vectors {
        let id = v["id"].as_str().expect("id");
        let n = v["n"].as_u64().expect("n") as usize;
        let attested = as_usize_vec(v, "attested");
        let expected_hex = v["bitfield_hex"].as_str().expect("bitfield_hex");

        let mut splits = vec![n, n / 2, n / 3, 0];
        splits.sort_unstable();
        splits.dedup();

        for b in splits {
            let (base, active) = split(n, b);
            let u = attestation_universe(&base, &active);

            assert_eq!(u.len(), n, "{id} b={b}: universe width must equal n");
            assert_eq!(
                key_bytes(&u[..base.len()]),
                key_bytes(&base),
                "{id} b={b}: O3a base prefix"
            );
            assert_eq!(
                hex::encode(encode_attestation_bitfield_vec(&attested, u.len())),
                expected_hex,
                "{id} b={b}: bytes must be identical to the pre-AH encoder"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// O3d — the M0 divergence fixture resolves to ONE width
// ---------------------------------------------------------------------------

/// REQ-BLS-004 (Must) — Decision: a failure means the shared universe disagrees with the width the shipped encoder emits for the exact epoch M0 measured, so replacing the hand-rolled copies with it would change live block bytes.
#[test]
fn req_bls_004_m0_divergence_fixture_resolves_to_a_single_width_of_fifty() {
    let base: Vec<PublicKey> = (0..45u16).map(pk).collect();
    let active: Vec<PublicKey> = (0..45u16)
        .filter(|s| *s != 7)
        .chain(100..105u16)
        .map(pk)
        .collect();

    let u = attestation_universe(&base, &active);
    assert_eq!(u.len(), 50, "45 frozen base + 5 mid-epoch additions");
    assert_eq!(
        key_bytes(&u[..45]),
        key_bytes(&base),
        "O3a: the exited producer keeps its base index"
    );
    let mut expected: Vec<[u8; 32]> = (100..105u16).map(|s| *pk(s).as_bytes()).collect();
    expected.sort_unstable();
    assert_eq!(key_bytes(&u[45..]), expected, "O3b");
    assert_universe_contract(&base, &active);
}
