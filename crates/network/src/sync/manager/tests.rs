//! Tests for the sync manager

use std::time::{Duration, Instant};

use libp2p::PeerId;

use crypto::Hash;
use doli_core::BlockHeader;

use super::*;
use crate::protocols::SyncResponse;

#[test]
fn test_sync_state_is_syncing() {
    assert!(!SyncState::Idle.is_syncing());
    assert!(!SyncState::Synchronized.is_syncing());
    assert!(SyncState::DownloadingHeaders {
        target_slot: 100,
        peer: PeerId::random(),
        headers_count: 0,
    }
    .is_syncing());
    assert!(SyncState::DownloadingBodies {
        pending: 10,
        total: 100,
    }
    .is_syncing());
    assert!(SyncState::Processing { height: 50 }.is_syncing());
}

#[test]
fn test_sync_manager_creation() {
    let manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    assert!(matches!(*manager.state(), SyncState::Idle));
    assert_eq!(manager.local_tip(), (0, Hash::ZERO, 0));
}

// =========================================================================
// P0 #2: "Ahead of network" detection tests
// Layer 7 (AheadOfPeers) was REMOVED (2026-02-25) — Satoshi principle.
// These tests now verify that production is ALLOWED even when ahead.
// =========================================================================

#[test]
fn test_production_allowed_when_ahead_of_peers() {
    // Layer 7 removed: node at height 992, peers at 910 — should still produce.
    // Forks are resolved by longest chain reorg, not by stopping production.
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 992;
    manager.local_slot = 992;

    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    manager.add_peer(peer1, 910, Hash::ZERO, 910);
    manager.add_peer(peer2, 910, Hash::ZERO, 910);

    manager.has_connected_to_peer = true;
    manager.first_peer_status_received = Some(std::time::Instant::now());

    let result = manager.can_produce(993);
    assert_eq!(result, ProductionAuthorization::Authorized);
}

#[test]
fn test_production_allowed_when_within_range_of_peers() {
    // Scenario: Node at height 912, peers at 910 (only 2 blocks ahead)
    // Should be allowed to produce (within threshold)
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Set local height to 912
    manager.local_height = 912;
    manager.local_slot = 912;

    // Add TWO peers at height 910 to satisfy min_peers_for_production
    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    manager.add_peer(peer1, 910, Hash::ZERO, 910);
    manager.add_peer(peer2, 910, Hash::ZERO, 910);

    // Need to clear bootstrap phase requirements
    manager.has_connected_to_peer = true;
    manager.first_peer_status_received = Some(std::time::Instant::now());

    // Verify: Should be authorized (2 blocks ahead is within default threshold of 5)
    let result = manager.can_produce(913);
    assert_eq!(result, ProductionAuthorization::Authorized);
}

#[test]
fn test_max_heights_ahead_no_longer_blocks() {
    // Layer 7 removed: configurable threshold no longer blocks production.
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    manager.set_max_heights_ahead(2);
    manager.local_height = 915;
    manager.local_slot = 915;

    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    manager.add_peer(peer1, 910, Hash::ZERO, 910);
    manager.add_peer(peer2, 910, Hash::ZERO, 910);

    manager.has_connected_to_peer = true;
    manager.first_peer_status_received = Some(std::time::Instant::now());

    // Even 5 blocks ahead should be authorized now
    let result = manager.can_produce(916);
    assert_eq!(result, ProductionAuthorization::Authorized);
}

// =========================================================================
// Combined scenario tests
// =========================================================================

#[test]
fn test_forked_node_scenario_produces_on_best_chain() {
    // Layer 7 removed (2026-02-25): A node ahead of peers should still produce.
    // If it's truly forked, the longest chain rule will resolve it via reorg.
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 992;
    manager.local_slot = 992;

    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    manager.add_peer(peer1, 910, Hash::ZERO, 910);
    manager.add_peer(peer2, 910, Hash::ZERO, 910);

    manager.has_connected_to_peer = true;
    manager.first_peer_status_received = Some(std::time::Instant::now());

    let result = manager.can_produce(993);
    assert_eq!(
        result,
        ProductionAuthorization::Authorized,
        "Node should produce on its best chain, got: {:?}",
        result
    );
}

