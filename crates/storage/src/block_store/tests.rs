//! Block store tests

use super::*;
use crypto::hash::hash_with_domain;
use crypto::{Hash, KeyPair, PublicKey, ADDRESS_DOMAIN};
use doli_core::{Block, BlockHeader, Transaction};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

/// Create a test BlockStore in a temporary directory
fn create_test_store() -> (BlockStore, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let store = BlockStore::open(temp_dir.path()).unwrap();
    (store, temp_dir)
}

/// Create a test block header with specified slot and producer
fn create_test_header(slot: u32, producer: &PublicKey) -> BlockHeader {
    BlockHeader {
        version: 1,
        prev_hash: Hash::ZERO,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 1000 + slot as u64 * 10,
        slot,
        producer: *producer,
        vdf_output: VdfOutput { value: Vec::new() },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    }
}

/// Create a simple test block with no transactions
fn create_test_block(slot: u32, producer: &PublicKey) -> Block {
    Block::new(create_test_header(slot, producer), vec![])
}

/// Create a block with an EpochReward transaction
fn create_epoch_reward_block(slot: u32, producer: &PublicKey, epoch: u64) -> Block {
    let epoch_reward_tx = Transaction::new_epoch_reward(
        epoch,
        *producer,
        100_000_000, // 1 DOLI
        hash_with_domain(ADDRESS_DOMAIN, producer.as_bytes()),
    );
    Block::new(create_test_header(slot, producer), vec![epoch_reward_tx])
}

#[test]
fn test_get_block_by_slot_empty() {
    let (store, _dir) = create_test_store();

    // Query non-existent slot
    let result = store.get_block_by_slot(100).unwrap();
    assert!(result.is_none(), "Empty slot should return None");
}

#[test]
fn test_get_block_by_slot_found() {
    let (store, _dir) = create_test_store();
    let keypair = KeyPair::generate();
    let producer = *keypair.public_key();

    // Store a block at slot 42
    let block = create_test_block(42, &producer);
    store.put_block_canonical(&block, 1).unwrap();

    // Retrieve by slot
    let retrieved = store.get_block_by_slot(42).unwrap();
    assert!(retrieved.is_some(), "Block should be found at slot 42");
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.slot(), 42);
    assert_eq!(retrieved.header.producer, producer);
}

#[test]
fn test_get_blocks_in_slot_range_empty() {
    let (store, _dir) = create_test_store();

    // Query empty range
    let blocks = store.get_blocks_in_slot_range(0, 100).unwrap();
    assert!(blocks.is_empty(), "Empty range should return empty vec");
}

#[test]
fn test_get_blocks_in_slot_range_with_gaps() {
    let (store, _dir) = create_test_store();
    let keypair = KeyPair::generate();
    let producer = *keypair.public_key();

    // Store blocks at slots 10, 12, 15 (with gaps at 11, 13, 14)
    let slots = [10u32, 12, 15];
    for (height, &slot) in slots.iter().enumerate() {
        let block = create_test_block(slot, &producer);
        store.put_block_canonical(&block, height as u64).unwrap();
    }

    // Query range 10..20
    let blocks = store.get_blocks_in_slot_range(10, 20).unwrap();
    assert_eq!(blocks.len(), 3, "Should return 3 blocks");
    assert_eq!(blocks[0].slot(), 10);
    assert_eq!(blocks[1].slot(), 12);
    assert_eq!(blocks[2].slot(), 15);
}

#[test]
fn test_get_blocks_in_slot_range_ordering() {
    let (store, _dir) = create_test_store();
    let keypair = KeyPair::generate();
    let producer = *keypair.public_key();

    // Store blocks in non-sequential order
    for (height, slot) in [(0, 5u32), (1, 3), (2, 7), (3, 1)] {
        let block = create_test_block(slot, &producer);
        store.put_block_canonical(&block, height).unwrap();
    }

    // Blocks should be returned in slot order
    let blocks = store.get_blocks_in_slot_range(0, 10).unwrap();
    assert_eq!(blocks.len(), 4);
    assert_eq!(blocks[0].slot(), 1);
    assert_eq!(blocks[1].slot(), 3);
    assert_eq!(blocks[2].slot(), 5);
    assert_eq!(blocks[3].slot(), 7);
}

#[test]
fn test_get_last_rewarded_epoch_no_rewards() {
    let (store, _dir) = create_test_store();
    let keypair = KeyPair::generate();
    let producer = *keypair.public_key();

    // Store blocks without epoch rewards
    for height in 0..5 {
        let block = create_test_block(height as u32, &producer);
        store.put_block_canonical(&block, height).unwrap();
    }

    // Should return 0 when no rewards exist
    let last_epoch = store.get_last_rewarded_epoch().unwrap();
    assert_eq!(last_epoch, 0, "No rewards should return epoch 0");
}

