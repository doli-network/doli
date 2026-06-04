//! Phase 5 tests: BlobDB on cf_utxo + F1 snap-sync size monitor.
//!
//! TDD tests written before implementation per workflow discipline.
//!
//! OUTPUT CONTRACT:
//!   Functions under test:
//!     1. StateDb::open() — BlobDB config applied to cf_utxo
//!        Outputs: On-disk layout (.blob files for values >= 4KB), identical get/iter bytes
//!     2. StateDb::serialize_canonical_utxo() — canonical byte stream
//!        Outputs: Vec<u8> (deterministic, invariant under BlobDB)
//!     3. StateDb::flush_cf_utxo() — forces memtable to SST/blob
//!        Outputs: On-disk files created
//!     4. UtxoSizeMonitor::compute_size() — raw canonical size
//!        Outputs: u64 byte count matching serialize_canonical_utxo().len()
//!     5. UtxoSizeMonitor::get_cached_size() — cached canonical size
//!        Outputs: u64 (cached), computation_count unchanged on cache hit
//!
//!   Code paths:
//!     P1: Large UTXO (>= 4KB extra_data) → BlobDB separates value to .blob file
//!     P2: Small UTXO (< 4KB extra_data) → value stays inline in SST
//!     P3: Mixed large/small → both round-trip correctly via get_utxo
//!     P4: serialize_canonical_utxo → deterministic across DB instances
//!     P5: UtxoSizeMonitor first call → computes and caches
//!     P6: UtxoSizeMonitor second call within window → returns cache, no recompute
//!
//!   INPUT PARTITIONS:
//!     T1 (P1): single 100KB UTXO → .blob files appear after flush
//!     T2 (P1+P3): 50 UTXOs 5-50KB → all round-trip via get + canonical iter
//!     T3 (P2+P3+P4): 20 mixed UTXOs in 2 DBs → bit-identical canonical output
//!     T4 (P5): 10 small UTXOs → monitor reports exact len()
//!     T5 (P5+P6): 5 UTXOs → cache hit on second call, computation_count stable

use std::sync::Arc;
use tempfile::TempDir;

/// Helper: open state_db in a tempdir (uses the real `StateDb::open` which
/// now includes BlobDB config on cf_utxo).
fn open_test_db() -> (storage::StateDb, TempDir) {
    let dir = TempDir::new().unwrap();
    let db = storage::StateDb::open(dir.path()).unwrap();
    (db, dir)
}

/// Build a synthetic UTXO entry with `extra_data_size` bytes of payload.
fn large_utxo_entry(extra_data_size: usize, height: u64) -> storage::UtxoEntry {
    let output = doli_core::transaction::Output {
        output_type: doli_core::OutputType::NFT,
        amount: 1_000_000,
        pubkey_hash: crypto::hash::hash(b"test-nft-owner"),
        lock_until: 0,
        extra_data: vec![0xAB; extra_data_size],
    };
    storage::UtxoEntry {
        output,
        height,
        is_coinbase: false,
        is_epoch_reward: false,
    }
}

/// Build a small normal UTXO entry.
fn small_utxo_entry(amount: u64, seed: &[u8], height: u64) -> storage::UtxoEntry {
    let output = doli_core::transaction::Output::normal(amount, crypto::hash::hash(seed));
    storage::UtxoEntry {
        output,
        height,
        is_coinbase: false,
        is_epoch_reward: false,
    }
}

// =========================================================================
// T1: BlobDB takes effect — .blob files appear after flush of large payload
// =========================================================================

#[test]
fn blobdb_takes_effect_blob_files_appear() {
    let (db, dir) = open_test_db();

    // Write a 100 KB UTXO (well above min_blob_size=4096).
    let outpoint = storage::Outpoint::new(crypto::hash::hash(b"blob-test-tx"), 0);
    let entry = large_utxo_entry(100_000, 1);
    db.insert_utxo(&outpoint, &entry);

    // Force a flush so the memtable is written to disk.
    db.flush_cf_utxo();

    // Check for .blob files in the data directory.
    let blob_count = count_blob_files(dir.path());
    assert!(
        blob_count > 0,
        "Expected .blob files after flushing a 100 KB UTXO, found {blob_count}"
    );
}

/// Count `.blob` files recursively under a directory.
fn count_blob_files(path: &std::path::Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                count += count_blob_files(&p);
            } else if let Some(ext) = p.extension() {
                if ext == "blob" {
                    count += 1;
                }
            }
        }
    }
    count
}

// =========================================================================
// T2: BlobDB roundtrip — 50 large-payload UTXOs survive get + iter
// =========================================================================

