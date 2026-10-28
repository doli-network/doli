//! What a Normal reorg refuses before its rollback: a fork block whose body
//! does not match its header commitments, and a fork block that a reorg refused before.
//! Also the boundary blocks those refused reorgs had applied (learned anchors).

use std::collections::VecDeque;

use super::reorg_restore::{ReorgMode, ReorgStop};
use super::reorg_schedule::ScheduleInputs;
use super::*;

/// Refusals remembered (FIFO).
const REFUSED_REORG_CAP: usize = 256;
/// Learned anchors remembered (FIFO).
const LEARNED_ANCHOR_CAP: usize = 16;

/// A boundary block a refused reorg had applied, with the schedule inputs of
/// the epoch after it on that branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LearnedAnchor {
    pub(super) hash: Hash,
    pub(super) height: u64,
    pub(super) inputs: ScheduleInputs,
}

impl LearnedAnchor {
    /// True for a height in the epoch after the anchor.
    pub(super) fn covers(&self, height: u64, blocks_per_epoch: u64) -> bool {
        height > self.height && height <= self.height.saturating_add(blocks_per_epoch)
    }
}

/// A fork block a reorg refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    /// Header hash: the in-reorg schedule check refused it.
    Unscheduled(Hash),
    /// Content key: apply_block refused this exact header and body.
    ApplyFailed(Hash),
}

/// BLAKE3 over the full block serialization (header and body).
pub(super) fn content_key(block: &Block) -> Hash {
    crypto::hash::hash(&block.serialize())
}

/// Fork blocks a reorg refused, and the anchors those reorgs had applied.
#[derive(Debug, Default)]
pub struct RefusedReorgBlocks {
    refused: VecDeque<Refusal>,
    anchors: VecDeque<LearnedAnchor>,
    body_refusals: u64,
}

impl RefusedReorgBlocks {
    pub(super) fn insert(&mut self, refusal: Refusal) {
        if self.refused.contains(&refusal) {
            return;
        }
        self.refused.push_back(refusal);
        if self.refused.len() > REFUSED_REORG_CAP {
            self.refused.pop_front();
        }
    }

    fn refuses(&self, block: &Block) -> bool {
        self.refusal_of(block).is_some()
    }

    /// The refusal that holds `block`: by its header hash, else by its content.
    fn refusal_of(&self, block: &Block) -> Option<Refusal> {
        let unscheduled = Refusal::Unscheduled(block.hash());
        if self.refused.contains(&unscheduled) {
            return Some(unscheduled);
        }
        if !self
            .refused
            .iter()
            .any(|r| matches!(r, Refusal::ApplyFailed(_)))
        {
            return None;
        }
        let failed = Refusal::ApplyFailed(content_key(block));
        self.refused.contains(&failed).then_some(failed)
    }

    /// Counts a reorg or walk stopped by a fork body, not by its header or branch.
    pub(super) fn note_body_refusal(&mut self) {
        self.body_refusals = self.body_refusals.wrapping_add(1);
    }

    pub(super) fn body_refusals(&self) -> u64 {
        self.body_refusals
    }

    /// False when an anchor with this hash is already held.
    fn learn(&mut self, anchor: LearnedAnchor) -> bool {
        if self.anchors.iter().any(|a| a.hash == anchor.hash) {
            return false;
        }
        self.anchors.push_back(anchor);
        if self.anchors.len() > LEARNED_ANCHOR_CAP {
            self.anchors.pop_front();
        }
        true
    }

    pub(super) fn has_anchors(&self) -> bool {
        !self.anchors.is_empty()
    }

    /// The learned anchor with this hash at this height.
    pub(super) fn anchor(&self, hash: &Hash, height: u64) -> Option<&LearnedAnchor> {
        self.anchors
            .iter()
            .find(|a| a.hash == *hash && a.height == height)
    }

    /// The learned anchor `block` is when it sits at `height` (`None`: unknown).
    pub(super) fn anchor_of(&self, block: &Block, height: Option<u64>) -> Option<&LearnedAnchor> {
        if self.anchors.is_empty() {
            return None;
        }
        self.anchor(&block.hash(), height?)
    }
}

