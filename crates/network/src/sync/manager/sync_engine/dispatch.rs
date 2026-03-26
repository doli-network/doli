//! Request dispatch — next_request, next_snap_requests, register_request

use std::time::Instant;

use libp2p::PeerId;
use tracing::{debug, info, warn};

use crate::protocols::SyncRequest;
use crate::sync::manager::{
    PendingRequest, SyncManager, SyncPipelineData, SyncRequestId, SyncState,
};

impl SyncManager {
    /// Get the next sync request to send
    pub fn next_request(&mut self) -> Option<(PeerId, SyncRequest)> {
        match &self.pipeline_data {
            SyncPipelineData::None => {
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

            SyncPipelineData::Headers { peer, .. } => {
                let mut peer = *peer;

                // If the designated sync peer was evicted (churn), pick a new one.
                // Without this, we keep sending requests to a dead peer forever —
                // they silently fail, time out, and the node stalls at 0 pending requests.
                if !self.peers.contains_key(&peer) {
                    if let Some(new_peer) = self.best_peer() {
                        warn!(
                            "[SYNC] Header peer {} lost (evicted) — switching to {}",
                            peer, new_peer
                        );
                        peer = new_peer;
                        if let SyncPipelineData::Headers {
                            peer: ref mut p, ..
                        } = &mut self.pipeline_data
                        {
                            *p = new_peer;
                        }
                    } else {
                        return None;
                    }
                }

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
                    // Small gaps (<=12): redirect to rollback instead of
                    // genesis resync. Snap sync for small gaps loses block history
                    // and creates a cascade (snap -> no block 1 -> future rollback
                    // fails -> re-snap). Rollback is O(1) per block and preserves
                    // full chain history.
                    if gap <= 12 && self.local_height > 0 {
                        warn!(
                            "Small fork (gap={}, empties={}): redirecting to rollback \
                             instead of genesis resync — rollback can handle this",
                            gap, self.fork.consecutive_empty_headers
                        );
                        self.fork.consecutive_empty_headers = 3; // Re-trigger resolve_shallow_fork
                        self.set_state(SyncState::Idle, "small_fork_redirect_to_rollback");
                        return None;
                    }
                    // Large gap with not enough peers — fall back to genesis resync
                    info!(
                        "Genesis fallback: {} consecutive empty headers — requesting full resync",
                        self.fork.consecutive_empty_headers
                    );
                    self.request_genesis_resync(
                        super::super::RecoveryReason::GenesisFallbackEmptyHeaders,
                    );
                    self.set_state(SyncState::Idle, "genesis_resync_fallback");
                    return None;
                }
                // INC-I-012 F1: After snap sync to a forked hash, use height-based
                // request to bypass the unrecognizable hash. The server looks up its
                // canonical chain at local_height and serves headers from there.
                if self.fork.use_height_based_headers {
                    let request = SyncRequest::get_headers_by_height(
                        self.local_height,
                        self.config.max_headers_per_request,
                    );
                    info!(
                        "[SYNC] Using GetHeadersByHeight(height={}) — post-snap hash fallback",
                        self.local_height
                    );
                    // Clear the flag — one height-based request is enough to get
                    // canonical headers. Subsequent requests use normal hash-based.
                    self.fork.use_height_based_headers = false;
                    let id = self.register_request(peer, request.clone());
                    if let Some(status) = self.peers.get_mut(&peer) {
                        status.pending_request = Some(id);
                    }
                    return Some((peer, request));
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

            SyncPipelineData::Bodies { .. } => {
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

            SyncPipelineData::Processing { .. } => None,

            // Snap sync root collection is batched via next_snap_requests().
            // Returning None here ensures the normal 1-per-tick path doesn't
            // interfere with the batch approach.
            SyncPipelineData::SnapCollecting { .. } => None,

            SyncPipelineData::SnapDownloading {
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

            SyncPipelineData::SnapReady { .. } => None,
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
        let (target_hash, already_asked) = match &self.pipeline_data {
            SyncPipelineData::SnapCollecting {
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
            if let SyncPipelineData::SnapCollecting { asked, .. } = &mut self.pipeline_data {
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
            id.inner(),
            peer,
            request,
            self.state
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
}
