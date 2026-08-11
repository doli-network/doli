//! INC-I-173 M3 — ITEM 2 / spec F6: the chain-derived maintainer-set digest.
//!
//! Closes the AUDIT-P1-003 minimum obligation: an operator must be able to ask
//! two nodes "do we hold the same release-verification trust root?" and get a
//! single comparable scalar, without shipping the member list around.
//!
//! TDD RED. This file does NOT compile against the tree at `32e0a650`:
//! `doli_core::maintainer::maintainer_set_digest` does not exist. That failure
//! IS the RED evidence for every assertion below.
//!
//! Contract: `docs/.workflow/inc-i-173-M3-design-contract.md` Item 2.
//!
//! ---------------------------------------------------------------------------
//! REQUIRED API (verbatim from the contract)
//! ---------------------------------------------------------------------------
//! ```ignore
//! // crates/core/src/maintainer/ — a LEAF function. The genesis hash arrives as
//! // a plain scalar, the idiom already used for `activation_height: u64` at
//! // set.rs:259-261, so `crates/core::maintainer` gains NO dependency edge
//! // toward node-local state (C-coupling).
//! pub fn maintainer_set_digest(set: &MaintainerSet, genesis_hash: &[u8]) -> [u8; 32];
//! ```
//! Preimage, domain-separated and order-independent:
//! ```text
//! BLAKE3_256( b"DOLI-MAINTAINER-SET-V1"
//!           || genesis_hash
//!           || (set.threshold as u64).to_le_bytes()
//!           || concat(member pubkey bytes, ASCENDING by raw bytes) )
//! ```
//!
//! REVISED at review iteration 1 (M3a / F2): `set.last_updated` was in the
//! contract's preimage and is NOT any more. See the section below.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT — `maintainer_set_digest(set, genesis_hash)`
//! ---------------------------------------------------------------------------
//! ENUMERATION OF OBSERVABLE OUTPUTS
//!   O1: the returned `[u8; 32]`. The ONLY value channel.
//!   O2: the derived EQUALITY relation between two digests. This is what an
//!       operator actually consumes ("same or not same"), so it is asserted
//!       directly rather than inferred from O1.
//!   mutable params   : NONE — `set` and `genesis_hash` are shared refs.
//!   receiver mutation: NONE — free function.
//!   persistent store : NONE.
//!   side channels    : NONE. The function performs no logging and no I/O.
//!
//! CODE PATHS
//!   P1: empty member vector (a defaulted / never-bootstrapped set).
//!   P2: non-empty, already in ascending byte order (sort is a no-op).
//!   P3: non-empty, NOT in ascending byte order (sort actually reorders).
//!
//! INPUT PARTITIONS
//!   IP-M  member set: {}, {one}, {five in order}, {five permuted}, {five with
//!         one key swapped}
//!   IP-T  threshold:  {0, 2, 3}
//!   IP-U  last_updated: {0, 1, 88_289, u64::MAX} — the EXCLUDED term. Every
//!         partition must map to the SAME digest.
//!   IP-G  genesis_hash: {mainnet, testnet, devnet, empty slice}
//! MATRIX: (O1,O2) x {P1,P2,P3} x {IP-M x IP-T x IP-U x IP-G} — the named tests
//!   below cover every cell that can change the answer, plus the exact-preimage
//!   pin which subsumes the arithmetic.
//!
//! ---------------------------------------------------------------------------
//! WHY `last_updated` IS EXCLUDED AND WHY MEMBERS ARE SORTED
//! ---------------------------------------------------------------------------
//! Both answer the SAME question: does a term belong in a scalar whose stated
//! job is "do we hold the same release-verification trust root?".
//!
//! Members are SORTED because member order is not a stable property of the set:
//! two honest nodes can hold the same five keys in different insertion order
//! (AUDIT-P3-014), and an order-sensitive digest reports a false mismatch.
//!
//! `last_updated` is EXCLUDED for the same reason, measured rather than
//! inferred. It is NODE-LOCAL and outside the state root, and the M3 security
//! audit measured it divergent across the live testnet fleet at an IDENTICAL
//! tip: `docs/.workflow/chain-state.md:36-39` — RPC 8512 reported
//! `last_change_block = 88289` while 12 peers reported `1`, all at tip 134,682,
//! all holding the same five members and the same threshold. Those 13 nodes
//! accept exactly the same release signatures, because `verify_multisig`
//! consults members + threshold and never `last_updated`. A digest that binds it
//! reports MISMATCH for a fleet that is aligned — the same false-signal failure
//! the sorted-members design exists to prevent, reintroduced through another
//! term.
//!
//! `last_updated` remains published SEPARATELY as `last_change_block` on the
//! same RPC response and in the apply-side log line, so no information is lost:
//! the operator keeps the history term and gains a comparison scalar that does
//! not false-alarm.

