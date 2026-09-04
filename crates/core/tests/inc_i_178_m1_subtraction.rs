//! INC-I-178 M1 — the deletions of D1/R2/R3/R4, and the guard against deleting
//! more than that.
//!
//! Rust cannot assert a symbol's ABSENCE at compile time, so the deletions are
//! locked by scanning the shipped source text, the way `inc_i_172_m2_ungated_tripwire`
//! already does in this crate. The over-deletion guard is the other half: every
//! symbol M1 must NOT touch is asserted PRESENT by the same scanner.
//!
//! OUTPUT CONTRACT
//!
//! F1: the source tree `crates/core/src/attestation/` (a build input, not a fn)
//!   Observable outputs:
//!     O1 file set — `attestation.rs` gone, `attestation/` present
//!     O2 per-file line count — every file < 500 (CLAUDE.md #19, spec R4)
//!     O3 token set — the D1 dead-store identifiers absent; the survivors present
//!     O4 mutable params / receiver / store / global / channel — NONE (read-only scan)
//!   Paths: P1 file missing -> fail | P2 token found where forbidden -> fail
//!          P3 token missing where required -> fail | P4 all clean -> pass
//!   INPUT PARTITIONS:
//!     P3a a symbol deleted by D1 (must be gone)
//!     P3b a symbol the scoping report DEFERS — the 3 Hash-variant codec fns and
//!         `new_with_bls` (must survive; deleting them breaks M0's golden store)
//!     P3c a wire-format field under SUB-C1 (must survive; bincode is positional)
//!
//! F2: `MinuteAttestationTracker::{record, fingerprint, attested_in_minute, total_entries, reset}`
//!   Observable outputs:
//!     O1 return of `fingerprint()` — a 32-byte digest of the `attested` map alone
//!     O2 return of `attested_in_minute()` / `total_entries()`
//!     O3 `&mut self` mutation — only the `attested` map exists to mutate
//!     O4 store / global / channel — NONE
//!   Paths: P1 record a new pubkey | P2 record an existing pubkey in a new minute
//!          P3 record a duplicate (pubkey, minute) | P4 reset
//!   INPUT PARTITIONS:
//!     P1a empty tracker | P1b one pubkey, two minutes | P1c two pubkeys, one shared minute
//!     P3a exact duplicate — the set must absorb it
//!
//! F3: `crates/core/tests/fixtures/attestation_baseline_vectors.json`
//!   Observable outputs: O1 SHA-256 of the file bytes. Paths: P1 unchanged | P2 regenerated.
//!   INPUT PARTITIONS: none — a single byte-exact artifact.
//!
//! MATRIX: F1 O1-O3 x P3a/P3b/P3c and F2 O1-O3 x P1a-P3a are covered by the tests
//! below; O4-class outputs are constant-NONE and are structural (no scanner or
//! tracker method takes a store handle).
//!
//! Requirement IDs: REQ-BLS-012 (drain the dead BLS surface), REQ-BLS-010
//! (liveness must not regress — nothing the encoder still reads may be deleted).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crypto::PublicKey;
use doli_core::attestation::MinuteAttestationTracker;
use sha2::{Digest, Sha256};

/// SHA-256 of the 66-vector M0 golden store as shipped by M0. Regenerating the
/// file changes this digest.
const GOLDEN_STORE_SHA256: &str =
    "91c5c9614e3eb225ea52f19dee41e2b433b853a0d09dc7eda650528af5094e29";

/// `fingerprint()` of a tracker holding {0x11..: {7, 9}, 0x22..: {7}}, measured
/// on the pre-M1 tree. M1 removes no input to this hash, so it must not move.
const TRACKER_FINGERPRINT_HEX: &str =
    "590b77ebe8cbe00a543a993f9773bb119a690ea6e9214bb4d3abab1540995550";

const R4_LINE_BUDGET: usize = 500;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root must resolve")
}

