//! INC-I-178 M4 — REQ-BLS-003 / REQ-BLS-005: the post-AH `presence_root` preimage
//! switch is UNCONDITIONAL above the activation height.
//!
//! OUTPUT CONTRACT
//!
//! F1: `commit::block_presence_root_at(ah, height, bitfield, aggregate) -> Hash`
//!   O1 return value — the root a validator expects for this body at this height
//!   O2 mutable params — NONE (both slices shared); asserted negatively
//!   O3 receiver/self — NONE (free fn)   O4 store/statics/channels — NONE
//!   PATHS: P-pre `height < ah` -> `BLAKE3(bitfield)`
//!          P-post `height >= ah` -> `presence_commitment(bitfield, aggregate)`,
//!            with NO further condition — nothing the producer chooses selects
//!            the arm
//!   INPUT PARTITIONS (crossed with both paths):
//!     I1 empty bitfield  x empty aggregate
//!     I2 non-empty bitfield x empty aggregate      (the producer-controlled case)
//!     I3 non-empty bitfield x 96-byte aggregate
//!     I4 non-empty bitfield x 1-byte aggregate     (a truncated aggregate)
//!
//! F2: `commit::build_attestation_commitment_at(..) -> AttestationCommitment`
//!   O1 `.bitfield`  O2 `.aggregate`  O3 `.presence_root`
//!   PATH P-post with an EMPTY pool: all three are the canonical empty.
//!
//! F3: `Node::validate_block_for_apply(&self, &Block, height, mode)`
//!   O1 return value — the root mismatch a post-AH legacy-preimage block earns
//!   O2 the block is not mutated   O3 no node state change
//!
//! MATRIX: F1 O1 claimed per path x partition below; F1 O2 asserted once; F2 O1-O3
//!   on P-post/I1; F3 O1-O3 on P-post/I2.

use crypto::Hash;
use doli_core::attestation::ParentSignaturePool;
use doli_core::presence_commitment;
use doli_core::validation::ValidationMode;
use doli_node::node::attestation::commit::{
    block_presence_root_at, build_attestation_commitment_at,
};

use crate::inc_i_178_m0_common::{
    active_at, assemble, build_via_production, err_text, make_node, record_attesters,
    safe_build_height, test_universe, unix_now, N_SMALL,
};

/// A probe gate with room for a strictly-below height on both sides.
const AH: u64 = 1_000;
const PRE: u64 = AH - 1;

fn bitfield() -> Vec<u8> {
    vec![0b1010_1101, 0x00, 0xff, 0x41]
}

fn aggregate96() -> Vec<u8> {
    (0..96u16)
        .map(|i| (i.wrapping_mul(37) & 0xff) as u8)
        .collect()
}

// ===========================================================================
// F1 x P-post x I1 — the canonical empty, from both ends.
// ===========================================================================

/// REQ-BLS-003 — Decision: a failure means a post-AH block with no attesters commits
/// to the legacy `BLAKE3([])` or to the `Hash::ZERO` sentinel, so the builder and the
/// validator disagree about a block every producer can emit on its first post-AH slot,
/// and the two halves of the fleet fork on it.
#[test]
fn req_bls_003_m4_post_ah_the_empty_body_root_is_the_canonical_commitment() {
    let canonical = presence_commitment(&[], &[]);
    let got = block_presence_root_at(AH, AH, &[], &[]);

    assert_eq!(
        got, canonical,
        "post-AH an empty bitfield with an empty aggregate commits to \
         presence_commitment([], [])"
    );
    assert_ne!(
        got,
        crypto::hash::hash(&[]),
        "the legacy preimage of an empty bitfield must NOT be reachable above the gate"
    );
    assert_ne!(
        got,
        Hash::ZERO,
        "the canonical empty is a real hash, never the no-attestation-data sentinel"
    );

    // F2 — the builder end of the same value, so builder and validator are pinned
    // to ONE constant rather than to each other.
    let built =
        build_attestation_commitment_at(AH, AH, &[], &[], &ParentSignaturePool::new(), &Hash::ZERO);
    assert!(
        built.bitfield.is_empty(),
        "F2 O1: empty pool -> empty bitfield"
    );
    assert!(
        built.aggregate.is_empty(),
        "F2 O2: empty pool -> empty aggregate"
    );
    assert_eq!(
        built.presence_root, canonical,
        "F2 O3: the builder's zero-pooled root is the value the validator expects"
    );
}