// =========================================================================
// Echo chamber prevention tests (P0 #5)
// =========================================================================

#[test]
fn test_insufficient_peers_blocks_production() {
    // Scenario: Node with only 1 peer (echo chamber risk)
    // Should be blocked from producing to prevent isolated cluster forks
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Node at height 100
    manager.local_height = 100;
    manager.local_slot = 100;

    // Only 1 peer - insufficient for safe production
    let peer = PeerId::random();
    manager.add_peer(peer, 100, Hash::ZERO, 100);
    manager.has_connected_to_peer = true;
    manager.first_peer_status_received = Some(std::time::Instant::now());

    let result = manager.can_produce(101);
    match result {
        ProductionAuthorization::BlockedInsufficientPeers {
            peer_count,
            min_required,
        } => {
            assert_eq!(peer_count, 1);
            assert_eq!(min_required, 2);
        }
        other => panic!("Expected BlockedInsufficientPeers, got: {:?}", other),
    }
}

#[test]
fn test_sufficient_peers_allows_production() {
    // Scenario: Node with 2+ peers (safe to produce)
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Node at height 100
    manager.local_height = 100;
    manager.local_slot = 100;

    // 2 peers - sufficient for safe production
    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    manager.add_peer(peer1, 100, Hash::ZERO, 100);
    manager.add_peer(peer2, 100, Hash::ZERO, 100);
    manager.has_connected_to_peer = true;
    manager.first_peer_status_received = Some(std::time::Instant::now());

    let result = manager.can_produce(101);
    assert_eq!(result, ProductionAuthorization::Authorized);
}

#[test]
fn test_insufficient_peers_check_skipped_at_genesis() {
    // Scenario: Node at height 0 (genesis) with only 1 peer
    // Should NOT be blocked by insufficient peers at genesis
    // (there may be legitimate first-producer scenarios)
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Node at height 0 (genesis)
    manager.local_height = 0;
    manager.local_slot = 0;

    // Only 1 peer at genesis
    let peer = PeerId::random();
    manager.add_peer(peer, 0, Hash::ZERO, 0);
    manager.has_connected_to_peer = true;
    manager.first_peer_status_received = Some(std::time::Instant::now());

    let result = manager.can_produce(0);
    // Should NOT be BlockedInsufficientPeers at height 0
    assert!(
        !matches!(
            result,
            ProductionAuthorization::BlockedInsufficientPeers { .. }
        ),
        "Should not block for insufficient peers at genesis, got: {:?}",
        result
    );
}

#[test]
fn test_ahead_of_network_tip_still_produces() {
    // Layer 7 removed (2026-02-25): Node ahead of network_tip should still produce.
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 136;
    manager.local_slot = 136;

    assert!(manager.peers.is_empty());
    manager.network_tip_height = 93;
    manager.network_tip_slot = 93;

    manager.set_min_peers_for_production(0);
    manager.has_connected_to_peer = false;

    let result = manager.can_produce(140);
    // With Layer 7 removed, this should be authorized
    assert!(
        !matches!(result, ProductionAuthorization::BlockedAheadOfPeers { .. }),
        "Layer 7 removed: should not block as AheadOfPeers, got: {:?}",
        result
    );
}

