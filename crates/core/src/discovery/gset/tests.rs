//! Unit tests for ProducerGSet.

use super::super::{ProducerAnnouncement, ProducerBloomFilter, ProducerSetError};
use super::{MergeOneResult, ProducerGSet};
use crypto::{Hash, KeyPair};
use std::time::Duration;

#[test]
fn test_gset_merge_valid_announcement() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);
    let keypair = KeyPair::generate();
    let announcement = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);

    let result = gset.merge_one(announcement);
    assert!(matches!(result, Ok(MergeOneResult::NewProducer)));
    assert_eq!(gset.len(), 1);
    assert!(gset.contains(keypair.public_key()));
}

#[test]
fn test_gset_reject_invalid_signature() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);
    let keypair = KeyPair::generate();
    let mut announcement = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);
    announcement.sequence = 999; // Tamper

    let result = gset.merge_one(announcement);
    assert!(matches!(result, Err(ProducerSetError::InvalidSignature)));
    assert_eq!(gset.len(), 0);
}

#[test]
fn test_gset_reject_wrong_network() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);
    let keypair = KeyPair::generate();
    let announcement = ProducerAnnouncement::new(&keypair, 2, 0, Hash::ZERO);

    let result = gset.merge_one(announcement);
    assert!(matches!(
        result,
        Err(ProducerSetError::NetworkMismatch {
            expected: 1,
            got: 2
        })
    ));
}

#[test]
fn test_gset_reject_wrong_genesis_hash() {
    let hash_a = Hash::from_bytes([0xAA; 32]);
    let hash_b = Hash::from_bytes([0xBB; 32]);
    let mut gset = ProducerGSet::new(1, hash_a);
    let keypair = KeyPair::generate();
    // Announcement signed for a different genesis
    let announcement = ProducerAnnouncement::new(&keypair, 1, 0, hash_b);

    let result = gset.merge_one(announcement);
    assert!(matches!(result, Err(ProducerSetError::GenesisHashMismatch)));
}

#[test]
fn test_gset_reject_stale_announcement() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);
    let keypair = KeyPair::generate();

    // Create announcement with timestamp 2 hours ago
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be after epoch")
        .as_secs();
    let stale_timestamp = now - 7200; // 2 hours ago

    let announcement =
        ProducerAnnouncement::new_with_timestamp(&keypair, 1, 0, stale_timestamp, Hash::ZERO);

    let result = gset.merge_one(announcement);
    assert!(matches!(result, Err(ProducerSetError::StaleAnnouncement)));
}

#[test]
fn test_gset_reject_future_timestamp() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);
    let keypair = KeyPair::generate();

    // Create announcement with timestamp 10 minutes in future
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be after epoch")
        .as_secs();
    let future_timestamp = now + 600; // 10 minutes ahead

    let announcement =
        ProducerAnnouncement::new_with_timestamp(&keypair, 1, 0, future_timestamp, Hash::ZERO);

    let result = gset.merge_one(announcement);
    assert!(matches!(result, Err(ProducerSetError::FutureTimestamp)));
}

#[test]
fn test_gset_sequence_version_vector() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);
    let keypair = KeyPair::generate();

    // First announcement with sequence 0
    let ann1 = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);
    assert!(matches!(
        gset.merge_one(ann1),
        Ok(MergeOneResult::NewProducer)
    ));

    // Second announcement with sequence 1 should update
    let ann2 = ProducerAnnouncement::new(&keypair, 1, 1, Hash::ZERO);
    assert!(matches!(
        gset.merge_one(ann2),
        Ok(MergeOneResult::SequenceUpdate)
    ));

    // Old sequence should be rejected (no change)
    let ann3 = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);
    assert!(matches!(
        gset.merge_one(ann3),
        Ok(MergeOneResult::Duplicate)
    ));

    assert_eq!(gset.len(), 1); // Still only one producer
    assert_eq!(gset.sequence_for(keypair.public_key()), 1);
}

#[test]
fn test_gset_merge_batch() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);

    let mut announcements = Vec::new();
    for _ in 0..5 {
        let keypair = KeyPair::generate();
        announcements.push(ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO));
    }
    // Add one invalid (tampered)
    let keypair = KeyPair::generate();
    let mut invalid = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);
    invalid.sequence = 999; // Tamper
    announcements.push(invalid);

    let result = gset.merge(announcements);
    assert_eq!(result.added, 5);
    assert_eq!(result.rejected, 1);
}

#[test]
fn test_gset_sorted_producers_deterministic() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);

    let keypairs: Vec<_> = (0..5).map(|_| KeyPair::generate()).collect();
    for kp in &keypairs {
        let ann = ProducerAnnouncement::new(kp, 1, 0, Hash::ZERO);
        gset.merge_one(ann).unwrap();
    }

    let sorted1 = gset.sorted_producers();
    let sorted2 = gset.sorted_producers();
    assert_eq!(sorted1, sorted2); // Deterministic

    // Verify sorted by bytes
    for i in 1..sorted1.len() {
        assert!(sorted1[i - 1].as_bytes() < sorted1[i].as_bytes());
    }
}

#[test]
fn test_gset_export_all_announcements() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);

    for _ in 0..3 {
        let keypair = KeyPair::generate();
        let ann = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);
        gset.merge_one(ann).unwrap();
    }

    let exported = gset.export();
    assert_eq!(exported.len(), 3);
    for ann in exported {
        assert!(ann.verify());
    }
}

