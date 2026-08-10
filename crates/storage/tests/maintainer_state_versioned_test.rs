// INC-I-172 M1 — `MaintainerState` must decode FAIL-CLOSED, never to an empty root,
// and must MIGRATE a pre-INC-I-172 file rather than refuse to boot on it.
// REQ-172-005 (Must), REQ-172-011 (Must)
//
// Contract: `docs/.workflow/inc-i-172-M1-api-contract.md` **§9** (RUNNER CORRECTION,
// binding, supersedes §4). Design: `specs/maintainer-trust-root-architecture.md` F5.
//
// WHY §9 EXISTS (recorded here so nobody "restores" the earlier assertions): the
// earlier §4 wording made a legacy file an `Err`, and `run.rs` makes a load `Err`
// fatal. A real `maintainer_state.bin` from a live node begins
// `05 00 00 00 00 00 00 00` — bincode's `u64` length prefix for a 5-member
// `set.members`, NOT a version tag. Every node that ever bootstrapped a maintainer set
// would have refused to start after an upgrade delivered by the auto-updater itself
// (the INC-I-153 failure class). The security property F5 actually requires is "never
// SILENTLY become an EMPTY root" — and decoding a legacy file into its real, non-empty
// set does not violate it. That property is still asserted explicitly below, in every
// error cell, by `assert_not_silently_defaulted`.
//
// ============================================================================
// OUTPUT CONTRACT
// ============================================================================
// Function under test:
//   `storage::MaintainerState::load(data_dir: &Path) -> Result<MaintainerState, StorageError>`
//   Paired encoder: `MaintainerState::save(&self, data_dir: &Path)`.
//   On-disk layout (§9): `MAGIC(4 = b"DMST") || VERSION(u32 LE) || bincode({set, last_derived_height})`.
//
// ENUMERATION OF OBSERVABLE OUTPUTS.
//   - mutable params      : NONE (`&Path`).
//   - receiver mutation   : NONE (associated fn).
//   - persistent store    : **CHANGED BY §9.** `load` is no longer read-only: on the
//     LEGACY branch it eagerly re-saves the migrated file. That write is an observable
//     output and is asserted (O5). On every other branch it writes nothing.
//   - return value        : the value channel.
//   - process state       : none (no exit, no env). The `warn!` migration line is a
//     side channel this harness cannot capture without a process-global subscriber;
//     the same four facts (path, members, threshold, height) are carried in the
//     returned value and are asserted there instead.
//
//   O1: Result discriminant                — Ok / Err.
//   O2: On Ok, the decoded value           — members / threshold / last_updated /
//                                            last_derived_height. The security-load-
//                                            bearing cell: an `Ok` carrying
//                                            `MaintainerState::default()` (0 members,
//                                            threshold 0) is the FAIL-OPEN outcome —
//                                            it re-arms the compiled leaked keys (FM-06)
//                                            and makes a zero-signature `AddMaintainer`
//                                            acceptable (FM-02).
//   O3: On Err, the Display text           — must NAME the file so an operator knows
//                                            what to look at; on the unknown-version
//                                            branch it must also state the version.
//   O4: encoder/decoder parity             — save(x) then load() == x, field for field
//                                            (CLAUDE.md: "Any encoder/decoder pair MUST
//                                            be verified for index parity").
//   O5: post-migration file state          — after a legacy load, the file on disk
//                                            carries the magic, and a SECOND load takes
//                                            the current-format path (one-shot).
//
// CODE PATHS (of `load`, §9's four branches):
//   P1: file ABSENT                        -> Ok(default())          [legitimate fresh node]
//   P2: no magic, decodes as legacy        -> Ok(MIGRATED, preserved) + re-save
//   P3: no magic, does NOT decode          -> Err                    [torn write, bad disk]
//   P4: magic + KNOWN version              -> Ok(value)              [steady state]
//   P5: magic + UNKNOWN version            -> Err (loud, fail-closed)[forward-only]
//
// INPUT PARTITIONS:
//   I1: legacy bytes with a POPULATED set (3 members, threshold 2, height 4242).
//       The realistic upgrade-day file, and the one whose loss would be maximally
//       damaging.
//   I2: legacy bytes with an EMPTY set (height 0). The all-zero-prefix shape.
//   I3: truncated bytes, short garbage, and a zero-byte file.
//   I4: a populated current-format value for round-trip.
//   I5: **the aliasing partition.** A legacy file whose member count is EXACTLY
//       `MAINTAINER_STATE_VERSION` (1). Under a bare-integer tag its length prefix
//       `01 00 00 00` reads as "version 1" and the file is misparsed as current-format.
//       This is the partition that a magic-less scheme is provably blind to, and it is
//       the reason §9 mandates a magic rather than a wider integer.
//   I6: **real bytes from a live node.** `~/testnet/seed/data/maintainer_state.bin`,
//       232 bytes, embedded verbatim (see `REAL_LEGACY_STATE_HEX`). Synthetic legacy
//       bytes are generated by THIS binary's `MaintainerSet` schema; only bytes written
//       by an older binary prove the migration works on what is actually on disk.
//   Rationale for exactly these six: the decision is now a three-term predicate
//   (magic present? / version known? / body decodes?). I4 is the positive control for
//   all three; I5 varies the first term at its single aliasing point; I1+I2 vary the
//   body content while the first term is false; I3 varies the third term; I6 replaces
//   the generator itself. A partition varying member CONTENT alone cannot change any
//   term and is provably blind to the defect.
//
// MATRIX (every cell asserted by the test named in it):
//
//  path | partition | O1  | O2                        | O3                    | O4  | O5
//  -----|-----------|-----|---------------------------|-----------------------|-----|----
//  P1   | n/a       | Ok  | == default()              | n/a                   | n/a | n/a [t01]
//  P2   | I1        | Ok  | == written, preserved     | n/a                   | n/a | yes [t02]
//  P2   | I2        | Ok  | empty set PRESERVED       | n/a                   | n/a | yes [t03]
//  P3   | I3        | Err | MUST NOT be default()     | names file            | n/a | n/a [t04]
//  P4   | I4        | Ok  | == written value          | n/a                   | eq  | n/a [t05]
//  P5   | I4'       | Err | MUST NOT be default()     | names file + version  | n/a | n/a [t06]
//  P2   | I5        | Ok  | 1 member PRESERVED        | n/a                   | n/a | yes [t07]
//  P2   | I6        | Ok  | 5 members, threshold 3    | n/a                   | n/a | yes [t08]
//
// 8 tests, 8 matrix rows, every cell asserted.
// ============================================================================

