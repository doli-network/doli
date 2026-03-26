use super::*;

#[test]
fn test_empty_commitment() {
    let commitment = PresenceCommitment::empty();

    assert!(commitment.is_empty());
    assert_eq!(commitment.present_count(), 0);
    assert_eq!(commitment.total_weight(), 0);
    assert!(!commitment.is_present(0));
    assert!(commitment.get_weight(0).is_none());
}

#[test]
fn test_bitfield_encoding() {
    // 10 producers, indices 2, 5, 7 present
    let present = [2, 5, 7];
    let weights = vec![1000, 2000, 3000];
    let commitment = PresenceCommitment::new(10, &present, weights.clone(), Hash::ZERO);

    // Check bitfield size
    assert_eq!(commitment.bitfield.len(), 2); // ceil(10/8) = 2 bytes

    // Check presence
    assert!(!commitment.is_present(0));
    assert!(!commitment.is_present(1));
    assert!(commitment.is_present(2));
    assert!(!commitment.is_present(3));
    assert!(!commitment.is_present(4));
    assert!(commitment.is_present(5));
    assert!(!commitment.is_present(6));
    assert!(commitment.is_present(7));
    assert!(!commitment.is_present(8));
    assert!(!commitment.is_present(9));

    // Out of bounds
    assert!(!commitment.is_present(100));
}

#[test]
fn test_get_weight() {
    // 5 producers, indices 1, 3 present with weights 100, 200
    let present = [1, 3];
    let weights = vec![100, 200];
    let commitment = PresenceCommitment::new(5, &present, weights, Hash::ZERO);

    assert_eq!(commitment.get_weight(0), None);
    assert_eq!(commitment.get_weight(1), Some(100));
    assert_eq!(commitment.get_weight(2), None);
    assert_eq!(commitment.get_weight(3), Some(200));
    assert_eq!(commitment.get_weight(4), None);
}

#[test]
fn test_total_weight() {
    let present = [0, 2, 4];
    let weights = vec![1000, 2000, 3000];
    let commitment = PresenceCommitment::new(5, &present, weights, Hash::ZERO);

    assert_eq!(commitment.total_weight(), 6000);
    assert_eq!(commitment.present_count(), 3);
}

#[test]
fn test_verify_total_weight() {
    let present = [0, 1];
    let weights = vec![100, 200];
    let commitment = PresenceCommitment::new(10, &present, weights, Hash::ZERO);

    assert!(commitment.verify_total_weight());
}

#[test]
fn test_verify_bitfield_weight_count() {
    let present = [0, 2, 4, 6];
    let weights = vec![10, 20, 30, 40];
    let commitment = PresenceCommitment::new(10, &present, weights, Hash::ZERO);

    assert!(commitment.verify_bitfield_weight_count());
}

#[test]
fn test_iter_present() {
    let present = [1, 3, 5];
    let weights = vec![100, 200, 300];
    let commitment = PresenceCommitment::new(8, &present, weights, Hash::ZERO);

    let collected: Vec<_> = commitment.iter_present().collect();
    assert_eq!(collected, vec![(1, 100), (3, 200), (5, 300)]);
}

#[test]
fn test_serialization_roundtrip() {
    let present = [0, 5, 10];
    let weights = vec![1000, 2000, 3000];
    let merkle = Hash::from_bytes([42u8; 32]);
    let commitment = PresenceCommitment::new(16, &present, weights, merkle);

    let serialized = commitment.serialize();
    let deserialized = PresenceCommitment::deserialize(&serialized).unwrap();

    assert_eq!(commitment, deserialized);
}

#[test]
fn test_large_producer_set() {
    // 100 producers, every 5th one present
    let present: Vec<usize> = (0..100).filter(|i| i % 5 == 0).collect();
    let weights: Vec<Amount> = present.iter().map(|i| (*i as Amount + 1) * 1000).collect();
    let commitment = PresenceCommitment::new(100, &present, weights.clone(), Hash::ZERO);

    // Bitfield size
    assert_eq!(commitment.bitfield.len(), 13); // ceil(100/8) = 13 bytes

    // Present count
    assert_eq!(commitment.present_count(), 20);

    // Check some values
    assert!(commitment.is_present(0));
    assert!(!commitment.is_present(1));
    assert!(commitment.is_present(5));
    assert!(commitment.is_present(95));
    assert!(!commitment.is_present(99));

    // Weights
    assert_eq!(commitment.get_weight(0), Some(1000));
    assert_eq!(commitment.get_weight(5), Some(6000));
    assert_eq!(commitment.get_weight(95), Some(96000));
}

