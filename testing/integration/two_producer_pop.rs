//! Two Producer Proof of Presence Test
//!
//! Tests the simplified PoP model where:
//! - One producer per slot receives 100% of block reward
//! - Presence score is tracked per producer
//! - No attestations or multi-signature required

#[path = "../common/mod.rs"]
mod common;

use std::sync::Arc;

use common::{create_coinbase, create_test_block, init_test_logging, TestNode, TestNodeConfig};
use crypto::{hash::hash, Hash, KeyPair};
use doli_core::{
    genesis::{generate_genesis_block, GenesisConfig},
    BlockHeight, ProducerState, Slot, INITIAL_PRESENCE_SCORE, SCORE_MISS_PENALTY,
    SCORE_PRODUCE_BONUS,
};
use tempfile::TempDir;

/// Test basic two producer alternating blocks
#[tokio::test]
async fn test_two_producers_alternating() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 100);
    let node = Arc::new(TestNode::new(config));

    // Create two producers
    let producer1 = KeyPair::generate();
    let producer2 = KeyPair::generate();

    let pubkey_hash1 = hash(producer1.public_key().as_bytes());
    let pubkey_hash2 = hash(producer2.public_key().as_bytes());

    // Initialize producer states
    let mut state1 = ProducerState::new(pubkey_hash1, 0);
    let mut state2 = ProducerState::new(pubkey_hash2, 0);

    assert_eq!(state1.presence_score, INITIAL_PRESENCE_SCORE);
    assert_eq!(state2.presence_score, INITIAL_PRESENCE_SCORE);

    // Generate genesis block
    let genesis_config = GenesisConfig::devnet();
    let genesis = generate_genesis_block(&genesis_config);
    node.add_block(genesis.clone()).await.unwrap();

    let mut prev_hash = genesis.hash();
    let reward: u64 = 5_000_000_000; // 50 DOLI

    // Produce 10 blocks alternating between producers
    for slot in 1u32..=10 {
        let (producer, pubkey_hash, state) = if slot % 2 == 1 {
            (&producer1, &pubkey_hash1, &mut state1)
        } else {
            (&producer2, &pubkey_hash2, &mut state2)
        };

        // Create coinbase - 100% of reward to the producer
        let coinbase = create_coinbase(slot as BlockHeight, pubkey_hash, reward);
        let block = create_test_block(
            slot as BlockHeight,
            prev_hash,
            producer.public_key(),
            vec![coinbase],
        );

        // Record production in state
        state.record_produced(slot);

        prev_hash = block.hash();
        node.add_block(block).await.unwrap();
    }

    // Verify final heights
    assert_eq!(node.height().await, 10);

    // Verify producer states
    // Producer 1 produced slots 1, 3, 5, 7, 9 = 5 blocks
    assert_eq!(state1.blocks_produced, 5);
    assert_eq!(
        state1.presence_score,
        INITIAL_PRESENCE_SCORE + 5 * SCORE_PRODUCE_BONUS
    );

    // Producer 2 produced slots 2, 4, 6, 8, 10 = 5 blocks
    assert_eq!(state2.blocks_produced, 5);
    assert_eq!(
        state2.presence_score,
        INITIAL_PRESENCE_SCORE + 5 * SCORE_PRODUCE_BONUS
    );
}

