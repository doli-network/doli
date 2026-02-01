//! Sync manager coordinating chain synchronization
//!
//! The SyncManager orchestrates the synchronization process:
//! 1. Discovers peers with higher chain tip
//! 2. Downloads headers first to validate VDF chain
//! 3. Downloads block bodies in parallel
//! 4. Applies blocks to local chain

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use libp2p::PeerId;
use tracing::{debug, info, warn};

use crypto::Hash;
use doli_core::{Block, BlockHeader};

use super::bodies::BodyDownloader;
use super::headers::HeaderDownloader;
use super::reorg::ReorgHandler;
use crate::protocols::{SyncRequest, SyncResponse};

/// Sync configuration
#[derive(Clone, Debug)]
pub struct SyncConfig {
    /// Maximum headers to request at once
    pub max_headers_per_request: u32,
    /// Maximum bodies to request at once
    pub max_bodies_per_request: usize,
    /// Maximum concurrent body requests
    pub max_concurrent_body_requests: usize,
    /// Timeout for sync requests
    pub request_timeout: Duration,
    /// Minimum peers to start sync
    pub min_peers_for_sync: usize,
    /// Stale peer timeout
    pub stale_timeout: Duration,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            max_headers_per_request: 2000,
            max_bodies_per_request: 128,
            max_concurrent_body_requests: 8,
            request_timeout: Duration::from_secs(30),
            min_peers_for_sync: 1,
            stale_timeout: Duration::from_secs(300), // 5 minutes - peers stay active longer
        }
    }
}

/// Synchronization state
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncState {
    /// Not syncing, waiting for peers
    Idle,
    /// Downloading headers from target
    DownloadingHeaders {
        /// Target slot we're syncing to
        target_slot: u32,
        /// Peer we're syncing from
        peer: PeerId,
        /// Headers downloaded so far
        headers_count: usize,
    },
    /// Downloading block bodies in parallel
    DownloadingBodies {
        /// Number of pending body requests
        pending: usize,
        /// Total bodies to download
        total: usize,
    },
    /// Processing downloaded blocks
    Processing {
        /// Current height being processed
        height: u64,
    },
    /// Fully synchronized
    Synchronized,
}

impl SyncState {
    /// Check if we're actively syncing
    pub fn is_syncing(&self) -> bool {
        !matches!(self, SyncState::Idle | SyncState::Synchronized)
    }
}

/// Peer sync status
#[derive(Clone, Debug)]
pub struct PeerSyncStatus {
    /// Peer's best height
    pub best_height: u64,
    /// Peer's best hash
    pub best_hash: Hash,
    /// Peer's best slot
    pub best_slot: u32,
    /// Last update time
    pub last_update: Instant,
    /// In-flight request
    pub pending_request: Option<SyncRequestId>,
}

/// Unique request identifier
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SyncRequestId(u64);

impl SyncRequestId {
    fn new(id: u64) -> Self {
        Self(id)
    }
}

/// Pending sync request
#[derive(Debug)]
struct PendingRequest {
    peer: PeerId,
    request: SyncRequest,
    sent_at: Instant,
}

/// Sync manager
pub struct SyncManager {
    /// Configuration
    config: SyncConfig,
    /// Current sync state
    state: SyncState,
    /// Our best height
    local_height: u64,
    /// Our best hash
    local_hash: Hash,
    /// Our best slot
    local_slot: u32,
    /// Peer sync statuses
    peers: HashMap<PeerId, PeerSyncStatus>,
    /// Headers waiting to be processed
    pending_headers: VecDeque<BlockHeader>,
    /// Header hashes we need bodies for
    headers_needing_bodies: VecDeque<Hash>,
    /// Downloaded blocks waiting to be applied
    pending_blocks: HashMap<Hash, Block>,
    /// Pending requests by ID
    pending_requests: HashMap<SyncRequestId, PendingRequest>,
    /// Next request ID
    next_request_id: u64,
    /// Header downloader
    header_downloader: HeaderDownloader,
    /// Body downloader
    body_downloader: BodyDownloader,
    /// Reorg handler
    reorg_handler: ReorgHandler,
    /// Blocks applied since last log
    blocks_applied: u64,
    /// Last progress log time
    last_progress_log: Instant,
}

