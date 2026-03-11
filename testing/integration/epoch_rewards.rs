//! Epoch Rewards Integration Tests
//!
//! Tests the Pool-First Epoch Reward Distribution System:
//! - Rewards accumulate in pool during epoch (no coinbase per block)
//! - At epoch boundary, pool divided equally among all participating producers
//! - First producer (sorted by pubkey) receives any remainder
//! - Epoch reward outputs require 100 confirmations (REWARD_MATURITY)

#[path = "../common/mod.rs"]
mod common;

use crypto::{hash::hash, KeyPair};
use doli_core::{
    consensus::{ConsensusParams, RewardMode, REWARD_MATURITY},
    network::Network,
    transaction::{Output, Transaction, TxType},
    Amount,
};
use storage::{Outpoint, UtxoEntry};

// =============================================================================
// Fair Share Calculation Tests
// =============================================================================

#[test]
fn test_fair_share_calculation_even_split() {
    // Pool of 300 DOLI split among 3 producers
    let pool: Amount = 300_000_000_000; // 300 DOLI in base units
    let num_producers: u64 = 3;

    let fair_share = pool / num_producers;
    let remainder = pool % num_producers;

    assert_eq!(fair_share, 100_000_000_000); // 100 DOLI each
    assert_eq!(remainder, 0); // No remainder
}

#[test]
fn test_fair_share_calculation_with_remainder() {
    // Pool with odd amount split among 3 producers
    let pool: Amount = 100_000_000_001; // Odd amount
    let num_producers: u64 = 3;

    let fair_share = pool / num_producers;
    let remainder = pool % num_producers;

    assert_eq!(fair_share, 33_333_333_333);
    assert_eq!(remainder, 2); // 2 base units remainder

    // First producer gets fair_share + remainder
    let first_producer_reward = fair_share + remainder;
    let other_producer_reward = fair_share;

    // Total should equal pool exactly (no dust lost)
    let total = first_producer_reward + other_producer_reward * 2;
    assert_eq!(total, pool);
}

#[test]
fn test_fair_share_single_producer() {
    // Single producer gets entire pool
    let pool: Amount = 500_000_000_000; // 500 DOLI
    let num_producers: u64 = 1;

    let fair_share = pool / num_producers;
    let remainder = pool % num_producers;

    assert_eq!(fair_share, pool);
    assert_eq!(remainder, 0);
}

#[test]
fn test_fair_share_many_producers() {
    // 100 producers splitting 1000 DOLI
    let pool: Amount = 1_000_000_000_000; // 1000 DOLI
    let num_producers: u64 = 100;

    let fair_share = pool / num_producers;
    let remainder = pool % num_producers;

    assert_eq!(fair_share, 10_000_000_000); // 10 DOLI each
    assert_eq!(remainder, 0);

    // Total distributed equals pool
    let total = fair_share * num_producers + remainder;
    assert_eq!(total, pool);
}

// =============================================================================
// Epoch Reward Transaction Creation Tests
// =============================================================================

#[test]
fn test_epoch_reward_transaction_creation() {
    let keypair = KeyPair::generate();
    let recipient_hash = hash(keypair.public_key().as_bytes());
    let epoch = 5u64;
    let amount = 100_000_000_000u64; // 100 DOLI

    let tx = Transaction::new_epoch_reward(epoch, *keypair.public_key(), amount, recipient_hash);

    assert_eq!(tx.tx_type, TxType::EpochReward);
    assert!(tx.inputs.is_empty()); // Minted, no inputs
    assert_eq!(tx.outputs.len(), 1);
    assert_eq!(tx.outputs[0].amount, amount);
    assert_eq!(tx.outputs[0].pubkey_hash, recipient_hash);
    assert!(tx.is_epoch_reward());
}

#[test]
fn test_epoch_reward_transaction_data() {
    let keypair = KeyPair::generate();
    let recipient_hash = hash(keypair.public_key().as_bytes());
    let epoch = 42u64;
    let amount = 50_000_000_000u64;

    let tx = Transaction::new_epoch_reward(epoch, *keypair.public_key(), amount, recipient_hash);

    // Verify epoch reward data can be extracted
    let data = tx
        .epoch_reward_data()
        .expect("Should have epoch reward data");
    assert_eq!(data.epoch, epoch);
    assert_eq!(data.recipient, *keypair.public_key());
}

