//! Block lifecycle — block application tracking, state reset, fork recovery/sync delegation

use std::time::{Duration, Instant};

use libp2p::PeerId;
use tracing::{info, warn};

use crypto::Hash;
use doli_core::Block;

use super::super::bodies::BodyDownloader;
use super::super::headers::HeaderDownloader;
use super::{ForkAction, SyncManager, SyncState};

impl SyncManager {
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
        self.network.blocks_applied += 1;
        self.network.last_block_applied = Instant::now();
        self.network.last_sync_activity = Instant::now();
        self.pipeline.body_stall_retries = 0;
        self.fork.consecutive_apply_failures = 0;

        // Track peak height for rollback death spiral prevention.
        if height > self.fork.peak_height {
            self.fork.peak_height = height;
        }

        // Applying a block means the chain is advancing — reset fork counters.
        self.fork.consecutive_empty_headers = 0;

        // INC-I-005 Fix C: Update confirmed height floor.
        // Once a node is Synchronized and has applied 10+ blocks, set the floor.
        // This prevents reset_local_state() from going below this height,
        // breaking the infinite snap sync death spiral.
        if matches!(self.state, SyncState::Synchronized) && height > self.confirmed_height_floor {
            // We're Synchronized and at a new high. After the PGD-001 block
            // (5 blocks after resync counter reset) we're confident the node is
            // healthy. Use the same threshold (resync counter cleared) as proxy.
            if self.consecutive_resync_count == 0 {
                self.confirmed_height_floor = height;
            }
        }

        // PGD-001: Track stable blocks after resync completion.
        if !matches!(self.recovery_phase, super::RecoveryPhase::ResyncInProgress)
            && self.consecutive_resync_count > 0
        {
            self.blocks_since_resync_completed += 1;
            if self.blocks_since_resync_completed >= 5 {
                info!(
                    "Resync counter reset: {} stable blocks applied after resync — \
                     clearing consecutive_resync_count (was {})",
                    self.blocks_since_resync_completed, self.consecutive_resync_count
                );
                self.reset_resync_counter();
                self.blocks_since_resync_completed = 0;
            }
        }

        // Post-recovery grace: clear after 10 blocks applied since recovery.
        if let super::RecoveryPhase::PostRecoveryGrace {
            ref mut blocks_applied,
            ..
        } = self.recovery_phase
        {
            *blocks_applied += 1;
            if *blocks_applied >= 10 {
                info!(
                    "Post-recovery grace cleared: {} blocks applied since recovery.",
                    blocks_applied
                );
                self.recovery_phase = super::RecoveryPhase::Normal;
            }
        }
        // Also clear grace by timeout (handled in cleanup(), not here — this
        // path only runs when blocks are applied, which doesn't happen on a fork).

        // Also update network tip - if we applied a block, the network has at least this height/slot
        // This helps the "behind peers" check work correctly even when peer status is stale
        self.update_network_tip_height(height);
        if slot > self.network.network_tip_slot {
            self.network.network_tip_slot = slot;
        }

