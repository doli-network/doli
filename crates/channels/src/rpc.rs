//! RPC client for chain interaction.
//!
//! Wraps DOLI node RPC calls needed by the channel system:
//! height, UTXOs, transaction submission, and status queries.

use doli_core::transaction::Transaction;
use doli_core::BlockHeight;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::error::{ChannelError, Result};

/// RPC client for interacting with a DOLI node.
#[derive(Clone)]
pub struct RpcClient {
    url: String,
    client: reqwest::Client,
}

/// JSON-RPC 2.0 response wrapper.
#[derive(Debug, Deserialize)]
struct JsonRpcResponse<T> {
    result: Option<T>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    message: String,
}

/// Chain info from getChainInfo.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChainInfoResult {
    best_height: u64,
}

/// UTXO as returned by the RPC.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcUtxo {
    pub tx_hash: String,
    pub output_index: u32,
    pub amount: u64,
    pub output_type: String,
    pub pubkey_hash: String,
    #[serde(default)]
    pub lock_until: u64,
    #[serde(default)]
    pub extra_data: String,
    #[serde(default)]
    pub spendable: bool,
}

/// Transaction response from getTransaction.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TxStatus {
    #[serde(default)]
    pub block_height: Option<u64>,
    #[serde(default)]
    pub confirmations: Option<u64>,
}

impl TxStatus {
    pub fn confirmed(&self) -> bool {
        self.confirmations.is_some_and(|c| c > 0)
    }
}

/// Block info for monitoring.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockInfo {
    pub height: u64,
    pub hash: String,
    #[serde(default)]
    pub tx_count: u32,
}

impl RpcClient {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Make a JSON-RPC 2.0 call.
    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<T> {
        let resp: JsonRpcResponse<T> = self
            .client
            .post(&self.url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": method,
                "params": params
            }))
            .send()
            .await?
            .json()
            .await?;

        resp.result.ok_or_else(|| {
            ChannelError::Rpc(
                resp.error
                    .map(|e| e.message)
                    .unwrap_or_else(|| "unknown error".to_string()),
            )
        })
    }

    /// Check connectivity.
    pub async fn ping(&self) -> bool {
        self.get_height().await.is_ok()
    }

    /// Get the current chain height.
    pub async fn get_height(&self) -> Result<BlockHeight> {
        let info: ChainInfoResult = self.call("getChainInfo", serde_json::json!({})).await?;
        Ok(info.best_height)
    }

    /// Get UTXOs for an address (bech32 or hex pubkey hash).
    pub async fn get_utxos(&self, address: &str) -> Result<Vec<RpcUtxo>> {
        self.call("getUtxos", serde_json::json!({ "address": address }))
            .await
    }

    /// Submit a raw transaction (hex-encoded bincode).
    pub async fn submit_transaction(&self, tx_hex: &str) -> Result<String> {
        self.call("sendTransaction", serde_json::json!({ "tx": tx_hex }))
            .await
    }

    /// Get transaction status (confirmation count).
    pub async fn get_transaction_status(&self, tx_hash: &str) -> Result<TxStatus> {
        self.call("getTransaction", serde_json::json!({ "hash": tx_hash }))
            .await
    }

    /// Get block at a given height.
    pub async fn get_block(&self, height: BlockHeight) -> Result<BlockInfo> {
        self.call("getBlockByHeight", serde_json::json!({ "height": height }))
            .await
    }

    /// Submit a serialized transaction.
    pub async fn broadcast_transaction(&self, tx: &Transaction) -> Result<String> {
        let tx_hex = hex::encode(tx.serialize());
        debug!("broadcasting tx: {}", &tx_hex[..40.min(tx_hex.len())]);
        self.submit_transaction(&tx_hex).await
    }
}
