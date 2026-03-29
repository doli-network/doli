//! Block store maintenance operations (cleanup, clearing, deletion)

use crypto::Hash;
use tracing::{debug, info, warn};

use crate::StorageError;

use super::types::{
    BlockStore, CF_ADDR_TX_INDEX, CF_BODIES, CF_HASH_TO_HEIGHT, CF_HEADERS, CF_HEIGHT_INDEX,
    CF_PRESENCE, CF_SLOT_INDEX, CF_TX_INDEX,
};

impl BlockStore {
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

    /// Clear only index column families (height_index, slot_index, hash_to_height).
    ///
    /// Preserves block data (headers, bodies, presence) so blocks remain available
    /// for future rollbacks/rebuilds. Indexes are rebuilt by `set_canonical_chain()`
    /// when sync re-applies blocks.
    ///
    /// Used by `reset_state_only()` during automatic recovery — avoids the costly
    /// re-download of block data while ensuring stale index entries (from fork blocks)
    /// don't pollute queries like `get_last_rewarded_epoch()`.
    pub fn clear_indexes(&self) -> Result<(), StorageError> {
        let index_cfs = [CF_HEIGHT_INDEX, CF_SLOT_INDEX, CF_HASH_TO_HEIGHT];

        for cf_name in &index_cfs {
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
                info!(
                    "[BLOCK_STORE] Cleared {} index entries from CF '{}'",
                    count, cf_name
                );
            }
        }

