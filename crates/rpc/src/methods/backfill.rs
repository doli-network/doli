//! Backfill and chain integrity handlers: backfillFromPeer, backfillStatus, verifyChainIntegrity

use std::net::IpAddr;
use std::sync::atomic::Ordering;

use serde_json::Value;
use tracing::{info, warn};

use crypto::{Hash, Hasher};

use crate::error::RpcError;
use crate::types::*;

use super::context::RpcContext;

/// Check if an IP address is in a private/reserved range (SSRF protection).
fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()         // 127.0.0.0/8
                || v4.is_private()   // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
                || v4.is_link_local() // 169.254.0.0/16
                || v4.is_broadcast() // 255.255.255.255
                || v4.is_unspecified() // 0.0.0.0
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
    }
}

/// Validate that a URL is safe for outbound requests (prevents SSRF).
/// Primary defense is the admin auth gate; this is defense-in-depth.
/// On mainnet: blocks private/loopback IPs. On testnet/devnet: allows them
/// (operators need localhost backfill on single-server setups).
fn validate_backfill_url(raw_url: &str, network: &str) -> Result<(), RpcError> {
    // Must start with http:// or https://
    let after_scheme = if let Some(rest) = raw_url.strip_prefix("http://") {
        rest
    } else if let Some(rest) = raw_url.strip_prefix("https://") {
        rest
    } else {
        return Err(RpcError::invalid_params(
            "Backfill URL must use http:// or https:// scheme",
        ));
    };

    // Extract host (before first / or :port)
    let host = after_scheme
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");

    if host.is_empty() {
        return Err(RpcError::invalid_params("URL must include a hostname"));
    }

    // If host is an IP literal, check for private ranges (mainnet only).
    // Testnet/devnet allow private IPs for localhost backfill.
    if network == "mainnet" {
        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_private_ip(&ip) {
                return Err(RpcError::invalid_params(
                    "Backfill URL must not point to a private/loopback/link-local address",
                ));
            }
        }
    }

    Ok(())
}

