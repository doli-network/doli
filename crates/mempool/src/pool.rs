//! Core mempool implementation

use std::collections::{BTreeSet, HashMap, HashSet};

use crypto::Hash;
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::{BlockHeight, Transaction, TxType};
use storage::{Outpoint, UtxoSet};
use thiserror::Error;
use tracing::{debug, info, warn};

use super::entry::MempoolEntry;
use super::policy::MempoolPolicy;

/// Mempool errors
#[derive(Debug, Error)]
pub enum MempoolError {
    #[error("transaction already exists in mempool")]
    AlreadyExists,

    #[error("mempool is full")]
    Full,

    #[error("invalid transaction: {0}")]
    InvalidTransaction(String),

    #[error("fee too low: {0} < {1}")]
    FeeTooLow(u64, u64),

    #[error("transaction too large: {0} > {1}")]
    TooLarge(usize, usize),

    #[error("too many ancestors: {0} > {1}")]
    TooManyAncestors(usize, usize),

    #[error("too many descendants: {0} > {1}")]
    TooManyDescendants(usize, usize),

    #[error("missing input: {0}:{1}")]
    MissingInput(Hash, u32),

    #[error("double spend with mempool transaction")]
    DoubleSpend,
}

/// Transaction mempool
pub struct Mempool {
    /// Transactions by hash
    pub(crate) entries: HashMap<Hash, MempoolEntry>,
    /// Sorted by fee rate (for eviction)
    pub(crate) by_fee_rate: BTreeSet<(u64, Hash)>,
    /// Transactions by pubkey hash (for balance queries)
    pub(crate) by_address: HashMap<Hash, HashSet<Hash>>,
    /// Spent outpoints in mempool
    pub(crate) spent_outputs: HashMap<Outpoint, Hash>,
    /// Policy settings
    pub(crate) policy: MempoolPolicy,
    /// Total size of all transactions
    pub(crate) total_size: usize,
    /// Consensus parameters
    pub(crate) params: ConsensusParams,
    /// Network type
    pub(crate) network: Network,
}

impl Mempool {
    /// Create a new mempool
    pub fn new(policy: MempoolPolicy, params: ConsensusParams, network: Network) -> Self {
        Self {
            entries: HashMap::new(),
            by_fee_rate: BTreeSet::new(),
            by_address: HashMap::new(),
            spent_outputs: HashMap::new(),
            policy,
            total_size: 0,
            params,
            network,
        }
    }

    /// Create with default mainnet settings
    pub fn mainnet() -> Self {
        Self::new(
            MempoolPolicy::mainnet(),
            ConsensusParams::mainnet(),
            Network::Mainnet,
        )
    }

    /// Create with testnet settings
    pub fn testnet() -> Self {
        Self::new(
            MempoolPolicy::testnet(),
            ConsensusParams::testnet(),
            Network::Testnet,
        )
    }

    /// Remove a transaction from the mempool
    pub fn remove_transaction(&mut self, tx_hash: &Hash) -> Option<MempoolEntry> {
        let entry = self.entries.remove(tx_hash)?;

        // Remove from fee rate index
        self.by_fee_rate.remove(&(entry.fee_rate, *tx_hash));

        // Remove from address index
        for output in &entry.tx.outputs {
            if let Some(txs) = self.by_address.get_mut(&output.pubkey_hash) {
                txs.remove(tx_hash);
                if txs.is_empty() {
                    self.by_address.remove(&output.pubkey_hash);
                }
            }
        }

        // Remove spent outputs
        for input in &entry.tx.inputs {
            let outpoint = Outpoint::new(input.prev_tx_hash, input.output_index);
            self.spent_outputs.remove(&outpoint);
        }

        // Update ancestors' descendants
        for ancestor_hash in &entry.ancestors {
            if let Some(ancestor) = self.entries.get_mut(ancestor_hash) {
                ancestor.remove_descendant(tx_hash);
            }
        }

        // Update descendants' ancestors
        for descendant_hash in &entry.descendants {
            if let Some(descendant) = self.entries.get_mut(descendant_hash) {
                descendant.remove_ancestor(tx_hash, entry.fee, entry.size);
            }
        }

        self.total_size -= entry.size;

        debug!("Removed transaction {} from mempool", tx_hash);
        Some(entry)
    }

    /// Remove transactions that were included in a block
    pub fn remove_for_block(&mut self, transactions: &[Transaction]) {
        for tx in transactions {
            self.remove_transaction(&tx.hash());
        }
    }

    /// Get a transaction entry
    pub fn get(&self, tx_hash: &Hash) -> Option<&MempoolEntry> {
        self.entries.get(tx_hash)
    }

