//! INC-I-289 K-1: branch weight at/above the INC-I-288 gate is the bond of the
//! branch's DISTINCT supporters from the common ancestor, each counted once.
//! Fork choice and finality weigh in this one unit.

use std::cell::Cell;
use std::collections::HashSet;

use crypto::Hash;
use doli_core::Block;

use super::{ForkChoiceFinality, ReorgHandler, ReorgResult};

/// A supporter: producer public-key bytes (the per-block adapter uses block-hash bytes).
pub type SupporterId = [u8; 32];

/// Who backs one block. The block's bits attest its PARENT, so they count only
/// when the parent is on the same branch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockSupport {
    pub parent: Hash,
    /// The block's producer and the verified gossip attesters held for the block.
    pub backers: Vec<(SupporterId, u64)>,
    /// The attesters the block's bitfield names (support for `parent`).
    pub parent_attesters: Vec<(SupporterId, u64)>,
    /// The block's slot and its parent's slot, when the caller knows the headers.
    pub slot: Option<u32>,
    pub parent_slot: Option<u32>,
}

impl BlockSupport {
    /// One supporter of `weight`.
    pub fn single(id: SupporterId, weight: u64) -> Self {
        Self {
            backers: vec![(id, weight)],
            ..Self::default()
        }
    }

    /// False only when both slots are known and apply's slot rule rejects the block.
    pub fn advances_slot(&self) -> bool {
        match (self.slot, self.parent_slot) {
            (Some(s), Some(p)) => super::fork_choice::slot_advances(s, p),
            _ => true,
        }
    }
}

/// Bond of the distinct supporters of `blocks`; `None` when a block is unknown.
pub fn union_weight(
    blocks: &[Hash],
    get_support: impl Fn(&Hash) -> Option<BlockSupport>,
) -> Option<u64> {
    let members: HashSet<&Hash> = blocks.iter().collect();
    let mut seen: HashSet<SupporterId> = HashSet::new();
    let mut sum = 0u64;
    for h in blocks {
        let s = get_support(h)?;
        let bits: &[(SupporterId, u64)] = if members.contains(&s.parent) {
            &s.parent_attesters
        } else {
            &[]
        };
        for (id, w) in s.backers.iter().chain(bits) {
            if seen.insert(*id) {
                sum = sum.saturating_add(*w);
            }
        }
    }
    Some(sum)
}

/// [`union_weight`] of a candidate branch: an unknown block adds nothing, and a
/// branch holding a block apply's slot rule rejects weighs 0 (it cannot be applied).
pub fn candidate_union(
    blocks: &[Hash],
    get_support: impl Fn(&Hash) -> Option<BlockSupport>,
) -> u64 {
    let stuck = Cell::new(false);
    let w = union_weight(blocks, |h| {
        let s = get_support(h).unwrap_or_default();
        stuck.set(stuck.get() || !s.advances_slot());
        Some(s)
    });
    if stuck.get() {
        0
    } else {
        w.unwrap_or(0)
    }
}

impl ReorgHandler {
    /// The per-block unit as support: each block its own supporter of its recorded
    /// producer weight, else `node_weight` (the doors' callers without identities).
    pub(super) fn per_block_support(
        &self,
        hash: &Hash,
        node_weight: impl Fn(&Hash) -> Option<u64>,
    ) -> Option<BlockSupport> {
        let recorded = self.block_weights.get(hash).map(|b| b.producer_weight);
        let w = recorded.or_else(|| node_weight(hash))?;
        Some(BlockSupport::single(*hash.as_bytes(), w))
    }
}

impl ReorgHandler {
    /// [`Self::check_reorg_supported`] in the per-block unit (each block its own supporter).
    pub fn check_reorg_weighted(
        &self,
        block: &Block,
        current_tip: Hash,
        block_producer_weight: u64,
        get_height: impl Fn(&Hash) -> Option<u64>,
        finality: ForkChoiceFinality,
    ) -> Option<ReorgResult> {
        let recorded = |h: &Hash| self.per_block_support(h, |_| None);
        self.check_reorg_supported(
            block,
            current_tip,
            block_producer_weight,
            get_height,
            recorded,
            finality,
        )
    }

    /// [`Self::plan_reorg_with_support`] in the per-block unit: a block's recorded
    /// producer weight, else `get_weight`.
    pub fn plan_reorg_with_weights(
        &self,
        current_tip: Hash,
        new_tip: Hash,
        get_parent: impl Fn(&Hash) -> Option<Hash>,
        get_height: impl Fn(&Hash) -> Option<u64>,
        get_weight: impl Fn(&Hash) -> Option<u64>,
        finality: ForkChoiceFinality,
    ) -> Option<ReorgResult> {
        let get_support = |h: &Hash| self.per_block_support(h, &get_weight);
        self.plan_reorg_with_support(
            current_tip,
            new_tip,
            get_parent,
            get_height,
            get_support,
            finality,
        )
    }
}
