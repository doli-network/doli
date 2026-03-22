//! Stats and debug handlers: getChainStats, getStateRootDebug, getUtxoDiff, getMempoolTransactions

use serde_json::Value;

use crypto::Hash;

use crate::error::RpcError;
use crate::types::*;

use super::context::RpcContext;

impl RpcContext {
    /// Get chain statistics (supply, address count, UTXO count, staking info)
    pub(super) async fn get_chain_stats(&self) -> Result<Value, RpcError> {
        let chain_state = self.chain_state.read().await;
        let height = chain_state.best_height;
        drop(chain_state);

        let utxo_set = self.utxo_set.read().await;
        let total_supply = utxo_set.total_supply();
        let address_count = utxo_set.address_count();
        let utxo_count = utxo_set.utxo_count();
        drop(utxo_set);

        let (active_producers, total_staked) = if let Some(ref ps) = self.producer_set {
            let producers = ps.read().await;
            let active_list = producers.active_producers();
            let staked: u64 = active_list.iter().map(|p| p.bond_amount).sum();
            (active_list.len(), staked)
        } else {
            (0, 0)
        };

        let response = ChainStatsResponse {
            total_supply,
            address_count,
            utxo_count,
            active_producers,
            total_staked,
            height,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Debug: compute state root and return per-component hashes.
    ///
    /// Returns `{ height, bestHash, stateRoot, csHash, utxoHash, psHash, utxoCount, producerCount }`
    /// so operators can identify which component diverges across nodes.
    pub(super) async fn get_state_root_debug(&self) -> Result<Value, RpcError> {
        let chain_state = self.chain_state.read().await;
        let utxo_set = self.utxo_set.read().await;

        let cs_bytes = chain_state.serialize_canonical();
        let utxo_bytes = utxo_set.serialize_canonical();

        let cs_hash = crypto::hash::hash(&cs_bytes);
        let utxo_hash = crypto::hash::hash(&utxo_bytes);

        let (ps_hash, producer_count) = if let Some(ref ps_arc) = self.producer_set {
            let ps = ps_arc.read().await;
            let ps_bytes = ps.serialize_canonical();
            let hash = crypto::hash::hash(&ps_bytes);
            let count = ps.active_count();
            (hash, count)
        } else {
            (Hash::ZERO, 0)
        };

        // Combine to get the full state root
        let mut combined = Vec::with_capacity(96);
        combined.extend_from_slice(cs_hash.as_bytes());
        combined.extend_from_slice(utxo_hash.as_bytes());
        combined.extend_from_slice(ps_hash.as_bytes());
        let state_root = crypto::hash::hash(&combined);

        Ok(serde_json::json!({
            "height": chain_state.best_height,
            "bestHash": chain_state.best_hash.to_string(),
            "stateRoot": state_root.to_string(),
            "csHash": cs_hash.to_string(),
            "utxoHash": utxo_hash.to_string(),
            "psHash": ps_hash.to_string(),
            "utxoCount": utxo_set.len(),
            "producerCount": producer_count,
            "totalMinted": chain_state.total_minted,
            "registrationSeq": chain_state.registration_sequence,
        }))
    }

    /// Debug: return per-UTXO canonical hashes for diffing across nodes.
    ///
    /// With no params: returns all `[(outpoint_hex, entry_hash)]` sorted by outpoint.
    /// With `{"referenceHashes": ["hash1", ...]}`: returns only entries that differ
    /// (missing or different hash) — enables efficient remote diff.
    pub(super) async fn get_utxo_diff(&self, params: Value) -> Result<Value, RpcError> {
        let utxo_set = self.utxo_set.read().await;
        let chain_state = self.chain_state.read().await;
        let height = chain_state.best_height;
        drop(chain_state);

        // Build sorted list of (outpoint_bytes, canonical_entry_hash)
        let mut entries: Vec<(String, String, String)> = Vec::new();
        match &*utxo_set {
            storage::UtxoSet::InMemory(store) => {
                let mut sorted: Vec<_> = store.iter().collect();
                sorted.sort_by(|(a, _), (b, _)| a.to_bytes().cmp(&b.to_bytes()));
                for (outpoint, entry) in sorted {
                    let op_hex = hex::encode(outpoint.to_bytes());
                    let canonical = entry.serialize_canonical_bytes();
                    let entry_hash = crypto::hash::hash(&canonical).to_string();
                    // Include key fields for human inspection
                    let detail = format!(
                        "amt={} h={} type={} cb={} er={} lock={} ed={} pk={}",
                        entry.output.amount,
                        entry.height,
                        entry.output.output_type as u8,
                        entry.is_coinbase as u8,
                        entry.is_epoch_reward as u8,
                        entry.output.lock_until,
                        hex::encode(&entry.output.extra_data),
                        &entry.output.pubkey_hash.to_string()[..16],
                    );
                    entries.push((op_hex, entry_hash, detail));
                }
            }
            storage::UtxoSet::RocksDb(_) => {
                return Err(RpcError::internal_error(
                    "RocksDb UTXO set not supported for diff".to_string(),
                ));
            }
        }

        // If reference hashes provided, only return differences
        if let Ok(ref_params) = serde_json::from_value::<serde_json::Map<String, Value>>(params) {
            if let Some(Value::Array(ref_hashes)) = ref_params.get("referenceHashes") {
                let ref_set: std::collections::HashSet<String> = ref_hashes
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                let diffs: Vec<_> = entries
                    .iter()
                    .filter(|(_, hash, _)| !ref_set.contains(hash))
                    .map(|(op, hash, detail)| {
                        serde_json::json!({"outpoint": op, "hash": hash, "detail": detail})
                    })
                    .collect();
                return Ok(serde_json::json!({
                    "height": height,
                    "totalEntries": entries.len(),
                    "diffCount": diffs.len(),
                    "diffs": diffs,
                }));
            }
        }

        // Full dump: return all entries
        let all: Vec<_> = entries
            .iter()
            .map(|(op, hash, detail)| {
                serde_json::json!({"outpoint": op, "hash": hash, "detail": detail})
            })
            .collect();

        Ok(serde_json::json!({
            "height": height,
            "count": all.len(),
            "entries": all,
        }))
    }

    /// Get pending mempool transactions
    pub(super) async fn get_mempool_transactions(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetMempoolTxsParams =
            serde_json::from_value(params).unwrap_or(GetMempoolTxsParams { limit: 100 });
        let limit = params.limit.min(500);

        let mempool = self.mempool.read().await;
        let mut txs: Vec<MempoolTxResponse> = mempool
            .iter()
            .take(limit)
            .map(|(_hash, entry)| MempoolTxResponse {
                hash: entry.tx_hash.to_hex(),
                tx_type: format!("{:?}", entry.tx.tx_type).to_lowercase(),
                size: entry.size,
                fee: entry.fee,
                fee_rate: entry.fee_rate,
                added_time: entry.added_time,
            })
            .collect();

        // Sort by fee rate descending (highest-fee first)
        txs.sort_by(|a, b| b.fee_rate.cmp(&a.fee_rate));

        serde_json::to_value(txs).map_err(|e| RpcError::internal_error(e.to_string()))
    }
}
