//! EpochState Structural Regression Tests
//!
//! Verifies that the EpochState refactor (INC-I-035) produces identical behavior
//! to the old scattered-fields implementation. Uses real Node + real RocksDB.
//!
//! Key invariants tested:
//! - derive_at_boundary at epoch boundary produces correct producer_list/active_list
//! - accumulate_block per-block tracking matches expected attestation state
//! - UndoData round-trip: rollback restores exact epoch_state
//! - Multi-epoch accumulator rotation is correct

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, EpochState, Transaction};
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
        data_root: crypto::Hash::ZERO,
        fork_id: crypto::Hash::ZERO,
    };

    Block::new(header, vec![coinbase])
}

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

async fn apply_chain(node: &mut Node, blocks: &[Block]) {
    for block in blocks {
        node.apply_block(block.clone(), ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("apply_block failed at h={}: {}", block.header.slot, e));
    }
}

// OUTPUT CONTRACT: fn Node::apply_block(block) via post_commit → accumulate_block
//   O1: self.epoch_state.attested_sets[0] — HashSet, contains block.header.producer
//   O2: self.epoch_state.blocks_produced — HashMap, producer entry = block count
//   O3: self.epoch_state.attestation_accum[0] — HashMap, producer has non-empty minute set
// PATHS: P1: 3 blocks applied by same producer
// MATRIX: 3 outputs × 1 path = 3 cells
//   P1: O1✓ O2✓ O3✓
#[tokio::test]
async fn test_accumulate_block_tracks_producer() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();
    let genesis_hash = node.best_hash().await;

    // Apply 3 blocks (all by producer[0]) — epoch boundary at h=4
    let chain = build_chain(1, 1, genesis_hash, &producers[0], 3, &params);
    apply_chain(&mut node, &chain).await;

    let pk0 = *producers[0].public_key();

    // O1: attested_sets[0] contains the producer
    assert!(
        node.epoch_state.attested_sets[0].contains(&pk0),
        "Producer should be in attested_sets[0] after producing blocks"
    );

    // O2: blocks_produced = 3
    assert_eq!(
        node.epoch_state.blocks_produced.get(&pk0).copied().unwrap_or(0),
        3,
        "blocks_produced should be 3 after 3 blocks"
    );

    // O3: attestation_accum[0] has entries for the producer
    assert!(
        node.epoch_state.attestation_accum[0].contains_key(&pk0),
        "attestation_accum[0] should track the producer's minutes"
    );
    assert!(
        !node.epoch_state.attestation_accum[0][&pk0].is_empty(),
        "attestation minute set should not be empty"
    );
}

// OUTPUT CONTRACT: fn Node::post_commit_actions(block) at epoch boundary
//   O1: self.epoch_state.epoch — u64, advances to new epoch number
//   O2: self.epoch_state.producer_list — Vec<PK>, sorted by pubkey bytes
//   O3: self.epoch_state.active_list — Vec<PK>, equals producer_list (<50 producers)
//   O4: self.epoch_state.bond_snapshot — HashMap, non-empty (entries for active producers)
//   O5: self.epoch_state.blocks_produced — HashMap, empty (new epoch, rotated)
//   O6: self.epoch_state.attested_sets[0] — HashSet, empty (new epoch, rotated)
//   O7: self.epoch_state.attested_sets[1] — HashSet, contains prev epoch producer
//   O8: self.epoch_state.attestation_accum[0] — HashMap, empty (new epoch, rotated)
// PATHS: P1: first epoch boundary (h=4, devnet blocks_per_epoch=4)
// MATRIX: 8 outputs × 1 path = 8 cells
//   P1: O1✓ O2✓ O3✓ O4✓ O5✓ O6✓ O7✓ O8✓
#[tokio::test]
async fn test_epoch_boundary_derives_correct_state() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();
    let genesis_hash = node.best_hash().await;

    let pk0 = *producers[0].public_key();

    // Build 4 blocks to reach epoch boundary (devnet: blocks_per_reward_epoch=4)
    let chain = build_chain(1, 1, genesis_hash, &producers[0], 4, &params);

    // Record pre-boundary state
    let pre_epoch = node.epoch_state.epoch;

    apply_chain(&mut node, &chain).await;

    // O1: epoch advanced
    assert!(
        node.epoch_state.epoch > pre_epoch || node.epoch_state.epoch == 1,
        "epoch should advance at boundary: was {}, now {}",
        pre_epoch, node.epoch_state.epoch
    );

    // O2: producer_list is sorted by pubkey bytes
    let pl = &node.epoch_state.producer_list;
    for i in 1..pl.len() {
        assert!(
            pl[i - 1].as_bytes() <= pl[i].as_bytes(),
            "producer_list should be sorted by pubkey bytes"
        );
    }

    // O3: active_list = producer_list (3 producers < ACTIVE_PRODUCERS_CAP=50)
    assert_eq!(
        node.epoch_state.active_list, node.epoch_state.producer_list,
        "active_list should equal producer_list with <50 producers"
    );

    // O4: bond_snapshot has entries
    assert!(
        !node.epoch_state.bond_snapshot.is_empty(),
        "bond_snapshot should have entries after epoch boundary"
    );

    // O5: blocks_produced is empty (new epoch)
    assert!(
        node.epoch_state.blocks_produced.is_empty(),
        "blocks_produced should be empty after epoch rotation"
    );

    // O6: attested_sets[0] is empty (new epoch, rotated)
    assert!(
        node.epoch_state.attested_sets[0].is_empty(),
        "attested_sets[0] should be empty after rotation"
    );

    // O7: attested_sets[1] contains the producer from prev epoch
    assert!(
        node.epoch_state.attested_sets[1].contains(&pk0),
        "attested_sets[1] should contain prev epoch's attested producer"
    );

    // O8: attestation_accum[0] is empty (new epoch)
    assert!(
        node.epoch_state.attestation_accum[0].is_empty(),
        "attestation_accum[0] should be empty after rotation"
    );
}