fn attestation_dir() -> PathBuf {
    repo_root().join("crates/core/src/attestation")
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => panic!("cannot list {}: {e}", dir.display()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn attestation_sources() -> Vec<(PathBuf, String)> {
    let dir = attestation_dir();
    assert!(
        dir.is_dir(),
        "R4: {} must exist as a directory",
        dir.display()
    );
    let mut files = Vec::new();
    rust_files(&dir, &mut files);
    assert!(
        !files.is_empty(),
        "R4: {} holds no .rs files",
        dir.display()
    );
    files
        .into_iter()
        .map(|p| {
            let s = fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
            (p, s)
        })
        .collect()
}

/// Source with every whole-line `//` comment removed, so a token surviving only
/// in prose does not trip a code-level assertion.
fn code_only(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

// REQ-BLS-012 — Decision: any of these tokens still in the tree means the
// write-only minute-keyed BLS store survived D1 and is still costing a
// BLS12-381 signature per applied block.
#[test]
fn m1_d1_the_dead_bls_store_symbols_are_gone_from_the_attestation_module() {
    const FORBIDDEN: [&str; 6] = [
        "RegionAggregate",
        "from_attestations",
        "bls_sigs_for_minute",
        "bls_sig_count",
        "record_with_bls",
        "bls_sigs",
    ];
    for (path, src) in attestation_sources() {
        for tok in FORBIDDEN {
            assert!(
                !src.contains(tok),
                "D1: `{tok}` still present in {}",
                path.display()
            );
        }
    }
}

// REQ-BLS-012 — Decision: the tracker is only "footprint = f(attested)" if the
// struct literally has nothing else to read; a second field is the whole defect.
#[test]
fn m1_d1_tracker_struct_declares_exactly_one_field() {
    let src = read("crates/core/src/attestation/tracker.rs");
    assert!(
        !src.contains("bls_sig"),
        "D1: tracker.rs still mentions `bls_sig`"
    );

    let start = src
        .find("struct MinuteAttestationTracker")
        .expect("tracker.rs must declare MinuteAttestationTracker");
    let open = src[start..].find('{').expect("struct body must open") + start;
    let close = src[open..].find("\n}").expect("struct body must close") + open;
    let body = &src[open + 1..close];

    let fields: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with('#'))
        .collect();

    assert_eq!(
        fields.len(),
        1,
        "D1: MinuteAttestationTracker must hold exactly one field, found {fields:?}"
    );
    assert!(
        fields[0].starts_with("attested"),
        "D1: the surviving field must be `attested`, found `{}`",
        fields[0]
    );
}

// REQ-BLS-010 — Decision: deleting these three functions breaks 30 M0 tests and
// makes the 66-vector golden store unverifiable, so the pre-AH byte-identity
// proof through M3/M4 would be gone.
#[test]
fn m1_deferred_codec_and_new_with_bls_survive_the_deletions() {
    const REQUIRED: [&str; 4] = [
        "pub fn encode_attestation_bitfield(",
        "pub fn decode_attestation_bitfield(",
        "pub fn validate_attestation_bitfield(",
        "pub fn new_with_bls(",
    ];
    let sources = attestation_sources();
    for tok in REQUIRED {
        assert!(
            sources.iter().any(|(_, s)| s.contains(tok)),
            "over-deletion: `{tok}` is DEFERRED past M1 and must still exist"
        );
    }
}

// REQ-BLS-010 — Decision: `Attestation` is bincode-positional and blocks are
// persisted in RocksDB, so removing a field silently reinterprets every stored
// block (SUB-C1).
#[test]
fn m1_all_seven_attestation_fields_and_the_block_aggregate_survive() {
    const FIELDS: [&str; 7] = [
        "pub block_hash:",
        "pub slot:",
        "pub height:",
        "pub attester:",
        "pub attester_weight:",
        "pub signature:",
        "pub bls_signature:",
    ];
    let sources = attestation_sources();
    for f in FIELDS {
        assert!(
            sources.iter().any(|(_, s)| s.contains(f)),
            "SUB-C1: Attestation field `{f}` was removed"
        );
    }
    assert!(
        read("crates/core/src/block.rs").contains("pub aggregate_bls_signature:"),
        "SUB-C1: Block.aggregate_bls_signature was removed"
    );
}

// REQ-BLS-012 — Decision: a 798-line competing presence model left in the tree
// is the comprehension failure this redesign exists to end.
#[test]
fn m1_r2_presence_module_and_its_exports_are_gone() {
    let p = repo_root().join("crates/core/src/presence.rs");
    assert!(!p.exists(), "R2: {} must be deleted", p.display());

    let lib = code_only(&read("crates/core/src/lib.rs"));
    assert!(
        !lib.contains("mod presence"),
        "R2: lib.rs still declares the module"
    );
    assert!(
        !lib.contains("presence::"),
        "R2: lib.rs still re-exports from presence"
    );

    let stanza = read("testing/integration/Cargo.toml");
    assert!(
        !stanza.contains("presence_manipulation_test"),
        "R2: the [[test]] stanza still names presence_manipulation_test"
    );
    assert!(
        !repo_root()
            .join("testing/integration/presence_manipulation_test.rs")
            .exists(),
        "R2: presence_manipulation_test.rs must be deleted with the module"
    );
}

// REQ-BLS-012 — Decision: a constant fixed at 0 makes `h < CONST` unsatisfiable
// on u64; leaving it invites the next reader to treat it as a live gate.
#[test]
fn m1_r3_bitfield_body_activation_height_constant_is_gone() {
    let src = code_only(&read("crates/core/src/consensus/constants.rs"));
    assert!(
        !src.contains("BITFIELD_BODY_ACTIVATION_HEIGHT"),
        "R3: constants.rs still defines BITFIELD_BODY_ACTIVATION_HEIGHT"
    );
}

// REQ-BLS-012 — Decision: the split is worthless if one child file inherits the
// 703-line blob; 500 is the project budget.
#[test]
fn m1_r4_module_is_split_and_every_file_is_under_the_line_budget() {
    let legacy = repo_root().join("crates/core/src/attestation.rs");
    assert!(
        !legacy.exists(),
        "R4: {} must be replaced by the attestation/ directory",
        legacy.display()
    );

    let sources = attestation_sources();
    let names: HashSet<String> = sources
        .iter()
        .filter_map(|(p, _)| p.file_name().and_then(|n| n.to_str()).map(String::from))
        .collect();
    for expected in [
        "mod.rs",
        "message.rs",
        "bitfield.rs",
        "tracker.rs",
        "pool.rs",
    ] {
        assert!(
            names.contains(expected),
            "R4: attestation/{expected} is missing (found {names:?})"
        );
    }

    for (path, src) in &sources {
        let n = src.lines().count();
        assert!(
            n < R4_LINE_BUDGET,
            "R4: {} is {n} lines, budget is {R4_LINE_BUDGET}",
            path.display()
        );
    }
}

// REQ-BLS-010 — Decision: a regenerated golden store silently re-baselines the
// pre-AH byte-identity proof onto post-M1 behaviour, which is the one thing it
// exists to detect.
#[test]
fn m1_m0_golden_store_is_byte_identical() {
    let bytes =
        fs::read(repo_root().join("crates/core/tests/fixtures/attestation_baseline_vectors.json"))
            .expect("the M0 golden store must exist");
    let digest = hex_of(&Sha256::digest(&bytes));
    assert_eq!(
        digest, GOLDEN_STORE_SHA256,
        "the M0 golden store was regenerated; M1 must not touch it"
    );
}

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn pk(tag: u8) -> PublicKey {
    PublicKey::from_bytes([tag; 32])
}

// REQ-BLS-012 — Decision: if the fingerprint moves, the attendance map's hashing
// changed and every cross-node divergence alarm built on it silently re-baselines.
#[test]
fn m1_tracker_fingerprint_of_a_fixed_attendance_set_is_unchanged() {
    let mut t = MinuteAttestationTracker::new();
    t.record(pk(0x11), 7);
    t.record(pk(0x11), 9);
    t.record(pk(0x22), 7);

    assert_eq!(
        hex_of(t.fingerprint().as_bytes()),
        TRACKER_FINGERPRINT_HEX,
        "D1 changed the attendance map's hash input"
    );
}

// REQ-BLS-012 — Decision: `record_with_bls` and `record` inserted the SAME
// `attested` entry (INV-12(3)); if collapsing them changed attendance, every
// bitfield bit and every reward denominator moves.
#[test]
fn m1_tracker_attendance_is_a_function_of_the_attested_map_alone() {
    let mut t = MinuteAttestationTracker::new();
    assert_eq!(t.total_entries(), 0);
    assert!(t.attested_in_minute(7).is_empty());

    t.record(pk(0x11), 7);
    t.record(pk(0x22), 7);
    t.record(pk(0x11), 9);
    // A duplicate (pubkey, minute) must be absorbed by the set.
    t.record(pk(0x11), 7);

    assert_eq!(t.total_entries(), 3);

    let mut m7: Vec<[u8; 32]> = t
        .attested_in_minute(7)
        .iter()
        .map(|k| *k.as_bytes())
        .collect();
    m7.sort();
    assert_eq!(m7, vec![[0x11u8; 32], [0x22u8; 32]]);

    let m9: Vec<[u8; 32]> = t
        .attested_in_minute(9)
        .iter()
        .map(|k| *k.as_bytes())
        .collect();
    assert_eq!(m9, vec![[0x11u8; 32]]);

    assert!(t.attested_in_minute(8).is_empty());
}

// REQ-BLS-012 — Decision: `reset()` used to clear two maps; if the surviving
// clear is dropped or partial, attendance leaks across the epoch boundary and
// inflates the next epoch's reward denominators.
#[test]
fn m1_tracker_reset_empties_the_attendance_map_completely() {
    let mut t = MinuteAttestationTracker::new();
    for i in 0..8u8 {
        t.record(pk(i), 3);
        t.record(pk(i), 4);
    }
    assert_eq!(t.total_entries(), 16);
    let populated = t.fingerprint();

    t.reset();

    assert_eq!(t.total_entries(), 0);
    assert!(t.attested_in_minute(3).is_empty());
    assert!(t.attested_in_minute(4).is_empty());
    assert_ne!(t.fingerprint(), populated);
    assert_eq!(
        t.fingerprint(),
        MinuteAttestationTracker::new().fingerprint()
    );
}

// REQ-BLS-010 — Decision: R4 moves the type; if the crate-root re-export is
// dropped, every `doli_core::MinuteAttestationTracker` caller breaks and the
// split stops being caller-invisible.
#[test]
fn m1_r4_public_api_is_reachable_from_both_paths() {
    let a = doli_core::MinuteAttestationTracker::new();
    let b = doli_core::attestation::MinuteAttestationTracker::new();
    assert_eq!(a.fingerprint(), b.fingerprint());
    assert_eq!(doli_core::attestation::attestation_minute(0), 0);
}
