//! Response handling — handle_response, handle_headers_response, handle_bodies_response,
//! get_blocks_to_apply, preserve_valid_sync_data

use std::time::Instant;

use libp2p::PeerId;
use tracing::{debug, info, warn};

use doli_core::{Block, BlockHeader};

use crate::protocols::SyncResponse;
use crate::sync::manager::{SyncManager, SyncPhase, SyncPipelineData, SyncState};

impl SyncManager {
    /// Handle a sync response
    pub fn handle_response(&mut self, peer: PeerId, response: SyncResponse) -> Vec<Block> {
        info!(
            "[SYNC_DEBUG] Processing response from peer={}, type={:?}, state={:?}",
            peer,
            std::mem::discriminant(&response),
            self.state
        );

        // Clear pending request for this peer and remove from tracking map.
        // Clear pending request for this peer and extract the epoch.
        // Only discard when the epoch is EXPLICITLY from a different (older) epoch.
        // None (no tracking info, e.g. after soft retry cleared requests) is NOT stale.
        let request_epoch = if let Some(status) = self.peers.get_mut(&peer) {
            if let Some(req_id) = status.pending_request.take() {
                self.pipeline
                    .pending_requests
                    .remove(&req_id)
                    .map(|r| r.epoch)
            } else {
                None
            }
        } else {
            None
        };

        let is_stale_epoch = matches!(request_epoch, Some(e) if e != self.pipeline.sync_epoch);

        match response {
            SyncResponse::Headers(headers) => {
                info!(
                    "[SYNC_DEBUG] Handling headers response: count={}",
                    headers.len()
                );
                // Discard responses from old sync epochs. Responses with no tracking
                // (request_epoch=None, e.g. after soft retry cleared requests) are
                // allowed through — the header chain linkage check in process_headers
                // is the definitive validation.
                if is_stale_epoch {
                    debug!(
                        "Discarding stale headers from {} ({} headers) — epoch {:?} != current {}",
                        peer,
                        headers.len(),
                        request_epoch,
                        self.pipeline.sync_epoch
                    );
                    return vec![];
                }
                self.handle_headers_response(peer, headers);
                vec![]
            }
            SyncResponse::Bodies(bodies) => {
                info!(
                    "[SYNC_DEBUG] Handling bodies response: count={}",
                    bodies.len()
                );
                if is_stale_epoch {
                    debug!(
                        "Discarding stale bodies from {} ({} bodies) — epoch {:?} != current {}",
                        peer,
                        bodies.len(),
                        request_epoch,
                        self.pipeline.sync_epoch
                    );
                    return vec![];
                }
                self.handle_bodies_response(peer, bodies)
            }
            SyncResponse::Block(maybe_block) => {
                info!(
                    "[SYNC_DEBUG] Handling block response: has_block={}",
                    maybe_block.is_some()
                );
                // Fork recovery intercept: consume blocks during active recovery
                if self.fork.fork_recovery.is_active()
                    && self
                        .fork
                        .fork_recovery
                        .handle_block(peer, maybe_block.clone())
                {
                    return vec![]; // Consumed by fork recovery
                }
                // Normal passthrough
                if let Some(block) = maybe_block {
                    vec![block]
                } else {
                    vec![]
                }
            }
            SyncResponse::Error(err) => {
                warn!("[SYNC_DEBUG] Sync error from peer {}: {}", peer, err);
                // INC-I-012 F2: Phase-aware error routing.
                // Previously ALL errors routed to snap download error handler,
                // which silently swallowed "busy" errors during header-first sync.
                if matches!(self.pipeline_data, SyncPipelineData::SnapDownloading { .. }) {
                    self.handle_snap_download_error(peer);
                } else if err.contains("busy") {
                    // Peer is rate-limited — temporarily blacklist and retry
                    // immediately with a different peer instead of waiting 30s
                    // for the next cleanup tick.
                    self.fork
                        .header_blacklisted_peers
                        .insert(peer, Instant::now());
                    self.set_state(SyncState::Idle, "peer_busy_retry");
                    if self.should_sync() {
                        self.start_sync();
                    }
                } else {
                    // Unknown error — go idle, let cleanup retry
                    self.set_state(SyncState::Idle, "sync_error");
                }
                vec![]
            }
            SyncResponse::StateSnapshot {
                block_hash,
                block_height,
                chain_state,
                utxo_set,
                producer_set,
                state_root,
            } => {
                info!(
                    "[SNAP_SYNC] Received state snapshot from peer {}: hash={}, height={}, root={}, size={}KB",
                    peer, block_hash, block_height, state_root,
                    (chain_state.len() + utxo_set.len() + producer_set.len()) / 1024
                );
                self.handle_snap_snapshot(
                    peer,
                    block_hash,
                    block_height,
                    chain_state,
                    utxo_set,
                    producer_set,
                    state_root,
                );
                vec![]
            }
            SyncResponse::StateRoot {
                block_hash,
                block_height,
                state_root,
            } => {
                info!(
                    "[SNAP_SYNC] Received state root from peer {}: hash={}, height={}, root={}",
                    peer, block_hash, block_height, state_root
                );
                self.handle_snap_state_root(peer, block_hash, block_height, state_root);
                vec![]
            }
        }
    }

