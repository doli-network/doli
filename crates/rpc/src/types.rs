//! JSON-RPC request and response types

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crypto::Hash;
use doli_core::{Amount, Block, BlockHeader, Transaction};

use crate::error::RpcError;

/// JSON-RPC 2.0 request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC version (must be "2.0")
    pub jsonrpc: String,
    /// Method name
    pub method: String,
    /// Method parameters
    #[serde(default)]
    pub params: Value,
    /// Request ID
    pub id: Value,
}

/// JSON-RPC 2.0 response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC version
    pub jsonrpc: String,
    /// Result (if successful)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Error (if failed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
    /// Request ID
    pub id: Value,
}

impl JsonRpcResponse {
    /// Create a successful response
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    /// Create an error response
    pub fn error(id: Value, error: RpcError) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(error),
            id,
        }
    }
}

// ==================== Response Types ====================

/// Presence commitment response (for block responses)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceResponse {
    /// Number of present producers
    pub present_count: usize,
    /// Total weight of all present producers
    pub total_weight: u64,
    /// Merkle root of heartbeat data (hex)
    pub merkle_root: String,
}

/// Block response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockResponse {
    /// Block hash
    pub hash: String,
    /// Previous block hash
    pub prev_hash: String,
    /// Block height
    pub height: u64,
    /// Slot number
    pub slot: u32,
    /// Timestamp
    pub timestamp: u64,
    /// Producer public key (hex)
    pub producer: String,
    /// Merkle root
    pub merkle_root: String,
    /// Number of transactions
    pub tx_count: usize,
    /// Transaction hashes
    pub transactions: Vec<String>,
    /// Block size in bytes
    pub size: usize,
    /// Presence commitment (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence: Option<PresenceResponse>,
}

impl From<&Block> for BlockResponse {
    fn from(block: &Block) -> Self {
        // NOTE: Presence removed in deterministic scheduler model
        // Blocks no longer contain presence commitments

        Self {
            hash: block.hash().to_hex(),
            prev_hash: block.header.prev_hash.to_hex(),
            height: 0, // Must be set externally
            slot: block.header.slot,
            timestamp: block.header.timestamp,
            producer: hex::encode(block.header.producer.as_bytes()),
            merkle_root: block.header.merkle_root.to_hex(),
            tx_count: block.transactions.len(),
            transactions: block
                .transactions
                .iter()
                .map(|tx| tx.hash().to_hex())
                .collect(),
            size: block.size(),
            presence: None, // Deprecated in deterministic scheduler model
        }
    }
}

/// Transaction response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionResponse {
    /// Transaction hash
    pub hash: String,
    /// Version
    pub version: u32,
    /// Transaction type
    pub tx_type: String,
    /// Inputs
    pub inputs: Vec<InputResponse>,
    /// Outputs
    pub outputs: Vec<OutputResponse>,
    /// Transaction size in bytes
    pub size: usize,
    /// Fee (if known)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee: Option<u64>,
    /// Block hash (if confirmed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_hash: Option<String>,
    /// Confirmations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmations: Option<u64>,
}

impl From<&Transaction> for TransactionResponse {
    fn from(tx: &Transaction) -> Self {
        let tx_type = match tx.tx_type {
            doli_core::TxType::Transfer => "transfer",
            doli_core::TxType::Registration => "registration",
            doli_core::TxType::Exit => "exit",
            doli_core::TxType::ClaimReward => "claim_reward",
            doli_core::TxType::ClaimBond => "claim_bond",
            doli_core::TxType::SlashProducer => "slash_producer",
            doli_core::TxType::Coinbase => "coinbase",
            doli_core::TxType::AddBond => "add_bond",
            doli_core::TxType::RequestWithdrawal => "request_withdrawal",
            doli_core::TxType::ClaimWithdrawal => "claim_withdrawal",
            doli_core::TxType::EpochReward => "epoch_reward",
        };

        Self {
            hash: tx.hash().to_hex(),
            version: tx.version,
            tx_type: tx_type.to_string(),
            inputs: tx.inputs.iter().map(|i| InputResponse::from(i)).collect(),
            outputs: tx.outputs.iter().map(|o| OutputResponse::from(o)).collect(),
            size: tx.size(),
            fee: None,
            block_hash: None,
            confirmations: None,
        }
    }
}

