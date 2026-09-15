//! UTXO-scalability M2 [F2] — streaming canonical fold byte-equality lock.
//!
//! REQ-SCALE-001 / INV-SYNC-007 — Decision: a failure here would reveal the streaming fold
//! hashes different bytes, or the same bytes in a different order, than the materialised
//! image every live node has been hashing — a silent chain split at the next root compare.
//! REQ-SCALE-006 — Decision: a divergence between the two backends would reveal that a
//! snap-synced (state_db) node and a full node no longer agree on the UTXO component.
//! REQ-SCALE-018 — Decision: a cursor that depends on the chunk size would reveal the range
//! API cannot serve a bounded transfer without re-reading or dropping entries.
//!
//! RED PHASE: `canonical_digest()` and `canonical_range()` do not exist yet. This file is
//! expected NOT TO COMPILE until M2 lands; that is the contract, not a defect.
//!
//! OWN TEST BINARY on purpose (declared in Cargo.toml): it names an API that does not exist
//! yet, so folding it into `tests/it/` would make every unrelated storage integration test
//! uncompilable for the whole milestone.
//!
//! OUTPUT CONTRACT:
//!   fn UtxoSet::canonical_digest(&self) -> Result<crypto::Hash, StorageError>
//!     O1 return Ok(Hash)                O2 return Err
//!   fn UtxoSet::canonical_range(&self, Option<&[u8]>, usize)
//!        -> Result<(Vec<u8>, Option<Vec<u8>>), StorageError>
//!     O3 body bytes of the chunk        O4 next cursor (Some/None)
//!     O5 return Err
//!   Outputs NOT produced (asserted by absence): the set itself is not mutated (O6 count).
//!   Paths:
//!     P1 InMemory, populated                   — THIS FILE (O1, O3, O4, O6)
//!     P2 RocksDb, populated                    — THIS FILE (O1, O3, O4, O6)
//!     P3 max_bytes < one entry                 — THIS FILE (O3, O4)
//!     P4 max_bytes > whole set                 — THIS FILE (O3, O4)
//!     P5 start_key between two keys            — THIS FILE (O3, O4)
//!     P6 start_key past the last key           — THIS FILE (O3, O4)
//!     P7 empty / single entry                  — m2_canonical_fold_fail_loud.rs
//!     P8 undecodable entry / iterator error    — m2_canonical_fold_fail_loud.rs (O2, O5)
//!   MATRIX: P1xO1 -> m2_digest_equals_hash_of_the_canonical_image_in_memory
//!           P2xO1 -> m2_digest_equals_hash_of_the_canonical_image_state_db
//!           P1xO3,O4 -> m2_range_concat_equals_the_canonical_body_in_memory
//!           P2xO3,O4 -> m2_range_concat_equals_the_canonical_body_state_db
//!           P1xO1,O3 vs P2xO1,O3 -> m2_backends_agree_on_digest_and_range_bytes
//!           P3xO3,O4 -> m2_range_makes_progress_when_max_bytes_is_below_one_entry
//!           P4xO3,O4 -> m2_range_wider_than_the_set_returns_the_whole_body_and_no_cursor
//!           P5xO3,O4 -> m2_range_starts_at_the_first_key_at_or_after_the_cursor
//!           P6xO3,O4 -> m2_range_past_the_last_key_is_empty_and_terminal
//!           P1xO6,P2xO6 -> m2_folding_does_not_mutate_the_set
//! INPUT PARTITIONS: >=100,000 randomized entries (the architecture gate's threshold) for
//!   the equality locks; a 64-entry set for the cursor-semantics partitions, where the exact
//!   boundary keys must be nameable.

mod common;

use storage::utxo::UtxoSet;
use tempfile::TempDir;

/// The architecture gate's threshold (`specs/utxo-scalability-architecture.md` [F2]).
const GATE_N: usize = 100_000;
const GATE_SEED: u64 = 0x00D0_1100_2026_0915;
const SMALL_N: usize = 64;
const SMALL_SEED: u64 = 0x5343_414C_4532_3032;

