use crypto::{Hash, PublicKey};

use crate::types::{Amount, BlockHeight};

use super::data::*;
use super::output::*;
use super::types::*;

use super::core::Transaction;

impl Transaction {
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
}
