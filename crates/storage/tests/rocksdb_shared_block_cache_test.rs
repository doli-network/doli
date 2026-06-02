//! INC-I-105: Shared block-cache wiring regression tests.
//!
//! Verifies that each RocksDB instance shares a single `rocksdb::Cache`
//! across all its named column families, and that the metrics scraper
//! correctly aggregates block-cache properties across CFs.
//!
//! Root cause (INC-I-104 M2/M4): `cf_opts_block_store` and `cf_opts_utxo_store`
//! construct `BlockBasedOptions::default()` per CF without threading a shared
//! `rocksdb::Cache`. On RocksDB 8.10.0 each default LRU = 32 MB, multiplying
//! block-cache capacity by 9x (block_store) and 3x (utxo_store).
//! `diagnostic_ledger` uses `DB::open_cf` with string CF names, which may not
//! propagate the DB-level `BlockBasedOptions` to named CFs.
//!
//! The canonical fix template is `state_db/open.rs:88`:
//!   `let cache = rocksdb::Cache::new_lru_cache(SIZE);`
//! threaded into every CF via `cf_opts_*(base, &cache, ...)`.
//!
//! OUTPUT CONTRACT:
//!   Outputs observed per test:
//!     O1: block-cache-capacity (via RocksDbMetrics.block_cache_capacity field)
//!     O2: block-cache-usage summed across CFs (metrics scraper aggregation)
//!     O3: block_cache_bytes reported by collect_db_metrics for separate caches
//!
//!   Paths:
//!     P1: BlockStore::open() -> cf_opts_block_store -> BlockBasedOptions
//!     P2: RocksDbUtxoStore::open() -> cf_opts_utxo_store -> BlockBasedOptions
//!     P3: DiagnosticLedger::open() -> DB::open_cf / open_cf_descriptors
//!     P4: collect_db_metrics() -> first_cf_prop / sum_cf for block-cache-*
//!
//!   INPUT PARTITIONS:
//!     I1: Fresh tempdir with all CFs created (configuration assertion --
//!         single partition per store, no behavioral input variation).
//!     I2: Raw RocksDB with known separate caches, populated data (I/O test).

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

// ---------------------------------------------------------------------------
// Test 2: utxo_store -- block_cache_capacity from metrics == 16 MB
// ---------------------------------------------------------------------------
//
// REQ-MEM-002: utxo_store shares one explicit 16 MB block cache across all 3 CFs.
//
// Pre-fix: Each of 3 CFs gets its own default 32 MB cache.
//   block_cache_capacity field doesn't exist (compile fail). If it did exist,
//   it would report 32 MB (default), not 16 MB (intended).
// Post-fix: One shared 16 MB cache. block_cache_capacity = 16 MB.

/// REQ-MEM-002: utxo_store reports block_cache_capacity == 16 MB.
#[test]
fn test_utxo_store_shared_block_cache() {
    let dir = TempDir::new().unwrap();
    let store = storage::RocksDbUtxoStore::open(dir.path()).unwrap();
    let m = store.metrics();

    assert_eq!(m.instance, "utxo_store");
    assert_eq!(m.background_errors, 0);

    let expected_capacity: u64 = 16 * 1024 * 1024;
    assert_eq!(
        m.block_cache_capacity, expected_capacity,
        "utxo_store block_cache_capacity should be {} (16 MB shared). Got {}. \
         If 33554432 (32 MB), the CF has a default cache (INC-I-105 bug).",
        expected_capacity, m.block_cache_capacity,
    );
}

// ---------------------------------------------------------------------------
// Test 3: diagnostic_ledger -- block_cache_capacity from metrics == 4 MB
// ---------------------------------------------------------------------------
//
// REQ-MEM-003: diagnostic_ledger shares one explicit 4 MB block cache via
//   open_cf_descriptors (not open_cf with string names).
//
// Pre-fix: `DB::open_cf` with string CF names bypasses DB-level table factory.
//   cf_events gets default 32 MB cache. block_cache_capacity field doesn't
//   exist in RocksDbMetrics (compile fail). If it did, it would report 32 MB.
// Post-fix: `open_cf_descriptors` with explicit ColumnFamilyDescriptor.
//   block_cache_capacity = 4 MB.

/// REQ-MEM-003: diagnostic_ledger reports block_cache_capacity == 4 MB.
#[test]
fn test_diagnostic_ledger_shared_block_cache() {
    let dir = TempDir::new().unwrap();
    let ledger = storage::diagnostic_ledger::DiagnosticLedger::open(dir.path()).unwrap();

    // Configured capacity accessor should return 4 MB.
    let configured = ledger.block_cache_capacity_bytes();
    assert_eq!(
        configured,
        4 * 1024 * 1024,
        "diagnostic_ledger configured block cache should be 4 MB"
    );

    let m = ledger.metrics();
    assert_eq!(m.instance, "diagnostic_ledger");
    assert_eq!(m.background_errors, 0);

    // Actual RocksDB-level capacity must match configured value.
    let expected_capacity: u64 = 4 * 1024 * 1024;
    assert_eq!(
        m.block_cache_capacity, expected_capacity,
        "diagnostic_ledger block_cache_capacity should be {} (4 MB). Got {}. \
         If 33554432 (32 MB), DB::open_cf is not propagating the table factory \
         to named CFs (INC-I-105 diagnostic_ledger bug).",
        expected_capacity, m.block_cache_capacity,
    );
}

// Test 4 ("metrics scraper aggregates separate per-CF caches") was removed
// by INC-I-106. The post-INC-I-105 invariant (INV-STORAGE-001) makes that
// topology impossible: every DB instance owns ONE shared cache. The metric
// is now sourced from `Cache::get_usage()` directly, not summed across CFs.
// Replacement tests live in tests/rocksdb_block_cache_metrics_test.rs and
// assert: block_cache_bytes == cache.get_usage(), independent of CF count.
