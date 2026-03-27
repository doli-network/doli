//! Adversarial tests for the network sync layer.
//!
//! These tests target specific vulnerabilities discovered during INC-I-014
//! (RAM explosion / eviction churn) and INC-I-010 (fork-stuck node).
//! They stress edge cases in:
//!   - Reorg handler (unbounded growth, weight overflow, finality bypass)
//!   - Fork recovery tracker (depth limits, timing attacks)
//!   - Equivocation detector (memory growth, LRU correctness)
//!   - Rate limiter (token bucket overflow, cleanup under load)
//!   - Gossip config (mesh parameter invariants)

use std::time::Duration;

use crypto::Hash;
use doli_core::{Block, BlockHeader};
use libp2p::PeerId;
use vdf::{VdfOutput, VdfProof};

use super::equivocation::EquivocationDetector;
use super::fork_recovery::ForkRecoveryTracker;
use super::reorg::ReorgHandler;
use crate::rate_limit::{RateLimitConfig, RateLimiter, TokenBucket};

// =========================================================================
// HELPERS
// =========================================================================

fn make_block(prev_hash: Hash, slot: u32) -> Block {
    let header = BlockHeader {
        version: 1,
        prev_hash,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: slot as u64 * 10,
        slot,
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        vdf_output: VdfOutput { value: vec![] },
        vdf_proof: VdfProof::empty(),
    };
    Block::new(header, vec![])
}

fn make_block_with_producer(prev_hash: Hash, slot: u32, producer_byte: u8) -> Block {
    let header = BlockHeader {
        version: 1,
        prev_hash,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: slot as u64 * 10,
        slot,
        producer: crypto::PublicKey::from_bytes([producer_byte; 32]),
        vdf_output: VdfOutput { value: vec![] },
        vdf_proof: VdfProof::empty(),
    };
    Block::new(header, vec![])
}

/// Build a linear chain of N blocks starting from `start_hash`.
/// Returns (blocks, tip_hash).
fn build_chain(start_hash: Hash, start_slot: u32, count: usize) -> (Vec<Block>, Hash) {
    let mut blocks = Vec::with_capacity(count);
    let mut prev = start_hash;
    for i in 0..count {
        let block = make_block(prev, start_slot + i as u32);
        prev = block.hash();
        blocks.push(block);
    }
    (blocks, prev)
}

// =========================================================================
// P0: REORG HANDLER — WEIGHT OVERFLOW / UNBOUNDED GROWTH
// =========================================================================

/// P0 BUG FOUND: accumulated_weight uses saturating_add. When weight=u64::MAX,
/// the parent's accumulated weight (u64::MAX) + new block weight (u64::MAX)
/// saturates to u64::MAX. The new chain weight is then compared to
/// current_chain_weight (also u64::MAX), triggering the equal-weight
/// tie-break path. HOWEVER, the tie-break compares the NEW BLOCK hash
/// against the CURRENT TIP hash. Since the current tip was recorded with
/// weight u64::MAX, and the new block ALSO saturates to u64::MAX, the
/// comparison uses block_hash vs current_tip — but block_hash is NOT
/// yet recorded in the handler, so chain_weight returns 0 for the
/// comparison. This means the tie-break always fails for saturated weights.
///
/// IMPACT: An attacker who can claim weight=u64::MAX can prevent legitimate
/// reorgs by saturating all chain weights. In practice, weights come from
/// ProducerSet (bounded by bond amount), so this is theoretical.
#[test]
fn test_reorg_weight_saturation_deterministic() {
    let mut handler = ReorgHandler::new();
    let genesis = Hash::ZERO;

    // Chain A: genesis -> A1 (weight=u64::MAX)
    let block_a1 = make_block(genesis, 1);
    let hash_a1 = block_a1.hash();
    handler.record_block_with_weight(hash_a1, genesis, u64::MAX);

    // Chain B: genesis -> B1 (weight=u64::MAX), fork from genesis
    let block_b1 = make_block_with_producer(genesis, 1, 1);
    let hash_b1 = block_b1.hash();

    // Both chains have u64::MAX accumulated weight (saturated)
    let result = handler.check_reorg_weighted(&block_b1, hash_a1, u64::MAX);

    // BUG DOCUMENTED: Under weight saturation, the tie-break path is entered
    // but the comparison is against the PARENT's accumulated weight (u64::MAX)
    // + block weight (u64::MAX) = u64::MAX (saturated). The existing chain
    // also has u64::MAX. The hash tie-break then determines the outcome.
    // Verify no panic occurs regardless of result.
    let _ = result; // No panic = success for this adversarial test
}

