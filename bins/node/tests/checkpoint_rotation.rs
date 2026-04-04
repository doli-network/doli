//! Checkpoint Rotation Integration Tests (INC-I-020)
//!
//! Verifies that auto-checkpoint rotation keeps the 5 HIGHEST-height checkpoints,
//! not the 5 lexicographically-last ones. Uses real Node::new_for_test() with real
//! RocksDB, real SyncManager, and real periodic tasks — no mocks.
//!
//! The production bug: directory names like h526 sorted AFTER h4535 lexicographically
//! (because '5' > '4' in ASCII), so old checkpoints survived and new ones were
//! immediately deleted after creation. This test reproduces that exact scenario.

#[allow(dead_code)]
mod test_network;

use crypto::KeyPair;
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Transaction};
use doli_node::node::Node;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

// ============================================================
// HELPERS
// ============================================================

async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    (node, producers, temp)
}

fn build_block(
    height: u64,
    slot: u32,
    prev_hash: crypto::Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
) -> Block {
    let reward = params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase));
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: crypto::Hash::ZERO,
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

/// Produce one block and run periodic tasks, simulating real node behavior.
/// Returns the new chain height.
async fn produce_one_and_tick(
    node: &mut Node,
    producer: &KeyPair,
    params: &ConsensusParams,
) -> u64 {
    let (height, prev_hash) = {
        let cs = node.chain_state.read().await;
        (cs.best_height + 1, cs.best_hash)
    };
    let block = build_block(height, height as u32, prev_hash, producer, params);
    node.apply_block(block, ValidationMode::Light)
        .await
        .unwrap_or_else(|e| panic!("apply_block failed at h={}: {}", height, e));
    node.run_periodic_tasks().await.unwrap();
    height
}

/// List checkpoint directory names sorted numerically by height.
fn list_checkpoints(data_dir: &std::path::Path) -> Vec<String> {
    let checkpoint_dir = data_dir.join("checkpoints");
    if !checkpoint_dir.exists() {
        return Vec::new();
    }
    let mut dirs: Vec<String> = std::fs::read_dir(&checkpoint_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    dirs.sort_by_key(|name| checkpoint_height(name));
    dirs
}

/// Parse the height from a checkpoint directory name like "h526-1774849792".
fn checkpoint_height(name: &str) -> u64 {
    name.strip_prefix('h')
        .and_then(|s| s.split('-').next())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
}

fn checkpoint_heights(data_dir: &std::path::Path) -> Vec<u64> {
    list_checkpoints(data_dir)
        .iter()
        .map(|c| checkpoint_height(c))
        .collect()
}

// ============================================================
// TEST: Single node — checkpoint rotation across 1→2 digit boundary
// ============================================================

/// Reproduces the INC-I-020 production bug with a single real node:
///
/// 1. Produce blocks one-by-one with periodic tasks (like production)
/// 2. Checkpoints created at h=5, h=10, h=15, h=20, h=25
/// 3. At h=30, rotation triggers — h=5 must be deleted, h=30 must survive
///
/// The bug: h5 sorts AFTER h30 lexicographically ('5' > '3' in ASCII),
/// so with the old code h5 survived and h30 was deleted.
#[tokio::test]
async fn test_checkpoint_rotation_keeps_highest_not_lexicographic() {
    let (mut node, producers, tmp) = make_node(3).await;
    let params = node.params.clone();
    let data_dir = tmp.path().to_path_buf();

    // Enable auto-checkpoint every 5 blocks
    node.config.auto_checkpoint_interval = Some(5);

    // Produce 30 blocks one-by-one, running periodic tasks after each
    // This creates checkpoints at h=5, h=10, h=15, h=20, h=25, h=30
    for _ in 0..30 {
        produce_one_and_tick(&mut node, &producers[0], &params).await;
    }

    let heights = checkpoint_heights(&data_dir);
    assert_eq!(heights.len(), 5, "Must have exactly 5 checkpoints");

    // h=5 must be rotated out (6th checkpoint triggered rotation)
    assert!(
        !heights.contains(&5),
        "h=5 must have been rotated out, but got: {:?}",
        heights
    );

    // h=30 must survive (the newest checkpoint)
    assert!(
        heights.contains(&30),
        "h=30 must survive rotation, but got: {:?}",
        heights
    );

    // Exact expected set: the 5 highest
    assert_eq!(
        heights,
        vec![10, 15, 20, 25, 30],
        "Must keep the 5 highest checkpoints"
    );
}

// ============================================================
// TEST: Multi-node gossip network — checkpoint rotation works
// ============================================================

/// Simulates a 3-node network with gossip propagation.
/// One node has auto-checkpoint enabled. After producing and propagating
/// blocks across the digit boundary, verifies checkpoints are correct.
#[tokio::test]
async fn test_checkpoint_rotation_with_gossip_network() {
    let net = test_network::TestNetwork::new(3, 3).await;

    // Enable auto-checkpoint on node 0 (the "seed")
    {
        let mut n = net.nodes[0].lock().await;
        n.config.auto_checkpoint_interval = Some(5);
    }

    let data_dir = {
        let n = net.nodes[0].lock().await;
        n.config.data_dir.clone()
    };

    // Produce 32 blocks with gossip propagation, running periodic tasks after each
    for _ in 1..=32u64 {
        let (height, prev_hash) = {
            let n = net.nodes[0].lock().await;
            let cs = n.chain_state.read().await;
            (cs.best_height + 1, cs.best_hash)
        };
        let producer_idx = (height as usize) % net.producers.len();
        let block = net.build_block(
            height,
            height as u32,
            prev_hash,
            &net.producers[producer_idx],
        );

        // Apply to producer, gossip-propagate to peers
        net.apply_to_node(0, block.clone()).await.unwrap();
        net.propagate(0, block).await;

        // Run periodic tasks on seed to trigger checkpoint logic
        {
            let mut n = net.nodes[0].lock().await;
            n.run_periodic_tasks().await.unwrap();
        }
    }

    // All nodes synced at h=32
    assert!(net.is_synced().await, "Network must be synced at h=32");

    // Verify checkpoint rotation on the seed
    let heights = checkpoint_heights(&data_dir);
    assert_eq!(heights.len(), 5, "Seed must have exactly 5 checkpoints");

    // h=5 must be rotated out, highest 5 must survive
    assert!(
        !heights.contains(&5),
        "h=5 must be rotated out, got: {:?}",
        heights
    );
    assert_eq!(
        heights,
        vec![10, 15, 20, 25, 30],
        "Must keep the 5 highest checkpoints after gossip network"
    );

    // Verify health.json exists in each surviving checkpoint
    let checkpoint_base = data_dir.join("checkpoints");
    for cp in &list_checkpoints(&data_dir) {
        let health_path = checkpoint_base.join(cp).join("health.json");
        assert!(
            health_path.exists(),
            "health.json must exist in checkpoint {}",
            cp
        );

        let health: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&health_path).unwrap()).unwrap();
        assert!(health.get("height").is_some());
        assert!(health.get("healthy").is_some());
    }
}