// ===========================================================================
// F1 x P-post x I2 — the producer-controlled case.
// ===========================================================================

/// REQ-BLS-003 — Decision: a failure means an empty aggregate still selects the legacy
/// preimage above the activation height. Every field of that choice is producer-side, so
/// each producer could pick which of two commitment rules its own block is judged under —
/// the aggregate would be unbound whenever it is absent, and the gate would be advisory.
#[test]
fn req_bls_003_m4_post_ah_an_empty_aggregate_does_not_buy_the_legacy_preimage() {
    let bf = bitfield();
    let got = block_presence_root_at(AH, AH, &bf, &[]);

    assert_eq!(
        got,
        presence_commitment(&bf, &[]),
        "post-AH a non-empty bitfield with an empty aggregate still commits to the \
         length-prefixed pair"
    );
    assert_ne!(
        got,
        crypto::hash::hash(&bf),
        "an empty aggregate must not select BLAKE3(bitfield) above the gate"
    );

    // O2: the inputs are unchanged by the call.
    assert_eq!(bf, bitfield(), "F1 O2: the bitfield slice is not mutated");
}

/// REQ-BLS-003 — Decision: a failure means the root a validator expects moves with a
/// value the producer picks, so two honest nodes handed the same block at the same
/// height can compute different expectations.
#[test]
fn req_bls_003_m4_post_ah_every_aggregate_shape_uses_the_same_commitment_rule() {
    let bf = bitfield();
    let shapes: [(&str, Vec<u8>); 3] = [
        ("empty", Vec::new()),
        ("96-byte", aggregate96()),
        ("1-byte", vec![0x7fu8]),
    ];

    let mut roots: Vec<Hash> = Vec::new();
    for (label, agg) in &shapes {
        let got = block_presence_root_at(AH, AH, &bf, agg);
        assert_eq!(
            got,
            presence_commitment(&bf, agg),
            "{label}: the post-AH arm is unconditional"
        );
        assert_ne!(
            got,
            crypto::hash::hash(&bf),
            "{label}: no aggregate shape reaches the legacy preimage above the gate"
        );
        roots.push(got);
    }

    assert_ne!(roots[0], roots[1], "empty and 96-byte must not collide");
    assert_ne!(roots[0], roots[2], "empty and 1-byte must not collide");
    assert_ne!(roots[1], roots[2], "96-byte and 1-byte must not collide");
}

/// REQ-BLS-003 — Decision: a failure means a producer that ships the legacy root above
/// the gate is indistinguishable from an honest one, so the aggregate can be stripped in
/// flight without the commitment noticing. The rejection itself is enforced by the M5
/// verifier; this pins only that the expected value differs from what the block carries.
#[test]
fn req_bls_003_m4_post_ah_a_legacy_root_does_not_match_the_expected_commitment() {
    let bf = bitfield();
    let agg = aggregate96();
    let carried = crypto::hash::hash(&bf);

    assert_ne!(
        block_presence_root_at(AH, AH, &bf, &agg),
        carried,
        "a post-AH block carrying BLAKE3(bitfield) is a root mismatch"
    );
}

// ===========================================================================
// F1 x P-pre — byte-identity below the gate (rolling-deploy safety).
// ===========================================================================

