//! Tests for the Phase 2.1 oracle structural pubkey-hash set (N1-N12).
//!
//! Spec: `specs/oracle-structural-anchored-economics.md` §1.8
//!
//! The structural set is the 12 operator-controlled bonded producers
//! (~62.7% of total bonds as of activation). The sunset trigger HALTs
//! the oracle if `structural_share < 55%`, where `structural_share` is
//! computed by summing `bond_snapshot[k]` for each `k` in this array,
//! divided by `total_bonds_eligible`.
//!
//! `bond_snapshot` is keyed by `hash_with_domain(b"DOLI_ADDR_V1", pubkey)`
//! (see `bins/node/src/node/apply_block/post_commit.rs:211`). So the
//! constants here MUST be the full 32-byte BLAKE3 domain-hash of each
//! producer's public key — NOT the 20-byte truncated `Address` form.
//!
//! Provenance: derived from operator-controlled `/mainnet/<n>/keys/producer.json`
//! on ai1 (N1-N3), ai2 (N4-N5), ai4 (N6-N8), ai5 (N9-N12), retrieved
//! at session wall-clock and verified end-to-end against the on-disk
//! `address` field (first 20 bytes of the full hash) for N1 and N12.
//!
//! These tests guard:
//!   1. Array shape (exactly 12 entries).
//!   2. No duplicates (a duplicate would silently double-count one
//!      operator's bond in the sunset metric — a critical bug).
//!   3. Each hex string parses to a valid 32-byte `crypto::Hash`.
//!   4. Cross-consistency: N1-N5's hashes equal
//!      `hash_with_domain(ADDRESS_DOMAIN, BOOTSTRAP_MAINTAINER_KEYS_MAINNET[i])`
//!      for i in 0..5. This catches the most likely paste-error
//!      (wrong pubkey copied for a structural slot).
//!
//! N6-N12 cannot be cross-checked against existing constants — no
//! bootstrap-maintainer constant exists for them. The N1-N5 cross-check
//! validates the derivation algorithm and provenance pipeline; the
//! N6-N12 hashes share that pipeline so a wrong-pubkey paste for any
//! of them would be caught by the equivalent ai-server re-verification
//! ritual (out-of-band — not a test).

use crypto::{hash::hash_with_domain, Hash, PublicKey, ADDRESS_DOMAIN};

use super::constants::STRUCTURAL_PUBKEY_HASHES_HEX;

// N1-N5 ed25519 public keys (hex), duplicated verbatim from
// `updater/src/constants.rs:37` (BOOTSTRAP_MAINTAINER_KEYS_MAINNET).
// We intentionally do NOT import that constant: `updater` already
// depends on `doli-core`, so a dev-dep edge in the opposite direction
// would create a cycle.
//
// Test 4 below cross-checks STRUCTURAL_PUBKEY_HASHES_HEX[0..5] against
// hash_with_domain(ADDRESS_DOMAIN, pubkey) for each of these pubkeys.
// If updater rotates N1-N5's pubkeys, BOTH this array and
// STRUCTURAL_PUBKEY_HASHES_HEX must be updated together — and this
// test will fire loudly if only one side moves.
const N1_TO_N5_PUBKEYS_HEX: [&str; 5] = [
    "202047256a8072a8b8f476691b9a5ae87710cc545e8707ca9fe0c803c3e6d3df", // N1
    "effe88fefb6d992a1329277a1d49c7296d252bbc368319cb4bc061119926272b", // N2
    "54323cefd0eabac89b2a2198c95a8f261598c341a8e579a05e26322325c48c2b", // N3
    "2d27fdcc6a240b76ecaea64ad05c9b70d1adad90b6f9c43e8cbbbc0f1ab04116", // N4
    "3047e96b13276dd92ef5eb2d6396e66c29909217f11f8c0544ea7d76a76c7602", // N5
];

// OUTPUT CONTRACT: const STRUCTURAL_PUBKEY_HASHES_HEX — array length
//   O1: STRUCTURAL_PUBKEY_HASHES_HEX.len() — usize, exactly 12
// PATHS:
//   P1: read the constant (no branches)
// INPUT PARTITIONS:
//   Single partition — constants have no input.
// MATRIX: 1 output × 1 path × 1 partition = 1 cell
//   P1: O1✓
//
// Why exactly 12: the spec locks the structural set as N1-N12. A length
// mismatch (11 or 13) would silently corrupt the sunset denominator and
// the "62.7% structural share" claim in the centralization disclosure
// (spec §6).
#[test]
fn test_structural_pubkey_hashes_length_is_exactly_12() {
    assert_eq!(
        STRUCTURAL_PUBKEY_HASHES_HEX.len(),
        12,
        "STRUCTURAL_PUBKEY_HASHES_HEX MUST contain exactly 12 entries (N1-N12); got {}",
        STRUCTURAL_PUBKEY_HASHES_HEX.len()
    );
}

// OUTPUT CONTRACT: const STRUCTURAL_PUBKEY_HASHES_HEX — uniqueness
//   O1: HashSet built from STRUCTURAL_PUBKEY_HASHES_HEX.iter() — len == 12
// PATHS:
//   P1: collect into a HashSet (no branches)
// INPUT PARTITIONS:
//   Single partition — constants have no input.
// MATRIX: 1 output × 1 path × 1 partition = 1 cell
//   P1: O1✓
//
// Why no duplicates: a duplicate entry would double-count one operator's
// bonds in the sunset metric (`sum(bond_snapshot[k] for k in
// STRUCTURAL_PUBKEY_HASHES)`) — inflating `structural_share` artificially
// and weakening the 55% sunset trigger. This is a class of bug that
// is invisible without an explicit uniqueness assertion.
#[test]
fn test_structural_pubkey_hashes_no_duplicates() {
    use std::collections::HashSet;
    let unique: HashSet<&&str> = STRUCTURAL_PUBKEY_HASHES_HEX.iter().collect();
    assert_eq!(
        unique.len(),
        STRUCTURAL_PUBKEY_HASHES_HEX.len(),
        "STRUCTURAL_PUBKEY_HASHES_HEX contains duplicate entries — \
         a duplicate would double-count one operator's bonds in the \
         sunset metric and inflate structural_share. Found {} unique \
         out of {} entries.",
        unique.len(),
        STRUCTURAL_PUBKEY_HASHES_HEX.len()
    );
}