#[test]
fn test_size_estimation() {
    let present = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]; // 10 present
    let weights = vec![1000; 10];
    let commitment = PresenceCommitment::new(100, &present, weights, Hash::ZERO);

    // Size should be: 4 (len prefix) + 13 (bitfield) + 32 (merkle) + 80 (10*8 weights) + 8 (total)
    // = 137 bytes
    let size = commitment.size();
    assert!(size > 100);
    assert!(size < 200);
}

#[test]
#[should_panic(expected = "present_indices and weights must have same length")]
fn test_mismatched_lengths_panics() {
    let present = [0, 1, 2];
    let weights = vec![100, 200]; // Wrong length!
    PresenceCommitment::new(10, &present, weights, Hash::ZERO);
}

#[test]
fn test_default() {
    let commitment = PresenceCommitment::default();
    assert!(commitment.is_empty());
    assert_eq!(commitment, PresenceCommitment::empty());
}

#[test]
fn test_ensure_present_adds_missing_producer() {
    // Start with producers 1 and 3 present
    let present = [1, 3];
    let weights = vec![100, 300];
    let mut commitment = PresenceCommitment::new(5, &present, weights, Hash::ZERO);

    assert_eq!(commitment.present_count(), 2);
    assert_eq!(commitment.total_weight(), 400);
    assert!(!commitment.is_present(2));

    // Add producer 2 (missing)
    commitment.ensure_present(2, 200, 5);

    assert_eq!(commitment.present_count(), 3);
    assert_eq!(commitment.total_weight(), 600);
    assert!(commitment.is_present(1));
    assert!(commitment.is_present(2));
    assert!(commitment.is_present(3));

    // Check weights are in correct order
    assert_eq!(commitment.get_weight(1), Some(100));
    assert_eq!(commitment.get_weight(2), Some(200));
    assert_eq!(commitment.get_weight(3), Some(300));
}

#[test]
fn test_ensure_present_noop_if_already_present() {
    let present = [0, 2, 4];
    let weights = vec![100, 200, 300];
    let mut commitment = PresenceCommitment::new(5, &present, weights, Hash::ZERO);

    let original_count = commitment.present_count();
    let original_weight = commitment.total_weight();

    // Try to add producer 2 who is already present
    commitment.ensure_present(2, 999, 5);

    // Should be unchanged
    assert_eq!(commitment.present_count(), original_count);
    assert_eq!(commitment.total_weight(), original_weight);
    assert_eq!(commitment.get_weight(2), Some(200)); // Original weight preserved
}

#[test]
fn test_ensure_present_at_beginning() {
    // Start with producers 2, 3, 4 present
    let present = [2, 3, 4];
    let weights = vec![200, 300, 400];
    let mut commitment = PresenceCommitment::new(5, &present, weights, Hash::ZERO);

    // Add producer 0 (at the beginning)
    commitment.ensure_present(0, 50, 5);

    assert_eq!(commitment.present_count(), 4);
    assert!(commitment.is_present(0));
    assert_eq!(commitment.get_weight(0), Some(50));
    assert_eq!(commitment.get_weight(2), Some(200));
    assert_eq!(commitment.get_weight(3), Some(300));
    assert_eq!(commitment.get_weight(4), Some(400));
}

#[test]
fn test_ensure_present_at_end() {
    // Start with producers 0, 1, 2 present
    let present = [0, 1, 2];
    let weights = vec![100, 200, 300];
    let mut commitment = PresenceCommitment::new(5, &present, weights, Hash::ZERO);

    // Add producer 4 (at the end)
    commitment.ensure_present(4, 500, 5);

    assert_eq!(commitment.present_count(), 4);
    assert!(commitment.is_present(4));
    assert_eq!(commitment.get_weight(0), Some(100));
    assert_eq!(commitment.get_weight(1), Some(200));
    assert_eq!(commitment.get_weight(2), Some(300));
    assert_eq!(commitment.get_weight(4), Some(500));
}

