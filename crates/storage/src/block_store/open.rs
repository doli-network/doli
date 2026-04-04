//! BlockStore initialization and one-time migrations

use std::path::Path;

use tracing::{info, warn};

use crate::StorageError;

use super::types::{
    deserialize_body, BlockStore, CF_ADDR_TX_INDEX, CF_BODIES, CF_HASH_TO_HEIGHT, CF_HEADERS,
    CF_HEIGHT_INDEX, CF_PRESENCE, CF_SLOT_INDEX, CF_TX_INDEX,
};

impl BlockStore {
    /// Open or create a block store
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
        opts.set_max_open_files(256);

        // Bloom filter: speeds up negative lookups (e.g., "does this hash exist?")
        let mut block_opts = rocksdb::BlockBasedOptions::default();
        block_opts.set_bloom_filter(10.0, false);
        opts.set_block_based_table_factory(&block_opts);

        let cfs = vec![
            CF_HEADERS,
            CF_BODIES,
            CF_HEIGHT_INDEX,
            CF_SLOT_INDEX,
            CF_PRESENCE,
            CF_HASH_TO_HEIGHT,
            CF_TX_INDEX,
            CF_ADDR_TX_INDEX,
        ];

        let db = rocksdb::DB::open_cf(&opts, path, cfs)?;

        // One-time migrations
        let store = Self { db };
        store.migrate_hash_to_height_index();
        store.cleanup_presence_cf();
        store.migrate_tx_index();
        store.migrate_addr_tx_index();

