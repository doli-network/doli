//! INC-I-174 M1 — REQ-174-004: the `cf_undo` schema change must not SILENTLY disable undo.
//!
//! covers: types.rs undo.rs batch.rs mod.rs maintainer.rs maintainer_wellformed.rs
//! covers: set.rs digest.rs rollback.rs block_handling.rs governance.rs periodic.rs
//! covers: maintainer_rewind/
//!
//! WHY THIS FILE IS THE HIGHEST-RISK PART OF THE FIX
//! -------------------------------------------------
//! `StateDb::get_undo` (`crates/storage/src/state_db/undo.rs`) is:
//!
//! ```ignore
//! let bytes = self.db.get_cf(cf, key).ok()??;
//! bincode::deserialize(&bytes).ok()
//! ```
//!
//! A DECODE FAILURE SILENTLY BECOMES `None`, and every caller reads `None` as
//! "no undo data for this height" — never as "this entry is unreadable". Bincode is
//! NON-SELF-DESCRIBING: field order and arity are implied by the type, not carried in the
//! bytes. So appending a field to `UndoData` makes every PRE-UPGRADE `cf_undo` entry
//! undecodable, and the node degrades to:
//!
//!   * `rollback_one_block` -> the rebuild-from-genesis fallback (AUDIT-P1-003 /
//!     INC-I-156 territory), and
//!   * `execute_reorg` -> its own fallback, because the
//!     `(target+1..=current).all(|h| get_undo(h).is_some())` gate
//!     (`bins/node/src/node/block_handling.rs`) evaluates false,
//!
//! for up to `UNDO_KEEP_DEPTH` blocks per node, starting at that node's restart —
//! with NOTHING in the logs saying so.
//!
//! `types.rs` already carries a `#[serde(default)]` on `epoch_state_snapshot` with a
//! "backward compat" comment. Analysis §9 item 2 asks whether bincode can honour that.
//! This file MEASURES the answer instead of assuming it.
//!
//! ===========================================================================
//! OUTPUT CONTRACT:
//! ===========================================================================
//! FUNCTIONS UNDER TEST
//!   F1 `bincode::serialize::<UndoData>` / `bincode::deserialize::<UndoData>` — the
//!      encoder/decoder pair that IS the `cf_undo` on-disk format
//!   F2 `StateDb::put_undo(height, &UndoData) -> Result<(), StorageError>`
//!   F3 `StateDb::get_undo(height) -> Option<UndoData>`
//!   F4 `StateDb::has_undo_data(from, to) -> bool` — the reorg-gate helper
//!
//! ENUMERATION OF OBSERVABLE OUTPUTS
//!   O1 return of F1 decode  — `Result<T, bincode::Error>`; Ok vs Err is the whole
//!                             question, and on Ok the FIELD VALUES must survive
//!   O2 return of F3         — `Some(UndoData)` vs `None`. The defect surface: `None`
//!                             is indistinguishable from "never written".
//!   O3 return of F4         — the boolean the reorg path branches on
//!   O4 persistent store     — the `cf_undo` bytes themselves, read back independently
//!   O5 blast-radius bound   — `UNDO_KEEP_DEPTH`, the number of blocks per node the
//!                             degradation can cover. An executable bound, not prose.
//!
//!   O6 `get_maintainer_undo(h)`— the SHIPPED mechanism's record, read back from the
//!                             real `cf_undo`. Its whole reason for existing is that it
//!                             does not disturb O1-O4 for any height.
//!
//! CODE PATHS
//!   P1 encode with schema A, decode with schema A            (round trip)
//!   P2 encode with the PRE-FIX schema, decode with a POST-FIX schema  (UPGRADE)
//!   P3 encode with a POST-FIX schema, decode with the PRE-FIX schema  (DOWNGRADE —
//!      the binary rollback that follows every bad deploy)
//!   P4 an entry present but undecodable, read through `get_undo` / `has_undo_data`
//!   P5 THE SHIPPED MECHANISM: a maintainer snapshot written under its own key family
//!      alongside an UNCHANGED `UndoData` record, read back through the real store
//!
//! INPUT PARTITIONS:
//!   IP-APPEND-OPT   post-fix schema = pre-fix + `Option<Vec<u8>>` with
//!                   `#[serde(default)]` — the NAIVE shape, and the one the existing
//!                   `epoch_state_snapshot` comment implies is safe
//!   IP-APPEND-BARE  post-fix schema = pre-fix + a non-Option `Vec<u8>` sentinel — the
//!                   shape that mirrors `producer_snapshot`'s empty-vector sentinel
//!   IP-HISTORIC     the schema that predates `epoch_state_snapshot` (3 fields),
//!                   decoded by TODAY's `UndoData`. Settles the "backward compat"
//!                   comment in `types.rs` with a measurement, in either direction.
//!   IP-MIXED        a height range where SOME entries decode and some do not — the
//!                   input the `execute_reorg` gate actually sees during the window
//!   IP-KEYED        the SHIPPED partition: a height range where SOME heights carry a
//!                   maintainer snapshot and some do not — i.e. the ordinary chain, in
//!                   which almost every block carries no rotation. This is the partition
//!                   that separates the shipped mechanism from every appended-field
//!                   alternative above: here the entries were written by the PRE-FIX
//!                   encoder and must STILL decode under the post-fix binary.
//!
//! MATRIX
//!   IP-APPEND-OPT  x P2 : O1
//!   IP-APPEND-BARE x P2 : O1
//!   IP-APPEND-OPT  x P3 : O1
//!   IP-HISTORIC    x P2 : O1
//!   round trip     x P1 : O1 O2 O4   (green control — the encoder must survive itself)
//!   IP-MIXED       x P4 : O2 O3
//!   IP-KEYED       x P5 : O1 O2 O3 O6
//!   bound              : O5
//!
//! ANTI-VACUITY
//!   The round-trip control (P1) is green today and must stay green. If a migration
//!   shape ever breaks the SAME-schema round trip, every assertion about cross-schema
//!   behaviour below is meaningless.
//!   For P5 the anti-vacuity control is `the_mirror_matches_the_real_undodata_encoding`:
//!   `PreFixUndo` is declared here as a five-field struct, so if the shipped fix had
//!   appended a sixth field to the real `UndoData`, that premise lock would FAIL and
//!   every P5 assertion would be known to be testing the wrong bytes.

