//! INC-I-172 M2 — CATEGORY A: reproduction tests for the maintainer-governance
//! authorization defects. These compile and RUN against the CURRENT tree and
//! MUST FAIL until M2 lands.
//!
//! Findings under test
//! -------------------
//! * **AUDIT-P0-010** (P0) — `MaintainerSet::verify_multisig` /
//!   `verify_multisig_excluding` (`crates/core/src/maintainer.rs:145-159,165-188`)
//!   count signature **ENTRIES** via `.filter(..).count()`, never distinct
//!   signers. THREE entries produced by ONE maintainer key therefore satisfy a
//!   "3-of-5" threshold. The mainnet-live correct shape is the covenant
//!   distinct-signer loop at `crates/core/src/conditions/eval.rs:51-68`
//!   (outer loop over expected keys, inner over witnesses, `break` on match).
//! * **AUDIT-P1-010 / FM-02** (P1) — `MaintainerSet::calculate_threshold(0) == 0`
//!   (`maintainer.rs:125-135`) makes `valid_count >= self.threshold` VACUOUS:
//!   an empty set accepts a **zero-signature** `AddMaintainer`. The
//!   distinct-signer fix alone does NOT repair this (`0 >= 0` still passes).
//!
//! Requirements
//! ------------
//! * REQ-172-012 (Must) — a k-of-n threshold must mean k DISTINCT signers on
//!   `AddMaintainer`, `RemoveMaintainer` and `ProtocolActivation`.
//! * REQ-172-002 (Must) — no authorization path may be satisfiable by a single
//!   key, and no state may make authorization vacuous.
//!
//! Spec: `specs/maintainer-trust-root-architecture.md` §F3.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! Functions under test (all pure — `&self` or associated; no mutable params,
//! no receiver mutation, no persistent-store writes, no channels):
//!
//!   F1: `MaintainerSet::verify_multisig(&self, &[MaintainerSignature], &[u8]) -> bool`
//!   F2: `MaintainerSet::verify_multisig_excluding(&self, &[MaintainerSignature], &[u8], &PublicKey) -> bool`
//!   F3: `MaintainerSet::calculate_threshold(usize) -> usize`
//!   F4: `derive_maintainer_set<R: BlockchainReader>(&R) -> MaintainerSet`
//!       (the replay path REQ-172-010 revives; it consumes F1/F2, so the
//!        vacuity defect is reachable end-to-end through it)
//!
//! OUTPUTS
//!   O1 (F1 return: bool)
//!   O2 (F2 return: bool)
//!   O3 (F3 return: usize)
//!   O4 (F4 return: MaintainerSet) — observed as `.members` and `.threshold`
//!   O5 (mutable params)            — NONE for F1..F4 (all take `&self`/values)
//!   O6 (receiver mutation)         — NONE for F1..F4 (all `&self`/associated)
//!   O7 (persistent store writes)   — NONE (crates/core has no storage edge)
//!
//! PATHS
//!   PA-accept  — the function returns `true` / applies the change
//!   PA-reject  — the function returns `false` / leaves the set untouched
//!   PT-zero    — F3 with member_count == 0
//!   PT-nonzero — F3 with member_count >= 1
//!
//! INPUT PARTITIONS
//!   IP-1  5-member set, 3 entries from 3 DISTINCT members    -> PA-accept  (control, GREEN today)
//!   IP-2  5-member set, 3 entries from ONE member (repeated) -> PA-reject  (RED today)
//!   IP-3  5-member set, 3 entries = 2 distinct + 1 repeat    -> PA-reject  (RED today)
//!   IP-4  5-member set, 3 entries from ONE NON-member        -> PA-reject  (control, GREEN today)
//!   IP-5  F2: 3 entries from ONE non-excluded member         -> PA-reject  (RED today)
//!   IP-6  F2: 3 entries from 3 distinct non-excluded members -> PA-accept  (control, GREEN today)
//!   IP-7  F2: 3 entries all from the EXCLUDED member         -> PA-reject  (control, GREEN today)
//!   IP-8  0-member set, ZERO signature entries               -> PA-reject  (RED today, FM-02)
//!   IP-9  0-member set via F2, ZERO entries                  -> PA-reject  (RED today, FM-02)
//!   IP-10 F3 member_count == 0                               -> PT-zero, MUST NOT be 0 (RED today)
//!   IP-11 F3 member_count in 1..=7                           -> PT-nonzero (control, GREEN today)
//!   IP-12 F4 with zero registrations + one zero-signature
//!         `Add` change                                       -> PA-reject  (RED today, FM-02 end-to-end)
//!   IP-13 F4 with 5 registrations + one 3-entry single-key
//!         `Add` change                                       -> PA-reject  (RED today, P0 end-to-end)
//!   IP-14 F4 with 5 registrations + one 3-DISTINCT-signer
//!         `Add` change                                       -> PA-accept  (control, GREEN today)
//!
//! MATRIX (every cell asserted)
//!   O1 x {IP-1, IP-2, IP-3, IP-4, IP-8}          = 5 assertions
//!   O2 x {IP-5, IP-6, IP-7, IP-9}                = 4 assertions
//!   O3 x {IP-10, IP-11(7 counts)}                = 8 assertions
//!   O4 x {IP-12, IP-13, IP-14} x {members, threshold} = 6 assertions
//!   O5/O6/O7 — structurally absent, nothing to assert.
//!
//! ANTI-VACUITY PAIRING (each RED partition has a GREEN control differing in
//! exactly one input):
//!   IP-2 <-> IP-1   (same 3 entries, same set; only signer DISTINCTNESS differs)
//!   IP-3 <-> IP-1   (same count 3; 2 distinct vs 3 distinct)
//!   IP-5 <-> IP-6   (same exclusion, same count; distinctness differs)
//!   IP-8 <-> IP-1   (same call; member count 0 vs 5)
//!   IP-13 <-> IP-14 (byte-identical reader except signer distinctness)

