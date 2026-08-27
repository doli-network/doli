//! INC-I-173 M3 / F6 — the chain-derived maintainer-set digest (AUDIT-P1-003).
//!
//! An operator must be able to ask two nodes "do we hold the same
//! release-verification trust root?" and get a single comparable scalar,
//! without shipping the member list around.
//!
//! This is a LEAF module by construction. The genesis hash arrives as a plain
//! byte slice — the idiom already used for `activation_height: u64` in
//! [`super::MaintainerSet::verify_multisig_at`] — so `crates::maintainer` gains
//! NO dependency edge toward `chainspec` or toward node-local maintainer state.

use crypto::Hasher;

use super::MaintainerSet;

/// Domain-separation tag for the maintainer-set digest preimage.
///
/// Without it the digest is an undifferentiated BLAKE3 over concatenated
/// fields, and nothing stops it colliding with another hash of the same shape.
const MAINTAINER_SET_DIGEST_DOMAIN: &[u8] = b"DOLI-MAINTAINER-SET-V1";

/// A single comparable scalar identifying a maintainer set on a given chain.
///
/// The digest covers EXACTLY the release-verification trust root: the domain
/// tag, the chain, the threshold and the members. Nothing else. Two nodes whose
/// digests match accept exactly the same release signatures.
///
/// Preimage:
///
/// ```text
/// BLAKE3_256( b"DOLI-MAINTAINER-SET-V1"
///           || genesis_hash
///           || (set.threshold as u64).to_le_bytes()
///           || concat(member pubkey bytes, ASCENDING by raw bytes) )
/// ```
///
/// # Why members are SORTED
///
/// Member order is not a stable property of the set: below
/// `maintainer_derivation_activation_height` the bootstrap derivation
/// stable-sorts a `HashMap` iteration with no pubkey tiebreak (AUDIT-P3-014),
/// so two honest nodes can legitimately hold the same five keys in different
/// insertion order. The digest answers "do we hold the same trust root", not
/// "did we build it the same way", so a false mismatch there would make the
/// instrument useless.
///
/// # Why `last_updated` is EXCLUDED
///
/// Same reason as the sort, but MEASURED rather than inferred. `last_updated` is
/// NODE-LOCAL: it lives in `maintainer_state.bin`, outside the state root. The
/// M3 security audit measured it divergent across the live testnet fleet at an
/// IDENTICAL tip (`docs/.workflow/chain-state.md:36-39`: RPC 8512 reported
/// `last_change_block = 88289` while 12 peers reported `1`, all at tip 134,682,
/// all holding the same five members and the same threshold).
///
/// Those 13 nodes accept exactly the same release signatures —
/// [`super::MaintainerSet::verify_multisig`] consults the members and the
/// threshold and never reads `last_updated` — so binding it would make this
/// scalar report a MISMATCH for a fleet that is aligned on the only property the
/// digest claims to compare. That is the same false-signal failure the sorted
/// member list exists to prevent, reintroduced through a different term, and a
/// divergence instrument that cries wolf is worse than none.
///
/// Nothing is lost: `last_updated` is published separately as
/// `last_change_block` on the same `getMaintainerSet` response and in the
/// apply-side `[MAINTAINER] MAINTAINER_SET_DIGEST=` log line, so an operator who
/// wants the history term still has it, unmixed.
///
/// Pinned by `audit_p1_003_digest_is_independent_of_last_updated`
/// (`crates/core/tests/inc_i_173_m3_maintainer_digest.rs`).
///
/// # Why the genesis hash is included
///
/// The mainnet and testnet bootstrap key arrays have been byte-identical
/// (AUDIT-P1-016), so without the chain binding the same member list on two
/// different networks would digest the same.
pub fn maintainer_set_digest(set: &MaintainerSet, genesis_hash: &[u8]) -> [u8; 32] {
    let mut sorted: Vec<[u8; 32]> = set.members.iter().map(|m| *m.as_bytes()).collect();
    sorted.sort();

    let mut hasher = Hasher::new();
    hasher.update(MAINTAINER_SET_DIGEST_DOMAIN);
    hasher.update(genesis_hash);
    hasher.update(&(set.threshold as u64).to_le_bytes());
    for member in &sorted {
        hasher.update(member);
    }
    *hasher.finalize().as_bytes()
}
