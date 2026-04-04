//! Block store types, serialization, and column family constants

use doli_core::transaction::legacy::LegacyTransactionV2;
use doli_core::transaction::Transaction;

use crate::StorageError;

/// Serializable block body (v3: includes BLS aggregate signature + attestation bitfield)
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct BlockBody {
    pub(super) transactions: Vec<Transaction>,
    #[serde(default)]
    pub(super) aggregate_bls_signature: Vec<u8>,
    #[serde(default)]
    pub(super) attestation_bitfield: Vec<u8>,
}

/// v3.6.0 block body — uses LegacyTransactionV2 (has sighash_type, no committed_output_count).
#[derive(serde::Deserialize)]
struct LegacyBlockBodyV2 {
    transactions: Vec<LegacyTransactionV2>,
    #[serde(default)]
    aggregate_bls_signature: Vec<u8>,
}

/// v3.5.0 block body — uses LegacyInput (no sighash_type, no committed_output_count).
#[derive(serde::Deserialize)]
struct LegacyBlockBodyV1 {
    transactions: Vec<doli_core::transaction::legacy::LegacyTransaction>,
    #[serde(default)]
    aggregate_bls_signature: Vec<u8>,
}

/// Deserialize block body bytes with backward compatibility.
/// Tries current format first, then v3.6.0 (sighash_type), then v3.5.0 (no sighash), then plain Vec.
/// Returns (transactions, bls_sig, attestation_bitfield).
#[allow(clippy::type_complexity)]
pub(super) fn deserialize_body(
    bytes: &[u8],
) -> Result<(Vec<Transaction>, Vec<u8>, Vec<u8>), StorageError> {
    // Try current format (v3.7.1+: Input has committed_output_count)
    if let Ok(body) = bincode::deserialize::<BlockBody>(bytes) {
        return Ok((
            body.transactions,
            body.aggregate_bls_signature,
            body.attestation_bitfield,
        ));
    }
    // Fallback: v3.6.0 (Input has sighash_type but no committed_output_count)
    if let Ok(body) = bincode::deserialize::<LegacyBlockBodyV2>(bytes) {
        let txs = body
            .transactions
            .into_iter()
            .map(|t| t.into_current())
            .collect();
        return Ok((txs, body.aggregate_bls_signature, Vec::new()));
    }
    // Fallback: v3.5.0 (Input has no sighash_type)
    if let Ok(body) = bincode::deserialize::<LegacyBlockBodyV1>(bytes) {
        let txs = body
            .transactions
            .into_iter()
            .map(|t| t.into_current())
            .collect();
        return Ok((txs, body.aggregate_bls_signature, Vec::new()));
    }
    // Fallback: pre-BLS plain Vec<Transaction> (try all 3 tx formats)
    if let Ok(txs) = bincode::deserialize::<Vec<Transaction>>(bytes) {
        return Ok((txs, Vec::new(), Vec::new()));
    }
    if let Ok(txs) = bincode::deserialize::<Vec<LegacyTransactionV2>>(bytes) {
        let txs = txs.into_iter().map(|t| t.into_current()).collect();
        return Ok((txs, Vec::new(), Vec::new()));
    }
    let txs: Vec<doli_core::transaction::legacy::LegacyTransaction> =
        bincode::deserialize(bytes).map_err(|e| StorageError::Serialization(e.to_string()))?;
    Ok((
        txs.into_iter().map(|t| t.into_current()).collect(),
        Vec::new(),
        Vec::new(),
    ))
}

/// Column family names
pub(super) const CF_HEADERS: &str = "headers";
pub(super) const CF_BODIES: &str = "bodies";
pub(super) const CF_HEIGHT_INDEX: &str = "height_index";
pub(super) const CF_SLOT_INDEX: &str = "slot_index";
pub(super) const CF_PRESENCE: &str = "presence";
pub(super) const CF_HASH_TO_HEIGHT: &str = "hash_to_height";
pub(super) const CF_TX_INDEX: &str = "tx_index";
pub(super) const CF_ADDR_TX_INDEX: &str = "addr_tx_index";

/// Block store
pub struct BlockStore {
    pub(super) db: rocksdb::DB,
}