use crypto::{KeyPair, PublicKey};
use doli_core::maintainer::{
    derive_maintainer_set, BlockchainReader, MaintainerChange, MaintainerChangeData, MaintainerSet,
    MaintainerSignature,
};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Five maintainer keypairs plus one outsider, generated once per test.
struct Fixture {
    members: Vec<KeyPair>,
    outsider: KeyPair,
}

impl Fixture {
    fn new() -> Self {
        Self {
            members: (0..5).map(|_| KeyPair::generate()).collect(),
            outsider: KeyPair::generate(),
        }
    }

    fn pubkeys(&self) -> Vec<PublicKey> {
        self.members.iter().map(|k| *k.public_key()).collect()
    }

    /// A 5-member set. `MaintainerSet::with_members` derives `threshold = 3`.
    fn set(&self) -> MaintainerSet {
        MaintainerSet::with_members(self.pubkeys(), 0)
    }
}

/// One genuine signature entry from `kp` over `message`.
fn sig(kp: &KeyPair, message: &[u8]) -> MaintainerSignature {
    MaintainerSignature::new(
        *kp.public_key(),
        crypto::signature::sign(message, kp.private_key()),
    )
}

/// `n` signature entries, ALL produced by the same key over the same message.
///
/// This is the attacker's payload: cryptographically these are `n` valid
/// Ed25519 signatures, indistinguishable one-by-one from an honest quorum. Only
/// a DISTINCT-SIGNER counter can tell them apart.
fn repeated(kp: &KeyPair, message: &[u8], n: usize) -> Vec<MaintainerSignature> {
    (0..n).map(|_| sig(kp, message)).collect()
}

// ---------------------------------------------------------------------------
// F1 — verify_multisig
// ---------------------------------------------------------------------------

/// IP-1 (control). O1 x PA-accept. Three DISTINCT maintainers must authorize.
/// Proves the harness produces signatures the production verifier accepts, so a
/// `false` in the RED tests below cannot be a harness artefact.
#[test]
fn control_three_distinct_maintainers_authorize() {
    let f = Fixture::new();
    let set = f.set();
    let msg = b"add:deadbeef";

    let sigs = vec![
        sig(&f.members[0], msg),
        sig(&f.members[1], msg),
        sig(&f.members[2], msg),
    ];

    assert!(
        set.verify_multisig(&sigs, msg),
        "CONTROL: 3 distinct maintainers over a 3-of-5 threshold MUST authorize"
    );
}

