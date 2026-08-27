//! BlockBatch implementation — atomic write batch for block application

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;

use crypto::Hash;
use doli_core::types::{Amount, BlockHeight};
use doli_core::validation::{UtxoInfo, UtxoProvider};

use crate::chain_state::ChainState;
use crate::producer::{PendingProducerUpdate, ProducerInfo, ProducerSet};
use crate::utxo::{
    uid_key, Outpoint, UtxoEntry, UID_PREFIX_ASSET, UID_PREFIX_NFT, UID_PREFIX_POOL,
};
use crate::StorageError;

use super::types::{
    BlockBatch, LastApplied, MaintainerUndoSnapshot, StateDb, UndoData, CF_EXIT_HISTORY, CF_META,
    CF_PRODUCERS, CF_UNDO, CF_UNIQUE_ID, CF_UTXO, CF_UTXO_BY_PUBKEY, META_ACTIVE_PRODUCTION_LIST,
    META_CHAIN_COMMITMENT, META_CHAIN_STATE, META_EPOCH_ATTESTATION_ACCUM, META_EPOCH_ATTESTED_SET,
    META_EPOCH_BLOCKS_PRODUCED, META_EPOCH_BOND_SNAPSHOT, META_EPOCH_PRODUCER_LIST,
    META_EPOCH_STATE, META_EPOCH_STATE_VERSION, META_LAST_APPLIED, META_PENDING_UPDATES,
};
use super::undo::MAINTAINER_UNDO_KEY_PREFIX;

// ==================== Batch Creation ====================

impl StateDb {
    /// Create a new BlockBatch for atomic writes.
    pub fn begin_batch(&self) -> BlockBatch<'_> {
        BlockBatch {
            db: self,
            batch: rocksdb::WriteBatch::default(),
            utxo_delta: 0,
            pending_utxos: HashMap::new(),
            spent_in_batch: Vec::new(),
            pending_unique_ids: HashSet::new(),
            removed_unique_ids: HashSet::new(),
        }
    }
}

// ==================== BlockBatch Implementation ====================

impl<'a> BlockBatch<'a> {
    /// Add a UTXO to the batch.
    pub fn add_utxo(&mut self, outpoint: Outpoint, entry: UtxoEntry) {
        let cf_utxo = self.db.db.cf_handle(CF_UTXO).unwrap();
        let cf_by_pk = self.db.db.cf_handle(CF_UTXO_BY_PUBKEY).unwrap();

        let key = outpoint.to_bytes();
        let value = bincode::serialize(&entry).expect("UtxoEntry serialization");

        self.batch.put_cf(cf_utxo, &key, &value);

        let mut idx_key = Vec::with_capacity(68);
        idx_key.extend_from_slice(entry.output.pubkey_hash.as_bytes());
        idx_key.extend_from_slice(&key);
        self.batch.put_cf(cf_by_pk, &idx_key, [0u8]);

        // Track in pending for same-block-spend
        self.pending_utxos.insert(outpoint, entry);
        self.utxo_delta += 1;
    }

    /// Spend a UTXO in the batch. Checks pending_utxos first (same-block-spend),
    /// then falls back to the committed DB.
    ///
    /// Phase 3: also removes unique IDs (NFT/Pool/FungibleAsset) from the
    /// pending set and tracks them in `removed_unique_ids` so
    /// `has_unique_id_check` returns false for spent outputs.
    pub fn spend_utxo(&mut self, outpoint: &Outpoint) -> Result<UtxoEntry, StorageError> {
        let cf_utxo = self.db.db.cf_handle(CF_UTXO).unwrap();
        let cf_by_pk = self.db.db.cf_handle(CF_UTXO_BY_PUBKEY).unwrap();

        // Check pending first (same-block-spend)
        let entry = if let Some(entry) = self.pending_utxos.remove(outpoint) {
            entry
        } else {
            // Fall back to committed DB
            let key = outpoint.to_bytes();
            let entry_bytes = self.db.db.get_cf(cf_utxo, &key)?.ok_or_else(|| {
                StorageError::NotFound(format!(
                    "[STOR018] UTXO not found in batch: {}:{}",
                    outpoint.tx_hash, outpoint.index
                ))
            })?;
            bincode::deserialize(&entry_bytes).map_err(|e| {
                StorageError::Serialization(format!(
                    "[STOR019] UTXO deserialize failed in batch for {}:{}: {}",
                    outpoint.tx_hash, outpoint.index, e
                ))
            })?
        };

        let key = outpoint.to_bytes();
        self.batch.delete_cf(cf_utxo, &key);

        let mut idx_key = Vec::with_capacity(68);
        idx_key.extend_from_slice(entry.output.pubkey_hash.as_bytes());
        idx_key.extend_from_slice(&key);
        self.batch.delete_cf(cf_by_pk, &idx_key);

        // Phase 3: remove unique IDs for spent NFT/Pool/FungibleAsset outputs
        self.remove_unique_id_for_entry(&entry);

        self.spent_in_batch.push(*outpoint);
        self.utxo_delta -= 1;

        Ok(entry)
    }