#[test]
fn test_get_last_rewarded_epoch_single_reward() {
    let (store, _dir) = create_test_store();
    let keypair = KeyPair::generate();
    let producer = *keypair.public_key();

    // Store a block with epoch reward for epoch 5
    let block = create_epoch_reward_block(360, &producer, 5);
    store.put_block_canonical(&block, 0).unwrap();

    let last_epoch = store.get_last_rewarded_epoch().unwrap();
    assert_eq!(last_epoch, 5, "Should return epoch 5");
}

#[test]
fn test_get_last_rewarded_epoch_multiple_rewards() {
    let (store, _dir) = create_test_store();
    let keypair = KeyPair::generate();
    let producer = *keypair.public_key();

    // Store blocks with epoch rewards for epochs 1, 2, 3
    let reward_blocks = [
        (360, 1u64), // epoch 1 at slot 360
        (720, 2),    // epoch 2 at slot 720
        (1080, 3),   // epoch 3 at slot 1080
    ];

    for (height, (slot, epoch)) in reward_blocks.iter().enumerate() {
        let block = create_epoch_reward_block(*slot, &producer, *epoch);
        store.put_block_canonical(&block, height as u64).unwrap();
    }

    // Should return the most recent (highest) epoch
    let last_epoch = store.get_last_rewarded_epoch().unwrap();
    assert_eq!(last_epoch, 3, "Should return most recent epoch 3");
}

#[test]
fn test_get_last_rewarded_epoch_mixed_blocks() {
    let (store, _dir) = create_test_store();
    let keypair = KeyPair::generate();
    let producer = *keypair.public_key();

    // Store a mix of regular blocks and epoch reward blocks
    // height 0: regular block
    store
        .put_block_canonical(&create_test_block(1, &producer), 0)
        .unwrap();

    // height 1: epoch reward for epoch 1
    store
        .put_block_canonical(&create_epoch_reward_block(360, &producer, 1), 1)
        .unwrap();

    // height 2: regular block
    store
        .put_block_canonical(&create_test_block(361, &producer), 2)
        .unwrap();

    // height 3: epoch reward for epoch 2
    store
        .put_block_canonical(&create_epoch_reward_block(720, &producer, 2), 3)
        .unwrap();

    // height 4: regular block (most recent)
    store
        .put_block_canonical(&create_test_block(721, &producer), 4)
        .unwrap();

    // Should still find epoch 2 as the last rewarded
    let last_epoch = store.get_last_rewarded_epoch().unwrap();
    assert_eq!(
        last_epoch, 2,
        "Should find epoch 2 through non-reward blocks"
    );
}

#[test]
fn test_get_hash_by_slot() {
    let (store, _dir) = create_test_store();
    let keypair = KeyPair::generate();
    let producer = *keypair.public_key();

    // Store a block
    let block = create_test_block(42, &producer);
    let expected_hash = block.hash();
    store.put_block_canonical(&block, 0).unwrap();

    // Retrieve hash by slot
    let hash = store.get_hash_by_slot(42).unwrap();
    assert!(hash.is_some());
    assert_eq!(hash.unwrap(), expected_hash);

    // Non-existent slot
    let hash = store.get_hash_by_slot(999).unwrap();
    assert!(hash.is_none());
}

#[test]
fn test_has_any_block_in_slot_range() {
    let (store, _dir) = create_test_store();
    let keypair = KeyPair::generate();
    let producer = *keypair.public_key();

    // Store blocks at slots 100, 150, 200
    for (height, slot) in [(0, 100u32), (1, 150), (2, 200)] {
        let block = create_test_block(slot, &producer);
        store.put_block_canonical(&block, height).unwrap();
    }

    // Range with blocks
    assert!(store.has_any_block_in_slot_range(100, 200).unwrap());
    assert!(store.has_any_block_in_slot_range(99, 101).unwrap());
    assert!(store.has_any_block_in_slot_range(150, 151).unwrap());

    // Range without blocks (empty epochs)
    assert!(!store.has_any_block_in_slot_range(0, 100).unwrap());
    assert!(!store.has_any_block_in_slot_range(101, 150).unwrap());
    assert!(!store.has_any_block_in_slot_range(201, 300).unwrap());

    // Edge cases
    assert!(!store.has_any_block_in_slot_range(100, 100).unwrap()); // Empty range
    assert!(!store.has_any_block_in_slot_range(200, 100).unwrap()); // Invalid range
}

