use crypto::{Hash, PublicKey};
use serde::{Deserialize, Serialize};

use crate::types::{Amount, BlockHeight};

use super::data::*;
use super::output::*;
use super::types::*;

// ==================== Proof of Time ====================
//
// In Proof of Time, there are NO multi-signature attestations.
// Each block has exactly one producer who receives 100% of the block reward.
//
// Time is proven by producing blocks with valid VDF when selected:
// - One producer per slot (10 seconds)
// - Selection based on bond count (deterministic round-robin)
// - VDF provides anti-grinding protection (~7s computation)
// - Producer receives full block reward via coinbase transaction
//
// This eliminates:
// - Attestation structs and signatures
// - Multi-signature overhead
// - Delegation incentives (giving keys away means losing all rewards)
//
// With EpochPool reward mode, rewards accumulate and are distributed
// fairly at epoch boundaries via EpochReward transactions.

/// A transaction
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    /// Protocol version
    pub version: u32,
    /// Transaction type
    pub tx_type: TxType,
    /// Inputs (references to previous outputs)
    pub inputs: Vec<Input>,
    /// Outputs (new coins being created)
    pub outputs: Vec<Output>,
    /// Extra data (type-specific)
    pub extra_data: Vec<u8>,
}

impl Transaction {
    /// Create a new transfer transaction
    pub fn new_transfer(inputs: Vec<Input>, outputs: Vec<Output>) -> Self {
        Self {
            version: 1,
            tx_type: TxType::Transfer,
            inputs,
            outputs,
            extra_data: Vec::new(),
        }
    }

    /// Create a coinbase transaction
    pub fn new_coinbase(amount: Amount, pubkey_hash: Hash, height: BlockHeight) -> Self {
        Self {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: Vec::new(),
            outputs: vec![Output::normal(amount, pubkey_hash)],
            extra_data: height.to_le_bytes().to_vec(),
        }
    }

    /// Create an epoch reward coinbase with multiple outputs.
    ///
    /// This is used at epoch boundaries to automatically distribute rewards
    /// to all producers who were present during the completed epoch.
    /// Each output pays the calculated reward to a producer's address.
    ///
    /// # Arguments
    /// * `outputs` - Vector of (amount, pubkey_hash) pairs for each producer
    /// * `height` - Block height (used as extra_data for uniqueness)
    /// * `epoch` - The completed epoch number (stored in extra_data)
    pub fn new_epoch_reward_coinbase(
        outputs: Vec<(Amount, Hash)>,
        height: BlockHeight,
        epoch: u64,
    ) -> Self {
        let tx_outputs: Vec<Output> = outputs
            .into_iter()
            .map(|(amount, pubkey_hash)| Output::normal(amount, pubkey_hash))
            .collect();

        // Store both height and epoch in extra_data for auditability
        let mut extra_data = height.to_le_bytes().to_vec();
        extra_data.extend_from_slice(&epoch.to_le_bytes());

        Self {
            version: 1,
            tx_type: TxType::EpochReward, // Use EpochReward type for automatic distribution
            inputs: Vec::new(),
            outputs: tx_outputs,
            extra_data,
        }
    }

    /// Check if this is a coinbase transaction
    ///
    /// A coinbase is a Transfer transaction with no inputs and one output.
    /// Note: ClaimReward transactions also have no inputs and one output,
    /// but they have a different tx_type.
    pub fn is_coinbase(&self) -> bool {
        self.tx_type == TxType::Transfer && self.inputs.is_empty() && self.outputs.len() == 1
    }

    /// Check if this is an epoch reward coinbase (automatic distribution at epoch boundaries)
    ///
    /// Epoch reward coinbase transactions have:
    /// - TxType::EpochReward
    /// - No inputs (minted coins)
    /// - One or more outputs (rewards distributed to present producers)
    pub fn is_epoch_reward_coinbase(&self) -> bool {
        self.tx_type == TxType::EpochReward && self.inputs.is_empty() && !self.outputs.is_empty()
    }

    /// Check if this is any type of reward-minting transaction (coinbase or epoch reward)
    ///
    /// Returns true for both regular single-output coinbase and epoch reward coinbase.
    /// Use this when validating blocks to check for reward transactions.
    pub fn is_reward_minting(&self) -> bool {
        self.is_coinbase() || self.is_epoch_reward_coinbase()
    }

    /// Check if this is an exit transaction
    pub fn is_exit(&self) -> bool {
        self.tx_type == TxType::Exit
    }

