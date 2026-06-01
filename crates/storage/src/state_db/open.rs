//! StateDb initialization

use std::path::Path;
use std::sync::atomic::AtomicU64;

use crate::StorageError;

use super::types::{
    StateDb, CF_EXIT_HISTORY, CF_META, CF_PRODUCERS, CF_UNDO, CF_UTXO, CF_UTXO_BY_PUBKEY,
};

impl StateDb {
    /// Open or create the unified state database at the given path.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
        opts.set_max_open_files(256);
        opts.enable_statistics();
        // WAL for crash recovery
        opts.set_wal_recovery_mode(rocksdb::DBRecoveryMode::PointInTime);
        // Cap total WAL size to prevent unbounded growth.
        // With 7 CFs, sparse ones (cf_producers, cf_exit_history) rarely flush,
        // pinning ALL WAL files. This forces RocksDB to flush the oldest CF
        // when total WAL exceeds the limit, allowing old WAL files to be deleted.
        opts.set_max_total_wal_size(64 * 1024 * 1024); // 64 MB

        // INC-I-104: cap total memtable budget across all 6 CFs.
        // Must be >= 32 MB to accommodate snap-sync atomic_replace WriteBatch
        // (~15-20 MB). See Failure Analyst C-002 in
        // docs/.workflow/architecture-reasoning.md and specs/rocksdb-configuration-architecture.md §state_db.
        opts.set_db_write_buffer_size(64 * 1024 * 1024); // 64 MB

        let cfs = vec![
            CF_UTXO,
            CF_UTXO_BY_PUBKEY,
            CF_PRODUCERS,
            CF_EXIT_HISTORY,
            CF_META,
            CF_UNDO,
        ];
        let db = rocksdb::DB::open_cf(&opts, path, cfs)?;

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
        })
    }

    /// RocksDB runtime metrics snapshot for Prometheus export.
    pub fn metrics(&self) -> crate::RocksDbMetrics {
        crate::collect_db_metrics(&self.db, "state_db")
    }
}
