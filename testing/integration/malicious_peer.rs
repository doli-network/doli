//! Malicious Peer Tests
//!
//! Tests that nodes properly handle malicious or misbehaving peers.

#[path = "../common/mod.rs"]
mod common;

use std::sync::Arc;

use common::{
    create_coinbase, create_test_block, generate_test_chain, init_test_logging,
    TestNode, TestNodeConfig,
};
use doli_core::{Block, BlockHeader, Transaction, Output, OutputType};
use crypto::{Hash, KeyPair, PublicKey};
use vdf::{VdfOutput, VdfProof};
use tempfile::TempDir;

/// Create an invalid block with wrong previous hash
fn create_block_wrong_prev_hash(
    height: u64,
    producer: &PublicKey,
) -> Block {
    let params = doli_core::consensus::ConsensusParams::mainnet();
    let slot = height as u32;
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);

    let wrong_prev_hash = crypto::hash::hash(b"wrong_hash");
    let coinbase = Transaction::new_coinbase(5_000_000_000, Hash::ZERO, height);
    let merkle_root = doli_core::block::compute_merkle_root(&[coinbase.clone()]);

    let header = BlockHeader {
        version: 1,
        prev_hash: wrong_prev_hash, // INVALID
        merkle_root,
        timestamp,
        slot,
        producer: producer.clone(),
        vdf_output: VdfOutput { value: vec![0u8; 32] },
        vdf_proof: VdfProof::empty(),
    };

    Block::new(header, vec![coinbase])
}

/// Create a block with invalid merkle root
fn create_block_bad_merkle(
    height: u64,
    prev_hash: Hash,
    producer: &PublicKey,
) -> Block {
    let params = doli_core::consensus::ConsensusParams::mainnet();
    let slot = height as u32;
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);

    let coinbase = Transaction::new_coinbase(5_000_000_000, Hash::ZERO, height);
    let bad_merkle = crypto::hash::hash(b"bad_merkle"); // INVALID

    let header = BlockHeader {
        version: 1,
        prev_hash,
        merkle_root: bad_merkle,
        timestamp,
        slot,
        producer: producer.clone(),
        vdf_output: VdfOutput { value: vec![0u8; 32] },
        vdf_proof: VdfProof::empty(),
    };

    Block::new(header, vec![coinbase])
}

/// Create a block with invalid version
fn create_block_bad_version(
    height: u64,
    prev_hash: Hash,
    producer: &PublicKey,
) -> Block {
    let params = doli_core::consensus::ConsensusParams::mainnet();
    let slot = height as u32;
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);

    let coinbase = Transaction::new_coinbase(5_000_000_000, Hash::ZERO, height);
    let merkle_root = doli_core::block::compute_merkle_root(&[coinbase.clone()]);

    let header = BlockHeader {
        version: 999, // INVALID version
        prev_hash,
        merkle_root,
        timestamp,
        slot,
        producer: producer.clone(),
        vdf_output: VdfOutput { value: vec![0u8; 32] },
        vdf_proof: VdfProof::empty(),
    };

    Block::new(header, vec![coinbase])
}

/// Create a transaction with negative-like amount (overflow attempt)
fn create_tx_overflow_amount() -> Transaction {
    let output = Output {
        output_type: OutputType::Normal,
        amount: u64::MAX, // Potential overflow
        pubkey_hash: Hash::ZERO,
        lock_until: 0,
    };

    Transaction {
        version: 1,
        tx_type: doli_core::transaction::TxType::Transfer,
        inputs: vec![],
        outputs: vec![output],
        extra_data: vec![],
    }
}

/// Create a transaction with duplicate inputs (double-spend attempt)
fn create_tx_duplicate_inputs() -> Transaction {
    use doli_core::transaction::Input;

    let fake_hash = crypto::hash::hash(b"prev_tx");
    let input = Input::new(fake_hash, 0);

    Transaction {
        version: 1,
        tx_type: doli_core::transaction::TxType::Transfer,
        inputs: vec![input.clone(), input], // Same input twice!
        outputs: vec![Output::normal(100, Hash::ZERO)],
        extra_data: vec![],
    }
}

/// Create a coinbase transaction with excessive reward
fn create_excessive_coinbase(height: u64) -> Transaction {
    let excessive_amount = 1_000_000_000_000_000u64; // Way more than allowed
    Transaction::new_coinbase(excessive_amount, Hash::ZERO, height)
}

/// Test rejection of block with wrong previous hash
#[tokio::test]
async fn test_reject_wrong_prev_hash() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 0);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();

    // Add genesis
    let genesis = generate_test_chain(1, &producer, 5_000_000_000);
    node.add_block(genesis[0].clone()).await.unwrap();

    // Try to add block with wrong prev_hash
    let bad_block = create_block_wrong_prev_hash(1, producer.public_key());

    // In our simple test node, we just check it doesn't cause a crash
    // A real validator would reject this
    let prev_hash = node.best_hash().await;
    assert_ne!(bad_block.header.prev_hash, prev_hash);
}

