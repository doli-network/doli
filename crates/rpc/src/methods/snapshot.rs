//! State snapshot RPC handler

use serde_json::Value;

use crate::error::RpcError;

use super::context::RpcContext;

impl RpcContext {
    /// Return a full state snapshot (chain_state, utxo_set, producer_set) as hex-encoded bytes.
    ///
    /// Response:
    /// ```json
    /// {
    ///   "height": 12345,
    ///   "blockHash": "abcd...",
    ///   "stateRoot": "ef01...",
    ///   "chainState": "hex...",
    ///   "utxoSet": "hex...",
    ///   "producerSet": "hex...",
    ///   "totalBytes": 123456
    /// }
    /// ```
    pub(super) async fn get_state_snapshot(&self) -> Result<Value, RpcError> {
        let chain_state = self.chain_state.read().await;
        let utxo_set = self.utxo_set.read().await;

        let producer_set_guard;
        let producer_set_ref = match &self.producer_set {
            Some(ps) => {
                producer_set_guard = ps.read().await;
                &*producer_set_guard
            }
            None => {
                return Err(RpcError::internal_error(
                    "Producer set not available".to_string(),
                ));
            }
        };

        let snapshot = storage::StateSnapshot::create(&chain_state, &utxo_set, producer_set_ref)
            .map_err(|e| RpcError::internal_error(format!("Failed to create snapshot: {}", e)))?;

        // Include persisted epoch meta keys so doli snap receivers get
        // the correct bond snapshot (computed at epoch boundary, not snap height).
        let epoch_meta = self.state_db.as_ref().and_then(|db| {
            let bond = db.get_epoch_bond_snapshot();
            bond.map(|(snap, epoch)| {
                let bytes = bincode::serialize(&(snap, epoch)).unwrap_or_default();
                hex::encode(&bytes)
            })
        });

        // Include attestation accumulators so doli snap receivers arrive at
        // the next epoch boundary with correct attestation data — eliminates
        // the 3-epoch convergence window where qualified-producer sets diverge.
        let epoch_accumulators = self.state_db.as_ref().and_then(|db| {
            db.get_attestation_accumulators().map(|data| {
                let bytes = bincode::serialize(&data).unwrap_or_default();
                hex::encode(&bytes)
            })
        });

        let response = serde_json::json!({
            "height": snapshot.block_height,
            "blockHash": snapshot.block_hash.to_hex(),
            "stateRoot": snapshot.state_root.to_hex(),
            "chainState": hex::encode(&snapshot.chain_state_bytes),
            "utxoSet": hex::encode(&snapshot.utxo_set_bytes),
            "producerSet": hex::encode(&snapshot.producer_set_bytes),
            "epochBondSnapshot": epoch_meta,
            "epochAccumulators": epoch_accumulators,
            "totalBytes": snapshot.total_bytes(),
        });

        Ok(response)
    }
}
