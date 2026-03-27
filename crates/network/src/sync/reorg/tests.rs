use super::*;

#[test]
fn test_reorg_handler_creation() {
    let handler = ReorgHandler::new();
    // Genesis (Hash::ZERO) is pre-seeded in recent_blocks
    assert_eq!(handler.tracked_count(), 1);
}

#[test]
fn test_record_block() {
    let mut handler = ReorgHandler::new();

    let genesis = Hash::ZERO;
    let hash1 = crypto::hash::hash(b"block1");

    handler.record_block(hash1, genesis);

    assert!(handler.knows_block(&hash1));
    assert_eq!(handler.get_parent(&hash1), Some(genesis));
    // 2 = genesis (pre-seeded) + block1
    assert_eq!(handler.tracked_count(), 2);
}

#[test]
fn test_no_reorg_on_tip() {
    let mut handler = ReorgHandler::new();

    let genesis = Hash::ZERO;
    let hash1 = crypto::hash::hash(b"block1");

    handler.record_block(hash1, genesis);

    // Create a block that builds on hash1
    let header = doli_core::BlockHeader {
        version: 1,
        prev_hash: hash1,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot: 1,
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
    };
    let block = Block::new(header, vec![]);

    // No reorg needed since it builds on current tip
    assert!(handler.check_reorg(&block, hash1).is_none());
}

#[test]
fn test_detect_reorg() {
    let mut handler = ReorgHandler::new();

    let genesis = Hash::ZERO;
    let hash1 = crypto::hash::hash(b"block1");
    let hash2 = crypto::hash::hash(b"block2");

    // Record main chain: genesis -> hash1 (weight=1) -> hash2 (weight=1)
    // Total accumulated weight = 2
    handler.record_block_with_weight(hash1, genesis, 1);
    handler.record_block_with_weight(hash2, hash1, 1);

    // Create a block that builds on hash1 (not hash2)
    let header = doli_core::BlockHeader {
        version: 1,
        prev_hash: hash1, // Fork from hash1, not hash2
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot: 2,
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
    };
    let block = Block::new(header, vec![]);

    // Fork block has weight=5, so fork chain has accumulated weight = 1 + 5 = 6
    // Current chain has weight 2, so fork is heavier -> should reorg
    let result = handler.check_reorg_weighted(&block, hash2, 5);
    assert!(result.is_some());

    let reorg_result = result.unwrap();
    assert_eq!(reorg_result.rollback.len(), 1);
    assert_eq!(reorg_result.rollback[0], hash2);
    assert!(reorg_result.weight_delta > 0); // New chain is heavier
}

#[test]
fn test_eviction() {
    let mut handler = ReorgHandler::new();
    handler.max_tracked = 10; // Small limit for testing

    let genesis = Hash::ZERO;
    let mut prev = genesis;

    // Add more blocks than limit
    for i in 0..20 {
        let hash = crypto::hash::hash(format!("block{}", i).as_bytes());
        handler.record_block(hash, prev);
        prev = hash;
    }

    // Should only have max_tracked blocks
    assert!(handler.tracked_count() <= 10);
}

#[test]
fn test_weight_accumulation() {
    let mut handler = ReorgHandler::new();

    let genesis = Hash::ZERO;
    let hash1 = crypto::hash::hash(b"block1");
    let hash2 = crypto::hash::hash(b"block2");
    let hash3 = crypto::hash::hash(b"block3");

    // Build chain with different weights
    handler.record_block_with_weight(hash1, genesis, 100); // Weight 100
    handler.record_block_with_weight(hash2, hash1, 200); // Weight 200
    handler.record_block_with_weight(hash3, hash2, 50); // Weight 50

    // Check accumulated weights
    assert_eq!(handler.chain_weight(&hash1), 100);
    assert_eq!(handler.chain_weight(&hash2), 300); // 100 + 200
    assert_eq!(handler.chain_weight(&hash3), 350); // 100 + 200 + 50
    assert_eq!(handler.current_weight(), 350);
}

