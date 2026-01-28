//! Network Partition and Healing Tests
//!
//! Tests that nodes properly handle network partitions and heal when reconnected.

#[path = "../common/mod.rs"]
mod common;

use std::sync::Arc;

use common::{
    create_coinbase, create_test_block, generate_test_chain, init_test_logging,
    MockPeer, TestNode, TestNodeConfig,
};
use doli_core::BlockHeight;
use crypto::{Hash, KeyPair};
use tempfile::TempDir;

/// Test network partition where two groups build separate chains
#[tokio::test]
async fn test_partition_separate_chains() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();

    // Create two node groups (simulating a network partition)
    let config_a = TestNodeConfig::new(&temp_dir, 0);
    let config_b = TestNodeConfig::new(&temp_dir, 1);

    let node_a = Arc::new(TestNode::new(config_a));
    let node_b = Arc::new(TestNode::new(config_b));

    let producer_a = KeyPair::generate();
    let producer_b = KeyPair::generate();
    let pubkey_hash_a = crypto::hash::hash(producer_a.public_key().as_bytes());
    let pubkey_hash_b = crypto::hash::hash(producer_b.public_key().as_bytes());

    // Both start with same genesis
    let genesis = generate_test_chain(1, &producer_a, 5_000_000_000);
    node_a.add_block(genesis[0].clone()).await.unwrap();
    node_b.add_block(genesis[0].clone()).await.unwrap();

    let common_ancestor = genesis[0].hash();

    // PARTITION: Each node builds its own chain
    // Node A builds 10 blocks
    let mut prev_hash_a = common_ancestor;
    for i in 1..11 {
        let coinbase = create_coinbase(i, &pubkey_hash_a, 5_000_000_000);
        let block = create_test_block(i, prev_hash_a, producer_a.public_key(), vec![coinbase]);
        prev_hash_a = block.hash();
        node_a.add_block(block).await.unwrap();
    }

    // Node B builds 8 blocks
    let mut prev_hash_b = common_ancestor;
    for i in 1..9 {
        let coinbase = create_coinbase(i, &pubkey_hash_b, 5_000_000_001);
        let block = create_test_block(i, prev_hash_b, producer_b.public_key(), vec![coinbase]);
        prev_hash_b = block.hash();
        node_b.add_block(block).await.unwrap();
    }

    // Verify divergent chains
    assert_eq!(node_a.height().await, 10);
    assert_eq!(node_b.height().await, 8);
    assert_ne!(node_a.best_hash().await, node_b.best_hash().await);

    // HEAL: Node B receives Node A's longer chain
    // First, Node B needs to revert its chain to the common ancestor
    node_b.revert_blocks(8).await.unwrap();
    assert_eq!(node_b.height().await, 0);
    assert_eq!(node_b.best_hash().await, common_ancestor);

    // Then sync Node A's blocks to Node B
    let blocks_a = node_a.blocks.read().await;
    let mut sorted_blocks: Vec<_> = blocks_a.values()
        .filter(|b| b.header.prev_hash != Hash::ZERO || b.hash() == common_ancestor)
        .cloned()
        .collect();
    drop(blocks_a);

    sorted_blocks.sort_by_key(|b| b.header.slot);

    for block in sorted_blocks.iter().skip(1) { // Skip genesis which is already there
        node_b.add_block(block.clone()).await.unwrap();
    }

    // Both nodes should now have the same chain
    assert_eq!(node_a.height().await, node_b.height().await);
    assert_eq!(node_a.best_hash().await, node_b.best_hash().await);
}