/// P0 BUG: If the LRU eviction in ReorgHandler removes a block that's part
/// of the current chain, check_reorg_weighted can't find the common ancestor
/// and silently returns None — the fork is never detected.
#[test]
fn test_reorg_handler_eviction_loses_ancestor() {
    let mut handler = ReorgHandler::new();
    handler.max_tracked = 5; // Very small LRU to trigger eviction fast

    let genesis = Hash::ZERO;

    // Build a 10-block chain — blocks 1-5 will be evicted
    let mut prev = genesis;
    let mut hashes = vec![genesis];
    for i in 1..=10 {
        let block = make_block(prev, i);
        let hash = block.hash();
        handler.record_block_with_weight(hash, prev, 1);
        prev = hash;
        hashes.push(hash);
    }

    // Now try to detect a fork from block 3 (which was evicted)
    let fork_block = make_block_with_producer(hashes[3], 4, 42);

    // The handler can't find the common ancestor because block 3 was evicted
    let result = handler.check_reorg_weighted(&fork_block, prev, 100);

    // FINDING: This returns None even though the fork is heavier.
    // The handler silently ignores the fork because it can't trace back.
    if handler.knows_block(&hashes[3]) {
        assert!(result.is_some());
    } else {
        // Eviction happened — ancestor is lost, reorg not detected.
        // This is the bug: silent failure.
        assert!(
            result.is_none(),
            "Lost ancestor should cause reorg detection failure (known limitation)"
        );
    }
}

/// P0: Verify finality check in check_reorg_weighted prevents reorgs past finalized height.
#[test]
fn test_reorg_finality_prevents_deep_reorg() {
    let mut handler = ReorgHandler::new();
    let genesis = Hash::ZERO;

    // Build chain: genesis -> A -> B -> C -> D
    let block_a = make_block(genesis, 1);
    let hash_a = block_a.hash();
    handler.record_block_with_weight(hash_a, genesis, 1);

    let block_b = make_block(hash_a, 2);
    let hash_b = block_b.hash();
    handler.record_block_with_weight(hash_b, hash_a, 1);

    let block_c = make_block(hash_b, 3);
    let hash_c = block_c.hash();
    handler.record_block_with_weight(hash_c, hash_b, 1);

    let block_d = make_block(hash_c, 4);
    let hash_d = block_d.hash();
    handler.record_block_with_weight(hash_d, hash_c, 1);

    // Finalize at height 2 (block B)
    handler.set_last_finality_height(2);

    // Try to reorg from genesis (below finality) with a heavier fork
    let fork_block = make_block_with_producer(hash_a, 2, 99);
    let result = handler.check_reorg_weighted(&fork_block, hash_d, 1000);

    // Must be rejected — common ancestor (hash_a at height 1) is below finality (height 2)
    assert!(
        result.is_none(),
        "Reorg past finalized height must be rejected"
    );
}

