//! Block pruning and storage info RPC methods

use serde_json::Value;
use tracing::{info, warn};

use crate::error::RpcError;

use super::context::RpcContext;

impl RpcContext {
    /// Prune old blocks below a given height to reclaim disk space.
    ///
    /// Enforces minimum retention of 2000 blocks from the chain tip.
    /// Optionally verifies that an archive directory contains blocks
    /// up to the prune height before deleting.
    ///
    /// Params: `[keep_last_n]` — number of recent blocks to keep (default: 2000).
    ///         If `keep_last_n` < 2000, it is clamped to 2000.
    pub(super) async fn prune_blocks(&self, params: Option<Value>) -> Result<Value, RpcError> {
        let chain_tip = {
            let cs = self.chain_state.read().await;
            cs.best_height
        };

        if chain_tip == 0 {
            return Err(RpcError::internal_error(
                "Chain is empty — nothing to prune".to_string(),
            ));
        }

        // Parse keep_last_n from params (default 2000)
        let keep_last_n = if let Some(Value::Array(arr)) = &params {
            arr.first()
                .and_then(|v| v.as_u64())
                .unwrap_or(2000)
                .max(2000) // Floor at 2000
        } else {
            2000
        };

        let cutoff = chain_tip.saturating_sub(keep_last_n);

        if cutoff <= 1 {
            return Ok(serde_json::json!({
                "status": "noop",
                "message": "Not enough blocks to prune",
                "chain_tip": chain_tip,
                "keep_last_n": keep_last_n,
                "pruned": 0
            }));
        }

        // Optionally check archive coverage
        let archive_verified = if let Some(ref data_dir) = self.data_dir {
            let archive_dir = data_dir.join("archive");
            if archive_dir.exists() {
                match storage::archiver::manifest_height(&archive_dir) {
                    Some(archived_height) if archived_height >= cutoff => {
                        info!(
                            "[PRUNE] Archive verified: {} blocks archived (cutoff={})",
                            archived_height, cutoff
                        );
                        true
                    }
                    Some(archived_height) => {
                        warn!(
                            "[PRUNE] Archive only covers height {} but prune cutoff is {} — pruning anyway (blocks will be lost)",
                            archived_height, cutoff
                        );
                        false
                    }
                    None => false,
                }
            } else {
                false
            }
        } else {
            false
        };

        // Execute pruning
        let (deleted, lowest_remaining) = self
            .block_store
            .prune_blocks_below(cutoff, chain_tip)
            .map_err(|e| RpcError::internal_error(format!("Prune failed: {}", e)))?;

        info!(
            "[PRUNE] Completed: {} blocks pruned, lowest remaining height={}",
            deleted, lowest_remaining
        );

        Ok(serde_json::json!({
            "status": "ok",
            "pruned": deleted,
            "lowest_remaining_height": lowest_remaining,
            "chain_tip": chain_tip,
            "keep_last_n": keep_last_n,
            "archive_verified": archive_verified
        }))
    }

    /// Get storage statistics for the block store.
    ///
    /// Reports entry counts per column family, height range, and estimated
    /// prunable blocks.
    pub(super) async fn get_storage_info(&self) -> Result<Value, RpcError> {
        let chain_tip = {
            let cs = self.chain_state.read().await;
            cs.best_height
        };

        // Get entry counts per CF
        let stats = self
            .block_store
            .storage_stats()
            .map_err(|e| RpcError::internal_error(format!("Stats failed: {}", e)))?;

        let cf_stats: serde_json::Map<String, Value> = stats
            .into_iter()
            .map(|(name, count)| (name.to_string(), Value::Number(count.into())))
            .collect();

        // Get height range
        let (min_height, max_height) = self
            .block_store
            .height_range()
            .map_err(|e| RpcError::internal_error(format!("Height range failed: {}", e)))?
            .unwrap_or((0, 0));

        let prunable = if max_height > 2000 {
            min_height.max(1)..max_height.saturating_sub(2000)
        } else {
            0..0
        };
        let prunable_count = prunable.end.saturating_sub(prunable.start);

        // Check archive status
        let archive_height = self.data_dir.as_ref().and_then(|d| {
            let archive_dir = d.join("archive");
            storage::archiver::manifest_height(&archive_dir)
        });

        Ok(serde_json::json!({
            "chain_tip": chain_tip,
            "height_range": {
                "min": min_height,
                "max": max_height,
            },
            "column_families": cf_stats,
            "prunable_blocks": prunable_count,
            "min_retention": 2000,
            "archive_height": archive_height,
        }))
    }
}
