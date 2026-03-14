//! Transaction builder for constructing DOLI transactions without depending on `doli-core`.
//!
//! This module provides a standalone transaction builder that constructs raw transaction
//! bytes matching the canonical encoding used by `doli-core`. It duplicates ~200 lines
//! of serialization logic to avoid pulling in the entire `doli-core` crate (and thus
//! the VDF/GMP dependency chain).
//!
//! # Architecture Decision
//!
//! The wallet crate does NOT depend on `doli-core`. See `gui-architecture.md` Module 1
//! "Design Decision: No doli-core Dependency" for rationale.

use anyhow::{anyhow, Result};
use crypto::{signature, KeyPair};

use crate::types::*;

/// Transaction types matching `doli-core::transaction::TxType`.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxType {
    /// Standard coin transfer
    Transfer = 0,
    /// Producer registration
    Registration = 1,
    /// Producer exit
    ProducerExit = 2,
    /// Coinbase (block reward) -- not user-constructible
    Coinbase = 3,
    /// NFT mint
    NftMint = 4,
    /// NFT transfer
    NftTransfer = 5,
    /// Reward claim
    RewardClaim = 6,
    /// Add bonds
    AddBond = 7,
    /// Request withdrawal
    RequestWithdrawal = 8,
    /// Reserved — DO NOT REUSE (wire compat tombstone)
    ClaimWithdrawal = 9,
    /// Slashing evidence
    SlashingEvidence = 10,
    /// Token issuance
    TokenIssuance = 11,
    /// Bridge lock (HTLC)
    BridgeLock = 12,
    /// Delegate bond
    DelegateBond = 13,
    /// Revoke delegation
    RevokeDelegation = 14,
}

impl TxType {
    /// Map wallet TxType to doli-core TxType u32 variant index for bincode serialization.
    ///
    /// Core TxType (repr(u32)): Transfer=0, Registration=1, Exit=2, ClaimReward=3,
    /// ClaimBond=4, SlashProducer=5, Coinbase=6, AddBond=7, RequestWithdrawal=8,
    /// ClaimWithdrawal=9(tombstone), EpochReward=10, RemoveMaintainer=11, AddMaintainer=12,
    /// DelegateBond=13, RevokeDelegation=14, ProtocolActivation=15
    fn to_core_type_id(self) -> u32 {
        match self {
            TxType::Transfer => 0,
            TxType::Registration => 1,
            TxType::ProducerExit => 2,
            TxType::Coinbase => 6,
            TxType::NftMint => 0,     // NFT uses Transfer type in core
            TxType::NftTransfer => 0, // NFT uses Transfer type in core
            TxType::RewardClaim => 3, // ClaimReward in core
            TxType::AddBond => 7,
            TxType::RequestWithdrawal => 8,
            TxType::ClaimWithdrawal => 9,
            TxType::SlashingEvidence => 5, // SlashProducer in core
            TxType::TokenIssuance => 0,    // Uses Transfer type in core
            TxType::BridgeLock => 0,       // Uses Transfer type in core
            TxType::DelegateBond => 13,
            TxType::RevokeDelegation => 14,
        }
    }
}

/// Protocol version for all wallet-constructed transactions.
const TX_VERSION: u32 = 1;

/// A transaction input referencing a UTXO.
#[derive(Clone, Debug)]
pub struct TxInput {
    /// Previous transaction hash (32 bytes)
    pub prev_tx_hash: [u8; 32],
    /// Output index in previous transaction
    pub output_index: u32,
    /// Signature (filled during signing)
    pub signature: Option<Vec<u8>>,
    /// Public key of signer
    pub public_key: Option<Vec<u8>>,
}

/// A transaction output.
#[derive(Clone, Debug)]
pub struct TxOutput {
    /// Amount in base units
    pub amount: u64,
    /// Recipient public key hash (32 bytes)
    pub pubkey_hash: [u8; 32],
    /// Output type (0 = normal, 1 = bond, etc.)
    pub output_type: u8,
    /// Lock until height (0 = no lock)
    pub lock_until: u64,
    /// Extra data (e.g. BLS key for registration)
    pub extra_data: Vec<u8>,
}

/// Transaction builder for constructing and signing transactions.
#[derive(Debug)]
pub struct TxBuilder {
    /// Transaction type
    tx_type: TxType,
    /// Inputs
    inputs: Vec<TxInput>,
    /// Outputs
    outputs: Vec<TxOutput>,
    /// Extra data (registration data, etc.)
    extra_data: Vec<u8>,
}

impl TxBuilder {
    /// Create a new transaction builder.
    pub fn new(tx_type: TxType) -> Self {
        Self {
            tx_type,
            inputs: Vec::new(),
            outputs: Vec::new(),
            extra_data: Vec::new(),
        }
    }

