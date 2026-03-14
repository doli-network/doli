//! Sync manager coordinating chain synchronization
//!
//! The SyncManager orchestrates the synchronization process:
//! 1. Discovers peers with higher chain tip
//! 2. Downloads headers first to validate VDF chain
//! 3. Downloads block bodies in parallel
//! 4. Applies blocks to local chain

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use libp2p::PeerId;
use tracing::{debug, info, warn};

use crypto::Hash;
use doli_core::{Block, BlockHeader};

use super::bodies::BodyDownloader;
use super::headers::HeaderDownloader;
use super::reorg::ReorgHandler;
use crate::protocols::{SyncRequest, SyncResponse};

/// Minimum peers required for block production, by tier.
/// Sane defaults for small-to-medium networks. The cap in set_tier()
/// ensures min_peers <= active_producers - 1, preventing deadlock.
const MIN_PEERS_TIER1: usize = 3;
const MIN_PEERS_TIER2: usize = 2;
const MIN_PEERS_TIER3: usize = 1;

/// Maximum consecutive force-resyncs before giving up.
/// After this many failed resyncs, the node stops retrying and requires
/// manual intervention (`recover --yes` or wipe + resync).
pub const MAX_CONSECUTIVE_RESYNCS: u32 = 5;

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
            max_headers_per_request: 500,
            max_bodies_per_request: 128,
            max_concurrent_body_requests: 8,
            request_timeout: Duration::from_secs(30),
            min_peers_for_sync: 1,
            stale_timeout: Duration::from_secs(300), // 5 minutes - peers stay active longer
        }
    }
}

/// Synchronization state
#[derive(Clone, Debug)]
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
    /// Snap sync: collecting state root votes from peers
    SnapCollectingRoots {
        /// Target block hash to snapshot at (initial estimate, may differ from quorum)
        target_hash: Hash,
        /// Target block height (initial estimate)
        target_height: u64,
        /// Collected (peer, block_hash, block_height, state_root) votes
        votes: Vec<(PeerId, Hash, u64, Hash)>,
        /// Peers already asked
        asked: HashSet<PeerId>,
        /// When this phase started
        started_at: Instant,
    },
    /// Snap sync: downloading full snapshot from a peer
    SnapDownloading {
        /// Target block hash
        target_hash: Hash,
        /// Target block height
        target_height: u64,
        /// Quorum-agreed state root
        quorum_root: Hash,
        /// Peer serving the snapshot
        peer: PeerId,
        /// Alternate peers that agreed on the same quorum root (fallback on error)
        alternate_peers: Vec<(PeerId, Hash, u64)>,
        /// When download started
        started_at: Instant,
    },
    /// Snap sync: snapshot ready for node to consume
    SnapReady {
        /// The verified snapshot
        snapshot: VerifiedSnapshot,
    },
}

/// A verified state snapshot ready for application by the node.
///
/// SyncManager collects this from the network; the Node verifies
/// the state root and applies it.
#[derive(Clone, Debug)]
pub struct VerifiedSnapshot {
    /// Block hash this snapshot is valid at
    pub block_hash: Hash,
    /// Block height
    pub block_height: u64,
    /// Serialized ChainState (bincode)
    pub chain_state: Vec<u8>,
    /// Serialized UtxoSet (bincode)
    pub utxo_set: Vec<u8>,
    /// Serialized ProducerSet (bincode)
    pub producer_set: Vec<u8>,
    /// Quorum-agreed state root (node re-verifies)
    pub state_root: Hash,
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
    /// Production is blocked - we're suspiciously ahead of all peers (likely forked)
    BlockedAheadOfPeers {
        /// Our height
        local_height: u64,
        /// Best peer height
        peer_height: u64,
        /// How far ahead we are
        height_ahead: u64,
    },
    /// Production blocked: critical chain mismatch with connected peer (P0 #1)
    BlockedChainMismatch {
        peer_id: PeerId,
        local_hash: Hash,
        peer_hash: Hash,
        local_height: u64,
    },
    /// Production is blocked - too few peers to safely validate chain (echo chamber prevention)
    BlockedInsufficientPeers {
        /// Current peer count
        peer_count: usize,
        /// Minimum required
        min_required: usize,
    },
    /// Production is blocked due to excessive sync failures (likely on a fork)
    BlockedSyncFailures {
        /// How many consecutive sync failures
        failure_count: u32,
    },
    /// Production is blocked due to no gossip activity (likely isolated)
    BlockedNoGossipActivity {
        /// Seconds since last gossip block
        seconds_since_gossip: u64,
        /// Number of connected peers
        peer_count: usize,
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
    /// Production is blocked because our chain conflicts with a finalized block.
    BlockedConflictsFinality {
        /// Height of the last finalized block.
        local_finalized_height: u64,
    },
    /// Production is blocked after snap sync until a canonical gossip block is received.
    /// Prevents producing on an empty block store before the node proves it's on the
    /// canonical chain.
    BlockedAwaitingCanonicalBlock,
}

impl SyncState {
    /// Check if we're actively syncing
    pub fn is_syncing(&self) -> bool {
        !matches!(self, SyncState::Idle | SyncState::Synchronized)
    }

