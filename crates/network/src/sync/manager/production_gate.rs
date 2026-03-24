//! Production gate — single source of truth for block production authorization
//!
//! 3 safety checks must ALL pass before block production is authorized.

use std::time::Instant;

use tracing::{debug, info, warn};

use super::{
    ProductionAuthorization, SyncManager, MIN_PEERS_TIER1, MIN_PEERS_TIER2, MIN_PEERS_TIER3,
};

impl SyncManager {
    // =========================================================================
    // PRODUCTION GATE - Single source of truth for block production authorization
    // =========================================================================

    /// Update fork detection and gossip state. Call BEFORE can_produce().
    ///
    /// Side effects extracted from can_produce() to make it a pure query.
    /// This fixes a class of race-like bugs where a "read" operation (can_produce)
    /// mutated state, causing the system to behave differently depending on how
    /// often production was checked.
    /// Pre-production state update. Currently a no-op after M2 simplification —
    /// can_produce() does its own hash comparison inline. Kept as a hook for
    /// callers that still call it before can_produce().
    pub fn update_production_state(&mut self) {
        // M2: All side effects removed. can_produce() now checks hash agreement inline each call.
    }

    /// Check if block production is authorized.
    ///
    /// M2 redesign: 3 checks replace the former 11-layer defense-in-depth.
    /// The old layers were defenses against problems created by the recovery system
    /// itself — most were removed along with fork_sync in M1.
    ///
    /// Checks:
    /// 1. Syncing — block during active sync or recovery
    /// 2. Minimum peers — prevent echo chambers and genesis forks
    /// 3. Finality — don't produce below finalized height
    ///
    /// Hash agreement was removed (INC-I-008): in large networks, non-producing
    /// peers outvote producers on tip hash, deadlocking the chain. Tip forks
    /// resolve via heaviest-chain fork choice; deep forks via is_deep_fork_detected().
    pub fn can_produce(&mut self, current_slot: u32) -> ProductionAuthorization {
        let best_peer_h = self.best_peer_height();
        info!(
            "[CAN_PRODUCE] slot={} local_h={} peers={} state={:?}",
            current_slot,
            self.local_height,
            self.peers.len(),
            self.state
        );

        // Safety override: explicit production block (invariant violations, manual)
        if let Some(ref reason) = self.production_blocked {
            return ProductionAuthorization::BlockedExplicit {
                reason: reason.clone(),
            };
        }

        // Check 1: Block during active sync or recovery
        if self.state.is_syncing()
            || matches!(self.recovery_phase, super::RecoveryPhase::ResyncInProgress)
        {
            return ProductionAuthorization::BlockedSyncing;
        }

        // Check 1b: Post-snap-sync — wait for first canonical gossip block
        // After snap sync the block store is empty. Producing immediately creates a
        // fork because there's no real parent block to build on.
        if matches!(
            self.recovery_phase,
            super::RecoveryPhase::AwaitingCanonicalBlock { .. }
        ) {
            return ProductionAuthorization::BlockedAwaitingCanonicalBlock;
        }

        // Check 2: Minimum peers (with bootstrap/genesis logic)
        //
        // At genesis (h=0): require chain evidence or peer quorum to prevent
        // competing genesis chains from late-joining nodes (INC-I-005).
        if self.is_in_bootstrap_phase() && self.first_peer_status_received.is_some() {
            // Lost all peers at h>0: wait for reconnection (with timeout for chain continuity)
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
            }

            // At h=0: require chain activity evidence or quorum
            let has_chain_activity = self.network.network_tip_slot > 0
                || self.network.network_tip_height > 0
                || best_peer_h > 0;

            if !has_chain_activity {
                if let Some(first_status) = self.first_peer_status_received {
                    let elapsed = first_status.elapsed().as_secs();
                    if elapsed < self.bootstrap_grace_period_secs {
                        return ProductionAuthorization::BlockedBootstrap {
                            reason: format!(
                                "Waiting for chain evidence ({}s/{}s)",
                                elapsed, self.bootstrap_grace_period_secs
                            ),
                        };
                    }
                    let min_genesis_quorum = self.min_peers_for_production.max(5);
                    if self.peers.len() < min_genesis_quorum {
                        return ProductionAuthorization::BlockedBootstrap {
                            reason: format!(
                                "Genesis quorum: {}/{} peers",
                                self.peers.len(),
                                min_genesis_quorum
                            ),
                        };
                    }
                }
            }
        }