use std::path::Path;

use doli_core::maintainer::MaintainerSet;
use serde::Serialize;
use storage::{MaintainerState, MAINTAINER_STATE_VERSION};

const STATE_FILE: &str = "maintainer_state.bin";

/// The current on-disk magic (`crates/storage/src/maintainer.rs`, private there).
/// Duplicated as a literal on purpose: this file is the external contract, so it must
/// break if the magic ever changes silently.
const MAGIC: &[u8; 4] = b"DMST";

/// The EXACT pre-INC-I-172 on-disk shape of `MaintainerState`, reproduced here so
/// the legacy bytes are generated by the same encoder (bincode, same field order)
/// that wrote them on every live node — not hand-rolled and not guessed.
/// See crates/storage/src/maintainer.rs:21-27 as of commit f2b66c19.
#[derive(Serialize)]
struct LegacyMaintainerState {
    set: MaintainerSet,
    last_derived_height: u64,
}

fn pubkey(seed: u8) -> crypto::PublicKey {
    crypto::PrivateKey::from_bytes([seed; 32]).public_key()
}

/// Write raw bytes as `<dir>/maintainer_state.bin`.
fn write_state_file(dir: &Path, bytes: &[u8]) {
    std::fs::write(dir.join(STATE_FILE), bytes).expect("failed to write test state file");
}

fn read_state_file(dir: &Path) -> Vec<u8> {
    std::fs::read(dir.join(STATE_FILE)).expect("failed to read back the state file")
}

fn legacy_bytes(set: MaintainerSet, last_derived_height: u64) -> Vec<u8> {
    bincode::serialize(&LegacyMaintainerState {
        set,
        last_derived_height,
    })
    .expect("legacy encode must succeed")
}

/// Assert a loaded state is NOT the empty default. Factored out because "returned
/// Ok(default())" is the single fail-open outcome this whole file exists to forbid,
/// and it must be reported in those words wherever it happens.
fn assert_not_silently_defaulted(loaded: &MaintainerState, ctx: &str) {
    assert!(
        !(loaded.set.members.is_empty()
            && loaded.set.threshold == 0
            && loaded.last_derived_height == 0),
        "{ctx}: load() returned MaintainerState::default() (0 members, threshold 0). \
         That is the FAIL-OPEN outcome: an empty root re-arms the compiled bootstrap \
         keys for release verification (FM-06) and makes threshold 0 vacuously \
         satisfiable by a zero-signature AddMaintainer (FM-02). It must be a loud Err."
    );
}

