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

/// Synchronization state — 3 variants only.
///
/// Data that was previously embedded in the state enum (peer IDs, slot targets,
/// vote vectors, snapshots) now lives in `SyncPipelineData` on SyncManager.
/// The state enum is a pure discriminant; the pipeline carries the data.
#[derive(Clone, Debug)]
pub enum SyncState {
    /// Not syncing, waiting for peers
    Idle,
    /// Actively syncing — the `SyncPhase` label tracks what the pipeline is doing.
    Syncing {
        /// Diagnostic label — what phase of sync we are in
        phase: SyncPhase,
        /// When this sync cycle started (for stuck detection)
        started_at: Instant,
    },
    /// Fully synchronized
    Synchronized,
}

/// Diagnostic label for the current sync phase.
///
/// NOT a state machine — just tracks what the sync pipeline is doing for
/// logging, progress reporting, and phase-specific timeout thresholds.
/// Transitions are implicit based on pipeline progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncPhase {
    DownloadingHeaders,
    DownloadingBodies,
    ProcessingBlocks,
    SnapCollecting,
    SnapDownloading,
}

/// Pipeline data — carries the data that was previously embedded in SyncState variants.
///
/// This is NOT a state machine. It holds operational data for the current sync phase.
/// State checks use `self.state` (SyncState); data access uses `self.pipeline_data`.
#[derive(Clone, Debug)]
pub enum SyncPipelineData {
    /// No active pipeline data
    None,
    /// Header-first sync data
    Headers {
        target_slot: u32,
        peer: PeerId,
        headers_count: usize,
    },
    /// Body download data
    Bodies { pending: usize, total: usize },
    /// Block processing data
    Processing { height: u64 },
    /// Snap sync: collecting state root votes
    SnapCollecting {
        target_hash: Hash,
        target_height: u64,
        votes: Vec<(PeerId, Hash, u64, Hash)>,
        asked: HashSet<PeerId>,
    },
    /// Snap sync: downloading snapshot from a peer
    SnapDownloading {
        target_hash: Hash,
        target_height: u64,
        quorum_root: Hash,
        peer: PeerId,
        alternate_peers: Vec<(PeerId, Hash, u64)>,
    },
    /// Snap sync: snapshot ready for node consumption
    SnapReady { snapshot: VerifiedSnapshot },
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
        matches!(self, SyncState::Syncing { .. })
    }
}

