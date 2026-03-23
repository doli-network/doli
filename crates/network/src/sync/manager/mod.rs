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
pub(crate) struct PendingRequest {
    pub peer: PeerId,
    pub request: SyncRequest,
    pub sent_at: Instant,
    /// Sync epoch when this request was created. Responses from old epochs are discarded.
    pub epoch: u64,
}

/// Sync pipeline state: pending headers, bodies, blocks, requests, downloaders,
/// and the sync epoch counter.
///
/// Groups all header-first sync pipeline fields that were previously flat on SyncManager.
pub(crate) struct SyncPipeline {
    /// Headers waiting to be processed
    pub pending_headers: VecDeque<BlockHeader>,
    /// Header hashes we need bodies for
    pub headers_needing_bodies: VecDeque<Hash>,
    /// Downloaded blocks waiting to be applied
    pub pending_blocks: HashMap<Hash, Block>,
    /// Pending requests by ID
    pub pending_requests: HashMap<SyncRequestId, PendingRequest>,
    /// Next request ID
    pub next_request_id: u64,
    /// Monotonic epoch counter
    pub sync_epoch: u64,
    /// Header downloader
    pub header_downloader: HeaderDownloader,
    /// Body downloader
    pub body_downloader: BodyDownloader,
    /// Body stall retry counter
    pub body_stall_retries: u32,
}

impl SyncPipeline {
    /// Create a new SyncPipeline with default values
    pub fn new(config: &SyncConfig) -> Self {
        Self {
            pending_headers: VecDeque::new(),
            headers_needing_bodies: VecDeque::new(),
            pending_blocks: HashMap::new(),
            pending_requests: HashMap::new(),
            next_request_id: 0,
            sync_epoch: 0,
            header_downloader: HeaderDownloader::new(
                config.max_headers_per_request,
                config.request_timeout,
            ),
            body_downloader: BodyDownloader::new(
                config.max_bodies_per_request,
                config.max_concurrent_body_requests,
                config.request_timeout,
            ),
            body_stall_retries: 0,
        }
    }
}

/// Recovery phase of the sync manager.
///
/// Replaces 5 independent booleans (`resync_in_progress`, `post_rollback`,
/// `post_recovery_grace`, `awaiting_canonical_block`, `stuck_fork_signal`) with
/// a single enum. Illegal states (e.g., resync AND post_recovery_grace simultaneously)
/// become unrepresentable. State space drops from 2^5=32 to 6 variants.
///
/// Lifecycle:
///   Normal → StuckForkDetected → (fork_sync/rollback) → PostRollback → (sync) → Normal
///   Normal → ResyncInProgress → PostRecoveryGrace → Normal
///   Normal → (snap sync) → AwaitingCanonicalBlock → Normal
#[derive(Clone, Debug)]
pub enum RecoveryPhase {
    /// No recovery in progress. Normal operation.
    Normal,
    /// Stuck fork detected by cleanup or sync engine.
    /// Waiting for fork_sync (binary search) to resolve.
    StuckForkDetected,
    /// Fork sync completed with rollback. Next start_sync() should
    /// NOT use header-first sync (our tip is still on the fork).
    PostRollback,
    /// Forced resync from genesis/snap is in progress.
    ResyncInProgress,
    /// Resync/recovery completed. Grace period suppresses fork_sync reactivation.
    PostRecoveryGrace {
        /// When grace started.
        started: Instant,
        /// Blocks applied since recovery (cleared at 10).
        blocks_applied: u32,
    },
    /// Snap sync completed. Waiting for first canonical gossip block
    /// before allowing production.
    AwaitingCanonicalBlock,
}

/// Network tip tracking, gossip timing, block application counters.
///
/// Groups fields related to network state observation (heights seen via gossip,
/// block application timing, progress logging) into a single sub-struct.
pub(crate) struct NetworkState {
    /// Best height seen via gossip/peers
    pub network_tip_height: u64,
    /// Best slot seen via gossip/peers
    pub network_tip_slot: u32,
    /// Last time we received ANY block (gossip or sync)
    pub last_block_seen: Instant,
    /// Last time we successfully applied a block
    pub last_block_applied: Instant,
    /// Last time any sync progress occurred
    pub last_sync_activity: Instant,
    /// Blocks applied since last log
    pub blocks_applied: u64,
    /// Last progress log time
    pub last_progress_log: Instant,
    /// Idle-behind retry counter
    pub idle_behind_retries: u32,
}