/// Assert the migration was PERSISTED: the file now carries the magic, so the next
/// start takes the current-format branch instead of migrating again.
fn assert_migration_persisted(dir: &Path, ctx: &str) {
    let bytes = read_state_file(dir);
    assert!(
        bytes.len() >= 8 && &bytes[..4] == MAGIC,
        "{ctx}: after migrating, load() must re-save the file in the current layout \
         (MAGIC || VERSION || body) so the migration is one-shot; found first bytes {:?}",
        &bytes[..bytes.len().min(8)]
    );
    assert_eq!(
        u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        MAINTAINER_STATE_VERSION,
        "{ctx}: the re-saved file must carry the CURRENT version tag"
    );
}

// ---------------------------------------------------------------------------
// P1 — a missing file is a legitimate fresh node
// ---------------------------------------------------------------------------

/// REQ-172-005 (Must). GREEN-lock.
/// Acceptance: a node that has never written the file loads the empty default and
/// starts normally. A fresh install must not be treated as corruption.
/// [P1 -> O1,O2]
#[test]
fn req_172_005_missing_file_is_ok_default() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        !dir.path().join(STATE_FILE).exists(),
        "precondition: the state file must be absent"
    );

    let loaded = MaintainerState::load(dir.path())
        .expect("a MISSING maintainer_state.bin is a legitimate fresh node, not an error");

    assert!(
        loaded.set.members.is_empty(),
        "a fresh node has no maintainers cached"
    );
    assert_eq!(loaded.set.threshold, 0, "a fresh node has threshold 0");
    assert_eq!(
        loaded.last_derived_height, 0,
        "a fresh node has derived nothing"
    );
    assert!(
        !dir.path().join(STATE_FILE).exists(),
        "loading a missing file must not CREATE one — a fresh node writes the file when \
         it first derives a set, not when it reads nothing"
    );
}

// ---------------------------------------------------------------------------
// P2 — a legacy unversioned file is MIGRATED, losslessly and once (§9)
// ---------------------------------------------------------------------------

/// REQ-172-005 (Must) / REQ-172-011 (Must).
/// Acceptance: a LEGACY (unversioned) `maintainer_state.bin` holding a POPULATED
/// maintainer set loads successfully with `members`, `threshold`, `set.last_updated`
/// and `last_derived_height` IDENTICAL to what was written, and the file is re-saved
/// in the current layout so a second load takes the current-format path.
/// [P2 x I1 -> O1,O2,O5]
///
/// This is upgrade day on every live node. Refusing here is a fleet outage delivered
/// by the auto-updater (§9 Evidence); accepting the file's REAL set is not a fail-open
/// — the set is preserved exactly, and the empty-root property is asserted anyway.
#[test]
fn req_172_005_legacy_unversioned_file_with_members_is_migrated_losslessly() {
    let dir = tempfile::tempdir().unwrap();

    let members = vec![pubkey(1), pubkey(2), pubkey(3)];
    let set = MaintainerSet::with_members(members.clone(), 900);
    let expected_threshold = set.threshold;
    assert_eq!(set.members.len(), 3, "fixture sanity: 3 members");
    assert_eq!(
        expected_threshold, 2,
        "fixture sanity: 3 members ⇒ threshold 2"
    );
    write_state_file(dir.path(), &legacy_bytes(set, 4242));

    let loaded = MaintainerState::load(dir.path()).unwrap_or_else(|e| {
        panic!(
            "load() REFUSED a legacy unversioned maintainer_state.bin: {e}\n\
             Every node that ever bootstrapped a maintainer set has exactly this file on \
             disk, and run.rs makes a load error FATAL. Refusing here bricks the fleet \
             through the auto-update path (api-contract §9). A legacy file must be \
             MIGRATED: decoded with the legacy schema, warned about, and re-saved."
        )
    });

    assert_not_silently_defaulted(&loaded, "legacy populated file");
    assert_eq!(
        loaded.set.members, members,
        "migration must preserve members verbatim, IN ORDER — a reordered or dropped \
         member changes which keys can authorise a release"
    );
    assert_eq!(
        loaded.set.threshold, expected_threshold,
        "migration must preserve the threshold; a decoded 0 is vacuously satisfiable (FM-02)"
    );
    assert_eq!(
        loaded.set.last_updated, 900,
        "migration must preserve set.last_updated"
    );
    assert_eq!(
        loaded.last_derived_height, 4242,
        "migration must preserve last_derived_height — it is the flag that distinguishes \
         'never bootstrapped' from 'set existed and was emptied' at run.rs"
    );

    // O5: the migration is persisted, so it happens once.
    assert_migration_persisted(dir.path(), "legacy populated file");
    let reloaded = MaintainerState::load(dir.path())
        .expect("the re-saved file must load through the CURRENT-format path");
    assert_eq!(
        reloaded.set.members, members,
        "second load must be identical"
    );
    assert_eq!(reloaded.set.threshold, expected_threshold);
    assert_eq!(reloaded.last_derived_height, 4242);
    assert_eq!(
        reloaded.version, MAINTAINER_STATE_VERSION,
        "the migrated value carries the current format version"
    );
}

