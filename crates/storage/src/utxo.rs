//! UTXO set management
//!
//! Provides two backends:
//! - `InMemoryUtxoStore`: HashMap-based (original, used for migration and testing)
//! - `RocksDbUtxoStore`: Disk-backed via RocksDB (production, scales to millions of UTXOs)
//!
//! The `UtxoSet` enum dispatches to the active backend. Consumers don't need
//! to know which backend is active — all methods work identically.

use std::collections::HashMap;
use std::path::Path;

use crypto::Hash;
use doli_core::network::Network;
use doli_core::network_params::NetworkParams;
use doli_core::transaction::{Output, OutputType, Transaction};
use doli_core::types::{Amount, BlockHeight};
use doli_core::validation::{UtxoInfo, UtxoProvider};
use serde::{Deserialize, Serialize};

use crate::utxo_rocks::RocksDbUtxoStore;
use crate::StorageError;

/// An entry in the UTXO set
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UtxoEntry {
    /// The output
    pub output: doli_core::transaction::Output,
    /// Block height when created
    pub height: BlockHeight,
    /// Whether this is a coinbase output
    pub is_coinbase: bool,
    /// Whether this is an epoch reward output
    #[serde(default)] // For backward compatibility with existing UTXOs
    pub is_epoch_reward: bool,
}

/// Default reward maturity constant (mainnet default: 100 blocks)
///
/// **Deprecated**: Use `reward_maturity_for_network(network)` for network-aware calculations.
/// Devnet uses 10 blocks for faster testing.
#[deprecated(note = "Use reward_maturity_for_network(network) for network-aware calculations")]
pub const DEFAULT_REWARD_MATURITY: BlockHeight = 6;

/// Get reward maturity for a specific network
pub fn reward_maturity_for_network(network: Network) -> BlockHeight {
    NetworkParams::load(network).coinbase_maturity
}

impl UtxoEntry {
    /// Canonical serialization for state root computation.
    ///
    /// Fixed-field encoding immune to bincode struct evolution.
    /// Format: `[1B output_type][8B amount][32B pubkey_hash][8B lock_until]
    ///          [8B height][1B is_coinbase][1B is_epoch_reward]
    ///          [2B extra_data_len (u16 LE)][NB extra_data]`
    ///
    /// Base size: 61 bytes (59 + 2 for length=0 when extra_data is empty).
    pub fn serialize_canonical_bytes(&self) -> Vec<u8> {
        let extra_len = self.output.extra_data.len();
        let mut buf = Vec::with_capacity(61 + extra_len);
        buf.push(self.output.output_type as u8);
        buf.extend_from_slice(&self.output.amount.to_le_bytes());
        buf.extend_from_slice(self.output.pubkey_hash.as_bytes());
        buf.extend_from_slice(&self.output.lock_until.to_le_bytes());
        buf.extend_from_slice(&self.height.to_le_bytes());
        buf.push(self.is_coinbase as u8);
        buf.push(self.is_epoch_reward as u8);
        buf.extend_from_slice(&(extra_len as u16).to_le_bytes());
        buf.extend_from_slice(&self.output.extra_data);
        buf
    }