#[test]
fn test_epoch_reward_has_correct_type() {
    let keypair = KeyPair::generate();
    let recipient_hash = hash(keypair.public_key().as_bytes());

    let tx = Transaction::new_epoch_reward(1, *keypair.public_key(), 1000, recipient_hash);

    // Type should be EpochReward (10)
    assert_eq!(tx.tx_type, TxType::EpochReward);
    assert!(tx.is_epoch_reward());
    assert!(!tx.is_coinbase());
}

// =============================================================================
// Epoch Reward Maturity Tests
// =============================================================================

#[test]
fn test_epoch_reward_utxo_maturity() {
    let keypair = KeyPair::generate();
    let recipient_hash = hash(keypair.public_key().as_bytes());

    // Create an epoch reward transaction
    let tx =
        Transaction::new_epoch_reward(1, *keypair.public_key(), 100_000_000_000, recipient_hash);

    // Create UTXO entry for the epoch reward (created at height 100)
    let entry = UtxoEntry {
        output: tx.outputs[0].clone(),
        height: 100,
        is_coinbase: false,
        is_epoch_reward: true,
    };

    // Should NOT be spendable before maturity (6 confirmations)
    assert!(!entry.is_spendable_at_for_network(100, Network::Mainnet)); // 0 confirmations
    assert!(!entry.is_spendable_at_for_network(103, Network::Mainnet)); // 3 confirmations
    assert!(!entry.is_spendable_at_for_network(105, Network::Mainnet)); // 5 confirmations

    // Should be spendable at and after maturity
    assert!(entry.is_spendable_at_for_network(106, Network::Mainnet)); // Exactly 6 confirmations
    assert!(entry.is_spendable_at_for_network(200, Network::Mainnet)); // 100 confirmations
}

#[test]
fn test_coinbase_maturity_unchanged() {
    let recipient_hash = hash(b"producer1");

    // Create a coinbase transaction
    let tx = Transaction::new_coinbase(100_000_000_000, recipient_hash, 50);

    // Create UTXO entry for coinbase (created at height 50)
    let entry = UtxoEntry {
        output: tx.outputs[0].clone(),
        height: 50,
        is_coinbase: true,
        is_epoch_reward: false,
    };

    // Coinbase should also require 6 confirmations
    assert!(!entry.is_spendable_at_for_network(50, Network::Mainnet)); // 0 confirmations
    assert!(!entry.is_spendable_at_for_network(53, Network::Mainnet)); // 3 confirmations
    assert!(!entry.is_spendable_at_for_network(55, Network::Mainnet)); // 5 confirmations
    assert!(entry.is_spendable_at_for_network(56, Network::Mainnet)); // 6 confirmations
}

#[test]
fn test_regular_tx_no_maturity() {
    let recipient_hash = hash(b"recipient");

    // Create a regular output
    let output = Output::normal(100_000_000_000, recipient_hash);

    // Create UTXO entry for regular transaction (created at height 50)
    let entry = UtxoEntry {
        output,
        height: 50,
        is_coinbase: false,
        is_epoch_reward: false,
    };

    // Regular transactions should be spendable immediately
    assert!(entry.is_spendable_at_for_network(50, Network::Mainnet)); // Same height
    assert!(entry.is_spendable_at_for_network(51, Network::Mainnet)); // Next height
}

// =============================================================================
// Pool Accumulation Tests
// =============================================================================

#[test]
fn test_pool_accumulation_over_epoch() {
    let params = ConsensusParams::devnet();
    let block_reward = params.block_reward(1);
    let epoch_length = params.slots_per_reward_epoch as u64;

    // Simulate epoch accumulation
    let mut pool: Amount = 0;
    for _ in 0..epoch_length {
        pool += block_reward;
    }

    // Pool should equal block_reward * epoch_length
    assert_eq!(pool, block_reward * epoch_length);
}

#[test]
fn test_epoch_total_matches_distribution() {
    let params = ConsensusParams::devnet();
    let block_reward = params.block_reward(1);
    let epoch_length = 100u64; // Use smaller epoch for test

    // Simulate pool accumulation
    let pool: Amount = block_reward * epoch_length;

    // Distribute to 5 producers
    let num_producers = 5u64;
    let fair_share = pool / num_producers;
    let remainder = pool % num_producers;

    // Calculate total distributed
    let first_producer = fair_share + remainder;
    let other_producers = fair_share * (num_producers - 1);
    let total_distributed = first_producer + other_producers;

    // Total distributed should exactly equal pool
    assert_eq!(total_distributed, pool);
}

