//! Transaction history handler: getHistory

use serde_json::Value;

use crate::error::RpcError;
use crate::types::*;

use super::context::RpcContext;

impl RpcContext {
    /// Get transaction history for an address (uses addr_tx_index for O(1) lookup)
    pub(super) async fn get_history(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetHistoryParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let pubkey_hash = self.resolve_address(&params.address)?;

        let chain_state = self.chain_state.read().await;
        let best_height = chain_state.best_height;
        drop(chain_state);

        let limit = params.limit.min(100);
        let before_height = params
            .before_height
            .map(|h| h.min(best_height + 1))
            .unwrap_or(best_height + 1);

        // Use address index to find relevant block heights (descending)
        let heights = self
            .block_store
            .get_address_heights(&pubkey_hash, Some(before_height), limit * 2)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        // Build tx output cache for these blocks (for input resolution)
        let mut tx_output_cache: std::collections::HashMap<
            crypto::Hash,
            Vec<doli_core::transaction::Output>,
        > = std::collections::HashMap::new();

        for &height in &heights {
            if let Ok(Some(block)) = self.block_store.get_block_by_height(height) {
                for tx in &block.transactions {
                    tx_output_cache.insert(tx.hash(), tx.outputs.clone());
                    // Also cache parent tx outputs for input resolution
                    for input in &tx.inputs {
                        if !tx_output_cache.contains_key(&input.prev_tx_hash) {
                            if let Ok(Some(parent_height)) =
                                self.block_store.get_tx_block_height(&input.prev_tx_hash)
                            {
                                if let Ok(Some(parent_block)) =
                                    self.block_store.get_block_by_height(parent_height)
                                {
                                    for ptx in &parent_block.transactions {
                                        tx_output_cache.insert(ptx.hash(), ptx.outputs.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Build history entries from indexed blocks
        let mut history: Vec<HistoryEntryResponse> = Vec::new();

        for &height in &heights {
            if history.len() >= limit {
                break;
            }

            let block = match self.block_store.get_block_by_height(height) {
                Ok(Some(b)) => b,
                _ => continue,
            };

            let block_hash = block.hash();
            let timestamp = block.header.timestamp;
            let confirmations = best_height.saturating_sub(height) + 1;

            for tx in &block.transactions {
                let mut amount_received: u64 = 0;
                let mut amount_sent: u64 = 0;
                let mut total_input: u64 = 0;
                let mut is_relevant = false;

                for output in &tx.outputs {
                    if output.pubkey_hash == pubkey_hash && output.output_type.is_native_amount() {
                        amount_received += output.amount;
                        is_relevant = true;
                    } else if output.pubkey_hash == pubkey_hash {
                        is_relevant = true;
                    }
                }

                for input in &tx.inputs {
                    if let Some(prev_outputs) = tx_output_cache.get(&input.prev_tx_hash) {
                        if let Some(prev_output) = prev_outputs.get(input.output_index as usize) {
                            if prev_output.output_type.is_native_amount() {
                                total_input += prev_output.amount;
                            }
                            if prev_output.pubkey_hash == pubkey_hash
                                && prev_output.output_type.is_native_amount()
                            {
                                amount_sent += prev_output.amount;
                                is_relevant = true;
                            } else if prev_output.pubkey_hash == pubkey_hash {
                                is_relevant = true;
                            }
                        }
                    }
                }

                if !is_relevant {
                    continue;
                }

                let total_output: u64 = tx
                    .outputs
                    .iter()
                    .filter(|o| o.output_type.is_native_amount())
                    .map(|o| o.amount)
                    .sum();
                let fee = if total_input > 0 && total_input >= total_output {
                    total_input - total_output
                } else {
                    0
                };

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
                };

                // Resolve sender addresses from inputs
                let mut from_addrs: Vec<String> = Vec::new();
                for input in &tx.inputs {
                    if let Some(prev_outputs) = tx_output_cache.get(&input.prev_tx_hash) {
                        if let Some(prev_output) = prev_outputs.get(input.output_index as usize) {
                            if let Ok(addr) =
                                crypto::address::encode(&prev_output.pubkey_hash, "doli")
                            {
                                if !from_addrs.contains(&addr) {
                                    from_addrs.push(addr);
                                }
                            }
                        }
                    }
                }

                // Collect recipient addresses from outputs (unique, excluding sender)
                let mut to_addrs: Vec<String> = Vec::new();
                for output in &tx.outputs {
                    if let Ok(addr) = crypto::address::encode(&output.pubkey_hash, "doli") {
                        if !to_addrs.contains(&addr) && !from_addrs.contains(&addr) {
                            to_addrs.push(addr);
                        }
                    }
                }

                history.push(HistoryEntryResponse {
                    hash: tx.hash().to_hex(),
                    tx_type: tx_type.to_string(),
                    block_hash: block_hash.to_hex(),
                    height,
                    timestamp,
                    amount_received,
                    amount_sent,
                    fee,
                    confirmations,
                    from: from_addrs,
                    to: to_addrs,
                });

                if history.len() >= limit {
                    break;
                }
            }
        }

        serde_json::to_value(history).map_err(|e| RpcError::internal_error(e.to_string()))
    }
}