/// Splits the gate set into ~166 chunks — the cursor must survive many mid-set boundaries.
const CHUNK_SMALL: usize = 65_536;
/// Larger than the whole gate set — one chunk, no cursor.
const CHUNK_HUGE: usize = 64 * 1024 * 1024;

/// Drive `canonical_range` to exhaustion and return the concatenated body.
/// Asserts the cursor strictly advances, so a non-terminating implementation fails
/// as a test failure rather than as a hung suite.
fn fold_body(set: &UtxoSet, max_bytes: usize) -> Vec<u8> {
    let mut body = Vec::new();
    let mut cursor: Option<Vec<u8>> = None;
    let mut rounds = 0usize;

    loop {
        let (chunk, next) = set
            .canonical_range(cursor.as_deref(), max_bytes)
            .expect("canonical_range must succeed on a well-formed set");
        rounds += 1;
        assert!(
            rounds <= 4 * GATE_N + 16,
            "canonical_range did not terminate after {} calls (max_bytes={})",
            rounds,
            max_bytes
        );

        match next {
            Some(next_key) => {
                assert!(
                    !chunk.is_empty(),
                    "a non-terminal chunk must carry at least one entry, or the cursor \
                     cannot make progress (max_bytes={})",
                    max_bytes
                );
                if let Some(prev) = cursor.as_ref() {
                    assert!(
                        next_key.as_slice() > prev.as_slice(),
                        "the cursor must advance strictly (prev={:?}, next={:?})",
                        prev,
                        next_key
                    );
                }
                body.extend_from_slice(&chunk);
                cursor = Some(next_key);
            }
            None => {
                body.extend_from_slice(&chunk);
                break;
            }
        }
    }

    body
}

fn canonical_header_count(image: &[u8]) -> u64 {
    u64::from_le_bytes(
        image[0..8]
            .try_into()
            .expect("canonical image has an 8-byte header"),
    )
}

/// REQ-SCALE-001 — Decision: a failure here would reveal the in-memory fold hashes a
/// different byte sequence than the image every node hashes today.
#[test]
fn m2_digest_equals_hash_of_the_canonical_image_in_memory() {
    let entries = common::randomized_entries(GATE_N, GATE_SEED);
    let set = common::build_in_memory(&entries);
    let image = set.serialize_canonical();

    assert_eq!(canonical_header_count(&image), GATE_N as u64);
    assert_eq!(
        set.canonical_digest()
            .expect("canonical_digest must succeed"),
        crypto::hash::hash(&image),
        "the streaming digest must equal the hash of the materialised canonical image"
    );
}

/// REQ-SCALE-001 — Decision: a failure here would reveal the state_db fold hashes a
/// different byte sequence than the production serializer it replaces.
#[test]
fn m2_digest_equals_hash_of_the_canonical_image_state_db() {
    let entries = common::randomized_entries(GATE_N, GATE_SEED);
    let dir = TempDir::new().expect("tempdir");
    let set = common::build_rocksdb(dir.path(), &entries);
    let image = set.serialize_canonical();

    assert_eq!(canonical_header_count(&image), GATE_N as u64);
    assert_eq!(
        set.canonical_digest()
            .expect("canonical_digest must succeed"),
        crypto::hash::hash(&image),
        "the streaming digest must equal the hash of the materialised canonical image"
    );
}