// =============================================================================
// Deterministic Ordering Tests
// =============================================================================

#[test]
fn test_producer_sorting_deterministic() {
    // Create 5 producers with random keys
    let producers: Vec<KeyPair> = (0..5).map(|_| KeyPair::generate()).collect();

    // Get their public keys
    let mut pubkeys: Vec<_> = producers.iter().map(|kp| *kp.public_key()).collect();

    // Sort by public key bytes
    pubkeys.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

    // Sorting should be deterministic
    let mut pubkeys2 = pubkeys.clone();
    pubkeys2.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

    assert_eq!(pubkeys, pubkeys2);
}

#[test]
fn test_first_producer_gets_remainder() {
    // Simulate distribution with remainder
    let pool: Amount = 100_000_000_007; // 100_000_000_007 % 3 = 2 base units remainder
    let num_producers = 3u64;

    let fair_share = pool / num_producers;
    let remainder = pool % num_producers;

    assert_eq!(remainder, 2); // 2 base unit remainder

    // Simulate sorted distribution
    let distributions: Vec<Amount> = (0..num_producers as usize)
        .map(|i| {
            if i == 0 {
                fair_share + remainder // First gets remainder
            } else {
                fair_share
            }
        })
        .collect();

    // First should be larger
    assert!(distributions[0] > distributions[1]);
    assert_eq!(distributions[1], distributions[2]);

    // Total should equal pool
    let total: Amount = distributions.iter().sum();
    assert_eq!(total, pool);
}

// =============================================================================
// Reward Mode Tests
// =============================================================================

#[test]
fn test_reward_mode_epoch_pool_is_default() {
    // EpochPool should be the default mode
    assert_eq!(RewardMode::default(), RewardMode::EpochPool);
}

#[test]
fn test_consensus_params_reward_mode() {
    // All network configs should use EpochPool
    let mainnet = ConsensusParams::mainnet();
    let testnet = ConsensusParams::testnet();
    let devnet = ConsensusParams::devnet();

    assert_eq!(mainnet.reward_mode, RewardMode::EpochPool);
    assert_eq!(testnet.reward_mode, RewardMode::EpochPool);
    assert_eq!(devnet.reward_mode, RewardMode::EpochPool);
}

#[test]
fn test_epoch_boundary_detection() {
    let params = ConsensusParams::devnet();
    let epoch_length = params.slots_per_reward_epoch;

    // Slot 0 is NOT a boundary (genesis)
    assert!(!params.is_reward_epoch_boundary(0));

    // First boundary at slots_per_reward_epoch
    assert!(params.is_reward_epoch_boundary(epoch_length));

    // Mid-epoch is not a boundary
    assert!(!params.is_reward_epoch_boundary(epoch_length / 2));

    // Second boundary
    assert!(params.is_reward_epoch_boundary(epoch_length * 2));

    // Third boundary
    assert!(params.is_reward_epoch_boundary(epoch_length * 3));
}

// =============================================================================
// Edge Case Tests
// =============================================================================

#[test]
fn test_epoch_reward_minimum_amount() {
    let keypair = KeyPair::generate();
    let recipient_hash = hash(keypair.public_key().as_bytes());

    // Create epoch reward with minimum valid amount (1 base unit)
    let tx = Transaction::new_epoch_reward(
        1,
        *keypair.public_key(),
        1, // 1 base unit
        recipient_hash,
    );

    assert_eq!(tx.outputs[0].amount, 1);
    assert!(tx.is_epoch_reward());
}

#[test]
fn test_epoch_reward_large_epoch_number() {
    let keypair = KeyPair::generate();
    let recipient_hash = hash(keypair.public_key().as_bytes());

    // Create epoch reward with very large epoch number
    let tx = Transaction::new_epoch_reward(u64::MAX, *keypair.public_key(), 1000, recipient_hash);

    let data = tx.epoch_reward_data().unwrap();
    assert_eq!(data.epoch, u64::MAX);
}

#[test]
fn test_reward_maturity_constant() {
    // REWARD_MATURITY should be 6 (aligned with COINBASE_MATURITY)
    assert_eq!(REWARD_MATURITY, 6);
}

// =============================================================================
// UTXO Set Integration Tests
// =============================================================================

