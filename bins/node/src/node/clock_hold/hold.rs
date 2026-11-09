use std::collections::VecDeque;

use crypto::Hash;
use doli_core::Block;
use network::PeerId;

/// Most blocks held at once; the oldest is dropped first.
pub(crate) const CLOCK_HOLD_CAP: usize = 32;

/// Blocks ahead of the local clock, keyed by hash, in insert order.
#[derive(Default)]
pub(crate) struct ClockHold {
    entries: VecDeque<(Hash, Block, PeerId)>,
}

impl ClockHold {
    /// Holds `block` from `supplier`; a hash already held keeps its first supplier.
    pub(crate) fn insert(&mut self, block: Block, supplier: PeerId) {
        let hash = block.hash();
        if self.contains(&hash) {
            return;
        }
        if self.entries.len() >= CLOCK_HOLD_CAP {
            self.entries.pop_front();
        }
        self.entries.push_back((hash, block, supplier));
    }

    /// Removes and returns every held block with `timestamp <= limit`, in insert order.
    pub(crate) fn take_due(&mut self, limit: u64) -> Vec<(Block, PeerId)> {
        let (due, keep): (Vec<_>, Vec<_>) = self
            .entries
            .drain(..)
            .partition(|(_, b, _)| b.header.timestamp <= limit);
        self.entries = keep.into();
        due.into_iter().map(|(_, b, p)| (b, p)).collect()
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn contains(&self, hash: &Hash) -> bool {
        self.entries.iter().any(|(h, _, _)| h == hash)
    }
}
