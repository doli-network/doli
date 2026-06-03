//! INC-I-106: Block-cache metric must not over-count shared caches.
//!
//! INC-I-105 introduced a per-CF `sum_cf("rocksdb.block-cache-usage")` aggregation
//! in `collect_db_metrics`. With INV-STORAGE-001 (one shared `rocksdb::Cache` per
//! DB instance, referenced by every CF), each CF reports the same shared usage —
//! so summing across N CFs inflates the reading by N. On ai5 mainnet that
//! reported `block_cache_bytes{instance="block_store"}=296 MB` against a 32 MB
//! configured cache (9 CFs × ~32 MB each), making the gauge useless for
//! operators and causing the HELP text to contradict the value.
//!
//! INC-I-106 root-cause fix: `collect_db_metrics` now takes the `Cache` handle
//! and queries it directly via `Cache::get_usage()` / `get_pinned_usage()`. The
//! per-CF property path is eliminated for cache fields — it cannot return
//! N × actual_usage no matter how many CFs share the cache.
//!
//! OUTPUT CONTRACT:
//!   Function under test: collect_db_metrics(db, instance, cf_names, cap, &cache, capacity)
//!   Observable outputs:
//!     O1: RocksDbMetrics.block_cache_bytes        — `cache.get_usage()` (NOT a sum)
//!     O2: RocksDbMetrics.block_cache_pinned_bytes — `cache.get_pinned_usage()` (NOT a sum)
//!     O3: RocksDbMetrics.block_cache_capacity     — caller-supplied capacity (NOT a property read)
//!
//!   Code paths:
//!     P1: cache reference passed → direct Cache method calls.
//!
//!   INPUT PARTITIONS:
//!     I1: DB with 2 CFs sharing one cache, both populated.
//!         Direct cache.get_usage() == reported block_cache_bytes (NOT 2× usage).
//!     I2: DB with 9 CFs sharing one cache (extreme N to amplify the regression
//!         if it ever re-emerges). Reported block_cache_bytes must remain
//!         ≤ capacity regardless of CF count.
//!
//!   Matrix:
//!     test_metrics_block_cache_matches_cache_get_usage_2cf: O1, O2, O3 × P1 × I1
//!     test_metrics_block_cache_below_capacity_with_many_cfs: O1 × P1 × I2

use tempfile::TempDir;

/// I1: 2 CFs sharing one cache. The metric must equal the cache's own
/// `get_usage()` — NOT 2 × that value.
#[test]
fn test_metrics_block_cache_matches_cache_get_usage_2cf() {
    let tmp = TempDir::new().unwrap();
    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    opts.enable_statistics();

    let capacity = 16 * 1024 * 1024;
    let cache = rocksdb::Cache::new_lru_cache(capacity);

    let make_cf_opts = |cache: &rocksdb::Cache| {
        let mut cf_opts = rocksdb::Options::default();
        let mut bbo = rocksdb::BlockBasedOptions::default();
        bbo.set_block_cache(cache);
        cf_opts.set_block_based_table_factory(&bbo);
        cf_opts
    };

    let descriptors = vec![
        rocksdb::ColumnFamilyDescriptor::new("cf_a", make_cf_opts(&cache)),
        rocksdb::ColumnFamilyDescriptor::new("cf_b", make_cf_opts(&cache)),
    ];
    let db = rocksdb::DB::open_cf_descriptors(&opts, tmp.path(), descriptors).unwrap();

    let cf_a = db.cf_handle("cf_a").unwrap();
    let cf_b = db.cf_handle("cf_b").unwrap();
    let big_value = vec![0xABu8; 4096];
    for i in 0..500u32 {
        let key = i.to_be_bytes();
        db.put_cf(&cf_a, key, &big_value).unwrap();
        db.put_cf(&cf_b, key, &big_value).unwrap();
    }
    db.flush_cf(&cf_a).unwrap();
    db.flush_cf(&cf_b).unwrap();
    for i in 0..500u32 {
        let key = i.to_be_bytes();
        let _ = db.get_cf(&cf_a, key).unwrap();
        let _ = db.get_cf(&cf_b, key).unwrap();
    }

    let direct_usage = cache.get_usage() as u64;
    let direct_pinned = cache.get_pinned_usage() as u64;
    assert!(
        direct_usage > 0,
        "cache.get_usage() should be > 0 after reads, got 0",
    );

    let cf_names = vec!["cf_a", "cf_b"];
    let m = storage::collect_db_metrics(
        &db,
        "test_shared_cache_2cf",
        &cf_names,
        16 * 1024 * 1024,
        &cache,
        capacity as u64,
    );

    // INC-I-106: metric must equal the cache's actual usage, NOT 2x that value.
    assert_eq!(
        m.block_cache_bytes,
        direct_usage,
        "block_cache_bytes ({}) must equal cache.get_usage() ({}). \
         If it equals 2 × that (={}), the per-CF sum_cf path was re-introduced.",
        m.block_cache_bytes,
        direct_usage,
        2 * direct_usage,
    );
    assert_eq!(
        m.block_cache_pinned_bytes, direct_pinned,
        "block_cache_pinned_bytes must equal cache.get_pinned_usage() — same root cause",
    );
    assert_eq!(
        m.block_cache_capacity, capacity as u64,
        "block_cache_capacity must equal the caller-supplied capacity",
    );
    assert!(
        m.block_cache_bytes <= m.block_cache_capacity,
        "block_cache_bytes ({}) must not exceed capacity ({}) under any circumstance",
        m.block_cache_bytes,
        m.block_cache_capacity,
    );
}

