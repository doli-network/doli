//! Undo log operations for block rollback

use doli_core::types::BlockHeight;

use super::types::{StateDb, UndoData, CF_UNDO};

impl StateDb {
    // ==================== Undo Log ====================

    /// Store undo data for a block height.
    pub fn put_undo(&self, height: BlockHeight, undo: &UndoData) {
        let cf = self.db.cf_handle(CF_UNDO).unwrap();
        let key = height.to_le_bytes();
        let value = bincode::serialize(undo).expect("UndoData serialization");
        self.db.put_cf(cf, key, value).expect("RocksDB put undo");
    }

    /// Get undo data for a block height.
    pub fn get_undo(&self, height: BlockHeight) -> Option<UndoData> {
        let cf = self.db.cf_handle(CF_UNDO).unwrap();
        let key = height.to_le_bytes();
        let bytes = self.db.get_cf(cf, key).ok()??;
        bincode::deserialize(&bytes).ok()
    }

    /// Delete undo data for a block height.
    pub fn delete_undo(&self, height: BlockHeight) {
        let cf = self.db.cf_handle(CF_UNDO).unwrap();
        let key = height.to_le_bytes();
        let _ = self.db.delete_cf(cf, key);
    }

    /// Prune undo data older than `keep_height`.
    /// Called after apply_block to keep only the last N blocks of undo data.
    pub fn prune_undo_before(&self, keep_height: BlockHeight) {
        if keep_height == 0 {
            return;
        }
        let cf = self.db.cf_handle(CF_UNDO).unwrap();
        let mut batch = rocksdb::WriteBatch::default();
        let mut count = 0u64;
        for (key, _) in self
            .db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if key.len() == 8 {
                let h = u64::from_le_bytes(key[..8].try_into().unwrap());
                if h < keep_height {
                    batch.delete_cf(cf, &key);
                    count += 1;
                }
            }
        }
        if count > 0 {
            let _ = self.db.write(batch);
            // Force compaction to reclaim disk space from deleted undo entries.
            // Without this, RocksDB keeps tombstones in SST files indefinitely.
            let start = 0u64.to_le_bytes();
            let end = keep_height.to_le_bytes();
            self.db
                .compact_range_cf(cf, Some(&start[..]), Some(&end[..]));
        }
    }

    /// Delete all undo data above the given height (for truncation).
    pub fn prune_undo_above(&self, keep_height: BlockHeight) {
        let cf = self.db.cf_handle(CF_UNDO).unwrap();
        let mut batch = rocksdb::WriteBatch::default();
        let mut count = 0u64;
        for (key, _) in self
            .db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if key.len() == 8 {
                let h = u64::from_le_bytes(key[..8].try_into().unwrap());
                if h > keep_height {
                    batch.delete_cf(cf, &key);
                    count += 1;
                }
            }
        }
        if count > 0 {
            let _ = self.db.write(batch);
            let start = (keep_height + 1).to_le_bytes();
            let end = u64::MAX.to_le_bytes();
            self.db
                .compact_range_cf(cf, Some(&start[..]), Some(&end[..]));
        }
    }
}
