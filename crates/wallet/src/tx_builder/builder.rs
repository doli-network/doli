use std::str::FromStr;

use anyhow::{anyhow, Result};
use crypto::{signature, Hash, KeyPair};
use doli_core::transaction::OutputType;
use doli_core::validation::bond_owner;
use doli_core::Network;
use doli_producer_tx::producer_tx::{
    build_add_bond_tx, chain_genesis_hash, finalize_producer_tx, AddBondSpec,
};
use doli_producer_tx::unbonding_tx::{
    build_withdrawal_tx, builder_params, FeeUtxo, WithdrawalSpec,
};

use super::types::{TxInput, TxOutput, TxType, TX_VERSION};
use super::{ProducerSigner, WithdrawalRequest};
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
        self.build_for_signing()?;

        // Input i signs core's per-input message, the one the node verifies.
        let unsigned = self.core_unsigned()?;
        for (i, input) in self.inputs.iter_mut().enumerate() {
            let msg = unsigned.signing_message_for_input(i);
            let sig = signature::sign_hash(&msg, keypair.private_key());
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
            // Input.public_key: Option<PublicKey> — part of wire format since v5.1.0 (P0-001).
            // bincode 1.x encodes Option as: 0u8 (None) or 1u8 + value (Some).
            // PublicKey serializes via serialize_bytes(): u64 LE length prefix + 32 bytes.
            if let Some(ref pk_bytes) = input.public_key {
                buf.push(1u8); // Some discriminant
                buf.extend_from_slice(&32u64.to_le_bytes()); // length prefix
                buf.extend_from_slice(pk_bytes);
            } else {
                buf.push(0u8); // None discriminant
            }
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

    /// This builder as an unsigned core transaction, for core's signing messages.
    fn core_unsigned(&self) -> Result<doli_core::Transaction> {
        let tx_type = doli_core::transaction::TxType::from_u32(self.tx_type.to_core_type_id())
            .ok_or_else(|| anyhow!("Unknown transaction type {:?}", self.tx_type))?;
        let mut outputs = Vec::with_capacity(self.outputs.len());
        for o in &self.outputs {
            outputs.push(doli_core::transaction::Output {
                output_type: OutputType::from_u8(o.output_type)
                    .ok_or_else(|| anyhow!("Unknown output type {}", o.output_type))?,
                amount: o.amount,
                pubkey_hash: Hash::from_bytes(o.pubkey_hash),
                lock_until: o.lock_until,
                extra_data: o.extra_data.clone(),
            });
        }
        Ok(doli_core::Transaction {
            version: TX_VERSION,
            tx_type,
            inputs: self
                .inputs
                .iter()
                .map(|i| doli_core::Input::new(Hash::from_bytes(i.prev_tx_hash), i.output_index))
                .collect(),
            outputs,
            extra_data: self.extra_data.clone(),
        })
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

    /// Build and sign an AddBond: greedy over spendable normal coins, change to
    /// the signer's own address, then the shared builder and its signing step.
    /// Returns the signed tx as hex.
    pub fn build_add_bond(
        utxos: &[Utxo],
        bond_count: u32,
        bond_unit: u64,
        lock_until: u64,
        fee: u64,
        signer: &ProducerSigner<'_>,
    ) -> Result<String> {
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
        let genesis = chain_genesis_hash(signer.genesis_hash).map_err(|e| anyhow!(e))?;
        let total_needed = u64::from(bond_count)
            .checked_mul(bond_unit)
            .and_then(|cost| cost.checked_add(fee))
            .ok_or_else(|| anyhow!("Bond cost + fee overflow"))?;

        let mut inputs = Vec::new();
        let mut selected_amount: u64 = 0;
        for utxo in utxos
            .iter()
            .filter(|u| u.spendable && u.output_type == "normal")
        {
            if selected_amount >= total_needed {
                break;
            }
            inputs.push(outpoint(utxo)?);
            selected_amount = selected_amount.saturating_add(utxo.amount);
        }
        if selected_amount < total_needed {
            return Err(anyhow!(
                "Insufficient balance for {} bonds: need {} DOLI",
                bond_count,
                units_to_coins(total_needed)
            ));
        }

        let producer = *signer.keypair.public_key();
        let mut tx = build_add_bond_tx(&AddBondSpec {
            inputs,
            producer,
            count: bond_count,
            bond_unit,
            lock_until,
            change: selected_amount - total_needed,
            change_to: bond_owner(&producer),
        });
        finalize_producer_tx(&mut tx, signer.keypair, &genesis);
        Ok(hex::encode(tx.serialize()))
    }

    /// Build and sign a RequestWithdrawal through the shared builder, with the
    /// node's params for `req.network`. Returns the signed tx as hex.
    pub fn build_request_withdrawal(
        req: &WithdrawalRequest<'_>,
        signer: &ProducerSigner<'_>,
    ) -> Result<String> {
        if req.bond_count == 0 {
            return Err(anyhow!("Bond count must be greater than 0"));
        }
        if req.bond_utxos.len() < req.bond_count as usize {
            return Err(anyhow!(
                "Not enough Bond UTXOs: need {}, own {}",
                req.bond_count,
                req.bond_utxos.len()
            ));
        }
        let genesis = chain_genesis_hash(signer.genesis_hash).map_err(|e| anyhow!(e))?;
        let network = Network::from_str(req.network)
            .map_err(|_| anyhow!("Unknown network reported by the node: {}", req.network))?;

        let producer = *signer.keypair.public_key();
        let bond_inputs = req.bond_utxos.iter().map(outpoint).collect::<Result<_>>()?;
        let mut fee_utxos = Vec::new();
        for utxo in req
            .fee_utxos
            .iter()
            .filter(|u| u.spendable && u.output_type == "normal")
        {
            let (tx_hash, output_index) = outpoint(utxo)?;
            fee_utxos.push(FeeUtxo {
                tx_hash,
                output_index,
                amount: utxo.amount,
            });
        }
        let spec = WithdrawalSpec {
            producer,
            bond_count: req.bond_count,
            bond_inputs,
            total_net: req.total_net,
            destination: req
                .destination
                .map_or_else(|| bond_owner(&producer), Hash::from_bytes),
            fee_utxos,
            fee: req.fee,
        };
        let mut tx = build_withdrawal_tx(builder_params(network), req.best_height, &spec)
            .map_err(|e| anyhow!(e))?;
        finalize_producer_tx(&mut tx, signer.keypair, &genesis);
        Ok(hex::encode(tx.serialize()))
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

fn outpoint(utxo: &Utxo) -> Result<(Hash, u32)> {
    let bytes: [u8; 32] = hex::decode(&utxo.tx_hash)
        .map_err(|_| anyhow!("Invalid tx hash hex"))?
        .try_into()
        .map_err(|_| anyhow!("Tx hash must be 32 bytes"))?;
    Ok((Hash::from_bytes(bytes), utxo.output_index))
}
