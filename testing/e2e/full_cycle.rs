//! Full Cycle End-to-End Test
//!
//! Tests the complete lifecycle: Genesis → 1000 blocks → reorganization

#[path = "../common/mod.rs"]
mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use common::{
    create_coinbase, create_test_block, create_transfer, generate_test_chain,
    init_test_logging, TestNode, TestNodeConfig,
};
use doli_core::{
    Block, BlockHeight, Transaction, Output,
    consensus::ConsensusParams,
    types::coins_to_units,
};
use crypto::{Hash, KeyPair};
use storage::{Outpoint, UtxoEntry};
use tempfile::TempDir;

/// Full lifecycle test: genesis to 1000 blocks
#[tokio::test]
async fn test_genesis_to_1000_blocks() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 0);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();
    let params = ConsensusParams::mainnet();

    let start = Instant::now();

    // Generate and add 1000 blocks
    let chain = generate_test_chain(1000, &producer, params.block_reward(0));

    for (i, block) in chain.iter().enumerate() {
        node.add_block(block.clone()).await.unwrap();

        // Progress indicator every 100 blocks
        if (i + 1) % 100 == 0 {
            println!("Added {} blocks...", i + 1);
        }
    }

    let elapsed = start.elapsed();

    // Verify final state
    assert_eq!(node.height().await, 999);
    assert_eq!(node.best_hash().await, chain.last().unwrap().hash());

    // Verify UTXO set
    {
        let utxos = node.utxo_set.read().await;
        assert_eq!(utxos.len(), 1000); // One coinbase per block
    }

    println!("1000 blocks processed in {:?}", elapsed);
    println!("Average per block: {:?}", elapsed / 1000);
}

/// Full cycle with reorganization
#[tokio::test]
async fn test_full_cycle_with_reorg() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 10);
    let node = Arc::new(TestNode::new(config));

    let producer1 = KeyPair::generate();
    let producer2 = KeyPair::generate();
    let params = ConsensusParams::mainnet();

    // Phase 1: Build 500 blocks
    println!("Phase 1: Building initial chain of 500 blocks...");
    let chain1 = generate_test_chain(500, &producer1, params.block_reward(0));
    for block in &chain1 {
        node.add_block(block.clone()).await.unwrap();
    }

    assert_eq!(node.height().await, 499);
    let state_at_500 = node.best_hash().await;

    // Phase 2: Continue to 750 blocks
    println!("Phase 2: Extending to 750 blocks...");
    let pubkey_hash1 = crypto::hash::hash(producer1.public_key().as_bytes());
    let mut prev_hash = state_at_500;

    for i in 500..750 {
        let coinbase = create_coinbase(i, &pubkey_hash1, params.block_reward(i));
        let block = create_test_block(i, prev_hash, producer1.public_key(), vec![coinbase]);
        prev_hash = block.hash();
        node.add_block(block).await.unwrap();
    }

    assert_eq!(node.height().await, 749);

    // Phase 3: Reorganization - revert 250 blocks, build 400 new ones
    println!("Phase 3: Reorganization - reverting 250 blocks...");
    node.revert_blocks(250).await.unwrap();
    assert_eq!(node.height().await, 499);
    assert_eq!(node.best_hash().await, state_at_500);

    println!("Phase 3: Building competing chain of 400 blocks...");
    let pubkey_hash2 = crypto::hash::hash(producer2.public_key().as_bytes());
    prev_hash = state_at_500;

    for i in 500..900 {
        let coinbase = create_coinbase(i, &pubkey_hash2, params.block_reward(i));
        let block = create_test_block(i, prev_hash, producer2.public_key(), vec![coinbase]);
        prev_hash = block.hash();
        node.add_block(block).await.unwrap();
    }

    assert_eq!(node.height().await, 899);

    // Phase 4: Continue to 1000 blocks
    println!("Phase 4: Completing to 1000 blocks...");
    for i in 900..1000 {
        let coinbase = create_coinbase(i, &pubkey_hash2, params.block_reward(i));
        let block = create_test_block(i, prev_hash, producer2.public_key(), vec![coinbase]);
        prev_hash = block.hash();
        node.add_block(block).await.unwrap();
    }

    assert_eq!(node.height().await, 999);

    // Verify final UTXO count (500 from original + 500 from new chain)
    {
        let utxos = node.utxo_set.read().await;
        assert_eq!(utxos.len(), 1000);
    }

    println!("Full cycle completed successfully!");
}

