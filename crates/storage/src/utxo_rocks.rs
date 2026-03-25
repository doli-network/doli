//! RocksDB-backed UTXO store
//!
//! Provides persistent UTXO storage with O(1) lookups by outpoint
//! and O(n) prefix scans by pubkey hash via a secondary index.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crypto::Hash;
use doli_core::transaction::Transaction;
use doli_core::types::{Amount, BlockHeight};
use tracing::info;

use crate::utxo::{Outpoint, UtxoEntry};
use crate::StorageError;

/// Column family for the primary UTXO index: outpoint -> UtxoEntry
const CF_UTXO: &str = "utxo";

/// Column family for the secondary index: pubkey_hash ++ outpoint -> empty
const CF_UTXO_BY_PUBKEY: &str = "utxo_by_pubkey";

/// Column family for the generic unique ID index: prefix(1B) + id(32B) -> empty
const CF_UNIQUE_ID: &str = "unique_id";

use crate::utxo::{uid_key, UID_PREFIX_ASSET, UID_PREFIX_NFT, UID_PREFIX_POOL};

/// RocksDB-backed UTXO store
pub struct RocksDbUtxoStore {
    db: rocksdb::DB,
    /// Cached count to avoid full scan on len()
    count: AtomicU64,
}

impl RocksDbUtxoStore {
    /// Open or create a RocksDB-backed UTXO store at the given path
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);

        let cfs = vec![CF_UTXO, CF_UTXO_BY_PUBKEY, CF_UNIQUE_ID];
        let db = rocksdb::DB::open_cf(&opts, path, cfs)?;

        // Count existing entries to initialize the atomic counter
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
            count: AtomicU64::new(count),
        })
    }

    /// Get a UTXO by outpoint (returns owned value -- RocksDB can't return references)
    pub fn get(&self, outpoint: &Outpoint) -> Option<UtxoEntry> {
        let cf = self.db.cf_handle(CF_UTXO).unwrap();
        let key = outpoint.to_bytes();
        match self.db.get_cf(cf, &key) {
            Ok(Some(bytes)) => bincode::deserialize(&bytes).ok(),
            _ => None,
        }
    }

    /// Check if a UTXO exists
    pub fn contains(&self, outpoint: &Outpoint) -> bool {
        let cf = self.db.cf_handle(CF_UTXO).unwrap();
        let key = outpoint.to_bytes();
        self.db.get_cf(cf, &key).ok().flatten().is_some()
    }

    /// Add outputs from a transaction, stamping Bond UTXOs with the block slot
    pub fn add_transaction(
        &self,
        tx: &Transaction,
        height: BlockHeight,
        is_coinbase: bool,
        slot: u32,
    ) -> Result<(), StorageError> {
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
            // Stamp Pool outputs: creation_slot, last_update_slot, TWAP accumulation
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

            let key = outpoint.to_bytes();
            let value = bincode::serialize(&entry)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;

            batch.put_cf(cf_utxo, &key, &value);

            // Secondary index: pubkey_hash (32 bytes) ++ outpoint (36 bytes) -> 0x00
            let mut idx_key = Vec::with_capacity(68);
            idx_key.extend_from_slice(output.pubkey_hash.as_bytes());
            idx_key.extend_from_slice(&key);
            batch.put_cf(cf_by_pk, &idx_key, [0u8]);

            // Update unique ID index
            let cf_uid = self.db.cf_handle(CF_UNIQUE_ID).unwrap();
            match entry.output.output_type {
                doli_core::OutputType::NFT => {
                    if let Some((token_id, _)) = entry.output.nft_metadata() {
                        batch.put_cf(cf_uid, uid_key(UID_PREFIX_NFT, &token_id), [0u8]);
                    }
                }
                doli_core::OutputType::Pool => {
                    if let Some(meta) = entry.output.pool_metadata() {
                        batch.put_cf(cf_uid, uid_key(UID_PREFIX_POOL, &meta.pool_id), [0u8]);
                    }
                }
                doli_core::OutputType::FungibleAsset => {
                    if let Some((asset_id, _, _)) = entry.output.fungible_asset_metadata() {
                        batch.put_cf(cf_uid, uid_key(UID_PREFIX_ASSET, &asset_id), [0u8]);
                    }
                }
                _ => {}
            }

            added += 1;
        }

        if added > 0 {
            self.db.write(batch)?;
            self.count.fetch_add(added, Ordering::Relaxed);
        }

        Ok(())
    }

    /// Remove inputs spent by a transaction
    pub fn spend_transaction(&self, tx: &Transaction) -> Result<Amount, StorageError> {
        let cf_utxo = self.db.cf_handle(CF_UTXO).unwrap();
        let cf_by_pk = self.db.cf_handle(CF_UTXO_BY_PUBKEY).unwrap();

        let mut batch = rocksdb::WriteBatch::default();
        let mut total_input: Amount = 0;
        let mut removed = 0u64;

        for input in &tx.inputs {
            let outpoint = Outpoint::new(input.prev_tx_hash, input.output_index);
            let key = outpoint.to_bytes();

            // Read entry first to get pubkey_hash for secondary index deletion
            let entry_bytes = self.db.get_cf(cf_utxo, &key)?.ok_or_else(|| {
                StorageError::NotFound(format!(
                    "UTXO not found: {}:{}",
                    input.prev_tx_hash, input.output_index
                ))
            })?;

            let entry: UtxoEntry = bincode::deserialize(&entry_bytes)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;

            if entry.output.output_type.is_native_amount() {
                total_input += entry.output.amount;
            }

            // Delete from primary index
            batch.delete_cf(cf_utxo, &key);

            // Delete from secondary index
            let mut idx_key = Vec::with_capacity(68);
            idx_key.extend_from_slice(entry.output.pubkey_hash.as_bytes());
            idx_key.extend_from_slice(&key);
            batch.delete_cf(cf_by_pk, &idx_key);

            // Remove from unique ID index
            let cf_uid = self.db.cf_handle(CF_UNIQUE_ID).unwrap();
            match entry.output.output_type {
                doli_core::OutputType::NFT => {
                    if let Some((token_id, _)) = entry.output.nft_metadata() {
                        batch.delete_cf(cf_uid, uid_key(UID_PREFIX_NFT, &token_id));
                    }
                }
                doli_core::OutputType::Pool => {
                    if let Some(meta) = entry.output.pool_metadata() {
                        batch.delete_cf(cf_uid, uid_key(UID_PREFIX_POOL, &meta.pool_id));
                    }
                }
                doli_core::OutputType::FungibleAsset => {
                    if let Some((asset_id, _, _)) = entry.output.fungible_asset_metadata() {
                        batch.delete_cf(cf_uid, uid_key(UID_PREFIX_ASSET, &asset_id));
                    }
                }
                _ => {}
            }

            removed += 1;
        }

        if removed > 0 {
            self.db.write(batch)?;
            self.count.fetch_sub(removed, Ordering::Relaxed);
        }

        Ok(total_input)
    }

    /// Get total native DOLI value in the UTXO set.
    ///
    /// Non-native output types (FungibleAsset, LPShare, Pool, Collateral) are
    /// excluded — their `amount` field holds token units, not DOLI.
    pub fn total_value(&self) -> Amount {
        let cf = self.db.cf_handle(CF_UTXO).unwrap();
        let mut total: Amount = 0;
        for (_, value) in self
            .db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if let Ok(entry) = bincode::deserialize::<UtxoEntry>(&value) {
                if entry.output.output_type.is_native_amount() {
                    total += entry.output.amount;
                }
            }
        }
        total
    }

    /// Get number of UTXOs (O(1) via cached counter)
    pub fn len(&self) -> usize {
        self.count.load(Ordering::Relaxed) as usize
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Count unique addresses (distinct pubkey hashes) in the UTXO set
    pub fn address_count(&self) -> u64 {
        let cf_by_pk = self.db.cf_handle(CF_UTXO_BY_PUBKEY).unwrap();
        let mut count = 0u64;
        let mut last_prefix = [0u8; 32];
        let mut first = true;

        for item in self
            .db
            .iterator_cf(cf_by_pk, rocksdb::IteratorMode::Start)
            .flatten()
        {
            let (key, _) = item;
            if key.len() < 32 {
                continue;
            }
            if first || key[..32] != last_prefix {
                count += 1;
                last_prefix.copy_from_slice(&key[..32]);
                first = false;
            }
        }

        count
    }

    /// Get all UTXOs for a given pubkey hash via secondary index prefix scan
    pub fn get_by_pubkey_hash(&self, pubkey_hash: &Hash) -> Vec<(Outpoint, UtxoEntry)> {
        let cf_by_pk = self.db.cf_handle(CF_UTXO_BY_PUBKEY).unwrap();
        let cf_utxo = self.db.cf_handle(CF_UTXO).unwrap();
        let prefix = pubkey_hash.as_bytes();

        let mut results = Vec::new();

        let iter = self.db.prefix_iterator_cf(cf_by_pk, prefix);
        for item in iter.flatten() {
            let (key, _) = item;
            // Key format: pubkey_hash (32) ++ outpoint (36) = 68 bytes
            if key.len() != 68 || &key[..32] != prefix {
                break; // Past our prefix
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

    /// Get spendable DOLI balance for a pubkey hash at a given height with custom maturity.
    /// Only counts native DOLI amounts (excludes FungibleAsset, LPShare, etc.).
    pub fn get_balance_with_maturity(
        &self,
        pubkey_hash: &Hash,
        height: BlockHeight,
        maturity: BlockHeight,
    ) -> Amount {
        self.get_by_pubkey_hash(pubkey_hash)
            .iter()
            .filter(|(_, entry)| {
                entry.output.output_type.is_native_amount()
                    && entry.is_spendable_at_with_maturity(height, maturity)
            })
            .map(|(_, entry)| entry.output.amount)
            .sum()
    }

    /// Get immature DOLI balance for a pubkey hash with custom maturity.
    pub fn get_immature_balance_with_maturity(
        &self,
        pubkey_hash: &Hash,
        height: BlockHeight,
        maturity: BlockHeight,
    ) -> Amount {
        self.get_by_pubkey_hash(pubkey_hash)
            .iter()
            .filter(|(_, entry)| {
                entry.output.output_type.is_native_amount()
                    && (entry.is_coinbase || entry.is_epoch_reward)
                    && !entry.is_spendable_at_with_maturity(height, maturity)
            })
            .map(|(_, entry)| entry.output.amount)
            .sum()
    }

    /// Get bonded balance (sum of Bond UTXOs for this address)
    pub fn get_bonded_balance(&self, pubkey_hash: &Hash) -> Amount {
        self.get_by_pubkey_hash(pubkey_hash)
            .iter()
            .filter(|(_, entry)| entry.output.output_type == doli_core::OutputType::Bond)
            .map(|(_, entry)| entry.output.amount)
            .sum()
    }

    /// Count bond units for this address (total bond amount / bond_unit)
    pub fn count_bonds(&self, pubkey_hash: &Hash, bond_unit: u64) -> u32 {
        let total: u64 = self
            .get_by_pubkey_hash(pubkey_hash)
            .iter()
            .filter(|(_, entry)| entry.output.output_type == doli_core::OutputType::Bond)
            .map(|(_, entry)| entry.output.amount)
            .sum();
        if bond_unit > 0 {
            (total / bond_unit) as u32
        } else {
            0
        }
    }

    /// Get bond details: (outpoint, creation_slot, amount) for each Bond UTXO, FIFO-ordered
    pub fn get_bond_entries(
        &self,
        pubkey_hash: &Hash,
    ) -> Vec<(crate::utxo::Outpoint, u32, doli_core::types::Amount)> {
        let mut bonds: Vec<_> = self
            .get_by_pubkey_hash(pubkey_hash)
            .into_iter()
            .filter(|(_, entry)| entry.output.output_type == doli_core::OutputType::Bond)
            .map(|(op, entry)| {
                let slot = entry.output.bond_creation_slot().unwrap_or(0);
                (op, slot, entry.output.amount)
            })
            .collect();
        bonds.sort_by_key(|(_, slot, _)| *slot);
        bonds
    }

    /// Get all Pool UTXOs.
    pub fn get_all_pools(&self) -> Vec<(Outpoint, UtxoEntry)> {
        let cf = self.db.cf_handle(CF_UTXO).unwrap();
        let mut results = Vec::new();
        for (key, value) in self
            .db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if let Ok(entry) = bincode::deserialize::<UtxoEntry>(&value) {
                if entry.output.output_type == doli_core::OutputType::Pool {
                    if let Some(outpoint) = Outpoint::from_bytes(&key) {
                        results.push((outpoint, entry));
                    }
                }
            }
        }
        results
    }

    /// Get all Collateral UTXOs.
    pub fn get_all_collateral(&self) -> Vec<(Outpoint, UtxoEntry)> {
        let cf = self.db.cf_handle(CF_UTXO).unwrap();
        let mut results = Vec::new();
        for (key, value) in self
            .db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if let Ok(entry) = bincode::deserialize::<UtxoEntry>(&value) {
                if entry.output.output_type == doli_core::OutputType::Collateral {
                    if let Some(outpoint) = Outpoint::from_bytes(&key) {
                        results.push((outpoint, entry));
                    }
                }
            }
        }
        results
    }

    /// Clear all UTXOs
    pub fn clear(&self) {
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
        self.count.store(0, Ordering::Relaxed);
    }

    /// Check if a unique ID exists in the index.
    pub fn has_unique_id(&self, prefix: u8, id: &Hash) -> bool {
        let cf = self.db.cf_handle(CF_UNIQUE_ID).unwrap();
        self.db
            .get_cf(cf, uid_key(prefix, id))
            .ok()
            .flatten()
            .is_some()
    }

    /// Insert a UTXO entry directly (for migration and reorgs)
    pub fn insert(&self, outpoint: Outpoint, entry: UtxoEntry) -> Result<(), StorageError> {
        let cf_utxo = self.db.cf_handle(CF_UTXO).unwrap();
        let cf_by_pk = self.db.cf_handle(CF_UTXO_BY_PUBKEY).unwrap();

        let key = outpoint.to_bytes();
        let value =
            bincode::serialize(&entry).map_err(|e| StorageError::Serialization(e.to_string()))?;

        let mut batch = rocksdb::WriteBatch::default();
        batch.put_cf(cf_utxo, &key, &value);

        let mut idx_key = Vec::with_capacity(68);
        idx_key.extend_from_slice(entry.output.pubkey_hash.as_bytes());
        idx_key.extend_from_slice(&key);
        batch.put_cf(cf_by_pk, &idx_key, [0u8]);

        self.db.write(batch)?;
        self.count.fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    /// Remove a UTXO entry directly (for reorgs)
    pub fn remove(&self, outpoint: &Outpoint) -> Result<Option<UtxoEntry>, StorageError> {
        let cf_utxo = self.db.cf_handle(CF_UTXO).unwrap();
        let cf_by_pk = self.db.cf_handle(CF_UTXO_BY_PUBKEY).unwrap();

        let key = outpoint.to_bytes();
        let entry_bytes = match self.db.get_cf(cf_utxo, &key)? {
            Some(bytes) => bytes,
            None => return Ok(None),
        };
        let entry: UtxoEntry = bincode::deserialize(&entry_bytes)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        let mut batch = rocksdb::WriteBatch::default();
        batch.delete_cf(cf_utxo, &key);

        let mut idx_key = Vec::with_capacity(68);
        idx_key.extend_from_slice(entry.output.pubkey_hash.as_bytes());
        idx_key.extend_from_slice(&key);
        batch.delete_cf(cf_by_pk, &idx_key);

        self.db.write(batch)?;
        self.count.fetch_sub(1, Ordering::Relaxed);

        Ok(Some(entry))
    }

    /// Produce canonical bytes for deterministic state root computation.
    ///
    /// Output: `[8-byte LE count] [sorted_key1][value1] [sorted_key2][value2] ...`
    ///
    /// RocksDB iterates in lexicographic key order, so no sorting needed.
    /// Values are re-encoded to the canonical 59-byte format (immune to RocksDB
    /// on-disk bincode format variations from struct evolution).
    pub fn serialize_canonical(&self) -> Vec<u8> {
        let cf = self.db.cf_handle(CF_UTXO).unwrap();
        let count = self.len() as u64;

        // 36 bytes outpoint key + 59 bytes canonical entry value + 8 bytes header
        let mut buf = Vec::with_capacity(8 + (count as usize) * 95);
        buf.extend_from_slice(&count.to_le_bytes());

        for (key, value) in self
            .db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .flatten()
        {
            // Deserialize from on-disk bincode (handles backward compat via #[serde(default)]),
            // then re-encode to canonical 59-byte format for deterministic state root.
            if let Ok(entry) = bincode::deserialize::<UtxoEntry>(&value) {
                buf.extend_from_slice(&key);
                buf.extend_from_slice(&entry.serialize_canonical_bytes());
            }
        }

        buf
    }

    /// Iterate all UTXO entries as `(Outpoint, UtxoEntry)` pairs.
    pub fn iter_entries(&self) -> Vec<(Outpoint, UtxoEntry)> {
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

    /// Bulk import from an in-memory HashMap (for migration).
    ///
    /// Clears existing data and writes all entries from the iterator.
    pub fn import_from<'a>(
        &self,
        entries: impl Iterator<Item = (&'a Outpoint, &'a UtxoEntry)>,
    ) -> Result<(), StorageError> {
        let cf_utxo = self.db.cf_handle(CF_UTXO).unwrap();
        let cf_by_pk = self.db.cf_handle(CF_UTXO_BY_PUBKEY).unwrap();

        let mut batch = rocksdb::WriteBatch::default();
        let mut count = 0u64;

        for (outpoint, entry) in entries {
            let key = outpoint.to_bytes();
            let value = bincode::serialize(entry)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;

            batch.put_cf(cf_utxo, &key, &value);

            let mut idx_key = Vec::with_capacity(68);
            idx_key.extend_from_slice(entry.output.pubkey_hash.as_bytes());
            idx_key.extend_from_slice(&key);
            batch.put_cf(cf_by_pk, &idx_key, [0u8]);

            count += 1;

            // Flush in batches to avoid huge memory usage
            if count.is_multiple_of(50_000) {
                self.db.write(batch)?;
                batch = rocksdb::WriteBatch::default();
                info!("[UTXO_ROCKS] Imported {} entries...", count);
            }
        }

        // Write remaining
        if !count.is_multiple_of(50_000) {
            self.db.write(batch)?;
        }

        self.count.store(count, Ordering::Relaxed);
        info!("[UTXO_ROCKS] Import complete: {} entries", count);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use doli_core::transaction::{Input, Output, Transaction, TxType};
    use tempfile::TempDir;

    fn create_test_store() -> (RocksDbUtxoStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = RocksDbUtxoStore::open(dir.path()).unwrap();
        (store, dir)
    }

    fn test_coinbase_tx(amount: Amount, pubkey_hash: Hash) -> Transaction {
        Transaction::new_coinbase(amount, pubkey_hash, 0)
    }

    fn test_transfer_tx(
        prev_hash: Hash,
        prev_index: u32,
        amount: Amount,
        pubkey_hash: Hash,
    ) -> Transaction {
        Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(prev_hash, prev_index)],
            outputs: vec![Output::normal(amount, pubkey_hash)],
            extra_data: vec![],
        }
    }

    #[test]
    fn test_rocksdb_insert_get_remove() {
        let (store, _dir) = create_test_store();
        let pk_hash = crypto::hash::hash(b"alice");

        let tx = test_coinbase_tx(500_000_000, pk_hash);
        let tx_hash = tx.hash();

        // Add
        store.add_transaction(&tx, 0, true, 0).unwrap();
        assert_eq!(store.len(), 1);

        // Get
        let outpoint = Outpoint::new(tx_hash, 0);
        let entry = store.get(&outpoint).unwrap();
        assert_eq!(entry.output.amount, 500_000_000);
        assert!(entry.is_coinbase);

        // Contains
        assert!(store.contains(&outpoint));
        assert!(!store.contains(&Outpoint::new(Hash::ZERO, 0)));

        // Remove
        let removed = store.remove(&outpoint).unwrap().unwrap();
        assert_eq!(removed.output.amount, 500_000_000);
        assert_eq!(store.len(), 0);
        assert!(!store.contains(&outpoint));
    }

    #[test]
    fn test_rocksdb_spend_transaction() {
        let (store, _dir) = create_test_store();
        let pk_hash = crypto::hash::hash(b"bob");

        // Create and add coinbase
        let coinbase = test_coinbase_tx(1_000_000, pk_hash);
        let cb_hash = coinbase.hash();
        store.add_transaction(&coinbase, 0, true, 0).unwrap();
        assert_eq!(store.len(), 1);

        // Spend it
        let spend_tx = test_transfer_tx(cb_hash, 0, 900_000, crypto::hash::hash(b"charlie"));
        let total = store.spend_transaction(&spend_tx).unwrap();
        assert_eq!(total, 1_000_000);
        assert_eq!(store.len(), 0);

        // Double-spend should fail
        let result = store.spend_transaction(&spend_tx);
        assert!(result.is_err());
    }

    #[test]
    fn test_rocksdb_secondary_index() {
        let (store, _dir) = create_test_store();
        let alice = crypto::hash::hash(b"alice");
        let bob = crypto::hash::hash(b"bob");

        // Add 3 UTXOs for alice, 1 for bob
        for i in 0..3 {
            let tx = test_coinbase_tx(100_000 * (i + 1), alice);
            store.add_transaction(&tx, i, true, 0).unwrap();
        }
        let bob_tx = test_coinbase_tx(500_000, bob);
        store.add_transaction(&bob_tx, 3, true, 0).unwrap();

        assert_eq!(store.len(), 4);

        // Query by pubkey
        let alice_utxos = store.get_by_pubkey_hash(&alice);
        assert_eq!(alice_utxos.len(), 3);

        let bob_utxos = store.get_by_pubkey_hash(&bob);
        assert_eq!(bob_utxos.len(), 1);
        assert_eq!(bob_utxos[0].1.output.amount, 500_000);

        // Non-existent pubkey
        let unknown = crypto::hash::hash(b"unknown");
        assert!(store.get_by_pubkey_hash(&unknown).is_empty());
    }

    #[test]
    fn test_rocksdb_batch_write_atomic() {
        let (store, _dir) = create_test_store();
        let pk_hash = crypto::hash::hash(b"alice");

        // A transaction with multiple outputs should add all atomically
        let tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![],
            outputs: vec![
                Output::normal(100, pk_hash),
                Output::normal(200, pk_hash),
                Output::normal(300, pk_hash),
            ],
            extra_data: vec![],
        };

        store.add_transaction(&tx, 0, false, 0).unwrap();
        assert_eq!(store.len(), 3);
        assert_eq!(store.total_value(), 600);
    }

    #[test]
    fn test_serialize_canonical_deterministic() {
        let (store, _dir) = create_test_store();
        let pk_hash = crypto::hash::hash(b"alice");

        for i in 0..5 {
            let tx = test_coinbase_tx(100_000 * (i + 1), pk_hash);
            store.add_transaction(&tx, i, true, 0).unwrap();
        }

        let bytes1 = store.serialize_canonical();
        let bytes2 = store.serialize_canonical();
        assert_eq!(
            bytes1, bytes2,
            "Canonical serialization must be deterministic"
        );
    }

    #[test]
    fn test_rocksdb_len_tracking() {
        let (store, _dir) = create_test_store();
        assert_eq!(store.len(), 0);
        assert!(store.is_empty());

        let pk_hash = crypto::hash::hash(b"test");
        let tx1 = test_coinbase_tx(100, pk_hash);
        store.add_transaction(&tx1, 0, true, 0).unwrap();
        assert_eq!(store.len(), 1);

        let tx2 = test_coinbase_tx(200, pk_hash);
        store.add_transaction(&tx2, 1, true, 0).unwrap();
        assert_eq!(store.len(), 2);

        // Remove one
        store.remove(&Outpoint::new(tx1.hash(), 0)).unwrap();
        assert_eq!(store.len(), 1);
        assert!(!store.is_empty());
    }

    #[test]
    fn test_rocksdb_clear() {
        let (store, _dir) = create_test_store();
        let pk_hash = crypto::hash::hash(b"test");

        for i in 0..10 {
            let tx = test_coinbase_tx(100 * (i + 1), pk_hash);
            store.add_transaction(&tx, i, true, 0).unwrap();
        }
        assert_eq!(store.len(), 10);

        store.clear();
        assert_eq!(store.len(), 0);
        assert!(store.is_empty());
        assert!(store.get_by_pubkey_hash(&pk_hash).is_empty());
    }

    // Test 7: RocksDB unique ID survives reopen
    #[test]
    fn test_unique_index_rocks_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let id = crypto::Hash::from_bytes([0x11; 32]);
        {
            let store = RocksDbUtxoStore::open(dir.path()).unwrap();
            let cf = store.db.cf_handle(CF_UNIQUE_ID).unwrap();
            store
                .db
                .put_cf(cf, uid_key(UID_PREFIX_NFT, &id), [0u8])
                .unwrap();
        }
        // Reopen
        {
            let store = RocksDbUtxoStore::open(dir.path()).unwrap();
            assert!(store.has_unique_id(UID_PREFIX_NFT, &id));
        }
    }

    // Test 8: RocksDB unique ID removal survives reopen
    #[test]
    fn test_unique_index_rocks_remove_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let id = crypto::Hash::from_bytes([0x22; 32]);
        {
            let store = RocksDbUtxoStore::open(dir.path()).unwrap();
            let cf = store.db.cf_handle(CF_UNIQUE_ID).unwrap();
            store
                .db
                .put_cf(cf, uid_key(UID_PREFIX_NFT, &id), [0u8])
                .unwrap();
            store
                .db
                .delete_cf(cf, uid_key(UID_PREFIX_NFT, &id))
                .unwrap();
        }
        {
            let store = RocksDbUtxoStore::open(dir.path()).unwrap();
            assert!(!store.has_unique_id(UID_PREFIX_NFT, &id));
        }
    }
}