/// REQ-172-005 (Must). Acceptance: a legacy file holding an EMPTY set migrates to an
/// empty set — the value is PRESERVED, not invented. The all-zero-prefix shape must
/// not be confused with a current-format file.
/// [P2 x I2 -> O1,O2,O5]
///
/// Note this cell is deliberately NOT asserted with `assert_not_silently_defaulted`:
/// here the empty set is the file's true content, faithfully preserved. The fail-open
/// this suite forbids is an empty set INVENTED from bytes that said otherwise, which
/// is what t02, t07 and t08 assert.
#[test]
fn req_172_005_legacy_unversioned_file_when_empty_migrates_to_the_same_empty_set() {
    let dir = tempfile::tempdir().unwrap();
    write_state_file(dir.path(), &legacy_bytes(MaintainerSet::new(), 0));

    let loaded = MaintainerState::load(dir.path())
        .expect("a decodable legacy file must migrate, not fail (api-contract §9)");

    assert!(
        loaded.set.members.is_empty(),
        "an empty legacy set must stay empty — migration copies, it does not invent"
    );
    assert_eq!(loaded.set.threshold, 0, "threshold preserved verbatim");
    assert_eq!(loaded.last_derived_height, 0, "height preserved verbatim");

    assert_migration_persisted(dir.path(), "legacy empty file");
    let reloaded =
        MaintainerState::load(dir.path()).expect("the re-saved empty state must load back");
    assert!(reloaded.set.members.is_empty());
    assert_eq!(reloaded.last_derived_height, 0);
}

/// REQ-172-011 (Must). REGRESSION for the aliasing hazard named in api-contract §9
/// ("Second, independent defect in the same scheme").
/// Acceptance: a legacy file whose member count is EXACTLY `MAINTAINER_STATE_VERSION`
/// is still detected as LEGACY and migrated — never misread as a current-format file.
/// [P2 x I5 -> O1,O2,O5]
///
/// Why this test cannot be dropped: a bare 4-byte integer tag at offset 0 occupies the
/// same bytes as bincode's `u64` length prefix for `set.members`. At `VERSION = 1`, a
/// 1-member legacy file presents `01 00 00 00` and passes a bare tag check, after which
/// the decoder reads `[4..12]` as the members length and produces garbage. The fix is
/// structural (a magic), so this test also pins the reason: it FAILS for any scheme that
/// discriminates on an integer sharing an offset with payload, and passes only for one
/// that uses a value no payload can produce.
#[test]
fn req_172_011_legacy_file_with_version_many_members_is_not_misread_as_current_format() {
    let dir = tempfile::tempdir().unwrap();

    let member_count = MAINTAINER_STATE_VERSION as usize;
    assert_eq!(
        member_count, 1,
        "this test is calibrated to the aliasing point: it must hold exactly \
         MAINTAINER_STATE_VERSION members so the legacy length prefix equals the version tag"
    );
    let members: Vec<crypto::PublicKey> = (0..member_count).map(|i| pubkey(40 + i as u8)).collect();
    let set = MaintainerSet::with_members(members.clone(), 55);
    assert_eq!(set.threshold, 1, "fixture sanity: 1 member ⇒ threshold 1");

    let bytes = legacy_bytes(set, 6060);
    assert_eq!(
        &bytes[..4],
        &(MAINTAINER_STATE_VERSION).to_le_bytes(),
        "fixture sanity: the legacy length prefix is byte-identical to a version-{MAINTAINER_STATE_VERSION} tag — \
         this IS the aliasing condition under test"
    );
    assert_ne!(
        &bytes[..4],
        MAGIC.as_slice(),
        "fixture sanity: the legacy file must NOT carry the magic"
    );
    write_state_file(dir.path(), &bytes);

    let loaded = MaintainerState::load(dir.path()).unwrap_or_else(|e| {
        panic!(
            "load() failed on a 1-member legacy file: {e}\n\
             Its length prefix is byte-identical to a version tag of \
             {MAINTAINER_STATE_VERSION}, so a decoder that discriminates on a bare integer \
             takes the current-format branch and misparses it. The file must be detected as \
             LEGACY (no magic) and migrated."
        )
    });

    assert_not_silently_defaulted(&loaded, "1-member legacy file (aliasing partition)");
    assert_eq!(
        loaded.set.members, members,
        "the single member must survive the migration verbatim"
    );
    assert_eq!(loaded.set.threshold, 1, "threshold preserved");
    assert_eq!(loaded.set.last_updated, 55, "last_updated preserved");
    assert_eq!(
        loaded.last_derived_height, 6060,
        "last_derived_height preserved — a misparse would produce a wild value here"
    );
    assert_migration_persisted(dir.path(), "1-member legacy file");
}

