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

// ---------------------------------------------------------------------------
// Test 4: metrics scraper aggregation
// ---------------------------------------------------------------------------
//
// REQ-MEM-004: collect_db_metrics reports block-cache-usage summed across
// all named CFs, not just the first CF.
//
// Pre-fix: `first_cf_prop` reads from the first CF only (metrics.rs:129-134).
//   With separate per-CF caches, total usage is under-reported by (N-1) CFs.
// Post-fix: scraper sums `block-cache-usage` across ALL named CFs.
//
// Signal: Open a raw RocksDB with 2 CFs, each with its own 1 MB cache.
// Write + flush + read data into both CFs. Then:
// - Pre-fix scraper: block_cache_bytes = cf_a usage only (misses cf_b).
// - Post-fix scraper: block_cache_bytes = cf_a + cf_b usage.
//
// This test uses ONLY the existing `collect_db_metrics` API and compiles
// on current main. It FAILS at runtime pre-fix.

/// REQ-MEM-004: metrics scraper sums block-cache-usage across all named CFs.
#[test]
fn test_metrics_aggregates_block_cache_across_cfs() {
    let tmp = TempDir::new().unwrap();
    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    opts.enable_statistics();

    // Create 2 CFs with SEPARATE caches (1 MB each). This mimics the pre-fix
    // block_store topology where each CF has its own default cache.
    let cache_a = rocksdb::Cache::new_lru_cache(1024 * 1024);
    let cache_b = rocksdb::Cache::new_lru_cache(1024 * 1024);

    let mut opts_a = rocksdb::Options::default();
    {
        let mut bbo = rocksdb::BlockBasedOptions::default();
        bbo.set_block_cache(&cache_a);
        opts_a.set_block_based_table_factory(&bbo);
    }
    let mut opts_b = rocksdb::Options::default();
    {
        let mut bbo = rocksdb::BlockBasedOptions::default();
        bbo.set_block_cache(&cache_b);
        opts_b.set_block_based_table_factory(&bbo);
    }

    let descriptors = vec![
        rocksdb::ColumnFamilyDescriptor::new("cf_a", opts_a),
        rocksdb::ColumnFamilyDescriptor::new("cf_b", opts_b),
    ];
    let db = rocksdb::DB::open_cf_descriptors(&opts, tmp.path(), descriptors).unwrap();

    // Write 4 KB values x 500 entries to each CF to generate SST data.
    let cf_a = db.cf_handle("cf_a").unwrap();
    let cf_b = db.cf_handle("cf_b").unwrap();
    let big_value = vec![0xABu8; 4096];
    for i in 0..500u32 {
        let key = i.to_be_bytes();
        db.put_cf(&cf_a, key, &big_value).unwrap();
        db.put_cf(&cf_b, key, &big_value).unwrap();
    }

    // Flush memtables to SST files. Block cache is only populated from SST reads.
    db.flush_cf(&cf_a).unwrap();
    db.flush_cf(&cf_b).unwrap();

    // Read back all data from both CFs to populate their block caches.
    for i in 0..500u32 {
        let key = i.to_be_bytes();
        let _ = db.get_cf(&cf_a, key).unwrap();
        let _ = db.get_cf(&cf_b, key).unwrap();
    }

    // Per-CF block-cache-usage (ground truth).
    let usage_a: u64 = db
        .property_int_value_cf(&cf_a, "rocksdb.block-cache-usage")
        .ok()
        .flatten()
        .unwrap_or(0);
    let usage_b: u64 = db
        .property_int_value_cf(&cf_b, "rocksdb.block-cache-usage")
        .ok()
        .flatten()
        .unwrap_or(0);

    assert!(
        usage_a > 0,
        "cf_a block-cache-usage should be > 0 after reads, got 0"
    );
    assert!(
        usage_b > 0,
        "cf_b block-cache-usage should be > 0 after reads, got 0"
    );

    // The scraper should report the SUM.
    let cf_names = vec!["cf_a", "cf_b"];
    let cap = 16 * 1024 * 1024;
    let m = storage::collect_db_metrics(&db, "test_separate_caches", &cf_names, cap);

    let total_usage = usage_a + usage_b;

    // Pre-fix: m.block_cache_bytes == usage_a (first CF only).
    //   Since usage_a != usage_a + usage_b (usage_b > 0), this assertion FAILS.
    // Post-fix: m.block_cache_bytes == usage_a + usage_b. Assertion PASSES.
    assert_eq!(
        m.block_cache_bytes, total_usage,
        "Metrics block_cache_bytes ({}) should equal sum of per-CF usage ({} + {} = {}). \
         If it equals only cf_a ({}) the scraper reads first-CF-only (INC-I-105 bug).",
        m.block_cache_bytes, usage_a, usage_b, total_usage, usage_a,
    );
}
