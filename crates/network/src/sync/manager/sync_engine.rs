//! Sync engine — orchestrates header-first sync, body download, and request/response handling

use std::time::Instant;

use libp2p::PeerId;
use tracing::{debug, info, warn};

use crypto::Hash;
use doli_core::{Block, BlockHeader};

use super::{PendingRequest, SyncManager, SyncRequestId, SyncState};
use crate::protocols::{SyncRequest, SyncResponse};

impl SyncManager {
    /// Check if we should start syncing
    pub(super) fn should_sync(&self) -> bool {
        if self.peers.len() < self.config.min_peers_for_sync {
            return false;
        }

        // CRITICAL: Only sync if a peer has MORE BLOCKS (higher height).
        // Do NOT use slot comparison alone — peers with different genesis times
        // can have much higher slots despite having far fewer blocks. This caused
        // N4 (40K blocks) to sync from N3 (1,235 blocks, higher slot) and reorg
        // its own chain down to 37K.
        let peer_ahead = self
            .peers
            .values()
            .any(|p| p.best_height > self.local_height);

        // Also check network_tip_height (updated by gossip) since peer best_height
        // in the HashMap can be stale if no status messages are received.
        let network_ahead = self.network_tip_height > self.local_height;

        peer_ahead || network_ahead
    }

    /// Get the best peer to sync from
    pub(super) fn best_peer(&self) -> Option<PeerId> {
        // CRITICAL: Only consider peers with MORE BLOCKS (higher height).
        // Slot comparison alone is dangerous — see should_sync() comment.
        // Skip peers that recently returned empty headers (cooldown).
        self.peers
            .iter()
            .filter(|(pid, status)| {
                status.best_height > self.local_height
                    && !self.header_blacklisted_peers.contains_key(pid)
            })
            .max_by_key(|(_, status)| (status.best_height, status.best_slot))
            .map(|(peer, _)| *peer)
    }

    /// Start the sync process
    pub(super) fn start_sync(&mut self) {
        if let Some(peer) = self.best_peer() {
            let target_slot = match self.peers.get(&peer) {
                Some(status) => status.best_slot,
                None => return,
            };

            // New epoch: any in-flight responses from previous cycles will be
            // discarded because their epoch won't match.
            self.sync_epoch += 1;

            // Clean slate for the new cycle. Clear ALL sync state from
            // previous cycles to prevent contamination.
            self.header_downloader.clear();
            self.pending_headers.clear();
            self.pending_blocks.clear();
            self.headers_needing_bodies.clear();
            self.pending_requests.clear();
            self.body_downloader.clear();
            self.body_stall_retries = 0;
            for s in self.peers.values_mut() {
                s.pending_request = None;
            }

            if self.post_rollback {
                // NT10 fix: After a fork rollback, our tip is still on the fork
                // (just 1 block shorter). Header-first sync will always get 0
                // headers because peers don't recognize our rolled-back fork tip.
                // Instead, try fork_sync (binary search for common ancestor).
                // If fork_sync can't start (e.g., no recovery peer), escalate to
                // snap sync via needs_genesis_resync.
                self.post_rollback = false;

                // Reorg cooldown: if we just rejected a fork sync (delta=0),
                // don't start another one immediately. This breaks the infinite
                // reorg loop when 50+ peers offer equal-weight competing chains.
                let cooldown_elapsed = self.last_fork_sync_rejection.elapsed().as_secs();
                if cooldown_elapsed < self.fork_sync_cooldown_secs {
                    info!(
                        "Post-rollback: fork sync in cooldown ({}s/{}) — using header-first",
                        cooldown_elapsed, self.fork_sync_cooldown_secs
                    );
                    self.state = SyncState::DownloadingHeaders {
                        target_slot,
                        peer,
                        headers_count: 0,
                    };
                } else if self.post_recovery_grace {
                    info!(
                        "Post-rollback: skipping fork_sync — post_recovery_grace active, using header-first",
                    );
                    self.state = SyncState::DownloadingHeaders {
                        target_slot,
                        peer,
                        headers_count: 0,
                    };
                } else if self.start_fork_sync() {
                    info!(
                        "Post-rollback: started fork_sync (binary search) instead of header-first",
                    );
                } else {
                    // Fall through to header-first as a last resort (it may fail, but cleanup() will retry).
                    let gap = self
                        .peers
                        .values()
                        .map(|p| p.best_height)
                        .max()
                        .unwrap_or(0)
                        .saturating_sub(self.local_height);
                    warn!(
                        "Post-rollback: fork_sync failed — falling back to header-first (gap={}, peers={})",
                        gap,
                        self.peers.len()
                    );
                    self.state = SyncState::DownloadingHeaders {
                        target_slot,
                        peer,
                        headers_count: 0,
                    };
                }
            } else if self.local_height == 0
                && self.checkpoint_height > 0
                && self.checkpoint_state_root != crypto::Hash::ZERO
            {
                // Checkpoint-anchored state sync: new node at height 0 with a
                // checkpoint that includes a state root. Download state instead
                // of replaying the full chain from genesis.
                info!(
                    "Starting checkpoint state sync epoch {} with peer {} (checkpoint_h={})",
                    self.sync_epoch, peer, self.checkpoint_height
                );

                let request = SyncRequest::GetStateAtCheckpoint {
                    height: self.checkpoint_height,
                };
                let id = self.register_request(peer, request.clone());
                if let Some(status) = self.peers.get_mut(&peer) {
                    status.pending_request = Some(id);
                }

                self.state = SyncState::CheckpointDownloading {
                    peer,
                    started_at: Instant::now(),
                };
            } else {
                info!(
                    "Starting sync epoch {} with peer {} (target_slot={})",
                    self.sync_epoch, peer, target_slot
                );

                self.state = SyncState::DownloadingHeaders {
                    target_slot,
                    peer,
                    headers_count: 0,
                };
            }

            // Reset the stuck-sync timer so this new attempt gets a full window
            // before cleanup() declares it stuck.
            self.last_sync_activity = Instant::now();
            self.last_block_applied = Instant::now();
        }
    }

