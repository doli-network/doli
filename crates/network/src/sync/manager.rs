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

/// Production authorization result - the single source of truth for whether block production is safe
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProductionAuthorization {
    /// Production is authorized - safe to produce blocks
    Authorized,
    /// Production is blocked during active sync
    BlockedSyncing,
    /// Production is blocked due to resync in progress
    BlockedResync {
        /// Seconds remaining in grace period
        grace_remaining_secs: u64,
    },
    /// Production is blocked - we're too far behind peers
    BlockedBehindPeers {
        /// Our height
        local_height: u64,
        /// Best peer height
        peer_height: u64,
        /// Height difference
        height_diff: u64,
    },
    /// Production is explicitly blocked (e.g., invariant violation)
    BlockedExplicit {
        /// Reason for block
        reason: String,
    },
    /// Production is blocked during bootstrap - waiting for fresh peer status
    BlockedBootstrap {
        /// Reason for bootstrap block
        reason: String,
    },
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

    // === PRODUCTION GATE FIELDS (Defense-in-depth) ===
    /// Explicit production block flag - when true, production is forbidden
    production_blocked: bool,
    /// Reason for explicit production block
    production_block_reason: Option<String>,
    /// True when a forced resync is in progress
    resync_in_progress: bool,
    /// Timestamp when last resync completed (for grace period)
    last_resync_completed: Option<Instant>,
    /// Number of consecutive resyncs (for exponential backoff)
    consecutive_resync_count: u32,
    /// Grace period after resync before production is allowed (seconds)
    resync_grace_period_secs: u64,
    /// Maximum slots behind peers before blocking production
    max_slots_behind: u32,
    /// Maximum heights behind peers before blocking production
    max_heights_behind: u64,

    // === NETWORK TIP TRACKING (from gossip) ===
    /// Best height seen via gossip (may not have a specific peer source)
    /// This tracks the network's actual state from received blocks
    network_tip_height: u64,
    /// Best slot seen via gossip
    network_tip_slot: u32,

    // === BOOTSTRAP GATE FIELDS ===
    /// Whether we have connected to any peer (set by node when first peer connects)
    has_connected_to_peer: bool,
    /// Time when we FIRST received a valid peer status response (for bootstrap grace period)
    first_peer_status_received: Option<Instant>,
    /// Time when we last received a valid peer status response (for freshness checks)
    last_peer_status_received: Option<Instant>,
    /// Bootstrap grace period (seconds) - time to wait at genesis for chain evidence
    bootstrap_grace_period_secs: u64,
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
            // Production gate defaults
            production_blocked: false,
            production_block_reason: None,
            resync_in_progress: false,
            last_resync_completed: None,
            consecutive_resync_count: 0,
            resync_grace_period_secs: 30, // Default 30 seconds after resync
            max_slots_behind: 2,          // Spec: "within 2 slots of peers"
            max_heights_behind: 2,        // Conservative: also check heights
            // Network tip tracking (from gossip)
            network_tip_height: 0,
            network_tip_slot: 0,
            // Bootstrap gate defaults
            has_connected_to_peer: false,
            first_peer_status_received: None,
            last_peer_status_received: None,
            bootstrap_grace_period_secs: 15, // Wait 15s at genesis for chain evidence
        }
    }

    /// Create a new sync manager with custom production gate settings
    pub fn new_with_settings(
        config: SyncConfig,
        genesis_hash: Hash,
        resync_grace_period_secs: u64,
        max_slots_behind: u32,
        max_heights_behind: u64,
    ) -> Self {
        let mut mgr = Self::new(config, genesis_hash);
        mgr.resync_grace_period_secs = resync_grace_period_secs;
        mgr.max_slots_behind = max_slots_behind;
        mgr.max_heights_behind = max_heights_behind;
        mgr
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
                    let was_syncing = self.state.is_syncing();
                    self.state = SyncState::Synchronized;
                    info!("Chain synchronized at height {}", height);

                    // If we were in a resync, complete it now
                    if self.resync_in_progress {
                        self.complete_resync();
                        info!(
                            "Resync complete at height {} - grace period started ({}s)",
                            height, self.resync_grace_period_secs
                        );
                    }

                    // If we've been stable for a while, reset the consecutive resync counter
                    // This prevents exponential backoff from persisting after recovery
                    if was_syncing && self.consecutive_resync_count > 0 {
                        // We'll reset the counter after the grace period in can_produce()
                        // For now, just note that sync completed successfully
                        debug!(
                            "Sync completed after {} consecutive resyncs",
                            self.consecutive_resync_count
                        );
                    }
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

        // NETWORK TIP FROM PEER STATUS: Update network tip based on peer claims
        // This is critical for production gating - even if we haven't received the
        // actual block via gossip yet, knowing that a peer claims a higher height
        // tells us we shouldn't produce until we're caught up.
        if height > self.network_tip_height {
            debug!(
                "Network tip height updated from peer status: {} -> {}",
                self.network_tip_height, height
            );
            self.network_tip_height = height;
        }
        if slot > self.network_tip_slot {
            debug!(
                "Network tip slot updated from peer status: {} -> {}",
                self.network_tip_slot, slot
            );
            self.network_tip_slot = slot;
        }

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

        // Also update network tip from peer status (same as add_peer)
        if height > self.network_tip_height {
            self.network_tip_height = height;
        }
        if slot > self.network_tip_slot {
            self.network_tip_slot = slot;
        }

        // Check if we should start syncing (same as add_peer)
        // This ensures we re-sync when peers advance beyond our height
        if self.state == SyncState::Idle && self.should_sync() {
            self.start_sync();
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

    /// Get an iterator over all connected peer IDs
    pub fn peer_ids(&self) -> impl Iterator<Item = PeerId> + '_ {
        self.peers.keys().copied()
    }

    /// Get the best (highest) height among all connected peers AND network gossip
    /// This considers both individual peer statuses and blocks received via gossip
    /// Returns 0 if no network data is available
    pub fn best_peer_height(&self) -> u64 {
        let peer_max = self
            .peers
            .values()
            .map(|p| p.best_height)
            .max()
            .unwrap_or(0);
        // Return the higher of peer data or network gossip tip
        peer_max.max(self.network_tip_height)
    }

    /// Get the best (highest) slot among all connected peers AND network gossip
    /// This considers both individual peer statuses and blocks received via gossip
    /// Returns 0 if no network data is available
    pub fn best_peer_slot(&self) -> u32 {
        let peer_max = self.peers.values().map(|p| p.best_slot).max().unwrap_or(0);
        // Return the higher of peer data or network gossip tip
        peer_max.max(self.network_tip_slot)
    }

    /// Update the network tip slot from a received block via gossip
    ///
    /// This should be called when receiving blocks from gossip, before applying them.
    /// Unlike `update_peer()`, this doesn't require knowing which peer sent the block.
    /// It tracks "what slot the network has reached" based on blocks we've seen.
    ///
    /// Note: Height is updated through the normal block application path (update_local_tip),
    /// since blocks don't directly contain their height - it's computed from chain position.
    pub fn update_network_tip_slot(&mut self, slot: u32) {
        if slot > self.network_tip_slot {
            debug!(
                "Network tip slot updated from gossip: {} -> {}",
                self.network_tip_slot, slot
            );
            self.network_tip_slot = slot;
        }
    }

    /// Update network tip height when we successfully apply a block
    /// This is called after block application, not from gossip
    pub fn update_network_tip_height(&mut self, height: u64) {
        if height > self.network_tip_height {
            self.network_tip_height = height;
        }
    }

    /// Get current network tip (from gossip and applied blocks)
    pub fn network_tip(&self) -> (u64, u32) {
        (self.network_tip_height, self.network_tip_slot)
    }

    // =========================================================================
    // PRODUCTION GATE - Single source of truth for block production authorization
    // =========================================================================

    /// Check if block production is authorized - THE SINGLE SOURCE OF TRUTH
    ///
    /// This method implements defense-in-depth for production safety:
    /// 1. Explicit block check (invariant violations, manual blocks)
    /// 2. Resync-in-progress check
    /// 3. Active sync check (downloading headers/bodies/processing)
    /// 4. Post-resync grace period check
    /// 5. Peer synchronization check (within N slots/heights)
    ///
    /// ALL checks must pass for production to be authorized.
    pub fn can_produce(&self, current_slot: u32) -> ProductionAuthorization {
        // Layer 1: Explicit production block
        if self.production_blocked {
            return ProductionAuthorization::BlockedExplicit {
                reason: self
                    .production_block_reason
                    .clone()
                    .unwrap_or_else(|| "Unknown reason".to_string()),
            };
        }

        // Layer 2: Resync in progress
        if self.resync_in_progress {
            return ProductionAuthorization::BlockedResync {
                grace_remaining_secs: self.resync_grace_period_secs,
            };
        }

        // Layer 3: Active sync in progress
        if self.state.is_syncing() {
            return ProductionAuthorization::BlockedSyncing;
        }

        // Layer 4: Bootstrap gate - CRITICAL for preventing isolated forks
        //
        // Defense in depth: We use is_in_bootstrap_phase() which DERIVES the bootstrap
        // state from actual conditions (height == 0, lost peers, etc.) rather than
        // relying on stored flags. This makes invalid states impossible.
        //
        // During bootstrap, ALL nodes start at height 0. If late-joining nodes only
        // connect to other late-joining nodes (also at height 0), they'll think
        // they're caught up and produce at height 1 - creating isolated forks.
        //
        // The fix: Wait until we have CREDIBLE EVIDENCE the network has advanced:
        // - Either a block arrived via gossip (network_tip_slot > 0)
        // - Or a peer reported height > 0
        // - Or we ARE at height 0 and can legitimately produce the first block
        //
        // This prevents the scenario where late nodes produce competing genesis chains.
        if self.is_in_bootstrap_phase() && self.has_connected_to_peer {
            // Bootstrap phase detected (derived from state):
            // - height == 0: We're at genesis, need to verify network state
            // - peers empty after connecting: Lost all peers, need to re-establish
            //
            // We need evidence the network is real before producing

            // Check 1: Have we received any peer status at all?
            if self.first_peer_status_received.is_none() {
                return ProductionAuthorization::BlockedBootstrap {
                    reason: "Waiting for peer status response".to_string(),
                };
            }

            // Check 2: If we lost all peers (height > 0 but peers empty), wait for reconnection
            if self.local_height > 0 && self.peers.is_empty() {
                return ProductionAuthorization::BlockedBootstrap {
                    reason: "Lost all peers - waiting for reconnection".to_string(),
                };
            }

            // Check 3: Have we seen any chain activity? (block via gossip OR peer with height > 0)
            let has_chain_activity = self.network_tip_slot > 0
                || self.network_tip_height > 0
                || self.best_peer_height() > 0
                || self.best_peer_slot() > 0;

            if !has_chain_activity {
                // All peers are at height 0 too - this could be:
                // (A) True genesis - we're the first producer
                // (B) Partition of late nodes - dangerous!
                //
                // To distinguish: Wait for the bootstrap grace period.
                // If we're truly first, no harm in waiting a bit.
                // If we're partitioned, waiting gives us time to connect to the real network.
                if let Some(first_status) = self.first_peer_status_received {
                    let elapsed = first_status.elapsed().as_secs();
                    if elapsed < self.bootstrap_grace_period_secs {
                        // Still in bootstrap window - wait longer for chain evidence
                        return ProductionAuthorization::BlockedBootstrap {
                            reason: format!(
                                "All peers at height 0 - waiting for chain evidence ({}s/{}s)",
                                elapsed, self.bootstrap_grace_period_secs
                            ),
                        };
                    }
                    // Grace period expired - if still no chain activity, we're probably first
                    // Allow production
                }
            }
        }

        // Layer 5: Post-resync grace period
        if let Some(completed) = self.last_resync_completed {
            let elapsed = completed.elapsed().as_secs();
            // Exponential backoff: base grace * 2^(consecutive_resyncs - 1)
            let effective_grace = if self.consecutive_resync_count > 1 {
                self.resync_grace_period_secs * (1 << (self.consecutive_resync_count - 1).min(4))
            } else {
                self.resync_grace_period_secs
            };

            if elapsed < effective_grace {
                return ProductionAuthorization::BlockedResync {
                    grace_remaining_secs: effective_grace - elapsed,
                };
            }
        }

        // Layer 6: Peer synchronization check
        let best_peer_height = self.best_peer_height();
        let best_peer_slot = self.best_peer_slot();

        // Only check if we have peer data
        if self.peers.len() > 0 && (best_peer_height > 0 || best_peer_slot > 0) {
            let height_diff = best_peer_height.saturating_sub(self.local_height);
            let slot_diff = best_peer_slot.saturating_sub(self.local_slot);

            // Per spec: "within 2 slots of peers" - we check both slots AND heights
            if height_diff > self.max_heights_behind || slot_diff > self.max_slots_behind {
                return ProductionAuthorization::BlockedBehindPeers {
                    local_height: self.local_height,
                    peer_height: best_peer_height,
                    height_diff,
                };
            }
        }

        // All checks passed - production is authorized
        ProductionAuthorization::Authorized
    }

    /// Quick boolean check for production authorization
    pub fn is_production_safe(&self, current_slot: u32) -> bool {
        matches!(
            self.can_produce(current_slot),
            ProductionAuthorization::Authorized
        )
    }

    /// Explicitly block production (e.g., due to invariant violation)
    pub fn block_production(&mut self, reason: &str) {
        warn!("Production blocked: {}", reason);
        self.production_blocked = true;
        self.production_block_reason = Some(reason.to_string());
    }

    /// Clear explicit production block
    pub fn unblock_production(&mut self) {
        if self.production_blocked {
            info!("Production unblocked");
            self.production_blocked = false;
            self.production_block_reason = None;
        }
    }

    /// Signal that a forced resync is starting
    ///
    /// This blocks production until the resync completes and grace period expires.
    pub fn start_resync(&mut self) {
        info!("Resync started - production blocked");
        self.resync_in_progress = true;
        self.consecutive_resync_count += 1;

        // Log exponential backoff info
        if self.consecutive_resync_count > 1 {
            let effective_grace =
                self.resync_grace_period_secs * (1 << (self.consecutive_resync_count - 1).min(4));
            warn!(
                "Consecutive resync #{} - grace period extended to {}s",
                self.consecutive_resync_count, effective_grace
            );
        }
    }

    /// Signal that a forced resync has completed
    ///
    /// Starts the grace period timer before production can resume.
    pub fn complete_resync(&mut self) {
        info!("Resync completed - starting grace period");
        self.resync_in_progress = false;
        self.last_resync_completed = Some(Instant::now());
    }

    /// Reset consecutive resync counter (call after stable operation)
    pub fn reset_resync_counter(&mut self) {
        if self.consecutive_resync_count > 0 {
            debug!(
                "Resetting consecutive resync counter (was {})",
                self.consecutive_resync_count
            );
            self.consecutive_resync_count = 0;
        }
    }

    /// Check if a resync is currently in progress
    pub fn is_resync_in_progress(&self) -> bool {
        self.resync_in_progress
    }

    /// Get the current consecutive resync count
    pub fn consecutive_resync_count(&self) -> u32 {
        self.consecutive_resync_count
    }

    /// Signal that we have connected to at least one peer
    ///
    /// This enables the bootstrap gate - production will be blocked until
    /// we receive at least one peer status response.
    pub fn set_peer_connected(&mut self) {
        if !self.has_connected_to_peer {
            debug!("First peer connection - enabling bootstrap gate");
            self.has_connected_to_peer = true;
        }
    }

    /// Signal that we received a valid peer status response
    ///
    /// This is called when a PeerStatus arrives. It updates the timestamps
    /// used by the bootstrap gate to determine if we have peer info.
    pub fn note_peer_status_received(&mut self) {
        let now = Instant::now();
        // Track FIRST status for grace period calculation
        if self.first_peer_status_received.is_none() {
            self.first_peer_status_received = Some(now);
            debug!("First peer status received - starting bootstrap grace period");
        }
        self.last_peer_status_received = Some(now);
    }

    /// Check if bootstrap gate is satisfied (have peer status or grace period expired)
    pub fn is_bootstrap_ready(&self) -> bool {
        if !self.has_connected_to_peer {
            // No peers connected yet - standalone mode, OK to produce
            return true;
        }
        // Need at least one peer status
        self.first_peer_status_received.is_some()
    }

    /// Check if we're in bootstrap phase - DERIVED FROM STATE, NOT STORED
    ///
    /// Defense in depth: This method computes bootstrap state from actual conditions
    /// rather than relying on a stored flag. This makes invalid states impossible:
    /// - If height == 0, we're definitionally in bootstrap (no matter how we got here)
    /// - If we lost all peers after connecting, we need to re-bootstrap
    ///
    /// This is the "make invalid states unrepresentable" principle.
    pub fn is_in_bootstrap_phase(&self) -> bool {
        // Primary: at genesis height = ALWAYS bootstrap mode
        // This is the key insight: height 0 means newbie, period.
        if self.local_height == 0 {
            return true;
        }

        // Secondary: connected to peers but lost them all
        // This could indicate network partition or need to resync
        if self.has_connected_to_peer && self.peers.is_empty() {
            return true;
        }

        false
    }

    /// Configure the production gate settings
    pub fn configure_production_gate(
        &mut self,
        grace_period_secs: u64,
        max_slots_behind: u32,
        max_heights_behind: u64,
    ) {
        self.resync_grace_period_secs = grace_period_secs;
        self.max_slots_behind = max_slots_behind;
        self.max_heights_behind = max_heights_behind;
    }

    // =========================================================================
    // END PRODUCTION GATE
    // =========================================================================

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

        // Also update network tip - if we applied a block, the network has at least this height/slot
        // This helps the "behind peers" check work correctly even when peer status is stale
        self.update_network_tip_height(height);
        if slot > self.network_tip_slot {
            self.network_tip_slot = slot;
        }

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
    ///
    /// This method:
    /// 1. Blocks production via the production gate
    /// 2. Resets all sync state to genesis
    /// 3. Triggers active sync from peers
    ///
    /// Production remains blocked until:
    /// - Sync completes (complete_resync() is called)
    /// - Grace period expires (configurable, default 30s)
    pub fn reset_local_state(&mut self, genesis_hash: Hash) {
        // CRITICAL: Block production FIRST via the production gate
        self.start_resync();

        // Reset chain tip to genesis
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
        self.header_downloader = HeaderDownloader::new(
            self.config.max_headers_per_request,
            self.config.request_timeout,
        );
        self.body_downloader = BodyDownloader::new(
            self.config.max_bodies_per_request,
            self.config.max_concurrent_body_requests,
            self.config.request_timeout,
        );

        info!(
            "Sync manager reset to genesis for forced resync (consecutive #{}, production blocked)",
            self.consecutive_resync_count
        );

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