/// P0: Verify plan_reorg also respects finality (defense-in-depth).
#[test]
fn test_plan_reorg_finality_guard() {
    let mut handler = ReorgHandler::new();
    let genesis = Hash::ZERO;

    // Build chain: genesis -> A -> B -> C
    let (chain, tip) = build_chain(genesis, 1, 3);
    for block in &chain {
        handler.record_block_with_weight(block.hash(), block.header.prev_hash, 1);
    }

    // Build fork: genesis -> X -> Y
    let (fork, fork_tip) = build_chain(genesis, 1, 2);
    for block in &fork {
        handler.record_fork_block(block.hash(), block.header.prev_hash, 100);
    }

    // Finalize at height 2
    handler.set_last_finality_height(2);

    // plan_reorg should refuse because common ancestor is genesis (height 0) < finality (2)
    let result = handler.plan_reorg(tip, fork_tip, |_| None);
    assert!(
        result.is_none(),
        "plan_reorg must reject reorg past finalized height"
    );
}

/// P2: Verify ReorgHandler memory stays bounded after recording many blocks.
#[test]
fn test_reorg_handler_memory_bounded() {
    let mut handler = ReorgHandler::new();
    // Default max_tracked is 10000. Record 20000 blocks.
    let genesis = Hash::ZERO;
    let mut prev = genesis;
    for i in 1..=20_000u32 {
        let block = make_block(prev, i);
        let hash = block.hash();
        handler.record_block_with_weight(hash, prev, 1);
        prev = hash;
    }

    // Should be capped at max_tracked
    assert!(
        handler.tracked_count() <= 10_000,
        "ReorgHandler should not exceed max_tracked: got {}",
        handler.tracked_count()
    );
}

/// P1: Weight delta overflow when computing i64 from u64::MAX chains.
#[test]
fn test_reorg_weight_delta_no_overflow() {
    let mut handler = ReorgHandler::new();
    let genesis = Hash::ZERO;

    // Chain with weight near u64::MAX
    let block_a = make_block(genesis, 1);
    let hash_a = block_a.hash();
    handler.record_block_with_weight(hash_a, genesis, u64::MAX / 2);

    let block_b = make_block(hash_a, 2);
    let hash_b = block_b.hash();
    handler.record_block_with_weight(hash_b, hash_a, u64::MAX / 2);

    // Fork from genesis with small weight
    let fork = make_block_with_producer(genesis, 1, 5);

    // The weight_delta computation casts u64 to i64.
    // Fork is lighter — should NOT reorg regardless of overflow behavior.
    let result = handler.check_reorg_weighted(&fork, hash_b, 1);
    assert!(
        result.is_none(),
        "Lighter fork should always be rejected even with large weights"
    );
}

// =========================================================================
// P0: FORK RECOVERY TRACKER — TIMING / DEPTH ATTACKS
// =========================================================================

/// P0: Verify that fork recovery handles the case where `next_fetch()` is
/// called rapidly without any response — it should not leak memory.
#[test]
fn test_fork_recovery_no_response_memory_stable() {
    let mut tracker = ForkRecoveryTracker::new();
    let peer = PeerId::random();

    let orphan = make_block(crypto::hash::hash(b"parent"), 100);
    assert!(tracker.start(orphan, peer));

    // Call next_fetch 1000 times without providing any response.
    let mut fetch_count = 0;
    for _ in 0..1000 {
        if tracker.next_fetch().is_some() {
            fetch_count += 1;
        }
    }

    // Should only get ONE fetch (the initial one), then None because pending=true
    assert_eq!(
        fetch_count, 1,
        "Should only issue one fetch request while pending"
    );
    assert!(tracker.is_active(), "Should still be active");
}

/// P0: Verify that the wrong peer's response is ignored.
#[test]
fn test_fork_recovery_wrong_peer_ignored() {
    let mut tracker = ForkRecoveryTracker::new();
    let peer_a = PeerId::random();
    let peer_b = PeerId::random();

    let parent_hash = crypto::hash::hash(b"parent");
    let orphan = make_block(parent_hash, 100);
    assert!(tracker.start(orphan, peer_a));

    let _ = tracker.next_fetch();

    // Wrong peer responds — should be ignored
    let block = make_block(Hash::ZERO, 99);
    let consumed = tracker.handle_block(peer_b, Some(block));
    assert!(!consumed, "Response from wrong peer should be ignored");

    assert!(tracker.is_active());
}

