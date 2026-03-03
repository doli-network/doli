//! Unified state database — single RocksDB with atomic WriteBatch per block.
//!
//! Merges UTXO set, producer set, and chain state into one database.
//! A crash at any point leaves the database in a consistent state:
//! either the full batch committed or none of it did.
//!
//! ## Column Families
//!
//! | CF | Key | Value |
//! |----|-----|-------|
//! | `cf_utxo` | Outpoint (36B) | UtxoEntry (bincode) |
//! | `cf_utxo_by_pubkey` | pubkey_hash(32B) ++ outpoint(36B) | 0x00 |
//! | `cf_producers` | pubkey_hash (32B) | ProducerInfo (bincode) |
//! | `cf_exit_history` | pubkey_hash (32B) | exit_height (8B LE) |
//! | `cf_meta` | string key | varies |

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crypto::Hash;
use doli_core::types::{Amount, BlockHeight};
use tracing::info;

use crate::chain_state::ChainState;
use crate::producer::{PendingProducerUpdate, ProducerInfo, ProducerSet};
use crate::utxo::{Outpoint, UtxoEntry};
use crate::StorageError;

// Column family names
const CF_UTXO: &str = "cf_utxo";
const CF_UTXO_BY_PUBKEY: &str = "cf_utxo_by_pubkey";
const CF_PRODUCERS: &str = "cf_producers";
const CF_EXIT_HISTORY: &str = "cf_exit_history";
const CF_META: &str = "cf_meta";

// Meta keys
const META_CHAIN_STATE: &[u8] = b"chain_state";
const META_PENDING_UPDATES: &[u8] = b"pending_updates";
const META_LAST_APPLIED: &[u8] = b"last_applied";

/// Unified state database wrapping a single RocksDB instance.
pub struct StateDb {
    db: rocksdb::DB,
    utxo_count: AtomicU64,
}

/// Atomic write batch for a single block application.
///
/// All mutations within a block go into this batch. On `commit()`,
/// the entire batch is written atomically. If the batch is dropped
/// without committing, no changes are persisted.
pub struct BlockBatch<'a> {
    db: &'a StateDb,
    batch: rocksdb::WriteBatch,
    utxo_delta: i64,
    /// UTXOs added in this batch but not yet committed to DB.
    /// Needed for same-block-spend: TX2 spending an output created by TX1
    /// in the same block won't find it via db.get() (not committed yet).
    pending_utxos: HashMap<Outpoint, UtxoEntry>,
    /// Outpoints removed in this batch (to avoid returning spent UTXOs from pending).
    spent_in_batch: Vec<Outpoint>,
}

/// The consistency canary — stored inside the same WriteBatch as state.
/// If this key exists and matches the chain_state, the DB is consistent.
#[derive(Debug, Clone)]
pub struct LastApplied {
    pub height: u64,
    pub hash: Hash,
    pub slot: u32,
}

impl LastApplied {
    const SIZE: usize = 44; // 8 + 32 + 4

    fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..8].copy_from_slice(&self.height.to_le_bytes());
        buf[8..40].copy_from_slice(self.hash.as_bytes());
        buf[40..44].copy_from_slice(&self.slot.to_le_bytes());
        buf
    }

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::SIZE {
            return None;
        }
        let height = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
        let hash = Hash::from_bytes(bytes[8..40].try_into().ok()?);
        let slot = u32::from_le_bytes(bytes[40..44].try_into().ok()?);
        Some(Self { height, hash, slot })
    }
}

impl StateDb {
    /// Open or create the unified state database at the given path.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
        // WAL for crash recovery
        opts.set_wal_recovery_mode(rocksdb::DBRecoveryMode::PointInTime);

        let cfs = vec![
            CF_UTXO,
            CF_UTXO_BY_PUBKEY,
            CF_PRODUCERS,
            CF_EXIT_HISTORY,
            CF_META,
        ];
        let db = rocksdb::DB::open_cf(&opts, path, cfs)?;

