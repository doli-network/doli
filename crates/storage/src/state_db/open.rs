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
        // WAL for crash recovery
        opts.set_wal_recovery_mode(rocksdb::DBRecoveryMode::PointInTime);

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
}
