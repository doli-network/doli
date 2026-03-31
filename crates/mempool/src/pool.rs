//! Core mempool implementation

use std::collections::{BTreeSet, HashMap, HashSet};

use crypto::Hash;
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::validation::{validate_transaction, ValidationContext};
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
    entries: HashMap<Hash, MempoolEntry>,
    /// Sorted by fee rate (for eviction)
    by_fee_rate: BTreeSet<(u64, Hash)>,
    /// Transactions by pubkey hash (for balance queries)
    by_address: HashMap<Hash, HashSet<Hash>>,
    /// Spent outpoints in mempool
    spent_outputs: HashMap<Outpoint, Hash>,
    /// Policy settings
    policy: MempoolPolicy,
    /// Total size of all transactions
    total_size: usize,
    /// Consensus parameters
    params: ConsensusParams,
    /// Network type
    network: Network,
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

    /// Add a transaction to the mempool
    pub fn add_transaction(
        &mut self,
        tx: Transaction,
        utxo_set: &UtxoSet,
        current_height: BlockHeight,
    ) -> Result<Hash, MempoolError> {
        let tx_hash = tx.hash();

        // Check if already in mempool
        if self.entries.contains_key(&tx_hash) {
            return Err(MempoolError::AlreadyExists);
        }

        // Check transaction size
        let tx_size = tx.size();
        if tx_size > self.policy.max_tx_size {
            return Err(MempoolError::TooLarge(tx_size, self.policy.max_tx_size));
        }

        // Validate transaction structure
        let ctx = ValidationContext::new(self.params.clone(), self.network, 0, current_height)
            .with_sig_verification_height(self.network.params().sig_verification_height);
        validate_transaction(&tx, &ctx)
            .map_err(|e| MempoolError::InvalidTransaction(e.to_string()))?;

        // Validate input spending conditions: pubkey + signature for Normal/Bond,
        // covenant evaluation for conditioned outputs. Reject early to prevent
        // mempool DoS with unsigned or wrongly-signed transactions.
        let sig_height = self.network.params().sig_verification_height;
        for (i, input) in tx.inputs.iter().enumerate() {
            let outpoint = Outpoint::new(input.prev_tx_hash, input.output_index);
            if let Some(entry) = utxo_set.get(&outpoint) {
                // Normal/Bond: verify pubkey presence + hash match + signature
                if !entry.output.output_type.is_conditioned()
                    && current_height >= sig_height
                {
                        let pk = match &input.public_key {
                            Some(pk) => pk,
                            None => {
                                return Err(MempoolError::InvalidTransaction(format!(
                                    "input {} missing public_key (required at height >= {})",
                                    i, sig_height,
                                )));
                            }
                        };
                        let expected = entry.output.pubkey_hash;
                        let actual = crypto::hash::hash_with_domain(
                            crypto::ADDRESS_DOMAIN,
                            pk.as_bytes(),
                        );
                        if expected != actual {
                            return Err(MempoolError::InvalidTransaction(format!(
                                "input {} pubkey hash mismatch",
                                i,
                            )));
                        }
                        let signing_hash = tx.signing_message_for_input(i);
                        if crypto::signature::verify_hash(&signing_hash, &input.signature, pk)
                            .is_err()
                        {
                            return Err(MempoolError::InvalidTransaction(format!(
                                "input {} invalid signature",
                                i,
                            )));
                        }
                }

                if entry.output.output_type.is_conditioned() {
                    use doli_core::conditions::{Condition, Witness, MAX_CONDITION_OPS};
                    let condition = Condition::decode_prefix(&entry.output.extra_data)
                        .map(|(cond, _consumed)| cond)
                        .map_err(|e| {
                            MempoolError::InvalidTransaction(format!(
                                "input {} invalid condition: {}",
                                i, e
                            ))
                        })?;
                    if condition.ops_count() > MAX_CONDITION_OPS {
                        return Err(MempoolError::InvalidTransaction(format!(
                            "input {} condition exceeds ops limit",
                            i
                        )));
                    }
                    let witness_bytes = tx.get_covenant_witness(i).unwrap_or(&[]);
                    let witness = Witness::decode(witness_bytes).map_err(|e| {
                        MempoolError::InvalidTransaction(format!(
                            "input {} invalid witness: {}",
                            i, e
                        ))
                    })?;
                    let signing_hash = tx.signing_message_for_input(i);
                    let eval_ctx = doli_core::conditions::EvalContext {
                        current_height,
                        signing_hash: &signing_hash,
                    };
                    let mut or_idx = 0usize;
                    if !doli_core::conditions::evaluate(
                        &condition,
                        &witness,
                        &eval_ctx,
                        &mut or_idx,
                    ) {
                        return Err(MempoolError::InvalidTransaction(format!(
                            "input {} covenant condition not satisfied",
                            i
                        )));
                    }
                }
            }
        }

        // Calculate fee by looking up inputs
        let (total_input, ancestors) = self.calculate_inputs(&tx, utxo_set, current_height)?;
        let total_output = tx.total_output();

        if total_input < total_output {
            return Err(MempoolError::InvalidTransaction(format!(
                "insufficient funds: {} < {}",
                total_input, total_output
            )));
        }

        let fee = total_input - total_output;
        let fee_rate = if tx_size > 0 { fee / tx_size as u64 } else { 0 };

        // Require fee >= minimum_fee (base + per-byte for output extra_data)
        let min_fee = tx.minimum_fee();
        if fee < min_fee {
            return Err(MempoolError::FeeTooLow(fee, min_fee));
        }

        // Check minimum fee rate (if configured)
        if self.policy.min_fee_rate > 0 && fee_rate < self.policy.min_fee_rate {
            return Err(MempoolError::FeeTooLow(fee_rate, self.policy.min_fee_rate));
        }

        // Check ancestor limits
        if ancestors.len() > self.policy.max_ancestors {
            return Err(MempoolError::TooManyAncestors(
                ancestors.len(),
                self.policy.max_ancestors,
            ));
        }

        // Check for double-spend with mempool
        for input in &tx.inputs {
            let outpoint = Outpoint::new(input.prev_tx_hash, input.output_index);
            if let Some(spending_tx) = self.spent_outputs.get(&outpoint) {
                if spending_tx != &tx_hash {
                    return Err(MempoolError::DoubleSpend);
                }
            }
        }

        // Make room if necessary
        while self.needs_eviction(tx_size) {
            if !self.evict_lowest_fee() {
                return Err(MempoolError::Full);
            }
        }

        // Create entry
        let mut entry = MempoolEntry::new(tx.clone(), fee);

        // Calculate ancestor fees
        for ancestor_hash in &ancestors {
            if let Some(ancestor) = self.entries.get(ancestor_hash) {
                entry.add_ancestor(*ancestor_hash, ancestor.fee, ancestor.size);
            }
        }

        // Mark outputs as spent
        for input in &tx.inputs {
            let outpoint = Outpoint::new(input.prev_tx_hash, input.output_index);
            self.spent_outputs.insert(outpoint, tx_hash);
        }

        // Index by address
        for output in &tx.outputs {
            self.by_address
                .entry(output.pubkey_hash)
                .or_default()
                .insert(tx_hash);
        }

        // Update ancestor descendants
        for ancestor_hash in &ancestors {
            if let Some(ancestor) = self.entries.get_mut(ancestor_hash) {
                ancestor.add_descendant(tx_hash);
            }
        }

        // Add to indexes
        self.by_fee_rate.insert((fee_rate, tx_hash));
        self.total_size += tx_size;
        self.entries.insert(tx_hash, entry);

        debug!(
            "Added transaction {} to mempool (fee_rate: {})",
            tx_hash, fee_rate
        );
        Ok(tx_hash)
    }

    /// Add a system transaction to the mempool (bypasses fee requirements)
    ///
    /// System transactions like SlashProducer have no inputs/outputs and therefore
    /// no fees. They are added with priority 0 (lowest) but are still valid.
    pub fn add_system_transaction(
        &mut self,
        tx: Transaction,
        current_height: BlockHeight,
    ) -> Result<Hash, MempoolError> {
        let tx_hash = tx.hash();

        // Check if already in mempool
        if self.entries.contains_key(&tx_hash) {
            return Err(MempoolError::AlreadyExists);
        }

        // Check transaction size
        let tx_size = tx.size();
        if tx_size > self.policy.max_tx_size {
            return Err(MempoolError::TooLarge(tx_size, self.policy.max_tx_size));
        }

        // Validate transaction structure (but skip UTXO checks)
        let ctx = ValidationContext::new(self.params.clone(), self.network, 0, current_height)
            .with_sig_verification_height(self.network.params().sig_verification_height);
        validate_transaction(&tx, &ctx)
            .map_err(|e| MempoolError::InvalidTransaction(e.to_string()))?;

        // Make room if necessary
        while self.needs_eviction(tx_size) {
            if !self.evict_lowest_fee() {
                return Err(MempoolError::Full);
            }
        }

        // Create entry with 0 fee (system transaction)
        let entry = MempoolEntry::new(tx.clone(), 0);

        // System transactions have no inputs to mark as spent
        // and no outputs to index by address

        // Add to indexes with lowest priority (fee_rate = 0)
        self.by_fee_rate.insert((0, tx_hash));
        self.total_size += tx_size;
        self.entries.insert(tx_hash, entry);

        debug!(
            "Added system transaction {} to mempool (type: {:?})",
            tx_hash, tx.tx_type
        );
        Ok(tx_hash)
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

    /// Remove TXs that could cause "already exists" errors in apply_block.
    /// Purges NFT, CreatePool, or Registration TXs based on the error message.
    /// Prevents toxic mempool TXs from freezing the chain indefinitely.
    /// See: testnet incident 2026-03-25.
    pub fn remove_by_error_pattern(&mut self, err_msg: &str) {
        let toxic: Vec<Hash> = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                let tx = &entry.tx;
                if (err_msg.contains("token_id") || err_msg.contains("NFT"))
                    && tx
                        .outputs
                        .iter()
                        .any(|o| o.output_type == doli_core::OutputType::NFT)
                {
                    return true;
                }
                if err_msg.contains("ool") && tx.tx_type == doli_core::TxType::CreatePool {
                    return true;
                }
                if err_msg.contains("egistration") && tx.tx_type == doli_core::TxType::Registration
                {
                    return true;
                }
                false
            })
            .map(|(hash, _)| *hash)
            .collect();
        for hash in toxic {
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
    fn calculate_inputs(
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
    fn needs_eviction(&self, new_tx_size: usize) -> bool {
        self.entries.len() >= self.policy.max_count
            || self.total_size + new_tx_size > self.policy.max_size
    }

    /// Evict the lowest fee rate transaction
    fn evict_lowest_fee(&mut self) -> bool {
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
mod tests {
    use super::*;
    use doli_core::transaction::{Input, Output, Transaction};
    use storage::{Outpoint, UtxoEntry, UtxoSet};

    /// Create a testnet mempool with min_fee_rate=0 for easier testing
    fn test_mempool() -> Mempool {
        Mempool::testnet()
    }

    /// Create a funding UTXO and return (utxo_set, tx_hash, pubkey_hash)
    fn funded_utxo(amount: u64) -> (UtxoSet, Hash, Hash) {
        let mut utxo_set = UtxoSet::new();
        let pubkey_hash = crypto::hash::hash(b"test_address");
        let tx_hash = crypto::hash::hash(b"funding_tx");
        let outpoint = Outpoint::new(tx_hash, 0);
        let entry = UtxoEntry {
            output: Output::normal(amount, pubkey_hash),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        utxo_set.insert(outpoint, entry).unwrap();
        (utxo_set, tx_hash, pubkey_hash)
    }

    /// Create a simple transfer spending from a funded UTXO
    fn simple_transfer(prev_hash: Hash, amount: u64, fee: u64, dest: Hash) -> Transaction {
        Transaction::new_transfer(
            vec![Input::new(prev_hash, 0)],
            vec![Output::normal(amount - fee, dest)],
        )
    }

    #[test]
    fn test_mempool_creation() {
        let mempool = Mempool::mainnet();
        assert!(mempool.is_empty());
        assert_eq!(mempool.len(), 0);
        assert_eq!(mempool.size(), 0);
    }

    #[test]
    fn test_add_valid_transaction() {
        let mut mempool = test_mempool();
        let (utxo_set, tx_hash, pubkey_hash) = funded_utxo(10_000);
        let tx = simple_transfer(tx_hash, 10_000, 1_000, pubkey_hash);

        let result = mempool.add_transaction(tx, &utxo_set, 100);
        assert!(result.is_ok());
        assert_eq!(mempool.len(), 1);
        assert!(!mempool.is_empty());
    }

    #[test]
    fn test_add_duplicate_rejected() {
        let mut mempool = test_mempool();
        let (utxo_set, tx_hash, pubkey_hash) = funded_utxo(10_000);
        let tx = simple_transfer(tx_hash, 10_000, 1_000, pubkey_hash);

        mempool.add_transaction(tx.clone(), &utxo_set, 100).unwrap();
        let result = mempool.add_transaction(tx, &utxo_set, 100);
        assert!(matches!(result, Err(MempoolError::AlreadyExists)));
    }

    #[test]
    fn test_missing_input_rejected() {
        let mut mempool = test_mempool();
        let utxo_set = UtxoSet::new(); // Empty UTXO set
        let fake_hash = crypto::hash::hash(b"nonexistent");
        let dest = crypto::hash::hash(b"dest");
        let tx = simple_transfer(fake_hash, 5_000, 500, dest);

        let result = mempool.add_transaction(tx, &utxo_set, 100);
        assert!(matches!(result, Err(MempoolError::MissingInput(_, _))));
    }

    #[test]
    fn test_insufficient_funds_rejected() {
        let mut mempool = test_mempool();
        let (utxo_set, tx_hash, _) = funded_utxo(1_000);
        let dest = crypto::hash::hash(b"dest");
        // Try to send more than available
        let tx = Transaction::new_transfer(
            vec![Input::new(tx_hash, 0)],
            vec![Output::normal(2_000, dest)],
        );

        let result = mempool.add_transaction(tx, &utxo_set, 100);
        assert!(matches!(result, Err(MempoolError::InvalidTransaction(_))));
    }

    #[test]
    fn test_double_spend_rejected() {
        let mut mempool = test_mempool();
        let (utxo_set, tx_hash, pubkey_hash) = funded_utxo(10_000);

        // First tx spends the UTXO
        let tx1 = Transaction::new_transfer(
            vec![Input::new(tx_hash, 0)],
            vec![Output::normal(9_000, pubkey_hash)],
        );
        mempool.add_transaction(tx1, &utxo_set, 100).unwrap();

        // Second tx tries to spend the same UTXO
        let dest2 = crypto::hash::hash(b"other_dest");
        let tx2 = Transaction::new_transfer(
            vec![Input::new(tx_hash, 0)],
            vec![Output::normal(8_000, dest2)],
        );
        let result = mempool.add_transaction(tx2, &utxo_set, 100);
        assert!(matches!(result, Err(MempoolError::DoubleSpend)));
    }

    #[test]
    fn test_remove_transaction() {
        let mut mempool = test_mempool();
        let (utxo_set, tx_hash, pubkey_hash) = funded_utxo(10_000);
        let tx = simple_transfer(tx_hash, 10_000, 1_000, pubkey_hash);

        let hash = mempool.add_transaction(tx, &utxo_set, 100).unwrap();
        assert_eq!(mempool.len(), 1);

        let removed = mempool.remove_transaction(&hash);
        assert!(removed.is_some());
        assert_eq!(mempool.len(), 0);
        assert!(mempool.is_empty());
    }

    #[test]
    fn test_remove_for_block() {
        let mut mempool = test_mempool();
        let (utxo_set, tx_hash, pubkey_hash) = funded_utxo(10_000);
        let tx = simple_transfer(tx_hash, 10_000, 1_000, pubkey_hash);

        mempool.add_transaction(tx.clone(), &utxo_set, 100).unwrap();
        assert_eq!(mempool.len(), 1);

        mempool.remove_for_block(&[tx]);
        assert_eq!(mempool.len(), 0);
    }

    #[test]
    fn test_select_for_block_respects_size() {
        let mut mempool = test_mempool();

        // Create two funded UTXOs
        let mut utxo_set = UtxoSet::new();
        let pubkey_hash = crypto::hash::hash(b"addr");
        for i in 0..2u8 {
            let tx_hash = crypto::hash::hash(&[i, 1, 2, 3]);
            let outpoint = Outpoint::new(tx_hash, 0);
            utxo_set
                .insert(
                    outpoint,
                    UtxoEntry {
                        output: Output::normal(50_000, pubkey_hash),
                        height: 1,
                        is_coinbase: false,
                        is_epoch_reward: false,
                    },
                )
                .unwrap();
            let tx = simple_transfer(tx_hash, 50_000, 5_000, pubkey_hash);
            mempool.add_transaction(tx, &utxo_set, 100).unwrap();
        }

        assert_eq!(mempool.len(), 2);

        // Select with tiny size limit — should get at most 1
        let selected = mempool.select_for_block(1);
        assert!(selected.len() <= 1);

        // Select with large limit — should get both
        let selected = mempool.select_for_block(1_000_000);
        assert_eq!(selected.len(), 2);
    }

    #[test]
    fn test_cpfp_effective_fee_rate() {
        let mut mempool = test_mempool();
        let pubkey_hash = crypto::hash::hash(b"addr");

        // Create parent UTXO (confirmed)
        let mut utxo_set = UtxoSet::new();
        let parent_funding = crypto::hash::hash(b"parent_fund");
        utxo_set
            .insert(
                Outpoint::new(parent_funding, 0),
                UtxoEntry {
                    output: Output::normal(100_000, pubkey_hash),
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();

        // Parent tx: low fee (fee = 100)
        let parent_tx = Transaction::new_transfer(
            vec![Input::new(parent_funding, 0)],
            vec![Output::normal(99_900, pubkey_hash)],
        );
        let parent_hash = parent_tx.hash();
        mempool.add_transaction(parent_tx, &utxo_set, 100).unwrap();

        // Child tx: high fee (fee = 50_000)
        let child_tx = Transaction::new_transfer(
            vec![Input::new(parent_hash, 0)],
            vec![Output::normal(49_900, pubkey_hash)],
        );
        let child_hash = child_tx.hash();
        mempool.add_transaction(child_tx, &utxo_set, 100).unwrap();

        // Verify CPFP: child's effective_fee_rate includes ancestor package
        let child_entry = mempool.get(&child_hash).unwrap();
        assert!(child_entry.ancestor_fee > child_entry.fee);
        assert!(child_entry.ancestor_size > child_entry.size);
        // effective_fee_rate should be the package rate
        assert!(child_entry.effective_fee_rate() > 0);
    }

    #[test]
    fn test_get_balance_tracking() {
        let mut mempool = test_mempool();
        let (utxo_set, tx_hash, pubkey_hash) = funded_utxo(10_000);

        // Spend UTXO in mempool, send to different address
        let other = crypto::hash::hash(b"other");
        let tx = Transaction::new_transfer(
            vec![Input::new(tx_hash, 0)],
            vec![Output::normal(9_000, other)],
        );
        mempool.add_transaction(tx, &utxo_set, 100).unwrap();

        // Original address should show outgoing
        let (incoming, outgoing) = mempool.calculate_unconfirmed_balance(&pubkey_hash, &utxo_set);
        assert_eq!(outgoing, 10_000); // Spent UTXO

        // Destination address should show incoming
        let (incoming_other, _) = mempool.calculate_unconfirmed_balance(&other, &utxo_set);
        assert_eq!(incoming_other, 9_000);
        assert_eq!(incoming, 0);
    }

    #[test]
    fn test_revalidate_removes_stale() {
        let mut mempool = test_mempool();
        let (utxo_set, tx_hash, pubkey_hash) = funded_utxo(10_000);
        let tx = simple_transfer(tx_hash, 10_000, 1_000, pubkey_hash);

        mempool.add_transaction(tx, &utxo_set, 100).unwrap();
        assert_eq!(mempool.len(), 1);

        // Revalidate against an empty UTXO set (simulates reorg removing the UTXO)
        let empty_utxo = UtxoSet::new();
        mempool.revalidate(&empty_utxo, 200);
        assert_eq!(mempool.len(), 0);
    }

    /// Test that mempool rejects transactions with insufficient per-byte fee.
    /// Uses a Bond output (4 bytes extra_data) which doesn't need covenant activation.
    #[test]
    fn test_mempool_rejects_insufficient_per_byte_fee() {
        let mut mempool = test_mempool();
        let pubkey_hash = crypto::hash::hash(b"fee_test_addr");
        let tx_hash = crypto::hash::hash(b"fee_funding_tx");

        // Bond output: 4 bytes extra_data => minimum_fee = 1 + 4/100 = 1
        // Fund with exactly bond_amount (fee = 0 < minimum 1)
        let bond_amount = 1_000_000_000u64;
        let mut utxo_set = UtxoSet::new();
        let outpoint = Outpoint::new(tx_hash, 0);
        let entry = UtxoEntry {
            output: Output::normal(bond_amount, pubkey_hash),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        utxo_set.insert(outpoint, entry).unwrap();

        // Fee: bond_amount - bond_amount = 0 sats, but needs 1.
        let tx = Transaction {
            version: 1,
            tx_type: doli_core::TxType::Transfer,
            inputs: vec![doli_core::Input::new(tx_hash, 0)],
            outputs: vec![doli_core::Output::bond(
                bond_amount,
                pubkey_hash,
                u64::MAX,
                0,
            )],
            extra_data: vec![],
        };

        let result = mempool.add_transaction(tx, &utxo_set, 100);
        assert!(
            matches!(result, Err(MempoolError::FeeTooLow(0, 1))),
            "Mempool should reject tx with fee 0 when minimum is 1: {:?}",
            result
        );
    }

    /// Test that mempool accepts transactions with sufficient per-byte fee.
    /// Uses a Bond output (4 bytes extra_data) which doesn't need covenant activation.
    #[test]
    fn test_mempool_accepts_sufficient_per_byte_fee() {
        let mut mempool = test_mempool();
        let pubkey_hash = crypto::hash::hash(b"fee_ok_addr");
        let tx_hash = crypto::hash::hash(b"fee_ok_funding");

        // Bond output: 4 bytes extra_data => minimum_fee = 1 + 4 = 5
        // Fund with bond_amount + 100 sats (fee = 100 >> 5)
        let bond_amount = 1_000_000_000u64;
        let mut utxo_set = UtxoSet::new();
        let outpoint = Outpoint::new(tx_hash, 0);
        let entry = UtxoEntry {
            output: Output::normal(bond_amount + 100, pubkey_hash),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        utxo_set.insert(outpoint, entry).unwrap();

        // Fee: (bond_amount + 100) - bond_amount = 100 sats >= 5. Should pass.
        let tx = Transaction {
            version: 1,
            tx_type: doli_core::TxType::Transfer,
            inputs: vec![doli_core::Input::new(tx_hash, 0)],
            outputs: vec![doli_core::Output::bond(
                bond_amount,
                pubkey_hash,
                u64::MAX,
                0,
            )],
            extra_data: vec![],
        };

        let result = mempool.add_transaction(tx, &utxo_set, 100);
        assert!(
            result.is_ok(),
            "Mempool should accept tx with sufficient per-byte fee: {:?}",
            result
        );
    }

    /// Test that mempool accepts plain transfers with fee = 1 (BASE_FEE, no extra_data).
    #[test]
    fn test_mempool_accepts_base_fee_for_transfers() {
        let mut mempool = test_mempool();
        let pubkey_hash = crypto::hash::hash(b"base_fee_addr");
        let tx_hash = crypto::hash::hash(b"base_fee_funding");

        // Fund with 101 sats (output 100 + fee 1 = BASE_FEE)
        let mut utxo_set = UtxoSet::new();
        let outpoint = Outpoint::new(tx_hash, 0);
        let entry = UtxoEntry {
            output: Output::normal(101, pubkey_hash),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        utxo_set.insert(outpoint, entry).unwrap();

        let dest = crypto::hash::hash(b"dest_base_fee");
        let tx = Transaction::new_transfer(
            vec![doli_core::Input::new(tx_hash, 0)],
            vec![Output::normal(100, dest)],
        );

        let result = mempool.add_transaction(tx, &utxo_set, 100);
        assert!(
            result.is_ok(),
            "Plain transfer with fee=1 should be accepted: {:?}",
            result
        );
    }
}
