// INC-I-172 M2 security-audit fix — AUDIT-P1-019.
// `maintainer_state.bin` is node-local, unsigned and attacker-writable given data-dir
// access, and M2's F4 promoted it to the SOLE `ProtocolActivation` authority above the
// gate. `MaintainerState::load` must therefore REJECT a set that is not well formed,
// rather than restore `members` and `threshold` verbatim.
//
// Finding: `docs/.workflow/security-audit-report-M2.md` § AUDIT-P1-019.
// Node-local, not consensus-visible, NO activation height (the file is never gossiped,
// never hashed, and absent from `ChainState::serialize_canonical`).
//
// THE DEFECT, in the report's words: `from_body` is three field copies, so
//   (a) `members: [K,K,K,K,K], threshold: 3` makes `count_distinct_signers` iterate
//       five member SLOTS that are all the same KEY — one signature clears a 3-of-5;
//   (b) `members: <the genuine five>, threshold: 1` makes a single signature authorize
//       anything, and M1's install-path containment compares KEYS ONLY, so the same
//       downgrade reaches the binary-install path too.
// The `set.rs:127-129` comment ("padding the vector with duplicate entries ... cannot
// clear a k-of-n threshold") is true of the SIGNATURE vector and false of the MEMBER
// vector. Nothing enforced distinctness on the load path — this file is that enforcement.
//
// ============================================================================
// OUTPUT CONTRACT
// ============================================================================
// Function under test:
//   `storage::MaintainerState::load(data_dir: &Path) -> Result<MaintainerState, StorageError>`
//   On-disk layout: `MAGIC(4 = b"DMST") || VERSION(u32 LE) || bincode({set, last_derived_height})`
//   for a current file; the bare bincode body alone for a pre-INC-I-172 (legacy) file.
//
// ENUMERATION OF OBSERVABLE OUTPUTS.
//   - mutable params      : NONE (`&Path`).
//   - receiver mutation   : NONE (associated fn).
//   - persistent store    : `load` WRITES on the legacy branch (the eager migration
//                           re-save). A REJECTED file must not be rewritten — asserted
//                           as O4. On every other branch it writes nothing.
//   - return value        : the value channel (O1, O2).
//   - process state       : none. `warn!` is a side channel this harness does not
//                           capture; every fact it carries is asserted through O1-O3.
//
//   O1: Result discriminant       — Ok / Err. The security-load-bearing cell.
//   O2: On Ok, the decoded value  — members / threshold / last_derived_height.
//   O3: On Err, the Display text  — must NAME the file (an operator has to know what to
//                                   look at) and NAME the defect (duplicate member /
//                                   member count / threshold), so the failure is not a
//                                   mystery on a node that will not boot.
//   O4: post-load file state      — a rejected file is NOT repaired and NOT rewritten.
//                                   Silent repair would make the attacker's set the
//                                   authority under a different threshold, which is
//                                   still an attacker-chosen set.
//
// CODE PATHS (both decoders must validate — the attacker chooses the file format):
//   P1: MAGIC present, known version  -> `decode_current`  -> validate
//   P2: no MAGIC (legacy)             -> `migrate_legacy`  -> validate BEFORE the re-save
//
// INPUT PARTITIONS:
//   I1: five member slots, ONE distinct key, threshold 3            [leg (a)]
//   I2: five DISTINCT genuine keys, threshold 1                     [leg (b)]
//   I3: two distinct keys, threshold 1 (sub-majority on a small set)[leg (b), small]
//   I4: six distinct keys (> MAX_MAINTAINERS), threshold reconciled [padding leg]
//   I5: a well-formed five-key set, threshold 3                     [GREEN-lock]
//   I6: the legitimate FRESH node — members [], threshold 0, height 0
//       [GREEN-lock: this is what `MaintainerState::default()` persists]
//   I7: the M1 EMPTIED-root case — members [], threshold 2, height 9000
//       [GREEN-lock: `bins/node/tests/inc_i_172_command_trust_root_test.rs`
//        `f3_an_emptied_on_chain_set_fails_closed_for_operator_commands` requires this
//        file to LOAD. An empty set is refused by `MaintainerSet::is_authorizable`
//        (governance) and by `TrustRoot::is_usable` (install) before its threshold is
//        ever consulted, so reconciling an empty set's threshold protects nothing —
//        while erroring would turn a survivable, already-fail-closed state into a boot
//        failure on a host the operator still has to recover. Empty is carved out on
//        purpose, and this partition is the lock on that carve-out.]
//
// TRUTH TABLE
//  case | path | input | O1  | O2                | O3            | O4
//  -----|------|-------|-----|-------------------|---------------|--------------
//  t01  | P1   | I1    | Err | n/a               | names dup     | unchanged
//  t02  | P2   | I1    | Err | n/a               | names dup     | unchanged
//  t03  | P1   | I2    | Err | n/a               | names thresh  | unchanged
//  t04  | P2   | I2    | Err | n/a               | names thresh  | unchanged
//  t05  | P1   | I3    | Err | n/a               | names thresh  | unchanged
//  t06  | P1   | I4    | Err | n/a               | names count   | unchanged
//  t07  | P1   | I5    | Ok  | 5 keys, thresh 3  | n/a           | unchanged
//  t08  | P2   | I6    | Ok  | 0 keys, thresh 0  | n/a           | migrated
//  t09  | P2   | I7    | Ok  | 0 keys, thresh 2  | n/a           | migrated
//  t10  | P1   | I1    | Err | n/a               | n/a           | one-key-clears-
//                                                                 3-of-5 is what
//                                                                 the reject buys

