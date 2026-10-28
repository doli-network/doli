//! What a Normal reorg refuses before its rollback: a fork block whose body
//! does not match its header, and a fork block that a reorg refused before.

use std::collections::VecDeque;

use super::reorg_restore::{ReorgMode, ReorgStop};
use super::*;

/// Refusals remembered (FIFO).
const REFUSED_REORG_CAP: usize = 256;

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

/// Fork blocks a reorg refused.
#[derive(Debug, Default)]
pub struct RefusedReorgBlocks(VecDeque<Refusal>);

impl RefusedReorgBlocks {
    pub(super) fn insert(&mut self, refusal: Refusal) {
        if self.0.contains(&refusal) {
            return;
        }
        self.0.push_back(refusal);
        if self.0.len() > REFUSED_REORG_CAP {
            self.0.pop_front();
        }
    }

    fn refuses(&self, block: &Block) -> bool {
        if self.0.contains(&Refusal::Unscheduled(block.hash())) {
            return true;
        }
        self.0.iter().any(|r| matches!(r, Refusal::ApplyFailed(_)))
            && self.0.contains(&Refusal::ApplyFailed(content_key(block)))
    }
}

/// `true` when some block's body does not match its header's merkle root.
pub(super) fn has_junk_body(blocks: &[Block]) -> bool {
    blocks.iter().any(|b| !b.verify_merkle_root())
}

impl Node {
    /// A Normal reorg stops here, before its rollback, on a junk body or a refused block.
    pub(super) async fn pre_rollback_stop(
        &self,
        new_blocks: &[Block],
        mode: ReorgMode,
    ) -> Option<ReorgStop> {
        if mode != ReorgMode::Normal {
            return None;
        }
        let junk: Vec<Hash> = new_blocks
            .iter()
            .filter(|b| !b.verify_merkle_root())
            .map(Block::hash)
            .collect();
        if let Some(first) = junk.first() {
            let mut cache = self.fork_block_cache.write().await;
            for hash in &junk {
                cache.remove(hash);
            }
            warn!(
                "[REORG] Fork block {:.16} has a body that does not match its header ({} such) — dropped, nothing changed (INC-I-281)",
                first,
                junk.len()
            );
            return Some(ReorgStop::JunkBody);
        }
        let hit = new_blocks
            .iter()
            .find(|b| self.refused_reorg_blocks.refuses(b))?;
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
        assert_eq!(set.0.len(), 1);
        for i in 0..REFUSED_REORG_CAP as u32 {
            set.insert(Refusal::ApplyFailed(crypto::hash::hash(&i.to_le_bytes())));
        }
        assert_eq!(set.0.len(), REFUSED_REORG_CAP);
        assert!(!set.0.contains(&first));
    }
}