    /// Spend all inputs of a transaction via the batch.
    ///
    /// Mirrors `UtxoSet::spend_transaction` but accumulates in the WriteBatch.
    /// Returns total input amount. Skips if no inputs (coinbase).
    pub fn spend_transaction_utxos(
        &mut self,
        tx: &doli_core::transaction::Transaction,
    ) -> Result<Amount, StorageError> {
        let mut total: Amount = 0;
        for input in &tx.inputs {
            let outpoint = Outpoint::new(input.prev_tx_hash, input.output_index);
            let entry = self.spend_utxo(&outpoint)?;
            if entry.output.output_type.is_native_amount() {
                total += entry.output.amount;
            }
        }
        Ok(total)
    }

    /// Add all outputs of a transaction via the batch.
    ///
    /// Mirrors `UtxoSet::add_transaction` but accumulates in the WriteBatch.
    /// Phase 3: also inserts unique IDs for NFT/Pool/FungibleAsset outputs.
    pub fn add_transaction_utxos(
        &mut self,
        tx: &doli_core::transaction::Transaction,
        height: BlockHeight,
        is_coinbase: bool,
        slot: u32,
    ) {
        let tx_hash = tx.hash();
        let is_epoch_reward = tx.is_epoch_reward();
        for (index, output) in tx.outputs.iter().enumerate() {
            let outpoint = Outpoint::new(tx_hash, index as u32);
            // Stamp Bond outputs with the block's slot as creation_slot
            let mut stamped_output = output.clone();
            if stamped_output.output_type == doli_core::OutputType::Bond {
                stamped_output.extra_data = slot.to_le_bytes().to_vec();
            }
            // Stamp Pool outputs: creation_slot, last_update_slot, TWAP accumulation
            // (BUG-001 fix: mirror utxo_rocks::add_transaction:237-268 so both
            // write paths produce byte-identical bytes for Pool UTXOs)
            if stamped_output.output_type == doli_core::OutputType::Pool {
                if let Some(mut meta) = stamped_output.pool_metadata() {
                    if meta.creation_slot == 0 {
                        meta.creation_slot = slot;
                    }
                    // Accumulate TWAP BEFORE updating last_update_slot
                    if meta.last_update_slot > 0
                        && slot > meta.last_update_slot
                        && meta.reserve_b > 0
                    {
                        meta.cumulative_price = doli_core::update_twap(
                            meta.cumulative_price,
                            meta.reserve_a,
                            meta.reserve_b,
                            slot,
                            meta.last_update_slot,
                        );
                    }
                    meta.last_update_slot = slot;
                    stamped_output = doli_core::transaction::Output::pool(
                        meta.pool_id,
                        meta.asset_b_id,
                        meta.reserve_a,
                        meta.reserve_b,
                        meta.total_lp_shares,
                        meta.cumulative_price,
                        meta.last_update_slot,
                        meta.fee_bps,
                        meta.creation_slot,
                    );
                }
            }
            let entry = UtxoEntry {
                output: stamped_output,
                height,
                is_coinbase,
                is_epoch_reward,
            };

            // Phase 3: insert unique IDs for NFT/Pool/FungibleAsset outputs
            self.add_unique_id_for_entry(&entry);

            self.add_utxo(outpoint, entry);
        }
    }

    // ==================== Unique ID (Phase 1 + Phase 3) ====================

    /// Add a unique ID to the pending set for same-block uniqueness checks.
    ///
    /// The ID is also queued in the WriteBatch for persistence on commit.
    /// Phase 1 of UTXO storage consolidation (specs/utxo-storage-architecture.md).
    pub fn add_pending_unique_id(&mut self, prefix: u8, id: Hash) {
        self.pending_unique_ids.insert((prefix, *id.as_bytes()));
        // If this was previously removed in the same block, un-remove it
        self.removed_unique_ids.remove(&(prefix, *id.as_bytes()));
        let cf = self.db.db.cf_handle(CF_UNIQUE_ID).unwrap();
        self.batch.put_cf(cf, uid_key(prefix, &id), [0u8]);
    }

