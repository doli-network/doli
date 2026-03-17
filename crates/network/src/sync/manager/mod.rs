//! Sync manager coordinating chain synchronization
//!
//! The SyncManager orchestrates the synchronization process:
//! 1. Discovers peers with higher chain tip
//! 2. Downloads headers first to validate VDF chain
//! 3. Downloads block bodies in parallel
//! 4. Applies blocks to local chain

mod block_lifecycle;
mod cleanup;
mod production_gate;
mod snap_sync;
mod sync_engine;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use libp2p::PeerId;
use tracing::{debug, info, warn};

use crypto::Hash;
use doli_core::{Block, BlockHeader};

use super::bodies::BodyDownloader;
use super::headers::HeaderDownloader;
use super::reorg::ReorgHandler;
use crate::protocols::SyncRequest;

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

    /// Reorg cooldown: when a fork sync is rejected (delta=0 or shorter chain),
    /// suppress further fork syncs for this duration. Prevents infinite reorg loops
    /// when multiple peers offer equal-weight competing chains (e.g., 50 nodes
    /// joining simultaneously with propagation delay).
    last_fork_sync_rejection: Instant,
    fork_sync_cooldown_secs: u64,

    /// Set after snap sync / force_recover to suppress fork_sync reactivation.
    /// The node needs time to sync via header-first / gossip before fork detection
    /// makes sense. Cleared after 10+ blocks are applied post-recovery.
    post_recovery_grace: bool,
    /// When post_recovery_grace was activated (for timeout-based clearing)
    post_recovery_grace_started: Instant,
    /// Blocks applied since last recovery — used to clear post_recovery_grace.
    blocks_applied_since_recovery: u32,
    /// When the node first became behind by >= 2 blocks (for L6.5 timeout).
    /// Reset when gap closes to < 2.
    behind_since: Option<Instant>,

    // === PRODUCTION GATE DEADLOCK FIX (PGD) FIELDS ===
    /// Hard cap on effective grace period after resync (seconds).
    /// Prevents exponential backoff from disabling producers for 480s+.
    /// Default: 60s (6 slots at 10s/slot).
    max_grace_cap_secs: u64,
    /// Blocks applied since the last resync completed.
    /// When this reaches 5, `reset_resync_counter()` is called to clear the
    /// exponential backoff. Only incremented when `resync_in_progress` is false.
    blocks_since_resync_completed: u32,
    /// Height offset detection: tracks (gap, timestamp) when we first observed
    /// a stable gap while blocks are still being applied. If the gap stays
    /// constant for >120s despite blocks being applied, the node has a corrupted
    /// height counter (from a bad reorg) and needs snap sync to correct it.
    stable_gap_since: Option<(u64, Instant)>,

    /// Dedicated fork signal from cleanup/stuck detection.
    ///
    /// Replaces the pattern of force-setting `consecutive_empty_headers = 3` in
    /// cleanup.rs and block_lifecycle.rs. The old pattern caused counter oscillation:
    /// cleanup sets to 3 → rollback resets to 0 → cleanup sets to 3 → repeat.
    /// This dedicated flag breaks the oscillation by decoupling fork signaling
    /// from the counter that tracks actual empty header responses.
    pub(crate) stuck_fork_signal: bool,
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
            max_solo_production_secs: 86400, // Disabled for local dev (was 50s)
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
            last_fork_sync_rejection: Instant::now() - std::time::Duration::from_secs(300),
            fork_sync_cooldown_secs: 30,
            post_recovery_grace: false,
            post_recovery_grace_started: Instant::now(),
            blocks_applied_since_recovery: 0,
            behind_since: None,
            stable_gap_since: None,
            // PGD fix defaults
            max_grace_cap_secs: 60,
            blocks_since_resync_completed: 0,
            stuck_fork_signal: false,
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

                    // Reset blocks-since-resync counter so block_applied_with_weight()
                    // can track stable operation and call reset_resync_counter() after
                    // 5 consecutive successful block applications.
                    if was_syncing && self.consecutive_resync_count > 0 {
                        self.blocks_since_resync_completed = 0;
                        debug!(
                            "Sync completed after {} consecutive resyncs — tracking stable blocks for counter reset",
                            self.consecutive_resync_count
                        );
                    }
                }
            }
        }
    }

    // =========================================================================
    // PEER MANAGEMENT
    // =========================================================================

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

        // FIX: Recompute network_tip_height from remaining peers + local height.
        // Without this, a peer that briefly reported an inflated height (e.g.,
        // during a fork) permanently inflates network_tip_height, creating a
        // phantom gap that triggers unnecessary sync/snap sync (Path E cascade).
        let peer_max_height = self
            .peers
            .values()
            .map(|p| p.best_height)
            .max()
            .unwrap_or(0);
        let peer_max_slot = self.peers.values().map(|p| p.best_slot).max().unwrap_or(0);
        self.network_tip_height = peer_max_height.max(self.local_height);
        self.network_tip_slot = peer_max_slot.max(self.local_slot);

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

    /// Pick the best peer for fork recovery (highest height).
    pub fn best_peer_for_recovery(&self) -> Option<PeerId> {
        self.peers
            .iter()
            .max_by_key(|(_, status)| status.best_height)
            .map(|(peer, _)| *peer)
    }
}
