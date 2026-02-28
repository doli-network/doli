//! Test that presence_root is included in block hash (DOLI_NETWORK_BUG.md fix)
//!
//! This test verifies that the security fix for presence manipulation works:
//! - presence_root is now part of the block hash
//! - Changing presence data changes the block hash
//! - Weighted rewards cannot be manipulated post-hoc

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::block::BlockHeader;
use doli_core::presence::PresenceCommitmentV2;
use doli_core::types::Amount;
use vdf::{VdfOutput, VdfProof};

/// Create a test block header with given presence_root
fn create_header_with_presence(slot: u32, presence_root: Hash) -> BlockHeader {
    BlockHeader {
        version: 1,
        prev_hash: Hash::ZERO,
        merkle_root: Hash::ZERO,
        presence_root,
        timestamp: 1700000000 + (slot as u64 * 10),
        slot,
        producer: PublicKey::from_bytes([1u8; 32]),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
    }
}

#[test]
fn test_presence_root_affects_block_hash() {
    // Create two headers that differ ONLY in presence_root
    let presence_root_a = crypto::hash::hash(b"presence_data_A");
    let presence_root_b = crypto::hash::hash(b"presence_data_B");

    let header_a = create_header_with_presence(100, presence_root_a);
    let header_b = create_header_with_presence(100, presence_root_b);

    // The block hashes MUST be different
    let hash_a = header_a.hash();
    let hash_b = header_b.hash();

    assert_ne!(
        hash_a, hash_b,
        "SECURITY BUG: Different presence_root should produce different block hash!"
    );

    println!("✓ presence_root affects block hash");
    println!("  Header A hash: {}", hash_a);
    println!("  Header B hash: {}", hash_b);
}

#[test]
fn test_presence_root_zero_vs_nonzero() {
    // Legacy blocks have presence_root = ZERO
    // V2 blocks have presence_root = commitment_hash
    let header_legacy = create_header_with_presence(100, Hash::ZERO);

    let commitment = PresenceCommitmentV2::new(
        crypto::hash::hash(b"heartbeats"),
        1_000_000_000, // 10 DOLI total weight
    );
    let header_v2 = create_header_with_presence(100, commitment.commitment_hash());

    let hash_legacy = header_legacy.hash();
    let hash_v2 = header_v2.hash();

    assert_ne!(
        hash_legacy, hash_v2,
        "Legacy (ZERO) and V2 presence_root should produce different hashes"
    );

    println!("✓ Legacy vs V2 presence produces different hashes");
}

#[test]
fn test_total_weight_manipulation_detected() {
    // Scenario from DOLI_NETWORK_BUG.md:
    // Producer 5 has 10 bonds, others have 1 bond each
    // If total_weight could be manipulated, rewards would be wrong

    // Original presence: 4 producers with weights [1, 1, 1, 10] = 13 total
    let original_commitment = PresenceCommitmentV2::new(
        crypto::hash::hash(b"heartbeats_slot_100"),
        13_000_000_000, // 13 DOLI total weight (correct)
    );

    // Attacker tries to claim total_weight was lower to inflate their share
    let manipulated_commitment = PresenceCommitmentV2::new(
        crypto::hash::hash(b"heartbeats_slot_100"), // Same heartbeats root
        4_000_000_000,                              // 4 DOLI total weight (manipulated!)
    );

    // Create headers with each commitment
    let header_original = create_header_with_presence(100, original_commitment.commitment_hash());
    let header_manipulated =
        create_header_with_presence(100, manipulated_commitment.commitment_hash());

    // The hashes MUST be different - manipulation is detected!
    assert_ne!(
        header_original.hash(),
        header_manipulated.hash(),
        "SECURITY BUG: total_weight manipulation should change block hash!"
    );

    println!("✓ total_weight manipulation changes block hash (attack detected)");
}

#[test]
fn test_weighted_reward_calculation() {
    // Verify the weighted reward formula works correctly
    // From DOLI_NETWORK_BUG.md: Producer 5 with 10 bonds should earn ~10x

    let block_reward: Amount = 1_000_000_000; // 10 DOLI per block

    // Scenario: 4 producers present
    // Producer 1-4: 1 bond each (weight = 1)
    // Producer 5: 10 bonds (weight = 10)
    // Total weight = 1 + 1 + 1 + 1 + 10 = 14

    let total_weight: u128 = 14;

    // Each producer's reward = block_reward * their_weight / total_weight
    let reward_1bond = (block_reward as u128) / total_weight;
    let reward_10bonds = (block_reward as u128 * 10) / total_weight;

    println!("Block reward: {} units", block_reward);
    println!("Total weight: {}", total_weight);
    println!(
        "1-bond producer reward: {} units ({:.2}%)",
        reward_1bond,
        (reward_1bond as f64 / block_reward as f64) * 100.0
    );
    println!(
        "10-bond producer reward: {} units ({:.2}%)",
        reward_10bonds,
        (reward_10bonds as f64 / block_reward as f64) * 100.0
    );

    // 10-bond producer should get ~10x more
    let ratio = reward_10bonds as f64 / reward_1bond as f64;
    assert!(
        (ratio - 10.0).abs() < 0.01,
        "10-bond producer should earn ~10x more. Got ratio: {:.2}",
        ratio
    );

    // Verify total distributed equals block reward (approximately, due to rounding)
    let total_distributed = reward_1bond * 4 + reward_10bonds;
    assert!(
        total_distributed <= block_reward as u128,
        "Total distributed should not exceed block reward"
    );

    println!("✓ Weighted reward calculation is correct (10x ratio verified)");
}

