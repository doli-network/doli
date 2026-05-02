//! Seed Guardian RPC methods — production halt, checkpoint creation, guardian status, recovery mode.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use serde_json::Value;
use tracing::{error, info, warn};

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

        // Determine output path — sanitize to prevent path traversal
        let default_base = data_dir.join("checkpoints");
        let checkpoint_base = if let Some(Value::Array(arr)) = &params {
            if let Some(Value::String(p)) = arr.first() {
                let requested = PathBuf::from(p);
                // Reject absolute paths and path traversal
                if requested.is_absolute()
                    || requested
                        .components()
                        .any(|c| c == std::path::Component::ParentDir)
                {
                    return Err(RpcError::invalid_params(
                        "Checkpoint path must be relative and cannot contain '..' segments",
                    ));
                }
                // Resolve relative to data_dir
                let resolved = data_dir.join(&requested);
                // Double-check: canonicalize parent to catch symlink escapes
                if let Ok(canon) = resolved.parent().unwrap_or(&resolved).canonicalize() {
                    let data_canon = data_dir.canonicalize().unwrap_or_else(|_| data_dir.clone());
                    if !canon.starts_with(&data_canon) {
                        return Err(RpcError::invalid_params(
                            "Checkpoint path must be within the node data directory",
                        ));
                    }
                }
                resolved
            } else {
                default_base
            }
        } else {
            default_base
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
            "recovery_mode": self.recovery_mode.load(Ordering::Relaxed),
        }))
    }

    /// Enter recovery mode — drop all inbound blocks and snap sync (anti-poisoning gate).
    ///
    /// Used during seed recovery to prevent non-recovered nodes from poisoning
    /// freshly restored seeds via gossip. The node continues serving RPC and
    /// snap sync to peers, but will not apply any state mutations.
    pub(super) async fn enter_recovery_mode(&self) -> Result<Value, RpcError> {
        self.recovery_mode.store(true, Ordering::Relaxed);
        warn!("[RECOVERY] Recovery mode ACTIVATED via RPC — all inbound blocks and snap sync will be dropped");

        Ok(serde_json::json!({
            "status": "recovery_mode_active",
            "message": "Recovery mode activated. All inbound blocks and snap sync are dropped. Use exitRecoveryMode to resume normal operation."
        }))
    }

    /// Exit recovery mode — resume normal block and snap sync processing.
    ///
    /// Note: fork_block_cache and rejected_fork_tips live on the Node struct
    /// (not accessible from RpcContext). A restart is recommended to clear any
    /// cached fork blocks that may have accumulated during recovery.
    pub(super) async fn exit_recovery_mode(&self) -> Result<Value, RpcError> {
        self.recovery_mode.store(false, Ordering::Relaxed);
        info!("[RECOVERY] Recovery mode DEACTIVATED via RPC — normal operation resumed");
        info!("[RECOVERY] Recommend restarting the node to clear any cached fork blocks");

        Ok(serde_json::json!({
            "status": "recovery_mode_inactive",
            "message": "Recovery mode deactivated. Normal block and snap sync processing resumed. Recommend restarting the node to clear any cached fork blocks."
        }))
    }

    /// Bridge checkpoint restore to archive backfill.
    ///
    /// After restoring a RocksDB checkpoint, this method:
    /// 1. Deletes the stale chain_commitment (unconditionally stale after restore)
    /// 2. Backfills missing blocks from the archive directory into the block store
    /// 3. Returns a summary of what was imported
    ///
    /// Optional param: `[true]` or `{"force": true}` to replace divergent blocks
    /// (INC-I-055: archive blocks override fork blocks via BLAKE3 comparison).
    ///
    /// Recovery mode should be active (enterRecoveryMode) before calling this
    /// to prevent concurrent block processing during the bridge operation.
    pub(super) async fn bridge_from_archive(
        &self,
        params: Option<Value>,
    ) -> Result<Value, RpcError> {
        // Parse optional force parameter
        let force = params
            .as_ref()
            .and_then(|p| {
                p.get(0)
                    .and_then(|v| v.as_bool())
                    .or_else(|| p.get("force").and_then(|v| v.as_bool()))
            })
            .unwrap_or(false);

        let recovery_active = self.recovery_mode.load(Ordering::Relaxed);

        // BC4: Warn if recovery mode is not active (race condition risk)
        if !recovery_active {
            warn!(
                "[BRIDGE] bridge_from_archive called without recovery mode active — \
                 concurrent block processing may interfere"
            );
        }

        // INC-I-055: force-replace requires recovery_mode. Replacing blocks while
        // sync/fork recovery is running causes race conditions — the sync manager
        // may be reading blocks that get replaced underneath it.
        if force && !recovery_active {
            return Err(RpcError::invalid_params(
                "force=true requires recovery mode to be active (enterRecoveryMode). \
                 Replacing blocks while sync is running risks data races.",
            ));
        }

        let archive_dir = self.archive_dir.clone().ok_or_else(|| {
            RpcError::internal_error(
                "archive_dir not configured — start node with --archive-to to enable".to_string(),
            )
        })?;

        let state_db = self
            .state_db
            .clone()
            .ok_or_else(|| RpcError::internal_error("state_db not available".to_string()))?;

        let block_store = self.block_store.clone();

        // The bridge is synchronous (file I/O) — run in blocking thread
        let result = tokio::task::spawn_blocking(move || -> Result<(u64, bool), String> {
            // Step 1: DELETE stale chain_commitment BEFORE backfill (BC3, FM-4).
            state_db.delete_chain_commitment();
            info!("[BRIDGE] Deleted stale chain_commitment after checkpoint restore");

            // FM-1: No archive directory — early return
            if !archive_dir.exists() {
                info!(
                    "[BRIDGE] Archive directory does not exist ({:?}), skipping backfill",
                    archive_dir
                );
                return Ok((0u64, false));
            }

            // Read genesis_hash from block store for cross-chain validation
            let genesis_hash = {
                let mut found = None;
                for h in 1..=10_000u64 {
                    if let Ok(Some(blk)) = block_store.get_block_by_height(h) {
                        found = Some(blk.header.genesis_hash);
                        break;
                    }
                }
                found
            };

            // Step 2: BACKFILL from archive (existing idempotent function, BC5)
            // INC-I-055: force mode replaces divergent blocks via checksum comparison.
            let imported = if force {
                storage::archiver::force_backfill_from_archive(
                    &archive_dir,
                    &block_store,
                    genesis_hash.as_ref(),
                )?
            } else {
                storage::archiver::backfill_from_archive(
                    &archive_dir,
                    &block_store,
                    genesis_hash.as_ref(),
                )?
            };

            Ok((imported, true))
        })
        .await
        .map_err(|e| RpcError::internal_error(format!("Bridge task failed: {}", e)))?;

        match result {
            Ok((imported, archive_found)) => {
                info!(
                    "[BRIDGE] Archive bridge complete: {} blocks imported",
                    imported
                );
                Ok(serde_json::json!({
                    "status": "ok",
                    "blocks_imported": imported,
                    "archive_found": archive_found,
                    "commitment_deleted": true
                }))
            }
            Err(e) => {
                // Archive errors are warnings, not hard failures (AC3)
                warn!("[BRIDGE] Archive bridge error (non-fatal): {}", e);
                Ok(serde_json::json!({
                    "status": "warning",
                    "blocks_imported": 0,
                    "archive_found": true,
                    "commitment_deleted": true,
                    "warning": format!("{}", e)
                }))
            }
        }
    }

    /// Repair the local archive by fetching missing blocks from a peer's RPC.
    ///
    /// INC-I-055: Replaces the manual tar+scp relay between seeds. Given a peer's
    /// RPC URL, scans the local archive directory for missing .block files up to
    /// the peer's chain tip, fetches each missing block via `getBlockRaw`, verifies
    /// the BLAKE3 checksum, validates genesis_hash, and writes the file to the
    /// local archive directory.
    ///
    /// Params: `{"rpc_url": "http://peer:8500"}` or `["http://peer:8500"]`
    pub(super) async fn repair_archive_from_peer(&self, params: Value) -> Result<Value, RpcError> {
        // Parse peer RPC URL
        let peer_url = params
            .get("rpc_url")
            .or_else(|| params.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| RpcError::invalid_params("Missing required parameter: rpc_url"))?
            .to_string();

        let archive_dir = self.archive_dir.clone().ok_or_else(|| {
            RpcError::internal_error(
                "archive_dir not configured — start node with --archive-to to enable".to_string(),
            )
        })?;

        if !archive_dir.exists() {
            std::fs::create_dir_all(&archive_dir).map_err(|e| {
                RpcError::internal_error(format!("Failed to create archive dir: {}", e))
            })?;
        }

        // Get peer's chain tip to know how far to scan
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| RpcError::internal_error(format!("HTTP client error: {}", e)))?;

        let peer_tip: u64 = {
            let resp = client
                .post(&peer_url)
                .json(&serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "getChainInfo",
                    "params": [],
                    "id": 0
                }))
                .send()
                .await
                .map_err(|e| RpcError::internal_error(format!("Failed to reach peer: {}", e)))?;

            let body: serde_json::Value = resp.json().await.map_err(|e| {
                RpcError::internal_error(format!("Invalid response from peer: {}", e))
            })?;

            body.pointer("/result/height")
                .or_else(|| body.pointer("/result/best_height"))
                .and_then(|v| v.as_u64())
                .ok_or_else(|| {
                    RpcError::internal_error(
                        "Peer did not return chain height in getChainInfo".to_string(),
                    )
                })?
        };

        // Read local genesis hash for validation
        let local_genesis = {
            let cs = self.chain_state.read().await;
            cs.genesis_hash
        };

        // Spawn background task to fetch missing blocks
        let archive_dir_bg = archive_dir.clone();
        let peer_url_bg = peer_url.clone();

        let result = tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| format!("HTTP client: {}", e))?;

            let mut fetched = 0u64;
            let mut skipped = 0u64;
            let mut errors = 0u64;

            for h in 1..=peer_tip {
                let block_path = archive_dir_bg.join(format!("{:010}.block", h));
                let checksum_path = archive_dir_bg.join(format!("{:010}.blake3", h));

                // Skip if both .block and .blake3 exist
                if block_path.exists() && checksum_path.exists() {
                    skipped += 1;
                    continue;
                }

                // Fetch from peer
                let resp = match client
                    .post(&peer_url_bg)
                    .json(&serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "getBlockRaw",
                        "params": { "height": h },
                        "id": h
                    }))
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        if errors < 5 {
                            tracing::warn!("[REPAIR_ARCHIVE] HTTP error at h={}: {}", h, e);
                        }
                        errors += 1;
                        continue;
                    }
                };

                let body: serde_json::Value = match resp.json().await {
                    Ok(v) => v,
                    Err(e) => {
                        if errors < 5 {
                            tracing::warn!("[REPAIR_ARCHIVE] JSON error at h={}: {}", h, e);
                        }
                        errors += 1;
                        continue;
                    }
                };

                let result = match body.get("result") {
                    Some(r) => r,
                    None => {
                        errors += 1;
                        continue;
                    }
                };

                let b64_data = match result.get("block").and_then(|v| v.as_str()) {
                    Some(s) => s,
                    None => {
                        errors += 1;
                        continue;
                    }
                };

                let expected_checksum = result.get("blake3").and_then(|v| v.as_str()).unwrap_or("");

                // Decode
                use base64::Engine;
                let data = match base64::engine::general_purpose::STANDARD.decode(b64_data) {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::warn!("[REPAIR_ARCHIVE] Base64 error at h={}: {}", h, e);
                        errors += 1;
                        continue;
                    }
                };

                // Verify BLAKE3 checksum
                let actual_checksum = crypto::hash::hash(&data).to_string();
                if !expected_checksum.is_empty() && actual_checksum != expected_checksum {
                    tracing::warn!("[REPAIR_ARCHIVE] BLAKE3 mismatch at h={}", h);
                    errors += 1;
                    continue;
                }

                // Validate genesis_hash
                if let Some(block) = doli_core::transaction::legacy::deserialize_block_compat(&data)
                {
                    if block.header.genesis_hash != local_genesis {
                        return Err(format!(
                            "Block at h={} has wrong genesis_hash — peer is on a different chain",
                            h
                        ));
                    }
                }

                // Write .block file (atomic via tmp rename)
                let tmp_block = archive_dir_bg.join(format!("{:010}.block.tmp", h));
                if let Err(e) = std::fs::write(&tmp_block, &data) {
                    tracing::warn!("[REPAIR_ARCHIVE] Write error at h={}: {}", h, e);
                    errors += 1;
                    continue;
                }
                let _ = std::fs::rename(&tmp_block, &block_path);

                // Write .blake3 checksum
                let tmp_blake3 = archive_dir_bg.join(format!("{:010}.blake3.tmp", h));
                let _ = std::fs::write(&tmp_blake3, &actual_checksum);
                let _ = std::fs::rename(&tmp_blake3, &checksum_path);

                fetched += 1;
                if fetched.is_multiple_of(100) {
                    tracing::info!(
                        "[REPAIR_ARCHIVE] Progress: {}/{} fetched ({} skipped, {} errors)",
                        h,
                        peer_tip,
                        skipped,
                        errors
                    );
                }

                // Yield every 50 blocks
                if fetched.is_multiple_of(50) {
                    tokio::task::yield_now().await;
                }
            }

            tracing::info!(
                "[REPAIR_ARCHIVE] Complete: {} fetched, {} already present, {} errors (1..={})",
                fetched,
                skipped,
                errors,
                peer_tip
            );

            Ok((fetched, skipped, errors, peer_tip))
        })
        .await
        .map_err(|e| RpcError::internal_error(format!("Repair task panicked: {}", e)))?;

        match result {
            Ok((fetched, skipped, errors, tip)) => Ok(serde_json::json!({
                "status": "ok",
                "fetched": fetched,
                "already_present": skipped,
                "errors": errors,
                "peer_tip": tip,
            })),
            Err(e) => Err(RpcError::internal_error(e)),
        }
    }
}