    /// Check if snap sync is in progress (collecting roots or downloading snapshot)
    pub fn is_snap_syncing(&self) -> bool {
        matches!(
            self,
            SyncState::SnapCollectingRoots { .. } | SyncState::SnapDownloading { .. }
        )
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
    /// Time of last status message (Ping/Status) - Reachability
    pub last_status_response: Instant,
    /// Time of last block received via gossip or sync - Data Flow (P1 #5)
    pub last_block_received: Option<Instant>,
    /// Pending sync request ID
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
    /// Sync epoch when this request was created. Responses from old epochs are discarded.
    epoch: u64,
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
    /// Max heights ahead of network before blocking (P0 #2)
    max_heights_ahead: u64,

    // === GOSSIP WATCHDOG FIELDS (P0 #3) ===
    /// Last time we received a block via gossip (for gossip activity watchdog)
    last_block_received_via_gossip: Option<Instant>,
    /// Gossip inactivity timeout in seconds (default 180s = 3 minutes)
    gossip_activity_timeout_secs: u64,
    /// Maximum slots to produce solo without receiving any peer block via gossip.
    /// After this many slots (slot_duration * N seconds of silence while we're the tip),
    /// production pauses to avoid building a long orphan chain in isolation.
    /// Default: 5 slots (50s at 10s/slot).
    max_solo_production_secs: u64,

    // === SYNC FAILURE TRACKING (P0 #4) ===
    /// Consecutive sync failures (empty header responses)
    consecutive_sync_failures: u32,
    /// Max sync failures before fork detection triggers
    max_sync_failures_before_fork_detection: u32,

    /// Minimum peers required for production (P0 #5)
    min_peers_for_production: usize,

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

    /// Producer tier (0=default, 1=validator, 2=attestor, 3=staker)
    tier: u8,

    /// Finality tracker — tracks attestation weight and determines finalized blocks.
    finality_tracker: doli_core::FinalityTracker,

    /// Fork recovery tracker — active parent chain download for automatic reorg.
    fork_recovery: super::fork_recovery::ForkRecoveryTracker,

    /// Fork sync — binary search for common ancestor when on a dead fork.
    fork_sync: Option<super::fork_sync::ForkSync>,

    /// Last time we received ANY block (gossip or sync). Used for stale chain detection.
    last_block_seen: Instant,

    /// Last time we successfully applied a block. Used to detect stuck Processing state.
    /// Unlike `last_block_seen` (reset by gossip), this only resets on actual chain advancement.
    last_block_applied: Instant,

    /// Last time any sync progress occurred (headers received, bodies received, blocks applied).
    /// Used by cleanup() to detect truly stuck sync — unlike `last_block_applied`, this covers
    /// all phases of header-first sync, not just the final block-application phase.
    last_sync_activity: Instant,

    /// Consecutive empty header responses (peers don't recognize our chain tip).
    /// When >= 10, we're on a deep fork and need genesis resync.
    /// Only incremented for truly EMPTY responses (peer doesn't have our hash).
    consecutive_empty_headers: u32,

    /// Flag set when genesis fallback confirms peers reject our chain —
    /// node must call force_resync_from_genesis().
    needs_genesis_resync: bool,

    /// Monotonic epoch counter incremented on every new sync cycle (start_sync).
    /// Requests are tagged with the epoch they belong to; responses from old
    /// epochs are discarded to prevent stale data from corrupting the current cycle.
    sync_epoch: u64,

    /// Retry counter for body download stalls (DownloadingBodies).
    /// Reset when blocks are applied. After a few retries, we fall back
    /// to the existing hard reset-to-Idle behavior to avoid infinite loops.
    body_stall_retries: u32,

    /// Consecutive block-apply failures (downloaded chain can't be applied).
    /// Unlike consecutive_empty_headers (reset by header downloads), this is
    /// ONLY reset by successful block_applied(). At >= 3, triggers genesis resync.
    consecutive_apply_failures: u32,

    /// Timestamp when all peers were lost (for peer loss timeout).
    /// After `peer_loss_timeout_secs`, production resumes solo.
    peers_lost_at: Option<Instant>,
    /// Seconds to wait after losing all peers before resuming production.
    peer_loss_timeout_secs: u64,

    // === SNAP SYNC FIELDS ===
    /// Height gap threshold to trigger snap sync instead of header-first
    snap_sync_threshold: u64,
    /// Number of peers that must agree on the same state root
    snap_sync_quorum: usize,
    /// Timeout for collecting state root votes
    snap_root_timeout: Duration,
    /// Timeout for downloading snapshot from a peer
    snap_download_timeout: Duration,
    /// Peers that served bad snapshots (blacklisted for this session)
    snap_blacklisted_peers: HashSet<PeerId>,
    /// Peers that returned empty headers (blacklisted with cooldown)
    header_blacklisted_peers: HashMap<PeerId, Instant>,
    /// Number of consecutive snap sync failures. After 3 failures, falls back
    /// to header-first sync. Reset on successful sync or state reset.
    snap_sync_attempts: u8,
    /// When a fresh node first started waiting for 5 peers (for snap sync timeout)
    fresh_node_wait_start: Option<Instant>,
    /// After snap sync, block production until at least one canonical gossip block
    /// is received and applied. This proves the node is on the canonical chain and
    /// its block store has a real parent to build on.
    awaiting_canonical_block: bool,
    /// Lowest block height available in the local block store.
    /// Set by the node after startup / snap sync. Used by fork sync to
    /// avoid binary-searching below available block data.
    store_floor: u64,

    /// Counter for ticks where we're Idle but behind peers by a small gap (≤5 blocks).
    /// After 5 ticks (~25s at 5s polling), forces a full sync restart instead of
    /// waiting for gossip that may never arrive due to I/O contention.
    idle_behind_retries: u32,

    /// Persistent fork detection flag.
    ///
    /// Set when Layer 9 (hash mismatch) detects we're in the minority.
    /// Blocks production until cleared by a successful resync from peers.
    ///
    /// Without this, Layer 9 oscillates: it detects the fork, blocks production,
    /// peers advance beyond the ±2 comparison window, Layer 9 "forgets" the fork,
    /// and the node resumes producing on its orphan chain. This flag prevents
    /// that oscillation by remembering "we saw a fork, don't produce until resolved."
    fork_mismatch_detected: bool,

    /// NT10 fix: Set by `reset_sync_for_rollback()` to signal that the next
    /// `start_sync()` call should NOT use header-first sync. After a fork rollback,
    /// our tip is still on the fork (just 1 block shorter), so header-first will
    /// always get 0 headers. Instead, attempt fork_sync or escalate to snap sync.
    post_rollback: bool,

    /// Consecutive empty header responses with small gap (≤50).
    /// Each occurrence is likely "peer already applied those blocks" (normal),
    /// but 5+ consecutive means gossip isn't delivering and we may be on a fork.
    /// At >= 5, escalate to fork_sync instead of waiting forever.
    consecutive_small_gap_empty_headers: u32,

    /// Set after snap sync / force_recover to suppress fork_sync reactivation.
    /// The node needs time to sync via header-first / gossip before fork detection
    /// makes sense. Cleared after 10+ blocks are applied post-recovery.
    post_recovery_grace: bool,
    /// Blocks applied since last recovery — used to clear post_recovery_grace.
    blocks_applied_since_recovery: u32,
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
            max_heights_ahead: 5,         // Fork detection: if >5 blocks ahead, suspicious
            last_block_received_via_gossip: Some(Instant::now()), // Grace period starts at boot
            gossip_activity_timeout_secs: 180, // 3 minutes default
            max_solo_production_secs: 50, // 5 slots × 10s — halt before building long orphan chain
            consecutive_sync_failures: 0,
            max_sync_failures_before_fork_detection: 3, // Block after 3 failed syncs
            min_peers_for_production: 2, // Need at least 2 peers to avoid echo chambers
            // Network tip tracking (from gossip)
            network_tip_height: 0,
            network_tip_slot: 0,
            // Bootstrap gate defaults
            has_connected_to_peer: false,
            first_peer_status_received: None,
            last_peer_status_received: None,
            bootstrap_grace_period_secs: 15, // Wait 15s at genesis for chain evidence
            tier: 0,
            finality_tracker: doli_core::FinalityTracker::new(),
            fork_recovery: super::fork_recovery::ForkRecoveryTracker::new(),
            fork_sync: None,
            last_block_seen: Instant::now(),
            last_block_applied: Instant::now(),
            last_sync_activity: Instant::now(),
            consecutive_empty_headers: 0,
            needs_genesis_resync: false,
            sync_epoch: 0,
            body_stall_retries: 0,
            consecutive_apply_failures: 0,
            peers_lost_at: None,
            peer_loss_timeout_secs: 30,
            // Snap sync defaults
            snap_sync_threshold: 1000,
            snap_sync_quorum: 5, // Minimum 5 peers for partition safety
            snap_root_timeout: Duration::from_secs(10),
            snap_download_timeout: Duration::from_secs(60),
            snap_blacklisted_peers: HashSet::new(),
            header_blacklisted_peers: HashMap::new(),
            snap_sync_attempts: 0,
            fresh_node_wait_start: None,
            awaiting_canonical_block: false,
            store_floor: 1, // Default: full-sync node has block 1
            idle_behind_retries: 0,
            fork_mismatch_detected: false,
            post_rollback: false,
            consecutive_small_gap_empty_headers: 0,
            post_recovery_grace: false,
            blocks_applied_since_recovery: 0,
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

        // Check if we're now synchronized (require both height AND slot alignment)
        if let Some(best_peer) = self.best_peer() {
            if let Some(status) = self.peers.get(&best_peer) {
                let slot_aligned = self.max_slots_behind == 0
                    || status.best_slot.saturating_sub(slot) <= self.max_slots_behind;
                if height >= status.best_height && slot_aligned {
                    let was_syncing = self.state.is_syncing();
                    self.state = SyncState::Synchronized;
                    self.header_blacklisted_peers.clear();
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

        // Clear peer loss tracker since we have a peer again
        self.peers_lost_at = None;

        self.peers.insert(
            peer,
            PeerSyncStatus {
                best_height: height,
                best_hash: hash,
                best_slot: slot,
                last_status_response: Instant::now(),
                last_block_received: None,
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
        // Note: Also check Synchronized state - after successful sync, state is Synchronized,
        // and we need to re-sync if peers advance beyond us
        // CHECKPOINT: Sync trigger check in add_peer
        let state_ok = matches!(self.state, SyncState::Idle | SyncState::Synchronized);
        if state_ok && self.should_sync() {
            self.start_sync();
        }
    }

    /// Update peer status
    pub fn update_peer(&mut self, peer: PeerId, height: u64, hash: Hash, slot: u32) {
        if let Some(status) = self.peers.get_mut(&peer) {
            status.best_height = height;
            status.best_hash = hash;
            status.best_slot = slot;
            status.last_status_response = Instant::now();
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
        // Note: Also check Synchronized state - after successful sync, state is Synchronized,
        // and we need to re-sync if peers advance beyond us
        let state_ok = matches!(self.state, SyncState::Idle | SyncState::Synchronized);
        if state_ok && self.should_sync() {
            self.start_sync();
        }
    }

    /// Refresh all peers' timestamps when activity is detected on the network
    /// Call this when receiving blocks/transactions via gossip to prevent stale timeouts
    pub fn refresh_all_peers(&mut self) {
        let now = Instant::now();
        for status in self.peers.values_mut() {
            status.last_status_response = now;
            status.last_block_received = Some(now); // Gossip proves both liveness and data flow
        }
    }

    /// Remove a peer
    pub fn remove_peer(&mut self, peer: &PeerId) {
        self.peers.remove(peer);

        // Track when we lost all peers (for peer loss timeout)
        if self.peers.is_empty() && self.peers_lost_at.is_none() {
            info!(
                "All peers lost — starting peer loss timeout ({}s)",
                self.peer_loss_timeout_secs
            );
            self.peers_lost_at = Some(Instant::now());
        }

        // Release body downloader hashes back to failed queue so they
        // can be re-requested from another peer. Without this, hashes
        // stay in in_flight forever and the body downloader stalls.
        self.body_downloader.cancel_peer(peer);

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

        // If snap downloading from this peer, try alternate or fall back
        if let SyncState::SnapDownloading {
            peer: snap_peer, ..
        } = &self.state
        {
            if snap_peer == peer {
                warn!(
                    "[SNAP_SYNC] Download peer {} disconnected — trying alternate",
                    peer
                );
                self.handle_snap_download_error(*peer);
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

    /// Get the LOWEST height among all connected peers
    /// Used for fork detection: if we're far ahead of ANY peer, something is wrong
    /// Returns None if no peers (can't determine lowest)
    pub fn lowest_peer_height(&self) -> Option<u64> {
        self.peers.values().map(|p| p.best_height).min()
    }

    /// Get the best (highest) slot among all connected peers AND network gossip
    /// This considers both individual peer statuses and blocks received via gossip
    /// Returns 0 if no network data is available
    pub fn best_peer_slot(&self) -> u32 {
        // Only use actual peer status data, not gossip-inflated network_tip_slot.
        // network_tip_slot can be permanently inflated by orphan/fork blocks
        // received via gossip before validation.
        self.peers.values().map(|p| p.best_slot).max().unwrap_or(0)
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
    pub fn can_produce(&mut self, current_slot: u32) -> ProductionAuthorization {
        // === CHECKPOINT: Entry point with all key values ===
        let best_peer_h = self.best_peer_height();
        let best_peer_s = self.best_peer_slot();
        info!(
            "[CAN_PRODUCE] slot={} local_h={} local_s={} peer_h={} peer_s={} peers={} state={:?}",
            current_slot,
            self.local_height,
            self.local_slot,
            best_peer_h,
            best_peer_s,
            self.peers.len(),
            self.state
        );

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

        // Layer 2.5: Post-snap-sync canonical block gate
        // After snap sync, the block store is empty — producing immediately would create
        // a fork because there's no real parent block to build on. Wait until at least
        // one canonical gossip block has been received and applied, proving we're on the
        // canonical chain and giving the block store a real parent.
        if self.awaiting_canonical_block {
            return ProductionAuthorization::BlockedAwaitingCanonicalBlock;
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

            // Check 2: If we lost all peers (height > 0 but peers empty), wait for reconnection.
            // After peer_loss_timeout_secs, allow production to resume solo — the peer
            // may be permanently down and halting the chain is worse than a temporary fork.
            if self.local_height > 0 && self.peers.is_empty() {
                let past_timeout = self
                    .peers_lost_at
                    .map(|t| t.elapsed().as_secs() >= self.peer_loss_timeout_secs)
                    .unwrap_or(false);
                if !past_timeout {
                    return ProductionAuthorization::BlockedBootstrap {
                        reason: "Lost all peers - waiting for reconnection".to_string(),
                    };
                }
                info!(
                    "Peer loss timeout reached ({}s) — resuming solo production at height {}",
                    self.peer_loss_timeout_secs, self.local_height
                );
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

        // Layer 5: Post-resync grace period (absorbs former Layer 5.6 first-sync grace)
        if let Some(completed) = self.last_resync_completed {
            let elapsed = completed.elapsed().as_secs();
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

        // Layer 5.5: Minimum peer count check (echo chamber prevention)
        //
        // With too few peers, a node might form an isolated cluster with other forked nodes
        // where all peers agree on the wrong chain. This check ensures we have enough
        // diverse viewpoints before trusting peer data for production decisions.
        //
        // Example: Node 8 has only 1 peer (another forked node at same height)
        // Without this check: height_ahead = 0 → "not ahead" → AUTHORIZED (bad!)
        // With this check: peers=1 < min=2 → BLOCKED (prevents echo chamber)
        info!(
            "[CAN_PRODUCE] Layer5.5: peers={} min_required={}",
            self.peers.len(),
            self.min_peers_for_production
        );
        let past_peer_loss_timeout = self
            .peers_lost_at
            .map(|t| t.elapsed().as_secs() >= self.peer_loss_timeout_secs)
            .unwrap_or(false);
        if self.peers.len() < self.min_peers_for_production
            && self.local_height > 0
            && !past_peer_loss_timeout
        {
            // Only apply this check if we're past genesis (height > 0)
            // At genesis (height 0), we may legitimately be the first producer
            // Skip if peer loss timeout expired — solo production is preferable to chain halt
            warn!(
                "FORK PREVENTION: Only {} peers (need {}) - blocking production to prevent echo chamber",
                self.peers.len(), self.min_peers_for_production
            );
            return ProductionAuthorization::BlockedInsufficientPeers {
                peer_count: self.peers.len(),
                min_required: self.min_peers_for_production,
            };
        }

        // Layer 6: Peer synchronization check - too far BEHIND
        //
        // IMPORTANT: Only compare SLOTS, not heights. Heights are unreliable because
        // forked nodes accumulate inflated block counts (height > slot). A single
        // forked peer with height=200 would block honest nodes at height=158.
        // Slots are time-based and can't be inflated by forks.
        //
        // GUARD: Skip this check when local_height >= best_peer_height.
        // When peers are still syncing (height=0) they report valid best_slot from
        // their clock, creating a false "behind" signal. A node whose height is at
        // or ahead of every peer is definitionally NOT behind the network —
        // its local_slot is only stale because it stopped producing, and it can't
        // produce because this layer blocks it, creating a deadlock.
        let best_peer_slot = self.best_peer_slot();

        // Only check if we have peer data
        if !self.peers.is_empty() && best_peer_slot > 0 {
            let slot_diff = best_peer_slot.saturating_sub(self.local_slot);

            if slot_diff > self.max_slots_behind {
                let best_peer_height = self.best_peer_height();

                // Guard: If we're at or ahead of all peers by height, slot lag
                // is a stale artifact — we ARE the tip, just haven't produced
                // recently. Don't block; let production advance local_slot.
                if self.local_height >= best_peer_height {
                    info!(
                        "[CAN_PRODUCE] Layer6: slot_diff={} exceeds max={}, but local_height={} >= peer_height={} - allowing",
                        slot_diff, self.max_slots_behind, self.local_height, best_peer_height
                    );
                } else {
                    return ProductionAuthorization::BlockedBehindPeers {
                        local_height: self.local_height,
                        peer_height: best_peer_height,
                        height_diff: best_peer_height.saturating_sub(self.local_height),
                    };
                }
            }
        }

        // Layer 6.5: Height lag check — block production when behind by >= 2 blocks.
        //
        // If we know peers have a higher height, producing from our stale state
        // creates a competing block that forks the chain. This is the #1 cause of
        // fork-then-snap-sync on nodes under I/O contention (ai2 testnet pattern).
        //
        // Unlike slot-based Layer 6, height lag is reliable for BEHIND detection:
        // a forked node can inflate its own height, but a behind node's height is
        // definitionally lower than the canonical chain. Threshold of 2 allows
        // normal 1-block propagation delay without blocking.
        let best_peer_height = self.best_peer_height();
        if !self.peers.is_empty() && best_peer_height > 0 {
            let height_lag = best_peer_height.saturating_sub(self.local_height);
            if height_lag >= 2 {
                info!(
                    "[CAN_PRODUCE] Layer6.5: BLOCKED — local_h={} peer_h={} lag={} (must catch up before producing)",
                    self.local_height, best_peer_height, height_lag
                );
                return ProductionAuthorization::BlockedBehindPeers {
                    local_height: self.local_height,
                    peer_height: best_peer_height,
                    height_diff: height_lag,
                };
            }
        }

        // Layer 7: REMOVED — Satoshi principle: always extend your best chain.
        //  Fork detection via AheadOfPeers caused chain deadlock (2026-02-25).
        //  When the tip node's peers are syncing behind, AheadOfPeers blocks
        //  production, which prevents peers from catching up, creating a
        //  permanent deadlock where nobody produces.
        //  Forks are resolved by: (1) longest chain reorg, (2) sync failures (Layer 8),
        //  (3) chain mismatch detection (Layer 9).
        info!(
            "[CAN_PRODUCE] Layer7: SKIPPED (removed) — peers={} best_peer={} local={} ahead={}",
            self.peers.len(),
            best_peer_height,
            self.local_height,
            self.local_height.saturating_sub(best_peer_height)
        );

        // Layer 9: Chain Hash Verification — INFORMATIONAL ONLY
        //
        // When our chain has diverged from peers:
        // - GetHeaders requests return empty (peer doesn't have our tip as ancestor)
        // - This increments consecutive_sync_failures
        // - After 3+ failures, we're likely on a fork
        //
        // This catches forks where height comparison is inconclusive.
        info!(
            "[CAN_PRODUCE] Layer8: sync_failures={} max_failures={}",
            self.consecutive_sync_failures, self.max_sync_failures_before_fork_detection
        );
        if self.consecutive_sync_failures >= self.max_sync_failures_before_fork_detection {
            warn!(
                "FORK DETECTION: {} consecutive sync failures - blocking production",
                self.consecutive_sync_failures
            );
            return ProductionAuthorization::BlockedSyncFailures {
                failure_count: self.consecutive_sync_failures,
            };
        }

        // Layer 8.5: Persistent fork mismatch flag.
        //
        // If a prior Layer 9 check detected we're in the minority, keep blocking
        // until a successful resync clears the flag. Without this, Layer 9 oscillates:
        // detects fork → blocks → peers advance beyond ±2 window → Layer 9 forgets
        // → node resumes producing on orphan chain → repeat.
        if self.fork_mismatch_detected {
            warn!(
                "[CAN_PRODUCE] Layer8.5: BLOCKED — fork_mismatch_detected flag set, awaiting resync (local_h={})",
                self.local_height
            );
            return ProductionAuthorization::BlockedChainMismatch {
                peer_id: self
                    .peers
                    .keys()
                    .next()
                    .copied()
                    .unwrap_or_else(PeerId::random),
                local_hash: self.local_hash,
                peer_hash: Hash::default(),
                local_height: self.local_height,
            };
        }

        // Layer 9: Chain Hash Verification (P0 #1)
        //
        // Count peers at same height that agree (same hash) vs disagree (different hash).
        // Only block production if we're in the clear minority — the majority keeps
        // producing so the heaviest chain rule resolves the fork naturally.
        let mut agree = 1u32; // Count ourselves — we agree with our own chain
        let mut disagree = 0u32;
        let mut first_mismatch_peer = None;
        let mut first_mismatch_hash = self.local_hash;
        for (peer_id, status) in &self.peers {
            if status.best_height == self.local_height {
                // Same height: compare hashes directly
                if status.best_hash == self.local_hash {
                    agree += 1;
                } else {
                    disagree += 1;
                    if first_mismatch_peer.is_none() {
                        first_mismatch_peer = Some(*peer_id);
                        first_mismatch_hash = status.best_hash;
                    }
                }
            } else if status.best_height > self.local_height
                && status.best_height <= self.local_height + 2
            {
                // Peer is 1-2 blocks ahead.
                // Hash::ZERO means peer has a snap sync gap — no block store
                // for this height. This is NOT a fork — skip silently.
                if status.best_hash == Hash::ZERO {
                    continue;
                }
                // Different non-zero hash = genuine divergence.
                disagree += 1;
                if first_mismatch_peer.is_none() {
                    first_mismatch_peer = Some(*peer_id);
                    first_mismatch_hash = status.best_hash;
                }
            }
        }
        // Only block if we're in the minority — majority keeps producing
        if disagree > 0 && agree < disagree {
            if let Some(peer_id) = first_mismatch_peer {
                warn!(
                    "FORK DETECTION: We are in minority at height {} ({} agree, {} disagree) — setting persistent fork flag",
                    self.local_height, agree, disagree
                );
                self.fork_mismatch_detected = true;
                return ProductionAuthorization::BlockedChainMismatch {
                    peer_id,
                    local_hash: self.local_hash,
                    peer_hash: first_mismatch_hash,
                    local_height: self.local_height,
                };
            }
        }

        // Layer 10: Gossip Activity Watchdog (P0 #3)
        //
        // If we have peers but haven't received ANY blocks via gossip for a long time,
        // we are likely isolated (e.g., in a "ping-only" partition).
        // Exceptions:
        // - No peers connected (handled by MinPeers check)
        // - Initial bootstrap (handled by BootstrapGate)
        // - No peer is ahead of us: gossip silence is expected when WE are the tip.
        //   Without this exception, all nodes deadlock: nobody produces → no gossip
        //   → watchdog blocks everyone → permanent halt.
        if !self.peers.is_empty() && best_peer_height > self.local_height {
            let last_gossip = self
                .last_block_received_via_gossip
                .unwrap_or(Instant::now());
            let elapsed = last_gossip.elapsed();

            if elapsed.as_secs() > self.gossip_activity_timeout_secs {
                warn!(
                    "FORK DETECTION: No gossip activity for {}s (timeout {}) with {} peers (peer_h={} > local_h={}) - blocking production",
                    elapsed.as_secs(), self.gossip_activity_timeout_secs, self.peers.len(),
                    best_peer_height, self.local_height
                );
                return ProductionAuthorization::BlockedNoGossipActivity {
                    seconds_since_gossip: elapsed.as_secs(),
                    peer_count: self.peers.len(),
                };
            }
        }

        // Layer 10.5: Solo production circuit breaker
        //
        // Complement to Layer 10: If WE are the tip (local_height >= best_peer_height),
        // gossip silence is expected — Layer 10 allows it. But if gossip has been silent
        // for max_solo_production_secs (default 50s = 5 slots), we're likely building
        // an orphan chain in isolation. Pause to prevent long parallel forks.
        //
        // Exception: at genesis (height <= 1) — first blocks legitimately have no gossip.
        if self.local_height > 1 && self.local_height >= best_peer_height {
            let last_gossip = self
                .last_block_received_via_gossip
                .unwrap_or(Instant::now());
            let silence_secs = last_gossip.elapsed().as_secs();

            if silence_secs > self.max_solo_production_secs {
                warn!(
                    "CIRCUIT BREAKER: Produced solo for {}s (limit {}s) with no gossip blocks received. \
                     Pausing production to avoid building orphan chain. local_h={} peer_h={}",
                    silence_secs, self.max_solo_production_secs,
                    self.local_height, best_peer_height
                );
                return ProductionAuthorization::BlockedNoGossipActivity {
                    seconds_since_gossip: silence_secs,
                    peer_count: self.peers.len(),
                };
            }
        }

        // Layer 11: Finality conflict check
        // If we have a finalized block, ensure our chain doesn't conflict with it.
        // This prevents producing blocks on a fork that has been superseded by finality.
        if let Some(finalized_height) = self.last_finalized_height() {
            if self.local_height < finalized_height {
                info!(
                    "[CAN_PRODUCE] Layer11: local_height={} < finalized_height={} - blocked",
                    self.local_height, finalized_height
                );
                return ProductionAuthorization::BlockedConflictsFinality {
                    local_finalized_height: finalized_height,
                };
            }
        }

        // All checks passed - production is authorized
        info!("[CAN_PRODUCE] AUTHORIZED - all checks passed");
        ProductionAuthorization::Authorized
    }

    /// Quick boolean check for production authorization
    pub fn is_production_safe(&mut self, current_slot: u32) -> bool {
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

    /// Clear the post-snap-sync production gate.
    /// Called when a canonical gossip block has been successfully applied,
    /// proving we're on the canonical chain.
    pub fn clear_awaiting_canonical_block(&mut self) {
        if self.awaiting_canonical_block {
            info!("[SNAP_SYNC] Canonical gossip block received — production gate cleared");
            self.awaiting_canonical_block = false;
        }
        if self.fork_mismatch_detected {
            info!("[FORK_RECOVERY] Canonical gossip block applied — fork mismatch flag cleared");
            self.fork_mismatch_detected = false;
        }
    }

    /// Check if we're waiting for a canonical block after snap sync.
    pub fn is_awaiting_canonical_block(&self) -> bool {
        self.awaiting_canonical_block
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

    /// Get blocks applied since last reset (indicates active sync progress)
    pub fn blocks_applied(&self) -> u64 {
        self.blocks_applied
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

    /// Set the bootstrap grace period (wait time at genesis for chain evidence)
    ///
    /// At genesis, when all peers are at height 0, the node waits this duration
    /// before allowing block production. This helps distinguish between:
    /// - True genesis (we're first producer, safe to start)
    /// - Network partition (we're isolated, dangerous to produce)
    pub fn set_bootstrap_grace_period_secs(&mut self, secs: u64) {
        self.bootstrap_grace_period_secs = secs;
    }

    /// Set the maximum heights ahead threshold for fork detection (P0 #2)
    pub fn set_max_heights_ahead(&mut self, heights: u64) {
        self.max_heights_ahead = heights;
    }

    /// Set the minimum peers required for production (P0 #5 echo chamber prevention)
    ///
    /// - For mainnet/testnet: 2 (default) - require multiple peers to prevent echo chambers
    /// - For devnet without DHT: 1 - allow single-peer production since discovery is limited
    pub fn set_min_peers_for_production(&mut self, min_peers: usize) {
        self.min_peers_for_production = min_peers;
        info!(
            "Set min_peers_for_production to {} for echo chamber prevention",
            min_peers
        );
    }

    /// Set the producer tier and adjust min_peers_for_production accordingly.
    ///
    /// If no blocks are received via gossip for this duration, production is blocked.
    /// This should be calibrated to the slot duration (e.g., 18 * slot_duration).
    pub fn set_gossip_activity_timeout_secs(&mut self, secs: u64) {
        self.gossip_activity_timeout_secs = secs;
        info!("Set gossip_activity_timeout_secs to {} seconds", secs);
    }

    /// Set the producer tier and adjust min_peers_for_production accordingly.
    ///
    /// Tier 1 validators need more peers (dense mesh), Tier 3 stakers need fewer.
    /// The `active_producer_count` caps min_peers so small networks aren't deadlocked
    /// (a node can have at most `active_producer_count - 1` peers).
    pub fn set_tier(&mut self, tier: u8, active_producer_count: usize) {
        self.tier = tier;

        // Tier-based min_peers only applies to large networks (500+ producers).
        // In small networks ALL producers are trivially "Tier 1", but the Tier 1
        // min_peers (3) is designed for dense validator meshes at scale.
        // Keep the default min_peers (2) until the network grows enough for
        // tiering to be meaningful.
        if active_producer_count < 500 {
            info!(
                "Set tier={} min_peers_for_production={} (skipped tier override: network_size={} < 500)",
                tier, self.min_peers_for_production, active_producer_count
            );
            return;
        }

        let tier_min = match tier {
            1 => MIN_PEERS_TIER1,
            2 => MIN_PEERS_TIER2,
            3 => MIN_PEERS_TIER3,
            _ => MIN_PEERS_TIER3, // Default: backward compatible
        };
        let max_possible = active_producer_count.saturating_sub(1).max(1);
        self.min_peers_for_production = tier_min.min(max_possible);
        info!(
            "Set tier={} min_peers_for_production={} (tier_req={}, network_size={})",
            tier, self.min_peers_for_production, tier_min, active_producer_count
        );
    }

    /// Get the current tier.
    pub fn tier(&self) -> u8 {
        self.tier
    }

    /// Track a newly applied block for finality.
    pub fn track_block_for_finality(
        &mut self,
        hash: crypto::Hash,
        height: u64,
        slot: u32,
        total_weight: u64,
    ) {
        self.finality_tracker
            .track_block(hash, height, slot, total_weight);
    }

    /// Add attestation weight to a pending block.
    pub fn add_attestation_weight(&mut self, block_hash: &crypto::Hash, weight: u64) {
        self.finality_tracker
            .add_attestation_weight(*block_hash, weight);
        // Check if this triggers finality
        if let Some(checkpoint) = self.finality_tracker.check_finality() {
            info!(
                "FINALITY: Block {} finalized at height {} (attestation {}/{})",
                checkpoint.block_hash,
                checkpoint.height,
                checkpoint.attestation_weight,
                checkpoint.total_weight
            );
            self.reorg_handler
                .set_last_finality_height(checkpoint.height);
        }
    }

    /// Prune stale pending blocks from the finality tracker.
    pub fn prune_finality(&mut self, current_slot: u32) {
        self.finality_tracker.prune_old_pending(current_slot);
    }

    /// Get the last finalized height, if any.
    pub fn last_finalized_height(&self) -> Option<u64> {
        self.finality_tracker
            .last_finalized
            .as_ref()
            .map(|c| c.height)
    }

    /// Check if sync failures indicate we're on a fork (no-op, kept for API compatibility)
    pub fn has_sync_failure_fork_indicator(&self) -> bool {
        false
    }

    /// Network tip height (best seen via gossip or peer status)
    pub fn network_tip_height(&self) -> u64 {
        self.network_tip_height
    }

    /// Network tip slot (best seen via gossip or peer status)
    pub fn network_tip_slot(&self) -> u32 {
        self.network_tip_slot
    }

    /// Get consecutive sync failure count (for health diagnostics)
    pub fn consecutive_sync_failure_count(&self) -> u32 {
        self.consecutive_sync_failures
    }

    /// Get consecutive empty header response count (for shallow fork detection)
    pub fn consecutive_empty_headers(&self) -> u32 {
        self.consecutive_empty_headers
    }

    /// Check if post-recovery grace period is active.
    /// During grace, fork_sync should not be activated — the node needs time
    /// to sync via header-first / gossip before fork detection is meaningful.
    pub fn post_recovery_grace_active(&self) -> bool {
        self.post_recovery_grace
    }

    /// Activate post-recovery grace period. Called after snap sync / forced recovery.
    pub fn set_post_recovery_grace(&mut self) {
        self.post_recovery_grace = true;
        self.blocks_applied_since_recovery = 0;
        self.consecutive_empty_headers = 0;
        self.consecutive_small_gap_empty_headers = 0;
        self.consecutive_apply_failures = 0;
        info!("Post-recovery grace activated: fork_sync suppressed until 10 blocks applied.");
    }

    /// Check if sync manager has signaled that a full genesis resync is needed
    pub fn needs_genesis_resync(&self) -> bool {
        self.needs_genesis_resync
    }

    /// Reset sync state after a shallow fork rollback.
    /// Clears the fork signal counters, resets downloaders, and returns to Idle
    /// so sync can restart from the new (rolled-back) tip.
    pub fn reset_sync_for_rollback(&mut self) {
        self.consecutive_empty_headers = 0;
        // needs_genesis_resync intentionally preserved — rollbacks must not
        // suppress the genesis resync signal set by the sync manager.
        self.consecutive_sync_failures = 0;
        self.fork_sync = None;
        self.state = SyncState::Idle;
        // NT10 fix: Signal that the next start_sync() should skip header-first sync.
        // After a fork rollback, our tip is still on the fork — header-first will
        // always get 0 headers because peers don't recognize our (rolled-back) tip.
        self.post_rollback = true;
        // Clear stale weight history to prevent minority fork weights from
        // contaminating future fork choice decisions after a resync.
        self.reorg_handler.clear();
        self.pending_headers.clear();
        self.pending_blocks.clear();
        self.headers_needing_bodies.clear();
        self.pending_requests.clear();
        self.header_downloader = HeaderDownloader::new(
            self.config.max_headers_per_request,
            self.config.request_timeout,
        );
        self.body_downloader = BodyDownloader::new(
            self.config.max_bodies_per_request,
            self.config.max_concurrent_body_requests,
            self.config.request_timeout,
        );
        self.last_block_seen = Instant::now();
        self.last_block_applied = Instant::now();
        self.last_sync_activity = Instant::now();
    }

    /// Human-readable sync state name for diagnostics
    pub fn sync_state_name(&self) -> &'static str {
        match &self.state {
            SyncState::Idle => "Idle",
            SyncState::DownloadingHeaders { .. } => "DownloadingHeaders",
            SyncState::DownloadingBodies { .. } => "DownloadingBodies",
            SyncState::Processing { .. } => "Processing",
            SyncState::Synchronized => "Synchronized",
            SyncState::SnapCollectingRoots { .. } => "SnapCollectingRoots",
            SyncState::SnapDownloading { .. } => "SnapDownloading",
            SyncState::SnapReady { .. } => "SnapReady",
        }
    }

    /// Note that we received a block via gossip network (P0 #3)
    pub fn note_block_received_via_gossip(&mut self) {
        self.last_block_seen = Instant::now();
        self.last_block_received_via_gossip = Some(Instant::now());
        // NOTE: We intentionally do NOT reset consecutive_empty_headers here.
        // Receiving gossip blocks proves the *network* is alive, but NOT that we're on
        // the canonical chain. If we're on a fork, we receive gossip blocks from the
        // canonical chain that we can't apply (orphans). Resetting the counter here
        // would prevent deep fork detection from ever triggering, leaving the node
        // permanently stuck on a dead fork.
    }

    /// Note that we received a block from a specific peer (P1 #5)
    pub fn note_block_received_from_peer(&mut self, peer_id: PeerId) {
        self.last_block_seen = Instant::now();
        if let Some(status) = self.peers.get_mut(&peer_id) {
            status.last_block_received = Some(Instant::now());
            // Implicitly, if they sent us a block, they are reachable
            status.last_status_response = Instant::now();
        }
    }

    /// Check if the chain is stale (no blocks received for `threshold` duration).
    /// Used by Node to detect stuck state and trigger re-sync.
    pub fn is_chain_stale(&self, threshold: Duration) -> bool {
        self.last_block_seen.elapsed() > threshold
    }

    /// Returns true if peers consistently reject our chain tip (deep fork).
    /// Requires ALL conditions:
    /// 1. Many consecutive empty header responses (peers don't recognize our chain)
    /// 2. We are significantly behind peers (not just a 1-block fork)
    /// 3. At least one peer is at a similar height (within 100 blocks) — proving
    ///    they SHOULD have our block range. Peers far ahead may be snap-synced
    ///    without old blocks; empty responses from them are history gaps, not forks.
    ///
    /// Short forks (1-2 blocks) are normal and resolve naturally via heaviest chain.
    /// Only trigger genesis resync for genuine deep forks where we're stuck.
    pub fn is_deep_fork_detected(&self) -> bool {
        if self.consecutive_empty_headers < 10 {
            return false;
        }
        // Must be significantly behind peers to qualify as deep fork
        let best_peer_height = self
            .peers
            .values()
            .map(|p| p.best_height)
            .max()
            .unwrap_or(0);
        if best_peer_height <= self.local_height + 5 {
            return false;
        }
        // If snap sync can handle this gap, don't escalate to deep fork.
        // next_request() will attempt snap sync first.
        let gap = best_peer_height.saturating_sub(self.local_height);
        let enough_peers = self.peers.len() >= 3;
        if enough_peers && gap > self.snap_sync_threshold {
            return false;
        }
        // Require at least one peer whose height is close to ours (within 100 blocks).
        // If ALL peers are far ahead, empty headers likely mean they snap-synced
        // and lack our block range — not that we're on a fork.
        let has_close_peer = self
            .peers
            .values()
            .any(|p| p.best_height <= self.local_height + 100);
        has_close_peer
    }

    // =========================================================================
    // END PRODUCTION GATE
    // =========================================================================

    // =========================================================================
    // FORK RECOVERY — Active parent chain download
    // =========================================================================

    /// Start fork recovery for an orphan block.
    /// Walks backward through parent chain requesting blocks from the peer.
    pub fn start_fork_recovery(&mut self, orphan: doli_core::Block, peer: PeerId) -> bool {
        self.fork_recovery.start(orphan, peer)
    }

    /// Check if fork recovery chain connected to our block_store.
    /// Node calls this with the result of `block_store.has_block(current_parent)`.
    pub fn check_fork_recovery_connection(
        &mut self,
        parent_known: bool,
    ) -> Option<super::fork_recovery::CompletedRecovery> {
        self.fork_recovery.check_connection(parent_known)
    }

    /// Get the parent hash the recovery is currently seeking.
    /// Returns None if no recovery active or waiting for response.
    pub fn fork_recovery_current_parent(&self) -> Option<Hash> {
        self.fork_recovery.current_parent()
    }

    /// Is fork recovery currently active?
    pub fn is_fork_recovery_active(&self) -> bool {
        self.fork_recovery.is_active()
    }

    /// Can a new fork recovery start? (not active and not on cooldown)
    pub fn can_start_fork_recovery(&self) -> bool {
        self.fork_recovery.can_start()
    }

    /// Check and consume the fork-recovery exceeded-max-depth flag.
    /// When true, the fork is deeper than MAX_RECOVERY_DEPTH and the node
    /// should escalate to force_resync_from_genesis().
    pub fn take_fork_exceeded_max_depth(&mut self) -> bool {
        self.fork_recovery.take_exceeded_max_depth()
    }

    /// Record a fork block's weight in reorg_handler WITHOUT updating local chain tip.
    /// Used during fork recovery to populate weights before plan_reorg.
    pub fn record_fork_block_weight(&mut self, hash: Hash, prev_hash: Hash, weight: u64) {
        self.reorg_handler
            .record_fork_block(hash, prev_hash, weight);
    }

    /// Seed the reorg handler with the snap sync tip so fork detection works immediately.
    /// Called once after snap sync completes — the snap tip becomes the root of recent_blocks.
    pub fn record_block_applied_after_snap(&mut self, hash: Hash, height: u64) {
        self.reorg_handler
            .record_block_with_weight(hash, Hash::ZERO, 1);
        self.local_height = height;
        self.local_hash = hash;
    }

    /// Get read-only access to the reorg handler (for plan_reorg from Node).
    pub fn reorg_handler(&self) -> &super::reorg::ReorgHandler {
        &self.reorg_handler
    }

    /// Pick the best peer for fork recovery (highest height).
    pub fn best_peer_for_recovery(&self) -> Option<PeerId> {
        self.peers
            .iter()
            .max_by_key(|(_, status)| status.best_height)
            .map(|(peer, _)| *peer)
    }

    // =========================================================================
    // END FORK RECOVERY
    // =========================================================================

    // =========================================================================
    // FORK SYNC (binary search for common ancestor)
    // =========================================================================

    /// Set the lowest block height available in the local block store.
    /// Called by the node after startup or snap sync to inform fork sync
    /// where the block store coverage begins.
    pub fn disable_snap_sync(&mut self) {
        self.snap_sync_threshold = u64::MAX;
    }

    pub fn set_store_floor(&mut self, floor: u64) {
        self.store_floor = floor;
    }

    /// Get the lowest block height available in the local block store.
    pub fn store_floor(&self) -> u64 {
        self.store_floor
    }

    /// Start fork sync: binary search for common ancestor with best peer.
    /// Returns true if fork sync was started, false if conditions aren't met.
    pub fn start_fork_sync(&mut self) -> bool {
        if self.fork_sync.is_some() {
            return false; // Already active
        }
        let peer = match self.best_peer_for_recovery() {
            Some(p) => p,
            None => {
                warn!("Fork sync: no peer available");
                return false;
            }
        };
        if self.local_height == 0 {
            return false; // Nothing to search at genesis
        }
        self.fork_sync = Some(super::fork_sync::ForkSync::new(
            peer,
            self.local_height,
            self.store_floor,
        ));
        // Pause normal sync so requests don't conflict
        self.state = SyncState::Idle;
        true
    }

    /// Returns the pending probe for Node to compare with block_store.
    /// (height, peer_hash) — Node checks if block_store has the same hash at that height.
    pub fn fork_sync_pending_probe(&self) -> Option<(u64, Hash)> {
        self.fork_sync.as_ref()?.pending_probe()
    }

    /// Feed the binary search comparison result from Node.
    pub fn fork_sync_handle_probe(&mut self, result: super::fork_sync::ProbeResult) {
        if let Some(ref mut fs) = self.fork_sync {
            fs.handle_probe_result(result);
        }
    }

    /// Check if fork sync search is complete and needs the ancestor hash.
    /// Returns Some(ancestor_height) when Node should look up the hash from block_store.
    pub fn fork_sync_ancestor_height(&self) -> Option<u64> {
        self.fork_sync.as_ref()?.search_complete_ancestor_height()
    }

    /// Check if fork sync binary search bottomed out at the floor without finding
    /// a common ancestor. Signals the node to do a full resync.
    pub fn fork_sync_bottomed_out(&self) -> bool {
        self.fork_sync
            .as_ref()
            .map(|fs| fs.search_bottomed_out())
            .unwrap_or(false)
    }

    /// Check if fork sync search stopped because the block store doesn't
    /// cover the search range (snap sync gap). NOT a deep fork signal.
    pub fn fork_sync_store_limited(&self) -> bool {
        self.fork_sync
            .as_ref()
            .map(|fs| fs.search_store_limited())
            .unwrap_or(false)
    }

    /// Clear fork sync state without resetting empty headers counter.
    pub fn fork_sync_clear(&mut self) {
        self.fork_sync = None;
        self.state = SyncState::Idle;
    }

    /// Set the ancestor hash to complete the search→download transition.
    pub fn fork_sync_set_ancestor(&mut self, ancestor_height: u64, ancestor_hash: Hash) {
        if let Some(ref mut fs) = self.fork_sync {
            fs.set_ancestor(ancestor_height, ancestor_hash);
        }
    }

    /// Take the completed fork sync result. Returns None if not yet complete.
    pub fn fork_sync_take_result(&mut self) -> Option<super::fork_sync::ForkSyncResult> {
        let result = self.fork_sync.as_mut()?.take_result();
        if result.is_some() {
            self.fork_sync = None; // Clear after taking result
        }
        result
    }

    /// Is fork sync active?
    pub fn is_fork_sync_active(&self) -> bool {
        self.fork_sync.is_some()
    }

    /// Cancel fork sync (timeout, state change, etc.)
    #[allow(dead_code)]
    fn cancel_fork_sync(&mut self, reason: &str) {
        if self.fork_sync.is_some() {
            warn!("Fork sync cancelled: {}", reason);
            self.fork_sync = None;
        }
    }

    // =========================================================================
    // END FORK SYNC
    // =========================================================================

    /// Check if we should start syncing
    fn should_sync(&self) -> bool {
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
    fn best_peer(&self) -> Option<PeerId> {
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
    fn start_sync(&mut self) {
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

            // Snap sync decision: use snap sync when far behind and enough peers
            let gap = best_height.saturating_sub(self.local_height);
            let enough_peers = self.peers.len() >= 3;
            let should_snap = enough_peers
                && self.snap_sync_attempts < 3
                && (self.local_height == 0 || gap > self.snap_sync_threshold);

            // Fresh node optimization: don't start slow header-first sync.
            // Wait for 5 peers so snap sync can activate — it downloads state
            // in seconds instead of replaying 60K+ blocks over hours.
            // BUT: timeout after 60s to avoid deadlock when <5 peers are discoverable.
            if self.local_height == 0
                && !enough_peers
                && self.snap_sync_attempts < 3
                && gap > self.snap_sync_threshold
            {
                let wait_start = self.fresh_node_wait_start.get_or_insert(Instant::now());
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
                self.fresh_node_wait_start = None;
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
                            self.sync_epoch, peer, target_slot
                        );
                        self.header_downloader.clear();
                        self.state = SyncState::DownloadingHeaders {
                            target_slot,
                            peer,
                            headers_count: 0,
                        };
                        return;
                    }
                };

                info!(
                    "[SNAP_SYNC] Starting snap sync epoch {} — gap={}, target_height={}, target_hash={:.16} (majority-preferred)",
                    self.sync_epoch, gap, best_height, target_hash
                );

                self.state = SyncState::SnapCollectingRoots {
                    target_hash,
                    target_height: best_height,
                    votes: Vec::new(),
                    asked: HashSet::new(),
                    started_at: Instant::now(),
                };
            } else if self.post_rollback {
                // NT10 fix: After a fork rollback, our tip is still on the fork
                // (just 1 block shorter). Header-first sync will always get 0
                // headers because peers don't recognize our rolled-back fork tip.
                // Instead, try fork_sync (binary search for common ancestor).
                // If fork_sync can't start (e.g., no recovery peer), escalate to
                // snap sync via needs_genesis_resync.
                self.post_rollback = false;
                if self.start_fork_sync() {
                    info!(
                        "Post-rollback: started fork_sync (binary search) instead of header-first",
                    );
                } else if enough_peers && self.snap_sync_attempts < 3 {
                    warn!(
                        "Post-rollback: fork_sync failed to start, escalating to snap sync \
                         (gap={}, peers={})",
                        gap,
                        self.peers.len()
                    );
                    self.needs_genesis_resync = true;
                } else {
                    // Not enough peers for snap sync — fall through to header-first
                    // as a last resort (it may fail, but cleanup() will retry).
                    warn!(
                        "Post-rollback: fork_sync failed, not enough peers for snap sync — \
                         falling back to header-first (gap={}, peers={})",
                        gap,
                        self.peers.len()
                    );
                    self.state = SyncState::DownloadingHeaders {
                        target_slot,
                        peer,
                        headers_count: 0,
                    };
                }
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
                warn!("Fork sync timed out — cancelling");
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
                    let enough_peers = self.peers.len() >= 3;
                    if enough_peers && gap > self.snap_sync_threshold {
                        info!(
                            "[SNAP_SYNC] Deep fork with {} consecutive empty headers — \
                             attempting snap sync before genesis resync (gap={})",
                            self.consecutive_empty_headers, gap
                        );
                        self.snap_sync_attempts = 0;
                        self.consecutive_empty_headers = 0;
                        self.state = SyncState::Idle;
                        self.start_sync();
                        return None;
                    }
                    // Not enough peers or gap too small — fall back to genesis resync
                    info!(
                        "Genesis fallback: {} consecutive empty headers — signaling node for full resync",
                        self.consecutive_empty_headers
                    );
                    self.needs_genesis_resync = true;
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
                !already_asked.contains(pid) && !self.snap_blacklisted_peers.contains(pid)
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
            SyncResponse::Block(maybe_block) => {
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
            if !self.headers_needing_bodies.is_empty() {
                let total = self.headers_needing_bodies.len();
                self.state = SyncState::DownloadingBodies { pending: 0, total };
                info!("Starting body download for {} blocks", total);
            } else if self.pending_headers.is_empty() {
                let peer_height = self.peers.get(&peer).map(|p| p.best_height).unwrap_or(0);
                let gap = peer_height.saturating_sub(self.local_height);

                if gap <= 50 {
                    // Peer is near our tip. Empty headers is NORMAL — the peer
                    // already applied those blocks and can't serve headers for them
                    // (especially if its block store has gaps from snap sync recovery).
                    self.consecutive_small_gap_empty_headers += 1;
                    if self.consecutive_small_gap_empty_headers >= 5 {
                        // 5+ consecutive empty headers with small gap = gossip isn't
                        // delivering. We're likely on a fork. Escalate to fork_sync.
                        warn!(
                            "Empty headers from {} (gap={}, consecutive={}) — \
                             gossip not delivering, activating fork_sync.",
                            peer, gap, self.consecutive_small_gap_empty_headers
                        );
                        self.consecutive_small_gap_empty_headers = 0;
                        self.consecutive_empty_headers = self.consecutive_empty_headers.max(3);
                    } else {
                        debug!(
                            "Empty headers from {} (gap={}, consecutive={}) — waiting for gossip.",
                            peer, gap, self.consecutive_small_gap_empty_headers
                        );
                    }
                    self.state = SyncState::Idle;
                    return;
                }

                // Large gap + empty headers = peer doesn't recognize our tip.
                // This is genuine fork evidence.
                self.consecutive_empty_headers += 1;
                self.header_blacklisted_peers.insert(peer, Instant::now());
                warn!(
                    "Empty headers from {} (peer_h={}, local_h={}, gap={}) — \
                     fork evidence (consecutive={}). Blacklisted peer, resetting to Idle.",
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
            // Successfully received valid headers — reset fork counter
            self.consecutive_empty_headers = 0;
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
            warn!(
                "No valid headers from peer {} - header chain broken (consecutive={}). \
                 Peer has different chain at our tip.",
                peer, self.consecutive_empty_headers
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
                    "Sync chain mismatch: first pending header (slot {}, prev={}) doesn't \
                     build on local tip (hash={}). Clearing {} useless synced blocks.",
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
        self.last_block_applied = Instant::now();
        self.last_sync_activity = Instant::now();
        self.body_stall_retries = 0;
        self.consecutive_apply_failures = 0;

        // Applying a block means the chain is advancing — reset fork counters.
        self.consecutive_empty_headers = 0;
        self.consecutive_small_gap_empty_headers = 0;

        // Post-recovery grace: clear after 10 blocks applied since recovery.
        if self.post_recovery_grace {
            self.blocks_applied_since_recovery += 1;
            if self.blocks_applied_since_recovery >= 10 {
                info!(
                    "Post-recovery grace cleared: {} blocks applied since recovery.",
                    self.blocks_applied_since_recovery
                );
                self.post_recovery_grace = false;
                self.blocks_applied_since_recovery = 0;
            }
        }

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

        // CHECK FOR SYNC COMPLETION: Transition from Processing/DownloadingBodies to Synchronized
        // This is the CRITICAL fix for the "late joiner not producing" bug.
        // Without this check, the sync state would get stuck in Processing forever,
        // blocking production even after all blocks are synced via gossip.
        //
        // NOTE: We use network_tip_height instead of best_peer() because:
        // - best_peer() filters for peers AHEAD of us (peer_height > local_height)
        // - When we catch up, there are no peers "ahead", so best_peer() returns None
        // - network_tip_height tracks the highest height seen from any peer
        let slot_ok = self.network_tip_slot.saturating_sub(slot) <= self.max_slots_behind;
        if self.state.is_syncing() && height >= self.network_tip_height && slot_ok {
            info!(
                "Sync complete: transitioning to Synchronized at height {} slot {} (network_tip_h={} network_tip_s={})",
                height, slot, self.network_tip_height, self.network_tip_slot
            );
            self.state = SyncState::Synchronized;
            self.snap_sync_attempts = 0;
            self.header_blacklisted_peers.clear();

            // If we were in a resync, complete it now
            if self.resync_in_progress {
                self.complete_resync();
                info!(
                    "Resync complete at height {} - grace period started ({}s)",
                    height, self.resync_grace_period_secs
                );
            }
        } else if matches!(self.state, SyncState::Processing { .. })
            && self.pending_requests.is_empty()
        {
            // No active network requests in Processing state. Either:
            // - All blocks applied but network moved ahead, OR
            // - Blocks downloaded but unchainable (hash mismatch)
            // Either way, nothing will change — clear stale data and restart.
            info!(
                "Processing complete (headers={}, blocks={}, local h={} s={}, tip h={} s={}) — starting new sync round",
                self.pending_headers.len(), self.pending_blocks.len(),
                height, slot, self.network_tip_height, self.network_tip_slot
            );
            self.pending_headers.clear();
            self.pending_blocks.clear();
            self.state = SyncState::Idle;
            if self.should_sync() {
                self.start_sync();
            }
        }
    }

    /// Notify sync manager that apply_block() failed for a synced block.
    ///
    /// Clears all pending sync state (the downloaded chain is suspect) and
    /// increments the apply failure counter. At 3+ failures, triggers genesis
    /// resync — we're stuck on a fork producing valid-looking but unapplyable chains.
    pub fn block_apply_failed(&mut self) {
        self.consecutive_apply_failures += 1;
        warn!(
            "Block apply failure #{} — clearing pending sync state",
            self.consecutive_apply_failures
        );

        // Clear everything — don't salvage blocks from a bad chain
        self.pending_headers.clear();
        self.pending_blocks.clear();
        self.headers_needing_bodies.clear();
        self.header_downloader.clear();
        self.body_downloader.clear();
        self.body_stall_retries = 0;
        self.state = SyncState::Idle;
        self.last_sync_activity = Instant::now();

        if self.consecutive_apply_failures >= 3 {
            let gap = self.network_tip_height.saturating_sub(self.local_height);
            if gap <= 50 {
                // Small gap + repeated apply failures = fork, not transient issue.
                // Gossip can't resolve this — the node is on a different chain and
                // keeps trying to apply canonical blocks at the wrong height.
                // Activate fork_sync (binary search for common ancestor) which will
                // find the divergence point, rollback, and re-sync correctly.
                warn!(
                    "3+ consecutive apply failures with gap={} — \
                     activating fork_sync (binary search for common ancestor)",
                    gap
                );
                self.consecutive_apply_failures = 0;
                // Signal fork_sync activation. The node's resolve_shallow_fork()
                // checks consecutive_empty_headers >= 3, so set it to trigger.
                self.consecutive_empty_headers = self.consecutive_empty_headers.max(3);
                self.state = SyncState::Idle;
            } else {
                warn!(
                    "3+ consecutive apply failures with gap={} — triggering genesis resync",
                    gap
                );
                self.needs_genesis_resync = true;
            }
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

        // Reset gossip tracking for fresh start
        self.last_block_received_via_gossip = Some(Instant::now());

        // Reset network tip to 0 — stale tip from pre-fork state distorts
        // the "behind peers" check (Layer 6) after resync, causing the node
        // at height 0 to think it's hundreds of blocks behind a ghost tip.
        self.network_tip_height = 0;
        self.network_tip_slot = 0;

        // Clear all pending sync state to start fresh
        self.pending_headers.clear();
        self.pending_blocks.clear();
        self.headers_needing_bodies.clear();
        self.pending_requests.clear();

        // Cancel any active fork recovery
        self.fork_recovery = super::fork_recovery::ForkRecoveryTracker::new();

        // Reset deep fork detection
        self.consecutive_empty_headers = 0;
        self.needs_genesis_resync = false;
        self.body_stall_retries = 0;
        self.consecutive_apply_failures = 0;

        // Clear snap sync production gate (genesis resync starts fresh)
        self.awaiting_canonical_block = false;

        // Clear fork mismatch flag (resync will re-establish correct chain)
        self.fork_mismatch_detected = false;

        // Reset snap sync attempt counter so recovery gets fresh tries.
        self.snap_sync_attempts = 0;

        // Reset stale chain timers
        self.last_block_seen = Instant::now();
        self.last_block_applied = Instant::now();
        self.last_sync_activity = Instant::now();

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

    // =========================================================================
    // SNAP SYNC — state root voting, snapshot handling, fallback
    // =========================================================================

    /// Handle a StateRoot response: add vote and check quorum.
    /// Votes are grouped by (height, root) — peers must agree on both to form
    /// a quorum. Votes from peers too far below the target height are rejected
    /// to prevent freshly-wiped peers (at height=0) from contaminating quorum
    /// with empty-state votes and triggering cascading snap sync failures.
    fn handle_snap_state_root(
        &mut self,
        peer: PeerId,
        block_hash: Hash,
        block_height: u64,
        state_root: Hash,
    ) {
        // Extract target_height before taking a mutable borrow below.
        let target_height = match &self.state {
            SyncState::SnapCollectingRoots { target_height, .. } => *target_height,
            _ => return,
        };

        // Reject votes from peers whose reported height is far below the
        // target. Tight window (100 blocks ≈ 16 min) prevents stale peers
        // from forming quorum during rolling deployments.
        let min_acceptable = target_height.saturating_sub(100);
        if block_height < min_acceptable {
            warn!(
                "[SNAP_SYNC] Rejecting stale state root vote from {} at height={} \
                 (target={}, min_acceptable={})",
                peer, block_height, target_height, min_acceptable
            );
            return;
        }

        // NOTE: We do NOT filter by block_hash here because peers advance
        // every ~10s. Fix 1 (consensus_target_hash) ensures we target the
        // majority chain, and Fix 2 (quorum from total peers) prevents a
        // minority partition from reaching quorum. Together these two
        // mechanisms make block_hash filtering unnecessary and avoid
        // rejecting valid votes from peers that simply advanced.

        if let SyncState::SnapCollectingRoots { votes, .. } = &mut self.state {
            votes.push((peer, block_hash, block_height, state_root));
        } else {
            return;
        }

        // Quorum: majority of ALL connected peers, not just votes received.
        //
        // FIX: Using vote_count as denominator allowed partition quorum —
        // 3 forked nodes on the same server could form quorum (3/3) while
        // 9 correct peers hadn't responded yet. Using total connected peers
        // as denominator ensures quorum represents a true network majority.
        //
        // Example: 12 connected peers → quorum = max(3, 12/2+1) = 7.
        // A partition of 3 nodes can never reach 7, even if they all agree.
        let total_peers = self.peers.len();
        let quorum = std::cmp::max(self.snap_sync_quorum, total_peers / 2 + 1);

        let votes_snapshot: Vec<(PeerId, Hash, u64, Hash)> =
            if let SyncState::SnapCollectingRoots { votes, .. } = &self.state {
                votes.clone()
            } else {
                return;
            };

        // Group votes by state_root only — peers at different heights with the
        // same root no longer split quorum (fixes ±1 height divergence).
        let mut groups: HashMap<Hash, Vec<(PeerId, Hash, u64)>> = HashMap::new();
        for (pid, bhash, bheight, sroot) in &votes_snapshot {
            groups
                .entry(*sroot)
                .or_default()
                .push((*pid, *bhash, *bheight));
        }

        if let Some((quorum_root, peers_with_info)) =
            groups.iter().find(|(_, peers)| peers.len() >= quorum)
        {
            // Pick peer with highest height for download
            let (download_peer, download_hash, best_height) = peers_with_info
                .iter()
                .max_by_key(|(_, _, h)| *h)
                .copied()
                .unwrap();
            // Build alternate peer list (excluding primary) for retry on error
            let alternate_peers: Vec<(PeerId, Hash, u64)> = peers_with_info
                .iter()
                .filter(|(pid, _, _)| *pid != download_peer)
                .copied()
                .collect();
            info!(
                "[SNAP_SYNC] Quorum reached: {} peers agree on root={:.16}, best_height={}, downloading from {} ({} alternates)",
                peers_with_info.len(), quorum_root, best_height, download_peer, alternate_peers.len()
            );
            let quorum_root = *quorum_root;
            self.state = SyncState::SnapDownloading {
                target_hash: download_hash,
                target_height: best_height,
                quorum_root,
                peer: download_peer,
                alternate_peers,
                started_at: Instant::now(),
            };
        }
    }

    /// Handle a StateSnapshot response: store as SnapReady for node consumption.
    ///
    /// The snapshot includes the peer's actual state_root. We use that as the
    /// expected root (the node verifies by recomputing from the raw bytes).
    /// If the peer advanced since the quorum vote, its root will differ from
    /// quorum_root — that's fine, the node's compute_state_root_from_bytes()
    /// is the ultimate check. The quorum vote ensured we're downloading from
    /// a peer that was recently on the canonical chain.
    #[allow(clippy::too_many_arguments)]
    fn handle_snap_snapshot(
        &mut self,
        peer: PeerId,
        block_hash: Hash,
        block_height: u64,
        chain_state: Vec<u8>,
        utxo_set: Vec<u8>,
        producer_set: Vec<u8>,
        response_root: Hash,
    ) {
        if let SyncState::SnapDownloading { quorum_root, .. } = &self.state {
            if response_root != *quorum_root {
                info!(
                    "[SNAP_SYNC] Peer {} advanced since vote: response_root={:.16} != quorum_root={:.16} (height={}). Accepting — node verifies independently.",
                    peer, response_root, quorum_root, block_height
                );
            }
            info!(
                "[SNAP_SYNC] Snapshot received from {} — height={}, storing as SnapReady (node will verify root)",
                peer, block_height
            );
            self.state = SyncState::SnapReady {
                snapshot: VerifiedSnapshot {
                    block_hash,
                    block_height,
                    chain_state,
                    utxo_set,
                    producer_set,
                    state_root: response_root,
                },
            };
        } else {
            warn!(
                "[SNAP_SYNC] Unexpected snapshot from {} — not in SnapDownloading state, ignoring",
                peer
            );
        }
    }

    /// Handle an error response while in SnapDownloading state.
    ///
    /// The most common cause: the peer advanced to a new block between the vote
    /// and the download request, so `GetStateSnapshot(old_hash)` is rejected with
    /// "Snapshot only available for current tip". We blacklist that peer and try
    /// the next alternate peer from the quorum group. If no alternates remain,
    /// fall back to normal sync (which may retry snap sync from scratch).
    fn handle_snap_download_error(&mut self, peer: PeerId) {
        if !matches!(self.state, SyncState::SnapDownloading { .. }) {
            return;
        }
        self.snap_blacklisted_peers.insert(peer);
        // Take the state so we can decompose it without borrow issues
        let old = std::mem::replace(&mut self.state, SyncState::Idle);
        if let SyncState::SnapDownloading {
            quorum_root,
            mut alternate_peers,
            ..
        } = old
        {
            if let Some((next_peer, next_hash, next_height)) = alternate_peers.pop() {
                warn!(
                    "[SNAP_SYNC] Peer {} failed, retrying with alternate peer {} at height={} ({} remaining)",
                    peer, next_peer, next_height, alternate_peers.len()
                );
                self.state = SyncState::SnapDownloading {
                    target_hash: next_hash,
                    target_height: next_height,
                    quorum_root,
                    peer: next_peer,
                    alternate_peers,
                    started_at: Instant::now(),
                };
            } else {
                warn!(
                    "[SNAP_SYNC] No alternate peers left after {} failed — restarting snap sync",
                    peer
                );
                self.state = SyncState::Idle; // restore before fallback
                self.snap_fallback_to_normal();
            }
        }
    }

    /// Take the ready snapshot for node application. Transitions to Synchronized.
    pub fn take_snap_snapshot(&mut self) -> Option<VerifiedSnapshot> {
        if matches!(self.state, SyncState::SnapReady { .. }) {
            let old = std::mem::replace(&mut self.state, SyncState::Synchronized);
            self.header_blacklisted_peers.clear();
            // Snap sync bypasses the normal sync pipeline, so complete_resync()
            // is never reached via handle_applied_block(). Clear it here so
            // production resumes after the grace period.
            if self.resync_in_progress {
                self.complete_resync();
            }
            // Block production until a canonical gossip block arrives.
            // Snap sync restores state but leaves the block store empty —
            // producing now would create a fork with no real parent.
            self.awaiting_canonical_block = true;
            info!("[SNAP_SYNC] Production gated: awaiting first canonical gossip block");
            if let SyncState::SnapReady { snapshot } = old {
                Some(snapshot)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Fall back from snap sync to normal header-first sync.
    /// Increments the snap attempt counter; after 3 failures, snap sync is skipped.
    pub fn snap_fallback_to_normal(&mut self) {
        self.snap_sync_attempts += 1;
        warn!(
            "[SNAP_SYNC] Attempt {}/3 failed, {}",
            self.snap_sync_attempts,
            if self.snap_sync_attempts >= 3 {
                "falling back to header-first sync"
            } else {
                "will retry with different peer"
            }
        );
        self.state = SyncState::Idle;
        if self.should_sync() {
            self.start_sync();
        }
    }

    /// Blacklist a peer for snap sync (bad snapshot).
    pub fn snap_blacklist_peer(&mut self, peer: PeerId) {
        warn!("[SNAP_SYNC] Blacklisting peer {} for bad snapshot", peer);
        self.snap_blacklisted_peers.insert(peer);
    }

    /// Clean up stale requests and peers
    pub fn cleanup(&mut self) {
        let now = Instant::now();

        // Log current state for debugging
        let pending_count = self
            .peers
            .values()
            .filter(|s| s.pending_request.is_some())
            .count();
        info!(
            "[SYNC_DEBUG] Cleanup: state={:?}, peers={}, pending_peer_requests={}, pending_requests={}",
            self.state,
            self.peers.len(),
            pending_count,
            self.pending_requests.len()
        );

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
                warn!(
                    "[SYNC_DEBUG] Request timeout: id={}, peer={}, elapsed={:?}, request={:?}",
                    id.0,
                    req.peer,
                    now.duration_since(req.sent_at),
                    req.request
                );
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
                now.duration_since(status.last_status_response) > self.config.stale_timeout
            })
            .map(|(peer, _)| *peer)
            .collect();

        for peer in stale {
            warn!("Removing stale peer {}", peer);
            self.remove_peer(&peer);
        }

        // Snap sync timeouts
        match &self.state {
            SyncState::SnapCollectingRoots { started_at, .. } => {
                if started_at.elapsed() > self.snap_root_timeout {
                    warn!(
                        "[SNAP_SYNC] State root collection timed out after {:?} — falling back",
                        self.snap_root_timeout
                    );
                    self.snap_fallback_to_normal();
                }
            }
            SyncState::SnapDownloading {
                started_at, peer, ..
            } => {
                if started_at.elapsed() > self.snap_download_timeout {
                    let peer = *peer;
                    warn!(
                        "[SNAP_SYNC] Snapshot download from {} timed out after {:?} — trying alternate peer",
                        peer, self.snap_download_timeout
                    );
                    self.handle_snap_download_error(peer);
                }
            }
            _ => {}
        }

        // Stall recovery: if "Synchronized" but significantly behind in slots,
        // we're in a deadlock (height matches but slot lags). Reset to Idle
        // to allow re-evaluation and potential resync.
        if matches!(self.state, SyncState::Synchronized) && !self.peers.is_empty() {
            let best_slot = self.best_peer_slot();
            let slot_lag = best_slot.saturating_sub(self.local_slot);
            let stall_threshold = self.max_slots_behind.saturating_mul(5);
            if slot_lag > stall_threshold {
                warn!(
                    "Stall detected: Synchronized but {} slots behind peers (threshold {}). Resetting to Idle for recovery.",
                    slot_lag, stall_threshold
                );
                self.state = SyncState::Idle;
                if self.should_sync() {
                    self.start_sync();
                }
            }
        }

        // Escape stuck sync states: if we're syncing and no sync progress for 30s,
        // either soft-retry the missing bodies or hard-reset.
        //
        // We use `last_sync_activity` which tracks ALL phases of header-first sync:
        // headers received, bodies received, and blocks applied. Previously this used
        // `last_block_applied` which only tracked the final phase, causing false "stuck"
        // detection during the header download phase (no blocks applied yet, but headers
        // were streaming in). This nuked 20-30K downloaded headers every 30s.
        // Bodies are much larger than headers and N4 may be serving 4 peers
        // simultaneously — give body download more time before declaring stuck.
        let stuck_threshold = if matches!(self.state, SyncState::DownloadingBodies { .. }) {
            Duration::from_secs(120)
        } else {
            Duration::from_secs(30)
        };
        if self.state.is_syncing() && self.last_sync_activity.elapsed() > stuck_threshold {
            let was_processing = matches!(self.state, SyncState::Processing { .. });
            let is_downloading_bodies = matches!(self.state, SyncState::DownloadingBodies { .. });
            let have_pending_headers = !self.pending_headers.is_empty();
            let have_downloaded_bodies = !self.pending_blocks.is_empty();

            // Soft recovery for the body-downloader stall:
            // Bodies are arriving but not contiguous from our tip (parallel
            // peer responses arrived out of order). Keep pending_headers and
            // pending_blocks, rebuild the needed-bodies list to contain only
            // what's actually missing, and let the body downloader retry.
            if is_downloading_bodies
                && have_pending_headers
                && have_downloaded_bodies
                && !was_processing
                && self.body_stall_retries < 3
            {
                self.body_stall_retries += 1;

                // Rebuild headers_needing_bodies: walk pending_headers from the
                // local tip, skip bodies we already have, enqueue the rest.
                let mut rebuilt = VecDeque::new();
                let mut current = self.local_hash;
                let mut gap_found = false;

                for header in &self.pending_headers {
                    if header.prev_hash != current {
                        gap_found = true;
                    }

                    let h = header.hash();

                    if !gap_found {
                        if self.pending_blocks.contains_key(&h) {
                            current = h;
                            continue;
                        } else {
                            gap_found = true;
                        }
                    }

                    if !self.pending_blocks.contains_key(&h) {
                        rebuilt.push_back(h);
                    }
                    current = h;
                }

                self.headers_needing_bodies = rebuilt;

                // Unwedge request bookkeeping so the missing bodies can be
                // re-requested immediately (hashes may be stuck in in_flight).
                for status in self.peers.values_mut() {
                    status.pending_request = None;
                }
                self.pending_requests.clear();
                self.body_downloader.clear();

                // Reset timer and stay in DownloadingBodies.
                self.last_block_applied = Instant::now();
                self.last_sync_activity = Instant::now();
                let total = self.pending_headers.len();
                let pending = self.headers_needing_bodies.len();
                self.state = SyncState::DownloadingBodies { pending, total };

                warn!(
                    "Body stall retry {}/3: retained {} headers + {} downloaded bodies; \
                     re-requesting {} missing bodies from h={}",
                    self.body_stall_retries,
                    total,
                    self.pending_blocks.len(),
                    pending,
                    self.local_height + 1
                );
            } else if is_downloading_bodies && have_pending_headers {
                // Body download exhausted all soft retries. Reset to Idle so
                // start_sync() can re-evaluate: if the gap is still >1000 blocks
                // with 3+ peers, snap sync will activate instead of repeating
                // the slow header-first path that already failed.
                warn!(
                    "Body download stuck after {}/3 retries — resetting to Idle \
                     (gap={}, local_h={}, snap sync will re-evaluate)",
                    self.body_stall_retries,
                    self.network_tip_height.saturating_sub(self.local_height),
                    self.local_height
                );

                // Full reset — let start_sync() decide header-first vs snap sync
                self.pending_headers.clear();
                self.pending_blocks.clear();
                self.headers_needing_bodies.clear();
                self.pending_requests.clear();
                for status in self.peers.values_mut() {
                    status.pending_request = None;
                }
                self.body_downloader.clear();
                self.body_stall_retries = 0;
                self.header_downloader.clear();

                self.state = SyncState::Idle;
                self.last_sync_activity = Instant::now();
                if self.should_sync() {
                    self.start_sync();
                }
            } else {
                // Hard reset — only for header download failures or processing stuck.
                warn!(
                    "Sync stuck in {:?} for >30s with no progress \
                     (last_activity={:.0?} ago, local_h={}, network_tip={}) — resetting to Idle",
                    self.state,
                    self.last_sync_activity.elapsed(),
                    self.local_height,
                    self.network_tip_height
                );
                self.state = SyncState::Idle;
                self.pending_headers.clear();
                self.pending_blocks.clear();
                self.headers_needing_bodies.clear();
                self.pending_requests.clear();
                for status in self.peers.values_mut() {
                    status.pending_request = None;
                }
                self.body_downloader.clear();
                self.body_stall_retries = 0;
                // Reset header downloader so next sync starts from local_hash,
                // not from a stale expected_prev_hash from a previous (failed) cycle.
                self.header_downloader.clear();
                // If stuck in Processing, count toward deep fork detection.
                // The node downloaded a chain it can't apply — this is fork evidence.
                if was_processing {
                    self.consecutive_empty_headers += 1;
                    info!(
                        "Stuck Processing counted as fork signal (consecutive_empty_headers={})",
                        self.consecutive_empty_headers
                    );
                }
            }
        }

        // Expire stale header blacklist entries (30s cooldown — reduced from 60s
        // so peers become available between stuck-sync timeout cycles)
        self.header_blacklisted_peers
            .retain(|_, added| added.elapsed() < Duration::from_secs(30));

        // NT8 fix: When ALL peers are blacklisted and we've been stuck for >120s
        // total, clear the blacklist entirely to retry with a fresh slate.
        // Without this, the node cycles Idle↔DownloadingHeaders forever because
        // blacklisted peers never expire before the next stuck-sync timeout.
        if !self.header_blacklisted_peers.is_empty()
            && self.best_peer().is_none()
            && self.should_sync()
            && matches!(self.state, SyncState::Idle)
        {
            let stuck_duration = self.last_block_applied.elapsed();
            if stuck_duration > Duration::from_secs(120) {
                if self.consecutive_empty_headers >= 20 {
                    // Genuinely on a dead fork: many consecutive empties + stuck for 2+ min.
                    // Escalate to snap sync if enough peers, otherwise clear and retry.
                    let enough_peers = self.peers.len() >= 3;
                    if enough_peers {
                        warn!(
                            "All peers blacklisted for >120s with {} consecutive empty headers — \
                             escalating to snap sync",
                            self.consecutive_empty_headers
                        );
                        self.header_blacklisted_peers.clear();
                        self.needs_genesis_resync = true;
                    } else {
                        warn!(
                            "All peers blacklisted for >120s with {} consecutive empty headers \
                             but only {} peers (need 3 for snap sync) — clearing blacklist to retry",
                            self.consecutive_empty_headers, self.peers.len()
                        );
                        self.header_blacklisted_peers.clear();
                    }
                } else {
                    // Temporary gossip hiccup — clear blacklist and retry normally.
                    warn!(
                        "All peers blacklisted for >120s (consecutive_empty_headers={}) — \
                         clearing blacklist for fresh retry",
                        self.consecutive_empty_headers
                    );
                    self.header_blacklisted_peers.clear();
                }
            }
        }

        // Periodic sync retry: if Idle and behind peers, restart sync.
        // This catches cases where sync was attempted, failed (e.g., empty headers
        // from gossip race), reset to Idle, and never retried.
        // IMPORTANT: Skip retry when fork_sync is active — its binary search runs
        // across multiple ticks and resetting to Idle interrupts it, causing infinite loops.
        if matches!(self.state, SyncState::Idle)
            && self.should_sync()
            && !self.is_fork_sync_active()
        {
            let gap = self.network_tip_height.saturating_sub(self.local_height);
            if gap <= 5 && gap > 0 {
                // Small gap: increment a retry counter. If we've been stuck in
                // Idle-but-behind for multiple ticks (5+), escalate to a full
                // start_sync which transitions to header download and forces
                // the pipeline to request blocks actively.
                self.idle_behind_retries = self.idle_behind_retries.saturating_add(1);
                if self.idle_behind_retries >= 5 {
                    info!(
                        "Sync retry: small gap ({} blocks) but stuck for {} ticks — forcing full sync restart.",
                        gap, self.idle_behind_retries
                    );
                    self.idle_behind_retries = 0;
                    self.start_sync();
                } else {
                    info!(
                        "Sync retry: Idle but behind peers (local_h={}, network_tip={}, gap={}, retry={}/5). Waiting for gossip.",
                        self.local_height, self.network_tip_height, gap, self.idle_behind_retries
                    );
                }
            } else {
                self.idle_behind_retries = 0;
                info!(
                    "Sync retry: Idle but behind peers (local_h={}, network_tip={}). Restarting sync.",
                    self.local_height, self.network_tip_height
                );
                self.start_sync();
            }
        } else if !matches!(self.state, SyncState::Idle) || !self.should_sync() {
            // Reset retry counter when no longer stuck
            self.idle_behind_retries = 0;
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
            SyncState::SnapCollectingRoots { .. } => Some(5.0),
            SyncState::SnapDownloading { .. } => Some(50.0),
            SyncState::SnapReady { .. } => Some(95.0),
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
        let manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        assert!(matches!(*manager.state(), SyncState::Idle));
        assert_eq!(manager.local_tip(), (0, Hash::ZERO, 0));
    }

    // =========================================================================
    // P0 #2: "Ahead of network" detection tests
    // Layer 7 (AheadOfPeers) was REMOVED (2026-02-25) — Satoshi principle.
    // These tests now verify that production is ALLOWED even when ahead.
    // =========================================================================

    #[test]
    fn test_production_allowed_when_ahead_of_peers() {
        // Layer 7 removed: node at height 992, peers at 910 — should still produce.
        // Forks are resolved by longest chain reorg, not by stopping production.
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.local_height = 992;
        manager.local_slot = 992;

        let peer1 = PeerId::random();
        let peer2 = PeerId::random();
        manager.add_peer(peer1, 910, Hash::ZERO, 910);
        manager.add_peer(peer2, 910, Hash::ZERO, 910);

        manager.has_connected_to_peer = true;
        manager.first_peer_status_received = Some(std::time::Instant::now());

        let result = manager.can_produce(993);
        assert_eq!(result, ProductionAuthorization::Authorized);
    }

    #[test]
    fn test_production_allowed_when_within_range_of_peers() {
        // Scenario: Node at height 912, peers at 910 (only 2 blocks ahead)
        // Should be allowed to produce (within threshold)
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Set local height to 912
        manager.local_height = 912;
        manager.local_slot = 912;

        // Add TWO peers at height 910 to satisfy min_peers_for_production
        let peer1 = PeerId::random();
        let peer2 = PeerId::random();
        manager.add_peer(peer1, 910, Hash::ZERO, 910);
        manager.add_peer(peer2, 910, Hash::ZERO, 910);

        // Need to clear bootstrap phase requirements
        manager.has_connected_to_peer = true;
        manager.first_peer_status_received = Some(std::time::Instant::now());

        // Verify: Should be authorized (2 blocks ahead is within default threshold of 5)
        let result = manager.can_produce(913);
        assert_eq!(result, ProductionAuthorization::Authorized);
    }

    #[test]
    fn test_max_heights_ahead_no_longer_blocks() {
        // Layer 7 removed: configurable threshold no longer blocks production.
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.set_max_heights_ahead(2);
        manager.local_height = 915;
        manager.local_slot = 915;

        let peer1 = PeerId::random();
        let peer2 = PeerId::random();
        manager.add_peer(peer1, 910, Hash::ZERO, 910);
        manager.add_peer(peer2, 910, Hash::ZERO, 910);

        manager.has_connected_to_peer = true;
        manager.first_peer_status_received = Some(std::time::Instant::now());

        // Even 5 blocks ahead should be authorized now
        let result = manager.can_produce(916);
        assert_eq!(result, ProductionAuthorization::Authorized);
    }

    // =========================================================================
    // Combined scenario tests
    // =========================================================================

    #[test]
    fn test_forked_node_scenario_produces_on_best_chain() {
        // Layer 7 removed (2026-02-25): A node ahead of peers should still produce.
        // If it's truly forked, the longest chain rule will resolve it via reorg.
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.local_height = 992;
        manager.local_slot = 992;

        let peer1 = PeerId::random();
        let peer2 = PeerId::random();
        manager.add_peer(peer1, 910, Hash::ZERO, 910);
        manager.add_peer(peer2, 910, Hash::ZERO, 910);

        manager.has_connected_to_peer = true;
        manager.first_peer_status_received = Some(std::time::Instant::now());

        let result = manager.can_produce(993);
        assert_eq!(
            result,
            ProductionAuthorization::Authorized,
            "Node should produce on its best chain, got: {:?}",
            result
        );
    }

    // =========================================================================
    // Echo chamber prevention tests (P0 #5)
    // =========================================================================

    #[test]
    fn test_insufficient_peers_blocks_production() {
        // Scenario: Node with only 1 peer (echo chamber risk)
        // Should be blocked from producing to prevent isolated cluster forks
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Node at height 100
        manager.local_height = 100;
        manager.local_slot = 100;

        // Only 1 peer - insufficient for safe production
        let peer = PeerId::random();
        manager.add_peer(peer, 100, Hash::ZERO, 100);
        manager.has_connected_to_peer = true;
        manager.first_peer_status_received = Some(std::time::Instant::now());

        let result = manager.can_produce(101);
        match result {
            ProductionAuthorization::BlockedInsufficientPeers {
                peer_count,
                min_required,
            } => {
                assert_eq!(peer_count, 1);
                assert_eq!(min_required, 2);
            }
            other => panic!("Expected BlockedInsufficientPeers, got: {:?}", other),
        }
    }

    #[test]
    fn test_sufficient_peers_allows_production() {
        // Scenario: Node with 2+ peers (safe to produce)
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Node at height 100
        manager.local_height = 100;
        manager.local_slot = 100;

        // 2 peers - sufficient for safe production
        let peer1 = PeerId::random();
        let peer2 = PeerId::random();
        manager.add_peer(peer1, 100, Hash::ZERO, 100);
        manager.add_peer(peer2, 100, Hash::ZERO, 100);
        manager.has_connected_to_peer = true;
        manager.first_peer_status_received = Some(std::time::Instant::now());

        let result = manager.can_produce(101);
        assert_eq!(result, ProductionAuthorization::Authorized);
    }

    #[test]
    fn test_insufficient_peers_check_skipped_at_genesis() {
        // Scenario: Node at height 0 (genesis) with only 1 peer
        // Should NOT be blocked by insufficient peers at genesis
        // (there may be legitimate first-producer scenarios)
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Node at height 0 (genesis)
        manager.local_height = 0;
        manager.local_slot = 0;

        // Only 1 peer at genesis
        let peer = PeerId::random();
        manager.add_peer(peer, 0, Hash::ZERO, 0);
        manager.has_connected_to_peer = true;
        manager.first_peer_status_received = Some(std::time::Instant::now());

        let result = manager.can_produce(0);
        // Should NOT be BlockedInsufficientPeers at height 0
        assert!(
            !matches!(
                result,
                ProductionAuthorization::BlockedInsufficientPeers { .. }
            ),
            "Should not block for insufficient peers at genesis, got: {:?}",
            result
        );
    }

    #[test]
    fn test_ahead_of_network_tip_still_produces() {
        // Layer 7 removed (2026-02-25): Node ahead of network_tip should still produce.
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.local_height = 136;
        manager.local_slot = 136;

        assert!(manager.peers.is_empty());
        manager.network_tip_height = 93;
        manager.network_tip_slot = 93;

        manager.set_min_peers_for_production(0);
        manager.has_connected_to_peer = false;

        let result = manager.can_produce(140);
        // With Layer 7 removed, this should be authorized
        assert!(
            !matches!(result, ProductionAuthorization::BlockedAheadOfPeers { .. }),
            "Layer 7 removed: should not block as AheadOfPeers, got: {:?}",
            result
        );
    }

    #[test]
    fn test_echo_chamber_check_disabled_allows_production_when_peer_behind() {
        // UPDATED TEST (2026-02-04):
        // The "lowest peer" echo chamber check was DISABLED because it caused
        // chain deadlock when peers legitimately fell behind.
        //
        // Scenario: Healthy node has peers at different heights
        // - Node has peers: {peer1: height=93, peer2: height=136}
        // - Node local_height = 136 (same as peer2, ahead of peer1)
        // - OLD: Blocked because 136 - 93 = 43 > 5 (ahead of lowest)
        // - NEW: AUTHORIZED - peer behind is OK, we're not ahead of BEST peer
        //
        // Echo chambers are now detected by other mechanisms:
        // - Sync failures (P0 #4)
        // - InsufficientPeers check (P0 #5)
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Node at height 136
        manager.local_height = 136;
        manager.local_slot = 136;

        // Two peers: one behind (93), one at same height (136)
        let behind_peer = PeerId::random();
        let synced_peer = PeerId::random();
        manager.add_peer(behind_peer, 93, Hash::ZERO, 93);
        manager.add_peer(synced_peer, 136, Hash::ZERO, 136);

        // Mark bootstrap checks as passed
        manager.has_connected_to_peer = true;
        manager.first_peer_status_received = Some(std::time::Instant::now());

        // Verify preconditions
        assert_eq!(manager.peers.len(), 2);
        assert_eq!(manager.best_peer_height(), 136);
        assert_eq!(manager.lowest_peer_height(), Some(93));

        let result = manager.can_produce(140);

        // Should be AUTHORIZED - having a peer behind doesn't mean we're forked
        // The sync failure check and other mechanisms catch actual forks
        match result {
            ProductionAuthorization::Authorized => {
                // Correct - we're not ahead of best peer, peer behind is OK
            }
            other => panic!(
                "Expected Authorized (echo chamber check disabled), got: {:?}",
                other
            ),
        }
    }

    // =========================================================================
    // Slot-aware sync recovery tests (sync stall deadlock fix)
    // =========================================================================

    #[test]
    fn test_should_sync_uses_height_not_slot() {
        // should_sync() uses HEIGHT only (not slot) to prevent forked peers
        // with inflated slots from triggering unnecessary sync.
        // Peer behind in height (834 < 876) but ahead in slot (919 > 261)
        // should NOT trigger sync.
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.local_height = 876;
        manager.local_slot = 261;

        let peer = PeerId::random();
        manager.peers.insert(
            peer,
            PeerSyncStatus {
                best_height: 834,
                best_hash: Hash::ZERO,
                best_slot: 919,
                last_status_response: Instant::now(),
                last_block_received: None,
                pending_request: None,
            },
        );

        assert!(
            !manager.should_sync(),
            "should_sync() must NOT sync when peer is behind in height (834 < 876), even with higher slot"
        );
    }

    #[test]
    fn test_should_sync_triggers_when_peer_ahead_in_height() {
        // should_sync() triggers when a peer has more blocks (higher height)
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.local_height = 100;
        manager.local_slot = 100;

        let peer = PeerId::random();
        manager.peers.insert(
            peer,
            PeerSyncStatus {
                best_height: 500,
                best_hash: Hash::ZERO,
                best_slot: 500,
                last_status_response: Instant::now(),
                last_block_received: None,
                pending_request: None,
            },
        );

        assert!(
            manager.should_sync(),
            "should_sync() must trigger when peer is ahead in height (500 > 100)"
        );
    }

    #[test]
    fn test_best_peer_ignores_peer_behind_in_height() {
        // best_peer() only returns peers with MORE BLOCKS (higher height).
        // A peer behind in height but ahead in slot should be ignored.
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.local_height = 876;
        manager.local_slot = 261;

        let peer = PeerId::random();
        manager.peers.insert(
            peer,
            PeerSyncStatus {
                best_height: 834,
                best_hash: Hash::ZERO,
                best_slot: 919,
                last_status_response: Instant::now(),
                last_block_received: None,
                pending_request: None,
            },
        );

        let result = manager.best_peer();
        assert_eq!(
            result, None,
            "best_peer() must return None when peer is behind in height (834 < 876)"
        );
    }

    #[test]
    fn test_stall_recovery_resets_to_idle() {
        // Scenario: Synchronized state but significantly behind in slots.
        // cleanup() should detect stall and reset to Idle.
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Simulate: height matches but slots diverge (the deadlock scenario)
        manager.local_height = 876;
        manager.local_slot = 261;
        manager.state = SyncState::Synchronized;

        let peer = PeerId::random();
        manager.peers.insert(
            peer,
            PeerSyncStatus {
                best_height: 876,
                best_hash: Hash::ZERO,
                best_slot: 920,
                last_status_response: Instant::now(),
                last_block_received: None,
                pending_request: None,
            },
        );

        // Slot lag = 920 - 261 = 659, threshold = 2 * 5 = 10 → 659 > 10 → stall detected
        manager.cleanup();

        // State should no longer be Synchronized (either Idle or started sync)
        assert!(
            !matches!(manager.state, SyncState::Synchronized),
            "cleanup() must reset Synchronized state when slot lag ({}) exceeds stall threshold",
            920 - 261
        );
    }

    #[test]
    fn test_update_local_tip_requires_slot_alignment() {
        // Scenario: peer at height 100/slot 500, we reach height 100 but only slot 100.
        // update_local_tip should NOT mark us as Synchronized because slots don't align.
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Start in a syncing state
        let peer = PeerId::random();
        manager.peers.insert(
            peer,
            PeerSyncStatus {
                best_height: 100,
                best_hash: Hash::ZERO,
                best_slot: 500,
                last_status_response: Instant::now(),
                last_block_received: None,
                pending_request: None,
            },
        );

        manager.state = SyncState::DownloadingHeaders {
            target_slot: 500,
            peer,
            headers_count: 0,
        };

        // Height matches peer but slot is way behind
        manager.update_local_tip(100, Hash::ZERO, 100);

        // Should NOT be Synchronized because slot lag = 500 - 100 = 400 >> max_slots_behind (2)
        assert!(
            !matches!(manager.state, SyncState::Synchronized),
            "update_local_tip must not mark Synchronized when slot lag is {} (max_slots_behind={})",
            400,
            manager.max_slots_behind
        );
    }

    #[test]
    fn test_processing_stuck_recovery_on_block_applied() {
        // Reproduce: node downloads 58 blocks, applies them all, but network_tip
        // advanced to 59 during processing. Processing state with no pending work
        // should transition to Idle and start a new sync round.
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Simulate: downloaded blocks 1-58, now in Processing state
        manager.state = SyncState::Processing { height: 1 };
        manager.network_tip_height = 59; // Gossip bumped this during processing
        manager.network_tip_slot = 64;

        let peer = PeerId::random();
        manager.peers.insert(
            peer,
            PeerSyncStatus {
                best_height: 59,
                best_hash: Hash::ZERO,
                best_slot: 64,
                last_status_response: Instant::now(),
                last_block_received: None,
                pending_request: None,
            },
        );

        // pending_headers and pending_blocks are empty (all applied)
        assert!(manager.pending_headers.is_empty());
        assert!(manager.pending_blocks.is_empty());

        // Apply the last block (h=58) — completion check fails: 58 < 59
        let hash = crypto::hash::hash(b"block58");
        manager.block_applied_with_weight(hash, 58, 60, 1, Hash::ZERO);

        // Should NOT be stuck in Processing — should have transitioned to Idle or started sync
        assert!(
            !matches!(manager.state, SyncState::Processing { .. }),
            "Must not stay stuck in Processing when no pending work remains (state={:?})",
            manager.state
        );
    }

    #[test]
    fn test_processing_stuck_recovery_via_cleanup() {
        // Safety net: even if block_applied doesn't fire, cleanup() detects
        // a stuck Processing state with no pending work.
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.state = SyncState::Processing { height: 1 };
        manager.local_height = 58;
        manager.local_slot = 60;
        manager.network_tip_height = 65;
        manager.network_tip_slot = 70;
        // Simulate stuck state: no sync activity for >30s
        manager.last_block_applied = Instant::now() - Duration::from_secs(60);
        manager.last_sync_activity = Instant::now() - Duration::from_secs(60);

        let peer = PeerId::random();
        manager.peers.insert(
            peer,
            PeerSyncStatus {
                best_height: 65,
                best_hash: Hash::ZERO,
                best_slot: 70,
                last_status_response: Instant::now(),
                last_block_received: None,
                pending_request: None,
            },
        );

        // No pending work
        assert!(manager.pending_headers.is_empty());
        assert!(manager.pending_blocks.is_empty());

        manager.cleanup();

        assert!(
            !matches!(manager.state, SyncState::Processing { .. }),
            "cleanup() must recover stuck Processing state (state={:?})",
            manager.state
        );
    }

    // =========================================================================
    // Fix verification: concurrent requests and stale response handling
    // =========================================================================

    fn create_test_header(prev_hash: Hash, slot: u32) -> BlockHeader {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        BlockHeader {
            version: 1,
            prev_hash,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: now,
            slot,
            producer: crypto::PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput { value: vec![0; 32] },
            vdf_proof: vdf::VdfProof::empty(),
        }
    }

    fn build_header_chain(genesis: Hash, count: usize) -> Vec<BlockHeader> {
        let mut headers = Vec::with_capacity(count);
        let mut prev = genesis;
        for i in 0..count {
            let h = create_test_header(prev, (i + 1) as u32);
            prev = h.hash();
            headers.push(h);
        }
        headers
    }

    #[test]
    fn test_next_request_guard_prevents_duplicate_requests() {
        // Fix 1: next_request() must return None when peer already has pending request
        let genesis = Hash::ZERO;
        let mut manager = SyncManager::new(SyncConfig::default(), genesis);

        let peer = PeerId::random();
        manager.add_peer(peer, 1000, Hash::ZERO, 1000);

        // Trigger sync
        manager.start_sync();
        assert!(matches!(
            manager.state,
            SyncState::DownloadingHeaders { .. }
        ));

        // First request should succeed
        let req1 = manager.next_request();
        assert!(req1.is_some(), "First request should be generated");

        // Second request should be blocked (peer has pending request)
        let req2 = manager.next_request();
        assert!(
            req2.is_none(),
            "Second request must be blocked — peer already has pending request"
        );
    }

    #[test]
    fn test_chain_break_preserves_state_on_stale_response() {
        // Fix 2: A single chain break (stale response) must NOT destroy progress.
        // process_headers() doesn't modify expected_prev_hash when valid_count=0,
        // so the downloader state is still correct — just skip and continue.
        let genesis = Hash::ZERO;
        let mut manager = SyncManager::new(SyncConfig::default(), genesis);

        let peer = PeerId::random();
        manager.add_peer(peer, 1000, Hash::ZERO, 1000);
        manager.start_sync();

        // First: download some valid headers to build up state
        let _ = manager.next_request();
        let chain = build_header_chain(genesis, 5);
        let expected_hash = chain[4].hash();
        let _blocks = manager.handle_response(peer, SyncResponse::Headers(chain));

        // Verify we have progress
        assert!(matches!(
            manager.state,
            SyncState::DownloadingHeaders {
                headers_count: 5,
                ..
            }
        ));

        // Now: simulate a stale response (doesn't chain)
        let _ = manager.next_request();
        let wrong_prev = Hash::from_bytes([0xAB; 32]);
        let bad_headers = vec![create_test_header(wrong_prev, 1)];
        let _blocks = manager.handle_response(peer, SyncResponse::Headers(bad_headers));

        // Verify: state STAYS in DownloadingHeaders (not reset to Idle)
        assert!(
            matches!(manager.state, SyncState::DownloadingHeaders { .. }),
            "Stale response must NOT reset state — got {:?}",
            manager.state
        );
        // Chain break correctly incremented as fork evidence
        assert_eq!(manager.consecutive_empty_headers, 1);
        // Verify: expected_prev_hash PRESERVED (not cleared)
        assert_eq!(
            manager.header_downloader.expected_prev_hash(),
            Some(expected_hash),
            "expected_prev_hash must be preserved after stale response"
        );
    }

    #[test]
    fn test_start_sync_clears_header_downloader() {
        // Fix 3: start_sync() must clear stale expected_prev_hash
        let genesis = Hash::ZERO;
        let mut manager = SyncManager::new(SyncConfig::default(), genesis);

        let peer = PeerId::random();
        manager.add_peer(peer, 1000, Hash::ZERO, 1000);

        // Poison the header downloader with a stale expected_prev_hash
        let chain = build_header_chain(genesis, 5);
        manager.header_downloader.process_headers(&chain, genesis);
        assert!(
            manager.header_downloader.expected_prev_hash().is_some(),
            "Setup: expected_prev_hash should be set after processing headers"
        );

        // start_sync must clear it
        manager.start_sync();
        assert_eq!(
            manager.header_downloader.expected_prev_hash(),
            None,
            "start_sync() must clear expected_prev_hash for a clean slate"
        );
    }

    #[test]
    fn test_stale_response_discarded_when_no_pending_request() {
        // Fix 4: responses with no matching pending_request must be discarded
        let genesis = Hash::ZERO;
        let mut manager = SyncManager::new(SyncConfig::default(), genesis);

        let peer = PeerId::random();
        manager.add_peer(peer, 1000, Hash::ZERO, 1000);
        manager.start_sync();

        // Send request and consume response (clears pending_request)
        let _ = manager.next_request();
        let chain = build_header_chain(genesis, 5);
        let _blocks = manager.handle_response(peer, SyncResponse::Headers(chain.clone()));

        // Now send a second (stale) response — no pending_request exists
        let stale_chain = build_header_chain(genesis, 3);
        let _blocks = manager.handle_response(peer, SyncResponse::Headers(stale_chain));

        // The stale response reached the handler but its headers don't chain to our tip.
        // This correctly counts as fork evidence (chain break path).
        assert_eq!(manager.consecutive_empty_headers, 1);
    }

    #[test]
    fn test_full_concurrent_scenario_no_corruption() {
        // Integration test: simulates the exact production scenario that caused the bug.
        // 1. Sync starts, peer has 100 blocks
        // 2. Due to Fix 1, only ONE request goes out (not 10)
        // 3. Response arrives with valid headers
        // 4. Next request goes out for the continuation
        // 5. Second response arrives — chain continues correctly
        let genesis = Hash::ZERO;
        let mut manager = SyncManager::new(SyncConfig::default(), genesis);

        let peer = PeerId::random();
        let full_chain = build_header_chain(genesis, 10);
        let tip_hash = full_chain.last().unwrap().hash();
        manager.add_peer(peer, 10, tip_hash, 100);
        manager.start_sync();

        // Round 1: request + response
        let req1 = manager.next_request();
        assert!(req1.is_some());
        // Guard: no second request while first is pending
        assert!(manager.next_request().is_none());

        let batch1 = full_chain[..5].to_vec();
        let _blocks = manager.handle_response(peer, SyncResponse::Headers(batch1));

        // After response processed: state should still be DownloadingHeaders
        // and expected_prev_hash should be at header 5
        let _expected_hash = full_chain[4].hash();
        if let SyncState::DownloadingHeaders { headers_count, .. } = manager.state {
            assert_eq!(headers_count, 5, "Should have 5 headers counted");
        } else {
            panic!("Expected DownloadingHeaders state");
        }

        // Round 2: continuation request
        let req2 = manager.next_request();
        assert!(req2.is_some(), "Should be able to request more headers");

        let batch2 = full_chain[5..10].to_vec();
        let _blocks = manager.handle_response(peer, SyncResponse::Headers(batch2));

        if let SyncState::DownloadingHeaders { headers_count, .. } = manager.state {
            assert_eq!(headers_count, 10, "Should have all 10 headers counted");
        } else {
            panic!("Expected DownloadingHeaders state");
        }

        // Verify: no empty headers (no fork detection triggered)
        assert_eq!(manager.consecutive_empty_headers, 0);
    }
}