use doli_core::consensus::UNDO_KEEP_DEPTH;
use doli_core::MaintainerSet;
use serde::{Deserialize, Serialize};
use storage::state_db::{MaintainerUndoSnapshot, StateDb, UndoData};
use storage::utxo::{Outpoint, UtxoEntry};

// ---------------------------------------------------------------------------
// Schema mirrors. Declared here, not imported, precisely BECAUSE the point is to
// encode with one arity and decode with another. Field ORDER and TYPES must track
// `crates/storage/src/state_db/types.rs` exactly — that is what makes the bytes
// produced here byte-identical to a real `cf_undo` value.
// ---------------------------------------------------------------------------

/// TODAY's `UndoData`, mirrored. Kept structurally identical so the fixture bytes are the
/// real pre-upgrade `cf_undo` encoding, and so a drift in the real type shows up as a
/// failure of `the_mirror_matches_the_real_undodata_encoding` below rather than as a
/// silently wrong premise for every other test in this file.
#[derive(Serialize, Deserialize)]
struct PreFixUndo {
    spent_utxos: Vec<(Outpoint, UtxoEntry)>,
    created_utxos: Vec<Outpoint>,
    producer_snapshot: Vec<u8>,
    #[serde(default)]
    epoch_state_snapshot: Option<Vec<u8>>,
    #[serde(default)]
    chain_commitment: Option<[u8; 32]>,
}

/// IP-APPEND-OPT — the naive post-fix shape: one appended `Option`, `#[serde(default)]`.
#[derive(Serialize, Deserialize)]
struct PostFixUndoAppendedOption {
    spent_utxos: Vec<(Outpoint, UtxoEntry)>,
    created_utxos: Vec<Outpoint>,
    producer_snapshot: Vec<u8>,
    #[serde(default)]
    epoch_state_snapshot: Option<Vec<u8>>,
    #[serde(default)]
    chain_commitment: Option<[u8; 32]>,
    #[serde(default)]
    maintainer_snapshot: Option<Vec<u8>>,
}

