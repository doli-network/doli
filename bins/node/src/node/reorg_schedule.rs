//! INC-I-281: execute_reorg checks each fork block's slot schedule on the
//! node state at the block's parent, with the predicate and list a Full apply
//! uses. A door cannot derive the list after a boundary block that only the
//! walked branch holds; here that boundary block is already applied.

use super::*;

impl Node {
    /// False (and a warn) when a Full apply on the current tip would refuse
    /// `block`'s producer for its slot. Skipped in the INC-I-010 layer-3
    /// Light window, as in handle_new_block.
    pub(super) async fn reorg_block_scheduled(
        &self,
        block: &Block,
        index: usize,
        total: usize,
    ) -> bool {
        if self.snap_sync_height.is_some() || self.floor_fallback_window {
            return true;
        }
        let height = self.chain_state.read().await.best_height + 1;
        let Err(e) = self.schedule_verdict(block, height).await else {
            return true;
        };
        warn!(
            "[REORG] Fork block {:.16} at h={} slot {} is not scheduled at its parent: {} — reorg stops after {}/{} blocks (INC-I-281)",
            block.hash(),
            height,
            block.header.slot,
            e,
            index,
            total
        );
        false
    }

    /// `validate_producer_eligibility` with the Full-apply inputs at `height`,
    /// except the predicate height: the block's slot, as at the doors. Past
    /// genesis both route to the round-robin; while `slot` is in the genesis
    /// phase the bootstrap arm accepts (no bootstrap list), as Light did.
    async fn schedule_verdict(
        &self,
        block: &Block,
        height: u64,
    ) -> Result<(), validation::ValidationError> {
        let active: Vec<PublicKey> = {
            let producers = self.producer_set.read().await;
            producers
                .active_producers_at_height(height)
                .iter()
                .map(|p| p.public_key)
                .collect()
        };
        let weighted = self.bond_weights_for_scheduling(active).await;
        let list = if self.epoch_state.active_list.is_empty() {
            self.epoch_state.producer_list.clone()
        } else {
            self.epoch_state.active_list.clone()
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let net = self.config.network;
        let mut ctx = validation::ValidationContext::new(
            ConsensusParams::for_network(net),
            net,
            now,
            block.header.slot as u64,
        )
        .with_producers_weighted(weighted)
        .with_epoch_producer_list(list)
        .with_inc_i_026_scheduler_activation_height(
            net.params().inc_i_026_scheduler_activation_height,
        );
        if let Some(ref spec) = self.config.chainspec {
            ctx.params.apply_chainspec(spec);
        }
        validation::validate_producer_eligibility(&block.header, &ctx)
    }
}