use crypto::{Hasher, PublicKey};
use doli_core::chainspec::ChainSpec;
use doli_core::maintainer::{maintainer_set_digest, MaintainerSet};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// The literal domain tag from the contract. Pinned as a test constant so a
/// silent change to the tag in the implementation fails LOUDLY here: the tag is
/// what stops a digest from colliding with any other BLAKE3 in the tree.
const DOMAIN_TAG: &[u8] = b"DOLI-MAINTAINER-SET-V1";

fn key(seed: u8) -> PublicKey {
    *crypto::KeyPair::from_seed([seed; 32]).public_key()
}

fn mainnet_genesis() -> Vec<u8> {
    ChainSpec::mainnet().genesis_hash().as_bytes().to_vec()
}

fn testnet_genesis() -> Vec<u8> {
    ChainSpec::testnet().genesis_hash().as_bytes().to_vec()
}

fn devnet_genesis() -> Vec<u8> {
    ChainSpec::devnet().genesis_hash().as_bytes().to_vec()
}

/// Five deterministic keys, in the order they were generated. Deliberately NOT
/// sorted by raw bytes — the seeds are chosen so that the generated pubkeys are
/// in an arbitrary byte order, which is what makes the order-independence tests
/// meaningful rather than vacuous.
fn five_members() -> Vec<PublicKey> {
    vec![key(11), key(22), key(33), key(44), key(55)]
}

fn set_of(members: Vec<PublicKey>, threshold: usize, last_updated: u64) -> MaintainerSet {
    MaintainerSet {
        members,
        threshold,
        last_updated,
    }
}

/// The preimage, computed INDEPENDENTLY of the implementation.
///
/// This is the strongest available instrument: it pins the exact byte layout, so
/// a transposed field, a big-endian length, a missing domain tag, an unsorted
/// member concatenation or a re-added `last_updated` term all fail here rather
/// than surviving as a "self-consistent but wrong" digest that every node agrees
/// on and no specification describes.
///
/// Note what is ABSENT: `set.last_updated` is deliberately not hashed (F2).
fn expected_digest(set: &MaintainerSet, genesis_hash: &[u8]) -> [u8; 32] {
    let mut sorted: Vec<[u8; 32]> = set.members.iter().map(|m| *m.as_bytes()).collect();
    sorted.sort();

    let mut hasher = Hasher::new();
    hasher.update(DOMAIN_TAG);
    hasher.update(genesis_hash);
    hasher.update(&(set.threshold as u64).to_le_bytes());
    for m in &sorted {
        hasher.update(m);
    }
    *hasher.finalize().as_bytes()
}

// ===========================================================================
// The exact-preimage pin
// ===========================================================================

/// AUDIT-P1-003 (Must) — the digest IS the contract's preimage, byte for byte.
///
/// Driven over P1 (empty), P2/P3 (five members, both orders), every threshold
/// partition and every genesis partition, so a single assertion covers the whole
/// arithmetic surface.
#[test]
fn audit_p1_003_digest_matches_the_specified_preimage_exactly() {
    let mut sorted_five = five_members();
    sorted_five.sort_by_key(|k| *k.as_bytes());

    let sets = [
        set_of(Vec::new(), 0, 0),                // P1
        set_of(vec![key(11)], 1, 7),             // P2 (single, trivially sorted)
        set_of(sorted_five.clone(), 3, 172_000), // P2
        set_of(five_members(), 3, 172_000),      // P3 (unsorted insertion order)
        set_of(five_members(), 2, u64::MAX),     // IP-T / IP-U extremes
    ];
    let genesis = [
        mainnet_genesis(),
        testnet_genesis(),
        devnet_genesis(),
        Vec::new(), // IP-G: the empty slice must not panic
    ];

    for set in &sets {
        for g in &genesis {
            assert_eq!(
                maintainer_set_digest(set, g),
                expected_digest(set, g),
                "O1 / AUDIT-P1-003: the digest must be exactly \
                 BLAKE3_256(\"DOLI-MAINTAINER-SET-V1\" || genesis_hash || \
                 threshold_le_u64 || sorted member bytes) — and NOTHING else, in \
                 particular NOT last_updated (F2). Mismatch for a {}-member set \
                 (threshold {}, last_updated {}) over a {}-byte genesis hash.",
                set.members.len(),
                set.threshold,
                set.last_updated,
                g.len()
            );
        }
    }
}