impl NetworkState {
    /// Create a new NetworkState with default values
    pub fn new() -> Self {
        Self {
            network_tip_height: 0,
            network_tip_slot: 0,
            last_block_seen: Instant::now(),
            last_block_applied: Instant::now(),
            last_sync_activity: Instant::now(),
            blocks_applied: 0,
            last_progress_log: Instant::now(),
            idle_behind_retries: 0,
        }
    }
}

/// Snap sync configuration and runtime state.
///
/// Groups all fields related to snap sync (state root voting, snapshot download,
/// quorum, blacklists, attempts) into a single sub-struct for clarity.
pub(crate) struct SnapSyncState {
    /// Height gap threshold to trigger snap sync instead of header-first
    pub threshold: u64,
    /// Number of peers that must agree on the same state root
    pub quorum: usize,
    /// Timeout for collecting state root votes
    pub root_timeout: Duration,
    /// Timeout for downloading snapshot from a peer
    pub download_timeout: Duration,
    /// Peers that served bad snapshots (blacklisted for this session)
    pub blacklisted_peers: HashSet<PeerId>,
    /// Number of consecutive snap sync failures
    pub attempts: u8,
    /// When a fresh node first started waiting for peers
    pub fresh_node_wait_start: Option<Instant>,
    /// Lowest block height in local store
    pub store_floor: u64,
}

impl SnapSyncState {
    /// Create a new SnapSyncState with default values
    pub fn new() -> Self {
        Self {
            // Disabled by default — snap sync caused 4+ incidents (INC-I-004, INC-I-005)
            // due to invariant violations (empty block store, false fork cascades).
            // Use `doli snap` CLI command for manual snap sync when needed.
            threshold: u64::MAX,
            quorum: 5, // Minimum 5 peers for partition safety
            root_timeout: Duration::from_secs(10),
            download_timeout: Duration::from_secs(60),
            blacklisted_peers: HashSet::new(),
            attempts: 0,
            fresh_node_wait_start: None,
            store_floor: 1, // Default: full-sync node has block 1
        }
    }
}

/// Fork detection and recovery state.
///
/// Groups all fork-related fields (counters, fork_sync, fork_recovery, blacklists,
/// cooldowns, circuit breakers) into a single sub-struct. Single point of truth for
/// "are we on a fork, and what are we doing about it?"
pub(crate) struct ForkState {
    /// Consecutive empty header responses (peers don't recognize our chain tip).
    /// When >= 10, we're on a deep fork and need genesis resync.
    /// Only incremented for truly EMPTY responses (peer doesn't have our hash).
    pub consecutive_empty_headers: u32,
    /// Consecutive sync failures (empty header responses)
    pub consecutive_sync_failures: u32,
    /// Max sync failures before fork detection triggers
    pub max_sync_failures_before_fork_detection: u32,
    /// Consecutive block-apply failures (downloaded chain can't be applied).
    /// Unlike consecutive_empty_headers (reset by header downloads), this is
    /// ONLY reset by successful block_applied(). At >= 3, triggers genesis resync.
    pub consecutive_apply_failures: u32,
    /// Flag set when genesis fallback confirms peers reject our chain —
    /// node must call force_resync_from_genesis().
    pub needs_genesis_resync: bool,
    /// Fork sync — binary search for common ancestor when on a dead fork.
    pub fork_sync: Option<super::fork_sync::ForkSync>,
    /// Reorg cooldown: when a fork sync is rejected (delta=0 or shorter chain),
    /// suppress further fork syncs for this duration. Prevents infinite reorg loops
    /// when multiple peers offer equal-weight competing chains.
    pub last_fork_sync_rejection: Instant,
    /// Reorg cooldown duration (seconds)
    pub fork_sync_cooldown_secs: u64,
    /// Fork sync loop circuit breaker count.
    pub consecutive_fork_syncs: u32,
    /// Timestamp of last fork sync (for breaker window expiry).
    pub last_fork_sync_at: Option<Instant>,
    /// Recently-held tip hashes to prevent equal-weight ping-pong.
    /// Capacity: 10. If a remedial reorg would switch to a tip we recently held,
    /// reject it to break the oscillation.
    pub recently_held_tips: Vec<(Hash, Instant)>,
    /// Fork recovery tracker — active parent chain download for automatic reorg.
    pub fork_recovery: super::fork_recovery::ForkRecoveryTracker,
    /// Persistent fork detection flag.
    ///
    /// Set when Layer 9 (hash mismatch) detects we're in the minority.
    /// Blocks production until cleared by a successful resync from peers.
    ///
    /// Without this, Layer 9 oscillates: it detects the fork, blocks production,
    /// peers advance beyond the +/-2 comparison window, Layer 9 "forgets" the fork,
    /// and the node resumes producing on its orphan chain. This flag prevents
    /// that oscillation by remembering "we saw a fork, don't produce until resolved."
    pub fork_mismatch_detected: bool,
    /// Peers that returned empty headers (blacklisted with cooldown)
    pub header_blacklisted_peers: HashMap<PeerId, Instant>,
    /// Height offset detection: tracks (gap, timestamp) when we first observed
    /// a stable gap while blocks are still being applied. If the gap stays
    /// constant for >120s despite blocks being applied, the node has a corrupted
    /// height counter (from a bad reorg) and needs snap sync to correct it.
    pub stable_gap_since: Option<(u64, Instant)>,
    /// Peak height ever reached by this node (monotonically increasing).
    /// Used to detect and prevent the rollback death spiral: if the node has
    /// rolled back more than MAX_SAFE_ROLLBACK blocks from its peak, further
    /// rollbacks and equal-weight reorgs are rejected in favor of a clean resync.
    pub peak_height: u64,
}