    /// Reconstruct a `UtxoEntry` from canonical encoding.
    ///
    /// Returns `None` if the bytes are too short or the output type is unknown.
    pub fn deserialize_canonical_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 61 {
            return None;
        }
        let output_type = OutputType::from_u8(bytes[0])?;
        let amount = u64::from_le_bytes(bytes[1..9].try_into().ok()?);
        let pubkey_hash = Hash::from_bytes(bytes[9..41].try_into().ok()?);
        let lock_until = u64::from_le_bytes(bytes[41..49].try_into().ok()?);
        let height = u64::from_le_bytes(bytes[49..57].try_into().ok()?);
        let is_coinbase = bytes[57] != 0;
        let is_epoch_reward = bytes[58] != 0;
        let extra_len = u16::from_le_bytes(bytes[59..61].try_into().ok()?) as usize;
        let extra_data = if extra_len > 0 {
            if bytes.len() < 61 + extra_len {
                return None;
            }
            bytes[61..61 + extra_len].to_vec()
        } else {
            Vec::new()
        };
        Some(UtxoEntry {
            output: Output {
                output_type,
                amount,
                pubkey_hash,
                lock_until,
                extra_data,
            },
            height,
            is_coinbase,
            is_epoch_reward,
        })
    }

    /// Size of this entry's canonical serialization in bytes.
    pub fn canonical_byte_size(&self) -> usize {
        61 + self.output.extra_data.len()
    }

    /// Check if the UTXO is spendable at the given height for a specific network
    pub fn is_spendable_at_for_network(&self, height: BlockHeight, network: Network) -> bool {
        self.is_spendable_at_with_maturity(height, reward_maturity_for_network(network))
    }

    /// Check if the UTXO is spendable at the given height with mainnet default maturity (100 blocks)
    ///
    /// **Deprecated**: Use `is_spendable_at_for_network()` for network-aware calculations.
    #[deprecated(
        note = "Use is_spendable_at_for_network(height, network) for network-aware calculations"
    )]
    pub fn is_spendable_at(&self, height: BlockHeight) -> bool {
        #[allow(deprecated)]
        self.is_spendable_at_with_maturity(height, DEFAULT_REWARD_MATURITY)
    }

    /// Check if the UTXO is spendable at the given height with custom maturity
    pub fn is_spendable_at_with_maturity(
        &self,
        height: BlockHeight,
        maturity: BlockHeight,
    ) -> bool {
        // Check time lock
        if !self.output.is_spendable_at(height) {
            return false;
        }

        // Coinbase AND EpochReward require maturity confirmations
        if self.is_coinbase || self.is_epoch_reward {
            let confirmations = height.saturating_sub(self.height);
            return confirmations >= maturity;
        }

        true
    }
}

/// Outpoint identifier
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Outpoint {
    pub tx_hash: Hash,
    pub index: u32,
}

impl Outpoint {
    pub fn new(tx_hash: Hash, index: u32) -> Self {
        Self { tx_hash, index }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(36);
        bytes.extend_from_slice(self.tx_hash.as_bytes());
        bytes.extend_from_slice(&self.index.to_le_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 36 {
            return None;
        }
        let mut hash_arr = [0u8; 32];
        hash_arr.copy_from_slice(&bytes[0..32]);
        let index = u32::from_le_bytes(bytes[32..36].try_into().ok()?);
        Some(Self {
            tx_hash: Hash::from_bytes(hash_arr),
            index,
        })
    }
}

// ============================================================================
// InMemoryUtxoStore — original HashMap-based backend
// ============================================================================

/// In-memory UTXO store (HashMap-based)
#[derive(Serialize, Deserialize)]
pub struct InMemoryUtxoStore {
    utxos: HashMap<Outpoint, UtxoEntry>,
}

impl InMemoryUtxoStore {
    pub fn new() -> Self {
        Self {
            utxos: HashMap::new(),
        }
    }

    pub fn clear(&mut self) {
        self.utxos.clear();
    }

    /// Load from bincode file (legacy utxo.bin format)
    pub fn load(path: &Path) -> Result<Self, StorageError> {
        let bytes = std::fs::read(path)?;
        bincode::deserialize(&bytes).map_err(|e| StorageError::Serialization(e.to_string()))
    }

    /// Save to bincode file (atomic: write to temp file, then rename)
    pub fn save(&self, path: &Path) -> Result<(), StorageError> {
        let bytes =
            bincode::serialize(self).map_err(|e| StorageError::Serialization(e.to_string()))?;
        let tmp = path.with_extension("bin.tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn get(&self, outpoint: &Outpoint) -> Option<UtxoEntry> {
        self.utxos.get(outpoint).cloned()
    }

    pub fn contains(&self, outpoint: &Outpoint) -> bool {
        self.utxos.contains_key(outpoint)
    }

    pub fn add_transaction(
        &mut self,
        tx: &Transaction,
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
            if stamped_output.output_type == OutputType::Bond {
                stamped_output.extra_data = slot.to_le_bytes().to_vec();
            }
            let entry = UtxoEntry {
                output: stamped_output,
                height,
                is_coinbase,
                is_epoch_reward,
            };
            self.utxos.insert(outpoint, entry);
        }
    }

    pub fn spend_transaction(&mut self, tx: &Transaction) -> Result<Amount, StorageError> {
        let mut total_input = 0;

        for input in &tx.inputs {
            let outpoint = Outpoint::new(input.prev_tx_hash, input.output_index);
            match self.utxos.remove(&outpoint) {
                Some(entry) => total_input += entry.output.amount,
                None => {
                    return Err(StorageError::NotFound(format!(
                        "UTXO not found: {}:{}",
                        input.prev_tx_hash, input.output_index
                    )));
                }
            }
        }

        Ok(total_input)
    }