/// IP-2. O1 x PA-reject. **AUDIT-P0-010 / REQ-172-012 — MUST FAIL TODAY.**
///
/// One compromised key, replayed three times, currently satisfies "3-of-5".
#[test]
fn three_entries_from_one_key_must_not_satisfy_threshold_three() {
    let f = Fixture::new();
    let set = f.set();
    let msg = b"add:deadbeef";

    assert_eq!(set.threshold, 3, "setup: a 5-member set has threshold 3");

    let sigs = repeated(&f.members[0], msg, 3);
    assert_eq!(sigs.len(), 3, "setup: exactly 3 signature ENTRIES");

    assert!(
        !set.verify_multisig(&sigs, msg),
        "AUDIT-P0-010: 3 signature ENTRIES from ONE maintainer key are 1 DISTINCT \
         signer, not 3. verify_multisig counts entries (.filter().count()), so a \
         single compromised key currently satisfies a 3-of-5 threshold and can \
         drive AddMaintainer/RemoveMaintainer/ProtocolActivation alone."
    );
}

/// IP-3. O1 x PA-reject. **AUDIT-P0-010 — MUST FAIL TODAY.**
///
/// The partial case: 2 distinct keys padded to 3 entries. Distinct signers = 2 < 3.
#[test]
fn two_distinct_signers_padded_to_three_entries_must_not_satisfy_threshold_three() {
    let f = Fixture::new();
    let set = f.set();
    let msg = b"remove:cafebabe";

    let sigs = vec![
        sig(&f.members[0], msg),
        sig(&f.members[1], msg),
        sig(&f.members[0], msg), // pad — same key as entry 0
    ];

    assert!(
        !set.verify_multisig(&sigs, msg),
        "AUDIT-P0-010: 2 DISTINCT signers must not clear a 3-of-5 threshold by \
         padding the vector with a duplicate entry"
    );
}

/// IP-4 (control). O1 x PA-reject. Non-members are already rejected — this
/// isolates the defect to DISTINCTNESS, not to membership checking.
#[test]
fn control_repeated_non_member_signature_is_rejected() {
    let f = Fixture::new();
    let set = f.set();
    let msg = b"add:deadbeef";

    let sigs = repeated(&f.outsider, msg, 5);

    assert!(
        !set.verify_multisig(&sigs, msg),
        "CONTROL: the membership filter already rejects non-maintainers; the \
         defect is the missing DISTINCTNESS check, nothing else"
    );
}

// ---------------------------------------------------------------------------
// F2 — verify_multisig_excluding
// ---------------------------------------------------------------------------

/// IP-6 (control). O2 x PA-accept.
#[test]
fn control_three_distinct_non_excluded_maintainers_authorize_removal() {
    let f = Fixture::new();
    let set = f.set();
    let msg = b"remove:target";
    let excluded = *f.members[4].public_key();

    let sigs = vec![
        sig(&f.members[0], msg),
        sig(&f.members[1], msg),
        sig(&f.members[2], msg),
    ];

    assert!(
        set.verify_multisig_excluding(&sigs, msg, &excluded),
        "CONTROL: 3 distinct non-excluded maintainers MUST authorize a removal"
    );
}

/// IP-5. O2 x PA-reject. **AUDIT-P0-010 / REQ-172-012 — MUST FAIL TODAY.**
///
/// The removal path is the dangerous one: one key can evict every other
/// maintainer one transaction at a time.
#[test]
fn three_entries_from_one_key_must_not_authorize_a_removal() {
    let f = Fixture::new();
    let set = f.set();
    let msg = b"remove:target";
    let excluded = *f.members[4].public_key();

    let sigs = repeated(&f.members[0], msg, 3);

    assert!(
        !set.verify_multisig_excluding(&sigs, msg, &excluded),
        "AUDIT-P0-010: verify_multisig_excluding also counts ENTRIES, so ONE \
         surviving key can evict the rest of the maintainer set one \
         RemoveMaintainer at a time"
    );
}

/// IP-7 (control). O2 x PA-reject. The exclusion filter itself still works.
#[test]
fn control_excluded_maintainer_cannot_authorize_own_removal() {
    let f = Fixture::new();
    let set = f.set();
    let msg = b"remove:self";
    let excluded = *f.members[0].public_key();

    let sigs = repeated(&f.members[0], msg, 5);

    assert!(
        !set.verify_multisig_excluding(&sigs, msg, &excluded),
        "CONTROL: the excluded maintainer's own entries are filtered out"
    );
}

