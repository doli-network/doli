//! INC-I-172 M2 — CATEGORY B: the contract for the NEW M2 core API.
//!
//! This file does NOT compile against the current tree. That is intentional and
//! is the point of the RED phase: it names the exact API M2 must provide. It is
//! kept in a SEPARATE file from the Category A reproductions
//! (`inc_i_172_m2_maintainer_governance.rs`) so that a compile error here can
//! never hide the runtime RED evidence there.
//!
//! ---------------------------------------------------------------------------
//! REQUIRED API (the developer must provide EXACTLY these names/signatures)
//! ---------------------------------------------------------------------------
//! In `crates/core/src/maintainer.rs`, re-exported from `crates/core/src/lib.rs`:
//!
//! ```ignore
//! /// F2/F8 — the ONE canonical, totally-ordered, pure derivation. Replaces the
//! /// three rival derivations at periodic.rs:52-64, governance.rs:112-124 and
//! /// the reader-order bootstrap inside derive_maintainer_set.
//! ///
//! /// Sorts by (registered_at ASC, pubkey_bytes ASC) — a TOTAL order, so ties on
//! /// registered_at can never leave the result dependent on input order — then
//! /// takes the first INITIAL_MAINTAINER_COUNT.
//! ///
//! /// Takes a VALUE slice, never `ProducerInfo`: `crates/core` must keep its
//! /// no-edge-to-`storage` boundary (C-R4).
//! pub fn derive_canonical_maintainer_set(
//!     registrations: &[(PublicKey, u64)],
//!     height: u64,
//! ) -> MaintainerSet;
//!
//! impl MaintainerSet {
//!     /// POST-activation semantics, and the DEFAULT: distinct-signer k-of-n,
//!     /// shaped after the mainnet-live covenant loop at conditions/eval.rs:51-68.
//!     pub fn verify_multisig(&self, sigs: &[MaintainerSignature], msg: &[u8]) -> bool;
//!     pub fn verify_multisig_excluding(
//!         &self, sigs: &[MaintainerSignature], msg: &[u8], excluded: &PublicKey,
//!     ) -> bool;
//!
//!     /// EXACT pre-activation behavior, preserved byte-for-byte (entry
//!     /// counting via `.filter(..).count() >= self.threshold`). Exists ONLY so
//!     /// a height gate can reproduce history. MUST NOT be called ungated.
//!     pub fn verify_multisig_legacy(&self, sigs: &[MaintainerSignature], msg: &[u8]) -> bool;
//!     pub fn verify_multisig_excluding_legacy(
//!         &self, sigs: &[MaintainerSignature], msg: &[u8], excluded: &PublicKey,
//!     ) -> bool;
//!
//!     /// The height-gated dispatcher every call site must use.
//!     /// height <  activation_height -> *_legacy
//!     /// height >= activation_height -> distinct-signer
//!     pub fn verify_multisig_at(
//!         &self, sigs: &[MaintainerSignature], msg: &[u8],
//!         height: u64, activation_height: u64,
//!     ) -> bool;
//!     pub fn verify_multisig_excluding_at(
//!         &self, sigs: &[MaintainerSignature], msg: &[u8], excluded: &PublicKey,
//!         height: u64, activation_height: u64,
//!     ) -> bool;
//! }
//! ```
//!
//! **UNGATED, by deliberate exception:** `MaintainerSet::calculate_threshold(0)`
//! must stop returning 0 at ALL heights.
//!
//! CORRECTED 2026-08-10 (M2 QA, OBS-004). The rationale originally written here
//! claimed `ProtocolActivation` "never reaches an empty set" because
//! `governance.rs` falls back to a producer-derived set. **That claim is WRONG.**
//! Below the gate, on a chain with fewer than 5 producers the seed never fires, so
//! the on-chain set stays empty; pre-M2 an empty set accepted a ZERO-signature
//! `AddMaintainer`, leaving `members=[attacker], threshold=1`; four more adds reach
//! 5, `is_fully_bootstrapped()` flips true, and `governance.rs` then DOES use the
//! attacker's set for `ProtocolActivation`. The empty set is reachable transitively.
//!
//! The real rationale, which does hold: an empty set has no legitimate authority to
//! preserve at any height; the change is strictly MORE restrictive (ACCEPT → REJECT
//! only, and only on inputs with zero valid distinct signatures); and the outcome is
//! not consensus-visible today, because `ChainState::serialize_canonical` excludes
//! both `active_protocol_version` and `pending_protocol_activation` from the state
//! root and `is_protocol_active` has zero production callers. The remaining consumers
//! write `maintainer_state.bin`, which is node-local and outside the state root.
//! **This exception is the one place M2 changes pre-activation behavior; the last two
//! facts are properties of the current tree, not invariants, and expire the moment
//! anything reads `active_protocol_version`.**
//!
//! Requirements: REQ-172-005, REQ-172-010, REQ-172-012.
//! Findings: AUDIT-P0-010, AUDIT-P1-010 (FM-02), AUDIT-P3-014.
//! Spec: `specs/maintainer-trust-root-architecture.md` §F2, §F3, §F8.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT — `derive_canonical_maintainer_set(&[(PublicKey, u64)], u64)`
//! ---------------------------------------------------------------------------
//! OUTPUTS
//!   O1 return `.members` (Vec<PublicKey>) — ORDER is observable: it is
//!      serialized verbatim into `maintainer_state.bin` and returned by the
//!      `getMaintainerSet` RPC, so "same set, different order" is a real
//!      divergence, not a cosmetic one
//!   O2 return `.threshold`
//!   O3 return `.last_updated`
//!   O4 (mutable params)          — NONE; the input is `&[..]`, read-only
//!   O5 (receiver mutation)       — NONE; free function
//!   O6 (persistent store writes) — NONE; `crates/core` has no storage edge
//!
//! PATHS
//!   PC-truncate — more than INITIAL_MAINTAINER_COUNT registrations: take the
//!                 first 5 under the total order
//!   PC-partial  — fewer than 5: take all of them
//!   PC-empty    — zero registrations
//!
//! INPUT PARTITIONS
//!   IP-B1  8 registrations, ALL registered_at == 0, fed in 8 rotations
//!          -> identical O1 (order included) for every rotation      [PC-truncate]
//!   IP-B2  8 registrations, ALL tied, fed reversed / shuffled
//!          -> O1 equals ascending pubkey-byte order                 [PC-truncate]
//!   IP-B3  mixed registered_at where the LOWEST height has the HIGHEST
//!          pubkey bytes -> registered_at is the PRIMARY key         [PC-truncate]
//!   IP-B4  3 registrations                                          [PC-partial]
//!   IP-B5  0 registrations                                          [PC-empty]
//!   IP-B6  duplicate pubkey present twice                           [PC-truncate]
//!
//! MATRIX
//!   O1 x {IP-B1..IP-B6} = 6 assertions (order-sensitive)
//!   O2 x {IP-B1, IP-B4, IP-B5} = 3 assertions
//!   O3 x {IP-B1} = 1 assertion
//!   O4/O5/O6 — structurally absent.