/// REQ-172-005 (Must). Acceptance: the migration works on REAL bytes written by an
/// older binary on a live node, not only on bytes this binary generated.
/// [P2 x I6 -> O1,O2,O5]
///
/// Source: `~/testnet/seed/data/maintainer_state.bin`, 232 bytes, captured 2026-08-10
/// (an identical-shaped file exists on every other node; `n7`'s differs only in member
/// ORDER). Embedded verbatim rather than read from `~/testnet` so the test is
/// self-contained and runs on any machine. These bytes are the exact evidence in
/// api-contract §9: they begin `05 00 00 00 00 00 00 00`, which the pre-correction
/// decoder read as "format version 5" and rejected — fatally, at startup.
#[test]
fn req_172_005_real_legacy_file_from_a_live_node_migrates() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = real_legacy_bytes();
    assert_eq!(
        bytes.len(),
        232,
        "fixture sanity: the captured file is 232 bytes"
    );
    assert_eq!(
        &bytes[..8],
        &[0x05, 0, 0, 0, 0, 0, 0, 0],
        "fixture sanity: the real file opens with bincode's u64 length prefix for 5 \
         members — THE evidence in api-contract §9, not a version tag"
    );
    write_state_file(dir.path(), &bytes);

    let loaded = MaintainerState::load(dir.path()).unwrap_or_else(|e| {
        panic!(
            "load() REFUSED a real maintainer_state.bin taken from a live node: {e}\n\
             run.rs makes this fatal, so shipping this would stop every node that ever \
             bootstrapped a maintainer set — delivered by the auto-updater itself."
        )
    });

    assert_not_silently_defaulted(&loaded, "real legacy file");
    assert_eq!(
        loaded.set.members.len(),
        5,
        "the real file holds the five bootstrap maintainers; a different count means the \
         body was misparsed"
    );
    assert_eq!(
        loaded.set.threshold, 3,
        "5 members ⇒ threshold 3 (MaintainerSet::calculate_threshold), preserved verbatim"
    );
    assert_eq!(loaded.set.last_updated, 1, "set.last_updated preserved");
    assert_eq!(
        loaded.last_derived_height, 1,
        "last_derived_height preserved"
    );
    for (i, m) in loaded.set.members.iter().enumerate() {
        assert_eq!(
            m.as_bytes().len(),
            32,
            "member {i} must decode as a 32-byte Ed25519 key"
        );
    }

    assert_migration_persisted(dir.path(), "real legacy file");
    let reloaded = MaintainerState::load(dir.path())
        .expect("the migrated real file must load through the CURRENT-format path");
    assert_eq!(
        reloaded.set.members, loaded.set.members,
        "second load must return the identical member list"
    );
    assert_eq!(reloaded.set.threshold, 3);
    assert_eq!(reloaded.last_derived_height, 1);
}