/// IP-APPEND-BARE — the `producer_snapshot`-shaped alternative: an appended `Vec<u8>`
/// whose EMPTY value is the "unchanged" sentinel.
#[derive(Serialize, Deserialize)]
struct PostFixUndoAppendedVec {
    spent_utxos: Vec<(Outpoint, UtxoEntry)>,
    created_utxos: Vec<Outpoint>,
    producer_snapshot: Vec<u8>,
    #[serde(default)]
    epoch_state_snapshot: Option<Vec<u8>>,
    #[serde(default)]
    chain_commitment: Option<[u8; 32]>,
    #[serde(default)]
    maintainer_snapshot: Vec<u8>,
}

/// IP-HISTORIC — the shape that predates `epoch_state_snapshot` and `chain_commitment`.
/// This is what the `types.rs` "None for blocks created before this field was added
/// (backward compat)" comment claims to support.
#[derive(Serialize, Deserialize)]
struct HistoricUndo {
    spent_utxos: Vec<(Outpoint, UtxoEntry)>,
    created_utxos: Vec<Outpoint>,
    producer_snapshot: Vec<u8>,
}

fn pre_fix_fixture() -> PreFixUndo {
    PreFixUndo {
        spent_utxos: Vec::new(),
        created_utxos: Vec::new(),
        producer_snapshot: vec![0xAB; 16],
        epoch_state_snapshot: Some(vec![0xCD; 8]),
        chain_commitment: None,
    }
}

/// A maintainer snapshot with a WELL-FORMED set: `validate_persisted_set` refuses a
/// duplicated member vector and an unreconciled threshold, so a lazy fixture would make
/// the restore path refuse for a reason these storage tests are not about.
fn snapshot_fixture() -> MaintainerUndoSnapshot {
    let members: Vec<crypto::PublicKey> = (0..4u8)
        .map(|i| crypto::PrivateKey::from_bytes([0x30 + i; 32]).public_key())
        .collect();
    // AUDIT-P1-001: the record is now self-describing and bound to a block. These storage
    // tests are about the KEY FAMILY (collision, pruning, round-trip), not about the
    // binding, so the block hash and digest are fixtures — but they go through the real
    // constructor so the header can never be forgotten here either.
    MaintainerUndoSnapshot::new(
        crypto::Hash::from_bytes([0x77; 32]),
        [0x88; 32],
        MaintainerSet::with_members(members, 9),
        9,
    )
}

fn real_fixture() -> UndoData {
    UndoData {
        spent_utxos: Vec::new(),
        created_utxos: Vec::new(),
        producer_snapshot: vec![0xAB; 16],
        epoch_state_snapshot: Some(vec![0xCD; 8]),
        chain_commitment: None,
    }
}

// ===========================================================================
// PREMISE LOCK — the mirror must encode exactly like the real type.
// ===========================================================================

/// Without this, every cross-schema assertion below is reasoning about a struct that is
/// not the one on disk.
#[test]
fn the_mirror_matches_the_real_undodata_encoding() {
    let mirrored = bincode::serialize(&pre_fix_fixture()).expect("mirror encodes");
    let real = bincode::serialize(&real_fixture()).expect("real encodes");
    assert_eq!(
        mirrored, real,
        "PREMISE: `PreFixUndo` must be a byte-exact mirror of `storage::UndoData`. If \
         this fails, the real type's field order or arity moved and every migration \
         assertion in this file is testing the wrong bytes."
    );
}

// ===========================================================================
// P1 — the same-schema round trip. GREEN control.
// ===========================================================================

/// O1 O2 O4 x P1. The encoder must survive its own decoder, through the REAL store.
#[test]
fn a_current_undo_entry_round_trips_through_the_real_cf_undo() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path()).expect("open StateDb");

    let entry = real_fixture();
    db.put_undo(7, &entry).expect("put_undo");

    // O2 — read back independently (assertion rule AQ-5).
    let got = db
        .get_undo(7)
        .expect("O2: a freshly written entry must read back as Some");
    // O4 — field values, not merely "it decoded".
    assert_eq!(got.producer_snapshot, vec![0xAB; 16], "O4");
    assert_eq!(got.epoch_state_snapshot, Some(vec![0xCD; 8]), "O4");
    assert_eq!(got.chain_commitment, None, "O4");
    assert!(got.spent_utxos.is_empty(), "O4");
    assert!(got.created_utxos.is_empty(), "O4");

    // O2 — and an absent height is None. This is the pair that makes the defect
    // invisible: `None` here and `None` for an UNREADABLE entry are the same value.
    assert!(
        db.get_undo(8).is_none(),
        "O2: an absent height reads as None — the SAME value an undecodable entry \
         produces. `get_undo` cannot distinguish them, and no caller can either."
    );

    // O3 — the gate helper agrees.
    assert!(db.has_undo_data(7, 7), "O3");
    assert!(
        !db.has_undo_data(7, 8),
        "O3: one missing height closes the gate"
    );
}

