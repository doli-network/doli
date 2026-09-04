//! INC-I-178 D5/D6 — the gated attestation universe and the block commitment.
//!
//! Each `*_at` function takes the activation height directly and is what the
//! three consensus sites call; the `&NetworkParams` overloads read the same
//! value out of shipped params and are dead in the binary's own module tree
//! (`main.rs` re-declares `mod node`), hence the per-item allow.
//! Below the gate every arm is the expression its call site used before M4.

use std::collections::HashSet;

use crypto::{BlsSignature, Hash, PublicKey};
use doli_core::attestation::ParentSignaturePool;
use doli_core::network_params::NetworkParams;
use doli_core::{attestation_universe, encode_attestation_bitfield_vec, presence_commitment};

/// What the builder commits to: the body bitfield, the aggregate, and the
/// `presence_root` that binds them.
#[derive(Debug, Clone)]
pub struct AttestationCommitment {
    pub bitfield: Vec<u8>,
    pub aggregate: Vec<u8>,
    pub presence_root: Hash,
}

#[allow(dead_code)]
pub fn attestation_bls_active(params: &NetworkParams, height: u64) -> bool {
    height >= params.inc_i_178_attestation_bls_activation_height
}

/// `[base | (active \ base) sorted by pubkey bytes]` — `assembly.rs` and
/// `post_commit.rs` each hand-rolled this before M4.
fn legacy_universe(base: &[PublicKey], active: &[PublicKey]) -> Vec<PublicKey> {
    let base_set: HashSet<&PublicKey> = base.iter().collect();
    let mut extra: Vec<PublicKey> = active
        .iter()
        .filter(|pk| !base_set.contains(pk))
        .copied()
        .collect();
    extra.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    let mut universe = base.to_vec();
    universe.extend(extra);
    universe
}

pub fn encoder_universe_at(
    ah: u64,
    height: u64,
    base: &[PublicKey],
    active: &[PublicKey],
) -> Vec<PublicKey> {
    if height >= ah {
        attestation_universe(base, active)
    } else {
        legacy_universe(base, active)
    }
}

#[allow(dead_code)]
pub fn encoder_universe(
    params: &NetworkParams,
    height: u64,
    base: &[PublicKey],
    active: &[PublicKey],
) -> Vec<PublicKey> {
    encoder_universe_at(
        params.inc_i_178_attestation_bls_activation_height,
        height,
        base,
        active,
    )
}

pub fn post_commit_universe_at(
    ah: u64,
    height: u64,
    base: &[PublicKey],
    active: &[PublicKey],
) -> Vec<PublicKey> {
    encoder_universe_at(ah, height, base, active)
}

#[allow(dead_code)]
pub fn post_commit_universe(
    params: &NetworkParams,
    height: u64,
    base: &[PublicKey],
    active: &[PublicKey],
) -> Vec<PublicKey> {
    encoder_universe(params, height, base, active)
}

/// The denominator `validate_attestation_bitfield_vec` gets. Pre-AH this is the
/// validator's own third, narrower universe: `active_at(h).len()`.
pub fn stray_bit_universe_width_at(
    ah: u64,
    height: u64,
    base: &[PublicKey],
    active: &[PublicKey],
) -> usize {
    if height >= ah {
        attestation_universe(base, active).len()
    } else {
        active.len()
    }
}

#[allow(dead_code)]
pub fn stray_bit_universe_width(
    params: &NetworkParams,
    height: u64,
    base: &[PublicKey],
    active: &[PublicKey],
) -> usize {
    stray_bit_universe_width_at(
        params.inc_i_178_attestation_bls_activation_height,
        height,
        base,
        active,
    )
}

/// The `presence_root` preimage for a block that already carries its body.
/// Post-AH the D6 commitment, unconditionally (C9): no producer-controlled guard.
pub fn block_presence_root_at(ah: u64, height: u64, bitfield: &[u8], aggregate: &[u8]) -> Hash {
    if height >= ah {
        presence_commitment(bitfield, aggregate)
    } else {
        crypto::hash::hash(bitfield)
    }
}