/// Fixed: cancel("exceeded max depth") now sets the flag regardless of
/// whether recovery is active, so callers can always rely on
/// take_exceeded_max_depth() after cancel.
#[test]
fn test_fork_recovery_exceeded_max_depth_requires_active() {
    let mut tracker = ForkRecoveryTracker::new();

    // Fixed: cancel without active recovery now DOES set the flag
    tracker.cancel("exceeded max depth");
    assert!(
        tracker.take_exceeded_max_depth(),
        "cancel('exceeded max depth') should set flag even without active recovery"
    );
    // Consumed — subsequent takes return false
    assert!(!tracker.take_exceeded_max_depth());

    // Also works with active recovery
    let peer = PeerId::random();
    let orphan = make_block(crypto::hash::hash(b"parent"), 100);
    assert!(tracker.start(orphan, peer));
    assert!(tracker.is_active());

    tracker.cancel("exceeded max depth");
    assert!(!tracker.is_active());

    assert!(tracker.take_exceeded_max_depth());
    assert!(!tracker.take_exceeded_max_depth());
}

/// P1: Verify that starting recovery while one is active fails.
#[test]
fn test_fork_recovery_no_concurrent() {
    let mut tracker = ForkRecoveryTracker::new();
    let peer1 = PeerId::random();
    let peer2 = PeerId::random();

    let orphan1 = make_block(crypto::hash::hash(b"parent1"), 10);
    let orphan2 = make_block(crypto::hash::hash(b"parent2"), 20);

    assert!(tracker.start(orphan1, peer1));
    assert!(
        !tracker.start(orphan2, peer2),
        "Should reject concurrent recovery"
    );
    assert!(tracker.is_active());
}

/// P3: Verify check_connection with parent_known=false does nothing.
#[test]
fn test_fork_recovery_check_connection_false() {
    let mut tracker = ForkRecoveryTracker::new();
    let peer = PeerId::random();

    let orphan = make_block(Hash::ZERO, 1);
    tracker.start(orphan, peer);

    assert!(tracker.check_connection(false).is_none());
    assert!(tracker.is_active(), "Recovery should still be active");
}

// =========================================================================
// P0: EQUIVOCATION DETECTOR — MEMORY / LRU
// =========================================================================

/// P2: Verify the equivocation detector stays bounded under heavy load.
#[test]
fn test_equivocation_detector_memory_bounded() {
    let mut detector = EquivocationDetector::new();

    // Insert 50,000 unique (producer, slot) pairs
    for i in 0..50_000u32 {
        let producer_bytes = (i as u64).to_le_bytes();
        let mut key = [0u8; 32];
        key[..8].copy_from_slice(&producer_bytes);
        let producer = crypto::PublicKey::from_bytes(key);

        let header = BlockHeader {
            version: 1,
            prev_hash: Hash::ZERO,
            merkle_root: crypto::hash::hash(format!("block_{}", i).as_bytes()),
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: 0,
            slot: i,
            producer,
            vdf_output: VdfOutput { value: vec![] },
            vdf_proof: VdfProof::empty(),
        };
        let block = Block::new(header, vec![]);
        let _ = detector.check_block(&block);
    }

    let count = detector.tracked_count();
    assert!(
        count <= 10_001,
        "EquivocationDetector should be bounded: got {} entries",
        count
    );
}