        // Minimum peer count — echo chamber prevention (INC-001)
        let genesis_bypass = self.local_height == 0 && self.min_peers_for_production <= 1;
        let peer_loss_bypass = self
            .peers_lost_at
            .map(|t| t.elapsed().as_secs() >= self.peer_loss_timeout_secs)
            .unwrap_or(false)
            && self.local_height > 0;
        if self.peers.len() < self.min_peers_for_production && !genesis_bypass && !peer_loss_bypass
        {
            return ProductionAuthorization::BlockedInsufficientPeers {
                peer_count: self.peers.len(),
                min_required: self.min_peers_for_production,
            };
        }

        // Check 3 (hash agreement) REMOVED — INC-I-008
        //
        // The old check compared our tip hash against all peers at the same height
        // and blocked production when we were in the "minority". In a network with
        // many non-producing relay/stress nodes, a single producer on a 1-block
        // tip fork could outvote the other producers (70 stress nodes vs 4 producers),
        // deadlocking the chain.
        //
        // Tip-level forks are NORMAL during propagation and resolve naturally via
        // heaviest-chain fork choice + reorg. Deep forks are detected separately
        // by is_deep_fork_detected(). No production gate check is needed.

        // Check 4: Finality
        if let Some(finalized_height) = self.last_finalized_height() {
            if self.local_height < finalized_height {
                return ProductionAuthorization::BlockedConflictsFinality {
                    local_finalized_height: finalized_height,
                };
            }
        }