impl ForkState {
    /// Create a new ForkState with default values
    pub fn new() -> Self {
        Self {
            consecutive_empty_headers: 0,
            consecutive_sync_failures: 0,
            max_sync_failures_before_fork_detection: 3,
            consecutive_apply_failures: 0,
            needs_genesis_resync: false,
            fork_sync: None,
            last_fork_sync_rejection: Instant::now() - Duration::from_secs(300),
            fork_sync_cooldown_secs: 30,
            consecutive_fork_syncs: 0,
            last_fork_sync_at: None,
            recently_held_tips: Vec::new(),
            fork_recovery: super::fork_recovery::ForkRecoveryTracker::new(),
            fork_mismatch_detected: false,
            header_blacklisted_peers: HashMap::new(),
            stable_gap_since: None,
            peak_height: 0,
        }
    }

    /// Check if the fork sync reorg cooldown is active.
    #[allow(dead_code)]
    fn is_fork_sync_on_cooldown(&self) -> bool {
        self.last_fork_sync_rejection.elapsed().as_secs() < self.fork_sync_cooldown_secs
    }

    /// Check if the fork sync circuit breaker is tripped.
    /// Returns true if 3+ fork syncs happened within 5 minutes.
    #[allow(dead_code)]
    fn is_fork_sync_breaker_tripped_internal(&self) -> bool {
        if self.consecutive_fork_syncs < 3 {
            return false;
        }
        match self.last_fork_sync_at {
            Some(ts) => ts.elapsed() < Duration::from_secs(300),
            None => false,
        }
    }

    /// Recommend a fork recovery action based on current state.
    /// The Node calls this from resolve_shallow_fork() and executes the returned action.
    #[allow(dead_code)]
    pub fn recommend_action(
        &self,
        gap: u64,
        consecutive_rollbacks: u32,
        max_rollback_depth: u32,
        best_peer: Option<PeerId>,
    ) -> ForkAction {
        // Deep fork: need genesis resync
        if self.consecutive_empty_headers >= 10 || self.needs_genesis_resync {
            return ForkAction::NeedsGenesisResync;
        }
        // Medium fork: binary search via fork_sync (if not already active, not on cooldown, breaker not tripped)
        if gap > max_rollback_depth as u64 {
            if let Some(peer) = best_peer {
                if self.fork_sync.is_none()
                    && !self.is_fork_sync_on_cooldown()
                    && !self.is_fork_sync_breaker_tripped_internal()
                {
                    return ForkAction::StartForkSync { peer };
                }
            }
            return ForkAction::None; // fork_sync active or on cooldown — wait
        }
        // Shallow fork: rollback one block
        if consecutive_rollbacks < max_rollback_depth && self.consecutive_empty_headers >= 3 {
            return ForkAction::RollbackOne;
        }
        ForkAction::None
    }
}

/// Recommended action when a fork is detected.
/// ForkState decides, Node executes.
#[allow(dead_code)]
pub enum ForkAction {
    /// No action needed
    None,
    /// Rollback one block (shallow fork, gap <= 12)
    RollbackOne,
    /// Start binary search for common ancestor (medium fork, gap <= 1000)
    StartForkSync {
        /// Peer to sync with
        peer: PeerId,
    },
    /// Node needs full genesis resync (deep fork or fork_sync bottomed out)
    NeedsGenesisResync,
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
    /// Sync pipeline: headers, bodies, blocks, requests, downloaders, epoch counter
    pub(crate) pipeline: SyncPipeline,
    /// Reorg handler
    reorg_handler: ReorgHandler,

