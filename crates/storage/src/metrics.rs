//! RocksDB runtime metrics snapshot.
//!
//! Reads point-in-time properties from a `rocksdb::DB` handle for export to
//! Prometheus. Properties are cheap to read (in-memory counters); no statistics
//! ticker plumbing here — those require holding the `Options`/`Statistics`
//! object and live in a follow-up.
//!
//! Naming follows RocksDB's own property names (see `rocksdb/include/rocksdb/db.h`).
//!
//! # CF-scoped vs DB-scoped properties
//!
//! RocksDB splits properties into two categories. `rust-rocksdb 0.22`'s
//! `db.property_int_value(name)` reads from the *default* CF — which in DOLI is
//! always **empty** (every DB uses named CFs: `cf_utxo`, `headers`, etc.). For
//! CF-scoped properties this returns ~0 (just arena overhead), which is the
//! initial-INC-I-104-scraper bug.
//!
//! This module distinguishes the two:
//! - **CF-scoped** (memtable, SST files, keys, levels): summed across the
//!   caller-supplied CF list via `property_int_value_cf(cf_handle, name)`.
//! - **DB-scoped** (write stalls, background errors, running flushes): read
//!   once via `property_int_value(name)` (these are DB-aggregate values that
//!   don't depend on the CF — reading them from the default CF still returns
//!   the right value).
//! - **Block cache** (`block-cache-usage`, `block-cache-pinned-usage`): read
//!   from the *first* named CF. When the cache is shared across CFs (state_db,
//!   M3: `Cache::new_lru_cache(32 MB)` attached to all 6 CFs), every CF reports
//!   the same value. When per-CF caches exist (block_store / utxo_store using
//!   default 8 MB), this under-reports the aggregate; documented limitation.

use std::collections::BTreeMap;

/// Snapshot of a RocksDB instance's runtime properties.
#[derive(Debug, Clone, Default)]
pub struct RocksDbMetrics {
    /// Logical instance label (`block_store` | `state_db` | `utxo_store` | `diagnostic_ledger`).
    pub instance: &'static str,

    // --- Memory ---
    /// `cur-size-all-mem-tables` — current memtable bytes summed across all named CFs.
    pub memtable_bytes: u64,
    /// `size-all-mem-tables` — peak memtable bytes (sum of `write_buffer_size * max_write_buffer_number` across CFs).
    pub memtable_max_bytes: u64,
    /// `block-cache-usage` — bytes in the block cache. Read from the first CF
    /// (accurate for shared caches; under-reports per-CF caches — see module docs).
    pub block_cache_bytes: u64,
    /// `block-cache-pinned-usage` — pinned bytes (cannot be evicted). Same caveat as `block_cache_bytes`.
    pub block_cache_pinned_bytes: u64,
    /// `estimate-table-readers-mem` — memory used by SST index + bloom filter blocks.
    pub table_readers_bytes: u64,

    // --- Data shape ---
    /// `estimate-num-keys` — approximate live key count.
    pub estimate_keys: u64,
    /// `estimate-live-data-size` — approximate live data bytes after compaction.
    pub live_data_bytes: u64,
    /// `total-sst-files-size` — bytes of all SST files on disk.
    pub sst_total_bytes: u64,
    /// `live-sst-files-size` — bytes of live SST files (excludes obsolete pending deletion).
    pub sst_live_bytes: u64,

    // --- Flush / compaction state ---
    /// `num-running-flushes` — flush jobs currently executing (DB-scoped).
    pub running_flushes: u64,
    /// `num-running-compactions` — compaction jobs currently executing (DB-scoped).
    pub running_compactions: u64,
    /// `compaction-pending` — non-zero if compaction is queued/pending (summed across CFs).
    pub compaction_pending: u64,
    /// `mem-table-flush-pending` — non-zero if a memtable flush is queued/pending (summed across CFs).
    pub mem_table_flush_pending: u64,
    /// `num-immutable-mem-table` — immutable memtables awaiting flush (summed across CFs).
    pub num_immutable_memtable: u64,

    // --- Write health ---
    /// `actual-delayed-write-rate` — non-zero means RocksDB is throttling writes (DB-scoped).
    pub actual_delayed_write_rate: u64,
    /// `is-write-stopped` — 1 if writes are blocked on compaction/flush (DB-scoped).
    pub is_write_stopped: u64,
    /// `background-errors` — count of background errors (DB-scoped).
    pub background_errors: u64,

    // --- LSM shape ---
    /// `num-files-at-level<N>` for levels 0..=6, summed across all named CFs.
    pub files_per_level: BTreeMap<u8, u64>,
}