/// I2: 9 CFs sharing one cache (mirrors block_store on mainnet — the largest
/// inflation factor the pre-INC-I-106 regression could produce). Reported
/// `block_cache_bytes` must stay ≤ capacity regardless of the CF count.
#[test]
fn test_metrics_block_cache_below_capacity_with_many_cfs() {
    let tmp = TempDir::new().unwrap();
    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);

    let capacity = 8 * 1024 * 1024;
    let cache = rocksdb::Cache::new_lru_cache(capacity);

    let cf_labels: Vec<String> = (0..9).map(|i| format!("cf_{i}")).collect();
    let descriptors: Vec<_> = cf_labels
        .iter()
        .map(|name| {
            let mut cf_opts = rocksdb::Options::default();
            let mut bbo = rocksdb::BlockBasedOptions::default();
            bbo.set_block_cache(&cache);
            cf_opts.set_block_based_table_factory(&bbo);
            rocksdb::ColumnFamilyDescriptor::new(name, cf_opts)
        })
        .collect();
    let db = rocksdb::DB::open_cf_descriptors(&opts, tmp.path(), descriptors).unwrap();

    // Populate every CF — generates cache pressure on the SHARED cache.
    let value = vec![0xCDu8; 4096];
    for name in &cf_labels {
        let cf = db.cf_handle(name).unwrap();
        for i in 0..200u32 {
            db.put_cf(&cf, i.to_be_bytes(), &value).unwrap();
        }
        db.flush_cf(&cf).unwrap();
        for i in 0..200u32 {
            let _ = db.get_cf(&cf, i.to_be_bytes()).unwrap();
        }
    }

    let cf_name_refs: Vec<&str> = cf_labels.iter().map(String::as_str).collect();
    let m = storage::collect_db_metrics(
        &db,
        "test_shared_cache_9cf",
        &cf_name_refs,
        8 * 1024 * 1024,
        &cache,
        capacity as u64,
    );

    // The regression's signature: sum_cf would report ~9 × cache.get_usage(),
    // exceeding capacity by up to 9x. The root-cause fix makes this
    // unreachable.
    assert!(
        m.block_cache_bytes <= m.block_cache_capacity,
        "block_cache_bytes ({}) must not exceed capacity ({}) — 9 CFs would have \
         inflated to ~9× capacity under the pre-INC-I-106 sum_cf path",
        m.block_cache_bytes,
        m.block_cache_capacity,
    );
    assert_eq!(
        m.block_cache_bytes,
        cache.get_usage() as u64,
        "block_cache_bytes must equal cache.get_usage(), independent of CF count",
    );
}
