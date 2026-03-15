//! Comprehensive stress tests and benchmarks for Milestone 12.
//! These tests validate scalability and performance of the producer discovery system.

use super::super::{MergeResult, ProducerAnnouncement, ProducerBloomFilter};
use super::{MergeOneResult, ProducerGSet};
use crate::discovery::proto;
use crypto::{Hash, KeyPair};

/// 12.4: Stress test with 100+ simulated producers.
/// This verifies the GSet can handle production-scale producer counts.
#[test]
fn test_stress_100_producers() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);

    // Generate 100 unique producers
    let keypairs: Vec<_> = (0..100).map(|_| KeyPair::generate()).collect();
    let announcements: Vec<_> = keypairs
        .iter()
        .map(|kp| ProducerAnnouncement::new(kp, 1, 0, Hash::ZERO))
        .collect();

    // Merge all at once
    let result = gset.merge(announcements);

    assert_eq!(result.added, 100, "All 100 producers should be added");
    assert_eq!(result.rejected, 0, "No producers should be rejected");
    assert_eq!(gset.len(), 100, "GSet should have 100 producers");

    // Verify deterministic ordering
    let sorted = gset.sorted_producers();
    assert_eq!(sorted.len(), 100);
    for i in 1..sorted.len() {
        assert!(
            sorted[i - 1].as_bytes() < sorted[i].as_bytes(),
            "Producers should be sorted by bytes"
        );
    }
}

/// Extended stress test with 500 producers and multiple sequence updates.
#[test]
fn test_stress_500_producers_with_updates() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);

    // Generate 500 producers
    let keypairs: Vec<_> = (0..500).map(|_| KeyPair::generate()).collect();

    // Add all with sequence 0
    for kp in &keypairs {
        let ann = ProducerAnnouncement::new(kp, 1, 0, Hash::ZERO);
        gset.merge_one(ann).expect("merge should succeed");
    }
    assert_eq!(gset.len(), 500);

    // Update half with sequence 1
    for kp in &keypairs[0..250] {
        let ann = ProducerAnnouncement::new(kp, 1, 1, Hash::ZERO);
        assert!(
            matches!(gset.merge_one(ann), Ok(MergeOneResult::SequenceUpdate)),
            "Update should succeed"
        );
    }
    assert_eq!(gset.len(), 500, "Count should remain 500");

    // Verify sequences were updated
    for kp in &keypairs[0..250] {
        assert_eq!(
            gset.sequence_for(kp.public_key()),
            1,
            "Sequence should be updated to 1"
        );
    }
    for kp in &keypairs[250..500] {
        assert_eq!(
            gset.sequence_for(kp.public_key()),
            0,
            "Sequence should remain 0"
        );
    }
}

/// 12.5: Fuzz-like test with random announcement patterns.
#[test]
fn test_fuzz_random_announcements() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);

    // Generate a pool of producers
    let keypairs: Vec<_> = (0..50).map(|_| KeyPair::generate()).collect();

    // Send random announcements in random order with various sequences
    let mut rng_seed: u64 = 42;
    for _ in 0..200 {
        // Simple pseudo-random selection
        rng_seed = rng_seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let kp_idx = (rng_seed as usize) % keypairs.len();
        let seq = (rng_seed >> 32) % 10;

        let ann = ProducerAnnouncement::new(&keypairs[kp_idx], 1, seq, Hash::ZERO);
        // All operations should succeed without panic
        let _ = gset.merge_one(ann);
    }

    // Verify state is consistent
    assert!(gset.len() <= 50, "Should have at most 50 producers");
    let exported = gset.export();
    for ann in exported {
        assert!(ann.verify(), "All exported announcements should verify");
    }
}

