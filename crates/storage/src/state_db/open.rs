//! StateDb initialization

use std::path::Path;
use std::sync::atomic::AtomicU64;

use crate::StorageError;

use super::types::{
    StateDb, CF_EXIT_HISTORY, CF_META, CF_PRODUCERS, CF_UNDO, CF_UTXO, CF_UTXO_BY_PUBKEY,
    DB_WRITE_BUFFER_SIZE_BYTES,
};

/// Build per-CF Options for state_db column families.
///
/// Each CF gets workload-appropriate tuning derived from the DB-level base
/// options. The `cache` reference is shared (Arc-internal) across all CFs
/// within this state_db instance — NOT shared with other DB instances (C-012).
///
/// See `specs/rocksdb-configuration-architecture.md` section state_db.
#[allow(clippy::too_many_arguments)]
fn cf_opts_state_db(
    base: &rocksdb::Options,
    cache: &rocksdb::Cache,
    write_buffer_mb: usize,
    max_write_buffer_num: i32,
    bloom: bool,
    block_size_kb: usize,
    compression: rocksdb::DBCompressionType,
    target_file_size_mb: u64,
    l0_slowdown: Option<i32>,
    l0_stop: Option<i32>,
) -> rocksdb::Options {
    let mut opts = base.clone();
    opts.set_write_buffer_size(write_buffer_mb * 1024 * 1024);
    opts.set_max_write_buffer_number(max_write_buffer_num);
    opts.set_compression_type(compression);
    opts.set_target_file_size_base(target_file_size_mb * 1024 * 1024);
    if let Some(t) = l0_slowdown {
        opts.set_level_zero_slowdown_writes_trigger(t);
    }
    if let Some(t) = l0_stop {
        opts.set_level_zero_stop_writes_trigger(t);
    }

    let mut bbo = rocksdb::BlockBasedOptions::default();
    bbo.set_block_cache(cache);
    bbo.set_block_size(block_size_kb * 1024);
    if bloom {
        bbo.set_bloom_filter(10.0, false);
    }
    opts.set_block_based_table_factory(&bbo);

    opts
}

impl StateDb {
    /// Open or create the unified state database at the given path.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
        opts.set_max_open_files(256);
        // INC-I-109 experiment: disable RocksDB statistics. perf showed
        // StatisticsImpl::recordTick at 51.87% of compaction CPU; never consumed
        // by application (no get_ticker/get_histogram calls). Only fed periodic
        // LOG dumps. Folsi-only canary to test as alternative spike cause.
        // opts.enable_statistics();
        // WAL for crash recovery
        opts.set_wal_recovery_mode(rocksdb::DBRecoveryMode::PointInTime);
        // Cap total WAL size to prevent unbounded growth.
        // With 7 CFs, sparse ones (cf_producers, cf_exit_history) rarely flush,
        // pinning ALL WAL files. This forces RocksDB to flush the oldest CF
        // when total WAL exceeds the limit, allowing old WAL files to be deleted.
        opts.set_max_total_wal_size(DB_WRITE_BUFFER_SIZE_BYTES);

        // INC-I-104: cap total memtable budget across all 6 CFs.
        // Must be >= 32 MB to accommodate snap-sync atomic_replace WriteBatch
        // (~15-20 MB). See Failure Analyst C-002 in
        // docs/.workflow/architecture-reasoning.md and specs/rocksdb-configuration-architecture.md §state_db.
        // DB_WRITE_BUFFER_SIZE_BYTES is shared with `metrics()` so the cap and
        // the reported gauge can never drift.
        opts.set_db_write_buffer_size(DB_WRITE_BUFFER_SIZE_BYTES as usize);

        // INC-I-104 M3: explicit background job limits.
        opts.set_max_background_jobs(2);
        opts.set_max_subcompactions(1);

        // INC-I-104 M3: explicit 32 MB LRU block cache shared across all 6 CFs.
        // Arc-internal in rust-rocksdb — multiple BlockBasedOptions builders
        // reference the same underlying cache. Per-instance only (C-012).
        let cache = rocksdb::Cache::new_lru_cache(32 * 1024 * 1024);

        // Shorthand aliases for compression types used in the spec table.
        use rocksdb::DBCompressionType::{Lz4, Zstd};

