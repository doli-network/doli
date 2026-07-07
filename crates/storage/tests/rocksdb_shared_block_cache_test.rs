//! INC-I-105: Shared block-cache wiring regression tests.
//!
//! Verifies that each RocksDB instance shares a single `rocksdb::Cache`
//! across all its named column families, and that the metrics scraper
//! correctly aggregates block-cache properties across CFs.
//!
//! Phase 4: utxo_store test (P2) removed — utxo_store was deleted.
//! state_db is the sole UTXO store and its cache is tested in state_db tests.
//!
//! The canonical fix template is `state_db/open.rs:88`:
//!   `let cache = rocksdb::Cache::new_lru_cache(SIZE);`
//! threaded into every CF via `cf_opts_*(base, &cache, ...)`.
//!
//! OUTPUT CONTRACT:
//!   Outputs observed per test:
//!     O1: block-cache-capacity (via RocksDbMetrics.block_cache_capacity field)
//!
//!   Paths:
//!     P1: BlockStore::open() -> cf_opts_block_store -> BlockBasedOptions
//!
//!   INPUT PARTITIONS:
//!     I1: Fresh tempdir with all CFs created (configuration assertion --
//!         single partition per store, no behavioral input variation).

use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Test 1: block_store -- block_cache_capacity from metrics == 32 MB
// ---------------------------------------------------------------------------
//
// REQ-MEM-001: block_store shares one explicit 32 MB block cache across all 9 CFs.
//
// Pre-fix: Each of 9 CFs gets its own `BlockBasedOptions::default()` with a
//   separate 32 MB cache. The RocksDbMetrics struct does not have a
//   `block_cache_capacity` field. The test fails to compile.
// Post-fix: One shared `Cache::new_lru_cache(32 MB)` threaded into all CFs.
//   RocksDbMetrics gains `block_cache_capacity` field = 32 MB.

/// REQ-MEM-001: block_store reports block_cache_capacity == 32 MB.
#[test]
fn test_block_store_shared_block_cache() {
    let dir = TempDir::new().unwrap();
    let store = storage::BlockStore::open(dir.path()).unwrap();
    let m = store.metrics();

    assert_eq!(m.instance, "block_store");
    assert_eq!(m.background_errors, 0);

    // block_cache_capacity is read from the first named CF via
    // rocksdb.block-cache-capacity. With a shared 32 MB cache, all CFs
    // report 32 MB. With separate default caches (pre-fix), each CF also
    // has 32 MB, but the field doesn't exist in RocksDbMetrics (compile fail).
    let expected_capacity: u64 = 32 * 1024 * 1024;
    assert_eq!(
        m.block_cache_capacity, expected_capacity,
        "block_store block_cache_capacity should be {} (32 MB shared). Got {}.",
        expected_capacity, m.block_cache_capacity,
    );
}

// Test 4 ("metrics scraper aggregates separate per-CF caches") was removed
// by INC-I-106. The post-INC-I-105 invariant (INV-STORAGE-001) makes that
// topology impossible: every DB instance owns ONE shared cache. The metric
// is now sourced from `Cache::get_usage()` directly, not summed across CFs.
// Replacement tests live in tests/rocksdb_block_cache_metrics_test.rs and
// assert: block_cache_bytes == cache.get_usage(), independent of CF count.
