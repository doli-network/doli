//! Direct write methods on StateDb (non-batch)

use std::sync::atomic::Ordering;

use doli_core::types::{Amount, BlockHeight};
use tracing::info;

use crate::chain_state::ChainState;
use crate::producer::ProducerSet;
use crate::utxo::{Outpoint, UtxoEntry};
use crate::StorageError;

use super::types::{
    LastApplied, StateDb, CF_EXIT_HISTORY, CF_META, CF_PRODUCERS, CF_UTXO, CF_UTXO_BY_PUBKEY,
    META_CHAIN_STATE, META_LAST_APPLIED, META_PENDING_UPDATES,
};

impl StateDb {
    // ==================== Direct Write (non-batch) ====================

    /// Insert a UTXO directly (for migration).
    pub fn insert_utxo(&self, outpoint: &Outpoint, entry: &UtxoEntry) {
        let cf_utxo = self.db.cf_handle(CF_UTXO).unwrap();
        let cf_by_pk = self.db.cf_handle(CF_UTXO_BY_PUBKEY).unwrap();

        let key = outpoint.to_bytes();
        let value = bincode::serialize(entry).expect("UtxoEntry serialization");

        let mut batch = rocksdb::WriteBatch::default();
        batch.put_cf(cf_utxo, &key, &value);

        let mut idx_key = Vec::with_capacity(68);
        idx_key.extend_from_slice(entry.output.pubkey_hash.as_bytes());
        idx_key.extend_from_slice(&key);
        batch.put_cf(cf_by_pk, &idx_key, [0u8]);

        self.db.write(batch).expect("RocksDB write batch");
        self.utxo_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Remove a UTXO directly (for migration/reorg).
    pub fn remove_utxo(&self, outpoint: &Outpoint) -> Option<UtxoEntry> {
        let cf_utxo = self.db.cf_handle(CF_UTXO).unwrap();
        let cf_by_pk = self.db.cf_handle(CF_UTXO_BY_PUBKEY).unwrap();

        let key = outpoint.to_bytes();
        let entry_bytes = self.db.get_cf(cf_utxo, &key).ok()??;
        let entry: UtxoEntry = bincode::deserialize(&entry_bytes).ok()?;

        let mut batch = rocksdb::WriteBatch::default();
        batch.delete_cf(cf_utxo, &key);

        let mut idx_key = Vec::with_capacity(68);
        idx_key.extend_from_slice(entry.output.pubkey_hash.as_bytes());
        idx_key.extend_from_slice(&key);
        batch.delete_cf(cf_by_pk, &idx_key);

        self.db.write(batch).expect("RocksDB write batch");
        self.utxo_count.fetch_sub(1, Ordering::Relaxed);

        Some(entry)
    }

    /// Clear all UTXOs (for reorg/resync).
    pub fn clear_utxos(&self) {
        let cf_utxo = self.db.cf_handle(CF_UTXO).unwrap();
        let cf_by_pk = self.db.cf_handle(CF_UTXO_BY_PUBKEY).unwrap();

        let mut batch = rocksdb::WriteBatch::default();
        for (key, _) in self
            .db
            .iterator_cf(cf_utxo, rocksdb::IteratorMode::Start)
            .flatten()
        {
            batch.delete_cf(cf_utxo, &key);
        }
        for (key, _) in self
            .db
            .iterator_cf(cf_by_pk, rocksdb::IteratorMode::Start)
            .flatten()
        {
            batch.delete_cf(cf_by_pk, &key);
        }
        let _ = self.db.write(batch);
        self.utxo_count.store(0, Ordering::Relaxed);
    }

    /// Atomically clear all CFs and write genesis ChainState in one WriteBatch.
    ///
    /// Used by force_resync_from_genesis. Never leaves the DB empty — the batch
    /// deletes old state AND writes genesis state in a single commit.
    /// Crash between "delete" and "write" is impossible because it's one batch.
    pub fn clear_and_write_genesis(&self, genesis_cs: &ChainState) {
        let cfs = [
            CF_UTXO,
            CF_UTXO_BY_PUBKEY,
            CF_PRODUCERS,
            CF_EXIT_HISTORY,
            CF_META,
        ];
        let mut batch = rocksdb::WriteBatch::default();

        // Delete everything in all CFs
        for cf_name in &cfs {
            let cf = self.db.cf_handle(cf_name).unwrap();
            for (key, _) in self
                .db
                .iterator_cf(cf, rocksdb::IteratorMode::Start)
                .flatten()
            {
                batch.delete_cf(cf, &key);
            }
        }

        // Write genesis chain state + last_applied into the same batch
        let cf_meta = self.db.cf_handle(CF_META).unwrap();
        let cs_bytes = bincode::serialize(genesis_cs).expect("ChainState serialization");
        batch.put_cf(cf_meta, META_CHAIN_STATE, &cs_bytes);
        let la = LastApplied {
            height: genesis_cs.best_height,
            hash: genesis_cs.best_hash,
            slot: genesis_cs.best_slot,
        };
        batch.put_cf(cf_meta, META_LAST_APPLIED, la.to_bytes());

        self.db.write(batch).expect("clear_and_write_genesis batch");
        self.utxo_count.store(0, Ordering::Relaxed);
    }

    /// Atomically replace ALL state (for reorg replay or snap sync).
    ///
    /// Deletes everything in all CFs, then writes the new state — all in one
    /// WriteBatch. On crash: either the old state is fully intact, or the new
    /// state is fully written. Never an empty DB.
    pub fn atomic_replace(
        &self,
        cs: &ChainState,
        ps: &ProducerSet,
        utxo_iter: impl Iterator<Item = (Outpoint, UtxoEntry)>,
    ) -> Result<(), StorageError> {
        let cf_utxo = self.db.cf_handle(CF_UTXO).unwrap();
        let cf_by_pk = self.db.cf_handle(CF_UTXO_BY_PUBKEY).unwrap();
        let cf_prod = self.db.cf_handle(CF_PRODUCERS).unwrap();
        let cf_exit = self.db.cf_handle(CF_EXIT_HISTORY).unwrap();
        let cf_meta = self.db.cf_handle(CF_META).unwrap();

        let mut batch = rocksdb::WriteBatch::default();

        // Phase 1: delete all existing keys
        let all_cfs: [(&str, &rocksdb::ColumnFamily); 5] = [
            (CF_UTXO, cf_utxo),
            (CF_UTXO_BY_PUBKEY, cf_by_pk),
            (CF_PRODUCERS, cf_prod),
            (CF_EXIT_HISTORY, cf_exit),
            (CF_META, cf_meta),
        ];
        for (_, cf) in &all_cfs {
            for (key, _) in self
                .db
                .iterator_cf(*cf, rocksdb::IteratorMode::Start)
                .flatten()
            {
                batch.delete_cf(*cf, &key);
            }
        }

        // Phase 2: write new state into the same batch

        // UTXOs
        let mut utxo_count = 0u64;
        for (outpoint, entry) in utxo_iter {
            let key = outpoint.to_bytes();
            let value = bincode::serialize(&entry)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            batch.put_cf(cf_utxo, &key, &value);

            let mut idx_key = Vec::with_capacity(68);
            idx_key.extend_from_slice(entry.output.pubkey_hash.as_bytes());
            idx_key.extend_from_slice(&key);
            batch.put_cf(cf_by_pk, &idx_key, [0u8]);

            utxo_count += 1;
        }

        // Producers + exit history
        let (producers, exit_history, pending_updates) = ps.as_parts();
        for (hash, info) in producers {
            let value =
                bincode::serialize(info).map_err(|e| StorageError::Serialization(e.to_string()))?;
            batch.put_cf(cf_prod, hash.as_bytes(), &value);
        }
        for (hash, height) in exit_history {
            batch.put_cf(cf_exit, hash.as_bytes(), height.to_le_bytes());
        }

        // Meta: chain state, pending updates, last_applied
        let cs_bytes =
            bincode::serialize(cs).map_err(|e| StorageError::Serialization(e.to_string()))?;
        batch.put_cf(cf_meta, META_CHAIN_STATE, &cs_bytes);

        let pending_bytes = bincode::serialize(pending_updates)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        batch.put_cf(cf_meta, META_PENDING_UPDATES, &pending_bytes);

        let la = LastApplied {
            height: cs.best_height,
            hash: cs.best_hash,
            slot: cs.best_slot,
        };
        batch.put_cf(cf_meta, META_LAST_APPLIED, la.to_bytes());

        // Single atomic commit
        self.db.write(batch)?;
        self.utxo_count.store(utxo_count, Ordering::Relaxed);

        Ok(())
    }

    /// Write a ChainState directly (non-batch, for genesis/migration).
    pub fn put_chain_state(&self, cs: &ChainState) -> Result<(), StorageError> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        let bincode_bytes =
            bincode::serialize(cs).map_err(|e| StorageError::Serialization(e.to_string()))?;
        // Versioned format: 0x01 prefix + bincode payload
        let mut bytes = Vec::with_capacity(1 + bincode_bytes.len());
        bytes.push(0x01);
        bytes.extend_from_slice(&bincode_bytes);
        self.db.put_cf(cf, META_CHAIN_STATE, &bytes)?;
        Ok(())
    }

