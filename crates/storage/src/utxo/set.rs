use std::path::Path;

use crypto::Hash;
use doli_core::transaction::Transaction;
use doli_core::types::{Amount, BlockHeight};
use doli_core::validation::{UtxoInfo, UtxoProvider};

use super::in_memory::InMemoryUtxoStore;
#[allow(deprecated)]
use super::types::DEFAULT_REWARD_MATURITY;
use super::types::{Outpoint, UtxoEntry};
use crate::utxo_rocks::RocksDbUtxoStore;
use crate::StorageError;

// ============================================================================
// UtxoSet — enum dispatch between backends
// ============================================================================

/// UTXO set with pluggable backend (in-memory or RocksDB)
pub enum UtxoSet {
    /// HashMap-based (original). Used during migration and testing.
    InMemory(InMemoryUtxoStore),
    /// RocksDB-backed (production). Durable, scales to millions of entries.
    RocksDb(RocksDbUtxoStore),
}

impl UtxoSet {
    /// Create a new empty in-memory UTXO set (default for backward compatibility)
    pub fn new() -> Self {
        UtxoSet::InMemory(InMemoryUtxoStore::new())
    }

    /// Open a RocksDB-backed UTXO set at the given path
    pub fn open_rocksdb(path: &Path) -> Result<Self, StorageError> {
        Ok(UtxoSet::RocksDb(RocksDbUtxoStore::open(path)?))
    }

    /// Load from legacy bincode file (utxo.bin). Returns InMemory backend.
    pub fn load(path: &Path) -> Result<Self, StorageError> {
        Ok(UtxoSet::InMemory(InMemoryUtxoStore::load(path)?))
    }

    /// Save to legacy bincode file. Only works for InMemory backend.
    /// RocksDB backend is already durable — this is a no-op.
    pub fn save(&self, path: &Path) -> Result<(), StorageError> {
        match self {
            UtxoSet::InMemory(store) => store.save(path),
            UtxoSet::RocksDb(_) => Ok(()), // RocksDB is always durable
        }
    }

    /// Clear all UTXOs
    pub fn clear(&mut self) {
        match self {
            UtxoSet::InMemory(store) => store.clear(),
            UtxoSet::RocksDb(store) => store.clear(),
        }
    }

    /// Get a UTXO by outpoint (returns owned value)
    pub fn get(&self, outpoint: &Outpoint) -> Option<UtxoEntry> {
        match self {
            UtxoSet::InMemory(store) => store.get(outpoint),
            UtxoSet::RocksDb(store) => store.get(outpoint),
        }
    }

    /// Check if a UTXO exists
    pub fn contains(&self, outpoint: &Outpoint) -> bool {
        match self {
            UtxoSet::InMemory(store) => store.contains(outpoint),
            UtxoSet::RocksDb(store) => store.contains(outpoint),
        }
    }