/// REQ-SCALE-001 / REQ-SCALE-018 — Decision: a failure here would reveal the range API
/// drops, duplicates, or reorders entries, so a chunked state transfer would rebuild a
/// set whose root does not match the header it was advertised under.
#[test]
fn m2_range_concat_equals_the_canonical_body_in_memory() {
    let entries = common::randomized_entries(GATE_N, GATE_SEED);
    let set = common::build_in_memory(&entries);
    let image = set.serialize_canonical();
    let body = &image[8..];

    assert_eq!(canonical_header_count(&image), GATE_N as u64);
    assert_eq!(
        fold_body(&set, CHUNK_SMALL),
        body,
        "the concatenated {}-byte chunks must equal the canonical body exactly",
        CHUNK_SMALL
    );
    assert_eq!(
        fold_body(&set, CHUNK_HUGE),
        body,
        "the canonical body must not depend on the chunk size"
    );
}

/// REQ-SCALE-001 / REQ-SCALE-018 — Decision: same divergence as the in-memory case, on the
/// backend that actually serves production state transfers.
#[test]
fn m2_range_concat_equals_the_canonical_body_state_db() {
    let entries = common::randomized_entries(GATE_N, GATE_SEED);
    let dir = TempDir::new().expect("tempdir");
    let set = common::build_rocksdb(dir.path(), &entries);
    let image = set.serialize_canonical();
    let body = &image[8..];

    assert_eq!(canonical_header_count(&image), GATE_N as u64);
    assert_eq!(fold_body(&set, CHUNK_SMALL), body);
    assert_eq!(fold_body(&set, CHUNK_HUGE), body);
}

/// REQ-SCALE-006 / INV-SYNC-007 — Decision: a failure here would reveal that which backend
/// holds the set changes the consensus bytes, which is the snap-sync fork class INV-SYNC-007
/// exists to forbid.
#[test]
fn m2_backends_agree_on_digest_and_range_bytes() {
    let entries = common::randomized_entries(GATE_N, GATE_SEED);
    let mem = common::build_in_memory(&entries);
    let dir = TempDir::new().expect("tempdir");
    let rocks = common::build_rocksdb(dir.path(), &entries);

    assert_eq!(
        mem.canonical_digest().expect("in-memory digest"),
        rocks.canonical_digest().expect("state_db digest"),
        "both backends must fold to the same digest"
    );
    assert_eq!(
        fold_body(&mem, CHUNK_SMALL),
        fold_body(&rocks, CHUNK_SMALL),
        "both backends must stream the same body bytes in the same order"
    );
}

/// REQ-SCALE-018 — Decision: a failure here would reveal a caller can request a chunk too
/// small to hold one entry and get an empty non-terminal chunk — an infinite transfer loop.
#[test]
fn m2_range_makes_progress_when_max_bytes_is_below_one_entry() {
    let entries = common::randomized_entries(SMALL_N, SMALL_SEED);
    let dir = TempDir::new().expect("tempdir");

    for set in [
        common::build_in_memory(&entries),
        common::build_rocksdb(dir.path(), &entries),
    ] {
        let image = set.serialize_canonical();
        let (chunk, next) = set
            .canonical_range(None, 1)
            .expect("canonical_range must succeed with an undersized budget");

        assert!(
            !chunk.is_empty(),
            "a budget below one entry must still emit exactly one entry"
        );
        assert_eq!(
            chunk,
            image[8..8 + chunk.len()],
            "the single emitted entry must be the first entry of the canonical body"
        );
        assert!(
            next.is_some(),
            "a set of {} entries is not exhausted by one entry",
            SMALL_N
        );
        assert_eq!(
            fold_body(&set, 1),
            image[8..],
            "a one-byte budget must still stream the whole body"
        );
    }
}

/// REQ-SCALE-018 — Decision: a failure here would reveal the terminal chunk still returns a
/// cursor, so a transfer would never learn it is finished.
#[test]
fn m2_range_wider_than_the_set_returns_the_whole_body_and_no_cursor() {
    let entries = common::randomized_entries(SMALL_N, SMALL_SEED);
    let dir = TempDir::new().expect("tempdir");

    for set in [
        common::build_in_memory(&entries),
        common::build_rocksdb(dir.path(), &entries),
    ] {
        let image = set.serialize_canonical();
        let (chunk, next) = set
            .canonical_range(None, CHUNK_HUGE)
            .expect("canonical_range must succeed");

        assert_eq!(
            chunk,
            image[8..],
            "one oversized chunk must be the whole body"
        );
        assert_eq!(next, None, "an exhausted range must report no next cursor");
    }
}