use crypto::{KeyPair, PublicKey};
use doli_core::maintainer::{
    derive_canonical_maintainer_set, derive_maintainer_set, BlockchainReader, MaintainerChange,
    MaintainerChangeData, MaintainerSet, MaintainerSignature, INITIAL_MAINTAINER_COUNT,
};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A deterministic public key. `seed` lands in byte 0, so byte-order equals
/// seed order and every expectation below can be written by hand.
fn pk(seed: u8) -> PublicKey {
    let mut b = [0u8; 32];
    b[0] = seed;
    PublicKey::from_bytes(b)
}

fn sig(kp: &KeyPair, message: &[u8]) -> MaintainerSignature {
    MaintainerSignature::new(
        *kp.public_key(),
        crypto::signature::sign(message, kp.private_key()),
    )
}

fn repeated(kp: &KeyPair, message: &[u8], n: usize) -> Vec<MaintainerSignature> {
    (0..n).map(|_| sig(kp, message)).collect()
}

/// Every rotation of `v` — enough to prove order-independence without a
/// factorial blow-up, and it covers the case where each element in turn is the
/// one the HashMap happened to yield first.
fn rotations<T: Clone>(v: &[T]) -> Vec<Vec<T>> {
    (0..v.len())
        .map(|k| {
            let mut r = v.to_vec();
            r.rotate_left(k);
            r
        })
        .collect()
}

