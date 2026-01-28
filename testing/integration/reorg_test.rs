//! Chain Reorganization Tests
//!
//! Tests that nodes properly handle chain reorganizations of various depths.

#[path = "../common/mod.rs"]
mod common;

use std::sync::Arc;

use common::{
    create_coinbase, create_test_block, generate_test_chain, init_test_logging,
    TestNode, TestNodeConfig,
};
use doli_core::BlockHeight;
use crypto::{Hash, KeyPair};
use tempfile::TempDir;

/// Test a simple 1-block reorganization
#[tokio::test]
async fn test_single_block_reorg() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 0);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());

    // Create initial chain of 5 blocks
    let chain = generate_test_chain(5, &producer, 5_000_000_000);
    for block in &chain {
        node.add_block(block.clone()).await.unwrap();
    }

    let original_tip = node.best_hash().await;
    assert_eq!(node.height().await, 4);

    // Revert the last block
    let reverted = node.revert_blocks(1).await.unwrap();
    assert_eq!(reverted.len(), 1);
    assert_eq!(reverted[0].hash(), original_tip);

    // Height should be 3 now
    assert_eq!(node.height().await, 3);
    assert_eq!(node.best_hash().await, chain[3].hash());

    // Add a different block at height 4 (competing chain)
    let competing_coinbase = create_coinbase(4, &pubkey_hash, 5_000_000_001); // Different amount
    let competing_block = create_test_block(
        4,
        chain[3].hash(),
        producer.public_key(),
        vec![competing_coinbase],
    );

    node.add_block(competing_block.clone()).await.unwrap();

    // New tip should be the competing block
    assert_eq!(node.height().await, 4);
    assert_eq!(node.best_hash().await, competing_block.hash());
    assert_ne!(node.best_hash().await, original_tip);
}

/// Test a deep reorganization of 10+ blocks
#[tokio::test]
async fn test_deep_reorg_10_blocks() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 10);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());

    // Create chain of 20 blocks
    let chain = generate_test_chain(20, &producer, 5_000_000_000);
    for block in &chain {
        node.add_block(block.clone()).await.unwrap();
    }

    assert_eq!(node.height().await, 19);
    let original_tip = node.best_hash().await;

    // Revert 10 blocks
    let reverted = node.revert_blocks(10).await.unwrap();
    assert_eq!(reverted.len(), 10);
    assert_eq!(node.height().await, 9);
    assert_eq!(node.best_hash().await, chain[9].hash());

    // Build a competing chain of 12 blocks from the fork point
    let mut prev_hash = chain[9].hash();
    let mut competing_chain = Vec::new();

    for i in 10..22 {
        let coinbase = create_coinbase(i, &pubkey_hash, 5_000_000_002); // Different amount
        let block = create_test_block(i, prev_hash, producer.public_key(), vec![coinbase]);
        prev_hash = block.hash();
        competing_chain.push(block);
    }

    // Add competing chain
    for block in &competing_chain {
        node.add_block(block.clone()).await.unwrap();
    }

    // Node should now be on the competing chain
    assert_eq!(node.height().await, 21);
    assert_eq!(node.best_hash().await, competing_chain.last().unwrap().hash());
    assert_ne!(node.best_hash().await, original_tip);
}

/// Test a very deep reorganization (15+ blocks)
#[tokio::test]
async fn test_very_deep_reorg() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 20);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());

    // Create chain of 30 blocks
    let chain = generate_test_chain(30, &producer, 5_000_000_000);
    for block in &chain {
        node.add_block(block.clone()).await.unwrap();
    }

    assert_eq!(node.height().await, 29);

    // Revert 15 blocks
    let reverted = node.revert_blocks(15).await.unwrap();
    assert_eq!(reverted.len(), 15);
    assert_eq!(node.height().await, 14);

    // Fork point should be block 14
    assert_eq!(node.best_hash().await, chain[14].hash());

    // Build a longer competing chain
    let mut prev_hash = chain[14].hash();
    let mut competing_chain = Vec::new();

    for i in 15..35 {
        let coinbase = create_coinbase(i, &pubkey_hash, 5_000_000_003);
        let block = create_test_block(i, prev_hash, producer.public_key(), vec![coinbase]);
        prev_hash = block.hash();
        competing_chain.push(block);
    }

    for block in &competing_chain {
        node.add_block(block.clone()).await.unwrap();
    }

    assert_eq!(node.height().await, 34);
    assert_eq!(node.best_hash().await, competing_chain.last().unwrap().hash());
}

