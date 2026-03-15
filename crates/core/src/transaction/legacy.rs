//! Legacy (v3.5.0) structs for backward-compatible bincode deserialization.
//!
//! v3.5.0 `Input` had no `sighash_type` field. Bincode is positional, so
//! `#[serde(default)]` does NOT work — the decoder reads past the end of the
//! old struct and misinterprets bytes from the next field as the enum discriminant.
//!
//! These structs mirror the v3.5.0 layout exactly. After deserializing, call
//! `.into_current()` to convert to the current types with `SighashType::All`.

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

/// Deserialize a block from bincode, trying current format first, then legacy.
pub fn deserialize_block_compat(data: &[u8]) -> Option<crate::block::Block> {
    // Try current format first (v3.7.1+: Input has committed_output_count)
    if let Ok(block) = bincode::deserialize::<crate::block::Block>(data) {
        return Some(block);
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
