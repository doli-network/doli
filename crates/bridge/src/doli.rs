//! DOLI chain monitor.
//!
//! Polls a DOLI node via JSON-RPC to detect:
//! - New BridgeHTLC outputs (swap initiated)
//! - BridgeHTLC claims (preimage revealed on DOLI)
//! - Block height changes (for expiry tracking)

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::error::{BridgeError, Result};

/// JSON-RPC request.
#[derive(Serialize)]
struct RpcRequest<T: Serialize> {
    jsonrpc: &'static str,
    method: &'static str,
    params: T,
    id: u64,
}

/// JSON-RPC response.
#[derive(Deserialize)]
struct RpcResponse<T> {
    result: Option<T>,
    error: Option<RpcError>,
}

#[derive(Deserialize, Debug)]
struct RpcError {
    #[allow(dead_code)]
    code: i32,
    message: String,
}

/// DOLI chain client.
pub struct DoliClient {
    endpoint: String,
    client: reqwest::Client,
}

/// Chain info from getChainInfo RPC.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainInfo {
    pub best_height: u64,
    pub best_hash: String,
    pub best_slot: u64,
}

/// UTXO from getUtxos RPC.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoliUtxo {
    pub tx_hash: String,
    pub output_index: u32,
    pub amount: u64,
    pub output_type: String,
    pub lock_until: u64,
    pub spendable: bool,
    pub pubkey_hash: Option<String>,
    pub bridge: Option<BridgeMetadata>,
}

/// Bridge metadata from UTXO.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeMetadata {
    pub target_chain: Option<String>,
    pub target_chain_id: Option<u8>,
    pub target_address: Option<String>,
}

/// A detected BridgeHTLC on the DOLI chain.
#[derive(Debug, Clone)]
pub struct DetectedHtlc {
    pub tx_hash: String,
    pub output_index: u32,
    pub amount: u64,
    pub hash: String,
    pub lock_height: u64,
    pub expiry_height: u64,
    pub creator_pubkey_hash: String,
    pub target_chain: u8,
    pub target_address: String,
}

/// A detected preimage reveal (from a claim transaction spending a BridgeHTLC).
#[derive(Debug, Clone)]
pub struct RevealedPreimage {
    /// The BridgeHTLC UTXO that was spent
    pub htlc_tx_hash: String,
    pub htlc_output_index: u32,
    /// The claim transaction
    pub claim_tx_hash: String,
    /// The revealed preimage (hex)
    pub preimage: String,
}

impl DoliClient {
    pub fn new(endpoint: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self {
            endpoint: endpoint.to_string(),
            client,
        }
    }

    async fn call<P: Serialize, R: serde::de::DeserializeOwned>(
        &self,
        method: &'static str,
        params: P,
    ) -> Result<R> {
        let req = RpcRequest {
            jsonrpc: "2.0",
            method,
            params,
            id: 1,
        };
        let resp = self
            .client
            .post(&self.endpoint)
            .json(&req)
            .send()
            .await?
            .json::<RpcResponse<R>>()
            .await?;

        if let Some(err) = resp.error {
            return Err(BridgeError::DoliRpc(err.message));
        }
        resp.result
            .ok_or_else(|| BridgeError::DoliRpc("null result".to_string()))
    }

    /// Get current chain info (height, hash, slot).
    pub async fn get_chain_info(&self) -> Result<ChainInfo> {
        self.call("getChainInfo", serde_json::json!({})).await
    }

    /// Get all BridgeHTLC UTXOs for a given address.
    pub async fn get_bridge_utxos(&self, pubkey_hash: &str) -> Result<Vec<DoliUtxo>> {
        #[derive(Deserialize)]
        struct UtxoResponse {
            utxos: Vec<DoliUtxo>,
        }
        let resp: UtxoResponse = self
            .call(
                "getUtxos",
                serde_json::json!({ "address": pubkey_hash, "spendableOnly": false }),
            )
            .await?;
        Ok(resp
            .utxos
            .into_iter()
            .filter(|u| u.output_type == "bridgeHtlc")
            .collect())
    }