    /// Check if this is a registration transaction
    pub fn is_registration(&self) -> bool {
        self.tx_type == TxType::Registration
    }

    /// Check if this is a claim reward transaction
    pub fn is_claim_reward(&self) -> bool {
        self.tx_type == TxType::ClaimReward
    }

    /// Check if this is an epoch reward transaction
    pub fn is_epoch_reward(&self) -> bool {
        self.tx_type == TxType::EpochReward
    }

    /// Create an epoch reward transaction for fair distribution
    ///
    /// This transaction type is used at epoch boundaries to distribute
    /// accumulated rewards fairly among all producers who participated
    /// in the epoch.
    pub fn new_epoch_reward(
        epoch: u64,
        recipient_pubkey: PublicKey,
        amount: Amount,
        recipient_hash: Hash,
    ) -> Self {
        let data = EpochRewardData::new(epoch, recipient_pubkey);
        Self {
            version: 1,
            tx_type: TxType::EpochReward,
            inputs: Vec::new(), // Minted, no inputs
            outputs: vec![Output::normal(amount, recipient_hash)],
            extra_data: data.to_bytes(),
        }
    }

    /// Parse epoch reward data from extra_data
    pub fn epoch_reward_data(&self) -> Option<EpochRewardData> {
        if self.tx_type != TxType::EpochReward {
            return None;
        }
        EpochRewardData::from_bytes(&self.extra_data)
    }

    /// Returns true if this transaction type has no UTXO inputs by design.
    ///
    /// State-only txs (Exit, RequestWithdrawal, etc.) operate on producer state
    /// and are spam-protected by requiring a registered producer bond. They bypass
    /// UTXO-based fee accounting in the mempool.
    ///
    /// Registration and AddBond are NOT state-only — they consume UTXO inputs.
    pub fn is_state_only(&self) -> bool {
        matches!(
            self.tx_type,
            TxType::Exit
                | TxType::ClaimReward
                | TxType::ClaimBond
                | TxType::SlashProducer
                | TxType::DelegateBond
                | TxType::RevokeDelegation
                | TxType::AddMaintainer
                | TxType::RemoveMaintainer
        )
    }

    /// Compute the transaction hash
    pub fn hash(&self) -> Hash {
        use crypto::Hasher;

        let mut hasher = Hasher::new();
        hasher.update(&self.version.to_le_bytes());
        hasher.update(&(self.tx_type as u32).to_le_bytes());

        // Hash inputs (without signatures for tx hash)
        hasher.update(&(self.inputs.len() as u32).to_le_bytes());
        for input in &self.inputs {
            hasher.update(input.prev_tx_hash.as_bytes());
            hasher.update(&input.output_index.to_le_bytes());
            // Signature is NOT included in tx hash
        }

        // Hash outputs
        hasher.update(&(self.outputs.len() as u32).to_le_bytes());
        for output in &self.outputs {
            hasher.update(&output.serialize());
        }

        // Hash extra data
        hasher.update(&(self.extra_data.len() as u32).to_le_bytes());
        hasher.update(&self.extra_data);

        hasher.finalize()
    }

    /// Get the message to sign for a given input.
    ///
    /// Excludes extra_data (covenant witnesses) from the hash, analogous to
    /// Bitcoin SegWit: witnesses are committed in `hash()` for immutability,
    /// but excluded from the signing message to avoid a chicken-and-egg where
    /// the witness signature depends on a hash that includes the witness itself.
    pub fn signing_message(&self) -> Hash {
        use crypto::Hasher;

        let mut hasher = Hasher::new();
        hasher.update(&self.version.to_le_bytes());
        hasher.update(&(self.tx_type as u32).to_le_bytes());

        hasher.update(&(self.inputs.len() as u32).to_le_bytes());
        for input in &self.inputs {
            hasher.update(input.prev_tx_hash.as_bytes());
            hasher.update(&input.output_index.to_le_bytes());
        }

        hasher.update(&(self.outputs.len() as u32).to_le_bytes());
        for output in &self.outputs {
            hasher.update(&output.serialize());
        }

        // extra_data (covenant witnesses) intentionally excluded — SegWit-style
        hasher.finalize()
    }