        Ok(store)
    }

    /// Populate hash_to_height index from existing height_index entries.
    /// Runs once on first startup after the index is added. No-op if already populated.
    fn migrate_hash_to_height_index(&self) {
        let cf_h2h = self.db.cf_handle(CF_HASH_TO_HEIGHT).unwrap();

        // Check if index already has entries (skip migration)
        if self
            .db
            .iterator_cf(cf_h2h, rocksdb::IteratorMode::Start)
            .flatten()
            .next()
            .is_some()
        {
            return;
        }

        let cf_height = self.db.cf_handle(CF_HEIGHT_INDEX).unwrap();
        let mut batch = rocksdb::WriteBatch::default();
        let mut count = 0u64;

        for (height_bytes, hash_bytes) in self
            .db
            .iterator_cf(cf_height, rocksdb::IteratorMode::Start)
            .flatten()
        {
            // height_index: height (u64 LE) → hash (32 bytes)
            // hash_to_height: hash (32 bytes) → height (u64 LE)
            batch.put_cf(cf_h2h, &hash_bytes, &height_bytes);
            count += 1;
        }

        if count > 0 {
            if let Err(e) = self.db.write(batch) {
                warn!("Failed to migrate hash_to_height index: {}", e);
            } else {
                info!(
                    "[BLOCK_STORE] Migrated hash_to_height index: {} entries",
                    count
                );
            }
        }
    }

    /// One-time cleanup of the deprecated `presence` column family.
    ///
    /// Presence tracking was removed in the deterministic scheduler model
    /// (rewards go 100% to block producer via coinbase). Any leftover data
    /// in CF_PRESENCE is wasted disk space.
    fn cleanup_presence_cf(&self) {
        let cf = self.db.cf_handle(CF_PRESENCE).unwrap();

        // Quick check: skip if already empty
        if self
            .db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .flatten()
            .next()
            .is_none()
        {
            return;
        }

        let mut batch = rocksdb::WriteBatch::default();
        let mut count = 0u64;
        for (key, _) in self
            .db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .flatten()
        {
            batch.delete_cf(cf, &key);
            count += 1;
        }

        if count > 0 {
            if let Err(e) = self.db.write(batch) {
                warn!("Failed to cleanup presence CF: {}", e);
            } else {
                info!(
                    "[BLOCK_STORE] Cleaned up deprecated presence CF: {} entries removed",
                    count
                );
            }
        }
    }

    /// Populate tx_index from existing canonical blocks.
    /// Runs once on first startup after the index is added. No-op if already populated.
    fn migrate_tx_index(&self) {
        let cf_tx = self.db.cf_handle(CF_TX_INDEX).unwrap();

        // Skip if index already has entries
        if self
            .db
            .iterator_cf(cf_tx, rocksdb::IteratorMode::Start)
            .flatten()
            .next()
            .is_some()
        {
            return;
        }

        let cf_height = self.db.cf_handle(CF_HEIGHT_INDEX).unwrap();
        let cf_bodies = self.db.cf_handle(CF_BODIES).unwrap();

        let mut batch = rocksdb::WriteBatch::default();
        let mut tx_count = 0u64;
        let mut block_count = 0u64;

        for (height_bytes, hash_bytes) in self
            .db
            .iterator_cf(cf_height, rocksdb::IteratorMode::Start)
            .flatten()
        {
            // Fetch block body
            let body_bytes = match self.db.get_cf(cf_bodies, &hash_bytes) {
                Ok(Some(b)) => b,
                _ => continue,
            };
            let (transactions, _, _) = match deserialize_body(&body_bytes) {
                Ok(b) => b,
                Err(_) => continue,
            };

            for tx in &transactions {
                let tx_hash = tx.hash();
                batch.put_cf(cf_tx, tx_hash.as_bytes(), &height_bytes);
                tx_count += 1;
            }
            block_count += 1;

            // Write in batches of 10k blocks to avoid huge memory usage
            if block_count.is_multiple_of(10_000) {
                if let Err(e) = self.db.write(std::mem::take(&mut batch)) {
                    warn!("Failed to write tx_index batch: {}", e);
                    return;
                }
                info!(
                    "[BLOCK_STORE] tx_index migration progress: {} blocks, {} txs",
                    block_count, tx_count
                );
            }
        }

        if tx_count > 0 {
            if let Err(e) = self.db.write(batch) {
                warn!("Failed to migrate tx_index: {}", e);
            } else {
                info!(
                    "[BLOCK_STORE] Migrated tx_index: {} txs from {} blocks",
                    tx_count, block_count
                );
            }
        }
    }

    /// Populate addr_tx_index from existing canonical blocks.
    /// Runs once on first startup. No-op if already populated.
    fn migrate_addr_tx_index(&self) {
        let cf_addr = self.db.cf_handle(CF_ADDR_TX_INDEX).unwrap();

        if self
            .db
            .iterator_cf(cf_addr, rocksdb::IteratorMode::Start)
            .flatten()
            .next()
            .is_some()
        {
            return;
        }

        let cf_height = self.db.cf_handle(CF_HEIGHT_INDEX).unwrap();
        let cf_bodies = self.db.cf_handle(CF_BODIES).unwrap();

        let mut batch = rocksdb::WriteBatch::default();
        let mut addr_count = 0u64;
        let mut block_count = 0u64;

        for (height_bytes, hash_bytes) in self
            .db
            .iterator_cf(cf_height, rocksdb::IteratorMode::Start)
            .flatten()
        {
            let body_bytes = match self.db.get_cf(cf_bodies, &hash_bytes) {
                Ok(Some(b)) => b,
                _ => continue,
            };
            let (transactions, _, _) = match deserialize_body(&body_bytes) {
                Ok(b) => b,
                Err(_) => continue,
            };

            let mut seen = std::collections::HashSet::new();
            for tx in &transactions {
                for output in &tx.outputs {
                    let addr_bytes = output.pubkey_hash.as_bytes();
                    if seen.insert(*addr_bytes) {
                        let mut key = [0u8; 40];
                        key[..32].copy_from_slice(addr_bytes);
                        key[32..].copy_from_slice(&height_bytes);
                        batch.put_cf(cf_addr, key, []);
                        addr_count += 1;
                    }
                }
            }
            block_count += 1;

            if block_count.is_multiple_of(10_000) {
                if let Err(e) = self.db.write(std::mem::take(&mut batch)) {
                    warn!("Failed to write addr_tx_index batch: {}", e);
                    return;
                }
                info!(
                    "[BLOCK_STORE] addr_tx_index migration: {} blocks, {} entries",
                    block_count, addr_count
                );
            }
        }

        if addr_count > 0 {
            if let Err(e) = self.db.write(batch) {
                warn!("Failed to migrate addr_tx_index: {}", e);
            } else {
                info!(
                    "[BLOCK_STORE] Migrated addr_tx_index: {} entries from {} blocks",
                    addr_count, block_count
                );
            }
        }
    }
}