#[test]
fn test_echo_chamber_check_disabled_allows_production_when_peer_behind() {
    // UPDATED TEST (2026-02-04):
    // The "lowest peer" echo chamber check was DISABLED because it caused
    // chain deadlock when peers legitimately fell behind.
    //
    // Scenario: Healthy node has peers at different heights
    // - Node has peers: {peer1: height=93, peer2: height=136}
    // - Node local_height = 136 (same as peer2, ahead of peer1)
    // - OLD: Blocked because 136 - 93 = 43 > 5 (ahead of lowest)
    // - NEW: AUTHORIZED - peer behind is OK, we're not ahead of BEST peer
    //
    // Echo chambers are now detected by other mechanisms:
    // - Sync failures (P0 #4)
    // - InsufficientPeers check (P0 #5)
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Node at height 136
    manager.local_height = 136;
    manager.local_slot = 136;

    // Two peers: one behind (93), one at same height (136)
    let behind_peer = PeerId::random();
    let synced_peer = PeerId::random();
    manager.add_peer(behind_peer, 93, Hash::ZERO, 93);
    manager.add_peer(synced_peer, 136, Hash::ZERO, 136);

    // Mark bootstrap checks as passed
    manager.has_connected_to_peer = true;
    manager.first_peer_status_received = Some(std::time::Instant::now());

    // Verify preconditions
    assert_eq!(manager.peers.len(), 2);
    assert_eq!(manager.best_peer_height(), 136);
    assert_eq!(manager.lowest_peer_height(), Some(93));

    let result = manager.can_produce(140);

    // Should be AUTHORIZED - having a peer behind doesn't mean we're forked
    // The sync failure check and other mechanisms catch actual forks
    match result {
        ProductionAuthorization::Authorized => {
            // Correct - we're not ahead of best peer, peer behind is OK
        }
        other => panic!(
            "Expected Authorized (echo chamber check disabled), got: {:?}",
            other
        ),
    }
}

// =========================================================================
// Slot-aware sync recovery tests (sync stall deadlock fix)
// =========================================================================

#[test]
fn test_should_sync_uses_height_not_slot() {
    // should_sync() uses HEIGHT only (not slot) to prevent forked peers
    // with inflated slots from triggering unnecessary sync.
    // Peer behind in height (834 < 876) but ahead in slot (919 > 261)
    // should NOT trigger sync.
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 876;
    manager.local_slot = 261;

    let peer = PeerId::random();
    manager.peers.insert(
        peer,
        PeerSyncStatus {
            best_height: 834,
            best_hash: Hash::ZERO,
            best_slot: 919,
            last_status_response: Instant::now(),
            last_block_received: None,
            pending_request: None,
        },
    );

    assert!(
        !manager.should_sync(),
        "should_sync() must NOT sync when peer is behind in height (834 < 876), even with higher slot"
    );
}

#[test]
fn test_should_sync_triggers_when_peer_ahead_in_height() {
    // should_sync() triggers when a peer has more blocks (higher height)
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 100;
    manager.local_slot = 100;

    let peer = PeerId::random();
    manager.peers.insert(
        peer,
        PeerSyncStatus {
            best_height: 500,
            best_hash: Hash::ZERO,
            best_slot: 500,
            last_status_response: Instant::now(),
            last_block_received: None,
            pending_request: None,
        },
    );

    assert!(
        manager.should_sync(),
        "should_sync() must trigger when peer is ahead in height (500 > 100)"
    );
}

#[test]
fn test_best_peer_ignores_peer_behind_in_height() {
    // best_peer() only returns peers with MORE BLOCKS (higher height).
    // A peer behind in height but ahead in slot should be ignored.
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 876;
    manager.local_slot = 261;

    let peer = PeerId::random();
    manager.peers.insert(
        peer,
        PeerSyncStatus {
            best_height: 834,
            best_hash: Hash::ZERO,
            best_slot: 919,
            last_status_response: Instant::now(),
            last_block_received: None,
            pending_request: None,
        },
    );

    let result = manager.best_peer();
    assert_eq!(
        result, None,
        "best_peer() must return None when peer is behind in height (834 < 876)"
    );
}

#[test]
fn test_stall_recovery_resets_to_idle() {
    // Scenario: Synchronized state but significantly behind in slots.
    // cleanup() should detect stall and reset to Idle.
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Simulate: height matches but slots diverge (the deadlock scenario)
    manager.local_height = 876;
    manager.local_slot = 261;
    manager.state = SyncState::Synchronized;

    let peer = PeerId::random();
    manager.peers.insert(
        peer,
        PeerSyncStatus {
            best_height: 876,
            best_hash: Hash::ZERO,
            best_slot: 920,
            last_status_response: Instant::now(),
            last_block_received: None,
            pending_request: None,
        },
    );

    // Slot lag = 920 - 261 = 659, threshold = 2 * 5 = 10 → 659 > 10 → stall detected
    manager.cleanup();

    // State should no longer be Synchronized (either Idle or started sync)
    assert!(
        !matches!(manager.state, SyncState::Synchronized),
        "cleanup() must reset Synchronized state when slot lag ({}) exceeds stall threshold",
        920 - 261
    );
}