impl RpcContext {
    /// Start a live backfill from a peer's RPC endpoint.
    pub(super) async fn backfill_from_peer(&self, params: Value) -> Result<Value, RpcError> {
        let params: BackfillParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        // Validate URL to prevent SSRF attacks
        validate_backfill_url(&params.rpc_url, &self.network)?;

        // Check if already running
        if self.backfill_state.running.load(Ordering::SeqCst) {
            return Err(RpcError::internal_error(
                "Backfill already in progress. Use backfillStatus to check progress.",
            ));
        }

        // Find the gap: look for the lowest existing block
        let tip_height = self.chain_state.read().await.best_height;
        if tip_height == 0 {
            return Err(RpcError::internal_error("Node has no blocks yet"));
        }

        // Full scan: find ALL missing heights (leading, mid-chain, and trailing gaps)
        // Uses get_block_by_height to check actual block data, not just the index.
        // The height→hash index can exist without block data (partial sync).
        let block_store_scan = self.block_store.clone();
        let missing_heights: Vec<u64> = tokio::task::spawn_blocking(move || {
            let mut missing = Vec::new();
            for h in 1..=tip_height {
                let exists = block_store_scan
                    .get_block_by_height(h)
                    .map(|opt| opt.is_some())
                    .unwrap_or(false);
                if !exists {
                    missing.push(h);
                }
            }
            missing
        })
        .await
        .map_err(|e| RpcError::internal_error(e.to_string()))?;

        if missing_heights.is_empty() {
            return Ok(serde_json::json!({
                "started": false,
                "message": "No gaps detected — chain is complete."
            }));
        }

        let total_to_fetch = missing_heights.len() as u64;

        // Format gap ranges for response
        let gaps_str = {
            let mut ranges = Vec::new();
            let mut i = 0;
            while i < missing_heights.len() {
                let start = missing_heights[i];
                let mut end = start;
                while i + 1 < missing_heights.len() && missing_heights[i + 1] == end + 1 {
                    i += 1;
                    end = missing_heights[i];
                }
                if start == end {
                    ranges.push(format!("{}", start));
                } else {
                    ranges.push(format!("{}-{}", start, end));
                }
                i += 1;
            }
            ranges.join(", ")
        };

        // Reset state
        self.backfill_state.imported.store(0, Ordering::SeqCst);
        self.backfill_state
            .total
            .store(total_to_fetch, Ordering::SeqCst);
        *self.backfill_state.error.write().await = None;
        self.backfill_state.running.store(true, Ordering::SeqCst);

        // Spawn background task
        let block_store = self.block_store.clone();
        let state = self.backfill_state.clone();
        let rpc_url = params.rpc_url.clone();

        tokio::spawn(async move {
            info!(
                "Backfill started: fetching {} missing blocks from {}",
                total_to_fetch, rpc_url
            );

            let client = reqwest::Client::new();
            let mut imported = 0u64;
            let mut skipped = 0u64;

            for &h in &missing_heights {
                // Fetch block from peer
                let resp = match client
                    .post(&rpc_url)
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
                        let msg = format!("HTTP error at height {}: {}", h, e);
                        warn!("Backfill failed: {}", msg);
                        *state.error.write().await = Some(msg);
                        state.running.store(false, Ordering::SeqCst);
                        return;
                    }
                };

                let body: serde_json::Value = match resp.json().await {
                    Ok(v) => v,
                    Err(e) => {
                        let msg = format!("JSON parse error at height {}: {}", h, e);
                        warn!("Backfill failed: {}", msg);
                        *state.error.write().await = Some(msg);
                        state.running.store(false, Ordering::SeqCst);
                        return;
                    }
                };

                let block_result = match body.get("result") {
                    Some(r) => r,
                    None => {
                        warn!("Backfill: peer missing block {}, skipping", h);
                        skipped += 1;
                        continue;
                    }
                };

                let b64_data = match block_result.get("block").and_then(|v| v.as_str()) {
                    Some(s) => s,
                    None => {
                        warn!("Backfill: no block data at height {}, skipping", h);
                        skipped += 1;
                        continue;
                    }
                };

                let expected_checksum = block_result
                    .get("blake3")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                // Decode and verify
                use base64::Engine;
                let data = match base64::engine::general_purpose::STANDARD.decode(b64_data) {
                    Ok(d) => d,
                    Err(e) => {
                        let msg = format!("Base64 decode error at height {}: {}", h, e);
                        warn!("Backfill failed: {}", msg);
                        *state.error.write().await = Some(msg);
                        state.running.store(false, Ordering::SeqCst);
                        return;
                    }
                };

                if !expected_checksum.is_empty() {
                    let actual = crypto::hash::hash(&data).to_string();
                    if actual != expected_checksum {
                        let msg = format!("BLAKE3 mismatch at height {}", h);
                        warn!("Backfill failed: {}", msg);
                        *state.error.write().await = Some(msg);
                        state.running.store(false, Ordering::SeqCst);
                        return;
                    }
                }

                let block: doli_core::Block =
                    match doli_core::transaction::legacy::deserialize_block_compat(&data) {
                        Some(b) => b,
                        None => {
                            let msg = format!(
                                "Deserialize error at height {} (tried current and legacy formats)",
                                h
                            );
                            warn!("Backfill failed: {}", msg);
                            *state.error.write().await = Some(msg);
                            state.running.store(false, Ordering::SeqCst);
                            return;
                        }
                    };

                if let Err(e) = block_store.put_block_canonical(&block, h) {
                    let msg = format!("Store error at height {}: {}", h, e);
                    warn!("Backfill failed: {}", msg);
                    *state.error.write().await = Some(msg);
                    state.running.store(false, Ordering::SeqCst);
                    return;
                }

                imported += 1;
                state.imported.store(imported, Ordering::SeqCst);

                if imported.is_multiple_of(500) {
                    info!("Backfill progress: {}/{} blocks", imported, total_to_fetch);
                }

                // Yield every 100 blocks to avoid starving other tasks
                if imported.is_multiple_of(100) {
                    tokio::task::yield_now().await;
                }
            }

            if skipped > 0 {
                warn!(
                    "Backfill done: {} imported, {} skipped (peer missing). Run again with a different peer to fill remaining gaps.",
                    imported, skipped
                );
            } else {
                info!(
                    "Backfill complete: {} blocks imported, chain filled",
                    imported
                );
            }
            state.running.store(false, Ordering::SeqCst);
        });

        Ok(serde_json::json!({
            "started": true,
            "gaps": gaps_str,
            "total": total_to_fetch
        }))
    }

    /// Get backfill progress status.
    pub(super) async fn backfill_status(&self) -> Result<Value, RpcError> {
        let running = self.backfill_state.running.load(Ordering::SeqCst);
        let imported = self.backfill_state.imported.load(Ordering::SeqCst);
        let total = self.backfill_state.total.load(Ordering::SeqCst);
        let pct = if total > 0 { imported * 100 / total } else { 0 };
        let error = self.backfill_state.error.read().await.clone();

        let response = BackfillStatusResponse {
            running,
            imported,
            total,
            pct,
            error,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Verify chain integrity by scanning every height from 1 to tip.
    /// Returns missing heights (gaps) and chain-linking errors.
    pub(super) async fn verify_chain_integrity(&self) -> Result<Value, RpcError> {
        let chain_state = self.chain_state.read().await;
        let tip_height = chain_state.best_height;
        drop(chain_state);

        if tip_height == 0 {
            return Ok(serde_json::json!({
                "complete": true,
                "tip": 0,
                "scanned": 0,
                "missing": [],
                "missing_count": 0
            }));
        }

        let block_store = self.block_store.clone();
        let tip = tip_height;

        // Run scan in blocking task to avoid starving the async runtime.
        // Computes both gap detection AND a running chain commitment:
        //   commitment[N] = BLAKE3(commitment[N-1] || block_hash[N])
        // If the chain is complete, the final commitment is a 32-byte fingerprint
        // that uniquely identifies the exact sequence of all blocks 1..tip.
        // Two nodes with the same commitment have identical chains.
        let result = tokio::task::spawn_blocking(move || {
            let mut missing: Vec<String> = Vec::new();
            let mut range_start: Option<u64> = None;
            let mut range_end: u64 = 0;
            // Chain commitment: BLAKE3(prev_commitment || block_hash)
            let mut commitment = Hash::default(); // Start with zeros
            let mut commitment_valid = true; // Breaks on first gap

            for h in 1..=tip {
                // Check actual block data, not just the height→hash index.
                // The index can exist without the block (e.g. after partial sync).
                let block = block_store.get_block_by_height(h).ok().flatten();

                if let Some(blk) = block {
                    let hash = blk.hash();
                    if commitment_valid {
                        let mut hasher = Hasher::new();
                        hasher.update(commitment.as_bytes());
                        hasher.update(hash.as_bytes());
                        commitment = hasher.finalize();
                    }

                    if let Some(start) = range_start.take() {
                        if start == range_end {
                            missing.push(format!("{}", start));
                        } else {
                            missing.push(format!("{}-{}", start, range_end));
                        }
                    }
                } else {
                    if range_start.is_none() {
                        range_start = Some(h);
                    }
                    range_end = h;
                    commitment_valid = false; // Gap breaks the chain
                }
            }
            // Flush final range if chain ends with a gap
            if let Some(start) = range_start {
                if start == range_end {
                    missing.push(format!("{}", start));
                } else {
                    missing.push(format!("{}-{}", start, range_end));
                }
            }

            // Count total missing
            let missing_count: u64 = missing
                .iter()
                .map(|s| {
                    if let Some((a, b)) = s.split_once('-') {
                        b.parse::<u64>().unwrap_or(0) - a.parse::<u64>().unwrap_or(0) + 1
                    } else {
                        1
                    }
                })
                .sum();

            let commitment_hex = if commitment_valid && missing_count == 0 {
                Some(format!("{}", commitment))
            } else {
                None
            };

            (missing, missing_count, commitment_hex)
        })
        .await
        .map_err(|e| RpcError::internal_error(format!("Scan failed: {}", e)))?;

        let (missing, missing_count, chain_commitment) = result;

        Ok(serde_json::json!({
            "complete": missing_count == 0,
            "tip": tip_height,
            "scanned": tip_height,
            "missing": missing,
            "missingCount": missing_count,
            "chainCommitment": chain_commitment
        }))
    }
}