    /// Write a full ProducerSet into the database (non-batch, for migration/reorg).
    pub fn write_producer_set(&self, ps: &ProducerSet) -> Result<(), StorageError> {
        let cf_prod = self.db.cf_handle(CF_PRODUCERS).unwrap();
        let cf_exit = self.db.cf_handle(CF_EXIT_HISTORY).unwrap();
        let cf_meta = self.db.cf_handle(CF_META).unwrap();

        let mut batch = rocksdb::WriteBatch::default();

        // Clear existing producers
        for (key, _) in self
            .db
            .iterator_cf(cf_prod, rocksdb::IteratorMode::Start)
            .flatten()
        {
            batch.delete_cf(cf_prod, &key);
        }
        // Clear existing exit history
        for (key, _) in self
            .db
            .iterator_cf(cf_exit, rocksdb::IteratorMode::Start)
            .flatten()
        {
            batch.delete_cf(cf_exit, &key);
        }

        // Write all producers
        let (producers, exit_history, pending_updates) = ps.as_parts();
        for (hash, info) in producers {
            let value =
                bincode::serialize(info).map_err(|e| StorageError::Serialization(e.to_string()))?;
            batch.put_cf(cf_prod, hash.as_bytes(), &value);
        }

        // Write exit history
        for (hash, height) in exit_history {
            batch.put_cf(cf_exit, hash.as_bytes(), height.to_le_bytes());
        }

        // Write pending updates
        let pending_bytes = bincode::serialize(pending_updates)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        batch.put_cf(cf_meta, META_PENDING_UPDATES, &pending_bytes);

        self.db.write(batch)?;
        Ok(())
    }

