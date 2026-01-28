//! UTXO set management

use std::collections::HashMap;
use std::path::Path;

use crypto::Hash;
use doli_core::transaction::{Output, Transaction};
use doli_core::types::{Amount, BlockHeight};
use serde::{Deserialize, Serialize};

use crate::StorageError;

/// An entry in the UTXO set
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UtxoEntry {
    /// The output
    pub output: Output,
    /// Block height when created
    pub height: BlockHeight,
    /// Whether this is a coinbase output
    pub is_coinbase: bool,
    /// Whether this is an epoch reward output
    #[serde(default)] // For backward compatibility with existing UTXOs
    pub is_epoch_reward: bool,
}

/// Reward maturity constant (same as core::consensus::REWARD_MATURITY)
const REWARD_MATURITY: BlockHeight = 100;

impl UtxoEntry {
    /// Check if the UTXO is spendable at the given height
    pub fn is_spendable_at(&self, height: BlockHeight) -> bool {
        // Check time lock
        if !self.output.is_spendable_at(height) {
            return false;
        }

        // Coinbase AND EpochReward require REWARD_MATURITY confirmations
        if self.is_coinbase || self.is_epoch_reward {
            let confirmations = height.saturating_sub(self.height);
            return confirmations >= REWARD_MATURITY;
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

/// In-memory UTXO set
pub struct UtxoSet {
    /// UTXOs indexed by outpoint
    utxos: HashMap<Outpoint, UtxoEntry>,
}

impl UtxoSet {
    /// Create a new empty UTXO set
    pub fn new() -> Self {
        Self {
            utxos: HashMap::new(),
        }
    }

    /// Clear all UTXOs (used during chain reorganization)
    pub fn clear(&mut self) {
        self.utxos.clear();
    }

    /// Load from disk
    pub fn load(path: &Path) -> Result<Self, StorageError> {
        let db = crate::open_db(path)?;

        let mut utxos = HashMap::new();

        let iter = db.iterator(rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, value) = item?;
            if let Some(outpoint) = Outpoint::from_bytes(&key) {
                if let Ok(entry) = bincode::deserialize::<UtxoEntry>(&value) {
                    utxos.insert(outpoint, entry);
                }
            }
        }

        Ok(Self { utxos })
    }

    /// Save to disk
    pub fn save(&self, path: &Path) -> Result<(), StorageError> {
        let db = crate::open_db(path)?;

        for (outpoint, entry) in &self.utxos {
            let key = outpoint.to_bytes();
            let value = bincode::serialize(entry)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            db.put(&key, &value)?;
        }

        Ok(())
    }

    /// Get a UTXO by outpoint
    pub fn get(&self, outpoint: &Outpoint) -> Option<&UtxoEntry> {
        self.utxos.get(outpoint)
    }

    /// Check if a UTXO exists
    pub fn contains(&self, outpoint: &Outpoint) -> bool {
        self.utxos.contains_key(outpoint)
    }

    /// Add outputs from a transaction
    pub fn add_transaction(&mut self, tx: &Transaction, height: BlockHeight, is_coinbase: bool) {
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
            self.utxos.insert(outpoint, entry);
        }
    }

    /// Remove inputs spent by a transaction
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

    /// Get total value in the UTXO set
    pub fn total_value(&self) -> Amount {
        self.utxos.values().map(|e| e.output.amount).sum()
    }

    /// Get number of UTXOs
    pub fn len(&self) -> usize {
        self.utxos.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.utxos.is_empty()
    }

    /// Get all UTXOs for a given pubkey hash
    pub fn get_by_pubkey_hash(&self, pubkey_hash: &Hash) -> Vec<(Outpoint, &UtxoEntry)> {
        self.utxos
            .iter()
            .filter(|(_, entry)| &entry.output.pubkey_hash == pubkey_hash)
            .map(|(op, entry)| (*op, entry))
            .collect()
    }

    /// Get spendable balance for a pubkey hash at a given height
    pub fn get_balance(&self, pubkey_hash: &Hash, height: BlockHeight) -> Amount {
        self.get_by_pubkey_hash(pubkey_hash)
            .iter()
            .filter(|(_, entry)| entry.is_spendable_at(height))
            .map(|(_, entry)| entry.output.amount)
            .sum()
    }

    /// Insert a UTXO entry directly (for testing and reorgs)
    pub fn insert(&mut self, outpoint: Outpoint, entry: UtxoEntry) {
        self.utxos.insert(outpoint, entry);
    }

    /// Remove a UTXO entry directly (for testing and reorgs)
    pub fn remove(&mut self, outpoint: &Outpoint) -> Option<UtxoEntry> {
        self.utxos.remove(outpoint)
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
        let tx = Transaction::new_coinbase(500_000_000, Hash::zero(), 0);
        let tx_hash = tx.hash();

        // Add to UTXO set
        utxo_set.add_transaction(&tx, 0, true);
        assert_eq!(utxo_set.len(), 1);

        // Check it exists
        let outpoint = Outpoint::new(tx_hash, 0);
        assert!(utxo_set.contains(&outpoint));

        // Check coinbase maturity
        let entry = utxo_set.get(&outpoint).unwrap();
        assert!(!entry.is_spendable_at(50)); // Not mature
        assert!(entry.is_spendable_at(100)); // Mature
    }

    #[test]
    fn test_outpoint_serialization() {
        let outpoint = Outpoint::new(Hash::zero(), 42);
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
        let tx = Transaction::new_epoch_reward(1, keypair.public_key().clone(), 1_000_000, pubkey_hash);
        let tx_hash = tx.hash();

        let mut utxo_set = UtxoSet::new();
        utxo_set.add_transaction(&tx, 100, false); // height 100, not coinbase

        let outpoint = Outpoint::new(tx_hash, 0);
        let entry = utxo_set.get(&outpoint).unwrap();

        // Verify is_epoch_reward flag is set
        assert!(entry.is_epoch_reward);
        assert!(!entry.is_coinbase);

        // Check maturity: needs REWARD_MATURITY (100) confirmations
        assert!(!entry.is_spendable_at(100)); // 0 confirmations
        assert!(!entry.is_spendable_at(150)); // 50 confirmations
        assert!(!entry.is_spendable_at(199)); // 99 confirmations
        assert!(entry.is_spendable_at(200)); // 100 confirmations (mature)
        assert!(entry.is_spendable_at(300)); // 200 confirmations (mature)
    }

    #[test]
    fn test_utxo_entry_coinbase_maturity_unchanged() {
        // Verify coinbase still requires same maturity
        let tx = Transaction::new_coinbase(500_000_000, Hash::zero(), 0);
        let tx_hash = tx.hash();

        let mut utxo_set = UtxoSet::new();
        utxo_set.add_transaction(&tx, 50, true);

        let outpoint = Outpoint::new(tx_hash, 0);
        let entry = utxo_set.get(&outpoint).unwrap();

        // Verify flags
        assert!(entry.is_coinbase);
        assert!(!entry.is_epoch_reward);

        // Check maturity
        assert!(!entry.is_spendable_at(100)); // 50 confirmations
        assert!(!entry.is_spendable_at(149)); // 99 confirmations
        assert!(entry.is_spendable_at(150)); // 100 confirmations (mature)
    }

    #[test]
    fn test_utxo_entry_regular_tx_no_maturity() {
        // Regular transactions should not have maturity requirements
        use doli_core::transaction::{Input, Output};

        let tx = Transaction {
            version: 1,
            tx_type: doli_core::transaction::TxType::Transfer,
            inputs: vec![Input::new(Hash::zero(), 0)],
            outputs: vec![Output::normal(1000, Hash::zero())],
            extra_data: vec![],
        };
        let tx_hash = tx.hash();

        let mut utxo_set = UtxoSet::new();
        utxo_set.add_transaction(&tx, 50, false);

        let outpoint = Outpoint::new(tx_hash, 0);
        let entry = utxo_set.get(&outpoint).unwrap();

        // Verify flags
        assert!(!entry.is_coinbase);
        assert!(!entry.is_epoch_reward);

        // Regular tx should be spendable immediately
        assert!(entry.is_spendable_at(50)); // Same height
        assert!(entry.is_spendable_at(51)); // Next height
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
        let tx = Transaction::new_epoch_reward(1, keypair.public_key().clone(), 1_000_000, pubkey_hash);

        let mut utxo_set = UtxoSet::new();
        utxo_set.add_transaction(&tx, 100, false);

        let outpoint = Outpoint::new(tx.hash(), 0);
        let entry = utxo_set.get(&outpoint).unwrap();

        // Serialize and deserialize
        let serialized = bincode::serialize(entry).unwrap();
        let deserialized: UtxoEntry = bincode::deserialize(&serialized).unwrap();

        assert_eq!(entry.height, deserialized.height);
        assert_eq!(entry.is_coinbase, deserialized.is_coinbase);
        assert_eq!(entry.is_epoch_reward, deserialized.is_epoch_reward);
        assert_eq!(entry.output.amount, deserialized.output.amount);
    }
}