use std::path::Path;

use doli_core::maintainer::{MaintainerSet, MaintainerSignature, MAX_MAINTAINERS};
use serde::Serialize;
use storage::{MaintainerState, MAINTAINER_STATE_VERSION};

const STATE_FILE: &str = "maintainer_state.bin";

/// The current on-disk magic (private in `crates/storage/src/maintainer.rs`).
/// Duplicated as a literal on purpose — this file is the external contract.
const MAGIC: &[u8; 4] = b"DMST";

/// The persisted BODY shape, reproduced so the fixtures are produced by the same
/// encoder (bincode, same field order) the node itself uses.
#[derive(Serialize)]
struct StateBody {
    set: MaintainerSet,
    last_derived_height: u64,
}

fn key(seed: u8) -> crypto::PrivateKey {
    crypto::PrivateKey::from_bytes([seed; 32])
}

fn pubkey(seed: u8) -> crypto::PublicKey {
    key(seed).public_key()
}

fn body_bytes(set: MaintainerSet, last_derived_height: u64) -> Vec<u8> {
    bincode::serialize(&StateBody {
        set,
        last_derived_height,
    })
    .expect("encoding a fixture body must succeed")
}

/// P2 — a pre-INC-I-172 file: the bare body, no header.
fn legacy_file(set: MaintainerSet, height: u64) -> Vec<u8> {
    body_bytes(set, height)
}

/// P1 — a current-format file: `MAGIC || VERSION || body`.
fn current_file(set: MaintainerSet, height: u64) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&MAINTAINER_STATE_VERSION.to_le_bytes());
    out.extend_from_slice(&body_bytes(set, height));
    out
}

fn write_state_file(dir: &Path, bytes: &[u8]) {
    std::fs::write(dir.join(STATE_FILE), bytes).expect("failed to write the test state file");
}

fn read_state_file(dir: &Path) -> Vec<u8> {
    std::fs::read(dir.join(STATE_FILE)).expect("failed to read back the state file")
}

/// A set built field-by-field, bypassing `with_members` — exactly what an attacker with
/// data-dir write does, and exactly what `from_body` used to restore verbatim.
fn raw_set(members: Vec<crypto::PublicKey>, threshold: usize, last_updated: u64) -> MaintainerSet {
    MaintainerSet {
        members,
        threshold,
        last_updated,
    }
}