    /// Get the next sync request to send
    pub fn next_request(&mut self) -> Option<(PeerId, SyncRequest)> {
        // Fork sync takes priority when active (runs in Idle state)
        if let Some(ref mut fs) = self.fork_sync {
            if fs.is_timed_out() {
                let timed_out_peer = fs.peer();
                warn!(
                    "Fork sync timed out — blacklisting peer {} for fork sync (5 min)",
                    timed_out_peer
                );
                self.fork_sync_blacklist
                    .insert(timed_out_peer, Instant::now());
                self.fork_sync = None;
            } else {
                let peer = fs.peer();
                // Guard: don't send if peer already has a pending request
                if let Some(status) = self.peers.get(&peer) {
                    if status.pending_request.is_some() {
                        return None;
                    }
                }
                if let Some(request) = fs.next_request() {
                    let id = self.register_request(peer, request.clone());
                    if let Some(status) = self.peers.get_mut(&peer) {
                        status.pending_request = Some(id);
                    }
                    return Some((peer, request));
                }
                return None; // Fork sync active but no request needed right now
            }
        }

        match &self.state {
            SyncState::Idle | SyncState::Synchronized => {
                // Serve fork recovery requests when main sync is idle
                if let Some((peer, hash)) = self.fork_recovery.next_fetch() {
                    let request = SyncRequest::GetBlockByHash { hash };
                    let id = self.register_request(peer, request.clone());
                    if let Some(status) = self.peers.get_mut(&peer) {
                        status.pending_request = Some(id);
                    }
                    return Some((peer, request));
                }
                None
            }

            SyncState::DownloadingHeaders { peer, .. } => {
                let peer = *peer;

                // Guard: don't send a new request if the peer already has one in flight.
                // Without this, cleanup() calls next_request() every ~1s, creating
                // duplicate requests whose stale responses corrupt expected_prev_hash.
                if let Some(status) = self.peers.get(&peer) {
                    if status.pending_request.is_some() {
                        debug!(
                            "Skipping header request: peer {} already has pending request",
                            peer
                        );
                        return None;
                    }
                }

                // After 10+ consecutive empty responses, peer doesn't recognize our tip.
                // Try snap sync first (seconds) before falling back to genesis resync (hours).
                if self.consecutive_empty_headers >= 10 {
                    let best_height = self
                        .peers
                        .values()
                        .map(|p| p.best_height)
                        .max()
                        .unwrap_or(0);
                    let gap = best_height.saturating_sub(self.local_height);
                    // Small gaps: redirect to fork_sync/rollback.
                    // Rollback is O(1) per block and preserves full chain history.
                    if gap <= 50 && self.local_height > 0 {
                        warn!(
                            "[SYNC] ESCALATE type=small_fork gap={} empties={} local_h={} — redirecting to rollback",
                            gap, self.consecutive_empty_headers, self.local_height
                        );
                        self.consecutive_empty_headers = 3; // Re-trigger resolve_shallow_fork
                        self.state = SyncState::Idle;
                        return None;
                    }
                    // Large gap: try fork_sync or log error for manual intervention
                    warn!(
                        "[SYNC] ESCALATE type=deep_fork gap={} empties={} local_h={} — resetting for fork_sync",
                        gap, self.consecutive_empty_headers, self.local_height
                    );
                    self.consecutive_empty_headers = 3; // Trigger fork_sync
                    self.state = SyncState::Idle;
                    return None;
                }
                let start_hash = self.local_hash;
                let request = self.header_downloader.create_request(start_hash);

                if let Some(req) = request {
                    let id = self.register_request(peer, req.clone());
                    if let Some(status) = self.peers.get_mut(&peer) {
                        status.pending_request = Some(id);
                    }
                    Some((peer, req))
                } else {
                    None
                }
            }

            SyncState::DownloadingBodies { .. } => {
                // Only request bodies from peers that actually HAVE the blocks
                // we need. A peer at height 130 can't serve bodies for blocks
                // 131-37000 — it'll return "Missing body" for everything,
                // polluting the failed queue and stalling the download.
                //
                // Use the best peer's height as anchor: peers within 10% of
                // the best are likely to have the data. This avoids filtering
                // out N4 when its status height is slightly stale, while still
                // excluding peers at height 130 when we need 37K blocks.
                let best = self
                    .peers
                    .values()
                    .map(|s| s.best_height)
                    .max()
                    .unwrap_or(0);
                let threshold = if best > 1000 {
                    best * 9 / 10 // Within 10% of best
                } else {
                    best.saturating_sub(10) // Small chain: within 10 blocks
                };
                let peers_with_data: Vec<PeerId> = self
                    .peers
                    .iter()
                    .filter(|(_, s)| s.best_height >= threshold)
                    .map(|(p, _)| *p)
                    .collect();
                if let Some((peer, request)) = self
                    .body_downloader
                    .next_request(&self.headers_needing_bodies, &peers_with_data)
                {
                    let id = self.register_request(peer, request.clone());
                    if let Some(status) = self.peers.get_mut(&peer) {
                        status.pending_request = Some(id);
                    }
                    Some((peer, request))
                } else {
                    None
                }
            }

            SyncState::Processing { .. } => None,
            SyncState::CheckpointDownloading { .. } => None, // Request already sent
            SyncState::CheckpointReady { .. } => None,       // Waiting for node to consume
        }
    }