/// Test producer missing assigned slots and score penalty
#[tokio::test]
async fn test_producer_misses_slots() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 101);
    let node = Arc::new(TestNode::new(config));

    // Create two producers
    let producer1 = KeyPair::generate();
    let producer2 = KeyPair::generate();

    let pubkey_hash1 = hash(producer1.public_key().as_bytes());
    let pubkey_hash2 = hash(producer2.public_key().as_bytes());

    // Initialize producer states
    let mut state1 = ProducerState::new(pubkey_hash1, 0);
    let mut state2 = ProducerState::new(pubkey_hash2, 0);

    // Generate genesis block
    let genesis_config = GenesisConfig::devnet();
    let genesis = generate_genesis_block(&genesis_config);
    node.add_block(genesis.clone()).await.unwrap();

    let mut prev_hash = genesis.hash();
    let reward: u64 = 5_000_000_000;

    // Simulate: Producer 1 is assigned slots 1, 3, 5 but misses slot 3
    // Producer 2 produces all assigned slots 2, 4, 6

    // Slot 1: Producer 1 produces
    let coinbase = create_coinbase(1, &pubkey_hash1, reward);
    let block = create_test_block(1, prev_hash, producer1.public_key(), vec![coinbase]);
    state1.record_produced(1);
    prev_hash = block.hash();
    node.add_block(block).await.unwrap();

    // Slot 2: Producer 2 produces
    let coinbase = create_coinbase(2, &pubkey_hash2, reward);
    let block = create_test_block(2, prev_hash, producer2.public_key(), vec![coinbase]);
    state2.record_produced(2);
    prev_hash = block.hash();
    node.add_block(block).await.unwrap();

    // Slot 3: Producer 1 MISSES - Producer 2 takes over as fallback
    state1.record_missed(); // Producer 1 gets penalty
    let coinbase = create_coinbase(3, &pubkey_hash2, reward);
    let block = create_test_block(3, prev_hash, producer2.public_key(), vec![coinbase]);
    state2.record_produced(3);
    prev_hash = block.hash();
    node.add_block(block).await.unwrap();

    // Slot 4: Producer 2 produces
    let coinbase = create_coinbase(4, &pubkey_hash2, reward);
    let block = create_test_block(4, prev_hash, producer2.public_key(), vec![coinbase]);
    state2.record_produced(4);
    prev_hash = block.hash();
    node.add_block(block).await.unwrap();

    // Slot 5: Producer 1 produces
    let coinbase = create_coinbase(5, &pubkey_hash1, reward);
    let block = create_test_block(5, prev_hash, producer1.public_key(), vec![coinbase]);
    state1.record_produced(5);
    prev_hash = block.hash();
    node.add_block(block).await.unwrap();

    // Slot 6: Producer 2 produces
    let coinbase = create_coinbase(6, &pubkey_hash2, reward);
    let block = create_test_block(6, prev_hash, producer2.public_key(), vec![coinbase]);
    state2.record_produced(6);
    node.add_block(block).await.unwrap();

    // Verify final state
    assert_eq!(node.height().await, 6);

    // Producer 1: 2 produced, 1 missed
    assert_eq!(state1.blocks_produced, 2);
    assert_eq!(state1.blocks_missed, 1);
    let expected_score1 = INITIAL_PRESENCE_SCORE + 2 * SCORE_PRODUCE_BONUS - SCORE_MISS_PENALTY;
    assert_eq!(state1.presence_score, expected_score1);

    // Producer 2: 4 produced (including taking over slot 3), 0 missed
    assert_eq!(state2.blocks_produced, 4);
    assert_eq!(state2.blocks_missed, 0);
    let expected_score2 = INITIAL_PRESENCE_SCORE + 4 * SCORE_PRODUCE_BONUS;
    assert_eq!(state2.presence_score, expected_score2);

    // Producer 2 should have higher score now
    assert!(state2.presence_score > state1.presence_score);
}

/// Test presence rate calculation
#[tokio::test]
async fn test_presence_rate_calculation() {
    let pubkey_hash = Hash::ZERO;
    let state = ProducerState::new(pubkey_hash, 0);

    // New producer with no history is assumed good (100%)
    assert_eq!(state.presence_rate(), 100);

    // Produce 8 blocks, miss 2
    let mut state = ProducerState::new(pubkey_hash, 0);
    for i in 0..8 {
        state.record_produced(i);
    }
    state.record_missed();
    state.record_missed();

    // 8 produced out of 10 total = 80%
    assert_eq!(state.presence_rate(), 80);

    // Perfect producer: 100 produced, 0 missed
    let mut perfect = ProducerState::new(pubkey_hash, 0);
    for i in 0..100 {
        perfect.record_produced(i);
    }
    assert_eq!(perfect.presence_rate(), 100);

    // Unreliable producer: 1 produced, 9 missed
    let mut unreliable = ProducerState::new(pubkey_hash, 0);
    unreliable.record_produced(0);
    for _ in 0..9 {
        unreliable.record_missed();
    }
    assert_eq!(unreliable.presence_rate(), 10);
}

/// Test minimum presence threshold
#[tokio::test]
async fn test_minimum_presence_threshold() {
    use doli_core::MIN_PRESENCE_RATE;

    let pubkey_hash = Hash::ZERO;
    let state = ProducerState::new(pubkey_hash, 0);

    // New producer with no history is assumed good (meets minimum by default)
    assert!(state.meets_minimum());

    // Producer with good presence (90%)
    let mut good = ProducerState::new(pubkey_hash, 0);
    for i in 0..90 {
        good.record_produced(i);
    }
    for _ in 0..10 {
        good.record_missed();
    }
    assert!(good.presence_rate() as u32 >= MIN_PRESENCE_RATE);
    assert!(good.meets_minimum());

    // Producer with poor presence (40%)
    let mut poor = ProducerState::new(pubkey_hash, 0);
    for i in 0..40 {
        poor.record_produced(i);
    }
    for _ in 0..60 {
        poor.record_missed();
    }
    assert_eq!(poor.presence_rate(), 40);
    assert!(!poor.meets_minimum());
}

/// Test producer activity detection
#[tokio::test]
async fn test_producer_activity() {
    let pubkey_hash = Hash::ZERO;
    let mut state = ProducerState::new(pubkey_hash, 100); // Registered at slot 100

    let inactivity_threshold: Slot = 3600; // 1 hour

    // Just registered, considered active
    assert!(state.is_active(100, inactivity_threshold));

    // Produced at slot 500
    state.record_produced(500);
    assert!(state.is_active(500, inactivity_threshold));

    // Still active within threshold
    assert!(state.is_active(500 + 3000, inactivity_threshold));

    // Inactive after threshold passes
    assert!(!state.is_active(500 + 4000, inactivity_threshold));
}