/// Post-AH a zero-attester block carries the canonical empty commitment, not
/// `Hash::ZERO`: an empty bitfield under that root is complete attendance.
pub fn is_canonical_empty_attendance_at(
    ah: u64,
    height: u64,
    presence_root: &Hash,
    bitfield: &[u8],
) -> bool {
    height >= ah && bitfield.is_empty() && *presence_root == presence_commitment(&[], &[])
}

fn empty_commitment() -> AttestationCommitment {
    AttestationCommitment {
        bitfield: Vec::new(),
        aggregate: Vec::new(),
        presence_root: presence_commitment(&[], &[]),
    }
}

/// Post-AH: bit `i` is set iff `universe[i]` has a pooled signature over
/// `parent`, and the aggregate covers exactly those signatures in index order.
/// Zero pooled signatures yields the canonical empty commitment — `bls_aggregate`
/// rejects an empty set, and a producer that holds none must still build
/// (REQ-BLS-010).
fn pooled_commitment(
    universe: &[PublicKey],
    pool: &ParentSignaturePool,
    parent: &Hash,
) -> AttestationCommitment {
    let Some(signed) = pool.signatures_for(parent) else {
        return empty_commitment();
    };
    let mut indices: Vec<usize> = Vec::new();
    let mut parts: Vec<BlsSignature> = Vec::new();
    for (i, pk) in universe.iter().enumerate() {
        if let Some(raw) = signed.get(pk) {
            if let Ok(sig) = BlsSignature::try_from_slice(raw) {
                indices.push(i);
                parts.push(sig);
            }
        }
    }
    if parts.is_empty() {
        return empty_commitment();
    }
    let Ok(aggregate) = crypto::bls_aggregate(&parts) else {
        return empty_commitment();
    };
    let bitfield = encode_attestation_bitfield_vec(&indices, universe.len());
    let aggregate = aggregate.as_bytes().to_vec();
    let presence_root = presence_commitment(&bitfield, &aggregate);
    AttestationCommitment {
        bitfield,
        aggregate,
        presence_root,
    }
}

/// Pre-AH: bits come from the minute-attendance set, the root is
/// `BLAKE3(bitfield)`, and an empty attester set keeps the `Hash::ZERO` sentinel.
fn legacy_commitment(universe: &[PublicKey], attested: &[PublicKey]) -> AttestationCommitment {
    if attested.is_empty() {
        return AttestationCommitment {
            bitfield: Vec::new(),
            aggregate: Vec::new(),
            presence_root: Hash::ZERO,
        };
    }
    let indices: Vec<usize> = attested
        .iter()
        .filter_map(|pk| universe.iter().position(|p| p == pk))
        .collect();
    let bitfield = encode_attestation_bitfield_vec(&indices, universe.len());
    let presence_root = crypto::hash::hash(&bitfield);
    AttestationCommitment {
        bitfield,
        aggregate: Vec::new(),
        presence_root,
    }
}

pub fn build_attestation_commitment_at(
    ah: u64,
    height: u64,
    universe: &[PublicKey],
    attested: &[PublicKey],
    pool: &ParentSignaturePool,
    parent: &Hash,
) -> AttestationCommitment {
    if height >= ah {
        pooled_commitment(universe, pool, parent)
    } else {
        legacy_commitment(universe, attested)
    }
}

#[allow(dead_code)]
pub fn build_attestation_commitment(
    params: &NetworkParams,
    height: u64,
    universe: &[PublicKey],
    attested: &[PublicKey],
    pool: &ParentSignaturePool,
    parent: &Hash,
) -> AttestationCommitment {
    build_attestation_commitment_at(
        params.inc_i_178_attestation_bls_activation_height,
        height,
        universe,
        attested,
        pool,
        parent,
    )
}