    /// Check if a unique ID exists: first in the pending set (same-block),
    /// then on disk (cf_unique_id), respecting removals in this batch.
    ///
    /// Phase 3: checks `removed_unique_ids` before falling through to disk
    /// to ensure IDs spent in the current block are not visible.
    pub fn has_unique_id_check(&self, prefix: u8, id: &Hash) -> bool {
        let key = (prefix, *id.as_bytes());
        // Check pending adds first (same-block insert)
        if self.pending_unique_ids.contains(&key) {
            return true;
        }
        // Check if removed in this batch — overrides disk
        if self.removed_unique_ids.contains(&key) {
            return false;
        }
        // Fall back to disk
        self.db.has_unique_id(prefix, id)
    }

    /// Remove a unique ID from the batch (queued for deletion on commit).
    /// Phase 3: also tracks in `removed_unique_ids` so `has_unique_id_check`
    /// returns false even if the ID is still on disk.
    pub fn remove_pending_unique_id(&mut self, prefix: u8, id: &Hash) {
        let key = (prefix, *id.as_bytes());
        self.pending_unique_ids.remove(&key);
        self.removed_unique_ids.insert(key);
        let cf = self.db.db.cf_handle(CF_UNIQUE_ID).unwrap();
        self.batch.delete_cf(cf, uid_key(prefix, id));
    }

    /// Phase 3 helper: extract and insert unique IDs from a UtxoEntry's output.
    fn add_unique_id_for_entry(&mut self, entry: &UtxoEntry) {
        match entry.output.output_type {
            doli_core::OutputType::NFT => {
                if let Some((token_id, _)) = entry.output.nft_metadata() {
                    self.add_pending_unique_id(UID_PREFIX_NFT, token_id);
                }
            }
            doli_core::OutputType::Pool => {
                if let Some(meta) = entry.output.pool_metadata() {
                    self.add_pending_unique_id(UID_PREFIX_POOL, meta.pool_id);
                }
            }
            doli_core::OutputType::FungibleAsset => {
                if let Some((asset_id, _, _)) = entry.output.fungible_asset_metadata() {
                    self.add_pending_unique_id(UID_PREFIX_ASSET, asset_id);
                }
            }
            _ => {}
        }
    }

    /// Phase 3 helper: remove unique IDs when a UtxoEntry is spent.
    fn remove_unique_id_for_entry(&mut self, entry: &UtxoEntry) {
        match entry.output.output_type {
            doli_core::OutputType::NFT => {
                if let Some((token_id, _)) = entry.output.nft_metadata() {
                    self.remove_pending_unique_id(UID_PREFIX_NFT, &token_id);
                }
            }
            doli_core::OutputType::Pool => {
                if let Some(meta) = entry.output.pool_metadata() {
                    self.remove_pending_unique_id(UID_PREFIX_POOL, &meta.pool_id);
                }
            }
            doli_core::OutputType::FungibleAsset => {
                if let Some((asset_id, _, _)) = entry.output.fungible_asset_metadata() {
                    self.remove_pending_unique_id(UID_PREFIX_ASSET, &asset_id);
                }
            }
            _ => {}
        }
    }

    // ==================== Phase 3: Overlay Reads ====================
    //
    // These methods check pending state first, then fall through to
    // committed state_db. They enable apply_block reads to see
    // uncommitted UTXO changes from earlier transactions in the same block.

    /// Get a UTXO by outpoint. Checks pending first, then disk.
    /// Returns None for outpoints spent in this batch.
    pub fn get_utxo(&self, outpoint: &Outpoint) -> Option<UtxoEntry> {
        // Check if spent in this batch
        if self.spent_in_batch.contains(outpoint) {
            return None;
        }
        // Check pending (same-block add)
        if let Some(entry) = self.pending_utxos.get(outpoint) {
            return Some(entry.clone());
        }
        // Fall through to committed DB
        self.db.get_utxo(outpoint)
    }

    /// Check if a UTXO exists. Checks pending first, then disk.
    /// Returns false for outpoints spent in this batch.
    pub fn contains_utxo(&self, outpoint: &Outpoint) -> bool {
        if self.spent_in_batch.contains(outpoint) {
            return false;
        }
        if self.pending_utxos.contains_key(outpoint) {
            return true;
        }
        self.db.contains_utxo(outpoint)
    }

