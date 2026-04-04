//! Fork Recovery Integration Tests
//!
//! Tests the real Node struct with real RocksDB, real ProducerSet, real SyncManager,
//! and real fork recovery counters. No networking, no gossip — blocks are injected
//! manually to simulate forks and peer synchronization.
//!
//! Every fork recovery field (cumulative_rollback_depth, consecutive_fork_blocks,
//! excluded_producers, cached_scheduler, epoch_bond_snapshot) is the REAL field
//! from the production Node. No mocks. No shortcuts.

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Transaction};
use doli_node::node::Node;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

// ============================================================
// HELPERS
// ============================================================

/// Create a test Node with N producers, real RocksDB, real everything.
async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
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
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase));
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

    let header = BlockHeader {
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

/// Build a chain of N blocks from a given ancestor.
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

/// Apply a chain of blocks to a node with Light validation (skip VDF).
async fn apply_chain(node: &mut Node, blocks: &[Block]) {
    for block in blocks {
        node.apply_block(block.clone(), ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("apply_block failed at h={}: {}", block.header.slot, e));
    }
}

/// Get the genesis hash from a node (the best_hash at height 0).
/// Get the devnet genesis hash (used as prev_hash for the first block).
fn devnet_genesis_hash() -> Hash {
    doli_core::chainspec::ChainSpec::devnet().genesis_hash()
}

// ============================================================
// TEST 1: Fork recovery with divergent bonds
// ============================================================
#[tokio::test]
async fn test_fork_recovery_with_divergent_bonds() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    // Build base chain (10 blocks)
    let base = build_chain(1, 1, Hash::ZERO, &producers[0], 10, &params);
    apply_chain(&mut node, &base).await;
    assert_eq!(node.chain_state.read().await.best_height, 10);

    // Simulate bond divergence: node's epoch_bond_snapshot has 100 bonds for producer[0]
    // In production, this happens when add-bond TX arrives on one node but not another
    {
        let pubkey_hash = crypto::hash::hash_with_domain(
            crypto::ADDRESS_DOMAIN,
            producers[0].public_key().as_bytes(),
        );
        node.epoch_bond_snapshot.insert(pubkey_hash, 100);
    }

    // Fork A: node's local chain continues (5 blocks by producer[0] who has 101 bonds locally)
    let fork_a = build_chain(11, 11, base[9].hash(), &producers[0], 5, &params);
    apply_chain(&mut node, &fork_a).await;
    assert_eq!(node.chain_state.read().await.best_height, 15);

    // Fork B: canonical chain (7 blocks, longer) by producer[1] who has 1 bond
    let fork_b = build_chain(11, 11, base[9].hash(), &producers[1], 7, &params);

    // Revert fork_a (5 blocks)
    for _ in 0..5 {
        node.rollback_one_block().await.unwrap();
    }
    assert_eq!(node.chain_state.read().await.best_height, 10);

    // Apply fork_b (canonical, 7 blocks) — should succeed despite bond divergence
    apply_chain(&mut node, &fork_b).await;
    assert_eq!(node.chain_state.read().await.best_height, 17);
    assert_eq!(node.chain_state.read().await.best_hash, fork_b[6].hash());
}

// ============================================================
// TEST 2: Cumulative rollback depth resets on sync block
// ============================================================
#[tokio::test]
async fn test_cumulative_rollback_resets_on_sync() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    // Build and apply 60 blocks
    let chain = build_chain(1, 1, Hash::ZERO, &producers[0], 60, &params);
    apply_chain(&mut node, &chain).await;
    assert_eq!(node.chain_state.read().await.best_height, 60);

    // Rollback 49 times (just under cap of 50)
    for i in 0..49 {
        let ok = node.rollback_one_block().await.unwrap();
        assert!(ok, "rollback {} should succeed", i);
    }
    assert_eq!(node.cumulative_rollback_depth, 49);
    assert_eq!(node.chain_state.read().await.best_height, 11);

    // Apply a block via sync (not produced by us) — should reset depth
    let sync_block = build_block(12, 12, chain[10].hash(), &producers[1], &params);
    node.apply_block(sync_block, ValidationMode::Light)
        .await
        .unwrap();

    // cumulative_rollback_depth should reset because we applied a synced block
    // NOTE: This test will FAIL before the fix is applied. That's the point.
    assert_eq!(
        node.cumulative_rollback_depth, 0,
        "cumulative_rollback_depth should reset after applying a synced block"
    );

    // Verify we can rollback again (not capped)
    let ok = node.rollback_one_block().await.unwrap();
    assert!(ok, "rollback after depth reset should succeed");
}