// =========================================================================
// Milestone 6: Additional Edge Case Tests
// =========================================================================

#[test]
fn test_get_block_by_slot_empty_in_chain_with_gaps() {
    // Query an empty slot in a chain that has blocks with gaps
    let (store, _dir) = create_test_store();
    let keypair = KeyPair::generate();
    let producer = *keypair.public_key();

    // Store blocks at slots 10, 12, 15 (gaps at 11, 13, 14)
    let slots = [10u32, 12, 15];
    for (height, &slot) in slots.iter().enumerate() {
        let block = create_test_block(slot, &producer);
        store.put_block_canonical(&block, height as u64).unwrap();
    }

    // Query empty slot 11 (between 10 and 12)
    let result = store.get_block_by_slot(11).unwrap();
    assert!(result.is_none(), "Empty slot 11 should return None");

    // Query empty slot 13 (between 12 and 15)
    let result = store.get_block_by_slot(13).unwrap();
    assert!(result.is_none(), "Empty slot 13 should return None");

    // Query empty slot 14 (between 12 and 15)
    let result = store.get_block_by_slot(14).unwrap();
    assert!(result.is_none(), "Empty slot 14 should return None");

    // Verify existing slots still work
    assert!(store.get_block_by_slot(10).unwrap().is_some());
    assert!(store.get_block_by_slot(12).unwrap().is_some());
    assert!(store.get_block_by_slot(15).unwrap().is_some());
}

#[test]
fn test_get_blocks_in_slot_range_boundary_conditions() {
    let (store, _dir) = create_test_store();
    let keypair = KeyPair::generate();
    let producer = *keypair.public_key();

    // Store blocks at slots 5, 10, 15
    for (height, slot) in [(0, 5u32), (1, 10), (2, 15)] {
        let block = create_test_block(slot, &producer);
        store.put_block_canonical(&block, height).unwrap();
    }

    // Test: start == end (empty range)
    let blocks = store.get_blocks_in_slot_range(10, 10).unwrap();
    assert!(blocks.is_empty(), "start == end should return empty vec");

    // Test: start > end (invalid range)
    let blocks = store.get_blocks_in_slot_range(15, 5).unwrap();
    assert!(blocks.is_empty(), "start > end should return empty vec");

    // Test: range before any blocks
    let blocks = store.get_blocks_in_slot_range(0, 4).unwrap();
    assert!(blocks.is_empty(), "Range before blocks should be empty");

    // Test: range after all blocks
    let blocks = store.get_blocks_in_slot_range(20, 30).unwrap();
    assert!(blocks.is_empty(), "Range after blocks should be empty");

    // Test: range exactly matches one block
    let blocks = store.get_blocks_in_slot_range(10, 11).unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].slot(), 10);

    // Test: range includes all blocks
    let blocks = store.get_blocks_in_slot_range(0, 20).unwrap();
    assert_eq!(blocks.len(), 3);
}

#[test]
fn test_get_last_rewarded_epoch_with_epoch_gaps() {
    // Test that get_last_rewarded_epoch handles non-sequential epoch numbers
    let (store, _dir) = create_test_store();
    let keypair = KeyPair::generate();
    let producer = *keypair.public_key();

    // Store epoch rewards for epochs 1, 3, 5 (skipping 2 and 4)
    // This simulates a scenario where some epochs had no blocks
    let epochs = [(360, 1u64), (1080, 3), (1800, 5)];
    for (height, (slot, epoch)) in epochs.iter().enumerate() {
        let block = create_epoch_reward_block(*slot, &producer, *epoch);
        store.put_block_canonical(&block, height as u64).unwrap();
    }

    // Should return the highest epoch (5), not just the most recent by height
    let last_epoch = store.get_last_rewarded_epoch().unwrap();
    assert_eq!(last_epoch, 5, "Should return highest epoch 5");
}