impl SyncPipelineData {
    /// Check if snap sync pipeline data is active (collecting or downloading)
    pub fn is_snap_syncing(&self) -> bool {
        matches!(
            self,
            SyncPipelineData::SnapCollecting { .. } | SyncPipelineData::SnapDownloading { .. }
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
/// Replaces independent booleans (`resync_in_progress`, `post_recovery_grace`,
/// `awaiting_canonical_block`) with a single enum. Illegal states (e.g., resync AND
/// post_recovery_grace simultaneously) become unrepresentable.
///
/// Lifecycle:
///   Normal → ResyncInProgress → PostRecoveryGrace → Normal
///   Normal → (snap sync) → AwaitingCanonicalBlock → Normal
#[derive(Clone, Debug)]
pub enum RecoveryPhase {
    /// No recovery in progress. Normal operation.
    Normal,
    /// Forced resync from genesis/snap is in progress.
    ResyncInProgress,
    /// Resync/recovery completed. Grace period suppresses reactivation.
    PostRecoveryGrace {
        /// When grace started.
        started: Instant,
        /// Blocks applied since recovery (cleared at 10).
        blocks_applied: u32,
    },
    /// Snap sync completed. Waiting for first canonical gossip block
    /// before allowing production. Carries a timestamp so cleanup() can
    /// enforce a 60s timeout (INC-I-005 Fix A).
    AwaitingCanonicalBlock {
        /// When this phase started (for timeout enforcement).
        started: Instant,
    },
}

/// Why a genesis resync was requested.
/// Used for logging, diagnostics, and potential future policy differentiation.
#[derive(Clone, Debug)]
pub enum RecoveryReason {
    /// All peers blacklisted with 20+ empty headers (cleanup.rs)
    AllPeersBlacklistedDeepFork,
    /// Stuck-sync with gap > 1000 blocks (cleanup.rs)
    StuckSyncLargeGap { gap: u64 },
    /// Height offset: stable gap while blocks applied (cleanup.rs)
    HeightOffsetDetected { gap: u64 },
    /// Genesis fallback: 10+ empty headers from peer (sync_engine.rs)
    GenesisFallbackEmptyHeaders,
    /// Body download peer error (sync_engine.rs)
    BodyDownloadPeerError,
    /// 3+ apply failures with snap threshold (block_lifecycle.rs)
    ApplyFailuresSnapThreshold { gap: u64 },
    /// Rollback death spiral exceeded max depth (Node rollback.rs)
    RollbackDeathSpiral { peak: u64, current: u64 },
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
    /// INC-I-012 F7: When snap sync last completed. Used to suppress peer
    /// blacklisting for empty headers within 5 minutes of snap sync — all
    /// canonical peers will return empty when local_hash is unrecognizable.
    pub last_snap_completed: Option<Instant>,
}

impl SnapSyncState {
    /// Create a new SnapSyncState with default values
    pub fn new() -> Self {
        Self {
            // INC-I-012: Snap sync re-enabled. Original incidents (INC-I-004, INC-I-005)
            // were caused by: (1) partition quorum (fixed: uses total peers as denominator),
            // (2) stale snapshot acceptance (fixed: 100-block height window), (3) cascading
            // resyncs (fixed: M2 recovery gate with rate limiting + floor checks).
            // Threshold 50 = snap sync activates when >50 blocks behind.
            // Header-first is still used for small gaps (≤50 blocks).
            // Use --no-snap-sync to disable if needed.
            threshold: 50,
            quorum: 3, // Minimum 3 peers for quorum (testnet-friendly, still partition-safe)
            root_timeout: Duration::from_secs(15),
            download_timeout: Duration::from_secs(60),
            blacklisted_peers: HashSet::new(),
            attempts: 0,
            fresh_node_wait_start: None,
            store_floor: 1, // Default: full-sync node has block 1
            last_snap_completed: None,
        }
    }
}

/// Fork detection and recovery state.
///
/// Groups all fork-related fields (counters, fork_recovery, blacklists,
/// cooldowns, circuit breakers) into a single sub-struct. Single point of truth for
/// "are we on a fork, and what are we doing about it?"
pub(crate) struct ForkState {
    /// Consecutive empty header responses (peers don't recognize our chain tip).
    /// When >= 10, we're on a deep fork and need genesis resync.
    /// Only incremented for truly EMPTY responses (peer doesn't have our hash).
    pub consecutive_empty_headers: u32,
    /// Consecutive sync failures (empty header responses)
    pub consecutive_sync_failures: u32,
    /// Consecutive block-apply failures (downloaded chain can't be applied).
    /// Unlike consecutive_empty_headers (reset by header downloads), this is
    /// ONLY reset by successful block_applied(). At >= 3, triggers genesis resync.
    pub consecutive_apply_failures: u32,
    /// Flag set when genesis fallback confirms peers reject our chain —
    /// node must call force_resync_from_genesis().
    pub needs_genesis_resync: bool,
    /// Fork recovery tracker — active parent chain download for automatic reorg.
    pub fork_recovery: super::fork_recovery::ForkRecoveryTracker,
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
    /// Stuck fork signal: set by cleanup() and block_apply_failed() when fork
    /// evidence is detected. Consumed by take_stuck_fork_signal() in resolve_shallow_fork().
    pub stuck_fork_signal: bool,
    /// INC-I-012 F1: Use height-based header request for the next sync cycle.
    /// Set after snap sync when local_hash is unrecognizable by peers. The next
    /// GetHeaders request uses GetHeadersByHeight(local_height) instead, which
    /// bypasses the hash lookup that causes the post-snap deadlock. Cleared
    /// after the first successful height-based response.
    pub use_height_based_headers: bool,
    /// INC-I-012 F1: Guard against re-entering height-based fallback.
    /// Set to true after the first height-based request is sent. If that
    /// request also returns empty, the post-snap fallback path skips the
    /// height-based retry and falls through to normal fork detection.
    /// Prevents a 60-second retry loop when the peer has no headers to serve.
    pub height_fallback_attempted: bool,
}

impl ForkState {
    /// Create a new ForkState with default values
    pub fn new() -> Self {
        Self {
            consecutive_empty_headers: 0,
            consecutive_sync_failures: 0,

            consecutive_apply_failures: 0,
            needs_genesis_resync: false,
            fork_recovery: super::fork_recovery::ForkRecoveryTracker::new(),
            header_blacklisted_peers: HashMap::new(),
            stable_gap_since: None,
            peak_height: 0,
            stuck_fork_signal: false,
            use_height_based_headers: false,
            height_fallback_attempted: false,
        }
    }

    /// Recommend a fork recovery action based on current state.
    /// The Node calls this from resolve_shallow_fork() and executes the returned action.
    ///
    /// Recovery levels (M1 redesign):
    /// 1. RollbackOne — shallow fork, up to max_rollback_depth blocks
    /// 2. NeedsGenesisResync — deep fork (10+ empty headers) or gap > rollback depth
    pub fn recommend_action(
        &self,
        gap: u64,
        consecutive_rollbacks: u32,
        max_rollback_depth: u32,
        _best_peer: Option<PeerId>,
    ) -> ForkAction {
        // Deep fork or exhausted rollbacks: need snap sync
        if self.consecutive_empty_headers >= 10 || self.needs_genesis_resync {
            return ForkAction::NeedsGenesisResync;
        }
        // Gap too large for rollback: escalate to snap sync
        if gap > max_rollback_depth as u64 {
            return ForkAction::NeedsGenesisResync;
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
pub enum ForkAction {
    /// No action needed
    None,
    /// Rollback one block (shallow fork, gap <= 12)
    RollbackOne,
    /// Node needs full genesis resync (deep fork or rollback exhausted)
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
    /// Pipeline data: operational data for the current sync phase.
    /// Previously embedded in SyncState variants, now separated for clarity.
    pub(crate) pipeline_data: SyncPipelineData,
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
    // === PRODUCTION GATE DEADLOCK FIX (PGD) FIELDS ===
    /// Hard cap on effective grace period after resync (seconds).
    /// Prevents exponential backoff from disabling producers for 480s+.
    /// Default: 60s (6 slots at 10s/slot).
    max_grace_cap_secs: u64,
    /// Blocks applied since the last resync completed.
    /// When this reaches 5, `reset_resync_counter()` is called to clear the
    /// exponential backoff. Only incremented when `resync_in_progress` is false.
    blocks_since_resync_completed: u32,
    // stuck_fork_signal is in ForkState

    // === INC-I-005 FIX C: MONOTONIC PROGRESS FLOOR ===
    /// Highest height at which the node was Synchronized and applied 10+
    /// blocks. Once set, `reset_local_state()` refuses to go below this
    /// floor — preventing the infinite snap sync death spiral.
    confirmed_height_floor: u64,
}

impl SyncManager {
    /// Create a new sync manager
    pub fn new(config: SyncConfig, genesis_hash: Hash) -> Self {
        Self {
            pipeline: SyncPipeline::new(&config),
            pipeline_data: SyncPipelineData::None,
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
            // PGD fix defaults
            max_grace_cap_secs: 60,
            blocks_since_resync_completed: 0,
            // INC-I-005 Fix C: monotonic progress floor
            confirmed_height_floor: 0,
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

    /// Check if snap sync is in progress (collecting roots or downloading snapshot)
    pub fn is_snap_syncing(&self) -> bool {
        self.pipeline_data.is_snap_syncing()
    }

    /// Get local chain tip
    pub fn local_tip(&self) -> (u64, Hash, u32) {
        (self.local_height, self.local_hash, self.local_slot)
    }

    /// Get the confirmed height floor (INC-I-005 Fix C).
    /// Once a node has been Synchronized and applied 10+ blocks at height H,
    /// this returns H. `reset_local_state()` refuses to go below this floor.
    pub fn confirmed_height_floor(&self) -> u64 {
        self.confirmed_height_floor
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
        if let SyncPipelineData::Headers {
            peer: sync_peer, ..
        } = &self.pipeline_data
        {
            if sync_peer == peer {
                self.set_state(SyncState::Idle, "header_peer_disconnected");
                self.pipeline_data = SyncPipelineData::None;
                if self.should_sync() {
                    self.start_sync();
                }
            }
        }

        // If snap downloading from this peer, try alternate or fall back
        if let SyncPipelineData::SnapDownloading {
            peer: snap_peer, ..
        } = &self.pipeline_data
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

    /// Transition to Syncing state with the given phase and pipeline data.
    /// Convenience method that sets both state and pipeline_data atomically.
    pub(crate) fn set_syncing(&mut self, phase: SyncPhase, data: SyncPipelineData, trigger: &str) {
        self.pipeline_data = data;
        let started_at = match &self.state {
            // Preserve existing started_at if already syncing (phase change within sync)
            SyncState::Syncing { started_at, .. } => *started_at,
            _ => Instant::now(),
        };
        self.set_state(SyncState::Syncing { phase, started_at }, trigger);
    }

    /// Transition to a new sync state with logging.
    /// All state transitions MUST go through this method for auditability.
    fn set_state(&mut self, new_state: SyncState, trigger: &str) {
        // Validate transition — hard-block invalid transitions (M3 enforcement)
        if !self.is_valid_transition(&new_state) {
            warn!(
                "[SYNC_STATE] INVALID transition {} -> {} BLOCKED (trigger: {})",
                self.state_label(),
                Self::label_for(&new_state),
                trigger
            );
            return; // Hard block: do NOT execute the transition
        }

        let old_label = self.state_label();
        // Clear pipeline data when leaving sync state
        if matches!(new_state, SyncState::Idle | SyncState::Synchronized) {
            self.pipeline_data = SyncPipelineData::None;
        }
        self.state = new_state;
        let new_label = self.state_label();
        if old_label != new_label {
            debug!(
                "[SYNC_STATE] {} -> {} (trigger: {})",
                old_label, new_label, trigger
            );
        }
    }

    /// Check if a state transition is valid.
    /// Called by set_state() to prevent illegal transitions.
    ///
    /// With 3 states, the valid transitions are:
    ///   Idle -> Syncing, Idle -> Synchronized, Idle -> Idle
    ///   Syncing -> Synchronized, Syncing -> Idle, Syncing -> Syncing
    ///   Synchronized -> Syncing, Synchronized -> Idle, Synchronized -> Synchronized
    /// All 9 combinations are valid — the 3-state model eliminates invalid transitions.
    fn is_valid_transition(&self, _new_state: &SyncState) -> bool {
        // With only 3 states, all transitions are valid:
        // - Idle is the reset state (anything -> Idle)
        // - Syncing covers all active sync phases (Idle/Synchronized -> Syncing)
        // - Synchronized is the terminal state (Syncing -> Synchronized)
        // - Self-transitions are always valid
        true
    }

    /// Short label for the current sync state (for logging)
    fn state_label(&self) -> &'static str {
        Self::label_for(&self.state)
    }

    /// Short label for any sync state (static version for use before state mutation)
    fn label_for(state: &SyncState) -> &'static str {
        match state {
            SyncState::Idle => "Idle",
            SyncState::Syncing { phase, .. } => match phase {
                SyncPhase::DownloadingHeaders => "Syncing:Headers",
                SyncPhase::DownloadingBodies => "Syncing:Bodies",
                SyncPhase::ProcessingBlocks => "Syncing:Processing",
                SyncPhase::SnapCollecting => "Syncing:SnapCollecting",
                SyncPhase::SnapDownloading => "Syncing:SnapDownloading",
            },
            SyncState::Synchronized => "Synchronized",
        }
    }

    /// Get the current sync phase (if syncing), or None.
    pub fn sync_phase(&self) -> Option<&SyncPhase> {
        if let SyncState::Syncing { phase, .. } = &self.state {
            Some(phase)
        } else {
            None
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
            SyncState::Syncing { .. } => {
                // Progress is derived from pipeline_data
                match &self.pipeline_data {
                    SyncPipelineData::Headers { target_slot, .. } => {
                        if *target_slot > 0 {
                            Some(self.local_slot as f64 / *target_slot as f64 * 100.0)
                        } else {
                            Some(0.0)
                        }
                    }
                    SyncPipelineData::Bodies { pending, total } => {
                        if *total > 0 {
                            Some((*total - *pending) as f64 / *total as f64 * 100.0)
                        } else {
                            Some(100.0)
                        }
                    }
                    SyncPipelineData::Processing { height } => {
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
                    SyncPipelineData::SnapCollecting { .. } => Some(5.0),
                    SyncPipelineData::SnapDownloading { .. } => Some(50.0),
                    SyncPipelineData::SnapReady { .. } => Some(95.0),
                    SyncPipelineData::None => Some(0.0),
                }
            }
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