/// O3: the message must name the FILE and the DEFECT.
fn assert_error_is_actionable(err: &storage::StorageError, defect: &str, ctx: &str) {
    let text = err.to_string();
    assert!(
        text.contains(STATE_FILE),
        "{ctx}: O3 — the error must name the offending file so an operator knows what \
         to inspect; got: {text}"
    );
    assert!(
        text.to_lowercase().contains(defect),
        "{ctx}: O3 — the error must name the defect (`{defect}`); got: {text}"
    );
}

// ---------------------------------------------------------------------------
// LEG (a) — duplicate members. RED before this fix.
// ---------------------------------------------------------------------------

/// AUDIT-P1-019 leg (a), path P1. [t01]
/// Acceptance: a current-format file whose five member slots hold ONE key is refused.
#[test]
fn p1_019_duplicate_members_are_refused_in_a_current_format_file() {
    let dir = tempfile::tempdir().unwrap();
    let k = pubkey(0xA1);
    write_state_file(
        dir.path(),
        &current_file(raw_set(vec![k; MAX_MAINTAINERS], 3, 100), 100),
    );

    let err = MaintainerState::load(dir.path()).expect_err(
        "five member slots holding ONE key is a 1-of-1 wearing a 3-of-5 costume: \
         count_distinct_signers iterates SLOTS, so one signature satisfies all five. \
         Loading it makes the single security property M2 adds void on this host.",
    );
    assert_error_is_actionable(&err, "duplicate", "t01");
}

/// AUDIT-P1-019 leg (a), path P2. [t02]
/// Acceptance: the LEGACY decoder validates too — the attacker picks the file format,
/// and a file with no magic takes the migration branch instead.
#[test]
fn p1_019_duplicate_members_are_refused_in_a_legacy_file() {
    let dir = tempfile::tempdir().unwrap();
    let k = pubkey(0xA2);
    let original = legacy_file(raw_set(vec![k; MAX_MAINTAINERS], 3, 100), 100);
    write_state_file(dir.path(), &original);

    let err = MaintainerState::load(dir.path()).expect_err(
        "validating only the versioned branch leaves the whole defect reachable: a \
         file with no magic is migrated, and the migration is what re-saves it into \
         the current format",
    );
    assert_error_is_actionable(&err, "duplicate", "t02");

    // O4 — a rejected file must not be repaired or rewritten.
    assert_eq!(
        read_state_file(dir.path()),
        original,
        "t02: O4 — a refused file must be left exactly as found. Repairing it (dedup, \
         or recomputing the threshold) would still install an ATTACKER-CHOSEN member \
         list as this host's authority, just under a different threshold."
    );
}

/// AUDIT-P1-019 leg (a), the capability the refusal removes. [t10]
/// Acceptance: pinned as an executable fact — with the duplicated vector seated, ONE
/// key's signature clears a threshold of 3. This is why loading it is unacceptable;
/// if this assertion ever flips, the load-path check has stopped being load-bearing.
#[test]
fn p1_019_one_key_clears_a_3_of_5_when_the_member_vector_is_padded() {
    let k = key(0xA3);
    let padded = raw_set(vec![k.public_key(); MAX_MAINTAINERS], 3, 1);
    let message = b"add:whatever";
    let one_signature = vec![MaintainerSignature::new(
        k.public_key(),
        crypto::signature::sign(message, &k),
    )];

    assert!(
        padded.verify_multisig(&one_signature, message),
        "t10: fixture sanity — a padded member vector IS satisfied by a single key. \
         That is the capability AUDIT-P1-019 grants and the load-path refusal removes."
    );
}

// ---------------------------------------------------------------------------
// LEG (b) — an unreconciled threshold. RED before this fix.
// ---------------------------------------------------------------------------

