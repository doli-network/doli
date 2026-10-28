//! INC-I-281: the schedule list a recovery door judges a block with.
//!
//! A node on a short dead branch that holds the epoch-boundary block has
//! already rotated to the next list. A competing block at a height of the
//! closing epoch is judged with the list that judged that height on the
//! node's own branch: the pre-rotation snapshot in the undo record of the
//! node's boundary block. A block after a boundary block that only its branch
//! holds has no locally derivable list: the door skips its round-robin arm and
//! execute_reorg checks it at its parent (`reorg_schedule.rs`).

use crypto::{Hash, PublicKey};
use doli_core::{Block, EpochState};
use tracing::{debug, warn};

use crate::node::Node;

/// True when the list for a block at `height` on a branch that leaves the
/// node's chain after `fork_point` is derivable: no boundary in
/// `(fork_point, height)`.
fn schedule_derivable(fork_point: u64, height: u64, blocks_per_epoch: u64) -> bool {
    if blocks_per_epoch == 0 {
        return true;
    }
    let next_boundary = (fork_point / blocks_per_epoch)
        .saturating_add(1)
        .saturating_mul(blocks_per_epoch);
    next_boundary >= height
}

/// (height, fork point) of a block whose ancestors down to the node's chain
/// are in `cached`; `None` when the chain does not connect.
fn position_via(
    block: &Block,
    cached: impl Fn(&Hash) -> Option<Hash>,
    canonical_height: impl Fn(&Hash) -> Option<u64>,
    max_depth: u64,
) -> Option<(u64, u64)> {
    let mut prev = block.header.prev_hash;
    for depth in 1..=max_depth {
        if let Some(h) = canonical_height(&prev) {
            return Some((h.saturating_add(depth), h));
        }
        prev = cached(&prev)?;
    }
    None
}

/// Why the closing-epoch list could not be read from the undo record.
#[derive(Debug, PartialEq, Eq)]
enum Unavailable {
    Missing,
    Undecodable(String),
}

/// The boundary whose undo record holds the list that judges `height` for a
/// node at `tip`, or `None` when the node's current list applies. The block
/// at boundary `B` (`B % bpe == 0`) is judged by the closing list; the
/// rotation runs after it, so the closing epoch is `(B - bpe, B]`.
fn closing_boundary_for(height: u64, tip: u64, blocks_per_epoch: u64) -> Option<u64> {
    if blocks_per_epoch == 0 {
        return None;
    }
    let boundary = tip / blocks_per_epoch * blocks_per_epoch;
    let closing = boundary > 0 && height <= boundary && height > boundary - blocks_per_epoch;
    closing.then_some(boundary)
}

fn effective_list(es: &EpochState) -> Vec<PublicKey> {
    if es.active_list.is_empty() {
        es.producer_list.clone()
    } else {
        es.active_list.clone()
    }
}

/// The effective list of an undo record's pre-block epoch-state snapshot.
fn closing_list(snapshot: Option<&[u8]>) -> Result<Vec<PublicKey>, Unavailable> {
    let bytes = snapshot.ok_or(Unavailable::Missing)?;
    let es = EpochState::deserialize(bytes).map_err(|e| Unavailable::Undecodable(e.to_string()))?;
    Ok(effective_list(&es))
}

impl Node {
    /// The epoch list a door feeds the schedule check for a block at
    /// `block_height` (`None` = unknown: the node's current list).
    pub(super) async fn door_epoch_list(&self, block_height: Option<u64>) -> Vec<PublicKey> {
        let tip = self.chain_state.read().await.best_height;
        let bpe = self.config.network.blocks_per_reward_epoch();
        let Some(boundary) = block_height.and_then(|h| closing_boundary_for(h, tip, bpe)) else {
            return effective_list(&self.epoch_state);
        };
        let undo = self.state_db.get_undo(boundary);
        match closing_list(undo.and_then(|u| u.epoch_state_snapshot).as_deref()) {
            Ok(list) => list,
            Err(Unavailable::Missing) => {
                debug!(
                    "[DOOR_EPOCH] no undo epoch state at boundary h={} for h={:?} — current list (INC-I-281)",
                    boundary, block_height
                );
                effective_list(&self.epoch_state)
            }
            Err(Unavailable::Undecodable(e)) => {
                warn!(
                    "[DOOR_EPOCH] undo epoch state at boundary h={} undecodable ({}) — current list (INC-I-281)",
                    boundary, e
                );
                effective_list(&self.epoch_state)
            }
        }
    }