#[test]
fn test_weight_based_fork_choice_rejects_lighter_chain() {
    let mut handler = ReorgHandler::new();

    let genesis = Hash::ZERO;
    let hash1 = crypto::hash::hash(b"block1");
    let hash2 = crypto::hash::hash(b"block2");

    // Build main chain with high weight
    handler.record_block_with_weight(hash1, genesis, 100);
    handler.record_block_with_weight(hash2, hash1, 200); // Total: 300

    // Create a fork block with low weight
    let header = doli_core::BlockHeader {
        version: 1,
        prev_hash: hash1, // Fork from hash1
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot: 2,
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
    };
    let fork_block = Block::new(header, vec![]);

    // Fork with weight 50 (total 150) should be rejected
    // Our chain has weight 300, fork would have 100 + 50 = 150
    let result = handler.check_reorg_weighted(&fork_block, hash2, 50);
    assert!(result.is_none(), "Should reject lighter fork");
}

#[test]
fn test_weight_based_fork_choice_accepts_heavier_chain() {
    let mut handler = ReorgHandler::new();

    let genesis = Hash::ZERO;
    let hash1 = crypto::hash::hash(b"block1");
    let hash2 = crypto::hash::hash(b"block2");

    // Build main chain with low weight
    handler.record_block_with_weight(hash1, genesis, 100);
    handler.record_block_with_weight(hash2, hash1, 50); // Total: 150

    // Create a fork block with high weight
    let header = doli_core::BlockHeader {
        version: 1,
        prev_hash: hash1, // Fork from hash1
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot: 2,
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
    };
    let fork_block = Block::new(header, vec![]);

    // Fork with weight 200 (total 300) should be accepted
    // Our chain has weight 150, fork would have 100 + 200 = 300
    let result = handler.check_reorg_weighted(&fork_block, hash2, 200);
    assert!(result.is_some(), "Should accept heavier fork");

    let reorg = result.unwrap();
    assert_eq!(reorg.rollback.len(), 1);
    assert_eq!(reorg.rollback[0], hash2);
    assert!(reorg.weight_delta > 0, "Weight delta should be positive");
}

#[test]
fn test_chain_comparison() {
    let mut handler = ReorgHandler::new();

    let genesis = Hash::ZERO;
    let chain_a = crypto::hash::hash(b"chain_a");
    let chain_b = crypto::hash::hash(b"chain_b");

    handler.record_block_with_weight(chain_a, genesis, 100);
    handler.record_block_with_weight(chain_b, genesis, 200);

    use std::cmp::Ordering;
    assert_eq!(handler.compare_chains(&chain_a, &chain_b), Ordering::Less);
    assert_eq!(
        handler.compare_chains(&chain_b, &chain_a),
        Ordering::Greater
    );
    assert_eq!(handler.compare_chains(&chain_a, &chain_a), Ordering::Equal);
}

#[test]
fn test_equal_weight_tiebreak_by_hash() {
    let mut handler = ReorgHandler::new();

    let genesis = Hash::ZERO;
    let hash1 = crypto::hash::hash(b"block1");
    let hash2 = crypto::hash::hash(b"block2");

    // Build main chain: genesis -> hash1 (weight=1) -> hash2 (weight=1)
    // Total accumulated weight = 2
    handler.record_block_with_weight(hash1, genesis, 1);
    handler.record_block_with_weight(hash2, hash1, 1);

    // Create a fork block on hash1 with weight=1 (equal total weight = 2)
    let header = doli_core::BlockHeader {
        version: 1,
        prev_hash: hash1,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot: 2,
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
    };
    let fork_block = Block::new(header, vec![]);
    let fork_hash = fork_block.hash();

    // Equal weight: tie-break by hash
    let result = handler.check_reorg_weighted(&fork_block, hash2, 1);

    // One of the two hashes must be "lower" — the tie-breaker picks it
    if fork_hash.as_bytes() < hash2.as_bytes() {
        assert!(
            result.is_some(),
            "Fork with lower hash should win tie-break"
        );
    } else {
        assert!(
            result.is_none(),
            "Fork with higher hash should lose tie-break"
        );
    }
}