#[test]
fn test_update_local_tip_requires_slot_alignment() {
    // Scenario: peer at height 100/slot 500, we reach height 100 but only slot 100.
    // update_local_tip should NOT mark us as Synchronized because slots don't align.
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Start in a syncing state
    let peer = PeerId::random();
    manager.peers.insert(
        peer,
        PeerSyncStatus {
            best_height: 100,
            best_hash: Hash::ZERO,
            best_slot: 500,
            last_status_response: Instant::now(),
            last_block_received: None,
            pending_request: None,
        },
    );

    manager.state = SyncState::DownloadingHeaders {
        target_slot: 500,
        peer,
        headers_count: 0,
    };

    // Height matches peer but slot is way behind
    manager.update_local_tip(100, Hash::ZERO, 100);

    // Should NOT be Synchronized because slot lag = 500 - 100 = 400 >> max_slots_behind (2)
    assert!(
        !matches!(manager.state, SyncState::Synchronized),
        "update_local_tip must not mark Synchronized when slot lag is {} (max_slots_behind={})",
        400,
        manager.max_slots_behind
    );
}

#[test]
fn test_processing_stuck_recovery_on_block_applied() {
    // Reproduce: node downloads 58 blocks, applies them all, but network_tip
    // advanced to 59 during processing. Processing state with no pending work
    // should transition to Idle and start a new sync round.
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Simulate: downloaded blocks 1-58, now in Processing state
    manager.state = SyncState::Processing { height: 1 };
    manager.network_tip_height = 59; // Gossip bumped this during processing
    manager.network_tip_slot = 64;

    let peer = PeerId::random();
    manager.peers.insert(
        peer,
        PeerSyncStatus {
            best_height: 59,
            best_hash: Hash::ZERO,
            best_slot: 64,
            last_status_response: Instant::now(),
            last_block_received: None,
            pending_request: None,
        },
    );

    // pending_headers and pending_blocks are empty (all applied)
    assert!(manager.pending_headers.is_empty());
    assert!(manager.pending_blocks.is_empty());

    // Apply the last block (h=58) — completion check fails: 58 < 59
    let hash = crypto::hash::hash(b"block58");
    manager.block_applied_with_weight(hash, 58, 60, 1, Hash::ZERO);

    // Should NOT be stuck in Processing — should have transitioned to Idle or started sync
    assert!(
        !matches!(manager.state, SyncState::Processing { .. }),
        "Must not stay stuck in Processing when no pending work remains (state={:?})",
        manager.state
    );
}

#[test]
fn test_processing_stuck_recovery_via_cleanup() {
    // Safety net: even if block_applied doesn't fire, cleanup() detects
    // a stuck Processing state with no pending work.
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.state = SyncState::Processing { height: 1 };
    manager.local_height = 58;
    manager.local_slot = 60;
    manager.network_tip_height = 65;
    manager.network_tip_slot = 70;
    // Simulate stuck state: no sync activity for >30s
    manager.last_block_applied = Instant::now() - Duration::from_secs(60);
    manager.last_sync_activity = Instant::now() - Duration::from_secs(60);

    let peer = PeerId::random();
    manager.peers.insert(
        peer,
        PeerSyncStatus {
            best_height: 65,
            best_hash: Hash::ZERO,
            best_slot: 70,
            last_status_response: Instant::now(),
            last_block_received: None,
            pending_request: None,
        },
    );

    // No pending work
    assert!(manager.pending_headers.is_empty());
    assert!(manager.pending_blocks.is_empty());

    manager.cleanup();

    assert!(
        !matches!(manager.state, SyncState::Processing { .. }),
        "cleanup() must recover stuck Processing state (state={:?})",
        manager.state
    );
}

// =========================================================================
// Fix verification: concurrent requests and stale response handling
// =========================================================================

fn create_test_header(prev_hash: Hash, slot: u32) -> BlockHeader {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    BlockHeader {
        version: 1,
        prev_hash,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: now,
        slot,
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![0; 32] },
        vdf_proof: vdf::VdfProof::empty(),
    }
}

