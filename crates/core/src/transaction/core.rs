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
    ///
    /// Post-UNIQUE_COINBASE_ACTIVATION_HEIGHT: extra_data includes slot (globally unique)
    /// to prevent duplicate TX hashes when producer is 1 block behind.
    pub fn new_coinbase(amount: Amount, pubkey_hash: Hash, height: BlockHeight, slot: u32) -> Self {
        let extra_data = if height >= crate::consensus::UNIQUE_COINBASE_ACTIVATION_HEIGHT {
            // height (8 bytes) + slot (4 bytes) = unique per block
            let mut data = height.to_le_bytes().to_vec();
            data.extend_from_slice(&slot.to_le_bytes());
            data
        } else {
            // Legacy: height only
            height.to_le_bytes().to_vec()
        };
        Self {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: Vec::new(),
            outputs: vec![Output::normal(amount, pubkey_hash)],
            extra_data,
        }
    }

    /// Create an epoch reward coinbase with multiple outputs.
    ///
    /// This is used at epoch boundaries to automatically distribute rewards
    /// to all producers who were present during the completed epoch.
    /// Each output pays the calculated reward to a producer's address.
    ///
    /// # Arguments
    /// * `pool_inputs` - Sorted pool UTXO outpoints to consume (empty pre-activation,
    ///   explicit post-`EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT`).
    /// * `outputs` - Vector of (amount, pubkey_hash) pairs for each producer
    /// * `height` - Block height (used as extra_data for uniqueness)
    /// * `epoch` - The completed epoch number (stored in extra_data)
    pub fn new_epoch_reward_coinbase(
        pool_inputs: Vec<(Hash, u32)>,
        outputs: Vec<(Amount, Hash)>,
        height: BlockHeight,
        epoch: u64,
    ) -> Self {
        let tx_inputs: Vec<Input> = pool_inputs
            .into_iter()
            .map(|(tx_hash, index)| Input::new(tx_hash, index))
            .collect();

        let tx_outputs: Vec<Output> = outputs
            .into_iter()
            .map(|(amount, pubkey_hash)| Output::normal(amount, pubkey_hash))
            .collect();

        // Store both height and epoch in extra_data for auditability
        let mut extra_data = height.to_le_bytes().to_vec();
        extra_data.extend_from_slice(&epoch.to_le_bytes());

        Self {
            version: 1,
            tx_type: TxType::EpochReward,
            inputs: tx_inputs,
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
    /// - One or more outputs (rewards distributed to present producers)
    /// - Pre-activation: no inputs (pool consumed by side-effect)
    /// - Post-activation: explicit pool UTXO inputs
    pub fn is_epoch_reward_coinbase(&self) -> bool {
        self.tx_type == TxType::EpochReward && !self.outputs.is_empty()
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

    /// Create a registration transaction
    ///
    /// This creates a transaction to register as a block producer.
    /// Inputs must cover the bond amount. The bond is locked for the specified duration.
    ///
    /// # Arguments
    /// - `inputs`: UTXOs to spend for the bond
    /// - `public_key`: Producer's public key
    /// - `bond_amount`: Total amount to lock as bond
    /// - `lock_until`: Block height until which the bond is locked (must be >= current_height + blocks_per_era)
    ///
    /// Note: For simplicity, this creates a basic registration. For full
    /// registration with VDF proofs, use the node's registration flow.
    pub fn new_registration(
        inputs: Vec<Input>,
        public_key: PublicKey,
        bond_amount: Amount,
        lock_until: BlockHeight,
        bond_count: u32,
    ) -> Self {
        // Create registration data without VDF (for CLI use)
        // Full VDF registration happens through node's registration flow
        let reg_data = RegistrationData {
            public_key,
            epoch: 0,
            vdf_output: Vec::new(),
            vdf_proof: Vec::new(),
            prev_registration_hash: Hash::ZERO,
            sequence_number: 0,
            bond_count,
            bls_pubkey: Vec::new(),
            bls_pop: Vec::new(),
        };
        let extra_data = bincode::serialize(&reg_data).unwrap_or_default();

        // Create one Bond UTXO per bond (each with bond_unit amount)
        // so each bond has its own creation_slot for FIFO vesting
        let pubkey_hash =
            crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, public_key.as_bytes());
        let bond_unit = if bond_count > 0 {
            bond_amount / bond_count as u64
        } else {
            bond_amount
        };
        let outputs: Vec<Output> = (0..bond_count)
            .map(|_| Output::bond(bond_unit, pubkey_hash, lock_until, 0))
            .collect();

        Self {
            version: 1,
            tx_type: TxType::Registration,
            inputs,
            outputs,
            extra_data,
        }
    }

    /// Create an exit transaction
    pub fn new_exit(public_key: PublicKey) -> Self {
        let exit_data = ExitData { public_key };
        let extra_data = bincode::serialize(&exit_data).unwrap_or_default();

        Self {
            version: 1,
            tx_type: TxType::Exit,
            inputs: Vec::new(),  // No inputs needed for exit
            outputs: Vec::new(), // No outputs - bond released after cooldown
            extra_data,
        }
    }

    /// Parse exit data from extra_data
    pub fn exit_data(&self) -> Option<ExitData> {
        if self.tx_type != TxType::Exit {
            return None;
        }
        bincode::deserialize(&self.extra_data).ok()
    }

    /// Create a claim reward transaction
    ///
    /// The producer claims their accumulated rewards as a single UTXO.
    /// The output amount is determined by the node based on pending_rewards.
    pub fn new_claim_reward(public_key: PublicKey, amount: Amount, recipient_hash: Hash) -> Self {
        let claim_data = ClaimData { public_key };
        let extra_data = bincode::serialize(&claim_data).unwrap_or_default();

        Self {
            version: 1,
            tx_type: TxType::ClaimReward,
            inputs: Vec::new(), // No inputs - rewards come from pending balance
            outputs: vec![Output::normal(amount, recipient_hash)],
            extra_data,
        }
    }

    /// Parse claim data from extra_data
    pub fn claim_data(&self) -> Option<ClaimData> {
        if self.tx_type != TxType::ClaimReward {
            return None;
        }
        bincode::deserialize(&self.extra_data).ok()
    }

    /// Create a claim bond transaction
    ///
    /// After the unbonding period completes, the producer can claim their bond.
    /// The amount depends on whether it was a normal exit (100% returned) or
    /// early exit (proportional penalty applied).
    pub fn new_claim_bond(public_key: PublicKey, amount: Amount, recipient_hash: Hash) -> Self {
        let claim_bond_data = ClaimBondData { public_key };
        let extra_data = bincode::serialize(&claim_bond_data).unwrap_or_default();

        Self {
            version: 1,
            tx_type: TxType::ClaimBond,
            inputs: Vec::new(), // No inputs - bond comes from protocol
            outputs: vec![Output::normal(amount, recipient_hash)],
            extra_data,
        }
    }

    /// Check if this is a claim bond transaction
    pub fn is_claim_bond(&self) -> bool {
        self.tx_type == TxType::ClaimBond
    }

    /// Parse claim bond data from extra_data
    pub fn claim_bond_data(&self) -> Option<ClaimBondData> {
        if self.tx_type != TxType::ClaimBond {
            return None;
        }
        bincode::deserialize(&self.extra_data).ok()
    }

    /// Create a slash producer transaction
    ///
    /// Anyone can submit slashing evidence against a misbehaving producer.
    /// If valid, the producer's bond is burned (100%).
    pub fn new_slash_producer(slash_data: SlashData) -> Self {
        let extra_data = bincode::serialize(&slash_data).unwrap_or_default();

        Self {
            version: 1,
            tx_type: TxType::SlashProducer,
            inputs: Vec::new(),  // No inputs
            outputs: Vec::new(), // No outputs - bond is burned
            extra_data,
        }
    }

    /// Check if this is a slash producer transaction
    pub fn is_slash_producer(&self) -> bool {
        self.tx_type == TxType::SlashProducer
    }

    /// Parse slash data from extra_data
    pub fn slash_data(&self) -> Option<SlashData> {
        if self.tx_type != TxType::SlashProducer {
            return None;
        }
        bincode::deserialize(&self.extra_data).ok()
    }

    // ==================== Bond Stacking Transactions ====================

    /// Create an add bond transaction.
    ///
    /// Inputs: Normal UTXOs covering bond amount + fee
    /// Outputs: Bond UTXO (locked) + optional change
    ///
    /// The producer must already be registered.
    pub fn new_add_bond(
        inputs: Vec<Input>,
        producer_pubkey: PublicKey,
        bond_count: u32,
        bond_amount: Amount,
        lock_until: BlockHeight,
    ) -> Self {
        let bond_data = AddBondData::new(producer_pubkey, bond_count);
        let extra_data = bond_data.to_bytes();

        // Create one Bond UTXO per bond (each with bond_unit amount)
        // so each bond has its own creation_slot for FIFO vesting
        let pubkey_hash =
            crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, producer_pubkey.as_bytes());
        let bond_unit = if bond_count > 0 {
            bond_amount / bond_count as u64
        } else {
            bond_amount
        };
        let outputs: Vec<Output> = (0..bond_count)
            .map(|_| Output::bond(bond_unit, pubkey_hash, lock_until, 0))
            .collect();

        Self {
            version: 1,
            tx_type: TxType::AddBond,
            inputs,
            outputs,
            extra_data,
        }
    }

    /// Check if this is an add bond transaction
    pub fn is_add_bond(&self) -> bool {
        self.tx_type == TxType::AddBond
    }

    /// Parse add bond data from extra_data
    pub fn add_bond_data(&self) -> Option<AddBondData> {
        if self.tx_type != TxType::AddBond {
            return None;
        }
        AddBondData::from_bytes(&self.extra_data)
    }

    /// Create a withdrawal request transaction.
    ///
    /// Lock/unlock model: Bond UTXOs are consumed as inputs, Normal UTXO created as output.
    /// Penalty is implicitly burned (sum(bond inputs) − net_amount = burned).
    /// Bonds are removed from registry at next epoch boundary.
    pub fn new_request_withdrawal(
        inputs: Vec<Input>,
        producer_pubkey: PublicKey,
        bond_count: u32,
        destination: Hash,
        net_amount: Amount,
    ) -> Self {
        let withdrawal_data = WithdrawalRequestData::new(producer_pubkey, bond_count, destination);
        let extra_data = withdrawal_data.to_bytes();

        Self {
            version: 1,
            tx_type: TxType::RequestWithdrawal,
            inputs, // Bond UTXOs consumed (lock → unlock)
            outputs: vec![Output::normal(net_amount, destination)],
            extra_data,
        }
    }

    /// Check if this is a withdrawal request transaction
    pub fn is_request_withdrawal(&self) -> bool {
        self.tx_type == TxType::RequestWithdrawal
    }

    /// Parse withdrawal request data from extra_data
    pub fn withdrawal_request_data(&self) -> Option<WithdrawalRequestData> {
        if self.tx_type != TxType::RequestWithdrawal {
            return None;
        }
        WithdrawalRequestData::from_bytes(&self.extra_data)
    }

    /// Parse registration data from extra_data
    pub fn registration_data(&self) -> Option<RegistrationData> {
        if self.tx_type != TxType::Registration {
            return None;
        }
        bincode::deserialize(&self.extra_data).ok()
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
    ///
    /// ENCODING SCHEMES FOR LENGTH-PREFIXED DATA
    ///
    /// Output::serialize() (hash-only, never deserialized):
    ///   extra_data.len() < 65536  -> u16 LE (backward-compatible with all existing blocks)
    ///   extra_data.len() >= 65536 -> u32 LE (for large NFTs >64KB)
    ///   No ambiguity because this is never deserialized -- bytes go into BLAKE3.
    ///
    /// Covenant witnesses (serialized AND deserialized):
    ///   witness.len() < 65535   -> u16 LE (backward-compatible)
    ///   witness.len() >= 65535  -> escape marker 0xFFFF + u32 LE
    ///   0xFFFF is unambiguous because real u16 lengths max at 65534.
    pub fn set_covenant_witnesses(&mut self, witnesses: &[Vec<u8>]) {
        assert_eq!(
            witnesses.len(),
            self.inputs.len(),
            "witness count must match input count"
        );
        let mut buf = Vec::new();
        for w in witnesses {
            if w.len() >= 65535 {
                buf.extend_from_slice(&0xFFFFu16.to_le_bytes());
                buf.extend_from_slice(&(w.len() as u32).to_le_bytes());
            } else {
                buf.extend_from_slice(&(w.len() as u16).to_le_bytes());
            }
            buf.extend_from_slice(w);
        }
        self.extra_data = buf;
    }

    /// Decode covenant witness data from extra_data.
    /// Returns a witness bytes slice for each input, or None if extra_data
    /// doesn't contain witness data.
    pub fn get_covenant_witness(&self, input_index: usize) -> Option<&[u8]> {
        if self.extra_data.is_empty() {
            return None;
        }
        let mut pos = 0;
        let data = &self.extra_data;
        for i in 0.. {
            if pos + 2 > data.len() {
                return None;
            }
            let raw_len = u16::from_le_bytes([data[pos], data[pos + 1]]);
            pos += 2;
            let len = if raw_len == 0xFFFF {
                // Escape marker: next 4 bytes are u32 length
                if pos + 4 > data.len() {
                    return None;
                }
                let l = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
                    as usize;
                pos += 4;
                l
            } else {
                raw_len as usize
            };
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

    /// Calculate the minimum required fee for this transaction.
    ///
    /// `fee = BASE_FEE + (sum(extra_data.len()) * FEE_PER_BYTE) / FEE_DIVISOR`
    ///
    /// With FEE_DIVISOR=100, this is 100x cheaper than 1:1 sat/byte:
    /// - Transfer (0 extra_data bytes): 1 sat
    /// - Bond (4 bytes): 1 sat (minimum BASE_FEE)
    /// - NFT (300 bytes): 4 sats
    /// - 512 KB image: 5,243 sats (~0.052 DOLI)
    pub fn minimum_fee(&self) -> Amount {
        let extra_bytes: u64 = self.outputs.iter().map(|o| o.extra_data.len() as u64).sum();
        let byte_fee =
            (extra_bytes * crate::consensus::FEE_PER_BYTE) / crate::consensus::FEE_DIVISOR;
        crate::consensus::BASE_FEE + byte_fee
    }

    /// Sum of native DOLI across all outputs.
    ///
    /// Non-native output types (FungibleAsset, LPShare, Pool, Collateral)
    /// store token units / LP shares / zero in `amount` and are excluded.
    pub fn total_output(&self) -> Amount {
        self.outputs
            .iter()
            .filter(|o| o.output_type.is_native_amount())
            .map(|o| o.amount)
            .sum()
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

    // ==================== Maintainer Transactions ====================

    /// Create a remove maintainer transaction
    ///
    /// This transaction removes a maintainer from the auto-update system.
    /// Requires 3/5 signatures from OTHER maintainers (target cannot sign).
    ///
    /// # Arguments
    /// * `target` - Public key of the maintainer to remove
    /// * `signatures` - Signatures from at least 3 other maintainers
    /// * `reason` - Optional reason for removal (for transparency)
    pub fn new_remove_maintainer(
        target: PublicKey,
        signatures: Vec<crate::maintainer::MaintainerSignature>,
        reason: Option<String>,
    ) -> Self {
        let data = if let Some(r) = reason {
            crate::maintainer::MaintainerChangeData::with_reason(target, signatures, r)
        } else {
            crate::maintainer::MaintainerChangeData::new(target, signatures)
        };

        Self {
            version: 1,
            tx_type: TxType::RemoveMaintainer,
            inputs: Vec::new(),
            outputs: Vec::new(),
            extra_data: data.to_bytes(),
        }
    }

    /// Create an add maintainer transaction
    ///
    /// This transaction adds a new maintainer to the auto-update system.
    /// Requires 3/5 signatures from current maintainers.
    /// Target must be a registered producer.
    ///
    /// # Arguments
    /// * `target` - Public key of the producer to add as maintainer
    /// * `signatures` - Signatures from at least 3 current maintainers
    pub fn new_add_maintainer(
        target: PublicKey,
        signatures: Vec<crate::maintainer::MaintainerSignature>,
    ) -> Self {
        let data = crate::maintainer::MaintainerChangeData::new(target, signatures);

        Self {
            version: 1,
            tx_type: TxType::AddMaintainer,
            inputs: Vec::new(),
            outputs: Vec::new(),
            extra_data: data.to_bytes(),
        }
    }

    /// Check if this is a remove maintainer transaction
    pub fn is_remove_maintainer(&self) -> bool {
        self.tx_type == TxType::RemoveMaintainer
    }

    /// Check if this is an add maintainer transaction
    pub fn is_add_maintainer(&self) -> bool {
        self.tx_type == TxType::AddMaintainer
    }

    /// Check if this is any maintainer change transaction
    pub fn is_maintainer_change(&self) -> bool {
        self.is_remove_maintainer() || self.is_add_maintainer()
    }

    /// Parse maintainer change data from extra_data
    pub fn maintainer_change_data(&self) -> Option<crate::maintainer::MaintainerChangeData> {
        if !self.is_maintainer_change() {
            return None;
        }
        crate::maintainer::MaintainerChangeData::from_bytes(&self.extra_data)
    }

    /// Check if this is a delegate bond transaction
    pub fn is_delegate_bond(&self) -> bool {
        self.tx_type == TxType::DelegateBond
    }

    /// Check if this is a revoke delegation transaction
    pub fn is_revoke_delegation(&self) -> bool {
        self.tx_type == TxType::RevokeDelegation
    }

    /// Parse delegate bond data from extra_data
    pub fn delegate_bond_data(&self) -> Option<DelegateBondData> {
        if !self.is_delegate_bond() {
            return None;
        }
        DelegateBondData::from_bytes(&self.extra_data)
    }

    /// Parse revoke delegation data from extra_data
    pub fn revoke_delegation_data(&self) -> Option<RevokeDelegationData> {
        if !self.is_revoke_delegation() {
            return None;
        }
        RevokeDelegationData::from_bytes(&self.extra_data)
    }

    /// Create a new delegate bond transaction
    pub fn new_delegate_bond(data: DelegateBondData) -> Self {
        Self {
            version: 1,
            tx_type: TxType::DelegateBond,
            inputs: Vec::new(),
            outputs: Vec::new(),
            extra_data: data.to_bytes(),
        }
    }

    /// Create a new revoke delegation transaction
    pub fn new_revoke_delegation(data: RevokeDelegationData) -> Self {
        Self {
            version: 1,
            tx_type: TxType::RevokeDelegation,
            inputs: Vec::new(),
            outputs: Vec::new(),
            extra_data: data.to_bytes(),
        }
    }

    // ==================== Protocol Activation Transactions ====================

    /// Create a new protocol activation transaction
    ///
    /// Schedules new consensus rules to activate at a future epoch boundary.
    /// Requires 3/5 maintainer multisig.
    pub fn new_protocol_activation(data: crate::maintainer::ProtocolActivationData) -> Self {
        Self {
            version: 1,
            tx_type: TxType::ProtocolActivation,
            inputs: Vec::new(),
            outputs: Vec::new(),
            extra_data: data.to_bytes(),
        }
    }

    /// Check if this is a protocol activation transaction
    pub fn is_protocol_activation(&self) -> bool {
        self.tx_type == TxType::ProtocolActivation
    }

    /// Parse protocol activation data from extra_data
    pub fn protocol_activation_data(&self) -> Option<crate::maintainer::ProtocolActivationData> {
        if !self.is_protocol_activation() {
            return None;
        }
        crate::maintainer::ProtocolActivationData::from_bytes(&self.extra_data)
    }
}

#[cfg(test)]
mod coinbase_hash_tests {
    use super::*;
    use crypto::Hash;

    #[test]
    fn coinbase_different_heights_produce_different_hashes() {
        let pool_hash = Hash::default();
        let cb1 = Transaction::new_coinbase(100_000_000, pool_hash, 1910, 0);
        let cb2 = Transaction::new_coinbase(100_000_000, pool_hash, 1911, 0);

        println!("cb1 extra_data: {:?}", cb1.extra_data);
        println!("cb2 extra_data: {:?}", cb2.extra_data);
        println!("cb1 hash: {}", cb1.hash());
        println!("cb2 hash: {}", cb2.hash());
        println!("Same hash: {}", cb1.hash() == cb2.hash());

        assert_ne!(
            cb1.hash(),
            cb2.hash(),
            "Coinbase at different heights must have different hashes"
        );
    }
}