    // === PRODUCTION GATE FIELDS (Defense-in-depth) ===
    /// Explicit production block: None = not blocked, Some(reason) = blocked
    production_blocked: Option<String>,
    /// Current recovery phase (replaces 5 independent booleans).
    /// See `RecoveryPhase` enum docs for lifecycle.
    pub(crate) recovery_phase: RecoveryPhase,
    /// Timestamp when last resync completed (for grace period)
    last_resync_completed: Option<Instant>,
    /// Number of consecutive resyncs (for exponential backoff)
    consecutive_resync_count: u32,
    /// Grace period after resync before production is allowed (seconds)
    resync_grace_period_secs: u64,
    /// Maximum slots behind peers before blocking production
    max_slots_behind: u32,

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

    /// Minimum peers required for production (P0 #5)
    min_peers_for_production: usize,

    // === NETWORK STATE (sub-struct) ===
    /// Network tip tracking, gossip timing, block application counters
    pub(crate) network: NetworkState,

    // === BOOTSTRAP GATE FIELDS ===
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

    // === FORK STATE (sub-struct) ===
    /// Fork detection/recovery coordination state
    pub(crate) fork: ForkState,

    /// Timestamp when all peers were lost (for peer loss timeout).
    /// After `peer_loss_timeout_secs`, production resumes solo.
    peers_lost_at: Option<Instant>,
    /// Seconds to wait after losing all peers before resuming production.
    peer_loss_timeout_secs: u64,

    // === SNAP SYNC STATE (sub-struct) ===
    /// Snap sync configuration and runtime state
    pub(crate) snap: SnapSyncState,

    // `post_recovery_grace`, `post_recovery_grace_started`, `blocks_applied_since_recovery`
    // moved to RecoveryPhase::PostRecoveryGrace { started, blocks_applied }
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
    // `stuck_fork_signal` moved to RecoveryPhase::StuckForkDetected
}

impl SyncManager {
    /// Create a new sync manager
    pub fn new(config: SyncConfig, genesis_hash: Hash) -> Self {
        Self {
            pipeline: SyncPipeline::new(&config),
            reorg_handler: ReorgHandler::new(),
            config,
            state: SyncState::Idle,
            local_height: 0,
            local_hash: genesis_hash,
            local_slot: 0,
            peers: HashMap::new(),
            // Production gate defaults
            production_blocked: None,
            recovery_phase: RecoveryPhase::Normal,
            last_resync_completed: None,
            consecutive_resync_count: 0,
            resync_grace_period_secs: 30, // Default 30 seconds after resync
            max_slots_behind: 2,          // Spec: "within 2 slots of peers"
            last_block_received_via_gossip: Some(Instant::now()), // Grace period starts at boot
            gossip_activity_timeout_secs: 180, // 3 minutes default
            max_solo_production_secs: 50, // INC-001: 5 slots (50s) — prevents long solo forks
            min_peers_for_production: 2,  // Need at least 2 peers to avoid echo chambers
            // Network state (sub-struct)
            network: NetworkState::new(),
            // Bootstrap gate defaults
            first_peer_status_received: None,
            last_peer_status_received: None,
            bootstrap_grace_period_secs: 15, // Wait 15s at genesis for chain evidence
            tier: 0,
            finality_tracker: doli_core::FinalityTracker::new(),
            // Fork state (sub-struct)
            fork: ForkState::new(),
            peers_lost_at: None,
            peer_loss_timeout_secs: 30,
            // Snap sync state (sub-struct)
            snap: SnapSyncState::new(),
            behind_since: None,
            // PGD fix defaults
            max_grace_cap_secs: 60,
            blocks_since_resync_completed: 0,
        }
    }

