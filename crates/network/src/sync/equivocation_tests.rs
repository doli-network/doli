//! Unit tests for `equivocation.rs`.

use super::*;
use crypto::Hash;
use doli_core::BlockHeader;
use vdf::{VdfOutput, VdfProof};

fn create_test_block(slot: u32, producer: &PublicKey, merkle_root: Hash) -> Block {
    let header = BlockHeader {
        version: 1,
        prev_hash: Hash::ZERO,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot,
        producer: *producer,
        vdf_output: VdfOutput { value: vec![] },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![])
}

#[test]
fn test_no_equivocation_same_block() {
    let mut detector = EquivocationDetector::new();
    let producer = PublicKey::from_bytes([1u8; 32]);

    let block = create_test_block(10, &producer, Hash::ZERO);

    // First time - no equivocation
    assert!(detector.check_block(&block).is_none());

    // Same block again - still no equivocation
    assert!(detector.check_block(&block).is_none());
}

#[test]
fn test_no_equivocation_different_slots() {
    let mut detector = EquivocationDetector::new();
    let producer = PublicKey::from_bytes([1u8; 32]);

    let block1 = create_test_block(10, &producer, Hash::ZERO);
    let block2 = create_test_block(11, &producer, crypto::hash::hash(b"diff"));

    // Different slots - no equivocation
    assert!(detector.check_block(&block1).is_none());
    assert!(detector.check_block(&block2).is_none());
}

#[test]
fn test_no_equivocation_different_producers() {
    let mut detector = EquivocationDetector::new();
    let producer1 = PublicKey::from_bytes([1u8; 32]);
    let producer2 = PublicKey::from_bytes([2u8; 32]);

    let block1 = create_test_block(10, &producer1, Hash::ZERO);
    let block2 = create_test_block(10, &producer2, crypto::hash::hash(b"diff"));

    // Same slot but different producers - no equivocation
    assert!(detector.check_block(&block1).is_none());
    assert!(detector.check_block(&block2).is_none());
}

#[test]
fn test_detect_equivocation() {
    let mut detector = EquivocationDetector::new();
    let producer = PublicKey::from_bytes([1u8; 32]);

    // Same producer, same slot, different blocks
    let block1 = create_test_block(10, &producer, Hash::ZERO);
    let block2 = create_test_block(10, &producer, crypto::hash::hash(b"different"));

    // First block - no equivocation
    assert!(detector.check_block(&block1).is_none());

    // Second different block for same slot - EQUIVOCATION!
    let proof = detector.check_block(&block2);
    assert!(proof.is_some());

    let proof = proof.unwrap();
    assert_eq!(proof.producer, producer);
    assert_eq!(proof.slot, 10);
    assert_eq!(proof.block_header_1.hash(), block1.hash());
    assert_eq!(proof.block_header_2.hash(), block2.hash());

    // Should also be in pending proofs
    assert!(detector.has_pending_proofs());
    let pending = detector.take_pending_proofs();
    assert_eq!(pending.len(), 1);
}

#[test]
fn test_proof_to_slash_transaction() {
    let producer = PublicKey::from_bytes([1u8; 32]);
    let reporter = KeyPair::generate();

    // Create test block headers
    let header1 = BlockHeader {
        version: 1,
        prev_hash: Hash::ZERO,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot: 42,
        producer,
        vdf_output: VdfOutput { value: vec![] },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    let header2 = BlockHeader {
        version: 1,
        prev_hash: Hash::ZERO,
        merkle_root: crypto::hash::hash(b"different"),
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot: 42,
        producer,
        vdf_output: VdfOutput { value: vec![] },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };

    let proof = EquivocationProof {
        producer,
        block_header_1: header1.clone(),
        block_header_2: header2.clone(),
        slot: 42,
    };

    let slash_tx = proof.to_slash_transaction(&reporter);

    assert!(slash_tx.is_slash_producer());
    let slash_data = slash_tx.slash_data().unwrap();
    assert_eq!(slash_data.producer_pubkey, producer);

    let SlashingEvidence::DoubleProduction {
        block_header_1,
        block_header_2,
    } = slash_data.evidence
    else {
        panic!("expected DoubleProduction evidence");
    };
    assert_eq!(block_header_1.slot, 42);
    assert_eq!(block_header_2.slot, 42);
    assert_eq!(block_header_1.hash(), header1.hash());
    assert_eq!(block_header_2.hash(), header2.hash());
}

#[test]
fn test_eviction() {
    let mut detector = EquivocationDetector::new();
    detector.max_tracked = 10;

    // Add more than max entries
    for i in 0..20u32 {
        let producer = PublicKey::from_bytes([i as u8; 32]);
        let block = create_test_block(i, &producer, Hash::ZERO);
        detector.check_block(&block);
    }

    // Should only have max_tracked entries
    assert!(detector.tracked_count() <= 10);
}

#[test]
fn test_sliding_window_cleanup() {
    let mut detector = EquivocationDetector::new();
    let producer = PublicKey::from_bytes([1u8; 32]);

    // Add entries at slots 100-109
    for slot in 100..110u32 {
        let block = create_test_block(slot, &producer, crypto::hash::hash(&slot.to_le_bytes()));
        detector.check_block(&block);
    }
    assert_eq!(detector.tracked_count(), 10);

    // Cleanup slots before 105
    detector.cleanup_before_slot(105);
    assert_eq!(detector.tracked_count(), 5);

    // Entries at 100-104 should be gone
    assert!(!detector.seen_blocks.contains_key(&(producer, 100)));
    assert!(!detector.seen_blocks.contains_key(&(producer, 104)));

    // Entries at 105-109 should still exist
    assert!(detector.seen_blocks.contains_key(&(producer, 105)));
    assert!(detector.seen_blocks.contains_key(&(producer, 109)));
}

#[test]
fn test_old_slots_evicted_on_check() {
    let mut detector = EquivocationDetector::new();
    let producer = PublicKey::from_bytes([1u8; 32]);

    // Add entry at slot 10
    let block = create_test_block(10, &producer, Hash::ZERO);
    detector.check_block(&block);
    assert_eq!(detector.tracked_count(), 1);

    // Check a block at slot 10 + SLIDING_WINDOW_SLOTS + 1 → triggers cleanup
    let far_slot = 10 + SLIDING_WINDOW_SLOTS + 1;
    let producer2 = PublicKey::from_bytes([2u8; 32]);
    let block2 = create_test_block(far_slot, &producer2, Hash::ZERO);
    detector.check_block(&block2);

    // Old slot 10 entry should be evicted
    assert!(!detector.seen_blocks.contains_key(&(producer, 10)));
    // New entry should exist
    assert!(detector.seen_blocks.contains_key(&(producer2, far_slot)));
}

#[test]
fn test_cleanup_idempotent() {
    let mut detector = EquivocationDetector::new();
    let producer = PublicKey::from_bytes([1u8; 32]);

    for slot in 0..5u32 {
        let block = create_test_block(slot, &producer, crypto::hash::hash(&slot.to_le_bytes()));
        detector.check_block(&block);
    }

    // Cleanup twice with same min_slot — second call is no-op
    detector.cleanup_before_slot(3);
    let count_after_first = detector.tracked_count();
    detector.cleanup_before_slot(3);
    assert_eq!(detector.tracked_count(), count_after_first);

    // Cleanup with lower min_slot — also no-op
    detector.cleanup_before_slot(1);
    assert_eq!(detector.tracked_count(), count_after_first);
}
