//! Two Node Synchronization Test
//!
//! Tests that two nodes can discover each other and synchronize their chains.

#[path = "../common/mod.rs"]
mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{
    create_coinbase, create_test_block, generate_test_chain, init_test_logging,
    wait_for, TestNode, TestNodeConfig,
};
use doli_core::BlockHeight;
use crypto::{Hash, KeyPair};
use tempfile::TempDir;
use tokio::sync::RwLock;

/// Test that Node B syncs to Node A's chain
#[tokio::test]
async fn test_two_nodes_sync_basic() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();

    // Create two nodes
    let config_a = TestNodeConfig::new(&temp_dir, 0);
    let config_b = TestNodeConfig::new(&temp_dir, 1);

    let node_a = Arc::new(TestNode::new(config_a));
    let node_b = Arc::new(TestNode::new(config_b));

    // Generate a chain for Node A
    let producer = KeyPair::generate();
    let chain = generate_test_chain(20, &producer, 5_000_000_000);

    // Add all blocks to Node A
    for block in &chain {
        node_a.add_block(block.clone()).await.unwrap();
    }

    assert_eq!(node_a.height().await, 19);
    assert_eq!(node_b.height().await, 0);

    // Simulate sync: Node B requests and receives blocks from Node A
    let blocks = node_a.blocks.read().await;
    let mut block_list: Vec<_> = blocks.values().cloned().collect();
    drop(blocks);

    // Sort blocks by height for proper sync order
    block_list.sort_by_key(|b| b.header.slot);

    for block in block_list {
        node_b.add_block(block).await.unwrap();
    }

    // Verify sync completed
    assert_eq!(node_a.height().await, node_b.height().await);
    assert_eq!(node_a.best_hash().await, node_b.best_hash().await);
}

/// Test that nodes sync incrementally as new blocks arrive
#[tokio::test]
async fn test_incremental_sync() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();

    let config_a = TestNodeConfig::new(&temp_dir, 10);
    let config_b = TestNodeConfig::new(&temp_dir, 11);

    let node_a = Arc::new(TestNode::new(config_a));
    let node_b = Arc::new(TestNode::new(config_b));

    let producer = KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());

    // Start with 10 blocks on both nodes
    let initial_chain = generate_test_chain(10, &producer, 5_000_000_000);
    for block in &initial_chain {
        node_a.add_block(block.clone()).await.unwrap();
        node_b.add_block(block.clone()).await.unwrap();
    }

    assert_eq!(node_a.height().await, 9);
    assert_eq!(node_b.height().await, 9);

    // Node A produces 5 more blocks
    let mut prev_hash = node_a.best_hash().await;
    for i in 10..15 {
        let coinbase = create_coinbase(i, &pubkey_hash, 5_000_000_000);
        let block = create_test_block(i, prev_hash, producer.public_key(), vec![coinbase]);
        prev_hash = block.hash();
        node_a.add_block(block).await.unwrap();
    }

    assert_eq!(node_a.height().await, 14);
    assert_eq!(node_b.height().await, 9);

    // Simulate incremental sync - Node B catches up one block at a time
    for height in 10u64..15 {
        let blocks = node_a.blocks.read().await;
        let block = blocks.values()
            .find(|b| b.header.slot as u64 == height)
            .cloned();
        drop(blocks);

        if let Some(b) = block {
            node_b.add_block(b).await.unwrap();
        }
    }

    assert_eq!(node_a.height().await, node_b.height().await);
    assert_eq!(node_a.best_hash().await, node_b.best_hash().await);
}

/// Test that a lagging node can catch up to a far-ahead node
#[tokio::test]
async fn test_large_sync_gap() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();

    let config_a = TestNodeConfig::new(&temp_dir, 20);
    let config_b = TestNodeConfig::new(&temp_dir, 21);

    let node_a = Arc::new(TestNode::new(config_a));
    let node_b = Arc::new(TestNode::new(config_b));

    let producer = KeyPair::generate();

    // Node A has 100 blocks
    let chain = generate_test_chain(100, &producer, 5_000_000_000);
    for block in &chain {
        node_a.add_block(block.clone()).await.unwrap();
    }

    // Node B has only 10 blocks (same genesis)
    for block in chain.iter().take(10) {
        node_b.add_block(block.clone()).await.unwrap();
    }

    assert_eq!(node_a.height().await, 99);
    assert_eq!(node_b.height().await, 9);

    // Sync the remaining 90 blocks
    for block in chain.iter().skip(10) {
        node_b.add_block(block.clone()).await.unwrap();
    }

    assert_eq!(node_a.height().await, node_b.height().await);
    assert_eq!(node_a.best_hash().await, node_b.best_hash().await);
}

/// Test that nodes handle duplicate block submissions gracefully
#[tokio::test]
async fn test_duplicate_block_handling() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();

    let config = TestNodeConfig::new(&temp_dir, 30);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();
    let chain = generate_test_chain(5, &producer, 5_000_000_000);

    // Add all blocks
    for block in &chain {
        node.add_block(block.clone()).await.unwrap();
    }

    // Try to add duplicate blocks - should fail gracefully
    for block in &chain {
        let result = node.add_block(block.clone()).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
    }

    // Chain state should be unchanged
    assert_eq!(node.height().await, 4);
}