#[test]
fn blobdb_roundtrip_large_utxos() {
    let (db, _dir) = open_test_db();

    let mut written: Vec<(storage::Outpoint, storage::UtxoEntry)> = Vec::new();
    for i in 0u32..50 {
        let tx_hash = crypto::hash::hash(&i.to_le_bytes());
        let outpoint = storage::Outpoint::new(tx_hash, 0);
        // Vary sizes: 5 KB to 50 KB
        let size = 5_000 + (i as usize) * 1_000;
        let entry = large_utxo_entry(size, i as u64 + 1);
        db.insert_utxo(&outpoint, &entry);
        written.push((outpoint, entry));
    }

    // Flush to force BlobDB to separate large values.
    db.flush_cf_utxo();

    // Point lookups: every UTXO must round-trip exactly.
    for (outpoint, expected) in &written {
        let got = db.get_utxo(outpoint).expect("UTXO must exist after insert");
        assert_eq!(got.output.amount, expected.output.amount);
        assert_eq!(got.output.extra_data, expected.output.extra_data);
        assert_eq!(got.height, expected.height);
        assert_eq!(got.output.output_type, expected.output.output_type);
        assert_eq!(got.output.pubkey_hash, expected.output.pubkey_hash);
    }

    // Full iteration: count must match.
    let canonical = db.serialize_canonical_utxo();
    // First 8 bytes = count (u64 LE).
    let count = u64::from_le_bytes(canonical[..8].try_into().unwrap());
    assert_eq!(
        count, 50,
        "canonical serialization must contain all 50 UTXOs"
    );
}

// =========================================================================
// T3: State root invariance — serialize_canonical_utxo is bit-identical
//     regardless of BlobDB (since BlobDB is on-disk only, the bytes
//     returned by get/iter are the same as without BlobDB).
// =========================================================================

#[test]
fn state_root_invariance_with_blobdb() {
    // Verify the canonical serialization is deterministic:
    // open two independent DBs, write the same data, verify identical output.
    let (db_a, _dir_a) = open_test_db();
    let (db_b, _dir_b) = open_test_db();

    // Write identical data to both.
    for i in 0u32..20 {
        let tx_hash = crypto::hash::hash(&i.to_le_bytes());
        let outpoint = storage::Outpoint::new(tx_hash, 0);
        let entry = if i % 3 == 0 {
            // Every 3rd UTXO is large (triggers BlobDB)
            large_utxo_entry(10_000, i as u64 + 1)
        } else {
            small_utxo_entry(1_000_000 + i as u64, &i.to_le_bytes(), i as u64 + 1)
        };
        db_a.insert_utxo(&outpoint, &entry);
        db_b.insert_utxo(&outpoint, &entry);
    }

    db_a.flush_cf_utxo();
    db_b.flush_cf_utxo();

    let canonical_a = db_a.serialize_canonical_utxo();
    let canonical_b = db_b.serialize_canonical_utxo();

    assert_eq!(
        canonical_a, canonical_b,
        "Canonical UTXO serialization must be bit-identical across DB instances"
    );

    // Verify state root hash is identical.
    let root_a = crypto::hash::hash(&canonical_a);
    let root_b = crypto::hash::hash(&canonical_b);
    assert_eq!(root_a, root_b, "State roots must be bit-identical");
}

// =========================================================================
// T4: F1 monitor — produces sensible value for known UTXO set
// =========================================================================

#[test]
fn f1_monitor_produces_sensible_value() {
    let (db, _dir) = open_test_db();

    // Insert 10 small UTXOs with known canonical size.
    for i in 0u32..10 {
        let tx_hash = crypto::hash::hash(&i.to_le_bytes());
        let outpoint = storage::Outpoint::new(tx_hash, 0);
        let entry = small_utxo_entry(100_000, &i.to_le_bytes(), i as u64 + 1);
        db.insert_utxo(&outpoint, &entry);
    }

    // Compute the expected canonical size.
    let canonical = db.serialize_canonical_utxo();
    let expected_size = canonical.len();

    // The monitor should return the same value.
    let monitor = storage::UtxoSizeMonitor::new(Arc::new(db));
    let reported = monitor.compute_size();

    assert_eq!(
        reported, expected_size as u64,
        "F1 monitor must report exact canonical serialization size"
    );
}

// =========================================================================
// T5: F1 monitor caches — rapid calls don't recompute
// =========================================================================

#[test]
fn f1_monitor_caches_value() {
    let (db, _dir) = open_test_db();

    // Insert a few UTXOs so the size is non-zero.
    for i in 0u32..5 {
        let tx_hash = crypto::hash::hash(&i.to_le_bytes());
        let outpoint = storage::Outpoint::new(tx_hash, 0);
        let entry = small_utxo_entry(50_000, &i.to_le_bytes(), i as u64 + 1);
        db.insert_utxo(&outpoint, &entry);
    }

    let monitor = storage::UtxoSizeMonitor::new(Arc::new(db));

    // First call computes.
    let size_1 = monitor.get_cached_size();
    assert!(size_1 > 0, "First call should return non-zero size");

    // Second call within 60s should return the cached value (same value,
    // and the computation_count should not increment).
    let computations_before = monitor.computation_count();
    let size_2 = monitor.get_cached_size();
    let computations_after = monitor.computation_count();

    assert_eq!(size_1, size_2, "Cached size should be identical");
    assert_eq!(
        computations_before, computations_after,
        "Second call within cache window must NOT recompute"
    );
}