#[test]
fn test_tiebreak_with_new_method() {
    let mut handler = ReorgHandler::new();

    let genesis = Hash::ZERO;
    let tip_a = crypto::hash::hash(b"tip_a");
    let tip_b = crypto::hash::hash(b"tip_b");

    // Both chains have equal weight
    handler.record_block_with_weight(tip_a, genesis, 100);
    // Reset current weight to test tiebreak method
    handler.set_current_weight(100);

    handler.block_weights.insert(
        tip_b,
        BlockWeight {
            prev_hash: genesis,
            producer_weight: 100,
            accumulated_weight: 100,
            height: 1,
        },
    );

    // The lower hash should win
    let a_wins = handler.should_reorg_by_weight_with_tiebreak(&tip_b, &tip_a);
    let b_wins = handler.should_reorg_by_weight_with_tiebreak(&tip_a, &tip_b);

    // Exactly one should win (they can't both have equal hashes)
    assert_ne!(
        a_wins, b_wins,
        "Tie-break must be deterministic: exactly one chain wins"
    );
}

#[test]
fn test_reorg_past_finality_rejected() {
    let mut handler = ReorgHandler::new();

    let genesis = Hash::ZERO;
    let block1 = crypto::hash::hash(b"block1");
    let block2 = crypto::hash::hash(b"block2");
    let _fork_block = crypto::hash::hash(b"fork");

    // Build main chain: genesis -> block1 -> block2
    handler.record_block_with_weight(block1, genesis, 10);
    handler.record_block_with_weight(block2, block1, 10);

    // Set finality at height 1 (block1)
    handler.set_last_finality_height(1);

    // Create a fork from genesis with higher weight
    // This would reorg past the finalized block1 — should be rejected
    let fork = Block {
        header: doli_core::BlockHeader {
            version: 1,
            prev_hash: genesis, // forks from genesis (height 0) < finality height 1
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: 100,
            slot: 1,
            producer: crypto::PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput {
                value: vec![0u8; 32],
            },
            vdf_proof: vdf::VdfProof { pi: vec![0u8; 32] },
        },
        transactions: vec![],
        aggregate_bls_signature: Vec::new(),
    };

    let result = handler.check_reorg_weighted(&fork, block2, 100);
    assert!(result.is_none(), "Reorg past finality should be rejected");
}

#[test]
fn test_reorg_after_finality_ok() {
    let mut handler = ReorgHandler::new();

    let genesis = Hash::ZERO;
    let block1 = crypto::hash::hash(b"block1");
    let block2 = crypto::hash::hash(b"block2");

    handler.record_block_with_weight(block1, genesis, 10);
    handler.record_block_with_weight(block2, block1, 10);

    // Set finality at height 0 (genesis)
    handler.set_last_finality_height(0);

    // Fork from block1 (height 1) which is above finality — should be allowed if heavier
    let fork = Block {
        header: doli_core::BlockHeader {
            version: 1,
            prev_hash: block1, // forks from block1 (height 1) > finality height 0
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: 200,
            slot: 2,
            producer: crypto::PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput {
                value: vec![0u8; 32],
            },
            vdf_proof: vdf::VdfProof { pi: vec![0u8; 32] },
        },
        transactions: vec![],
        aggregate_bls_signature: Vec::new(),
    };

    let result = handler.check_reorg_weighted(&fork, block2, 100);
    assert!(result.is_some(), "Reorg above finality should be allowed");
}

