//! Tauri command handlers.
//!
//! Each submodule corresponds to a feature domain. Commands are the IPC bridge
//! between the Svelte frontend and the wallet library. Private keys NEVER cross
//! this boundary (GUI-NF-004).

pub mod bridge;
pub mod governance;
pub mod network;
pub mod nft;
pub mod node;
pub mod producer;
pub mod rewards;
pub mod transaction;
pub mod wallet;

use serde::{Deserialize, Serialize};

// ============================================================================
// Shared response types for Tauri commands
// ============================================================================

/// Response for wallet creation (includes seed phrase -- shown once, never stored).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWalletResponse {
    pub name: String,
    pub seed_phrase: String,
    pub primary_address: String,
    pub bech32_address: String,
}

/// Wallet info (public data only -- no private keys).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletInfo {
    pub name: String,
    pub version: u32,
    pub address_count: usize,
    pub primary_address: String,
    pub primary_public_key: String,
    pub bech32_address: String,
    pub has_bls_key: bool,
}

/// Address info (public data only).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddressInfo {
    pub address: String,
    pub public_key: String,
    pub label: Option<String>,
    pub bech32_address: String,
    pub has_bls_key: bool,
}

/// Balance response.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceResponse {
    pub confirmed: u64,
    pub unconfirmed: u64,
    pub immature: u64,
    pub total: u64,
    pub formatted_total: String,
    pub formatted_confirmed: String,
}

/// Send transaction response.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendResponse {
    pub tx_hash: String,
    pub amount: u64,
    pub fee: u64,
    pub formatted_amount: String,
}

/// Generic transaction response (for producer, rewards, NFT, etc.).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TxResponse {
    pub tx_hash: String,
    pub tx_type: String,
    pub message: String,
}

/// History entry response.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntryResponse {
    pub hash: String,
    pub tx_type: String,
    pub height: u64,
    pub timestamp: u64,
    pub amount_received: u64,
    pub amount_sent: u64,
    pub fee: u64,
    pub confirmations: u64,
    pub formatted_received: String,
    pub formatted_sent: String,
    pub net_amount: String,
}

/// Producer status response.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProducerStatusResponse {
    pub is_registered: bool,
    pub status: String,
    pub bond_count: u32,
    pub bond_amount: u64,
    pub formatted_bond_amount: String,
    pub registration_height: u64,
    pub era: u64,
}

/// Withdrawal simulation response.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulateResponse {
    pub bond_count: u32,
    pub total_staked: u64,
    pub total_penalty: u64,
    pub net_amount: u64,
    pub formatted_total_staked: String,
    pub formatted_penalty: String,
    pub formatted_net: String,
}

/// Reward epoch response.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RewardEpochResponse {
    pub epoch: u64,
    pub estimated_reward: u64,
    pub formatted_reward: String,
    pub qualified: bool,
    pub claimed: bool,
}

/// NFT info response.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NftInfoResponse {
    pub utxo_ref: String,
    pub content: String,
    pub value: u64,
    pub formatted_value: String,
}

/// Token info response.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenInfoResponse {
    pub utxo_ref: String,
    pub ticker: String,
    pub supply: String,
}

/// Bridge lock parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeLockParams {
    pub recipient: String,
    pub amount: String,
    pub hash_lock: String,
    pub timeout_height: u64,
}

/// Update info response.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub description: String,
    pub votes_for: u64,
    pub votes_against: u64,
    pub status: String,
}

/// Update status response.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatusResponse {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
}

/// Chain info response.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainInfoResponse {
    pub network: String,
    pub best_hash: String,
    pub best_height: u64,
    pub best_slot: u64,
    pub genesis_hash: String,
}

/// Connection status.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStatus {
    pub connected: bool,
    pub endpoint: String,
    pub network: String,
    pub chain_height: Option<u64>,
    pub status: String,
}

/// Connection test result.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTestResult {
    pub success: bool,
    pub network: Option<String>,
    pub height: Option<u64>,
    pub error: Option<String>,
}