    /// Handle headers response
    fn handle_headers_response(&mut self, peer: PeerId, headers: Vec<BlockHeader>) {
        if headers.is_empty() {
            debug!("Received empty headers response from {}", peer);

            // No more headers, transition to body download
            if !self.pipeline.headers_needing_bodies.is_empty() {
                let total = self.pipeline.headers_needing_bodies.len();
                self.set_syncing(
                    SyncPhase::DownloadingBodies,
                    SyncPipelineData::Bodies { pending: 0, total },
                    "headers_complete",
                );
                info!("Starting body download for {} blocks", total);
            } else if self.pipeline.pending_headers.is_empty() {
                let peer_height = self.peers.get(&peer).map(|p| p.best_height).unwrap_or(0);
                let gap = peer_height.saturating_sub(self.local_height);

                // INC-I-012 F1: Post-snap empty headers — use height-based request.
                // After snap sync, local_hash may be from a forked peer that no
                // canonical peer recognizes. Instead of retrying snap sync (which
                // may snap to the same fork), switch to GetHeadersByHeight which
                // bypasses the hash lookup entirely. The peer serves canonical
                // headers from local_height+1, giving us a valid chain anchor.
                //
                // Guard: only attempt height-based fallback ONCE. If the height-
                // based request also returned empty (peer at same height or below),
                // fall through to normal fork detection to avoid a 60s retry loop.
                if matches!(
                    self.recovery_phase,
                    super::super::RecoveryPhase::AwaitingCanonicalBlock { .. }
                ) && self.snap.threshold < u64::MAX
                    && !self.fork.height_fallback_attempted
                {
                    warn!(
                        "Post-snap empty headers from {} (gap={}) — snap hash not recognized. \
                         Switching to height-based headers (INC-I-012 F1).",
                        peer, gap
                    );
                    // Don't blacklist — the peer is correct, our hash is wrong
                    // Don't increment consecutive_empty_headers — not fork evidence
                    self.fork.use_height_based_headers = true;
                    self.fork.height_fallback_attempted = true;
                    self.set_state(SyncState::Idle, "post_snap_height_fallback");
                    if self.should_sync() {
                        self.start_sync();
                    }
                    return;
                }

                // Empty headers = peer doesn't recognize our tip hash.
                // Two cases:
                // 1. Small gap (<=50): we're on a minor fork. Signal immediate
                //    rollback — each rollback tries the parent hash until peers
                //    recognize it. Recovery in seconds, not minutes.
                // 2. Large gap (>50): too deep for rollback. Accumulate fork
                //    evidence for snap sync escalation.
                self.fork.consecutive_empty_headers += 1;
                // NOTE: Do NOT inflate consecutive_sync_failures here.
                // Empty headers = peer doesn't recognize our hash (fork evidence),
                // not a sync failure. Incrementing sync_failures on empties
                // causes premature snap sync escalation during shallow forks.

                if gap <= 50 && self.local_height > 0 {
                    // Small fork: signal rollback. The node's periodic task
                    // (resolve_shallow_fork) will roll back 1 block per tick,
                    // changing local_hash to the parent. After 1-3 rollbacks,
                    // GetHeaders succeeds and sync resumes normally.
                    warn!(
                        "Empty headers from {} (gap={}, consecutive={}) — minor fork. \
                         Signaling rollback to find common ancestor.",
                        peer, gap, self.fork.consecutive_empty_headers
                    );
                    // Set to 3 immediately to trigger resolve_shallow_fork
                    // on the next periodic tick (no waiting for 3 separate responses)
                    self.fork.consecutive_empty_headers =
                        self.fork.consecutive_empty_headers.max(3);
                    self.set_state(SyncState::Idle, "small_fork_signal_rollback");
                    return;
                }

                if self.fork.consecutive_empty_headers >= 3 {
                    // 3+ consecutive empties from different peers = WE are on a fork.
                    // Stop blacklisting — removing canonical peers prevents recovery.
                    // Clear any existing blacklist and signal fork recovery.
                    warn!(
                        "Empty headers from {} (peer_h={}, local_h={}, gap={}, consecutive={}) — \
                         node is on a fork (3+ peers reject our chain). Clearing blacklist.",
                        peer,
                        peer_height,
                        self.local_height,
                        gap,
                        self.fork.consecutive_empty_headers
                    );
                    self.fork.header_blacklisted_peers.clear();
                    self.signal_stuck_fork();
                } else {
                    // First 1-2 empties: could be a peer-specific issue.
                    // INC-I-012 F7: Skip blacklisting if snap sync completed
                    // within the last 5 minutes — ALL canonical peers will return
                    // empty for our unrecognizable hash, so blacklisting just
                    // removes good peers from the pool.
                    let recently_snapped = self
                        .snap
                        .last_snap_completed
                        .map(|t| t.elapsed().as_secs() < 300)
                        .unwrap_or(false);
                    if !recently_snapped {
                        self.fork
                            .header_blacklisted_peers
                            .insert(peer, Instant::now());
                    }
                    warn!(
                        "Empty headers from {} (peer_h={}, local_h={}, gap={}) — \
                         fork evidence (consecutive={}){}.",
                        peer,
                        peer_height,
                        self.local_height,
                        gap,
                        self.fork.consecutive_empty_headers,
                        if recently_snapped {
                            " (skipping blacklist — recent snap sync)"
                        } else {
                            ". Blacklisted peer"
                        }
                    );
                }
                self.set_state(SyncState::Idle, "empty_headers_fork_detected");
            } else {
                self.set_state(SyncState::Synchronized, "headers_empty_already_synced");
                self.fork.header_blacklisted_peers.clear();
                info!("Chain synchronized");
            }
            return;
        }

        info!("Received {} headers from {}", headers.len(), peer);

        // Validate and store headers
        let valid_count = self
            .pipeline
            .header_downloader
            .process_headers(&headers, self.local_hash);

        if valid_count > 0 {
            // Successfully received valid headers — reset fork counters
            self.fork.consecutive_empty_headers = 0;
            self.fork.consecutive_sync_failures = 0;
            self.fork.height_fallback_attempted = false;
            self.network.last_sync_activity = Instant::now();

            // Queue header hashes for body download
            for header in headers.iter().take(valid_count) {
                self.pipeline
                    .headers_needing_bodies
                    .push_back(header.hash());
                self.pipeline.pending_headers.push_back(header.clone());
            }

            // Update pipeline data with new headers count
            if let SyncPipelineData::Headers {
                target_slot,
                peer,
                headers_count,
            } = &self.pipeline_data
            {
                self.pipeline_data = SyncPipelineData::Headers {
                    target_slot: *target_slot,
                    peer: *peer,
                    headers_count: headers_count + valid_count,
                };
            }
        } else {
            // CHAIN BREAK: Received headers but none chain to expected_prev_hash.
            // This is fork evidence — the peer has a different chain at our tip height.
            // process_headers() does NOT modify expected_prev_hash when valid_count == 0,
            // so the downloader state is still correct.
            self.fork.consecutive_empty_headers += 1;
            self.fork.consecutive_sync_failures += 1;
            warn!(
                "No valid headers from peer {} - header chain broken (consecutive={}). \
                 Peer has different chain at our tip.",
                peer, self.fork.consecutive_empty_headers
            );
        }
    }

