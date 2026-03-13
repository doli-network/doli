//! JSON-RPC request and response types

use serde::{Deserialize, Serialize};
use serde_json::Value;

use doli_core::{Block, Transaction};

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
    /// Attestation bitfield from presence_root (hex, 64 chars = 32 bytes).
    /// Bit N = producer at sorted index N attested this minute.
    /// "0000...0000" means no attestations committed in this block.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_root: Option<String>,
    /// BLS aggregate attestation signature (hex, 192 chars = 96 bytes).
    /// Present when block producer aggregated BLS sigs from attesting producers.
    /// Null/absent for pre-BLS blocks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregate_bls_sig: Option<String>,
    /// Number of attestation bits set in the presence_root bitfield.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attestation_count: Option<u32>,
}

impl From<&Block> for BlockResponse {
    fn from(block: &Block) -> Self {
        let pr_hex = block.header.presence_root.to_hex();
        let pr_is_zero = block.header.presence_root == crypto::Hash::ZERO;

        // Count set bits in presence_root
        let att_count = if pr_is_zero {
            0u32
        } else {
            block
                .header
                .presence_root
                .as_bytes()
                .iter()
                .map(|b| b.count_ones())
                .sum()
        };

        let agg_sig = if block.aggregate_bls_signature.is_empty() {
            None
        } else {
            Some(hex::encode(&block.aggregate_bls_signature))
        };

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
            presence: None,
            presence_root: if pr_is_zero { None } else { Some(pr_hex) },
            aggregate_bls_sig: agg_sig,
            attestation_count: if att_count > 0 { Some(att_count) } else { None },
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
    /// Block height (if confirmed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_height: Option<u64>,
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
            doli_core::TxType::RemoveMaintainer => "remove_maintainer",
            doli_core::TxType::AddMaintainer => "add_maintainer",
            doli_core::TxType::DelegateBond => "delegate_bond",
            doli_core::TxType::RevokeDelegation => "revoke_delegation",
            doli_core::TxType::ProtocolActivation => "protocol_activation",
        };