        // Log progress periodically
        if self.network.last_progress_log.elapsed() > Duration::from_secs(5) {
            info!(
                "Sync progress: height {}, {} blocks/sec",
                height,
                self.network.blocks_applied / 5
            );
            self.network.blocks_applied = 0;
            self.network.last_progress_log = Instant::now();
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
        let slot_ok = self.network.network_tip_slot.saturating_sub(slot) <= self.max_slots_behind;
        if self.state.is_syncing() && height >= self.network.network_tip_height && slot_ok {
            info!(
                "Sync complete: transitioning to Synchronized at height {} slot {} (network_tip_h={} network_tip_s={})",
                height, slot, self.network.network_tip_height, self.network.network_tip_slot
            );
            self.set_state(SyncState::Synchronized, "sync_complete_block_applied");
            self.snap.attempts = 0;
            self.fork.header_blacklisted_peers.clear();

            // If we were in a resync, complete it now
            if matches!(self.recovery_phase, super::RecoveryPhase::ResyncInProgress) {
                self.complete_resync();
                info!(
                    "Resync complete at height {} - grace period started ({}s)",
                    height, self.resync_grace_period_secs
                );
            }
        } else if matches!(
            self.pipeline_data,
            super::SyncPipelineData::Processing { .. }
        ) && self.pipeline.pending_requests.is_empty()
        {
            // No active network requests in Processing state. Either:
            // - All blocks applied but network moved ahead, OR
            // - Blocks downloaded but unchainable (hash mismatch)
            // Either way, nothing will change — clear stale data and restart.
            info!(
                "Processing complete (headers={}, blocks={}, local h={} s={}, tip h={} s={}) — starting new sync round",
                self.pipeline.pending_headers.len(), self.pipeline.pending_blocks.len(),
                height, slot, self.network.network_tip_height, self.network.network_tip_slot
            );
            self.pipeline.pending_headers.clear();
            self.pipeline.pending_blocks.clear();
            self.set_state(SyncState::Idle, "processing_complete_restart");
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
        self.fork.consecutive_apply_failures += 1;
        warn!(
            "Block apply failure #{} — clearing pending sync state",
            self.fork.consecutive_apply_failures
        );

        // Clear everything — don't salvage blocks from a bad chain
        self.pipeline.pending_headers.clear();
        self.pipeline.pending_blocks.clear();
        self.pipeline.headers_needing_bodies.clear();
        self.pipeline.header_downloader.clear();
        self.pipeline.body_downloader.clear();
        self.pipeline.body_stall_retries = 0;
        self.set_state(SyncState::Idle, "block_apply_failed");
        self.network.last_sync_activity = Instant::now();

        if self.fork.consecutive_apply_failures >= 3 {
            let gap = self
                .network
                .network_tip_height
                .saturating_sub(self.local_height);
            if gap <= 50 {
                // Small gap + repeated apply failures = fork, not transient issue.
                // Gossip can't resolve this — the node is on a different chain and
                // keeps trying to apply canonical blocks at the wrong height.
                // Signal stuck fork so resolve_shallow_fork() can rollback and
                // find the divergence point.
                warn!(
                    "3+ consecutive apply failures with gap={} — \
                     signaling stuck fork for rollback-based recovery",
                    gap
                );
                self.fork.consecutive_apply_failures = 0;
                // Signal fork recovery via RecoveryPhase (replaces old stuck_fork_signal boolean).
                self.signal_stuck_fork();
                self.set_state(SyncState::Idle, "apply_failures_fork_detected");
            } else {
                // INC-I-005 Fix: Force snap sync via request_genesis_resync() instead of
                // direct flag write. The gated method enforces monotonic floor, concurrent
                // recovery check, rate limiting, and snap availability.
                // Original split: snap available → force snap; else → raw resync.
                // After M2: both paths use the same gated method with ApplyFailuresSnapThreshold.
                warn!(
                    "3+ consecutive apply failures with gap={} — requesting genesis resync \
                     (snap_threshold={}, snap_attempts={}, state divergence confirmed)",
                    gap, self.snap.threshold, self.snap.attempts
                );
                self.fork.consecutive_apply_failures = 0;
                self.request_genesis_resync(super::RecoveryReason::ApplyFailuresSnapThreshold {
                    gap,
                });
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
        // INC-I-005 Fix C: Refuse to reset below confirmed height floor.
        // If the node was previously healthy (Synchronized + applied blocks),
        // resetting to 0 creates an infinite snap sync death spiral.
        // Instead, log the refusal and let the node try header-first sync
        // from its current position. Manual intervention required for deeper issues.
        if self.confirmed_height_floor > 0 {
            warn!(
                "reset_local_state REFUSED: confirmed_height_floor={}, will not reset below. \
                 Node was previously healthy at this height. Manual intervention required \
                 for recovery below this point (INC-I-005 Fix C).",
                self.confirmed_height_floor
            );
            // Still block production and clear stale state, but don't reset height
            self.start_resync();
            self.pipeline.pending_headers.clear();
            self.pipeline.pending_blocks.clear();
            self.pipeline.headers_needing_bodies.clear();
            self.pipeline.pending_requests.clear();
            self.fork.fork_recovery = super::super::fork_recovery::ForkRecoveryTracker::new();
            self.fork.consecutive_empty_headers = 0;
            self.fork.needs_genesis_resync = false;
            self.pipeline.body_stall_retries = 0;
            self.fork.consecutive_apply_failures = 0;
            self.snap.attempts = 0;
            self.network.last_block_seen = Instant::now();
            self.network.last_block_applied = Instant::now();
            self.network.last_sync_activity = Instant::now();
            self.last_block_received_via_gossip = Some(Instant::now());
            self.pipeline.header_downloader = super::super::headers::HeaderDownloader::new(
                self.config.max_headers_per_request,
                self.config.request_timeout,
            );
            self.pipeline.body_downloader = super::super::bodies::BodyDownloader::new(
                self.config.max_bodies_per_request,
                self.config.max_concurrent_body_requests,
                self.config.request_timeout,
            );
            self.set_state(SyncState::Idle, "reset_refused_height_floor");
            if self.should_sync() {
                self.start_sync();
            }
            return;
        }

        // CRITICAL: Block production FIRST via the production gate
        self.start_resync();

        // Reset chain tip to genesis
        self.local_height = 0;
        self.local_hash = genesis_hash;
        self.local_slot = 0;
        self.network.blocks_applied = 0;
        self.set_state(SyncState::Idle, "reset_local_state");
        self.reorg_handler.clear();

        // Reset gossip tracking for fresh start
        self.last_block_received_via_gossip = Some(Instant::now());

        // Reset network tip to 0 — stale tip from pre-fork state distorts
        // the "behind peers" check (Layer 6) after resync, causing the node
        // at height 0 to think it's hundreds of blocks behind a ghost tip.
        self.network.network_tip_height = 0;
        self.network.network_tip_slot = 0;

        // Clear all pending sync state to start fresh
        self.pipeline.pending_headers.clear();
        self.pipeline.pending_blocks.clear();
        self.pipeline.headers_needing_bodies.clear();
        self.pipeline.pending_requests.clear();

        // Cancel any active fork recovery
        self.fork.fork_recovery = super::super::fork_recovery::ForkRecoveryTracker::new();

        // Reset deep fork detection
        self.fork.consecutive_empty_headers = 0;
        // Note: recovery_phase is already ResyncInProgress (set by start_resync above)
        self.fork.needs_genesis_resync = false;
        self.pipeline.body_stall_retries = 0;
        self.fork.consecutive_apply_failures = 0;

        // Reset snap sync attempt counter so recovery gets fresh tries.
        self.snap.attempts = 0;

        // Reset stale chain timers
        self.network.last_block_seen = Instant::now();
        self.network.last_block_applied = Instant::now();
        self.network.last_sync_activity = Instant::now();

        // Reset downloaders
        self.pipeline.header_downloader = HeaderDownloader::new(
            self.config.max_headers_per_request,
            self.config.request_timeout,
        );
        self.pipeline.body_downloader = BodyDownloader::new(
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
    ) -> Option<super::super::reorg::ReorgResult> {
        self.reorg_handler
            .check_reorg_weighted(&block, self.local_hash, producer_weight)
    }

    // =========================================================================
    // FORK RECOVERY — Active parent chain download
    // =========================================================================

    /// Start fork recovery for an orphan block.
    /// Walks backward through parent chain requesting blocks from the peer.
    pub fn start_fork_recovery(&mut self, orphan: doli_core::Block, peer: PeerId) -> bool {
        self.fork.fork_recovery.start(orphan, peer)
    }

    /// Check if fork recovery chain connected to our block_store.
    /// Node calls this with the result of `block_store.has_block(current_parent)`.
    pub fn check_fork_recovery_connection(
        &mut self,
        parent_known: bool,
    ) -> Option<super::super::fork_recovery::CompletedRecovery> {
        self.fork.fork_recovery.check_connection(parent_known)
    }

    /// Get the parent hash the recovery is currently seeking.
    /// Returns None if no recovery active or waiting for response.
    pub fn fork_recovery_current_parent(&self) -> Option<Hash> {
        self.fork.fork_recovery.current_parent()
    }

    /// Is fork recovery currently active?
    pub fn is_fork_recovery_active(&self) -> bool {
        self.fork.fork_recovery.is_active()
    }

    /// Can a new fork recovery start? (not active and not on cooldown)
    pub fn can_start_fork_recovery(&self) -> bool {
        self.fork.fork_recovery.can_start()
    }

    /// Check and consume the fork-recovery exceeded-max-depth flag.
    /// When true, the fork is deeper than MAX_RECOVERY_DEPTH and the node
    /// should escalate to force_resync_from_genesis().
    pub fn take_fork_exceeded_max_depth(&mut self) -> bool {
        self.fork.fork_recovery.take_exceeded_max_depth()
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
    pub fn reorg_handler(&self) -> &super::super::reorg::ReorgHandler {
        &self.reorg_handler
    }

    pub fn disable_snap_sync(&mut self) {
        self.snap.threshold = u64::MAX;
    }

    pub fn set_store_floor(&mut self, floor: u64) {
        self.snap.store_floor = floor;
    }

    /// Get the lowest block height available in the local block store.
    pub fn store_floor(&self) -> u64 {
        self.snap.store_floor
    }

    /// Recommend a fork recovery action.
    /// Delegates to ForkState::recommend_action() — centralizes fork decision logic.
    /// Called by Node::resolve_shallow_fork() to decide the recovery strategy.
    pub fn recommend_fork_action(
        &self,
        gap: u64,
        consecutive_rollbacks: u32,
        max_rollback_depth: u32,
        best_peer: Option<PeerId>,
    ) -> ForkAction {
        self.fork
            .recommend_action(gap, consecutive_rollbacks, max_rollback_depth, best_peer)
    }

    /// Get the peak height ever reached by this node.
    /// Used by reorg validation to prevent accepting reorgs
    /// that roll back too far from the peak.
    pub fn peak_height(&self) -> u64 {
        self.fork.peak_height
    }

    pub fn reset_sync_for_rollback(&mut self) {
        // Gate: monotonic progress floor (REQ-SYNC-102)
        if self.local_height > 0 && self.local_height <= self.confirmed_height_floor {
            warn!(
                "[RECOVERY] reset_sync_for_rollback REFUSED: height {} at or below floor {}",
                self.local_height, self.confirmed_height_floor
            );
            return;
        }

        // NOTE: consecutive_empty_headers is NOT reset here. It must keep climbing
        // toward the escalation threshold (10) that triggers genesis resync via
        // is_deep_fork_detected(). Resetting after rejected reorgs prevents
        // escalation (counter oscillates 0→3→0 instead of reaching 10).
        // The counter is reset explicitly on SUCCESSFUL reorgs by the caller.
        //
        // needs_genesis_resync intentionally preserved — rollbacks must not
        // suppress the genesis resync signal set by the sync manager.
        self.fork.consecutive_sync_failures = 0;
        self.set_state(SyncState::Idle, "reset_sync_for_rollback");
        // After a fork rollback, set Normal — header-first sync will
        // retry from the new (rolled-back) tip. If peers don't recognize it,
        // the empty-headers counter will escalate to genesis resync.
        self.recovery_phase = super::RecoveryPhase::Normal;
        // Clear stale weight history to prevent minority fork weights from
        // contaminating future fork choice decisions after a resync.
        self.reorg_handler.clear();
        self.reset_sync_buffers();
    }

    /// Reset sync state after a SUCCESSFUL reorg.
    ///
    /// Sets `recovery_phase = Normal` because after a successful reorg,
    /// our tip IS on the canonical chain — peers WILL recognize our tip hash,
    /// so header-first sync works normally.
    pub fn reset_sync_after_successful_reorg(&mut self) {
        self.fork.consecutive_sync_failures = 0;
        self.set_state(SyncState::Idle, "successful_reorg_reset");
        self.recovery_phase = super::RecoveryPhase::Normal;
        // Preserve reorg_handler weights — successful reorg weights are valid
        // context for future fork choice decisions. Only clear on rejection.
        self.reset_sync_buffers();
    }

    /// Reset sync buffers (shared between rollback and success paths).
    fn reset_sync_buffers(&mut self) {
        self.pipeline.pending_headers.clear();
        self.pipeline.pending_blocks.clear();
        self.pipeline.headers_needing_bodies.clear();
        self.pipeline.pending_requests.clear();
        self.pipeline.header_downloader = HeaderDownloader::new(
            self.config.max_headers_per_request,
            self.config.request_timeout,
        );
        self.pipeline.body_downloader = BodyDownloader::new(
            self.config.max_bodies_per_request,
            self.config.max_concurrent_body_requests,
            self.config.request_timeout,
        );
        self.network.last_block_seen = Instant::now();
        self.network.last_block_applied = Instant::now();
        self.network.last_sync_activity = Instant::now();
    }
}