#[test]
fn test_get_blocks_in_slot_range_multiple_epochs() {
    // Test querying blocks across multiple epochs
    let (store, _dir) = create_test_store();
    let keypair1 = KeyPair::generate();
    let keypair2 = KeyPair::generate();
    let producer1 = *keypair1.public_key();
    let producer2 = *keypair2.public_key();

    // Store blocks across 3 epochs (using devnet 30 slots/epoch)
    // Epoch 0: slots 1-29
    // Epoch 1: slots 30-59
    // Epoch 2: slots 60-89

    // Add some blocks in each epoch
    let mut height = 0u64;

    // Epoch 0: producer1 at slots 5, 10, 15
    for slot in [5u32, 10, 15] {
        store
            .put_block_canonical(&create_test_block(slot, &producer1), height)
            .unwrap();
        height += 1;
    }

    // Epoch 1: producer2 at slots 35, 40, 45
    for slot in [35u32, 40, 45] {
        store
            .put_block_canonical(&create_test_block(slot, &producer2), height)
            .unwrap();
        height += 1;
    }

    // Epoch 2: both producers at slots 65, 70, 75, 80
    for (i, slot) in [65u32, 70, 75, 80].iter().enumerate() {
        let producer = if i % 2 == 0 { &producer1 } else { &producer2 };
        store
            .put_block_canonical(&create_test_block(*slot, producer), height)
            .unwrap();
        height += 1;
    }

    // Query epoch 0 range (1-30)
    let epoch0_blocks = store.get_blocks_in_slot_range(1, 30).unwrap();
    assert_eq!(epoch0_blocks.len(), 3, "Epoch 0 should have 3 blocks");
    for block in &epoch0_blocks {
        assert!(block.slot() < 30, "All blocks should be in epoch 0");
        assert_eq!(block.header.producer, producer1);
    }

    // Query epoch 1 range (30-60)
    let epoch1_blocks = store.get_blocks_in_slot_range(30, 60).unwrap();
    assert_eq!(epoch1_blocks.len(), 3, "Epoch 1 should have 3 blocks");
    for block in &epoch1_blocks {
        assert!(block.slot() >= 30 && block.slot() < 60);
        assert_eq!(block.header.producer, producer2);
    }

    // Query epoch 2 range (60-90)
    let epoch2_blocks = store.get_blocks_in_slot_range(60, 90).unwrap();
    assert_eq!(epoch2_blocks.len(), 4, "Epoch 2 should have 4 blocks");

    // Query cross-epoch range (20-70)
    // Should include: epoch 1 blocks (35, 40, 45) + epoch 2 block at 65
    // 70 is excluded (exclusive end), so 4 blocks total
    let cross_epoch = store.get_blocks_in_slot_range(20, 70).unwrap();
    assert_eq!(
        cross_epoch.len(),
        4,
        "Cross-epoch query should return 4 blocks"
    );
    // Verify sorted by slot
    for i in 1..cross_epoch.len() {
        assert!(
            cross_epoch[i - 1].slot() < cross_epoch[i].slot(),
            "Blocks should be sorted by slot"
        );
    }
}

#[test]
fn test_get_block_by_slot_out_of_range() {
    // Test querying slots before first block and after last block
    let (store, _dir) = create_test_store();
    let keypair = KeyPair::generate();
    let producer = *keypair.public_key();

    // Store a single block at slot 100
    let block = create_test_block(100, &producer);
    store.put_block_canonical(&block, 0).unwrap();

    // Query before first block
    assert!(store.get_block_by_slot(0).unwrap().is_none());
    assert!(store.get_block_by_slot(50).unwrap().is_none());
    assert!(store.get_block_by_slot(99).unwrap().is_none());

    // Query exact slot
    assert!(store.get_block_by_slot(100).unwrap().is_some());

    // Query after last block
    assert!(store.get_block_by_slot(101).unwrap().is_none());
    assert!(store.get_block_by_slot(1000).unwrap().is_none());
    assert!(store.get_block_by_slot(u32::MAX).unwrap().is_none());
}

#[test]
fn test_get_last_rewarded_epoch_empty_chain() {
    // Test that empty chain returns 0
    let (store, _dir) = create_test_store();

    let last_epoch = store.get_last_rewarded_epoch().unwrap();
    assert_eq!(last_epoch, 0, "Empty chain should return epoch 0");
}

#[test]
fn test_clear_removes_all_data() {
    // Regression test: force_resync_from_genesis must clear the block store
    // to purge stale fork blocks. Without clear(), old HEIGHT_INDEX and
    // SLOT_INDEX entries persist and pollute queries.
    let (store, _dir) = create_test_store();
    let keypair = KeyPair::generate();
    let producer = *keypair.public_key();

    // Populate store with blocks (simulating a fork)
    for height in 0..10u64 {
        let block = create_test_block(height as u32 + 1, &producer);
        store.put_block_canonical(&block, height).unwrap();
    }

    // Verify data exists
    assert!(store.get_block_by_height(0).unwrap().is_some());
    assert!(store.get_block_by_height(9).unwrap().is_some());
    assert!(store.get_block_by_slot(1).unwrap().is_some());
    assert!(store.get_block_by_slot(10).unwrap().is_some());
    assert_eq!(store.count_slot_index_entries().unwrap(), 10);

    // Clear the store
    store.clear().unwrap();

    // Verify all data is gone
    for h in 0..10u64 {
        assert!(
            store.get_block_by_height(h).unwrap().is_none(),
            "Height {} should be empty after clear",
            h
        );
    }
    for s in 1..=10u32 {
        assert!(
            store.get_block_by_slot(s).unwrap().is_none(),
            "Slot {} should be empty after clear",
            s
        );
    }
    assert_eq!(store.count_slot_index_entries().unwrap(), 0);
    assert_eq!(store.get_last_rewarded_epoch().unwrap(), 0);
}