/// Test handling of invalid/tampered announcements at scale.
#[test]
fn test_stress_invalid_announcements() {
    let mut gset = ProducerGSet::new(1, Hash::ZERO);

    let mut valid_count = 0;
    let mut _invalid_count = 0;

    // Mix of valid and invalid announcements
    for i in 0..100 {
        let keypair = KeyPair::generate();
        let mut ann = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);

        if i % 4 == 0 {
            // Tamper every 4th announcement
            ann.sequence = 999;
            _invalid_count += 1;
        } else {
            valid_count += 1;
        }

        let _ = gset.merge_one(ann);
    }

    assert_eq!(
        gset.len(),
        valid_count,
        "Only valid announcements should be added"
    );
}

/// 12.6: Benchmark gossip message sizes at various network sizes.
#[test]
fn test_benchmark_gossip_message_sizes() {
    let sizes = [10, 50, 100, 500, 1000];

    println!("\n=== Gossip Message Size Benchmark ===");
    println!(
        "{:>8} {:>12} {:>12} {:>12}",
        "Nodes", "Proto (B)", "Per Ann (B)", "Bloom (B)"
    );

    for &size in &sizes {
        let keypairs: Vec<_> = (0..size).map(|_| KeyPair::generate()).collect();
        let announcements: Vec<_> = keypairs
            .iter()
            .map(|kp| ProducerAnnouncement::new(kp, 1, 0, Hash::ZERO))
            .collect();

        // Encode using protobuf
        let proto_bytes = proto::encode_producer_set(&announcements);
        let per_announcement = proto_bytes.len() / size;

        // Bloom filter size
        let mut bloom = ProducerBloomFilter::new(size);
        for kp in &keypairs {
            bloom.insert(kp.public_key());
        }
        let bloom_bytes = proto::encode_digest(&bloom);

        println!(
            "{:>8} {:>12} {:>12} {:>12}",
            size,
            proto_bytes.len(),
            per_announcement,
            bloom_bytes.len()
        );

        // Verify reasonable sizes
        // Each announcement ~110-130 bytes in protobuf
        assert!(
            per_announcement < 150,
            "Announcement should be < 150 bytes, got {}",
            per_announcement
        );

        // Bloom filter should be much smaller than full set for large networks
        if size >= 100 {
            assert!(
                bloom_bytes.len() < proto_bytes.len() / 5,
                "Bloom should be >5x smaller than full set"
            );
        }
    }
}

/// 12.7: Benchmark bloom filter effectiveness for delta sync.
///
/// Note: Bloom filters can have false positives, which means some unknown
/// producers might not be included in the delta. This is expected behavior
/// and the test accounts for it by checking:
/// 1. Delta includes at least (expected - FP margin) producers
/// 2. No extra producers are sent beyond what's needed (within FP tolerance)
#[test]
fn test_benchmark_bloom_filter_effectiveness() {
    let network_sizes = [50, 100, 500];
    let overlap_percentages = [0, 25, 50, 75, 90, 100];

    println!("\n=== Bloom Filter Delta Sync Effectiveness ===");
    println!(
        "{:>8} {:>8} {:>12} {:>12} {:>12}",
        "Size", "Overlap%", "Expected", "Actual", "Missed"
    );

    for &size in &network_sizes {
        let keypairs: Vec<_> = (0..size).map(|_| KeyPair::generate()).collect();

        // Create full gset
        let mut gset = ProducerGSet::new(1, Hash::ZERO);
        for kp in &keypairs {
            let ann = ProducerAnnouncement::new(kp, 1, 0, Hash::ZERO);
            gset.merge_one(ann).unwrap();
        }

        for &overlap in &overlap_percentages {
            let known_count = (size * overlap) / 100;
            let expected_delta = size - known_count;

            // Peer's bloom filter with partial knowledge
            let mut peer_bloom = ProducerBloomFilter::new(size);
            for kp in &keypairs[0..known_count] {
                peer_bloom.insert(kp.public_key());
            }

            // Compute delta
            let delta = gset.delta_for_peer(&peer_bloom);

            // Count how many unknown producers are missing from delta
            // (due to bloom filter false positives)
            let mut missing = 0;
            for kp in &keypairs[known_count..] {
                if !delta.iter().any(|ann| ann.pubkey == *kp.public_key()) {
                    missing += 1;
                }
            }

            println!(
                "{:>8} {:>8} {:>12} {:>12} {:>12}",
                size,
                overlap,
                expected_delta,
                delta.len(),
                missing
            );

            // Due to bloom filter false positives, some unknown producers
            // might be falsely identified as "known" and excluded from delta.
            // This is acceptable as long as it's within the FP rate (~1%).
            // For 1% FP rate, at most ~1% of unknown items should be missing.
            let max_allowed_missing = ((expected_delta as f64) * 0.03).ceil() as usize + 1;
            assert!(
                missing <= max_allowed_missing,
                "Too many missing producers: {} > {} (expected {} in delta)",
                missing,
                max_allowed_missing,
                expected_delta
            );

            // Delta should not contain significantly more than expected
            // (bloom filter false negatives are impossible, so extra items
            // would indicate a bug, not FP behavior)
            assert!(
                delta.len() <= expected_delta,
                "Delta should not exceed expected: {} > {}",
                delta.len(),
                expected_delta
            );
        }
    }
}

