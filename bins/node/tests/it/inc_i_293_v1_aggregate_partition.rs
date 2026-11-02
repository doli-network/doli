//! INC-I-293 V-1: proof that 73e709be (U-1) left block validation bit-identical.
//! U-1 moved the "no bit set -> the aggregate is empty" rule out of
//! `decide_attestation` (verify.rs, ungated block validation above INC-I-178)
//! into `doli_core::aggregate_fits_bitfield`. `old_rule` below is the pre-U-1
//! `decide_attestation` (the removed lines verbatim) behind the unchanged height
//! arm of `verify_block_attestation`. The live node is the new rule.
//!
//! `decide_attestation` and `verify_block_attestation` are `pub(crate)`, so the new
//! side is driven through their one public caller, `Node::validate_block_for_apply`
//! (Full mode), which returns the reason label in `AttestationVerifyFailed`.
//!
// OUTPUT CONTRACT:
//   F1: fn Node::validate_block_for_apply(&Block, height, Full) -> Result<(), ValidationError>
//     O1: return value — Ok(()) or AttestationVerifyFailed { reason } (outcome AND
//         reason label); any other error fails the cell (it did not reach the rule)
//     O2: mutable params — NONE (&Block); receiver — &self, NONE
//   F2: fn doli_core::aggregate_fits_bitfield(&[u8], &[u8]) -> bool
//     O3: return value — false exactly when the removed expression rejects
// PATHS: height arm (below AH: verifier inert) · no-bit arm (removed lines) ·
//   bits-set fall-through to the unchanged BLS tail
// INPUT PARTITIONS: aggregate {E empty, N non-empty} × bitfield {0 all-zero, B
//   non-zero} × height {below AH, at AH}; all-zero has two inputs: zero-length
//   and honest-width zero bytes
// MATRIX: E0-below E0-at · N0-below N0-at · EB-below EB-at · NB-below NB-at — F1 O1 per cell, F2 O3 per (aggregate, bitfield) input; O2 N/A
//
//! covers: verify.rs commitment.rs

use crypto::{BlsPublicKey, BlsSignature, Hash, PublicKey};
use doli_core::attestation::bls_attest_msg;
use doli_core::validation::ValidationError;
use doli_core::{aggregate_fits_bitfield, presence_root_at, presence_root_matches, Block};

use crate::inc_i_178_m0_common::N_SMALL;
use crate::inc_i_178_m5_common::{
    post_ah_fixture, Fixture, REASON_AGGREGATE_INVALID,
    REASON_AGGREGATE_NONEMPTY_FOR_EMPTY_BITFIELD, REASON_MISSING_BLS_KEY, REASON_ROOT_MISMATCH,
};

const N_SIGNERS: usize = 8;

/// (name, aggregate, bitfield inputs, expected outcome at the AH).
type Cell<'a> = (&'a str, &'a Vec<u8>, &'a [Vec<u8>], Result<(), &'a str>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Accepted,
    SkippedLight,
}

/// `decide_attestation` as it stood before 73e709be (verbatim body).
fn old_decide(
    root_matches: bool,
    bitfield: &[u8],
    aggregate: &[u8],
    parent_hash: &Hash,
    universe_divergent: bool,
    set_bit_keys: Result<Vec<BlsPublicKey>, PublicKey>,
) -> Result<Outcome, &'static str> {
    if !root_matches {
        return Err(REASON_ROOT_MISMATCH);
    }
    if universe_divergent {
        return Ok(Outcome::SkippedLight);
    }
    // The lines 73e709be removed, verbatim:
    if !bitfield.iter().any(|b| *b != 0) {
        return if aggregate.is_empty() {
            Ok(Outcome::Accepted)
        } else {
            Err(REASON_AGGREGATE_NONEMPTY_FOR_EMPTY_BITFIELD)
        };
    }
    let keys = set_bit_keys.map_err(|_| REASON_MISSING_BLS_KEY)?;
    let Ok(sig) = BlsSignature::try_from_slice(aggregate) else {
        return Err(REASON_AGGREGATE_INVALID);
    };
    crypto::bls_verify_aggregate(&bls_attest_msg(parent_hash), &sig, &keys)
        .map(|()| Outcome::Accepted)
        .map_err(|_| REASON_AGGREGATE_INVALID)
}