#[test]
fn test_clear_then_repopulate() {
    // After clear(), new blocks should be stored correctly without
    // interference from previously cleared data.
    let (store, _dir) = create_test_store();
    let keypair = KeyPair::generate();
    let producer = *keypair.public_key();

    // Store "fork" blocks at heights 0-4, slots 100-104
    for i in 0..5u64 {
        let block = create_test_block(100 + i as u32, &producer);
        store.put_block_canonical(&block, i).unwrap();
    }

    // Clear (simulating resync)
    store.clear().unwrap();

    // Store "canonical" blocks at heights 0-2, slots 1-3
    for i in 0..3u64 {
        let block = create_test_block(i as u32 + 1, &producer);
        store.put_block_canonical(&block, i).unwrap();
    }

    // Canonical blocks exist
    assert!(store.get_block_by_height(0).unwrap().is_some());
    assert!(store.get_block_by_height(2).unwrap().is_some());

    // Old fork heights should NOT exist
    assert!(
        store.get_block_by_height(3).unwrap().is_none(),
        "Old fork height 3 should be gone after clear"
    );
    assert!(
        store.get_block_by_height(4).unwrap().is_none(),
        "Old fork height 4 should be gone after clear"
    );

    // Old fork slots should NOT exist
    assert!(
        store.get_block_by_slot(100).unwrap().is_none(),
        "Old fork slot 100 should be gone after clear"
    );

    // Only 3 entries in slot index
    assert_eq!(store.count_slot_index_entries().unwrap(), 3);
}

#[test]
fn test_set_canonical_chain_reorg() {
    let (store, _dir) = create_test_store();
    let keypair = KeyPair::generate();
    let producer = *keypair.public_key();

    // Block A (genesis) at height 0
    let block_a = create_test_block(1, &producer);
    let hash_a = block_a.hash();
    store.put_block(&block_a, 0).unwrap();
    store.set_canonical_chain(hash_a, 0).unwrap();

    // Block B (child of A) at height 1
    let mut header_b = create_test_header(2, &producer);
    header_b.prev_hash = hash_a;
    let block_b = Block::new(header_b, vec![]);
    let hash_b = block_b.hash();
    store.put_block(&block_b, 1).unwrap();
    store.set_canonical_chain(hash_b, 1).unwrap();

    // Block C (child of B) at height 2
    let mut header_c = create_test_header(3, &producer);
    header_c.prev_hash = hash_b;
    let block_c = Block::new(header_c, vec![]);
    let hash_c = block_c.hash();
    store.put_block(&block_c, 2).unwrap();
    store.set_canonical_chain(hash_c, 2).unwrap();

    assert_eq!(store.get_hash_by_height(0).unwrap(), Some(hash_a));
    assert_eq!(store.get_hash_by_height(1).unwrap(), Some(hash_b));
    assert_eq!(store.get_hash_by_height(2).unwrap(), Some(hash_c));

    // Fork: B' at height 1
    let mut header_b2 = create_test_header(4, &producer);
    header_b2.prev_hash = hash_a;
    let block_b2 = Block::new(header_b2, vec![]);
    let hash_b2 = block_b2.hash();
    store.put_block(&block_b2, 1).unwrap();

    // C' at height 2
    let mut header_c2 = create_test_header(5, &producer);
    header_c2.prev_hash = hash_b2;
    let block_c2 = Block::new(header_c2, vec![]);
    let hash_c2 = block_c2.hash();
    store.put_block(&block_c2, 2).unwrap();

    // D' at height 3
    let mut header_d2 = create_test_header(6, &producer);
    header_d2.prev_hash = hash_c2;
    let block_d2 = Block::new(header_d2, vec![]);
    let hash_d2 = block_d2.hash();
    store.put_block(&block_d2, 3).unwrap();

    store.set_canonical_chain(hash_d2, 3).unwrap();

    assert_eq!(store.get_hash_by_height(0).unwrap(), Some(hash_a));
    assert_eq!(store.get_hash_by_height(1).unwrap(), Some(hash_b2));
    assert_eq!(store.get_hash_by_height(2).unwrap(), Some(hash_c2));
    assert_eq!(store.get_hash_by_height(3).unwrap(), Some(hash_d2));

    assert!(store.get_block(&hash_b).unwrap().is_some());
    assert!(store.get_block(&hash_c).unwrap().is_some());
    assert_eq!(store.get_height_by_hash(&hash_b2).unwrap(), Some(1));
    assert_eq!(store.get_height_by_hash(&hash_d2).unwrap(), Some(3));
}