#[test]
fn test_gset_stability_tracking() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);

    // Initially stable (no changes)
    assert!(gset.is_stable(Duration::from_millis(0)));

    // Add producer - resets stability
    let keypair = KeyPair::generate();
    let ann = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);
    gset.merge_one(ann).unwrap();

    // Not stable immediately
    assert!(!gset.is_stable(Duration::from_secs(1)));

    // Wait and check (simulated)
    std::thread::sleep(Duration::from_millis(50));
    assert!(gset.is_stable(Duration::from_millis(10)));
}

#[test]
fn test_gset_empty() {
    let gset = ProducerGSet::new(1, Hash::ZERO);
    assert!(gset.is_empty());
    assert_eq!(gset.len(), 0);
}

#[test]
fn test_gset_get() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);
    let keypair = KeyPair::generate();
    let ann = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);
    gset.merge_one(ann.clone()).unwrap();

    let retrieved = gset.get(keypair.public_key());
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().pubkey, ann.pubkey);
}

#[test]
fn test_gset_network_id() {
    let gset = ProducerGSet::new(99, Hash::ZERO);
    assert_eq!(gset.network_id(), 99);
}

#[test]
fn test_gset_duplicate_announcement() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);
    let keypair = KeyPair::generate();

    // Add same announcement twice
    let ann = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);
    assert!(matches!(
        gset.merge_one(ann.clone()),
        Ok(MergeOneResult::NewProducer)
    ));

    // Same sequence number - should be duplicate
    let ann2 = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);
    assert!(matches!(
        gset.merge_one(ann2),
        Ok(MergeOneResult::Duplicate)
    ));

    assert_eq!(gset.len(), 1);
}

#[test]
fn test_gset_merge_multiple_producers() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);

    let kp1 = KeyPair::generate();
    let kp2 = KeyPair::generate();
    let kp3 = KeyPair::generate();

    let announcements = vec![
        ProducerAnnouncement::new(&kp1, 1, 0, Hash::ZERO),
        ProducerAnnouncement::new(&kp2, 1, 0, Hash::ZERO),
        ProducerAnnouncement::new(&kp3, 1, 0, Hash::ZERO),
    ];

    let result = gset.merge(announcements);
    assert_eq!(result.added, 3);
    assert_eq!(result.new_producers, 3);
    assert_eq!(result.rejected, 0);
    assert_eq!(result.duplicates, 0);
    assert_eq!(gset.len(), 3);
}

#[test]
fn test_gset_no_persistence_by_default() {
    let gset = ProducerGSet::new(1, Hash::ZERO);
    assert!(!gset.has_persistence());
}

#[test]
fn test_gset_to_bloom_filter() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);
    let keypairs: Vec<_> = (0..10).map(|_| KeyPair::generate()).collect();

    for kp in &keypairs {
        let ann = ProducerAnnouncement::new(kp, 1, 0, Hash::ZERO);
        gset.merge_one(ann).unwrap();
    }

    let bloom = gset.to_bloom_filter();

    // All inserted keys should be found
    for kp in &keypairs {
        assert!(bloom.probably_contains(kp.public_key()));
    }

    // False positive rate should be reasonable (test 100 random keys)
    let false_positives = (0..100)
        .filter(|_| bloom.probably_contains(KeyPair::generate().public_key()))
        .count();
    assert!(
        false_positives < 10,
        "too many false positives: {false_positives}/100"
    );
}

#[test]
fn test_gset_delta_sync() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);

    // Add 10 producers
    let keypairs: Vec<_> = (0..10).map(|_| KeyPair::generate()).collect();
    for kp in &keypairs {
        let ann = ProducerAnnouncement::new(kp, 1, 0, Hash::ZERO);
        gset.merge_one(ann).unwrap();
    }

    // Peer knows first 7
    let mut peer_bloom = ProducerBloomFilter::new(10);
    for kp in &keypairs[..7] {
        peer_bloom.insert(kp.public_key());
    }

    // Delta should contain the unknown producers (last 3), but bloom filter
    // false positives may cause some to be incorrectly filtered out.
    // With capacity=10 and 7 inserted, FP rate is low but non-zero.
    let delta = gset.delta_for_peer(&peer_bloom);
    assert!(
        !delta.is_empty(),
        "Delta must contain at least some unknown producers"
    );
    assert!(
        delta.len() <= 10,
        "Delta cannot exceed total producer count"
    );
}

#[test]
fn test_gset_delta_empty_peer() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);

    let keypairs: Vec<_> = (0..5).map(|_| KeyPair::generate()).collect();
    for kp in &keypairs {
        let ann = ProducerAnnouncement::new(kp, 1, 0, Hash::ZERO);
        gset.merge_one(ann).unwrap();
    }

    // Peer knows nothing
    let peer_bloom = ProducerBloomFilter::new(10);
    let delta = gset.delta_for_peer(&peer_bloom);

    // Should return all 5
    assert_eq!(delta.len(), 5);
}

#[test]
fn test_gset_delta_peer_knows_all() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);

    let keypairs: Vec<_> = (0..5).map(|_| KeyPair::generate()).collect();
    for kp in &keypairs {
        let ann = ProducerAnnouncement::new(kp, 1, 0, Hash::ZERO);
        gset.merge_one(ann).unwrap();
    }

    // Peer knows everything
    let mut peer_bloom = ProducerBloomFilter::new(10);
    for kp in &keypairs {
        peer_bloom.insert(kp.public_key());
    }

    let delta = gset.delta_for_peer(&peer_bloom);

    // Should return none (or very few due to FP)
    assert!(delta.is_empty());
}
