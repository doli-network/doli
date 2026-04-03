//! Integration tests for the EpochReward explicit inputs hard fork.
//!
//! Tests the consensus-breaking change at EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT (25,560):
//! - Pre-activation: EpochReward TX has empty inputs, pool consumed by side-effect
//! - Post-activation: EpochReward TX has explicit sorted pool UTXO inputs
//!
//! Tests 1-6 are fast unit-level tests (~ms).
//! Tests 7-8 build a full chain to the activation height (~60s each).

use std::collections::HashMap;

use crypto::{Hash, KeyPair};
use doli_core::consensus::{self, ConsensusParams, EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT};
use doli_core::transaction::{Output, Transaction, TxType};
use doli_core::validation::validate_transaction_with_utxos;
use doli_core::Block;
use doli_node::node::Node;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

// ============================================================
// HELPERS
// ============================================================

/// A non-zero recipient hash for test outputs (Hash::ZERO is rejected by structural validation).
fn test_recipient() -> Hash {
    crypto::hash::hash(b"test_recipient")
}

/// Mock UTXO provider for unit tests — maps (tx_hash, index) → UtxoInfo.
struct MockUtxoProvider {
    utxos: HashMap<(Hash, u32), doli_core::validation::UtxoInfo>,
}

impl MockUtxoProvider {
    fn new() -> Self {
        Self {
            utxos: HashMap::new(),
        }
    }

    fn add_pool_utxo(&mut self, tx_hash: Hash, index: u32, amount: u64) {
        let pool_hash = consensus::reward_pool_pubkey_hash();
        self.utxos.insert(
            (tx_hash, index),
            doli_core::validation::UtxoInfo {
                output: Output::normal(amount, pool_hash),
                pubkey: None,
                spent: false,
            },
        );
    }

    fn add_non_pool_utxo(&mut self, tx_hash: Hash, index: u32, amount: u64) {
        let random_hash = crypto::hash::hash(b"not_pool");
        self.utxos.insert(
            (tx_hash, index),
            doli_core::validation::UtxoInfo {
                output: Output::normal(amount, random_hash),
                pubkey: None,
                spent: false,
            },
        );
    }
}

impl doli_core::validation::UtxoProvider for MockUtxoProvider {
    fn get_utxo(
        &self,
        tx_hash: &Hash,
        output_index: u32,
    ) -> Option<doli_core::validation::UtxoInfo> {
        self.utxos.get(&(*tx_hash, output_index)).cloned()
    }
}

fn test_context() -> doli_core::validation::ValidationContext {
    doli_core::validation::ValidationContext::new(
        ConsensusParams::devnet(),
        doli_core::Network::Devnet,
        0,
        1,
    )
}

/// Create a test Node with N producers, real RocksDB.
/// Overrides blocks_per_era so the block reward never drops to 0 in the test range.
async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    // Devnet blocks_per_era = 576 → reward hits 0 at era 27 (h=15,552).
    // Set it large enough that reward stays positive through activation height.
    node.params.blocks_per_era = 100_000;
    (node, producers, temp)
}

/// Build a block with valid coinbase going to reward pool.
fn build_block(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
) -> Block {
    let reward = params.block_reward(height);
    let pool_hash = consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height);
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase));
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

    let header = doli_core::BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash,
        timestamp,
        slot,
        producer: *producer.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
    };

    Block::new(header, vec![coinbase])
}

/// Build a chain of N blocks.
fn build_chain(
    start_height: u64,
    start_slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    count: usize,
    params: &ConsensusParams,
) -> Vec<Block> {
    let mut blocks = Vec::with_capacity(count);
    let mut prev = prev_hash;
    for i in 0..count {
        let h = start_height + i as u64;
        let s = start_slot + i as u32;
        let block = build_block(h, s, prev, producer, params);
        prev = block.hash();
        blocks.push(block);
    }
    blocks
}