/// Verbatim bytes of `~/testnet/seed/data/maintainer_state.bin` (232 bytes).
/// Kept as hex so the fixture is reviewable in a diff.
fn real_legacy_bytes() -> Vec<u8> {
    const REAL_LEGACY_STATE_HEX: &str = concat!(
        "0500000000000000200000000000000054323cefd0eabac89b2a2198c95a8f26",
        "1598c341a8e579a05e26322325c48c2b2000000000000000effe88fefb6d992a",
        "1329277a1d49c7296d252bbc368319cb4bc061119926272b2000000000000000",
        "2d27fdcc6a240b76ecaea64ad05c9b70d1adad90b6f9c43e8cbbbc0f1ab04116",
        "2000000000000000202047256a8072a8b8f476691b9a5ae87710cc545e8707ca",
        "9fe0c803c3e6d3df20000000000000003047e96b13276dd92ef5eb2d6396e66c",
        "29909217f11f8c0544ea7d76a76c760203000000000000000100000000000000",
        "0100000000000000",
    );
    (0..REAL_LEGACY_STATE_HEX.len() / 2)
        .map(|i| {
            u8::from_str_radix(&REAL_LEGACY_STATE_HEX[i * 2..i * 2 + 2], 16)
                .expect("fixture hex must be valid")
        })
        .collect()
}

// ---------------------------------------------------------------------------
// P3 — corruption is an error, never a default
// ---------------------------------------------------------------------------

/// REQ-172-011 (Must).
/// Acceptance: truncated or corrupt bytes produce an Err naming the file. Never a
/// silent default. This is the case `run.rs`'s fatal path exists for, and it stays
/// fatal: a file that decodes as neither layout is not a migration, it is damage.
/// [P3 x I3 -> O1,O2,O3]
#[test]
fn req_172_011_truncated_or_corrupt_file_is_an_error_not_a_default() {
    // (a) A valid legacy encoding cut in half.
    let dir = tempfile::tempdir().unwrap();
    let full = legacy_bytes(
        MaintainerSet::with_members(vec![pubkey(7), pubkey(8), pubkey(9)], 500),
        777,
    );
    write_state_file(dir.path(), &full[..full.len() / 2]);
    match MaintainerState::load(dir.path()) {
        Ok(loaded) => {
            assert_not_silently_defaulted(&loaded, "truncated file");
            panic!("load() ACCEPTED a truncated maintainer_state.bin");
        }
        Err(e) => assert!(
            e.to_string().contains(STATE_FILE),
            "a decode failure must NAME the file so the operator can act; got: {:?}",
            e.to_string()
        ),
    }

    // (b) Short garbage that is not a valid encoding of anything.
    let dir = tempfile::tempdir().unwrap();
    write_state_file(dir.path(), &[0xFFu8; 7]);
    match MaintainerState::load(dir.path()) {
        Ok(loaded) => {
            assert_not_silently_defaulted(&loaded, "garbage file");
            panic!("load() ACCEPTED 7 bytes of garbage as a maintainer state");
        }
        Err(e) => assert!(
            e.to_string().contains(STATE_FILE),
            "a decode failure must NAME the file; got: {:?}",
            e.to_string()
        ),
    }

    // (c) An EMPTY file — zero bytes. Present on disk, decodes to nothing.
    let dir = tempfile::tempdir().unwrap();
    write_state_file(dir.path(), &[]);
    match MaintainerState::load(dir.path()) {
        Ok(loaded) => {
            assert_not_silently_defaulted(&loaded, "zero-byte file");
            panic!(
                "load() ACCEPTED a zero-byte maintainer_state.bin. A torn write that \
                 truncates the file to 0 must not be indistinguishable from a fresh node."
            );
        }
        Err(e) => assert!(
            e.to_string().contains(STATE_FILE),
            "a decode failure must NAME the file; got: {:?}",
            e.to_string()
        ),
    }

    // (d) Magic present, KNOWN version, but the body is garbage. The header must not
    //     buy a corrupt body a free pass.
    let dir = tempfile::tempdir().unwrap();
    let mut header_only = MAGIC.to_vec();
    header_only.extend_from_slice(&MAINTAINER_STATE_VERSION.to_le_bytes());
    header_only.extend_from_slice(&[0xABu8; 5]);
    write_state_file(dir.path(), &header_only);
    match MaintainerState::load(dir.path()) {
        Ok(loaded) => {
            assert_not_silently_defaulted(&loaded, "current-format file with a corrupt body");
            panic!("load() ACCEPTED a current-format file whose body does not decode");
        }
        Err(e) => assert!(
            e.to_string().contains(STATE_FILE),
            "a body decode failure must NAME the file; got: {:?}",
            e.to_string()
        ),
    }
}