// ===========================================================================
// P2 — UPGRADE. The load-bearing measurement.
// ===========================================================================

/// REQ-174-004 bullet 1, IP-APPEND-OPT x P2. O1. **MEASURED — assert the real outcome.**
///
/// A `cf_undo` entry serialized by the PRE-FIX `UndoData` is fed to a POST-FIX schema that
/// appends one `Option<Vec<u8>>` carrying `#[serde(default)]`. No test may assume
/// `#[serde(default)]` works under bincode without demonstrating it — so this asserts what
/// actually happens, and the message states the consequence either way.
#[test]
fn req_174_004_a_pre_upgrade_entry_under_a_naively_appended_option_field() {
    let bytes = bincode::serialize(&pre_fix_fixture()).expect("pre-fix encode");
    let decoded = bincode::deserialize::<PostFixUndoAppendedOption>(&bytes);

    assert!(
        decoded.is_err(),
        "MEASUREMENT: appending an `Option<Vec<u8>>` with `#[serde(default)]` to \
         `UndoData` does NOT let bincode read pre-upgrade `cf_undo` entries. Bincode is \
         non-self-describing: `#[serde(default)]` only fires for a format that can report \
         a MISSING FIELD, and bincode reports EOF instead. If this assertion ever \
         FLIPS to Ok, bincode's behaviour changed and the whole migration hazard \
         (REQ-174-004) is void — re-derive it, do not assume either way.\n\
         CONSEQUENCE while this holds: every pre-upgrade entry reads as `None` through \
         `get_undo`, so `rollback_one_block` takes the rebuild-from-genesis fallback and \
         `execute_reorg`'s `all(|h| get_undo(h).is_some())` gate closes — SILENTLY, for \
         up to UNDO_KEEP_DEPTH blocks per node after its restart."
    );
}

/// IP-APPEND-BARE x P2. O1. **MEASURED.**
///
/// The `producer_snapshot`-shaped alternative — an appended bare `Vec<u8>` whose empty
/// value is the sentinel — is measured too, so the developer does not have to guess that
/// the two appended shapes behave alike under bincode.
#[test]
fn req_174_004_a_pre_upgrade_entry_under_a_naively_appended_vec_field() {
    let bytes = bincode::serialize(&pre_fix_fixture()).expect("pre-fix encode");
    let decoded = bincode::deserialize::<PostFixUndoAppendedVec>(&bytes);

    assert!(
        decoded.is_err(),
        "MEASUREMENT: the empty-vector sentinel shape fails identically. Choosing \
         `Vec<u8>` over `Option<Vec<u8>>` is a BLOAT decision (INC-I-071), not a \
         MIGRATION decision — it buys nothing here."
    );
}

/// IP-HISTORIC x P2. O1. **MEASURED — settles the comment already in `types.rs`.**
///
/// `types.rs` says `epoch_state_snapshot` is `None` "for blocks created before this field
/// was added (backward compat)". Analysis §9 item 2 flags that bincode probably cannot
/// honour it. This measures it directly, and its result decides whether that comment is a
/// documented behaviour or a documented MISTAKE — one the current fix would otherwise
/// repeat verbatim.
#[test]
fn req_174_004_the_existing_serde_default_backward_compat_claim_is_measured() {
    let historic = HistoricUndo {
        spent_utxos: Vec::new(),
        created_utxos: Vec::new(),
        producer_snapshot: vec![0x11; 4],
    };
    let bytes = bincode::serialize(&historic).expect("historic encode");
    let decoded = bincode::deserialize::<UndoData>(&bytes);

    assert!(
        decoded.is_err(),
        "MEASUREMENT: an undo entry written BEFORE `epoch_state_snapshot` existed cannot \
         be decoded by today's `UndoData`, despite the `#[serde(default)]` and the \
         \"backward compat\" comment at `types.rs`. The precedent in the file is \
         therefore a FALSE reassurance, and it is the reason REQ-174-004 exists: the next \
         field must not be added on the strength of that comment."
    );
}

