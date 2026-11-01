//! Fork recovery walk entry points on `SyncManager` (INC-I-235).

use crypto::Hash;
use doli_core::consensus::max_block_size;
use doli_core::validation::verify_block_signature_at;
use libp2p::PeerId;
use tracing::{debug, warn};

use super::SyncManager;
use crate::protocols::SyncRequest;
use crate::sync::fork_recovery::WALK_FULL_BLOCKS;

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

    /// INC-I-293 S-2: evidence peers the active walk moves to when its peer answers
    /// `None` (the node sets them at/above the INC-I-288 gate only).
    pub fn set_fork_walk_handover(&mut self, peers: Vec<PeerId>) {
        self.fork.fork_recovery.set_handover_peers(peers);
    }

    /// Feed a block response to the walk. At `local_height + 1 >= AH` a pending
    /// answer whose signature fails is consumed without advancing the walk; the
    /// fetch fails over on its request timeout. At/above the INC-I-288 gate a body
    /// that does not match its header (INV-FORK-005) is blamed, and it or an answer
    /// that would pass `WALK_FULL_BLOCKS` x `max_block_size(local + 1)` (not blamed)
    /// marks its peer and hands the walk to the next evidence peer (INC-I-293 U-1).
    pub(super) fn walk_intake(&mut self, peer: PeerId, block: Option<doli_core::Block>) -> bool {
        let ah = self.config.inc_i_236_block_signature_activation_height;
        let height = self.local_height.saturating_add(1);
        let refused = block.as_ref().is_some_and(|b| {
            verify_block_signature_at(&b.header, height, ah).is_err()
                && self.fork.fork_recovery.is_pending_answer(peer, b)
        });
        if refused {
            warn!(
                "Fork recovery: refusing walk block from {}: invalid block signature",
                peer
            );
            return true;
        }
        let gated = height >= self.config.inc_i_288_fork_choice_activation_height;
        let pending = block
            .as_ref()
            .filter(|b| gated && self.fork.fork_recovery.is_pending_answer(peer, b));
        if let Some(b) = pending {
            // The walked height is unknown here: either presence arm is accepted, and
            // the height-free aggregate rule refuses a padded unattested body (U-1).
            let ah = self.config.inc_i_178_attestation_bls_activation_height;
            if !b.body_matches_header(ah, None)
                || !doli_core::aggregate_fits_bitfield(
                    &b.attestation_bitfield,
                    &b.aggregate_bls_signature,
                )
            {
                warn!(
                    "Fork recovery: refusing walk block {:.16} from {}: body does not match its header",
                    b.hash(),
                    peer
                );
                self.blame_body_supplier(peer, b.header.slot);
                // No evidence peer left: wait out REQUEST_TIMEOUT as before.
                (self.fork.fork_recovery).hand_over(peer, "sent a refused body for");
                return true;
            }
            let max = WALK_FULL_BLOCKS.saturating_mul(max_block_size(height));
            if !self.fork.fork_recovery.admit_bytes(b, max) {
                warn!(
                    "Fork recovery: walk from {} would hold more than {} body bytes ({} full era blocks) — the peer is marked, not blamed",
                    peer, max, WALK_FULL_BLOCKS
                );
                let tracker = &mut self.fork.fork_recovery;
                if !tracker.hand_over(peer, "overflowed the walk bound at") {
                    tracker.cancel("held body bytes over the walk bound");
                }
                return true;
            }
        }
        self.fork.fork_recovery.handle_block(peer, block)
    }

    /// Record that the walked branch ending at `branch_tip` was not adopted on `local_tip`.
    pub fn mark_fork_branch_rejected(&mut self, branch_tip: Hash, local_tip: Hash) {
        self.fork
            .fork_recovery
            .mark_branch_rejected(branch_tip, local_tip);
    }

    /// (peer, reported best height) of every peer with a known status.
    pub fn peer_heights(&self) -> Vec<(PeerId, u64)> {
        self.peers
            .iter()
            .map(|(p, s)| (*p, s.best_height))
            .collect()
    }

    /// (peer, reported best hash) of every peer with a known status.
    pub fn peer_tips(&self) -> Vec<(PeerId, Hash)> {
        self.peers.iter().map(|(p, s)| (*p, s.best_hash)).collect()
    }

    /// Was this branch already walked and not adopted on this local tip?
    pub fn is_fork_branch_rejected(&self, branch_tip: Hash, local_tip: Hash) -> bool {
        self.fork
            .fork_recovery
            .is_branch_rejected(branch_tip, local_tip)
    }

    /// INC-I-293 U-2: at `local_height + 1 >= inc_i_288` a reply from the walk peer
    /// that answers any request but the walk's own by-hash fetch is not the walk's
    /// answer. An unknown request (`None`) keeps the old rule.
    pub(super) fn reply_skips_walk(&self, peer: PeerId, answered: Option<&SyncRequest>) -> bool {
        let height = self.local_height.saturating_add(1);
        if height < self.config.inc_i_288_fork_choice_activation_height {
            return false;
        }
        let (Some(req), Some(wanted)) = (answered, self.fork.fork_recovery.awaited_from(peer))
        else {
            return false;
        };
        let skips = !matches!(req, SyncRequest::GetBlockByHash { hash } if *hash == wanted);
        if skips {
            debug!(
                "Fork recovery: reply from {} answers {:?}, not the walk's fetch of {:.16} — not the walk's answer",
                peer, req, wanted
            );
        }
        skips
    }

    /// INC-I-293 U-2: the peer's pending request id is the walk's own by-hash fetch.
    pub(super) fn pending_is_walk_fetch(&self, peer: PeerId) -> bool {
        let wanted = self.fork.fork_recovery.awaited_from(peer);
        self.peers
            .get(&peer)
            .and_then(|s| s.pending_request)
            .and_then(|id| self.pipeline.pending_requests.get(&id))
            .is_some_and(|p| {
                matches!(p.request, SyncRequest::GetBlockByHash { hash } if Some(hash) == wanted)
            })
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

    // OUTPUT CONTRACT: fn SyncManager::handle_response_to(peer, answered, Block(None))
    //   with a walk waiting on `peer` above the INC-I-288 gate (INC-I-293 U-2)
    //   O1: is_fork_recovery_active(); O2: the peer's pending_request id and its
    //   pipeline.pending_requests entry
    // PATHS: U answered unknown -> old rule (walk ends); H answered GetBlockByHeight ->
    //   walk kept, walk's pending id kept; X answered GetBlockByHash{other} -> walk kept;
    //   W answered GetBlockByHash{wanted} -> pending id taken
    //   Q reply from a peer the walk does not wait on -> passes through, walk kept
    // MATRIX: U x O1 -> u2_unknown · H x O1 O2, W x O2 -> u2_pending · X x O1 -> u2_other_hash
    //   · Q x O1 O2 O3 -> u2_reply_from_a_peer (O3: returned blocks)
    fn gated_walk() -> (SyncManager, PeerId, Hash) {
        let config = SyncConfig {
            inc_i_288_fork_choice_activation_height: 10,
            ..SyncConfig::default()
        };
        let mut m = SyncManager::new(config, Hash::ZERO);
        m.update_local_tip(20, Hash::from_bytes([1; 32]), 20);
        let p = PeerId::random();
        m.add_peer(p, 20, Hash::ZERO, 20);
        assert!(m.start_fork_recovery(orphan(b"u2"), p));
        let parent = crypto::hash::hash(b"u2");
        let fetch = m.next_request();
        assert!(
            matches!(&fetch, Some((to, SyncRequest::GetBlockByHash { hash })) if *to == p && *hash == parent),
            "setup: the walk asks P for the parent by hash, got {fetch:?}"
        );
        (m, p, parent)
    }

    #[test]
    fn u2_unknown_request_keeps_the_old_rule_above_the_gate() {
        let (mut m, p, _) = gated_walk();
        m.handle_response_to(p, None, crate::protocols::SyncResponse::Block(None));
        assert!(
            !m.is_fork_recovery_active(),
            "U-2: an empty reply whose request is unknown is the walk's answer (old rule)"
        );
    }

    #[test]
    fn u2_pending_walk_fetch_survives_a_reply_to_another_request() {
        let (mut m, p, parent) = gated_walk();
        let id = m.peers[&p]
            .pending_request
            .expect("setup: walk fetch registered");
        let chase = SyncRequest::GetBlockByHeight { height: 21 };
        m.handle_response_to(p, Some(&chase), crate::protocols::SyncResponse::Block(None));
        assert!(m.is_fork_recovery_active(), "O1: the walk survives");
        assert_eq!(
            m.peers[&p].pending_request,
            Some(id),
            "U-2: a reply to GetBlockByHeight took the walk's pending request id"
        );
        assert!(
            m.pipeline.pending_requests.contains_key(&id),
            "O2: entry kept"
        );
        let ask = SyncRequest::GetBlockByHash { hash: parent };
        m.handle_response_to(p, Some(&ask), crate::protocols::SyncResponse::Block(None));
        assert_eq!(
            m.peers[&p].pending_request, None,
            "O2: the walk's answer takes it"
        );
        assert!(!m.pipeline.pending_requests.contains_key(&id));
    }

    #[test]
    fn u2_empty_reply_to_another_by_hash_request_is_not_the_walks() {
        let (mut m, p, _) = gated_walk();
        let other = SyncRequest::GetBlockByHash {
            hash: Hash::from_bytes([9; 32]),
        };
        m.handle_response_to(p, Some(&other), crate::protocols::SyncResponse::Block(None));
        assert!(
            m.is_fork_recovery_active(),
            "U-2: an empty reply to a by-hash request for another hash ended the walk"
        );
    }

    #[test]
    fn u2_reply_from_a_peer_the_walk_does_not_wait_on_passes_through() {
        let (mut m, p, _) = gated_walk();
        let q = PeerId::random();
        m.add_peer(q, 20, Hash::ZERO, 20);
        let id = m.peers[&p].pending_request;
        let blk = orphan(b"other");
        let chase = SyncRequest::GetBlockByHeight { height: 21 };
        let resp = crate::protocols::SyncResponse::Block(Some(blk.clone()));
        let out = m.handle_response_to(q, Some(&chase), resp);
        assert_eq!(out.len(), 1, "O3: Q's block passes through to apply");
        assert!(m.is_fork_recovery_active(), "O1: P's walk is untouched");
        assert_eq!(m.peers[&p].pending_request, id, "O2: P's pending id kept");
    }
}
