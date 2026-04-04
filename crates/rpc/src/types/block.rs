//! Block and transaction response types, including input/output details
//! and covenant condition rendering.

use serde::{Deserialize, Serialize};

use doli_core::{Block, Transaction};

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
    /// Covenant witnesses (one per input, for conditioned output spends)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub covenant_witnesses: Vec<serde_json::Value>,
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
            doli_core::TxType::MintAsset => "mint_asset",
            doli_core::TxType::EpochReward => "epoch_reward",
            doli_core::TxType::RemoveMaintainer => "remove_maintainer",
            doli_core::TxType::AddMaintainer => "add_maintainer",
            doli_core::TxType::DelegateBond => "delegate_bond",
            doli_core::TxType::RevokeDelegation => "revoke_delegation",
            doli_core::TxType::ProtocolActivation => "protocol_activation",
            doli_core::TxType::BurnAsset => "burn_asset",
            doli_core::TxType::CreatePool => "create_pool",
            doli_core::TxType::AddLiquidity => "add_liquidity",
            doli_core::TxType::RemoveLiquidity => "remove_liquidity",
            doli_core::TxType::Swap => "swap",
            doli_core::TxType::CreateLoan => "create_loan",
            doli_core::TxType::RepayLoan => "repay_loan",
            doli_core::TxType::LiquidateLoan => "liquidate_loan",
            doli_core::TxType::LendingDeposit => "lending_deposit",
            doli_core::TxType::LendingWithdraw => "lending_withdraw",
            doli_core::TxType::FractionalizeNft => "fractionalize_nft",
            doli_core::TxType::RedeemNft => "redeem_nft",
        };

        // Decode covenant witnesses from tx.extra_data (if present)
        let covenant_witnesses = (0..tx.inputs.len())
            .filter_map(|i| {
                let witness_bytes = tx.get_covenant_witness(i)?;
                if witness_bytes.is_empty() {
                    return Some(serde_json::json!(null));
                }
                let witness = doli_core::conditions::Witness::decode(witness_bytes).ok()?;
                let mut obj = serde_json::Map::new();
                if let Some(preimage) = &witness.preimage {
                    obj.insert(
                        "preimage".to_string(),
                        serde_json::Value::String(hex::encode(preimage)),
                    );
                }
                if !witness.or_branches.is_empty() {
                    obj.insert(
                        "branches".to_string(),
                        serde_json::Value::Array(
                            witness
                                .or_branches
                                .iter()
                                .map(|&b| {
                                    serde_json::Value::String(
                                        if b { "right" } else { "left" }.to_string(),
                                    )
                                })
                                .collect(),
                        ),
                    );
                }
                if !witness.signatures.is_empty() {
                    obj.insert(
                        "signatures".to_string(),
                        serde_json::Value::Array(
                            witness
                                .signatures
                                .iter()
                                .map(|ws| {
                                    serde_json::json!({
                                        "pubkey": hex::encode(ws.pubkey.as_bytes()),
                                        "signature": hex::encode(ws.signature.as_bytes()),
                                    })
                                })
                                .collect(),
                        ),
                    );
                }
                Some(serde_json::Value::Object(obj))
            })
            .collect::<Vec<_>>();

        Self {
            hash: tx.hash().to_hex(),
            version: tx.version,
            tx_type: tx_type.to_string(),
            inputs: tx.inputs.iter().map(InputResponse::from).collect(),
            outputs: tx.outputs.iter().map(OutputResponse::from).collect(),
            covenant_witnesses,
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
    /// Resolved sender address (bech32m, populated when input can be resolved)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// Amount of the spent output (populated when input can be resolved)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<u64>,
}

impl From<&doli_core::Input> for InputResponse {
    fn from(input: &doli_core::Input) -> Self {
        Self {
            prev_tx_hash: input.prev_tx_hash.to_hex(),
            output_index: input.output_index,
            signature: hex::encode(input.signature.as_bytes()),
            address: None,
            amount: None,
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
    /// Public key hash (hex)
    pub pubkey_hash: String,
    /// Human-readable address (bech32m)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
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
    /// Size of extra_data in bytes (useful for debugging/inspection)
    pub extra_data_size: usize,
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
            doli_core::OutputType::Pool => "pool",
            doli_core::OutputType::LPShare => "lpShare",
            doli_core::OutputType::Collateral => "collateral",
            doli_core::OutputType::LendingDeposit => "lendingDeposit",
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
                .map(|(chain_id, target_addr, counter_hash)| {
                    let mut obj = serde_json::json!({
                        "targetChain": doli_core::Output::bridge_chain_name(chain_id),
                        "targetChainId": chain_id,
                        "targetAddress": String::from_utf8(target_addr.clone())
                            .unwrap_or_else(|_| hex::encode(&target_addr))
                    });
                    if let Some(ch) = counter_hash {
                        obj["counterHash"] = serde_json::Value::String(ch.to_hex());
                    }
                    obj
                })
        } else {
            None
        };

        // Encode bech32m address from pubkey_hash (mainnet "doli" prefix)
        let address = crypto::address::encode(&output.pubkey_hash, "doli").ok();

        Self {
            output_type: output_type.to_string(),
            amount: output.amount,
            pubkey_hash: output.pubkey_hash.to_hex(),
            address,
            lock_until: output.lock_until,
            condition,
            nft: nft_metadata,
            asset: asset_metadata,
            bridge: bridge_metadata,
            extra_data_size: output.extra_data.len(),
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
        doli_core::Condition::AmountGuard {
            min_amount,
            output_index,
        } => json!({
            "type": "amountGuard",
            "minAmount": min_amount,
            "outputIndex": output_index
        }),
        doli_core::Condition::OutputTypeGuard {
            expected_type,
            output_index,
        } => json!({
            "type": "outputTypeGuard",
            "expectedType": format!("{:?}", expected_type),
            "outputIndex": output_index
        }),
        doli_core::Condition::RecipientGuard {
            expected_pubkey_hash,
            output_index,
        } => json!({
            "type": "recipientGuard",
            "expectedPubkeyHash": expected_pubkey_hash.to_hex(),
            "outputIndex": output_index
        }),
    }
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