/// Test UTXO set consistency during reorg
#[tokio::test]
async fn test_utxo_consistency_during_reorg() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 30);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());

    // Create initial chain
    let chain = generate_test_chain(10, &producer, 5_000_000_000);
    for block in &chain {
        node.add_block(block.clone()).await.unwrap();
    }

    // Should have 10 UTXOs (one per block coinbase)
    {
        let utxos = node.utxo_set.read().await;
        assert_eq!(utxos.len(), 10);
    }

    // Revert 5 blocks
    node.revert_blocks(5).await.unwrap();

    // Should have 5 UTXOs now
    {
        let utxos = node.utxo_set.read().await;
        assert_eq!(utxos.len(), 5);
    }

    // Add new competing chain
    let mut prev_hash = node.best_hash().await;
    for i in 5..12 {
        let coinbase = create_coinbase(i, &pubkey_hash, 5_000_000_004);
        let block = create_test_block(i, prev_hash, producer.public_key(), vec![coinbase]);
        prev_hash = block.hash();
        node.add_block(block).await.unwrap();
    }

    // Should have 5 + 7 = 12 UTXOs
    {
        let utxos = node.utxo_set.read().await;
        assert_eq!(utxos.len(), 12);
    }
}

/// Test multiple sequential reorgs
#[tokio::test]
async fn test_multiple_sequential_reorgs() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 40);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());

    // Initial chain of 15 blocks
    let chain = generate_test_chain(15, &producer, 5_000_000_000);
    for block in &chain {
        node.add_block(block.clone()).await.unwrap();
    }

    assert_eq!(node.height().await, 14);

    // First reorg: go back 3 blocks, add 5 new
    node.revert_blocks(3).await.unwrap();
    assert_eq!(node.height().await, 11);

    let mut prev_hash = node.best_hash().await;
    for i in 12..17 {
        let coinbase = create_coinbase(i, &pubkey_hash, 5_000_000_005);
        let block = create_test_block(i, prev_hash, producer.public_key(), vec![coinbase]);
        prev_hash = block.hash();
        node.add_block(block).await.unwrap();
    }

    assert_eq!(node.height().await, 16);
    let tip_after_first = node.best_hash().await;

    // Second reorg: go back 5 blocks, add 7 new
    node.revert_blocks(5).await.unwrap();
    assert_eq!(node.height().await, 11);

    prev_hash = node.best_hash().await;
    for i in 12..19 {
        let coinbase = create_coinbase(i, &pubkey_hash, 5_000_000_006);
        let block = create_test_block(i, prev_hash, producer.public_key(), vec![coinbase]);
        prev_hash = block.hash();
        node.add_block(block).await.unwrap();
    }

    assert_eq!(node.height().await, 18);
    let tip_after_second = node.best_hash().await;

    // Third reorg: go back 2 blocks, add 4 new
    node.revert_blocks(2).await.unwrap();
    assert_eq!(node.height().await, 16);

    prev_hash = node.best_hash().await;
    for i in 17..21 {
        let coinbase = create_coinbase(i, &pubkey_hash, 5_000_000_007);
        let block = create_test_block(i, prev_hash, producer.public_key(), vec![coinbase]);
        prev_hash = block.hash();
        node.add_block(block).await.unwrap();
    }

    assert_eq!(node.height().await, 20);

    // All tips should be different
    assert_ne!(tip_after_first, tip_after_second);
    assert_ne!(tip_after_second, node.best_hash().await);
}