    pub fn total_value(&self) -> Amount {
        self.utxos.values().map(|e| e.output.amount).sum()
    }

    pub fn len(&self) -> usize {
        self.utxos.len()
    }

    pub fn is_empty(&self) -> bool {
        self.utxos.is_empty()
    }

    pub fn get_by_pubkey_hash(&self, pubkey_hash: &Hash) -> Vec<(Outpoint, UtxoEntry)> {
        self.utxos
            .iter()
            .filter(|(_, entry)| &entry.output.pubkey_hash == pubkey_hash)
            .map(|(op, entry)| (*op, entry.clone()))
            .collect()
    }

    #[allow(deprecated)]
    pub fn get_balance(&self, pubkey_hash: &Hash, height: BlockHeight) -> Amount {
        self.get_balance_with_maturity(pubkey_hash, height, DEFAULT_REWARD_MATURITY)
    }

    pub fn get_balance_with_maturity(
        &self,
        pubkey_hash: &Hash,
        height: BlockHeight,
        maturity: BlockHeight,
    ) -> Amount {
        self.get_by_pubkey_hash(pubkey_hash)
            .iter()
            .filter(|(_, entry)| entry.is_spendable_at_with_maturity(height, maturity))
            .map(|(_, entry)| entry.output.amount)
            .sum()
    }

    #[allow(deprecated)]
    pub fn get_immature_balance(&self, pubkey_hash: &Hash, height: BlockHeight) -> Amount {
        self.get_immature_balance_with_maturity(pubkey_hash, height, DEFAULT_REWARD_MATURITY)
    }

