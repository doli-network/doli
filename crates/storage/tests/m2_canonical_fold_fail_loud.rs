//! UTXO-scalability M2 [F2] — AP-7 fail-loud lock for the streaming canonical fold.
//!
//! REQ-SCALE-001 / INV-SYNC-007 — Decision: a failure here would reveal the fold can emit a
//! self-consistent digest over N-1 entries when one value is undecodable — a node that
//! computes and gossips a WRONG state root while believing it is healthy.
//! REQ-SCALE-006 — Decision: a silent skip would reveal the M2 fold reproduced the
//! `.filter_map(|r| r.ok())` defect that `queries.rs:483-501` carries today inside a
//! consensus serializer.
//!
//! RED PHASE: `canonical_digest()` and `canonical_range()` do not exist yet. Every test here
//! except the clearly-marked PRE-M2 COMPANION is expected NOT TO COMPILE until M2 lands.
//!
//! OWN TEST BINARY on purpose (declared in Cargo.toml): same reason as the sibling
//! byte-equality lock — it names an API that does not exist yet.
//!
//! No production seam was added. The corrupt value is injected by dropping the `StateDb`
//! handle, re-opening the same directory with the raw `rocksdb` API, and writing bytes into
//! `cf_utxo`. `StateDb::open` counts rows without deserialising them, so the injected row
//! survives the re-open and is seen only by a decoder.
//!
//! OUTPUT CONTRACT:
//!   fn UtxoSet::canonical_digest(&self) -> Result<crypto::Hash, StorageError>
//!     O1 Ok(Hash)     O2 Err
//!   fn UtxoSet::canonical_range(&self, Option<&[u8]>, usize)
//!        -> Result<(Vec<u8>, Option<Vec<u8>>), StorageError>
//!     O3 Ok(body, cursor)     O4 Err
//!   Paths:
//!     P1 undecodable value mid-set, state_db  — THIS FILE (O2, O4)
//!     P2 empty set, both backends             — THIS FILE (O1, O3)
//!     P3 single entry, both backends          — THIS FILE (O1, O3)
//!     P4 populated well-formed set            — m2_canonical_fold_byte_equality.rs
//!     P5 RocksDB iterator error               — NOT reachable from the stable API; the
//!        decode-failure partition (P1) is the reachable half of the same `.ok()` defect.
//!   MATRIX: P1xO2 -> m2_canonical_digest_is_err_on_an_undecodable_entry
//!           P1xO4 -> m2_canonical_range_is_err_on_an_undecodable_entry
//!           P1xO2 -> m2_digest_over_a_corrupt_set_is_never_the_digest_of_the_survivors
//!           P2xO1,O3 -> m2_empty_set_digest_and_range_agree_with_the_canonical_image
//!           P3xO1,O3 -> m2_single_entry_digest_and_range_agree_with_the_canonical_image
//! INPUT PARTITIONS: one undecodable value at the median key of a 256-entry set; the empty
//!   set; the single-entry set. Median placement is deliberate — a corrupt FIRST or LAST
//!   entry could pass an implementation that only guards the loop edges.

mod common;

use std::path::Path;
use std::sync::Arc;

use storage::utxo::UtxoSet;
use storage::{StateDb, UtxoEntry};
use tempfile::TempDir;

const CORRUPT_N: usize = 256;
const CORRUPT_SEED: u64 = 0x4641_494C_4C4F_5544;
const CHUNK_SMALL: usize = 1_024;
const CHUNK_HUGE: usize = 64 * 1024 * 1024;

/// Not a valid bincode `UtxoEntry`: the leading u32 is an out-of-range `OutputType`
/// variant index. Asserted undecodable before use, so the fixture cannot go vacuous.
fn corrupt_value() -> Vec<u8> {
    vec![0xFFu8; 24]
}

fn inject_raw_utxo_value(dir: &Path, key: &[u8], value: &[u8]) {
    let opts = rocksdb::Options::default();
    let cfs = rocksdb::DB::list_cf(&opts, dir).expect("fixture: list_cf on a closed state_db");
    let db = rocksdb::DB::open_cf(&opts, dir, &cfs).expect("fixture: raw open_cf");
    let cf = db.cf_handle("cf_utxo").expect("fixture: cf_utxo handle");
    db.put_cf(cf, key, value)
        .expect("fixture: raw put into cf_utxo");
}

/// Build a state_db-backed set of `CORRUPT_N` entries, then overwrite the value at the
/// median key with undecodable bytes. Returns the reopened set and that key.
fn set_with_one_undecodable_entry(dir: &TempDir) -> (UtxoSet, Vec<u8>) {
    let entries = common::randomized_entries(CORRUPT_N, CORRUPT_SEED);
    drop(common::build_rocksdb(dir.path(), &entries));

    let mut keys: Vec<Vec<u8>> = entries.iter().map(|(o, _)| o.to_bytes()).collect();
    keys.sort();
    let key = keys[CORRUPT_N / 2].clone();

    let corrupt = corrupt_value();
    assert!(
        bincode::deserialize::<UtxoEntry>(&corrupt).is_err(),
        "fixture: the injected value must be undecodable or the test asserts nothing"
    );
    inject_raw_utxo_value(dir.path(), &key, &corrupt);

    let sdb = Arc::new(StateDb::open(dir.path()).expect("fixture: reopen state_db"));
    assert_eq!(
        sdb.utxo_len(),
        CORRUPT_N,
        "fixture: the corrupt row must still be a row in cf_utxo"
    );

    (UtxoSet::from_state_db(sdb), key)
}