    /// Handle bodies response
    fn handle_bodies_response(&mut self, peer: PeerId, bodies: Vec<Block>) -> Vec<Block> {
        // CRITICAL: Process through body_downloader first to:
        // 1. Clear the active request for this peer (allows more requests to this peer)
        // 2. Remove received hashes from in_flight set
        // 3. Mark any missing hashes as failed for retry
        if bodies.is_empty() {
            debug!("Received empty bodies response from {}", peer);
            // Still need to clear the active request even for empty response
            self.pipeline.body_downloader.process_response(peer, vec![]);
            return vec![];
        }

        info!("Received {} bodies from {}", bodies.len(), peer);
        self.network.last_sync_activity = Instant::now();

        // Process through body_downloader to update its internal state
        let processed_bodies = self.pipeline.body_downloader.process_response(peer, bodies);

        // Remove these hashes from needed list and store blocks
        let mut blocks_to_apply = Vec::new();

        for block in processed_bodies {
            let hash = block.hash();
            self.pipeline.headers_needing_bodies.retain(|h| h != &hash);

            // Store block for later application
            self.pipeline.pending_blocks.insert(hash, block.clone());
            blocks_to_apply.push(block);
        }

        // Update state
        let remaining = self.pipeline.headers_needing_bodies.len();
        if remaining == 0 {
            self.set_syncing(
                SyncPhase::ProcessingBlocks,
                SyncPipelineData::Processing {
                    height: self.local_height + 1,
                },
                "bodies_complete",
            );
            info!("All bodies downloaded, starting processing");
        } else if let SyncPipelineData::Bodies { total, .. } = &self.pipeline_data {
            let total_copy = *total; // Copy before reassigning
            self.set_syncing(
                SyncPhase::DownloadingBodies,
                SyncPipelineData::Bodies {
                    pending: remaining,
                    total: total_copy,
                },
                "bodies_progress",
            );
            debug!(
                "Body download progress: {} remaining of {} total",
                remaining, total_copy
            );
        }

        blocks_to_apply
    }