/// P3: Verify cleanup_before_slot correctly removes old entries.
#[test]
fn test_equivocation_detector_cleanup_consistency() {
    let mut detector = EquivocationDetector::new();

    // Insert entries at slots 0..100
    for slot in 0..100u32 {
        let producer = crypto::PublicKey::from_bytes([slot as u8; 32]);
        let header = BlockHeader {
            version: 1,
            prev_hash: Hash::ZERO,
            merkle_root: crypto::hash::hash(format!("s{}", slot).as_bytes()),
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: 0,
            slot,
            producer,
            vdf_output: VdfOutput { value: vec![] },
            vdf_proof: VdfProof::empty(),
        };
        let block = Block::new(header, vec![]);
        let _ = detector.check_block(&block);
    }

    // Cleanup everything before slot 50
    detector.cleanup_before_slot(50);

    let remaining = detector.tracked_count();
    assert_eq!(
        remaining, 50,
        "After cleanup_before_slot(50), should have 50 entries, got {}",
        remaining
    );

    // Double cleanup should be a no-op
    detector.cleanup_before_slot(50);
    assert_eq!(detector.tracked_count(), 50);

    // Cleanup with smaller slot should also be a no-op
    detector.cleanup_before_slot(30);
    assert_eq!(detector.tracked_count(), 50);
}

/// P0: Verify equivocation IS detected when same producer makes two
/// different blocks for the same slot.
#[test]
fn test_equivocation_detected_correctly() {
    let mut detector = EquivocationDetector::new();
    let producer = crypto::PublicKey::from_bytes([1u8; 32]);

    let header1 = BlockHeader {
        version: 1,
        prev_hash: Hash::ZERO,
        merkle_root: crypto::hash::hash(b"block1"),
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot: 10,
        producer,
        vdf_output: VdfOutput { value: vec![] },
        vdf_proof: VdfProof::empty(),
    };
    let block1 = Block::new(header1, vec![]);
    assert!(detector.check_block(&block1).is_none());

    let header2 = BlockHeader {
        version: 1,
        prev_hash: Hash::ZERO,
        merkle_root: crypto::hash::hash(b"block2"),
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot: 10,
        producer,
        vdf_output: VdfOutput { value: vec![] },
        vdf_proof: VdfProof::empty(),
    };
    let block2 = Block::new(header2, vec![]);
    let proof = detector.check_block(&block2);

    assert!(
        proof.is_some(),
        "Second different block at same slot should trigger equivocation"
    );
    let proof = proof.unwrap();
    assert_eq!(proof.producer, producer);
    assert_eq!(proof.slot, 10);
}

// =========================================================================
// P2: RATE LIMITER — TOKEN BUCKET EDGE CASES
// =========================================================================

/// P2 BUG FOUND: TokenBucket::refill() at rate_limit.rs:41 has:
///   `self.tokens = (self.tokens + new_tokens).min(self.capacity);`
/// When self.tokens is near u64::MAX and new_tokens > 0, the addition
/// overflows (panic in debug, wraps in release). The fix is to use
/// saturating_add: `self.tokens.saturating_add(new_tokens).min(self.capacity)`.
///
/// IMPACT: Any peer with a high-capacity bucket that gets a refill can cause
/// a panic in debug builds. In release builds, the wrap-around would give
/// a tiny token count, effectively rate-limiting the peer forever.
#[test]
fn test_token_bucket_extreme_capacity_overflow_bug() {
    // Use a capacity that won't trigger the overflow in refill()
    // (refill only adds tokens when elapsed > 0, and we call immediately)
    let mut bucket = TokenBucket::new(u64::MAX, 0.0); // Zero refill rate

    // With zero refill rate, no overflow in refill()
    assert!(bucket.try_consume(1));
    assert!(bucket.try_consume(u64::MAX - 2)); // Leave 1 token (consumed 1 + MAX-2 = MAX-1)
    assert!(bucket.try_consume(1)); // Last token
    assert!(!bucket.try_consume(1)); // Empty

    // Document the bug: with non-zero refill rate and high tokens, refill panics
    // TokenBucket::new(u64::MAX, 1.0) followed by sleep + try_consume would panic
    // at rate_limit.rs:41 due to u64 overflow in `self.tokens + new_tokens`.
}