// ---------------------------------------------------------------------------
// F2/F8 — canonical derivation determinism
// ---------------------------------------------------------------------------

/// IP-B1. O1, O2, O3 x PC-truncate. REQ-172-005 / AUDIT-P3-014.
///
/// Every rotation of the SAME tied registrations must derive a byte-identical
/// set. This is the property the current `all_producers()` + stable-sort path
/// cannot hold: the RED evidence is in
/// `bins/node/tests/inc_i_172_m2_maintainer_reset.rs::bootstrap_derivation_must_be_identical_across_nodes_with_tied_producers`.
#[test]
fn canonical_derivation_is_identical_for_every_input_rotation_when_all_tied() {
    let regs: Vec<(PublicKey, u64)> = (1..=8u8).map(|i| (pk(i), 0u64)).collect();

    let mut seen: Vec<Vec<PublicKey>> = Vec::new();
    for rot in rotations(&regs) {
        let set = derive_canonical_maintainer_set(&rot, 42);
        assert_eq!(
            set.member_count(),
            INITIAL_MAINTAINER_COUNT,
            "O1: 8 registrations must truncate to exactly {INITIAL_MAINTAINER_COUNT}"
        );
        assert_eq!(set.threshold, 3, "O2: a 5-member set has threshold 3");
        assert_eq!(
            set.last_updated, 42,
            "O3: last_updated is the passed height"
        );
        seen.push(set.members);
    }

    let first = &seen[0];
    for (i, s) in seen.iter().enumerate().skip(1) {
        assert_eq!(
            s, first,
            "REQ-172-005 / AUDIT-P3-014: rotation {i} of the SAME tied \
             registrations derived a different maintainer root. The canonical \
             derivation must sort by the TOTAL order (registered_at, \
             pubkey_bytes), so input order is unobservable in the output — \
             ORDER included, because .members is serialized verbatim into \
             maintainer_state.bin and returned by getMaintainerSet."
        );
    }
}

/// IP-B2. O1 x PC-truncate. Pins the concrete order, not merely its stability.
/// A derivation that is deterministic but sorted the other way would still fork
/// a fleet mid-upgrade, so the direction is part of the contract.
#[test]
fn canonical_derivation_orders_ties_by_ascending_pubkey_bytes() {
    let regs: Vec<(PublicKey, u64)> = (1..=8u8).rev().map(|i| (pk(i), 0u64)).collect();

    let set = derive_canonical_maintainer_set(&regs, 0);

    assert_eq!(
        set.members,
        vec![pk(1), pk(2), pk(3), pk(4), pk(5)],
        "O1: on a registered_at tie the tiebreak is ASCENDING pubkey bytes, and \
         the first {INITIAL_MAINTAINER_COUNT} are taken"
    );
}

/// IP-B3. O1 x PC-truncate. `registered_at` is the PRIMARY key: the pubkey
/// tiebreak must never reorder across different registration heights.
#[test]
fn canonical_derivation_uses_registered_at_as_the_primary_key() {
    // The EARLIEST registration carries the HIGHEST pubkey bytes, so a
    // pubkey-first sort would drop it.
    let regs = vec![
        (pk(99), 1u64),
        (pk(10), 5u64),
        (pk(11), 6u64),
        (pk(12), 7u64),
        (pk(13), 8u64),
        (pk(14), 9u64),
    ];

    let set = derive_canonical_maintainer_set(&regs, 0);

    assert_eq!(
        set.members,
        vec![pk(99), pk(10), pk(11), pk(12), pk(13)],
        "O1: registered_at is the PRIMARY sort key; pubkey_bytes breaks ties ONLY"
    );
}

/// IP-B4. O1, O2 x PC-partial.
#[test]
fn canonical_derivation_accepts_fewer_than_five_registrations() {
    let regs = vec![(pk(3), 0u64), (pk(1), 0u64), (pk(2), 0u64)];

    let set = derive_canonical_maintainer_set(&regs, 7);

    assert_eq!(
        set.members,
        vec![pk(1), pk(2), pk(3)],
        "O1: a partial set is still canonically ordered"
    );
    assert_eq!(set.threshold, 2, "O2: a 3-member set has threshold 2");
}