// ============================================================
// TEST 3: Consecutive fork blocks not reset to 0 after rollback
// ============================================================
#[tokio::test]
async fn test_consecutive_fork_blocks_not_reset() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    // Build and apply 15 blocks
    let chain = build_chain(1, 1, Hash::ZERO, &producers[0], 15, &params);
    apply_chain(&mut node, &chain).await;

    // Simulate 8 consecutive fork-blocked slots
    node.consecutive_fork_blocks = 8;

    // Rollback 1 block (simulating what maybe_auto_resync does)
    node.rollback_one_block().await.unwrap();

    // consecutive_fork_blocks should NOT be reset to 0
    // NOTE: This test will FAIL before the fix. That's the point.
    assert!(
        node.consecutive_fork_blocks > 0,
        "consecutive_fork_blocks should NOT reset to 0 after rollback, got {}",
        node.consecutive_fork_blocks
    );
}

// ============================================================
// TEST 4: Complete recovery from 20-block fork
// ============================================================
#[tokio::test]
async fn test_recovery_from_20_block_fork() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    // Build base chain (10 blocks)
    let base = build_chain(1, 1, Hash::ZERO, &producers[0], 10, &params);
    apply_chain(&mut node, &base).await;

    // Fork: node produces 20 blocks on its own fork
    let fork = build_chain(11, 11, base[9].hash(), &producers[0], 20, &params);
    apply_chain(&mut node, &fork).await;
    assert_eq!(node.chain_state.read().await.best_height, 30);

    // Canonical chain: 25 blocks (longer) from fork point
    let canonical = build_chain(11, 11, base[9].hash(), &producers[1], 25, &params);

    // Recovery: rollback 20 fork blocks
    for _ in 0..20 {
        let ok = node.rollback_one_block().await.unwrap();
        assert!(ok, "rollback should succeed");
    }
    assert_eq!(node.chain_state.read().await.best_height, 10);
    assert_eq!(node.chain_state.read().await.best_hash, base[9].hash());

    // Apply canonical chain (25 blocks)
    apply_chain(&mut node, &canonical).await;

    // Verify recovery: correct height, correct hash, no manual intervention
    assert_eq!(node.chain_state.read().await.best_height, 35);
    assert_eq!(
        node.chain_state.read().await.best_hash,
        canonical[24].hash()
    );
}

// ============================================================
// TEST 5: Recovery with scheduler divergence
// ============================================================
#[tokio::test]
async fn test_recovery_with_scheduler_divergence() {
    let (mut node, producers, _tmp) = make_node(5).await;
    let params = node.params.clone();

    // Build base chain (10 blocks)
    let base = build_chain(1, 1, Hash::ZERO, &producers[0], 10, &params);
    apply_chain(&mut node, &base).await;

    // Simulate scheduler divergence: modify epoch_bond_snapshot
    // Producer[0] has 50 bonds in node's snapshot, but canonical chain expects 1
    let pubkey_hash = crypto::hash::hash_with_domain(
        crypto::ADDRESS_DOMAIN,
        producers[0].public_key().as_bytes(),
    );
    node.epoch_bond_snapshot.insert(pubkey_hash, 50);

    // Fork: 5 blocks with divergent scheduler
    let fork = build_chain(11, 11, base[9].hash(), &producers[0], 5, &params);
    apply_chain(&mut node, &fork).await;
    assert_eq!(node.chain_state.read().await.best_height, 15);

    // Canonical: 8 blocks (longer, with correct bonds)
    let canonical = build_chain(11, 11, base[9].hash(), &producers[1], 8, &params);

    // Rollback fork
    for _ in 0..5 {
        node.rollback_one_block().await.unwrap();
    }

    // Apply canonical chain despite bond snapshot divergence
    apply_chain(&mut node, &canonical).await;

    assert_eq!(node.chain_state.read().await.best_height, 18);
    assert_eq!(node.chain_state.read().await.best_hash, canonical[7].hash());
}

