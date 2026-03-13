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

/// RPC response wrapper.
#[derive(Debug, Deserialize)]
struct RpcResponse<T> {
    result: Option<T>,
    error: Option<String>,
}

/// UTXO as returned by the RPC.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcUtxo {
    pub tx_hash: String,
    pub output_index: u32,
    pub amount: u64,
    pub output_type: u8,
    pub pubkey_hash: String,
    pub lock_until: u64,
    pub extra_data: String,
}

/// Transaction status.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TxStatus {
    pub confirmed: bool,
    pub confirmations: u32,
    pub block_height: Option<u64>,
}

/// Block info for monitoring.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockInfo {
    pub height: u64,
    pub hash: String,
    pub tx_count: u32,
}

impl RpcClient {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Check connectivity.
    pub async fn ping(&self) -> bool {
        self.get_height().await.is_ok()
    }

    /// Get the current chain height.
    pub async fn get_height(&self) -> Result<BlockHeight> {
        let resp: RpcResponse<u64> = self
            .client
            .post(format!("{}/rpc", self.url))
            .json(&serde_json::json!({
                "method": "getHeight",
                "params": {}
            }))
            .send()
            .await?
            .json()
            .await?;

        resp.result
            .ok_or_else(|| ChannelError::Rpc(resp.error.unwrap_or_default()))
    }

    /// Get UTXOs for a pubkey hash.
    pub async fn get_utxos(&self, pubkey_hash: &str) -> Result<Vec<RpcUtxo>> {
        let resp: RpcResponse<Vec<RpcUtxo>> = self
            .client
            .post(format!("{}/rpc", self.url))
            .json(&serde_json::json!({
                "method": "getUtxos",
                "params": { "pubkey_hash": pubkey_hash }
            }))
            .send()
            .await?
            .json()
            .await?;

        resp.result
            .ok_or_else(|| ChannelError::Rpc(resp.error.unwrap_or_default()))
    }

    /// Submit a raw transaction (hex-encoded bincode).
    pub async fn submit_transaction(&self, tx_hex: &str) -> Result<String> {
        let resp: RpcResponse<String> = self
            .client
            .post(format!("{}/rpc", self.url))
            .json(&serde_json::json!({
                "method": "submitTransaction",
                "params": { "tx": tx_hex }
            }))
            .send()
            .await?
            .json()
            .await?;

        resp.result
            .ok_or_else(|| ChannelError::Rpc(resp.error.unwrap_or_default()))
    }

    /// Get transaction status (confirmation count).
    pub async fn get_transaction_status(&self, tx_hash: &str) -> Result<TxStatus> {
        let resp: RpcResponse<TxStatus> = self
            .client
            .post(format!("{}/rpc", self.url))
            .json(&serde_json::json!({
                "method": "getTransactionStatus",
                "params": { "tx_hash": tx_hash }
            }))
            .send()
            .await?
            .json()
            .await?;

        resp.result
            .ok_or_else(|| ChannelError::Rpc(resp.error.unwrap_or_default()))
    }

    /// Get block at a given height.
    pub async fn get_block(&self, height: BlockHeight) -> Result<BlockInfo> {
        let resp: RpcResponse<BlockInfo> = self
            .client
            .post(format!("{}/rpc", self.url))
            .json(&serde_json::json!({
                "method": "getBlock",
                "params": { "height": height }
            }))
            .send()
            .await?
            .json()
            .await?;

        resp.result
            .ok_or_else(|| ChannelError::Rpc(resp.error.unwrap_or_default()))
    }

    /// Submit a serialized transaction.
    pub async fn broadcast_transaction(&self, tx: &Transaction) -> Result<String> {
        let tx_hex = hex::encode(tx.serialize());
        debug!("broadcasting tx: {}", &tx_hex[..40.min(tx_hex.len())]);
        self.submit_transaction(&tx_hex).await
    }
}
