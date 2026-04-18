//! Transaction handlers: getTransaction, sendTransaction

use serde_json::Value;

use crypto::Hash;
use doli_core::Transaction;

use crate::error::RpcError;
use crate::types::*;
use mempool::MempoolError;

use super::context::RpcContext;

impl RpcContext {
    /// Get transaction by hash
    pub(super) async fn get_transaction(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetTransactionParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let hash = Hash::from_hex(&params.hash)
            .ok_or_else(|| RpcError::invalid_params("Invalid hash format"))?;

        // First check mempool
        {
            let mempool = self.mempool.read().await;
            if let Some(entry) = mempool.get(&hash) {
                let mut response = TransactionResponse::from(&entry.tx);
                response.fee = Some(entry.fee);
                return serde_json::to_value(response)
                    .map_err(|e| RpcError::internal_error(e.to_string()));
            }
        }

        // Look up confirmed transaction via tx index
        let height = self
            .block_store
            .get_tx_block_height(&hash)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::tx_not_found_by_hash(&params.hash))?;

        let block = self
            .block_store
            .get_block_by_height(height)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::tx_not_found_by_hash(&params.hash))?;

        let block_hash = block.hash().to_hex();
        let best_height = self.chain_state.read().await.best_height;
        let confirmations = best_height.saturating_sub(height) + 1;

        for tx in &block.transactions {
            if tx.hash() == hash {
                let mut response = TransactionResponse::from(tx);
                response.block_hash = Some(block_hash);
                response.block_height = Some(height);
                response.confirmations = Some(confirmations);

                // Resolve input addresses from referenced outputs
                self.resolve_input_addresses(tx, &mut response);

                // Calculate fee from resolved inputs
                if response.fee.is_none() {
                    let total_in: u64 = response.inputs.iter().filter_map(|i| i.amount).sum();
                    let total_out: u64 = response.outputs.iter().map(|o| o.amount).sum();
                    if total_in > 0 && total_in >= total_out {
                        response.fee = Some(total_in - total_out);
                    }
                }

                return serde_json::to_value(response)
                    .map_err(|e| RpcError::internal_error(e.to_string()));
            }
        }

        Err(RpcError::tx_not_found_by_hash(&params.hash))
    }

    /// Resolve input addresses by looking up referenced outputs
    fn resolve_input_addresses(&self, tx: &Transaction, response: &mut TransactionResponse) {
        for (i, input) in tx.inputs.iter().enumerate() {
            if let Ok(Some(parent_height)) =
                self.block_store.get_tx_block_height(&input.prev_tx_hash)
            {
                if let Ok(Some(parent_block)) = self.block_store.get_block_by_height(parent_height)
                {
                    for ptx in &parent_block.transactions {
                        if ptx.hash() == input.prev_tx_hash {
                            if let Some(output) = ptx.outputs.get(input.output_index as usize) {
                                if let Some(resp_input) = response.inputs.get_mut(i) {
                                    resp_input.address =
                                        crypto::address::encode(&output.pubkey_hash, "doli").ok();
                                    resp_input.amount = Some(output.amount);
                                }
                            }
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Get NFT by token ID — scans UTXO set for the NFT with matching token_id
    pub(super) async fn get_nft_by_token_id(&self, params: Value) -> Result<Value, RpcError> {
        let token_id_hex: String = match params {
            Value::Array(ref arr) if !arr.is_empty() => arr[0]
                .as_str()
                .ok_or_else(|| RpcError::invalid_params("token_id must be a string"))?
                .to_string(),
            Value::Object(ref obj) => obj
                .get("tokenId")
                .or_else(|| obj.get("token_id"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| RpcError::invalid_params("missing tokenId field"))?
                .to_string(),
            _ => {
                return Err(RpcError::invalid_params(
                    "Expected [tokenId] or {tokenId: ...}",
                ))
            }
        };

        let token_id = Hash::from_hex(&token_id_hex)
            .ok_or_else(|| RpcError::invalid_params("Invalid token ID hex"))?;

        let utxo_set = self.utxo_set.read().await;
        let result = utxo_set.find_nft_by_token_id(&token_id);
        drop(utxo_set);

        match result {
            Some((outpoint, entry)) => {
                let address =
                    crypto::address::encode(&entry.output.pubkey_hash, "doli").unwrap_or_default();
                let mut response = serde_json::json!({
                    "tokenId": token_id_hex,
                    "outpoint": format!("{}:{}", outpoint.tx_hash.to_hex(), outpoint.index),
                    "owner": address,
                    "amount": entry.output.amount,
                    "height": entry.height,
                });

                if let Some((_, content)) = entry.output.nft_metadata() {
                    // Check if content starts with PNG magic bytes
                    let content_hex = hex::encode(&content);
                    response["contentHash"] = Value::String(content_hex);
                    response["contentSize"] = Value::Number(content.len().into());
                }

                // Royalty info from extra_data
                if let Ok(output_resp) = serde_json::to_value(OutputResponse::from(&entry.output)) {
                    if let Some(nft) = output_resp.get("nft") {
                        if let Some(royalty) = nft.get("royalty") {
                            response["royalty"] = royalty.clone();
                        }
                    }
                }

                Ok(response)
            }
            None => Err(RpcError::invalid_params("NFT not found with this token ID")),
        }
    }

    /// Send transaction
    pub(super) async fn send_transaction(&self, params: Value) -> Result<Value, RpcError> {
        let params: SendTransactionParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let tx_bytes = hex::decode(&params.tx)
            .map_err(|e| RpcError::invalid_params(format!("Invalid hex: {}", e)))?;

        let tx = Transaction::deserialize(&tx_bytes)
            .ok_or_else(|| RpcError::invalid_params("Failed to deserialize transaction"))?;

        let tx_hash = tx.hash();

        // Add to mempool — state-only txs (Exit, RequestWithdrawal, etc.) bypass
        // UTXO fee accounting since they have no inputs by design. Their spam
        // protection comes from requiring a registered producer bond.
        {
            let chain_state = self.chain_state.read().await;
            let current_height = chain_state.best_height;
            drop(chain_state);

            let mut mempool = self.mempool.write().await;

            if tx.is_state_only() {
                mempool
                    .add_system_transaction(tx.clone(), current_height)
                    .map_err(|e| match e {
                        MempoolError::AlreadyExists => RpcError::tx_already_known(),
                        MempoolError::Full => RpcError::mempool_full(),
                        MempoolError::InvalidTransaction(msg) => RpcError::invalid_tx(msg),
                        _ => RpcError::internal_error(e.to_string()),
                    })?;
            } else {
                let utxo_set = self.utxo_set.read().await;
                mempool
                    .add_transaction(tx.clone(), &utxo_set, current_height)
                    .map_err(|e| match e {
                        MempoolError::AlreadyExists => RpcError::tx_already_known(),
                        MempoolError::Full => RpcError::mempool_full(),
                        MempoolError::InvalidTransaction(msg) => RpcError::invalid_tx(msg),
                        _ => RpcError::internal_error(e.to_string()),
                    })?;
            }
        }

        // Broadcast to network
        (self.broadcast_tx)(tx);

        Ok(Value::String(tx_hash.to_hex()))
    }
}
