//! Seed Guardian RPC methods — production halt, checkpoint creation, guardian status.

use std::path::PathBuf;

use serde_json::Value;
use tracing::{error, info};

use crate::error::RpcError;

use super::context::RpcContext;

impl RpcContext {
    /// Pause block production on this node.
    ///
    /// Sets an explicit production block via the SyncManager. The node remains
    /// running (serving RPC, syncing blocks) but will not produce new blocks.
    /// Seeds are unaffected since they never produce.
    pub(super) async fn pause_production(&self) -> Result<Value, RpcError> {
        let sync_manager = self
            .sync_manager
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("sync manager not available".to_string()))?;

        let mut sm = sync_manager.write().await;
        sm.block_production("Emergency halt via RPC (pauseProduction)");

        info!("[GUARDIAN] Production PAUSED via RPC");

        Ok(serde_json::json!({
            "status": "paused",
            "message": "Block production has been paused. Use resumeProduction to resume."
        }))
    }

    /// Resume block production on this node.
    ///
    /// Clears the explicit production block set by pauseProduction.
    pub(super) async fn resume_production(&self) -> Result<Value, RpcError> {
        let sync_manager = self
            .sync_manager
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("sync manager not available".to_string()))?;

        let mut sm = sync_manager.write().await;
        sm.unblock_production();

        info!("[GUARDIAN] Production RESUMED via RPC");

        Ok(serde_json::json!({
            "status": "resumed",
            "message": "Block production has been resumed."
        }))
    }

    /// Create a RocksDB checkpoint (hot backup) of the state database and block store.
    ///
    /// Checkpoints use hard links — near-instant, near-zero extra disk space.
    /// Output directory: `{data_dir}/checkpoints/{height}-{timestamp}/`
    ///
    /// Params: optional `[path]` to override the default checkpoint directory.
    pub(super) async fn create_checkpoint(&self, params: Option<Value>) -> Result<Value, RpcError> {
        let state_db = self
            .state_db
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("state_db not available".to_string()))?;

        let data_dir = self
            .data_dir
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("data_dir not available".to_string()))?;

        // Get current height for naming
        let height = {
            let cs = self.chain_state.read().await;
            cs.best_height
        };

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Determine output path
        let checkpoint_base = if let Some(Value::Array(arr)) = &params {
            if let Some(Value::String(p)) = arr.first() {
                PathBuf::from(p)
            } else {
                data_dir.join("checkpoints")
            }
        } else {
            data_dir.join("checkpoints")
        };

        let checkpoint_name = format!("h{}-{}", height, timestamp);
        let checkpoint_dir = checkpoint_base.join(&checkpoint_name);

        // RocksDB create_checkpoint requires the target directory to NOT exist.
        // Only create the parent; RocksDB creates the leaf dirs.
        let state_checkpoint_dir = checkpoint_dir.join("state_db");
        let blocks_checkpoint_dir = checkpoint_dir.join("blocks");

        std::fs::create_dir_all(&checkpoint_dir).map_err(|e| {
            RpcError::internal_error(format!("Failed to create checkpoint dir: {}", e))
        })?;

        // Checkpoint state_db
        state_db
            .create_checkpoint(&state_checkpoint_dir)
            .map_err(|e| {
                error!("[GUARDIAN] State DB checkpoint failed: {}", e);
                RpcError::internal_error(format!("State DB checkpoint failed: {}", e))
            })?;

        // Checkpoint block store
        self.block_store
            .create_checkpoint(&blocks_checkpoint_dir)
            .map_err(|e| {
                error!("[GUARDIAN] Block store checkpoint failed: {}", e);
                RpcError::internal_error(format!("Block store checkpoint failed: {}", e))
            })?;

        info!(
            "[GUARDIAN] Checkpoint created: {} (height={})",
            checkpoint_dir.display(),
            height
        );

        Ok(serde_json::json!({
            "status": "ok",
            "path": checkpoint_dir.display().to_string(),
            "height": height,
            "timestamp": timestamp,
            "components": ["state_db", "blocks"]
        }))
    }

    /// Get guardian system status — production state, last checkpoint, chain health.
    pub(super) async fn get_guardian_status(&self) -> Result<Value, RpcError> {
        let chain_state = self.chain_state.read().await;

        let (production_paused, production_block_reason) = if let Some(ref sm) = self.sync_manager {
            match sm.try_read() {
                Ok(guard) => {
                    let reason = guard.production_block_reason().map(|s| s.to_string());
                    (reason.is_some(), reason)
                }
                Err(_) => (false, None),
            }
        } else {
            (false, None)
        };

        // Check for existing checkpoints and find last healthy one
        let (last_checkpoint, last_healthy_checkpoint) = if let Some(ref data_dir) = self.data_dir {
            let checkpoint_dir = data_dir.join("checkpoints");
            if checkpoint_dir.exists() {
                let mut dirs: Vec<_> = std::fs::read_dir(&checkpoint_dir)
                    .ok()
                    .map(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                            .collect()
                    })
                    .unwrap_or_default();
                dirs.sort_by_key(|e| {
                    // Sort numerically by height, not lexicographically.
                    // h526 must sort BEFORE h4535 (lexicographic gets this wrong).
                    e.file_name()
                        .to_string_lossy()
                        .strip_prefix('h')
                        .and_then(|s| s.split('-').next())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(0)
                });

                let last = dirs
                    .last()
                    .map(|e| e.file_name().to_string_lossy().to_string());

                // Scan in reverse to find last healthy checkpoint
                let last_healthy = dirs.iter().rev().find_map(|entry| {
                    let health_path = entry.path().join("health.json");
                    let data = std::fs::read_to_string(&health_path).ok()?;
                    let health: serde_json::Value = serde_json::from_str(&data).ok()?;
                    if health.get("healthy")?.as_bool()? {
                        Some(entry.file_name().to_string_lossy().to_string())
                    } else {
                        None
                    }
                });

                (last, last_healthy)
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        Ok(serde_json::json!({
            "production_paused": production_paused,
            "production_block_reason": production_block_reason,
            "chain_height": chain_state.best_height,
            "chain_slot": chain_state.best_slot,
            "best_hash": chain_state.best_hash.to_hex(),
            "last_checkpoint": last_checkpoint,
            "last_healthy_checkpoint": last_healthy_checkpoint,
        }))
    }
}
