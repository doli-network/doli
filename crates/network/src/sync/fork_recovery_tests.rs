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

// covers: fork_recovery
// INC-I-235 M4 — walk session deadline measured from last progress.

fn age(t: &mut ForkRecoveryTracker, by: Duration) {
    let r = t.active.as_mut().expect("active session");
    r.last_progress_at = r
        .last_progress_at
        .checked_sub(by)
        .expect("instant underflow");
    if let Some(sent) = r.request_sent_at {
        r.request_sent_at = Some(sent.checked_sub(by).expect("instant underflow"));
    }
}

/// Returns (connection root, ancestors oldest-first, orphan tip).
fn m4_chain(depth: usize) -> (Hash, Vec<Block>, Block) {
    let root = crypto::hash::hash(b"m4-root");
    let mut prev = root;
    let mut ancestors = Vec::with_capacity(depth);
    for i in 0..depth {
        let blk = make_block(prev, i as u32 + 1);
        prev = blk.hash();
        ancestors.push(blk);
    }
    (root, ancestors, make_block(prev, depth as u32 + 1))
}

fn m4_feed(t: &mut ForkRecoveryTracker, ancestors: &mut Vec<Block>, steps: usize) -> bool {
    for _ in 0..steps {
        age(t, Duration::from_secs(1));
        let Some((p, h)) = t.next_fetch() else {
            return false;
        };
        let Some(blk) = ancestors.pop() else {
            return false;
        };
        if blk.hash() != h || !t.handle_block(p, Some(blk)) {
            return false;
        }
    }
    true
}

fn walk_steadily(depth: usize) -> bool {
    assert!(
        depth < MAX_RECOVERY_DEPTH,
        "orphan + depth must fit the bound"
    );
    let mut t = ForkRecoveryTracker::new();
    let (root, mut ancestors, orphan) = m4_chain(depth);
    assert!(t.start(orphan, PeerId::random()));
    if !m4_feed(&mut t, &mut ancestors, depth) || !t.is_active() {
        return false;
    }
    t.check_connection(true)
        .map(|c| c.blocks.len() == depth + 1 && c.connection_point == root)
        .unwrap_or(false)
}

#[test]
fn m4_repro_walk_depth_100_completes() {
    // REQ-I235-016 — Decision: a failure means even a shallow walk (100 s) no longer completes.
    assert!(walk_steadily(100));
}

#[test]
fn m4_repro_walk_depth_200_completes() {
    // REQ-I235-016 — Decision: a failure means the 120 s cumulative cap still kills progressing walks.
    assert!(walk_steadily(200));
}

#[test]
fn m4_repro_walk_depth_500_completes() {
    // REQ-I235-016 — Decision: a failure means mid-depth forks still cannot be walked at 1 block/s.
    assert!(walk_steadily(500));
}

#[test]
fn m4_repro_walk_depth_999_completes() {
    // REQ-I235-016 — Decision: a failure means a walk at the MAX_RECOVERY_DEPTH edge is time-capped.
    assert!(walk_steadily(MAX_RECOVERY_DEPTH - 1));
}

#[test]
fn m4_no_progress_session_expires_in_next_fetch() {
    // REQ-I235-016 — Decision: a failure means a session with no progress can live forever.
    let mut t = ForkRecoveryTracker::new();
    let (_, _, orphan) = m4_chain(3);
    assert!(t.start(orphan, PeerId::random()));
    assert!(t.next_fetch().is_some());
    age(&mut t, Duration::from_secs(121));
    assert!(t.next_fetch().is_none());
    assert!(!t.is_active());
}

#[test]
fn m4_progress_then_stall_expires() {
    // REQ-I235-016 — Decision: a failure means one early block grants a stalled walk unbounded life.
    let mut t = ForkRecoveryTracker::new();
    let (_, mut ancestors, orphan) = m4_chain(10);
    assert!(t.start(orphan, PeerId::random()));
    assert!(m4_feed(&mut t, &mut ancestors, 5));
    assert!(t.is_active());
    age(&mut t, Duration::from_secs(121));
    assert!(t.next_fetch().is_none());
    assert!(!t.is_active());
    assert!(!t.can_start(), "cancel arms the cooldown");
}

#[test]
fn m4_stale_session_expires_via_start_and_can_start() {
    // REQ-I235-016 — Decision: a failure means a stale session blocks start() or skips the cooldown.
    let mut t = ForkRecoveryTracker::new();
    let (_, _, orphan) = m4_chain(3);
    assert!(t.start(orphan, PeerId::random()));
    age(&mut t, Duration::from_secs(121));
    let other = make_block(crypto::hash::hash(b"m4-other"), 77);
    assert!(
        !t.start(other, PeerId::random()),
        "cooldown armed by expiry"
    );
    assert!(!t.is_active(), "stale session dropped");
    assert!(!t.can_start());
}

#[test]
fn m4_progress_within_timeout_keeps_session_alive() {
    // REQ-I235-016 — Decision: a failure means the deadline is still measured from session start.
    let mut t = ForkRecoveryTracker::new();
    let (_, mut ancestors, orphan) = m4_chain(10);
    assert!(t.start(orphan, PeerId::random()));
    assert!(m4_feed(&mut t, &mut ancestors, 3));
    age(&mut t, Duration::from_secs(119));
    assert!(
        t.next_fetch().is_some(),
        "119 s after last progress is alive"
    );
    assert!(t.is_active());
}

#[test]
fn m4_depth_limit_still_hard_bound() {
    // REQ-I235-016 — Decision: a failure means the timeout change removed the depth bound.
    let mut t = ForkRecoveryTracker::new();
    let (_, mut ancestors, orphan) = m4_chain(MAX_RECOVERY_DEPTH);
    assert!(t.start(orphan, PeerId::random()));
    assert!(m4_feed(&mut t, &mut ancestors, MAX_RECOVERY_DEPTH - 1));
    assert!(t.is_active());
    assert!(m4_feed(&mut t, &mut ancestors, 1));
    assert!(
        !t.is_active(),
        "orphan + 1000 ancestors > MAX_RECOVERY_DEPTH"
    );
    assert!(t.take_exceeded_max_depth());
}
