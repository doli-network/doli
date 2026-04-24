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
    // 5. Minimum peer count (echo chamber prevention)
    //
    // ALL checks must pass. This prevents the infinite resync loop bug where
    // nodes at height 0 would produce orphan blocks for far-ahead slots.
    // =========================================================================

    /// Handle production authorization from SyncManager.
    /// Returns `true` if production is authorized, `false` if blocked.
    pub async fn handle_production_authorization(&mut self, current_slot: u32) -> bool {
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
            ProductionAuthorization::BlockedAwaitingCanonicalBlock => {
                info!(
                    "[NODE_PRODUCE] slot={} BLOCKED: Awaiting canonical block after snap sync",
                    current_slot
                );
                false
            }
        }
    }
}