/// Test with realistic transaction load
#[tokio::test]
async fn test_full_cycle_with_transactions() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 20);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();
    let recipient = KeyPair::generate();
    let params = ConsensusParams::mainnet();

    let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());
    let recipient_hash = crypto::hash::hash(recipient.public_key().as_bytes());

    // Build 100 blocks with multiple transactions each
    println!("Building 100 blocks with transactions...");

    let mut prev_hash = Hash::ZERO;
    let mut available_utxos: Vec<(Hash, u32, u64)> = Vec::new(); // (tx_hash, index, amount)

    for height in 0..100u64 {
        let mut transactions = Vec::new();

        // Coinbase
        let coinbase_amount = params.block_reward(height);
        let coinbase = create_coinbase(height, &pubkey_hash, coinbase_amount);
        let coinbase_hash = coinbase.hash();
        transactions.push(coinbase);

        // Add some transfer transactions using available UTXOs
        if !available_utxos.is_empty() && height > 0 {
            // Take up to 5 UTXOs for spending
            let to_spend: Vec<_> = available_utxos.drain(..available_utxos.len().min(5)).collect();

            for (utxo_hash, utxo_idx, utxo_amount) in to_spend {
                if utxo_amount > 1000 {
                    // Create a simple transfer
                    let send_amount = utxo_amount / 2;
                    let change_amount = utxo_amount - send_amount - 100; // 100 for fee

                    let tx = create_transfer(
                        vec![(utxo_hash, utxo_idx)],
                        vec![
                            (send_amount, recipient_hash),
                            (change_amount, pubkey_hash),
                        ],
                        &producer,
                    );

                    let tx_hash = tx.hash();
                    transactions.push(tx);

                    // The change output becomes available
                    available_utxos.push((tx_hash, 1, change_amount));
                }
            }
        }

        // Create block
        let block = create_test_block(height, prev_hash, producer.public_key(), transactions);
        prev_hash = block.hash();

        // Track the coinbase UTXO (maturity not simulated in this test)
        available_utxos.push((coinbase_hash, 0, coinbase_amount));

        node.add_block(block).await.unwrap();
    }

    assert_eq!(node.height().await, 99);

    // Count UTXOs
    let utxo_count = {
        let utxos = node.utxo_set.read().await;
        utxos.len()
    };

    println!("Final UTXO count: {}", utxo_count);
    assert!(utxo_count > 100); // At least coinbases + change outputs
}

/// Test era transitions (long-running simulation)
#[tokio::test]
#[ignore] // This test takes a while, run with --ignored
async fn test_era_transition_simulation() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 30);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();
    let params = ConsensusParams::mainnet();

    // Simulate blocks around era boundary
    // Era 1 ends at block 2,102,400 (4 years at 1 block/minute)
    // For testing, we'll simulate at a compressed scale

    println!("Simulating era transitions...");

    let test_heights = [0u64, 1000, 10000, 100000];
    let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());

    for &height in &test_heights {
        let reward = params.block_reward(height);
        let era = params.height_to_era(height);

        println!("Height {}: Era {}, Reward: {} units", height, era, reward);

        // Verify era calculations
        assert!(era >= 1);

        // Verify reward decreases over time
        if height > 0 {
            let prev_reward = params.block_reward(height - 1);
            assert!(reward <= prev_reward);
        }
    }
}

/// Test block production simulation
#[tokio::test]
async fn test_block_production_simulation() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 40);
    let node = Arc::new(TestNode::new(config));

    // Multiple producers
    let producers: Vec<KeyPair> = (0..5).map(|_| KeyPair::generate()).collect();

    let params = ConsensusParams::mainnet();
    let mut prev_hash = Hash::ZERO;
    let mut block_counts: HashMap<String, u32> = HashMap::new();

    // Simulate 100 blocks with rotating producers
    for height in 0..100u64 {
        // Round-robin producer selection (simplified)
        let producer_idx = (height as usize) % producers.len();
        let producer = &producers[producer_idx];
        let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());

        let coinbase = create_coinbase(height, &pubkey_hash, params.block_reward(height));
        let block = create_test_block(height, prev_hash, producer.public_key(), vec![coinbase]);

        prev_hash = block.hash();
        node.add_block(block).await.unwrap();

        // Track production
        *block_counts.entry(format!("producer_{}", producer_idx)).or_insert(0) += 1;
    }

    // Verify distribution
    println!("Block production distribution:");
    for (producer, count) in &block_counts {
        println!("  {}: {} blocks", producer, count);
        assert_eq!(*count, 20); // Equal distribution with round-robin
    }

    assert_eq!(node.height().await, 99);
}

