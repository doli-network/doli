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
}

impl From<&Block> for BlockResponse {
    fn from(block: &Block) -> Self {
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
            doli_core::TxType::ClaimEpochReward => "claim_epoch_reward",
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
    /// Total balance
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

// ==================== Reward/Claim Types ====================

/// Parameters for getClaimableRewards
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetClaimableRewardsParams {
    /// Producer public key (hex)
    pub producer: String,
}

/// A single claimable epoch entry
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Parameters for getClaimHistory
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetClaimHistoryParams {
    /// Producer public key (hex)
    pub producer: String,
    /// Maximum number of entries to return
    #[serde(default = "default_claim_history_limit")]
    pub limit: usize,
}

fn default_claim_history_limit() -> usize {
    50
}

/// A single claim history entry
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Parameters for estimateEpochReward
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EstimateEpochRewardParams {
    /// Producer public key (hex)
    pub producer: String,
    /// Epoch number to estimate
    pub epoch: u64,
}

/// Response for estimateEpochReward
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Parameters for buildClaimTx
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildClaimTxParams {
    /// Producer public key (hex)
    pub producer: String,
    /// Epoch to claim
    pub epoch: u64,
    /// Recipient address (hex) - defaults to producer's address if not specified
    pub recipient: Option<String>,
}

/// Response for buildClaimTx
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    fn test_claimable_rewards_response_serialization() {
        let response = ClaimableRewardsResponse {
            producer: "abc123".to_string(),
            current_height: 1000,
            current_epoch: 2,
            claimable_count: 2,
            total_estimated_reward: 100_000_000,
            epochs: vec![
                ClaimableEpochEntry {
                    epoch: 0,
                    blocks_present: 350,
                    total_blocks: 360,
                    presence_rate: 97,
                    estimated_reward: 50_000_000,
                },
                ClaimableEpochEntry {
                    epoch: 1,
                    blocks_present: 360,
                    total_blocks: 360,
                    presence_rate: 100,
                    estimated_reward: 50_000_000,
                },
            ],
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"currentHeight\":1000"));
        assert!(json.contains("\"currentEpoch\":2"));
        assert!(json.contains("\"claimableCount\":2"));
        assert!(json.contains("\"presenceRate\":97"));

        let parsed: ClaimableRewardsResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.claimable_count, 2);
        assert_eq!(parsed.epochs.len(), 2);
    }

    #[test]
    fn test_claim_history_response_serialization() {
        let response = ClaimHistoryResponse {
            producer: "abc123".to_string(),
            total_claims: 1,
            total_claimed: 50_000_000,
            claims: vec![ClaimHistoryEntry {
                epoch: 5,
                tx_hash: "deadbeef".to_string(),
                height: 2200,
                amount: 50_000_000,
                timestamp: 1234567890,
            }],
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"totalClaims\":1"));
        assert!(json.contains("\"txHash\":\"deadbeef\""));

        let parsed: ClaimHistoryResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.claims.len(), 1);
        assert_eq!(parsed.claims[0].epoch, 5);
    }

    #[test]
    fn test_reward_estimate_response_serialization() {
        let response = RewardEstimateResponse {
            producer: "abc123".to_string(),
            epoch: 10,
            is_complete: true,
            is_claimed: false,
            blocks_present: 340,
            total_blocks: 360,
            presence_rate: 94,
            block_reward: 5_000_000_000,
            estimated_reward: 4_722_222_222,
            average_weight: 1000,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"isComplete\":true"));
        assert!(json.contains("\"isClaimed\":false"));
        assert!(json.contains("\"presenceRate\":94"));

        let parsed: RewardEstimateResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.epoch, 10);
        assert!(parsed.is_complete);
        assert!(!parsed.is_claimed);
    }

    #[test]
    fn test_build_claim_tx_response_serialization() {
        let response = BuildClaimTxResponse {
            epoch: 5,
            amount: 50_000_000,
            recipient: "abc123".to_string(),
            unsigned_tx: "0102030405".to_string(),
            signing_message: "deadbeef".to_string(),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"unsignedTx\":\"0102030405\""));
        assert!(json.contains("\"signingMessage\":\"deadbeef\""));

        let parsed: BuildClaimTxResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.epoch, 5);
        assert_eq!(parsed.unsigned_tx, "0102030405");
    }

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

    #[test]
    fn test_get_claimable_rewards_params() {
        let json = r#"{"producer": "abc123"}"#;
        let params: GetClaimableRewardsParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.producer, "abc123");
    }

    #[test]
    fn test_build_claim_tx_params_optional_recipient() {
        // Without recipient
        let json = r#"{"producer": "abc123", "epoch": 5}"#;
        let params: BuildClaimTxParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.producer, "abc123");
        assert_eq!(params.epoch, 5);
        assert!(params.recipient.is_none());

        // With recipient
        let json = r#"{"producer": "abc123", "epoch": 5, "recipient": "def456"}"#;
        let params: BuildClaimTxParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.recipient, Some("def456".to_string()));
    }
}