/// Apply blocks with Light validation.
async fn apply_chain(node: &mut Node, blocks: &[Block]) {
    for block in blocks {
        node.apply_block(block.clone(), doli_core::validation::ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("apply_block failed at h={}: {}", block.header.slot, e));
    }
}

// ============================================================
// TEST 1: TX construction — pre vs post activation format
// ============================================================
#[test]
fn test_epoch_reward_tx_construction_pre_activation() {
    let recipient = test_recipient();
    let outputs = vec![(100, recipient), (200, crypto::hash::hash(b"producer2"))];
    let tx = Transaction::new_epoch_reward_coinbase(vec![], outputs, 720, 1);

    assert_eq!(tx.tx_type, TxType::EpochReward);
    assert!(tx.inputs.is_empty());
    assert_eq!(tx.outputs.len(), 2);
    assert_eq!(tx.outputs[0].amount, 100);
    assert_eq!(tx.outputs[1].amount, 200);
}

#[test]
fn test_epoch_reward_tx_construction_post_activation() {
    let hash_a = crypto::hash::hash(b"tx_a");
    let hash_b = crypto::hash::hash(b"tx_b");
    let pool_inputs = vec![(hash_a, 0), (hash_b, 0)];
    let recipient = test_recipient();
    let tx =
        Transaction::new_epoch_reward_coinbase(pool_inputs, vec![(150, recipient)], 25_560, 71);

    assert_eq!(tx.tx_type, TxType::EpochReward);
    assert_eq!(tx.inputs.len(), 2);
    assert_eq!(tx.inputs[0].prev_tx_hash, hash_a);
    assert_eq!(tx.inputs[1].prev_tx_hash, hash_b);
    assert_eq!(tx.outputs[0].amount, 150);
}

// ============================================================
// TEST 2: UTXO validation — pre-activation empty inputs → OK
// ============================================================
#[test]
fn test_utxo_validation_pre_activation_empty_inputs() {
    let recipient = test_recipient();
    let tx = Transaction::new_epoch_reward_coinbase(vec![], vec![(100, recipient)], 720, 1);
    let provider = MockUtxoProvider::new();
    let ctx = test_context();

    let result = validate_transaction_with_utxos(&tx, &ctx, &provider);
    assert!(
        result.is_ok(),
        "Pre-activation EpochReward with empty inputs should pass: {:?}",
        result
    );
}

// ============================================================
// TEST 3: UTXO validation — post-activation explicit inputs → OK
// ============================================================
#[test]
fn test_utxo_validation_post_activation_explicit_inputs() {
    let hash_a = crypto::hash::hash(b"pool_tx_1");
    let hash_b = crypto::hash::hash(b"pool_tx_2");
    let recipient = test_recipient();

    let pool_inputs = vec![(hash_a, 0), (hash_b, 0)];
    let tx = Transaction::new_epoch_reward_coinbase(
        pool_inputs,
        vec![(200, recipient)],
        EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT,
        71,
    );

    let mut provider = MockUtxoProvider::new();
    provider.add_pool_utxo(hash_a, 0, 100);
    provider.add_pool_utxo(hash_b, 0, 100);

    let ctx = test_context();
    let result = validate_transaction_with_utxos(&tx, &ctx, &provider);
    assert!(
        result.is_ok(),
        "Post-activation EpochReward with explicit pool inputs should pass: {:?}",
        result
    );
}

// ============================================================
// TEST 4: UTXO validation — post-activation rejects non-pool input
// ============================================================
#[test]
fn test_utxo_validation_post_activation_non_pool_input_rejected() {
    let hash_a = crypto::hash::hash(b"pool_tx_1");
    let hash_b = crypto::hash::hash(b"non_pool_tx");
    let recipient = test_recipient();

    let pool_inputs = vec![(hash_a, 0), (hash_b, 0)];
    let tx = Transaction::new_epoch_reward_coinbase(
        pool_inputs,
        vec![(200, recipient)],
        EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT,
        71,
    );

    let mut provider = MockUtxoProvider::new();
    provider.add_pool_utxo(hash_a, 0, 100);
    provider.add_non_pool_utxo(hash_b, 0, 100); // NOT a pool UTXO

    let ctx = test_context();
    let result = validate_transaction_with_utxos(&tx, &ctx, &provider);
    assert!(
        result.is_err(),
        "EpochReward with non-pool input should be rejected"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("not a pool UTXO"),
        "Error should mention pool: {}",
        err
    );
}