/// Test chain state consistency after many operations
#[tokio::test]
async fn test_chain_state_consistency() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 50);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();
    let params = ConsensusParams::mainnet();

    // Build chain
    let chain = generate_test_chain(200, &producer, params.block_reward(0));
    for block in &chain {
        node.add_block(block.clone()).await.unwrap();
    }

    // Verify chain links
    for i in 1..chain.len() {
        assert_eq!(chain[i].header.prev_hash, chain[i-1].hash());
    }

    // Multiple reorgs
    for cycle in 0..5u64 {
        // Revert 20 blocks
        node.revert_blocks(20).await.unwrap();

        // Build 30 new blocks (add unique offset to avoid hash collisions)
        let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());
        let mut prev = node.best_hash().await;
        let start_height = node.height().await + 1;

        for i in 0..30 {
            let height = start_height + i;
            // Add base offset + cycle to differentiate from original chain and between cycles
            let reward = params.block_reward(height) + 1_000_000 + cycle * 100;
            let coinbase = create_coinbase(height, &pubkey_hash, reward);
            let block = create_test_block(height, prev, producer.public_key(), vec![coinbase]);
            prev = block.hash();
            node.add_block(block).await.unwrap();
        }
    }

    // Verify final state is consistent
    let final_height = node.height().await;
    let final_hash = node.best_hash().await;

    // Block at tip should exist
    let tip_block = node.get_block(&final_hash).await;
    assert!(tip_block.is_some());

    // Height should be reasonable
    assert!(final_height >= 200); // At least original chain

    println!("Final chain state: height={}, tip={:?}", final_height, final_hash);
}

/// Test genesis block handling
#[tokio::test]
async fn test_genesis_block() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 60);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();

    // Create explicit genesis block
    let genesis = generate_test_chain(1, &producer, 5_000_000_000);
    let genesis_block = &genesis[0];

    assert!(genesis_block.is_genesis());
    assert_eq!(genesis_block.header.prev_hash, Hash::ZERO);

    node.add_block(genesis_block.clone()).await.unwrap();

    assert_eq!(node.height().await, 0);
    assert_eq!(node.best_hash().await, genesis_block.hash());

    // Verify genesis coinbase is in UTXO set
    {
        let utxos = node.utxo_set.read().await;
        assert_eq!(utxos.len(), 1);
    }
}

/// Test performance metrics for full cycle
#[tokio::test]
async fn test_performance_metrics() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 70);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();
    let params = ConsensusParams::mainnet();

    let chain = generate_test_chain(500, &producer, params.block_reward(0));

    // Measure block addition time
    let start = Instant::now();
    for block in &chain {
        node.add_block(block.clone()).await.unwrap();
    }
    let block_add_time = start.elapsed();

    // Measure revert time
    let start = Instant::now();
    node.revert_blocks(100).await.unwrap();
    let revert_time = start.elapsed();

    // Measure rebuild time
    let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());
    let mut prev = node.best_hash().await;
    let start_height = node.height().await + 1;

    let start = Instant::now();
    for i in 0..150 {
        let height = start_height + i;
        // Add offset to differentiate from original chain
        let reward = params.block_reward(height) + 1_000_000;
        let coinbase = create_coinbase(height, &pubkey_hash, reward);
        let block = create_test_block(height, prev, producer.public_key(), vec![coinbase]);
        prev = block.hash();
        node.add_block(block).await.unwrap();
    }
    let rebuild_time = start.elapsed();

    println!("Performance Metrics:");
    println!("  500 blocks added: {:?} ({:?}/block)", block_add_time, block_add_time / 500);
    println!("  100 blocks reverted: {:?} ({:?}/block)", revert_time, revert_time / 100);
    println!("  150 blocks rebuilt: {:?} ({:?}/block)", rebuild_time, rebuild_time / 150);

    // Basic performance assertions
    assert!(block_add_time.as_millis() < 10000); // Should be fast
    assert!(revert_time.as_millis() < 5000);
    assert!(rebuild_time.as_millis() < 5000);
}