fn build_header_chain(genesis: Hash, count: usize) -> Vec<BlockHeader> {
    let mut headers = Vec::with_capacity(count);
    let mut prev = genesis;
    for i in 0..count {
        let h = create_test_header(prev, (i + 1) as u32);
        prev = h.hash();
        headers.push(h);
    }
    headers
}

#[test]
fn test_next_request_guard_prevents_duplicate_requests() {
    // Fix 1: next_request() must return None when peer already has pending request
    let genesis = Hash::ZERO;
    let mut manager = SyncManager::new(SyncConfig::default(), genesis);

    let peer = PeerId::random();
    manager.add_peer(peer, 1000, Hash::ZERO, 1000);

    // Trigger sync
    manager.start_sync();
    assert!(matches!(
        manager.state,
        SyncState::DownloadingHeaders { .. }
    ));

    // First request should succeed
    let req1 = manager.next_request();
    assert!(req1.is_some(), "First request should be generated");

    // Second request should be blocked (peer has pending request)
    let req2 = manager.next_request();
    assert!(
        req2.is_none(),
        "Second request must be blocked — peer already has pending request"
    );
}

#[test]
fn test_chain_break_preserves_state_on_stale_response() {
    // Fix 2: A single chain break (stale response) must NOT destroy progress.
    // process_headers() doesn't modify expected_prev_hash when valid_count=0,
    // so the downloader state is still correct — just skip and continue.
    let genesis = Hash::ZERO;
    let mut manager = SyncManager::new(SyncConfig::default(), genesis);

    let peer = PeerId::random();
    manager.add_peer(peer, 1000, Hash::ZERO, 1000);
    manager.start_sync();

    // First: download some valid headers to build up state
    let _ = manager.next_request();
    let chain = build_header_chain(genesis, 5);
    let expected_hash = chain[4].hash();
    let _blocks = manager.handle_response(peer, SyncResponse::Headers(chain));

    // Verify we have progress
    assert!(matches!(
        manager.state,
        SyncState::DownloadingHeaders {
            headers_count: 5,
            ..
        }
    ));

    // Now: simulate a stale response (doesn't chain)
    let _ = manager.next_request();
    let wrong_prev = Hash::from_bytes([0xAB; 32]);
    let bad_headers = vec![create_test_header(wrong_prev, 1)];
    let _blocks = manager.handle_response(peer, SyncResponse::Headers(bad_headers));

    // Verify: state STAYS in DownloadingHeaders (not reset to Idle)
    assert!(
        matches!(manager.state, SyncState::DownloadingHeaders { .. }),
        "Stale response must NOT reset state — got {:?}",
        manager.state
    );
    // Chain break correctly incremented as fork evidence
    assert_eq!(manager.consecutive_empty_headers, 1);
    // Verify: expected_prev_hash PRESERVED (not cleared)
    assert_eq!(
        manager.header_downloader.expected_prev_hash(),
        Some(expected_hash),
        "expected_prev_hash must be preserved after stale response"
    );
}

#[test]
fn test_start_sync_clears_header_downloader() {
    // Fix 3: start_sync() must clear stale expected_prev_hash
    let genesis = Hash::ZERO;
    let mut manager = SyncManager::new(SyncConfig::default(), genesis);

    let peer = PeerId::random();
    manager.add_peer(peer, 1000, Hash::ZERO, 1000);

    // Poison the header downloader with a stale expected_prev_hash
    let chain = build_header_chain(genesis, 5);
    manager.header_downloader.process_headers(&chain, genesis);
    assert!(
        manager.header_downloader.expected_prev_hash().is_some(),
        "Setup: expected_prev_hash should be set after processing headers"
    );

    // start_sync must clear it
    manager.start_sync();
    assert_eq!(
        manager.header_downloader.expected_prev_hash(),
        None,
        "start_sync() must clear expected_prev_hash for a clean slate"
    );
}