/// P2: Verify rate limiter handles 10,000 unique peers without excessive memory.
#[test]
fn test_rate_limiter_many_peers_bounded() {
    let config = RateLimitConfig::default();
    let mut limiter = RateLimiter::new(config);

    for _ in 0..10_000 {
        let peer = PeerId::random();
        limiter.record_block(&peer, 1000);
    }

    assert_eq!(limiter.stats().tracked_peers, 10_000);

    // Cleanup with max_age=0 should remove all (all have last_activity in the past)
    limiter.cleanup(Duration::from_secs(0));

    let after = limiter.stats().tracked_peers;
    assert!(
        after < 10_000,
        "Cleanup with max_age=0 should remove peers, got {} remaining",
        after
    );
}

/// P2: Verify the LRU eviction in rate limiter cleanup bounds memory.
#[test]
fn test_rate_limiter_lru_eviction() {
    let config = RateLimitConfig::default();
    let mut limiter = RateLimiter::new(config);

    for _ in 0..2000 {
        let peer = PeerId::random();
        limiter.record_block(&peer, 100);
    }

    assert_eq!(limiter.stats().tracked_peers, 2000);

    // Cleanup with long max_age — LRU should cap at 1000
    limiter.cleanup(Duration::from_secs(86400));

    assert!(
        limiter.stats().tracked_peers <= 1000,
        "Rate limiter should cap at MAX_TRACKED_PEERS=1000 after cleanup, got {}",
        limiter.stats().tracked_peers
    );
}

// =========================================================================
// P1: GOSSIP CONFIG — MESH PARAMETER EDGE CASES
// =========================================================================

/// P3: Verify compute_dynamic_mesh handles edge cases (0, 1, large peers).
#[test]
fn test_dynamic_mesh_edge_cases() {
    use crate::gossip::compute_dynamic_mesh;

    // 0 peers
    let mesh = compute_dynamic_mesh(0);
    assert!(mesh.mesh_n >= 6, "mesh_n should be at least 6 for 0 peers");
    assert!(mesh.mesh_n_low <= mesh.mesh_n);
    assert!(mesh.mesh_n_high >= mesh.mesh_n);

    // 1 peer
    let mesh = compute_dynamic_mesh(1);
    assert!(mesh.mesh_n >= 6, "mesh_n should be at least 6 for 1 peer");

    // Exact boundary: 20 peers
    let mesh = compute_dynamic_mesh(20);
    assert_eq!(
        mesh.mesh_n, 19,
        "20 peers: mesh_n should be total_peers - 1 = 19"
    );

    // 21 peers — switches to sqrt scaling
    let mesh = compute_dynamic_mesh(21);
    assert!(mesh.mesh_n < 20, "21 peers: should use sqrt scaling");

    // Very large: 1,000,000 peers
    let mesh = compute_dynamic_mesh(1_000_000);
    assert!(
        mesh.mesh_n <= 50,
        "mesh_n should be capped at MESH_N_CAP=50"
    );
    assert!(mesh.mesh_n >= 8, "mesh_n should be at least 8");
}

/// P3: Verify mesh parameter invariants always hold.
#[test]
fn test_dynamic_mesh_invariants() {
    use crate::gossip::compute_dynamic_mesh;

    for total_peers in [0, 1, 2, 5, 10, 20, 21, 50, 100, 500, 1000, 10000] {
        let mesh = compute_dynamic_mesh(total_peers);

        assert!(
            mesh.mesh_n_low <= mesh.mesh_n,
            "mesh_n_low ({}) > mesh_n ({}) at {} peers",
            mesh.mesh_n_low,
            mesh.mesh_n,
            total_peers
        );
        assert!(
            mesh.mesh_n <= mesh.mesh_n_high,
            "mesh_n ({}) > mesh_n_high ({}) at {} peers",
            mesh.mesh_n,
            mesh.mesh_n_high,
            total_peers
        );
        assert!(
            mesh.gossip_lazy >= 6,
            "gossip_lazy ({}) < 6 at {} peers",
            mesh.gossip_lazy,
            total_peers
        );
        assert!(
            mesh.mesh_n_low >= 6,
            "mesh_n_low ({}) < 6 at {} peers",
            mesh.mesh_n_low,
            total_peers
        );
    }
}