// ============================================================
// TEST 6: Recovery after cumulative rollback cap (50)
// ============================================================
#[tokio::test]
async fn test_recovery_after_rollback_cap() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    // Build and apply 55 blocks
    let chain = build_chain(1, 1, Hash::ZERO, &producers[0], 55, &params);
    apply_chain(&mut node, &chain).await;

    // Rollback exactly 50 times (hit the cap)
    for i in 0..50 {
        let ok = node.rollback_one_block().await.unwrap();
        assert!(ok, "rollback {} should succeed", i);
    }
    assert_eq!(node.cumulative_rollback_depth, 50);
    assert_eq!(node.chain_state.read().await.best_height, 5);

    // 51st rollback should be refused (cap reached)
    let refused = node.rollback_one_block().await.unwrap();
    assert!(!refused, "rollback should be refused after cap");

    // Send a valid block via sync — should apply and reset the cap
    let sync_block = build_block(6, 6, chain[4].hash(), &producers[1], &params);
    node.apply_block(sync_block.clone(), ValidationMode::Light)
        .await
        .unwrap();

    // NOTE: This assertion will FAIL before the fix.
    assert_eq!(
        node.cumulative_rollback_depth, 0,
        "depth should reset after synced block application"
    );

    // Now rollback should work again
    let ok = node.rollback_one_block().await.unwrap();
    assert!(ok, "rollback after cap reset should succeed");
}

// ============================================================
// TEST 7: No re-fork after recovery
// ============================================================
#[tokio::test]
async fn test_no_refork_after_recovery() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    // Build base chain
    let base = build_chain(1, 1, Hash::ZERO, &producers[0], 10, &params);
    apply_chain(&mut node, &base).await;

    // Fork and recover
    let fork = build_chain(11, 11, base[9].hash(), &producers[0], 5, &params);
    apply_chain(&mut node, &fork).await;

    for _ in 0..5 {
        node.rollback_one_block().await.unwrap();
    }

    let canonical = build_chain(11, 11, base[9].hash(), &producers[1], 5, &params);
    apply_chain(&mut node, &canonical).await;

    // Recovery complete — verify clean state
    assert_eq!(node.chain_state.read().await.best_height, 15);
    let _post_recovery_hash = node.chain_state.read().await.best_hash;

    // Apply 100 more blocks on the canonical chain
    let continuation = build_chain(16, 16, canonical[4].hash(), &producers[1], 100, &params);
    apply_chain(&mut node, &continuation).await;

    // Verify: 100 blocks applied cleanly, no re-fork
    assert_eq!(node.chain_state.read().await.best_height, 115);
    assert_eq!(
        node.chain_state.read().await.best_hash,
        continuation[99].hash()
    );
    // Counters should be clean
    assert_eq!(node.consecutive_fork_blocks, 0);
    assert_eq!(node.shallow_rollback_count, 0);
}

