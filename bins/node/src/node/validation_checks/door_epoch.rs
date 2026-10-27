//! INC-I-281: the schedule list a recovery door judges a block with.
//!
//! A node on a short dead branch that holds the epoch-boundary block has
//! already rotated to the next list. A competing block at a height of the
//! closing epoch is judged with the list that judged that height on the
//! node's own branch: the pre-rotation snapshot in the undo record of the
//! node's boundary block. Heights after the node's boundary keep the current
//! list (a later canonical boundary's list is not derivable here).

use crypto::PublicKey;
use doli_core::EpochState;
use tracing::{debug, warn};

use crate::node::Node;

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
