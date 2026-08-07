use std::sync::Arc;

use crypto::Hash;
use doli_core::transaction::Transaction;
use doli_core::types::{Amount, BlockHeight};
use doli_core::validation::{UtxoInfo, UtxoProvider};

use super::in_memory::InMemoryUtxoStore;
#[allow(deprecated)]
use super::types::DEFAULT_REWARD_MATURITY;
use super::types::{Outpoint, UtxoEntry};
use crate::state_db::StateDb;
use crate::StorageError;

// ============================================================================
// UtxoSet — enum dispatch between backends
// ============================================================================

/// UTXO set with pluggable backend (in-memory or state_db-backed).
///
/// ## Phase 4 — state_db is sole UTXO store
///
/// The `RocksDb` variant wraps `Arc<StateDb>` directly. The former separate
/// `utxo_store` RocksDB instance (utxo_rocks.rs) was eliminated in Phase 4.
/// All reads and writes route through state_db's cf_utxo column family.
///
/// **Apply-block writes** go through `BlockBatch` (atomic WriteBatch per block).
/// **Reads** during apply_block use the batch overlay (pending + committed state_db).
/// **RPC reads** go directly to state_db.
///
/// The `InMemory` variant is retained for snap sync deserialization and testing.
pub enum UtxoSet {
    /// HashMap-based. Used during snap sync deserialization and testing.
    InMemory(InMemoryUtxoStore),
    /// state_db-backed (production). state_db is the sole UTXO store since Phase 4.
    RocksDb(Arc<StateDb>),
}

impl UtxoSet {
    /// Create a new empty in-memory UTXO set (default for backward compatibility)
    pub fn new() -> Self {
        UtxoSet::InMemory(InMemoryUtxoStore::new())
    }

    /// Create a state_db-backed UTXO set (production path since Phase 4).
    pub fn from_state_db(state_db: Arc<StateDb>) -> Self {
        UtxoSet::RocksDb(state_db)
    }

    /// Load from legacy bincode file (utxo.bin). Returns InMemory backend.
    pub fn load(path: &std::path::Path) -> Result<Self, StorageError> {
        Ok(UtxoSet::InMemory(InMemoryUtxoStore::load(path)?))
    }

    /// Save to legacy bincode file. Only works for InMemory backend.
    /// RocksDB backend is already durable — this is a no-op.
    pub fn save(&self, path: &std::path::Path) -> Result<(), StorageError> {
        match self {
            UtxoSet::InMemory(store) => store.save(path),
            UtxoSet::RocksDb(..) => Ok(()), // state_db is always durable
        }
    }

    /// Empty the UTXO set. Post-condition: after `Ok(())` the set is empty on
    /// EITHER backend — `iter_all()` is empty, `len() == 0`, and no
    /// pubkey-index row survives. `InMemory` clears the HashMap; `RocksDb`
    /// delegates to [`StateDb::clear_utxos`], which deletes `cf_utxo` AND
    /// `cf_utxo_by_pubkey` in one `WriteBatch` and resets `utxo_count` to 0
    /// (INV-GUARD-001).
    ///
    /// SCOPE: the unique-id index (`cf_unique_id` on `RocksDb`, `unique_ids` on
    /// `InMemory` — see [`Self::has_unique_id`]) is deliberately NOT cleared by
    /// either backend. It is not rolled back on any path (`UndoData` carries no
    /// unique-id field), so wiping it here would make already-minted NFT/asset/
    /// pool ids re-mintable. Individual rows ARE deleted — `BlockBatch` removes
    /// exactly one id per spend of an id-bearing output (`spend_utxo` calls
    /// `remove_unique_id_for_entry` at `state_db/batch.rs:104`, which reaches
    /// `remove_pending_unique_id`'s `batch.delete_cf` at `batch.rs:236-242`) —
    /// but that is the mint's mirror image, one id at a time, and NO path
    /// deletes the index WHOLESALE: `StateDb::clear_utxos` (`writes.rs:89-105`)
    /// batches deletes over `cf_utxo` and `cf_utxo_by_pubkey` only, and
    /// `atomic_replace` likewise omits `cf_unique_id` from its `deletable_cfs`
    /// (`writes.rs:216-221`). `clear()` must not become the first such path,
    /// because the RocksDb replay (`insert_utxo`) does NOT write
    /// `cf_unique_id` — only `BlockBatch` does — so a wholesale wipe here would
    /// be unrecoverable without a full re-apply (INC-I-156 / AUDIT-P3-001,
    /// AUDIT-P3-104).
    ///
    /// On `Err` nothing was written and the set is unchanged; a caller MUST NOT
    /// proceed as if it were empty (INC-I-156 / REQ-I156-002).
    pub fn clear(&mut self) -> Result<(), StorageError> {
        match self {
            UtxoSet::InMemory(store) => {
                store.clear();
                Ok(())
            }
            UtxoSet::RocksDb(sdb) => sdb.clear_utxos(),
        }
    }