    /// Check if transaction exists
    pub fn contains(&self, tx_hash: &Hash) -> bool {
        self.entries.contains_key(tx_hash)
    }

    /// Get number of transactions
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get total size
    pub fn size(&self) -> usize {
        self.total_size
    }

    /// Get max size
    pub fn max_size(&self) -> usize {
        self.policy.max_size
    }

    /// Get max count
    pub fn max_count(&self) -> usize {
        self.policy.max_count
    }

    /// Check if an outpoint is being spent by a mempool transaction.
    pub fn is_outpoint_spent(&self, outpoint: &Outpoint) -> bool {
        self.spent_outputs.contains_key(outpoint)
    }

    /// Get minimum fee rate for acceptance
    pub fn min_fee_rate(&self) -> u64 {
        // If mempool is getting full, dynamically increase minimum fee
        if self.len() > self.policy.max_count * 9 / 10 {
            // Get lowest fee rate in mempool
            if let Some(&(lowest_rate, _)) = self.by_fee_rate.iter().next() {
                return lowest_rate.max(self.policy.min_fee_rate);
            }
        }
        self.policy.min_fee_rate
    }

    /// Select transactions for a block using CPFP-aware ordering.
    ///
    /// Sorts by `effective_fee_rate()` (package fee rate including ancestors)
    /// so that high-fee children pull their low-fee parents into blocks.
    pub fn select_for_block(&self, max_size: usize) -> Vec<Transaction> {
        let mut selected = Vec::new();
        let mut selected_size = 0;
        let mut selected_hashes = HashSet::new();

        // Build a list sorted by effective fee rate (CPFP-aware)
        let mut candidates: Vec<(&Hash, &MempoolEntry)> = self.entries.iter().collect();
        candidates.sort_by(|a, b| b.1.effective_fee_rate().cmp(&a.1.effective_fee_rate()));

        for &(tx_hash, entry) in &candidates {
            if selected_hashes.contains(tx_hash) {
                continue;
            }

            // Check if we have room
            if selected_size + entry.size > max_size {
                continue;
            }

            // Check if all ancestors are selected
            if !entry.ancestors.iter().all(|a| selected_hashes.contains(a)) {
                continue;
            }

            selected.push(entry.tx.clone());
            selected_size += entry.size;
            selected_hashes.insert(*tx_hash);

            if selected_size >= max_size {
                break;
            }
        }

        selected
    }