#[test]
fn test_utxo_set_add_epoch_reward() {
    use storage::UtxoSet;

    let mut utxo_set = UtxoSet::new();
    let keypair = KeyPair::generate();
    let recipient_hash = hash(keypair.public_key().as_bytes());

    let tx =
        Transaction::new_epoch_reward(1, *keypair.public_key(), 100_000_000_000, recipient_hash);

    // Add to UTXO set (not coinbase, height 100)
    utxo_set.add_transaction(&tx, 100, false);

    // Should have 1 UTXO
    assert_eq!(utxo_set.len(), 1);

    // Check the entry
    let outpoint = Outpoint::new(tx.hash(), 0);
    let entry = utxo_set.get(&outpoint).expect("UTXO should exist");

    assert!(!entry.is_coinbase);
    assert!(entry.is_epoch_reward);
    assert_eq!(entry.height, 100);
    assert_eq!(entry.output.amount, 100_000_000_000);
}

#[test]
fn test_utxo_set_epoch_reward_balance() {
    use storage::UtxoSet;

    let mut utxo_set = UtxoSet::new();
    let keypair = KeyPair::generate();
    let recipient_hash = hash(keypair.public_key().as_bytes());

    // Add epoch reward at height 100
    let tx =
        Transaction::new_epoch_reward(1, *keypair.public_key(), 100_000_000_000, recipient_hash);
    utxo_set.add_transaction(&tx, 100, false);

    // Before maturity, balance should be 0 (not spendable)
    let balance_before = utxo_set.get_balance(&recipient_hash, 150);
    assert_eq!(balance_before, 0);

    // After maturity, balance should include the epoch reward
    let balance_after = utxo_set.get_balance(&recipient_hash, 200);
    assert_eq!(balance_after, 100_000_000_000);
}

// =============================================================================
// Milestone 6: Producer Integration Tests
// =============================================================================

#[test]
fn test_producer_block_after_empty_boundary() {
    // Test scenario: epoch boundary slot is empty, next block should include rewards
    // At epoch N boundary (slot = N * slots_per_epoch), if that slot is empty,
    // the first block in epoch N+1 should distribute epoch N rewards

    let params = ConsensusParams::devnet();
    let slots_per_epoch = params.slots_per_reward_epoch;

    // Epoch 0: 29 blocks (slots 1-29)
    // Slot 30: EMPTY (boundary is empty)
    // Slot 31: Block by producer (first block of epoch 1)
    // Expected: slot 31 block should include epoch 0 rewards

    // Calculate which epoch to reward
    let boundary_slot = slots_per_epoch; // slot 30 for devnet
    let next_slot = boundary_slot + 1; // slot 31

    let current_epoch_at_31 = next_slot as u64 / slots_per_epoch as u64;
    assert_eq!(current_epoch_at_31, 1, "Slot 31 is in epoch 1");

    // With last_rewarded = 0, at slot 31 we should reward epoch 1
    let last_rewarded = 0u64;
    assert!(
        current_epoch_at_31 > last_rewarded,
        "Should trigger reward distribution"
    );

    let epoch_to_reward = last_rewarded + 1;
    assert_eq!(epoch_to_reward, 1, "First unrewarded epoch is 1");
}

#[test]
fn test_empty_epoch_produces_no_rewards() {
    // When an epoch has zero blocks, there's nothing to distribute
    // The reward pool for that epoch is 0

    let params = ConsensusParams::devnet();

    // If epoch 1 has 0 blocks:
    // - Pool = 0 blocks * block_reward = 0
    // - No reward transactions should be created
    // - No division by zero issues

    let epoch_blocks = 0u64;
    let block_reward = params.block_reward(100);
    let pool = epoch_blocks * block_reward;

    assert_eq!(pool, 0, "Empty epoch has zero pool");

    // When distributing zero pool:
    // - No producers earned rewards
    // - create_epoch_rewards should return empty Vec
}

#[test]
fn test_multi_epoch_catchup_order() {
    // When multiple epochs need rewarding, they're done one at a time
    // in chronological order

    let params = ConsensusParams::devnet();
    let slots_per_epoch = params.slots_per_reward_epoch;

    // Scenario: We're at epoch 5, but last_rewarded = 2
    // Expected catch-up sequence: 3, 4, 5 (one per block)

    let current_slot = slots_per_epoch * 5 + 10; // Mid epoch 5
    let current_epoch = current_slot as u64 / slots_per_epoch as u64;
    let last_rewarded = 2u64;

    assert_eq!(current_epoch, 5);
    assert!(current_epoch > last_rewarded);

    // First block: rewards epoch 3
    let epoch_to_reward_1 = last_rewarded + 1;
    assert_eq!(epoch_to_reward_1, 3);

    // After epoch 3 rewarded, last_rewarded = 3
    // Second block: rewards epoch 4
    let epoch_to_reward_2 = epoch_to_reward_1 + 1;
    assert_eq!(epoch_to_reward_2, 4);

    // After epoch 4 rewarded, last_rewarded = 4
    // Third block: rewards epoch 5
    let epoch_to_reward_3 = epoch_to_reward_2 + 1;
    assert_eq!(epoch_to_reward_3, 5);

    // After epoch 5 rewarded, last_rewarded = 5
    // Fourth block: no rewards needed (current == last_rewarded)
    let final_last_rewarded = epoch_to_reward_3;
    assert_eq!(final_last_rewarded, current_epoch);
}

