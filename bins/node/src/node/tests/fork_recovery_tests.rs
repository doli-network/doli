//! Fork Recovery Integration Tests
//!
//! Tests the real Node struct with real RocksDB, real ProducerSet, real SyncManager,
//! and real fork recovery counters. No networking, no gossip — blocks are injected
//! manually to simulate forks and peer synchronization.
//!
//! Every fork recovery field (cumulative_rollback_depth, consecutive_fork_blocks,
//! excluded_producers, cached_scheduler, epoch_bond_snapshot) is the REAL field
//! from the production Node. No mocks. No shortcuts.

use super::*;
use tempfile::TempDir;

/// Helper: create a Node with N producers, all registered with 1 bond each
async fn make_test_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    (node, producers, temp)
}

/// Helper: build a valid block that the node can apply
fn build_test_block(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
) -> Block {
    let reward = params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(&[coinbase.clone()]);

    let header = BlockHeader {
        version: 1,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
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

/// Helper: build a chain of N blocks from a given ancestor
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
        let block = build_test_block(h, s, prev, producer, params);
        prev = block.hash();
        blocks.push(block);
    }
    blocks
}

// ============================================================
// TEST 0: Node::new_for_test creates a working node
// ============================================================
#[tokio::test]
async fn test_node_for_test_initializes() {
    let (node, producers, _tmp) = make_test_node(3).await;

    // Real state
    assert_eq!(node.chain_state.read().await.best_height, 0);
    assert_eq!(node.producer_set.read().await.active_count(), 3);
    assert_eq!(node.cumulative_rollback_depth, 0);
    assert_eq!(node.consecutive_fork_blocks, 0);
    assert_eq!(node.shallow_rollback_count, 0);
    assert!(node.excluded_producers.is_empty());
    assert_eq!(node.epoch_bond_snapshot.len(), 3);
    assert!(node.producer_key.is_some());

    // Verify each producer has 1 bond in the snapshot
    for kp in &producers {
        let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, kp.public_key().as_bytes());
        assert_eq!(node.epoch_bond_snapshot.get(&pubkey_hash), Some(&1));
    }
}

// ============================================================
// TEST 0b: Node can apply blocks
// ============================================================
#[tokio::test]
async fn test_node_can_apply_blocks() {
    let (mut node, producers, _tmp) = make_test_node(3).await;
    let params = node.params.clone();
    let genesis_hash = node.chain_state.read().await.best_hash;

    // Build and apply 5 blocks
    let chain = build_chain(1, 1, genesis_hash, &producers[0], 5, &params);
    for block in &chain {
        node.apply_block(block.clone(), ValidationMode::Light)
            .await
            .expect("apply_block failed");
    }

    assert_eq!(node.chain_state.read().await.best_height, 5);
    assert_eq!(node.chain_state.read().await.best_hash, chain[4].hash());
}

// ============================================================
// TEST 0c: Node can rollback blocks
// ============================================================
#[tokio::test]
async fn test_node_can_rollback() {
    let (mut node, producers, _tmp) = make_test_node(3).await;
    let params = node.params.clone();
    let genesis_hash = node.chain_state.read().await.best_hash;

    // Build and apply 5 blocks
    let chain = build_chain(1, 1, genesis_hash, &producers[0], 5, &params);
    for block in &chain {
        node.apply_block(block.clone(), ValidationMode::Light)
            .await
            .unwrap();
    }
    assert_eq!(node.chain_state.read().await.best_height, 5);

    // Rollback 1 block
    let rolled = node.rollback_one_block().await.unwrap();
    assert!(rolled);
    assert_eq!(node.chain_state.read().await.best_height, 4);
    assert_eq!(node.cumulative_rollback_depth, 1);
}