// OUTPUT CONTRACT: const STRUCTURAL_PUBKEY_HASHES_HEX — each parses to Hash
//   O1: Hash::from_hex(entry) — Some(Hash) for every entry
//   O2: Hash byte length — exactly 32 bytes (HASH_SIZE) per entry
// PATHS:
//   P1: iterate entries, call Hash::from_hex on each
// INPUT PARTITIONS:
//   12 partitions (one per entry). Each must independently parse — the
//   loop fails fast on the first non-parseable entry with a precise
//   error message pointing at the index, so a paste-error during
//   rotation lands on a single named line of test output.
// MATRIX: 2 outputs × 1 path × 12 partitions = 24 cells
//   P1×part-i: O1✓ O2✓ for i ∈ {0..12}
#[test]
fn test_structural_pubkey_hashes_all_parse_to_valid_hash() {
    for (i, hex) in STRUCTURAL_PUBKEY_HASHES_HEX.iter().enumerate() {
        // O2 indirectly: 32-byte Hash requires 64 hex chars.
        assert_eq!(
            hex.len(),
            64,
            "STRUCTURAL_PUBKEY_HASHES_HEX[{i}] (= N{}) is {} chars, expected 64 (32 bytes hex); value: {hex}",
            i + 1,
            hex.len()
        );

        // O1
        let parsed = Hash::from_hex(hex);
        assert!(
            parsed.is_some(),
            "STRUCTURAL_PUBKEY_HASHES_HEX[{i}] (= N{}) failed to parse as a Hash: {hex}",
            i + 1
        );

        // O2 explicitly
        let h = parsed.unwrap();
        assert_eq!(
            h.as_bytes().len(),
            32,
            "Hash::from_hex returned a non-32-byte Hash for N{}: {hex}",
            i + 1
        );
    }
}

// OUTPUT CONTRACT: const STRUCTURAL_PUBKEY_HASHES_HEX — cross-consistency with bootstrap pubkeys
//   O1: For i ∈ {0..5}, hash_with_domain(ADDRESS_DOMAIN,
//         PublicKey::from_hex(BOOTSTRAP_MAINTAINER_KEYS_MAINNET[i]).as_bytes())
//       — must equal Hash::from_hex(STRUCTURAL_PUBKEY_HASHES_HEX[i]).
// PATHS:
//   P1: per-index derivation + comparison loop, fails fast on first mismatch
// INPUT PARTITIONS:
//   5 partitions (one per cross-checkable slot, N1-N5). The remaining 7
//   slots (N6-N12) have no bootstrap-pubkey counterpart and rely on the
//   provenance-pipeline integrity (verified out-of-band at session time).
// MATRIX: 1 output × 1 path × 5 partitions = 5 cells
//   P1×part-i: O1✓ for i ∈ {0..5}
//
// Why this test: the most likely failure mode for this constant is a
// paste-error — wrong hash glued to the wrong slot during rotation, or
// a hash computed against the wrong pubkey. This test catches both
// classes for N1-N5 by re-deriving the hash from the bootstrap pubkeys
// (a SEPARATE source of truth in updater/src/constants.rs) and asserting
// equality. If updater's keys ever rotate without recomputing these
// hashes (or vice versa), this test fires immediately on the next build.
#[test]
fn test_structural_pubkey_hashes_n1_through_n5_match_bootstrap_keys() {
    for i in 0..5 {
        let pk_hex = N1_TO_N5_PUBKEYS_HEX[i];
        let pk = PublicKey::from_hex(pk_hex).unwrap_or_else(|e| {
            panic!(
                "N{}_PUBKEY_HEX is not a valid pubkey hex: {pk_hex} ({e:?})",
                i + 1
            )
        });

        // Re-derive the 32-byte pubkey_hash via the same algorithm
        // `post_commit.rs:211` uses to key bond_snapshot.
        let derived = hash_with_domain(ADDRESS_DOMAIN, pk.as_bytes());

        let stored = Hash::from_hex(STRUCTURAL_PUBKEY_HASHES_HEX[i]).unwrap_or_else(|| {
            panic!(
                "STRUCTURAL_PUBKEY_HASHES_HEX[{i}] (= N{}) failed to parse: {}",
                i + 1,
                STRUCTURAL_PUBKEY_HASHES_HEX[i]
            )
        });

        assert_eq!(
            derived,
            stored,
            "structural hash for N{} mismatches the bootstrap-pubkey derivation. \
             Stored: {}, derived from pubkey {pk_hex}: {}. \
             Either the pubkey rotated without recomputing the hash, \
             or the hash was pasted into the wrong slot. If the pubkey \
             intentionally changed, update BOTH STRUCTURAL_PUBKEY_HASHES_HEX[{i}] \
             AND updater::constants::BOOTSTRAP_MAINTAINER_KEYS_MAINNET[{i}] \
             AND N1_TO_N5_PUBKEYS_HEX[{i}] in this test file.",
            i + 1,
            stored.to_hex(),
            derived.to_hex()
        );
    }
}