/// Test healing when partition node has longer chain initially
#[tokio::test]
async fn test_heal_longer_partition_chain() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();

    let config_a = TestNodeConfig::new(&temp_dir, 10);
    let config_b = TestNodeConfig::new(&temp_dir, 11);

    let node_a = Arc::new(TestNode::new(config_a));
    let node_b = Arc::new(TestNode::new(config_b));

    let producer = KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());

    // Common history of 5 blocks
    let common_chain = generate_test_chain(5, &producer, 5_000_000_000);
    for block in &common_chain {
        node_a.add_block(block.clone()).await.unwrap();
        node_b.add_block(block.clone()).await.unwrap();
    }

    let fork_point = common_chain.last().unwrap().hash();

    // PARTITION
    // Node A: adds 5 more blocks (total 10)
    let mut prev_hash = fork_point;
    for i in 5..10 {
        let coinbase = create_coinbase(i, &pubkey_hash, 5_000_000_002);
        let block = create_test_block(i, prev_hash, producer.public_key(), vec![coinbase]);
        prev_hash = block.hash();
        node_a.add_block(block).await.unwrap();
    }

    // Node B: adds 15 more blocks (total 20)
    prev_hash = fork_point;
    let mut longer_chain = Vec::new();
    for i in 5..20 {
        let coinbase = create_coinbase(i, &pubkey_hash, 5_000_000_003);
        let block = create_test_block(i, prev_hash, producer.public_key(), vec![coinbase]);
        prev_hash = block.hash();
        longer_chain.push(block.clone());
        node_b.add_block(block).await.unwrap();
    }

    assert_eq!(node_a.height().await, 9);
    assert_eq!(node_b.height().await, 19);

    // HEAL: Node A discovers Node B has longer chain
    // Revert Node A's post-fork blocks
    node_a.revert_blocks(5).await.unwrap();
    assert_eq!(node_a.height().await, 4);

    // Sync Node B's chain to Node A
    for block in &longer_chain {
        node_a.add_block(block.clone()).await.unwrap();
    }

    assert_eq!(node_a.height().await, 19);
    assert_eq!(node_a.best_hash().await, node_b.best_hash().await);
}

/// Test three-way partition healing
#[tokio::test]
async fn test_three_way_partition() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();

    let config_a = TestNodeConfig::new(&temp_dir, 20);
    let config_b = TestNodeConfig::new(&temp_dir, 21);
    let config_c = TestNodeConfig::new(&temp_dir, 22);

    let node_a = Arc::new(TestNode::new(config_a));
    let node_b = Arc::new(TestNode::new(config_b));
    let node_c = Arc::new(TestNode::new(config_c));

    let producer_a = KeyPair::generate();
    let producer_b = KeyPair::generate();
    let producer_c = KeyPair::generate();
    let pubkey_hash_a = crypto::hash::hash(producer_a.public_key().as_bytes());
    let pubkey_hash_b = crypto::hash::hash(producer_b.public_key().as_bytes());
    let pubkey_hash_c = crypto::hash::hash(producer_c.public_key().as_bytes());

    // Common genesis
    let genesis = generate_test_chain(1, &producer_a, 5_000_000_000);
    node_a.add_block(genesis[0].clone()).await.unwrap();
    node_b.add_block(genesis[0].clone()).await.unwrap();
    node_c.add_block(genesis[0].clone()).await.unwrap();

    let common_ancestor = genesis[0].hash();

    // PARTITION: Three separate chains
    // Node A: 5 blocks
    let mut prev = common_ancestor;
    for i in 1..6 {
        let coinbase = create_coinbase(i, &pubkey_hash_a, 5_000_000_000);
        let block = create_test_block(i, prev, producer_a.public_key(), vec![coinbase]);
        prev = block.hash();
        node_a.add_block(block).await.unwrap();
    }

    // Node B: 8 blocks
    prev = common_ancestor;
    for i in 1..9 {
        let coinbase = create_coinbase(i, &pubkey_hash_b, 5_000_000_001);
        let block = create_test_block(i, prev, producer_b.public_key(), vec![coinbase]);
        prev = block.hash();
        node_b.add_block(block).await.unwrap();
    }

    // Node C: 12 blocks (longest)
    prev = common_ancestor;
    let mut winning_chain = Vec::new();
    for i in 1..13 {
        let coinbase = create_coinbase(i, &pubkey_hash_c, 5_000_000_002);
        let block = create_test_block(i, prev, producer_c.public_key(), vec![coinbase]);
        prev = block.hash();
        winning_chain.push(block.clone());
        node_c.add_block(block).await.unwrap();
    }

    assert_eq!(node_a.height().await, 5);
    assert_eq!(node_b.height().await, 8);
    assert_eq!(node_c.height().await, 12);

    // HEAL: All nodes converge on C's chain (longest)
    // Node A reorgs
    node_a.revert_blocks(5).await.unwrap();
    for block in &winning_chain {
        node_a.add_block(block.clone()).await.unwrap();
    }

    // Node B reorgs
    node_b.revert_blocks(8).await.unwrap();
    for block in &winning_chain {
        node_b.add_block(block.clone()).await.unwrap();
    }

    // All nodes should be in sync
    assert_eq!(node_a.height().await, 12);
    assert_eq!(node_b.height().await, 12);
    assert_eq!(node_c.height().await, 12);

    assert_eq!(node_a.best_hash().await, node_c.best_hash().await);
    assert_eq!(node_b.best_hash().await, node_c.best_hash().await);
}

