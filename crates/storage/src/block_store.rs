//! Block storage

use std::path::Path;

use crypto::Hash;
use doli_core::{Block, BlockHeader};
use tracing::{debug, info, warn};

use crate::StorageError;

/// Column family names
const CF_HEADERS: &str = "headers";
const CF_BODIES: &str = "bodies";
const CF_HEIGHT_INDEX: &str = "height_index";
const CF_SLOT_INDEX: &str = "slot_index";
const CF_PRESENCE: &str = "presence";
const CF_HASH_TO_HEIGHT: &str = "hash_to_height";

/// Block store
pub struct BlockStore {
    db: rocksdb::DB,
}

impl BlockStore {
    /// Open or create a block store
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);

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
        ];

        let db = rocksdb::DB::open_cf(&opts, path, cfs)?;

        // One-time migration: populate hash_to_height from height_index
        // for blocks stored before this index was added.
        let store = Self { db };
        store.migrate_hash_to_height_index();
        store.cleanup_presence_cf();

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

    /// Remove non-canonical (fork) blocks from the store.
    ///
    /// Iterates all headers and checks if each hash exists in the
    /// canonical chain index (hash_to_height). Blocks not in the
    /// canonical chain are deleted from headers, bodies, and slot_index.
    ///
    /// Returns the number of fork blocks removed.
    pub fn cleanup_fork_blocks(&self) -> Result<u64, StorageError> {
        let cf_headers = self.db.cf_handle(CF_HEADERS).unwrap();
        let cf_bodies = self.db.cf_handle(CF_BODIES).unwrap();
        let cf_slot = self.db.cf_handle(CF_SLOT_INDEX).unwrap();
        let cf_h2h = self.db.cf_handle(CF_HASH_TO_HEIGHT).unwrap();

        let mut batch = rocksdb::WriteBatch::default();
        let mut removed = 0u64;
        let mut total = 0u64;

        for (key_bytes, _) in self
            .db
            .iterator_cf(cf_headers, rocksdb::IteratorMode::Start)
            .flatten()
        {
            total += 1;
            // Check if this hash is in the canonical index
            if self.db.get_cf(cf_h2h, &key_bytes)?.is_none() {
                // Not canonical — delete header + body
                batch.delete_cf(cf_headers, &key_bytes);
                batch.delete_cf(cf_bodies, &key_bytes);

                // Also try to remove from slot_index if this block has a slot entry
                // pointing to this non-canonical hash
                if key_bytes.len() == 32 {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&key_bytes);
                    let hash = Hash::from_bytes(arr);
                    if let Ok(Some(header)) = self.get_header(&hash) {
                        let slot_bytes = header.slot.to_le_bytes();
                        // Only delete slot entry if it points to this fork block
                        if let Ok(Some(slot_hash_bytes)) = self.db.get_cf(cf_slot, slot_bytes) {
                            if slot_hash_bytes[..] == key_bytes[..] {
                                batch.delete_cf(cf_slot, slot_bytes);
                            }
                        }
                    }
                }

                removed += 1;
            }
        }

        if removed > 0 {
            self.db.write(batch)?;
            info!(
                "[BLOCK_STORE] Fork cleanup: removed {} non-canonical blocks ({} total scanned)",
                removed, total
            );
        } else {
            debug!(
                "[BLOCK_STORE] Fork cleanup: no fork blocks found ({} total scanned)",
                total
            );
        }

        Ok(removed)
    }

    /// Clear all block data from the store.
    ///
    /// Removes all entries from every column family (headers, bodies,
    /// height_index, slot_index, presence). Used during force resync
    /// from genesis to purge stale fork blocks.
    pub fn clear(&self) -> Result<(), StorageError> {
        let cf_names = [
            CF_HEADERS,
            CF_BODIES,
            CF_HEIGHT_INDEX,
            CF_SLOT_INDEX,
            CF_PRESENCE,
            CF_HASH_TO_HEIGHT,
        ];

        for cf_name in &cf_names {
            let cf = self.db.cf_handle(cf_name).unwrap();
            let mut batch = rocksdb::WriteBatch::default();
            let iter = self.db.iterator_cf(&cf, rocksdb::IteratorMode::Start);
            let mut count = 0u64;
            for (key, _) in iter.flatten() {
                batch.delete_cf(&cf, &key);
                count += 1;
            }
            if count > 0 {
                self.db.write(batch)?;
                warn!(
                    "[BLOCK_STORE] Cleared {} entries from CF '{}'",
                    count, cf_name
                );
            }
        }

        info!("[BLOCK_STORE] All column families cleared");
        Ok(())
    }

    /// Store a block (header + body + slot index).
    ///
    /// Does NOT update height_index or hash_to_height — those are managed
    /// exclusively by `set_canonical_chain()` to ensure they always reflect
    /// the canonical chain, not fork blocks.
    pub fn put_block(&self, block: &Block, height: u64) -> Result<(), StorageError> {
        let hash = block.hash();
        let hash_bytes = hash.as_bytes();
        let slot = block.slot();

        info!("[BLOCK_STORE] put_block: height={}, slot={}", height, slot);

        // Store header (keyed by hash — stores ALL blocks including forks)
        let header_bytes = bincode::serialize(&block.header)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let cf_headers = self.db.cf_handle(CF_HEADERS).unwrap();
        self.db.put_cf(cf_headers, hash_bytes, &header_bytes)?;

        // Store body (keyed by hash)
        let body_bytes = bincode::serialize(&block.transactions)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let cf_bodies = self.db.cf_handle(CF_BODIES).unwrap();
        self.db.put_cf(cf_bodies, hash_bytes, &body_bytes)?;

        // Update slot index
        let cf_slot = self.db.cf_handle(CF_SLOT_INDEX).unwrap();
        self.db.put_cf(cf_slot, slot.to_le_bytes(), hash_bytes)?;

        Ok(())
    }

    /// Store a block and immediately mark it as canonical at the given height.
    ///
    /// Convenience method for simple chains (no forks). Equivalent to
    /// `put_block()` + direct height_index/hash_to_height update.
    /// For chains with forks, use `put_block()` + `set_canonical_chain()`.
    pub fn put_block_canonical(&self, block: &Block, height: u64) -> Result<(), StorageError> {
        let hash = block.hash();
        self.put_block(block, height)?;

        let cf_height = self.db.cf_handle(CF_HEIGHT_INDEX).unwrap();
        self.db
            .put_cf(cf_height, height.to_le_bytes(), hash.as_bytes())?;

        let cf_h2h = self.db.cf_handle(CF_HASH_TO_HEIGHT).unwrap();
        self.db
            .put_cf(cf_h2h, hash.as_bytes(), height.to_le_bytes())?;

        Ok(())
    }

    /// Update canonical chain indexes (height_index + hash_to_height).
    ///
    /// Walks backwards from `tip_hash` at `tip_height` via prev_hash,
    /// updating height_index and hash_to_height for each block. Stops
    /// early at the common ancestor (where height_index already points
    /// to this hash), so a 10-block reorg only writes 10 entries.
    ///
    /// This is the ONLY method that writes to height_index/hash_to_height.
    /// Called after every block insertion that becomes the new tip.
    pub fn set_canonical_chain(&self, tip_hash: Hash, tip_height: u64) -> Result<(), StorageError> {
        let cf_height = self.db.cf_handle(CF_HEIGHT_INDEX).unwrap();
        let cf_h2h = self.db.cf_handle(CF_HASH_TO_HEIGHT).unwrap();

        let mut batch = rocksdb::WriteBatch::default();
        let mut current_hash = tip_hash;
        let mut height = tip_height;
        let mut updated = 0u64;

        loop {
            // Early exit: if height_index already points to this hash,
            // the chain below is already canonical — no need to continue.
            if let Some(existing) = self.get_hash_by_height(height)? {
                if existing == current_hash {
                    break;
                }
            }

            // Update both indexes for this height
            batch.put_cf(cf_height, height.to_le_bytes(), current_hash.as_bytes());
            batch.put_cf(cf_h2h, current_hash.as_bytes(), height.to_le_bytes());
            updated += 1;

            if height == 0 {
                break;
            }

            // Walk backwards via prev_hash
            let header = self.get_header(&current_hash)?.ok_or_else(|| {
                StorageError::NotFound(format!("header {} missing", current_hash))
            })?;
            current_hash = header.prev_hash;
            height -= 1;
        }

        if updated > 0 {
            self.db.write(batch)?;
            if updated > 1 {
                info!(
                    "[BLOCK_STORE] Canonical chain updated: {} entries (tip={}, h={})",
                    updated,
                    &tip_hash.to_string()[..16],
                    tip_height
                );
            }
        }

        Ok(())
    }

    /// Rebuild canonical chain index from scratch by scanning the headers CF.
    ///
    /// Does NOT use height_index at all. Finds the true chain tip by scanning
    /// all headers for the one with the highest slot, then walks backwards via
    /// prev_hash assigning heights decrementally. Overwrites height_index and
    /// hash_to_height completely.
    ///
    /// Use this when height_index is corrupt (e.g., fork blocks polluted it).
    pub fn rebuild_canonical_index(&self) -> Result<(Hash, u64), StorageError> {
        let cf_headers = self.db.cf_handle(CF_HEADERS).unwrap();
        let cf_height = self.db.cf_handle(CF_HEIGHT_INDEX).unwrap();
        let cf_h2h = self.db.cf_handle(CF_HASH_TO_HEIGHT).unwrap();

        // Step 1: Scan ALL headers to find the one with the highest slot (true tip).
        info!("[REINDEX] Scanning all headers to find chain tip...");
        let mut best_slot = 0u32;
        let mut best_hash = Hash::ZERO;
        let mut total_headers = 0u64;

        for (key_bytes, val_bytes) in self
            .db
            .iterator_cf(cf_headers, rocksdb::IteratorMode::Start)
            .flatten()
        {
            total_headers += 1;
            let header: BlockHeader = bincode::deserialize(&val_bytes)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;

            if header.slot > best_slot {
                best_slot = header.slot;
                if key_bytes.len() == 32 {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&key_bytes);
                    best_hash = Hash::from_bytes(arr);
                }
            }
        }

        if best_hash == Hash::ZERO {
            info!("[REINDEX] No headers found — nothing to index");
            return Ok((Hash::ZERO, 0));
        }

        info!(
            "[REINDEX] Found {} total headers. Tip: slot={}, hash={}...",
            total_headers,
            best_slot,
            &best_hash.to_string()[..16]
        );

        // Step 2: Walk backwards from tip via prev_hash, collecting the canonical chain.
        // We collect first, then write, because we don't know the height until we
        // reach the genesis block.
        let mut chain: Vec<Hash> = Vec::new();
        let mut current = best_hash;

        loop {
            chain.push(current);
            let header = self.get_header(&current)?.ok_or_else(|| {
                StorageError::NotFound(format!("header {} missing during reindex", current))
            })?;

            if header.prev_hash == Hash::ZERO {
                break; // Block explicitly has no parent (unlikely in DOLI)
            }

            // Check parent exists in the store
            if !self.has_block(&header.prev_hash)? {
                // Parent hash is not stored as a block — it's the genesis hash
                // (a virtual block). Include it as height 0 so the height
                // assignment matches the original chain (genesis=0, first block=1).
                chain.push(header.prev_hash);
                break;
            }

            current = header.prev_hash;
        }

        // chain is [tip, tip-1, ..., first_block, genesis_hash] — reverse
        chain.reverse();
        let tip_height = (chain.len() - 1) as u64;

        info!(
            "[REINDEX] Canonical chain: {} blocks (height 0..={})",
            chain.len(),
            tip_height
        );

        // Step 3: Clear old height_index and hash_to_height, write fresh entries.
        // Clear height_index
        let mut clear_batch = rocksdb::WriteBatch::default();
        for (key, _) in self
            .db
            .iterator_cf(cf_height, rocksdb::IteratorMode::Start)
            .flatten()
        {
            clear_batch.delete_cf(cf_height, &key);
        }
        for (key, _) in self
            .db
            .iterator_cf(cf_h2h, rocksdb::IteratorMode::Start)
            .flatten()
        {
            clear_batch.delete_cf(cf_h2h, &key);
        }
        self.db.write(clear_batch)?;

        // Write canonical entries
        let mut write_batch = rocksdb::WriteBatch::default();
        for (height, hash) in chain.iter().enumerate() {
            let h = height as u64;
            write_batch.put_cf(cf_height, h.to_le_bytes(), hash.as_bytes());
            write_batch.put_cf(cf_h2h, hash.as_bytes(), h.to_le_bytes());
        }
        self.db.write(write_batch)?;

        info!(
            "[REINDEX] Canonical index rebuilt: {} entries, tip height={}",
            chain.len(),
            tip_height
        );

        Ok((best_hash, tip_height))
    }

    /// Get a block by hash
    pub fn get_block(&self, hash: &Hash) -> Result<Option<Block>, StorageError> {
        let cf_headers = self.db.cf_handle(CF_HEADERS).unwrap();
        let cf_bodies = self.db.cf_handle(CF_BODIES).unwrap();

        let header_bytes = match self.db.get_cf(cf_headers, hash.as_bytes())? {
            Some(b) => b,
            None => return Ok(None),
        };

        let body_bytes = match self.db.get_cf(cf_bodies, hash.as_bytes())? {
            Some(b) => b,
            None => return Ok(None),
        };

        let header: BlockHeader = bincode::deserialize(&header_bytes)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let transactions = bincode::deserialize(&body_bytes)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        // NOTE: Presence loading removed - deterministic scheduler model
        // uses coinbase rewards (100% to producer), not presence-based rewards

        Ok(Some(Block::new(header, transactions)))
    }

    /// Get a header by hash
    pub fn get_header(&self, hash: &Hash) -> Result<Option<BlockHeader>, StorageError> {
        let cf_headers = self.db.cf_handle(CF_HEADERS).unwrap();

        let bytes = match self.db.get_cf(cf_headers, hash.as_bytes())? {
            Some(b) => b,
            None => return Ok(None),
        };

        let header: BlockHeader =
            bincode::deserialize(&bytes).map_err(|e| StorageError::Serialization(e.to_string()))?;

        Ok(Some(header))
    }

    /// Get block height by hash (O(1) reverse index lookup)
    pub fn get_height_by_hash(&self, hash: &Hash) -> Result<Option<u64>, StorageError> {
        let cf = self.db.cf_handle(CF_HASH_TO_HEIGHT).unwrap();
        let bytes = match self.db.get_cf(cf, hash.as_bytes())? {
            Some(b) => b,
            None => return Ok(None),
        };
        if bytes.len() != 8 {
            return Err(StorageError::Serialization("invalid height length".into()));
        }
        let mut arr = [0u8; 8];
        arr.copy_from_slice(&bytes);
        Ok(Some(u64::from_le_bytes(arr)))
    }

    /// Get block hash by height
    pub fn get_hash_by_height(&self, height: u64) -> Result<Option<Hash>, StorageError> {
        let cf_height = self.db.cf_handle(CF_HEIGHT_INDEX).unwrap();

        let bytes = match self.db.get_cf(cf_height, height.to_le_bytes())? {
            Some(b) => b,
            None => return Ok(None),
        };

        if bytes.len() != 32 {
            return Err(StorageError::Serialization("invalid hash length".into()));
        }

        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Some(Hash::from_bytes(arr)))
    }

    /// Get block by height
    pub fn get_block_by_height(&self, height: u64) -> Result<Option<Block>, StorageError> {
        let hash = match self.get_hash_by_height(height)? {
            Some(h) => h,
            None => return Ok(None),
        };
        self.get_block(&hash)
    }

    /// Check if block exists
    pub fn has_block(&self, hash: &Hash) -> Result<bool, StorageError> {
        let cf_headers = self.db.cf_handle(CF_HEADERS).unwrap();
        Ok(self.db.get_cf(cf_headers, hash.as_bytes())?.is_some())
    }

    // ==================== Milestone 1: BlockStore Query Methods ====================
    //
    // These methods support deterministic epoch reward calculation from the BlockStore.
    // See REWARDS.md for the complete refactoring plan.

    /// Get block hash by slot
    ///
    /// Uses CF_SLOT_INDEX to look up the block hash for a given slot.
    /// Returns None for empty slots (slots where no block was produced).
    pub fn get_hash_by_slot(&self, slot: u32) -> Result<Option<Hash>, StorageError> {
        let cf_slot = self.db.cf_handle(CF_SLOT_INDEX).unwrap();

        let bytes = match self.db.get_cf(cf_slot, slot.to_le_bytes())? {
            Some(b) => b,
            None => return Ok(None),
        };

        if bytes.len() != 32 {
            return Err(StorageError::Serialization("invalid hash length".into()));
        }

        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Some(Hash::from_bytes(arr)))
    }

    /// Get block by slot
    ///
    /// Returns the block at the given slot, or None if the slot is empty.
    /// This is the primary method for slot-based block retrieval.
    pub fn get_block_by_slot(&self, slot: u32) -> Result<Option<Block>, StorageError> {
        let hash = match self.get_hash_by_slot(slot)? {
            Some(h) => h,
            None => return Ok(None),
        };
        self.get_block(&hash)
    }

    /// Check if a block exists for the given slot.
    ///
    /// This is a fast check used by block production to avoid producing
    /// duplicate blocks. If a block already exists for this slot, the
    /// producer should skip production to prevent forks.
    ///
    /// # Arguments
    /// * `slot` - The slot to check (u64 for compatibility with slot calculations)
    ///
    /// # Returns
    /// `true` if a block exists for this slot, `false` otherwise.
    pub fn has_block_for_slot(&self, slot: u64) -> bool {
        // Convert to u32 for the slot index lookup
        let slot_u32 = slot as u32;
        self.get_hash_by_slot(slot_u32)
            .map(|opt| opt.is_some())
            .unwrap_or(false)
    }

    /// Get all blocks in a slot range (inclusive start, exclusive end)
    ///
    /// Returns blocks in slot order, skipping empty slots.
    /// Used for calculating epoch rewards from a deterministic slot range.
    ///
    /// # Arguments
    /// * `start` - First slot in range (inclusive)
    /// * `end` - Last slot in range (exclusive)
    ///
    /// # Returns
    /// Vec of blocks in slot order. Empty slots are skipped.
    pub fn get_blocks_in_slot_range(
        &self,
        start: u32,
        end: u32,
    ) -> Result<Vec<Block>, StorageError> {
        let mut blocks = Vec::new();

        for slot in start..end {
            if let Some(block) = self.get_block_by_slot(slot)? {
                blocks.push(block);
            }
        }

        Ok(blocks)
    }

    /// Check if any block exists in the specified slot range.
    ///
    /// Range is `[start, end)` - inclusive start, exclusive end.
    /// Returns true if at least one block exists in the range.
    ///
    /// This is more efficient than `get_blocks_in_slot_range` when you only
    /// need to check for emptiness (e.g., skipping empty epochs during catch-up).
    pub fn has_any_block_in_slot_range(&self, start: u32, end: u32) -> Result<bool, StorageError> {
        if start >= end {
            return Ok(false);
        }
        for slot in start..end {
            if self.get_hash_by_slot(slot)?.is_some() {
                debug!(
                    "[BLOCK_STORE] Found block at slot {} in range [{}, {})",
                    slot, start, end
                );
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Count total blocks in the slot index (diagnostic method)
    pub fn count_slot_index_entries(&self) -> Result<u64, StorageError> {
        let cf_slot = self.db.cf_handle(CF_SLOT_INDEX).unwrap();
        let iter = self.db.iterator_cf(cf_slot, rocksdb::IteratorMode::Start);
        let mut count = 0u64;
        let mut min_slot = u32::MAX;
        let mut max_slot = 0u32;

        for (key, _) in iter.flatten() {
            count += 1;
            if key.len() == 4 {
                let slot = u32::from_le_bytes([key[0], key[1], key[2], key[3]]);
                min_slot = min_slot.min(slot);
                max_slot = max_slot.max(slot);
            }
        }

        if count > 0 {
            debug!(
                "[BLOCK_STORE] Slot index: {} entries, slots {} to {}",
                count, min_slot, max_slot
            );
        } else {
            debug!("[BLOCK_STORE] Slot index: EMPTY");
        }

        Ok(count)
    }

    /// Get the last rewarded epoch number from the chain
    ///
    /// Scans backwards from the chain tip to find the most recent block
    /// containing an EpochReward transaction, then extracts the epoch number.
    ///
    /// Returns 0 if no epoch rewards have ever been distributed.
    ///
    /// This is used to determine which epoch(s) need reward distribution
    /// at the current block production time.
    pub fn get_last_rewarded_epoch(&self) -> Result<u64, StorageError> {
        use doli_core::transaction::EpochRewardData;

        // Iterate backwards through the height index to find the most recent
        // block with an EpochReward transaction
        let cf_height = self.db.cf_handle(CF_HEIGHT_INDEX).unwrap();

        // Use RocksDB iterator in reverse mode
        let mut iter = self.db.iterator_cf(
            cf_height,
            rocksdb::IteratorMode::End, // Start from the end (highest height)
        );

        while let Some(Ok((_, hash_bytes))) = iter.next() {
            if hash_bytes.len() != 32 {
                continue;
            }

            let mut arr = [0u8; 32];
            arr.copy_from_slice(&hash_bytes);
            let hash = Hash::from_bytes(arr);

            if let Some(block) = self.get_block(&hash)? {
                // Check if this block has any EpochReward transactions
                for tx in &block.transactions {
                    if tx.is_epoch_reward() {
                        if let Some(data) = EpochRewardData::from_bytes(&tx.extra_data) {
                            return Ok(data.epoch);
                        }
                    }
                }
            }
        }

        // No epoch rewards found - this is epoch 0 or no rewards distributed yet
        Ok(0)
    }
}

/// Implement EpochBlockSource trait for BlockStore to enable deterministic
/// epoch reward validation in the core crate.
impl doli_core::validation::EpochBlockSource for BlockStore {
    fn last_rewarded_epoch(&self) -> Result<u64, String> {
        self.get_last_rewarded_epoch()
            .map_err(|e| format!("storage error: {}", e))
    }

    fn blocks_in_slot_range(&self, start: u32, end: u32) -> Result<Vec<Block>, String> {
        self.get_blocks_in_slot_range(start, end)
            .map_err(|e| format!("storage error: {}", e))
    }

    fn has_any_block_in_slot_range(&self, start: u32, end: u32) -> Result<bool, String> {
        self.has_any_block_in_slot_range(start, end)
            .map_err(|e| format!("storage error: {}", e))
    }
}

/// Implement BlockSource trait for BlockStore to enable weighted reward calculation.
///
/// This allows the WeightedRewardCalculator to access blocks by height for
/// calculating epoch rewards based on presence commitments.
impl doli_core::rewards::BlockSource for BlockStore {
    fn get_block_by_height(
        &self,
        height: doli_core::BlockHeight,
    ) -> Result<Option<Block>, doli_core::rewards::RewardError> {
        self.get_block_by_height(height)
            .map_err(|e| doli_core::rewards::RewardError::StorageError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::hash::{hash as crypto_hash, hash_with_domain};
    use crypto::{KeyPair, PublicKey, ADDRESS_DOMAIN};
    use doli_core::{Block, BlockHeader, Transaction};
    use tempfile::TempDir;
    use vdf::{VdfOutput, VdfProof};

    /// Create a test BlockStore in a temporary directory
    fn create_test_store() -> (BlockStore, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let store = BlockStore::open(temp_dir.path()).unwrap();
        (store, temp_dir)
    }

    /// Create a test block header with specified slot and producer
    fn create_test_header(slot: u32, producer: &PublicKey) -> BlockHeader {
        BlockHeader {
            version: 1,
            prev_hash: Hash::ZERO,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            timestamp: 1000 + slot as u64 * 10,
            slot,
            producer: producer.clone(),
            vdf_output: VdfOutput { value: Vec::new() },
            vdf_proof: VdfProof::empty(),
        }
    }

    /// Create a simple test block with no transactions
    fn create_test_block(slot: u32, producer: &PublicKey) -> Block {
        Block::new(create_test_header(slot, producer), vec![])
    }

    /// Create a block with an EpochReward transaction
    fn create_epoch_reward_block(slot: u32, producer: &PublicKey, epoch: u64) -> Block {
        let epoch_reward_tx = Transaction::new_epoch_reward(
            epoch,
            producer.clone(),
            100_000_000, // 1 DOLI
            hash_with_domain(ADDRESS_DOMAIN, producer.as_bytes()),
        );
        Block::new(create_test_header(slot, producer), vec![epoch_reward_tx])
    }

    #[test]
    fn test_get_block_by_slot_empty() {
        let (store, _dir) = create_test_store();

        // Query non-existent slot
        let result = store.get_block_by_slot(100).unwrap();
        assert!(result.is_none(), "Empty slot should return None");
    }

    #[test]
    fn test_get_block_by_slot_found() {
        let (store, _dir) = create_test_store();
        let keypair = KeyPair::generate();
        let producer = keypair.public_key().clone();

        // Store a block at slot 42
        let block = create_test_block(42, &producer);
        store.put_block_canonical(&block, 1).unwrap();

        // Retrieve by slot
        let retrieved = store.get_block_by_slot(42).unwrap();
        assert!(retrieved.is_some(), "Block should be found at slot 42");
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.slot(), 42);
        assert_eq!(retrieved.header.producer, producer);
    }

    #[test]
    fn test_get_blocks_in_slot_range_empty() {
        let (store, _dir) = create_test_store();

        // Query empty range
        let blocks = store.get_blocks_in_slot_range(0, 100).unwrap();
        assert!(blocks.is_empty(), "Empty range should return empty vec");
    }

    #[test]
    fn test_get_blocks_in_slot_range_with_gaps() {
        let (store, _dir) = create_test_store();
        let keypair = KeyPair::generate();
        let producer = keypair.public_key().clone();

        // Store blocks at slots 10, 12, 15 (with gaps at 11, 13, 14)
        let slots = [10u32, 12, 15];
        for (height, &slot) in slots.iter().enumerate() {
            let block = create_test_block(slot, &producer);
            store.put_block_canonical(&block, height as u64).unwrap();
        }

        // Query range 10..20
        let blocks = store.get_blocks_in_slot_range(10, 20).unwrap();
        assert_eq!(blocks.len(), 3, "Should return 3 blocks");
        assert_eq!(blocks[0].slot(), 10);
        assert_eq!(blocks[1].slot(), 12);
        assert_eq!(blocks[2].slot(), 15);
    }

    #[test]
    fn test_get_blocks_in_slot_range_ordering() {
        let (store, _dir) = create_test_store();
        let keypair = KeyPair::generate();
        let producer = keypair.public_key().clone();

        // Store blocks in non-sequential order
        for (height, slot) in [(0, 5u32), (1, 3), (2, 7), (3, 1)] {
            let block = create_test_block(slot, &producer);
            store.put_block_canonical(&block, height).unwrap();
        }

        // Blocks should be returned in slot order
        let blocks = store.get_blocks_in_slot_range(0, 10).unwrap();
        assert_eq!(blocks.len(), 4);
        assert_eq!(blocks[0].slot(), 1);
        assert_eq!(blocks[1].slot(), 3);
        assert_eq!(blocks[2].slot(), 5);
        assert_eq!(blocks[3].slot(), 7);
    }

    #[test]
    fn test_get_last_rewarded_epoch_no_rewards() {
        let (store, _dir) = create_test_store();
        let keypair = KeyPair::generate();
        let producer = keypair.public_key().clone();

        // Store blocks without epoch rewards
        for height in 0..5 {
            let block = create_test_block(height as u32, &producer);
            store.put_block_canonical(&block, height).unwrap();
        }

        // Should return 0 when no rewards exist
        let last_epoch = store.get_last_rewarded_epoch().unwrap();
        assert_eq!(last_epoch, 0, "No rewards should return epoch 0");
    }

    #[test]
    fn test_get_last_rewarded_epoch_single_reward() {
        let (store, _dir) = create_test_store();
        let keypair = KeyPair::generate();
        let producer = keypair.public_key().clone();

        // Store a block with epoch reward for epoch 5
        let block = create_epoch_reward_block(360, &producer, 5);
        store.put_block_canonical(&block, 0).unwrap();

        let last_epoch = store.get_last_rewarded_epoch().unwrap();
        assert_eq!(last_epoch, 5, "Should return epoch 5");
    }

    #[test]
    fn test_get_last_rewarded_epoch_multiple_rewards() {
        let (store, _dir) = create_test_store();
        let keypair = KeyPair::generate();
        let producer = keypair.public_key().clone();

        // Store blocks with epoch rewards for epochs 1, 2, 3
        let reward_blocks = [
            (360, 1u64), // epoch 1 at slot 360
            (720, 2),    // epoch 2 at slot 720
            (1080, 3),   // epoch 3 at slot 1080
        ];

        for (height, (slot, epoch)) in reward_blocks.iter().enumerate() {
            let block = create_epoch_reward_block(*slot, &producer, *epoch);
            store.put_block_canonical(&block, height as u64).unwrap();
        }

        // Should return the most recent (highest) epoch
        let last_epoch = store.get_last_rewarded_epoch().unwrap();
        assert_eq!(last_epoch, 3, "Should return most recent epoch 3");
    }

    #[test]
    fn test_get_last_rewarded_epoch_mixed_blocks() {
        let (store, _dir) = create_test_store();
        let keypair = KeyPair::generate();
        let producer = keypair.public_key().clone();

        // Store a mix of regular blocks and epoch reward blocks
        // height 0: regular block
        store
            .put_block_canonical(&create_test_block(1, &producer), 0)
            .unwrap();

        // height 1: epoch reward for epoch 1
        store
            .put_block_canonical(&create_epoch_reward_block(360, &producer, 1), 1)
            .unwrap();

        // height 2: regular block
        store
            .put_block_canonical(&create_test_block(361, &producer), 2)
            .unwrap();

        // height 3: epoch reward for epoch 2
        store
            .put_block_canonical(&create_epoch_reward_block(720, &producer, 2), 3)
            .unwrap();

        // height 4: regular block (most recent)
        store
            .put_block_canonical(&create_test_block(721, &producer), 4)
            .unwrap();

        // Should still find epoch 2 as the last rewarded
        let last_epoch = store.get_last_rewarded_epoch().unwrap();
        assert_eq!(
            last_epoch, 2,
            "Should find epoch 2 through non-reward blocks"
        );
    }

    #[test]
    fn test_get_hash_by_slot() {
        let (store, _dir) = create_test_store();
        let keypair = KeyPair::generate();
        let producer = keypair.public_key().clone();

        // Store a block
        let block = create_test_block(42, &producer);
        let expected_hash = block.hash();
        store.put_block_canonical(&block, 0).unwrap();

        // Retrieve hash by slot
        let hash = store.get_hash_by_slot(42).unwrap();
        assert!(hash.is_some());
        assert_eq!(hash.unwrap(), expected_hash);

        // Non-existent slot
        let hash = store.get_hash_by_slot(999).unwrap();
        assert!(hash.is_none());
    }

    #[test]
    fn test_has_any_block_in_slot_range() {
        let (store, _dir) = create_test_store();
        let keypair = KeyPair::generate();
        let producer = keypair.public_key().clone();

        // Store blocks at slots 100, 150, 200
        for (height, slot) in [(0, 100u32), (1, 150), (2, 200)] {
            let block = create_test_block(slot, &producer);
            store.put_block_canonical(&block, height).unwrap();
        }

        // Range with blocks
        assert!(store.has_any_block_in_slot_range(100, 200).unwrap());
        assert!(store.has_any_block_in_slot_range(99, 101).unwrap());
        assert!(store.has_any_block_in_slot_range(150, 151).unwrap());

        // Range without blocks (empty epochs)
        assert!(!store.has_any_block_in_slot_range(0, 100).unwrap());
        assert!(!store.has_any_block_in_slot_range(101, 150).unwrap());
        assert!(!store.has_any_block_in_slot_range(201, 300).unwrap());

        // Edge cases
        assert!(!store.has_any_block_in_slot_range(100, 100).unwrap()); // Empty range
        assert!(!store.has_any_block_in_slot_range(200, 100).unwrap()); // Invalid range
    }

    // =========================================================================
    // Milestone 6: Additional Edge Case Tests
    // =========================================================================

    #[test]
    fn test_get_block_by_slot_empty_in_chain_with_gaps() {
        // Query an empty slot in a chain that has blocks with gaps
        let (store, _dir) = create_test_store();
        let keypair = KeyPair::generate();
        let producer = keypair.public_key().clone();

        // Store blocks at slots 10, 12, 15 (gaps at 11, 13, 14)
        let slots = [10u32, 12, 15];
        for (height, &slot) in slots.iter().enumerate() {
            let block = create_test_block(slot, &producer);
            store.put_block_canonical(&block, height as u64).unwrap();
        }

        // Query empty slot 11 (between 10 and 12)
        let result = store.get_block_by_slot(11).unwrap();
        assert!(result.is_none(), "Empty slot 11 should return None");

        // Query empty slot 13 (between 12 and 15)
        let result = store.get_block_by_slot(13).unwrap();
        assert!(result.is_none(), "Empty slot 13 should return None");

        // Query empty slot 14 (between 12 and 15)
        let result = store.get_block_by_slot(14).unwrap();
        assert!(result.is_none(), "Empty slot 14 should return None");

        // Verify existing slots still work
        assert!(store.get_block_by_slot(10).unwrap().is_some());
        assert!(store.get_block_by_slot(12).unwrap().is_some());
        assert!(store.get_block_by_slot(15).unwrap().is_some());
    }

    #[test]
    fn test_get_blocks_in_slot_range_boundary_conditions() {
        let (store, _dir) = create_test_store();
        let keypair = KeyPair::generate();
        let producer = keypair.public_key().clone();

        // Store blocks at slots 5, 10, 15
        for (height, slot) in [(0, 5u32), (1, 10), (2, 15)] {
            let block = create_test_block(slot, &producer);
            store.put_block_canonical(&block, height).unwrap();
        }

        // Test: start == end (empty range)
        let blocks = store.get_blocks_in_slot_range(10, 10).unwrap();
        assert!(blocks.is_empty(), "start == end should return empty vec");

        // Test: start > end (invalid range)
        let blocks = store.get_blocks_in_slot_range(15, 5).unwrap();
        assert!(blocks.is_empty(), "start > end should return empty vec");

        // Test: range before any blocks
        let blocks = store.get_blocks_in_slot_range(0, 4).unwrap();
        assert!(blocks.is_empty(), "Range before blocks should be empty");

        // Test: range after all blocks
        let blocks = store.get_blocks_in_slot_range(20, 30).unwrap();
        assert!(blocks.is_empty(), "Range after blocks should be empty");

        // Test: range exactly matches one block
        let blocks = store.get_blocks_in_slot_range(10, 11).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].slot(), 10);

        // Test: range includes all blocks
        let blocks = store.get_blocks_in_slot_range(0, 20).unwrap();
        assert_eq!(blocks.len(), 3);
    }

    #[test]
    fn test_get_last_rewarded_epoch_with_epoch_gaps() {
        // Test that get_last_rewarded_epoch handles non-sequential epoch numbers
        let (store, _dir) = create_test_store();
        let keypair = KeyPair::generate();
        let producer = keypair.public_key().clone();

        // Store epoch rewards for epochs 1, 3, 5 (skipping 2 and 4)
        // This simulates a scenario where some epochs had no blocks
        let epochs = [(360, 1u64), (1080, 3), (1800, 5)];
        for (height, (slot, epoch)) in epochs.iter().enumerate() {
            let block = create_epoch_reward_block(*slot, &producer, *epoch);
            store.put_block_canonical(&block, height as u64).unwrap();
        }

        // Should return the highest epoch (5), not just the most recent by height
        let last_epoch = store.get_last_rewarded_epoch().unwrap();
        assert_eq!(last_epoch, 5, "Should return highest epoch 5");
    }

    #[test]
    fn test_get_blocks_in_slot_range_multiple_epochs() {
        // Test querying blocks across multiple epochs
        let (store, _dir) = create_test_store();
        let keypair1 = KeyPair::generate();
        let keypair2 = KeyPair::generate();
        let producer1 = keypair1.public_key().clone();
        let producer2 = keypair2.public_key().clone();

        // Store blocks across 3 epochs (using devnet 30 slots/epoch)
        // Epoch 0: slots 1-29
        // Epoch 1: slots 30-59
        // Epoch 2: slots 60-89

        // Add some blocks in each epoch
        let mut height = 0u64;

        // Epoch 0: producer1 at slots 5, 10, 15
        for slot in [5u32, 10, 15] {
            store
                .put_block_canonical(&create_test_block(slot, &producer1), height)
                .unwrap();
            height += 1;
        }

        // Epoch 1: producer2 at slots 35, 40, 45
        for slot in [35u32, 40, 45] {
            store
                .put_block_canonical(&create_test_block(slot, &producer2), height)
                .unwrap();
            height += 1;
        }

        // Epoch 2: both producers at slots 65, 70, 75, 80
        for (i, slot) in [65u32, 70, 75, 80].iter().enumerate() {
            let producer = if i % 2 == 0 { &producer1 } else { &producer2 };
            store
                .put_block_canonical(&create_test_block(*slot, producer), height)
                .unwrap();
            height += 1;
        }

        // Query epoch 0 range (1-30)
        let epoch0_blocks = store.get_blocks_in_slot_range(1, 30).unwrap();
        assert_eq!(epoch0_blocks.len(), 3, "Epoch 0 should have 3 blocks");
        for block in &epoch0_blocks {
            assert!(block.slot() < 30, "All blocks should be in epoch 0");
            assert_eq!(block.header.producer, producer1);
        }

        // Query epoch 1 range (30-60)
        let epoch1_blocks = store.get_blocks_in_slot_range(30, 60).unwrap();
        assert_eq!(epoch1_blocks.len(), 3, "Epoch 1 should have 3 blocks");
        for block in &epoch1_blocks {
            assert!(block.slot() >= 30 && block.slot() < 60);
            assert_eq!(block.header.producer, producer2);
        }

        // Query epoch 2 range (60-90)
        let epoch2_blocks = store.get_blocks_in_slot_range(60, 90).unwrap();
        assert_eq!(epoch2_blocks.len(), 4, "Epoch 2 should have 4 blocks");

        // Query cross-epoch range (20-70)
        // Should include: epoch 1 blocks (35, 40, 45) + epoch 2 block at 65
        // 70 is excluded (exclusive end), so 4 blocks total
        let cross_epoch = store.get_blocks_in_slot_range(20, 70).unwrap();
        assert_eq!(
            cross_epoch.len(),
            4,
            "Cross-epoch query should return 4 blocks"
        );
        // Verify sorted by slot
        for i in 1..cross_epoch.len() {
            assert!(
                cross_epoch[i - 1].slot() < cross_epoch[i].slot(),
                "Blocks should be sorted by slot"
            );
        }
    }

    #[test]
    fn test_get_block_by_slot_out_of_range() {
        // Test querying slots before first block and after last block
        let (store, _dir) = create_test_store();
        let keypair = KeyPair::generate();
        let producer = keypair.public_key().clone();

        // Store a single block at slot 100
        let block = create_test_block(100, &producer);
        store.put_block_canonical(&block, 0).unwrap();

        // Query before first block
        assert!(store.get_block_by_slot(0).unwrap().is_none());
        assert!(store.get_block_by_slot(50).unwrap().is_none());
        assert!(store.get_block_by_slot(99).unwrap().is_none());

        // Query exact slot
        assert!(store.get_block_by_slot(100).unwrap().is_some());

        // Query after last block
        assert!(store.get_block_by_slot(101).unwrap().is_none());
        assert!(store.get_block_by_slot(1000).unwrap().is_none());
        assert!(store.get_block_by_slot(u32::MAX).unwrap().is_none());
    }

    #[test]
    fn test_get_last_rewarded_epoch_empty_chain() {
        // Test that empty chain returns 0
        let (store, _dir) = create_test_store();

        let last_epoch = store.get_last_rewarded_epoch().unwrap();
        assert_eq!(last_epoch, 0, "Empty chain should return epoch 0");
    }

    #[test]
    fn test_clear_removes_all_data() {
        // Regression test: force_resync_from_genesis must clear the block store
        // to purge stale fork blocks. Without clear(), old HEIGHT_INDEX and
        // SLOT_INDEX entries persist and pollute queries.
        let (store, _dir) = create_test_store();
        let keypair = KeyPair::generate();
        let producer = keypair.public_key().clone();

        // Populate store with blocks (simulating a fork)
        for height in 0..10u64 {
            let block = create_test_block(height as u32 + 1, &producer);
            store.put_block_canonical(&block, height).unwrap();
        }

        // Verify data exists
        assert!(store.get_block_by_height(0).unwrap().is_some());
        assert!(store.get_block_by_height(9).unwrap().is_some());
        assert!(store.get_block_by_slot(1).unwrap().is_some());
        assert!(store.get_block_by_slot(10).unwrap().is_some());
        assert_eq!(store.count_slot_index_entries().unwrap(), 10);

        // Clear the store
        store.clear().unwrap();

        // Verify all data is gone
        for h in 0..10u64 {
            assert!(
                store.get_block_by_height(h).unwrap().is_none(),
                "Height {} should be empty after clear",
                h
            );
        }
        for s in 1..=10u32 {
            assert!(
                store.get_block_by_slot(s).unwrap().is_none(),
                "Slot {} should be empty after clear",
                s
            );
        }
        assert_eq!(store.count_slot_index_entries().unwrap(), 0);
        assert_eq!(store.get_last_rewarded_epoch().unwrap(), 0);
    }

    #[test]
    fn test_clear_then_repopulate() {
        // After clear(), new blocks should be stored correctly without
        // interference from previously cleared data.
        let (store, _dir) = create_test_store();
        let keypair = KeyPair::generate();
        let producer = keypair.public_key().clone();

        // Store "fork" blocks at heights 0-4, slots 100-104
        for i in 0..5u64 {
            let block = create_test_block(100 + i as u32, &producer);
            store.put_block_canonical(&block, i).unwrap();
        }

        // Clear (simulating resync)
        store.clear().unwrap();

        // Store "canonical" blocks at heights 0-2, slots 1-3
        for i in 0..3u64 {
            let block = create_test_block(i as u32 + 1, &producer);
            store.put_block_canonical(&block, i).unwrap();
        }

        // Canonical blocks exist
        assert!(store.get_block_by_height(0).unwrap().is_some());
        assert!(store.get_block_by_height(2).unwrap().is_some());

        // Old fork heights should NOT exist
        assert!(
            store.get_block_by_height(3).unwrap().is_none(),
            "Old fork height 3 should be gone after clear"
        );
        assert!(
            store.get_block_by_height(4).unwrap().is_none(),
            "Old fork height 4 should be gone after clear"
        );

        // Old fork slots should NOT exist
        assert!(
            store.get_block_by_slot(100).unwrap().is_none(),
            "Old fork slot 100 should be gone after clear"
        );

        // Only 3 entries in slot index
        assert_eq!(store.count_slot_index_entries().unwrap(), 3);
    }

    #[test]
    fn test_set_canonical_chain_reorg() {
        // Insert chain A→B→C, then fork A→B'→C'→D' (longer).
        // After set_canonical_chain(D'), height_index must point to B', C', D'.
        let (store, _dir) = create_test_store();
        let keypair = KeyPair::generate();
        let producer = keypair.public_key().clone();

        // Block A (genesis) at height 0
        let block_a = create_test_block(1, &producer);
        let hash_a = block_a.hash();
        store.put_block(&block_a, 0).unwrap();
        store.set_canonical_chain(hash_a, 0).unwrap();

        // Block B (child of A) at height 1
        let mut header_b = create_test_header(2, &producer);
        header_b.prev_hash = hash_a;
        let block_b = Block::new(header_b, vec![]);
        let hash_b = block_b.hash();
        store.put_block(&block_b, 1).unwrap();
        store.set_canonical_chain(hash_b, 1).unwrap();

        // Block C (child of B) at height 2
        let mut header_c = create_test_header(3, &producer);
        header_c.prev_hash = hash_b;
        let block_c = Block::new(header_c, vec![]);
        let hash_c = block_c.hash();
        store.put_block(&block_c, 2).unwrap();
        store.set_canonical_chain(hash_c, 2).unwrap();

        // Verify canonical chain: A→B→C
        assert_eq!(store.get_hash_by_height(0).unwrap(), Some(hash_a));
        assert_eq!(store.get_hash_by_height(1).unwrap(), Some(hash_b));
        assert_eq!(store.get_hash_by_height(2).unwrap(), Some(hash_c));

        // Fork: B' (child of A, different block) at height 1
        let mut header_b2 = create_test_header(4, &producer); // Different slot → different hash
        header_b2.prev_hash = hash_a;
        let block_b2 = Block::new(header_b2, vec![]);
        let hash_b2 = block_b2.hash();
        store.put_block(&block_b2, 1).unwrap(); // Store but DON'T set canonical yet

        // C' (child of B') at height 2
        let mut header_c2 = create_test_header(5, &producer);
        header_c2.prev_hash = hash_b2;
        let block_c2 = Block::new(header_c2, vec![]);
        let hash_c2 = block_c2.hash();
        store.put_block(&block_c2, 2).unwrap();

        // D' (child of C') at height 3 — fork is now longer
        let mut header_d2 = create_test_header(6, &producer);
        header_d2.prev_hash = hash_c2;
        let block_d2 = Block::new(header_d2, vec![]);
        let hash_d2 = block_d2.hash();
        store.put_block(&block_d2, 3).unwrap();

        // Reorg: set canonical chain to the fork (longer chain)
        store.set_canonical_chain(hash_d2, 3).unwrap();

        // Verify: height_index now points to fork blocks
        assert_eq!(
            store.get_hash_by_height(0).unwrap(),
            Some(hash_a),
            "Height 0 unchanged (common ancestor)"
        );
        assert_eq!(
            store.get_hash_by_height(1).unwrap(),
            Some(hash_b2),
            "Height 1 should point to B' after reorg"
        );
        assert_eq!(
            store.get_hash_by_height(2).unwrap(),
            Some(hash_c2),
            "Height 2 should point to C' after reorg"
        );
        assert_eq!(
            store.get_hash_by_height(3).unwrap(),
            Some(hash_d2),
            "Height 3 should point to D'"
        );

        // Old blocks still accessible by hash (stored but not canonical)
        assert!(store.get_block(&hash_b).unwrap().is_some());
        assert!(store.get_block(&hash_c).unwrap().is_some());

        // hash_to_height reflects canonical chain
        assert_eq!(store.get_height_by_hash(&hash_b2).unwrap(), Some(1));
        assert_eq!(store.get_height_by_hash(&hash_d2).unwrap(), Some(3));
    }
}