    /// RocksDB runtime metrics snapshot for Prometheus export.
    /// Returns `None` for in-memory backend (no RocksDB to scrape).
    /// For RocksDb variant, returns state_db metrics.
    pub fn metrics(&self) -> Option<crate::RocksDbMetrics> {
        match self {
            UtxoSet::InMemory(_) => None,
            UtxoSet::RocksDb(sdb) => Some(sdb.metrics()),
        }
    }

    // ==================== Read methods ====================
    // All reads route through state_db for the RocksDb variant.

    /// Get a UTXO by outpoint (returns owned value).
    pub fn get(&self, outpoint: &Outpoint) -> Option<UtxoEntry> {
        match self {
            UtxoSet::InMemory(store) => store.get(outpoint),
            UtxoSet::RocksDb(sdb) => sdb.get_utxo(outpoint),
        }
    }

    /// Check if a UTXO exists.
    pub fn contains(&self, outpoint: &Outpoint) -> bool {
        match self {
            UtxoSet::InMemory(store) => store.contains(outpoint),
            UtxoSet::RocksDb(sdb) => sdb.get_utxo(outpoint).is_some(),
        }
    }

    /// Get all UTXOs for a given pubkey hash (returns owned entries).
    pub fn get_by_pubkey_hash(&self, pubkey_hash: &Hash) -> Vec<(Outpoint, UtxoEntry)> {
        match self {
            UtxoSet::InMemory(store) => store.get_by_pubkey_hash(pubkey_hash),
            UtxoSet::RocksDb(sdb) => sdb.get_utxos_by_pubkey(pubkey_hash),
        }
    }

    /// Check if a unique ID exists in the index.
    pub fn has_unique_id(&self, prefix: u8, id: &Hash) -> bool {
        match self {
            Self::InMemory(store) => store.has_unique_id(prefix, id),
            Self::RocksDb(sdb) => sdb.has_unique_id(prefix, id),
        }
    }

    // ==================== Write methods ====================
    // InMemory: writes to HashMap.
    // RocksDb: writes to state_db directly (used only for rollback undo operations).
    // Normal apply_block writes go through BlockBatch, not through UtxoSet.

    /// Add outputs from a transaction, stamping Bond UTXOs with the block slot.
    ///
    /// For InMemory: adds to HashMap.
    /// For RocksDb: inserts directly into state_db (used during rollback/undo).
    pub fn add_transaction(
        &mut self,
        tx: &Transaction,
        height: BlockHeight,
        is_coinbase: bool,
        slot: u32,
    ) -> Result<(), StorageError> {
        match self {
            UtxoSet::InMemory(store) => {
                store.add_transaction(tx, height, is_coinbase, slot);
                Ok(())
            }
            UtxoSet::RocksDb(sdb) => {
                // Direct write to state_db — used only in rollback paths.
                // Normal block processing uses BlockBatch.
                for (index, output) in tx.outputs.iter().enumerate() {
                    let outpoint = Outpoint::new(tx.hash(), index as u32);
                    let mut entry = UtxoEntry {
                        output: output.clone(),
                        height,
                        is_coinbase,
                        is_epoch_reward: false,
                    };
                    // Stamp Bond UTXOs with creation slot
                    if entry.output.output_type == doli_core::OutputType::Bond
                        && entry.output.extra_data.len() >= 4
                    {
                        entry.output.extra_data[..4].copy_from_slice(&slot.to_le_bytes());
                    }
                    sdb.insert_utxo(&outpoint, &entry)?;
                }
                Ok(())
            }
        }
    }