// ---------------------------------------------------------------------------
// P4 — encoder/decoder parity
// ---------------------------------------------------------------------------

/// REQ-172-005 (Must). GREEN-lock.
/// Acceptance: save -> load preserves `members` (order and content), `threshold`,
/// `set.last_updated` and `last_derived_height`. Adding a header must not reorder or
/// drop any body field.
/// [P4 x I4 -> O1,O2,O4]
///
/// CLAUDE.md: "Any encoder/decoder pair MUST be verified for index parity." The header
/// precedes the body (§9); this test is what catches a header written at the wrong
/// offset, which would silently reinterpret `members`.
#[test]
fn req_172_005_versioned_round_trip_preserves_every_field() {
    let dir = tempfile::tempdir().unwrap();

    let members = vec![pubkey(21), pubkey(22), pubkey(23), pubkey(24), pubkey(25)];
    let set = MaintainerSet::with_members(members.clone(), 31_337);
    let expected_threshold = set.threshold;
    assert_eq!(
        expected_threshold, 3,
        "fixture sanity: a 5-member set has threshold 3 (calculate_threshold)"
    );

    let mut state = MaintainerState::default();
    state
        .update(set, 99_101, dir.path())
        .expect("update() must persist the versioned state");

    // The written file carries the header, so a legacy decoder cannot silently read it.
    let written = read_state_file(dir.path());
    assert_eq!(
        &written[..4],
        MAGIC.as_slice(),
        "a file written by this binary must carry the magic"
    );

    let loaded = MaintainerState::load(dir.path())
        .expect("a state this process just wrote must load back cleanly");

    assert_eq!(
        loaded.set.members, members,
        "members must round-trip verbatim, IN ORDER — a reordered or shifted decode \
         changes which keys can authorise a release"
    );
    assert_eq!(
        loaded.set.threshold, expected_threshold,
        "threshold must round-trip; a decoded 0 is vacuously satisfiable (FM-02)"
    );
    assert_eq!(
        loaded.set.last_updated, 31_337,
        "set.last_updated must round-trip"
    );
    assert_eq!(
        loaded.last_derived_height, 99_101,
        "last_derived_height must round-trip — it is the flag that distinguishes \
         'never bootstrapped' (bootstrap root, REQ-172-005) from 'set existed and was \
         emptied' (fail closed) at run.rs"
    );

    // Byte-level determinism: writing the same value twice yields the same file.
    let first = read_state_file(dir.path());
    loaded.save(dir.path()).expect("re-save must succeed");
    let second = read_state_file(dir.path());
    assert_eq!(
        first, second,
        "encoding must be deterministic: re-saving a loaded state must produce \
         byte-identical output"
    );
}

// ---------------------------------------------------------------------------
// P5 — magic present, version unknown: loud and fail-closed
// ---------------------------------------------------------------------------

/// REQ-172-011 (Must).
/// Acceptance: a file that carries the magic but an UNKNOWN version is refused with an
/// error naming the file and the version mismatch. Never accepted, never defaulted.
/// [P5 x I4' -> O1,O2,O3]
///
/// This branch is forward-only — no file in the wild carries the magic today — so
/// unlike the legacy branch it cannot brick the current fleet. It is what makes a
/// FUTURE format change a defined, loud migration instead of a silent misparse.
#[test]
fn req_172_011_current_magic_with_unknown_version_is_a_loud_error() {
    let dir = tempfile::tempdir().unwrap();

    // Build a well-formed CURRENT file, then bump only its version tag.
    let mut state = MaintainerState::default();
    state
        .update(
            MaintainerSet::with_members(vec![pubkey(31), pubkey(32), pubkey(33)], 12),
            8_080,
            dir.path(),
        )
        .expect("writing the current-format fixture must succeed");
    let mut bytes = read_state_file(dir.path());
    let future = MAINTAINER_STATE_VERSION + 7;
    bytes[4..8].copy_from_slice(&future.to_le_bytes());
    write_state_file(dir.path(), &bytes);

    let err = MaintainerState::load(dir.path()).err().unwrap_or_else(|| {
        panic!(
            "load() ACCEPTED a file whose format version is {future}, which this binary does \
             not understand. Accepting it means reinterpreting an unknown body layout as the \
             release-verification trust root."
        )
    });

    let msg = err.to_string();
    assert!(
        msg.contains(STATE_FILE),
        "the error must NAME the offending file ({STATE_FILE}); got: {msg:?}"
    );
    assert!(
        msg.to_lowercase().contains("version"),
        "the error must state the version mismatch (found vs expected); got: {msg:?}"
    );
    assert!(
        msg.contains(&future.to_string()),
        "the error must report the version FOUND ({future}); got: {msg:?}"
    );
}

