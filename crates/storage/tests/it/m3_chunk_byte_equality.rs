//! UTXO-scalability M3 [F3] — the chunk stream is the canonical image, byte for byte.
//!
//! REQ-SCALE-006 / Res-1 / Rad-1 / INV-SYNC-007 — Decision: a failure here would reveal
//! that a client reassembling chunks in cursor order does NOT reconstruct the consensus
//! `utxo_hash`, which means every chunked snap-sync installs a state whose root can never
//! match the quorum root. That is a transport change that silently became a consensus
//! change, on one backend or on both.
//! REQ-SCALE-002 — Decision: a divergence between the two backends' cursor sequences would
//! reveal that a snap-synced node and a genesis-synced node cannot agree on set contents
//! when they serve each other, because the split points depend on who is serving.
//!
//! OUTPUT CONTRACT: `UtxoSet::canonical_range(Option<&[u8]>, usize)
//!                     -> Result<(Vec<u8>, Option<Vec<u8>>), StorageError>`
//!   Outputs observable from the stable API:
//!     O1 the BODY bytes returned for one range
//!     O2 the `next_key` cursor (Some = more rows, None = exhausted)
//!     O3 the concatenation of every body across a full walk
//!     O4 BLAKE3 over `count.to_le_bytes() || O3`, i.e. the reconstructed digest
//!   Paths:
//!     P1 InMemory backend, full walk at the chunk budget    — B1 (O1..O4)
//!     P2 RocksDb backend, full walk at the chunk budget     — B2 (O1..O4)
//!     P3 both backends, cursor sequences compared           — B3 (O2)
//!     P4 a budget smaller than one entry                    — B4 (O1, O2)
//!   MATRIX: P1xO3 + P1xO4 -> B1 | P2xO3 + P2xO4 -> B2 | P3xO2 -> B3 | P4xO1 + P4xO2 -> B4
//! INPUT PARTITIONS: (a) >= 20_000 randomized entries with varying `extra_data` lengths, so
//!   the walk crosses many chunk boundaries and the boundaries are not entry-aligned by
//!   accident; (b) `max_bytes = 1`, the degenerate budget that could stall the cursor.
//!
//! `CHUNK_MAX_BYTES` below MIRRORS `network::protocols::sync::STATE_CHUNK_MAX_BYTES`.
//! `storage` does not depend on `network`, so the value cannot be imported; the network-side
//! constant is pinned by `m3_session_limits_are_named_constants` in
//! `crates/network/tests/it/m3_protocol_session.rs`, and these two must be read together.

use storage::utxo::UtxoSet;
use storage::{Outpoint, UtxoEntry};
use tempfile::TempDir;

use crate::common;

/// Mirror of `network::protocols::sync::STATE_CHUNK_MAX_BYTES` (1 MiB).
const CHUNK_MAX_BYTES: usize = 1024 * 1024;
const ENTRY_COUNT: usize = 20_000;
const FIXTURE_SEED: u64 = 0x00D0_1100_2026_0915;

struct Walk {
    body: Vec<u8>,
    cursors: Vec<Vec<u8>>,
    chunks: usize,
}

/// Walk `set` to exhaustion at `budget`, collecting the concatenated BODY bytes and the
/// full cursor sequence. Fails loudly rather than looping if the cursor ever stalls.
fn walk(set: &UtxoSet, budget: usize) -> Walk {
    let mut body = Vec::new();
    let mut cursors = Vec::new();
    let mut chunks = 0usize;
    let mut cursor: Option<Vec<u8>> = None;
    let mut previous: Option<Vec<u8>> = None;

    loop {
        let (bytes, next) = set
            .canonical_range(cursor.as_deref(), budget)
            .expect("canonical_range must succeed on a well-formed set");
        chunks += 1;
        assert!(
            chunks <= ENTRY_COUNT + 16,
            "the cursor walk did not terminate after {} ranges — a non-advancing cursor is an \
             infinite snap-sync, not a slow one",
            chunks
        );
        body.extend_from_slice(&bytes);

        match next {
            Some(key) => {
                if let Some(prev) = previous.as_ref() {
                    assert!(
                        key.as_slice() > prev.as_slice(),
                        "the cursor must advance strictly: {:?} did not exceed {:?}",
                        key,
                        prev
                    );
                }
                previous = Some(key.clone());
                cursors.push(key.clone());
                cursor = Some(key);
            }
            None => break,
        }
    }

    Walk {
        body,
        cursors,
        chunks,
    }
}