/// REQ-BLS-005 — Decision: a failure means M4 changed the root rule for blocks BELOW the
/// activation height, so an upgraded node and a not-yet-upgraded node disagree about
/// already-final history and the rolling deploy forks before the gate is ever reached.
#[test]
fn req_bls_005_m4_below_the_gate_the_root_is_still_blake3_of_the_bitfield() {
    let bf = bitfield();

    assert_eq!(
        block_presence_root_at(AH, PRE, &bf, &[]),
        crypto::hash::hash(&bf),
        "pre-AH with an empty aggregate: BLAKE3(bitfield), byte-identical to the old binary"
    );
    assert_eq!(
        block_presence_root_at(AH, PRE, &bf, &aggregate96()),
        crypto::hash::hash(&bf),
        "pre-AH a carried aggregate is NOT part of the preimage"
    );
    assert_eq!(
        block_presence_root_at(AH, PRE, &[], &[]),
        crypto::hash::hash(&[]),
        "pre-AH the empty body keeps the legacy empty root"
    );
    assert_ne!(
        block_presence_root_at(AH, PRE, &[], &[]),
        presence_commitment(&[], &[]),
        "the two rules must be distinguishable, or the pre-AH assertions are vacuous"
    );
}

// ===========================================================================
// F3 — the same switch through the REAL node validator.
// ===========================================================================

/// REQ-BLS-003 (Must) — Decision: a failure means an honest post-AH block whose
/// producer holds no BLS signatures is judged under the legacy rule at the validator
/// while the builder commits under the new one. That block is either accepted with an
/// unbound aggregate or rejected outright, and the producer cannot tell which.
#[tokio::test]
async fn req_bls_003_m4_the_validator_expects_the_commitment_for_an_empty_aggregate() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let height = safe_build_height(&node);
    node.inc_i_178_attestation_bls_activation_height = height + 1;

    let all: Vec<crypto::PublicKey> = producers.iter().map(|k| *k.public_key()).collect();
    let slot = node.params.timestamp_to_slot(unix_now());
    record_attesters(&mut node, slot, &all);
    let (header, txs, bf) = build_via_production(&mut node, height).await;
    assert!(
        !bf.is_empty(),
        "the fixture needs a non-empty body, or the case under test is not reached"
    );

    let active = active_at(&node, height).await;
    let universe = test_universe(&node.epoch_state.producer_list, &active);
    assert_eq!(bf.len(), universe.len().div_ceil(8), "body width sanity");

    let legacy_block = assemble(header, txs, bf.clone());
    assert!(
        legacy_block.aggregate_bls_signature.is_empty(),
        "the case under test IS the empty aggregate"
    );
    assert_eq!(
        legacy_block.header.presence_root,
        crypto::hash::hash(&bf),
        "built below the gate, so the block carries the legacy root"
    );

    // Move ONLY the gate. The block, the height and the node are unchanged.
    node.inc_i_178_attestation_bls_activation_height = height;
    let verdict = node
        .validate_block_for_apply(&legacy_block, height, ValidationMode::Light)
        .await;
    assert!(
        err_text(&verdict).contains("presence_root mismatch"),
        "above the gate the legacy root must not validate against an empty aggregate; \
         got {:?}",
        err_text(&verdict)
    );

    // The same body re-committed under the post-AH rule validates.
    let mut post_block = legacy_block.clone();
    post_block.header.presence_root = presence_commitment(&bf, &post_block.aggregate_bls_signature);
    let verdict = node
        .validate_block_for_apply(&post_block, height, ValidationMode::Light)
        .await;
    assert!(
        verdict.is_ok(),
        "the post-AH commitment over the same honest body must validate; got {:?}",
        err_text(&verdict)
    );

    // O2/O3: validation is read-only on both verdicts.
    assert_eq!(
        post_block.attestation_bitfield, bf,
        "O2: bitfield untouched"
    );
    assert_eq!(
        node.epoch_state.producer_list.len(),
        N_SMALL,
        "O3: no epoch-list rotation during validation"
    );
}