    /// Add outputs from a transaction, stamping Bond UTXOs with the block slot
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
            UtxoSet::RocksDb(store) => store.add_transaction(tx, height, is_coinbase, slot),
        }
    }

    /// Remove inputs spent by a transaction
    pub fn spend_transaction(&mut self, tx: &Transaction) -> Result<Amount, StorageError> {
        match self {
            UtxoSet::InMemory(store) => store.spend_transaction(tx),
            UtxoSet::RocksDb(store) => store.spend_transaction(tx),
        }
    }

    /// Get total value in the UTXO set
    pub fn total_value(&self) -> Amount {
        match self {
            UtxoSet::InMemory(store) => store.total_value(),
            UtxoSet::RocksDb(store) => store.total_value(),
        }
    }

    /// Get number of UTXOs
    pub fn len(&self) -> usize {
        match self {
            UtxoSet::InMemory(store) => store.len(),
            UtxoSet::RocksDb(store) => store.len(),
        }
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        match self {
            UtxoSet::InMemory(store) => store.is_empty(),
            UtxoSet::RocksDb(store) => store.is_empty(),
        }
    }

    /// Convenience aliases for chain stats
    pub fn total_supply(&self) -> u64 {
        self.total_value()
    }

    pub fn utxo_count(&self) -> u64 {
        self.len() as u64
    }

    /// Count unique addresses (pubkey hashes) in the UTXO set
    pub fn address_count(&self) -> u64 {
        match self {
            UtxoSet::InMemory(store) => {
                let addrs: std::collections::HashSet<_> =
                    store.iter().map(|(_, e)| e.output.pubkey_hash).collect();
                addrs.len() as u64
            }
            UtxoSet::RocksDb(store) => store.address_count(),
        }
    }

    /// Get all UTXOs for a given pubkey hash (returns owned entries)
    pub fn get_by_pubkey_hash(&self, pubkey_hash: &Hash) -> Vec<(Outpoint, UtxoEntry)> {
        match self {
            UtxoSet::InMemory(store) => store.get_by_pubkey_hash(pubkey_hash),
            UtxoSet::RocksDb(store) => store.get_by_pubkey_hash(pubkey_hash),
        }
    }

    /// Get spendable balance for a pubkey hash at a given height with default maturity
    #[allow(deprecated)]
    pub fn get_balance(&self, pubkey_hash: &Hash, height: BlockHeight) -> Amount {
        self.get_balance_with_maturity(pubkey_hash, height, DEFAULT_REWARD_MATURITY)
    }

    /// Get spendable balance for a pubkey hash at a given height with custom maturity
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
            UtxoSet::RocksDb(store) => {
                store.get_balance_with_maturity(pubkey_hash, height, maturity)
            }
        }
    }

    /// Get immature balance for a pubkey hash at a given height with default maturity
    #[allow(deprecated)]
    pub fn get_immature_balance(&self, pubkey_hash: &Hash, height: BlockHeight) -> Amount {
        self.get_immature_balance_with_maturity(pubkey_hash, height, DEFAULT_REWARD_MATURITY)
    }

    /// Get immature balance for a pubkey hash with custom maturity
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
            UtxoSet::RocksDb(store) => {
                store.get_immature_balance_with_maturity(pubkey_hash, height, maturity)
            }
        }
    }

    /// Get bonded balance (sum of Bond UTXOs for this address)
    pub fn get_bonded_balance(&self, pubkey_hash: &Hash) -> Amount {
        match self {
            UtxoSet::InMemory(store) => store.get_bonded_balance(pubkey_hash),
            UtxoSet::RocksDb(store) => store.get_bonded_balance(pubkey_hash),
        }
    }

    /// Count bond units for this address (total bond amount / bond_unit)
    pub fn count_bonds(&self, pubkey_hash: &Hash, bond_unit: u64) -> u32 {
        match self {
            UtxoSet::InMemory(store) => store.count_bonds(pubkey_hash, bond_unit),
            UtxoSet::RocksDb(store) => store.count_bonds(pubkey_hash, bond_unit),
        }
    }

    /// Get bond details: (outpoint, creation_slot, amount) for each Bond UTXO, FIFO-ordered
    pub fn get_bond_entries(&self, pubkey_hash: &Hash) -> Vec<(Outpoint, u32, Amount)> {
        match self {
            UtxoSet::InMemory(store) => store.get_bond_entries(pubkey_hash),
            UtxoSet::RocksDb(store) => store.get_bond_entries(pubkey_hash),
        }
    }

    /// Insert a UTXO entry directly (for testing and reorgs)
    pub fn insert(&mut self, outpoint: Outpoint, entry: UtxoEntry) -> Result<(), StorageError> {
        match self {
            UtxoSet::InMemory(store) => {
                store.insert(outpoint, entry);
                Ok(())
            }
            UtxoSet::RocksDb(store) => store.insert(outpoint, entry),
        }
    }

    /// Remove a UTXO entry directly (for testing and reorgs)
    pub fn remove(&mut self, outpoint: &Outpoint) -> Result<Option<UtxoEntry>, StorageError> {
        match self {
            UtxoSet::InMemory(store) => Ok(store.remove(outpoint)),
            UtxoSet::RocksDb(store) => store.remove(outpoint),
        }
    }

    /// Produce canonical bytes for deterministic state root computation.
    ///
    /// Both backends produce identical output for the same UTXO set:
    /// `[8-byte LE count] [sorted_key1][value1] [sorted_key2][value2] ...`
    pub fn serialize_canonical(&self) -> Vec<u8> {
        match self {
            UtxoSet::InMemory(store) => store.serialize_canonical(),
            UtxoSet::RocksDb(store) => store.serialize_canonical(),
        }
    }

    /// Check if this is the RocksDB backend
    pub fn is_rocksdb(&self) -> bool {
        matches!(self, UtxoSet::RocksDb(_))
    }
}

impl UtxoProvider for UtxoSet {
    fn get_utxo(&self, tx_hash: &Hash, output_index: u32) -> Option<UtxoInfo> {
        let outpoint = Outpoint::new(*tx_hash, output_index);
        self.get(&outpoint).map(|entry| UtxoInfo {
            output: entry.output,
            pubkey: None, // pay-to-pubkey-hash — signature verification uses the input's pubkey
            spent: false, // present in UTXO set = unspent (spent entries are removed)
        })
    }
}

impl Default for UtxoSet {
    fn default() -> Self {
        Self::new()
    }
}