/// Reassemble the way a CLIENT must: the 8-byte LE count header first, then every chunk
/// body in cursor order.
fn reassemble(count: u64, body: &[u8]) -> Vec<u8> {
    let mut image = Vec::with_capacity(8 + body.len());
    image.extend_from_slice(&count.to_le_bytes());
    image.extend_from_slice(body);
    image
}

fn assert_stream_is_the_image(set: &UtxoSet, label: &str) {
    let count = set.utxo_count();
    let w = walk(set, CHUNK_MAX_BYTES);

    assert!(
        w.chunks > 1,
        "[{}] the fixture produced {} chunk(s) at a {}-byte budget — a single-chunk walk \
         never crosses a boundary and proves nothing about reassembly",
        label,
        w.chunks,
        CHUNK_MAX_BYTES
    );

    let rebuilt = reassemble(count, &w.body);
    let reference = set.serialize_canonical();

    assert_eq!(
        rebuilt.len(),
        reference.len(),
        "[{}] reassembled image is {} bytes but the canonical image is {} bytes",
        label,
        rebuilt.len(),
        reference.len()
    );
    assert_eq!(
        rebuilt, reference,
        "[{}] the reassembled chunk stream is NOT byte-identical to the canonical image",
        label
    );

    let rebuilt_digest = crypto::hash::hash(&rebuilt);
    let canonical_digest = set
        .canonical_digest()
        .expect("canonical_digest must succeed on a well-formed set");
    assert_eq!(
        rebuilt_digest, canonical_digest,
        "[{}] BLAKE3 over the reassembled stream must equal canonical_digest() — this is the \
         value the manifest's utxo_hash carries and the F4 root gate compares",
        label
    );
}

// ==================== B1 / B2 — reassembly on each backend ====================

/// REQ-SCALE-006 / INV-SYNC-007 — Decision: a failure here means chunked transfer changes
/// the consensus UTXO component on the InMemory backend, so a node that snap-syncs would
/// hold a different `utxo_hash` from every node that synced from genesis.
#[test]
fn m3_chunks_reassemble_byte_identical_to_canonical_image_inmemory() {
    let entries = common::randomized_entries(ENTRY_COUNT, FIXTURE_SEED);
    let set = common::build_in_memory(&entries);
    assert_stream_is_the_image(&set, "InMemory");
}

/// REQ-SCALE-006 / Rad-1 — Decision: the RocksDb backend is the PRODUCTION one; a failure
/// here that the InMemory test does not show means the pinned-iterator path emits different
/// bytes from the sorted-HashMap path, which is the two-write-paths divergence class.
#[test]
fn m3_chunks_reassemble_byte_identical_to_canonical_image_rocksdb() {
    let entries = common::randomized_entries(ENTRY_COUNT, FIXTURE_SEED);
    let dir = TempDir::new().expect("tempdir");
    let set = common::build_rocksdb(dir.path(), &entries);
    assert_stream_is_the_image(&set, "RocksDb");
}

// ==================== B3 — identical boundaries across backends ====================

/// REQ-SCALE-002 / Rad-1 — Decision: if the two backends split the same set at different
/// keys, a client that resumes a session against a DIFFERENT peer (the alternate-peer path
/// the attempt cap depends on) would request a cursor the new server cannot honour
/// identically, and the reassembled bytes would diverge without any error being raised.
#[test]
fn m3_inmemory_and_rocksdb_chunk_boundaries_are_identical() {
    let entries = common::randomized_entries(ENTRY_COUNT, FIXTURE_SEED);
    let mem = common::build_in_memory(&entries);
    let dir = TempDir::new().expect("tempdir");
    let rocks = common::build_rocksdb(dir.path(), &entries);

    let mem_walk = walk(&mem, CHUNK_MAX_BYTES);
    let rocks_walk = walk(&rocks, CHUNK_MAX_BYTES);

    assert_eq!(
        mem_walk.chunks, rocks_walk.chunks,
        "the two backends split the same set into {} and {} chunks",
        mem_walk.chunks, rocks_walk.chunks
    );
    assert_eq!(
        mem_walk.cursors, rocks_walk.cursors,
        "the cursor sequences must be identical across backends — a resumed session may be \
         answered by a peer running the other backend"
    );
    assert_eq!(
        mem_walk.body, rocks_walk.body,
        "the concatenated chunk bodies must be identical across backends"
    );
}