/// Input response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputResponse {
    /// Previous transaction hash
    pub prev_tx_hash: String,
    /// Output index
    pub output_index: u32,
    /// Signature (hex)
    pub signature: String,
}

impl From<&doli_core::Input> for InputResponse {
    fn from(input: &doli_core::Input) -> Self {
        Self {
            prev_tx_hash: input.prev_tx_hash.to_hex(),
            output_index: input.output_index,
            signature: hex::encode(input.signature.as_bytes()),
        }
    }
}

/// Output response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputResponse {
    /// Output type
    pub output_type: String,
    /// Amount in base units
    pub amount: u64,
    /// Public key hash
    pub pubkey_hash: String,
    /// Lock height (for bonds)
    pub lock_until: u64,
}

impl From<&doli_core::Output> for OutputResponse {
    fn from(output: &doli_core::Output) -> Self {
        let output_type = match output.output_type {
            doli_core::OutputType::Normal => "normal",
            doli_core::OutputType::Bond => "bond",
        };

        Self {
            output_type: output_type.to_string(),
            amount: output.amount,
            pubkey_hash: output.pubkey_hash.to_hex(),
            lock_until: output.lock_until,
        }
    }
}

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
}

/// Chain info response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainInfoResponse {
    /// Network name
    pub network: String,
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
    /// Total balance (confirmed + unconfirmed + immature)
    pub total: u64,
}

// ==================== Request Parameters ====================

/// Parameters for getBlockByHash
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBlockByHashParams {
    /// Block hash (hex)
    pub hash: String,
}

/// Parameters for getBlockByHeight
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBlockByHeightParams {
    /// Block height
    pub height: u64,
}

/// Parameters for getTransaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTransactionParams {
    /// Transaction hash (hex)
    pub hash: String,
}

/// Parameters for sendTransaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendTransactionParams {
    /// Serialized transaction (hex)
    pub tx: String,
}

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

/// Parameters for getProducer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetProducerParams {
    /// Producer public key (hex)
    pub public_key: String,
}

/// Parameters for getProducers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetProducersParams {
    /// Only return active producers
    #[serde(default)]
    pub active_only: bool,
}

/// Vote message for governance veto system
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoteMessageParams {
    /// Version being voted on
    pub version: String,
    /// Vote type: "approve" or "veto"
    pub vote: String,
    /// Producer's public key (hex)
    pub producer_id: String,
    /// Unix timestamp of the vote
    pub timestamp: u64,
    /// Signature over "version:vote:timestamp" (hex)
    pub signature: String,
}

/// Parameters for submitVote
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitVoteParams {
    /// The signed vote message
    pub vote: VoteMessageParams,
}

/// Producer response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProducerResponse {
    /// Public key (hex)
    pub public_key: String,
    /// Registration height
    pub registration_height: u64,
    /// Bond amount (total)
    pub bond_amount: u64,
    /// Number of bonds staked
    pub bond_count: u32,
    /// Status (active, unbonding, exited, slashed)
    pub status: String,
    /// Current era
    pub era: u64,
    /// Pending withdrawals
    #[serde(default)]
    pub pending_withdrawals: Vec<PendingWithdrawalResponse>,
}

/// Pending withdrawal response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingWithdrawalResponse {
    /// Number of bonds being withdrawn
    pub bond_count: u32,
    /// Slot when withdrawal was requested
    pub request_slot: u32,
    /// Net amount after penalty
    pub net_amount: u64,
    /// Whether this withdrawal can be claimed now
    pub claimable: bool,
}

/// Response for getEpochInfo
#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_epoch_info_response_serialization() {
        let response = EpochInfoResponse {
            current_height: 1500,
            current_epoch: 4,
            last_complete_epoch: Some(3),
            blocks_per_epoch: 360,
            blocks_remaining: 60,
            epoch_start_height: 1440,
            epoch_end_height: 1800,
            block_reward: 5_000_000_000,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"blocksPerEpoch\":360"));
        assert!(json.contains("\"lastCompleteEpoch\":3"));

        let parsed: EpochInfoResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.current_epoch, 4);
        assert_eq!(parsed.blocks_remaining, 60);
    }
}