#[test]
fn test_fork_block_recording_does_not_corrupt_current_weight() {
    // Regression test for the N4 solo-fork bug:
    // record_fork_block() was using record_block_with_weight() which
    // unconditionally overwrote current_chain_weight, making the fork
    // recovery comparison always see delta=0.
    let mut handler = ReorgHandler::new();

    let genesis = Hash::ZERO;
    let shared = crypto::hash::hash(b"shared_block");

    // Our chain: genesis -> shared (w=100) -> our_tip (w=100) = 200
    handler.record_block_with_weight(shared, genesis, 100);
    let our_tip = crypto::hash::hash(b"our_solo_block");
    handler.record_block_with_weight(our_tip, shared, 100);
    assert_eq!(handler.current_weight(), 200);

    // Fork recovery finds canonical chain: shared -> fork_tip (w=100) = 200
    let fork_tip_hash = crypto::hash::hash(b"canonical_block");
    handler.record_fork_block(fork_tip_hash, shared, 100);

    // CRITICAL: current_chain_weight must still be 200 (our chain), not
    // overwritten to 200 (fork chain). Before the fix, this was corrupted.
    assert_eq!(
        handler.current_weight(),
        200,
        "record_fork_block must NOT overwrite current_chain_weight"
    );

    // Now test with a heavier fork: shared -> fork_heavy (w=500) = 600
    let fork_heavy = crypto::hash::hash(b"canonical_heavy");
    handler.record_fork_block(fork_heavy, shared, 500);

    // current_weight still unchanged
    assert_eq!(handler.current_weight(), 200);

    // But the fork's accumulated weight should be queryable
    assert_eq!(handler.chain_weight(&fork_heavy), 600);

    // check_reorg_weighted should detect the heavier fork
    let fork_block = Block {
        header: doli_core::BlockHeader {
            version: 1,
            prev_hash: shared,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: 0,
            slot: 2,
            producer: crypto::PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput {
                value: vec![0u8; 32],
            },
            vdf_proof: vdf::VdfProof { pi: vec![0u8; 32] },
        },
        transactions: vec![],
        aggregate_bls_signature: Vec::new(),
    };
    let result = handler.check_reorg_weighted(&fork_block, our_tip, 500);
    assert!(
        result.is_some(),
        "Heavier fork should trigger reorg after correct weight tracking"
    );
}

#[test]
fn test_plan_reorg_past_finality_rejected() {
    let mut handler = ReorgHandler::new();

    let genesis = Hash::ZERO;
    let block1 = crypto::hash::hash(b"block1");
    let block2 = crypto::hash::hash(b"block2");
    let fork_tip = crypto::hash::hash(b"fork_tip");

    // Build main chain: genesis -> block1 -> block2
    handler.record_block_with_weight(block1, genesis, 10);
    handler.record_block_with_weight(block2, block1, 10);

    // Build fork chain: genesis -> fork_tip (higher weight)
    handler.record_fork_block(fork_tip, genesis, 100);

    // Set finality at height 1 (block1 is finalized)
    handler.set_last_finality_height(1);

    // plan_reorg from block2 to fork_tip — common ancestor is genesis (height 0)
    // which is below finality height 1. Must be rejected.
    let result = handler.plan_reorg(block2, fork_tip, |_| None);
    assert!(
        result.is_none(),
        "plan_reorg must reject reorg past finalized height"
    );
}

#[test]
fn test_plan_reorg_above_finality_ok() {
    let mut handler = ReorgHandler::new();

    let genesis = Hash::ZERO;
    let block1 = crypto::hash::hash(b"block1");
    let block2 = crypto::hash::hash(b"block2");
    let fork_tip = crypto::hash::hash(b"fork_from_b1");

    // Build main chain: genesis -> block1 -> block2
    handler.record_block_with_weight(block1, genesis, 10);
    handler.record_block_with_weight(block2, block1, 10);

    // Build fork from block1: block1 -> fork_tip (higher weight)
    handler.record_fork_block(fork_tip, block1, 100);

    // Set finality at height 0 (genesis)
    handler.set_last_finality_height(0);

    // plan_reorg from block2 to fork_tip — common ancestor is block1 (height 1)
    // which is above finality height 0. Should be allowed.
    let result = handler.plan_reorg(block2, fork_tip, |_| None);
    assert!(
        result.is_some(),
        "plan_reorg should allow reorg above finality"
    );
}

#[test]
fn test_last_finality_height_getter() {
    let mut handler = ReorgHandler::new();
    assert_eq!(handler.last_finality_height(), None);

    handler.set_last_finality_height(42);
    assert_eq!(handler.last_finality_height(), Some(42));

    handler.set_last_finality_height(100);
    assert_eq!(handler.last_finality_height(), Some(100));
}
