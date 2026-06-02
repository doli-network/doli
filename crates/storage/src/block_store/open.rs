//! BlockStore initialization and one-time migrations

use std::path::Path;

use tracing::{info, warn};

use crate::StorageError;

use super::types::{
    deserialize_body, BlockStore, CF_ADDR_TX_INDEX, CF_BODIES, CF_HASH_TO_HEIGHT, CF_HEADERS,
    CF_HEIGHT_INDEX, CF_META, CF_PRESENCE, CF_SLOT_INDEX, CF_TX_INDEX, DB_WRITE_BUFFER_SIZE_BYTES,
};

/// Build per-CF Options for block_store column families.
///
/// Each CF gets workload-appropriate tuning derived from the DB-level base
/// options. The `cache` reference is shared (Arc-internal) across all CFs
/// within this block_store instance — NOT shared with other DB instances (C-012).
///
/// See `specs/rocksdb-configuration-architecture.md` section block_store.
#[allow(clippy::too_many_arguments)]
fn cf_opts_block_store(
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

impl BlockStore {
    /// Open or create a block store
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
        opts.set_max_open_files(256);
        opts.enable_statistics();

        // INC-I-104 M0: cap total memtable budget across all CFs.
        // DB_WRITE_BUFFER_SIZE_BYTES is shared with `metrics()` so the cap and
        // the reported gauge can never drift.
        opts.set_db_write_buffer_size(DB_WRITE_BUFFER_SIZE_BYTES as usize);
        opts.set_max_total_wal_size(DB_WRITE_BUFFER_SIZE_BYTES);

        // INC-I-104 M2: explicit background job limits.
        opts.set_max_background_jobs(2);
        opts.set_max_subcompactions(1);

        // INC-I-105: explicit 32 MB LRU block cache shared across all 9 CFs.
        // Arc-internal in rust-rocksdb — multiple BlockBasedOptions builders
        // reference the same underlying cache. Per-instance only (C-012).
        let cache = rocksdb::Cache::new_lru_cache(32 * 1024 * 1024);

        // Shorthand aliases for compression types used in the spec table.
        use rocksdb::DBCompressionType::{Lz4, None as NoCompression};

        // INC-I-104 M2: per-CF descriptors with workload-derived tuning.
        // Spec: specs/rocksdb-configuration-architecture.md section block_store.
        //
        // | CF              | wbuf MB | #buf | bloom | blk KB | compr | tgt MB | L0 slow | L0 stop |
        // |-----------------|---------|------|-------|--------|-------|--------|---------|---------|
        // | headers         |    8    |  2   | yes   |   4    | Lz4   |   16   |   40    |   60    |
        // | bodies          |    8    |  2   | yes   |  16    | Lz4   |   32   |   40    |   60    |
        // | height_index    |    4    |  2   | no    |   4    | Lz4   |    8   |  def    |  def    |
        // | slot_index      |    4    |  2   | no    |   4    | Lz4   |    8   |  def    |  def    |
        // | hash_to_height  |    4    |  2   | yes   |   4    | Lz4   |    8   |  def    |  def    |
        // | tx_index        |    4    |  2   | yes   |   4    | Lz4   |    8   |  def    |  def    |
        // | addr_tx_index   |    4    |  2   | NO    |   4    | Lz4   |    8   |  def    |  def    |
        // | presence        |    1    |  1   | no    |   4    | None  |    2   |  def    |  def    |
        // | meta            |    1    |  1   | no    |   4    | None  |    2   |  def    |  def    |
        let cf_descriptors = vec![
            // Hot write + hot point-lookup
            rocksdb::ColumnFamilyDescriptor::new(
                CF_HEADERS,
                cf_opts_block_store(&opts, &cache, 8, 2, true, 4, Lz4, 16, Some(40), Some(60)),
            ),
            // Hot write, large values
            rocksdb::ColumnFamilyDescriptor::new(
                CF_BODIES,
                cf_opts_block_store(&opts, &cache, 8, 2, true, 16, Lz4, 32, Some(40), Some(60)),
            ),
            // Warm index
            rocksdb::ColumnFamilyDescriptor::new(
                CF_HEIGHT_INDEX,
                cf_opts_block_store(&opts, &cache, 4, 2, false, 4, Lz4, 8, None, None),
            ),
            // Warm index
            rocksdb::ColumnFamilyDescriptor::new(
                CF_SLOT_INDEX,
                cf_opts_block_store(&opts, &cache, 4, 2, false, 4, Lz4, 8, None, None),
            ),
            // C-004: deprecated CF — minimal allocation, kept in descriptor list
            rocksdb::ColumnFamilyDescriptor::new(
                CF_PRESENCE,
                cf_opts_block_store(&opts, &cache, 1, 1, false, 4, NoCompression, 2, None, None),
            ),
            // Hot point-lookup
            rocksdb::ColumnFamilyDescriptor::new(
                CF_HASH_TO_HEIGHT,
                cf_opts_block_store(&opts, &cache, 4, 2, true, 4, Lz4, 8, None, None),
            ),
            // Warm write, cold read
            rocksdb::ColumnFamilyDescriptor::new(
                CF_TX_INDEX,
                cf_opts_block_store(&opts, &cache, 4, 2, true, 4, Lz4, 8, None, None),
            ),
            // Prefix-scan reads — C-010: NO bloom (bloom hurts prefix iteration)
            rocksdb::ColumnFamilyDescriptor::new(
                CF_ADDR_TX_INDEX,
                cf_opts_block_store(&opts, &cache, 4, 2, false, 4, Lz4, 8, None, None),
            ),
            // Cold, 1 key
            rocksdb::ColumnFamilyDescriptor::new(
                CF_META,
                cf_opts_block_store(&opts, &cache, 1, 1, false, 4, NoCompression, 2, None, None),
            ),
        ];

        let db = rocksdb::DB::open_cf_descriptors(&opts, path, cf_descriptors)?;

        // One-time migrations
        let store = Self {
            db,
            block_cache: cache,
            block_cache_capacity_bytes: 32 * 1024 * 1024,
        };
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