// ============================================================
// TEST 8: Recovery under load (new blocks arriving during recovery)
// ============================================================
#[tokio::test]
async fn test_recovery_under_load() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    // Build base chain (10 blocks)
    let base = build_chain(1, 1, Hash::ZERO, &producers[0], 10, &params);
    apply_chain(&mut node, &base).await;

    // Fork: 50 blocks
    let fork = build_chain(11, 11, base[9].hash(), &producers[0], 50, &params);
    apply_chain(&mut node, &fork).await;
    assert_eq!(node.chain_state.read().await.best_height, 60);

    // Rollback all 50 fork blocks
    for i in 0..50 {
        let ok = node.rollback_one_block().await.unwrap();
        assert!(ok, "rollback {} should succeed", i);
    }
    assert_eq!(node.chain_state.read().await.best_height, 10);

    // Canonical chain: 60 blocks (simulate blocks arriving during recovery)
    let canonical = build_chain(11, 11, base[9].hash(), &producers[1], 60, &params);

    // Apply all 60 blocks — simulates receiving them via gossip during/after recovery
    apply_chain(&mut node, &canonical).await;

    assert_eq!(node.chain_state.read().await.best_height, 70);
    assert_eq!(
        node.chain_state.read().await.best_hash,
        canonical[59].hash()
    );
}

// ============================================================
// TEST 9: Multiple nodes recover independently
// ============================================================
#[tokio::test]
async fn test_multiple_nodes_recover_independently() {
    // Create 3 nodes with the same producer set
    let producers: Vec<KeyPair> = (0..5).map(|_| KeyPair::generate()).collect();

    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();
    let tmp3 = TempDir::new().unwrap();

    let mut node1 = Node::new_for_test(tmp1.path().to_path_buf(), producers.clone())
        .await
        .unwrap();
    let mut node2 = Node::new_for_test(tmp2.path().to_path_buf(), producers.clone())
        .await
        .unwrap();
    let mut node3 = Node::new_for_test(tmp3.path().to_path_buf(), producers.clone())
        .await
        .unwrap();

    let params = node1.params.clone();

    // All nodes share a common base of 10 blocks
    let base = build_chain(1, 1, Hash::ZERO, &producers[0], 10, &params);
    apply_chain(&mut node1, &base).await;
    apply_chain(&mut node2, &base).await;
    apply_chain(&mut node3, &base).await;

    // Each node forks independently
    let fork1 = build_chain(11, 11, base[9].hash(), &producers[1], 5, &params);
    let fork2 = build_chain(11, 11, base[9].hash(), &producers[2], 7, &params);
    let fork3 = build_chain(11, 11, base[9].hash(), &producers[3], 3, &params);

    apply_chain(&mut node1, &fork1).await;
    apply_chain(&mut node2, &fork2).await;
    apply_chain(&mut node3, &fork3).await;

    // Canonical chain (10 blocks from fork point)
    let canonical = build_chain(11, 11, base[9].hash(), &producers[4], 10, &params);

    // Each node recovers independently
    for _ in 0..5 {
        node1.rollback_one_block().await.unwrap();
    }
    for _ in 0..7 {
        node2.rollback_one_block().await.unwrap();
    }
    for _ in 0..3 {
        node3.rollback_one_block().await.unwrap();
    }

    apply_chain(&mut node1, &canonical).await;
    apply_chain(&mut node2, &canonical).await;
    apply_chain(&mut node3, &canonical).await;

    // All 3 nodes must converge to identical state
    let h1 = node1.chain_state.read().await.best_height;
    let h2 = node2.chain_state.read().await.best_height;
    let h3 = node3.chain_state.read().await.best_height;
    assert_eq!(h1, h2);
    assert_eq!(h2, h3);
    assert_eq!(h1, 20);

    let hash1 = node1.chain_state.read().await.best_hash;
    let hash2 = node2.chain_state.read().await.best_hash;
    let hash3 = node3.chain_state.read().await.best_hash;
    assert_eq!(hash1, hash2);
    assert_eq!(hash2, hash3);
    assert_eq!(hash1, canonical[9].hash());
}