#[test]
fn test_presence_commitment_v2_determinism() {
    // Same inputs should always produce same commitment hash
    let heartbeats_root = crypto::hash::hash(b"test_heartbeats");
    let total_weight = 5_000_000_000u64;

    let commitment1 = PresenceCommitmentV2::new(heartbeats_root, total_weight);
    let commitment2 = PresenceCommitmentV2::new(heartbeats_root, total_weight);

    assert_eq!(
        commitment1.commitment_hash(),
        commitment2.commitment_hash(),
        "Same inputs should produce same commitment hash"
    );

    println!("✓ PresenceCommitmentV2 is deterministic");
}

#[test]
fn test_block_hash_includes_all_presence_components() {
    // Verify that changing ANY component of presence changes the block hash

    let base_heartbeats = crypto::hash::hash(b"heartbeats");
    let base_weight = 10_000_000_000u64;

    let commitment_base = PresenceCommitmentV2::new(base_heartbeats, base_weight);
    let header_base = create_header_with_presence(100, commitment_base.commitment_hash());
    let hash_base = header_base.hash();

    // Change only heartbeats_root
    let different_heartbeats = crypto::hash::hash(b"different_heartbeats");
    let commitment_diff_heartbeats = PresenceCommitmentV2::new(different_heartbeats, base_weight);
    let header_diff_heartbeats =
        create_header_with_presence(100, commitment_diff_heartbeats.commitment_hash());

    assert_ne!(
        hash_base,
        header_diff_heartbeats.hash(),
        "Different heartbeats_root should produce different block hash"
    );

    // Change only total_weight
    let different_weight = 20_000_000_000u64;
    let commitment_diff_weight = PresenceCommitmentV2::new(base_heartbeats, different_weight);
    let header_diff_weight =
        create_header_with_presence(100, commitment_diff_weight.commitment_hash());

    assert_ne!(
        hash_base,
        header_diff_weight.hash(),
        "Different total_weight should produce different block hash"
    );

    println!("✓ All presence components affect block hash");
}

/// Integration test simulating the DOLI_NETWORK_BUG scenario
#[test]
fn test_doli_network_bug_scenario() {
    println!("\n=== DOLI_NETWORK_BUG.md Scenario Test ===\n");

    // Simulate epoch with 5 producers
    // Producers 1-4: 1 bond each
    // Producer 5: 10 bonds

    let producers: Vec<(KeyPair, u64)> = vec![
        (KeyPair::generate(), 1),  // Producer 1
        (KeyPair::generate(), 1),  // Producer 2
        (KeyPair::generate(), 1),  // Producer 3
        (KeyPair::generate(), 1),  // Producer 4
        (KeyPair::generate(), 10), // Producer 5 (10 bonds!)
    ];

    let total_weight: u64 = producers.iter().map(|(_, w)| w).sum();
    println!("Total weight: {} (4×1 + 1×10 = 14)", total_weight);

    // Create presence commitment for a block where all 5 are present
    let heartbeats_root = crypto::hash::hash(b"all_5_producers_heartbeats");
    let commitment = PresenceCommitmentV2::new(heartbeats_root, total_weight);

    // Create block header with this presence
    let header = create_header_with_presence(100, commitment.commitment_hash());
    let block_hash = header.hash();

    println!("Block hash: {}", block_hash);
    println!("Presence root: {}", commitment.commitment_hash());

    // Try to manipulate: attacker claims total_weight was 4 (excluding Producer 5)
    let fake_commitment = PresenceCommitmentV2::new(heartbeats_root, 4);
    let fake_header = create_header_with_presence(100, fake_commitment.commitment_hash());
    let fake_hash = fake_header.hash();

    println!("\nAttack attempt: claim total_weight=4 instead of 14");
    println!("Fake block hash: {}", fake_hash);

    // The attack should be detected because hashes don't match
    assert_ne!(
        block_hash, fake_hash,
        "CRITICAL: Manipulation attack should be detected!"
    );

    println!("\n✓ Attack detected! Block hash mismatch.");
    println!("  Original: {}", block_hash);
    println!("  Fake:     {}", fake_hash);

    // Calculate what rewards SHOULD be for Producer 5
    let block_reward: u128 = 1_000_000_000; // 10 DOLI
    let producer5_reward = (block_reward * 10) / total_weight as u128;
    let producer1_reward = block_reward / total_weight as u128;

    println!("\nCorrect reward distribution:");
    println!(
        "  Producer 1-4 (1 bond): {:.4} DOLI each",
        producer1_reward as f64 / 100_000_000.0
    );
    println!(
        "  Producer 5 (10 bonds): {:.4} DOLI",
        producer5_reward as f64 / 100_000_000.0
    );
    println!(
        "  Ratio: {:.1}x",
        producer5_reward as f64 / producer1_reward as f64
    );

    println!("\n=== DOLI_NETWORK_BUG.md Fix Verified ===\n");
}