#[test]
fn set_canonical_chain_stops_at_snap_horizon() {
    let (store, _dir) = create_test_store();
    let keypair = KeyPair::generate();
    let producer = *keypair.public_key();

    let snap_height = 100u64;
    let snap_hash = Hash::from_bytes([0xAA; 32]);
    store.seed_canonical_index(snap_hash, snap_height).unwrap();
    assert_eq!(store.get_snap_horizon().unwrap(), Some(snap_height));

    let mut header_101 = create_test_header(200, &producer);
    header_101.prev_hash = snap_hash;
    let block_101 = Block::new(header_101, vec![]);
    let hash_101 = block_101.hash();
    store.put_block(&block_101, snap_height + 1).unwrap();

    let mut header_102 = create_test_header(201, &producer);
    header_102.prev_hash = hash_101;
    let block_102 = Block::new(header_102, vec![]);
    let hash_102 = block_102.hash();
    store.put_block(&block_102, snap_height + 2).unwrap();

    store
        .set_canonical_chain(hash_102, snap_height + 2)
        .unwrap();

    assert_eq!(
        store.get_hash_by_height(snap_height + 1).unwrap(),
        Some(hash_101)
    );
    assert_eq!(
        store.get_hash_by_height(snap_height + 2).unwrap(),
        Some(hash_102)
    );
    assert_eq!(
        store.get_hash_by_height(snap_height).unwrap(),
        Some(snap_hash)
    );
}

#[test]
fn get_snap_horizon_returns_none_without_snap_sync() {
    let (store, _dir) = create_test_store();
    assert_eq!(store.get_snap_horizon().unwrap(), None);
}

// ============================================================
// REQ-REDESIGN-011 -- ensure_blocks_present (FORK_GUARD backfill)
// ============================================================

#[test]
fn ensure_blocks_present_empty_range_is_ok() {
    let (store, _dir) = create_test_store();
    assert!(store.ensure_blocks_present(5, 4).is_ok());
}

#[test]
fn ensure_blocks_present_low_zero_skips_genesis() {
    let (store, _dir) = create_test_store();
    let kp = KeyPair::generate();
    let producer = *kp.public_key();
    let b1 = create_test_block(1, &producer);
    let b2 = create_test_block(2, &producer);
    store.put_block_canonical(&b1, 1).unwrap();
    store.put_block_canonical(&b2, 2).unwrap();
    assert!(store.ensure_blocks_present(0, 2).is_ok());
}

#[test]
fn ensure_blocks_present_dense_range_returns_ok() {
    let (store, _dir) = create_test_store();
    let kp = KeyPair::generate();
    let producer = *kp.public_key();
    for h in 1..=5 {
        let b = create_test_block(h as u32, &producer);
        store.put_block_canonical(&b, h).unwrap();
    }
    assert!(store.ensure_blocks_present(1, 5).is_ok());
    assert!(store.ensure_blocks_present(2, 4).is_ok());
}

#[test]
fn ensure_blocks_present_reports_first_missing_height() {
    let (store, _dir) = create_test_store();
    let kp = KeyPair::generate();
    let producer = *kp.public_key();
    for h in 1..=5 {
        let b = create_test_block(h as u32, &producer);
        store.put_block_canonical(&b, h).unwrap();
    }
    let deleted = store.delete_blocks_above(2).unwrap();
    assert_eq!(deleted, 3);
    let err = store.ensure_blocks_present(1, 5).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("FORK_GUARD_BACKFILL"),
        "error must mention FORK_GUARD_BACKFILL, got: {}",
        msg
    );
    assert!(
        msg.contains("height 3"),
        "error must name FIRST missing height (3), got: {}",
        msg
    );
}