/// IP-B5. O1, O2 x PC-empty. Ties back to FM-02: an empty derivation must
/// produce a set that authorizes NOTHING.
#[test]
fn canonical_derivation_of_nothing_authorizes_nothing() {
    let set = derive_canonical_maintainer_set(&[], 0);

    assert!(set.members.is_empty(), "O1: no registrations, no members");
    assert_ne!(
        set.threshold, 0,
        "O2 / FM-02: an empty derived set must NOT carry threshold 0, or \
         `valid_count >= threshold` is vacuous and a zero-signature \
         AddMaintainer is accepted"
    );
    assert!(
        !set.verify_multisig(&[], b"add:attacker"),
        "FM-02: an empty derived set must reject a zero-signature payload"
    );
}

/// IP-B6. O1 x PC-truncate. A duplicated registration must not occupy two
/// seats — `MaintainerSet` membership is a set, and a duplicate would silently
/// shrink the effective quorum while keeping threshold at 3.
#[test]
fn canonical_derivation_does_not_seat_a_duplicate_twice() {
    let regs = vec![
        (pk(1), 0u64),
        (pk(1), 0u64),
        (pk(2), 0u64),
        (pk(3), 0u64),
        (pk(4), 0u64),
        (pk(5), 0u64),
    ];

    let set = derive_canonical_maintainer_set(&regs, 0);

    let mut uniq = set.members.clone();
    uniq.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    uniq.dedup();
    assert_eq!(
        uniq.len(),
        set.members.len(),
        "O1: a duplicated registration must not take two seats — that would cut \
         the effective quorum from 3 distinct keys to 2 while threshold stays 3"
    );
    assert_eq!(
        set.members,
        vec![pk(1), pk(2), pk(3), pk(4), pk(5)],
        "O1: the duplicate is collapsed and the next registration takes the seat"
    );
}

// ---------------------------------------------------------------------------
// F3 — the height-gated counter and its pre-activation parity
// ---------------------------------------------------------------------------

const AH: u64 = 15_087; // testnet maintainer_derivation_activation_height

struct Five {
    kps: Vec<KeyPair>,
}

impl Five {
    fn new() -> Self {
        Self {
            kps: (0..5).map(|_| KeyPair::generate()).collect(),
        }
    }
    fn set(&self) -> MaintainerSet {
        MaintainerSet::with_members(self.kps.iter().map(|k| *k.public_key()).collect(), 0)
    }
}

/// ACTIVATION-GATING REGRESSION — pre-height parity for the counter.
/// `height < activation_height` must reproduce the OLD entry-counting result
/// EXACTLY, including the result that today's audit calls a defect. Consensus
/// history is not rewritten.
#[test]
fn pre_activation_counter_preserves_entry_counting_exactly() {
    let f = Five::new();
    let set = f.set();
    let msg = b"activate:2:500";

    let one_key_three_entries = repeated(&f.kps[0], msg, 3);

    assert!(
        set.verify_multisig_legacy(&one_key_three_entries, msg),
        "PARITY: verify_multisig_legacy must reproduce the pre-activation result \
         byte-for-byte — 3 ENTRIES from one key were accepted, and replaying \
         history below the gate must still accept them"
    );
    assert!(
        set.verify_multisig_at(&one_key_three_entries, msg, AH - 1, AH),
        "PARITY: at height AH-1 the dispatcher must take the legacy branch"
    );
    assert!(
        set.verify_multisig_excluding_legacy(&one_key_three_entries, msg, f.kps[4].public_key()),
        "PARITY: the excluding path keeps its pre-activation result too"
    );
    assert!(
        set.verify_multisig_excluding_at(
            &one_key_three_entries,
            msg,
            f.kps[4].public_key(),
            AH - 1,
            AH
        ),
        "PARITY: at height AH-1 the excluding dispatcher takes the legacy branch"
    );
}