    /// Get a transaction by hash.
    pub async fn get_transaction(&self, tx_hash: &str) -> Result<serde_json::Value> {
        self.call("getTransaction", serde_json::json!({ "hash": tx_hash }))
            .await
    }

    /// Send a raw transaction (hex-encoded).
    pub async fn send_transaction(&self, tx_hex: &str) -> Result<String> {
        let result: serde_json::Value = self
            .call(
                "sendTransaction",
                serde_json::json!({ "transaction": tx_hex }),
            )
            .await?;
        Ok(result
            .get("hash")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string())
    }

    /// Scan recent blocks for BridgeHTLC outputs.
    /// Returns newly detected HTLCs since `from_height`.
    pub async fn scan_for_htlcs(&self, from_height: u64) -> Result<Vec<DetectedHtlc>> {
        let chain_info = self.get_chain_info().await?;
        let current_height = chain_info.best_height;

        if current_height <= from_height {
            return Ok(Vec::new());
        }

        let mut detected = Vec::new();

        // Scan blocks from_height+1 to current
        // For efficiency, we scan at most 100 blocks at a time
        let scan_from = from_height + 1;
        let scan_to = current_height.min(scan_from + 100);

        for height in scan_from..=scan_to {
            match self.scan_block_for_htlcs(height).await {
                Ok(htlcs) => detected.extend(htlcs),
                Err(e) => {
                    debug!("Error scanning block {}: {}", height, e);
                    // Continue scanning other blocks
                }
            }
        }

        if !detected.is_empty() {
            info!(
                "Found {} new BridgeHTLC(s) in blocks {}-{}",
                detected.len(),
                scan_from,
                scan_to
            );
        }

        Ok(detected)
    }

