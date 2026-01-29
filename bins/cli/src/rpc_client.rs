//! RPC client for communicating with DOLI nodes
//!
//! This module provides a JSON-RPC client for querying node state
//! and submitting transactions.

use anyhow::{anyhow, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// JSON-RPC request
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

/// JSON-RPC response
#[derive(Deserialize)]
struct RpcResponse<T> {
    result: Option<T>,
    error: Option<RpcError>,
}

/// JSON-RPC error
#[derive(Deserialize, Debug)]
struct RpcError {
    code: i32,
    message: String,
}

/// Balance information
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Balance {
    /// Confirmed balance (in base units)
    pub confirmed: u64,
    /// Unconfirmed balance (in base units)
    pub unconfirmed: u64,
    /// Total balance (confirmed + unconfirmed)
    pub total: u64,
}

/// UTXO information
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Utxo {
    /// Transaction hash
    pub tx_hash: String,
    /// Output index
    pub output_index: u32,
    /// Amount in base units
    pub amount: u64,
    /// Output type (normal, bond)
    pub output_type: String,
    /// Lock until height (0 if not locked)
    pub lock_until: u64,
    /// Block height where this UTXO was created
    pub height: u64,
    /// Whether this UTXO is currently spendable
    pub spendable: bool,
}

/// Chain information
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainInfo {
    /// Network name (mainnet, testnet)
    pub network: String,
    /// Best block hash
    pub best_hash: String,
    /// Best block height
    pub best_height: u64,
    /// Best block slot
    pub best_slot: u64,
    /// Genesis block hash
    pub genesis_hash: String,
}

/// Transaction information
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionInfo {
    /// Transaction hash
    pub hash: String,
    /// Version
    pub version: u32,
    /// Transaction type (0=transfer, 1=registration, 2=exit)
    pub tx_type: u32,
    /// Block hash (if confirmed)
    pub block_hash: Option<String>,
    /// Block height (if confirmed)
    pub block_height: Option<u64>,
    /// Confirmations
    pub confirmations: u64,
    /// Timestamp
    pub timestamp: Option<u64>,
    /// Fee
    pub fee: u64,
}

/// Producer information
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProducerInfo {
    /// Public key (hex)
    pub public_key: String,
    /// Registration height
    pub registration_height: u64,
    /// Bond amount (total)
    pub bond_amount: u64,
    /// Number of bonds staked
    #[serde(default = "default_bond_count")]
    pub bond_count: u32,
    /// Status (active, unbonding, exited, slashed)
    pub status: String,
    /// Blocks produced
    pub blocks_produced: u64,
    /// Pending rewards (unclaimed)
    #[serde(default)]
    pub pending_rewards: u64,
    /// Current era
    pub era: u64,
    /// Pending withdrawals
    #[serde(default)]
    pub pending_withdrawals: Vec<PendingWithdrawalInfo>,
}

fn default_bond_count() -> u32 {
    1
}

/// Pending withdrawal information
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingWithdrawalInfo {
    /// Number of bonds being withdrawn
    pub bond_count: u32,
    /// Slot when withdrawal was requested
    pub request_slot: u32,
    /// Net amount after penalty
    pub net_amount: u64,
    /// Whether this withdrawal can be claimed now
    pub claimable: bool,
}

/// RPC client for communicating with DOLI nodes
pub struct RpcClient {
    /// Node RPC endpoint
    endpoint: String,
    /// HTTP client
    client: reqwest::Client,
}

impl RpcClient {
    /// Create a new RPC client
    pub fn new(endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Make an RPC call
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

    /// Get balance for an address
    pub async fn get_balance(&self, address: &str) -> Result<Balance> {
        #[derive(Serialize)]
        struct Params<'a> {
            address: &'a str,
        }

        self.call("getBalance", Params { address }).await
    }

    /// Get UTXOs for an address
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

    /// Send a signed transaction
    pub async fn send_transaction(&self, tx_hex: &str) -> Result<String> {
        #[derive(Serialize)]
        struct Params<'a> {
            tx: &'a str,
        }

        self.call("sendTransaction", Params { tx: tx_hex }).await
    }

    /// Get chain information
    pub async fn get_chain_info(&self) -> Result<ChainInfo> {
        #[derive(Serialize)]
        struct Params {}

        self.call("getChainInfo", Params {}).await
    }

    /// Get transaction by hash
    pub async fn get_transaction(&self, hash: &str) -> Result<TransactionInfo> {
        #[derive(Serialize)]
        struct Params<'a> {
            hash: &'a str,
        }

        self.call("getTransaction", Params { hash }).await
    }

    /// Get transaction history for an address
    pub async fn get_history(&self, address: &str, limit: usize) -> Result<Vec<TransactionInfo>> {
        #[derive(Serialize)]
        struct Params<'a> {
            address: &'a str,
            limit: usize,
        }

        self.call("getHistory", Params { address, limit }).await
    }

    /// Get producer information
    pub async fn get_producer(&self, public_key: &str) -> Result<ProducerInfo> {
        #[derive(Serialize)]
        struct Params<'a> {
            public_key: &'a str,
        }

        self.call("getProducer", Params { public_key }).await
    }

    /// Get all producers in the network
    pub async fn get_producers(&self, active_only: bool) -> Result<Vec<ProducerInfo>> {
        #[derive(Serialize)]
        struct Params {
            active_only: bool,
        }

        self.call("getProducers", Params { active_only }).await
    }

    /// Register as a producer
    pub async fn register_producer(&self, tx_hex: &str) -> Result<String> {
        #[derive(Serialize)]
        struct Params<'a> {
            tx: &'a str,
        }

        self.call("registerProducer", Params { tx: tx_hex }).await
    }

    /// Request to withdraw as a producer
    pub async fn withdraw_producer(&self, tx_hex: &str) -> Result<String> {
        #[derive(Serialize)]
        struct Params<'a> {
            tx: &'a str,
        }

        self.call("withdrawProducer", Params { tx: tx_hex }).await
    }

    /// Check if the node is reachable
    pub async fn ping(&self) -> Result<bool> {
        match self.get_chain_info().await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }
}

/// Convert base units to display coins (8 decimal places)
pub fn units_to_coins(units: u64) -> f64 {
    units as f64 / 100_000_000.0
}

/// Convert display coins to base units
pub fn coins_to_units(coins: f64) -> u64 {
    (coins * 100_000_000.0) as u64
}

/// Format balance for display
pub fn format_balance(units: u64) -> String {
    let coins = units_to_coins(units);
    format!("{:.8} DOLI", coins)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_units_to_coins() {
        assert_eq!(units_to_coins(100_000_000), 1.0);
        assert_eq!(units_to_coins(500_000_000), 5.0);
        assert_eq!(units_to_coins(12_345_678), 0.12345678);
    }

    #[test]
    fn test_coins_to_units() {
        assert_eq!(coins_to_units(1.0), 100_000_000);
        assert_eq!(coins_to_units(5.0), 500_000_000);
        assert_eq!(coins_to_units(0.12345678), 12_345_678);
    }

    #[test]
    fn test_format_balance() {
        assert_eq!(format_balance(100_000_000), "1.00000000 DOLI");
        assert_eq!(format_balance(12_345_678), "0.12345678 DOLI");
    }
}