/// The gate flips exactly AT the activation height, not one block late.
#[test]
fn counter_becomes_distinct_signer_at_the_activation_height() {
    let f = Five::new();
    let set = f.set();
    let msg = b"activate:2:500";

    let one_key_three_entries = repeated(&f.kps[0], msg, 3);

    assert!(
        !set.verify_multisig_at(&one_key_three_entries, msg, AH, AH),
        "REQ-172-012: at height == activation_height the distinct-signer counter \
         is in force; 1 distinct signer must not clear a 3-of-5 threshold"
    );
    assert!(
        !set.verify_multisig_at(&one_key_three_entries, msg, AH + 1, AH),
        "REQ-172-012: and it stays in force above the gate"
    );
    assert!(
        !set.verify_multisig_excluding_at(
            &one_key_three_entries,
            msg,
            f.kps[4].public_key(),
            AH,
            AH
        ),
        "REQ-172-012: the removal path flips at the same height"
    );
}

/// Liveness control across the gate: a GENUINE 3-distinct-signer quorum must be
/// accepted on BOTH sides of the activation height. Without this, the fix could
/// be a governance lock-out rather than a security fix.
#[test]
fn control_genuine_quorum_is_accepted_on_both_sides_of_the_gate() {
    let f = Five::new();
    let set = f.set();
    let msg = b"activate:2:500";

    let quorum = vec![
        sig(&f.kps[0], msg),
        sig(&f.kps[1], msg),
        sig(&f.kps[2], msg),
    ];

    assert!(
        set.verify_multisig_at(&quorum, msg, AH - 1, AH),
        "CONTROL: a genuine quorum is accepted below the gate"
    );
    assert!(
        set.verify_multisig_at(&quorum, msg, AH, AH),
        "CONTROL: a genuine quorum is accepted at and above the gate"
    );
}

// ---------------------------------------------------------------------------
// REQ-172-010 — replay completeness
// ---------------------------------------------------------------------------

struct Chain {
    registrations: Vec<(u64, PublicKey)>,
    changes: Vec<(u64, MaintainerChange)>,
    slashed: Vec<(u64, PublicKey)>,
}

impl BlockchainReader for Chain {
    fn get_registrations_in_order(&self) -> Vec<(u64, PublicKey)> {
        self.registrations.clone()
    }
    fn get_maintainer_changes(&self) -> Vec<(u64, MaintainerChange)> {
        self.changes.clone()
    }
    fn get_slashed_producers(&self) -> Vec<(u64, PublicKey)> {
        self.slashed.clone()
    }
}

/// Replay the WHOLE chain under the POST-activation distinct-signer rule.
///
/// INC-I-172 M2 review F1 gave `derive_maintainer_set` a height bound
/// (`up_to_height`) and an `activation_height`, so each governance action is
/// verified under the rule in force AT its own height. These tests assert the
/// post-activation rule over the whole history, which is what they asserted
/// before the parameters existed — no assertion changes.
fn derive_maintainer_set_at_tip<R: BlockchainReader>(reader: &R) -> MaintainerSet {
    derive_maintainer_set(reader, u64::MAX, 0)
}

/// Build a `RemoveMaintainer` change authorized by three DISTINCT maintainers.
fn remove_change(target: &PublicKey, signers: [&KeyPair; 3]) -> MaintainerChangeData {
    let mut d = MaintainerChangeData::new(*target, vec![]);
    let msg = d.signing_message(false);
    d.signatures = signers.iter().map(|kp| sig(kp, &msg)).collect();
    d
}

/// REQ-172-010. Two readers presenting an IDENTICAL block history in a
/// DIFFERENT enumeration order must derive byte-identical roots. This is the
/// fresh-sync / backfill convergence property: two nodes that walked the same
/// chain by different routes must agree.
#[test]
fn replay_is_byte_identical_for_identical_history_enumerated_differently() {
    let kps: Vec<KeyPair> = (0..8).map(|_| KeyPair::generate()).collect();

    // Every registration ties at height 0 — the genesis shape.
    let regs_a: Vec<(u64, PublicKey)> = kps.iter().map(|k| (0u64, *k.public_key())).collect();
    let mut regs_b = regs_a.clone();
    regs_b.reverse();

    // Derive the seeded root first so the removal target is well defined under
    // the canonical order, independent of enumeration.
    let seed = derive_canonical_maintainer_set(
        &regs_a.iter().map(|(h, k)| (*k, *h)).collect::<Vec<_>>(),
        0,
    );
    let target = seed.members[0];
    let signers: Vec<&KeyPair> = kps
        .iter()
        .filter(|k| seed.is_maintainer(k.public_key()) && *k.public_key() != target)
        .take(3)
        .collect();
    let change = remove_change(&target, [signers[0], signers[1], signers[2]]);

    let a = derive_maintainer_set_at_tip(&Chain {
        registrations: regs_a,
        changes: vec![(100, MaintainerChange::Remove(change.clone()))],
        slashed: vec![],
    });
    let b = derive_maintainer_set_at_tip(&Chain {
        registrations: regs_b,
        changes: vec![(100, MaintainerChange::Remove(change))],
        slashed: vec![],
    });

    assert_eq!(
        a.members, b.members,
        "REQ-172-010: derive_maintainer_set must seat the genesis five through \
         the CANONICAL derivation, not in reader-enumeration order, or two nodes \
         that walked the same chain by different routes disagree on the install \
         trust root"
    );
    assert_eq!(
        a.threshold, b.threshold,
        "REQ-172-010: threshold must agree"
    );
    assert!(
        !a.is_maintainer(&target),
        "REQ-172-010: the governance removal must survive the replay"
    );
}

