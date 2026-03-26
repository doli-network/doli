//! JSON-RPC client for communicating with DOLI nodes.
//!
//! Provides an async HTTP client that wraps all DOLI RPC methods.
//! Extracted from `bins/cli/src/rpc_client.rs` to be shared between CLI and GUI.

use anyhow::{anyhow, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::time::Duration;

use crate::types::*;

/// JSON-RPC request envelope.
#[derive(Serialize)]
struct RpcRequest<T: Serialize> {
    jsonrpc: &'static str,
    method: &'static str,
    params: T,
    id: u64,
}

impl<T: Serialize> RpcRequest<T> {
    fn new(method: &'static str, params: T) -> Self {
        Self {
            jsonrpc: "2.0",
            method,
            params,
            id: 1,
        }
    }
}

/// JSON-RPC response envelope.
#[derive(Deserialize)]
struct RpcResponse<T> {
    result: Option<T>,
    error: Option<RpcError>,
}

/// JSON-RPC error.
#[derive(Deserialize, Debug)]
struct RpcError {
    code: i32,
    message: String,
}

/// Default RPC timeout.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Default connection timeout.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// JSON-RPC client for DOLI node communication.
pub struct RpcClient {
    endpoint: String,
    client: reqwest::Client,
}

impl RpcClient {
    /// Create a new RPC client pointing at the given endpoint URL.
    pub fn new(endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.to_string(),
            client: reqwest::Client::builder()
                .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
                .timeout(DEFAULT_TIMEOUT)
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// Get the current endpoint URL.
    pub fn url(&self) -> &str {
        &self.endpoint
    }

    /// Internal method: make an RPC call and deserialize the result.
    async fn call<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &'static str,
        params: P,
    ) -> Result<R> {
        let request = RpcRequest::new(method, params);

        let response = self
            .client
            .post(&self.endpoint)
            .json(&request)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to connect to node: {}", e))?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "RPC request failed with status: {}",
                response.status()
            ));
        }

        let rpc_response: RpcResponse<R> = response
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse response: {}", e))?;

        if let Some(error) = rpc_response.error {
            return Err(anyhow!("RPC error {}: {}", error.code, error.message));
        }

        rpc_response
            .result
            .ok_or_else(|| anyhow!("No result in response"))
    }

    /// Get balance for an address (pubkey_hash).
    pub async fn get_balance(&self, address: &str) -> Result<Balance> {
        #[derive(Serialize)]
        struct Params<'a> {
            address: &'a str,
        }
        self.call("getBalance", Params { address }).await
    }

    /// Get UTXOs for an address.
    pub async fn get_utxos(&self, address: &str, spendable_only: bool) -> Result<Vec<Utxo>> {
        #[derive(Serialize)]
        struct Params<'a> {
            address: &'a str,
            spendable_only: bool,
        }
        self.call(
            "getUtxos",
            Params {
                address,
                spendable_only,
            },
        )
        .await
    }

    /// Send a signed transaction (hex-encoded).
    pub async fn send_transaction(&self, tx_hex: &str) -> Result<String> {
        #[derive(Serialize)]
        struct Params<'a> {
            tx: &'a str,
        }
        self.call("sendTransaction", Params { tx: tx_hex }).await
    }

    /// Get chain info (network, height, hash, slot, genesis).
    pub async fn get_chain_info(&self) -> Result<ChainInfo> {
        self.call("getChainInfo", serde_json::json!({})).await
    }

    /// Get transaction history for an address.
    pub async fn get_history(&self, pubkey_hash: &str, limit: u32) -> Result<Vec<HistoryEntry>> {
        #[derive(Serialize)]
        struct Params<'a> {
            address: &'a str,
            limit: u32,
        }
        self.call(
            "getHistory",
            Params {
                address: pubkey_hash,
                limit,
            },
        )
        .await
    }

    /// Get list of all producers.
    pub async fn get_producers(&self) -> Result<Vec<ProducerInfo>> {
        self.call("getProducers", serde_json::json!({})).await
    }

    /// Get network parameters.
    pub async fn get_network_params(&self) -> Result<NetworkParams> {
        self.call("getNetworkParams", serde_json::json!({})).await
    }

    /// Get epoch info.
    pub async fn get_epoch_info(&self) -> Result<EpochInfo> {
        self.call("getEpochInfo", serde_json::json!({})).await
    }

    /// Get rewards list for a producer public key.
    pub async fn get_rewards_list(&self, pubkey: &str) -> Result<Vec<RewardEpoch>> {
        #[derive(Serialize)]
        struct Params<'a> {
            #[serde(rename = "publicKey")]
            public_key: &'a str,
        }
        self.call("getRewardsList", Params { public_key: pubkey })
            .await
    }

    /// Get bond details for a producer.
    pub async fn get_bond_details(&self, pubkey: &str) -> Result<BondDetailsInfo> {
        #[derive(Serialize)]
        struct Params<'a> {
            #[serde(rename = "publicKey")]
            public_key: &'a str,
        }
        self.call("getBondDetails", Params { public_key: pubkey })
            .await
    }

    /// Simulate withdrawal for a producer.
    pub async fn simulate_withdrawal(
        &self,
        pubkey: &str,
        bond_count: u32,
    ) -> Result<WithdrawalSimulation> {
        #[derive(Serialize)]
        struct Params<'a> {
            #[serde(rename = "publicKey")]
            public_key: &'a str,
            bond_count: u32,
        }
        self.call(
            "simulateWithdrawal",
            Params {
                public_key: pubkey,
                bond_count,
            },
        )
        .await
    }

    /// Test connectivity to the endpoint. Returns Ok(true) if reachable.
    pub async fn test_connection(&self) -> Result<bool> {
        match self.get_chain_info().await {
            Ok(_) => Ok(true),
            Err(e) => Err(e),
        }
    }
}

/// Default RPC endpoints per network.
pub fn default_endpoints(network: &str) -> Vec<String> {
    match network {
        "mainnet" => vec![
            "https://rpc1.doli.network".to_string(),
            "https://rpc2.doli.network".to_string(),
        ],
        "testnet" => vec!["https://testnet-rpc.doli.network".to_string()],
        "devnet" => vec!["http://127.0.0.1:28500".to_string()],
        _ => vec![],
    }
}

/// Address prefix for a given network.
pub fn network_prefix(network: &str) -> &str {
    match network {
        "mainnet" => "doli",
        "testnet" => "tdoli",
        "devnet" => "ddoli",
        _ => "doli",
    }
}

#[cfg(test)]
#[path = "rpc_client_tests.rs"]
mod tests;