    /// Get all UTXOs for a given pubkey hash. Merges pending adds with
    /// disk results, excluding outpoints spent in this batch.
    ///
    /// Phase 3: replaces `UtxoSet::get_by_pubkey_hash` for apply_block reads.
    pub fn get_utxos_by_pubkey(&self, pubkey_hash: &Hash) -> Vec<(Outpoint, UtxoEntry)> {
        // Start with disk results, excluding spent-in-batch
        let mut results: Vec<(Outpoint, UtxoEntry)> = self
            .db
            .get_utxos_by_pubkey(pubkey_hash)
            .into_iter()
            .filter(|(op, _)| !self.spent_in_batch.contains(op))
            .collect();

        // Add pending UTXOs for this pubkey (same-block adds)
        for (op, entry) in &self.pending_utxos {
            if &entry.output.pubkey_hash == pubkey_hash {
                // Don't duplicate if also on disk (pending_utxos wins)
                if !results.iter().any(|(existing, _)| existing == op) {
                    results.push((*op, entry.clone()));
                }
            }
        }

        results
    }

    // ==================== Producer Operations ====================

    /// Put a producer info record.
    pub fn put_producer(&mut self, pubkey_hash: &Hash, info: &ProducerInfo) {
        let cf = self.db.db.cf_handle(CF_PRODUCERS).unwrap();
        let value = bincode::serialize(info).expect("ProducerInfo serialization");
        self.batch.put_cf(cf, pubkey_hash.as_bytes(), &value);
    }

    /// Remove a producer record.
    pub fn remove_producer(&mut self, pubkey_hash: &Hash) {
        let cf = self.db.db.cf_handle(CF_PRODUCERS).unwrap();
        self.batch.delete_cf(cf, pubkey_hash.as_bytes());
    }

    /// Put an exit history entry.
    pub fn put_exit_history(&mut self, pubkey_hash: &Hash, exit_height: u64) {
        let cf = self.db.db.cf_handle(CF_EXIT_HISTORY).unwrap();
        self.batch
            .put_cf(cf, pubkey_hash.as_bytes(), exit_height.to_le_bytes());
    }

    /// Put the ChainState into the batch.
    pub fn put_chain_state(&mut self, cs: &ChainState) {
        let cf = self.db.db.cf_handle(CF_META).unwrap();
        let bincode_bytes = bincode::serialize(cs).expect("ChainState serialization");
        // Versioned format: 0x01 prefix + bincode payload
        let mut bytes = Vec::with_capacity(1 + bincode_bytes.len());
        bytes.push(0x01);
        bytes.extend_from_slice(&bincode_bytes);
        self.batch.put_cf(cf, META_CHAIN_STATE, &bytes);
    }

    /// Put pending producer updates into the batch.
    pub fn put_pending_updates(&mut self, updates: &[PendingProducerUpdate]) {
        let cf = self.db.db.cf_handle(CF_META).unwrap();
        let bytes = bincode::serialize(updates).expect("PendingProducerUpdate serialization");
        self.batch.put_cf(cf, META_PENDING_UPDATES, &bytes);
    }

    /// Persist the frozen epoch producer list.
    ///
    /// Serialized as concatenated 32-byte public keys. Loaded on restart
    /// to avoid reconstructing from inconsistent ProducerSet + block store.
    pub fn put_epoch_producer_list(&mut self, keys: &[crypto::PublicKey]) {
        let cf = self.db.db.cf_handle(CF_META).unwrap();
        let mut bytes = Vec::with_capacity(keys.len() * 32);
        for pk in keys {
            bytes.extend_from_slice(pk.as_bytes());
        }
        self.batch.put_cf(cf, META_EPOCH_PRODUCER_LIST, &bytes);
    }

    /// Persist attestation accumulators (survives restarts).
    /// Serialized with bincode for simplicity — these are local state, not consensus.
    pub fn put_attestation_accumulators(
        &mut self,
        attested_set: &[std::collections::HashSet<crypto::PublicKey>; 3],
        attestation_accum: &[std::collections::HashMap<crypto::PublicKey, std::collections::HashSet<u32>>;
             3],
        blocks_produced: &std::collections::HashMap<crypto::PublicKey, u32>,
    ) {
        let cf = self.db.db.cf_handle(CF_META).unwrap();
        if let Ok(bytes) = bincode::serialize(attested_set) {
            self.batch.put_cf(cf, META_EPOCH_ATTESTED_SET, &bytes);
        }
        if let Ok(bytes) = bincode::serialize(attestation_accum) {
            self.batch.put_cf(cf, META_EPOCH_ATTESTATION_ACCUM, &bytes);
        }
        if let Ok(bytes) = bincode::serialize(blocks_produced) {
            self.batch.put_cf(cf, META_EPOCH_BLOCKS_PRODUCED, &bytes);
        }
    }

