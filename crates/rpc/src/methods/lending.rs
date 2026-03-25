//! Lending query handlers: getLoanInfo, getLoanList

use serde_json::Value;

use crate::error::RpcError;

use super::context::RpcContext;

impl RpcContext {
    /// Get detailed loan info by Collateral UTXO outpoint.
    ///
    /// Params: `{ "txHash": "hex", "outputIndex": number }`
    pub(super) async fn get_loan_info(&self, params: Value) -> Result<Value, RpcError> {
        let tx_hash_hex = params
            .get("txHash")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RpcError::invalid_params("missing 'txHash' parameter"))?;

        let output_index = params
            .get("outputIndex")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| RpcError::invalid_params("missing 'outputIndex' parameter"))?
            as u32;

        let tx_hash = crypto::Hash::from_hex(tx_hash_hex)
            .ok_or_else(|| RpcError::invalid_params("invalid txHash hex"))?;

        let utxo_set = self.utxo_set.read().await;
        let outpoint = storage::Outpoint {
            tx_hash,
            index: output_index,
        };

        let entry = utxo_set
            .get(&outpoint)
            .ok_or_else(RpcError::utxo_not_found)?;

        if entry.output.output_type != doli_core::OutputType::Collateral {
            return Err(RpcError::invalid_params(
                "UTXO is not a Collateral output type",
            ));
        }

        let meta = entry
            .output
            .collateral_metadata()
            .ok_or_else(|| RpcError::internal_error("failed to decode collateral metadata"))?;

        let chain_state = self.chain_state.read().await;
        let current_slot = chain_state.best_slot;
        let elapsed = current_slot.saturating_sub(meta.creation_slot) as u64;

        let interest = doli_core::compute_interest(meta.principal, meta.interest_rate_bps, elapsed);
        let total_debt = meta.principal + interest;
        let ltv = doli_core::compute_ltv_bps(total_debt, entry.output.amount);
        let liquidatable =
            doli_core::is_liquidatable(total_debt, entry.output.amount, meta.liquidation_ratio_bps);

        let response = serde_json::json!({
            "outpoint": {
                "txHash": tx_hash.to_hex(),
                "outputIndex": output_index,
            },
            "poolId": meta.pool_id.to_hex(),
            "borrowerHash": meta.borrower_hash.to_hex(),
            "collateralAmount": entry.output.amount,
            "collateralAssetId": meta.collateral_asset_id.to_hex(),
            "principal": meta.principal,
            "interestRateBps": meta.interest_rate_bps,
            "creationSlot": meta.creation_slot,
            "liquidationRatioBps": meta.liquidation_ratio_bps,
            "accruedInterest": interest,
            "totalDebt": total_debt,
            "elapsedSlots": elapsed,
            "ltvBps": ltv,
            "liquidatable": liquidatable,
        });

        Ok(response)
    }

    /// List active loans (Collateral UTXOs), optionally filtered by borrower.
    ///
    /// Params: `{ "borrower": "hex" }` (optional)
    pub(super) async fn get_loan_list(&self, params: Value) -> Result<Value, RpcError> {
        let borrower_filter = params
            .get("borrower")
            .and_then(|v| v.as_str())
            .and_then(crypto::Hash::from_hex);

        let utxo_set = self.utxo_set.read().await;
        let chain_state = self.chain_state.read().await;
        let current_slot = chain_state.best_slot;

        let mut list = Vec::new();
        for (outpoint, entry) in utxo_set.get_all_collateral() {
            let Some(meta) = entry.output.collateral_metadata() else {
                continue;
            };

            if let Some(ref filter) = borrower_filter {
                if meta.borrower_hash != *filter {
                    continue;
                }
            }

            let elapsed = current_slot.saturating_sub(meta.creation_slot) as u64;
            let interest =
                doli_core::compute_interest(meta.principal, meta.interest_rate_bps, elapsed);
            let total_debt = meta.principal + interest;

            list.push(serde_json::json!({
                "outpoint": {
                    "txHash": outpoint.tx_hash.to_hex(),
                    "outputIndex": outpoint.index,
                },
                "borrowerHash": meta.borrower_hash.to_hex(),
                "collateralAmount": entry.output.amount,
                "principal": meta.principal,
                "totalDebt": total_debt,
                "interestRateBps": meta.interest_rate_bps,
                "liquidatable": doli_core::is_liquidatable(total_debt, entry.output.amount, meta.liquidation_ratio_bps),
            }));
        }

        serde_json::to_value(list).map_err(|e| RpcError::internal_error(e.to_string()))
    }
}