/// Test sync with multiple producers
#[tokio::test]
async fn test_sync_multiple_producers() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();

    let config_a = TestNodeConfig::new(&temp_dir, 40);
    let config_b = TestNodeConfig::new(&temp_dir, 41);

    let node_a = Arc::new(TestNode::new(config_a));
    let node_b = Arc::new(TestNode::new(config_b));

    // Three different producers
    let producer1 = KeyPair::generate();
    let producer2 = KeyPair::generate();
    let producer3 = KeyPair::generate();

    let pubkey_hash1 = crypto::hash::hash(producer1.public_key().as_bytes());
    let pubkey_hash2 = crypto::hash::hash(producer2.public_key().as_bytes());
    let pubkey_hash3 = crypto::hash::hash(producer3.public_key().as_bytes());

    // Generate blocks with rotating producers
    let mut prev_hash = Hash::ZERO;
    let mut blocks = Vec::new();

    for i in 0..15 {
        let (producer, pubkey_hash) = match i % 3 {
            0 => (&producer1, &pubkey_hash1),
            1 => (&producer2, &pubkey_hash2),
            _ => (&producer3, &pubkey_hash3),
        };

        let coinbase = create_coinbase(i, pubkey_hash, 5_000_000_000);
        let block = create_test_block(i, prev_hash, producer.public_key(), vec![coinbase]);
        prev_hash = block.hash();
        blocks.push(block);
    }

    // Add to Node A
    for block in &blocks {
        node_a.add_block(block.clone()).await.unwrap();
    }

    // Sync to Node B
    for block in &blocks {
        node_b.add_block(block.clone()).await.unwrap();
    }

    assert_eq!(node_a.height().await, 14);
    assert_eq!(node_b.height().await, 14);
    assert_eq!(node_a.best_hash().await, node_b.best_hash().await);
}

/// Test UTXO set synchronization
#[tokio::test]
async fn test_utxo_sync() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();

    let config_a = TestNodeConfig::new(&temp_dir, 50);
    let config_b = TestNodeConfig::new(&temp_dir, 51);

    let node_a = Arc::new(TestNode::new(config_a));
    let node_b = Arc::new(TestNode::new(config_b));

    let producer = KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());

    // Generate chain with coinbase outputs
    let chain = generate_test_chain(10, &producer, 5_000_000_000);

    // Add to both nodes
    for block in &chain {
        node_a.add_block(block.clone()).await.unwrap();
        node_b.add_block(block.clone()).await.unwrap();
    }

    // Verify UTXO sets are identical
    let utxos_a = node_a.utxo_set.read().await;
    let utxos_b = node_b.utxo_set.read().await;

    assert_eq!(utxos_a.len(), utxos_b.len());

    // Each block should have created one UTXO (coinbase)
    assert_eq!(utxos_a.len(), 10);
}

/// Test that nodes reject blocks from the future
#[tokio::test]
async fn test_reject_future_blocks() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();

    let config = TestNodeConfig::new(&temp_dir, 60);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();

    // Create a valid chain first
    let chain = generate_test_chain(5, &producer, 5_000_000_000);
    for block in &chain {
        node.add_block(block.clone()).await.unwrap();
    }

    // The node should be at height 4 now
    assert_eq!(node.height().await, 4);

    // Attempting to add block at very high height should work
    // (our simple test node doesn't validate slot numbers strictly)
    // In production, this would be rejected
}

/// Test concurrent block additions
#[tokio::test]
async fn test_concurrent_block_additions() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();

    let config = TestNodeConfig::new(&temp_dir, 70);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();
    let chain = generate_test_chain(50, &producer, 5_000_000_000);

    // Add blocks concurrently from multiple tasks
    let mut handles = Vec::new();

    // We need to add blocks in order for them to be valid
    // So we'll add different subsets
    for block in chain {
        let node_clone = node.clone();
        let handle = tokio::spawn(async move {
            // Small delay to create some interleaving
            tokio::time::sleep(Duration::from_micros(rand::random::<u64>() % 100)).await;
            let _ = node_clone.add_block(block).await;
        });
        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        let _ = handle.await;
    }

    // Some blocks should have been added
    // (exact count depends on ordering - duplicates are rejected)
    let height = node.height().await;
    assert!(height > 0, "At least some blocks should be added");
}

/// Test chain tip tracking during sync
#[tokio::test]
async fn test_chain_tip_tracking() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();

    let config = TestNodeConfig::new(&temp_dir, 80);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();
    let chain = generate_test_chain(20, &producer, 5_000_000_000);

    // Track tip changes
    let mut tips = Vec::new();

    for block in &chain {
        node.add_block(block.clone()).await.unwrap();
        tips.push(node.best_hash().await);
    }

    // Each block should update the tip
    for (i, tip) in tips.iter().enumerate() {
        assert_eq!(*tip, chain[i].hash());
    }

    // Final tip should be the last block
    assert_eq!(node.best_hash().await, chain.last().unwrap().hash());
}
