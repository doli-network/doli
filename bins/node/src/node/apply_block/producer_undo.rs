//! Undo boundary of `apply_block`: the pre-apply ProducerSet and maintainer
//! snapshots double as the in-memory (and, for the maintainer root, on-disk)
//! restore point when the apply returns `Err`.

use super::*;

pub(super) struct ProducerUndo {
    /// Empty = the block cannot mutate the set (the undo-record sentinel).
    pub(super) snapshot: Vec<u8>,
    /// The INC-I-174 record; `None` = the block carries no maintainer rotation.
    pub(super) maintainer: Option<storage::MaintainerUndoSnapshot>,
    armed: bool,
}

impl ProducerUndo {
    /// Set right before the transaction loop takes the ProducerSet write lock.
    pub(super) fn arm(&mut self) {
        self.armed = true;
    }
}

/// Snapshot the ProducerSet only when this block can mutate it, and the
/// maintainer root before the first rotation in the block.
pub(super) async fn capture(node: &Node, block: &Block, height: u64) -> ProducerUndo {
    let blocks_per_epoch = node.config.network.blocks_per_reward_epoch();
    let at_epoch_boundary = blocks_per_epoch > 0 && height.is_multiple_of(blocks_per_epoch);
    let genesis_blocks = node.config.network.genesis_blocks();
    let genesis_completion = genesis_blocks != 0 && height == genesis_blocks + 1;
    let maintainer = node.capture_maintainer_undo(block).await;
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
        maintainer,
        armed: false,
    }
}

/// Put the pre-apply state back after the apply returned `Err`; each state
/// class is restored on its own so one failing does not skip the other.
async fn restore_after_failed_apply(node: &mut Node, undo: &ProducerUndo, height: u64) {
    restore_producer_set(node, undo, height).await;
    restore_maintainer_set(node, undo, height).await;
}

async fn restore_producer_set(node: &mut Node, undo: &ProducerUndo, height: u64) {
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

/// Memory and `maintainer_state.bin`; no write when the root never moved.
async fn restore_maintainer_set(node: &Node, undo: &ProducerUndo, height: u64) {
    let (Some(snapshot), Some(state)) = (&undo.maintainer, &node.maintainer_state) else {
        return;
    };
    let mut ms = state.write().await;
    if ms.set == snapshot.set && ms.last_derived_height == snapshot.last_derived_height {
        debug!("[APPLY_UNDO] no maintainer restore needed after failed apply at h={height}");
        return;
    }
    match crate::node::maintainer_rewind::install_maintainer_snapshot(
        &mut ms,
        snapshot,
        &node.config.data_dir,
    ) {
        Ok(()) => warn!("[APPLY_UNDO] restored maintainer set after failed apply at h={height}"),
        Err(e) => error!(
            "[APPLY_UNDO] maintainer restore failed to persist at h={height}: {e}; \
             the in-memory root was left unchanged"
        ),
    }
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
