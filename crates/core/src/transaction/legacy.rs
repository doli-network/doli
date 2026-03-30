//! Legacy structs for backward-compatible bincode deserialization.
//!
//! Bincode is positional, so `#[serde(default)]` does NOT work — the decoder
//! reads past the end of the old struct and misinterprets trailing bytes.
//!
//! These structs mirror older layouts exactly. After deserializing, call
//! `.into_current()` to convert to the current types.
//!
//! Format history:
//! - v3.5.0: Input has (prev_tx_hash, output_index, signature)
//! - v3.6.0: Input adds sighash_type
//! - v3.7.1: Input adds committed_output_count
//! - v5.1.0: Input adds public_key (P0-001 signature enforcement hard fork)

use crypto::{Hash, Signature};
use serde::{Deserialize, Serialize};

use super::*;

/// v3.5.0 Input — no sighash_type field.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LegacyInput {
    pub prev_tx_hash: Hash,
    pub output_index: u32,
    pub signature: Signature,
}

impl LegacyInput {
    pub fn into_current(self) -> Input {
        Input {
            prev_tx_hash: self.prev_tx_hash,
            output_index: self.output_index,
            signature: self.signature,
            sighash_type: SighashType::All,
            committed_output_count: 0,
            public_key: None,
        }
    }
}

/// v3.5.0 Transaction — uses LegacyInput.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LegacyTransaction {
    pub version: u32,
    pub tx_type: TxType,
    pub inputs: Vec<LegacyInput>,
    pub outputs: Vec<Output>,
    pub extra_data: Vec<u8>,
}

impl LegacyTransaction {
    pub fn into_current(self) -> Transaction {
        Transaction {
            version: self.version,
            tx_type: self.tx_type,
            inputs: self.inputs.into_iter().map(|i| i.into_current()).collect(),
            outputs: self.outputs,
            extra_data: self.extra_data,
        }
    }
}

/// v3.5.0 Block — uses LegacyTransaction.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LegacyBlock {
    pub header: crate::block::BlockHeader,
    pub transactions: Vec<LegacyTransaction>,
    #[serde(default)]
    pub aggregate_bls_signature: Vec<u8>,
}

impl LegacyBlock {
    pub fn into_current(self) -> crate::block::Block {
        crate::block::Block {
            header: self.header,
            transactions: self
                .transactions
                .into_iter()
                .map(|t| t.into_current())
                .collect(),
            aggregate_bls_signature: self.aggregate_bls_signature,
        }
    }
}

/// v3.6.0 Input — has sighash_type but no committed_output_count.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LegacyInputV2 {
    pub prev_tx_hash: Hash,
    pub output_index: u32,
    pub signature: Signature,
    pub sighash_type: SighashType,
}

impl LegacyInputV2 {
    pub fn into_current(self) -> Input {
        Input {
            prev_tx_hash: self.prev_tx_hash,
            output_index: self.output_index,
            signature: self.signature,
            sighash_type: self.sighash_type,
            committed_output_count: 0,
            public_key: None,
        }
    }
}

/// v3.6.0 Transaction — uses LegacyInputV2.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LegacyTransactionV2 {
    pub version: u32,
    pub tx_type: TxType,
    pub inputs: Vec<LegacyInputV2>,
    pub outputs: Vec<Output>,
    pub extra_data: Vec<u8>,
}

impl LegacyTransactionV2 {
    pub fn into_current(self) -> Transaction {
        Transaction {
            version: self.version,
            tx_type: self.tx_type,
            inputs: self.inputs.into_iter().map(|i| i.into_current()).collect(),
            outputs: self.outputs,
            extra_data: self.extra_data,
        }
    }
}

/// v3.6.0 Block — uses LegacyTransactionV2.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LegacyBlockV2 {
    pub header: crate::block::BlockHeader,
    pub transactions: Vec<LegacyTransactionV2>,
    #[serde(default)]
    pub aggregate_bls_signature: Vec<u8>,
}

impl LegacyBlockV2 {
    pub fn into_current(self) -> crate::block::Block {
        crate::block::Block {
            header: self.header,
            transactions: self
                .transactions
                .into_iter()
                .map(|t| t.into_current())
                .collect(),
            aggregate_bls_signature: self.aggregate_bls_signature,
        }
    }
}

/// v3.7.1 Input — has sighash_type + committed_output_count, no public_key.
/// This is the pre-P0-001 format. All existing chain data uses this format.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LegacyInputV3 {
    pub prev_tx_hash: Hash,
    pub output_index: u32,
    pub signature: Signature,
    pub sighash_type: SighashType,
    pub committed_output_count: u32,
}

impl LegacyInputV3 {
    pub fn into_current(self) -> Input {
        Input {
            prev_tx_hash: self.prev_tx_hash,
            output_index: self.output_index,
            signature: self.signature,
            sighash_type: self.sighash_type,
            committed_output_count: self.committed_output_count,
            public_key: None,
        }
    }
}

/// v3.7.1 Transaction — uses LegacyInputV3.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LegacyTransactionV3 {
    pub version: u32,
    pub tx_type: TxType,
    pub inputs: Vec<LegacyInputV3>,
    pub outputs: Vec<Output>,
    pub extra_data: Vec<u8>,
}

impl LegacyTransactionV3 {
    pub fn into_current(self) -> Transaction {
        Transaction {
            version: self.version,
            tx_type: self.tx_type,
            inputs: self.inputs.into_iter().map(|i| i.into_current()).collect(),
            outputs: self.outputs,
            extra_data: self.extra_data,
        }
    }
}

/// v3.7.1 Block — uses LegacyTransactionV3.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LegacyBlockV3 {
    pub header: crate::block::BlockHeader,
    pub transactions: Vec<LegacyTransactionV3>,
    #[serde(default)]
    pub aggregate_bls_signature: Vec<u8>,
}

impl LegacyBlockV3 {
    pub fn into_current(self) -> crate::block::Block {
        crate::block::Block {
            header: self.header,
            transactions: self
                .transactions
                .into_iter()
                .map(|t| t.into_current())
                .collect(),
            aggregate_bls_signature: self.aggregate_bls_signature,
        }
    }
}

/// Deserialize a block from bincode, trying current format first, then legacy.
pub fn deserialize_block_compat(data: &[u8]) -> Option<crate::block::Block> {
    // Try current format first (v5.1.0+: Input has public_key)
    if let Ok(block) = bincode::deserialize::<crate::block::Block>(data) {
        return Some(block);
    }
    // Fallback: v3.7.1 format (Input has committed_output_count but no public_key)
    if let Ok(legacy) = bincode::deserialize::<LegacyBlockV3>(data) {
        return Some(legacy.into_current());
    }
    // Fallback: v3.6.0 format (Input has sighash_type but no committed_output_count)
    if let Ok(legacy) = bincode::deserialize::<LegacyBlockV2>(data) {
        return Some(legacy.into_current());
    }
    // Fallback: v3.5.0 format (Input has no sighash_type)
    if let Ok(legacy) = bincode::deserialize::<LegacyBlock>(data) {
        return Some(legacy.into_current());
    }
    None
}