    /// Whether a door judges the round-robin of a block at `height` whose
    /// branch leaves the node's chain after `fork_point` (unknown: judged).
    pub(super) fn door_judges_schedule(
        &self,
        height: Option<u64>,
        fork_point: Option<u64>,
    ) -> bool {
        let bpe = self.config.network.blocks_per_reward_epoch();
        match (height, fork_point) {
            (Some(h), Some(f)) => schedule_derivable(f, h, bpe),
            _ => true,
        }
    }

    /// (height, fork point) of a gossip sibling: its parent is in the node's
    /// chain or, through fork-cache bodies, descends from a block that is.
    pub(crate) async fn door_position(&self, block: &Block) -> Option<(u64, u64)> {
        let cache = self.fork_block_cache.read().await;
        position_via(
            block,
            |h| cache.get(h).map(|b| b.header.prev_hash),
            |h| self.block_store.get_height_by_hash(h).ok().flatten(),
            cache.len() as u64 + 1,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::KeyPair;

    #[test]
    fn closing_epoch_maps_to_the_nodes_last_boundary() {
        // bpe = 4: boundary 8 is judged by the list rotated at 4.
        assert_eq!(closing_boundary_for(8, 9, 4), Some(8));
        assert_eq!(closing_boundary_for(5, 9, 4), Some(8));
        assert_eq!(closing_boundary_for(8, 8, 4), Some(8));
        assert_eq!(closing_boundary_for(4, 9, 4), None, "two epochs back");
        assert_eq!(closing_boundary_for(9, 9, 4), None, "current epoch");
        assert_eq!(
            closing_boundary_for(13, 9, 4),
            None,
            "after a later boundary"
        );
        assert_eq!(
            closing_boundary_for(8, 7, 4),
            None,
            "tip before the boundary"
        );
        assert_eq!(closing_boundary_for(3, 3, 4), None, "no rotation yet");
        assert_eq!(
            closing_boundary_for(8, 9, 0),
            None,
            "degenerate epoch length"
        );
    }

    #[test]
    fn schedule_derivable_until_a_branch_only_boundary() {
        // bpe = 4, fork point F = 7: the branch's boundary block is at 8.
        assert!(schedule_derivable(7, 8, 4), "boundary block: closing list");
        assert!(!schedule_derivable(7, 9, 4), "after the branch's boundary");
        assert!(!schedule_derivable(7, 13, 4), "two boundaries later");
        assert!(schedule_derivable(8, 9, 4), "boundary is the fork point");
        assert!(schedule_derivable(8, 12, 4), "next boundary block");
        assert!(!schedule_derivable(8, 13, 4), "after the next boundary");
        assert!(schedule_derivable(5, 6, 4), "mid-epoch");
        assert!(schedule_derivable(7, 9, 0), "degenerate epoch length");
        assert!(schedule_derivable(u64::MAX - 1, u64::MAX, 4), "no overflow");
    }

    fn blk(prev: Hash, slot: u32) -> Block {
        let header = doli_core::BlockHeader {
            version: 1,
            prev_hash: prev,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: slot as u64 * 10,
            slot,
            producer: PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput { value: vec![] },
            vdf_proof: vdf::VdfProof::empty(),
            missed_producers: Vec::new(),
            data_root: Hash::ZERO,
            fork_id: Hash::ZERO,
        };
        Block::new(header, vec![])
    }

    #[test]
    fn position_walks_cached_ancestors_to_the_chain() {
        let f = crypto::hash::hash(b"f");
        let c1 = blk(f, 1);
        let c2 = blk(c1.hash(), 2);
        let cache = std::collections::HashMap::from([(c1.hash(), c1.clone())]);
        let parent = |h: &Hash| cache.get(h).map(|b| b.header.prev_hash);
        let stored = |h: &Hash| (*h == f).then_some(7);
        assert_eq!(position_via(&c1, parent, stored, 2), Some((8, 7)));
        assert_eq!(position_via(&c2, parent, stored, 2), Some((9, 7)));
        assert_eq!(position_via(&c2, parent, stored, 1), None, "depth bound");
        let orphan = blk(crypto::hash::hash(b"unknown"), 3);
        assert_eq!(position_via(&orphan, parent, stored, 4), None);
    }

    #[test]
    fn closing_list_reads_active_then_producer_list() {
        let pks: Vec<PublicKey> = (0..3).map(|_| *KeyPair::generate().public_key()).collect();
        let mut es = EpochState::genesis();
        es.producer_list = pks.clone();
        assert_eq!(closing_list(Some(&es.serialize())), Ok(pks.clone()));
        es.active_list = pks[..2].to_vec();
        assert_eq!(closing_list(Some(&es.serialize())), Ok(pks[..2].to_vec()));
        assert_eq!(closing_list(None), Err(Unavailable::Missing));
        assert!(matches!(
            closing_list(Some(&[0xff, 0x01])),
            Err(Unavailable::Undecodable(_))
        ));
    }
}
