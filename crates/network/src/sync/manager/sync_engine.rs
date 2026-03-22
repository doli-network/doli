//! Sync engine — orchestrates header-first sync, body download, and request/response handling

use std::collections::{HashMap, HashSet};
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
        let network_ahead = self.network.network_tip_height > self.local_height;

        peer_ahead || network_ahead
    }

    /// Get the best_hash that the MAJORITY of eligible peers agree on.
    ///
    /// This prevents a partitioned minority from steering snap sync to a
    /// forked chain. Returns None if no hash has strict majority (>50%).
    fn consensus_target_hash(&self) -> Option<Hash> {
        // Group eligible peers by height, then pick the hash at the most common
        // height. During normal operation peers are at slightly different heights
        // (tip race), so requiring >50% on the same best_hash always fails.
        // Instead: find the height with the most peers, use any peer's hash at
        // that height. This is safe because all honest peers at the same height
        // MUST have the same hash (deterministic consensus).
        let mut height_counts: HashMap<u64, (usize, Hash)> = HashMap::new();
        for status in self.peers.values() {
            if status.best_height > self.local_height {
                let entry = height_counts
                    .entry(status.best_height)
                    .or_insert((0, status.best_hash));
                entry.0 += 1;
            }
        }
        if height_counts.is_empty() {
            return None;
        }
        // Pick the height with the most peers (break ties by highest height)
        let (_best_height, (count, hash)) = height_counts
            .into_iter()
            .max_by_key(|(h, (c, _))| (*c, *h))?;
        // Require at least 2 peers to agree (prevents single-peer poisoning)
        if count >= 2 {
            Some(hash)
        } else {
            None
        }
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
                    && !self.fork.header_blacklisted_peers.contains_key(pid)
            })
            .max_by_key(|(_, status)| (status.best_height, status.best_slot))
            .map(|(peer, _)| *peer)
    }

    /// Start the sync process.
    ///
    /// Idempotent: if we're already actively syncing (DownloadingHeaders, DownloadingBodies,
    /// Processing, or snap syncing), this is a no-op. This prevents the 43-calls/sec destructive
    /// reset that occurs on large networks when every update_peer() status response triggers
    /// start_sync(), nuking in-flight requests each time (INC-I-004 / INC-I-005 root cause).
    pub(super) fn start_sync(&mut self) {
        // Guard: don't nuke in-flight sync state if already syncing.
        // Callers like update_peer() fire on every StatusResponse from every peer.
        // Without this guard, a 44-node network calls start_sync() ~43 times/sec,
        // each time incrementing sync_epoch and clearing all pending requests.
        if self.state.is_syncing() {
            return;
        }

        if let Some(peer) = self.best_peer() {
            let best_height = match self.peers.get(&peer) {
                Some(status) => status.best_height,
                None => return,
            };
            let target_slot = match self.peers.get(&peer) {
                Some(status) => status.best_slot,
                None => return,
            };

            // New epoch: any in-flight responses from previous cycles will be
            // discarded because their epoch won't match.
            self.pipeline.sync_epoch += 1;

            // Clean slate for the new cycle. Clear ALL sync state from
            // previous cycles to prevent contamination.
            self.pipeline.header_downloader.clear();
            self.pipeline.pending_headers.clear();
            self.pipeline.pending_blocks.clear();
            self.pipeline.headers_needing_bodies.clear();
            self.pipeline.pending_requests.clear();
            self.pipeline.body_downloader.clear();
            self.pipeline.body_stall_retries = 0;
            for s in self.peers.values_mut() {
                s.pending_request = None;
            }

            // Snap sync decision: use snap sync when far behind and enough peers
            let gap = best_height.saturating_sub(self.local_height);
            let enough_peers = self.peers.len() >= 3;
            let snap_allowed = self.snap.threshold < u64::MAX;
            let should_snap = enough_peers
                && self.snap.attempts < 3
                && snap_allowed
                && (self.local_height == 0 || gap > self.snap.threshold);

            // Fresh node optimization: don't start slow header-first sync.
            // Wait for 5 peers so snap sync can activate — it downloads state
            // in seconds instead of replaying 60K+ blocks over hours.
            // BUT: timeout after 60s to avoid deadlock when <5 peers are discoverable.
            if self.local_height == 0
                && !enough_peers
                && self.snap.attempts < 3
                && snap_allowed
                && gap > self.snap.threshold
            {
                let wait_start = self
                    .snap
                    .fresh_node_wait_start
                    .get_or_insert(Instant::now());
                let waited = wait_start.elapsed();
                if waited.as_secs() < 60 {
                    info!(
                        "[SNAP_SYNC] Fresh node: waiting for {} more peer(s) for snap sync ({}/3, gap={}, waited={}s)",
                        3 - self.peers.len(), self.peers.len(), gap, waited.as_secs()
                    );
                    return;
                }
                warn!(
                    "[SNAP_SYNC] Fresh node waited {}s for 5 peers but only have {} — falling back to header-first sync",
                    waited.as_secs(), self.peers.len()
                );
                self.snap.fresh_node_wait_start = None;
            }

            if should_snap {
                // FIX: Use the hash that the MAJORITY of eligible peers agree on,
                // not a single peer's hash. This prevents a partitioned minority
                // (e.g. 3 forked nodes on one server) from steering snap sync
                // to the wrong chain. If no majority exists, fall back to
                // header-first sync — the network is too fragmented for snap sync.
                let target_hash = match self.consensus_target_hash() {
                    Some(hash) => hash,
                    None => {
                        warn!(
                            "[SNAP_SYNC] No majority best_hash among {} peers — network too fragmented for snap sync, falling back to header-first",
                            self.peers.len()
                        );
                        // Fall through to header-first sync below
                        let target_slot = match self.peers.get(&peer) {
                            Some(status) => status.best_slot,
                            None => return,
                        };
                        info!(
                            "Starting sync epoch {} with peer {} (target_slot={})",
                            self.pipeline.sync_epoch, peer, target_slot
                        );
                        self.pipeline.header_downloader.clear();
                        self.set_state(
                            SyncState::DownloadingHeaders {
                                target_slot,
                                peer,
                                headers_count: 0,
                            },
                            "snap_no_majority_fallback",
                        );
                        return;
                    }
                };

                info!(
                    "[SNAP_SYNC] Starting snap sync epoch {} — gap={}, target_height={}, target_hash={:.16} (majority-preferred)",
                    self.pipeline.sync_epoch, gap, best_height, target_hash
                );

                self.set_state(
                    SyncState::SnapCollectingRoots {
                        target_hash,
                        target_height: best_height,
                        votes: Vec::new(),
                        asked: HashSet::new(),
                        started_at: Instant::now(),
                    },
                    "start_snap_sync",
                );
            } else if matches!(self.recovery_phase, super::RecoveryPhase::PostRollback) {
                // NT10 fix: After a fork rollback, our tip is still on the fork
                // (just 1 block shorter). Header-first sync will always get 0
                // headers because peers don't recognize our rolled-back fork tip.
                // Instead, try fork_sync (binary search for common ancestor).
                // If fork_sync can't start (e.g., no recovery peer), escalate to
                // snap sync via needs_genesis_resync.
                self.recovery_phase = super::RecoveryPhase::Normal;

                // INC-001 fix: Circuit breaker — if 3+ fork syncs within 5 minutes,
                // stop trying fork sync and use header-first only.
                if self.is_fork_sync_breaker_tripped() {
                    warn!(
                        "Post-rollback: fork sync circuit breaker tripped ({} fork syncs) — \
                         forcing header-first sync",
                        self.fork.consecutive_fork_syncs
                    );
                    self.set_state(
                        SyncState::DownloadingHeaders {
                            target_slot,
                            peer,
                            headers_count: 0,
                        },
                        "post_rollback_breaker_tripped",
                    );
                // Reorg cooldown: if we just rejected a fork sync (delta=0),
                // don't start another one immediately. This breaks the infinite
                // reorg loop when 50+ peers offer equal-weight competing chains.
                } else if self.fork.last_fork_sync_rejection.elapsed().as_secs()
                    < self.fork.fork_sync_cooldown_secs
                {
                    info!(
                        "Post-rollback: fork sync in cooldown ({}s/{}) — using header-first",
                        self.fork.last_fork_sync_rejection.elapsed().as_secs(),
                        self.fork.fork_sync_cooldown_secs
                    );
                    self.set_state(
                        SyncState::DownloadingHeaders {
                            target_slot,
                            peer,
                            headers_count: 0,
                        },
                        "post_rollback_cooldown_fallback",
                    );
                } else if self.start_fork_sync() {
                    info!(
                        "Post-rollback: started fork_sync (binary search) instead of header-first",
                    );
                } else if enough_peers && snap_allowed && self.snap.attempts < 3 {
                    warn!(
                        "Post-rollback: fork_sync failed to start, escalating to snap sync \
                         (gap={}, peers={})",
                        gap,
                        self.peers.len()
                    );
                    self.fork.needs_genesis_resync = true;
                } else {
                    // Not enough peers for snap sync — fall through to header-first
                    // as a last resort (it may fail, but cleanup() will retry).
                    warn!(
                        "Post-rollback: fork_sync failed, not enough peers for snap sync — \
                         falling back to header-first (gap={}, peers={})",
                        gap,
                        self.peers.len()
                    );
                    self.set_state(
                        SyncState::DownloadingHeaders {
                            target_slot,
                            peer,
                            headers_count: 0,
                        },
                        "post_rollback_no_peers_fallback",
                    );
                }
            } else {
                info!(
                    "Starting sync epoch {} with peer {} (target_slot={})",
                    self.pipeline.sync_epoch, peer, target_slot
                );

                self.set_state(
                    SyncState::DownloadingHeaders {
                        target_slot,
                        peer,
                        headers_count: 0,
                    },
                    "start_header_sync",
                );
            }

            // Reset the stuck-sync timer so this new attempt gets a full window
            // before cleanup() declares it stuck.
            self.network.last_sync_activity = Instant::now();
            self.network.last_block_applied = Instant::now();
        }
    }

    /// Get the next sync request to send
    pub fn next_request(&mut self) -> Option<(PeerId, SyncRequest)> {
        // Fork sync takes priority when active (runs in Idle state)
        if let Some(ref mut fs) = self.fork.fork_sync {
            if fs.is_timed_out() {
                warn!("Fork sync timed out — cancelling");
                self.fork.fork_sync = None;
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
                if let Some((peer, hash)) = self.fork.fork_recovery.next_fetch() {
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
                if self.fork.consecutive_empty_headers >= 10 {
                    let best_height = self
                        .peers
                        .values()
                        .map(|p| p.best_height)
                        .max()
                        .unwrap_or(0);
                    let gap = best_height.saturating_sub(self.local_height);
                    let enough_peers = self.peers.len() >= 3;
                    if enough_peers && gap > self.snap.threshold {
                        info!(
                            "[SNAP_SYNC] Deep fork with {} consecutive empty headers — \
                             attempting snap sync before genesis resync (gap={})",
                            self.fork.consecutive_empty_headers, gap
                        );
                        self.snap.attempts = 0;
                        self.fork.consecutive_empty_headers = 0;
                        self.set_state(SyncState::Idle, "deep_fork_snap_redirect");
                        self.start_sync();
                        return None;
                    }
                    // Small gaps (≤12): redirect to fork_sync/rollback instead of
                    // genesis resync. Snap sync for small gaps loses block history
                    // and creates a cascade (snap → no block 1 → future rollback
                    // fails → re-snap). Rollback is O(1) per block and preserves
                    // full chain history.
                    if gap <= 12 && self.local_height > 0 {
                        warn!(
                            "Small fork (gap={}, empties={}): redirecting to fork_sync \
                             instead of genesis resync — rollback can handle this",
                            gap, self.fork.consecutive_empty_headers
                        );
                        self.fork.consecutive_empty_headers = 3; // Re-trigger resolve_shallow_fork
                        self.set_state(SyncState::Idle, "small_fork_redirect_to_rollback");
                        return None;
                    }
                    // Large gap with not enough peers — fall back to genesis resync
                    info!(
                        "Genesis fallback: {} consecutive empty headers — signaling node for full resync",
                        self.fork.consecutive_empty_headers
                    );
                    self.fork.needs_genesis_resync = true;
                    self.set_state(SyncState::Idle, "genesis_resync_fallback");
                    return None;
                }
                let start_hash = self.local_hash;
                let request = self.pipeline.header_downloader.create_request(start_hash);

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
                    .pipeline
                    .body_downloader
                    .next_request(&self.pipeline.headers_needing_bodies, &peers_with_data)
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

            // Snap sync root collection is batched via next_snap_requests().
            // Returning None here ensures the normal 1-per-tick path doesn't
            // interfere with the batch approach.
            SyncState::SnapCollectingRoots { .. } => None,

            SyncState::SnapDownloading {
                target_hash, peer, ..
            } => {
                let target_hash = *target_hash;
                let peer = *peer;
                // Guard: don't send if peer already has a pending request
                if let Some(status) = self.peers.get(&peer) {
                    if status.pending_request.is_some() {
                        return None;
                    }
                }
                let request = SyncRequest::get_state_snapshot(target_hash);
                let id = self.register_request(peer, request.clone());
                if let Some(status) = self.peers.get_mut(&peer) {
                    status.pending_request = Some(id);
                }
                Some((peer, request))
            }

            SyncState::SnapReady { .. } => None,
        }
    }

    /// Return GetStateRoot requests for ALL un-asked peers at once.
    ///
    /// On an active chain, blocks arrive every 10 seconds. If we send
    /// GetStateRoot one-peer-per-tick, responses spread over ~12 seconds and
    /// peers advance between votes — state roots diverge and quorum is never
    /// reached. Batching all requests in one tick keeps responses within
    /// ~1-2 seconds so most peers respond at the same height.
    pub fn next_snap_requests(&mut self) -> Vec<(PeerId, SyncRequest)> {
        let (target_hash, already_asked) = match &self.state {
            SyncState::SnapCollectingRoots {
                target_hash, asked, ..
            } => (*target_hash, asked.clone()),
            _ => return vec![],
        };

        let candidates: Vec<PeerId> = self
            .peers
            .iter()
            .filter(|(pid, _)| {
                !already_asked.contains(pid) && !self.snap.blacklisted_peers.contains(pid)
            })
            .map(|(pid, _)| *pid)
            .collect();

        if candidates.is_empty() {
            return vec![];
        }

        let mut batch = Vec::with_capacity(candidates.len());
        for pid in candidates {
            if let SyncState::SnapCollectingRoots { asked, .. } = &mut self.state {
                asked.insert(pid);
            }
            let request = SyncRequest::get_state_root(target_hash);
            let id = self.register_request(pid, request.clone());
            if let Some(status) = self.peers.get_mut(&pid) {
                status.pending_request = Some(id);
            }
            batch.push((pid, request));
        }

        if !batch.is_empty() {
            info!(
                "[SNAP_SYNC] Batch-sending GetStateRoot to {} peers simultaneously",
                batch.len()
            );
        }

        batch
    }

    /// Register a pending request
    fn register_request(&mut self, peer: PeerId, request: SyncRequest) -> SyncRequestId {
        let id = SyncRequestId::new(self.pipeline.next_request_id);
        self.pipeline.next_request_id += 1;

        info!(
            "[SYNC_DEBUG] Registering request id={}: peer={}, request={:?}, state={:?}",
            id.0, peer, request, self.state
        );

        self.pipeline.pending_requests.insert(
            id,
            PendingRequest {
                peer,
                request,
                sent_at: Instant::now(),
                epoch: self.pipeline.sync_epoch,
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
                // Fork sync intercept: route headers to fork sync when active
                if let Some(ref mut fs) = self.fork.fork_sync {
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
                // Fork sync intercept: route bodies to fork sync when active
                if let Some(ref mut fs) = self.fork.fork_sync {
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
                // Fork sync intercept: consume block responses during binary search
                if let Some(ref mut fs) = self.fork.fork_sync {
                    if fs.peer() == peer {
                        fs.handle_block_response(maybe_block);
                        return vec![];
                    }
                }
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
                self.handle_snap_download_error(peer);
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
                self.set_state(
                    SyncState::DownloadingBodies { pending: 0, total },
                    "headers_complete",
                );
                info!("Starting body download for {} blocks", total);
            } else if self.pipeline.pending_headers.is_empty() {
                let peer_height = self.peers.get(&peer).map(|p| p.best_height).unwrap_or(0);
                let gap = peer_height.saturating_sub(self.local_height);

                // Empty headers = peer doesn't recognize our tip hash.
                // Two cases:
                // 1. Small gap (<=50): we're on a minor fork. Signal immediate
                //    rollback — each rollback tries the parent hash until peers
                //    recognize it. Recovery in seconds, not minutes.
                // 2. Large gap (>50): too deep for rollback. Accumulate fork
                //    evidence for snap sync escalation.
                self.fork.consecutive_empty_headers += 1;
                // Activate Layer 8 (sync failure fork detection) — peers
                // don't recognize our tip hash, which is fork evidence.
                self.fork.consecutive_sync_failures += 1;

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
                    // First 1-2 empties: could be a peer-specific issue. Blacklist this peer.
                    self.fork
                        .header_blacklisted_peers
                        .insert(peer, Instant::now());
                    warn!(
                        "Empty headers from {} (peer_h={}, local_h={}, gap={}) — \
                         fork evidence (consecutive={}). Blacklisted peer.",
                        peer,
                        peer_height,
                        self.local_height,
                        gap,
                        self.fork.consecutive_empty_headers
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
            self.network.last_sync_activity = Instant::now();
            // INC-001: Reset fork sync circuit breaker on successful header-first sync
            self.reset_fork_sync_breaker();

            // Queue header hashes for body download
            for header in headers.iter().take(valid_count) {
                self.pipeline
                    .headers_needing_bodies
                    .push_back(header.hash());
                self.pipeline.pending_headers.push_back(header.clone());
            }

            // Update state
            if let SyncState::DownloadingHeaders {
                target_slot,
                peer,
                headers_count,
            } = &self.state
            {
                let new_state = SyncState::DownloadingHeaders {
                    target_slot: *target_slot,
                    peer: *peer,
                    headers_count: headers_count + valid_count,
                };
                self.set_state(new_state, "headers_received");
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
            self.set_state(
                SyncState::Processing {
                    height: self.local_height + 1,
                },
                "bodies_complete",
            );
            info!("All bodies downloaded, starting processing");
        } else if let SyncState::DownloadingBodies { total, .. } = &self.state {
            let total_copy = *total; // Copy before reassigning self.state
            self.set_state(
                SyncState::DownloadingBodies {
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
        let in_processing = matches!(self.state, SyncState::Processing { .. });
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
        if blocks.is_empty() && matches!(self.state, SyncState::Processing { .. }) {
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
}