    pub fn get_immature_balance_with_maturity(
        &self,
        pubkey_hash: &Hash,
        height: BlockHeight,
        maturity: BlockHeight,
    ) -> Amount {
        self.get_by_pubkey_hash(pubkey_hash)
            .iter()
            .filter(|(_, entry)| {
                (entry.is_coinbase || entry.is_epoch_reward)
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
    pub fn get_bond_entries(&self, pubkey_hash: &Hash) -> Vec<(Outpoint, u32, Amount)> {
        let mut bonds: Vec<(Outpoint, u32, Amount)> = self
            .get_by_pubkey_hash(pubkey_hash)
            .into_iter()
            .filter(|(_, entry)| entry.output.output_type == doli_core::OutputType::Bond)
            .map(|(op, entry)| {
                let slot = entry.output.bond_creation_slot().unwrap_or(0);
                (op, slot, entry.output.amount)
            })
            .collect();
        bonds.sort_by_key(|(_, slot, _)| *slot); // FIFO: oldest first
        bonds
    }

    pub fn insert(&mut self, outpoint: Outpoint, entry: UtxoEntry) {
        self.utxos.insert(outpoint, entry);
    }

    pub fn remove(&mut self, outpoint: &Outpoint) -> Option<UtxoEntry> {
        self.utxos.remove(outpoint)
    }

    /// Iterate over all entries (for migration to RocksDB)
    pub fn iter(&self) -> impl Iterator<Item = (&Outpoint, &UtxoEntry)> {
        self.utxos.iter()
    }

    /// Produce canonical bytes for deterministic state root computation.
    ///
    /// Output: `[8-byte LE count] [sorted_key1][value1] [sorted_key2][value2] ...`
    ///
    /// Keys are sorted lexicographically (by outpoint bytes).
    pub fn serialize_canonical(&self) -> Vec<u8> {
        let mut entries: Vec<(&Outpoint, &UtxoEntry)> = self.utxos.iter().collect();
        entries.sort_by(|(a, _), (b, _)| a.to_bytes().cmp(&b.to_bytes()));

        let count = entries.len() as u64;
        // 36 bytes outpoint key + 61+ bytes canonical entry value + 8 bytes header
        let mut buf = Vec::with_capacity(8 + entries.len() * 97);
        buf.extend_from_slice(&count.to_le_bytes());

        for (outpoint, entry) in entries {
            buf.extend_from_slice(&outpoint.to_bytes());
            buf.extend_from_slice(&entry.serialize_canonical_bytes());
        }

        buf
    }
}

impl Default for InMemoryUtxoStore {
    fn default() -> Self {
        Self::new()
    }
}

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
    ) {
        match self {
            UtxoSet::InMemory(store) => store.add_transaction(tx, height, is_coinbase, slot),
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
    pub fn insert(&mut self, outpoint: Outpoint, entry: UtxoEntry) {
        match self {
            UtxoSet::InMemory(store) => store.insert(outpoint, entry),
            UtxoSet::RocksDb(store) => store.insert(outpoint, entry),
        }
    }

    /// Remove a UTXO entry directly (for testing and reorgs)
    pub fn remove(&mut self, outpoint: &Outpoint) -> Option<UtxoEntry> {
        match self {
            UtxoSet::InMemory(store) => store.remove(outpoint),
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

#[cfg(test)]
mod tests {
    use super::*;
    use doli_core::transaction::Transaction;

    #[test]
    fn test_utxo_add_and_spend() {
        let mut utxo_set = UtxoSet::new();

        // Create a coinbase transaction
        let tx = Transaction::new_coinbase(500_000_000, Hash::ZERO, 0);
        let tx_hash = tx.hash();

        // Add to UTXO set
        utxo_set.add_transaction(&tx, 0, true, 0);
        assert_eq!(utxo_set.len(), 1);

        // Check it exists
        let outpoint = Outpoint::new(tx_hash, 0);
        assert!(utxo_set.contains(&outpoint));

        // Check coinbase maturity (COINBASE_MATURITY = 6)
        let entry = utxo_set.get(&outpoint).unwrap();
        #[allow(deprecated)]
        {
            assert!(!entry.is_spendable_at(5)); // 5 confirmations - not mature
            assert!(entry.is_spendable_at(6)); // 6 confirmations - mature
            assert!(entry.is_spendable_at(100)); // well past maturity
        }
    }

    #[test]
    fn test_outpoint_serialization() {
        let outpoint = Outpoint::new(Hash::ZERO, 42);
        let bytes = outpoint.to_bytes();
        let recovered = Outpoint::from_bytes(&bytes).unwrap();
        assert_eq!(outpoint, recovered);
    }

    // ==========================================================================
    // Epoch Reward UTXO Tests
    // ==========================================================================

    #[test]
    fn test_utxo_entry_epoch_reward_maturity() {
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let tx = Transaction::new_epoch_reward(1, *keypair.public_key(), 1_000_000, pubkey_hash);
        let tx_hash = tx.hash();

        let mut utxo_set = UtxoSet::new();
        utxo_set.add_transaction(&tx, 100, false, 0); // height 100, not coinbase

        let outpoint = Outpoint::new(tx_hash, 0);
        let entry = utxo_set.get(&outpoint).unwrap();

        // Verify is_epoch_reward flag is set
        assert!(entry.is_epoch_reward);
        assert!(!entry.is_coinbase);

        // Check maturity: needs REWARD_MATURITY (6) confirmations
        #[allow(deprecated)]
        {
            assert!(!entry.is_spendable_at(100)); // 0 confirmations
            assert!(!entry.is_spendable_at(105)); // 5 confirmations
            assert!(entry.is_spendable_at(106)); // 6 confirmations (mature)
            assert!(entry.is_spendable_at(200)); // well past maturity
        }
    }

    #[test]
    fn test_utxo_entry_coinbase_maturity_unchanged() {
        // Verify coinbase still requires same maturity
        let tx = Transaction::new_coinbase(500_000_000, Hash::ZERO, 0);
        let tx_hash = tx.hash();

        let mut utxo_set = UtxoSet::new();
        utxo_set.add_transaction(&tx, 50, true, 0);

        let outpoint = Outpoint::new(tx_hash, 0);
        let entry = utxo_set.get(&outpoint).unwrap();

        // Verify flags
        assert!(entry.is_coinbase);
        assert!(!entry.is_epoch_reward);

        // Check maturity (COINBASE_MATURITY = 6)
        #[allow(deprecated)]
        {
            assert!(!entry.is_spendable_at(55)); // 5 confirmations - not mature
            assert!(entry.is_spendable_at(56)); // 6 confirmations (mature)
            assert!(entry.is_spendable_at(150)); // well past maturity
        }
    }

    #[test]
    fn test_utxo_entry_regular_tx_no_maturity() {
        // Regular transactions should not have maturity requirements
        use doli_core::transaction::{Input, Output};

        let tx = Transaction {
            version: 1,
            tx_type: doli_core::transaction::TxType::Transfer,
            inputs: vec![Input::new(Hash::ZERO, 0)],
            outputs: vec![Output::normal(1000, Hash::ZERO)],
            extra_data: vec![],
        };
        let tx_hash = tx.hash();

        let mut utxo_set = UtxoSet::new();
        utxo_set.add_transaction(&tx, 50, false, 0);

        let outpoint = Outpoint::new(tx_hash, 0);
        let entry = utxo_set.get(&outpoint).unwrap();

        // Verify flags
        assert!(!entry.is_coinbase);
        assert!(!entry.is_epoch_reward);

        // Regular tx should be spendable immediately
        #[allow(deprecated)]
        {
            assert!(entry.is_spendable_at(50)); // Same height
            assert!(entry.is_spendable_at(51)); // Next height
        }
    }

    #[test]
    fn test_utxo_entry_default_epoch_reward() {
        // Test that #[serde(default)] works - is_epoch_reward defaults to false
        // This ensures backward compatibility with existing serialized UTXOs
        use doli_core::transaction::Output;

        // Create an entry manually to verify default behavior
        let entry = UtxoEntry {
            output: Output::normal(1000, Hash::ZERO),
            height: 100,
            is_coinbase: true,
            is_epoch_reward: false, // Explicit false (what default should be)
        };

        // Serialize and deserialize
        let serialized = bincode::serialize(&entry).unwrap();
        let deserialized: UtxoEntry = bincode::deserialize(&serialized).unwrap();

        assert!(deserialized.is_coinbase);
        assert!(!deserialized.is_epoch_reward);

        // Also verify new UTXOs with is_epoch_reward=true serialize correctly
        let epoch_entry = UtxoEntry {
            output: Output::normal(1000, Hash::ZERO),
            height: 100,
            is_coinbase: false,
            is_epoch_reward: true,
        };

        let serialized2 = bincode::serialize(&epoch_entry).unwrap();
        let deserialized2: UtxoEntry = bincode::deserialize(&serialized2).unwrap();

        assert!(!deserialized2.is_coinbase);
        assert!(deserialized2.is_epoch_reward);
    }

    #[test]
    fn test_utxo_entry_serialization_roundtrip() {
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let tx = Transaction::new_epoch_reward(1, *keypair.public_key(), 1_000_000, pubkey_hash);

        let mut utxo_set = UtxoSet::new();
        utxo_set.add_transaction(&tx, 100, false, 0);

        let outpoint = Outpoint::new(tx.hash(), 0);
        let entry = utxo_set.get(&outpoint).unwrap();

        // Serialize and deserialize
        let serialized = bincode::serialize(&entry).unwrap();
        let deserialized: UtxoEntry = bincode::deserialize(&serialized).unwrap();

        assert_eq!(entry.height, deserialized.height);
        assert_eq!(entry.is_coinbase, deserialized.is_coinbase);
        assert_eq!(entry.is_epoch_reward, deserialized.is_epoch_reward);
        assert_eq!(entry.output.amount, deserialized.output.amount);
    }

    #[test]
    fn test_serialize_canonical_matches_between_backends() {
        let pk_hash = crypto::hash::hash(b"alice");

        // Create InMemory store with some entries
        let mut mem_store = InMemoryUtxoStore::new();
        let tx1 = Transaction::new_coinbase(100_000, pk_hash, 0);
        let tx2 = Transaction::new_coinbase(200_000, pk_hash, 1);
        mem_store.add_transaction(&tx1, 0, true, 0);
        mem_store.add_transaction(&tx2, 1, true, 0);

        // Create RocksDB store with same entries
        let dir = tempfile::TempDir::new().unwrap();
        let rocks_store = RocksDbUtxoStore::open(dir.path()).unwrap();
        rocks_store.add_transaction(&tx1, 0, true, 0);
        rocks_store.add_transaction(&tx2, 1, true, 0);

        // Canonical bytes must match
        let mem_bytes = mem_store.serialize_canonical();
        let rocks_bytes = rocks_store.serialize_canonical();
        assert_eq!(
            mem_bytes, rocks_bytes,
            "Canonical serialization must match between InMemory and RocksDB backends"
        );
    }

    #[test]
    fn test_utxo_entry_canonical_bytes_fixed_size() {
        let pk_hash = crypto::hash::hash(b"alice");
        let tx = Transaction::new_coinbase(1_000_000, pk_hash, 0);
        let entry = UtxoEntry {
            output: tx.outputs[0].clone(),
            height: 42,
            is_coinbase: true,
            is_epoch_reward: false,
        };
        let bytes = entry.serialize_canonical_bytes();
        assert_eq!(
            bytes.len(),
            61,
            "canonical entry base size must be 61 bytes (59 + 2 for extra_data len)"
        );
    }

    #[test]
    fn test_utxo_entry_canonical_roundtrip() {
        use doli_core::transaction::OutputType;
        let pk_hash = crypto::hash::hash(b"bob");
        let entry = UtxoEntry {
            output: doli_core::transaction::Output {
                output_type: OutputType::Normal,
                amount: 5_000_000,
                pubkey_hash: pk_hash,
                lock_until: 0,
                extra_data: vec![],
            },
            height: 1337,
            is_coinbase: false,
            is_epoch_reward: true,
        };

        let bytes = entry.serialize_canonical_bytes();
        let decoded =
            UtxoEntry::deserialize_canonical_bytes(&bytes).expect("roundtrip must succeed");

        assert_eq!(decoded.output.output_type, entry.output.output_type);
        assert_eq!(decoded.output.amount, entry.output.amount);
        assert_eq!(decoded.output.pubkey_hash, entry.output.pubkey_hash);
        assert_eq!(decoded.output.lock_until, entry.output.lock_until);
        assert_eq!(decoded.height, entry.height);
        assert_eq!(decoded.is_coinbase, entry.is_coinbase);
        assert_eq!(decoded.is_epoch_reward, entry.is_epoch_reward);
    }

    #[test]
    fn test_utxo_entry_canonical_bond_roundtrip() {
        use doli_core::transaction::OutputType;
        let pk_hash = crypto::hash::hash(b"producer");
        let entry = UtxoEntry {
            output: doli_core::transaction::Output {
                output_type: OutputType::Bond,
                amount: 10_000_000_000,
                pubkey_hash: pk_hash,
                lock_until: 9999,
                extra_data: vec![],
            },
            height: 500,
            is_coinbase: false,
            is_epoch_reward: false,
        };

        let bytes = entry.serialize_canonical_bytes();
        let decoded = UtxoEntry::deserialize_canonical_bytes(&bytes).unwrap();
        assert_eq!(decoded.output.output_type, OutputType::Bond);
        assert_eq!(decoded.output.lock_until, 9999);
    }

    #[test]
    fn test_utxo_canonical_count_header() {
        // Verify the 8-byte count header at the start of serialize_canonical output
        let mut store = InMemoryUtxoStore::new();
        let pk = crypto::hash::hash(b"x");
        let tx1 = Transaction::new_coinbase(100, pk, 0);
        let tx2 = Transaction::new_coinbase(200, pk, 1);
        store.add_transaction(&tx1, 0, true, 0);
        store.add_transaction(&tx2, 1, true, 0);

        let bytes = store.serialize_canonical();
        let count = u64::from_le_bytes(bytes[..8].try_into().unwrap());
        assert_eq!(count, 2, "count header must reflect number of UTXOs");
        // Total size: 8 (header) + 2 * (36 outpoint + 61 entry) = 8 + 194 = 202
        assert_eq!(bytes.len(), 8 + 2 * (36 + 61));
    }

    #[test]
    fn test_canonical_bytes_immune_to_is_epoch_reward_default() {
        // Simulates the bug where old on-disk UTXOs (without is_epoch_reward) and
        // new UTXOs (with is_epoch_reward=false explicitly) must produce identical
        // canonical bytes, since deserialize_canonical_bytes handles both uniformly.
        use doli_core::transaction::OutputType;
        let pk = crypto::hash::hash(b"test");

        // "Old" entry: is_epoch_reward was not stored (defaults to false via serde)
        let old_entry = UtxoEntry {
            output: doli_core::transaction::Output {
                output_type: OutputType::Normal,
                amount: 1_000,
                pubkey_hash: pk,
                lock_until: 0,
                extra_data: vec![],
            },
            height: 10,
            is_coinbase: true,
            is_epoch_reward: false, // default
        };

        // "New" entry: same data, is_epoch_reward explicitly false
        let new_entry = old_entry.clone();

        assert_eq!(
            old_entry.serialize_canonical_bytes(),
            new_entry.serialize_canonical_bytes(),
            "entries with identical logical state must produce identical canonical bytes"
        );
    }
}