    /// Create a new sync manager with custom production gate settings
    pub fn new_with_settings(
        config: SyncConfig,
        genesis_hash: Hash,
        resync_grace_period_secs: u64,
        max_slots_behind: u32,
    ) -> Self {
        let mut mgr = Self::new(config, genesis_hash);
        mgr.resync_grace_period_secs = resync_grace_period_secs;
        mgr.max_slots_behind = max_slots_behind;
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
                    self.set_state(SyncState::Synchronized, "update_local_tip_caught_up");
                    self.fork.header_blacklisted_peers.clear();
                    info!("Chain synchronized at height {}", height);

                    // If we were in a resync, complete it now
                    if matches!(self.recovery_phase, RecoveryPhase::ResyncInProgress) {
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
        if height > self.network.network_tip_height {
            debug!(
                "Network tip height updated from peer status: {} -> {}",
                self.network.network_tip_height, height
            );
            self.network.network_tip_height = height;
        }
        if slot > self.network.network_tip_slot {
            debug!(
                "Network tip slot updated from peer status: {} -> {}",
                self.network.network_tip_slot, slot
            );
            self.network.network_tip_slot = slot;
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
        if height > self.network.network_tip_height {
            self.network.network_tip_height = height;
        }
        if slot > self.network.network_tip_slot {
            self.network.network_tip_slot = slot;
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
        self.network.network_tip_height = peer_max_height.max(self.local_height);
        self.network.network_tip_slot = peer_max_slot.max(self.local_slot);

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
        self.pipeline.body_downloader.cancel_peer(peer);

        // Cancel any pending requests from this peer
        self.pipeline
            .pending_requests
            .retain(|_, req| &req.peer != peer);

        // If we were syncing from this peer, try another
        if let SyncState::DownloadingHeaders {
            peer: sync_peer, ..
        } = &self.state
        {
            if sync_peer == peer {
                self.set_state(SyncState::Idle, "header_peer_disconnected");
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
        peer_max.max(self.network.network_tip_height)
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
        if slot > self.network.network_tip_slot {
            debug!(
                "Network tip slot updated from gossip: {} -> {}",
                self.network.network_tip_slot, slot
            );
            self.network.network_tip_slot = slot;
        }
    }

    /// Update network tip height when we successfully apply a block
    /// This is called after block application, not from gossip
    pub fn update_network_tip_height(&mut self, height: u64) {
        if height > self.network.network_tip_height {
            self.network.network_tip_height = height;
        }
    }

    /// Get current network tip (from gossip and applied blocks)
    pub fn network_tip(&self) -> (u64, u32) {
        (
            self.network.network_tip_height,
            self.network.network_tip_slot,
        )
    }

    /// Note that we received a block via gossip network (P0 #3)
    pub fn note_block_received_via_gossip(&mut self) {
        self.network.last_block_seen = Instant::now();
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
        self.network.last_block_seen = Instant::now();
        if let Some(status) = self.peers.get_mut(&peer_id) {
            status.last_block_received = Some(Instant::now());
            // Implicitly, if they sent us a block, they are reachable
            status.last_status_response = Instant::now();
        }
    }

    /// Check if the chain is stale (no blocks received for `threshold` duration).
    /// Used by Node to detect stuck state and trigger re-sync.
    pub fn is_chain_stale(&self, threshold: Duration) -> bool {
        self.network.last_block_seen.elapsed() > threshold
    }

    /// Transition to a new sync state with logging.
    /// All state transitions MUST go through this method for auditability.
    fn set_state(&mut self, new_state: SyncState, trigger: &str) {
        let old_label = self.state_label();
        self.state = new_state;
        let new_label = self.state_label();
        if old_label != new_label {
            debug!(
                "[SYNC_STATE] {} -> {} (trigger: {})",
                old_label, new_label, trigger
            );
        }
    }

    /// Short label for the current sync state (for logging)
    fn state_label(&self) -> &'static str {
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

    /// Human-readable sync state name for diagnostics
    pub fn sync_state_name(&self) -> &'static str {
        self.state_label()
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

    /// Pick the best peer for fork recovery.
    /// Prefers peers near the network tip to avoid syncing from other stuck/low nodes.
    /// Falls back to max-height peer if no near-tip peers are available.
    pub fn best_peer_for_recovery(&self) -> Option<PeerId> {
        let min_acceptable = self.network.network_tip_height.saturating_sub(10);
        // First: try peers near the network tip
        let near_tip = self
            .peers
            .iter()
            .filter(|(_, status)| status.best_height >= min_acceptable)
            .max_by_key(|(_, status)| status.best_height)
            .map(|(peer, _)| *peer);
        // Fallback: best available (preserves current behavior when no good peers exist)
        near_tip.or_else(|| {
            self.peers
                .iter()
                .max_by_key(|(_, status)| status.best_height)
                .map(|(peer, _)| *peer)
        })
    }
}
