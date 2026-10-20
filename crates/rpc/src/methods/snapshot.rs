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
    ///   "totalBytes": 123456,
    ///   "maintainerState": { "members": ["hex..."], "threshold": 3, "lastChangeBlock": 0, "lastDerivedHeight": 12345, "digest": "hex...", "genesisHash": "hex..." }
    /// }
    /// ```
    pub(super) async fn get_state_snapshot(&self) -> Result<Value, RpcError> {
        let snapshot = {
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

            storage::StateSnapshot::create_gated(
                &chain_state,
                &utxo_set,
                producer_set_ref,
                self.inc_i_181_pending_root_activation_height,
            )
            .map_err(|e| RpcError::internal_error(format!("Failed to create snapshot: {}", e)))?
        };

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

        let maintainer_state = self.snapshot_maintainer_state().await;

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
            "maintainerState": maintainer_state,
        });

        Ok(response)
    }

    /// The `maintainerState` field; `null` when no maintainer state is attached.
    async fn snapshot_maintainer_state(&self) -> Value {
        let Some(ms) = &self.maintainer_state else {
            return Value::Null;
        };
        let state = ms.read().await;
        let genesis_hash = self.params.genesis_hash;
        let digest =
            doli_core::maintainer::maintainer_set_digest(&state.set, genesis_hash.as_bytes());
        serde_json::json!({
            "members": state.set.members.iter().map(|k| k.to_hex()).collect::<Vec<_>>(),
            "threshold": state.set.threshold,
            "lastChangeBlock": state.set.last_updated,
            "lastDerivedHeight": state.last_derived_height,
            "digest": hex::encode(digest),
            "genesisHash": genesis_hash.to_hex(),
        })
    }
}
