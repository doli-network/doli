//! Block write operations and canonical chain management

use crypto::Hash;
use doli_core::{Block, BlockHeader};
use tracing::info;

use crate::StorageError;

use super::types::{
    BlockBody, BlockStore, CF_ADDR_TX_INDEX, CF_BODIES, CF_HASH_TO_HEIGHT, CF_HEADERS,
    CF_HEIGHT_INDEX, CF_SLOT_INDEX, CF_TX_INDEX,
};

impl BlockStore {
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

        // Store body (keyed by hash) — includes BLS aggregate signature + attestation bitfield
        let body = BlockBody {
            transactions: block.transactions.clone(),
            aggregate_bls_signature: block.aggregate_bls_signature.clone(),
            attestation_bitfield: block.attestation_bitfield.clone(),
        };
        let body_bytes =
            bincode::serialize(&body).map_err(|e| StorageError::Serialization(e.to_string()))?;
        let cf_bodies = self.db.cf_handle(CF_BODIES).unwrap();
        self.db.put_cf(cf_bodies, hash_bytes, &body_bytes)?;

        // Update slot index
        let cf_slot = self.db.cf_handle(CF_SLOT_INDEX).unwrap();
        self.db.put_cf(cf_slot, slot.to_le_bytes(), hash_bytes)?;

        // Update tx index: tx_hash → block height
        let cf_tx = self.db.cf_handle(CF_TX_INDEX).unwrap();
        let cf_addr = self.db.cf_handle(CF_ADDR_TX_INDEX).unwrap();
        let mut indexed_addrs = std::collections::HashSet::new();
        for tx in &block.transactions {
            let tx_hash = tx.hash();
            self.db
                .put_cf(cf_tx, tx_hash.as_bytes(), height.to_le_bytes())?;

            // Index output addresses: addr(32) || height(8 BE) → empty
            for output in &tx.outputs {
                let addr_bytes = output.pubkey_hash.as_bytes();
                if indexed_addrs.insert(*addr_bytes) {
                    let mut key = [0u8; 40];
                    key[..32].copy_from_slice(addr_bytes);
                    key[32..].copy_from_slice(&height.to_be_bytes());
                    self.db.put_cf(cf_addr, key, [])?;
                }
            }
            indexed_addrs.clear();
        }

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
                StorageError::NotFound(format!(
                    "[STOR020] header {} missing during chain walk",
                    current_hash
                ))
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

    /// Seed the canonical chain index with a single anchor entry.
    ///
    /// Called after snap sync to establish the invariant that `height_index`
    /// contains an entry for the snap-synced tip. Without this, the very first
    /// `set_canonical_chain` call after snap sync walks backwards into an empty
    /// store and crashes with "header N missing".
    ///
    /// Two writes: `height_index[height] = hash` and `hash_to_height[hash] = height`.
    /// This is enough for `set_canonical_chain` to exit early at the snap height.
    pub fn seed_canonical_index(&self, hash: Hash, height: u64) -> Result<(), StorageError> {
        let cf_height = self.db.cf_handle(CF_HEIGHT_INDEX).unwrap();
        let cf_h2h = self.db.cf_handle(CF_HASH_TO_HEIGHT).unwrap();
        let mut batch = rocksdb::WriteBatch::default();
        batch.put_cf(cf_height, height.to_le_bytes(), hash.as_bytes());
        batch.put_cf(cf_h2h, hash.as_bytes(), height.to_le_bytes());
        self.db.write(batch)?;
        info!(
            "[BLOCK_STORE] Snap sync anchor seeded: height={}, hash={:.16}",
            height, hash
        );
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
                StorageError::NotFound(format!(
                    "[STOR021] header {} missing during reindex",
                    current
                ))
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
}