/// AUDIT-P1-003 (Must) — the domain tag is really there.
///
/// Anti-vacuity partner of the preimage pin. Without the tag the digest is a
/// bare BLAKE3 over concatenated fields and can collide with any other such
/// hash in the tree. This computes the SAME preimage with the tag removed and
/// requires the answers to differ.
#[test]
fn audit_p1_003_digest_is_domain_separated() {
    let set = set_of(five_members(), 3, 172_000);
    let g = mainnet_genesis();

    let mut sorted: Vec<[u8; 32]> = set.members.iter().map(|m| *m.as_bytes()).collect();
    sorted.sort();
    let mut untagged = Hasher::new();
    untagged.update(&g);
    untagged.update(&(set.threshold as u64).to_le_bytes());
    for m in &sorted {
        untagged.update(m);
    }

    assert_ne!(
        maintainer_set_digest(&set, &g),
        *untagged.finalize().as_bytes(),
        "O1: the digest must be DOMAIN-SEPARATED by \"{}\". Without the tag it is \
         an undifferentiated BLAKE3 over the same bytes and nothing stops it \
         colliding with another hash of the same shape.",
        String::from_utf8_lossy(DOMAIN_TAG)
    );
}

// ===========================================================================
// The behavioural properties an operator relies on
// ===========================================================================

/// AUDIT-P1-003 (Must) — DETERMINISM. The same set and genesis hash always give
/// the same digest, across repeated calls and across separately constructed but
/// equal values.
#[test]
fn audit_p1_003_digest_is_deterministic() {
    let g = mainnet_genesis();
    let a = set_of(five_members(), 3, 172_000);
    let b = set_of(five_members(), 3, 172_000);

    let d = maintainer_set_digest(&a, &g);
    for i in 0..8 {
        assert_eq!(
            maintainer_set_digest(&a, &g),
            d,
            "O1: call {} returned a different digest for the same input — the \
             function is not deterministic and cannot be compared across nodes",
            i
        );
    }
    assert_eq!(
        maintainer_set_digest(&b, &g),
        d,
        "O2: two separately constructed but EQUAL sets must digest identically"
    );
}

/// AUDIT-P1-003 (Must) — ORDER INDEPENDENCE over member insertion order.
///
/// This is the property that makes the digest usable at all. Member order is not
/// a stable property of the maintainer set: below
/// `maintainer_derivation_activation_height` the derivation stable-sorts a
/// `HashMap` iteration with no pubkey tiebreak (AUDIT-P3-014), so two honest
/// nodes on the same chain can hold the same five keys in different order. An
/// order-SENSITIVE digest would report a false mismatch and send an operator
/// chasing a divergence that does not exist — the exact failure the
/// `getMaintainerSet` M2 review F4 fix already had to repair once.
///
/// Driven over rotations AND a reversal, so the assertion is not satisfied by a
/// digest that merely happens to be stable under one permutation.
#[test]
fn audit_p1_003_digest_is_independent_of_member_insertion_order() {
    let g = mainnet_genesis();
    let base = five_members();
    let reference = maintainer_set_digest(&set_of(base.clone(), 3, 172_000), &g);

    // Every rotation.
    for shift in 1..base.len() {
        let mut permuted = base.clone();
        permuted.rotate_left(shift);
        assert_ne!(
            permuted, base,
            "fixture: rotation by {} must actually reorder the vector",
            shift
        );
        assert_eq!(
            maintainer_set_digest(&set_of(permuted, 3, 172_000), &g),
            reference,
            "O2: rotating the member vector by {} changed the digest. Two nodes \
             holding the SAME five keys in different insertion order hold the SAME \
             trust root; the digest must say so.",
            shift
        );
    }

    // The reversal.
    let mut reversed = base.clone();
    reversed.reverse();
    assert_eq!(
        maintainer_set_digest(&set_of(reversed, 3, 172_000), &g),
        reference,
        "O2: reversing the member vector changed the digest"
    );

    // And the fully sorted form, which is what the implementation should be
    // hashing internally.
    let mut sorted = base.clone();
    sorted.sort_by_key(|k| *k.as_bytes());
    assert_eq!(
        maintainer_set_digest(&set_of(sorted, 3, 172_000), &g),
        reference,
        "O2: the ascending-byte-order form must digest identically"
    );
}

