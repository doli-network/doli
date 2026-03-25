//! BlockBatch implementation — atomic write batch for block application

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;

use crypto::Hash;
use doli_core::types::{Amount, BlockHeight};

use crate::chain_state::ChainState;
use crate::producer::{PendingProducerUpdate, ProducerInfo, ProducerSet};
use crate::utxo::{Outpoint, UtxoEntry};
use crate::StorageError;

use super::types::{
    BlockBatch, LastApplied, StateDb, CF_EXIT_HISTORY, CF_META, CF_PRODUCERS, CF_UTXO,
    CF_UTXO_BY_PUBKEY, META_CHAIN_STATE, META_LAST_APPLIED, META_PENDING_UPDATES,
};

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
                    "UTXO not found: {}:{}",
                    outpoint.tx_hash, outpoint.index
                ))
            })?;
            bincode::deserialize(&entry_bytes)
                .map_err(|e| StorageError::Serialization(e.to_string()))?
        };

        let key = outpoint.to_bytes();
        self.batch.delete_cf(cf_utxo, &key);

        let mut idx_key = Vec::with_capacity(68);
        idx_key.extend_from_slice(entry.output.pubkey_hash.as_bytes());
        idx_key.extend_from_slice(&key);
        self.batch.delete_cf(cf_by_pk, &idx_key);

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
            let entry = UtxoEntry {
                output: stamped_output,
                height,
                is_coinbase,
                is_epoch_reward,
            };
            self.add_utxo(outpoint, entry);
        }
    }

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

    /// Set the last_applied consistency canary.
    pub fn set_last_applied(&mut self, height: u64, hash: Hash, slot: u32) {
        let cf = self.db.db.cf_handle(CF_META).unwrap();
        let la = LastApplied { height, hash, slot };
        self.batch.put_cf(cf, META_LAST_APPLIED, la.to_bytes());
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
