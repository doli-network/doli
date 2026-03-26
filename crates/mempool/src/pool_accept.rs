//! Mempool transaction acceptance logic
//!
//! Contains `add_transaction` and `add_system_transaction` — the two entry
//! points for inserting transactions into the mempool.

use crypto::Hash;
use doli_core::validation::{validate_transaction, ValidationContext};
use doli_core::{BlockHeight, Transaction};
use storage::{Outpoint, UtxoSet};
use tracing::debug;

use super::entry::MempoolEntry;
use super::pool::{Mempool, MempoolError};

impl Mempool {
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
        let ctx = ValidationContext::new(self.params.clone(), self.network, 0, current_height);
        validate_transaction(&tx, &ctx)
            .map_err(|e| MempoolError::InvalidTransaction(e.to_string()))?;

        // Validate covenant conditions for conditioned inputs (hashlock, timelock, multisig, HTLC)
        for (i, input) in tx.inputs.iter().enumerate() {
            let outpoint = Outpoint::new(input.prev_tx_hash, input.output_index);
            if let Some(entry) = utxo_set.get(&outpoint) {
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

        // Require fee > 0 (flat fee model: any non-zero fee is accepted)
        if fee == 0 {
            return Err(MempoolError::FeeTooLow(0, 1));
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
        let ctx = ValidationContext::new(self.params.clone(), self.network, 0, current_height);
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
}