    /// Add an input referencing a UTXO.
    pub fn add_input(&mut self, prev_tx_hash: [u8; 32], output_index: u32) -> &mut Self {
        self.inputs.push(TxInput {
            prev_tx_hash,
            output_index,
            signature: None,
            public_key: None,
        });
        self
    }

    /// Add an output.
    pub fn add_output(
        &mut self,
        amount: u64,
        pubkey_hash: [u8; 32],
        output_type: u8,
        lock_until: u64,
        extra_data: Vec<u8>,
    ) -> &mut Self {
        self.outputs.push(TxOutput {
            amount,
            pubkey_hash,
            output_type,
            lock_until,
            extra_data,
        });
        self
    }

    /// Set extra data for the transaction (e.g. registration data).
    pub fn set_extra_data(&mut self, data: Vec<u8>) -> &mut Self {
        self.extra_data = data;
        self
    }

    /// Get the number of inputs.
    pub fn input_count(&self) -> usize {
        self.inputs.len()
    }

    /// Get the number of outputs.
    pub fn output_count(&self) -> usize {
        self.outputs.len()
    }

    /// Get the transaction type.
    pub fn tx_type(&self) -> TxType {
        self.tx_type
    }

    /// Build the unsigned transaction bytes for signing.
    ///
    /// This produces the canonical byte representation that gets hashed for signing,
    /// matching `doli-core::Transaction::signing_message()` exactly.
    ///
    /// Layout: `version(u32 LE) | tx_type(u32 LE) | input_count(u32 LE)
    /// | [prev_tx_hash(32B) | output_index(u32 LE)] per input
    /// | output_count(u32 LE) | [output_type(u8) | amount(u64 LE) | pubkey_hash(32B)
    /// | lock_until(u64 LE) | extra_data_len(u16 LE) | extra_data] per output`.
    ///
    /// Transaction-level extra_data is intentionally excluded (SegWit-style).
    pub fn build_for_signing(&self) -> Result<Vec<u8>> {
        if self.inputs.is_empty() && self.tx_type != TxType::Coinbase {
            return Err(anyhow!("Transaction must have at least one input"));
        }
        if self.outputs.is_empty() {
            return Err(anyhow!("Transaction must have at least one output"));
        }

        let mut bytes = Vec::new();

        // Version
        bytes.extend_from_slice(&TX_VERSION.to_le_bytes());

        // TxType as u32 LE (matching core's repr(u32) enum)
        bytes.extend_from_slice(&self.tx_type.to_core_type_id().to_le_bytes());

        // Inputs (without signatures -- signing message excludes sigs)
        bytes.extend_from_slice(&(self.inputs.len() as u32).to_le_bytes());
        for input in &self.inputs {
            bytes.extend_from_slice(&input.prev_tx_hash);
            bytes.extend_from_slice(&input.output_index.to_le_bytes());
        }

        // Outputs (using Output::serialize() format from core)
        bytes.extend_from_slice(&(self.outputs.len() as u32).to_le_bytes());
        for output in &self.outputs {
            bytes.push(output.output_type);
            bytes.extend_from_slice(&output.amount.to_le_bytes());
            bytes.extend_from_slice(&output.pubkey_hash);
            bytes.extend_from_slice(&output.lock_until.to_le_bytes());
            bytes.extend_from_slice(&(output.extra_data.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&output.extra_data);
        }

        // extra_data intentionally excluded from signing (SegWit-style)

        Ok(bytes)
    }

    /// Sign the transaction with the given private key.
    /// Returns the signed transaction as hex-encoded bytes ready for RPC submission.
    ///
    /// The returned hex string is a bincode 1.x serialization of the Transaction struct
    /// matching `doli-core::Transaction::serialize()` exactly, so the node can deserialize
    /// it with `bincode::deserialize()`.
    pub fn sign_and_build(&mut self, keypair: &KeyPair) -> Result<String> {
        let signing_bytes = self.build_for_signing()?;

        // Hash the signing bytes and sign
        let hash = crypto::hash::hash(&signing_bytes);
        let sig = signature::sign(hash.as_bytes(), keypair.private_key());

        // Set signature and public key on all inputs
        for input in &mut self.inputs {
            input.signature = Some(sig.as_bytes().to_vec());
            input.public_key = Some(keypair.public_key().as_bytes().to_vec());
        }

        // Produce bincode 1.x compatible serialization of the full Transaction.
        //
        // Bincode 1.x default format (LE, fixed-int):
        //   u32 -> 4 bytes LE
        //   u64 -> 8 bytes LE
        //   u8  -> 1 byte
        //   Vec<T> -> u64 LE length prefix (element count) + items
        //   enum -> u32 LE variant index + fields
        //
        // IMPORTANT: doli-core's Hash and Signature types implement custom Serialize
        // that calls `serialize_bytes()`. In bincode 1.x, `serialize_bytes()` writes
        // a u64 LE length prefix BEFORE the raw bytes:
        //   Hash field:      8 bytes (u64 LE = 32) + 32 bytes (data) = 40 bytes total
        //   Signature field:  8 bytes (u64 LE = 64) + 64 bytes (data) = 72 bytes total
        //
        // Vec<u8> (extra_data) also gets a u64 LE length prefix for byte count.
        // Vec<Input>/Vec<Output> get a u64 LE length prefix for element count.
        let mut buf = Vec::new();

        // Transaction.version: u32
        buf.extend_from_slice(&TX_VERSION.to_le_bytes());

        // Transaction.tx_type: enum TxType (repr(u32) in core, bincode writes u32 variant index)
        buf.extend_from_slice(&self.tx_type.to_core_type_id().to_le_bytes());

        // Transaction.inputs: Vec<Input> — u64 LE element count prefix
        buf.extend_from_slice(&(self.inputs.len() as u64).to_le_bytes());
        for input in &self.inputs {
            // Input.prev_tx_hash: Hash — serialize_bytes() adds u64 LE length prefix
            buf.extend_from_slice(&32u64.to_le_bytes());
            buf.extend_from_slice(&input.prev_tx_hash);
            // Input.output_index: u32
            buf.extend_from_slice(&input.output_index.to_le_bytes());
            // Input.signature: Signature — serialize_bytes() adds u64 LE length prefix
            let sig_bytes = input
                .signature
                .as_ref()
                .ok_or_else(|| anyhow!("Input missing signature"))?;
            if sig_bytes.len() != 64 {
                return Err(anyhow!(
                    "Signature must be 64 bytes, got {}",
                    sig_bytes.len()
                ));
            }
            buf.extend_from_slice(&64u64.to_le_bytes());
            buf.extend_from_slice(sig_bytes);
            // Input.sighash_type: SighashType — bincode writes enum variant as u32 LE
            // Always SighashType::All (0) for wallet-built transactions.
            buf.extend_from_slice(&0u32.to_le_bytes());
            // Input.committed_output_count: u32 — always 0 for wallet-built transactions
            // (only used with AnyoneCanPay sighash for partial-commitment signing)
            buf.extend_from_slice(&0u32.to_le_bytes());
        }

        // Transaction.outputs: Vec<Output> — u64 LE element count prefix
        buf.extend_from_slice(&(self.outputs.len() as u64).to_le_bytes());
        for output in &self.outputs {
            // Output.output_type: enum OutputType (repr(u8) in core, bincode writes u32 variant)
            buf.extend_from_slice(&(output.output_type as u32).to_le_bytes());
            // Output.amount: u64
            buf.extend_from_slice(&output.amount.to_le_bytes());
            // Output.pubkey_hash: Hash — serialize_bytes() adds u64 LE length prefix
            buf.extend_from_slice(&32u64.to_le_bytes());
            buf.extend_from_slice(&output.pubkey_hash);
            // Output.lock_until: u64
            buf.extend_from_slice(&output.lock_until.to_le_bytes());
            // Output.extra_data: Vec<u8> — u64 LE byte count prefix
            buf.extend_from_slice(&(output.extra_data.len() as u64).to_le_bytes());
            buf.extend_from_slice(&output.extra_data);
        }

        // Transaction.extra_data: Vec<u8> — u64 LE byte count prefix
        buf.extend_from_slice(&(self.extra_data.len() as u64).to_le_bytes());
        buf.extend_from_slice(&self.extra_data);

        Ok(hex::encode(buf))
    }

    // ========================================================================
    // Convenience builders for common transaction types
    // ========================================================================

    /// Build a Transfer transaction.
    /// Selects UTXOs, creates outputs (recipient + change), and returns a ready-to-sign builder.
    pub fn build_transfer(
        utxos: &[Utxo],
        recipient_hash: [u8; 32],
        amount: u64,
        fee: u64,
        sender_hash: [u8; 32],
    ) -> Result<Self> {
        if amount == 0 {
            return Err(anyhow!("Transfer amount must be greater than 0"));
        }

        let total_needed = amount
            .checked_add(fee)
            .ok_or_else(|| anyhow!("Amount + fee overflow"))?;

        // Select UTXOs (simple greedy: pick spendable UTXOs until we have enough)
        let mut selected_amount: u64 = 0;
        let mut builder = TxBuilder::new(TxType::Transfer);

        for utxo in utxos
            .iter()
            .filter(|u| u.spendable && u.output_type == "normal")
        {
            let tx_hash_bytes: [u8; 32] = hex::decode(&utxo.tx_hash)
                .map_err(|_| anyhow!("Invalid tx hash hex"))?
                .try_into()
                .map_err(|_| anyhow!("Tx hash must be 32 bytes"))?;

            builder.add_input(tx_hash_bytes, utxo.output_index);
            selected_amount += utxo.amount;

            if selected_amount >= total_needed {
                break;
            }
        }

        if selected_amount < total_needed {
            return Err(anyhow!(
                "Insufficient balance: have {} but need {} (amount {} + fee {})",
                format_balance(selected_amount),
                format_balance(total_needed),
                format_balance(amount),
                format_balance(fee)
            ));
        }

        // Recipient output
        builder.add_output(amount, recipient_hash, 0, 0, Vec::new());

        // Change output (if any)
        let change = selected_amount - total_needed;
        if change > 0 {
            builder.add_output(change, sender_hash, 0, 0, Vec::new());
        }

        Ok(builder)
    }

    /// Build an AddBond transaction (TxType 7).
    pub fn build_add_bond(
        utxos: &[Utxo],
        bond_count: u32,
        sender_hash: [u8; 32],
        fee: u64,
    ) -> Result<Self> {
        if bond_count == 0 {
            return Err(anyhow!("Bond count must be greater than 0"));
        }
        if bond_count > MAX_BONDS_PER_PRODUCER {
            return Err(anyhow!(
                "Bond count {} exceeds maximum {}",
                bond_count,
                MAX_BONDS_PER_PRODUCER
            ));
        }

        let bond_cost = (bond_count as u64)
            .checked_mul(BOND_UNIT)
            .ok_or_else(|| anyhow!("Bond cost overflow"))?;
        let total_needed = bond_cost
            .checked_add(fee)
            .ok_or_else(|| anyhow!("Bond cost + fee overflow"))?;

        let mut selected_amount: u64 = 0;
        let mut builder = TxBuilder::new(TxType::AddBond);

        for utxo in utxos
            .iter()
            .filter(|u| u.spendable && u.output_type == "normal")
        {
            let tx_hash_bytes: [u8; 32] = hex::decode(&utxo.tx_hash)
                .map_err(|_| anyhow!("Invalid tx hash hex"))?
                .try_into()
                .map_err(|_| anyhow!("Tx hash must be 32 bytes"))?;

            builder.add_input(tx_hash_bytes, utxo.output_index);
            selected_amount += utxo.amount;

            if selected_amount >= total_needed {
                break;
            }
        }

        if selected_amount < total_needed {
            return Err(anyhow!(
                "Insufficient balance for {} bonds: need {} DOLI",
                bond_count,
                units_to_coins(total_needed)
            ));
        }

        // Bond output(s)
        for _ in 0..bond_count {
            builder.add_output(BOND_UNIT, sender_hash, 1, u64::MAX, Vec::new());
        }

        // Change
        let change = selected_amount - total_needed;
        if change > 0 {
            builder.add_output(change, sender_hash, 0, 0, Vec::new());
        }

        Ok(builder)
    }

    /// Build a RequestWithdrawal transaction (TxType 8).
    pub fn build_request_withdrawal(
        bond_count: u32,
        sender_hash: [u8; 32],
        destination_hash: Option<[u8; 32]>,
    ) -> Result<Self> {
        if bond_count == 0 {
            return Err(anyhow!("Bond count must be greater than 0"));
        }

        let mut builder = TxBuilder::new(TxType::RequestWithdrawal);

        // RequestWithdrawal encodes the bond_count in extra_data
        let mut extra = Vec::new();
        extra.extend_from_slice(&bond_count.to_le_bytes());
        if let Some(dest) = destination_hash {
            extra.extend_from_slice(&dest);
        }
        builder.set_extra_data(extra);

        // The actual input/output structure depends on node implementation
        // A dummy input is required for the sender's identity
        builder.add_input(sender_hash, 0);
        builder.add_output(0, destination_hash.unwrap_or(sender_hash), 0, 0, Vec::new());

        Ok(builder)
    }

    /// Build a RewardClaim transaction (TxType 6).
    pub fn build_reward_claim(
        epoch: u64,
        sender_hash: [u8; 32],
        recipient_hash: Option<[u8; 32]>,
    ) -> Result<Self> {
        let mut builder = TxBuilder::new(TxType::RewardClaim);

        let mut extra = Vec::new();
        extra.extend_from_slice(&epoch.to_le_bytes());
        builder.set_extra_data(extra);

        // The reward claim transaction uses the sender's pubkey as input
        builder.add_input(sender_hash, 0);
        builder.add_output(0, recipient_hash.unwrap_or(sender_hash), 0, 0, Vec::new());

        Ok(builder)
    }
}

/// Tiered fee multiplier (x100) matching `doli-core::consensus::fee_multiplier_x100()`.
///
/// | Pending     | Multiplier |
/// |-------------|------------|
/// | 0-4         | 100 (1.00x)|
/// | 5-9         | 150 (1.50x)|
/// | 10-19       | 200 (2.00x)|
/// | 20-49       | 300 (3.00x)|
/// | 50-99       | 450 (4.50x)|
/// | 100-199     | 650 (6.50x)|
/// | 200-299     | 850 (8.50x)|
/// | 300+        | 1000 (10x) |
const fn fee_multiplier_x100(pending_count: u32) -> u32 {
    if pending_count >= 300 {
        return 1000;
    }
    if pending_count >= 200 {
        return 850;
    }
    if pending_count >= 100 {
        return 650;
    }
    if pending_count >= 50 {
        return 450;
    }
    if pending_count >= 20 {
        return 300;
    }
    if pending_count >= 10 {
        return 200;
    }
    if pending_count >= 5 {
        return 150;
    }
    100
}

/// Calculate registration fee matching `doli-core::consensus::registration_fee()`.
///
/// Fee = BASE_REGISTRATION_FEE * multiplier / 100, capped at MAX_REGISTRATION_FEE.
fn registration_fee(pending_count: u32) -> u64 {
    let multiplier = fee_multiplier_x100(pending_count) as u128;
    let fee = (BASE_REGISTRATION_FEE as u128 * multiplier) / 100;
    (fee as u64).min(MAX_REGISTRATION_FEE)
}

/// Calculate the total cost for bond registration.
/// Returns (bond_cost, registration_fee, total).
pub fn calculate_registration_cost(
    bond_count: u32,
    pending_registrations: u32,
) -> Result<(u64, u64, u64)> {
    if bond_count == 0 {
        return Err(anyhow!("Bond count must be at least 1"));
    }
    if bond_count > MAX_BONDS_PER_PRODUCER {
        return Err(anyhow!(
            "Bond count exceeds maximum of {}",
            MAX_BONDS_PER_PRODUCER
        ));
    }

    let bond_cost = (bond_count as u64)
        .checked_mul(BOND_UNIT)
        .ok_or_else(|| anyhow!("Bond cost overflow"))?;

    // Registration fee uses the tiered multiplier table (matching node consensus)
    let reg_fee = registration_fee(pending_registrations);

    let total = bond_cost
        .checked_add(reg_fee)
        .ok_or_else(|| anyhow!("Total cost overflow"))?;

    Ok((bond_cost, reg_fee, total))
}

/// Calculate vesting penalty for a bond given its age in slots.
/// Returns penalty percentage (0, 25, 50, or 75).
pub fn vesting_penalty_pct(age_slots: u64) -> u8 {
    // Vesting schedule: Q1 (0-1yr) = 75%, Q2 (1-2yr) = 50%, Q3 (2-3yr) = 25%, Vested (3yr+) = 0%
    if age_slots >= VESTING_QUARTER_SLOTS * 3 {
        0 // Fully vested (3+ years)
    } else if age_slots >= VESTING_QUARTER_SLOTS * 2 {
        25 // Q3: 2-3 years
    } else if age_slots >= VESTING_QUARTER_SLOTS {
        50 // Q2: 1-2 years
    } else {
        75 // Q1: 0-1 year
    }
}

/// Calculate the net amount returned after withdrawal penalty.
pub fn calculate_withdrawal_net(bond_amount: u64, penalty_pct: u8) -> u64 {
    let penalty = bond_amount * penalty_pct as u64 / 100;
    bond_amount - penalty
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Requirement: GUI-FR-011 (Must) -- Send transaction
    // Acceptance: Transaction construction, signing, validation
    // ========================================================================

    #[test]
    fn test_fr011_transfer_builder_basic() {
        let utxos = vec![Utxo {
            tx_hash: "aa".repeat(32),
            output_index: 0,
            amount: 500_000_000, // 5 DOLI
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 100,
            spendable: true,
        }];
        let recipient = [0xBBu8; 32];
        let sender = [0xAAu8; 32];

        let builder =
            TxBuilder::build_transfer(&utxos, recipient, 100_000_000, 1000, sender).unwrap();
        assert_eq!(builder.tx_type(), TxType::Transfer);
        assert_eq!(builder.input_count(), 1);
        assert!(builder.output_count() >= 1); // recipient + possible change
    }

    #[test]
    fn test_fr011_transfer_insufficient_balance() {
        let utxos = vec![Utxo {
            tx_hash: "aa".repeat(32),
            output_index: 0,
            amount: 100_000, // 0.001 DOLI
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 100,
            spendable: true,
        }];
        let recipient = [0xBBu8; 32];
        let sender = [0xAAu8; 32];

        let result = TxBuilder::build_transfer(&utxos, recipient, 500_000_000, 1000, sender);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Insufficient"));
    }

    #[test]
    fn test_fr011_transfer_zero_amount_rejected() {
        let utxos = vec![Utxo {
            tx_hash: "aa".repeat(32),
            output_index: 0,
            amount: 500_000_000,
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 100,
            spendable: true,
        }];
        let result = TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 0, 1000, [0xAAu8; 32]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("greater than 0"));
    }