    /// Remove inputs spent by a transaction
    pub fn spend_transaction(&mut self, tx: &Transaction) -> Result<Amount, StorageError> {
        match self {
            UtxoSet::InMemory(store) => store.spend_transaction(tx),
            UtxoSet::RocksDb(sdb) => {
                let mut total_spent = 0u64;
                for input in &tx.inputs {
                    let outpoint = Outpoint::new(input.prev_tx_hash, input.output_index);
                    if let Some(entry) = sdb.get_utxo(&outpoint) {
                        total_spent = total_spent.saturating_add(entry.output.amount);
                        sdb.remove_utxo(&outpoint)?;
                    }
                }
                Ok(total_spent)
            }
        }
    }

    /// Insert a UTXO entry directly (for testing and reorgs)
    pub fn insert(&mut self, outpoint: Outpoint, entry: UtxoEntry) -> Result<(), StorageError> {
        match self {
            UtxoSet::InMemory(store) => {
                store.insert(outpoint, entry);
                Ok(())
            }
            UtxoSet::RocksDb(sdb) => {
                sdb.insert_utxo(&outpoint, &entry)?;
                Ok(())
            }
        }
    }

    /// Remove a UTXO entry directly (for testing and reorgs)
    pub fn remove(&mut self, outpoint: &Outpoint) -> Result<Option<UtxoEntry>, StorageError> {
        match self {
            UtxoSet::InMemory(store) => Ok(store.remove(outpoint)),
            UtxoSet::RocksDb(sdb) => {
                if let Some(entry) = sdb.get_utxo(outpoint) {
                    sdb.remove_utxo(outpoint)?;
                    Ok(Some(entry))
                } else {
                    Ok(None)
                }
            }
        }
    }

    // ==================== Query methods ====================

    /// Get total value in the UTXO set.
    pub fn total_value(&self) -> Amount {
        match self {
            UtxoSet::InMemory(store) => store.total_value(),
            UtxoSet::RocksDb(sdb) => sdb.utxo_total_value(),
        }
    }