    /// Get the signing message for a specific input, respecting its sighash type.
    ///
    /// BIP-143 style: each input's signing hash includes its own outpoint
    /// (prevTxHash || outputIndex), producing unique signatures per input.
    ///
    /// - `SighashType::All`: all inputs + all outputs + THIS input's outpoint.
    /// - `SighashType::AnyoneCanPay`: only THIS input + all outputs.
    ///   Allows other parties to add inputs after the signer has committed.
    pub fn signing_message_for_input(&self, input_index: usize) -> Hash {
        let input = &self.inputs[input_index];
        match input.sighash_type {
            SighashType::All => {
                use crypto::Hasher;
                let mut hasher = Hasher::new();
                hasher.update(&self.version.to_le_bytes());
                hasher.update(&(self.tx_type as u32).to_le_bytes());

                // All inputs (same as signing_message)
                hasher.update(&(self.inputs.len() as u32).to_le_bytes());
                for inp in &self.inputs {
                    hasher.update(inp.prev_tx_hash.as_bytes());
                    hasher.update(&inp.output_index.to_le_bytes());
                }

                // All outputs
                hasher.update(&(self.outputs.len() as u32).to_le_bytes());
                for output in &self.outputs {
                    hasher.update(&output.serialize());
                }

                // BIP-143: per-input outpoint for unique signing hash
                hasher.update(input.prev_tx_hash.as_bytes());
                hasher.update(&input.output_index.to_le_bytes());

                hasher.finalize()
            }
            SighashType::AnyoneCanPay => {
                use crypto::Hasher;
                let mut hasher = Hasher::new();
                hasher.update(&self.version.to_le_bytes());
                hasher.update(&(self.tx_type as u32).to_le_bytes());

                // ANYONECANPAY: hash only this single input
                hasher.update(&1u32.to_le_bytes()); // input count = 1
                hasher.update(input.prev_tx_hash.as_bytes());
                hasher.update(&input.output_index.to_le_bytes());

                // Determine how many outputs to commit to.
                // committed_output_count == 0 → all outputs (backward compat).
                // committed_output_count > 0 → first N outputs only, allowing
                // the buyer to append outputs (e.g. change) after signing.
                let output_count = if input.committed_output_count > 0 {
                    (input.committed_output_count as usize).min(self.outputs.len())
                } else {
                    self.outputs.len()
                };

                hasher.update(&(output_count as u32).to_le_bytes());
                for output in &self.outputs[..output_count] {
                    hasher.update(&output.serialize());
                }

                hasher.finalize()
            }
        }
    }

    /// Encode covenant witness data into extra_data for a Transfer transaction.
    ///
    /// Witness map format: for each input, `[u16 LE length][witness bytes]`.
    /// For inputs spending Normal/Bond outputs, length is 0.
    /// For inputs spending conditioned outputs, length > 0 with encoded Witness.
    ///
    /// This is the DOLI equivalent of Bitcoin's SegWit: witness data lives in
    /// extra_data (which IS part of the tx hash, preventing malleability).
    pub fn set_covenant_witnesses(&mut self, witnesses: &[Vec<u8>]) {
        assert_eq!(
            witnesses.len(),
            self.inputs.len(),
            "witness count must match input count"
        );
        let mut buf = Vec::new();
        for w in witnesses {
            buf.extend_from_slice(&(w.len() as u16).to_le_bytes());
            buf.extend_from_slice(w);
        }
        self.extra_data = buf;
    }

    /// Decode covenant witness data from extra_data.
    /// Returns a witness bytes slice for each input, or None if extra_data
    /// doesn't contain witness data (normal transfers, coinbase, etc.).
    pub fn get_covenant_witness(&self, input_index: usize) -> Option<&[u8]> {
        if self.extra_data.is_empty() || self.tx_type != TxType::Transfer {
            return None;
        }
        let mut pos = 0;
        let data = &self.extra_data;
        for i in 0.. {
            if pos + 2 > data.len() {
                return None;
            }
            let len = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
            pos += 2;
            if i == input_index {
                if len == 0 {
                    return Some(&[]);
                }
                if pos + len > data.len() {
                    return None;
                }
                return Some(&data[pos..pos + len]);
            }
            pos += len;
        }
        None // unreachable in practice
    }

    /// Calculate total input amount (requires UTXO lookup - returns 0 here)
    pub fn total_output(&self) -> Amount {
        self.outputs.iter().map(|o| o.amount).sum()
    }

    /// Serialize the transaction
    pub fn serialize(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    /// Deserialize a transaction
    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        bincode::deserialize(bytes).ok()
    }

    /// Get the size in bytes
    pub fn size(&self) -> usize {
        self.serialize().len()
    }
}