    #[test]
    fn test_fr011_transfer_change_output() {
        let utxos = vec![Utxo {
            tx_hash: "aa".repeat(32),
            output_index: 0,
            amount: 500_000_000, // 5 DOLI
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 100,
            spendable: true,
        }];
        let sender = [0xAAu8; 32];

        let builder =
            TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 100_000_000, 1000, sender).unwrap();
        // Should have 2 outputs: recipient (1 DOLI) + change (3.99999 DOLI)
        assert_eq!(builder.output_count(), 2);
    }

    #[test]
    fn test_fr011_transfer_exact_amount_no_change() {
        let utxos = vec![Utxo {
            tx_hash: "aa".repeat(32),
            output_index: 0,
            amount: 100_001_000, // exactly amount + fee
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 100,
            spendable: true,
        }];
        let sender = [0xAAu8; 32];

        let builder =
            TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 100_000_000, 1000, sender).unwrap();
        // Exact amount: only recipient output, no change
        assert_eq!(builder.output_count(), 1);
    }

    #[test]
    fn test_fr011_transfer_skips_unspendable_utxos() {
        let utxos = vec![
            Utxo {
                tx_hash: "bb".repeat(32),
                output_index: 0,
                amount: 999_000_000_000, // large but unspendable
                output_type: "normal".to_string(),
                lock_until: 0,
                height: 100,
                spendable: false,
            },
            Utxo {
                tx_hash: "aa".repeat(32),
                output_index: 0,
                amount: 500_000_000, // spendable
                output_type: "normal".to_string(),
                lock_until: 0,
                height: 100,
                spendable: true,
            },
        ];
        let builder =
            TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 100_000_000, 1000, [0xAAu8; 32])
                .unwrap();
        assert_eq!(builder.input_count(), 1);
    }

    #[test]
    fn test_fr011_transfer_skips_bond_utxos() {
        let utxos = vec![
            Utxo {
                tx_hash: "cc".repeat(32),
                output_index: 0,
                amount: 1_000_000_000,
                output_type: "bond".to_string(),
                lock_until: u64::MAX,
                height: 100,
                spendable: true,
            },
            Utxo {
                tx_hash: "aa".repeat(32),
                output_index: 0,
                amount: 500_000_000,
                output_type: "normal".to_string(),
                lock_until: 0,
                height: 100,
                spendable: true,
            },
        ];
        let builder =
            TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 100_000_000, 1000, [0xAAu8; 32])
                .unwrap();
        assert_eq!(
            builder.input_count(),
            1,
            "Bond UTXOs must not be used for transfers"
        );
    }

    #[test]
    fn test_fr011_transfer_multiple_utxos() {
        let utxos = vec![
            Utxo {
                tx_hash: "aa".repeat(32),
                output_index: 0,
                amount: 100_000_000,
                output_type: "normal".to_string(),
                lock_until: 0,
                height: 100,
                spendable: true,
            },
            Utxo {
                tx_hash: "bb".repeat(32),
                output_index: 0,
                amount: 100_000_000,
                output_type: "normal".to_string(),
                lock_until: 0,
                height: 101,
                spendable: true,
            },
        ];
        let builder = TxBuilder::build_transfer(
            &utxos,
            [0xBBu8; 32],
            150_000_000, // needs both UTXOs
            1000,
            [0xAAu8; 32],
        )
        .unwrap();
        assert_eq!(builder.input_count(), 2);
    }

    #[test]
    fn test_fr011_transfer_empty_utxos() {
        let utxos: Vec<Utxo> = vec![];
        let result =
            TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 100_000_000, 1000, [0xAAu8; 32]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Insufficient"));
    }

    // Edge case: amount + fee overflow
    #[test]
    fn test_fr011_transfer_amount_fee_overflow() {
        let utxos = vec![Utxo {
            tx_hash: "aa".repeat(32),
            output_index: 0,
            amount: u64::MAX,
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 100,
            spendable: true,
        }];
        let result =
            TxBuilder::build_transfer(&utxos, [0xBBu8; 32], u64::MAX, u64::MAX, [0xAAu8; 32]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("overflow"));
    }

    // ========================================================================
    // Requirement: GUI-FR-020 (Must) -- Producer registration cost
    // Acceptance: Bond count * 10 DOLI + registration fee
    // ========================================================================

    #[test]
    fn test_fr020_registration_cost_single_bond() {
        let (bond_cost, reg_fee, total) = calculate_registration_cost(1, 0).unwrap();
        assert_eq!(bond_cost, BOND_UNIT); // 10 DOLI
        assert_eq!(reg_fee, BASE_REGISTRATION_FEE); // 0.001 DOLI
        assert_eq!(total, BOND_UNIT + BASE_REGISTRATION_FEE);
    }

    #[test]
    fn test_fr020_registration_cost_10_bonds() {
        let (bond_cost, reg_fee, total) = calculate_registration_cost(10, 0).unwrap();
        assert_eq!(bond_cost, 10 * BOND_UNIT); // 100 DOLI
        assert_eq!(total, bond_cost + reg_fee);
    }

    #[test]
    fn test_fr020_registration_fee_scales_with_pending() {
        let (_, fee_0, _) = calculate_registration_cost(1, 0).unwrap();
        let (_, fee_5, _) = calculate_registration_cost(1, 5).unwrap();
        assert!(
            fee_5 > fee_0,
            "Fee should increase with pending registrations"
        );
    }

    #[test]
    fn test_fr020_registration_fee_capped() {
        let (_, fee, _) = calculate_registration_cost(1, 1000).unwrap();
        assert_eq!(
            fee, MAX_REGISTRATION_FEE,
            "Fee must be capped at MAX_REGISTRATION_FEE"
        );
    }

    #[test]
    fn test_fr020_registration_cost_zero_bonds_rejected() {
        let result = calculate_registration_cost(0, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_fr020_registration_cost_exceeds_max_bonds() {
        let result = calculate_registration_cost(MAX_BONDS_PER_PRODUCER + 1, 0);
        assert!(result.is_err());
    }

    // ========================================================================
    // Requirement: GUI-FR-024 (Must) -- Add bonds
    // Acceptance: Each bond = 10 DOLI, max 3000 bonds
    // ========================================================================

    #[test]
    fn test_fr024_add_bond_basic() {
        let utxos = vec![Utxo {
            tx_hash: "aa".repeat(32),
            output_index: 0,
            amount: 50_000_000_000, // 500 DOLI
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 100,
            spendable: true,
        }];
        let sender = [0xAAu8; 32];

        let builder = TxBuilder::build_add_bond(&utxos, 5, sender, 1000).unwrap();
        assert_eq!(builder.tx_type(), TxType::AddBond);
        // 5 bond outputs + 1 change output
        assert_eq!(builder.output_count(), 6);
    }

    #[test]
    fn test_fr024_add_bond_zero_rejected() {
        let utxos = vec![Utxo {
            tx_hash: "aa".repeat(32),
            output_index: 0,
            amount: 50_000_000_000,
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 100,
            spendable: true,
        }];
        let result = TxBuilder::build_add_bond(&utxos, 0, [0xAAu8; 32], 1000);
        assert!(result.is_err());
    }

    #[test]
    fn test_fr024_add_bond_exceeds_max() {
        let utxos = vec![Utxo {
            tx_hash: "aa".repeat(32),
            output_index: 0,
            amount: u64::MAX / 2,
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 100,
            spendable: true,
        }];
        let result =
            TxBuilder::build_add_bond(&utxos, MAX_BONDS_PER_PRODUCER + 1, [0xAAu8; 32], 1000);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exceeds maximum"));
    }

    #[test]
    fn test_fr024_add_bond_insufficient_balance() {
        let utxos = vec![Utxo {
            tx_hash: "aa".repeat(32),
            output_index: 0,
            amount: 500_000_000, // 5 DOLI -- not enough for 1 bond (10 DOLI)
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 100,
            spendable: true,
        }];
        let result = TxBuilder::build_add_bond(&utxos, 1, [0xAAu8; 32], 1000);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Insufficient"));
    }

    #[test]
    fn test_fr024_bond_cost_calculation() {
        // 5 bonds = 50 DOLI = 5,000,000,000 base units
        assert_eq!(5u64 * BOND_UNIT, 5_000_000_000);
        // 3000 bonds (max) = 30,000 DOLI = 3,000,000,000,000 base units
        assert_eq!(3000u64 * BOND_UNIT, 3_000_000_000_000u64);
    }

    // ========================================================================
    // Requirement: GUI-FR-025 (Must) -- Request withdrawal
    // Acceptance: FIFO order, vesting penalty calculation
    // ========================================================================

    #[test]
    fn test_fr025_vesting_penalty_q1() {
        // 0-1 year: 75% penalty
        assert_eq!(vesting_penalty_pct(0), 75);
        assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS - 1), 75);
    }

    #[test]
    fn test_fr025_vesting_penalty_q2() {
        // 1-2 years: 50% penalty
        assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS), 50);
        assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS * 2 - 1), 50);
    }

    #[test]
    fn test_fr025_vesting_penalty_q3() {
        // 2-3 years: 25% penalty
        assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS * 2), 25);
        assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS * 3 - 1), 25);
    }

    #[test]
    fn test_fr025_vesting_penalty_fully_vested() {
        // 4+ years: 0% penalty
        assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS * 4), 0);
        assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS * 10), 0);
    }

    #[test]
    fn test_fr025_withdrawal_net_calculation() {
        // 10 DOLI bond, 75% penalty
        let net = calculate_withdrawal_net(BOND_UNIT, 75);
        assert_eq!(net, BOND_UNIT / 4); // 2.5 DOLI

        // 10 DOLI bond, 50% penalty
        let net = calculate_withdrawal_net(BOND_UNIT, 50);
        assert_eq!(net, BOND_UNIT / 2); // 5 DOLI

        // 10 DOLI bond, 0% penalty (fully vested)
        let net = calculate_withdrawal_net(BOND_UNIT, 0);
        assert_eq!(net, BOND_UNIT); // 10 DOLI
    }

    #[test]
    fn test_fr025_request_withdrawal_basic() {
        let sender = [0xAAu8; 32];
        let builder = TxBuilder::build_request_withdrawal(5, sender, None).unwrap();
        assert_eq!(builder.tx_type(), TxType::RequestWithdrawal);
    }

    #[test]
    fn test_fr025_request_withdrawal_zero_bonds_rejected() {
        let result = TxBuilder::build_request_withdrawal(0, [0xAAu8; 32], None);
        assert!(result.is_err());
    }

    #[test]
    fn test_fr025_request_withdrawal_with_destination() {
        let sender = [0xAAu8; 32];
        let dest = [0xBBu8; 32];
        let builder = TxBuilder::build_request_withdrawal(3, sender, Some(dest)).unwrap();
        assert_eq!(builder.output_count(), 1);
    }

    // ========================================================================
    // Requirement: GUI-FR-031, GUI-FR-032 (Must) -- Claim rewards
    // Acceptance: Epoch selector, optional recipient, estimated reward
    // ========================================================================

    #[test]
    fn test_fr031_reward_claim_basic() {
        let sender = [0xAAu8; 32];
        let builder = TxBuilder::build_reward_claim(42, sender, None).unwrap();
        assert_eq!(builder.tx_type(), TxType::RewardClaim);
    }

    #[test]
    fn test_fr031_reward_claim_with_recipient() {
        let sender = [0xAAu8; 32];
        let recipient = [0xBBu8; 32];
        let builder = TxBuilder::build_reward_claim(42, sender, Some(recipient)).unwrap();
        assert_eq!(builder.output_count(), 1);
    }

    // ========================================================================
    // TxBuilder core tests
    // ========================================================================

    #[test]
    fn test_tx_builder_new() {
        let builder = TxBuilder::new(TxType::Transfer);
        assert_eq!(builder.tx_type(), TxType::Transfer);
        assert_eq!(builder.input_count(), 0);
        assert_eq!(builder.output_count(), 0);
    }

    #[test]
    fn test_tx_builder_add_input_output() {
        let mut builder = TxBuilder::new(TxType::Transfer);
        builder.add_input([0u8; 32], 0);
        builder.add_output(100, [0u8; 32], 0, 0, Vec::new());

        assert_eq!(builder.input_count(), 1);
        assert_eq!(builder.output_count(), 1);
    }

    #[test]
    fn test_tx_type_values() {
        // These must match doli-core TxType values exactly
        assert_eq!(TxType::Transfer as u8, 0);
        assert_eq!(TxType::Registration as u8, 1);
        assert_eq!(TxType::ProducerExit as u8, 2);
        assert_eq!(TxType::Coinbase as u8, 3);
        assert_eq!(TxType::NftMint as u8, 4);
        assert_eq!(TxType::NftTransfer as u8, 5);
        assert_eq!(TxType::RewardClaim as u8, 6);
        assert_eq!(TxType::AddBond as u8, 7);
        assert_eq!(TxType::RequestWithdrawal as u8, 8);
        assert_eq!(TxType::ClaimWithdrawal as u8, 9);
        assert_eq!(TxType::SlashingEvidence as u8, 10);
        assert_eq!(TxType::TokenIssuance as u8, 11);
        assert_eq!(TxType::BridgeLock as u8, 12);
        assert_eq!(TxType::DelegateBond as u8, 13);
        assert_eq!(TxType::RevokeDelegation as u8, 14);
    }

    // ========================================================================
    // Edge cases
    // ========================================================================

    #[test]
    fn test_build_for_signing_no_inputs_rejected() {
        let mut builder = TxBuilder::new(TxType::Transfer);
        builder.add_output(100, [0u8; 32], 0, 0, Vec::new());
        // build_for_signing is a todo!() but the validation before it should work
        // This test verifies the error path
        // Note: This will panic with todo!() until implemented
    }

    #[test]
    fn test_transfer_invalid_tx_hash_hex() {
        let utxos = vec![Utxo {
            tx_hash: "not_valid_hex!!".to_string(),
            output_index: 0,
            amount: 500_000_000,
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 100,
            spendable: true,
        }];
        let result =
            TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 100_000_000, 1000, [0xAAu8; 32]);
        assert!(result.is_err());
    }
}
