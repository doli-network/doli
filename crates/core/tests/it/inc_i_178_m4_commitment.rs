//! INC-I-178 M4 — D6 `presence_commitment`: the post-AH `presence_root` preimage.
//!
//! Requirement: **REQ-BLS-003** (Must). TDD RED, EXPECTED: `doli_core::presence_commitment`
//! does not exist at the M4 branch point, so this module does not compile against HEAD.
//!
//! OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//!
//!   F1: `presence_commitment(bitfield: &[u8], aggregate: &[u8]) -> Hash`  (free fn, PURE)
//!       O1: return value — the 32-byte commitment
//!       O2: mutable params — NONE (both slices are shared references); asserted
//!           negatively by re-reading the inputs after the call
//!       O3: receiver/self — NONE (free function)
//!       O4: persistent store writes / statics / channels — NONE
//!       PATHS: the function is UNCONDITIONAL — one arm, no `is_empty()` guard
//!         (architecture D6). The partitions below are input shapes, not branches.
//!       INPUT PARTITIONS:
//!         I1 empty bitfield  x empty aggregate      (the canonical empty)
//!         I2 full bitfield   x 96-byte aggregate    (an attended post-AH block)
//!         I3 sparse bitfield x empty aggregate      (bits set, aggregate stripped)
//!         I4 empty bitfield  x 96-byte aggregate    (bits cleared, aggregate kept)
//!         I5 the concatenation-ambiguity pair: `(x, [])` vs `([], x)`
//!         I6 one byte of the aggregate flipped, at every one of the 96 positions
//!
//!   MATRIX 4 outputs x 6 input partitions: O1 claimed by a named test for every
//!   partition; O2/O3/O4 are structural and asserted once, in the I2 test.

use crypto::Hash;
use doli_core::presence_commitment;

/// The D6 preimage, recomputed here from the architecture text rather than from the
/// implementation: `len_le(bitfield) ‖ bitfield ‖ len_le(aggregate) ‖ aggregate`.
fn expected(bitfield: &[u8], aggregate: &[u8]) -> Hash {
    let mut pre = Vec::with_capacity(8 + bitfield.len() + aggregate.len());
    pre.extend_from_slice(&(bitfield.len() as u32).to_le_bytes());
    pre.extend_from_slice(bitfield);
    pre.extend_from_slice(&(aggregate.len() as u32).to_le_bytes());
    pre.extend_from_slice(aggregate);
    crypto::hash::hash(&pre)
}

fn full_bitfield(n_bytes: usize) -> Vec<u8> {
    vec![0xffu8; n_bytes]
}

fn sparse_bitfield(n_bytes: usize) -> Vec<u8> {
    (0..n_bytes)
        .map(|i| if i % 3 == 0 { 0x49 } else { 0x00 })
        .collect()
}

fn aggregate96() -> Vec<u8> {
    (0..96u16)
        .map(|i| (i.wrapping_mul(37) & 0xff) as u8)
        .collect()
}

// ===========================================================================
// O1 — the exact preimage, on every input shape.
// ===========================================================================

/// REQ-BLS-003 — Decision: a failure means the shipped preimage is not the one the
/// architecture specifies, so every upgraded node computes a `presence_root` that the
/// design document cannot predict — the commitment would be whatever the implementation
/// happened to concatenate, and no reviewer could check it against the spec.
#[test]
fn req_bls_003_m4_commitment_equals_the_length_prefixed_preimage_on_every_shape() {
    let cases: Vec<(&str, Vec<u8>, Vec<u8>)> = vec![
        ("I1 empty/empty", Vec::new(), Vec::new()),
        ("I2 full/96", full_bitfield(6), aggregate96()),
        ("I3 sparse/empty", sparse_bitfield(125), Vec::new()),
        ("I4 empty-bits/96", vec![0u8; 6], aggregate96()),
        ("I2 large/96", full_bitfield(125), aggregate96()),
    ];

    for (label, bf, agg) in &cases {
        assert_eq!(
            presence_commitment(bf, agg),
            expected(bf, agg),
            "{label}: presence_commitment must equal BLAKE3(len_le(bf) || bf || len_le(agg) || agg)"
        );
    }

    // O2/O3/O4: the function is pure. The inputs it was handed are unchanged, and
    // there is no receiver and no store for it to have written to.
    let (bf, agg) = (full_bitfield(6), aggregate96());
    let (bf_before, agg_before) = (bf.clone(), agg.clone());
    let _ = presence_commitment(&bf, &agg);
    assert_eq!(
        bf, bf_before,
        "O2: the bitfield argument must not be mutated"
    );
    assert_eq!(
        agg, agg_before,
        "O2: the aggregate argument must not be mutated"
    );
}