// ============================================================
// TEST: Checkpoint rotation across 2→3 digit boundary
// ============================================================

/// Produces enough blocks to create checkpoints crossing the 2→3 digit
/// boundary (h=90, h=100, h=110). This is the exact class of bug that
/// hit production when checkpoints crossed from 3→4 digits.
///
/// With the old bug, h=90 ('9') sorts AFTER h=100 ('1') lexicographically,
/// so h=100 would be deleted and h=90 would survive forever.
#[tokio::test]
async fn test_checkpoint_rotation_2_to_3_digit_boundary() {
    let (mut node, producers, tmp) = make_node(3).await;
    let params = node.params.clone();
    let data_dir = tmp.path().to_path_buf();

    // Interval=10 for test speed
    node.config.auto_checkpoint_interval = Some(10);

    // Produce 110 blocks one-by-one
    // Creates checkpoints at h=10, 20, 30, ..., 100, 110
    // After h=60: rotation kicks in (6 checkpoints)
    for _ in 0..110 {
        produce_one_and_tick(&mut node, &producers[0], &params).await;
    }

    let heights = checkpoint_heights(&data_dir);
    assert_eq!(heights.len(), 5, "Must have exactly 5 checkpoints");

    // Critical: 3-digit h=100 and h=110 must survive alongside 2-digit h=70,h=80,h=90.
    // With the old bug, h=90 ('9') sorts AFTER h=100 ('1'), so h=100 would be deleted.
    assert_eq!(
        heights,
        vec![70, 80, 90, 100, 110],
        "2-digit and 3-digit checkpoints must coexist correctly"
    );

    // Verify last_checkpoint_height tracks correctly
    assert_eq!(
        node.last_checkpoint_height, 110,
        "last_checkpoint_height must track the highest checkpoint"
    );
}