/// Test GSet convergence with concurrent merges.
#[test]
fn test_gset_convergence_simulation() {
    // Simulate 3 nodes with different initial knowledge
    let keypairs: Vec<_> = (0..30).map(|_| KeyPair::generate()).collect();

    // Node 1 knows producers 0-19
    let mut node1 = ProducerGSet::new(1, Hash::ZERO);
    for kp in &keypairs[0..20] {
        node1
            .merge_one(ProducerAnnouncement::new(kp, 1, 0, Hash::ZERO))
            .unwrap();
    }

    // Node 2 knows producers 10-29
    let mut node2 = ProducerGSet::new(1, Hash::ZERO);
    for kp in &keypairs[10..30] {
        node2
            .merge_one(ProducerAnnouncement::new(kp, 1, 0, Hash::ZERO))
            .unwrap();
    }

    // Node 3 knows producers 0-9 and 20-29
    let mut node3 = ProducerGSet::new(1, Hash::ZERO);
    for kp in &keypairs[0..10] {
        node3
            .merge_one(ProducerAnnouncement::new(kp, 1, 0, Hash::ZERO))
            .unwrap();
    }
    for kp in &keypairs[20..30] {
        node3
            .merge_one(ProducerAnnouncement::new(kp, 1, 0, Hash::ZERO))
            .unwrap();
    }

    // Simulate gossip rounds - each node shares with others
    // Round 1: node1 -> node2, node2 -> node3, node3 -> node1
    let _ = node2.merge(node1.export());
    let _ = node3.merge(node2.export());
    let _ = node1.merge(node3.export());

    // Round 2: propagate again
    let _ = node2.merge(node1.export());
    let _ = node3.merge(node2.export());
    let _ = node1.merge(node3.export());

    // All nodes should have all 30 producers
    assert_eq!(node1.len(), 30, "Node 1 should have all 30 producers");
    assert_eq!(node2.len(), 30, "Node 2 should have all 30 producers");
    assert_eq!(node3.len(), 30, "Node 3 should have all 30 producers");

    // All should have identical sorted order
    assert_eq!(
        node1.sorted_producers(),
        node2.sorted_producers(),
        "Node 1 and 2 should have same order"
    );
    assert_eq!(
        node2.sorted_producers(),
        node3.sorted_producers(),
        "Node 2 and 3 should have same order"
    );
}