/// Test partition with UTXO set reconciliation
#[tokio::test]
async fn test_partition_utxo_reconciliation() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();

    let config_a = TestNodeConfig::new(&temp_dir, 30);
    let config_b = TestNodeConfig::new(&temp_dir, 31);

    let node_a = Arc::new(TestNode::new(config_a));
    let node_b = Arc::new(TestNode::new(config_b));

    let producer = KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());

    // Common chain
    let common_chain = generate_test_chain(5, &producer, 5_000_000_000);
    for block in &common_chain {
        node_a.add_block(block.clone()).await.unwrap();
        node_b.add_block(block.clone()).await.unwrap();
    }

    // Check initial UTXO count
    {
        let utxos_a = node_a.utxo_set.read().await;
        let utxos_b = node_b.utxo_set.read().await;
        assert_eq!(utxos_a.len(), 5);
        assert_eq!(utxos_b.len(), 5);
    }

    let fork_point = common_chain.last().unwrap().hash();

    // PARTITION: Both add different blocks
    let mut prev = fork_point;
    let mut chain_a = Vec::new();
    for i in 5..10 {
        let coinbase = create_coinbase(i, &pubkey_hash, 5_000_000_004);
        let block = create_test_block(i, prev, producer.public_key(), vec![coinbase]);
        prev = block.hash();
        chain_a.push(block.clone());
        node_a.add_block(block).await.unwrap();
    }

    prev = fork_point;
    for i in 5..8 {
        let coinbase = create_coinbase(i, &pubkey_hash, 5_000_000_005);
        let block = create_test_block(i, prev, producer.public_key(), vec![coinbase]);
        prev = block.hash();
        node_b.add_block(block).await.unwrap();
    }

    // Different UTXO counts
    {
        let utxos_a = node_a.utxo_set.read().await;
        let utxos_b = node_b.utxo_set.read().await;
        assert_eq!(utxos_a.len(), 10); // 5 + 5
        assert_eq!(utxos_b.len(), 8);  // 5 + 3
    }

    // HEAL: Node B adopts Node A's chain
    node_b.revert_blocks(3).await.unwrap();

    {
        let utxos_b = node_b.utxo_set.read().await;
        assert_eq!(utxos_b.len(), 5); // Back to common ancestor
    }

    for block in &chain_a {
        node_b.add_block(block.clone()).await.unwrap();
    }

    // UTXO sets should match
    {
        let utxos_a = node_a.utxo_set.read().await;
        let utxos_b = node_b.utxo_set.read().await;
        assert_eq!(utxos_a.len(), utxos_b.len());
        assert_eq!(utxos_a.len(), 10);
    }
}