    /// Persist the epoch bond snapshot {pubkey_hash -> bond_count}.
    pub fn put_epoch_bond_snapshot(
        &mut self,
        snapshot: &std::collections::HashMap<crypto::Hash, u64>,
        epoch: u64,
    ) {
        let cf = self.db.db.cf_handle(CF_META).unwrap();
        if let Ok(bytes) = bincode::serialize(&(snapshot, epoch)) {
            self.batch.put_cf(cf, META_EPOCH_BOND_SNAPSHOT, &bytes);
        }
    }

    /// Persist the complete EpochState as a single key.
    pub fn put_epoch_state(&mut self, bytes: &[u8]) {
        let cf = self.db.db.cf_handle(CF_META).unwrap();
        self.batch.put_cf(cf, META_EPOCH_STATE, bytes);
    }

    /// Persist the protocol version that produced this epoch_state.
    /// Called alongside `put_epoch_state` so the version marker stays in sync.
    pub fn put_epoch_state_version(&mut self, version: u32) {
        let cf = self.db.db.cf_handle(CF_META).unwrap();
        self.batch
            .put_cf(cf, META_EPOCH_STATE_VERSION, version.to_le_bytes());
    }

    /// Update the incremental chain commitment atomically with the block.
    pub fn put_chain_commitment(&mut self, commitment: &crypto::Hash) {
        let cf = self.db.db.cf_handle(CF_META).unwrap();
        self.batch
            .put_cf(cf, META_CHAIN_COMMITMENT, commitment.as_bytes());
    }

    /// Persist the active production list (round-robin subset of epoch list).
    pub fn put_active_production_list(&mut self, keys: &[crypto::PublicKey]) {
        let cf = self.db.db.cf_handle(CF_META).unwrap();
        let mut bytes = Vec::with_capacity(keys.len() * 32);
        for pk in keys {
            bytes.extend_from_slice(pk.as_bytes());
        }
        self.batch.put_cf(cf, META_ACTIVE_PRODUCTION_LIST, &bytes);
    }

    /// Set the last_applied consistency canary.
    pub fn set_last_applied(&mut self, height: u64, hash: Hash, slot: u32) {
        let cf = self.db.db.cf_handle(CF_META).unwrap();
        let la = LastApplied { height, hash, slot };
        self.batch.put_cf(cf, META_LAST_APPLIED, la.to_bytes());
    }

    /// Add undo data for a block height into this batch.
    /// Writing undo in the same WriteBatch as state avoids a separate WAL entry.
    pub fn put_undo(&mut self, height: u64, undo: &UndoData) {
        let cf = self.db.db.cf_handle(CF_UNDO).unwrap();
        let key = height.to_le_bytes();
        let value = bincode::serialize(undo).expect("UndoData serialization");
        self.batch.put_cf(cf, key, &value);
    }

    /// Add the pre-block maintainer trust root for a height into this batch (INC-I-174).
    ///
    /// Called ONLY for a block that carries an `AddMaintainer` / `RemoveMaintainer`;
    /// absence of the record is the "unchanged at this height" sentinel. Goes into the
    /// SAME `WriteBatch` as the block commit, so a crash can never leave a block applied
    /// with no way to undo its rotation.
    ///
    /// Separate key family, NOT a field on [`UndoData`] — see the append-hostility note
    /// on that type.
    pub fn put_maintainer_undo(&mut self, height: u64, snapshot: &MaintainerUndoSnapshot) {
        let cf = self.db.db.cf_handle(CF_UNDO).unwrap();
        let mut key = [0u8; 9];
        key[0] = MAINTAINER_UNDO_KEY_PREFIX;
        key[1..].copy_from_slice(&height.to_le_bytes());
        let value = bincode::serialize(snapshot).expect("MaintainerUndoSnapshot serialization");
        self.batch.put_cf(cf, key, &value);
    }

