use super::*;

/// Whether peer height stops a build of `next_height`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerHeightDecision {
    Defer,
    Proceed,
}

/// `gate` is the fork-choice activation height; `latch_holds` is the first-reply latch.
pub fn peer_height_gate(
    next_height: u64,
    best_peer_height: u64,
    gate: u64,
    latch_holds: bool,
) -> PeerHeightDecision {
    let tip = next_height.saturating_sub(1);
    let gap = best_peer_height.saturating_sub(tip);
    let defer = if next_height < 10 {
        gap > 3
    } else if tip < gate {
        gap > 5
    } else {
        latch_holds
    };
    if defer {
        PeerHeightDecision::Defer
    } else {
        PeerHeightDecision::Proceed
    }
}

/// True iff the block stored for `slot` has a parent on this node's branch; any store error is false.
pub fn slot_block_on_my_branch(store: &BlockStore, slot: u32, my_tip: &Hash) -> bool {
    let Ok(Some(hash)) = store.get_hash_by_slot(slot) else {
        return false;
    };
    let Ok(Some(header)) = store.get_header(&hash) else {
        return false;
    };
    let parent = header.prev_hash;
    if parent == *my_tip {
        return true;
    }
    let Ok(Some(height)) = store.get_height_by_hash(&parent) else {
        return false;
    };
    matches!(store.get_hash_by_height(height), Ok(Some(h)) if h == parent)
}

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
            // Per-tick diagnostic — kept at debug! to avoid CPU burn from log volume.
            // try_produce_block() runs every production timer tick (1 Hz) plus event-drain
            // escape hatch; post-INC-I-104 this was ~33% of producer log volume.
            debug!(
                "[NODE_PRODUCE] slot={} can_produce result: {:?}",
                current_slot, result
            );
            result
        }; // sync_state guard dropped here — safe to call &mut self methods below

        match auth_result {
            ProductionAuthorization::Authorized => {
                // INC-I-204 M3: `shallow_rollback_count` is NOT reset here. This arm
                // runs only on producers, so the budget refilled every authorized slot
                // on a producer and never on a seed — the same rung looked live on one
                // and dead on the other. Both now reset on a successful apply
                // (`apply_block`), which is the condition the budget actually measures.
                self.cumulative_rollback_depth = 0;
                debug!(
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
