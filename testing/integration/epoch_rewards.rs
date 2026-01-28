//! Epoch Rewards Integration Tests
//!
//! Tests the Pool-First Epoch Reward Distribution System:
//! - Rewards accumulate in pool during epoch (no coinbase per block)
//! - At epoch boundary, pool divided equally among all participating producers
//! - First producer (sorted by pubkey) receives any remainder
//! - Epoch reward outputs require 100 confirmations (REWARD_MATURITY)

#[path = "../common/mod.rs"]
mod common;

use doli_core::{
    consensus::{ConsensusParams, RewardMode, REWARD_MATURITY},
    transaction::{Transaction, TxType, Output},
    Amount,
};
use crypto::{Hash, KeyPair, hash::hash};
use storage::{UtxoEntry, Outpoint};

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

    let tx = Transaction::new_epoch_reward(
        epoch,
        keypair.public_key().clone(),
        amount,
        recipient_hash,
    );

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

    let tx = Transaction::new_epoch_reward(
        epoch,
        keypair.public_key().clone(),
        amount,
        recipient_hash,
    );

    // Verify epoch reward data can be extracted
    let data = tx.epoch_reward_data().expect("Should have epoch reward data");
    assert_eq!(data.epoch, epoch);
    assert_eq!(data.recipient, *keypair.public_key());
}

#[test]
fn test_epoch_reward_has_correct_type() {
    let keypair = KeyPair::generate();
    let recipient_hash = hash(keypair.public_key().as_bytes());

    let tx = Transaction::new_epoch_reward(
        1,
        keypair.public_key().clone(),
        1000,
        recipient_hash,
    );

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
    let tx = Transaction::new_epoch_reward(
        1,
        keypair.public_key().clone(),
        100_000_000_000,
        recipient_hash,
    );

    // Create UTXO entry for the epoch reward (created at height 100)
    let entry = UtxoEntry {
        output: tx.outputs[0].clone(),
        height: 100,
        is_coinbase: false,
        is_epoch_reward: true,
    };

    // Should NOT be spendable before maturity (100 confirmations)
    assert!(!entry.is_spendable_at(100)); // 0 confirmations
    assert!(!entry.is_spendable_at(150)); // 50 confirmations
    assert!(!entry.is_spendable_at(199)); // 99 confirmations

    // Should be spendable at and after maturity
    assert!(entry.is_spendable_at(200)); // Exactly 100 confirmations
    assert!(entry.is_spendable_at(300)); // 200 confirmations
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

    // Coinbase should also require 100 confirmations
    assert!(!entry.is_spendable_at(50));  // 0 confirmations
    assert!(!entry.is_spendable_at(100)); // 50 confirmations
    assert!(!entry.is_spendable_at(149)); // 99 confirmations
    assert!(entry.is_spendable_at(150));  // 100 confirmations
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
    assert!(entry.is_spendable_at(50)); // Same height
    assert!(entry.is_spendable_at(51)); // Next height
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
    let mut pubkeys: Vec<_> = producers.iter().map(|kp| kp.public_key().clone()).collect();

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
        keypair.public_key().clone(),
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
    let tx = Transaction::new_epoch_reward(
        u64::MAX,
        keypair.public_key().clone(),
        1000,
        recipient_hash,
    );

    let data = tx.epoch_reward_data().unwrap();
    assert_eq!(data.epoch, u64::MAX);
}

#[test]
fn test_reward_maturity_constant() {
    // REWARD_MATURITY should be 100
    assert_eq!(REWARD_MATURITY, 100);
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

    let tx = Transaction::new_epoch_reward(
        1,
        keypair.public_key().clone(),
        100_000_000_000,
        recipient_hash,
    );

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
    let tx = Transaction::new_epoch_reward(
        1,
        keypair.public_key().clone(),
        100_000_000_000,
        recipient_hash,
    );
    utxo_set.add_transaction(&tx, 100, false);

    // Before maturity, balance should be 0 (not spendable)
    let balance_before = utxo_set.get_balance(&recipient_hash, 150);
    assert_eq!(balance_before, 0);

    // After maturity, balance should include the epoch reward
    let balance_after = utxo_set.get_balance(&recipient_hash, 200);
    assert_eq!(balance_after, 100_000_000_000);
}
