//! Block store query methods

use crypto::Hash;
use doli_core::{Block, BlockHeader};
use tracing::debug;

use crate::StorageError;

use super::types::{
    deserialize_body, BlockStore, CF_ADDR_TX_INDEX, CF_BODIES, CF_HASH_TO_HEIGHT, CF_HEADERS,
    CF_HEIGHT_INDEX, CF_SLOT_INDEX, CF_TX_INDEX,
};

impl BlockStore {
    /// Create a RocksDB checkpoint (point-in-time snapshot) at the given path.
    ///
    /// Uses hard links — near-instant, near-zero extra disk space.
    pub fn create_checkpoint(&self, path: &std::path::Path) -> Result<(), StorageError> {
        let checkpoint = rocksdb::checkpoint::Checkpoint::new(&self.db)?;
        checkpoint
            .create_checkpoint(path)
            .map_err(StorageError::from)
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
        let (transactions, bls_sig) = deserialize_body(&body_bytes)?;

        let mut block = Block::new(header, transactions);
        block.aggregate_bls_signature = bls_sig;
        Ok(Some(block))
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
            return Err(StorageError::Serialization(format!(
                "[STOR022] invalid height length: expected 8 bytes, got {}",
                bytes.len()
            )));
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
            return Err(StorageError::Serialization(format!(
                "[STOR023] invalid hash length: expected 32 bytes, got {}",
                bytes.len()
            )));
        }

        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Some(Hash::from_bytes(arr)))
    }

    /// Look up which block height contains a given transaction hash.
    pub fn get_tx_block_height(&self, tx_hash: &Hash) -> Result<Option<u64>, StorageError> {
        let cf_tx = self.db.cf_handle(CF_TX_INDEX).unwrap();
        let bytes = match self.db.get_cf(cf_tx, tx_hash.as_bytes())? {
            Some(b) => b,
            None => return Ok(None),
        };
        if bytes.len() != 8 {
            return Err(StorageError::Serialization(format!(
                "[STOR024] invalid tx_index height length: expected 8 bytes, got {}",
                bytes.len()
            )));
        }
        let mut arr = [0u8; 8];
        arr.copy_from_slice(&bytes);
        Ok(Some(u64::from_le_bytes(arr)))
    }

    /// Get block heights containing transactions for a given address.
    /// Returns heights in descending order, starting before `before_height` (exclusive).
    pub fn get_address_heights(
        &self,
        pubkey_hash: &Hash,
        before_height: Option<u64>,
        limit: usize,
    ) -> Result<Vec<u64>, StorageError> {
        let cf_addr = self.db.cf_handle(CF_ADDR_TX_INDEX).unwrap();
        let prefix = pubkey_hash.as_bytes();

        // Build start key: addr || start_height (big-endian for ordered iteration)
        let start_height = before_height.unwrap_or(u64::MAX);
        let mut start_key = [0u8; 40];
        start_key[..32].copy_from_slice(prefix);
        start_key[32..].copy_from_slice(&start_height.to_be_bytes());

        let mut heights = Vec::with_capacity(limit);
        let iter = self.db.iterator_cf(
            cf_addr,
            rocksdb::IteratorMode::From(&start_key, rocksdb::Direction::Reverse),
        );

        for item in iter.flatten() {
            let (key, _) = item;
            if key.len() != 40 || &key[..32] != prefix {
                break; // Past this address's entries
            }
            let mut h_bytes = [0u8; 8];
            h_bytes.copy_from_slice(&key[32..40]);
            let h = u64::from_be_bytes(h_bytes);

            // Skip the before_height itself (exclusive)
            if let Some(bh) = before_height {
                if h >= bh {
                    continue;
                }
            }

            heights.push(h);
            if heights.len() >= limit {
                break;
            }
        }

        Ok(heights)
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
            return Err(StorageError::Serialization(format!(
                "[STOR025] invalid slot hash length: expected 32 bytes, got {}",
                bytes.len()
            )));
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
        let Ok(slot_u32) = u32::try_from(slot) else {
            return false; // slot > u32::MAX cannot exist
        };
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