/// (hash, height) of the last boundary block of `applied`, the fork blocks
/// that end at `tip_height`.
fn last_boundary(applied: &[Hash], tip_height: u64, blocks_per_epoch: u64) -> Option<(Hash, u64)> {
    if blocks_per_epoch == 0 {
        return None;
    }
    let first = (tip_height + 1).saturating_sub(applied.len() as u64);
    applied
        .iter()
        .enumerate()
        .rev()
        .map(|(k, h)| (*h, first + k as u64))
        .find(|(_, h)| h.is_multiple_of(blocks_per_epoch))
}

impl Node {
    /// Err for a block a reorg refused before: a door refuses it before any
    /// rollback or recorded weight.
    pub(super) fn door_refusal(&self, block: &Block) -> Result<()> {
        if self.refused_reorg_blocks.refuses(block) {
            anyhow::bail!(
                "fork block {:.16} was refused by a reorg before",
                block.hash()
            );
        }
        Ok(())
    }

    /// Before a restore: remember the last boundary block of the applied fork
    /// prefix and the schedule inputs at the refused block's parent.
    pub(super) async fn learn_anchor(&mut self, applied: &[Hash]) {
        let bpe = self.config.network.blocks_per_reward_epoch();
        let (tip, tip_height) = {
            let cs = self.chain_state.read().await;
            (cs.best_hash, cs.best_height)
        };
        if applied.last() != Some(&tip) {
            return;
        }
        let Some((hash, height)) = last_boundary(applied, tip_height, bpe) else {
            return;
        };
        let inputs = self.schedule_inputs(tip_height + 1).await;
        if self.refused_reorg_blocks.learn(LearnedAnchor {
            hash,
            height,
            inputs,
        }) {
            info!(
                "[REORG] Boundary block {:.16} at h={} of a refused branch is a learned anchor: doors judge its next epoch with its list (INC-I-281)",
                hash, height
            );
        }
    }