    /// Remove all Registration-type transactions from the mempool.
    ///
    /// Called when block production fails due to a stale/duplicate registration TX.
    /// Prevents infinite retry loops where invalid registrations poison every block.
    pub fn remove_registration_txs(&mut self) {
        let reg_hashes: Vec<Hash> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.tx.tx_type == doli_core::TxType::Registration)
            .map(|(hash, _)| *hash)
            .collect();
        for hash in reg_hashes {
            self.remove_transaction(&hash);
        }
    }

    /// Iterate all mempool entries
    pub fn iter(&self) -> impl Iterator<Item = (&Hash, &MempoolEntry)> {
        self.entries.iter()
    }

    /// Get transactions for an address
    pub fn get_by_address(&self, pubkey_hash: &Hash) -> Vec<&MempoolEntry> {
        self.by_address
            .get(pubkey_hash)
            .map(|hashes| hashes.iter().filter_map(|h| self.entries.get(h)).collect())
            .unwrap_or_default()
    }

    /// Calculate unconfirmed balance change for an address
    ///
    /// Returns a tuple of (incoming, outgoing) amounts:
    /// - incoming: sum of outputs in mempool transactions to this address
    /// - outgoing: sum of spent UTXOs in mempool transactions from this address
    ///
    /// The net unconfirmed balance change = incoming - outgoing
    pub fn calculate_unconfirmed_balance(
        &self,
        pubkey_hash: &Hash,
        utxo_set: &UtxoSet,
    ) -> (u64, u64) {
        let mut incoming = 0u64;
        let mut outgoing = 0u64;

        // Calculate incoming: outputs in mempool transactions to this address
        for entry in self.get_by_address(pubkey_hash) {
            for output in &entry.tx.outputs {
                if &output.pubkey_hash == pubkey_hash {
                    incoming = incoming.saturating_add(output.amount);
                }
            }
        }

        // Calculate outgoing: spent UTXOs from this address
        for outpoint in self.spent_outputs.keys() {
            if let Some(utxo) = utxo_set.get(outpoint) {
                if &utxo.output.pubkey_hash == pubkey_hash {
                    outgoing = outgoing.saturating_add(utxo.output.amount);
                }
            }
        }

        (incoming, outgoing)
    }

    /// Get net unconfirmed balance for an address
    ///
    /// Returns the net balance change from mempool transactions.
    /// A positive value means more incoming than outgoing.
    /// Note: This can be negative if spending more than receiving.
    pub fn get_unconfirmed_balance(&self, pubkey_hash: &Hash, utxo_set: &UtxoSet) -> i64 {
        let (incoming, outgoing) = self.calculate_unconfirmed_balance(pubkey_hash, utxo_set);
        incoming as i64 - outgoing as i64
    }

    /// Calculate total input value and ancestor set
    pub(crate) fn calculate_inputs(
        &self,
        tx: &Transaction,
        utxo_set: &UtxoSet,
        current_height: BlockHeight,
    ) -> Result<(u64, HashSet<Hash>), MempoolError> {
        let mut total = 0u64;
        let mut ancestors = HashSet::new();

        for input in &tx.inputs {
            let outpoint = Outpoint::new(input.prev_tx_hash, input.output_index);

            // First check if output is in mempool
            if let Some(parent_entry) = self.entries.get(&input.prev_tx_hash) {
                if let Some(output) = parent_entry.tx.outputs.get(input.output_index as usize) {
                    total += output.amount;
                    ancestors.insert(input.prev_tx_hash);
                    // Include parent's ancestors
                    ancestors.extend(parent_entry.ancestors.iter().cloned());
                    continue;
                }
            }

            // Then check UTXO set
            if let Some(utxo) = utxo_set.get(&outpoint) {
                // Skip lock check for RequestWithdrawal and Exit — they unlock Bond UTXOs
                // (matches validation.rs:2527 exemption)
                let skip_lock =
                    tx.tx_type == TxType::RequestWithdrawal || tx.tx_type == TxType::Exit;
                if !skip_lock {
                    let maturity = self.network.coinbase_maturity();
                    if !utxo.is_spendable_at_with_maturity(current_height, maturity) {
                        return Err(MempoolError::InvalidTransaction(format!(
                            "Output not spendable until height {}",
                            utxo.height + maturity
                        )));
                    }
                }
                total += utxo.output.amount;
            } else {
                return Err(MempoolError::MissingInput(
                    input.prev_tx_hash,
                    input.output_index,
                ));
            }
        }

        Ok((total, ancestors))
    }

    /// Check if we need to evict transactions
    pub(crate) fn needs_eviction(&self, new_tx_size: usize) -> bool {
        self.entries.len() >= self.policy.max_count
            || self.total_size + new_tx_size > self.policy.max_size
    }

    /// Evict the lowest fee rate transaction
    pub(crate) fn evict_lowest_fee(&mut self) -> bool {
        // Find lowest fee rate transaction without descendants
        let to_evict = self
            .by_fee_rate
            .iter()
            .find(|&&(_, hash)| {
                self.entries
                    .get(&hash)
                    .map(|e| e.descendants.is_empty())
                    .unwrap_or(false)
            })
            .map(|&(_, hash)| hash);

        if let Some(hash) = to_evict {
            self.remove_transaction(&hash);
            true
        } else {
            false
        }
    }

    /// Remove old transactions
    pub fn expire_old(&mut self) {
        let hashes: Vec<Hash> = self
            .entries
            .iter()
            .filter(|(_, e)| e.age() > self.policy.max_age)
            .map(|(h, _)| *h)
            .collect();

        for hash in hashes {
            warn!("Expiring old transaction {}", hash);
            self.remove_transaction(&hash);
        }
    }

    /// Revalidate all transactions against current UTXO set
    /// Used after chain reorganization to remove invalid transactions
    pub fn revalidate(&mut self, utxo_set: &UtxoSet, _current_height: BlockHeight) {
        let mut to_remove = Vec::new();

        for (hash, entry) in &self.entries {
            let tx = &entry.tx;

            // Check that all inputs still exist in the UTXO set
            let mut valid = true;
            for input in &tx.inputs {
                let outpoint = Outpoint::new(input.prev_tx_hash, input.output_index);
                if utxo_set.get(&outpoint).is_none() {
                    // Input no longer exists - likely spent in new canonical chain
                    valid = false;
                    break;
                }
            }

            if !valid {
                to_remove.push(*hash);
            }
        }

        for hash in to_remove {
            info!("Removing invalidated transaction {}", hash);
            self.remove_transaction(&hash);
        }
    }
}

impl Default for Mempool {
    fn default() -> Self {
        Self::mainnet()
    }
}

#[cfg(test)]
#[path = "pool_tests.rs"]
mod tests;
