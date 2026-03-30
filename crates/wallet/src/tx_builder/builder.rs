use anyhow::{anyhow, Result};
use crypto::{signature, KeyPair};

use super::types::{TxInput, TxOutput, TxType, TX_VERSION};
use crate::types::*;

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
            // Input.public_key: Option<PublicKey> — bincode: 0u8=None, 1u8+bytes=Some
            // Wallet always provides the public key for signature verification (P0-001).
            let pk_bytes = input
                .public_key
                .as_ref()
                .ok_or_else(|| anyhow!("Input missing public_key"))?;
            if pk_bytes.len() != 32 {
                return Err(anyhow!(
                    "Public key must be 32 bytes, got {}",
                    pk_bytes.len()
                ));
            }
            buf.push(1u8); // Option::Some discriminant
            buf.extend_from_slice(&32u64.to_le_bytes()); // PublicKey serialize_bytes length prefix
            buf.extend_from_slice(pk_bytes);
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