    /// Get blocks ready to be applied (in order)
    pub fn get_blocks_to_apply(&mut self) -> Vec<Block> {
        let mut blocks = Vec::new();
        let mut current_hash = self.local_hash;

        // Detect chain mismatch: if we have pending headers but the first one
        // doesn't build on our local chain, we're on a fork. The downloaded
        // chain is useless — clear it and signal fork so deep fork detection
        // can trigger genesis resync.
        //
        // IMPORTANT: Only trigger during Processing state (all bodies downloaded).
        // During DownloadingBodies, bodies arrive out of order — the first header's
        // body may not have arrived yet, but that's normal. Clearing everything
        // during active body download causes an infinite retry loop: download 13K
        // headers → download partial bodies → mismatch → clear → repeat forever.
        let in_processing = matches!(self.pipeline_data, SyncPipelineData::Processing { .. });
        if let Some(header) = self.pipeline.pending_headers.front() {
            if header.prev_hash != current_hash && in_processing {
                warn!(
                    "Sync chain mismatch: first pending header (slot {}, prev={}) doesn't \
                     build on local tip (hash={}). Clearing {} useless synced blocks.",
                    header.slot,
                    &header.prev_hash.to_string()[..16],
                    &current_hash.to_string()[..16],
                    self.pipeline.pending_headers.len()
                );
                self.pipeline.pending_headers.clear();
                self.pipeline.pending_blocks.clear();
                self.pipeline.headers_needing_bodies.clear();
                self.pipeline.header_downloader.clear();
                // Count toward deep fork detection so genesis resync triggers
                self.fork.consecutive_empty_headers += 1;
                // Always reset to Idle — staying in DownloadingBodies with
                // empty headers_needing_bodies leaves the body downloader in
                // a zombie state (needed=0, no requests, waits 120s for soft
                // recovery). Idle lets cleanup() start a fresh sync immediately.
                self.set_state(SyncState::Idle, "chain_mismatch_detected");
                return blocks;
            }
        }

        // Get pending headers in order
        while let Some(header) = self.pipeline.pending_headers.front() {
            if header.prev_hash != current_hash {
                break;
            }

            let hash = header.hash();
            if let Some(block) = self.pipeline.pending_blocks.remove(&hash) {
                current_hash = hash;
                blocks.push(block);
                self.pipeline.pending_headers.pop_front();
            } else {
                // Don't have the body yet
                break;
            }
        }

        // INC-001 RC-6: PROCESSING STALL FIX
        // If we're in Processing state but extracted ZERO blocks, nothing will call
        // block_applied_with_weight() to transition us out. The 30s stuck timeout in
        // cleanup is too slow — 3 slots are wasted per stall. Reset to Idle immediately
        // so the node can resume production and process gossip blocks.
        if blocks.is_empty() && matches!(self.pipeline_data, SyncPipelineData::Processing { .. }) {
            warn!(
                "[PROCESSING_STALL] No blocks extractable in Processing state \
                 (pending_headers={}, pending_blocks={}, local_h={}, local_hash={:.12}) \
                 — resetting to Idle immediately",
                self.pipeline.pending_headers.len(),
                self.pipeline.pending_blocks.len(),
                self.local_height,
                self.local_hash,
            );
            self.pipeline.pending_headers.clear();
            self.pipeline.pending_blocks.clear();
            self.set_state(SyncState::Idle, "processing_stall_reset");
            // Don't start a new sync here — let cleanup() handle it on the next tick
            // to avoid recursive lock issues.
        }

        blocks
    }

