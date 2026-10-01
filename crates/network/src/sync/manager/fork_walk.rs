//! Fork recovery walk entry points on `SyncManager` (INC-I-235).

use crypto::Hash;
use libp2p::PeerId;

use super::SyncManager;

/// Alternate peers handed to a walk for REQUEST_TIMEOUT failover.
const MAX_WALK_ALTERNATES: usize = 4;

impl SyncManager {
    /// Start fork recovery for an orphan block.
    /// Walks backward through parent chain requesting blocks from the peer.
    pub fn start_fork_recovery(&mut self, orphan: doli_core::Block, peer: PeerId) -> bool {
        if !self.fork.fork_recovery.start(orphan, peer) {
            return false;
        }
        let mut alternates: Vec<(u64, PeerId)> = self
            .peers
            .iter()
            .filter(|(p, s)| **p != peer && s.best_height > self.local_height)
            .map(|(p, s)| (s.best_height, *p))
            .collect();
        alternates.sort();
        let skip = alternates.len().saturating_sub(MAX_WALK_ALTERNATES);
        self.fork
            .fork_recovery
            .set_alternate_peers(alternates.into_iter().skip(skip).map(|(_, p)| p).collect());
        true
    }

    /// Record that the walked branch ending at `branch_tip` was not adopted on `local_tip`.
    pub fn mark_fork_branch_rejected(&mut self, branch_tip: Hash, local_tip: Hash) {
        self.fork
            .fork_recovery
            .mark_branch_rejected(branch_tip, local_tip);
    }

    /// Was this branch already walked and not adopted on this local tip?
    pub fn is_fork_branch_rejected(&self, branch_tip: Hash, local_tip: Hash) -> bool {
        self.fork
            .fork_recovery
            .is_branch_rejected(branch_tip, local_tip)
    }
}

#[cfg(test)]
mod tests {
    use super::super::SyncConfig;
    use super::*;

    fn orphan(seed: &[u8]) -> doli_core::Block {
        let header = doli_core::BlockHeader {
            version: 1,
            prev_hash: crypto::hash::hash(seed),
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: 10,
            slot: 1,
            producer: crypto::PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput { value: vec![] },
            vdf_proof: vdf::VdfProof::empty(),
            missed_producers: Vec::new(),
            data_root: Hash::ZERO,
            fork_id: Hash::ZERO,
        };
        doli_core::Block::new(header, vec![])
    }

    #[test]
    fn start_walks_from_given_peer_and_refuses_a_second_walk() {
        let mut m = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        m.local_height = 10;
        let (walk, alt, low) = (PeerId::random(), PeerId::random(), PeerId::random());
        m.add_peer(walk, 20, Hash::ZERO, 20);
        m.add_peer(alt, 15, Hash::ZERO, 15);
        m.add_peer(low, 5, Hash::ZERO, 5);
        assert!(m.start_fork_recovery(orphan(b"p1"), walk));
        assert!(!m.start_fork_recovery(orphan(b"p2"), alt));
        let parent = crypto::hash::hash(b"p1");
        assert_eq!(m.fork.fork_recovery.next_fetch(), Some((walk, parent)));
    }
}