/// Test that genesis block is valid
#[tokio::test]
async fn test_genesis_block_valid() {
    use doli_core::genesis::verify_genesis_block;
    use doli_core::Network;

    // Test devnet genesis
    let devnet_config = GenesisConfig::devnet();
    let devnet_genesis = generate_genesis_block(&devnet_config);
    assert!(verify_genesis_block(&devnet_genesis, Network::Devnet).is_ok());
    assert_eq!(devnet_genesis.header.slot, 0);
    assert_eq!(devnet_genesis.transactions.len(), 1);

    // Test testnet genesis
    let testnet_config = GenesisConfig::testnet();
    let testnet_genesis = generate_genesis_block(&testnet_config);
    assert!(verify_genesis_block(&testnet_genesis, Network::Testnet).is_ok());

    // Test mainnet genesis
    let mainnet_config = GenesisConfig::mainnet();
    let mainnet_genesis = generate_genesis_block(&mainnet_config);
    assert!(verify_genesis_block(&mainnet_genesis, Network::Mainnet).is_ok());
}

/// Test full chain with two producers and scoring
#[tokio::test]
async fn test_full_pop_chain() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 102);
    let node = Arc::new(TestNode::new(config));

    // Create two producers
    let producer1 = KeyPair::generate();
    let producer2 = KeyPair::generate();

    let pubkey_hash1 = hash(producer1.public_key().as_bytes());
    let pubkey_hash2 = hash(producer2.public_key().as_bytes());

    let mut state1 = ProducerState::new(pubkey_hash1, 0);
    let mut state2 = ProducerState::new(pubkey_hash2, 0);

    // Genesis
    let genesis_config = GenesisConfig::devnet();
    let genesis = generate_genesis_block(&genesis_config);
    node.add_block(genesis.clone()).await.unwrap();

    let mut prev_hash = genesis.hash();
    let reward: u64 = 5_000_000_000;

    // Simulate 100 slots with realistic distribution
    // Producer 1 produces more reliably (95% rate)
    // Producer 2 produces less reliably (70% rate)
    let mut total_rewards_p1: u64 = 0;
    let mut total_rewards_p2: u64 = 0;

    for slot in 1u32..=100 {
        let assigned_to_p1 = slot % 2 == 1;

        let (actual_producer, actual_pubkey_hash) = if assigned_to_p1 {
            // Producer 1 assigned
            if slot % 20 == 13 {
                // P1 misses ~5% (slot 13, 33, 53, 73, 93)
                state1.record_missed();
                // P2 takes over
                state2.record_produced(slot);
                total_rewards_p2 += reward;
                (&producer2, &pubkey_hash2)
            } else {
                state1.record_produced(slot);
                total_rewards_p1 += reward;
                (&producer1, &pubkey_hash1)
            }
        } else {
            // Producer 2 assigned
            if slot % 10 == 4 || slot % 10 == 8 {
                // P2 misses ~30% (slots 4, 8, 14, 18, 24, 28, ...)
                state2.record_missed();
                // P1 takes over
                state1.record_produced(slot);
                total_rewards_p1 += reward;
                (&producer1, &pubkey_hash1)
            } else {
                state2.record_produced(slot);
                total_rewards_p2 += reward;
                (&producer2, &pubkey_hash2)
            }
        };

        let coinbase = create_coinbase(slot as BlockHeight, actual_pubkey_hash, reward);
        let block = create_test_block(
            slot as BlockHeight,
            prev_hash,
            actual_producer.public_key(),
            vec![coinbase],
        );
        prev_hash = block.hash();
        node.add_block(block).await.unwrap();
    }

    // Verify chain
    assert_eq!(node.height().await, 100);

    // Producer 1 should have higher score (more reliable)
    println!(
        "Producer 1: score={}, produced={}, missed={}, rate={}%",
        state1.presence_score,
        state1.blocks_produced,
        state1.blocks_missed,
        state1.presence_rate()
    );
    println!(
        "Producer 2: score={}, produced={}, missed={}, rate={}%",
        state2.presence_score,
        state2.blocks_produced,
        state2.blocks_missed,
        state2.presence_rate()
    );

    assert!(state1.presence_score > state2.presence_score);
    assert!(state1.presence_rate() > state2.presence_rate());

    // P1 took over some of P2's slots, so earned more rewards
    assert!(total_rewards_p1 > total_rewards_p2);

    // Both should still meet minimum presence
    assert!(state1.meets_minimum());
    // P2 with 70% may or may not meet minimum depending on MIN_PRESENCE_RATE
}