// ============================================================
// INC-I-104 M2: per-CF tuning regression tests
// ============================================================
//
// OUTPUT CONTRACT for BlockStore::open() per-CF tuning (M2):
//   Function under test: BlockStore::open(path) -- configures per-CF Options
//   Observable outputs:
//     1. Return value: Result<BlockStore, StorageError> (Ok on success)
//     2. RocksDB DB handle: all 9 CFs present with per-CF tuning applied
//     3. Metrics (via metrics()): memtable_max_bytes bounded by db_write_buffer_size
//     4. Side effect: DB accepts read/write operations on all CFs
//
//   Code paths:
//     P1: Normal open (fresh directory) -- creates DB + all CFs
//     P2: Reopen (existing DB) -- opens existing CFs with new options
//
//   INPUT PARTITIONS:
//     I1: Fresh tempdir (P1) -- exercises CF creation with per-CF options
//     I2: After write+read (P1) -- exercises that per-CF options don't break I/O
//
//   Matrix:
//     m2_per_cf_memtable_budget_bounded: O3 x P1 x I1
//     m2_all_nine_cfs_present:           O2 x P1 x I1
//     m2_open_write_metrics_smoke:        O1,O4 x P1 x I2

/// Verify that the DB-level memtable cap from M0 is still effective after
/// M2's per-CF tuning. Sum of per-CF write_buffer_size * max_write_buffer_number:
///   headers(8*2) + bodies(8*2) + height(4*2) + slot(4*2) +
///   h2h(4*2) + tx(4*2) + addr(4*2) + presence(1*1) + meta(1*1) = 66 MB
/// The db_write_buffer_size=48 MB caps actual usage below 66 MB.
#[test]
fn m2_per_cf_memtable_budget_bounded() {
    let (store, _dir) = create_test_store();
    let m = store.metrics();
    // 48 MB cap + 10% overhead margin for RocksDB internal accounting
    let cap_with_margin = (48 * 1024 * 1024) as f64 * 1.1;
    assert!(
        (m.memtable_max_bytes as f64) <= cap_with_margin,
        "memtable_max_bytes={} exceeds 48 MB cap (with 10% margin={})",
        m.memtable_max_bytes,
        cap_with_margin as u64,
    );
}

/// Verify all 9 CFs are present after open (C-004: presence kept).
#[test]
fn m2_all_nine_cfs_present() {
    let (store, _dir) = create_test_store();
    let cfs = [
        "headers",
        "bodies",
        "height_index",
        "slot_index",
        "presence",
        "hash_to_height",
        "tx_index",
        "addr_tx_index",
        "meta",
    ];
    for cf in &cfs {
        assert!(
            store.db.cf_handle(cf).is_some(),
            "CF '{}' missing after open",
            cf
        );
    }
}

/// Verify block_store opens, accepts writes, and metrics are sane.
/// This catches any per-CF option that RocksDB rejects at open time.
#[test]
fn m2_open_write_metrics_smoke() {
    let (store, _dir) = create_test_store();
    let keypair = KeyPair::generate();
    let producer = *keypair.public_key();

    // Write a block to exercise multiple CFs
    let block = create_test_block(1, &producer);
    store.put_block_canonical(&block, 0).unwrap();

    // Read it back
    let retrieved = store.get_block_by_height(0).unwrap();
    assert!(retrieved.is_some());

    // Metrics: writes should not be stopped and no background errors
    let m = store.metrics();
    assert_eq!(m.is_write_stopped, 0, "writes should not be stopped");
    assert_eq!(m.background_errors, 0, "no background errors expected");
}

// ============================================================
// INC-I-136 M2: has_contiguous_bodies (block-store body gap detection)
// ============================================================
//
// OUTPUT CONTRACT: fn has_contiguous_bodies(&self, from: u64, to: u64) -> bool
// Outputs:
//   O1: return (bool) — true iff every height in [from,to] has a stored block body
//
// Paths:
//   P1: full store — all heights in [from,to] have block bodies
//   P2: missing interior height — at least one height in (from,to) has no body
//   P3: empty range — from > to
//   P4: single height — from == to
//
// INPUT PARTITIONS:
//   P1a: dense range [1..5], all blocks present (standard happy path)
//   P1b: single block at from==to, block present (degenerate range)
//   P2a: 5 blocks stored at heights 1-5, height 3 deleted (interior gap)
//   P2b: 5 blocks stored at heights 1-5, height 1 deleted (leading gap)
//   P2c: 5 blocks stored at heights 1-5, height 5 deleted (trailing gap)
//   P3a: from=5, to=4 (inverted range → empty → true)
//   P4a: from=3, to=3, block present → true
//   P4b: from=3, to=3, block absent → false
//
// MATRIX: 1 output x 8 partitions = 8 cells
//   P1a: O1(true)  ✓
//   P1b: O1(true)  ✓
//   P2a: O1(false) ✓
//   P2b: O1(false) ✓
//   P2c: O1(false) ✓
//   P3a: O1(true)  ✓
//   P4a: O1(true)  ✓
//   P4b: O1(false) ✓
//
// Requirement: REQ-GUARD-003 (Must) — F4
// Acceptance: No healthy checkpoint can exist that fails to serve its own tip

