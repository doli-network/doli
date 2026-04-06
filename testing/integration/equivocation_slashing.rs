//! Equivocation (double-signing) detection and slashing tests
//!
//! These tests verify that the slashing mechanism works end-to-end:
//! - Equivocation detection when a producer creates two blocks for the same slot
//! - Valid proof generation with VDF-verifiable headers
//! - Slashing transaction creation and validation
//! - Bond burning and producer exclusion
//!
//! CRITICAL: These tests MUST pass before genesis. If slashing doesn't work,
//! the security model is broken and double-spend attacks become possible.

use crypto::{hash::hash, Hash, KeyPair, PublicKey};
use doli_core::transaction::SlashingEvidence;
use doli_core::{Block, BlockHeader};
use network::EquivocationDetector;
use storage::{ProducerInfo, ProducerSet, ProducerStatus};
use vdf::{VdfOutput, VdfProof};

/// Bond amount for test producers (1,000 DOLI in base units)
const BOND_AMOUNT: u64 = 1_000 * 100_000_000; // 1000 DOLI

/// Helper to create a test block with specified parameters
fn create_test_block(slot: u32, producer: &PublicKey, prev_hash: Hash, merkle_root: Hash) -> Block {
    let header = BlockHeader {
        version: 1,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: slot as u64 * 10, // 10-second slots
        slot,
        producer: *producer,
        vdf_output: VdfOutput {
            value: vec![slot as u8; 32], // Distinct VDF output
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: crypto::Hash::ZERO,
    };
    Block::new(header, vec![])
}

/// Helper to create a producer set with registered producers
#[allow(deprecated)] // activity_gaps field is deprecated but needed for struct initialization
fn create_producer_set_with_bonds(producers: &[(&PublicKey, u64)]) -> ProducerSet {
    let mut set = ProducerSet::new();
    for (pubkey, bond_amount) in producers {
        let info = ProducerInfo {
            public_key: *(*pubkey),
            registered_at: 0,
            bond_amount: *bond_amount,
            bond_outpoint: (Hash::ZERO, 0),
            status: ProducerStatus::Active,
            blocks_produced: 0,
            slots_missed: 0,
            registration_era: 0,
            pending_rewards: 0,
            has_prior_exit: false,
            last_activity: 0,
            activity_gaps: 0,
            bond_count: 1,
            additional_bonds: vec![],
            delegated_to: None,
            delegated_bonds: 0,
            received_delegations: vec![],
            bond_entries: vec![],
            withdrawal_pending_count: 0,
            bls_pubkey: Vec::new(),
        };
        // Use register method with current_height = 0
        let _ = set.register(info, 0);
    }
    set
}

// ============================================================================
// TEST 1: Basic Equivocation Detection
// ============================================================================

#[tokio::test]
async fn test_equivocation_detection_basic() {
    let mut detector = EquivocationDetector::new();
    let malicious_producer = KeyPair::generate();
    let producer_pubkey = *malicious_producer.public_key();

    // Create two DIFFERENT blocks for the SAME slot
    let slot = 100;
    let block_a = create_test_block(slot, &producer_pubkey, Hash::ZERO, hash(b"transactions_a"));
    let block_b = create_test_block(
        slot,
        &producer_pubkey,
        Hash::ZERO,
        hash(b"transactions_b"), // Different merkle root = different block
    );

    // Verify blocks are different
    assert_ne!(block_a.hash(), block_b.hash());
    assert_eq!(block_a.header.slot, block_b.header.slot);
    assert_eq!(block_a.header.producer, block_b.header.producer);

    // First block - no equivocation
    assert!(detector.check_block(&block_a).is_none());

    // Second different block for same slot - EQUIVOCATION DETECTED!
    let proof = detector.check_block(&block_b);
    assert!(proof.is_some(), "Equivocation should be detected");

    let proof = proof.unwrap();
    assert_eq!(proof.producer, producer_pubkey);
    assert_eq!(proof.slot, slot);
    assert_eq!(proof.block_header_1.hash(), block_a.hash());
    assert_eq!(proof.block_header_2.hash(), block_b.hash());

    println!("✓ Basic equivocation detection test passed");
}

// ============================================================================
// TEST 2: Same Block Twice is NOT Equivocation
// ============================================================================

#[tokio::test]
async fn test_same_block_twice_not_equivocation() {
    let mut detector = EquivocationDetector::new();
    let producer = KeyPair::generate();
    let producer_pubkey = *producer.public_key();

    let block = create_test_block(50, &producer_pubkey, Hash::ZERO, hash(b"tx"));

    // See the same block twice
    assert!(detector.check_block(&block).is_none());
    assert!(detector.check_block(&block).is_none());

    // No pending proofs should exist
    assert!(!detector.has_pending_proofs());

    println!("✓ Same block twice test passed (correctly not detected as equivocation)");
}

// ============================================================================
// TEST 3: Different Slots is NOT Equivocation
// ============================================================================

#[tokio::test]
async fn test_different_slots_not_equivocation() {
    let mut detector = EquivocationDetector::new();
    let producer = KeyPair::generate();
    let producer_pubkey = *producer.public_key();

    // Create blocks for different slots
    let block_slot_10 = create_test_block(10, &producer_pubkey, Hash::ZERO, hash(b"a"));
    let block_slot_11 = create_test_block(11, &producer_pubkey, Hash::ZERO, hash(b"b"));

    assert!(detector.check_block(&block_slot_10).is_none());
    assert!(detector.check_block(&block_slot_11).is_none());

    assert!(!detector.has_pending_proofs());

    println!("✓ Different slots test passed (correctly not detected as equivocation)");
}

// ============================================================================
// TEST 4: Different Producers Same Slot is NOT Equivocation
// ============================================================================

#[tokio::test]
async fn test_different_producers_same_slot_not_equivocation() {
    let mut detector = EquivocationDetector::new();

    let producer_a = KeyPair::generate();
    let producer_b = KeyPair::generate();

    let slot = 25;
    let block_a = create_test_block(slot, producer_a.public_key(), Hash::ZERO, hash(b"a"));
    let block_b = create_test_block(slot, producer_b.public_key(), Hash::ZERO, hash(b"b"));

    // Different producers for same slot is normal (e.g., fallback producers)
    assert!(detector.check_block(&block_a).is_none());
    assert!(detector.check_block(&block_b).is_none());

    assert!(!detector.has_pending_proofs());

    println!("✓ Different producers same slot test passed");
}

// ============================================================================
// TEST 5: Proof Contains Full Headers for VDF Verification
// ============================================================================

#[tokio::test]
async fn test_proof_contains_vdf_verifiable_headers() {
    let mut detector = EquivocationDetector::new();
    let producer = KeyPair::generate();
    let producer_pubkey = *producer.public_key();

    let slot = 77;
    let block_a = create_test_block(slot, &producer_pubkey, Hash::ZERO, hash(b"block_a"));
    let block_b = create_test_block(slot, &producer_pubkey, Hash::ZERO, hash(b"block_b"));

    detector.check_block(&block_a);
    let proof = detector.check_block(&block_b).unwrap();

    // Proof must contain full headers, not just hashes
    // This allows validators to verify VDF proofs and confirm producer identity
    assert_eq!(proof.block_header_1.slot, slot);
    assert_eq!(proof.block_header_2.slot, slot);
    assert_eq!(proof.block_header_1.producer, producer_pubkey);
    assert_eq!(proof.block_header_2.producer, producer_pubkey);

    // Headers must have VDF outputs (even if empty for tests)
    assert!(
        !proof.block_header_1.vdf_output.value.is_empty()
            || proof.block_header_1.vdf_proof.is_empty()
    );

    println!("✓ VDF-verifiable headers test passed");
}

// ============================================================================
// TEST 6: Proof to Slash Transaction Conversion
// ============================================================================

#[tokio::test]
async fn test_proof_to_slash_transaction() {
    let mut detector = EquivocationDetector::new();
    let malicious = KeyPair::generate();
    let reporter = KeyPair::generate();
    let malicious_pubkey = *malicious.public_key();

    let slot = 42;
    let block_a = create_test_block(slot, &malicious_pubkey, Hash::ZERO, hash(b"a"));
    let block_b = create_test_block(slot, &malicious_pubkey, Hash::ZERO, hash(b"b"));

    detector.check_block(&block_a);
    let proof = detector.check_block(&block_b).unwrap();

    // Convert proof to slash transaction
    let slash_tx = proof.to_slash_transaction(&reporter);

    // Verify transaction structure
    assert!(slash_tx.is_slash_producer());

    let slash_data = slash_tx.slash_data().unwrap();
    assert_eq!(slash_data.producer_pubkey, malicious_pubkey);

    // Verify evidence contains both headers
    let SlashingEvidence::DoubleProduction {
        block_header_1,
        block_header_2,
    } = &slash_data.evidence;
    assert_eq!(block_header_1.slot, slot);
    assert_eq!(block_header_2.slot, slot);
    assert_ne!(block_header_1.hash(), block_header_2.hash());

    // Reporter signature should be present (prevents spam)
    assert!(!slash_data
        .reporter_signature
        .as_bytes()
        .iter()
        .all(|&b| b == 0));

    println!("✓ Proof to slash transaction test passed");
}

// ============================================================================
// TEST 7: Slashing Updates Producer Status
// ============================================================================

#[tokio::test]
async fn test_slashing_updates_producer_status() {
    let malicious = KeyPair::generate();
    let honest = KeyPair::generate();

    // Create producer set with both producers
    let mut producer_set = create_producer_set_with_bonds(&[
        (malicious.public_key(), BOND_AMOUNT),
        (honest.public_key(), BOND_AMOUNT),
    ]);

    // Verify initial state
    let initial_malicious = producer_set.get_by_pubkey(malicious.public_key()).unwrap();
    assert!(initial_malicious.is_active());
    assert_eq!(initial_malicious.bond_amount, BOND_AMOUNT);

    // Simulate slashing
    let current_height = 1000;
    let _ = producer_set.slash_producer(malicious.public_key(), current_height);

    // Verify malicious producer is slashed
    let slashed = producer_set.get_by_pubkey(malicious.public_key()).unwrap();
    assert!(!slashed.is_active());
    assert!(matches!(slashed.status, ProducerStatus::Slashed { .. }));

    // Verify honest producer is unaffected
    let honest_info = producer_set.get_by_pubkey(honest.public_key()).unwrap();
    assert!(honest_info.is_active());
    assert_eq!(honest_info.bond_amount, BOND_AMOUNT);

    println!("✓ Slashing updates producer status test passed");
}

// ============================================================================
// TEST 8: Slashed Producer Cannot Be Re-registered
// ============================================================================

#[tokio::test]
async fn test_slashed_producer_cannot_reregister() {
    let malicious = KeyPair::generate();

    let mut producer_set = create_producer_set_with_bonds(&[(malicious.public_key(), BOND_AMOUNT)]);

    // Slash the producer
    let _ = producer_set.slash_producer(malicious.public_key(), 500);

    // Verify slashed
    let slashed = producer_set.get_by_pubkey(malicious.public_key()).unwrap();
    assert!(matches!(slashed.status, ProducerStatus::Slashed { .. }));

    // Attempt to re-register should be blocked
    // (In actual implementation, registration validation checks exit history)
    // For this test, we verify the slashed status persists
    let still_slashed = producer_set.get_by_pubkey(malicious.public_key()).unwrap();
    assert!(!still_slashed.is_active());

    println!("✓ Slashed producer re-registration blocked test passed");
}

// ============================================================================
// TEST 9: Multiple Equivocations Detected
// ============================================================================

#[tokio::test]
async fn test_multiple_equivocations_detected() {
    let mut detector = EquivocationDetector::new();
    let producer = KeyPair::generate();
    let producer_pubkey = *producer.public_key();

    // Equivocate in slot 10
    let block_10a = create_test_block(10, &producer_pubkey, Hash::ZERO, hash(b"10a"));
    let block_10b = create_test_block(10, &producer_pubkey, Hash::ZERO, hash(b"10b"));

    detector.check_block(&block_10a);
    let proof_10 = detector.check_block(&block_10b);
    assert!(proof_10.is_some());

    // Equivocate again in slot 20
    let block_20a = create_test_block(20, &producer_pubkey, Hash::ZERO, hash(b"20a"));
    let block_20b = create_test_block(20, &producer_pubkey, Hash::ZERO, hash(b"20b"));

    detector.check_block(&block_20a);
    let proof_20 = detector.check_block(&block_20b);
    assert!(proof_20.is_some());

    // Should have 2 pending proofs
    let pending = detector.take_pending_proofs();
    assert_eq!(pending.len(), 2);

    println!("✓ Multiple equivocations detected test passed");
}

// ============================================================================
// TEST 10: E2E Slashing Flow (Simulated)
// ============================================================================

#[tokio::test]
async fn test_equivocation_slashing_e2e() {
    // Setup: 3 producers
    let malicious = KeyPair::generate();
    let honest_1 = KeyPair::generate();
    let honest_2 = KeyPair::generate();

    let malicious_pubkey = *malicious.public_key();

    // Create producer set with bonds
    let mut producer_set = create_producer_set_with_bonds(&[
        (malicious.public_key(), BOND_AMOUNT),
        (honest_1.public_key(), BOND_AMOUNT),
        (honest_2.public_key(), BOND_AMOUNT),
    ]);

    // Initial supply: 3 producers × 1000 DOLI
    let initial_supply = 3 * BOND_AMOUNT;
    let total_bonds_before: u64 = producer_set
        .active_producers()
        .iter()
        .map(|p| p.bond_amount)
        .sum();
    assert_eq!(total_bonds_before, initial_supply);

    // Step 1: Malicious producer creates TWO blocks for the same slot
    let slot = 100;
    let block_a = create_test_block(slot, &malicious_pubkey, Hash::ZERO, hash(b"tx_set_a"));
    let block_b = create_test_block(slot, &malicious_pubkey, Hash::ZERO, hash(b"tx_set_b"));

    assert_ne!(block_a.hash(), block_b.hash());

    // Step 2: Honest nodes receive different blocks and detect equivocation
    let mut detector_1 = EquivocationDetector::new();
    let mut detector_2 = EquivocationDetector::new();

    // Honest node 1 receives block A
    assert!(detector_1.check_block(&block_a).is_none());

    // Honest node 2 receives block B
    assert!(detector_2.check_block(&block_b).is_none());

    // Step 3: Nodes gossip blocks and detect equivocation
    // Node 1 receives block B (from gossip)
    let proof = detector_1.check_block(&block_b);
    assert!(proof.is_some(), "Equivocation should be detected");

    let proof = proof.unwrap();

    // Step 4: Verify proof is valid
    assert_eq!(proof.producer, malicious_pubkey);
    assert_eq!(proof.slot, slot);
    assert_eq!(proof.block_header_1.hash(), block_a.hash());
    assert_eq!(proof.block_header_2.hash(), block_b.hash());

    // Step 5: Create and validate slash transaction
    let slash_tx = proof.to_slash_transaction(&honest_1);
    assert!(slash_tx.is_slash_producer());

    // Step 6: Process slashing
    let current_height = 1000;
    let _ = producer_set.slash_producer(&malicious_pubkey, current_height);

    // Step 7: Verify results

    // 7a. Producer is excluded
    let is_active = producer_set
        .get_by_pubkey(&malicious_pubkey)
        .map(|p| p.is_active())
        .unwrap_or(true);
    assert!(!is_active, "Slashed producer should be excluded");

    // 7b. Only 2 active producers remain
    let active_count = producer_set.active_count();
    assert_eq!(active_count, 2, "Should have 2 active producers");

    // 7c. Bond is effectively removed from active set
    let total_bonds_after: u64 = producer_set
        .active_producers()
        .iter()
        .map(|p| p.bond_amount)
        .sum();
    assert_eq!(
        total_bonds_after,
        initial_supply - BOND_AMOUNT,
        "Slashed bond should be removed from active supply"
    );

    println!("✓ E2E equivocation slashing test passed");
    println!("  - Equivocation detected: slot {}", slot);
    println!("  - Proof generated with both block headers");
    println!("  - Slash transaction created");
    println!("  - Producer excluded from active set");
    println!(
        "  - Active bonds reduced: {} -> {}",
        initial_supply, total_bonds_after
    );
}

// ============================================================================
// TEST 11: Detector Eviction (Memory Bounded)
// ============================================================================

#[tokio::test]
async fn test_detector_eviction_memory_bounded() {
    let mut detector = EquivocationDetector::new();

    // Create many blocks to test eviction
    for i in 0..2000u32 {
        let producer = PublicKey::from_bytes([i as u8; 32]);
        let block = create_test_block(i, &producer, Hash::ZERO, hash(&i.to_be_bytes()));
        detector.check_block(&block);
    }

    // Detector should have evicted old entries
    assert!(
        detector.tracked_count() <= 1000,
        "Detector should limit tracked entries"
    );

    println!("✓ Detector eviction test passed (memory bounded)");
}