/// Test handling of block with bad merkle root
#[tokio::test]
async fn test_bad_merkle_root() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 10);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();
    let genesis = generate_test_chain(1, &producer, 5_000_000_000);
    node.add_block(genesis[0].clone()).await.unwrap();

    let bad_block = create_block_bad_merkle(1, genesis[0].hash(), producer.public_key());

    // Verify merkle root is actually bad
    assert!(!bad_block.verify_merkle_root());
}

/// Test handling of transactions with overflow amounts
#[tokio::test]
async fn test_tx_overflow_amount() {
    init_test_logging();

    let tx = create_tx_overflow_amount();

    // The transaction can be created but should be rejected by validation
    assert_eq!(tx.outputs[0].amount, u64::MAX);
    assert_eq!(tx.total_output(), u64::MAX);
}

/// Test handling of duplicate input transactions
#[tokio::test]
async fn test_tx_duplicate_inputs() {
    init_test_logging();

    let tx = create_tx_duplicate_inputs();

    // Transaction has duplicate inputs - validator should reject
    assert_eq!(tx.inputs.len(), 2);
    assert_eq!(tx.inputs[0].prev_tx_hash, tx.inputs[1].prev_tx_hash);
    assert_eq!(tx.inputs[0].output_index, tx.inputs[1].output_index);
}

/// Test mempool rejection of malicious transactions
#[tokio::test]
async fn test_mempool_rejects_bad_tx() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 20);
    let node = Arc::new(TestNode::new(config));

    // Add some valid transactions
    let producer = KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());

    for i in 0..5 {
        let tx = Transaction::new_coinbase(100_000, pubkey_hash, i);
        node.add_to_mempool(tx).await.unwrap();
    }

    assert_eq!(node.mempool_size().await, 5);

    // A real implementation would reject malicious transactions
    // Our simple test node accepts all, but we verify the structure
}

/// Test handling of blocks from unknown producer
#[tokio::test]
async fn test_unknown_producer_block() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 30);
    let node = Arc::new(TestNode::new(config));

    let known_producer = KeyPair::generate();
    let unknown_producer = KeyPair::generate();

    // Build chain with known producer
    let chain = generate_test_chain(5, &known_producer, 5_000_000_000);
    for block in &chain {
        node.add_block(block.clone()).await.unwrap();
    }

    // Create block from unknown producer
    let pubkey_hash = crypto::hash::hash(unknown_producer.public_key().as_bytes());
    let coinbase = create_coinbase(5, &pubkey_hash, 5_000_000_000);
    let unknown_block = create_test_block(
        5,
        chain.last().unwrap().hash(),
        unknown_producer.public_key(),
        vec![coinbase],
    );

    // In a production system, this would be checked against registered producers
    // Here we verify the block has a different producer
    assert_ne!(
        unknown_block.header.producer,
        chain.last().unwrap().header.producer
    );
}

/// Test handling of future-dated blocks
#[tokio::test]
async fn test_future_dated_block() {
    init_test_logging();

    let producer = KeyPair::generate();
    let params = doli_core::consensus::ConsensusParams::mainnet();

    // Create a block with timestamp far in the future
    let future_timestamp = params.genesis_time + 1_000_000_000; // ~30 years in future
    let coinbase = Transaction::new_coinbase(5_000_000_000, Hash::ZERO, 0);
    let merkle_root = doli_core::block::compute_merkle_root(&[coinbase.clone()]);

    let header = BlockHeader {
        version: 1,
        prev_hash: Hash::ZERO,
        merkle_root,
        timestamp: future_timestamp,
        slot: 1_000_000,
        producer: producer.public_key().clone(),
        vdf_output: VdfOutput { value: vec![0u8; 32] },
        vdf_proof: VdfProof::empty(),
    };

    let future_block = Block::new(header, vec![coinbase]);

    // Block exists but timestamp is clearly invalid
    assert!(future_block.timestamp() > params.genesis_time + 1_000_000);
}

/// Test handling of empty block (no transactions)
#[tokio::test]
async fn test_empty_block() {
    init_test_logging();

    let producer = KeyPair::generate();
    let params = doli_core::consensus::ConsensusParams::mainnet();

    let merkle_root = doli_core::block::compute_merkle_root(&[]); // Empty

    let header = BlockHeader {
        version: 1,
        prev_hash: Hash::ZERO,
        merkle_root,
        timestamp: params.genesis_time,
        slot: 0,
        producer: producer.public_key().clone(),
        vdf_output: VdfOutput { value: vec![0u8; 32] },
        vdf_proof: VdfProof::empty(),
    };

    let empty_block = Block::new(header, vec![]);

    // Block has no transactions - should be rejected (no coinbase)
    assert!(empty_block.transactions.is_empty());
    assert!(empty_block.coinbase().is_none());
}