        info!("[CAN_PRODUCE] AUTHORIZED");
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
        self.production_blocked = Some(reason.to_string());
    }

    /// Clear explicit production block
    pub fn unblock_production(&mut self) {
        if self.production_blocked.is_some() {
            info!("Production unblocked");
            self.production_blocked = None;
        }
    }

    /// Signal that a forced resync is starting
    ///
    /// This blocks production until the resync completes and grace period expires.
    pub fn start_resync(&mut self) {
        info!("Resync started - production blocked");
        self.recovery_phase = super::RecoveryPhase::ResyncInProgress;
        self.consecutive_resync_count += 1;
        self.blocks_since_resync_completed = 0; // PGD-001: reset stable block counter

        // Log exponential backoff info (PGD-002: capped at max_grace_cap_secs)
        if self.consecutive_resync_count > 1 {
            let uncapped =
                self.resync_grace_period_secs * (1 << (self.consecutive_resync_count - 1).min(4));
            let effective_grace = uncapped.min(self.max_grace_cap_secs);
            warn!(
                "Consecutive resync #{} - grace period {}s (capped from {}s)",
                self.consecutive_resync_count, effective_grace, uncapped
            );
        }
    }

    /// Signal that a forced resync has completed
    ///
    /// Starts the grace period timer before production can resume.
    pub fn complete_resync(&mut self) {
        info!("Resync completed - starting grace period");
        self.recovery_phase = super::RecoveryPhase::Normal;
        self.last_resync_completed = Some(Instant::now());
    }

    /// Clear the post-snap-sync production gate.
    /// Called when a canonical gossip block has been successfully applied,
    /// proving we're on the canonical chain.
    pub fn clear_awaiting_canonical_block(&mut self) {
        if matches!(
            self.recovery_phase,
            super::RecoveryPhase::AwaitingCanonicalBlock { .. }
        ) {
            info!("[SNAP_SYNC] Canonical gossip block received — production gate cleared");
            self.recovery_phase = super::RecoveryPhase::Normal;
        }
    }

    /// Check if we're waiting for a canonical block after snap sync.
    pub fn is_awaiting_canonical_block(&self) -> bool {
        matches!(
            self.recovery_phase,
            super::RecoveryPhase::AwaitingCanonicalBlock { .. }
        )
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
        matches!(self.recovery_phase, super::RecoveryPhase::ResyncInProgress)
    }

    /// Get the current consecutive resync count
    pub fn consecutive_resync_count(&self) -> u32 {
        self.consecutive_resync_count
    }

    /// Get blocks applied since last reset (indicates active sync progress)
    pub fn blocks_applied(&self) -> u64 {
        self.network.blocks_applied
    }

    /// Signal that we have connected to at least one peer
    ///
    /// Bootstrap gate is now driven by `first_peer_status_received` (set via
    /// `note_peer_status_received()`). This method is kept for callers that
    /// signal peer connection before status exchange completes.
    pub fn set_peer_connected(&mut self) {
        if self.first_peer_status_received.is_none() {
            debug!("First peer connection noted - awaiting peer status for bootstrap gate");
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
        // first_peer_status_received tracks both connection and status in one field:
        // None = no peer has sent status yet (standalone mode, OK to produce)
        // Some(_) = at least one peer status received (bootstrap can proceed)
        //
        // When None and peers.is_empty(), we're standalone — safe to produce.
        // When None and peers exist, we're waiting for status — handled by bootstrap gate.
        self.first_peer_status_received.is_some() || self.peers.is_empty()
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

        // Secondary: had peer status but lost all peers
        // This could indicate network partition or need to resync
        if self.first_peer_status_received.is_some() && self.peers.is_empty() {
            return true;
        }

        false
    }

    /// Configure the production gate settings
    pub fn configure_production_gate(&mut self, grace_period_secs: u64, max_slots_behind: u32) {
        self.resync_grace_period_secs = grace_period_secs;
        self.max_slots_behind = max_slots_behind;
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

    // =========================================================================
    // FINALITY TRACKING
    // =========================================================================

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

    // =========================================================================
    // DIAGNOSTICS
    // =========================================================================

    /// Check if sync failures indicate we're on a fork (no-op, kept for API compatibility)
    pub fn has_sync_failure_fork_indicator(&self) -> bool {
        false
    }

    /// Network tip height (best seen via gossip or peer status)
    pub fn network_tip_height(&self) -> u64 {
        self.network.network_tip_height
    }

    /// Network tip slot (best seen via gossip or peer status)
    pub fn network_tip_slot(&self) -> u32 {
        self.network.network_tip_slot
    }

    /// Get consecutive sync failure count (for health diagnostics)
    pub fn consecutive_sync_failure_count(&self) -> u32 {
        self.fork.consecutive_sync_failures
    }

    /// Get consecutive empty header response count (for shallow fork detection)
    pub fn consecutive_empty_headers(&self) -> u32 {
        self.fork.consecutive_empty_headers
    }

    /// Reset empty headers counter after a rollback changes the local tip.
    /// The next sync attempt will use the new tip hash.
    pub fn reset_empty_headers(&mut self) {
        self.fork.consecutive_empty_headers = 0;
    }

    /// Check if post-recovery grace period is active.
    /// During grace, the node needs time to sync via header-first / gossip
    /// before fork detection is meaningful.
    pub fn post_recovery_grace_active(&self) -> bool {
        matches!(
            self.recovery_phase,
            super::RecoveryPhase::PostRecoveryGrace { .. }
        )
    }

    /// Check if a stuck-fork signal was raised by cleanup or apply-failure detection.
    /// Reads and clears the signal.
    pub fn take_stuck_fork_signal(&mut self) -> bool {
        let was_signaled = self.fork.stuck_fork_signal;
        self.fork.stuck_fork_signal = false;
        was_signaled
    }

    /// Signal a stuck fork. Sets a flag that resolve_shallow_fork() consumes.
    /// Only sets the flag when in Normal phase — other phases have higher priority.
    pub fn signal_stuck_fork(&mut self) {
        match self.recovery_phase {
            super::RecoveryPhase::Normal => {
                self.fork.stuck_fork_signal = true;
            }
            _ => {
                // Don't override active resync, post-recovery grace, or snap sync
                debug!(
                    "Stuck fork signal ignored — recovery phase {:?} has priority",
                    self.recovery_phase
                );
            }
        }
    }

    /// Activate post-recovery grace period. Called after snap sync / forced recovery.
    pub fn set_post_recovery_grace(&mut self) {
        self.recovery_phase = super::RecoveryPhase::PostRecoveryGrace {
            started: Instant::now(),
            blocks_applied: 0,
        };
        self.fork.consecutive_empty_headers = 0;
        self.fork.consecutive_apply_failures = 0;
        info!("Post-recovery grace activated: recovery suppressed until 10 blocks applied or 120s timeout.");
    }

    /// Check if sync manager has signaled that a full genesis resync is needed.
    /// Returns false if snap sync is disabled (--no-snap-sync), regardless of
    /// how many internal paths set the flag. This is the SINGLE gate that
    /// prevents snap sync from firing when the operator has forbidden it.
    pub fn needs_genesis_resync(&self) -> bool {
        if self.snap.threshold == u64::MAX {
            // --no-snap-sync: allow the genesis resync signal through.
            // The recovery path (reset_state_only) preserves block data — it only
            // resets UTXO, ProducerSet, and ChainState to genesis, then header-first
            // sync rebuilds state from preserved blocks. No snap sync is needed.
            // Previously this hardcoded false, creating a permanent deadlock for
            // forked --no-snap-sync nodes with no recovery path.
            if self.fork.needs_genesis_resync {
                tracing::warn!(
                    "--no-snap-sync: genesis resync signal active (local_h={}, gap={}). \
                     Recovery will use header-first full resync (block data preserved).",
                    self.local_height,
                    self.best_peer_height().saturating_sub(self.local_height)
                );
            }
        }
        self.fork.needs_genesis_resync
    }

    /// Central gate for all genesis resync requests.
    ///
    /// Replaces 9 scattered `needs_genesis_resync = true` assignments with a single
    /// decision point that enforces:
    /// 1. Monotonic progress floor (won't reset below confirmed_height_floor)
    /// 2. No concurrent recovery (won't trigger if ResyncInProgress)
    /// 3. Rate limiting (max MAX_CONSECUTIVE_RESYNCS, with cooldown)
    /// 4. Snap sync availability (won't trigger if snap sync disabled)
    /// 5. Snap attempt limit (won't trigger after 3 failed snap attempts)
    ///
    /// Returns true if the request was honored, false if refused.
    pub fn request_genesis_resync(&mut self, reason: super::RecoveryReason) -> bool {
        // Emergency recovery: reasons that indicate the node is critically stuck
        // and MUST snap sync regardless of user preferences (--no-snap-sync, floor).
        // These are situations where header-first sync cannot recover:
        // - Deep fork: peers don't recognize our chain at all
        // - Apply failures: blocks from peers fail validation (state divergence)
        let is_emergency = matches!(
            reason,
            super::RecoveryReason::GenesisFallbackEmptyHeaders
                | super::RecoveryReason::AllPeersBlacklistedDeepFork
                | super::RecoveryReason::ApplyFailuresSnapThreshold { .. }
        );

        // Gate 1: Monotonic progress floor — bypassed for emergencies
        if self.confirmed_height_floor > 0 && !is_emergency {
            warn!(
                "[RECOVERY] Genesis resync REFUSED: confirmed_height_floor={} \
                 (reason: {:?}). Manual intervention required.",
                self.confirmed_height_floor, reason
            );
            return false;
        }
        if self.confirmed_height_floor > 0 && is_emergency {
            warn!(
                "[RECOVERY] Genesis resync BYPASSING floor={} for emergency recovery \
                 (reason: {:?}). Node is critically stuck.",
                self.confirmed_height_floor, reason
            );
        }

        // Gate 2: No concurrent recovery
        if matches!(self.recovery_phase, super::RecoveryPhase::ResyncInProgress) {
            info!(
                "[RECOVERY] Genesis resync REFUSED: resync already in progress \
                 (reason: {:?})",
                reason
            );
            return false;
        }

        // Gate 3: Rate limiting
        if self.consecutive_resync_count >= super::MAX_CONSECUTIVE_RESYNCS {
            warn!(
                "[RECOVERY] Genesis resync REFUSED: {} consecutive resyncs (max {}) \
                 (reason: {:?}). Manual intervention required.",
                self.consecutive_resync_count,
                super::MAX_CONSECUTIVE_RESYNCS,
                reason
            );
            return false;
        }

        // Gate 4: Snap sync must be available — bypassed for emergencies.
        // --no-snap-sync is a preference for normal sync, not a ban on recovery.
        if self.snap.threshold == u64::MAX && !is_emergency {
            info!(
                "[RECOVERY] Genesis resync REFUSED: snap sync disabled \
                 (reason: {:?}). Header-first recovery only.",
                reason
            );
            return false;
        }
        if self.snap.threshold == u64::MAX && is_emergency {
            warn!(
                "[RECOVERY] Enabling snap sync for emergency recovery \
                 (reason: {:?}). Node cannot recover via header-first.",
                reason
            );
            // Temporarily enable snap sync for this recovery
            self.snap.threshold = 10;
        }

        // Gate 5: Snap attempt limit
        if self.snap.attempts >= 3 {
            info!(
                "[RECOVERY] Genesis resync REFUSED: snap attempts exhausted ({}/3) \
                 (reason: {:?})",
                self.snap.attempts, reason
            );
            return false;
        }

        // All gates passed -- honor the request
        info!(
            "[RECOVERY] Genesis resync ACCEPTED: {:?} \
             (floor={}, resync_count={}, snap_attempts={}, phase={:?})",
            reason,
            self.confirmed_height_floor,
            self.consecutive_resync_count,
            self.snap.attempts,
            self.recovery_phase
        );
        self.fork.needs_genesis_resync = true;
        true
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
        if self.fork.consecutive_empty_headers < 10 {
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
        // Small gaps (≤12 blocks) are NOT deep forks — resolve_shallow_fork()
        // and rollback can handle them without wiping state.
        // Snap sync for small gaps loses block history and creates a cascade:
        // snap → no block 1 → next fork → rollback impossible → re-snap.
        let gap = best_peer_height.saturating_sub(self.local_height);
        if gap <= 12 {
            return false;
        }
        // If snap sync can handle this gap, don't escalate to deep fork.
        // next_request() will attempt snap sync first.
        let gap = best_peer_height.saturating_sub(self.local_height);
        let enough_peers = self.peers.len() >= 3;
        if enough_peers && gap > self.snap.threshold {
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
}
