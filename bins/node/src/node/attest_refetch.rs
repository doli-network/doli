//! INC-I-293 U-2: a relayer that answered `None` for a hash is not asked for it
//! again by the attestation refetch (at/above the INC-I-288 gate).

use super::*;

/// A (hash, peer) `None` is remembered as long as a tracker entry lives.
const RETIRED_TTL_SECS: u64 = 30;

impl Node {
    /// `peer` answered `None` for `hash`. `hash` is `None` when the answer cannot
    /// be attributed to a request (a refetch reply carries no request today).
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
    /// tip that did not answer `None` for it (the 103 inheritance rule), else none.
    pub(super) async fn attest_refetch_peer(&self, hash: Hash, recorded: PeerId) -> Option<PeerId> {
        let retired = |p: &PeerId| self.attest_fetch_retired.contains_key(&(hash, *p));
        if !retired(&recorded) {
            return self.attest_fetch_peer(recorded).await;
        }
        let sm = self.sync_manager.read().await;
        let (_, tip) = sm.peer_tips().into_iter().find(|(p, _)| *p == recorded)?;
        self.branch_peer_except(&sm, &recorded, Some(tip), retired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // OUTPUT CONTRACT: fn Node::retire_attest_fetch(peer, Option<hash>) then
    //   Node::attest_refetch_peer(hash, relayer) -> Option<PeerId>
    //   O1: refetch target for (hash, relayer)  O2: attest_fetch_retired entries
    // PATHS: hash None -> no-op | below gate -> no-op | retired + heir (same
    //   off-chain tip) -> heir | retired, no heir left -> None | other hash -> relayer
    // MATRIX: above gate x {None, Some(h) heir, Some(h) no heir, other hash} x O1 O2
    //   -> u2_walk_none_retires_the_relayer_for_that_hash · below gate x O1 O2 -> u2_below_gate
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
        assert_eq!(node.attest_refetch_peer(h, p).await, Some(p2), "heir");
        assert_eq!(
            node.attest_refetch_peer(other, p).await,
            Some(p),
            "other hash"
        );
        node.retire_attest_fetch(p2, Some(h)).await;
        assert_eq!(node.attest_refetch_peer(h, p).await, None, "no heir left");
        assert_eq!(node.attest_fetch_retired.len(), 2, "O2: two refusals held");
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