/// AUDIT-P1-003 (Must) — the digest CHANGES when a MEMBER changes.
///
/// Anti-vacuity for order-independence: a constant function is also
/// order-independent. Three shapes: swap one key, drop one key, add one key.
#[test]
fn audit_p1_003_digest_changes_when_a_member_changes() {
    let g = mainnet_genesis();
    let base = five_members();
    let reference = maintainer_set_digest(&set_of(base.clone(), 3, 172_000), &g);

    let mut swapped = base.clone();
    swapped[2] = key(99);
    assert_ne!(
        maintainer_set_digest(&set_of(swapped, 3, 172_000), &g),
        reference,
        "O2: replacing one member must change the digest — otherwise the digest \
         cannot detect a rotated trust root, which is its entire purpose"
    );

    let mut dropped = base.clone();
    dropped.pop();
    assert_ne!(
        maintainer_set_digest(&set_of(dropped, 3, 172_000), &g),
        reference,
        "O2: removing a member must change the digest"
    );

    let mut added = base.clone();
    added.push(key(99));
    assert_ne!(
        maintainer_set_digest(&set_of(added, 3, 172_000), &g),
        reference,
        "O2: adding a member must change the digest"
    );
}

/// AUDIT-P1-003 (Must) — the digest CHANGES when `threshold` changes.
///
/// `calculate_threshold` is a function of member count today, but the field is
/// persisted VERBATIM in `maintainer_state.bin` and a hand-edited file can carry
/// any value. A host-local attacker who lowers the threshold without touching
/// the members has weakened the trust root; the digest must expose that.
#[test]
fn audit_p1_003_digest_changes_when_threshold_changes() {
    let g = mainnet_genesis();
    let members = five_members();
    let at_three = maintainer_set_digest(&set_of(members.clone(), 3, 172_000), &g);

    for threshold in [0usize, 1, 2, 4, 5] {
        assert_ne!(
            maintainer_set_digest(&set_of(members.clone(), threshold, 172_000), &g),
            at_three,
            "O2: threshold {} must digest differently from threshold 3. A set whose \
             members match but whose threshold was lowered is a WEAKENED trust \
             root, and `threshold` is persisted verbatim so a hand-edited file can \
             carry any value.",
            threshold
        );
    }
}

/// AUDIT-P1-003 / F2 (Must) — the digest IGNORES `last_updated`.
///
/// THE EXCLUSION PIN. This is the assertion that makes the operator-facing claim
/// in `docs/rpc_reference.md` — "Two nodes that accept the same release
/// signatures always return the same digest; two nodes that would accept
/// different ones never do" — true rather than aspirational.
///
/// `last_updated` is NODE-LOCAL and outside the state root, and it was MEASURED
/// divergent across the live testnet fleet at an identical tip
/// (`docs/.workflow/chain-state.md:36-39`: RPC 8512 `last_change_block = 88289`
/// vs 12 peers at `1`, all at tip 134,682, all holding the same five members and
/// the same threshold). Those 13 nodes accept exactly the same release
/// signatures — `verify_multisig` consults members + threshold and never
/// `last_updated` — so a digest binding it would report a MISMATCH for a fleet
/// that is aligned on the only thing the digest claims to compare.
///
/// The IP-U partitions include the two values actually measured on the fleet.
#[test]
fn audit_p1_003_digest_is_independent_of_last_updated() {
    let members = five_members();

    for g in [mainnet_genesis(), testnet_genesis(), Vec::new()] {
        let reference = maintainer_set_digest(&set_of(members.clone(), 3, 1), &g);

        for h in [0u64, 88_289, 172_000, u64::MAX] {
            assert_eq!(
                maintainer_set_digest(&set_of(members.clone(), 3, h), &g),
                reference,
                "O2 / F2: last_updated {} must digest IDENTICALLY to last_updated 1. \
                 The measured testnet fleet held `last_change_block` 88289 on one \
                 host and 1 on twelve others at the SAME tip with the SAME five \
                 members and the SAME threshold; binding this node-local term makes \
                 the digest report a divergence that does not exist on the trust \
                 root it claims to compare.",
                h
            );
        }
    }

    // Anti-vacuity: the exclusion must not have been achieved by making the
    // digest insensitive in general. A member change and a threshold change must
    // still move it (both are asserted in full by their own tests above; these
    // two lines guard against this test passing for the wrong reason).
    let g = mainnet_genesis();
    let reference = maintainer_set_digest(&set_of(members.clone(), 3, 1), &g);
    let mut swapped = members.clone();
    swapped[0] = key(99);
    assert_ne!(
        maintainer_set_digest(&set_of(swapped, 3, u64::MAX), &g),
        reference,
        "anti-vacuity: a MEMBER change must still move the digest"
    );
    assert_ne!(
        maintainer_set_digest(&set_of(members, 2, u64::MAX), &g),
        reference,
        "anti-vacuity: a THRESHOLD change must still move the digest"
    );
}

