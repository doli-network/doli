//! Backfill and chain integrity handlers: backfillFromPeer, backfillStatus, verifyChainIntegrity

use std::net::IpAddr;
use std::sync::atomic::Ordering;

use serde_json::Value;
use tracing::{info, warn};

use crypto::{Hash, Hasher};

use crate::error::RpcError;
use crate::types::*;

use super::context::RpcContext;

#[path = "backfill_anchor.rs"]
mod anchor;
#[path = "backfill_peer_commitment.rs"]
mod peer_commitment;

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
pub(super) fn validate_backfill_url(raw_url: &str, network: &str) -> Result<(), RpcError> {
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

        validate_backfill_url(&params.rpc_url, &self.network)?;

        if self.backfill_state.running.load(Ordering::SeqCst) {
            return Err(RpcError::internal_error(
                "Backfill already in progress. Use backfillStatus to check progress.",
            ));
        }

        let (tip_height, tip_hash) = {
            let cs = self.chain_state.read().await;
            (cs.best_height, cs.best_hash)
        };
        if tip_height == 0 {
            return Err(RpcError::internal_error("Node has no blocks yet"));
        }

        // Preflight only saves time; the anchor walk is the safety gate (INC-I-308 R-1).
        if params.skip_divergence_check {
            info!("Backfill: skip_divergence_check=true — bypassing tip agreement preflight");
        } else {
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

        // Scan: missing heights, plus stored heights whose child does not link to them.
        let block_store_scan = self.block_store.clone();
        let peer_url = params.rpc_url.clone();
        let (todo, local_commitment) = tokio::task::spawn_blocking(move || {
            let mut todo = Vec::new();
            let mut commitment = crypto::Hash::default();
            let mut last: Option<(u64, Hash)> = None;
            for h in 1..=tip_height {
                match block_store_scan.get_block_by_height(h) {
                    Ok(Some(blk)) => {
                        let hash = blk.hash();
                        if let Some((ph, phash)) = last {
                            if ph + 1 == h && blk.header.prev_hash != phash {
                                todo.push(ph);
                            }
                        }
                        last = Some((h, hash));
                        let mut hasher = Hasher::new();
                        hasher.update(commitment.as_bytes());
                        hasher.update(hash.as_bytes());
                        commitment = hasher.finalize();
                    }
                    _ => todo.push(h),
                }
            }
            if let Some((lh, lhash)) = last {
                if lh == tip_height && lhash != tip_hash {
                    todo.push(tip_height);
                }
            }
            todo.sort_unstable();
            todo.dedup();
            (todo, commitment)
        })
        .await
        .map_err(|e| RpcError::internal_error(e.to_string()))?;

        if todo.is_empty() {
            let token = params.peer_token.as_deref();
            let check = peer_commitment::report(
                &self.block_store,
                &peer_url,
                token,
                local_commitment,
                tip_height,
            )
            .await;
            let message = if check == "ok" {
                "No gaps detected — chain is complete.".to_string()
            } else {
                format!("No gaps detected — chain not verified against the peer ({check}).")
            };
            return Ok(serde_json::json!({
                "started": false,
                "message": message,
                "divergence_check": check
            }));
        }

        let total_to_fetch = todo.len() as u64;
        let gaps_str = anchor::format_ranges(&todo);

        self.backfill_state.imported.store(0, Ordering::SeqCst);
        self.backfill_state
            .total
            .store(total_to_fetch, Ordering::SeqCst);
        *self.backfill_state.error.write().await = None;
        self.backfill_state.running.store(true, Ordering::SeqCst);

        let walk = anchor::Walk {
            store: self.block_store.clone(),
            state: self.backfill_state.clone(),
            url: params.rpc_url.clone(),
            tip_height,
            tip_hash,
            sig_ah: self.inc_i_236_block_signature_activation_height,
            bls_ah: self.inc_i_178_attestation_bls_activation_height,
        };
        info!(
            "Backfill started: {} heights to check from {}",
            total_to_fetch, params.rpc_url
        );
        tokio::spawn(walk.run(todo));

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