    /// Commit the batch atomically. All-or-nothing.
    pub fn commit(self) -> Result<(), StorageError> {
        self.db.db.write(self.batch)?;

        // Update the cached UTXO count
        if self.utxo_delta > 0 {
            self.db
                .utxo_count
                .fetch_add(self.utxo_delta as u64, Ordering::Relaxed);
        } else if self.utxo_delta < 0 {
            self.db
                .utxo_count
                .fetch_sub((-self.utxo_delta) as u64, Ordering::Relaxed);
        }

        Ok(())
    }

    /// Write only the dirty (changed) producers + exit history into this batch.
    ///
    /// Instead of rewriting all 1,000 producers when only 1 changed, this takes
    /// a set of dirty pubkey hashes and only writes those. Pending updates are
    /// always written (small).
    ///
    /// For removed producers (slashed/exited), pass them in `removed_keys`.
    pub fn write_dirty_producers(
        &mut self,
        ps: &ProducerSet,
        dirty_keys: &HashSet<Hash>,
        removed_keys: &HashSet<Hash>,
        dirty_exit_keys: &HashSet<Hash>,
    ) {
        let cf_prod = self.db.db.cf_handle(CF_PRODUCERS).unwrap();
        let cf_exit = self.db.db.cf_handle(CF_EXIT_HISTORY).unwrap();
        let cf_meta = self.db.db.cf_handle(CF_META).unwrap();

        let (producers, exit_history, pending_updates) = ps.as_parts();

        // Write only changed producers
        for key in dirty_keys {
            if let Some(info) = producers.get(key) {
                let value = bincode::serialize(info).unwrap_or_default();
                self.batch.put_cf(cf_prod, key.as_bytes(), &value);
            }
        }

        // Delete removed producers
        for key in removed_keys {
            self.batch.delete_cf(cf_prod, key.as_bytes());
        }

        // Write only changed exit history entries
        for key in dirty_exit_keys {
            if let Some(height) = exit_history.get(key) {
                self.batch
                    .put_cf(cf_exit, key.as_bytes(), height.to_le_bytes());
            }
        }

        // Pending updates are small — always write the full vec
        let pending_bytes = bincode::serialize(pending_updates).unwrap_or_default();
        self.batch
            .put_cf(cf_meta, META_PENDING_UPDATES, &pending_bytes);
    }

    /// Write the full ProducerSet into this batch (for reorg/migration).
    ///
    /// Clears existing producer/exit_history CFs in the batch and writes
    /// the entire in-memory state. Use write_dirty_producers for normal blocks.
    pub fn write_full_producer_set(&mut self, ps: &ProducerSet) {
        let cf_prod = self.db.db.cf_handle(CF_PRODUCERS).unwrap();
        let cf_exit = self.db.db.cf_handle(CF_EXIT_HISTORY).unwrap();
        let cf_meta = self.db.db.cf_handle(CF_META).unwrap();

        // Clear existing producers by iterating current DB
        for (key, _) in self
            .db
            .db
            .iterator_cf(cf_prod, rocksdb::IteratorMode::Start)
            .flatten()
        {
            self.batch.delete_cf(cf_prod, &key);
        }
        // Clear existing exit history
        for (key, _) in self
            .db
            .db
            .iterator_cf(cf_exit, rocksdb::IteratorMode::Start)
            .flatten()
        {
            self.batch.delete_cf(cf_exit, &key);
        }

        let (producers, exit_history, pending_updates) = ps.as_parts();

        for (hash, info) in producers {
            let value = bincode::serialize(info).unwrap_or_default();
            self.batch.put_cf(cf_prod, hash.as_bytes(), &value);
        }

        for (hash, height) in exit_history {
            self.batch
                .put_cf(cf_exit, hash.as_bytes(), height.to_le_bytes());
        }

        let pending_bytes = bincode::serialize(pending_updates).unwrap_or_default();
        self.batch
            .put_cf(cf_meta, META_PENDING_UPDATES, &pending_bytes);
    }
}

// ==================== Phase 3: UtxoProvider for BlockBatch ====================
//
// Enables `validate_transaction_with_utxos` to read from the batch overlay
// during apply_block, seeing uncommitted writes from earlier transactions
// in the same block.

impl UtxoProvider for BlockBatch<'_> {
    fn get_utxo(&self, tx_hash: &Hash, output_index: u32) -> Option<UtxoInfo> {
        let outpoint = Outpoint::new(*tx_hash, output_index);
        self.get_utxo(&outpoint).map(|entry| UtxoInfo {
            output: entry.output,
            pubkey: None,
            spent: false,
        })
    }
}