fn clean_set(dir: &TempDir) -> UtxoSet {
    let entries = common::randomized_entries(CORRUPT_N, CORRUPT_SEED);
    common::build_rocksdb(dir.path(), &entries)
}

/// REQ-SCALE-001 — Decision: a failure here would reveal the node computes a state root over
/// the entries it happened to decode, publishing a wrong-but-self-consistent root instead of
/// halting — the exact AP-7 defect M2 removes.
#[test]
fn m2_canonical_digest_is_err_on_an_undecodable_entry() {
    let dir = TempDir::new().expect("tempdir");
    let (set, _key) = set_with_one_undecodable_entry(&dir);

    assert!(
        set.canonical_digest().is_err(),
        "an undecodable cf_utxo value must fail the digest loud, not be skipped"
    );
}

/// REQ-SCALE-001 — Decision: a failure here would reveal a state-transfer server can ship a
/// chunk that silently omits an entry, so the receiver rebuilds a set with a different root.
#[test]
fn m2_canonical_range_is_err_on_an_undecodable_entry() {
    let dir = TempDir::new().expect("tempdir");
    let (set, _key) = set_with_one_undecodable_entry(&dir);

    assert!(
        set.canonical_range(None, CHUNK_HUGE).is_err(),
        "a single oversized range spanning the corrupt entry must be an Err"
    );

    let mut cursor: Option<Vec<u8>> = None;
    let mut saw_err = false;
    for _ in 0..(4 * CORRUPT_N + 16) {
        match set.canonical_range(cursor.as_deref(), CHUNK_SMALL) {
            Err(_) => {
                saw_err = true;
                break;
            }
            Ok((_, Some(next))) => cursor = Some(next),
            Ok((_, None)) => break,
        }
    }
    assert!(
        saw_err,
        "a chunked walk across the corrupt entry must surface an Err, not finish clean"
    );
}

/// REQ-SCALE-001 — Decision: a failure here would reveal the fold produced exactly the
/// digest of the surviving N-1 entries, which is the wrong root a healthy-looking node
/// would gossip.
#[test]
fn m2_digest_over_a_corrupt_set_is_never_the_digest_of_the_survivors() {
    let corrupt_dir = TempDir::new().expect("tempdir");
    let (corrupt, _key) = set_with_one_undecodable_entry(&corrupt_dir);

    let clean_dir = TempDir::new().expect("tempdir");
    let clean = clean_set(&clean_dir);
    let clean_digest = clean
        .canonical_digest()
        .expect("a well-formed set must fold");

    match corrupt.canonical_digest() {
        Err(_) => {}
        Ok(d) => panic!(
            "the corrupt set folded to {} (clean set folds to {}); a skipped entry is a \
             wrong state root, not a degraded one",
            d.to_hex(),
            clean_digest.to_hex()
        ),
    }
}

/// PRE-M2 COMPANION — documents the behavior M2 replaces. Delete this whole fn together
/// with `UtxoSet::serialize_canonical()`.
/// REQ-SCALE-001 — Decision: a failure here would reveal the injection fixture no longer
/// reaches the defect, which would make every Err assertion above vacuous.
#[test]
fn m2_pre_m2_serialize_canonical_silently_drops_the_undecodable_entry() {
    let dir = TempDir::new().expect("tempdir");
    let (set, _key) = set_with_one_undecodable_entry(&dir);

    let image = set.serialize_canonical();
    let header = u64::from_le_bytes(image[0..8].try_into().expect("8-byte header"));

    assert_eq!(
        header,
        (CORRUPT_N - 1) as u64,
        "pre-M2 the serializer emits N-1 entries and a header that agrees with itself"
    );
}

/// REQ-SCALE-001 — Decision: a failure here would reveal the fold's empty case differs from
/// the materialised image, so a node with a drained set would compute a different root than
/// one that serialised it.
#[test]
fn m2_empty_set_digest_and_range_agree_with_the_canonical_image() {
    let dir = TempDir::new().expect("tempdir");
    let empty: [(storage::Outpoint, UtxoEntry); 0] = [];

    for set in [
        common::build_in_memory(&empty),
        common::build_rocksdb(dir.path(), &empty),
    ] {
        let image = set.serialize_canonical();
        assert_eq!(image.len(), 8, "an empty image is the header alone");
        assert_eq!(
            set.canonical_digest().expect("empty digest"),
            crypto::hash::hash(&image),
            "the empty-set digest must still hash the 8-byte zero header"
        );

        let (chunk, next) = set
            .canonical_range(None, CHUNK_HUGE)
            .expect("empty range must succeed");
        assert!(chunk.is_empty(), "an empty set has an empty body");
        assert_eq!(next, None, "an empty set is immediately exhausted");
    }
}

/// REQ-SCALE-001 — Decision: a failure here would reveal an off-by-one at the single-entry
/// boundary, where the first entry is also the last and a loop-edge guard can hide.
#[test]
fn m2_single_entry_digest_and_range_agree_with_the_canonical_image() {
    let dir = TempDir::new().expect("tempdir");
    let one = common::randomized_entries(1, CORRUPT_SEED);

    for set in [
        common::build_in_memory(&one),
        common::build_rocksdb(dir.path(), &one),
    ] {
        let image = set.serialize_canonical();
        assert_eq!(
            set.canonical_digest().expect("single-entry digest"),
            crypto::hash::hash(&image)
        );

        let (chunk, next) = set
            .canonical_range(None, CHUNK_HUGE)
            .expect("single-entry range must succeed");
        assert_eq!(chunk, image[8..], "the body is the one entry");
        assert_eq!(next, None, "one entry exhausts the range");
    }
}