    /// Register a pending request
    fn register_request(&mut self, peer: PeerId, request: SyncRequest) -> SyncRequestId {
        let id = SyncRequestId::new(self.next_request_id);
        self.next_request_id += 1;

        info!(
            "[SYNC_DEBUG] Registering request id={}: peer={}, request={:?}, state={:?}",
            id.0, peer, request, self.state
        );

        self.pending_requests.insert(
            id,
            PendingRequest {
                peer,
                request,
                sent_at: Instant::now(),
                epoch: self.sync_epoch,
            },
        );

        id
    }

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
                self.pending_requests.remove(&req_id).map(|r| r.epoch)
            } else {
                None
            }
        } else {
            None
        };

        let is_stale_epoch = matches!(request_epoch, Some(e) if e != self.sync_epoch);

        match response {
            SyncResponse::Headers(headers) => {
                info!(
                    "[SYNC_DEBUG] Handling headers response: count={}",
                    headers.len()
                );
                // Fork sync intercept: route headers to fork sync when active
                if let Some(ref mut fs) = self.fork_sync {
                    if fs.peer() == peer {
                        fs.handle_headers_response(headers);
                        return vec![];
                    }
                }
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
                        self.sync_epoch
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
                // Fork sync intercept: route bodies to fork sync when active
                if let Some(ref mut fs) = self.fork_sync {
                    if fs.peer() == peer {
                        fs.handle_bodies_response(bodies);
                        return vec![];
                    }
                }
                if is_stale_epoch {
                    debug!(
                        "Discarding stale bodies from {} ({} bodies) — epoch {:?} != current {}",
                        peer,
                        bodies.len(),
                        request_epoch,
                        self.sync_epoch
                    );
                    return vec![];
                }
                self.handle_bodies_response(peer, bodies)
            }
            SyncResponse::Block(boxed_block) => {
                let maybe_block = *boxed_block;
                info!(
                    "[SYNC_DEBUG] Handling block response: has_block={}",
                    maybe_block.is_some()
                );
                // Fork sync intercept: consume block responses during binary search
                if let Some(ref mut fs) = self.fork_sync {
                    if fs.peer() == peer {
                        fs.handle_block_response(maybe_block);
                        return vec![];
                    }
                }
                // Fork recovery intercept: consume blocks during active recovery
                if self.fork_recovery.is_active()
                    && self.fork_recovery.handle_block(peer, maybe_block.clone())
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
            SyncResponse::StateAtCheckpoint {
                block_hash,
                block_height,
                chain_state,
                utxo_set,
                producer_set,
                state_root,
            } => {
                info!(
                    "[CHECKPOINT] Received state at height={} from peer={}, size={}KB, root={}",
                    block_height,
                    peer,
                    (chain_state.len() + utxo_set.len() + producer_set.len()) / 1024,
                    &state_root.to_string()[..16]
                );

                // Verify height matches our checkpoint
                if block_height != self.checkpoint_height {
                    warn!(
                        "[CHECKPOINT] Height mismatch: got {} expected {} — rejecting",
                        block_height, self.checkpoint_height
                    );
                    self.state = SyncState::Idle;
                    return vec![];
                }

                // Verify block hash matches checkpoint (if non-zero)
                if self.checkpoint_hash != Hash::ZERO && block_hash != self.checkpoint_hash {
                    warn!(
                        "[CHECKPOINT] Block hash mismatch: got {} expected {} — rejecting",
                        &block_hash.to_string()[..16],
                        &self.checkpoint_hash.to_string()[..16]
                    );
                    self.state = SyncState::Idle;
                    return vec![];
                }

                // Store as CheckpointReady — the node will verify state_root
                // against the hardcoded CHECKPOINT_STATE_ROOT constant.
                self.state = SyncState::CheckpointReady {
                    block_hash,
                    block_height,
                    chain_state,
                    utxo_set,
                    producer_set,
                    state_root,
                };
                info!(
                    "[CHECKPOINT] State stored as CheckpointReady at height={}",
                    block_height
                );

                vec![]
            }
            SyncResponse::Error(err) => {
                warn!("[SYNC_DEBUG] Sync error from peer {}: {}", peer, err);
                vec![]
            }
        }
    }

    /// Handle headers response
    fn handle_headers_response(&mut self, peer: PeerId, headers: Vec<BlockHeader>) {
        if headers.is_empty() {
            debug!("Received empty headers response from {}", peer);

            // No more headers, transition to body download
            if !self.headers_needing_bodies.is_empty() {
                let total = self.headers_needing_bodies.len();
                self.state = SyncState::DownloadingBodies { pending: 0, total };
                info!("Starting body download for {} blocks", total);
            } else if self.pending_headers.is_empty() {
                let peer_height = self.peers.get(&peer).map(|p| p.best_height).unwrap_or(0);
                let gap = peer_height.saturating_sub(self.local_height);

                // Empty headers = peer doesn't recognize our tip hash.
                // Two cases:
                // 1. Small gap (<=50): we're on a minor fork. Signal immediate
                //    rollback — each rollback tries the parent hash until peers
                //    recognize it. Recovery in seconds, not minutes.
                // 2. Large gap (>50): too deep for rollback. Accumulate fork
                //    evidence for snap sync escalation.
                self.consecutive_empty_headers += 1;
                // Empty headers are fork evidence, NOT sync failures.
                // Don't inflate consecutive_sync_failures — that triggers
                // false "Deep fork detected" from unreachable/flaky peers.
                // Only count actual block validation failures as sync failures.

                if gap <= 50 && self.local_height > 0 {
                    // Small fork: signal rollback. The node's periodic task
                    // (resolve_shallow_fork) will roll back 1 block per tick,
                    // changing local_hash to the parent. After 1-3 rollbacks,
                    // GetHeaders succeeds and sync resumes normally.
                    warn!(
                        "[SYNC] EMPTY_HEADERS peer={} gap={} empties={} local_h={} — minor fork, signaling rollback",
                        peer, gap, self.consecutive_empty_headers, self.local_height
                    );
                    // Set to 3 immediately to trigger resolve_shallow_fork
                    // on the next periodic tick (no waiting for 3 separate responses)
                    self.consecutive_empty_headers = self.consecutive_empty_headers.max(3);
                    self.state = SyncState::Idle;
                    return;
                }

                self.header_blacklisted_peers.insert(peer, Instant::now());
                warn!(
                    "[SYNC] EMPTY_HEADERS peer={} peer_h={} local_h={} gap={} empties={} — deep fork, blacklisted peer",
                    peer, peer_height, self.local_height, gap, self.consecutive_empty_headers
                );
                self.state = SyncState::Idle;
            } else {
                self.state = SyncState::Synchronized;
                self.header_blacklisted_peers.clear();
                info!("Chain synchronized");
            }
            return;
        }

        info!("Received {} headers from {}", headers.len(), peer);

        // Validate and store headers
        let valid_count = self
            .header_downloader
            .process_headers(&headers, self.local_hash);

        if valid_count > 0 {
            // Successfully received valid headers — reset fork counters
            self.consecutive_empty_headers = 0;
            self.consecutive_sync_failures = 0;
            self.last_sync_activity = Instant::now();

            // Queue header hashes for body download
            for header in headers.iter().take(valid_count) {
                self.headers_needing_bodies.push_back(header.hash());
                self.pending_headers.push_back(header.clone());
            }

            // Update state
            if let SyncState::DownloadingHeaders {
                target_slot,
                peer,
                headers_count,
            } = &self.state
            {
                self.state = SyncState::DownloadingHeaders {
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
            self.consecutive_empty_headers += 1;
            self.consecutive_sync_failures += 1;
            warn!(
                "[SYNC] CHAIN_BREAK peer={} local_h={} empties={} sync_failures={} — peer has different chain",
                peer, self.local_height, self.consecutive_empty_headers, self.consecutive_sync_failures
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
            self.body_downloader.process_response(peer, vec![]);
            return vec![];
        }

        info!("Received {} bodies from {}", bodies.len(), peer);
        self.last_sync_activity = Instant::now();

        // Process through body_downloader to update its internal state
        let processed_bodies = self.body_downloader.process_response(peer, bodies);

        // Remove these hashes from needed list and store blocks
        let mut blocks_to_apply = Vec::new();

        for block in processed_bodies {
            let hash = block.hash();
            self.headers_needing_bodies.retain(|h| h != &hash);

            // Store block for later application
            self.pending_blocks.insert(hash, block.clone());
            blocks_to_apply.push(block);
        }

        // Update state
        let remaining = self.headers_needing_bodies.len();
        if remaining == 0 {
            self.state = SyncState::Processing {
                height: self.local_height + 1,
            };
            info!("All bodies downloaded, starting processing");
        } else if let SyncState::DownloadingBodies { total, .. } = &self.state {
            let total_copy = *total; // Copy before reassigning self.state
            self.state = SyncState::DownloadingBodies {
                pending: remaining,
                total: total_copy,
            };
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
        let in_processing = matches!(self.state, SyncState::Processing { .. });
        if let Some(header) = self.pending_headers.front() {
            if header.prev_hash != current_hash && in_processing {
                warn!(
                    "[SYNC] CHAIN_MISMATCH slot={} prev={} local_tip={} pending={} — clearing stale headers",
                    header.slot,
                    &header.prev_hash.to_string()[..16],
                    &current_hash.to_string()[..16],
                    self.pending_headers.len()
                );
                self.pending_headers.clear();
                self.pending_blocks.clear();
                self.headers_needing_bodies.clear();
                self.header_downloader.clear();
                // Count toward deep fork detection so genesis resync triggers
                self.consecutive_empty_headers += 1;
                // Always reset to Idle — staying in DownloadingBodies with
                // empty headers_needing_bodies leaves the body downloader in
                // a zombie state (needed=0, no requests, waits 120s for soft
                // recovery). Idle lets cleanup() start a fresh sync immediately.
                self.state = SyncState::Idle;
                return blocks;
            }
        }

        // Get pending headers in order
        while let Some(header) = self.pending_headers.front() {
            if header.prev_hash != current_hash {
                break;
            }

            let hash = header.hash();
            if let Some(block) = self.pending_blocks.remove(&hash) {
                current_hash = hash;
                blocks.push(block);
                self.pending_headers.pop_front();
            } else {
                // Don't have the body yet
                break;
            }
        }

        blocks
    }
}