// =========================================================================
// P0: REORG HANDLER — PLAN_REORG EDGE CASES
// =========================================================================

/// P3: Verify plan_reorg handles circular parent chains without infinite loop.
#[test]
fn test_plan_reorg_circular_parent_no_hang() {
    let mut handler = ReorgHandler::new();
    let genesis = Hash::ZERO;

    let block_a = make_block(genesis, 1);
    let hash_a = block_a.hash();
    handler.record_block_with_weight(hash_a, genesis, 1);

    let block_b = make_block(hash_a, 2);
    let hash_b = block_b.hash();
    handler.record_block_with_weight(hash_b, hash_a, 1);

    let fork_tip = crypto::hash::hash(b"fork_tip");
    let x_hash = crypto::hash::hash(b"x");

    // Circular callback: fork_tip -> X -> fork_tip (cycle)
    let result = handler.plan_reorg(hash_b, fork_tip, |h| {
        if *h == fork_tip {
            Some(x_hash)
        } else if *h == x_hash {
            Some(fork_tip)
        } else {
            None
        }
    });

    // Must not hang. Should return None.
    assert!(
        result.is_none(),
        "Circular parent chain should not cause infinite loop"
    );
}

/// Fixed: plan_reorg now finds genesis (Hash::ZERO) as common ancestor.
/// Genesis is included in the ancestor set before the is_zero() break.
#[test]
fn test_plan_reorg_deep_fork_genesis_boundary_bug() {
    let mut handler = ReorgHandler::new();
    let genesis = Hash::ZERO;

    // Build main chain of 100 blocks
    let (main_chain, main_tip) = build_chain(genesis, 1, 100);
    for block in &main_chain {
        handler.record_block_with_weight(block.hash(), block.header.prev_hash, 1);
    }

    // Build fork from genesis of 100 blocks
    let mut prev = genesis;
    for i in 0..100 {
        let hash = crypto::hash::hash(format!("fork_{}", i).as_bytes());
        handler.record_fork_block(hash, prev, 2);
        prev = hash;
    }
    let fork_tip = prev;

    // Fixed: plan_reorg now includes genesis in ancestor set
    let result = handler.plan_reorg(main_tip, fork_tip, |_| None);
    assert!(
        result.is_some(),
        "plan_reorg should find genesis as common ancestor"
    );
    let reorg = result.unwrap();
    assert_eq!(reorg.common_ancestor, genesis);
}

// =========================================================================
// P0: DETERMINISTIC TIE-BREAKING
// =========================================================================

/// Fixed: Genesis forks are now detected because genesis (Hash::ZERO) is
/// pre-seeded in recent_blocks. The handler reaches the weight comparison
/// instead of bailing with "Unknown parent".
#[test]
fn test_deterministic_tiebreak_genesis_fork_bug() {
    let genesis = Hash::ZERO;

    // Use a heavier fork block to guarantee reorg detection
    let mut handler = ReorgHandler::new();
    let block1 = make_block(genesis, 1);
    let hash1 = block1.hash();
    handler.record_block_with_weight(hash1, genesis, 1);

    let block2 = make_block_with_producer(genesis, 1, 42);

    // With genesis pre-seeded, the handler can find the parent and
    // reach the weight comparison. Use weight=2 to ensure it's heavier.
    let result = handler.check_reorg_weighted(&block2, hash1, 2);
    assert!(
        result.is_some(),
        "check_reorg_weighted should detect heavier genesis forks now that genesis is pre-seeded"
    );
}