/// The old rule on one block: the unchanged height arm of
/// `verify_block_attestation` (`height < ah` -> Ok), then `old_decide` with the
/// same inputs the node passes in Full mode (no snap sync: not divergent).
fn old_rule(block: &Block, height: u64, ah: u64, keys: &[BlsPublicKey]) -> Result<(), String> {
    if height < ah {
        return Ok(());
    }
    let (bf, agg) = (&block.attestation_bitfield, &block.aggregate_bls_signature);
    let root = presence_root_matches(ah, Some(height), &block.header.presence_root, bf, agg);
    old_decide(
        root,
        bf,
        agg,
        &block.header.prev_hash,
        false,
        Ok(keys.to_vec()),
    )
    .map(|_| ())
    .map_err(str::to_string)
}

/// The new rule: the live node. Only an attestation-verifier reject is a valid
/// answer; any other error means the cell never reached the rule under test.
async fn new_rule(f: &Fixture, block: &Block, cell: &str) -> Result<(), String> {
    match f.validate_full(block).await {
        Ok(()) => Ok(()),
        Err(ValidationError::AttestationVerifyFailed { reason, .. }) => Err(reason),
        Err(other) => panic!("{cell}: rejected before the attestation rule: {other:?}"),
    }
}

/// The fixture block with this body, its header recommitted at `height` under `ah`.
fn with_body(f: &Fixture, ah: u64, bitfield: Vec<u8>, aggregate: Vec<u8>) -> Block {
    let mut b = f.block.clone();
    b.attestation_bitfield = bitfield;
    b.aggregate_bls_signature = aggregate;
    b.header.presence_root = presence_root_at(
        ah,
        f.height,
        &b.attestation_bitfield,
        &b.aggregate_bls_signature,
    );
    crate::block_sig::resign(&mut b.header);
    b
}

// INC-I-293 V-1 — Decision: a failure means 73e709be changed the outcome or the reason label of ungated block validation for some aggregate × bitfield × height cell, so U-1 was not a pure refactor and needs a gate.
#[tokio::test]
async fn v1_inc_i_293_aggregate_fits_bitfield_matches_the_old_verify_rule_in_all_8_cells() {
    let mut f = post_ah_fixture(N_SMALL, N_SIGNERS).await;
    let (honest_bf, honest_agg) = (
        f.block.attestation_bitfield.clone(),
        f.block.aggregate_bls_signature.clone(),
    );
    let set: u32 = honest_bf.iter().map(|b| b.count_ones()).sum();
    assert_eq!(set as usize, N_SIGNERS, "precondition: one bit per signer");
    assert!(!honest_agg.is_empty(), "precondition: honest aggregate");
    let keys: Vec<BlsPublicKey> = f.signers.iter().map(|s| *s.bls.public_key()).collect();
    let zero_bfs = [Vec::new(), vec![0u8; honest_bf.len()]];

    // (cell, aggregate, bitfields, expected at the AH); below the AH every cell is Ok.
    let cells: [Cell; 4] = [
        ("E0", &Vec::new(), &zero_bfs, Ok(())),
        (
            "N0",
            &honest_agg,
            &zero_bfs,
            Err(REASON_AGGREGATE_NONEMPTY_FOR_EMPTY_BITFIELD),
        ),
        (
            "EB",
            &Vec::new(),
            std::slice::from_ref(&honest_bf),
            Err(REASON_AGGREGATE_INVALID),
        ),
        ("NB", &honest_agg, std::slice::from_ref(&honest_bf), Ok(())),
    ];
    let height = f.height;
    let at_ah = f.ah();
    assert_eq!(at_ah, height, "precondition: the fixture sits at the AH");

    for (arm, ah) in [("below", height + 1), ("at", at_ah)] {
        f.node.inc_i_178_attestation_bls_activation_height = ah;
        for (name, agg, bfs, expected_at) in &cells {
            for bf in bfs.iter() {
                let cell = format!("{name}-{arm} (bitfield len {})", bf.len());
                let block = with_body(&f, ah, bf.clone(), (*agg).clone());
                let old = old_rule(&block, height, ah, &keys);
                let new = new_rule(&f, &block, &cell).await;
                assert_eq!(
                    new, old,
                    "{cell}: new outcome/reason differs from the old rule"
                );
                let expected = if ah > height {
                    Ok(())
                } else {
                    expected_at.map_err(str::to_string)
                };
                assert_eq!(new, expected, "{cell}: outcome/reason");
                assert_eq!(
                    aggregate_fits_bitfield(bf, agg),
                    old_decide(true, bf, agg, &Hash::ZERO, false, Ok(keys.clone()))
                        != Err(REASON_AGGREGATE_NONEMPTY_FOR_EMPTY_BITFIELD),
                    "{cell}: aggregate_fits_bitfield differs from the removed expression"
                );
            }
        }
    }
}