// OUTPUT CONTRACT: fn Node::rollback_one_block() with epoch_state_snapshot in UndoData
//   O1: self.epoch_state.hash() — Hash, equals hash captured before the rolled-back block
//   O2: self.epoch_state.blocks_produced — HashMap, reverts to pre-block-2 count
//   O3: self.epoch_state.attested_sets[0] — HashSet, reverts to pre-block-2 state
// PATHS: P1: rollback single block mid-epoch (undo data has epoch_state_snapshot)
// MATRIX: 3 outputs × 1 path = 3 cells
//   P1: O1✓ O2✓ O3✓
#[tokio::test]
async fn test_undo_data_roundtrip_preserves_epoch_state() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();
    let genesis_hash = node.best_hash().await;

    // Apply 2 blocks
    let chain = build_chain(1, 1, genesis_hash, &producers[0], 2, &params);
    apply_chain(&mut node, &chain[..1]).await;

    // Snapshot epoch state BEFORE block 2
    let state_before_block2 = node.epoch_state.clone();
    let hash_before_block2 = state_before_block2.hash();

    // Apply block 2
    apply_chain(&mut node, &chain[1..2]).await;

    // Verify block 2 changed the state
    let pk0 = *producers[0].public_key();
    assert_eq!(
        node.epoch_state.blocks_produced.get(&pk0).copied().unwrap_or(0),
        2,
        "Should have 2 blocks produced after 2 blocks"
    );

    // Rollback block 2
    let rolled_back = node.rollback_one_block().await.expect("rollback failed");
    assert!(rolled_back, "rollback should succeed");

    // O1+O4: epoch_state hash matches pre-block-2 state
    // Note: the undo snapshot was taken BEFORE block 2, so after rollback
    // the state should match what we saw before block 2.
    assert_eq!(
        node.epoch_state.hash(),
        hash_before_block2,
        "epoch_state hash should match pre-rollback state"
    );

    // O2: blocks_produced reverted
    assert_eq!(
        node.epoch_state.blocks_produced.get(&pk0).copied().unwrap_or(0),
        state_before_block2.blocks_produced.get(&pk0).copied().unwrap_or(0),
        "blocks_produced should revert to pre-block-2 value"
    );

    // O3: attested_sets reverted
    assert_eq!(
        node.epoch_state.attested_sets[0],
        state_before_block2.attested_sets[0],
        "attested_sets should revert to pre-block-2 state"
    );
}

