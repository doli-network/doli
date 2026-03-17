use super::*;

impl Node {
    // =========================================================================
    // PRODUCTION GATE CHECK - Single source of truth for production safety
    //
    // This is the FIRST and MOST CRITICAL check. The SyncManager's ProductionGate
    // implements defense-in-depth with multiple layers:
    // 1. Explicit block check (invariant violations)
    // 2. Resync-in-progress check
    // 3. Active sync check (downloading headers/bodies)
    // 4. Bootstrap gate (must have fresh peer status before producing)
    // 5. Post-resync grace period with exponential backoff
    // 6. Peer synchronization check (within N slots/heights)
    //
    // ALL checks must pass. This prevents the infinite resync loop bug where
    // nodes at height 0 would produce orphan blocks for far-ahead slots.
    // =========================================================================

    /// Handle production authorization from SyncManager.
    /// Returns `true` if production is authorized, `false` if blocked.
    pub(super) async fn handle_production_authorization(&mut self, current_slot: u32) -> bool {
        let auth_result = {
            let mut sync_state = self.sync_manager.write().await;
            let result = sync_state.can_produce(current_slot);
            info!(
                "[NODE_PRODUCE] slot={} can_produce result: {:?}",
                current_slot, result
            );
            result
        }; // sync_state guard dropped here — safe to call &mut self methods below

        match auth_result {
            ProductionAuthorization::Authorized => {
                self.consecutive_fork_blocks = 0;
                self.shallow_rollback_count = 0;
                self.cumulative_rollback_depth = 0;
                info!(
                    "[NODE_PRODUCE] slot={} AUTHORIZED - proceeding",
                    current_slot
                );
                true
            }
            ProductionAuthorization::BlockedSyncing => {
                info!("[NODE_PRODUCE] slot={} BLOCKED: Syncing", current_slot);
                false
            }
            ProductionAuthorization::BlockedResync { .. } => {
                info!("[NODE_PRODUCE] slot={} BLOCKED: Resync", current_slot);
                false
            }
            ProductionAuthorization::BlockedBehindPeers {
                local_height,
                peer_height,
                height_diff,
            } => {
                // Being behind peers is NOT fork evidence — it's normal sync lag.
                // Never increment fork counter or trigger rollback from BehindPeers.
                // Sync manager handles catching up; rollbacks only make it worse.
                info!(
                    "[NODE_PRODUCE] slot={} BLOCKED: BehindPeers local_h={} peer_h={} diff={} (not fork, sync will catch up)",
                    current_slot, local_height, peer_height, height_diff
                );
                false
            }
            ProductionAuthorization::BlockedAheadOfPeers {
                local_height,
                peer_height,
                height_ahead,
            } => {
                self.consecutive_fork_blocks += 1;
                warn!(
                    "[NODE_PRODUCE] FORK DETECTED via AheadOfPeers: slot={} local_h={} peer_h={} ahead={} (consecutive={})",
                    current_slot, local_height, peer_height, height_ahead, self.consecutive_fork_blocks
                );
                // Try fork recovery first, then auto-resync if threshold exceeded
                self.try_trigger_fork_recovery().await;
                self.maybe_auto_resync(current_slot).await;
                false
            }
            ProductionAuthorization::BlockedSyncFailures { failure_count } => {
                // Low sync failure counts (< 50) are normal network churn.
                // But 50+ consecutive failures means our chain has genuinely diverged
                // from ALL peers — every GetHeaders returns empty because no peer
                // recognizes our tip hash. This IS a fork.
                if failure_count >= 50 {
                    self.consecutive_fork_blocks += 1;
                    error!(
                        "[NODE_PRODUCE] FORK DETECTED via persistent SyncFailures: slot={} failures={} (consecutive={})",
                        current_slot, failure_count, self.consecutive_fork_blocks
                    );
                    self.maybe_auto_resync(current_slot).await;
                } else {
                    warn!(
                        "[NODE_PRODUCE] slot={} BLOCKED: SyncFailures (failures={})",
                        current_slot, failure_count
                    );
                }
                false
            }
            ProductionAuthorization::BlockedInsufficientPeers {
                peer_count,
                min_required,
            } => {
                warn!(
                    "[NODE_PRODUCE] slot={} BLOCKED: InsufficientPeers - only {} peers (need {})",
                    current_slot, peer_count, min_required
                );
                false
            }
            ProductionAuthorization::BlockedChainMismatch {
                peer_id,
                local_hash,
                peer_hash,
                local_height,
            } => {
                self.consecutive_fork_blocks += 1;
                error!(
                    "[NODE_PRODUCE] slot={} BLOCKED: Chain mismatch with peer {} at height {} (local={}, peer={}) (consecutive={})",
                    current_slot, peer_id, local_height, local_hash, peer_hash, self.consecutive_fork_blocks
                );
                // Try fork recovery first, then auto-resync if threshold exceeded
                self.try_trigger_fork_recovery().await;
                self.maybe_auto_resync(current_slot).await;
                false
            }
            ProductionAuthorization::BlockedNoGossipActivity {
                seconds_since_gossip,
                peer_count,
            } => {
                // No gossip activity is NOT a definitive fork signal — it can happen
                // during network startup or temporary connectivity issues.
                warn!(
                    "[NODE_PRODUCE] slot={} BLOCKED: No gossip activity for {}s with {} peers",
                    current_slot, seconds_since_gossip, peer_count
                );
                false
            }
            ProductionAuthorization::BlockedExplicit { reason } => {
                info!(
                    "[NODE_PRODUCE] slot={} BLOCKED: Explicit - {}",
                    current_slot, reason
                );
                false
            }
            ProductionAuthorization::BlockedBootstrap { reason } => {
                info!(
                    "[NODE_PRODUCE] slot={} BLOCKED: Bootstrap - {}",
                    current_slot, reason
                );
                false
            }
            ProductionAuthorization::BlockedConflictsFinality {
                local_finalized_height,
            } => {
                warn!(
                    "[NODE_PRODUCE] slot={} BLOCKED: Chain conflicts with finalized block at height {}",
                    current_slot, local_finalized_height
                );
                false
            }
        }
    }
}
