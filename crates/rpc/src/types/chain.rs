//! Chain, network, balance, UTXO, history, mempool, stats, attestation,
//! and backfill types.

use serde::{Deserialize, Serialize};

// ==================== Response Types ====================

/// UTXO response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UtxoResponse {
    /// Transaction hash
    pub tx_hash: String,
    /// Output index
    pub output_index: u32,
    /// Amount
    pub amount: u64,
    /// Output type
    pub output_type: String,
    /// Lock until height
    pub lock_until: u64,
    /// Height when created
    pub height: u64,
    /// Whether spendable at current height
    pub spendable: bool,
    /// Whether this UTXO is from a pending mempool transaction (not yet confirmed)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pending: bool,
    /// Decoded covenant condition (only for conditioned output types)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<serde_json::Value>,
    /// NFT metadata (only for NFT output types)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nft: Option<serde_json::Value>,
    /// Fungible asset metadata (only for FungibleAsset output types)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<serde_json::Value>,
    /// Bridge HTLC metadata (only for BridgeHTLC output types)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bridge: Option<serde_json::Value>,
}

/// Chain info response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainInfoResponse {
    /// Network name
    pub network: String,
    /// Node version (e.g. "1.1.11")
    pub version: String,
    /// Best block hash
    pub best_hash: String,
    /// Best block height
    pub best_height: u64,
    /// Best block slot
    pub best_slot: u32,
    /// Genesis hash
    pub genesis_hash: String,
}

/// Network info response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInfoResponse {
    /// Local peer ID
    pub peer_id: String,
    /// Number of connected peers
    pub peer_count: usize,
    /// Whether syncing
    pub syncing: bool,
    /// Sync progress (0-100)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_progress: Option<f64>,
}

/// Individual peer info for getPeerInfo response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerInfoEntry {
    /// Peer ID (libp2p PeerId)
    pub peer_id: String,
    /// Remote address
    pub address: String,
    /// Best known height
    pub best_height: u64,
    /// Connection duration in seconds
    pub connected_secs: u64,
    /// Seconds since last message
    pub last_seen_secs: u64,
    /// Latency in milliseconds (if known)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
}

/// Mempool info response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MempoolInfoResponse {
    /// Number of transactions
    pub tx_count: usize,
    /// Total size in bytes
    pub total_size: usize,
    /// Minimum fee rate for acceptance
    pub min_fee_rate: u64,
    /// Maximum size limit
    pub max_size: usize,
    /// Maximum transaction count limit
    pub max_count: usize,
}

/// Balance response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceResponse {
    /// Confirmed balance (spendable)
    pub confirmed: u64,
    /// Unconfirmed balance (in mempool)
    pub unconfirmed: u64,
    /// Immature balance (coinbase/rewards pending maturity)
    pub immature: u64,
    /// Bonded balance (locked in Bond UTXOs)
    pub bonded: u64,
    /// Total balance (confirmed + unconfirmed + immature + bonded)
    pub total: u64,
}

// ==================== Request Parameters ====================

/// Parameters for getBalance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBalanceParams {
    /// Address or pubkey hash (hex)
    pub address: String,
}

/// Parameters for getUtxos
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetUtxosParams {
    /// Address or pubkey hash (hex)
    pub address: String,
    /// Only return spendable UTXOs
    #[serde(default)]
    pub spendable_only: bool,
}

/// Parameters for getHistory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetHistoryParams {
    /// Address or pubkey hash (hex)
    pub address: String,
    /// Maximum number of transactions to return
    #[serde(default = "default_history_limit")]
    pub limit: usize,
    /// Start scanning from this height (exclusive). Omit to start from chain tip.
    #[serde(default)]
    pub before_height: Option<u64>,
}

fn default_history_limit() -> usize {
    10
}

/// History entry response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntryResponse {
    /// Transaction hash
    pub hash: String,
    /// Transaction type
    pub tx_type: String,
    /// Block hash
    pub block_hash: String,
    /// Block height
    pub height: u64,
    /// Timestamp
    pub timestamp: u64,
    /// Amount received (if any)
    pub amount_received: u64,
    /// Amount sent (if any)
    pub amount_sent: u64,
    /// Fee (for outgoing transactions)
    pub fee: u64,
    /// Confirmations
    pub confirmations: u64,
}

// ==================== Stats & Attestation ====================

/// Chain statistics response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainStatsResponse {
    /// Total UTXO supply (base units)
    pub total_supply: u64,
    /// Number of unique addresses with UTXOs
    pub address_count: u64,
    /// Total UTXO count
    pub utxo_count: u64,
    /// Number of active producers
    pub active_producers: usize,
    /// Total bonds staked (base units)
    pub total_staked: u64,
    /// Chain height
    pub height: u64,
    /// Reward pool balance (base units) — sum of all coinbase UTXOs held by the pool
    pub reward_pool_balance: u64,
}

/// Attestation statistics for current epoch
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttestationStatsResponse {
    /// Current epoch number
    pub epoch: u32,
    /// Epoch start height
    pub epoch_start: u64,
    /// Current chain height
    pub current_height: u64,
    /// Total blocks in this epoch so far
    pub blocks_in_epoch: u64,
    /// Blocks with non-zero presence_root (attestation bitfield committed)
    pub blocks_with_attestations: u64,
    /// Blocks with BLS aggregate signature
    pub blocks_with_bls: u64,
    /// Current attestation minute within the epoch
    pub current_minute: u32,
    /// Per-producer attestation stats (sorted by pubkey)
    pub producers: Vec<ProducerAttestationStats>,
}

/// Per-producer attestation stats within the current epoch
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProducerAttestationStats {
    /// Producer public key (hex)
    pub public_key: String,
    /// Number of unique minutes this producer was attested (bit set in presence_root)
    pub attested_minutes: u32,
    /// Total minutes elapsed in epoch so far
    pub total_minutes: u32,
    /// Qualification threshold (54 minutes)
    pub threshold: u32,
    /// Whether this producer qualifies for epoch rewards
    pub qualified: bool,
    /// Whether this producer has a BLS key registered
    pub has_bls: bool,
}

// ==================== Mempool Transactions ====================

/// Mempool transaction entry
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MempoolTxResponse {
    /// Transaction hash
    pub hash: String,
    /// Transaction type
    pub tx_type: String,
    /// Size in bytes
    pub size: usize,
    /// Fee
    pub fee: u64,
    /// Fee rate (per byte)
    pub fee_rate: u64,
    /// Time added to mempool (unix timestamp)
    pub added_time: u64,
}

/// Parameters for getMempoolTransactions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMempoolTxsParams {
    /// Maximum number of transactions to return (default 100)
    #[serde(default = "default_mempool_limit")]
    pub limit: usize,
}

fn default_mempool_limit() -> usize {
    100
}

// ==================== Backfill ====================

/// Parameters for backfillFromPeer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackfillParams {
    /// RPC URL of a peer with complete block history
    pub rpc_url: String,
}

/// Response for backfillStatus
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackfillStatusResponse {
    /// Whether a backfill is currently running
    pub running: bool,
    /// Number of blocks imported so far
    pub imported: u64,
    /// Total number of blocks to import
    pub total: u64,
    /// Progress percentage (0-100)
    pub pct: u64,
    /// Error message if backfill failed
    pub error: Option<String>,
}