        // INC-I-104 M3: per-CF descriptors with workload-derived tuning.
        // Spec: specs/rocksdb-configuration-architecture.md section state_db.
        //
        // | CF              | wbuf MB | #buf | bloom | blk KB | compr | tgt MB | L0 slow | L0 stop |
        // |-----------------|---------|------|-------|--------|-------|--------|---------|---------|
        // | cf_utxo         |   16    |  2   | YES   |   4    | Lz4   |   32   |   40    |   60    |
        // | cf_utxo_by_pk   |    8    |  2   | NO    |   4    | Lz4   |   16   |   40    |   60    |
        // | cf_meta         |    4    |  2   | no    |   4    | Lz4   |    8   |  def    |  def    |
        // | cf_undo         |    4    |  2   | no    |  16    | Zstd  |   16   |  def    |  def    |
        // | cf_producers    |    2    |  2   | YES   |   4    | Lz4   |    8   |  def    |  def    |
        // | cf_exit_history |    1    |  1   | YES   |   4    | Lz4   |    2   |  def    |  def    |
        let cf_descriptors = vec![
            // Hottest CF — point lookups on every tx validation.
            // BLOOM (10 bits/key) + L0 slowdown=40/stop=60 (C-003 MANDATORY).
            rocksdb::ColumnFamilyDescriptor::new(
                CF_UTXO,
                cf_opts_state_db(&opts, &cache, 16, 2, true, 4, Lz4, 32, Some(40), Some(60)),
            ),
            // Hot write, prefix scan reads — NO bloom (C-010).
            // L0 slowdown=40/stop=60 (C-003 MANDATORY — shrunk from 64 MB to 8 MB).
            rocksdb::ColumnFamilyDescriptor::new(
                CF_UTXO_BY_PUBKEY,
                cf_opts_state_db(&opts, &cache, 8, 2, false, 4, Lz4, 16, Some(40), Some(60)),
            ),
            // Hot write, known keys — no bloom needed.
            rocksdb::ColumnFamilyDescriptor::new(
                CF_META,
                cf_opts_state_db(&opts, &cache, 4, 2, false, 4, Lz4, 8, None, None),
            ),
            // One entry per block, 1-100+ KB. Cold read, highly compressible.
            // Zstd compression, 16 KB block size for large values.
            // INC-I-108: periodic_compaction_seconds replaces the synchronous
            // compact_range_cf that prune_undo_before used to fire every 100
            // blocks (~17 min), blocking the apply event loop and triggering
            // fleet-wide CPU spikes via 4-process simultaneity on multi-producer
            // hosts. RocksDB recompacts SSTs older than 1 hour in background
            // threads, naturally decorrelated across processes by independent
            // SST creation times.
            {
                let mut cf_undo_opts =
                    cf_opts_state_db(&opts, &cache, 4, 2, false, 16, Zstd, 16, None, None);
                cf_undo_opts.set_periodic_compaction_seconds(3600);
                rocksdb::ColumnFamilyDescriptor::new(CF_UNDO, cf_undo_opts)
            },
            // Epoch-boundary writes. Point lookup by pubkey — bloom helps.
            rocksdb::ColumnFamilyDescriptor::new(
                CF_PRODUCERS,
                cf_opts_state_db(&opts, &cache, 2, 2, true, 4, Lz4, 8, None, None),
            ),
            // Near-zero writes. Anti-Sybil point lookup — bloom helps.
            rocksdb::ColumnFamilyDescriptor::new(
                CF_EXIT_HISTORY,
                cf_opts_state_db(&opts, &cache, 1, 1, true, 4, Lz4, 2, None, None),
            ),
        ];

        let db = rocksdb::DB::open_cf_descriptors(&opts, path, cf_descriptors)?;

        // Count existing UTXO entries
        let cf_utxo = db.cf_handle(CF_UTXO).unwrap();
        let mut count = 0u64;
        for _ in db
            .iterator_cf(cf_utxo, rocksdb::IteratorMode::Start)
            .flatten()
        {
            count += 1;
        }

        Ok(Self {
            db,
            utxo_count: AtomicU64::new(count),
            block_cache: cache,
            block_cache_capacity_bytes: 32 * 1024 * 1024,
        })
    }

    /// RocksDB runtime metrics snapshot for Prometheus export.
    ///
    /// Passes the 6 named CFs so the collector aggregates across them
    /// (the default CF is unused and would return ~0 for CF-scoped properties).
    pub fn metrics(&self) -> crate::RocksDbMetrics {
        crate::collect_db_metrics(
            &self.db,
            "state_db",
            &[
                CF_UTXO,
                CF_UTXO_BY_PUBKEY,
                CF_PRODUCERS,
                CF_EXIT_HISTORY,
                CF_META,
                CF_UNDO,
            ],
            DB_WRITE_BUFFER_SIZE_BYTES,
            &self.block_cache,
            self.block_cache_capacity_bytes,
        )
    }
}
