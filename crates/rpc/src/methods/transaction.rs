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
            .ok_or_else(RpcError::tx_not_found)?;

        let block = self
            .block_store
            .get_block_by_height(height)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(RpcError::tx_not_found)?;

        let block_hash = block.hash().to_hex();
        let best_height = self.chain_state.read().await.best_height;
        let confirmations = best_height.saturating_sub(height) + 1;

        for tx in &block.transactions {
            if tx.hash() == hash {
                let mut response = TransactionResponse::from(tx);
                response.block_hash = Some(block_hash);
                response.block_height = Some(height);
                response.confirmations = Some(confirmations);
                return serde_json::to_value(response)
                    .map_err(|e| RpcError::internal_error(e.to_string()));
            }
        }

        Err(RpcError::tx_not_found())
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