#[test]
fn test_m2_contiguous_bodies_full_range() {
    // P1a: all heights [1..5] present → true
    // Requirement: REQ-GUARD-003 (Must)
    let (store, _dir) = create_test_store();
    let kp = KeyPair::generate();
    let producer = *kp.public_key();

    for h in 1..=5u64 {
        let block = create_test_block(h as u32, &producer);
        store.put_block_canonical(&block, h).unwrap();
    }

    assert_eq!(
        store.has_contiguous_bodies(1, 5),
        true,
        "P1a O1: full range [1..5] with all blocks must return true"
    );
}

#[test]
fn test_m2_contiguous_bodies_interior_gap() {
    // P2a: heights 1-5 present, then height 3 block deleted → false
    // This is the core failure mode from INC-I-136: checkpoint captures
    // a block store with a body gap.
    // Requirement: REQ-GUARD-003 (Must)
    let (store, _dir) = create_test_store();
    let kp = KeyPair::generate();
    let producer = *kp.public_key();

    for h in 1..=5u64 {
        let block = create_test_block(h as u32, &producer);
        store.put_block_canonical(&block, h).unwrap();
    }

    // Delete blocks above height 2 to create a gap at height 3
    // (delete_blocks_above removes heights 3, 4, 5 from height index)
    store.delete_blocks_above(2).unwrap();

    // Re-add heights 4 and 5 (height 3 stays missing)
    let block4 = create_test_block(4, &producer);
    store.put_block_canonical(&block4, 4).unwrap();
    let block5 = create_test_block(5, &producer);
    store.put_block_canonical(&block5, 5).unwrap();

    assert_eq!(
        store.has_contiguous_bodies(1, 5),
        false,
        "P2a O1: missing height 3 body must return false"
    );
}

#[test]
fn test_m2_contiguous_bodies_empty_range() {
    // P3a: from > to (inverted/empty range) → true
    // Requirement: REQ-GUARD-003 (Must)
    let (store, _dir) = create_test_store();

    assert_eq!(
        store.has_contiguous_bodies(5, 4),
        true,
        "P3a O1: empty range (from > to) must return true"
    );
}

#[test]
fn test_m2_contiguous_bodies_single_present() {
    // P4a: from==to==3, block present → true
    // Requirement: REQ-GUARD-003 (Must)
    let (store, _dir) = create_test_store();
    let kp = KeyPair::generate();
    let producer = *kp.public_key();

    let block = create_test_block(3, &producer);
    store.put_block_canonical(&block, 3).unwrap();

    assert_eq!(
        store.has_contiguous_bodies(3, 3),
        true,
        "P4a O1: single height with block present must return true"
    );
}

#[test]
fn test_m2_contiguous_bodies_single_absent() {
    // P4b: from==to==3, no block at height 3 → false
    // Requirement: REQ-GUARD-003 (Must)
    let (store, _dir) = create_test_store();

    assert_eq!(
        store.has_contiguous_bodies(3, 3),
        false,
        "P4b O1: single height with no block must return false"
    );
}

#[test]
fn test_m2_contiguous_bodies_leading_gap() {
    // P2b: heights 2-5 present but height 1 missing → false over [1..5]
    // Requirement: REQ-GUARD-003 (Must)
    let (store, _dir) = create_test_store();
    let kp = KeyPair::generate();
    let producer = *kp.public_key();

    // Only store heights 2-5, leaving height 1 empty
    for h in 2..=5u64 {
        let block = create_test_block(h as u32, &producer);
        store.put_block_canonical(&block, h).unwrap();
    }

    assert_eq!(
        store.has_contiguous_bodies(1, 5),
        false,
        "P2b O1: missing leading height 1 must return false"
    );
}

#[test]
fn test_m2_contiguous_bodies_trailing_gap() {
    // P2c: heights 1-4 present but height 5 missing → false over [1..5]
    // Requirement: REQ-GUARD-003 (Must)
    let (store, _dir) = create_test_store();
    let kp = KeyPair::generate();
    let producer = *kp.public_key();

    // Only store heights 1-4, leaving height 5 empty
    for h in 1..=4u64 {
        let block = create_test_block(h as u32, &producer);
        store.put_block_canonical(&block, h).unwrap();
    }

    assert_eq!(
        store.has_contiguous_bodies(1, 5),
        false,
        "P2c O1: missing trailing height 5 must return false"
    );
}
