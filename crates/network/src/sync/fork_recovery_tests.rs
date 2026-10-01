// covers: fork_recovery
use super::*;

fn make_block(prev_hash: Hash, slot: u32) -> Block {
    let header = doli_core::BlockHeader {
        version: 1,
        prev_hash,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: slot as u64 * 10,
        slot,
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![])
}

#[test]
fn test_start_and_walk_parents() {
    let mut tracker = ForkRecoveryTracker::new();
    let peer = PeerId::random();

    // Build a chain: genesis → A → B → C (orphan tip)
    let genesis = Hash::ZERO;
    let block_a = make_block(genesis, 1);
    let hash_a = block_a.hash();
    let block_b = make_block(hash_a, 2);
    let hash_b = block_b.hash();
    let block_c = make_block(hash_b, 3);

    // Start with orphan C
    assert!(tracker.start(block_c, peer));
    assert!(tracker.is_active());

    // Fetch parent of C → should be hash_b
    let (_, fetch_hash) = tracker.next_fetch().unwrap();
    assert_eq!(fetch_hash, hash_b);

    // Feed block B
    assert!(tracker.handle_block(peer, Some(block_b)));

    // Not connected yet (genesis not in "store")
    assert!(tracker.check_connection(false).is_none());

    // Fetch parent of B → should be hash_a
    let (_, fetch_hash) = tracker.next_fetch().unwrap();
    assert_eq!(fetch_hash, hash_a);

    // Feed block A
    assert!(tracker.handle_block(peer, Some(block_a)));

    // Parent of A is genesis — "in store"
    let completed = tracker.check_connection(true).unwrap();
    assert_eq!(completed.connection_point, genesis);
    assert_eq!(completed.blocks.len(), 3); // A, B, C in forward order
    assert_eq!(completed.blocks[0].header.slot, 1); // A first
    assert_eq!(completed.blocks[2].header.slot, 3); // C last
    assert!(!tracker.is_active());
}

#[test]
fn test_depth_limit_cancels() {
    let mut tracker = ForkRecoveryTracker::new();
    let peer = PeerId::random();

    // Orphan + MAX_RECOVERY_DEPTH real ancestors: the last one overflows the bound.
    let mut chain = vec![make_block(crypto::hash::hash(b"root"), 1)];
    for i in 0..MAX_RECOVERY_DEPTH {
        let prev = chain.last().unwrap().hash();
        chain.push(make_block(prev, i as u32 + 2));
    }
    let orphan = chain.pop().unwrap();
    assert!(tracker.start(orphan, peer));

    for _ in 1..MAX_RECOVERY_DEPTH {
        let (_, hash) = tracker.next_fetch().unwrap();
        let parent = chain.pop().unwrap();
        assert_eq!(parent.hash(), hash);
        assert!(tracker.handle_block(peer, Some(parent)));
        assert!(tracker.is_active());
    }
    let (_, hash) = tracker.next_fetch().unwrap();
    let parent = chain.pop().unwrap();
    assert_eq!(parent.hash(), hash);
    assert!(tracker.handle_block(peer, Some(parent)));
    assert!(!tracker.is_active(), "depth > MAX_RECOVERY_DEPTH cancels");
    assert!(tracker.take_exceeded_max_depth());
}

#[test]
fn test_cooldown_blocks_restart() {
    let mut tracker = ForkRecoveryTracker::new();
    let peer = PeerId::random();

    let orphan = make_block(Hash::ZERO, 1);
    assert!(tracker.start(orphan.clone(), peer));

    // Complete the recovery
    let _ = tracker.check_connection(true);
    assert!(!tracker.is_active());

    // Try to start again immediately — should fail (cooldown)
    assert!(!tracker.start(orphan, peer));
    assert!(!tracker.can_start());
}

#[test]
fn test_peer_missing_block_cancels() {
    let mut tracker = ForkRecoveryTracker::new();
    let peer = PeerId::random();

    let orphan = make_block(crypto::hash::hash(b"unknown"), 5);
    assert!(tracker.start(orphan, peer));

    let _ = tracker.next_fetch().unwrap();

    // Peer doesn't have the block
    assert!(tracker.handle_block(peer, None));
    assert!(!tracker.is_active()); // cancelled
}

#[test]
fn test_wrong_hash_passes_through() {
    let mut tracker = ForkRecoveryTracker::new();
    let peer = PeerId::random();

    let orphan = make_block(crypto::hash::hash(b"expected_parent"), 5);
    assert!(tracker.start(orphan, peer));

    let _ = tracker.next_fetch().unwrap();

    // INC-I-235 C2: an unrelated block is not consumed and the walk stays pending.
    let wrong_block = make_block(Hash::ZERO, 99);
    assert!(!tracker.handle_block(peer, Some(wrong_block)));
    assert!(tracker.is_active());
    assert_eq!(tracker.current_parent(), None);
    assert!(
        tracker.next_fetch().is_none(),
        "still one request in flight"
    );
}

#[test]
fn test_rejected_branch_memo_is_keyed_on_both_tips() {
    let mut tracker = ForkRecoveryTracker::new();
    let (branch, tip, other) = (
        crypto::hash::hash(b"branch"),
        crypto::hash::hash(b"tip"),
        crypto::hash::hash(b"other"),
    );
    assert!(!tracker.is_branch_rejected(branch, tip));
    tracker.mark_branch_rejected(branch, tip);
    tracker.mark_branch_rejected(branch, tip);
    assert_eq!(
        tracker.rejected.len(),
        1,
        "duplicate verdicts are not re-recorded"
    );
    assert!(tracker.is_branch_rejected(branch, tip));
    assert!(!tracker.is_branch_rejected(branch, other));
    for i in 0..MAX_REJECTED_BRANCHES {
        tracker.mark_branch_rejected(crypto::hash::hash(&i.to_le_bytes()), tip);
    }
    assert!(
        !tracker.is_branch_rejected(branch, tip),
        "bounded: oldest evicted"
    );
}

#[test]
fn test_not_consumed_when_inactive() {
    let mut tracker = ForkRecoveryTracker::new();
    let peer = PeerId::random();

    let block = make_block(Hash::ZERO, 1);
    // No active recovery — handle_block should return false
    assert!(!tracker.handle_block(peer, Some(block)));
}
