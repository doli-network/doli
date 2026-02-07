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
    /// Confirmed balance (in base units) - spendable
    pub confirmed: u64,
    /// Unconfirmed balance (in base units) - in mempool
    pub unconfirmed: u64,
    /// Immature balance (in base units) - coinbase/rewards pending maturity
    #[serde(default)]
    pub immature: u64,
    /// Total balance (confirmed + unconfirmed + immature)
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

/// Network parameters (from getNetworkParams RPC)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkParams {
    /// Network name (mainnet, testnet, devnet)
    pub network: String,
    /// Bond unit in base units (amount per bond)
    pub bond_unit: u64,
    /// Slot duration in seconds
    pub slot_duration: u64,
    /// Slots per epoch
    pub slots_per_epoch: u32,
    /// Blocks per reward epoch
    pub blocks_per_reward_epoch: u64,
    /// Coinbase maturity (blocks before spendable)
    pub coinbase_maturity: u64,
    /// Initial block reward
    pub initial_reward: u64,
    /// Genesis time
    pub genesis_time: u64,
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

/// History entry from getHistory RPC
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    /// Transaction hash
    pub hash: String,
    /// Transaction type (string)
    pub tx_type: String,
    /// Block hash
    pub block_hash: String,
    /// Block height
    pub height: u64,
    /// Timestamp
    pub timestamp: u64,
    /// Amount received
    pub amount_received: u64,
    /// Amount sent
    pub amount_sent: u64,
    /// Fee
    pub fee: u64,
    /// Confirmations
    pub confirmations: u64,
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

    /// Get network parameters (bond_unit, slot_duration, etc.)
    pub async fn get_network_params(&self) -> Result<NetworkParams> {
        #[derive(Serialize)]
        struct Params {}

        self.call("getNetworkParams", Params {}).await
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
    pub async fn get_history(&self, address: &str, limit: usize) -> Result<Vec<HistoryEntry>> {
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

    /// Get block by hash
    pub async fn get_block(&self, hash: &str) -> Result<BlockInfo> {
        #[derive(Serialize)]
        struct Params<'a> {
            hash: &'a str,
        }

        self.call("getBlockByHash", Params { hash }).await
    }

    // ==================== Reward/Claim Methods ====================

    /// Get claimable rewards for a producer
    pub async fn get_claimable_rewards(&self, producer: &str) -> Result<ClaimableRewardsResponse> {
        #[derive(Serialize)]
        struct Params<'a> {
            producer: &'a str,
        }

        self.call("getClaimableRewards", Params { producer }).await
    }

    /// Get claim history for a producer
    pub async fn get_claim_history(
        &self,
        producer: &str,
        limit: usize,
    ) -> Result<ClaimHistoryResponse> {
        #[derive(Serialize)]
        struct Params<'a> {
            producer: &'a str,
            limit: usize,
        }

        self.call("getClaimHistory", Params { producer, limit })
            .await
    }

    /// Estimate reward for a specific epoch
    pub async fn estimate_epoch_reward(
        &self,
        producer: &str,
        epoch: u64,
    ) -> Result<RewardEstimateResponse> {
        #[derive(Serialize)]
        struct Params<'a> {
            producer: &'a str,
            epoch: u64,
        }

        self.call("estimateEpochReward", Params { producer, epoch })
            .await
    }

    /// Build an unsigned claim transaction
    pub async fn build_claim_tx(
        &self,
        producer: &str,
        epoch: u64,
        recipient: Option<&str>,
    ) -> Result<BuildClaimTxResponse> {
        #[derive(Serialize)]
        struct Params<'a> {
            producer: &'a str,
            epoch: u64,
            #[serde(skip_serializing_if = "Option::is_none")]
            recipient: Option<&'a str>,
        }

        self.call(
            "buildClaimTx",
            Params {
                producer,
                epoch,
                recipient,
            },
        )
        .await
    }

    /// Get current reward epoch information
    pub async fn get_epoch_info(&self) -> Result<EpochInfoResponse> {
        #[derive(Serialize)]
        struct Params {}

        self.call("getEpochInfo", Params {}).await
    }

    /// Get update status (pending updates, veto status, etc.)
    pub async fn get_update_status(&self) -> Result<serde_json::Value> {
        #[derive(Serialize)]
        struct Params {}

        self.call("getUpdateStatus", Params {}).await
    }

    /// Submit a vote for a pending update
    pub async fn submit_vote(&self, vote_json: serde_json::Value) -> Result<serde_json::Value> {
        #[derive(Serialize)]
        struct Params {
            vote: serde_json::Value,
        }

        self.call("submitVote", Params { vote: vote_json }).await
    }

    /// Get maintainer set
    pub async fn get_maintainer_set(&self) -> Result<serde_json::Value> {
        #[derive(Serialize)]
        struct Params {}

        self.call("getMaintainerSet", Params {}).await
    }
}