// ---------------------------------------------------------------------------
// F4 — `save` must be ATOMIC (review pass 1)
//
// OUTPUT CONTRACT ADDENDUM. Function under test:
//   `MaintainerState::save(&self, data_dir: &Path) -> Result<(), StorageError>`
// Outputs: (O-A) the Result; (O-B) the CONTENTS of maintainer_state.bin after the call;
// (O-C) the presence of any staging file left in the directory. O-B is the one that
// matters: a bare `fs::write` is create + TRUNCATE + write_all, so a crash between the
// truncate and the write leaves a zero-byte file — and a zero-byte file is FATAL at
// startup (asserted at `req_172_011_truncated_or_corrupt_file_is_an_error_not_a_default`
// above). `migrate_legacy` performs exactly this write on EVERY node's first boot after
// this upgrade, inside the rolling-deploy window, on a restart the auto-updater itself
// triggers.
//
// PATHS: (Pa) save succeeds; (Pb) save fails before the target is touched.
// A true mid-write crash cannot be provoked in-process, so Pb is provoked at the
// nearest observable boundary: the staging file cannot be created. Under the old
// `fs::write` implementation the target was already truncated by that point; under the
// atomic one the target is not opened at all until `rename`.
// ---------------------------------------------------------------------------

/// F4. Acceptance: a successful save leaves no staging file behind — the directory a
/// node boots from contains exactly the state file.
/// [Pa -> O-C]
#[test]
fn f4_successful_save_leaves_no_staging_file_behind() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = MaintainerState::default();
    state
        .update(
            MaintainerSet::with_members(vec![pubkey(41), pubkey(42)], 9),
            777,
            dir.path(),
        )
        .expect("save must succeed on a writable dir");

    let leftovers: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n != STATE_FILE)
        .collect();
    assert!(
        leftovers.is_empty(),
        "save left {leftovers:?} in the data dir. The staging file must be renamed onto \
         the target, not accumulated: a stale `.tmp` is indistinguishable from a torn \
         write to an operator inspecting the directory."
    );
    assert_eq!(
        MaintainerState::load(dir.path())
            .unwrap()
            .last_derived_height,
        777
    );
}

/// F4. RED before the fix. Acceptance: a save that cannot complete leaves the PREVIOUS
/// state file fully intact and loadable — never truncated to zero bytes.
///
/// With the old `std::fs::write(&path, out)`, the target is opened with truncate FIRST,
/// so any failure after that point is a zero-byte `maintainer_state.bin` — which the
/// node treats as fatal and refuses to start on. That is a brick delivered by the
/// auto-updater itself (the INC-I-153 class).
/// [Pb -> O-A, O-B]
#[test]
fn f4_a_failing_save_never_destroys_the_existing_state_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = MaintainerState::default();
    state
        .update(
            MaintainerSet::with_members(vec![pubkey(51), pubkey(52), pubkey(53)], 11),
            5150,
            dir.path(),
        )
        .expect("the first save establishes the good file");
    let good_bytes = read_state_file(dir.path());

    // Block the staging path with a DIRECTORY: `File::create` on it fails, so the save
    // aborts at the earliest point it can.
    std::fs::create_dir(dir.path().join("maintainer_state.bin.tmp")).unwrap();

    let mut next = MaintainerState::default();
    let result = next.update(
        MaintainerSet::with_members(vec![pubkey(61)], 3),
        6000,
        dir.path(),
    );
    assert!(
        result.is_err(),
        "a save that cannot write its staging file must report the failure, not claim success"
    );

    assert_eq!(
        read_state_file(dir.path()),
        good_bytes,
        "the previous maintainer_state.bin was modified by a FAILED save. The trust root \
         on disk must be either the old complete file or the new complete file — never a \
         truncated one, because an undecodable file stops the node from starting."
    );
    let reloaded = MaintainerState::load(dir.path())
        .expect("the surviving file must still load after a failed save");
    assert_eq!(reloaded.last_derived_height, 5150);
    assert_eq!(reloaded.set.members.len(), 3);
}