#[test]
fn test_stale_response_discarded_when_no_pending_request() {
    // Fix 4: responses with no matching pending_request must be discarded
    let genesis = Hash::ZERO;
    let mut manager = SyncManager::new(SyncConfig::default(), genesis);

    let peer = PeerId::random();
    manager.add_peer(peer, 1000, Hash::ZERO, 1000);
    manager.start_sync();

    // Send request and consume response (clears pending_request)
    let _ = manager.next_request();
    let chain = build_header_chain(genesis, 5);
    let _blocks = manager.handle_response(peer, SyncResponse::Headers(chain.clone()));

    // Now send a second (stale) response — no pending_request exists
    let stale_chain = build_header_chain(genesis, 3);
    let _blocks = manager.handle_response(peer, SyncResponse::Headers(stale_chain));

    // The stale response reached the handler but its headers don't chain to our tip.
    // This correctly counts as fork evidence (chain break path).
    assert_eq!(manager.consecutive_empty_headers, 1);
}

// =========================================================================
// Production Gate Deadlock (PGD) — Reproduction & Fix Verification Tests
// REQ-PGD-001 through REQ-PGD-008
// =========================================================================

/// REQ-PGD-001: reset_resync_counter() is dead code — counter never resets.
/// This test FAILS before the fix (counter stays at 5 after stable blocks).
#[test]
fn test_pgd001_resync_counter_resets_after_stable_blocks() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Simulate 5 consecutive resyncs
    for _ in 0..5 {
        manager.start_resync();
        manager.complete_resync();
    }
    assert_eq!(
        manager.consecutive_resync_count(),
        5,
        "Setup: should have 5 consecutive resyncs"
    );

    // Now simulate stable operation: apply 5 canonical blocks
    manager.has_connected_to_peer = true;
    manager.first_peer_status_received = Some(Instant::now());
    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    manager.add_peer(peer1, 100, Hash::ZERO, 100);
    manager.add_peer(peer2, 100, Hash::ZERO, 100);

    for i in 1..=5 {
        let hash = crypto::hash::hash(format!("stable_block_{}", i).as_bytes());
        manager.block_applied_with_weight(hash, i, i as u32, 1, Hash::ZERO);
    }

    // After 5 stable blocks, counter should reset to 0
    assert_eq!(
        manager.consecutive_resync_count(),
        0,
        "REQ-PGD-001: consecutive_resync_count must reset to 0 after 5 stable blocks"
    );
}

/// REQ-PGD-001: Counter must NOT reset during active resync
#[test]
fn test_pgd001_counter_not_reset_during_active_resync() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Resync 3 times, then start a 4th (still in progress)
    for _ in 0..3 {
        manager.start_resync();
        manager.complete_resync();
    }
    manager.start_resync(); // 4th resync in progress
    assert!(manager.is_resync_in_progress());
    assert_eq!(manager.consecutive_resync_count(), 4);

    // Apply blocks during active resync — counter should NOT reset
    for i in 1..=5 {
        let hash = crypto::hash::hash(format!("sync_block_{}", i).as_bytes());
        manager.block_applied_with_weight(hash, i, i as u32, 1, Hash::ZERO);
    }

    assert!(
        manager.consecutive_resync_count() > 0,
        "Counter must NOT reset while resync is in progress"
    );
}

/// REQ-PGD-002: Grace period must be capped at 60s
#[test]
fn test_pgd002_grace_period_capped() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Set local state FIRST (before adding peers, to prevent sync trigger)
    manager.local_height = 100;
    manager.local_slot = 100;
    let local_hash = crypto::hash::hash(b"block_100");
    manager.local_hash = local_hash;

    // Set up bootstrap + peers at SAME height (no sync trigger)
    manager.has_connected_to_peer = true;
    manager.first_peer_status_received = Some(Instant::now());
    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    manager.add_peer(peer1, 100, local_hash, 100);
    manager.add_peer(peer2, 100, local_hash, 100);

    // Simulate 5 resyncs → uncapped would be 30 * 2^4 = 480s
    for _ in 0..5 {
        manager.start_resync();
        manager.complete_resync();
    }

    // Set last_resync_completed to just now (grace period active)
    manager.last_resync_completed = Some(Instant::now());

    // Check can_produce — should be blocked, but remaining grace should be ≤ 60s, NOT 480s
    let result = manager.can_produce(101);
    match result {
        ProductionAuthorization::BlockedResync {
            grace_remaining_secs,
        } => {
            assert!(
                grace_remaining_secs <= 60,
                "REQ-PGD-002: Grace period must be capped at 60s, got {}s (uncapped would be 480s)",
                grace_remaining_secs
            );
        }
        other => panic!("Expected BlockedResync with capped grace, got: {:?}", other),
    }
}

