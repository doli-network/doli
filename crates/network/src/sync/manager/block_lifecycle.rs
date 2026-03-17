//! Block lifecycle — block application tracking, state reset, fork recovery/sync delegation

use std::time::{Duration, Instant};

use libp2p::PeerId;
use tracing::{info, warn};

use crypto::Hash;
use doli_core::Block;

use super::super::bodies::BodyDownloader;
use super::super::headers::HeaderDownloader;
use super::{SyncManager, SyncState};

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
        self.blocks_applied += 1;
        self.last_block_applied = Instant::now();
        self.last_sync_activity = Instant::now();
        self.body_stall_retries = 0;
        self.consecutive_apply_failures = 0;

        // Applying a block means the chain is advancing — reset fork counters.
        self.consecutive_empty_headers = 0;
        if matches!(self.recovery_phase, super::RecoveryPhase::StuckForkDetected) {
            self.recovery_phase = super::RecoveryPhase::Normal;
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
            if matches!(self.recovery_phase, super::RecoveryPhase::ResyncInProgress) {
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
                // Signal fork recovery via RecoveryPhase (replaces old stuck_fork_signal boolean).
                self.signal_stuck_fork();
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
        self.fork_recovery = super::super::fork_recovery::ForkRecoveryTracker::new();

        // Reset deep fork detection
        self.consecutive_empty_headers = 0;
        // Note: recovery_phase is already ResyncInProgress (set by start_resync above)
        self.needs_genesis_resync = false;
        self.body_stall_retries = 0;
        self.consecutive_apply_failures = 0;

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
        self.fork_recovery.start(orphan, peer)
    }

    /// Check if fork recovery chain connected to our block_store.
    /// Node calls this with the result of `block_store.has_block(current_parent)`.
    pub fn check_fork_recovery_connection(
        &mut self,
        parent_known: bool,
    ) -> Option<super::super::fork_recovery::CompletedRecovery> {
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
    pub fn reorg_handler(&self) -> &super::super::reorg::ReorgHandler {
        &self.reorg_handler
    }

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
        self.fork_sync = Some(super::super::fork_sync::ForkSync::new(
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
    pub fn fork_sync_handle_probe(&mut self, result: super::super::fork_sync::ProbeResult) {
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
    pub fn fork_sync_take_result(&mut self) -> Option<super::super::fork_sync::ForkSyncResult> {
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

    /// Reset sync state after a shallow fork rollback.
    /// Clears the fork signal counters, resets downloaders, and returns to Idle
    /// so sync can restart from the new (rolled-back) tip.
    /// Record that a fork sync was rejected (equal or lighter chain).
    /// Activates a cooldown to prevent infinite reorg loops.
    pub fn mark_fork_sync_rejected(&mut self) {
        self.last_fork_sync_rejection = Instant::now();
    }

    pub fn reset_sync_for_rollback(&mut self) {
        // NOTE: consecutive_empty_headers is NOT reset here. It must keep climbing
        // toward the escalation threshold (10) that triggers genesis resync via
        // is_deep_fork_detected(). Resetting after rejected reorgs prevents
        // escalation (counter oscillates 0→3→0 instead of reaching 10).
        // The counter is reset explicitly on SUCCESSFUL reorgs by the caller.
        //
        // needs_genesis_resync intentionally preserved — rollbacks must not
        // suppress the genesis resync signal set by the sync manager.
        self.consecutive_sync_failures = 0;
        self.fork_sync = None;
        self.state = SyncState::Idle;
        // NT10 fix: Signal that the next start_sync() should skip header-first sync.
        // After a fork rollback, our tip is still on the fork — header-first will
        // always get 0 headers because peers don't recognize our (rolled-back) tip.
        self.recovery_phase = super::RecoveryPhase::PostRollback;
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
}