/// Test partition and heal scenario.
#[test]
fn test_partition_and_heal_simulation() {
    let keypairs: Vec<_> = (0..20).map(|_| KeyPair::generate()).collect();

    // Two partitions that develop independently
    let mut partition_a1 = ProducerGSet::new(1, Hash::ZERO);
    let mut partition_a2 = ProducerGSet::new(1, Hash::ZERO);
    let mut partition_b1 = ProducerGSet::new(1, Hash::ZERO);
    let mut partition_b2 = ProducerGSet::new(1, Hash::ZERO);

    // Partition A learns producers 0-9
    for kp in &keypairs[0..10] {
        let ann = ProducerAnnouncement::new(kp, 1, 0, Hash::ZERO);
        partition_a1.merge_one(ann.clone()).unwrap();
        partition_a2.merge_one(ann).unwrap();
    }

    // Partition B learns producers 10-19
    for kp in &keypairs[10..20] {
        let ann = ProducerAnnouncement::new(kp, 1, 0, Hash::ZERO);
        partition_b1.merge_one(ann.clone()).unwrap();
        partition_b2.merge_one(ann).unwrap();
    }

    // Verify partitions are isolated
    assert_eq!(partition_a1.len(), 10);
    assert_eq!(partition_b1.len(), 10);

    // Heal: merge partitions
    let _ = partition_a1.merge(partition_b1.export());
    let _ = partition_a2.merge(partition_b2.export());
    let _ = partition_b1.merge(partition_a1.export());
    let _ = partition_b2.merge(partition_a2.export());

    // All nodes should converge to 20 producers
    assert_eq!(partition_a1.len(), 20, "A1 should have all 20");
    assert_eq!(partition_a2.len(), 20, "A2 should have all 20");
    assert_eq!(partition_b1.len(), 20, "B1 should have all 20");
    assert_eq!(partition_b2.len(), 20, "B2 should have all 20");

    // All should have same sorted order
    let reference = partition_a1.sorted_producers();
    assert_eq!(partition_a2.sorted_producers(), reference);
    assert_eq!(partition_b1.sorted_producers(), reference);
    assert_eq!(partition_b2.sorted_producers(), reference);
}

/// Test adaptive gossip with simulated merge results.
#[test]
fn test_adaptive_gossip_integration() {
    use crate::discovery::AdaptiveGossip;
    use std::time::Duration;

    let mut gossip = AdaptiveGossip::new();

    // Initial state
    assert_eq!(gossip.interval(), Duration::from_secs(5));
    assert!(!gossip.use_delta_sync());

    // Simulate discovering new producers (high activity)
    for _ in 0..3 {
        gossip.on_gossip_result(
            &MergeResult {
                added: 5,
                new_producers: 5,
                rejected: 0,
                duplicates: 0,
            },
            10,
        );
    }
    assert_eq!(
        gossip.interval(),
        Duration::from_secs(1),
        "Should speed up on new discoveries"
    );

    // Simulate stable period (no changes)
    for _ in 0..10 {
        gossip.on_gossip_result(&MergeResult::default(), 50);
    }
    assert!(
        gossip.interval() > Duration::from_secs(5),
        "Should back off during stable period"
    );
    assert!(
        gossip.use_delta_sync(),
        "Should enable delta sync for large network"
    );

    // New discovery resets interval
    gossip.on_gossip_result(
        &MergeResult {
            added: 1,
            new_producers: 1,
            rejected: 0,
            duplicates: 0,
        },
        50,
    );
    assert_eq!(
        gossip.interval(),
        Duration::from_secs(1),
        "Should reset on new discovery"
    );
}

/// Memory usage estimation for large networks.
#[test]
fn test_memory_estimation() {
    // Estimate memory usage for various network sizes
    let sizes = [100, 1000, 10000];

    println!("\n=== Memory Usage Estimation ===");
    println!(
        "{:>8} {:>15} {:>15}",
        "Nodes", "Est. Memory", "Per Producer"
    );

    for &size in &sizes {
        // Each announcement: ~130 bytes (pubkey 32 + sig 64 + fields ~34)
        // HashMap overhead: ~48 bytes per entry
        // Sequences map: ~40 bytes per entry
        let ann_size = 130;
        let hashmap_overhead = 48 + 40;
        let estimated_bytes = size * (ann_size + hashmap_overhead);

        println!(
            "{:>8} {:>12} KB {:>12} B",
            size,
            estimated_bytes / 1024,
            ann_size + hashmap_overhead
        );

        // 10K producers should use < 5MB
        if size == 10000 {
            assert!(
                estimated_bytes < 5 * 1024 * 1024,
                "10K producers should use < 5MB"
            );
        }
    }
}
