//! Regression test: active producers with delegated bonds must still attest.
//!
//! Bug: create_and_broadcast_attestation() used selection_weight==0 as a proxy
//! for "not active", but producers who delegated all their bonds are active
//! with weight=0. This prevented them from attesting, causing 0% qualification
//! and no epoch rewards.
//!
//! Fix: check is_active() instead of weight==0.

use crypto::{Hash, KeyPair};
use doli_node::node::Node;
use tempfile::TempDir;

// OUTPUT CONTRACT: fn create_and_broadcast_attestation(block_hash, slot, height)
// O1: Option<Attestation> — Some when producer is active, None when not
// PATHS:
//   P1: active producer, weight > 0 → Some(attestation) with correct weight
//   P2: active producer, weight = 0 (delegated bonds) → Some(attestation) with weight=0
//   P3: non-producer node → None
// INPUT PARTITIONS:
//   P1: {bond_count=1, delegated_bonds=0} — standard active producer
//   P2: {bond_count=1, delegated_bonds=1} — fully delegated active producer
//   P3: {producer_key=None} — node without producer key
// MATRIX:
//   O1×P1: Some(att), att.attester_weight > 0
//   O1×P2: Some(att), att.attester_weight == 0
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

/// P2: Active producer with weight=0 (all bonds delegated) MUST still attest.
/// This is the core regression: weight==0 was used as a proxy for "not active",
/// but delegation reduces weight without deactivating the producer.
#[tokio::test]
async fn active_producer_with_delegated_bonds_still_attests() {
    let (node, producers, _tmp) = make_node(3).await;
    let block_hash = Hash::from_bytes([2u8; 32]);

    // Simulate full bond delegation: set delegated_bonds = bond_count
    // This makes selection_weight_at() return 0, but the producer is still active.
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
        // Verify precondition: weight is now 0 but producer is active
        assert_eq!(
            info.selection_weight_at(1, 0),
            0,
            "weight should be 0 after full delegation"
        );
        assert!(
            info.is_active(),
            "producer should still be active after delegation"
        );
    }

    let result = node
        .create_and_broadcast_attestation(block_hash, 1, 1)
        .await;

    // CRITICAL assertion: active producers MUST attest regardless of weight
    assert!(
        result.is_some(),
        "Active producer with delegated bonds (weight=0) must still create attestation"
    );
    let att = result.unwrap();
    assert_eq!(att.block_hash, block_hash);
    assert_eq!(att.attester, *producers[0].public_key());
    assert_eq!(
        att.attester_weight, 0,
        "Attestation weight should be 0 (bonds delegated, correct for finality)"
    );
}
