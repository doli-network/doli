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
        let (tip_height, tip_hash) = {
            let cs = self.chain_state.read().await;
            (cs.best_height, cs.best_hash)
        };
        if tip_height == 0 {
            return Err(RpcError::internal_error("Node has no blocks yet"));
        }

        // Fix #3 (2026-04-15): preflight tip agreement check.
        // backfillFromPeer only fills block_store gaps — it does NOT reapply state.
        // If our tip hash differs from the peer's block at our tip height, we have a
        // state divergence (e.g. from a rollback cascade). Running backfill in that
        // case fills "holes" but leaves bond_snapshot / accumulators / UTXO diverged,
        // producing a FALSE integrity signal ("chain complete") while consensus
        // state is still wrong. Prevent that here.
        {
            let client = reqwest::Client::new();
            match client
                .post(&params.rpc_url)
                .json(&serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "getBlockByHeight",
                    "params": { "height": tip_height },
                    "id": 0
                }))
                .send()
                .await
            {
                Ok(resp) => {
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        if let Some(peer_hash_str) =
                            body.pointer("/result/hash").and_then(|v| v.as_str())
                        {
                            let our_hash_str = tip_hash.to_string();
                            if peer_hash_str != our_hash_str {
                                return Err(RpcError::invalid_params(format!(
                                    "Tip divergence detected — our h={} hash={:.16} vs peer hash={:.16}. \
                                     Backfill only fills block_store gaps; it cannot repair state \
                                     divergence. Use snap sync or wipe+rsync from a canonical peer.",
                                    tip_height, our_hash_str, peer_hash_str
                                )));
                            }
                        } else {
                            warn!(
                                "Backfill preflight: peer did not return a tip hash for h={} — \
                                 proceeding without divergence check",
                                tip_height
                            );
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        "Backfill preflight: could not reach peer for tip check ({}) — \
                         proceeding without divergence check",
                        e
                    );
                }
            }
        }

        // Full scan: find ALL missing heights AND divergent blocks.
        // Phase 1: detect gaps (missing block data)
        // Phase 2: compare chain commitment with peer — if different,
        //          find divergent heights by comparing block hashes.
        let block_store_scan = self.block_store.clone();
        let peer_url = params.rpc_url.clone();
        let (missing_heights, local_commitment) = tokio::task::spawn_blocking(move || {
            let mut missing = Vec::new();
            let mut commitment = crypto::Hash::default();
            for h in 1..=tip_height {
                match block_store_scan.get_block_by_height(h) {
                    Ok(Some(blk)) => {
                        let hash = blk.hash();
                        let mut hasher = Hasher::new();
                        hasher.update(commitment.as_bytes());
                        hasher.update(hash.as_bytes());
                        commitment = hasher.finalize();
                    }
                    _ => {
                        missing.push(h);
                    }
                }
            }
            (missing, commitment)
        })
        .await
        .map_err(|e| RpcError::internal_error(e.to_string()))?;

        // Phase 2: check peer commitment — if different, find divergent heights
        let mut repair_heights: Vec<u64> = Vec::new();
        if missing_heights.is_empty() {
            let client = reqwest::Client::new();
            if let Ok(resp) = client
                .post(&peer_url)
                .json(&serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "verifyChainIntegrity",
                    "params": [],
                    "id": 1
                }))
                .send()
                .await
            {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    let peer_commitment = body["result"]["chainCommitment"].as_str().unwrap_or("");
                    let local_hex = hex::encode(local_commitment.as_bytes());
                    if !peer_commitment.is_empty() && peer_commitment != local_hex {
                        info!(
                            "Backfill: commitment mismatch (local={:.16} peer={:.16}), scanning for divergent blocks",
                            local_hex, peer_commitment
                        );
                        // Reverse scan: walk backwards from tip to find the fork point.
                        // Most forks are recent — this finds divergence in O(depth) not O(n).
                        // Once we find a matching block, everything below is identical.
                        let block_store_repair = self.block_store.clone();
                        let mut fork_point = tip_height + 1;
                        for h in (1..=tip_height).rev() {
                            if let Ok(Some(local_block)) = block_store_repair.get_block_by_height(h)
                            {
                                if let Ok(resp) = client
                                    .post(&peer_url)
                                    .json(&serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "method": "getBlockByHeight",
                                        "params": [h],
                                        "id": h
                                    }))
                                    .send()
                                    .await
                                {
                                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                                        let peer_hash =
                                            body["result"]["hash"].as_str().unwrap_or("");
                                        let local_hash = local_block.hash().to_hex();
                                        if peer_hash.is_empty() {
                                            continue;
                                        }
                                        if peer_hash == local_hash {
                                            break;
                                        }
                                        fork_point = h;
                                    }
                                }
                            }
                        }
                        if fork_point <= tip_height {
                            info!(
                                "Backfill repair: fork diverges at h={}, refetching h={}..={}",
                                fork_point, fork_point, tip_height
                            );
                            repair_heights.extend(fork_point..=tip_height);
                        }
                        if repair_heights.is_empty() {
                            info!("Backfill: commitment differs but no individual block divergence found (height difference)");
                        }
                    }
                }
            }
        }

        let all_heights: Vec<u64> = {
            let mut combined = missing_heights.clone();
            combined.extend(&repair_heights);
            combined.sort_unstable();
            combined.dedup();
            combined
        };

        if all_heights.is_empty() {
            return Ok(serde_json::json!({
                "started": false,
                "message": "No gaps detected — chain is complete."
            }));
        }

        let missing_heights = all_heights;

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
        let pct = (imported * 100).checked_div(total).unwrap_or(0);
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

    /// Verify chain integrity by scanning every height from 1 to tip (or up_to_height).
    /// Returns missing heights (gaps) and chain-linking errors.
    ///
    /// Optional param: `up_to_height` (u64) — scan blocks 1..=up_to_height instead of
    /// 1..=tip. Allows the explorer to request all nodes compute their commitment at the
    /// same height, making hashes directly comparable.
    pub(super) async fn verify_chain_integrity(&self, params: Value) -> Result<Value, RpcError> {
        let chain_state = self.chain_state.read().await;
        let tip_height = chain_state.best_height;
        drop(chain_state);

        // Parse optional up_to_height from params: [height] or {"up_to_height": height}
        let up_to_height = params
            .get(0)
            .or_else(|| params.get("up_to_height"))
            .and_then(|v| v.as_u64());

        // Parse optional from_height: [up_to_height, from_height] or {"from_height": height}
        let from_height = params
            .get(1)
            .or_else(|| params.get("from_height"))
            .and_then(|v| v.as_u64());

        let scan_ceiling = match up_to_height {
            Some(h) if h > tip_height => {
                return Err(RpcError::invalid_params(format!(
                    "up_to_height {} exceeds current tip {}",
                    h, tip_height
                )));
            }
            Some(0) => {
                return Ok(serde_json::json!({
                    "complete": true,
                    "tip": tip_height,
                    "scanned": 0,
                    "fromHeight": from_height.unwrap_or(1),
                    "missing": [],
                    "missingCount": 0
                }));
            }
            Some(h) => h,
            None => tip_height,
        };

        let scan_floor = from_height.unwrap_or(1).max(1); // minimum 1
        if scan_floor > scan_ceiling {
            return Err(RpcError::invalid_params(format!(
                "from_height {} exceeds up_to_height/tip {}",
                scan_floor, scan_ceiling
            )));
        }

        if tip_height == 0 {
            return Ok(serde_json::json!({
                "complete": true,
                "tip": 0,
                "scanned": 0,
                "fromHeight": scan_floor,
                "missing": [],
                "missingCount": 0
            }));
        }

        let is_full_scan = scan_floor == 1;

        // Fast path: if periodic commitment exists AND covers the requested height,
        // return it in O(1). The commitment is recomputed every 100 blocks via full
        // BLAKE3 scan in periodic.rs. Always correct by construction.
        // Only usable when no specific up_to_height was requested, or when the
        // persisted scan_tip matches the requested height exactly.
        let persisted = self
            .state_db
            .as_ref()
            .and_then(|db| db.get_chain_commitment_with_tip());
        if let Some((commitment, scan_tip)) = persisted {
            let persisted_matches = is_full_scan
                && match up_to_height {
                    None => true, // no specific height requested, use whatever we have
                    Some(h) => scan_tip > 0 && scan_tip == h, // exact match required
                };
            if persisted_matches {
                // Still need gap detection — commitment only covers applied blocks,
                // not block_store completeness. Quick scan for gaps only (no hashing).
                let block_store = self.block_store.clone();
                let ceiling = scan_ceiling;
                let floor = scan_floor;
                let gaps = tokio::task::spawn_blocking(move || {
                    let mut missing: Vec<String> = Vec::new();
                    let mut range_start: Option<u64> = None;
                    let mut range_end: u64 = 0;
                    for h in floor..=ceiling {
                        let exists = block_store
                            .get_block_by_height(h)
                            .map(|opt| opt.is_some())
                            .unwrap_or(false);
                        if !exists {
                            if range_start.is_none() {
                                range_start = Some(h);
                            }
                            range_end = h;
                        } else if let Some(start) = range_start.take() {
                            if start == range_end {
                                missing.push(format!("{}", start));
                            } else {
                                missing.push(format!("{}-{}", start, range_end));
                            }
                        }
                    }
                    if let Some(start) = range_start {
                        if start == range_end {
                            missing.push(format!("{}", start));
                        } else {
                            missing.push(format!("{}-{}", start, range_end));
                        }
                    }
                    missing
                })
                .await
                .map_err(|e| RpcError::internal_error(format!("Gap scan failed: {}", e)))?;

                let missing_count: u64 = gaps
                    .iter()
                    .map(|s| {
                        if let Some((a, b)) = s.split_once('-') {
                            b.parse::<u64>().unwrap_or(0) - a.parse::<u64>().unwrap_or(0) + 1
                        } else {
                            1
                        }
                    })
                    .sum();

                let reported_scan = if scan_tip > 0 { scan_tip } else { tip_height };

                return Ok(serde_json::json!({
                    "complete": missing_count == 0,
                    "tip": tip_height,
                    "scanned": reported_scan,
                    "fromHeight": scan_floor,
                    "missing": gaps,
                    "missingCount": missing_count,
                    "chainCommitment": format!("{}", commitment)
                }));
            }
        }

        // Compute commitment over scan_floor..=scan_ceiling.
        // Also used when up_to_height doesn't match the persisted scan_tip.
        let block_store = self.block_store.clone();
        let state_db_opt = self.state_db.clone();
        let tip = scan_ceiling;
        let floor = scan_floor;
        let full_scan = is_full_scan;
        let persist_commitment = up_to_height.is_none() && full_scan; // only persist for natural full scans

        let result = tokio::task::spawn_blocking(move || {
            let mut missing: Vec<String> = Vec::new();
            let mut range_start: Option<u64> = None;
            let mut range_end: u64 = 0;
            let mut commitment = Hash::default();
            let mut commitment_valid = true;

            for h in floor..=tip {
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
                    commitment_valid = false;
                }
            }
            if let Some(start) = range_start {
                if start == range_end {
                    missing.push(format!("{}", start));
                } else {
                    missing.push(format!("{}-{}", start, range_end));
                }
            }

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

            // Only persist for natural scans (no up_to_height). Targeted scans
            // compute a partial commitment that must not overwrite the full one.
            if commitment_valid && missing_count == 0 && persist_commitment {
                if let Some(ref db) = state_db_opt {
                    if db.get_chain_commitment().is_none() {
                        db.put_chain_commitment_with_tip(&commitment, tip);
                    }
                }
            }

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
            "scanned": scan_ceiling,
            "fromHeight": scan_floor,
            "missing": missing,
            "missingCount": missing_count,
            "chainCommitment": chain_commitment
        }))
    }
}
