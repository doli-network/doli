//! INC-I-105: Metrics scraper block-cache aggregation regression test.
//!
//! Verifies that `collect_db_metrics` sums `block-cache-usage` and
//! `block-cache-pinned-usage` across ALL named column families, not just
//! the first CF. This is the primary bug-reproduction test for INC-I-105.
//!
//! Root cause: `metrics.rs:129-134` uses `first_cf_prop` which reads from
//! the first named CF only. When each CF has its own separate cache
//! (the topology created by INC-I-104 M2/M4), the scraper under-reports
//! block-cache-usage by (N-1) CFs.
//!
//! OUTPUT CONTRACT:
//!   Function under test: collect_db_metrics(db, instance, cf_names, cap)
//!   Observable outputs:
//!     O1: RocksDbMetrics.block_cache_bytes — reported block cache usage
//!     O2: RocksDbMetrics.block_cache_pinned_bytes — reported pinned bytes
//!
//!   Code paths:
//!     P1: first_cf_prop (pre-fix) — reads from first CF only
//!     P2: sum_cf (post-fix) — sums across all named CFs
//!
//!   INPUT PARTITIONS:
//!     I1: DB with 2 CFs, each with separate 1 MB cache, both populated.
//!         Pre-fix: block_cache_bytes = cf_a usage only (under-reports).
//!         Post-fix: block_cache_bytes = cf_a + cf_b usage (correct).
//!     I2: DB with 2 CFs, shared cache, both populated.
//!         Pre-fix and post-fix: block_cache_bytes = shared usage (same).
//!         (This partition is not broken pre-fix; included for completeness.)
//!
//!   Matrix:
//!     test_metrics_aggregates_block_cache_across_cfs: O1 x P1/P2 x I1

use tempfile::TempDir;

/// REQ-MEM-004: metrics scraper sums block-cache-usage across all named CFs.
///
/// Opens a raw RocksDB with 2 CFs, each with its own separate 1 MB cache.
/// Writes, flushes, and reads data into both CFs to populate their caches.
/// Then verifies the scraper reports the SUM of per-CF usage.
///
/// Pre-fix: scraper reads first CF only. block_cache_bytes = cf_a usage.
///   Since cf_b also has > 0 usage, total != cf_a. Assertion FAILS.
/// Post-fix: scraper sums all CFs. block_cache_bytes = cf_a + cf_b. PASSES.
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

    // Per-CF block-cache-usage (ground truth from RocksDB properties).
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

    // The scraper should report the SUM of both CFs.
    let cf_names = vec!["cf_a", "cf_b"];
    let cap = 16 * 1024 * 1024;
    let m = storage::collect_db_metrics(&db, "test_separate_caches", &cf_names, cap);

    let total_usage = usage_a + usage_b;

    // Pre-fix: m.block_cache_bytes == usage_a (first CF only).
    //   Since usage_b > 0, total_usage > usage_a. Assertion FAILS.
    // Post-fix: m.block_cache_bytes == usage_a + usage_b. Assertion PASSES.
    assert_eq!(
        m.block_cache_bytes, total_usage,
        "Metrics block_cache_bytes ({}) should equal sum of per-CF usage ({} + {} = {}). \
         If it equals only cf_a ({}) the scraper reads first-CF-only (INC-I-105 bug).",
        m.block_cache_bytes, usage_a, usage_b, total_usage, usage_a,
    );
}