/// REQ-SCALE-018 — Decision: a failure here would reveal the cursor is exclusive rather than
/// inclusive (`>=`), which silently drops exactly one entry per chunk boundary.
#[test]
fn m2_range_starts_at_the_first_key_at_or_after_the_cursor() {
    let entries = common::randomized_entries(SMALL_N, SMALL_SEED);
    let dir = TempDir::new().expect("tempdir");

    let mut keys: Vec<Vec<u8>> = entries.iter().map(|(o, _)| o.to_bytes()).collect();
    keys.sort();

    for set in [
        common::build_in_memory(&entries),
        common::build_rocksdb(dir.path(), &entries),
    ] {
        let image = set.serialize_canonical();
        let body = &image[8..];

        let (from_start, _) = set
            .canonical_range(Some(&keys[0]), CHUNK_HUGE)
            .expect("canonical_range from the first key");
        assert_eq!(
            from_start, body,
            "a cursor equal to the lowest key must include that key (>= semantics)"
        );

        let mid = &keys[SMALL_N / 2];
        let (from_mid, _) = set
            .canonical_range(Some(mid), CHUNK_HUGE)
            .expect("canonical_range from the middle key");
        assert_eq!(
            &from_mid[0..36],
            mid.as_slice(),
            "the first entry served must be keyed by the cursor itself"
        );
        assert_eq!(
            from_mid,
            body[body.len() - from_mid.len()..],
            "the tail from the middle cursor must be the canonical body's own tail"
        );

        let mut below_first = keys[0].clone();
        below_first[35] = 0;
        below_first[0] = 0;
        let (from_below, _) = set
            .canonical_range(Some(&below_first), CHUNK_HUGE)
            .expect("canonical_range from below the lowest key");
        assert_eq!(
            from_below, body,
            "a cursor below every key must serve the whole body"
        );
    }
}

/// REQ-SCALE-018 — Decision: a failure here would reveal a past-the-end cursor still yields
/// bytes, which would make a resumed transfer append duplicate entries.
#[test]
fn m2_range_past_the_last_key_is_empty_and_terminal() {
    let entries = common::randomized_entries(SMALL_N, SMALL_SEED);
    let dir = TempDir::new().expect("tempdir");
    let past_end = vec![0xFFu8; 36];

    for set in [
        common::build_in_memory(&entries),
        common::build_rocksdb(dir.path(), &entries),
    ] {
        let (chunk, next) = set
            .canonical_range(Some(&past_end), CHUNK_HUGE)
            .expect("canonical_range past the last key must succeed");
        assert!(chunk.is_empty(), "no entry sorts at or after 0xFF..FF");
        assert_eq!(next, None, "a past-the-end cursor must be terminal");
    }
}

/// REQ-SCALE-006 — Decision: a failure here would reveal the fold mutates the set it reads,
/// which would make a root compute change the state it is committing to.
#[test]
fn m2_folding_does_not_mutate_the_set() {
    let entries = common::randomized_entries(SMALL_N, SMALL_SEED);
    let dir = TempDir::new().expect("tempdir");

    for set in [
        common::build_in_memory(&entries),
        common::build_rocksdb(dir.path(), &entries),
    ] {
        let before = set.serialize_canonical();
        let count_before = set.utxo_count();

        let _ = set.canonical_digest().expect("canonical_digest");
        let _ = fold_body(&set, CHUNK_SMALL);

        assert_eq!(
            set.utxo_count(),
            count_before,
            "the fold must not change the entry count"
        );
        assert_eq!(
            set.serialize_canonical(),
            before,
            "the fold must not change the canonical image"
        );
    }
}