// ============================================================
// TEST 5: UTXO validation — conservation (outputs > inputs → reject)
// ============================================================
#[test]
fn test_utxo_validation_conservation_violated() {
    let hash_a = crypto::hash::hash(b"pool_tx_1");
    let recipient = test_recipient();

    // 100 input, 200 output → violation
    let tx = Transaction::new_epoch_reward_coinbase(
        vec![(hash_a, 0)],
        vec![(200, recipient)],
        EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT,
        71,
    );

    let mut provider = MockUtxoProvider::new();
    provider.add_pool_utxo(hash_a, 0, 100);

    let ctx = test_context();
    let result = validate_transaction_with_utxos(&tx, &ctx, &provider);
    assert!(
        result.is_err(),
        "EpochReward where outputs > inputs should be rejected"
    );
}

// ============================================================
// TEST 6: Deterministic ordering of pool outpoints
// ============================================================
#[test]
fn test_deterministic_ordering() {
    let hash_c = crypto::hash::hash(b"tx_c");
    let hash_a = crypto::hash::hash(b"tx_a");
    let hash_b = crypto::hash::hash(b"tx_b");

    let mut outpoints_1: Vec<(Hash, u32)> =
        vec![(hash_c, 0), (hash_a, 1), (hash_b, 0), (hash_a, 0)];
    let mut outpoints_2: Vec<(Hash, u32)> =
        vec![(hash_b, 0), (hash_a, 0), (hash_c, 0), (hash_a, 1)];

    outpoints_1.sort();
    outpoints_2.sort();

    assert_eq!(
        outpoints_1, outpoints_2,
        "Sorted outpoints must be identical regardless of insertion order"
    );

    for w in outpoints_1.windows(2) {
        assert!(w[0] <= w[1], "Outpoints must be in ascending order");
    }
}

// ============================================================
// TEST 7: Pre-activation EpochReward drains pool via side-effect
//
// Verifies backward compat: at height < activation, an EpochReward TX
// with empty inputs correctly consumes pool UTXOs via the side-effect
// path in tx_processing.rs. Post-activation path is covered by unit
// tests 3-5 (UTXO validation with mock provider).
// ============================================================
#[tokio::test]
#[ignore] // Pre-activation path doesn't exist — all features active from genesis
async fn test_pre_activation_epoch_reward_drains_pool() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    // Build 7 blocks (devnet epoch=4, h=8 is epoch boundary, completed_epoch=1).
    let chain = build_chain(1, 1, Hash::ZERO, &producers[0], 7, &params);
    apply_chain(&mut node, &chain).await;

    let pool_hash = consensus::reward_pool_pubkey_hash();

    let pool_count_before = {
        let utxo = node.utxo_set.read().await;
        let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
        assert_eq!(
            pool_utxos.len(),
            7,
            "Pool should have 7 UTXOs after 7 blocks"
        );
        pool_utxos.len()
    };

    // Get pool total for the EpochReward output.
    let pool_total = {
        let utxo = node.utxo_set.read().await;
        let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
        pool_utxos.iter().map(|(_, e)| e.output.amount).sum::<u64>()
    };

    // Build an EpochReward TX with EMPTY inputs (pre-activation format).
    let recipient = crypto::hash::hash_with_domain(
        crypto::ADDRESS_DOMAIN,
        producers[0].public_key().as_bytes(),
    );
    let epoch_reward_tx = Transaction::new_epoch_reward_coinbase(
        vec![], // empty inputs — pre-activation format
        vec![(pool_total, recipient)],
        8,
        1,
    );

    // Build a block at h=8 (epoch boundary).
    let prev_hash = chain[6].hash();
    let coinbase = Transaction::new_coinbase(params.block_reward(8), pool_hash, 8);
    let txs = vec![coinbase, epoch_reward_tx];
    let merkle_root = doli_core::block::compute_merkle_root(&txs);
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

    let header = doli_core::BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash,
        timestamp: params.genesis_time + (8 * params.slot_duration),
        slot: 8,
        producer: *producers[0].public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
    };
    let block = Block::new(header, txs);

    node.apply_block(block, doli_core::validation::ValidationMode::Light)
        .await
        .expect("Pre-activation EpochReward should succeed");

    // Pool should have 0 UTXOs — the side-effect consumes ALL pool UTXOs
    // including the coinbase from this block (which was processed before EpochReward).
    let pool_count_after = {
        let utxo = node.utxo_set.read().await;
        utxo.get_by_pubkey_hash(&pool_hash).len()
    };

    assert_eq!(
        pool_count_after, 0,
        "Pool should be empty after EpochReward (all {} + coinbase consumed by side-effect)",
        pool_count_before
    );
}

