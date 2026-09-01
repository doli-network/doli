//! INC-I-204 M4.1 / REQ-FORK-012 — `forceReorgTo`, the audited operator escape
//! from a `tip == finality` wedge.
//!
//! Admin-gated (`server.rs` `ADMIN_METHODS`). Arming only records a directive on
//! the shared `SyncManager`; the node evaluates it on its next periodic tick, and
//! the directive expires on a wall-clock TTL, on a height span, and on the first
//! decision it reaches. Split out of `guardian.rs`, which is over the module-size
//! budget.

use crypto::Hash;
use serde_json::Value;
use tracing::warn;

use crate::error::RpcError;
use crate::types::ForceReorgToParams;

use super::context::RpcContext;

impl RpcContext {
    /// Arm a single-shot force-reorg onto the named branch tip.
    pub(super) async fn force_reorg_to(&self, params: Value) -> Result<Value, RpcError> {
        let params: ForceReorgToParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let target = Hash::from_hex(&params.hash).ok_or_else(|| {
            RpcError::invalid_params("Invalid hash format: expected 64 lowercase hex characters")
        })?;

        let sync_manager = self.sync_manager.as_ref().ok_or_else(|| {
            RpcError::internal_error(
                "sync manager not available — nothing to arm, the directive was NOT accepted"
                    .to_string(),
            )
        })?;

        let (ttl_secs, max_height_span) = {
            let mut sm = sync_manager.write().await;
            sm.arm_force_reorg(target);
            (
                network::FORCE_REORG_TTL_SECS,
                network::FORCE_REORG_MAX_HEIGHT_SPAN,
            )
        };

        warn!(
            "[FORCE_REORG] operator armed forceReorgTo target={} via RPC",
            target
        );

        Ok(serde_json::json!({
            "status": "armed",
            "target": target.to_hex(),
            "ttlSeconds": ttl_secs,
            "maxHeightSpan": max_height_span,
            "message": "Single-shot force reorg armed. It is evaluated on the next periodic \
                        tick, is refused unless the target branch carries at least two thirds \
                        of local producer weight and every branch block passes producer \
                        eligibility, and it expires on restart, on the TTL, or on the first \
                        decision it reaches."
        }))
    }
}