    /// Scan a single block for BridgeHTLC outputs.
    async fn scan_block_for_htlcs(&self, height: u64) -> Result<Vec<DetectedHtlc>> {
        let block: serde_json::Value = self
            .call("getBlockByHeight", serde_json::json!({ "height": height }))
            .await?;

        let txs = block
            .get("transactions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut detected = Vec::new();

        for tx in &txs {
            let tx_hash = tx.get("hash").and_then(|v| v.as_str()).unwrap_or_default();
            let outputs = tx
                .get("outputs")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            for (idx, output) in outputs.iter().enumerate() {
                let output_type = output
                    .get("outputType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if output_type != "bridgeHtlc" {
                    continue;
                }

                let amount = output.get("amount").and_then(|v| v.as_u64()).unwrap_or(0);
                let pubkey_hash = output
                    .get("pubkeyHash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                // Extract bridge metadata
                let bridge = output.get("bridge");
                let target_chain_id = bridge
                    .and_then(|b| b.get("targetChainId"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u8;
                let target_address = bridge
                    .and_then(|b| b.get("targetAddress"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                // Extract condition (lock/expiry heights)
                let condition = output.get("condition");
                let (lock_height, expiry_height, hash) = Self::parse_htlc_condition(condition);

                detected.push(DetectedHtlc {
                    tx_hash: tx_hash.to_string(),
                    output_index: idx as u32,
                    amount,
                    hash,
                    lock_height,
                    expiry_height,
                    creator_pubkey_hash: pubkey_hash,
                    target_chain: target_chain_id,
                    target_address,
                });
            }

            // Also check inputs for preimage reveals (spending BridgeHTLCs)
            // This is handled separately in scan_for_preimage_reveals
        }

        Ok(detected)
    }

    /// Parse HTLC condition from RPC output to extract lock, expiry, and hash.
    fn parse_htlc_condition(condition: Option<&serde_json::Value>) -> (u64, u64, String) {
        let cond = match condition {
            Some(c) => c,
            None => return (0, 0, String::new()),
        };

        // HTLC condition structure: Or(And(Hashlock(H), Timelock(L)), TimelockExpiry(E))
        // RPC serializes as nested JSON
        let mut lock = 0u64;
        let mut expiry = 0u64;
        let mut hash = String::new();

        // Try to extract from the condition tree
        if let Some(hashlock) = Self::find_in_condition(cond, "hashlock") {
            hash = hashlock
                .get("hash")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
        }
        if let Some(timelock) = Self::find_in_condition(cond, "timelock") {
            lock = timelock.get("height").and_then(|v| v.as_u64()).unwrap_or(0);
        }
        if let Some(expiry_cond) = Self::find_in_condition(cond, "timelockExpiry") {
            expiry = expiry_cond
                .get("height")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
        }

        (lock, expiry, hash)
    }

    /// Recursively find a condition type in a condition tree.
    fn find_in_condition<'a>(
        cond: &'a serde_json::Value,
        cond_type: &str,
    ) -> Option<&'a serde_json::Value> {
        if let Some(t) = cond.get("type").and_then(|v| v.as_str()) {
            if t == cond_type {
                return Some(cond);
            }
        }

        // Check children: And/Or have "left"/"right", Threshold has "conditions"
        for key in &["left", "right", "a", "b"] {
            if let Some(child) = cond.get(key) {
                if let Some(found) = Self::find_in_condition(child, cond_type) {
                    return Some(found);
                }
            }
        }
        if let Some(children) = cond.get("conditions").and_then(|v| v.as_array()) {
            for child in children {
                if let Some(found) = Self::find_in_condition(child, cond_type) {
                    return Some(found);
                }
            }
        }

        None
    }

    /// Scan recent blocks for preimage reveals (claims spending BridgeHTLCs).
    pub async fn scan_for_preimage_reveals(
        &self,
        from_height: u64,
    ) -> Result<Vec<RevealedPreimage>> {
        let chain_info = self.get_chain_info().await?;
        let current_height = chain_info.best_height;

        if current_height <= from_height {
            return Ok(Vec::new());
        }

        let mut reveals = Vec::new();
        let scan_from = from_height + 1;
        let scan_to = current_height.min(scan_from + 100);

        for height in scan_from..=scan_to {
            match self.scan_block_for_reveals(height).await {
                Ok(r) => reveals.extend(r),
                Err(e) => {
                    debug!("Error scanning block {} for reveals: {}", height, e);
                }
            }
        }

        if !reveals.is_empty() {
            info!("Found {} preimage reveal(s)", reveals.len());
        }

        Ok(reveals)
    }

    /// Scan a single block for preimage reveals.
    async fn scan_block_for_reveals(&self, height: u64) -> Result<Vec<RevealedPreimage>> {
        let block: serde_json::Value = self
            .call("getBlockByHeight", serde_json::json!({ "height": height }))
            .await?;

        let txs = block
            .get("transactions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut reveals = Vec::new();

        for tx in &txs {
            let tx_hash = tx.get("hash").and_then(|v| v.as_str()).unwrap_or_default();

            // Check if any input spends a BridgeHTLC (has covenant witness with preimage)
            let witnesses = tx.get("covenantWitnesses").and_then(|v| v.as_array());
            let inputs = tx.get("inputs").and_then(|v| v.as_array());

            if let (Some(witnesses), Some(inputs)) = (witnesses, inputs) {
                for (i, witness) in witnesses.iter().enumerate() {
                    let preimage = witness
                        .get("preimage")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    if let Some(preimage) = preimage {
                        if !preimage.is_empty() {
                            let input = &inputs[i];
                            let prev_hash = input
                                .get("prevTxHash")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string();
                            let prev_idx = input
                                .get("outputIndex")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0) as u32;

                            reveals.push(RevealedPreimage {
                                htlc_tx_hash: prev_hash,
                                htlc_output_index: prev_idx,
                                claim_tx_hash: tx_hash.to_string(),
                                preimage,
                            });
                        }
                    }
                }
            }
        }

        Ok(reveals)
    }

    /// Get current chain height.
    pub async fn height(&self) -> Result<u64> {
        let info = self.get_chain_info().await?;
        Ok(info.best_height)
    }

    /// Verify connectivity to the DOLI node.
    pub async fn ping(&self) -> bool {
        self.get_chain_info().await.is_ok()
    }
}