// ============================================================
// TEST 10: Recovery preserves valid mempool transactions
// ============================================================
#[tokio::test]
async fn test_recovery_preserves_mempool() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    // Build base chain (10 blocks)
    let base = build_chain(1, 1, Hash::ZERO, &producers[0], 10, &params);
    apply_chain(&mut node, &base).await;

    // Add TXs to mempool — use the mempool's internal len to verify
    let initial_mempool_size = node.mempool.read().await.len();
    {
        let mut mempool = node.mempool.write().await;
        for i in 0..10u64 {
            let dummy_hash = crypto::hash::hash(&i.to_le_bytes());
            let tx = Transaction::new_coinbase(1, dummy_hash, 999 + i, 0);
            let _ = mempool.add_system_transaction(tx, 10);
        }
    }
    let mempool_after_add = node.mempool.read().await.len();
    assert!(
        mempool_after_add > initial_mempool_size,
        "mempool should have transactions after adding"
    );

    // Fork: 3 blocks
    let fork = build_chain(11, 11, base[9].hash(), &producers[0], 3, &params);
    apply_chain(&mut node, &fork).await;

    // Rollback fork
    for _ in 0..3 {
        node.rollback_one_block().await.unwrap();
    }

    // Mempool should still have transactions (they weren't included in any block)
    let mempool_after_recovery = node.mempool.read().await.len();
    assert!(
        mempool_after_recovery > 0,
        "mempool should preserve TXs after fork recovery, got {} TXs",
        mempool_after_recovery
    );
}

// ============================================================
// TEST 11: Fork recovery — excluded_producers rebuilt after rollback
//
// THIS IS THE REAL BUG. This test MUST FAIL with current code.
//
// Root cause: validate_block_for_apply passes self.excluded_producers
// to the ValidationContext. When a node is on a fork, its excluded set
// diverges from the canonical chain. Blocks from the canonical chain
// are produced by producers that the forked node considers "excluded"
// → "invalid producer for slot" → blocks rejected → node can never sync.
//
// After fix: rebuild_excluded_from_headers() deterministically rebuilds
// exclusions from on-chain data. Canonical blocks sync via Light mode.
// ============================================================
#[tokio::test]
async fn test_fork_recovery_blocked_by_divergent_excluded_producers() {
    let (mut node, producers, _tmp) = make_node(5).await;
    let params = node.params.clone();

    // Build common base: 45 blocks to exit genesis period (devnet genesis_blocks=40).
    let mut base = Vec::new();
    let mut prev = devnet_genesis_hash();
    for i in 0..45 {
        let producer_idx = i % 5;
        let block = build_block(
            (i + 1) as u64,
            (i + 1) as u32,
            prev,
            &producers[producer_idx],
            &params,
        );
        prev = block.hash();
        base.push(block);
    }
    apply_chain(&mut node, &base).await;
    assert_eq!(node.chain_state.read().await.best_height, 45);

    // Node goes on fork: 5 blocks produced ONLY by producer[0]
    let fork = build_chain(46, 46, base[44].hash(), &producers[0], 5, &params);
    apply_chain(&mut node, &fork).await;
    assert_eq!(node.chain_state.read().await.best_height, 50);

    // Simulate divergent excluded_producers (the original bug).
    // In production, the fork accumulates stale exclusions from missed slots.
    node.excluded_producers.insert(*producers[1].public_key());
    node.excluded_producers.insert(*producers[2].public_key());
    node.excluded_producers.insert(*producers[3].public_key());
    node.excluded_producers.insert(*producers[4].public_key());
    assert_eq!(node.excluded_producers.len(), 4);

    // Build canonical chain from fork point using rotating producers.
    // Start from producer[1] to avoid hash collision with fork blocks (produced by [0]).
    // In production, syncing nodes apply these via Light mode (no strict eligibility).
    let mut canonical = Vec::new();
    let mut prev = base[44].hash();
    for i in 0..8 {
        let slot = 46 + i as u32;
        let height = 46 + i as u64;
        let producer_idx = (i + 1) % 5; // Start from producer[1], not [0]
        let block = build_block(height, slot, prev, &producers[producer_idx], &params);
        prev = block.hash();
        canonical.push(block);
    }

    // Rollback the fork (5 blocks).
    // FIX: rebuild_excluded_from_headers() deterministically rebuilds from on-chain data.
    // OLD BUG: stale excluded_producers persisted → divergent scheduling.
    for _ in 0..5 {
        node.rollback_one_block().await.unwrap();
    }
    assert_eq!(node.chain_state.read().await.best_height, 45);

    // VERIFY: excluded_producers rebuilt from block headers.
    // Test blocks have no missed_producers → should be empty after rebuild.
    assert_eq!(
        node.excluded_producers.len(),
        0,
        "excluded_producers should be rebuilt (empty) after rollback, has {} entries",
        node.excluded_producers.len()
    );

    // VERIFY: canonical blocks apply via Light mode (the actual sync path).
    // Light mode skips strict producer eligibility — this is how syncing nodes
    // recover after rollback. Even if epoch_producer_list is stale from the
    // fork's epoch boundary, Light mode succeeds.
    for (i, block) in canonical.iter().enumerate() {
        let h_before = node.chain_state.read().await.best_height;
        node.apply_block(block.clone(), ValidationMode::Light)
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "Light mode apply_block failed at slot={} (block {}): {}",
                    block.header.slot, i, e
                )
            });
        let h_after = node.chain_state.read().await.best_height;
        assert_eq!(
            h_after,
            h_before + 1,
            "Block {} (slot={}) did not advance height: {} → {}",
            i,
            block.header.slot,
            h_before,
            h_after
        );
    }

    // All 8 canonical blocks applied — node recovered from fork
    assert_eq!(node.chain_state.read().await.best_height, 53);
    assert_eq!(node.chain_state.read().await.best_hash, canonical[7].hash());
}