// ===========================================================================
// P3 — DOWNGRADE. The rollback that follows every bad deploy.
// ===========================================================================

/// IP-APPEND-OPT x P3. O1. **MEASURED.**
///
/// Entries written by the POST-FIX binary, read by the PRE-FIX binary. This is what a
/// `doli upgrade` rollback does, and no requirement mentions it — which is exactly why it
/// needs a measurement rather than an assumption.
#[test]
fn req_174_004_a_post_fix_entry_read_by_the_pre_fix_binary_is_measured() {
    let post = PostFixUndoAppendedOption {
        spent_utxos: Vec::new(),
        created_utxos: Vec::new(),
        producer_snapshot: vec![0xAB; 16],
        epoch_state_snapshot: Some(vec![0xCD; 8]),
        chain_commitment: None,
        maintainer_snapshot: Some(vec![0xEE; 32]),
    };
    let bytes = bincode::serialize(&post).expect("post-fix encode");
    let decoded = bincode::deserialize::<UndoData>(&bytes);

    assert!(
        decoded.is_ok(),
        "MEASUREMENT: `bincode::deserialize` tolerates TRAILING BYTES, so the pre-fix \
         binary reads a post-fix entry successfully and simply ignores the appended \
         maintainer snapshot. DOWNGRADE is therefore SILENT AND LOSSY, not loud: a node \
         rolled back to the pre-fix binary keeps rewinding without ever restoring the \
         maintainer root, and nothing reports it. If this assertion flips, downgrade \
         becomes loud instead — also acceptable, but a DIFFERENT deploy story that the \
         release note must then describe."
    );
}

// ===========================================================================
// P4 — the mixed range the reorg gate actually sees, and the bound.
// ===========================================================================

/// REQ-174-004 bullet 3, IP-MIXED x P4. O2 O3. **MEASURED.**
///
/// `execute_reorg` gates on `(target+1..=current).all(|h| get_undo(h).is_some())`
/// (`bins/node/src/node/block_handling.rs`) and `has_undo_data` is the same question in
/// library form. During the migration window a rewind range spans BOTH pre-upgrade and
/// post-upgrade heights. This pins that ONE unreadable height in the range closes the gate
/// for the WHOLE range — the degradation is range-wide, not per-height.
#[test]
fn req_174_004_one_unreadable_height_closes_the_gate_for_the_whole_range() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path()).expect("open StateDb");

    // Heights 10..=14 written by the "post-upgrade" binary; height 12 stands in for a
    // pre-upgrade entry that the new schema cannot decode — simulated by leaving it
    // absent, because `get_undo` maps BOTH conditions to the same `None` (asserted in
    // `a_current_undo_entry_round_trips_through_the_real_cf_undo`).
    for h in [10u64, 11, 13, 14] {
        db.put_undo(h, &real_fixture()).expect("put_undo");
    }

    assert!(
        db.get_undo(12).is_none(),
        "O2: the stand-in for an undecodable entry"
    );
    assert!(
        !db.has_undo_data(10, 14),
        "O3: a single unreadable height inside the rewind range closes the gate for the \
         ENTIRE range. During the migration window that is the common case, not the \
         corner case — so the reorg path drops to its fallback for every reorg that \
         spans the upgrade point."
    );
    assert!(
        db.has_undo_data(13, 14),
        "O3: control — a range entirely above the unreadable height still passes, so the \
         assertion above is about the mixed range and not about a broken helper"
    );
}

// ===========================================================================
// P5 — THE SHIPPED MECHANISM. Every measurement above rejects an alternative;
// these measure what was actually built.
//
// REQ-174-004 asked what a pre-upgrade `cf_undo` entry does under the new binary.
// Under the shipped mechanism the answer is "exactly what it did before", because
// `UndoData`'s encoding never moved and the maintainer snapshot lives under its own
// key family. These tests hold that answer in place: if anyone later appends a field
// to `UndoData` after all, they FAIL, and they fail naming the reason.
// ===========================================================================