/// Test partition where nodes have very different chain lengths
#[tokio::test]
async fn test_partition_large_length_difference() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();

    let config_a = TestNodeConfig::new(&temp_dir, 40);
    let config_b = TestNodeConfig::new(&temp_dir, 41);

    let node_a = Arc::new(TestNode::new(config_a));
    let node_b = Arc::new(TestNode::new(config_b));

    let producer = KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());

    // Single common block (genesis)
    let genesis = generate_test_chain(1, &producer, 5_000_000_000);
    node_a.add_block(genesis[0].clone()).await.unwrap();
    node_b.add_block(genesis[0].clone()).await.unwrap();

    let common = genesis[0].hash();

    // Node A: 100 blocks
    let mut prev = common;
    let mut long_chain = Vec::new();
    for i in 1..101 {
        let coinbase = create_coinbase(i, &pubkey_hash, 5_000_000_000);
        let block = create_test_block(i, prev, producer.public_key(), vec![coinbase]);
        prev = block.hash();
        long_chain.push(block.clone());
        node_a.add_block(block).await.unwrap();
    }

    // Node B: only 3 blocks
    prev = common;
    for i in 1..4 {
        let coinbase = create_coinbase(i, &pubkey_hash, 5_000_000_006);
        let block = create_test_block(i, prev, producer.public_key(), vec![coinbase]);
        prev = block.hash();
        node_b.add_block(block).await.unwrap();
    }

    assert_eq!(node_a.height().await, 100);
    assert_eq!(node_b.height().await, 3);

    // HEAL
    node_b.revert_blocks(3).await.unwrap();
    assert_eq!(node_b.height().await, 0);

    for block in &long_chain {
        node_b.add_block(block.clone()).await.unwrap();
    }

    assert_eq!(node_b.height().await, 100);
    assert_eq!(node_a.best_hash().await, node_b.best_hash().await);
}

/// Test mock peer partition simulation
#[tokio::test]
async fn test_mock_peer_partition() {
    init_test_logging();

    let mut peer_a = MockPeer::new("peer_a");
    let mut peer_b = MockPeer::new("peer_b");

    // Initially connected
    peer_a.connect();
    peer_b.connect();

    let producer = KeyPair::generate();
    let blocks = generate_test_chain(5, &producer, 5_000_000_000);

    // Exchange blocks while connected
    for block in &blocks {
        peer_a.send_block(block.clone());
        peer_b.receive_block(block.clone());
    }

    assert_eq!(peer_a.blocks_sent.len(), 5);
    assert_eq!(peer_b.blocks_received.len(), 5);

    // PARTITION
    peer_a.disconnect();
    peer_b.disconnect();

    // Blocks sent during partition are lost
    let new_blocks = generate_test_chain(3, &producer, 4_000_000_000);
    for block in &new_blocks {
        peer_a.send_block(block.clone());
    }

    // No new blocks received (disconnected)
    assert_eq!(peer_a.blocks_sent.len(), 5); // Unchanged because disconnected

    // HEAL
    peer_a.connect();
    peer_b.connect();

    for block in &new_blocks {
        peer_a.send_block(block.clone());
        peer_b.receive_block(block.clone());
    }

    assert_eq!(peer_a.blocks_sent.len(), 8);
    assert_eq!(peer_b.blocks_received.len(), 8);
}

/// Test gradual healing (blocks arriving one by one)
#[tokio::test]
async fn test_gradual_healing() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();

    let config = TestNodeConfig::new(&temp_dir, 50);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();

    // Initial chain
    let chain = generate_test_chain(10, &producer, 5_000_000_000);
    for block in &chain {
        node.add_block(block.clone()).await.unwrap();
    }

    // Simulate partition by reverting
    node.revert_blocks(5).await.unwrap();

    // Generate new chain that should be synced
    let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());
    let mut prev = node.best_hash().await;
    let mut new_chain = Vec::new();

    for i in 5..20 {
        let coinbase = create_coinbase(i, &pubkey_hash, 5_000_000_007);
        let block = create_test_block(i, prev, producer.public_key(), vec![coinbase]);
        prev = block.hash();
        new_chain.push(block);
    }

    // Heal gradually - one block at a time
    for (idx, block) in new_chain.iter().enumerate() {
        node.add_block(block.clone()).await.unwrap();

        // Verify incremental progress
        assert_eq!(node.height().await, (5 + idx) as u64);
    }

    assert_eq!(node.height().await, 19);
}