/// REQ-PGD-003: Circuit breaker bypassed when all peers agree on same height (network stall).
/// Before the fix: circuit breaker fires and locks permanently. After: bypasses to break stall.
#[test]
fn test_pgd003_circuit_breaker_bypassed_when_peers_agree() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Setup: node at height 100, 2 peers at height 100, all agree
    manager.local_height = 100;
    manager.local_slot = 100;
    manager.has_connected_to_peer = true;
    manager.first_peer_status_received = Some(Instant::now());

    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    let same_hash = crypto::hash::hash(b"canonical_block_100");
    manager.local_hash = same_hash;
    manager.add_peer(peer1, 100, same_hash, 100);
    manager.add_peer(peer2, 100, same_hash, 100);

    // Gossip silent for 60s (exceeds 50s threshold). All peers at same height.
    // Before fix: BlockedNoGossipActivity (permanent deadlock).
    // After fix: Authorized (network stall bypass).
    manager.last_block_received_via_gossip = Some(Instant::now() - Duration::from_secs(60));

    let result = manager.can_produce(101);
    assert_eq!(
        result,
        ProductionAuthorization::Authorized,
        "REQ-PGD-003: Circuit breaker must bypass when all peers at same height (network stall)"
    );
}

/// REQ-PGD-003: Circuit breaker still fires when peers are at DIFFERENT heights (genuine isolation)
#[test]
fn test_pgd003_circuit_breaker_fires_when_peers_at_different_heights() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Setup: node at height 100, peers at different heights (99 and 100)
    manager.local_height = 100;
    manager.local_slot = 100;
    manager.has_connected_to_peer = true;
    manager.first_peer_status_received = Some(Instant::now());

    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    let same_hash = crypto::hash::hash(b"block_100");
    manager.local_hash = same_hash;
    manager.add_peer(peer1, 100, same_hash, 100);
    manager.add_peer(peer2, 99, Hash::ZERO, 99); // Peer at different height

    // Gossip silent for 60s. Not all peers at our height → genuine isolation.
    manager.last_block_received_via_gossip = Some(Instant::now() - Duration::from_secs(60));

    let result = manager.can_produce(101);
    assert!(
        matches!(
            result,
            ProductionAuthorization::BlockedNoGossipActivity { .. }
        ),
        "Circuit breaker must fire when peers are at different heights, got: {:?}",
        result
    );
}

/// REQ-PGD-003: Circuit breaker must NOT recover when node is behind peers
#[test]
fn test_pgd003_circuit_breaker_stays_locked_when_behind() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Setup: node at height 100, peers at height 105
    manager.local_height = 100;
    manager.local_slot = 100;
    manager.has_connected_to_peer = true;
    manager.first_peer_status_received = Some(Instant::now());

    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    manager.add_peer(peer1, 105, Hash::ZERO, 105);
    manager.add_peer(peer2, 105, Hash::ZERO, 105);

    // Trigger gossip silence
    manager.last_block_received_via_gossip = Some(Instant::now() - Duration::from_secs(200));

    // Should be blocked (peers ahead, not a network stall)
    let result = manager.can_produce(101);
    assert!(
        !matches!(result, ProductionAuthorization::Authorized),
        "Circuit breaker must NOT recover when node is behind peers, got: {:?}",
        result
    );
}

