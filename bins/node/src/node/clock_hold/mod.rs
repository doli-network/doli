use crypto::Hash;
use doli_core::consensus::{MAX_DRIFT, MAX_FUTURE_SLOTS};
use doli_core::validation::ValidationError;
use doli_core::Block;
use network::PeerId;
use tracing::{debug, warn};

use super::Node;

mod hold;
pub(crate) use hold::ClockHold;

#[cfg(test)]
mod tests;

/// True only for the typed future-time rule failure.
pub(crate) fn not_yet(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<ValidationError>(),
        Some(ValidationError::TimestampTooFuture(_))
    )
}

impl Node {
    /// Wall-clock seconds since the Unix epoch, plus the test clock offset (0 in production).
    pub fn wall_now_secs(&self) -> u64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now.saturating_add_signed(self.clock_offset_secs)
    }

    /// Moves this node's wall clock by `secs` for tests.
    #[doc(hidden)]
    #[allow(dead_code)]
    pub fn set_clock_offset_for_test(&mut self, secs: i64) {
        self.clock_offset_secs = secs;
    }

    /// True when `err` is the future-time rule and the body is whole and within size at `height`.
    pub(crate) fn not_yet_block(&self, err: &anyhow::Error, block: &Block, height: u64) -> bool {
        not_yet(err)
            && block.body_matches_header(
                self.inc_i_178_attestation_bls_activation_height,
                Some(height),
            )
            && block.size() <= doli_core::consensus::max_block_size(height)
    }

    /// Holds a not-yet block when its timestamp is in its slot, the slot is in the band, it is
    /// authenticated, and its supplier is known. Returns true when the block is held.
    pub(crate) async fn try_hold(&mut self, block: &Block, supplier: Option<PeerId>) -> bool {
        let Some(supplier) = supplier else {
            debug!(
                "[BLOCK] NOT_YET {:.16}: no supplier, not held",
                block.hash()
            );
            return false;
        };
        if self.params.timestamp_to_slot(block.header.timestamp) != block.header.slot {
            debug!(
                "[BLOCK] NOT_YET {:.16}: timestamp outside its slot, not held",
                block.hash()
            );
            return false;
        }
        let wall_slot = self.params.timestamp_to_slot(self.wall_now_secs()) as u64;
        if u64::from(block.header.slot) > wall_slot + MAX_FUTURE_SLOTS {
            debug!(
                "[BLOCK] NOT_YET {:.16}: beyond the slot band, not held",
                block.hash()
            );
            return false;
        }
        let best_height = self.chain_state.read().await.best_height;
        if !self.gossip_block_authenticated(block, best_height) {
            return false;
        }
        debug!(
            "[BLOCK] HELD {:.16} slot {}",
            block.hash(),
            block.header.slot
        );
        self.clock_hold.insert(block.clone(), supplier);
        true
    }

    /// Re-submits every held block whose time has come through `handle_new_block`.
    pub(crate) async fn release_clock_hold(&mut self) {
        let limit = self.wall_now_secs().saturating_add(MAX_DRIFT);
        for (block, supplier) in self.clock_hold.take_due(limit) {
            let hash = block.hash();
            if let Err(e) = self.handle_new_block(block, supplier).await {
                warn!("[BLOCK] held block {:.16} failed at its time: {}", hash, e);
            }
        }
    }

    /// Number of blocks in the clock hold; test seam.
    #[doc(hidden)]
    #[allow(dead_code)]
    pub fn clock_hold_len(&self) -> usize {
        self.clock_hold.len()
    }

    /// True when `hash` is in the clock hold; test seam.
    #[doc(hidden)]
    #[allow(dead_code)]
    pub fn clock_held(&self, hash: &Hash) -> bool {
        self.clock_hold.contains(hash)
    }
}
