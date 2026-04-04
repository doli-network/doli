//! Types for the sync manager — structs, enums, and their inherent impls.
//!
//! Everything except the `SyncManager` struct itself lives here.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use libp2p::PeerId;

use crypto::Hash;
use doli_core::BlockHeader;

use super::super::bodies::BodyDownloader;
use super::super::fork_recovery::ForkRecoveryTracker;
use super::super::headers::HeaderDownloader;
use crate::protocols::SyncRequest;

// ---------------------------------------------------------------------------
// SyncConfig
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// SyncState / SyncPhase
// ---------------------------------------------------------------------------

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

impl SyncState {
    /// Check if we're actively syncing
    pub fn is_syncing(&self) -> bool {
        matches!(self, SyncState::Syncing { .. })
    }
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

// ---------------------------------------------------------------------------
// SyncPipelineData / VerifiedSnapshot
// ---------------------------------------------------------------------------

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

impl SyncPipelineData {
    /// Check if snap sync pipeline data is active (collecting or downloading)
    pub fn is_snap_syncing(&self) -> bool {
        matches!(
            self,
            SyncPipelineData::SnapCollecting { .. } | SyncPipelineData::SnapDownloading { .. }
        )
    }
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

// ---------------------------------------------------------------------------
// ProductionAuthorization
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// PeerSyncStatus / SyncRequestId / PendingRequest
// ---------------------------------------------------------------------------

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
    pub(super) fn new(id: u64) -> Self {
        Self(id)
    }

    /// Get the inner request ID value (for logging)
    pub(super) fn inner(&self) -> u64 {
        self.0
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

// ---------------------------------------------------------------------------
// SyncPipeline
// ---------------------------------------------------------------------------

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
    pub pending_blocks: HashMap<Hash, doli_core::Block>,
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

// ---------------------------------------------------------------------------
// RecoveryPhase / RecoveryReason
// ---------------------------------------------------------------------------

/// Recovery phase of the sync manager.
///
/// Replaces independent booleans (`resync_in_progress`, `post_recovery_grace`,
/// `awaiting_canonical_block`) with a single enum. Illegal states (e.g., resync AND
/// post_recovery_grace simultaneously) become unrepresentable.
///
/// Lifecycle:
///   Normal -> ResyncInProgress -> PostRecoveryGrace -> Normal
///   Normal -> (snap sync) -> AwaitingCanonicalBlock -> Normal
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

// ---------------------------------------------------------------------------
// NetworkState
// ---------------------------------------------------------------------------

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
    /// When network_tip_height first became unsupported by any connected peer.
    /// After 42s (~4 slots) without a peer within 5 blocks of the tip, the tip decays
    /// to the actual peer max — preventing permanent phantom sync gaps.
    pub tip_unsupported_since: Option<Instant>,
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
            tip_unsupported_since: None,
        }
    }
}

// ---------------------------------------------------------------------------
// SnapSyncState
// ---------------------------------------------------------------------------

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
    /// Discv5 peer discovery grace: when set, the sync engine waits up to
    /// this deadline for discv5 to discover enough peers before falling back
    /// to header-first sync. Prevents the 8/10 → 10/10 gap where the sync
    /// engine decides "not enough peers" before discv5 completes its first walk.
    pub discv5_peer_grace_deadline: Option<Instant>,
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
            // Header-first is still used for small gaps (<=50 blocks).
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
            discv5_peer_grace_deadline: None,
        }
    }
}

// ---------------------------------------------------------------------------
// ForkState / ForkAction
// ---------------------------------------------------------------------------

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
    pub fork_recovery: ForkRecoveryTracker,
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
            fork_recovery: ForkRecoveryTracker::new(),
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