    /// Bulk import UTXOs from an iterator (for migration).
    pub fn import_utxos<'a>(&self, entries: impl Iterator<Item = (&'a Outpoint, &'a UtxoEntry)>) {
        let cf_utxo = self.db.cf_handle(CF_UTXO).unwrap();
        let cf_by_pk = self.db.cf_handle(CF_UTXO_BY_PUBKEY).unwrap();

        let mut batch = rocksdb::WriteBatch::default();
        let mut count = 0u64;

        for (outpoint, entry) in entries {
            let key = outpoint.to_bytes();
            let value = bincode::serialize(entry).expect("UtxoEntry serialization");

            batch.put_cf(cf_utxo, &key, &value);

            let mut idx_key = Vec::with_capacity(68);
            idx_key.extend_from_slice(entry.output.pubkey_hash.as_bytes());
            idx_key.extend_from_slice(&key);
            batch.put_cf(cf_by_pk, &idx_key, [0u8]);

            count += 1;

            if count.is_multiple_of(50_000) {
                self.db.write(batch).expect("RocksDB write batch");
                batch = rocksdb::WriteBatch::default();
                info!("[STATE_DB] Imported {} UTXO entries...", count);
            }
        }

        if !count.is_multiple_of(50_000) {
            self.db.write(batch).expect("RocksDB write batch");
        }