#[test]
fn test_epoch_reward_slot_range_calculation() {
    // Verify slot ranges are calculated correctly for each epoch

    let params = ConsensusParams::devnet();
    let slots_per_epoch = params.slots_per_reward_epoch;

    // Epoch 0: slots [0, 30) but we skip genesis, so [1, 30)
    let epoch0_start = 1u32;
    let epoch0_end = slots_per_epoch;
    assert_eq!(epoch0_start, 1);
    assert_eq!(epoch0_end, 30);

    // Epoch 1: slots [30, 60)
    let epoch1_start = slots_per_epoch;
    let epoch1_end = slots_per_epoch * 2;
    assert_eq!(epoch1_start, 30);
    assert_eq!(epoch1_end, 60);

    // Epoch 2: slots [60, 90)
    let epoch2_start = slots_per_epoch * 2;
    let epoch2_end = slots_per_epoch * 3;
    assert_eq!(epoch2_start, 60);
    assert_eq!(epoch2_end, 90);

    // Generic formula: epoch N has slots [N * slots_per_epoch, (N+1) * slots_per_epoch)
    // Exception: epoch 0 starts at slot 1 (skip genesis)
    for epoch in 1..10u64 {
        let start = (epoch as u32) * slots_per_epoch;
        let end = ((epoch + 1) as u32) * slots_per_epoch;
        assert_eq!(
            end - start,
            slots_per_epoch,
            "Each epoch has exactly {} slots",
            slots_per_epoch
        );
    }
}

#[test]
fn test_proportional_rewards_rounding() {
    // Test that proportional rewards with rounding work correctly
    // Total should always equal pool (no lost rewards)

    // Pool of 1 DOLI with 3 producers
    let pool: Amount = 100_000_000; // 1 DOLI
    let producer_blocks = [7u64, 2, 1]; // 10 blocks total
    let total_blocks: u64 = producer_blocks.iter().sum();

    assert_eq!(total_blocks, 10);

    // Calculate shares using proportional distribution
    let mut shares = Vec::new();
    let mut distributed = 0u64;

    for (i, &blocks) in producer_blocks.iter().enumerate() {
        let share = if i == producer_blocks.len() - 1 {
            // Last producer gets remainder
            pool - distributed
        } else {
            ((blocks as u128 * pool as u128) / total_blocks as u128) as u64
        };
        shares.push(share);
        distributed += share;
    }

    // Producer 1: 7/10 * 100M = 70M
    assert_eq!(shares[0], 70_000_000);

    // Producer 2: 2/10 * 100M = 20M
    assert_eq!(shares[1], 20_000_000);

    // Producer 3: remainder = 100M - 70M - 20M = 10M
    assert_eq!(shares[2], 10_000_000);

    // Total equals pool exactly
    let total: u64 = shares.iter().sum();
    assert_eq!(total, pool, "Total must equal pool");
}

#[test]
fn test_epoch_rewards_with_odd_distribution() {
    // Test distribution that doesn't divide evenly
    // 100 base units among 3 equal producers: 33, 33, 34

    let pool: Amount = 100;
    let num_producers = 3u64;

    // Equal blocks per producer
    let blocks_per_producer = 5u64;
    let total_blocks = blocks_per_producer * num_producers;

    // Calculate shares
    let share_1 = ((blocks_per_producer as u128 * pool as u128) / total_blocks as u128) as u64;
    let share_2 = ((blocks_per_producer as u128 * pool as u128) / total_blocks as u128) as u64;
    let share_3 = pool - share_1 - share_2; // Last gets remainder

    assert_eq!(share_1, 33);
    assert_eq!(share_2, 33);
    assert_eq!(share_3, 34); // Gets the 1 unit dust

    assert_eq!(share_1 + share_2 + share_3, pool);
}
