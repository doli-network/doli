//! INC-I-068: Fully-delegated producers must NOT attest or be scheduled.
//!
//! A producer who delegates ALL bonds has selection_weight=0. They should:
//! 1. NOT attest (create_and_broadcast_attestation returns None)
//! 2. NOT be scheduled for block production (excluded from active_list)
//!
//! The delegation design: bond holders who don't want to run infrastructure
//! delegate production to another producer. The delegator's node becomes
//! irrelevant to the network. They receive passive rewards (90% to bond owner,
//! 10% to operator) without participating in consensus.

use crypto::{Hash, KeyPair};
use doli_node::node::Node;
use tempfile::TempDir;

// OUTPUT CONTRACT: fn create_and_broadcast_attestation(block_hash, slot, height)
// O1: Option<Attestation> — Some when producer should attest, None when not
// PATHS:
//   P1: active producer, weight > 0 → Some(attestation) with correct weight
//   P2: active producer, weight = 0 (fully delegated) → None (delegators don't attest)
//   P3: non-producer node → None
// INPUT PARTITIONS:
//   P1: {bond_count=1, delegated_bonds=0} — standard active producer
//   P2: {bond_count=1, delegated_bonds=1} — fully delegated producer (weight=0)
//   P3: {producer_key=None} — node without producer key
// MATRIX:
//   O1×P1: Some(att), att.attester_weight > 0
//   O1×P2: None — delegators do not participate in consensus
//   O1×P3: None

async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    (node, producers, temp)
}

/// P1: Active producer with weight > 0 attests successfully.
#[tokio::test]
async fn active_producer_with_bonds_attests() {
    let (node, producers, _tmp) = make_node(3).await;
    let block_hash = Hash::from_bytes([1u8; 32]);

    let result = node
        .create_and_broadcast_attestation(block_hash, 1, 1)
        .await;

    assert!(
        result.is_some(),
        "Active producer with bonds should create attestation"
    );
    let att = result.unwrap();
    assert_eq!(att.block_hash, block_hash);
    assert_eq!(att.attester, *producers[0].public_key());
    assert!(
        att.attester_weight > 0,
        "Weight should be > 0 for undelegated bonds"
    );
}

/// P2: Fully-delegated producer (weight=0) must NOT attest.
///
/// A delegator has opted out of consensus participation. Their node is
/// irrelevant. Attesting with weight=0 is wasteful and confusing (shows
/// 0% attestation in explorer when they shouldn't be expected to attest).
#[tokio::test]
async fn fully_delegated_producer_does_not_attest() {
    let (node, producers, _tmp) = make_node(3).await;
    let block_hash = Hash::from_bytes([2u8; 32]);

    // Simulate full bond delegation: set delegated_bonds = bond_count
    {
        let mut ps = node.producer_set.write().await;
        let info = ps
            .get_by_pubkey_mut(producers[0].public_key())
            .expect("producer should exist");
        assert!(
            info.is_active(),
            "producer should be active before delegation"
        );
        info.delegated_bonds = info.bond_count;
        // Verify precondition: weight is now 0 but status is still Active
        assert_eq!(
            info.selection_weight_at(1, 0),
            0,
            "weight should be 0 after full delegation"
        );
        assert!(info.is_active(), "status should still be Active");
    }

    let result = node
        .create_and_broadcast_attestation(block_hash, 1, 1)
        .await;

    // Fully-delegated producers must NOT attest — they opted out of consensus
    assert!(
        result.is_none(),
        "Fully-delegated producer (weight=0) must NOT create attestation"
    );
}