/// IP-KEYED x P5. O1 O2 O6. **MEASURED — the property the whole mechanism choice buys.**
///
/// Entries written by the PRE-FIX encoder (`PreFixUndo`, byte-locked to the real
/// `UndoData` by `the_mirror_matches_the_real_undodata_encoding`) are put into the real
/// `cf_undo` at the same heights the post-fix binary then writes maintainer snapshots to.
/// Every pre-upgrade entry must STILL read back as `Some`, with its field values intact.
///
/// This is the exact input that `req_174_004_a_pre_upgrade_entry_under_a_naively_appended_option_field`
/// measures to be UNDECODABLE under the rejected alternative.
#[test]
fn req_174_004_the_shipped_key_family_leaves_every_pre_upgrade_entry_readable() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path()).expect("open StateDb");

    // Write heights 20..=24 with the PRE-FIX encoder, straight into the real CF, so the
    // bytes on disk are the bytes a node running the previous binary would have left.
    let cf_undo_bytes = bincode::serialize(&pre_fix_fixture()).expect("pre-fix encode");
    for h in 20u64..=24 {
        db.put_undo(
            h,
            &bincode::deserialize::<UndoData>(&cf_undo_bytes).expect(
                "PREMISE: the pre-fix encoding must decode as today's UndoData — that is what \
             makes these fixtures genuine pre-upgrade entries rather than new ones",
            ),
        )
        .expect("put_undo");
    }

    // Now the post-fix binary records a maintainer rotation at h=22 only.
    db.put_maintainer_undo(22, &snapshot_fixture())
        .expect("put_maintainer_undo");

    for h in 20u64..=24 {
        let got = db.get_undo(h).unwrap_or_else(|| {
            panic!(
                "O1/O2: the pre-upgrade `cf_undo` entry at h={h} must still decode after the \
                 INC-I-174 fix. If this is None, a field was appended to `UndoData` after \
                 all, and every node is now silently on the rebuild-from-genesis fallback \
                 for UNDO_KEEP_DEPTH blocks after its restart."
            )
        });
        assert_eq!(
            got.producer_snapshot,
            vec![0xAB; 16],
            "O1: field values too"
        );
        assert_eq!(got.epoch_state_snapshot, Some(vec![0xCD; 8]), "O1");
    }

    // O6 — the maintainer record is present at h=22 and ONLY at h=22: absence is the
    // "unchanged at this height" sentinel and costs zero bytes (INC-I-071 discipline).
    assert_eq!(
        db.get_maintainer_undo(22),
        Some(snapshot_fixture()),
        "O6: the snapshot must round trip through the real cf_undo, field for field"
    );
    for h in [20u64, 21, 23, 24] {
        assert!(
            db.get_maintainer_undo(h).is_none(),
            "O6: h={h} carried no rotation, so it must hold NO maintainer record"
        );
    }
}

/// IP-KEYED x P5. O3. **MEASURED — the reorg gate is not disturbed.**
///
/// `req_174_004_one_unreadable_height_closes_the_gate_for_the_whole_range` measures that
/// ONE unreadable height closes `execute_reorg`'s gate for the entire rewind range. That
/// is precisely the damage the rejected alternative would have done at every height
/// written before the upgrade. This measures that the shipped mechanism does not: a range
/// spanning the upgrade point, with a maintainer record inserted in the middle, still
/// passes the gate.
#[test]
fn req_174_004_the_shipped_key_family_does_not_close_the_reorg_gate() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path()).expect("open StateDb");

    for h in 30u64..=34 {
        db.put_undo(h, &real_fixture()).expect("put_undo");
    }
    db.put_maintainer_undo(32, &snapshot_fixture())
        .expect("put_maintainer_undo");

    assert!(
        db.has_undo_data(30, 34),
        "O3: the gate `execute_reorg` branches on must stay OPEN across a range that \
         spans the upgrade point. Under the rejected appended-field shape this is where \
         the reorg would drop to its rebuild-from-genesis fallback."
    );
}