        info!("[BLOCK_STORE] Index column families cleared (block data preserved)");
        Ok(())
    }

    /// Clear all block data from the store.
    ///
    /// Removes all entries from every column family (headers, bodies,
    /// height_index, slot_index, presence). Used only by manual CLI
    /// `recover --yes` as a last resort.
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

    /// Delete all blocks above `keep_height` from all column families.
    /// Returns the number of blocks deleted.
    pub fn delete_blocks_above(&self, keep_height: u64) -> Result<u64, StorageError> {
        let cf_height = self.db.cf_handle(CF_HEIGHT_INDEX).unwrap();
        let cf_headers = self.db.cf_handle(CF_HEADERS).unwrap();
        let cf_bodies = self.db.cf_handle(CF_BODIES).unwrap();
        let cf_h2h = self.db.cf_handle(CF_HASH_TO_HEIGHT).unwrap();
        let cf_slot = self.db.cf_handle(CF_SLOT_INDEX).unwrap();
        let cf_tx = self.db.cf_handle(CF_TX_INDEX).unwrap();
        let cf_addr = self.db.cf_handle(CF_ADDR_TX_INDEX).unwrap();

        let mut deleted = 0u64;
        let mut height = keep_height + 1;

        while let Some(hash_bytes) = self.db.get_cf(cf_height, height.to_le_bytes())? {
            // Load the block to get tx hashes and slot for index cleanup
            if let Some(block) = self.get_block_by_height(height)? {
                let mut batch = rocksdb::WriteBatch::default();

                // Delete from height_index
                batch.delete_cf(cf_height, height.to_le_bytes());
                // Delete from hash_to_height
                batch.delete_cf(cf_h2h, &hash_bytes);
                // Delete header and body
                batch.delete_cf(cf_headers, &hash_bytes);
                batch.delete_cf(cf_bodies, &hash_bytes);
                // Delete slot index
                batch.delete_cf(cf_slot, block.header.slot.to_le_bytes());
                // Delete tx and address indexes
                for tx in &block.transactions {
                    let tx_hash = tx.hash();
                    batch.delete_cf(cf_tx, tx_hash.as_bytes());
                    for output in &tx.outputs {
                        let mut key = [0u8; 40];
                        key[..32].copy_from_slice(output.pubkey_hash.as_bytes());
                        key[32..].copy_from_slice(&height.to_be_bytes());
                        batch.delete_cf(cf_addr, key);
                    }
                }

                self.db.write(batch)?;
                deleted += 1;
            } else {
                // Height index exists but block data doesn't — clean up orphan index
                let mut batch = rocksdb::WriteBatch::default();
                batch.delete_cf(cf_height, height.to_le_bytes());
                batch.delete_cf(cf_h2h, &hash_bytes);
                self.db.write(batch)?;
                deleted += 1;
            }

            height += 1;
        }

        // Compact the affected ranges to reclaim disk space
        self.db.compact_range_cf(
            cf_height,
            Some(&(keep_height + 1).to_le_bytes()[..]),
            None::<&[u8]>,
        );

        Ok(deleted)
    }

    /// Minimum number of recent blocks that must always be retained.
    /// Set to 2x MAX_REORG_DEPTH for safety margin (matches UNDO_KEEP_DEPTH).
    const MIN_RETENTION: u64 = 2000;

    /// Delete all blocks below `keep_above_height` from all column families.
    ///
    /// Prunes old block history to reclaim disk space. Keeps blocks at and above
    /// `keep_above_height`. Enforces a minimum retention of 2000 blocks from the
    /// chain tip to preserve rollback safety.
    ///
    /// Returns `(deleted_count, lowest_remaining_height)`.
    pub fn prune_blocks_below(
        &self,
        keep_above_height: u64,
        chain_tip: u64,
    ) -> Result<(u64, u64), StorageError> {
        // Safety: never prune within MIN_RETENTION of the tip
        let safe_floor = chain_tip.saturating_sub(Self::MIN_RETENTION);
        let effective_cutoff = keep_above_height.min(safe_floor);

        if effective_cutoff <= 1 {
            info!(
                "[PRUNE] Nothing to prune (cutoff={}, tip={})",
                effective_cutoff, chain_tip
            );
            return Ok((0, 1));
        }

        let cf_height = self.db.cf_handle(CF_HEIGHT_INDEX).unwrap();
        let cf_headers = self.db.cf_handle(CF_HEADERS).unwrap();
        let cf_bodies = self.db.cf_handle(CF_BODIES).unwrap();
        let cf_h2h = self.db.cf_handle(CF_HASH_TO_HEIGHT).unwrap();
        let cf_slot = self.db.cf_handle(CF_SLOT_INDEX).unwrap();
        let cf_tx = self.db.cf_handle(CF_TX_INDEX).unwrap();
        let cf_addr = self.db.cf_handle(CF_ADDR_TX_INDEX).unwrap();

        let mut deleted = 0u64;
        let mut batch = rocksdb::WriteBatch::default();
        let batch_size = 500; // Write in batches to limit memory

        // Prune heights 1 through effective_cutoff-1 (keep genesis=0 and cutoff+)
        for height in 1..effective_cutoff {
            let hash_bytes = match self.db.get_cf(cf_height, height.to_le_bytes())? {
                Some(b) => b,
                None => continue, // Already missing (snap sync gap, etc.)
            };

            // Load block to get tx hashes and slot for index cleanup
            if let Some(block) = self.get_block_by_height(height)? {
                batch.delete_cf(cf_height, height.to_le_bytes());
                batch.delete_cf(cf_h2h, &hash_bytes);
                batch.delete_cf(cf_headers, &hash_bytes);
                batch.delete_cf(cf_bodies, &hash_bytes);
                batch.delete_cf(cf_slot, block.header.slot.to_le_bytes());

                for tx in &block.transactions {
                    let tx_hash = tx.hash();
                    batch.delete_cf(cf_tx, tx_hash.as_bytes());
                    for output in &tx.outputs {
                        let mut key = [0u8; 40];
                        key[..32].copy_from_slice(output.pubkey_hash.as_bytes());
                        key[32..].copy_from_slice(&height.to_be_bytes());
                        batch.delete_cf(cf_addr, key);
                    }
                }
            } else {
                // Height index exists but block data doesn't — clean orphan index
                batch.delete_cf(cf_height, height.to_le_bytes());
                batch.delete_cf(cf_h2h, &hash_bytes);
            }

            deleted += 1;

            // Flush batch periodically
            if deleted.is_multiple_of(batch_size) {
                self.db.write(std::mem::take(&mut batch))?;
                if deleted.is_multiple_of(5000) {
                    info!(
                        "[PRUNE] Progress: {} blocks pruned (height {}/{})",
                        deleted, height, effective_cutoff
                    );
                }
            }
        }

        // Write remaining batch
        if !batch.is_empty() {
            self.db.write(batch)?;
        }

        // Compact all affected column families to reclaim disk space
        let low = 1u64.to_le_bytes();
        let high = effective_cutoff.to_le_bytes();
        self.db
            .compact_range_cf(cf_height, Some(&low[..]), Some(&high[..]));
        self.db
            .compact_range_cf(cf_headers, None::<&[u8]>, None::<&[u8]>);
        self.db
            .compact_range_cf(cf_bodies, None::<&[u8]>, None::<&[u8]>);
        self.db
            .compact_range_cf(cf_slot, None::<&[u8]>, None::<&[u8]>);
        self.db
            .compact_range_cf(cf_tx, None::<&[u8]>, None::<&[u8]>);
        self.db
            .compact_range_cf(cf_addr, None::<&[u8]>, None::<&[u8]>);

        if deleted > 0 {
            info!(
                "[PRUNE] Pruned {} blocks below height {} (tip={}, retained={}+)",
                deleted, effective_cutoff, chain_tip, effective_cutoff
            );
        }

        Ok((deleted, effective_cutoff))
    }

    /// Count entries in each column family for storage diagnostics.
    ///
    /// Returns a map of CF name → entry count.
    pub fn storage_stats(&self) -> Result<Vec<(&'static str, u64)>, StorageError> {
        let cfs: &[&str] = &[
            CF_HEADERS,
            CF_BODIES,
            CF_HEIGHT_INDEX,
            CF_SLOT_INDEX,
            CF_HASH_TO_HEIGHT,
            CF_TX_INDEX,
            CF_ADDR_TX_INDEX,
        ];

        let mut stats = Vec::with_capacity(cfs.len());
        for cf_name in cfs {
            let cf = self.db.cf_handle(cf_name).unwrap();
            let count = self
                .db
                .iterator_cf(cf, rocksdb::IteratorMode::Start)
                .flatten()
                .count() as u64;
            stats.push((*cf_name, count));
        }

        Ok(stats)
    }

    /// Get the lowest and highest heights in the height index.
    ///
    /// Returns `(min_height, max_height)` or `None` if the index is empty.
    pub fn height_range(&self) -> Result<Option<(u64, u64)>, StorageError> {
        let cf_height = self.db.cf_handle(CF_HEIGHT_INDEX).unwrap();

        // First entry (lowest height)
        let min = self
            .db
            .iterator_cf(cf_height, rocksdb::IteratorMode::Start)
            .flatten()
            .next();

        let max = self
            .db
            .iterator_cf(cf_height, rocksdb::IteratorMode::End)
            .flatten()
            .next();

        match (min, max) {
            (Some((min_key, _)), Some((max_key, _))) => {
                let min_h = if min_key.len() == 8 {
                    let mut arr = [0u8; 8];
                    arr.copy_from_slice(&min_key);
                    u64::from_le_bytes(arr)
                } else {
                    return Ok(None);
                };
                let max_h = if max_key.len() == 8 {
                    let mut arr = [0u8; 8];
                    arr.copy_from_slice(&max_key);
                    u64::from_le_bytes(arr)
                } else {
                    return Ok(None);
                };
                Ok(Some((min_h, max_h)))
            }
            _ => Ok(None),
        }
    }
}