// ---------------------------------------------------------------------------
// F3 / FM-02 — the vacuous threshold
// ---------------------------------------------------------------------------

/// IP-10. O3 x PT-zero. **AUDIT-P1-010 / FM-02 — MUST FAIL TODAY.**
#[test]
fn threshold_for_an_empty_set_must_never_be_zero() {
    assert_ne!(
        MaintainerSet::calculate_threshold(0),
        0,
        "AUDIT-P1-010 / FM-02: calculate_threshold(0) == 0 makes the guard \
         `valid_count >= self.threshold` VACUOUS. An empty set (fresh install, \
         wiped data dir, snap-synced node, or a MaintainerState decode that \
         degraded to default) accepts a ZERO-signature AddMaintainer. The \
         distinct-signer fix does not repair this: 0 >= 0 still passes."
    );
}

/// IP-11 (control). O3 x PT-nonzero. Pins the majority table so a fix to
/// `calculate_threshold(0)` cannot silently move any populated value.
#[test]
fn control_threshold_table_for_populated_sets_is_unchanged() {
    assert_eq!(MaintainerSet::calculate_threshold(1), 1);
    assert_eq!(MaintainerSet::calculate_threshold(2), 2);
    assert_eq!(MaintainerSet::calculate_threshold(3), 2);
    assert_eq!(MaintainerSet::calculate_threshold(4), 3);
    assert_eq!(MaintainerSet::calculate_threshold(5), 3);
    assert_eq!(MaintainerSet::calculate_threshold(6), 4);
    assert_eq!(MaintainerSet::calculate_threshold(7), 4);
}

/// IP-8. O1 x PA-reject. **AUDIT-P1-010 / FM-02 — MUST FAIL TODAY.**
#[test]
fn empty_set_must_not_accept_zero_signatures() {
    let set = MaintainerSet::new();
    assert_eq!(set.member_count(), 0, "setup: the set is empty");

    let msg = b"add:attacker_key";

    assert!(
        !set.verify_multisig(&[], msg),
        "FM-02: an EMPTY maintainer set currently accepts an AddMaintainer that \
         carries ZERO signatures, because threshold == 0. That is the whole \
         install-trust-root takeover in one transaction on any node whose set \
         is empty when it applies the block."
    );
}

/// IP-9. O2 x PA-reject. **FM-02 — MUST FAIL TODAY.** Same vacuity on the
/// removal path.
#[test]
fn empty_set_must_not_accept_zero_signatures_on_removal_path() {
    let set = MaintainerSet::new();
    let msg = b"remove:anything";
    let target = *KeyPair::generate().public_key();

    assert!(
        !set.verify_multisig_excluding(&[], msg, &target),
        "FM-02: verify_multisig_excluding is vacuous on an empty set for the \
         same reason (0 >= 0)"
    );
}

// ---------------------------------------------------------------------------
// F4 — the same defects, reached END TO END through the public replay API
// ---------------------------------------------------------------------------

/// Replay the WHOLE chain (`up_to_height`), under the POST-activation
/// distinct-signer rule for all of it (`activation_height == 0`).
///
/// INC-I-172 M2 review F1 gave `derive_maintainer_set` a height bound and an
/// activation height, so it verifies each governance action under the rule that
/// was in force AT that action's height instead of applying today's rule to all
/// of history. These F4 tests assert the post-activation rule, which is exactly
/// what they asserted before the parameters existed — so no assertion changes.
const REPLAY_TO_TIP: u64 = u64::MAX;
const REPLAY_ALL_POST_ACTIVATION: u64 = 0;

/// Minimal `BlockchainReader` over explicit vectors — no HashMap, so the
/// replay input order is fully caller-controlled.
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

/// IP-12. O4 x PA-reject. **FM-02 end to end — MUST FAIL TODAY.**
///
/// A chain with NO registrations (the wiped/snap-synced shape) plus a single
/// `AddMaintainer` carrying an EMPTY signature vector installs the attacker.
#[test]
fn replay_must_not_install_a_maintainer_from_a_zero_signature_add() {
    let attacker = KeyPair::generate();
    let data = MaintainerChangeData::new(*attacker.public_key(), vec![]);

    let chain = Chain {
        registrations: vec![],
        changes: vec![(10, MaintainerChange::Add(data))],
        slashed: vec![],
    };

    let set = derive_maintainer_set(&chain, REPLAY_TO_TIP, REPLAY_ALL_POST_ACTIVATION);

    assert!(
        !set.is_maintainer(attacker.public_key()),
        "FM-02: replaying a chain with no registrations and ONE zero-signature \
         AddMaintainer must not install the attacker as a maintainer"
    );
    assert_eq!(
        set.member_count(),
        0,
        "FM-02: the derived set must stay empty"
    );
}

