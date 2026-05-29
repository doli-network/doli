//! D4 (foundations cycle): `getDefiHealthMetric` — bond-to-TVL monitoring metric.
//!
//! Implements AC-6 (per `specs/defi-subsystem-architecture.md` Acceptance
//! Criteria block, ACCEPTED 2026-05-29): publish
//! `R = total_active_bonds / max_pool_TVL` continuously. This is a
//! disclosure metric, never a transaction-rejection gate.
//!
//! OUTPUT CONTRACT:
//!   getDefiHealthMetric -> Result<Value, RpcError> where Value is
//!     {
//!       "totalActiveBonds": u64,           // base units (8 decimals)
//!       "maxPoolTvl": u64,                 // base units, DOLI numeraire
//!       "maxPoolId": "<hex32>" | null,     // null if no pools
//!       "bondToTvlRatio": f64 | null,      // null if maxPoolTvl == 0
//!       "status": "ok" | "degraded" | "no_pools",
//!       "disclosure": string,              // human-readable status note
//!       "note": string                     // pre-oracle numeraire caveat
//!     }
//!
//! INPUT PARTITIONS:
//!   utxo_state = { no_pools_no_bonds, bonds_only, pools_only,
//!                  bonds_ge_max_pool, bonds_lt_max_pool,
//!                  multi_pool_max_picks_largest, duplicate_pool_id_dedupes,
//!                  extreme_bond_sum_overflow_safe }

use serde_json::Value;

use crate::error::RpcError;

use super::context::RpcContext;

const NUMERAIRE_NOTE: &str =
    "TVL denominated in DOLI using pool's own spot price. Pre-oracle Phase 1 — self-referential.";

const DISCLOSURE_OK: &str =
    "Total active bonds cover the largest single pool's TVL. Economic security against pool-level capture is intact.";

const DISCLOSURE_DEGRADED: &str =
    "Single-pool capital exceeds bonded security budget. Economic security against pool-level capture is degraded.";

const DISCLOSURE_NO_POOLS: &str =
    "No AMM pools exist. Bond-to-TVL ratio is not yet defined for this chain state.";

impl RpcContext {
    /// D4 AC-6 monitoring metric: `R = total_active_bonds / max_pool_TVL`.
    /// Read-only; never rejects.
    pub(super) async fn get_defi_health_metric(&self, _params: Value) -> Result<Value, RpcError> {
        let (total_active_bonds, max_pool) = {
            let utxo_set = self.utxo_set.read().await;
            utxo_set.defi_health_inputs()
        };

        match max_pool {
            None => Ok(serde_json::json!({
                "totalActiveBonds": total_active_bonds,
                "maxPoolTvl": 0u64,
                "maxPoolId": Value::Null,
                "bondToTvlRatio": Value::Null,
                "status": "no_pools",
                "disclosure": DISCLOSURE_NO_POOLS,
                "note": NUMERAIRE_NOTE,
            })),
            Some((pool_id, max_pool_tvl)) => {
                let ratio: f64 = if max_pool_tvl == 0 {
                    0.0
                } else {
                    (total_active_bonds as f64) / (max_pool_tvl as f64)
                };
                let status = if ratio >= 1.0 { "ok" } else { "degraded" };
                let disclosure = if status == "ok" {
                    DISCLOSURE_OK
                } else {
                    DISCLOSURE_DEGRADED
                };
                Ok(serde_json::json!({
                    "totalActiveBonds": total_active_bonds,
                    "maxPoolTvl": max_pool_tvl,
                    "maxPoolId": pool_id.to_hex(),
                    "bondToTvlRatio": ratio,
                    "status": status,
                    "disclosure": disclosure,
                    "note": NUMERAIRE_NOTE,
                }))
            }
        }
    }
}

#[cfg(test)]
#[path = "tests_defi_health.rs"]
mod tests;
