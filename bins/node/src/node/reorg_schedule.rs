//! INC-I-281: execute_reorg checks each fork block's slot schedule on the
//! node state at the block's parent. The round-robin arm uses the Full-apply
//! list and producer weights; the context carries no bootstrap list, so at a
//! genesis-phase height the bootstrap arm accepts any producer, as the Light
//! apply did. A door cannot derive the list after a boundary block that only
//! the walked branch holds; here that boundary block is already applied.
//! Doors that judge a block under a learned anchor call the same
//! [`Node::schedule_verdict`] with the anchor's inputs.

use super::*;

/// The Full-apply list and producer weights the schedule check reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScheduleInputs {
    pub(super) list: Vec<PublicKey>,
    pub(super) weights: Vec<(PublicKey, u64)>,
}

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
        let inputs = self.schedule_inputs(height).await;
        let Err(e) = self.schedule_verdict(block, &inputs, height) else {
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

    /// The schedule inputs at `height` on the node's current state.
    pub(super) async fn schedule_inputs(&self, height: u64) -> ScheduleInputs {
        let active: Vec<PublicKey> = {
            let producers = self.producer_set.read().await;
            producers
                .active_producers_at_height(height)
                .iter()
                .map(|p| p.public_key)
                .collect()
        };
        let weights = self.bond_weights_for_scheduling(active).await;
        let list = if self.epoch_state.active_list.is_empty() {
            self.epoch_state.producer_list.clone()
        } else {
            self.epoch_state.active_list.clone()
        };
        ScheduleInputs { list, weights }
    }

    /// `validate_producer_eligibility` at `height` with `inputs`; no bootstrap
    /// list, so the genesis-phase arm accepts any producer, as the Light apply did.
    pub(super) fn schedule_verdict(
        &self,
        block: &Block,
        inputs: &ScheduleInputs,
        height: u64,
    ) -> Result<(), validation::ValidationError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let net = self.config.network;
        let mut ctx =
            validation::ValidationContext::new(ConsensusParams::for_network(net), net, now, height)
                .with_producers_weighted(inputs.weights.clone())
                .with_epoch_producer_list(inputs.list.clone())
                .with_inc_i_026_scheduler_activation_height(
                    net.params().inc_i_026_scheduler_activation_height,
                );
        if let Some(ref spec) = self.config.chainspec {
            ctx.params.apply_chainspec(spec);
        }
        validation::validate_producer_eligibility(&block.header, &ctx)
    }
}