impl SyncManager {
    /// Create a new sync manager
    pub fn new(config: SyncConfig, genesis_hash: Hash) -> Self {
        Self {
            header_downloader: HeaderDownloader::new(
                config.max_headers_per_request,
                config.request_timeout,
            ),
            body_downloader: BodyDownloader::new(
                config.max_bodies_per_request,
                config.max_concurrent_body_requests,
                config.request_timeout,
            ),
            reorg_handler: ReorgHandler::new(),
            config,
            state: SyncState::Idle,
            local_height: 0,
            local_hash: genesis_hash,
            local_slot: 0,
            peers: HashMap::new(),
            pending_headers: VecDeque::new(),
            headers_needing_bodies: VecDeque::new(),
            pending_blocks: HashMap::new(),
            pending_requests: HashMap::new(),
            next_request_id: 0,
            blocks_applied: 0,
            last_progress_log: Instant::now(),
        }
    }

    /// Get current sync state
    pub fn state(&self) -> &SyncState {
        &self.state
    }

    /// Get local chain tip
    pub fn local_tip(&self) -> (u64, Hash, u32) {
        (self.local_height, self.local_hash, self.local_slot)
    }

    /// Update local chain tip
    pub fn update_local_tip(&mut self, height: u64, hash: Hash, slot: u32) {
        self.local_height = height;
        self.local_hash = hash;
        self.local_slot = slot;

        // Check if we're now synchronized
        if let Some(best_peer) = self.best_peer() {
            if let Some(status) = self.peers.get(&best_peer) {
                if height >= status.best_height {
                    self.state = SyncState::Synchronized;
                    info!("Chain synchronized at height {}", height);
                }
            }
        }
    }

    /// Register a new peer
    pub fn add_peer(&mut self, peer: PeerId, height: u64, hash: Hash, slot: u32) {
        info!("Adding peer {} with height {}, slot {}", peer, height, slot);

        self.peers.insert(
            peer,
            PeerSyncStatus {
                best_height: height,
                best_hash: hash,
                best_slot: slot,
                last_update: Instant::now(),
                pending_request: None,
            },
        );

        // Check if we should start syncing
        if self.state == SyncState::Idle && self.should_sync() {
            self.start_sync();
        }
    }

    /// Update peer status
    pub fn update_peer(&mut self, peer: PeerId, height: u64, hash: Hash, slot: u32) {
        if let Some(status) = self.peers.get_mut(&peer) {
            status.best_height = height;
            status.best_hash = hash;
            status.best_slot = slot;
            status.last_update = Instant::now();
        }
    }

    /// Refresh all peers' timestamps when activity is detected on the network
    /// Call this when receiving blocks/transactions via gossip to prevent stale timeouts
    pub fn refresh_all_peers(&mut self) {
        let now = Instant::now();
        for status in self.peers.values_mut() {
            status.last_update = now;
        }
    }

    /// Remove a peer
    pub fn remove_peer(&mut self, peer: &PeerId) {
        self.peers.remove(peer);

        // Cancel any pending requests from this peer
        self.pending_requests.retain(|_, req| &req.peer != peer);

        // If we were syncing from this peer, try another
        if let SyncState::DownloadingHeaders {
            peer: sync_peer, ..
        } = &self.state
        {
            if sync_peer == peer {
                self.state = SyncState::Idle;
                if self.should_sync() {
                    self.start_sync();
                }
            }
        }
    }

    /// Get the number of connected peers with known status
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Get the best (highest) height among all connected peers
    /// Returns 0 if no peers are connected
    pub fn best_peer_height(&self) -> u64 {
        self.peers
            .values()
            .map(|p| p.best_height)
            .max()
            .unwrap_or(0)
    }

    /// Get the best (highest) slot among all connected peers
    /// Returns 0 if no peers are connected
    pub fn best_peer_slot(&self) -> u32 {
        self.peers.values().map(|p| p.best_slot).max().unwrap_or(0)
    }

    /// Check if we should start syncing
    fn should_sync(&self) -> bool {
        if self.peers.len() < self.config.min_peers_for_sync {
            return false;
        }

        // Check if any peer is ahead of us
        self.peers
            .values()
            .any(|p| p.best_height > self.local_height)
    }