    /// Preserve valid downloaded sync data across a sync restart.
    ///
    /// When sync restarts (timeout, peer disconnect, etc.), instead of clearing
    /// ALL accumulated headers and bodies, this walks pending_headers and keeps
    /// the valid prefix that chains from our current local_hash.
    ///
    /// Returns true if any data was preserved, false if everything was stale/forked.
    pub(in crate::sync::manager) fn preserve_valid_sync_data(&mut self) -> bool {
        if self.pipeline.pending_headers.is_empty() {
            return false;
        }

        let local_hash = self.local_hash;

        // Find the first header that chains from our current tip.
        // Headers before this point have already been applied or are on a dead fork.
        let chain_start = self
            .pipeline
            .pending_headers
            .iter()
            .position(|h| h.prev_hash == local_hash);

        let chain_start = match chain_start {
            Some(idx) => idx,
            None => {
                // No header chains from local_hash — data is from a different fork
                info!(
                    "[SYNC_PRESERVE] No headers chain from local_hash={:.12} — discarding all {} headers",
                    self.local_hash,
                    self.pipeline.pending_headers.len()
                );
                return false;
            }
        };

        let total_before = self.pipeline.pending_headers.len();

        // Remove stale headers (already applied or on dead fork)
        for _ in 0..chain_start {
            if let Some(stale) = self.pipeline.pending_headers.pop_front() {
                self.pipeline.pending_blocks.remove(&stale.hash());
            }
        }

        // Rebuild headers_needing_bodies from surviving headers
        self.pipeline.headers_needing_bodies.clear();
        for header in &self.pipeline.pending_headers {
            let hash = header.hash();
            if !self.pipeline.pending_blocks.contains_key(&hash) {
                self.pipeline.headers_needing_bodies.push_back(hash);
            }
        }

        // Resume header downloader from the last preserved header
        if let Some(last) = self.pipeline.pending_headers.back() {
            self.pipeline.header_downloader.resume_from(last.hash());
        } else {
            self.pipeline.header_downloader.clear();
        }

        let preserved = self.pipeline.pending_headers.len();
        let bodies_have = self.pipeline.pending_blocks.len();
        let bodies_need = self.pipeline.headers_needing_bodies.len();

        info!(
            "[SYNC_PRESERVE] Preserved {}/{} headers ({} stale removed), \
             {} bodies cached, {} bodies still needed",
            preserved, total_before, chain_start, bodies_have, bodies_need
        );

        preserved > 0
    }
}