    /// A Normal reorg stops here, before its rollback, on a junk body or a refused block.
    pub(super) async fn pre_rollback_stop(
        &self,
        new_blocks: &[Block],
        mode: ReorgMode,
    ) -> Option<ReorgStop> {
        if mode != ReorgMode::Normal {
            return None;
        }
        let parent = new_blocks.first()?.header.prev_hash;
        let base = self.block_store.get_height_by_hash(&parent).ok().flatten();
        let junk = self.drop_junk_bodies(new_blocks, base).await;
        if let Some(first) = junk.first() {
            warn!(
                "[REORG] Fork block {:.16} has a body that does not match its header ({} such) — dropped, nothing changed (INC-I-281)",
                first,
                junk.len()
            );
            return Some(ReorgStop::JunkBody);
        }
        let (hit, refusal) = new_blocks.iter().find_map(|b| {
            let r = self.refused_reorg_blocks.refusal_of(b)?;
            Some((b, r))
        })?;
        if let Refusal::ApplyFailed(key) = refusal {
            self.evict_cached_body(&hit.hash(), key).await;
            warn!(
                "[REORG] Fork block {:.16} has a body a reorg refused before — dropped, nothing changed (INC-I-281)",
                hit.hash()
            );
            return Some(ReorgStop::JunkBody);
        }
        let branch_tip = new_blocks.last()?.hash();
        warn!(
            "[REORG] Branch tip {:.16} holds fork block {:.16} that a reorg refused before — reorg refused, nothing changed (INC-I-281)",
            branch_tip,
            hit.hash()
        );
        Some(ReorgStop::Remembered { branch_tip })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blk(slot: u32, bitfield: Vec<u8>) -> Block {
        let header = doli_core::BlockHeader {
            version: 1,
            prev_hash: Hash::ZERO,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: slot as u64 * 10,
            slot,
            producer: crypto::PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput { value: vec![] },
            vdf_proof: vdf::VdfProof::empty(),
            missed_producers: Vec::new(),
            data_root: Hash::ZERO,
            fork_id: Hash::ZERO,
        };
        let mut b = Block::new(header, vec![]);
        b.attestation_bitfield = bitfield;
        b
    }

    #[test]
    fn apply_failed_refusal_is_by_content_not_header() {
        let bad = blk(1, vec![0xff]);
        let honest = blk(1, vec![]);
        assert_eq!(bad.hash(), honest.hash());
        let mut set = RefusedReorgBlocks::default();
        assert!(!set.refuses(&bad));
        set.insert(Refusal::ApplyFailed(content_key(&bad)));
        assert!(set.refuses(&bad));
        assert!(!set.refuses(&honest));
    }

    #[test]
    fn unscheduled_refusal_is_by_header_hash() {
        let refused = blk(1, vec![0xff]);
        let mut set = RefusedReorgBlocks::default();
        set.insert(Refusal::Unscheduled(refused.hash()));
        assert!(set.refuses(&blk(1, vec![])));
        assert!(!set.refuses(&blk(2, vec![])));
    }

    #[test]
    fn insert_is_deduplicated_and_fifo_bounded() {
        let first = Refusal::Unscheduled(crypto::hash::hash(b"first"));
        let mut set = RefusedReorgBlocks::default();
        set.insert(first);
        set.insert(first);
        assert_eq!(set.refused.len(), 1);
        for i in 0..REFUSED_REORG_CAP as u32 {
            set.insert(Refusal::ApplyFailed(crypto::hash::hash(&i.to_le_bytes())));
        }
        assert_eq!(set.refused.len(), REFUSED_REORG_CAP);
        assert!(!set.refused.contains(&first));
    }

    fn anchor(seed: u32, height: u64) -> LearnedAnchor {
        LearnedAnchor {
            hash: crypto::hash::hash(&seed.to_le_bytes()),
            height,
            inputs: ScheduleInputs {
                list: vec![],
                weights: vec![],
            },
        }
    }

    #[test]
    fn learned_anchors_are_deduplicated_fifo_bounded_and_keyed_by_height() {
        let mut set = RefusedReorgBlocks::default();
        assert!(!set.has_anchors());
        let first = anchor(0, 8);
        assert!(set.learn(first.clone()));
        assert!(!set.learn(first.clone()), "same hash");
        assert_eq!(set.anchor(&first.hash, 8), Some(&first));
        assert_eq!(set.anchor(&first.hash, 12), None, "other height");
        for i in 1..=LEARNED_ANCHOR_CAP as u32 {
            assert!(set.learn(anchor(i, 8)));
        }
        assert_eq!(set.anchors.len(), LEARNED_ANCHOR_CAP);
        assert_eq!(set.anchor(&first.hash, 8), None, "evicted first");
    }

    #[test]
    fn anchor_of_needs_a_known_height_and_a_held_anchor() {
        let b = blk(1, vec![]);
        let mut set = RefusedReorgBlocks::default();
        assert_eq!(set.anchor_of(&b, Some(8)), None, "no anchors");
        let a = LearnedAnchor {
            hash: b.hash(),
            ..anchor(0, 8)
        };
        set.learn(a.clone());
        assert_eq!(set.anchor_of(&b, Some(8)), Some(&a));
        assert_eq!(set.anchor_of(&b, None), None, "unknown height");
        assert_eq!(set.anchor_of(&blk(2, vec![]), Some(8)), None);
    }

    #[test]
    fn last_boundary_of_the_applied_prefix() {
        let h: Vec<Hash> = (0..6u32)
            .map(|i| crypto::hash::hash(&i.to_le_bytes()))
            .collect();
        // bpe = 4, prefix at heights 7..=12: boundaries 8 and 12.
        assert_eq!(last_boundary(&h, 12, 4), Some((h[5], 12)));
        assert_eq!(last_boundary(&h[..5], 11, 4), Some((h[1], 8)));
        assert_eq!(last_boundary(&h[2..4], 10, 4), None, "heights 9, 10");
        assert_eq!(last_boundary(&[], 10, 4), None);
        assert_eq!(last_boundary(&h, 12, 0), None, "degenerate epoch length");
    }

    #[test]
    fn anchor_covers_the_next_epoch_only() {
        let a = anchor(0, 8);
        assert!(!a.covers(8, 4), "the anchor itself");
        assert!(a.covers(9, 4));
        assert!(a.covers(12, 4), "next boundary block");
        assert!(!a.covers(13, 4), "after the next boundary");
        assert!(!a.covers(7, 4));
        assert!(anchor(0, u64::MAX - 1).covers(u64::MAX, 4), "no overflow");
    }
}