// ============================================================
// TEST 8: Rollback of post-activation EpochReward restores pool UTXOs
// ============================================================
#[tokio::test]
#[ignore] // Test assumes pre-activation blocks exist — not applicable with genesis activation
async fn test_rollback_post_activation_restores_pool() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    // Build 7 blocks (h=8 is epoch boundary with completed_epoch=1).
    let chain = build_chain(1, 1, Hash::ZERO, &producers[0], 7, &params);
    apply_chain(&mut node, &chain).await;

    let pool_hash = consensus::reward_pool_pubkey_hash();

    let pool_total = {
        let utxo = node.utxo_set.read().await;
        let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
        pool_utxos.iter().map(|(_, e)| e.output.amount).sum::<u64>()
    };

    let recipient = crypto::hash::hash_with_domain(
        crypto::ADDRESS_DOMAIN,
        producers[0].public_key().as_bytes(),
    );
    let epoch_reward_tx = Transaction::new_epoch_reward_coinbase(
        vec![], // empty inputs — pre-activation
        vec![(pool_total, recipient)],
        8,
        1,
    );

    let prev_hash = chain[6].hash();
    let coinbase = Transaction::new_coinbase(params.block_reward(8), pool_hash, 8);
    let txs = vec![coinbase, epoch_reward_tx];
    let merkle_root = doli_core::block::compute_merkle_root(&txs);
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

    let header = doli_core::BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash,
        timestamp: params.genesis_time + (8 * params.slot_duration),
        slot: 8,
        producer: *producers[0].public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
    };
    let block = Block::new(header, txs);
    node.apply_block(block, doli_core::validation::ValidationMode::Light)
        .await
        .expect("apply_block should succeed");

    // Pool has 0 UTXOs (all consumed by side-effect, including this block's coinbase).
    let pool_after_apply = {
        let utxo = node.utxo_set.read().await;
        utxo.get_by_pubkey_hash(&pool_hash).len()
    };
    assert_eq!(pool_after_apply, 0);

    // Rollback: should restore all consumed pool UTXOs (7 old + 1 coinbase = 8).
    node.rollback_one_block().await.unwrap();

    let pool_after_rollback = {
        let utxo = node.utxo_set.read().await;
        utxo.get_by_pubkey_hash(&pool_hash).len()
    };

    // After rollback: 8 pool UTXOs restored (7 original + h=8 coinbase that was consumed).
    assert_eq!(
        pool_after_rollback, 8,
        "Rollback should restore 8 pool UTXOs (7 original + coinbase), got {}",
        pool_after_rollback
    );
}