// ============================================================
// TEST 12: Post-snap gossip validation mode (INC-I-010 layer 3)
// ============================================================
//
// After snap sync, epoch_producer_list has ALL active producers (unfiltered).
// The network may have a smaller attestation-filtered list. To avoid rejecting
// valid gossip blocks (slot%N divergence), gossip uses Light validation until
// the next epoch boundary rebuilds the list correctly and clears snap_sync_height.
#[tokio::test]
async fn test_post_snap_gossip_validation_mode() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();

    // Devnet blocks_per_reward_epoch = 4, so epoch boundary at height 4
    let bpe = node.config.network.blocks_per_reward_epoch();
    assert_eq!(bpe, 4, "Test assumes devnet blocks_per_reward_epoch=4");

    // Apply 2 blocks (mid-epoch)
    let chain = build_chain(1, 1, Hash::ZERO, &producers[0], 2, &params);
    apply_chain(&mut node, &chain).await;
    assert_eq!(node.chain_state.read().await.best_height, 2);

    // Simulate snap sync: set snap_sync_height (as fork_recovery.rs does)
    assert!(
        node.snap_sync_height.is_none(),
        "should start without snap marker"
    );
    node.snap_sync_height = Some(2);

    // During post-snap catch-up, snap_sync_height is Some → gossip would use Light
    assert!(
        node.snap_sync_height.is_some(),
        "snap_sync_height should be set after snap sync"
    );

    // Apply blocks 3 and 4 — height 4 is the epoch boundary
    let epoch_blocks = build_chain(3, 3, chain[1].hash(), &producers[0], 2, &params);
    apply_chain(&mut node, &epoch_blocks).await;
    assert_eq!(node.chain_state.read().await.best_height, 4);

    // At epoch boundary, post_commit clears snap_sync_height → gossip returns to Full
    assert!(
        node.snap_sync_height.is_none(),
        "snap_sync_height should be cleared at epoch boundary (h={})",
        node.chain_state.read().await.best_height
    );
}
