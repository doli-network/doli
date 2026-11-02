//! INC-I-293 U-2: a relayer that answered `None` for a hash is not asked for it
//! again by the attestation refetch (at/above the INC-I-288 gate).

use super::*;
use network::protocols::SyncRequest;

/// A (hash, peer) `None` is remembered as long as a tracker entry lives.
const RETIRED_TTL_SECS: u64 = 30;

impl Node {
    /// The hash a `Block(None)` from `peer` refuses: the hash of the request it
    /// answers (matched by the local outbound request id); none for any other
    /// request; for an unknown request, the walk's pending fetch from `peer`.
    /// Call before the walk consumes the answer.
    pub(super) async fn none_answer_hash(
        &self,
        peer: PeerId,
        answered: Option<&SyncRequest>,
    ) -> Option<Hash> {
        match answered {
            Some(SyncRequest::GetBlockByHash { hash }) => Some(*hash),
            Some(_) => None,
            None => (self.sync_manager.read().await.fork_walk_pending_fetch())
                .filter(|(p, _)| *p == peer)
                .map(|(_, h)| h),
        }
    }

    /// `peer` answered `None` for `hash`; `None` when no hash can be named.
    pub(super) async fn retire_attest_fetch(&mut self, peer: PeerId, hash: Option<Hash>) {
        let Some(hash) = hash else { return };
        if !self.weighs_by_support().await {
            return;
        }
        let now = Instant::now();
        self.attest_fetch_retired
            .retain(|_, t| now.duration_since(*t).as_secs() < RETIRED_TTL_SECS);
        self.attest_fetch_retired.insert((hash, peer), now);
    }

    /// The refetch target for `hash`: the recorded relayer, unless it answered
    /// `None` for `hash`; then a connected peer reporting the relayer's off-chain
    /// tip that did not answer `None` for it (the 103 inheritance rule). The
    /// tracker entry moves to that heir; with no heir it is removed.
    pub(super) async fn attest_refetch_peer(
        &mut self,
        hash: Hash,
        recorded: PeerId,
    ) -> Option<PeerId> {
        let retired = |p: &PeerId| self.attest_fetch_retired.contains_key(&(hash, *p));
        if !retired(&recorded) {
            return self.attest_fetch_peer(recorded).await;
        }
        let heir = {
            let sm = self.sync_manager.read().await;
            let tip = sm.peer_tips().into_iter().find(|(p, _)| *p == recorded);
            tip.and_then(|(_, t)| self.branch_peer_except(&sm, &recorded, Some(t), retired))
        };
        if let Some(h) = heir {
            if let Some(entry) = self.attest_fetch_tracker.get_mut(&hash) {
                entry.2 = h;
            }
        } else {
            self.attest_fetch_tracker.remove(&hash);
        }
        heir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // OUTPUT CONTRACT: fn Node::retire_attest_fetch(peer, Option<hash>) then
    //   Node::attest_refetch_peer(hash, relayer) -> Option<PeerId>
    //   O1: refetch target for (hash, relayer)  O2: attest_fetch_retired entries
    //   O3: attest_fetch_tracker entry for hash: kept / moved to the heir / removed
    // PATHS: hash None -> no-op | below gate -> no-op | retired + heir (same
    //   off-chain tip) -> heir | retired, no heir left -> None | other hash -> relayer
    // MATRIX: above gate x {None, Some(h) heir, Some(h) no heir, other hash} x O1 O2
    //   -> u2_walk_none_retires_the_relayer_for_that_hash · below gate x O1 O2 -> u2_below_gate
    //   (O1 O2 O3) · fn Node::none_answer_hash(peer, answered) -> Option<Hash> (O4:
    //   refused hash): answered {by-hash, by-height, unknown + no walk} -> u2_none_answer_*
    async fn node_with_peers(gate: u64) -> (Node, tempfile::TempDir, PeerId, PeerId) {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut node = Node::new_for_test(tmp.path().to_path_buf(), vec![KeyPair::generate()])
            .await
            .expect("Node::new_for_test failed");
        let genesis = node.chain_state.read().await.genesis_hash;
        let config = SyncConfig {
            inc_i_288_fork_choice_activation_height: gate,
            ..SyncConfig::default()
        };
        let mut sm = SyncManager::new(config, genesis);
        let (p, p2) = (PeerId::random(), PeerId::random());
        let off_chain_tip = crypto::hash::hash(b"u2 off-chain tip");
        sm.add_peer(p, 50, off_chain_tip, 50);
        sm.add_peer(p2, 50, off_chain_tip, 50);
        node.sync_manager = Arc::new(RwLock::new(sm));
        (node, tmp, p, p2)
    }

    // U2-WALK INC-I-293 SEC-FIXVERIFY-647-104 (Must) — Decision: a failure means a walk
    // None does not stop the refetch from asking the same peer for the same hash.
    #[tokio::test]
    async fn u2_walk_none_retires_the_relayer_for_that_hash() {
        let (mut node, _tmp, p, p2) = node_with_peers(0).await;
        assert!(node.weighs_by_support().await, "fixture: above the gate");
        let (h, other) = (crypto::hash::hash(b"h"), crypto::hash::hash(b"other"));
        node.retire_attest_fetch(p, None).await;
        assert_eq!(
            node.attest_refetch_peer(h, p).await,
            Some(p),
            "unattributed"
        );
        node.retire_attest_fetch(p, Some(h)).await;
        node.attest_fetch_tracker.insert(h, (Instant::now(), 0, p));
        assert_eq!(node.attest_refetch_peer(h, p).await, Some(p2), "heir");
        assert_eq!(
            node.attest_fetch_tracker[&h].2, p2,
            "O3: entry moved to the heir"
        );
        assert_eq!(
            node.attest_refetch_peer(other, p).await,
            Some(p),
            "other hash"
        );
        node.retire_attest_fetch(p2, Some(h)).await;
        assert_eq!(node.attest_refetch_peer(h, p).await, None, "no heir left");
        assert!(
            !node.attest_fetch_tracker.contains_key(&h),
            "O3: entry removed"
        );
        assert_eq!(node.attest_fetch_retired.len(), 2, "O2: two refusals held");
    }

    // U2-ATTRIB INC-I-293 (Must) — Decision: a failure means a `None` is charged to
    // a hash other than the one its request asked for.
    #[tokio::test]
    async fn u2_none_answer_names_the_requested_hash_only() {
        let (node, _tmp, p, _) = node_with_peers(0).await;
        let h = crypto::hash::hash(b"h");
        let by_hash = SyncRequest::GetBlockByHash { hash: h };
        let by_height = SyncRequest::GetBlockByHeight { height: 7 };
        assert_eq!(node.none_answer_hash(p, Some(&by_hash)).await, Some(h));
        assert_eq!(node.none_answer_hash(p, Some(&by_height)).await, None);
        assert_eq!(node.none_answer_hash(p, None).await, None, "no walk");
    }

    // U2-PIN INC-I-293 (Must, pin) — Decision: a failure means a below-gate node
    // records refusals the old one-shot fetch never had.
    #[tokio::test]
    async fn u2_below_gate_retire_is_a_noop() {
        let (mut node, _tmp, p, _) = node_with_peers(u64::MAX).await;
        let h = crypto::hash::hash(b"h");
        node.retire_attest_fetch(p, Some(h)).await;
        assert!(node.attest_fetch_retired.is_empty(), "O2: nothing recorded");
        assert_eq!(node.attest_refetch_peer(h, p).await, Some(p), "O1: relayer");
    }
}
