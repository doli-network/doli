//! INC-I-053: Epoch mode startup sync gate
//!
//! Verifies that after a restart (detected by recent first_peer_connected),
//! epoch-mode production is deferred for a grace period to prevent mass-restart
//! forks where simultaneously-restarted nodes produce for each other before
//! syncing the canonical chain tip.
//!
//! Root cause: bootstrap mode has several sync-before-produce guards
//! (scheduling.rs:90-167), but epoch mode had none — a restarted node
//! produced immediately if it was the designated producer for the current slot.

// OUTPUT CONTRACT: fn should_defer_epoch_production(&self) -> bool
// O1: return — bool, true = defer production, false = proceed
// PATHS:
//   P1: first_peer_connected is None → false (seed node or pre-peer, other guards handle)
//   P2: first_peer_connected is recent (< epoch_startup_grace_secs) → true (defer)
//   P3: first_peer_connected is old (>= epoch_startup_grace_secs) → false (proceed)
// MATRIX: O1×P1=false, O1×P2=true, O1×P3=false

use std::time::{Duration, Instant};

use crypto::KeyPair;
use doli_node::node::Node;
use tempfile::TempDir;

async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    (node, producers, temp)
}

/// P1: No first_peer_connected → should NOT defer (other guards handle pre-peer state)
#[tokio::test]
async fn test_no_peer_connected_no_defer() {
    let (node, _producers, _tmp) = make_node(5).await;
    assert_eq!(node.first_peer_connected, None);
    assert!(
        !node.should_defer_epoch_production(),
        "P1: should NOT defer when first_peer_connected is None"
    );
}

/// P2: Recent first_peer_connected → should defer (startup grace active)
#[tokio::test]
async fn test_recent_peer_connected_defers() {
    let (mut node, _producers, _tmp) = make_node(5).await;
    node.first_peer_connected = Some(Instant::now());
    assert!(
        node.should_defer_epoch_production(),
        "P2: should defer when first_peer_connected is recent (within grace period)"
    );
}

/// P3: Old first_peer_connected → should NOT defer (grace period expired)
#[tokio::test]
async fn test_old_peer_connected_no_defer() {
    let (mut node, _producers, _tmp) = make_node(5).await;
    // Set first_peer_connected to 60 seconds ago (well beyond any grace period)
    node.first_peer_connected = Some(Instant::now() - Duration::from_secs(60));
    assert!(
        !node.should_defer_epoch_production(),
        "P3: should NOT defer when first_peer_connected is old (grace period expired)"
    );
}