/// IP-13. O4 x PA-reject. **AUDIT-P0-010 end to end — MUST FAIL TODAY.**
#[test]
fn replay_must_not_install_a_maintainer_from_one_key_signing_three_times() {
    let f = Fixture::new();
    let attacker_target = KeyPair::generate();

    let mut data = MaintainerChangeData::new(*attacker_target.public_key(), vec![]);
    let msg = data.signing_message(true);
    data.signatures = repeated(&f.members[0], &msg, 3);

    // The set is already at MAX_MAINTAINERS after 5 registrations, so first
    // remove one with the same single-key payload, then add. Both steps are
    // authorized today by ONE key.
    let victim = *f.members[4].public_key();
    let mut remove_data = MaintainerChangeData::new(victim, vec![]);
    let remove_msg = remove_data.signing_message(false);
    remove_data.signatures = repeated(&f.members[0], &remove_msg, 3);

    let chain = Chain {
        registrations: f
            .pubkeys()
            .into_iter()
            .enumerate()
            .map(|(i, k)| (i as u64, k))
            .collect(),
        changes: vec![
            (10, MaintainerChange::Remove(remove_data)),
            (11, MaintainerChange::Add(data)),
        ],
        slashed: vec![],
    };

    let set = derive_maintainer_set(&chain, REPLAY_TO_TIP, REPLAY_ALL_POST_ACTIVATION);

    assert!(
        set.is_maintainer(&victim),
        "AUDIT-P0-010: ONE key signing three times must not evict a maintainer"
    );
    assert!(
        !set.is_maintainer(attacker_target.public_key()),
        "AUDIT-P0-010: ONE key signing three times must not install a maintainer"
    );
    assert_eq!(
        set.member_count(),
        5,
        "AUDIT-P0-010: the replayed set must be untouched by a single-key payload"
    );
}

/// IP-14 (control). O4 x PA-accept. Byte-identical to IP-13 except that the
/// three signature entries come from three DISTINCT maintainers. A genuine
/// quorum must still be able to rotate the set — otherwise the M2 fix would be
/// a liveness break, not a security fix.
#[test]
fn control_replay_applies_a_genuine_three_distinct_signer_rotation() {
    let f = Fixture::new();
    let newcomer = KeyPair::generate();
    let victim = *f.members[4].public_key();

    let mut remove_data = MaintainerChangeData::new(victim, vec![]);
    let remove_msg = remove_data.signing_message(false);
    remove_data.signatures = vec![
        sig(&f.members[0], &remove_msg),
        sig(&f.members[1], &remove_msg),
        sig(&f.members[2], &remove_msg),
    ];

    let mut add_data = MaintainerChangeData::new(*newcomer.public_key(), vec![]);
    let add_msg = add_data.signing_message(true);
    add_data.signatures = vec![
        sig(&f.members[0], &add_msg),
        sig(&f.members[1], &add_msg),
        sig(&f.members[2], &add_msg),
    ];

    let chain = Chain {
        registrations: f
            .pubkeys()
            .into_iter()
            .enumerate()
            .map(|(i, k)| (i as u64, k))
            .collect(),
        changes: vec![
            (10, MaintainerChange::Remove(remove_data)),
            (11, MaintainerChange::Add(add_data)),
        ],
        slashed: vec![],
    };

    let set = derive_maintainer_set(&chain, REPLAY_TO_TIP, REPLAY_ALL_POST_ACTIVATION);

    assert!(
        !set.is_maintainer(&victim),
        "CONTROL: a genuine 3-distinct-signer quorum MUST be able to remove"
    );
    assert!(
        set.is_maintainer(newcomer.public_key()),
        "CONTROL: a genuine 3-distinct-signer quorum MUST be able to add"
    );
    assert_eq!(set.member_count(), 5, "CONTROL: 5 - 1 + 1 == 5");
}