        Self {
            hash: tx.hash().to_hex(),
            version: tx.version,
            tx_type: tx_type.to_string(),
            inputs: tx.inputs.iter().map(InputResponse::from).collect(),
            outputs: tx.outputs.iter().map(OutputResponse::from).collect(),
            size: tx.size(),
            fee: None,
            block_hash: None,
            block_height: None,
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

impl From<&doli_core::Output> for OutputResponse {
    fn from(output: &doli_core::Output) -> Self {
        let output_type = match output.output_type {
            doli_core::OutputType::Normal => "normal",
            doli_core::OutputType::Bond => "bond",
            doli_core::OutputType::Multisig => "multisig",
            doli_core::OutputType::Hashlock => "hashlock",
            doli_core::OutputType::HTLC => "htlc",
            doli_core::OutputType::Vesting => "vesting",
            doli_core::OutputType::NFT => "nft",
            doli_core::OutputType::FungibleAsset => "fungibleAsset",
            doli_core::OutputType::BridgeHTLC => "bridgeHtlc",
        };

        // Decode covenant condition for conditioned outputs
        let condition = if output.output_type.is_conditioned() {
            output
                .condition()
                .and_then(|r| r.ok())
                .map(|c| condition_to_json(&c))
        } else {
            None
        };

        // Decode NFT metadata if present
        let nft_metadata = if output.output_type == doli_core::OutputType::NFT {
            output.nft_metadata().map(|(token_id, content_hash)| {
                let mut nft_json = serde_json::json!({
                    "tokenId": token_id.to_hex(),
                    "contentHash": hex::encode(&content_hash)
                });
                if let Some((creator_hash, royalty_bps)) = output.nft_royalty() {
                    nft_json["royalty"] = serde_json::json!({
                        "creator": creator_hash.to_hex(),
                        "bps": royalty_bps,
                        "percent": format!("{:.2}", royalty_bps as f64 / 100.0)
                    });
                }
                nft_json
            })
        } else {
            None
        };

        // Decode fungible asset metadata if present
        let asset_metadata = if output.output_type == doli_core::OutputType::FungibleAsset {
            output
                .fungible_asset_metadata()
                .map(|(asset_id, total_supply, ticker)| {
                    serde_json::json!({
                        "assetId": asset_id.to_hex(),
                        "totalSupply": total_supply,
                        "ticker": ticker
                    })
                })
        } else {
            None
        };

        // Decode bridge HTLC metadata if present
        let bridge_metadata = if output.output_type == doli_core::OutputType::BridgeHTLC {
            output
                .bridge_htlc_metadata()
                .map(|(chain_id, target_addr)| {
                    serde_json::json!({
                        "targetChain": doli_core::Output::bridge_chain_name(chain_id),
                        "targetChainId": chain_id,
                        "targetAddress": String::from_utf8(target_addr.clone())
                            .unwrap_or_else(|_| hex::encode(&target_addr))
                    })
                })
        } else {
            None
        };

        Self {
            output_type: output_type.to_string(),
            amount: output.amount,
            pubkey_hash: output.pubkey_hash.to_hex(),
            lock_until: output.lock_until,
            condition,
            nft: nft_metadata,
            asset: asset_metadata,
            bridge: bridge_metadata,
        }
    }
}

/// Convert a Condition to a human-readable JSON value.
pub fn condition_to_json(cond: &doli_core::Condition) -> serde_json::Value {
    use serde_json::json;
    match cond {
        doli_core::Condition::Signature(pkh) => json!({
            "type": "signature",
            "pubkeyHash": pkh.to_hex()
        }),
        doli_core::Condition::Multisig { threshold, keys } => json!({
            "type": "multisig",
            "threshold": threshold,
            "keys": keys.iter().map(|k| k.to_hex()).collect::<Vec<_>>()
        }),
        doli_core::Condition::Hashlock(h) => json!({
            "type": "hashlock",
            "hash": h.to_hex()
        }),
        doli_core::Condition::Timelock(h) => json!({
            "type": "timelock",
            "minHeight": h
        }),
        doli_core::Condition::TimelockExpiry(h) => json!({
            "type": "timelockExpiry",
            "maxHeight": h
        }),
        doli_core::Condition::And(a, b) => json!({
            "type": "and",
            "left": condition_to_json(a),
            "right": condition_to_json(b)
        }),
        doli_core::Condition::Or(a, b) => json!({
            "type": "or",
            "left": condition_to_json(a),
            "right": condition_to_json(b)
        }),
        doli_core::Condition::Threshold { n, conditions } => json!({
            "type": "threshold",
            "n": n,
            "conditions": conditions.iter().map(condition_to_json).collect::<Vec<_>>()
        }),
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

/// Parameters for getBlockRaw
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBlockRawParams {
    /// Block height
    pub height: u64,
}

/// Response for getBlockRaw — bincode-serialized block as base64
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockRawResponse {
    /// Base64-encoded bincode(Block)
    pub block: String,
    /// BLAKE3 checksum of the raw bincode bytes (hex)
    pub blake3: String,
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

/// Parameters for getProducer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetProducerParams {
    /// Producer public key (hex)
    #[serde(alias = "publicKey")]
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
    /// Address hash (hex) — BLAKE3(ADDRESS_DOMAIN || pubkey), used for balance lookups
    pub address_hash: String,
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
    /// Pending epoch-deferred updates
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_updates: Vec<PendingUpdateInfo>,
    /// BLS12-381 public key for aggregate attestation (hex, 96 chars = 48 bytes).
    /// Empty string if producer registered before BLS was available.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bls_pubkey: String,
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

/// Pending producer update info (epoch-deferred)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingUpdateInfo {
    /// Update type (register, exit, add_bond, withdrawal, etc.)
    pub update_type: String,
    /// Bond count affected (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bond_count: Option<u32>,
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

/// Parameters for getBondDetails
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBondDetailsParams {
    /// Producer public key (hex)
    #[serde(alias = "publicKey")]
    pub public_key: String,
}

/// Response for getBondDetails
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BondDetailsResponse {
    /// Producer public key (hex)
    pub public_key: String,
    /// Total bond count
    pub bond_count: u32,
    /// Total staked amount (base units)
    pub total_staked: u64,
    /// Registration slot (proxy for bond creation)
    pub registration_slot: u64,
    /// Age in slots since oldest bond
    pub age_slots: u64,
    /// Penalty percentage for oldest bond (0-75)
    pub penalty_pct: u8,
    /// Whether all bonds are fully vested
    pub vested: bool,
    /// Slot when newest bond becomes fully vested (0% penalty)
    pub maturation_slot: u64,
    /// Vesting quarter duration in slots
    pub vesting_quarter_slots: u64,
    /// Full vesting period in slots
    pub vesting_period_slots: u64,
    /// Summary by vesting quarter
    pub summary: BondsSummaryResponse,
    /// Per-bond details (sorted oldest first)
    #[serde(default)]
    pub bonds: Vec<BondEntryResponse>,
    /// Bonds pending withdrawal this epoch
    #[serde(default)]
    pub withdrawal_pending_count: u32,
}

/// Individual bond entry response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BondEntryResponse {
    /// Slot when this bond was created
    pub creation_slot: u32,
    /// Amount staked (base units)
    pub amount: u64,
    /// Age in slots
    pub age_slots: u64,
    /// Current penalty percentage (0-75)
    pub penalty_pct: u8,
    /// Whether this bond is fully vested
    pub vested: bool,
    /// Slot when this bond becomes fully vested
    pub maturation_slot: u64,
}

/// Bond summary by vesting quarter
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BondsSummaryResponse {
    /// Bonds in Q1 (0-6h, 75% penalty)
    pub q1: u32,
    /// Bonds in Q2 (6-12h, 50% penalty)
    pub q2: u32,
    /// Bonds in Q3 (12-18h, 25% penalty)
    pub q3: u32,
    /// Fully vested bonds (18h+, 0% penalty)
    pub vested: u32,
}

/// Parameters for getSlotSchedule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSlotScheduleParams {
    /// Starting slot (default = current slot)
    #[serde(default, alias = "fromSlot")]
    pub from_slot: Option<u32>,
    /// Number of slots to return (default = 20, max 360)
    #[serde(default, alias = "count")]
    pub count: Option<u32>,
}

/// Slot schedule entry
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotScheduleEntry {
    /// Slot number
    pub slot: u32,
    /// Producer public key (hex)
    pub producer: String,
    /// Rank (0 = primary producer)
    pub rank: usize,
}

/// Response for getSlotSchedule
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotScheduleResponse {
    /// Scheduled slots
    pub slots: Vec<SlotScheduleEntry>,
    /// Current slot
    pub current_slot: u32,
    /// Current epoch
    pub epoch: u64,
    /// Slots remaining in current epoch
    pub slots_remaining_in_epoch: u64,
    /// Total bonds across all producers
    pub total_bonds: u64,
    /// Slot duration in seconds
    pub slot_duration: u64,
    /// Genesis time (unix timestamp)
    pub genesis_time: u64,
}

/// Parameters for getProducerSchedule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetProducerScheduleParams {
    /// Producer public key (hex)
    #[serde(alias = "publicKey")]
    pub public_key: String,
}

/// Response for getProducerSchedule
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProducerScheduleResponse {
    /// Producer public key (hex)
    pub public_key: String,
    /// Current slot
    pub current_slot: u32,
    /// Current epoch
    pub epoch: u64,
    /// Next slot where this producer is primary
    pub next_slot: Option<u32>,
    /// Seconds until next assigned slot
    pub seconds_until_next: Option<u64>,
    /// All slots this epoch where producer is primary
    pub slots_this_epoch: Vec<u32>,
    /// Number of assigned slots this epoch
    pub assigned_count: u32,
    /// Number of blocks actually produced this epoch
    pub produced_count: u32,
    /// Fill rate (produced / assigned)
    pub fill_rate: f64,
    /// Producer's bond count
    pub bond_count: u32,
    /// Total network bonds
    pub total_network_bonds: u64,
    /// Estimated weekly earnings (base units)
    pub weekly_earnings: u64,
    /// Weeks until bond investment doubles from rewards
    pub doubling_weeks: f64,
    /// Current block reward (base units)
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

    #[test]
    fn test_chain_info_response_serialization() {
        let response = ChainInfoResponse {
            network: "mainnet".to_string(),
            version: "1.1.11".to_string(),
            best_hash: "abc123".to_string(),
            best_height: 50_000,
            best_slot: 50_100,
            genesis_hash: "genesis".to_string(),
        };

        let json = serde_json::to_string(&response).unwrap();
        let parsed: ChainInfoResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.network, "mainnet");
        assert_eq!(parsed.best_height, 50_000);
        assert_eq!(parsed.best_slot, 50_100);
    }

    #[test]
    fn test_balance_response_serialization() {
        let response = BalanceResponse {
            confirmed: 1_000_000,
            unconfirmed: 500,
            immature: 0,
            bonded: 0,
            total: 1_000_500,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"confirmed\":1000000"));

        let parsed: BalanceResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.total, 1_000_500);
    }

    #[test]
    fn test_producer_response_with_withdrawals() {
        let response = ProducerResponse {
            public_key: "aabbcc".to_string(),
            address_hash: "ddeeff".to_string(),
            registration_height: 100,
            bond_amount: 10_000_000_000,
            bond_count: 1,
            status: "unbonding".to_string(),
            era: 0,
            pending_withdrawals: vec![PendingWithdrawalResponse {
                bond_count: 1,
                request_slot: 5000,
                net_amount: 10_000_000_000,
                claimable: false,
            }],
            pending_updates: Vec::new(),
            bls_pubkey: String::new(),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"pendingWithdrawals\""));
        assert!(json.contains("\"claimable\":false"));

        let parsed: ProducerResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.pending_withdrawals.len(), 1);
        assert!(!parsed.pending_withdrawals[0].claimable);
    }

    #[test]
    fn test_producer_response_empty_withdrawals() {
        let response = ProducerResponse {
            public_key: "aabbcc".to_string(),
            address_hash: "ddeeff".to_string(),
            registration_height: 100,
            bond_amount: 10_000_000_000,
            bond_count: 1,
            status: "active".to_string(),
            era: 0,
            pending_withdrawals: Vec::new(),
            pending_updates: Vec::new(),
            bls_pubkey: String::new(),
        };

        let json = serde_json::to_string(&response).unwrap();
        let parsed: ProducerResponse = serde_json::from_str(&json).unwrap();
        assert!(parsed.pending_withdrawals.is_empty());
    }

    #[test]
    fn test_mempool_info_response() {
        let response = MempoolInfoResponse {
            tx_count: 42,
            total_size: 1024,
            min_fee_rate: 1,
            max_size: 10_485_760,
            max_count: 5000,
        };

        let json = serde_json::to_string(&response).unwrap();
        let parsed: MempoolInfoResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tx_count, 42);
        assert_eq!(parsed.max_count, 5000);
    }

    #[test]
    fn test_json_rpc_response_success() {
        let resp =
            JsonRpcResponse::success(serde_json::json!(1), serde_json::json!({"height": 100}));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"height\":100"));
        assert!(!json.contains("error"));
    }

    #[test]
    fn test_json_rpc_response_error() {
        let err = crate::error::RpcError::invalid_params("bad param");
        let resp = JsonRpcResponse::error(serde_json::json!(1), err);
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"error\""));
        assert!(json.contains("bad param"));
    }

    #[test]
    fn test_history_entry_response() {
        let entry = HistoryEntryResponse {
            hash: "txhash123".to_string(),
            tx_type: "transfer".to_string(),
            block_hash: "blockhash".to_string(),
            height: 1000,
            timestamp: 1700000000,
            amount_received: 5000,
            amount_sent: 0,
            fee: 100,
            confirmations: 50,
        };

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: HistoryEntryResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.amount_received, 5000);
        assert_eq!(parsed.confirmations, 50);
    }
}

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