#[test]
fn test_ensure_present_on_empty() {
    let mut commitment = PresenceCommitment::empty();

    commitment.ensure_present(3, 1000, 5);

    assert_eq!(commitment.present_count(), 1);
    assert_eq!(commitment.total_weight(), 1000);
    assert!(commitment.is_present(3));
    assert_eq!(commitment.get_weight(3), Some(1000));
}

#[test]
fn test_ensure_present_expands_bitfield() {
    // Start with small bitfield (8 producers)
    let present = [0, 1];
    let weights = vec![100, 200];
    let mut commitment = PresenceCommitment::new(8, &present, weights, Hash::ZERO);

    assert_eq!(commitment.bitfield.len(), 1); // 8 bits = 1 byte

    // Add producer at index 15 (requires 2 bytes)
    commitment.ensure_present(15, 500, 16);

    assert_eq!(commitment.bitfield.len(), 2); // Now needs 16 bits = 2 bytes
    assert!(commitment.is_present(15));
    assert_eq!(commitment.get_weight(15), Some(500));
}

#[test]
fn test_commitment_hash_deterministic() {
    let present = [1, 3, 5];
    let weights = vec![100, 200, 300];
    let merkle = Hash::from_bytes([42u8; 32]);
    let commitment = PresenceCommitment::new(8, &present, weights, merkle);

    let hash1 = commitment.commitment_hash();
    let hash2 = commitment.commitment_hash();

    assert_eq!(hash1, hash2);
    assert!(!hash1.is_zero());
}

#[test]
fn test_commitment_hash_different_for_different_data() {
    let present = [1, 3];
    let weights = vec![100, 200];

    let commitment1 = PresenceCommitment::new(8, &present, weights.clone(), Hash::ZERO);
    let commitment2 = PresenceCommitment::new(8, &[1, 4], weights, Hash::ZERO); // Different indices

    assert_ne!(commitment1.commitment_hash(), commitment2.commitment_hash());
}

// =========================================================================
// PresenceCommitmentV2 Tests
// =========================================================================

#[test]
fn test_v2_empty() {
    let commitment = PresenceCommitmentV2::empty();

    assert!(commitment.is_empty());
    assert_eq!(commitment.total_weight(), 0);
    assert_eq!(commitment.heartbeats_root, Hash::ZERO);
}

#[test]
fn test_v2_size_is_constant() {
    let commitment1 = PresenceCommitmentV2::empty();
    let commitment2 = PresenceCommitmentV2::new(Hash::from_bytes([42u8; 32]), 1_000_000_000);

    assert_eq!(commitment1.size(), 40);
    assert_eq!(commitment2.size(), 40);
    assert_eq!(PresenceCommitmentV2::SIZE, 40);
}

#[test]
fn test_v2_commitment_hash_deterministic() {
    let root = Hash::from_bytes([1u8; 32]);
    let commitment = PresenceCommitmentV2::new(root, 5_000_000_000_000);

    let hash1 = commitment.commitment_hash();
    let hash2 = commitment.commitment_hash();

    assert_eq!(hash1, hash2);
    assert!(!hash1.is_zero());
}

#[test]
fn test_v2_commitment_hash_different_for_different_data() {
    let root = Hash::from_bytes([1u8; 32]);

    let commitment1 = PresenceCommitmentV2::new(root, 5_000_000_000_000);
    let commitment2 = PresenceCommitmentV2::new(root, 5_000_000_000_001); // Different weight

    assert_ne!(commitment1.commitment_hash(), commitment2.commitment_hash());

    let commitment3 = PresenceCommitmentV2::new(Hash::from_bytes([2u8; 32]), 5_000_000_000_000);
    assert_ne!(commitment1.commitment_hash(), commitment3.commitment_hash());
}

#[test]
fn test_v2_serialization_roundtrip() {
    let root = Hash::from_bytes([42u8; 32]);
    let commitment = PresenceCommitmentV2::new(root, 123_456_789_000);

    let serialized = commitment.serialize();
    let deserialized = PresenceCommitmentV2::deserialize(&serialized).unwrap();

    assert_eq!(commitment, deserialized);
}

#[test]
fn test_v2_default() {
    let commitment = PresenceCommitmentV2::default();
    assert!(commitment.is_empty());
    assert_eq!(commitment, PresenceCommitmentV2::empty());
}