    /// Get the best peer to sync from
    fn best_peer(&self) -> Option<PeerId> {
        self.peers
            .iter()
            .filter(|(_, status)| status.best_height > self.local_height)
            .max_by_key(|(_, status)| status.best_height)
            .map(|(peer, _)| *peer)
    }

    /// Start the sync process
    fn start_sync(&mut self) {
        if let Some(peer) = self.best_peer() {
            if let Some(status) = self.peers.get(&peer) {
                info!(
                    "Starting sync with peer {} (height {}, slot {})",
                    peer, status.best_height, status.best_slot
                );

                self.state = SyncState::DownloadingHeaders {
                    target_slot: status.best_slot,
                    peer,
                    headers_count: 0,
                };
            }
        }
    }

    /// Get the next sync request to send
    pub fn next_request(&mut self) -> Option<(PeerId, SyncRequest)> {
        match &self.state {
            SyncState::Idle | SyncState::Synchronized => None,

            SyncState::DownloadingHeaders { peer, .. } => {
                let peer = *peer;

                // Request headers from our current tip
                let request = self.header_downloader.create_request(self.local_hash);

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
                // Get next batch of bodies to request
                if let Some((peer, request)) = self.body_downloader.next_request(
                    &self.headers_needing_bodies,
                    &self.peers.keys().cloned().collect::<Vec<_>>(),
                ) {
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
        }
    }

    /// Register a pending request
    fn register_request(&mut self, peer: PeerId, request: SyncRequest) -> SyncRequestId {
        let id = SyncRequestId::new(self.next_request_id);
        self.next_request_id += 1;

        self.pending_requests.insert(
            id,
            PendingRequest {
                peer,
                request,
                sent_at: Instant::now(),
            },
        );

        id
    }

    /// Handle a sync response
    pub fn handle_response(&mut self, peer: PeerId, response: SyncResponse) -> Vec<Block> {
        // Clear pending request for this peer
        if let Some(status) = self.peers.get_mut(&peer) {
            status.pending_request = None;
        }

        match response {
            SyncResponse::Headers(headers) => {
                self.handle_headers_response(peer, headers);
                vec![]
            }
            SyncResponse::Bodies(bodies) => self.handle_bodies_response(peer, bodies),
            SyncResponse::Block(maybe_block) => {
                if let Some(block) = maybe_block {
                    vec![block]
                } else {
                    vec![]
                }
            }
            SyncResponse::Error(err) => {
                warn!("Sync error from peer {}: {}", peer, err);
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
            } else {
                self.state = SyncState::Synchronized;
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
            warn!("No valid headers from peer {}", peer);
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

    /// Mark a block as applied (legacy - uses weight 1)
    pub fn block_applied(&mut self, hash: Hash, height: u64, slot: u32) {
        self.block_applied_with_weight(hash, height, slot, 1, Hash::ZERO);
    }

    /// Mark a block as applied with its producer's effective weight
    ///
    /// This enables the weight-based fork choice rule. When a fork occurs,
    /// we'll choose the chain with higher accumulated producer weight.
    pub fn block_applied_with_weight(
        &mut self,
        hash: Hash,
        height: u64,
        slot: u32,
        producer_weight: u64,
        prev_hash: Hash,
    ) {
        // Record block with weight for fork choice rule
        self.reorg_handler
            .record_block_with_weight(hash, prev_hash, producer_weight);

        self.local_height = height;
        self.local_hash = hash;
        self.local_slot = slot;
        self.blocks_applied += 1;

        // Log progress periodically
        if self.last_progress_log.elapsed() > Duration::from_secs(5) {
            info!(
                "Sync progress: height {}, {} blocks/sec",
                height,
                self.blocks_applied / 5
            );
            self.blocks_applied = 0;
            self.last_progress_log = Instant::now();
        }
    }

    /// Reset local state to genesis (for forced resync)
    pub fn reset_local_state(&mut self, genesis_hash: Hash) {
        self.local_height = 0;
        self.local_hash = genesis_hash;
        self.local_slot = 0;
        self.blocks_applied = 0;
        self.state = SyncState::Idle;
        self.reorg_handler.clear();

        // Clear all pending sync state to start fresh
        self.pending_headers.clear();
        self.pending_blocks.clear();
        self.headers_needing_bodies.clear();
        self.pending_requests.clear();

        // Reset downloaders
        self.header_downloader =
            HeaderDownloader::new(self.config.max_headers_per_request, self.config.request_timeout);
        self.body_downloader = BodyDownloader::new(
            self.config.max_bodies_per_request,
            self.config.max_concurrent_body_requests,
            self.config.request_timeout,
        );

        info!("Sync manager reset to genesis for forced resync");

        // Trigger sync if we have peers that are ahead (which we should after reset)
        if self.should_sync() {
            info!("Triggering sync after reset - peers are ahead of us");
            self.start_sync();
        }
    }

    /// Get the current chain's accumulated weight
    pub fn chain_weight(&self) -> u64 {
        self.reorg_handler.current_weight()
    }

    /// Handle a potential chain reorganization (legacy - no weight check)
    pub fn handle_new_block(&mut self, block: Block) -> Option<Vec<Hash>> {
        self.reorg_handler.check_reorg(&block, self.local_hash)
    }

    /// Handle a potential chain reorganization with weight-based fork choice
    ///
    /// Returns a ReorgResult only if the new chain is heavier than our current chain.
    /// This implements "heaviest chain wins" fork choice rule.
    pub fn handle_new_block_weighted(
        &mut self,
        block: Block,
        producer_weight: u64,
    ) -> Option<super::reorg::ReorgResult> {
        self.reorg_handler
            .check_reorg_weighted(&block, self.local_hash, producer_weight)
    }

    /// Clean up stale requests and peers
    pub fn cleanup(&mut self) {
        let now = Instant::now();

        // Clean up timed out body download requests
        // This moves timed-out hashes back to the failed queue for retry
        self.body_downloader.cleanup_timeouts();

        // Remove timed out requests
        let timed_out: Vec<SyncRequestId> = self
            .pending_requests
            .iter()
            .filter(|(_, req)| now.duration_since(req.sent_at) > self.config.request_timeout)
            .map(|(id, _)| *id)
            .collect();

        for id in timed_out {
            if let Some(req) = self.pending_requests.remove(&id) {
                warn!("Request to {} timed out", req.peer);
                if let Some(status) = self.peers.get_mut(&req.peer) {
                    status.pending_request = None;
                }
            }
        }

        // Remove stale peers
        let stale: Vec<PeerId> = self
            .peers
            .iter()
            .filter(|(_, status)| {
                now.duration_since(status.last_update) > self.config.stale_timeout
            })
            .map(|(peer, _)| *peer)
            .collect();

        for peer in stale {
            warn!("Removing stale peer {}", peer);
            self.remove_peer(&peer);
        }
    }

    /// Get sync progress as a percentage
    pub fn progress(&self) -> Option<f64> {
        match &self.state {
            SyncState::Idle | SyncState::Synchronized => None,
            SyncState::DownloadingHeaders { target_slot, .. } => {
                if *target_slot > 0 {
                    Some(self.local_slot as f64 / *target_slot as f64 * 100.0)
                } else {
                    Some(0.0)
                }
            }
            SyncState::DownloadingBodies { pending, total } => {
                if *total > 0 {
                    Some((*total - *pending) as f64 / *total as f64 * 100.0)
                } else {
                    Some(100.0)
                }
            }
            SyncState::Processing { height } => {
                if let Some(best) = self.best_peer().and_then(|p| self.peers.get(&p)) {
                    if best.best_height > 0 {
                        Some(*height as f64 / best.best_height as f64 * 100.0)
                    } else {
                        Some(100.0)
                    }
                } else {
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_state_is_syncing() {
        assert!(!SyncState::Idle.is_syncing());
        assert!(!SyncState::Synchronized.is_syncing());
        assert!(SyncState::DownloadingHeaders {
            target_slot: 100,
            peer: PeerId::random(),
            headers_count: 0,
        }
        .is_syncing());
        assert!(SyncState::DownloadingBodies {
            pending: 10,
            total: 100,
        }
        .is_syncing());
        assert!(SyncState::Processing { height: 50 }.is_syncing());
    }

    #[test]
    fn test_sync_manager_creation() {
        let manager = SyncManager::new(SyncConfig::default(), Hash::zero());
        assert_eq!(*manager.state(), SyncState::Idle);
        assert_eq!(manager.local_tip(), (0, Hash::zero(), 0));
    }
}