// OUTPUT CONTRACT: fn Node::post_commit_actions — two consecutive epoch boundaries
//   O1: self.epoch_state.attested_sets[1] — HashSet, epoch 0 producer after 1st boundary
//   O2: self.epoch_state.attested_sets[2] — HashSet, empty after 1st boundary; epoch 0 producer after 2nd
//   O3: self.epoch_state.attested_sets[1] — HashSet, epoch 1 producer after 2nd boundary
//   O4: self.epoch_state.attested_sets[0] — HashSet, empty after 2nd boundary (new epoch)
// PATHS: P1: 4 blocks epoch 0 (pk0) + 4 blocks epoch 1 (pk1) = two boundaries
// MATRIX: 4 outputs × 1 path = 4 cells
//   P1: O1✓ O2✓ O3✓ O4✓
#[tokio::test]
async fn test_multi_epoch_accumulator_rotation() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();
    let genesis_hash = node.best_hash().await;

    let pk0 = *producers[0].public_key();
    let pk1 = *producers[1].public_key();

    // Epoch 0: 4 blocks by producer[0] → hits epoch boundary
    let chain0 = build_chain(1, 1, genesis_hash, &producers[0], 4, &params);
    let tip0 = chain0.last().unwrap().hash();
    apply_chain(&mut node, &chain0).await;

    // O1: After epoch 0→1 boundary
    assert!(
        node.epoch_state.attested_sets[1].contains(&pk0),
        "After epoch 1 boundary: [1] should contain epoch 0 producer"
    );
    assert!(
        node.epoch_state.attested_sets[2].is_empty(),
        "After epoch 1 boundary: [2] should be empty (only 1 epoch of history)"
    );

    // Epoch 1: 4 blocks by producer[1] → hits epoch boundary
    let chain1 = build_chain(5, 5, tip0, &producers[1], 4, &params);
    apply_chain(&mut node, &chain1).await;

    // O2: After epoch 1→2 boundary: [2] = epoch 0 data
    assert!(
        node.epoch_state.attested_sets[2].contains(&pk0),
        "After epoch 2 boundary: [2] should contain epoch 0 producer (shifted from [1])"
    );

    // O3: After epoch 1→2 boundary: [1] = epoch 1 data
    assert!(
        node.epoch_state.attested_sets[1].contains(&pk1),
        "After epoch 2 boundary: [1] should contain epoch 1 producer"
    );

    // O4: After epoch 1→2 boundary: [0] = empty
    assert!(
        node.epoch_state.attested_sets[0].is_empty(),
        "After epoch 2 boundary: [0] should be empty (new epoch)"
    );
}

// OUTPUT CONTRACT: fn EpochState::hash() — cross-node determinism
//   O1: node1.epoch_state.hash() — Hash, equals node2.epoch_state.hash()
//   O2: node1.epoch_state.producer_list — Vec<PK>, equals node2's
//   O3: node1.epoch_state.active_list — Vec<PK>, equals node2's
// PATHS: P1: two independent nodes, same producers, same 4-block chain
// MATRIX: 3 outputs × 1 path = 3 cells
//   P1: O1✓ O2✓ O3✓
#[tokio::test]
async fn test_epoch_state_hash_deterministic_across_nodes() {
    let producers: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();

    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();

    let mut node1 = Node::new_for_test(tmp1.path().to_path_buf(), producers.clone())
        .await
        .unwrap();
    let mut node2 = Node::new_for_test(tmp2.path().to_path_buf(), producers.clone())
        .await
        .unwrap();

    let params = node1.params.clone();
    let genesis_hash1 = node1.best_hash().await;
    let genesis_hash2 = node2.best_hash().await;
    assert_eq!(genesis_hash1, genesis_hash2, "Genesis hashes should match");

    // Apply identical chains to both nodes
    let chain = build_chain(1, 1, genesis_hash1, &producers[0], 4, &params);
    apply_chain(&mut node1, &chain).await;
    apply_chain(&mut node2, &chain).await;

    // O1: epoch_state hashes should be identical
    assert_eq!(
        node1.epoch_state.hash(),
        node2.epoch_state.hash(),
        "Two nodes with identical blocks should have identical epoch_state hash"
    );
    assert_eq!(
        node1.epoch_state.producer_list,
        node2.epoch_state.producer_list,
        "producer_list should match"
    );
    assert_eq!(
        node1.epoch_state.active_list,
        node2.epoch_state.active_list,
        "active_list should match"
    );
}

