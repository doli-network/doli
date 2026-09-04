//! Canonical attestation-bitfield universe — REQ-BLS-004 (INC-I-178).
//!
//! Order is `[base | (active \ base) sorted by pubkey bytes]`; `base` is never
//! re-sorted. Every encoder/decoder pair must share this order (Full Bitfield
//! Decode pillar, v6.17.1) or indices misalign.
//!
//! No production caller in M3 by design: the encoder, the stray-bit validator,
//! `post_commit` and the rewards/schedule decoders adopt it in M4 behind
//! `inc_i_178_attestation_bls_activation_height` — unifying their five decode
//! widths is consensus-visible (REQ-BLS-014, INV-DEPLOY-001).

use crypto::PublicKey;
use std::collections::HashSet;

/// Duplicate-free: a key repeated in `base`, or in both slices, keeps only its
/// FIRST `base` position. The caller supplies `active`; no height, no storage.
pub fn attestation_universe(base: &[PublicKey], active: &[PublicKey]) -> Vec<PublicKey> {
    let cap = base.len() + active.len();
    let mut seen: HashSet<[u8; 32]> = HashSet::with_capacity(cap);
    let mut universe: Vec<PublicKey> = Vec::with_capacity(cap);
    for pk in base {
        if seen.insert(*pk.as_bytes()) {
            universe.push(*pk);
        }
    }
    let mut extra: Vec<PublicKey> = active
        .iter()
        .filter(|pk| seen.insert(*pk.as_bytes()))
        .copied()
        .collect();
    extra.sort_unstable_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    universe.extend(extra);
    universe
}