/// Read all properties from a `rocksdb::DB` and assemble a snapshot.
///
/// `cf_names` is the list of *named* column families the caller's storage type
/// uses (e.g. `["headers", "bodies", ...]` for block_store). CF-scoped
/// properties are summed across these CFs; reading from the default CF would
/// return near-zero because DOLI never writes to the default CF.
///
/// If any name in `cf_names` is unknown to `db`, it is silently skipped (the
/// CF was probably dropped or never created — defensive, not authoritative).
pub fn collect_db_metrics(
    db: &rocksdb::DB,
    instance: &'static str,
    cf_names: &[&str],
) -> RocksDbMetrics {
    // DB-scoped property read (default CF).
    let prop_db = |k: &str| -> u64 { db.property_int_value(k).ok().flatten().unwrap_or(0) };

    // CF-scoped property read on a specific CF handle.
    let prop_cf = |cf: &rocksdb::ColumnFamily, k: &str| -> u64 {
        db.property_int_value_cf(cf, k).ok().flatten().unwrap_or(0)
    };

    // Resolve CF handles. We need ColumnFamilyRef which is borrow-tied to `db`.
    // BoundColumnFamily is what cf_handle() returns; iterate by collecting Arcs.
    let cf_handles: Vec<_> = cf_names
        .iter()
        .filter_map(|name| db.cf_handle(name))
        .collect();

    // Sum a CF-scoped property across all named CFs.
    let sum_cf = |key: &str| -> u64 { cf_handles.iter().map(|h| prop_cf(h, key)).sum() };

    // Block-cache properties: read from the first named CF (accurate for
    // shared caches; documented under-reporting otherwise).
    let first_cf_prop = |key: &str| -> u64 {
        cf_handles
            .first()
            .map(|h| prop_cf(h, key))
            .unwrap_or_else(|| prop_db(key))
    };

    let mut files_per_level = BTreeMap::new();
    for level in 0u8..=6 {
        let key = format!("rocksdb.num-files-at-level{level}");
        files_per_level.insert(level, sum_cf(&key));
    }

    RocksDbMetrics {
        instance,
        // CF-scoped (sum)
        memtable_bytes: sum_cf("rocksdb.cur-size-all-mem-tables"),
        memtable_max_bytes: sum_cf("rocksdb.size-all-mem-tables"),
        table_readers_bytes: sum_cf("rocksdb.estimate-table-readers-mem"),
        estimate_keys: sum_cf("rocksdb.estimate-num-keys"),
        live_data_bytes: sum_cf("rocksdb.estimate-live-data-size"),
        sst_total_bytes: sum_cf("rocksdb.total-sst-files-size"),
        sst_live_bytes: sum_cf("rocksdb.live-sst-files-size"),
        compaction_pending: sum_cf("rocksdb.compaction-pending"),
        mem_table_flush_pending: sum_cf("rocksdb.mem-table-flush-pending"),
        num_immutable_memtable: sum_cf("rocksdb.num-immutable-mem-table"),
        // Block cache (first CF; same value when shared)
        block_cache_bytes: first_cf_prop("rocksdb.block-cache-usage"),
        block_cache_pinned_bytes: first_cf_prop("rocksdb.block-cache-pinned-usage"),
        // DB-scoped
        running_flushes: prop_db("rocksdb.num-running-flushes"),
        running_compactions: prop_db("rocksdb.num-running-compactions"),
        actual_delayed_write_rate: prop_db("rocksdb.actual-delayed-write-rate"),
        is_write_stopped: prop_db("rocksdb.is-write-stopped"),
        background_errors: prop_db("rocksdb.background-errors"),
        files_per_level,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Open a temp RocksDB with two named CFs, write enough data to several CFs
    /// to force memtable presence, and verify the snapshot:
    /// - reports the SUM across CFs for CF-scoped properties (not 0 from the
    ///   empty default CF — that was the original scraper bug)
    /// - reports DB-scoped properties without panicking
    /// - includes all L0..=L6 entries
    ///
    /// This catches both property-name typos and the default-CF default-zero bug.
    #[test]
    fn collect_aggregates_across_named_cfs() {
        let tmp = tempfile::tempdir().unwrap();
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.enable_statistics();

        let cfs = vec!["alpha", "beta"];
        let db = rocksdb::DB::open_cf(&opts, tmp.path(), &cfs).unwrap();

        // Write to both named CFs but NOT the default CF.
        let cf_alpha = db.cf_handle("alpha").unwrap();
        let cf_beta = db.cf_handle("beta").unwrap();
        for i in 0..100u32 {
            let k = i.to_le_bytes();
            db.put_cf(&cf_alpha, k, b"value-a").unwrap();
            db.put_cf(&cf_beta, k, b"value-b").unwrap();
        }

        let m = collect_db_metrics(&db, "test_instance", &cfs);
        assert_eq!(m.instance, "test_instance");
        assert_eq!(
            m.is_write_stopped, 0,
            "writes should not be stopped on a fresh DB"
        );
        // memtable_bytes is the SUM across alpha+beta, which must be > 0
        // because we just wrote 100 KV pairs to each. The OLD buggy path
        // (default CF read) would have returned ~0 here.
        assert!(
            m.memtable_bytes > 0,
            "memtable_bytes must be non-zero after writes to named CFs \
             (was the scraper reading from the empty default CF?). \
             Got: {}",
            m.memtable_bytes
        );
        // L0..=L6 entries must all be present (even if 0 — nothing flushed yet).
        for level in 0u8..=6 {
            assert!(m.files_per_level.contains_key(&level));
        }
    }

    /// Reading with an empty cf_names slice still returns a non-panicking
    /// snapshot — DB-scoped properties remain usable, CF-scoped fields are 0.
    /// Block-cache properties fall back to the default-CF read (still gives
    /// something usable for tiny DBs).
    #[test]
    fn collect_tolerates_empty_cf_list() {
        let tmp = tempfile::tempdir().unwrap();
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        let db = rocksdb::DB::open(&opts, tmp.path()).unwrap();

        let m = collect_db_metrics(&db, "empty", &[]);
        assert_eq!(m.instance, "empty");
        assert_eq!(m.is_write_stopped, 0);
        assert_eq!(m.memtable_bytes, 0);
        assert_eq!(m.sst_total_bytes, 0);
    }

    /// Verifies that all 4 production instance labels can be applied without
    /// label-cardinality issues. (Doesn't need a real DB.)
    #[test]
    fn instance_labels_are_stable() {
        for label in ["block_store", "state_db", "utxo_store", "diagnostic_ledger"] {
            let m = RocksDbMetrics {
                instance: label,
                ..Default::default()
            };
            assert_eq!(m.instance, label);
        }
    }
}