/// REQ-PGD-003/RC-2: Demonstrate the current deadlock — circuit breaker counter only grows.
/// This test documents the bug: silence_secs at 100s, 200s, 300s all blocked, no recovery.
#[test]
fn test_pgd_circuit_breaker_deadlock_demonstrated() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 100;
    manager.local_slot = 100;
    manager.has_connected_to_peer = true;
    manager.first_peer_status_received = Some(Instant::now());

    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    let same_hash = crypto::hash::hash(b"block100");
    manager.local_hash = same_hash;
    manager.add_peer(peer1, 100, same_hash, 100);
    manager.add_peer(peer2, 100, same_hash, 100);

    // All peers agree. Simulate growing gossip silence.
    for silence in [60, 120, 300, 600, 2500] {
        manager.last_block_received_via_gossip =
            Some(Instant::now() - Duration::from_secs(silence));
        let result = manager.can_produce(101);

        // After fix: at least ONE of these should return Authorized (recovery retry)
        // Before fix: ALL return BlockedNoGossipActivity (permanent deadlock)
        if silence >= 90 {
            // 90s = 60s initial + 30s retry period
            assert_eq!(
                result,
                ProductionAuthorization::Authorized,
                "REQ-PGD-003: At {}s silence with peers agreeing, circuit breaker must allow retry",
                silence
            );
            break; // First recovery attempt should succeed
        }
    }
}

/// REQ-PGD-008: Cross-layer interaction — resync grace + circuit breaker cascade
#[test]
fn test_pgd008_cross_layer_deadlock_scenario() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.has_connected_to_peer = true;
    manager.first_peer_status_received = Some(Instant::now());
    manager.local_height = 37406;
    manager.local_slot = 38874;

    let same_hash = crypto::hash::hash(b"last_block_37406");
    manager.local_hash = same_hash;

    // Add 11 peers all at same height (perfect consensus)
    for _ in 0..11 {
        let peer = PeerId::random();
        manager.add_peer(peer, 37406, same_hash, 38874);
    }

    // Simulate gossip silence (no blocks for 70s)
    manager.last_block_received_via_gossip = Some(Instant::now() - Duration::from_secs(70));

    let result = manager.can_produce(38875);

    // With 11 peers all at same height, the chain is healthy but stalled.
    // The circuit breaker must NOT permanently lock production in this case.
    assert_eq!(
        result,
        ProductionAuthorization::Authorized,
        "REQ-PGD-008: With 11 peers at identical height/hash (network stall), production must be allowed. \
         Got {:?} — this is the exact deadlock that killed testnet at h=37406.",
        result
    );
}

#[test]
fn test_full_concurrent_scenario_no_corruption() {
    // Integration test: simulates the exact production scenario that caused the bug.
    // 1. Sync starts, peer has 100 blocks
    // 2. Due to Fix 1, only ONE request goes out (not 10)
    // 3. Response arrives with valid headers
    // 4. Next request goes out for the continuation
    // 5. Second response arrives — chain continues correctly
    let genesis = Hash::ZERO;
    let mut manager = SyncManager::new(SyncConfig::default(), genesis);

    let peer = PeerId::random();
    let full_chain = build_header_chain(genesis, 10);
    let tip_hash = full_chain.last().unwrap().hash();
    manager.add_peer(peer, 10, tip_hash, 100);
    manager.start_sync();

    // Round 1: request + response
    let req1 = manager.next_request();
    assert!(req1.is_some());
    // Guard: no second request while first is pending
    assert!(manager.next_request().is_none());

    let batch1 = full_chain[..5].to_vec();
    let _blocks = manager.handle_response(peer, SyncResponse::Headers(batch1));

    // After response processed: state should still be DownloadingHeaders
    // and expected_prev_hash should be at header 5
    let _expected_hash = full_chain[4].hash();
    if let SyncState::DownloadingHeaders { headers_count, .. } = manager.state {
        assert_eq!(headers_count, 5, "Should have 5 headers counted");
    } else {
        panic!("Expected DownloadingHeaders state");
    }

    // Round 2: continuation request
    let req2 = manager.next_request();
    assert!(req2.is_some(), "Should be able to request more headers");

    let batch2 = full_chain[5..10].to_vec();
    let _blocks = manager.handle_response(peer, SyncResponse::Headers(batch2));

    if let SyncState::DownloadingHeaders { headers_count, .. } = manager.state {
        assert_eq!(headers_count, 10, "Should have all 10 headers counted");
    } else {
        panic!("Expected DownloadingHeaders state");
    }

    // Verify: no empty headers (no fork detection triggered)
    assert_eq!(manager.consecutive_empty_headers, 0);
}
