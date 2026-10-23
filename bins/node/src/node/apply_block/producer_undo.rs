//! ProducerSet undo boundary of `apply_block`: the pre-loop snapshot doubles as
//! the in-memory restore point when the apply returns `Err`.

use super::*;

pub(super) struct ProducerUndo {
    /// Empty = the block cannot mutate the set (the undo-record sentinel).
    pub(super) snapshot: Vec<u8>,
    armed: bool,
}

impl ProducerUndo {
    /// Set right before the transaction loop takes the ProducerSet write lock.
    pub(super) fn arm(&mut self) {
        self.armed = true;
    }
}

/// Snapshot the ProducerSet only when this block can mutate it.
pub(super) async fn capture(node: &Node, block: &Block, height: u64) -> ProducerUndo {
    let blocks_per_epoch = node.config.network.blocks_per_reward_epoch();
    let at_epoch_boundary = blocks_per_epoch > 0 && height.is_multiple_of(blocks_per_epoch);
    let genesis_blocks = node.config.network.genesis_blocks();
    let genesis_completion = genesis_blocks != 0 && height == genesis_blocks + 1;
    let producers = node.producer_set.read().await;
    let needs_producer_snapshot = at_epoch_boundary
        || genesis_completion
        || helpers::block_mutates_producer_set(block)
        || producers.has_unbonding_due(height, UNBONDING_PERIOD);
    let snapshot = if needs_producer_snapshot {
        bincode::serialize(&*producers).unwrap_or_default()
    } else {
        Vec::new()
    };
    ProducerUndo {
        snapshot,
        armed: false,
    }
}

/// Put the pre-apply ProducerSet back after the apply returned `Err`.
pub(super) async fn restore_after_failed_apply(node: &mut Node, undo: &ProducerUndo, height: u64) {
    if !undo.armed || undo.snapshot.is_empty() {
        debug!("[APPLY_UNDO] no producer restore needed after failed apply at h={height}");
        return;
    }
    // Bytes serialized by this process moments earlier; a decode error means memory corruption.
    let restored = match ProducerSet::from_snapshot_bytes(&undo.snapshot) {
        Ok(set) => set,
        Err(e) => {
            error!("[APPLY_UNDO] producer snapshot decode failed at h={height}: {e}");
            return;
        }
    };
    {
        let mut producers = node.producer_set.write().await;
        *producers = restored;
    }
    warn!("[APPLY_UNDO] restored producer set after failed apply at h={height}");
}

impl Node {
    /// Apply a block to the chain
    ///
    /// `mode`: `Full` for gossip/production (checks MAX_PAST_SLOTS, VDF),
    ///         `Light` for sync/reorg (skips time-based checks that reject old blocks).
    pub async fn apply_block(&mut self, block: Block, mode: ValidationMode) -> Result<()> {
        let height = self.chain_state.read().await.best_height + 1;
        let mut undo = capture(self, &block, height).await;
        let result = self.apply_block_inner(block, mode, &mut undo).await;
        if result.is_err() {
            restore_after_failed_apply(self, &undo, height).await;
        }
        result
    }
}