/// Test reorg with different producers
#[tokio::test]
async fn test_reorg_different_producers() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 50);
    let node = Arc::new(TestNode::new(config));

    let producer1 = KeyPair::generate();
    let producer2 = KeyPair::generate();
    let pubkey_hash1 = crypto::hash::hash(producer1.public_key().as_bytes());
    let pubkey_hash2 = crypto::hash::hash(producer2.public_key().as_bytes());

    // Initial chain from producer1
    let chain1 = generate_test_chain(10, &producer1, 5_000_000_000);
    for block in &chain1 {
        node.add_block(block.clone()).await.unwrap();
    }

    assert_eq!(node.height().await, 9);

    // Reorg: producer2 builds a longer chain from block 5
    node.revert_blocks(4).await.unwrap();
    assert_eq!(node.height().await, 5);

    let mut prev_hash = node.best_hash().await;
    for i in 6..15 {
        let coinbase = create_coinbase(i, &pubkey_hash2, 5_000_000_000);
        let block = create_test_block(i, prev_hash, producer2.public_key(), vec![coinbase]);
        prev_hash = block.hash();
        node.add_block(block).await.unwrap();
    }

    assert_eq!(node.height().await, 14);

    // Verify the tip is from producer2
    let tip_block = node.get_block(&node.best_hash().await).await.unwrap();
    assert_eq!(&tip_block.header.producer, producer2.public_key());
}

/// Test that reorg preserves chain integrity
#[tokio::test]
async fn test_reorg_chain_integrity() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 60);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());

    // Create chain
    let chain = generate_test_chain(20, &producer, 5_000_000_000);
    for block in &chain {
        node.add_block(block.clone()).await.unwrap();
    }

    // Revert and rebuild
    node.revert_blocks(10).await.unwrap();

    let mut prev_hash = node.best_hash().await;
    let mut new_blocks = Vec::new();

    for i in 10..25 {
        let coinbase = create_coinbase(i, &pubkey_hash, 5_000_000_008);
        let block = create_test_block(i, prev_hash, producer.public_key(), vec![coinbase]);
        prev_hash = block.hash();
        new_blocks.push(block.clone());
        node.add_block(block).await.unwrap();
    }

    // Verify chain links
    for i in 1..new_blocks.len() {
        let block = &new_blocks[i];
        let prev_block = &new_blocks[i - 1];
        assert_eq!(block.header.prev_hash, prev_block.hash());
    }

    // Verify first new block links to the fork point
    let fork_point = chain[9].hash();
    assert_eq!(new_blocks[0].header.prev_hash, fork_point);
}

/// Test reorg to equal length chain (tie-breaker scenario)
#[tokio::test]
async fn test_reorg_equal_length() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 70);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());

    // Create initial chain
    let chain = generate_test_chain(10, &producer, 5_000_000_000);
    for block in &chain {
        node.add_block(block.clone()).await.unwrap();
    }

    let original_tip = node.best_hash().await;
    assert_eq!(node.height().await, 9);

    // Revert 3 blocks
    node.revert_blocks(3).await.unwrap();
    assert_eq!(node.height().await, 6);

    // Add exactly 3 new blocks (same length as before)
    let mut prev_hash = node.best_hash().await;
    let mut competing_tip = Hash::ZERO;

    for i in 7..10 {
        let coinbase = create_coinbase(i, &pubkey_hash, 5_000_000_009);
        let block = create_test_block(i, prev_hash, producer.public_key(), vec![coinbase]);
        prev_hash = block.hash();
        competing_tip = block.hash();
        node.add_block(block).await.unwrap();
    }

    // Should be at same height but different tip
    assert_eq!(node.height().await, 9);
    assert_eq!(node.best_hash().await, competing_tip);
    assert_ne!(node.best_hash().await, original_tip);
}

/// Test that empty revert does nothing
#[tokio::test]
async fn test_empty_revert() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 80);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();
    let chain = generate_test_chain(5, &producer, 5_000_000_000);

    for block in &chain {
        node.add_block(block.clone()).await.unwrap();
    }

    let original_tip = node.best_hash().await;
    let original_height = node.height().await;

    // Revert 0 blocks
    let reverted = node.revert_blocks(0).await.unwrap();
    assert!(reverted.is_empty());

    // State unchanged
    assert_eq!(node.best_hash().await, original_tip);
    assert_eq!(node.height().await, original_height);
}