/// REQ-172-010 — a property of the PURE FUNCTION `derive_maintainer_set`, not
/// of the running node.
///
/// `derive_maintainer_set(reader, up_to_height, activation_height)` — whole
/// history replayed in one pass — must return the same `MaintainerSet` as
/// applying the genesis seed once and then each governance action incrementally,
/// which is what `process_transaction_governance` does per block.
///
/// **This is NOT the production wipe path, and it does NOT prove the wiped-node
/// system property** (M2 review F5). `derive_maintainer_set` has ZERO production
/// callers. A node whose `maintainer_state.bin` is deleted does not replay at
/// all: `maybe_bootstrap_maintainer_set` re-seeds it from LIVE producer state,
/// which measurably re-arms a governance-removed key (M2 QA PROBE-1:
/// `after_wipe_len=5`, `removed_key_back=true`). Wiring this function into the
/// seed path so that the system property holds is INC-I-172 M3 / **R1** —
/// `docs/.workflow/inc-i-172-M3-scope.md`. What this test buys is that the
/// replay engine R1 will use is already convergent.
#[test]
fn replay_function_converges_with_incremental_application() {
    let kps: Vec<KeyPair> = (0..8).map(|_| KeyPair::generate()).collect();
    let regs_value: Vec<(PublicKey, u64)> = kps.iter().map(|k| (*k.public_key(), 0u64)).collect();
    let regs_reader: Vec<(u64, PublicKey)> = kps.iter().map(|k| (0u64, *k.public_key())).collect();

    let seed = derive_canonical_maintainer_set(&regs_value, 0);
    let target = seed.members[0];
    let signers: Vec<&KeyPair> = kps
        .iter()
        .filter(|k| seed.is_maintainer(k.public_key()) && *k.public_key() != target)
        .take(3)
        .collect();
    let change = remove_change(&target, [signers[0], signers[1], signers[2]]);

    // ONLINE node: seeded once at genesis, then the governance action applied
    // incrementally, exactly as process_transaction_governance does.
    let mut online = seed.clone();
    let msg = change.signing_message(false);
    assert!(
        online.verify_multisig_excluding(&change.signatures, &msg, &change.target),
        "setup: the governance action must be genuinely authorized"
    );
    online
        .remove_maintainer(&change.target, 100)
        .expect("setup: removal must apply on the online node");

    // REPLAYED: one whole-history pass over the same block data.
    let replayed = derive_maintainer_set_at_tip(&Chain {
        registrations: regs_reader,
        changes: vec![(100, MaintainerChange::Remove(change))],
        slashed: vec![],
    });

    assert_eq!(
        replayed.members, online.members,
        "REQ-172-010: a whole-history replay must return the SAME members as \
         incremental per-block application. This is the convergence property of \
         the replay FUNCTION. It is a precondition for M3/R1 (replay on a missing \
         maintainer_state.bin), NOT a demonstration that the running node has \
         that behavior today — today it re-seeds from live producer state."
    );
    assert_eq!(
        replayed.threshold, online.threshold,
        "REQ-172-010: thresholds must converge too"
    );
    assert!(
        !replayed.is_maintainer(&target),
        "REQ-172-010: the replay must not resurrect the removed maintainer"
    );
}