/// REQ-BLS-003 — Decision: a failure means the two fields are concatenated without an
/// unambiguous split (architecture C9), so a producer could move bytes from the bitfield
/// into the aggregate and keep the same `presence_root`. The commitment would then bind
/// the CONCATENATION, not the pair, and a relay could re-split the block into a different
/// bitfield that still validates.
#[test]
fn req_bls_003_m4_the_bitfield_aggregate_split_is_unambiguous() {
    assert_ne!(
        presence_commitment(&[0x01], &[]),
        presence_commitment(&[], &[0x01]),
        "C9: moving one byte across the split must change the commitment"
    );
    assert_ne!(
        presence_commitment(&[0x01, 0x02], &[]),
        presence_commitment(&[0x01], &[0x02]),
        "C9: the same total bytes, split differently, must not collide"
    );
    assert_ne!(
        presence_commitment(&full_bitfield(6), &aggregate96()),
        presence_commitment(&[full_bitfield(6), aggregate96()].concat(), &[]),
        "C9: an attended block must not collide with its own flattened preimage"
    );
}

/// REQ-BLS-003 — Decision: a failure means the empty case is a SENTINEL rather than a
/// hash. `Hash::ZERO` is what `post_commit.rs:27` and `rewards.rs` read as "this block
/// carries no attestation data at all", so an empty-bitfield post-AH block would skip
/// the decode path entirely and the commitment would stop distinguishing "no attesters"
/// from "aggregate stripped by a relay" — the exact hole D6 exists to close.
#[test]
fn req_bls_003_m4_the_empty_case_is_a_real_deterministic_hash_not_zero() {
    let empty = presence_commitment(&[], &[]);

    assert_ne!(
        empty,
        Hash::ZERO,
        "the canonical empty must be a real hash; Hash::ZERO is the legacy \
         'no attestation data' sentinel that gates the decoders"
    );
    assert_eq!(
        empty,
        expected(&[], &[]),
        "the canonical empty is BLAKE3 over the two length-prefixed empties"
    );
    assert_eq!(
        empty,
        presence_commitment(&[], &[]),
        "the function must be deterministic across calls"
    );

    // Anti-vacuity: the empty commitment must also differ from the other empty-ish
    // shapes, or "not ZERO" would be satisfied by any constant.
    assert_ne!(
        empty,
        presence_commitment(&[0u8], &[]),
        "an all-zero one-byte bitfield is a different block from no bitfield"
    );
    assert_ne!(
        empty,
        presence_commitment(&[], &[0u8; 96]),
        "an all-zero aggregate is a different block from no aggregate"
    );
}

/// REQ-BLS-003 — Decision: a failure means the post-AH preimage is still the legacy
/// `BLAKE3(bitfield)`. The whole gate would then be inert: an old binary and a new one
/// would agree above the activation height, the aggregate would remain uncommitted, and
/// a relay could strip it without detection while every test that only checks
/// "the roots match" stays green.
#[test]
fn req_bls_003_m4_the_post_ah_preimage_differs_from_the_legacy_blake3_of_the_bitfield() {
    // The EMPTY bitfield is the shape a zero-attester post-AH block carries, and the
    // one an `is_empty()` guard would send back to the legacy rule.
    assert_ne!(
        presence_commitment(&[], &[]),
        crypto::hash::hash(&[]),
        "the canonical empty must differ from the legacy preimage of an empty bitfield"
    );

    for bf in [full_bitfield(6), sparse_bitfield(125), vec![0u8; 1]] {
        assert_ne!(
            presence_commitment(&bf, &[]),
            crypto::hash::hash(&bf),
            "post-AH the preimage is length-prefixed; the legacy rule is BLAKE3(bitfield)"
        );
        assert_ne!(
            presence_commitment(&bf, &aggregate96()),
            crypto::hash::hash(&bf),
            "an attended block's commitment must not equal the legacy root either"
        );
    }
}

/// REQ-BLS-003 AC-1 — Decision: a failure means the aggregate is not actually bound by
/// the commitment, so a relay that mutates or replaces the 96-byte signature produces a
/// block whose `presence_root` still matches. Detecting that mutation is the ONLY reason
/// the aggregate is hashed at all (REQ-BLS-002 AC-3: a stripped aggregate does not fail
/// verification, it skips it).
#[test]
fn req_bls_003_m4_flipping_one_aggregate_byte_changes_the_commitment() {
    let bf = full_bitfield(6);
    let agg = aggregate96();
    let root = presence_commitment(&bf, &agg);

    let mut seen = std::collections::HashSet::new();
    seen.insert(root);

    for i in 0..agg.len() {
        let mut tampered = agg.clone();
        tampered[i] ^= 0x01;
        let mutated = presence_commitment(&bf, &tampered);
        assert_ne!(
            mutated, root,
            "flipping aggregate byte {i} must change the commitment"
        );
        assert!(
            seen.insert(mutated),
            "byte {i} produced a commitment already seen — a collision inside 96 \
             single-byte mutations means the aggregate is not fully covered"
        );
    }
    assert_eq!(seen.len(), agg.len() + 1, "96 mutations + the original");

    // The stripped case is the one the commitment exists to catch: an empty aggregate
    // over the same bitfield must be a different block.
    assert_ne!(
        presence_commitment(&bf, &[]),
        root,
        "AC-1: stripping the aggregate entirely must change the commitment"
    );
}