/// Block information (for CLI display)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockInfo {
    /// Block hash
    pub hash: String,
    /// Block height
    pub height: u64,
    /// Slot number
    pub slot: u32,
    /// Producer public key (hex)
    pub producer: String,
    /// Timestamp
    pub timestamp: u64,
}

// ==================== Reward/Claim Types ====================

/// A single claimable epoch entry
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimableEpochEntry {
    /// Epoch number
    pub epoch: u64,
    /// Number of blocks where producer was present
    pub blocks_present: u64,
    /// Total blocks in the epoch
    pub total_blocks: u64,
    /// Presence rate as percentage (0-100)
    pub presence_rate: u8,
    /// Estimated reward amount (in base units)
    pub estimated_reward: u64,
}

/// Response for getClaimableRewards
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimableRewardsResponse {
    /// Producer public key
    pub producer: String,
    /// Current block height
    pub current_height: u64,
    /// Current reward epoch
    pub current_epoch: u64,
    /// Total claimable epochs
    pub claimable_count: usize,
    /// Total estimated reward across all unclaimed epochs
    pub total_estimated_reward: u64,
    /// List of claimable epochs
    pub epochs: Vec<ClaimableEpochEntry>,
}

/// A single claim history entry
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimHistoryEntry {
    /// Epoch that was claimed
    pub epoch: u64,
    /// Transaction hash that executed the claim
    pub tx_hash: String,
    /// Block height where claim was confirmed
    pub height: u64,
    /// Amount claimed (in base units)
    pub amount: u64,
    /// Timestamp when claimed
    pub timestamp: u64,
}

/// Response for getClaimHistory
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimHistoryResponse {
    /// Producer public key
    pub producer: String,
    /// Total number of claims made
    pub total_claims: usize,
    /// Total amount claimed
    pub total_claimed: u64,
    /// List of claim entries (most recent first)
    pub claims: Vec<ClaimHistoryEntry>,
}

/// Response for estimateEpochReward
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RewardEstimateResponse {
    /// Producer public key
    pub producer: String,
    /// Epoch number
    pub epoch: u64,
    /// Whether epoch is complete
    pub is_complete: bool,
    /// Whether epoch is already claimed
    pub is_claimed: bool,
    /// Number of blocks where producer was present
    pub blocks_present: u64,
    /// Total blocks in epoch
    pub total_blocks: u64,
    /// Presence rate as percentage
    pub presence_rate: u8,
    /// Block reward rate
    pub block_reward: u64,
    /// Estimated reward amount
    pub estimated_reward: u64,
    /// Average producer weight per block
    pub average_weight: u64,
}

/// Response for buildClaimTx
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildClaimTxResponse {
    /// Epoch being claimed
    pub epoch: u64,
    /// Calculated reward amount
    pub amount: u64,
    /// Recipient address hash
    pub recipient: String,
    /// Raw unsigned transaction (hex)
    pub unsigned_tx: String,
    /// Message to sign (hex)
    pub signing_message: String,
}

/// Response for getEpochInfo
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpochInfoResponse {
    /// Current block height
    pub current_height: u64,
    /// Current reward epoch
    pub current_epoch: u64,
    /// Last complete epoch (if any)
    pub last_complete_epoch: Option<u64>,
    /// Blocks per reward epoch
    pub blocks_per_epoch: u64,
    /// Blocks remaining in current epoch
    pub blocks_remaining: u64,
    /// Current epoch start height
    pub epoch_start_height: u64,
    /// Current epoch end height (exclusive)
    pub epoch_end_height: u64,
    /// Current block reward rate
    pub block_reward: u64,
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