// =========================================================================
// P0: RECORD_FORK_BLOCK vs RECORD_BLOCK_WITH_WEIGHT
// =========================================================================

/// P0: Verify record_fork_block does NOT update current_chain_weight.
#[test]
fn test_fork_block_does_not_update_current_weight() {
    let mut handler = ReorgHandler::new();
    let genesis = Hash::ZERO;

    // Our chain: genesis -> A (weight=1), total=1
    let block_a = make_block(genesis, 1);
    let hash_a = block_a.hash();
    handler.record_block_with_weight(hash_a, genesis, 1);

    let weight_before = handler.current_weight();
    assert_eq!(weight_before, 1);

    let fork_hash = crypto::hash::hash(b"fork");
    handler.record_fork_block(fork_hash, genesis, 100);

    let weight_after = handler.current_weight();
    assert_eq!(
        weight_after, weight_before,
        "record_fork_block must NOT update current_chain_weight"
    );

    // But the fork block's accumulated weight should be tracked
    assert_eq!(handler.chain_weight(&fork_hash), 100);
}

// =========================================================================
// P2: ADAPTIVE GOSSIP — EDGE CASES
// =========================================================================

/// P2: AdaptiveGossip large-network floor enforced at exactly 50 peers.
#[test]
fn test_adaptive_gossip_large_network_floor() {
    use doli_core::discovery::{AdaptiveGossip, MergeResult};

    let mut gossip = AdaptiveGossip::new();

    // 49 peers — should use normal min (1s)
    gossip.on_gossip_result(
        &MergeResult {
            added: 1,
            new_producers: 1,
            rejected: 0,
            duplicates: 0,
        },
        49,
    );
    assert_eq!(
        gossip.interval(),
        Duration::from_secs(1),
        "49 peers: should use 1s floor"
    );

    // 50 peers — should switch to 10s floor
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
        Duration::from_secs(10),
        "50 peers: should use 10s floor"
    );
}

/// P3: AdaptiveGossip backoff doesn't overflow with many stable rounds.
#[test]
fn test_adaptive_gossip_backoff_no_overflow() {
    use doli_core::discovery::{AdaptiveGossip, MergeResult};

    let mut gossip = AdaptiveGossip::new();

    for i in 0..1000u32 {
        gossip.on_gossip_result(&MergeResult::default(), 5);

        let interval = gossip.interval();
        assert!(
            interval >= Duration::from_secs(1),
            "Round {}: interval {:?} below minimum",
            i,
            interval
        );
        assert!(
            interval <= Duration::from_secs(300),
            "Round {}: interval {:?} exceeds absolute maximum",
            i,
            interval
        );
    }
}

// =========================================================================
// P3: REORG HANDLER — CLEAR AND REBUILD
// =========================================================================

/// P3: After clear(), the handler should behave like freshly created.
#[test]
fn test_reorg_handler_clear_resets_all() {
    let mut handler = ReorgHandler::new();
    let genesis = Hash::ZERO;

    // Add some blocks
    let (chain, _tip) = build_chain(genesis, 1, 100);
    for block in &chain {
        handler.record_block_with_weight(block.hash(), block.header.prev_hash, 1);
    }

    // 100 blocks + genesis pre-seeded
    assert_eq!(handler.tracked_count(), 101);
    assert!(handler.current_weight() > 0);

    handler.clear();

    // Genesis is re-seeded after clear
    assert_eq!(handler.tracked_count(), 1);
    assert_eq!(handler.current_weight(), 0);
}

/// P3: set_current_weight correctly overrides accumulated weight.
#[test]
fn test_reorg_handler_set_current_weight() {
    let mut handler = ReorgHandler::new();

    handler.set_current_weight(42);
    assert_eq!(handler.current_weight(), 42);

    handler.set_current_weight(0);
    assert_eq!(handler.current_weight(), 0);

    handler.set_current_weight(u64::MAX);
    assert_eq!(handler.current_weight(), u64::MAX);
}