/// AUDIT-P1-019 leg (b), path P1. [t03]
/// Acceptance: the GENUINE five with `threshold: 1` is refused. This leg needs no key
/// theft at all — it downgrades the quorum of an honest, freshly rotated set, and M1's
/// install-path containment compares KEYS ONLY, so it passes containment.
#[test]
fn p1_019_a_downgraded_threshold_is_refused_even_with_genuine_members() {
    let dir = tempfile::tempdir().unwrap();
    let members: Vec<_> = (0..5).map(|i| pubkey(0xB0 + i)).collect();
    write_state_file(dir.path(), &current_file(raw_set(members, 1, 700), 700));

    let err = MaintainerState::load(dir.path()).expect_err(
        "five genuine keys with threshold 1 is a 1-of-5: one signature authorizes \
         governance AND a root binary install, while getMaintainerSet still reports \
         the right five members",
    );
    assert_error_is_actionable(&err, "threshold", "t03");
}

/// AUDIT-P1-019 leg (b), path P2. [t04]
#[test]
fn p1_019_a_downgraded_threshold_is_refused_in_a_legacy_file() {
    let dir = tempfile::tempdir().unwrap();
    let members: Vec<_> = (0..5).map(|i| pubkey(0xC0 + i)).collect();
    let original = legacy_file(raw_set(members, 1, 700), 700);
    write_state_file(dir.path(), &original);

    let err = MaintainerState::load(dir.path())
        .expect_err("the legacy branch must reconcile the threshold too");
    assert_error_is_actionable(&err, "threshold", "t04");
    assert_eq!(
        read_state_file(dir.path()),
        original,
        "t04: O4 — no silent repair"
    );
}

/// AUDIT-P1-019 leg (b), small set. [t05]
/// Acceptance: the reconciliation is against `calculate_threshold(len)`, not merely
/// against `threshold >= 1`. Two members require TWO signatures.
#[test]
fn p1_019_a_sub_majority_threshold_is_refused_on_a_small_set() {
    let dir = tempfile::tempdir().unwrap();
    write_state_file(
        dir.path(),
        &current_file(raw_set(vec![pubkey(0xD1), pubkey(0xD2)], 1, 40), 40),
    );

    let err = MaintainerState::load(dir.path()).expect_err(
        "MaintainerSet::calculate_threshold(2) is 2. Accepting 1 would leave a \
         is_authorizable()-passing 1-of-2 on disk",
    );
    assert_error_is_actionable(&err, "threshold", "t05");
}

// ---------------------------------------------------------------------------
// The padding leg — more members than the type admits. RED before this fix.
// ---------------------------------------------------------------------------

/// AUDIT-P1-019 padding leg. [t06]
/// Acceptance: a member list longer than `MAX_MAINTAINERS` is refused. Both live
/// derivations cap at `INITIAL_MAINTAINER_COUNT` (`derivation.rs` and
/// `periodic.rs`) and `add_maintainer` refuses at the cap, so an over-long list can
/// only have been written by hand. Left accepted, it is the same quorum collapse with
/// a self-consistent threshold: 6 members with a threshold of 4 hands anyone who
/// supplies 4 of those keys the whole governance path.
#[test]
fn p1_019_more_members_than_max_maintainers_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let members: Vec<_> = (0..(MAX_MAINTAINERS as u8 + 1))
        .map(|i| pubkey(0xE0 + i))
        .collect();
    let threshold = MaintainerSet::calculate_threshold(members.len());
    write_state_file(
        dir.path(),
        &current_file(raw_set(members, threshold, 12), 12),
    );

    let err = MaintainerState::load(dir.path())
        .expect_err("a set larger than MAX_MAINTAINERS is unreachable by any live path");
    assert_error_is_actionable(&err, "member", "t06");
}

// ---------------------------------------------------------------------------
// GREEN-LOCKS — the states that MUST still load. A check that bricks a node is not
// a security control, it is the INC-I-153 failure class delivered through the file.
// ---------------------------------------------------------------------------