// OUTPUT CONTRACT: fn Node::rollback_one_block() — rollback the epoch boundary block
//   O1: self.epoch_state.epoch — u64, reverts to pre-boundary epoch
//   O2: self.epoch_state.hash() — Hash, matches pre-boundary snapshot
//   O3: self.epoch_state.blocks_produced — HashMap, reverts to pre-boundary count
// PATHS: P1: apply 3 blocks + boundary block (h=4), then rollback h=4
// MATRIX: 3 outputs × 1 path = 3 cells
//   P1: O1✓ O2✓ O3✓
#[tokio::test]
async fn test_rollback_across_epoch_boundary() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();
    let genesis_hash = node.best_hash().await;
    let pk0 = *producers[0].public_key();

    // Apply 3 blocks (pre-boundary)
    let chain = build_chain(1, 1, genesis_hash, &producers[0], 3, &params);
    apply_chain(&mut node, &chain).await;

    // Snapshot state before epoch boundary block
    let pre_boundary_hash = node.epoch_state.hash();
    let pre_boundary_epoch = node.epoch_state.epoch;
    let pre_boundary_blocks_produced = node.epoch_state.blocks_produced.get(&pk0).copied().unwrap_or(0);

    // Apply block 4 (epoch boundary)
    let boundary_block = build_block(4, 4, chain.last().unwrap().hash(), &producers[0], &params);
    node.apply_block(boundary_block.clone(), ValidationMode::Light)
        .await
        .unwrap();

    // Verify epoch boundary was crossed
    assert!(
        node.epoch_state.blocks_produced.is_empty(),
        "After epoch boundary, blocks_produced should be empty (rotated)"
    );

    // Rollback the boundary block
    let rolled_back = node.rollback_one_block().await.expect("rollback failed");
    assert!(rolled_back);

    // O1: epoch reverts
    assert_eq!(
        node.epoch_state.epoch, pre_boundary_epoch,
        "epoch should revert to pre-boundary value"
    );

    // O2: attested_sets un-rotated
    assert_eq!(
        node.epoch_state.hash(), pre_boundary_hash,
        "epoch_state hash should match pre-boundary state"
    );

    // O3: blocks_produced reverts
    assert_eq!(
        node.epoch_state.blocks_produced.get(&pk0).copied().unwrap_or(0),
        pre_boundary_blocks_produced,
        "blocks_produced should revert to pre-boundary count"
    );
}

// OUTPUT CONTRACT: fn StateDb::put_epoch_state() + get_epoch_state() via post_commit
//   O1: restored.hash() — Hash, equals in-memory epoch_state hash
//   O2: restored.epoch — u64, equals in-memory epoch
//   O3: restored.producer_list — Vec<PK>, same len as in-memory
//   O4: restored.active_list — Vec<PK>, equals in-memory
//   O5: restored.bond_snapshot — HashMap, equals in-memory
//   O6: restored.attested_sets — [HashSet;3], equals in-memory
// PATHS: P1: after epoch boundary (post_commit persists epoch_state)
// MATRIX: 6 outputs × 1 path = 6 cells
//   P1: O1✓ O2✓ O3✓ O4✓ O5✓ O6✓
#[tokio::test]
async fn test_epoch_state_persistence_roundtrip() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();
    let genesis_hash = node.best_hash().await;

    // Apply 4 blocks (epoch boundary)
    let chain = build_chain(1, 1, genesis_hash, &producers[0], 4, &params);
    apply_chain(&mut node, &chain).await;

    // The epoch boundary should have persisted the epoch state
    let original_hash = node.epoch_state.hash();
    let original_epoch = node.epoch_state.epoch;
    let original_producers_len = node.epoch_state.producer_list.len();

    // Read persisted epoch state from RocksDB
    let persisted_bytes = node
        .state_db
        .get_epoch_state()
        .expect("epoch_state should be persisted after epoch boundary");

    let restored =
        EpochState::deserialize_canonical(&persisted_bytes).expect("deserialization should work");

    // O1: hash matches
    assert_eq!(
        restored.hash(),
        original_hash,
        "Persisted epoch_state hash should match in-memory"
    );

    // O2: all fields match
    assert_eq!(restored.epoch, original_epoch);
    assert_eq!(restored.producer_list.len(), original_producers_len);
    assert_eq!(restored.active_list, node.epoch_state.active_list);
    assert_eq!(restored.bond_snapshot, node.epoch_state.bond_snapshot);
    assert_eq!(restored.attested_sets, node.epoch_state.attested_sets);
}
