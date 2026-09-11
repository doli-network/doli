use crypto::{Hash, Signature};
use serde::{Deserialize, Serialize};

use super::types::SighashType;

/// Transaction input (reference to a previous output)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    /// Hash of the transaction containing the output
    pub prev_tx_hash: Hash,
    /// Index of the output in that transaction
    pub output_index: u32,
    /// Signature proving ownership
    pub signature: Signature,
    /// Sighash type: what this input's signature covers.
    /// Default: All (backwards-compatible with v1 transactions).
    #[serde(default)]
    pub sighash_type: SighashType,
    /// Number of outputs this input's signature commits to (AnyoneCanPay only).
    /// 0 = all outputs (backward compat with pre-v3.7.1 transactions).
    /// N > 0 = sighash covers only the first N outputs, allowing the buyer
    /// to append additional outputs (e.g. change) without invalidating
    /// the seller's signature.
    #[serde(default)]
    pub committed_output_count: u32,
    /// Public key of the spender (P2PKH: reveals pubkey at spend time).
    /// Pre-fork transactions have `None` (signature verification skipped).
    /// Post-fork transactions MUST have `Some(pk)` for signature enforcement.
    ///
    /// Part of the bincode wire format since v5.1.0 (P0-001 hard fork).
    /// Old chain data (pre-fork) is deserialized via `LegacyInputV3` / `deserialize_block_compat`
    /// which converts to `Input` with `public_key: None`.
    pub public_key: Option<crypto::PublicKey>,
}

impl Input {
    /// Create a new input (default sighash: All, no pubkey)
    pub fn new(prev_tx_hash: Hash, output_index: u32) -> Self {
        Self {
            prev_tx_hash,
            output_index,
            signature: Signature::default(),
            sighash_type: SighashType::All,
            committed_output_count: 0,
            public_key: None,
        }
    }

    /// Create a new input with AnyoneCanPay sighash type.
    /// The signature covers only this input + all outputs (not other inputs).
    /// Used for NFT marketplace: seller signs their NFT input, buyer adds payment inputs later.
    pub fn new_anyone_can_pay(prev_tx_hash: Hash, output_index: u32) -> Self {
        Self {
            prev_tx_hash,
            output_index,
            signature: Signature::default(),
            sighash_type: SighashType::AnyoneCanPay,
            committed_output_count: 0,
            public_key: None,
        }
    }

    /// Create an AnyoneCanPay input that commits to only the first N outputs.
    /// The buyer can append additional outputs (e.g. change) without
    /// invalidating the seller's signature.
    pub fn new_anyone_can_pay_partial(
        prev_tx_hash: Hash,
        output_index: u32,
        committed_output_count: u32,
    ) -> Self {
        Self {
            prev_tx_hash,
            output_index,
            signature: Signature::default(),
            sighash_type: SighashType::AnyoneCanPay,
            committed_output_count,
            public_key: None,
        }
    }

    /// Builder method: set the spender's public key for signature verification.
    pub fn with_public_key(mut self, pk: crypto::PublicKey) -> Self {
        self.public_key = Some(pk);
        self
    }

    /// Create an outpoint identifier
    pub fn outpoint(&self) -> (Hash, u32) {
        (self.prev_tx_hash, self.output_index)
    }

    /// Serialize for signing
    pub fn serialize_for_signing(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(self.prev_tx_hash.as_bytes());
        bytes.extend_from_slice(&self.output_index.to_le_bytes());
        bytes
    }
}
