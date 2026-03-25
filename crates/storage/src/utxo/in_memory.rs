use std::collections::HashMap;
use std::path::Path;

use crypto::Hash;
use doli_core::transaction::{Output, OutputType, Transaction};
use doli_core::types::{Amount, BlockHeight};
use serde::{Deserialize, Serialize};

#[allow(deprecated)]
use super::types::DEFAULT_REWARD_MATURITY;
use super::types::{Outpoint, UtxoEntry};
use crate::StorageError;

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
            // Stamp Pool outputs: creation_slot, last_update_slot, TWAP accumulation
            if stamped_output.output_type == OutputType::Pool {
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
                    stamped_output = Output::pool(
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
            self.utxos.insert(outpoint, entry);
        }
    }

    pub fn spend_transaction(&mut self, tx: &Transaction) -> Result<Amount, StorageError> {
        let mut total_input = 0;

        for input in &tx.inputs {
            let outpoint = Outpoint::new(input.prev_tx_hash, input.output_index);
            match self.utxos.remove(&outpoint) {
                Some(entry) => {
                    if entry.output.output_type.is_native_amount() {
                        total_input += entry.output.amount;
                    }
                }
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
        self.utxos
            .values()
            .filter(|e| e.output.output_type.is_native_amount())
            .map(|e| e.output.amount)
            .sum()
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
            .filter(|(_, entry)| {
                entry.output.output_type.is_native_amount()
                    && entry.is_spendable_at_with_maturity(height, maturity)
            })
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

    /// Get all Pool UTXOs.
    pub fn get_all_pools(&self) -> Vec<(Outpoint, UtxoEntry)> {
        self.utxos
            .iter()
            .filter(|(_, entry)| entry.output.output_type == doli_core::OutputType::Pool)
            .map(|(op, entry)| (*op, entry.clone()))
            .collect()
    }

    /// Get all Collateral UTXOs.
    pub fn get_all_collateral(&self) -> Vec<(Outpoint, UtxoEntry)> {
        self.utxos
            .iter()
            .filter(|(_, entry)| entry.output.output_type == doli_core::OutputType::Collateral)
            .map(|(op, entry)| (*op, entry.clone()))
            .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use doli_core::transaction::{Output, Transaction, TxType};

    #[test]
    fn test_pool_twap_accumulates_on_add_transaction() {
        let mut store = InMemoryUtxoStore::new();
        let asset_b = crypto::Hash::from_bytes([0xBB; 32]);
        let pool_id = Output::compute_pool_id(&crypto::Hash::ZERO, &asset_b);

        // Create pool at slot 100, cumulative=0
        let pool_output = Output::pool(pool_id, asset_b, 1000, 2000, 707, 0, 100, 30, 100);
        let tx1 = Transaction {
            version: 1,
            tx_type: TxType::CreatePool,
            inputs: vec![],
            outputs: vec![pool_output],
            extra_data: vec![],
        };
        store.add_transaction(&tx1, 1, false, 100);

        // Verify: creation_slot stamped, cumulative=0 (first block, no elapsed)
        let (_, entry1) = store.get_all_pools()[0].clone();
        let meta1 = entry1.output.pool_metadata().unwrap();
        assert_eq!(meta1.creation_slot, 100);
        assert_eq!(meta1.last_update_slot, 100);
        assert_eq!(meta1.cumulative_price, 0);

        // Swap at slot 110 (10 slots later) — should accumulate TWAP
        let pool_output2 = Output::pool(pool_id, asset_b, 1100, 1900, 707, 0, 100, 30, 100);
        let tx2 = Transaction {
            version: 1,
            tx_type: TxType::Swap,
            inputs: vec![],
            outputs: vec![pool_output2],
            extra_data: vec![],
        };
        // Remove old pool first (swap consumes it)
        let old_outpoint = store.get_all_pools()[0].0;
        store.remove(&old_outpoint);
        store.add_transaction(&tx2, 2, false, 110);

        // Verify: TWAP accumulated
        let (_, entry2) = store.get_all_pools()[0].clone();
        let meta2 = entry2.output.pool_metadata().unwrap();
        assert_eq!(meta2.last_update_slot, 110);
        assert!(
            meta2.cumulative_price > 0,
            "TWAP must accumulate after 10 slots, got {}",
            meta2.cumulative_price
        );

        // Verify the TWAP value is correct:
        // price = reserve_a << 64 / reserve_b = 1100 << 64 / 1900
        // cumulative = price * (110 - 100) = price * 10
        let expected_price = (1100u128 << 64) / 1900;
        let expected_cum = expected_price * 10;
        assert_eq!(meta2.cumulative_price, expected_cum);
    }

    #[test]
    fn test_pool_twap_zero_on_first_block() {
        let mut store = InMemoryUtxoStore::new();
        let asset_b = crypto::Hash::from_bytes([0xCC; 32]);
        let pool_id = Output::compute_pool_id(&crypto::Hash::ZERO, &asset_b);

        // Create pool with last_update_slot=0 (CLI sends 0)
        let pool_output = Output::pool(pool_id, asset_b, 500, 500, 100, 0, 0, 30, 0);
        let tx = Transaction {
            version: 1,
            tx_type: TxType::CreatePool,
            inputs: vec![],
            outputs: vec![pool_output],
            extra_data: vec![],
        };
        store.add_transaction(&tx, 1, false, 200);

        let (_, entry) = store.get_all_pools()[0].clone();
        let meta = entry.output.pool_metadata().unwrap();
        // First block: creation_slot stamped, but no TWAP (no previous slot to diff)
        assert_eq!(meta.creation_slot, 200);
        assert_eq!(meta.last_update_slot, 200);
        assert_eq!(meta.cumulative_price, 0);
    }

    #[test]
    fn test_get_pool_utxo_finds_pool_by_id() {
        let mut store = InMemoryUtxoStore::new();
        let asset_b = crypto::Hash::from_bytes([0xBB; 32]);
        let pool_id =
            doli_core::transaction::Output::compute_pool_id(&crypto::Hash::ZERO, &asset_b);

        let pool_output = doli_core::transaction::Output::pool(
            pool_id, asset_b, 1000, 2000, 707, 0, 100, 30, 100,
        );

        // The pool output has pubkey_hash = pool_id
        assert_eq!(pool_output.pubkey_hash, pool_id);

        let outpoint = Outpoint::new(crypto::Hash::from_bytes([0xFF; 32]), 0);
        let entry = UtxoEntry {
            output: pool_output,
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        store.insert(outpoint, entry);

        // get_by_pubkey_hash should find it
        let results = store.get_by_pubkey_hash(&pool_id);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.output.output_type, doli_core::OutputType::Pool);
    }
}