        // Count existing UTXO entries
        let cf_utxo = db.cf_handle(CF_UTXO).unwrap();
        let mut count = 0u64;
        for _ in db
            .iterator_cf(cf_utxo, rocksdb::IteratorMode::Start)
            .flatten()
        {
            count += 1;
        }

        Ok(Self {
            db,
            utxo_count: AtomicU64::new(count),
        })
    }

    /// Check if the database has any state (non-empty).
    pub fn has_state(&self) -> bool {
        let cf = self.db.cf_handle(CF_META).unwrap();
        self.db
            .get_cf(cf, META_CHAIN_STATE)
            .ok()
            .flatten()
            .is_some()
    }

    // ==================== Read Methods ====================

    /// Get a UTXO by outpoint.
    pub fn get_utxo(&self, outpoint: &Outpoint) -> Option<UtxoEntry> {
        let cf = self.db.cf_handle(CF_UTXO).unwrap();
        let key = outpoint.to_bytes();
        match self.db.get_cf(cf, &key) {
            Ok(Some(bytes)) => bincode::deserialize(&bytes).ok(),
            _ => None,
        }
    }

    /// Check if a UTXO exists.
    pub fn contains_utxo(&self, outpoint: &Outpoint) -> bool {
        let cf = self.db.cf_handle(CF_UTXO).unwrap();
        let key = outpoint.to_bytes();
        self.db.get_cf(cf, &key).ok().flatten().is_some()
    }

    /// Get all UTXOs for a given pubkey hash via secondary index prefix scan.
    pub fn get_utxos_by_pubkey(&self, pubkey_hash: &Hash) -> Vec<(Outpoint, UtxoEntry)> {
        let cf_by_pk = self.db.cf_handle(CF_UTXO_BY_PUBKEY).unwrap();
        let cf_utxo = self.db.cf_handle(CF_UTXO).unwrap();
        let prefix = pubkey_hash.as_bytes();

        let mut results = Vec::new();
        let iter = self.db.prefix_iterator_cf(cf_by_pk, prefix);
        for item in iter.flatten() {
            let (key, _) = item;
            if key.len() != 68 || &key[..32] != prefix {
                break;
            }
            if let Some(outpoint) = Outpoint::from_bytes(&key[32..68]) {
                let op_key = outpoint.to_bytes();
                if let Ok(Some(val)) = self.db.get_cf(cf_utxo, &op_key) {
                    if let Ok(entry) = bincode::deserialize::<UtxoEntry>(&val) {
                        results.push((outpoint, entry));
                    }
                }
            }
        }
        results
    }

    /// Get number of UTXOs (O(1) via cached counter).
    pub fn utxo_len(&self) -> usize {
        self.utxo_count.load(Ordering::Relaxed) as usize
    }

    /// Get total value in the UTXO set.
    pub fn utxo_total_value(&self) -> Amount {
        let cf = self.db.cf_handle(CF_UTXO).unwrap();
        let mut total: Amount = 0;
        for (_, value) in self
            .db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if let Ok(entry) = bincode::deserialize::<UtxoEntry>(&value) {
                total += entry.output.amount;
            }
        }
        total
    }

    /// Get spendable balance for a pubkey hash at a given height with custom maturity.
    pub fn get_balance_with_maturity(
        &self,
        pubkey_hash: &Hash,
        height: BlockHeight,
        maturity: BlockHeight,
    ) -> Amount {
        self.get_utxos_by_pubkey(pubkey_hash)
            .iter()
            .filter(|(_, entry)| entry.is_spendable_at_with_maturity(height, maturity))
            .map(|(_, entry)| entry.output.amount)
            .sum()
    }

    /// Get immature balance for a pubkey hash with custom maturity.
    pub fn get_immature_balance_with_maturity(
        &self,
        pubkey_hash: &Hash,
        height: BlockHeight,
        maturity: BlockHeight,
    ) -> Amount {
        self.get_utxos_by_pubkey(pubkey_hash)
            .iter()
            .filter(|(_, entry)| {
                (entry.is_coinbase || entry.is_epoch_reward)
                    && !entry.is_spendable_at_with_maturity(height, maturity)
            })
            .map(|(_, entry)| entry.output.amount)
            .sum()
    }

    /// Get a producer by pubkey hash.
    pub fn get_producer(&self, pubkey_hash: &Hash) -> Option<ProducerInfo> {
        let cf = self.db.cf_handle(CF_PRODUCERS).unwrap();
        match self.db.get_cf(cf, pubkey_hash.as_bytes()) {
            Ok(Some(bytes)) => bincode::deserialize(&bytes).ok(),
            _ => None,
        }
    }

    /// Iterate all producers. Returns (pubkey_hash, ProducerInfo) pairs.
    pub fn iter_producers(&self) -> Vec<(Hash, ProducerInfo)> {
        let cf = self.db.cf_handle(CF_PRODUCERS).unwrap();
        let mut results = Vec::new();
        for (key, value) in self
            .db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if key.len() == 32 {
                let hash = Hash::from_bytes(key[..32].try_into().unwrap());
                if let Ok(info) = bincode::deserialize::<ProducerInfo>(&value) {
                    results.push((hash, info));
                }
            }
        }
        results
    }

    /// Get exit history entry for a pubkey hash.
    pub fn get_exit_height(&self, pubkey_hash: &Hash) -> Option<u64> {
        let cf = self.db.cf_handle(CF_EXIT_HISTORY).unwrap();
        match self.db.get_cf(cf, pubkey_hash.as_bytes()) {
            Ok(Some(bytes)) if bytes.len() >= 8 => {
                Some(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
            }
            _ => None,
        }
    }

    /// Iterate all exit history entries.
    pub fn iter_exit_history(&self) -> Vec<(Hash, u64)> {
        let cf = self.db.cf_handle(CF_EXIT_HISTORY).unwrap();
        let mut results = Vec::new();
        for (key, value) in self
            .db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if key.len() == 32 && value.len() >= 8 {
                let hash = Hash::from_bytes(key[..32].try_into().unwrap());
                let height = u64::from_le_bytes(value[..8].try_into().unwrap());
                results.push((hash, height));
            }
        }
        results
    }

    /// Load ChainState from cf_meta.
    pub fn get_chain_state(&self) -> Option<ChainState> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        match self.db.get_cf(cf, META_CHAIN_STATE) {
            Ok(Some(bytes)) => bincode::deserialize(&bytes).ok(),
            _ => None,
        }
    }

    /// Load pending producer updates from cf_meta.
    pub fn get_pending_updates(&self) -> Vec<PendingProducerUpdate> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        match self.db.get_cf(cf, META_PENDING_UPDATES) {
            Ok(Some(bytes)) => bincode::deserialize(&bytes).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Load the last_applied consistency canary.
    pub fn get_last_applied(&self) -> Option<LastApplied> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        match self.db.get_cf(cf, META_LAST_APPLIED) {
            Ok(Some(bytes)) => LastApplied::from_bytes(&bytes),
            _ => None,
        }
    }

    /// Produce canonical UTXO bytes for state root computation.
    ///
    /// RocksDB iterates in lexicographic key order — no sorting needed.
    /// Values are re-encoded to canonical 59-byte format.
    pub fn serialize_canonical_utxo(&self) -> Vec<u8> {
        let cf = self.db.cf_handle(CF_UTXO).unwrap();
        let count = self.utxo_len() as u64;

        let mut buf = Vec::with_capacity(8 + (count as usize) * 95);
        buf.extend_from_slice(&count.to_le_bytes());

        for (key, value) in self
            .db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if let Ok(entry) = bincode::deserialize::<UtxoEntry>(&value) {
                buf.extend_from_slice(&key);
                buf.extend_from_slice(&entry.serialize_canonical_bytes());
            }
        }
        buf
    }

    /// Load the full ProducerSet from the database.
    ///
    /// Rebuilds the in-memory ProducerSet from cf_producers, cf_exit_history,
    /// and the pending_updates stored in cf_meta.
    pub fn load_producer_set(&self) -> ProducerSet {
        let mut producers = HashMap::new();
        let cf_prod = self.db.cf_handle(CF_PRODUCERS).unwrap();
        for (key, value) in self
            .db
            .iterator_cf(cf_prod, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if key.len() == 32 {
                let hash = Hash::from_bytes(key[..32].try_into().unwrap());
                if let Ok(info) = bincode::deserialize::<ProducerInfo>(&value) {
                    producers.insert(hash, info);
                }
            }
        }

        let mut exit_history = HashMap::new();
        let cf_exit = self.db.cf_handle(CF_EXIT_HISTORY).unwrap();
        for (key, value) in self
            .db
            .iterator_cf(cf_exit, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if key.len() == 32 && value.len() >= 8 {
                let hash = Hash::from_bytes(key[..32].try_into().unwrap());
                let height = u64::from_le_bytes(value[..8].try_into().unwrap());
                exit_history.insert(hash, height);
            }
        }

        let pending_updates = self.get_pending_updates();

        ProducerSet::from_parts(producers, exit_history, pending_updates)
    }

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
        let bytes =
            bincode::serialize(cs).map_err(|e| StorageError::Serialization(e.to_string()))?;
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

    // ==================== Batch Creation ====================

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

    /// Add outputs from a transaction directly (non-batch, for reorg rebuild).
    pub fn add_transaction(
        &self,
        tx: &doli_core::transaction::Transaction,
        height: BlockHeight,
        is_coinbase: bool,
    ) {
        let tx_hash = tx.hash();
        let is_epoch_reward = tx.is_epoch_reward();
        let cf_utxo = self.db.cf_handle(CF_UTXO).unwrap();
        let cf_by_pk = self.db.cf_handle(CF_UTXO_BY_PUBKEY).unwrap();

        let mut batch = rocksdb::WriteBatch::default();
        let mut added = 0u64;

        for (index, output) in tx.outputs.iter().enumerate() {
            let outpoint = Outpoint::new(tx_hash, index as u32);
            let entry = UtxoEntry {
                output: output.clone(),
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
            total += entry.output.amount;
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
    ) {
        let tx_hash = tx.hash();
        let is_epoch_reward = tx.is_epoch_reward();
        for (index, output) in tx.outputs.iter().enumerate() {
            let outpoint = Outpoint::new(tx_hash, index as u32);
            let entry = UtxoEntry {
                output: output.clone(),
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
        let bytes = bincode::serialize(cs).expect("ChainState serialization");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::hash::hash as crypto_hash;
    use crypto::KeyPair;
    use doli_core::transaction::{Output, Transaction};
    use tempfile::TempDir;

    fn create_test_db() -> (StateDb, TempDir) {
        let dir = TempDir::new().unwrap();
        let db = StateDb::open(dir.path()).unwrap();
        (db, dir)
    }

    fn test_coinbase_tx(amount: Amount, pubkey_hash: Hash) -> Transaction {
        Transaction::new_coinbase(amount, pubkey_hash, 0)
    }

    #[test]
    fn test_open_and_has_state() {
        let (db, _dir) = create_test_db();
        assert!(!db.has_state());

        // Write chain state
        let cs = ChainState::new(Hash::ZERO);
        db.put_chain_state(&cs).unwrap();
        assert!(db.has_state());
    }

    #[test]
    fn test_utxo_crud() {
        let (db, _dir) = create_test_db();
        let pk_hash = crypto_hash(b"alice");

        let outpoint = Outpoint::new(crypto_hash(b"tx1"), 0);
        let entry = UtxoEntry {
            output: Output::normal(500_000, pk_hash),
            height: 1,
            is_coinbase: true,
            is_epoch_reward: false,
        };

        // Insert
        db.insert_utxo(&outpoint, &entry);
        assert_eq!(db.utxo_len(), 1);

        // Get
        let got = db.get_utxo(&outpoint).unwrap();
        assert_eq!(got.output.amount, 500_000);
        assert!(got.is_coinbase);

        // Contains
        assert!(db.contains_utxo(&outpoint));
        assert!(!db.contains_utxo(&Outpoint::new(Hash::ZERO, 0)));

        // By pubkey
        let utxos = db.get_utxos_by_pubkey(&pk_hash);
        assert_eq!(utxos.len(), 1);

        // Remove
        let removed = db.remove_utxo(&outpoint).unwrap();
        assert_eq!(removed.output.amount, 500_000);
        assert_eq!(db.utxo_len(), 0);
    }

    #[test]
    fn test_batch_commit_atomic() {
        let (db, _dir) = create_test_db();
        let pk_hash = crypto_hash(b"alice");

        let outpoint1 = Outpoint::new(crypto_hash(b"tx1"), 0);
        let outpoint2 = Outpoint::new(crypto_hash(b"tx2"), 0);
        let entry1 = UtxoEntry {
            output: Output::normal(100, pk_hash),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        let entry2 = UtxoEntry {
            output: Output::normal(200, pk_hash),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };

        // Batch add two UTXOs + chain state
        let mut batch = db.begin_batch();
        batch.add_utxo(outpoint1, entry1);
        batch.add_utxo(outpoint2, entry2);

        let cs = ChainState::new(Hash::ZERO);
        batch.put_chain_state(&cs);
        batch.set_last_applied(1, crypto_hash(b"block1"), 1);

        batch.commit().unwrap();

        // Verify all committed
        assert_eq!(db.utxo_len(), 2);
        assert!(db.get_utxo(&outpoint1).is_some());
        assert!(db.get_utxo(&outpoint2).is_some());
        assert!(db.get_chain_state().is_some());
        assert!(db.get_last_applied().is_some());
    }

    #[test]
    fn test_batch_drop_no_commit() {
        let (db, _dir) = create_test_db();
        let pk_hash = crypto_hash(b"alice");

        let outpoint = Outpoint::new(crypto_hash(b"tx1"), 0);
        let entry = UtxoEntry {
            output: Output::normal(100, pk_hash),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };

        // Create batch, add UTXO, but DON'T commit
        {
            let mut batch = db.begin_batch();
            batch.add_utxo(outpoint, entry);
            // Drop without commit
        }

        // Nothing persisted
        assert_eq!(db.utxo_len(), 0);
        assert!(db.get_utxo(&outpoint).is_none());
    }

    #[test]
    fn test_same_block_spend() {
        let (db, _dir) = create_test_db();
        let pk_hash = crypto_hash(b"alice");
        let bob_hash = crypto_hash(b"bob");

        let outpoint = Outpoint::new(crypto_hash(b"tx1"), 0);
        let entry = UtxoEntry {
            output: Output::normal(1000, pk_hash),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };

        // TX1 creates UTXO, TX2 spends it — both in same block
        let mut batch = db.begin_batch();
        batch.add_utxo(outpoint, entry);

        // Same-block spend should work (finds in pending_utxos)
        let spent = batch.spend_utxo(&outpoint).unwrap();
        assert_eq!(spent.output.amount, 1000);

        // Add TX2's output
        let outpoint2 = Outpoint::new(crypto_hash(b"tx2"), 0);
        let entry2 = UtxoEntry {
            output: Output::normal(900, bob_hash),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        batch.add_utxo(outpoint2, entry2);

        batch.commit().unwrap();

        // Only TX2's output should exist
        assert_eq!(db.utxo_len(), 1);
        assert!(db.get_utxo(&outpoint).is_none());
        assert!(db.get_utxo(&outpoint2).is_some());
    }

    #[test]
    fn test_producer_crud() {
        let (db, _dir) = create_test_db();

        let kp = KeyPair::generate();
        let pk = *kp.public_key();
        let pk_hash = crypto_hash(pk.as_bytes());

        let info = ProducerInfo::new_with_bonds(pk, 0, 1_000_000_000, (Hash::ZERO, 0), 0, 1);

        let mut batch = db.begin_batch();
        batch.put_producer(&pk_hash, &info);
        batch.commit().unwrap();

        let got = db.get_producer(&pk_hash).unwrap();
        assert_eq!(got.public_key, pk);
        assert_eq!(got.bond_count, 1);
    }

    #[test]
    fn test_exit_history() {
        let (db, _dir) = create_test_db();
        let pk_hash = crypto_hash(b"producer1");

        let mut batch = db.begin_batch();
        batch.put_exit_history(&pk_hash, 5000);
        batch.commit().unwrap();

        assert_eq!(db.get_exit_height(&pk_hash), Some(5000));

        let history = db.iter_exit_history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0], (pk_hash, 5000));
    }

    #[test]
    fn test_chain_state_roundtrip() {
        let (db, _dir) = create_test_db();

        let mut cs = ChainState::new(crypto_hash(b"genesis"));
        cs.update(crypto_hash(b"block42"), 42, 100);
        cs.total_minted = 999_000_000;

        db.put_chain_state(&cs).unwrap();

        let loaded = db.get_chain_state().unwrap();
        assert_eq!(loaded.best_height, 42);
        assert_eq!(loaded.best_slot, 100);
        assert_eq!(loaded.total_minted, 999_000_000);
    }

    #[test]
    fn test_last_applied_roundtrip() {
        let (db, _dir) = create_test_db();

        let mut batch = db.begin_batch();
        let hash = crypto_hash(b"block100");
        batch.set_last_applied(100, hash, 200);
        batch.commit().unwrap();

        let la = db.get_last_applied().unwrap();
        assert_eq!(la.height, 100);
        assert_eq!(la.hash, hash);
        assert_eq!(la.slot, 200);
    }

    #[test]
    fn test_clear_and_write_genesis() {
        let (db, _dir) = create_test_db();
        let pk_hash = crypto_hash(b"alice");

        // Add some state
        let outpoint = Outpoint::new(crypto_hash(b"tx1"), 0);
        let entry = UtxoEntry {
            output: Output::normal(100, pk_hash),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        db.insert_utxo(&outpoint, &entry);

        let mut cs = ChainState::new(crypto_hash(b"genesis"));
        cs.update(crypto_hash(b"block50"), 50, 100);
        db.put_chain_state(&cs).unwrap();

        assert!(db.has_state());
        assert_eq!(db.utxo_len(), 1);

        // Atomic clear + write genesis
        let genesis_cs = ChainState::new(crypto_hash(b"genesis"));
        db.clear_and_write_genesis(&genesis_cs);

        // DB is never empty — genesis state is there
        assert!(db.has_state());
        let loaded = db.get_chain_state().unwrap();
        assert_eq!(loaded.best_height, 0);

        // UTXOs gone
        assert_eq!(db.utxo_len(), 0);
        assert!(db.get_utxo(&outpoint).is_none());

        // last_applied is set to genesis
        let la = db.get_last_applied().unwrap();
        assert_eq!(la.height, 0);
    }

    #[test]
    fn test_atomic_replace() {
        let (db, _dir) = create_test_db();
        let pk_hash = crypto_hash(b"alice");

        // Seed initial state
        let outpoint = Outpoint::new(crypto_hash(b"tx_old"), 0);
        let entry = UtxoEntry {
            output: Output::normal(100, pk_hash),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        db.insert_utxo(&outpoint, &entry);
        db.put_chain_state(&ChainState::new(Hash::ZERO)).unwrap();

        // New state after reorg
        let mut new_cs = ChainState::new(Hash::ZERO);
        new_cs.update(crypto_hash(b"new_tip"), 100, 200);
        let new_ps = ProducerSet::new();

        let bob_hash = crypto_hash(b"bob");
        let new_outpoint = Outpoint::new(crypto_hash(b"tx_new"), 0);
        let new_entry = UtxoEntry {
            output: Output::normal(999, bob_hash),
            height: 100,
            is_coinbase: false,
            is_epoch_reward: false,
        };

        db.atomic_replace(&new_cs, &new_ps, std::iter::once((new_outpoint, new_entry)))
            .unwrap();

        // Old UTXO gone
        assert!(db.get_utxo(&outpoint).is_none());
        // New UTXO present
        assert_eq!(db.get_utxo(&new_outpoint).unwrap().output.amount, 999);
        assert_eq!(db.utxo_len(), 1);
        // Chain state is new
        assert_eq!(db.get_chain_state().unwrap().best_height, 100);
    }

    #[test]
    fn test_write_dirty_producers() {
        let (db, _dir) = create_test_db();

        // Seed 3 producers via full write
        let mut ps = ProducerSet::new();
        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();
        let kp3 = KeyPair::generate();
        let pk1 = *kp1.public_key();
        let pk2 = *kp2.public_key();
        let pk3 = *kp3.public_key();
        let _ = ps.register_genesis_producer(pk1, 1, 1_000_000_000);
        let _ = ps.register_genesis_producer(pk2, 1, 1_000_000_000);
        let _ = ps.register_genesis_producer(pk3, 1, 1_000_000_000);
        db.write_producer_set(&ps).unwrap();
        assert_eq!(db.iter_producers().len(), 3);

        // Simulate: only pk2 changed (e.g., added bond)
        let pk2_hash = crypto_hash(pk2.as_bytes());
        let dirty = HashSet::from([pk2_hash]);
        let removed = HashSet::new();
        let dirty_exits = HashSet::new();

        // Modify pk2 in-memory
        if let Some(info) = ps.get_by_pubkey_mut(&pk2) {
            info.bond_count = 5;
        }

        let mut batch = db.begin_batch();
        batch.write_dirty_producers(&ps, &dirty, &removed, &dirty_exits);
        batch.commit().unwrap();

        // pk2 updated
        let got = db.get_producer(&pk2_hash).unwrap();
        assert_eq!(got.bond_count, 5);
        // pk1 unchanged
        let pk1_hash = crypto_hash(pk1.as_bytes());
        let got1 = db.get_producer(&pk1_hash).unwrap();
        assert_eq!(got1.bond_count, 1);
        // Still 3 producers total
        assert_eq!(db.iter_producers().len(), 3);
    }

    #[test]
    fn test_serialize_canonical_utxo_deterministic() {
        let (db, _dir) = create_test_db();
        let pk_hash = crypto_hash(b"alice");

        for i in 0..5u64 {
            let tx = test_coinbase_tx(100_000 * (i + 1), pk_hash);
            db.add_transaction(&tx, i, true);
        }

        let bytes1 = db.serialize_canonical_utxo();
        let bytes2 = db.serialize_canonical_utxo();
        assert_eq!(
            bytes1, bytes2,
            "Canonical serialization must be deterministic"
        );
    }

    #[test]
    fn test_load_producer_set_roundtrip() {
        let (db, _dir) = create_test_db();

        let mut ps = ProducerSet::new();
        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();
        ps.register_genesis_producer(*kp1.public_key(), 1, 1_000_000_000)
            .expect("register pk1");
        ps.register_genesis_producer(*kp2.public_key(), 1, 1_000_000_000)
            .expect("register pk2");
        assert_eq!(ps.active_count(), 2);

        db.write_producer_set(&ps).unwrap();

        let raw = db.iter_producers();
        assert_eq!(raw.len(), 2, "DB should have 2 producer entries");

        let loaded = db.load_producer_set();
        assert_eq!(loaded.active_count(), 2);
    }
}
