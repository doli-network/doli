//! Balance and UTXO query handlers: getBalance, getUtxos

use serde_json::Value;

use crate::error::RpcError;
use crate::types::*;

use super::context::RpcContext;

impl RpcContext {
    /// Get balance for address
    pub(super) async fn get_balance(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetBalanceParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let pubkey_hash = self.resolve_address(&params.address)?;

        let utxo_set = self.utxo_set.read().await;
        let chain_state = self.chain_state.read().await;
        let mempool = self.mempool.read().await;

        // Use network-specific coinbase maturity
        let maturity = self.coinbase_maturity;
        let confirmed =
            utxo_set.get_balance_with_maturity(&pubkey_hash, chain_state.best_height, maturity);
        let immature = utxo_set.get_immature_balance_with_maturity(
            &pubkey_hash,
            chain_state.best_height,
            maturity,
        );
        let bonded = utxo_set.get_bonded_balance(&pubkey_hash);

        // Get incoming (change outputs) and outgoing (spent UTXOs) from mempool
        let (incoming, outgoing) = mempool.calculate_unconfirmed_balance(&pubkey_hash, &utxo_set);

        // Spendable = confirmed minus mempool-spent UTXOs (matches getUtxos filtering)
        let spendable = confirmed.saturating_sub(outgoing);

        // Total = all assets: spendable + pending change + immature rewards + bonded
        let total = spendable
            .saturating_add(incoming)
            .saturating_add(immature)
            .saturating_add(bonded);

        let response = BalanceResponse {
            confirmed: spendable,
            unconfirmed: incoming,
            immature,
            bonded,
            total,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get UTXOs for address
    pub(super) async fn get_utxos(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetUtxosParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let pubkey_hash = self.resolve_address(&params.address)?;

        let utxo_set = self.utxo_set.read().await;
        let chain_state = self.chain_state.read().await;
        let current_height = chain_state.best_height;
        let mempool = self.mempool.read().await;

        let utxos = utxo_set.get_by_pubkey_hash(&pubkey_hash);
        let maturity = self.coinbase_maturity;

        let mut responses: Vec<UtxoResponse> = utxos
            .into_iter()
            .filter(|(outpoint, entry)| {
                // Exclude UTXOs being spent by mempool transactions
                if mempool.is_outpoint_spent(outpoint) {
                    return false;
                }
                !params.spendable_only
                    || entry.is_spendable_at_with_maturity(current_height, maturity)
            })
            .map(|(outpoint, entry)| {
                let output_type = match entry.output.output_type {
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
                };

                let condition = if entry.output.output_type.is_conditioned() {
                    entry
                        .output
                        .condition()
                        .and_then(|r| r.ok())
                        .map(|c| condition_to_json(&c))
                } else {
                    None
                };

                let nft = if entry.output.output_type == doli_core::OutputType::NFT {
                    entry.output.nft_metadata().map(|(token_id, content_hash)| {
                        let mut nft_json = serde_json::json!({
                            "tokenId": token_id.to_hex(),
                            "contentHash": hex::encode(&content_hash)
                        });
                        if let Some((creator_hash, royalty_bps)) = entry.output.nft_royalty() {
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

                let asset = if entry.output.output_type == doli_core::OutputType::FungibleAsset {
                    entry.output.fungible_asset_metadata().map(
                        |(asset_id, total_supply, ticker)| {
                            serde_json::json!({
                                "assetId": asset_id.to_hex(),
                                "totalSupply": total_supply,
                                "ticker": ticker
                            })
                        },
                    )
                } else {
                    None
                };

                let bridge = if entry.output.output_type == doli_core::OutputType::BridgeHTLC {
                    entry.output.bridge_htlc_metadata().map(
                        |(chain_id, target_addr, counter_hash)| {
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
                        },
                    )
                } else {
                    None
                };

                UtxoResponse {
                    tx_hash: outpoint.tx_hash.to_hex(),
                    output_index: outpoint.index,
                    amount: entry.output.amount,
                    output_type: output_type.to_string(),
                    lock_until: entry.output.lock_until,
                    height: entry.height,
                    spendable: entry.is_spendable_at_with_maturity(current_height, maturity),
                    pending: false,
                    condition,
                    nft,
                    asset,
                    bridge,
                }
            })
            .collect();

        // Add pending outputs from mempool transactions owned by this address.
        // This enables chained transactions: spend change from a pending TX
        // without waiting for confirmation.
        for (_tx_hash, entry) in mempool.iter() {
            let tx = &entry.tx;
            let tx_hash_hex = tx.hash().to_hex();
            for (idx, output) in tx.outputs.iter().enumerate() {
                if output.pubkey_hash == pubkey_hash
                    && output.output_type == doli_core::OutputType::Normal
                {
                    // Skip if this output is already being spent by another mempool TX
                    let outpoint = storage::Outpoint::new(tx.hash(), idx as u32);
                    if mempool.is_outpoint_spent(&outpoint) {
                        continue;
                    }
                    responses.push(UtxoResponse {
                        tx_hash: tx_hash_hex.clone(),
                        output_index: idx as u32,
                        amount: output.amount,
                        output_type: "normal".to_string(),
                        lock_until: 0,
                        height: 0,
                        spendable: true,
                        pending: true,
                        condition: None,
                        nft: None,
                        asset: None,
                        bridge: None,
                    });
                }
            }
        }

        serde_json::to_value(responses).map_err(|e| RpcError::internal_error(e.to_string()))
    }
}