    /// Get number of UTXOs.
    pub fn len(&self) -> usize {
        match self {
            UtxoSet::InMemory(store) => store.len(),
            UtxoSet::RocksDb(sdb) => sdb.utxo_len(),
        }
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Total confirmed (spendable) DOLI excluding bonds and reward pool.
    pub fn total_confirmed(&self, height: u64, coinbase_maturity: u64, pool_pkh: &[u8; 32]) -> u64 {
        match self {
            UtxoSet::InMemory(store) => store.total_confirmed(height, coinbase_maturity, pool_pkh),
            UtxoSet::RocksDb(sdb) => sdb.total_confirmed(height, coinbase_maturity, pool_pkh),
        }
    }

    /// Convenience aliases for chain stats
    pub fn total_supply(&self) -> u64 {
        self.total_value()
    }

    pub fn utxo_count(&self) -> u64 {
        self.len() as u64
    }

    /// Iterate all UTXOs as (Outpoint, UtxoEntry) pairs.
    pub fn iter_all(&self) -> Vec<(Outpoint, UtxoEntry)> {
        match self {
            UtxoSet::InMemory(store) => store.iter().map(|(o, e)| (*o, e.clone())).collect(),
            UtxoSet::RocksDb(sdb) => sdb.iter_utxos(),
        }
    }

    /// Count unique addresses (pubkey hashes) in the UTXO set.
    pub fn address_count(&self) -> u64 {
        match self {
            UtxoSet::InMemory(store) => {
                let addrs: std::collections::HashSet<_> =
                    store.iter().map(|(_, e)| e.output.pubkey_hash).collect();
                addrs.len() as u64
            }
            UtxoSet::RocksDb(sdb) => sdb.address_count(),
        }
    }

    /// Get the current Pool UTXO by pool ID.
    /// Pool outputs use pubkey_hash = pool_id, so this is just a filtered lookup.
    pub fn get_pool_utxo(&self, pool_id: &Hash) -> Option<(Outpoint, UtxoEntry)> {
        self.get_by_pubkey_hash(pool_id)
            .into_iter()
            .find(|(_, entry)| entry.output.output_type == doli_core::OutputType::Pool)
    }

    /// Get all active Pool UTXOs.
    pub fn get_all_pools(&self) -> Vec<(Outpoint, UtxoEntry)> {
        match self {
            Self::InMemory(store) => store.get_all_pools(),
            Self::RocksDb(sdb) => sdb.get_all_pools(),
        }
    }

    /// Inputs for the D4 AC-6 bond-to-TVL economic-security metric.
    ///
    /// Returns `(total_active_bonds, max_pool)` where:
    ///   - `total_active_bonds` is the sum of every `OutputType::Bond` UTXO
    ///     amount, saturated at `u64::MAX` (u128 accumulator).
    ///   - `max_pool` is `Some((pool_id, tvl_doli))` for the pool with the
    ///     largest DOLI-denominated TVL using the Phase-1 self-referential
    ///     numeraire (`tvl = 2 * reserve_a`), deduplicated by `pool_id`
    ///     (max `reserve_a` wins). `None` when no Pool UTXOs exist.
    ///
    /// Read-only. Never mutates state. Spec: `specs/defi-subsystem-architecture.md`
    /// Acceptance Criteria block (AC-6) and `specs/defi-foundations-economics.md`
    /// S9 Decision 4 (ACCEPTED 2026-05-29).
    pub fn defi_health_inputs(&self) -> (u64, Option<(Hash, u64)>) {
        use doli_core::OutputType;

        let total_bonds_u128: u128 = self
            .iter_all()
            .into_iter()
            .filter(|(_, entry)| entry.output.output_type == OutputType::Bond)
            .map(|(_, entry)| entry.output.amount as u128)
            .sum();
        let total_active_bonds = if total_bonds_u128 > u64::MAX as u128 {
            u64::MAX
        } else {
            total_bonds_u128 as u64
        };

        let mut best_per_id: std::collections::HashMap<Hash, u64> =
            std::collections::HashMap::new();
        for (_outpoint, entry) in self.get_all_pools() {
            if let Some(meta) = entry.output.pool_metadata() {
                let cur = best_per_id.entry(meta.pool_id).or_insert(0);
                if meta.reserve_a > *cur {
                    *cur = meta.reserve_a;
                }
            }
        }
        let mut best: Option<(Hash, u64)> = None;
        for (pool_id, reserve_a) in best_per_id {
            let tvl_u128 = (reserve_a as u128).saturating_mul(2);
            let tvl = if tvl_u128 > u64::MAX as u128 {
                u64::MAX
            } else {
                tvl_u128 as u64
            };
            if best.map(|(_, t)| tvl > t).unwrap_or(true) {
                best = Some((pool_id, tvl));
            }
        }

        (total_active_bonds, best)
    }

    /// Get spendable balance for a pubkey hash at a given height with default maturity
    #[allow(deprecated)]
    pub fn get_balance(&self, pubkey_hash: &Hash, height: BlockHeight) -> Amount {
        self.get_balance_with_maturity(pubkey_hash, height, DEFAULT_REWARD_MATURITY)
    }

    /// Get spendable balance for a pubkey hash at a given height with custom maturity.
    pub fn get_balance_with_maturity(
        &self,
        pubkey_hash: &Hash,
        height: BlockHeight,
        maturity: BlockHeight,
    ) -> Amount {
        match self {
            UtxoSet::InMemory(store) => {
                store.get_balance_with_maturity(pubkey_hash, height, maturity)
            }
            UtxoSet::RocksDb(sdb) => sdb.get_balance_with_maturity(pubkey_hash, height, maturity),
        }
    }

    /// Get immature balance for a pubkey hash at a given height with default maturity
    #[allow(deprecated)]
    pub fn get_immature_balance(&self, pubkey_hash: &Hash, height: BlockHeight) -> Amount {
        self.get_immature_balance_with_maturity(pubkey_hash, height, DEFAULT_REWARD_MATURITY)
    }

    /// Get immature balance for a pubkey hash with custom maturity.
    pub fn get_immature_balance_with_maturity(
        &self,
        pubkey_hash: &Hash,
        height: BlockHeight,
        maturity: BlockHeight,
    ) -> Amount {
        match self {
            UtxoSet::InMemory(store) => {
                store.get_immature_balance_with_maturity(pubkey_hash, height, maturity)
            }
            UtxoSet::RocksDb(sdb) => {
                sdb.get_immature_balance_with_maturity(pubkey_hash, height, maturity)
            }
        }
    }

    /// Get bonded balance (sum of Bond UTXOs for this address).
    pub fn get_bonded_balance(&self, pubkey_hash: &Hash) -> Amount {
        match self {
            UtxoSet::InMemory(store) => store.get_bonded_balance(pubkey_hash),
            UtxoSet::RocksDb(sdb) => sdb.get_bonded_balance(pubkey_hash),
        }
    }

    /// Count bond units for this address (total bond amount / bond_unit).
    pub fn count_bonds(&self, pubkey_hash: &Hash, bond_unit: u64) -> u32 {
        match self {
            UtxoSet::InMemory(store) => store.count_bonds(pubkey_hash, bond_unit),
            UtxoSet::RocksDb(sdb) => sdb.count_bonds(pubkey_hash, bond_unit),
        }
    }

    /// Get bond details: (outpoint, creation_slot, amount) for each Bond UTXO, FIFO-ordered.
    pub fn get_bond_entries(&self, pubkey_hash: &Hash) -> Vec<(Outpoint, u32, Amount)> {
        match self {
            UtxoSet::InMemory(store) => store.get_bond_entries(pubkey_hash),
            UtxoSet::RocksDb(sdb) => sdb.get_bond_entries(pubkey_hash),
        }
    }

    /// Produce canonical bytes for deterministic state root computation.
    ///
    /// Both backends produce identical output for the same UTXO set:
    /// `[8-byte LE count] [sorted_key1][value1] [sorted_key2][value2] ...`
    ///
    /// Consensus-critical — Phase 1 equivalence tests proved bit-identity.
    pub fn serialize_canonical(&self) -> Vec<u8> {
        match self {
            UtxoSet::InMemory(store) => store.serialize_canonical(),
            UtxoSet::RocksDb(sdb) => sdb.serialize_canonical_utxo(),
        }
    }

    /// Deserialize a UtxoSet from canonical bytes (inverse of `serialize_canonical`).
    ///
    /// Always produces an InMemory backend (sufficient for state root verification).
    /// Format: `[8-byte LE count] [36-byte outpoint][entry_bytes] ...`
    pub fn deserialize_canonical(bytes: &[u8]) -> Result<Self, StorageError> {
        if bytes.len() < 8 {
            return Err(StorageError::Serialization(format!(
                "[STOR027] UTXO canonical bytes too short: {} bytes (min 8)",
                bytes.len()
            )));
        }
        let count = u64::from_le_bytes(bytes[0..8].try_into().unwrap()) as usize;
        let mut store = InMemoryUtxoStore::new();
        let mut pos = 8;

        for _ in 0..count {
            // Read 36-byte outpoint
            if pos + 36 > bytes.len() {
                return Err(StorageError::Serialization(format!(
                    "[STOR028] UTXO canonical bytes truncated at outpoint (pos={}, len={})",
                    pos,
                    bytes.len()
                )));
            }
            let outpoint = Outpoint::from_bytes(&bytes[pos..pos + 36]).ok_or_else(|| {
                StorageError::Serialization(format!(
                    "[STOR029] invalid outpoint in canonical bytes at pos={}",
                    pos
                ))
            })?;
            pos += 36;

            // Read entry (variable length: min 61 bytes)
            if pos + 61 > bytes.len() {
                return Err(StorageError::Serialization(format!(
                    "[STOR030] UTXO canonical bytes truncated at entry (pos={}, len={})",
                    pos,
                    bytes.len()
                )));
            }
            // Peek at extra_data length to determine total entry size
            // 0xFFFF marker -> u32 length follows (large NFTs >64KB)
            let raw_len = u16::from_le_bytes(bytes[pos + 59..pos + 61].try_into().unwrap());
            let (extra_len, header_overhead) = if raw_len == 0xFFFF {
                if pos + 65 > bytes.len() {
                    return Err(StorageError::Serialization(
                        "UTXO canonical bytes truncated (u32 length)".to_string(),
                    ));
                }
                let len =
                    u32::from_le_bytes(bytes[pos + 61..pos + 65].try_into().unwrap()) as usize;
                (len, 65) // 61 base + 4 bytes u32
            } else {
                (raw_len as usize, 61) // 59 base + 2 bytes u16
            };
            let entry_size = header_overhead + extra_len;
            if pos + entry_size > bytes.len() {
                return Err(StorageError::Serialization(format!(
                    "[STOR031] UTXO canonical bytes truncated at extra_data (pos={}, entry_size={}, len={})",
                    pos, entry_size, bytes.len()
                )));
            }
            let entry = UtxoEntry::deserialize_canonical_bytes(&bytes[pos..pos + entry_size])
                .ok_or_else(|| {
                    StorageError::Serialization(format!(
                        "[STOR032] invalid UTXO entry in canonical bytes at pos={} (entry_size={})",
                        pos, entry_size
                    ))
                })?;
            pos += entry_size;

            store.insert(outpoint, entry);
        }

        Ok(UtxoSet::InMemory(store))
    }

    /// Find an NFT UTXO by its token ID.
    /// Returns (outpoint, utxo_entry) if found in the current UTXO set.
    pub fn find_nft_by_token_id(&self, token_id: &Hash) -> Option<(Outpoint, UtxoEntry)> {
        use super::types::UID_PREFIX_NFT;
        match self {
            Self::InMemory(store) => {
                // Fast check: if the token ID isn't in the unique_ids index, it doesn't exist
                if !store.has_unique_id(UID_PREFIX_NFT, token_id) {
                    return None;
                }
                for (outpoint, entry) in store.iter() {
                    if entry.output.output_type == doli_core::OutputType::NFT {
                        if let Some((tid, _)) = entry.output.nft_metadata() {
                            if &tid == token_id {
                                return Some((*outpoint, entry.clone()));
                            }
                        }
                    }
                }
                None
            }
            Self::RocksDb(sdb) => {
                if !sdb.has_unique_id(UID_PREFIX_NFT, token_id) {
                    return None;
                }
                sdb.find_nft_by_token_id(token_id)
            }
        }
    }

    /// Check if this is the RocksDB backend
    pub fn is_rocksdb(&self) -> bool {
        matches!(self, UtxoSet::RocksDb(..))
    }
}

impl UtxoProvider for UtxoSet {
    fn get_utxo(&self, tx_hash: &Hash, output_index: u32) -> Option<UtxoInfo> {
        let outpoint = Outpoint::new(*tx_hash, output_index);
        self.get(&outpoint).map(|entry| UtxoInfo {
            output: entry.output,
            pubkey: None, // pay-to-pubkey-hash -- signature verification uses the input's pubkey
            spent: false, // present in UTXO set = unspent (spent entries are removed)
        })
    }
}

impl Default for UtxoSet {
    fn default() -> Self {
        Self::new()
    }
}
