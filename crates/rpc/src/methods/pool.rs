//! Pool AMM query handlers: getPoolInfo, getPoolList, getPoolPrice, getSwapQuote

use serde_json::Value;

use crate::error::RpcError;

use super::context::RpcContext;

impl RpcContext {
    /// Get detailed pool info by pool ID.
    pub(super) async fn get_pool_info(&self, params: Value) -> Result<Value, RpcError> {
        let pool_id_hex = params
            .get("poolId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RpcError::invalid_params("missing 'poolId' parameter"))?;

        let pool_id = crypto::Hash::from_hex(pool_id_hex)
            .ok_or_else(|| RpcError::invalid_params("invalid poolId hex"))?;

        let utxo_set = self.utxo_set.read().await;
        let (_outpoint, entry) = utxo_set
            .get_pool_utxo(&pool_id)
            .ok_or_else(RpcError::pool_not_found)?;

        let meta = entry
            .output
            .pool_metadata()
            .ok_or_else(|| RpcError::internal_error("failed to decode pool metadata"))?;

        let price = if meta.reserve_a > 0 {
            meta.reserve_b as f64 / meta.reserve_a as f64
        } else {
            0.0
        };

        let response = serde_json::json!({
            "poolId": meta.pool_id.to_hex(),
            "assetA": crypto::Hash::ZERO.to_hex(),
            "assetB": meta.asset_b_id.to_hex(),
            "reserveA": meta.reserve_a,
            "reserveB": meta.reserve_b,
            "totalShares": meta.total_lp_shares,
            "feeBps": meta.fee_bps,
            "price": price,
            "twapCumulativePrice": format!("{}", meta.cumulative_price),
            "lastUpdateSlot": meta.last_update_slot,
            "creationSlot": meta.creation_slot,
            "status": meta.status,
        });

        Ok(response)
    }

    /// List all pools.
    pub(super) async fn get_pool_list(&self, _params: Value) -> Result<Value, RpcError> {
        let utxo_set = self.utxo_set.read().await;
        let pools = utxo_set.get_all_pools();

        let mut list = Vec::new();
        for (_outpoint, entry) in pools {
            if let Some(meta) = entry.output.pool_metadata() {
                let price = if meta.reserve_a > 0 {
                    meta.reserve_b as f64 / meta.reserve_a as f64
                } else {
                    0.0
                };
                list.push(serde_json::json!({
                    "poolId": meta.pool_id.to_hex(),
                    "assetB": meta.asset_b_id.to_hex(),
                    "reserveA": meta.reserve_a,
                    "reserveB": meta.reserve_b,
                    "feeBps": meta.fee_bps,
                    "price": price,
                }));
            }
        }

        serde_json::to_value(list).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get pool spot price and optional TWAP.
    pub(super) async fn get_pool_price(&self, params: Value) -> Result<Value, RpcError> {
        let pool_id_hex = params
            .get("poolId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RpcError::invalid_params("missing 'poolId' parameter"))?;

        let pool_id = crypto::Hash::from_hex(pool_id_hex)
            .ok_or_else(|| RpcError::invalid_params("invalid poolId hex"))?;

        let utxo_set = self.utxo_set.read().await;
        let (_outpoint, entry) = utxo_set
            .get_pool_utxo(&pool_id)
            .ok_or_else(RpcError::pool_not_found)?;

        let meta = entry
            .output
            .pool_metadata()
            .ok_or_else(|| RpcError::internal_error("failed to decode pool metadata"))?;

        let spot_price = if meta.reserve_a > 0 {
            meta.reserve_b as f64 / meta.reserve_a as f64
        } else {
            0.0
        };

        let mut response = serde_json::json!({
            "spotPrice": spot_price,
        });

        // If windowSlots is provided, compute TWAP
        if let Some(window) = params.get("windowSlots").and_then(|v| v.as_u64()) {
            if window > 0 {
                let chain_state = self.chain_state.read().await;
                let current_slot = chain_state.best_slot;
                let elapsed = current_slot.saturating_sub(meta.creation_slot);
                let actual_window = (window as u32).min(elapsed);

                if actual_window > 0 && meta.cumulative_price > 0 {
                    // TWAP is cumulative / window (fixed-point >> 64)
                    let twap_fp =
                        doli_core::compute_twap_price(0, meta.cumulative_price, actual_window);
                    if let Some(fp) = twap_fp {
                        let twap_price = fp as f64 / (1u128 << 64) as f64;
                        response["twapPrice"] = serde_json::json!(twap_price);
                        response["twapWindow"] = serde_json::json!(actual_window);
                    }
                }
            }
        }

        Ok(response)
    }

    /// Get a swap quote (simulated, no transaction).
    pub(super) async fn get_swap_quote(&self, params: Value) -> Result<Value, RpcError> {
        let pool_id_hex = params
            .get("poolId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RpcError::invalid_params("missing 'poolId' parameter"))?;

        let amount_in = params
            .get("amountIn")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| RpcError::invalid_params("missing or invalid 'amountIn' parameter"))?;

        let direction = params
            .get("direction")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                RpcError::invalid_params("missing 'direction' parameter (a2b or b2a)")
            })?;

        let pool_id = crypto::Hash::from_hex(pool_id_hex)
            .ok_or_else(|| RpcError::invalid_params("invalid poolId hex"))?;

        let utxo_set = self.utxo_set.read().await;
        let (_outpoint, entry) = utxo_set
            .get_pool_utxo(&pool_id)
            .ok_or_else(RpcError::pool_not_found)?;

        let meta = entry
            .output
            .pool_metadata()
            .ok_or_else(|| RpcError::internal_error("failed to decode pool metadata"))?;

        let (reserve_in, reserve_out) = match direction {
            "a2b" => (meta.reserve_a, meta.reserve_b),
            "b2a" => (meta.reserve_b, meta.reserve_a),
            _ => return Err(RpcError::invalid_params("direction must be 'a2b' or 'b2a'")),
        };

        let (amount_out, _new_reserve_in, _new_reserve_out) =
            doli_core::compute_swap(reserve_in, reserve_out, amount_in, meta.fee_bps).ok_or_else(
                || RpcError::invalid_params("swap computation failed (zero amount or reserves)"),
            )?;

        // Price impact: how much worse than the mid-market rate
        let mid_price = reserve_out as f64 / reserve_in as f64;
        let effective_price = amount_out as f64 / amount_in as f64;
        let price_impact = if mid_price > 0.0 {
            ((mid_price - effective_price) / mid_price) * 100.0
        } else {
            0.0
        };

        // Fee portion
        let fee = (amount_in as u128 * meta.fee_bps as u128 / 10000) as u64;

        let response = serde_json::json!({
            "amountOut": amount_out,
            "priceImpact": price_impact,
            "fee": fee,
        });

        Ok(response)
    }
}