/// AUDIT-P1-003 (Must) — the digest DIFFERS across networks.
///
/// `bootstrap_maintainer_keys` returns a BYTE-IDENTICAL array for mainnet and
/// testnet (`crates/updater/src/constants.rs:53-86`, and its own doc says the
/// selection "is NOT a cross-network security boundary"). So a maintainer-set
/// digest that omitted the genesis hash would be IDENTICAL on mainnet and
/// testnet for the bootstrap five, and an operator comparing a mainnet node
/// against a testnet node would see a match. The genesis hash is what makes the
/// digest a per-CHAIN answer.
#[test]
fn audit_p1_003_digest_differs_across_genesis_hashes() {
    let members = five_members();
    let set = set_of(members, 3, 172_000);

    let m = maintainer_set_digest(&set, &mainnet_genesis());
    let t = maintainer_set_digest(&set, &testnet_genesis());
    let d = maintainer_set_digest(&set, &devnet_genesis());

    assert_ne!(
        mainnet_genesis(),
        testnet_genesis(),
        "fixture: mainnet and testnet genesis hashes must differ, or this test is \
         vacuous"
    );
    assert_ne!(
        m, t,
        "O2: the SAME maintainer set on mainnet and on testnet must digest \
         DIFFERENTLY. The bootstrap key arrays are byte-identical across those two \
         networks, so without the genesis hash in the preimage an operator \
         comparing a mainnet node to a testnet node sees a false MATCH."
    );
    assert_ne!(m, d, "O2: mainnet and devnet must digest differently");
    assert_ne!(t, d, "O2: testnet and devnet must digest differently");
}

/// AUDIT-P1-003 (Should) — P1: an EMPTY set is representable and does not panic.
///
/// Worst-scenario #1. `MaintainerState::default()` carries an empty set with
/// threshold 0, and that value is reachable on every fresh node, so the digest
/// must be total over it. It must also NOT collide with a populated set.
#[test]
fn audit_p1_003_empty_set_digests_without_panicking_and_does_not_collide() {
    let g = mainnet_genesis();
    let empty = maintainer_set_digest(&MaintainerSet::new(), &g);

    assert_eq!(
        empty,
        maintainer_set_digest(&MaintainerSet::new(), &g),
        "O1: the empty-set digest must be deterministic"
    );
    assert_ne!(
        empty,
        maintainer_set_digest(&set_of(five_members(), 3, 172_000), &g),
        "O2: an empty set must not digest identically to a populated one"
    );
    assert_ne!(
        empty, [0u8; 32],
        "O1: the empty-set digest must be a real hash, not a zero sentinel — a \
         zero digest is indistinguishable from an uninitialised field on the wire"
    );
}

/// AUDIT-P1-003 (Should) — DUPLICATE members do not collide with the deduplicated
/// set.
///
/// `validate_persisted_set` refuses duplicates on the storage path, but
/// `maintainer_set_digest` is a leaf function with no such guard, and a caller
/// can hand it any `MaintainerSet`. Sorting without dedup means `[A, A, B]` and
/// `[A, B]` must digest differently — if they collided, a duplicate-padded file
/// would report the same trust root as the honest one.
#[test]
fn audit_p1_003_duplicate_members_do_not_collide_with_the_deduplicated_set() {
    let g = mainnet_genesis();
    let a = key(11);
    let b = key(22);

    assert_ne!(
        maintainer_set_digest(&set_of(vec![a, a, b], 2, 5), &g),
        maintainer_set_digest(&set_of(vec![a, b], 2, 5), &g),
        "O2: a duplicate-padded member vector must NOT digest identically to the \
         deduplicated one. The digest reports what is ON DISK; collapsing \
         duplicates would hide a malformed trust root behind an honest-looking \
         value."
    );
}