/// [t07] A well-formed set is untouched by the new validation.
#[test]
fn a_well_formed_set_still_loads_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let members: Vec<_> = (0..5).map(|i| pubkey(0x10 + i)).collect();
    let set = MaintainerSet::with_members(members.clone(), 4242);
    assert_eq!(set.threshold, 3, "fixture sanity: 5 members ⇒ threshold 3");
    write_state_file(dir.path(), &current_file(set, 4242));

    let loaded = MaintainerState::load(dir.path())
        .expect("the honest steady state must load; refusing it would brick every node");
    assert_eq!(loaded.set.members, members, "t07: O2 — members preserved");
    assert_eq!(loaded.set.threshold, 3, "t07: O2 — threshold preserved");
    assert_eq!(loaded.last_derived_height, 4242);
}

/// [t08] The legitimate fresh node: `MaintainerState::default()` persists an EMPTY set
/// with `threshold: 0`. `calculate_threshold(0)` is `MAINTAINER_THRESHOLD` (3), so a
/// blanket reconciliation would refuse the one state every new node starts from.
#[test]
fn the_fresh_empty_state_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    write_state_file(dir.path(), &legacy_file(MaintainerSet::new(), 0));

    let loaded = MaintainerState::load(dir.path()).expect(
        "an empty set with threshold 0 is what MaintainerState::default() writes; \
         refusing it would refuse every fresh node",
    );
    assert!(loaded.set.members.is_empty(), "t08: O2");
    assert_eq!(loaded.set.threshold, 0, "t08: O2 — preserved verbatim");
}

/// [t09] The M1 EMPTIED-root case. `bins/node/tests/inc_i_172_command_trust_root_test.rs`
/// requires this exact file — `members: []`, a threshold left over from before the
/// clear, a real derivation height — to LOAD, so the operator commands resolve it to an
/// unusable `OnChain` root instead of falling back to the compiled keys.
///
/// The empty carve-out costs nothing: `MaintainerSet::is_authorizable` short-circuits on
/// `!members.is_empty()` and `TrustRoot::is_usable` needs `keys.len() >= threshold`, so
/// an empty set authorizes nothing at ANY threshold value.
#[test]
fn the_m1_emptied_root_case_still_loads_and_stays_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let mut set = MaintainerSet::with_members(vec![pubkey(111), pubkey(112)], 8);
    set.members.clear(); // threshold 2 survives the clear — the M1 fixture, verbatim
    assert_eq!(
        set.threshold, 2,
        "fixture sanity: the stale threshold survives"
    );

    write_state_file(dir.path(), &legacy_file(set, 9_000));

    let loaded = MaintainerState::load(dir.path()).expect(
        "M1 chose 'load it, then refuse to use it' over 'refuse to boot' on purpose: \
         an emptied trust root must not become an unrecoverable host",
    );
    assert!(loaded.set.members.is_empty(), "t09: O2");
    assert!(
        !loaded.set.is_authorizable(),
        "t09: the empty set must still authorize nothing — that is what makes the \
         carve-out safe"
    );
}

/// [t07/t08] Encoder/decoder parity: the node's own `save` output must survive its own
/// `load` for every set the live paths can produce. A validation that rejects what the
/// writer writes is a self-inflicted outage.
#[test]
fn every_set_the_live_paths_produce_round_trips_through_save_and_load() {
    for n in 0..=MAX_MAINTAINERS {
        let dir = tempfile::tempdir().unwrap();
        let members: Vec<_> = (0..n as u8).map(|i| pubkey(0x70 + i)).collect();
        let set = if n == 0 {
            MaintainerSet::new()
        } else {
            MaintainerSet::with_members(members, 5)
        };
        let state = MaintainerState {
            version: MAINTAINER_STATE_VERSION,
            set: set.clone(),
            last_derived_height: 5,
        };
        state.save(dir.path()).expect("save must succeed");

        let loaded = MaintainerState::load(dir.path())
            .unwrap_or_else(|e| panic!("{n}-member set written by save() must load: {e}"));
        assert_eq!(
            loaded.set, set,
            "{n}-member set must round-trip field-for-field"
        );
    }
}