/// IP-KEYED x P5. O6. **MEASURED — the key families cannot collide.**
///
/// The two families are distinguished by key LENGTH (8 bytes vs 9), which is what makes
/// a collision impossible at ANY height rather than merely unlikely. Height `0x4D` (77)
/// is the adversarial case: its 8-byte little-endian key STARTS with the same byte the
/// maintainer family uses as its prefix.
#[test]
fn req_174_004_the_two_cf_undo_key_families_cannot_collide() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path()).expect("open StateDb");

    // 0x4D == 77 — the maintainer key prefix, read as a height.
    for h in [0u64, 77, 1] {
        db.put_undo(h, &real_fixture()).expect("put_undo");
        db.put_maintainer_undo(h, &snapshot_fixture())
            .expect("put_maintainer_undo");
    }

    for h in [0u64, 77, 1] {
        assert!(
            db.get_undo(h).is_some(),
            "O6: the UndoData record at h={h} must survive a maintainer record at the \
             SAME height"
        );
        assert_eq!(
            db.get_maintainer_undo(h),
            Some(snapshot_fixture()),
            "O6: and vice versa, at h={h}"
        );
    }

    // Deleting one family must not touch the other.
    db.delete_maintainer_undo(77).expect("delete");
    assert!(
        db.get_maintainer_undo(77).is_none(),
        "O6: the maintainer record at h=77 is gone"
    );
    assert!(
        db.get_undo(77).is_some(),
        "O6: and the UndoData record at h=77 — whose 8-byte key begins with the SAME \
         0x4D byte the maintainer family uses as its prefix — is untouched"
    );
}

/// IP-KEYED x P5. O6. **MEASURED — the new records do not leak past the horizon.**
///
/// INC-I-071 is the precedent: `cf_undo` grew to 605 MB on mainnet because per-block
/// snapshots were written and the pruning story was not checked. Both bulk pruners used
/// to filter on `key.len() == 8`, which would have skipped every maintainer record
/// forever. This pins that BOTH families are pruned, in both directions.
#[test]
fn req_174_004_pruning_covers_the_maintainer_key_family_in_both_directions() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path()).expect("open StateDb");

    for h in 40u64..=44 {
        db.put_undo(h, &real_fixture()).expect("put_undo");
        db.put_maintainer_undo(h, &snapshot_fixture())
            .expect("put_maintainer_undo");
    }

    // Per-block horizon walk: h=40 ages out.
    db.prune_undo_before(41);
    assert!(
        db.get_undo(40).is_none(),
        "control: UndoData at h=40 pruned"
    );
    assert!(
        db.get_maintainer_undo(40).is_none(),
        "O6: the maintainer record must age out WITH its height. Left behind, it grows \
         without bound — the INC-I-071 shape — and a much later re-apply of that height \
         would find a stale pre-block trust root."
    );

    // Truncation above a height.
    db.prune_undo_above(42);
    for h in [43u64, 44] {
        assert!(
            db.get_undo(h).is_none(),
            "control: UndoData at h={h} pruned"
        );
        assert!(
            db.get_maintainer_undo(h).is_none(),
            "O6: prune_undo_above must cover the maintainer family at h={h}"
        );
    }
    assert!(
        db.get_maintainer_undo(42).is_some(),
        "O6: control — the boundary height itself is KEPT, so the assertion above is \
         about the range and not about a pruner that deletes everything"
    );

    // Bulk catch-up below a height.
    let deleted = db.prune_undo_below(42);
    assert!(
        deleted >= 2,
        "O6: prune_undo_below must count BOTH families it deleted at h=41, not just the \
         UndoData one (got {deleted})"
    );
    assert!(
        db.get_maintainer_undo(41).is_none(),
        "O6: prune_undo_below must cover the maintainer family too"
    );
}

/// REQ-174-004 bullet 2, O5. The blast radius, as an executable bound.
///
/// The requirement says the degradation must be "bounded". This pins the number the bound
/// is made of, so a future change to `UNDO_KEEP_DEPTH` cannot silently widen the exposure
/// the deploy note describes.
#[test]
fn req_174_004_the_migration_window_is_bounded_by_undo_keep_depth() {
    assert_eq!(
        UNDO_KEEP_DEPTH, 100,
        "REQ-174-004: the exposure window is `UNDO_KEEP_DEPTH` blocks PER NODE, starting \
         at that node's restart — about 17 minutes at 10 s slots. The deploy note and the \
         staggered-restart instruction are both written against this number; if it moves, \
         they must move with it."
    );
}
