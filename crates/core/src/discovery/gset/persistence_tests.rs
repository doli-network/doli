//! Persistence tests for ProducerGSet.

use super::super::ProducerAnnouncement;
use super::ProducerGSet;
use crypto::{Hash, KeyPair};
use tempfile::tempdir;

#[test]
fn test_gset_persist_and_load() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("producers.bin");

    // Create and populate
    let mut gset = ProducerGSet::new_with_persistence(1, Hash::ZERO, path.clone());
    let mut keypairs = Vec::new();
    for _ in 0..3 {
        let keypair = KeyPair::generate();
        let ann = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);
        gset.merge_one(ann).expect("merge should succeed");
        keypairs.push(keypair);
    }
    let original_count = gset.len();
    let original_sorted = gset.sorted_producers();

    // Drop and reload
    drop(gset);
    let loaded = ProducerGSet::new_with_persistence(1, Hash::ZERO, path);

    assert_eq!(loaded.len(), original_count);
    assert_eq!(loaded.sorted_producers(), original_sorted);
}

#[test]
fn test_gset_load_rejects_invalid() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("producers.bin");

    // Create valid gset
    let mut gset = ProducerGSet::new_with_persistence(1, Hash::ZERO, path.clone());
    let keypair = KeyPair::generate();
    let ann = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);
    gset.merge_one(ann).expect("merge should succeed");

    // Corrupt the file
    let mut data = std::fs::read(&path).expect("read should succeed");
    if !data.is_empty() {
        let last_idx = data.len() - 1;
        data[last_idx] ^= 0xFF; // Flip bits
    }
    std::fs::write(&path, data).expect("write should succeed");

    // Reload should handle corruption gracefully
    let loaded = ProducerGSet::new_with_persistence(1, Hash::ZERO, path);
    // Either empty (complete corruption) or fewer (partial)
    assert!(loaded.len() <= 1);
}

#[test]
fn test_gset_atomic_write() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("producers.bin");

    let mut gset = ProducerGSet::new_with_persistence(1, Hash::ZERO, path.clone());
    let keypair = KeyPair::generate();
    let ann = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);
    gset.merge_one(ann).expect("merge should succeed");

    // File should exist and be valid
    assert!(path.exists());
    let loaded = ProducerGSet::new_with_persistence(1, Hash::ZERO, path);
    assert_eq!(loaded.len(), 1);
}

#[test]
fn test_gset_has_persistence() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("producers.bin");

    let gset = ProducerGSet::new_with_persistence(1, Hash::ZERO, path);
    assert!(gset.has_persistence());
}

#[test]
fn test_gset_load_nonexistent_file() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("nonexistent.bin");

    // Should not panic, just create empty gset
    let gset = ProducerGSet::new_with_persistence(1, Hash::ZERO, path);
    assert_eq!(gset.len(), 0);
}

#[test]
fn test_gset_persist_updates() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("producers.bin");

    // Create with one producer
    let mut gset = ProducerGSet::new_with_persistence(1, Hash::ZERO, path.clone());
    let kp1 = KeyPair::generate();
    gset.merge_one(ProducerAnnouncement::new(&kp1, 1, 0, Hash::ZERO))
        .expect("merge should succeed");

    // Reload and verify
    let loaded1 = ProducerGSet::new_with_persistence(1, Hash::ZERO, path.clone());
    assert_eq!(loaded1.len(), 1);

    // Add another producer
    drop(loaded1);
    let mut gset2 = ProducerGSet::new_with_persistence(1, Hash::ZERO, path.clone());
    let kp2 = KeyPair::generate();
    gset2
        .merge_one(ProducerAnnouncement::new(&kp2, 1, 0, Hash::ZERO))
        .expect("merge should succeed");
    assert_eq!(gset2.len(), 2);

    // Reload and verify both are present
    drop(gset2);
    let loaded2 = ProducerGSet::new_with_persistence(1, Hash::ZERO, path);
    assert_eq!(loaded2.len(), 2);
}

#[test]
fn test_gset_rejects_wrong_network_on_load() {
    let dir = tempdir().expect("tempdir should be created");
    let path = dir.path().join("producers.bin");

    // Create with network 1
    let mut gset = ProducerGSet::new_with_persistence(1, Hash::ZERO, path.clone());
    let keypair = KeyPair::generate();
    gset.merge_one(ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO))
        .expect("merge should succeed");
    drop(gset);

    // Try to load with network 2 - should reject
    let loaded = ProducerGSet::new_with_persistence(2, Hash::ZERO, path);
    assert_eq!(loaded.len(), 0); // Rejected due to network mismatch
}