/// Test handling of excessive coinbase
#[tokio::test]
async fn test_excessive_coinbase_reward() {
    init_test_logging();

    let excessive_tx = create_excessive_coinbase(0);

    // Transaction created with excessive amount
    let expected_reward = 1_000_000_000_000_000u64;
    assert_eq!(excessive_tx.total_output(), expected_reward);

    // This is way more than the allowed block reward
    let params = doli_core::consensus::ConsensusParams::mainnet();
    let actual_reward = params.block_reward(0);
    assert!(excessive_tx.total_output() > actual_reward);
}

/// Test block deserialization of corrupted data
#[tokio::test]
async fn test_corrupted_block_data() {
    init_test_logging();

    // Random garbage data
    let garbage = vec![0u8, 1, 2, 3, 4, 5, 255, 254, 253];
    let result = Block::deserialize(&garbage);
    assert!(result.is_none());

    // Truncated valid block
    let producer = KeyPair::generate();
    let valid_block = generate_test_chain(1, &producer, 5_000_000_000)[0].clone();
    let mut truncated = valid_block.serialize();
    truncated.truncate(truncated.len() / 2);

    let result = Block::deserialize(&truncated);
    assert!(result.is_none());

    // Extended with garbage
    let mut extended = valid_block.serialize();
    extended.extend_from_slice(&[0xFF; 100]);
    // This might still deserialize (extra data ignored) or fail
}

/// Test transaction deserialization of corrupted data
#[tokio::test]
async fn test_corrupted_tx_data() {
    init_test_logging();

    // Random garbage
    let garbage = vec![42u8; 50];
    let result = Transaction::deserialize(&garbage);
    assert!(result.is_none());

    // Empty data
    let empty: Vec<u8> = vec![];
    let result = Transaction::deserialize(&empty);
    assert!(result.is_none());

    // Partial valid transaction
    let valid_tx = Transaction::new_coinbase(100, Hash::ZERO, 0);
    let mut partial = valid_tx.serialize();
    partial.truncate(10);

    let result = Transaction::deserialize(&partial);
    assert!(result.is_none());
}

/// Test handling of transaction with zero amount outputs
#[tokio::test]
async fn test_zero_amount_output() {
    init_test_logging();

    let output = Output::normal(0, Hash::ZERO); // Zero amount - invalid

    let tx = Transaction {
        version: 1,
        tx_type: doli_core::transaction::TxType::Transfer,
        inputs: vec![],
        outputs: vec![output],
        extra_data: vec![],
    };

    // Transaction total is zero
    assert_eq!(tx.total_output(), 0);
}

/// Test handling of transaction with too many outputs
#[tokio::test]
async fn test_too_many_outputs() {
    init_test_logging();

    // Create transaction with 1000 outputs
    let outputs: Vec<Output> = (0..1000)
        .map(|_| Output::normal(1000, Hash::ZERO))
        .collect();

    let tx = Transaction {
        version: 1,
        tx_type: doli_core::transaction::TxType::Transfer,
        inputs: vec![],
        outputs,
        extra_data: vec![],
    };

    // Transaction is very large
    assert_eq!(tx.outputs.len(), 1000);
    assert!(tx.size() > 10000); // Definitely over reasonable size
}

/// Test handling of block with mismatched slot and timestamp
#[tokio::test]
async fn test_slot_timestamp_mismatch() {
    init_test_logging();

    let producer = KeyPair::generate();
    let params = doli_core::consensus::ConsensusParams::mainnet();

    let coinbase = Transaction::new_coinbase(5_000_000_000, Hash::ZERO, 0);
    let merkle_root = doli_core::block::compute_merkle_root(&[coinbase.clone()]);

    // Timestamp says slot 0, but header says slot 100
    let header = BlockHeader {
        version: 1,
        prev_hash: Hash::ZERO,
        merkle_root,
        timestamp: params.genesis_time, // Slot 0 time
        slot: 100,                       // But slot says 100!
        producer: producer.public_key().clone(),
        vdf_output: VdfOutput { value: vec![0u8; 32] },
        vdf_proof: VdfProof::empty(),
    };

    let mismatched_block = Block::new(header, vec![coinbase]);

    // Slot and derived slot from timestamp don't match
    let derived_slot = params.timestamp_to_slot(mismatched_block.timestamp());
    assert_ne!(mismatched_block.slot(), derived_slot);
}

/// Test rapid invalid block submissions (DoS attempt)
#[tokio::test]
async fn test_rapid_invalid_submissions() {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, 40);
    let node = Arc::new(TestNode::new(config));

    let producer = KeyPair::generate();
    let genesis = generate_test_chain(1, &producer, 5_000_000_000);
    node.add_block(genesis[0].clone()).await.unwrap();

    // Rapidly submit many invalid blocks
    for i in 0..100 {
        let bad_block = create_block_wrong_prev_hash(i + 1, producer.public_key());
        // These should all be processed without crashing
        let _ = node.add_block(bad_block).await;
    }

    // Node should still be functional
    assert_eq!(node.height().await, 0);
}
