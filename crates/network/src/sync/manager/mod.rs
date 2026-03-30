//! Sync manager coordinating chain synchronization
//!
//! The SyncManager orchestrates the synchronization process:
//! 1. Discovers peers with higher chain tip
//! 2. Downloads headers first to validate VDF chain
//! 3. Downloads block bodies in parallel
//! 4. Applies blocks to local chain

mod block_lifecycle;
mod cleanup;
mod peers;
mod production_gate;
mod snap_sync;
mod sync_engine;
mod types;

#[cfg(test)]
mod tests;

// Re-export all public types from types.rs
pub use types::{
    ForkAction, ProductionAuthorization, RecoveryPhase, RecoveryReason, SyncConfig, SyncPhase,
    SyncPipelineData, SyncState, VerifiedSnapshot,
};
// Re-export pub(crate) types used by sibling modules
pub(crate) use types::{
    ForkState, NetworkState, PeerSyncStatus, PendingRequest, SnapSyncState, SyncPipeline,
    SyncRequestId,
};

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use libp2p::PeerId;
use tracing::{debug, info, warn};

use crypto::Hash;

use super::reorg::ReorgHandler;

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

    // === CHECKPOINT HEALTH: CANONICAL HASH RING BUFFER ===
    /// Recent (height, hash) pairs from our canonical chain. Used by
    /// `checkpoint_health()` to compare at the peer's reported height
    /// instead of our tip — tolerating normal status-protocol lag while
    /// still detecting real forks. Capped at 200 entries (~33 min at 10s slots).
    recent_canonical_hashes: VecDeque<(u64, Hash)>,
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
            // Checkpoint health: canonical hash ring buffer
            recent_canonical_hashes: VecDeque::with_capacity(200),
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

    // =========================================================================
    // STATE GETTERS
    // =========================================================================

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

    // =========================================================================
    // CHAIN TIP UPDATES
    // =========================================================================

    /// Update local chain tip
    pub fn update_local_tip(&mut self, height: u64, hash: Hash, slot: u32) {
        self.local_height = height;
        self.local_hash = hash;
        self.local_slot = slot;

        // Track canonical (height, hash) for checkpoint_health() comparisons.
        // On reorg, earlier heights may appear again — replace stale entries.
        while self
            .recent_canonical_hashes
            .back()
            .is_some_and(|(h, _)| *h >= height)
        {
            self.recent_canonical_hashes.pop_back();
        }
        self.recent_canonical_hashes.push_back((height, hash));
        if self.recent_canonical_hashes.len() > 200 {
            self.recent_canonical_hashes.pop_front();
        }

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
    // STATE TRANSITIONS
    // =========================================================================

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

    // =========================================================================
    // DIAGNOSTICS
    // =========================================================================

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

    /// Pipeline memory stats: (pending_headers, pending_blocks, active_requests)
    pub fn pipeline_stats(&self) -> (usize, usize, usize) {
        (
            self.pipeline.pending_headers.len(),
            self.pipeline.pending_blocks.len(),
            self.pipeline.pending_requests.len(),
        )
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