        self.utxo_count.store(count, Ordering::Relaxed);
        info!("[STATE_DB] UTXO import complete: {} entries", count);
    }

    /// Iterate all UTXOs in the database.
    ///
    /// Returns (Outpoint, UtxoEntry) pairs. Used for loading into in-memory
    /// UtxoSet on startup or for rebuilding state.
    pub fn iter_utxos(&self) -> Vec<(Outpoint, UtxoEntry)> {
        let cf = self.db.cf_handle(CF_UTXO).unwrap();
        let mut result = Vec::new();

        for (key, value) in self
            .db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if let (Some(outpoint), Ok(entry)) = (
                Outpoint::from_bytes(&key),
                bincode::deserialize::<UtxoEntry>(&value),
            ) {
                result.push((outpoint, entry));
            }
        }

        result
    }

    /// Add outputs from a transaction directly (non-batch, for reorg rebuild).
    pub fn add_transaction(
        &self,
        tx: &doli_core::transaction::Transaction,
        height: BlockHeight,
        is_coinbase: bool,
        slot: u32,
    ) {
        let tx_hash = tx.hash();
        let is_epoch_reward = tx.is_epoch_reward();
        let cf_utxo = self.db.cf_handle(CF_UTXO).unwrap();
        let cf_by_pk = self.db.cf_handle(CF_UTXO_BY_PUBKEY).unwrap();

        let mut batch = rocksdb::WriteBatch::default();
        let mut added = 0u64;

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

            let key = outpoint.to_bytes();
            let value = bincode::serialize(&entry).expect("UtxoEntry serialization");

            batch.put_cf(cf_utxo, &key, &value);

            let mut idx_key = Vec::with_capacity(68);
            idx_key.extend_from_slice(output.pubkey_hash.as_bytes());
            idx_key.extend_from_slice(&key);
            batch.put_cf(cf_by_pk, &idx_key, [0u8]);

            added += 1;
        }

        if added > 0 {
            self.db.write(batch).expect("RocksDB write batch");
            self.utxo_count.fetch_add(added, Ordering::Relaxed);
        }
    }

    /// Spend inputs of a transaction directly (non-batch, for reorg rebuild).
    pub fn spend_transaction(
        &self,
        tx: &doli_core::transaction::Transaction,
    ) -> Result<Amount, StorageError> {
        let cf_utxo = self.db.cf_handle(CF_UTXO).unwrap();
        let cf_by_pk = self.db.cf_handle(CF_UTXO_BY_PUBKEY).unwrap();

        let mut batch = rocksdb::WriteBatch::default();
        let mut total_input: Amount = 0;
        let mut removed = 0u64;

        for input in &tx.inputs {
            let outpoint = Outpoint::new(input.prev_tx_hash, input.output_index);
            let key = outpoint.to_bytes();

            let entry_bytes = self.db.get_cf(cf_utxo, &key)?.ok_or_else(|| {
                StorageError::NotFound(format!(
                    "UTXO not found: {}:{}",
                    input.prev_tx_hash, input.output_index
                ))
            })?;

            let entry: UtxoEntry = bincode::deserialize(&entry_bytes)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;

            total_input += entry.output.amount;

            batch.delete_cf(cf_utxo, &key);

            let mut idx_key = Vec::with_capacity(68);
            idx_key.extend_from_slice(entry.output.pubkey_hash.as_bytes());
            idx_key.extend_from_slice(&key);
            batch.delete_cf(cf_by_pk, &idx_key);

            removed += 1;
        }

        if removed > 0 {
            self.db.write(batch)?;
            self.utxo_count.fetch_sub(removed, Ordering::Relaxed);
        }

        Ok(total_input)
    }
}