// ==================== B4 — the degenerate budget ====================

/// REQ-SCALE-013 — Decision: a failure here means a budget smaller than one entry returns
/// an empty body with a cursor that never moves, so a peer that advertises a tiny
/// `chunk_max_bytes` wedges the client in an infinite request loop instead of transferring
/// slowly. This is the stall mode, reduced to its smallest reproducible input.
#[test]
fn m3_undersized_budget_still_advances() {
    let entries = common::randomized_entries(64, FIXTURE_SEED ^ 0xB4B4);
    let set = common::build_in_memory(&entries);

    let (body, next) = set
        .canonical_range(None, 1)
        .expect("canonical_range with a 1-byte budget must succeed, not error");

    assert!(
        !body.is_empty(),
        "a 1-byte budget must still emit exactly one entry — an empty body cannot advance"
    );
    assert!(
        next.is_some(),
        "with 64 entries in the set, a one-entry range must report a next cursor"
    );

    // Exactly ONE entry: the second entry's key must be what the cursor points at.
    let first_key = &body[..36];
    let cursor = next.as_ref().expect("cursor");
    assert!(
        cursor.as_slice() > first_key,
        "the cursor {:?} must point past the single emitted entry {:?}",
        cursor,
        first_key
    );

    let reference = set.serialize_canonical();
    assert_eq!(
        &reference[8..8 + body.len()],
        body.as_slice(),
        "the single emitted entry must be the FIRST entry of the canonical image"
    );

    // And the whole walk at budget 1 still reassembles the image.
    let w = walk(&set, 1);
    assert_eq!(
        w.chunks, 64,
        "a 1-byte budget over 64 entries must take exactly 64 ranges, one entry each"
    );
    assert_eq!(
        reassemble(set.utxo_count(), &w.body),
        reference,
        "a one-entry-at-a-time walk must reassemble the same canonical image"
    );
}

/// REQ-SCALE-013 — Decision: an empty set that returns `Some(cursor)` would start a session
/// that can never complete; a fresh chain is exactly when snap-sync runs.
#[test]
fn m3_empty_set_yields_one_terminal_empty_range() {
    let set = UtxoSet::new();
    let (body, next) = set
        .canonical_range(None, CHUNK_MAX_BYTES)
        .expect("canonical_range on an empty set must succeed");

    assert!(body.is_empty(), "an empty set has no body bytes");
    assert!(
        next.is_none(),
        "an empty set must terminate the walk immediately"
    );
    assert_eq!(
        reassemble(set.utxo_count(), &body),
        set.serialize_canonical(),
        "the empty image is the 8-byte zero count header and nothing else"
    );
}

/// REQ-SCALE-013 — Decision: a cursor past every key that returns rows would let a resumed
/// session double-install entries it already wrote to staging.
#[test]
fn m3_cursor_past_the_last_key_returns_nothing() {
    let entries = common::randomized_entries(256, FIXTURE_SEED ^ 0xC0DE);
    let set = common::build_in_memory(&entries);

    let past_everything = vec![0xFFu8; 36];
    let (body, next) = set
        .canonical_range(Some(&past_everything), CHUNK_MAX_BYTES)
        .expect("canonical_range past the last key must succeed");

    assert!(
        body.is_empty(),
        "a cursor above every key must emit no entries, got {} bytes",
        body.len()
    );
    assert!(next.is_none(), "a cursor above every key must terminate");

    // Non-vacuity: the fixture really does hold keys below the sentinel.
    let max_key = entries
        .iter()
        .map(outpoint_key)
        .max()
        .expect("fixture must be non-empty");
    assert!(
        max_key.to_vec() < past_everything,
        "the fixture's largest key must sit below the sentinel cursor, else the assertion \
         above is vacuous"
    );
}

fn outpoint_key((outpoint, _): &(Outpoint, UtxoEntry)) -> [u8; 36] {
    let mut key = [0u8; 36];
    key[..32].copy_from_slice(outpoint.tx_hash.as_ref());
    key[32..].copy_from_slice(&outpoint.index.to_le_bytes());
    key
}
