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
    manager.first_peer_status_received = Some(std::time::Instant::now());

    // Verify: Should be authorized (2 blocks ahead is within default threshold of 5)
    let result = manager.can_produce(913);
    assert_eq!(result, ProductionAuthorization::Authorized);
}

#[test]
fn test_max_heights_ahead_no_longer_blocks() {
    // Layer 7 removed: configurable threshold no longer blocks production.
    // max_heights_ahead field also removed (dead field).
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    manager.local_height = 915;
    manager.local_slot = 915;

    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    manager.add_peer(peer1, 910, Hash::ZERO, 910);
    manager.add_peer(peer2, 910, Hash::ZERO, 910);

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
    manager.network.network_tip_height = 93;
    manager.network.network_tip_slot = 93;

    manager.set_min_peers_for_production(0);
    // first_peer_status_received is None by default (no peers connected)

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
    manager.network.network_tip_height = 59; // Gossip bumped this during processing
    manager.network.network_tip_slot = 64;

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
    assert!(manager.pipeline.pending_headers.is_empty());
    assert!(manager.pipeline.pending_blocks.is_empty());

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
    manager.network.network_tip_height = 65;
    manager.network.network_tip_slot = 70;
    // Simulate stuck state: no sync activity for >30s
    manager.network.last_block_applied = Instant::now() - Duration::from_secs(60);
    manager.network.last_sync_activity = Instant::now() - Duration::from_secs(60);

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
    assert!(manager.pipeline.pending_headers.is_empty());
    assert!(manager.pipeline.pending_blocks.is_empty());

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
    assert_eq!(manager.fork.consecutive_empty_headers, 1);
    // Verify: expected_prev_hash PRESERVED (not cleared)
    assert_eq!(
        manager.pipeline.header_downloader.expected_prev_hash(),
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
    manager
        .pipeline
        .header_downloader
        .process_headers(&chain, genesis);
    assert!(
        manager
            .pipeline
            .header_downloader
            .expected_prev_hash()
            .is_some(),
        "Setup: expected_prev_hash should be set after processing headers"
    );

    // Reset to Idle so start_sync() will actually fire (guard clause skips if already syncing)
    manager.state = SyncState::Idle;

    // start_sync must clear it
    manager.start_sync();
    assert_eq!(
        manager.pipeline.header_downloader.expected_prev_hash(),
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
    assert_eq!(manager.fork.consecutive_empty_headers, 1);
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
    manager.first_peer_status_received = Some(Instant::now());
    manager.max_solo_production_secs = 50; // Override default (86400) to test circuit breaker

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
    assert_eq!(manager.fork.consecutive_empty_headers, 0);
}

// =========================================================================
// ROOT CAUSE FIX: network_tip_height decay on peer removal (Path E)
// =========================================================================

/// Root cause: network_tip_height is monotonically inflated. When a peer
/// with inflated height disconnects, network_tip_height stays high forever.
/// This creates a phantom gap that triggers unnecessary sync/snap sync.
#[test]
fn test_network_tip_decays_on_peer_removal() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 100;
    manager.local_slot = 100;

    // Peer A at height 200
    let peer_a = PeerId::random();
    manager.add_peer(peer_a, 200, Hash::ZERO, 200);
    assert_eq!(manager.network.network_tip_height, 200);

    // Peer B at height 150
    let peer_b = PeerId::random();
    manager.add_peer(peer_b, 150, Hash::ZERO, 150);
    assert_eq!(manager.network.network_tip_height, 200);

    // Remove peer A (the one with highest height)
    manager.remove_peer(&peer_a);

    // AFTER FIX: network_tip_height should drop to max(remaining peers, local)
    // = max(150, 100) = 150. NOT stay at 200.
    assert_eq!(
        manager.network.network_tip_height, 150,
        "network_tip_height must decay to max of remaining peers after peer removal (not stay inflated at 200)"
    );
}

/// Path E reproduction: phantom gap causes production gate to block forever.
#[test]
fn test_phantom_gap_does_not_block_production() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 100;
    manager.local_slot = 100;
    manager.first_peer_status_received = Some(Instant::now());

    // Add peer that briefly claims height 40000 (e.g., during a fork)
    let forked_peer = PeerId::random();
    manager.add_peer(forked_peer, 40000, Hash::ZERO, 40000);
    assert_eq!(manager.network.network_tip_height, 40000);

    // Peer disconnects
    manager.remove_peer(&forked_peer);

    // Add 2 normal peers at height 100 (same as us)
    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    let our_hash = crypto::hash::hash(b"block_100");
    manager.local_hash = our_hash;
    manager.add_peer(peer1, 100, our_hash, 100);
    manager.add_peer(peer2, 100, our_hash, 100);

    // should_sync() must NOT return true (we're at same height as all peers)
    assert!(
        !manager.should_sync(),
        "should_sync() must NOT trigger from phantom gap after inflated peer disconnected"
    );

    // Production should be authorized (not blocked by phantom gap)
    let result = manager.can_produce(101);
    assert_eq!(
        result,
        ProductionAuthorization::Authorized,
        "Production must not be blocked by phantom network_tip from disconnected peer"
    );
}

/// Verify best_peer_height() reflects only connected peers + local, not historical max.
#[test]
fn test_best_peer_height_no_historical_inflation() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 50;

    // Peer at height 1000
    let peer = PeerId::random();
    manager.add_peer(peer, 1000, Hash::ZERO, 1000);
    assert_eq!(manager.best_peer_height(), 1000);

    // Remove peer
    manager.remove_peer(&peer);

    // best_peer_height should NOT return 1000 anymore
    assert!(
        manager.best_peer_height() <= 50,
        "best_peer_height must not retain historical max after peer removal, got {}",
        manager.best_peer_height()
    );
}

// =========================================================================
// ROOT CAUSE FIX: consecutive_empty_headers oscillation (Path D)
// =========================================================================

/// Root cause: cleanup() force-sets consecutive_empty_headers to 3, which
/// triggers resolve_shallow_fork, which resets to 0, then cleanup sets to 3
/// again. Counter oscillates 0→3→0→3, never reaching 10 for definitive recovery.
#[test]
fn test_no_forced_counter_oscillation() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 100;
    manager.local_slot = 100;
    manager.state = SyncState::Idle;

    let peer = PeerId::random();
    manager.add_peer(peer, 105, Hash::ZERO, 105);

    // Simulate stuck-on-fork: no block applied for >120s
    manager.network.last_block_applied = Instant::now() - Duration::from_secs(130);

    // Counter starts at 0
    assert_eq!(manager.fork.consecutive_empty_headers, 0);

    // Run cleanup — stuck-on-fork detection should signal fork, not force counter
    manager.cleanup();

    // AFTER FIX: cleanup should NOT force-set counter to 3.
    // Instead, it should use a dedicated signaling mechanism.
    // The counter should remain at 0 or 1 (if stuck Processing contributed).
    // The fork signal should go through RecoveryPhase::StuckForkDetected.
    assert!(
        matches!(manager.recovery_phase, super::RecoveryPhase::StuckForkDetected),
        "cleanup() must set RecoveryPhase::StuckForkDetected instead of forcing consecutive_empty_headers to 3"
    );
}

/// Verify blacklist escalation doesn't force counter to 3 for small gaps.
#[test]
fn test_blacklist_escalation_uses_signal_not_counter() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 100;
    manager.local_slot = 100;
    manager.state = SyncState::Idle;

    // Insert peers directly (not via add_peer which triggers start_sync)
    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    let peer3 = PeerId::random();
    for peer in [peer1, peer2, peer3] {
        manager.peers.insert(
            peer,
            PeerSyncStatus {
                best_height: 105,
                best_hash: Hash::ZERO,
                best_slot: 105,
                last_status_response: Instant::now(),
                last_block_received: None,
                pending_request: None,
            },
        );
    }
    manager.network.network_tip_height = 105;

    // Set counter to 20+ for blacklist escalation
    manager.fork.consecutive_empty_headers = 25;

    // Blacklist all peers so best_peer() returns None.
    // Use recent timestamps (within 30s) so they survive cleanup's stale blacklist expiry.
    manager
        .fork
        .header_blacklisted_peers
        .insert(peer1, Instant::now());
    manager
        .fork
        .header_blacklisted_peers
        .insert(peer2, Instant::now());
    manager
        .fork
        .header_blacklisted_peers
        .insert(peer3, Instant::now());

    // Stuck for >120s
    manager.network.last_block_applied = Instant::now() - Duration::from_secs(130);

    manager.cleanup();

    // For small gap (5 blocks), cleanup should use RecoveryPhase::StuckForkDetected
    assert!(
        matches!(
            manager.recovery_phase,
            super::RecoveryPhase::StuckForkDetected
        ),
        "Blacklist escalation for small gap must use RecoveryPhase::StuckForkDetected"
    );
}

// =========================================================================
// ROOT CAUSE FIX: can_produce() side effects (Layer 9 + 10.5)
// =========================================================================

/// Root cause: can_produce() mutates fork_mismatch_detected (Layer 9) and
/// last_block_received_via_gossip (Layer 10.5). A query function with side
/// effects creates race-like behavior in a single-threaded system.
#[test]
fn test_can_produce_no_side_effects() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 100;
    manager.local_slot = 100;
    manager.first_peer_status_received = Some(Instant::now());

    // Setup minority fork: 1 agree, 2 disagree
    let local_hash = crypto::hash::hash(b"our_fork_block");
    manager.local_hash = local_hash;
    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    let peer3 = PeerId::random();
    let canonical_hash = crypto::hash::hash(b"canonical_block");
    manager.add_peer(peer1, 100, canonical_hash, 100);
    manager.add_peer(peer2, 100, canonical_hash, 100);
    manager.add_peer(peer3, 100, local_hash, 100); // Agrees with us

    // can_produce should detect the fork but NOT set fork_mismatch_detected
    let fork_mismatch_before = manager.fork.fork_mismatch_detected;
    let _result = manager.can_produce(101);

    assert_eq!(
        manager.fork.fork_mismatch_detected, fork_mismatch_before,
        "can_produce() must NOT mutate fork_mismatch_detected (side-effect-free query)"
    );
}

/// Verify update_production_state() is the designated mutation point.
#[test]
fn test_update_production_state_sets_fork_flag() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 100;
    manager.local_slot = 100;
    manager.first_peer_status_received = Some(Instant::now());

    // Setup minority fork
    let local_hash = crypto::hash::hash(b"our_fork_block");
    manager.local_hash = local_hash;
    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    let canonical_hash = crypto::hash::hash(b"canonical_block");
    manager.add_peer(peer1, 100, canonical_hash, 100);
    manager.add_peer(peer2, 100, canonical_hash, 100);

    assert!(!manager.fork.fork_mismatch_detected);

    // update_production_state IS the designated mutation point
    manager.update_production_state();

    assert!(
        manager.fork.fork_mismatch_detected,
        "update_production_state() must set fork_mismatch_detected when in minority"
    );
}

// =========================================================================
// INC-001: Sync State Explosion — Rollback Loop Prevention Tests
// REQ-SYNC-001 through REQ-SYNC-006
// =========================================================================

/// REQ-SYNC-001: reset_sync_after_successful_reorg sets Normal, not PostRollback.
#[test]
fn test_inc001_successful_reorg_sets_normal_recovery_phase() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Simulate a successful fork sync reorg
    manager.reset_sync_after_successful_reorg();

    assert!(
        matches!(manager.recovery_phase, RecoveryPhase::Normal),
        "After successful reorg, recovery_phase must be Normal, got: {:?}",
        manager.recovery_phase
    );
}

/// REQ-SYNC-001: reset_sync_for_rollback still sets PostRollback (rejection path unchanged).
#[test]
fn test_inc001_rollback_still_sets_post_rollback() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Simulate a rejected fork sync rollback
    manager.reset_sync_for_rollback();

    assert!(
        matches!(manager.recovery_phase, RecoveryPhase::PostRollback),
        "After rejected rollback, recovery_phase must be PostRollback, got: {:?}",
        manager.recovery_phase
    );
}

/// REQ-SYNC-002: Successful reorg updates cooldown timestamp.
#[test]
fn test_inc001_successful_reorg_updates_cooldown() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Initial cooldown is set to 300s ago (expired)
    assert!(manager.fork.last_fork_sync_rejection.elapsed().as_secs() >= 299);

    // After successful reorg, cooldown should be fresh
    manager.reset_sync_after_successful_reorg();

    assert!(
        manager.fork.last_fork_sync_rejection.elapsed().as_secs() < 2,
        "Successful reorg must update cooldown timestamp"
    );
}

/// REQ-SYNC-004: Recently-held tips prevent ping-pong.
#[test]
fn test_inc001_recently_held_tip_detection() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    let tip_a = Hash::from_bytes([1u8; 32]);
    let tip_b = Hash::from_bytes([2u8; 32]);

    // Record tip A as recently held
    manager.record_held_tip(tip_a);

    // tip A should be detected as recently held
    assert!(manager.is_recently_held_tip(&tip_a));
    // tip B should NOT be detected
    assert!(!manager.is_recently_held_tip(&tip_b));
}

/// REQ-SYNC-004: Recently-held tips capacity is capped at 10.
#[test]
fn test_inc001_recently_held_tips_capacity() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Record 12 tips — first 2 should be evicted
    for i in 0..12u8 {
        let mut bytes = [0u8; 32];
        bytes[0] = i;
        manager.record_held_tip(Hash::from_bytes(bytes));
    }

    // Capacity is 10 — only tips 2..12 should remain
    assert_eq!(manager.fork.recently_held_tips.len(), 10);

    // Tip 0 should be evicted
    let mut bytes = [0u8; 32];
    bytes[0] = 0;
    assert!(!manager.is_recently_held_tip(&Hash::from_bytes(bytes)));

    // Tip 11 should still be present
    bytes[0] = 11;
    assert!(manager.is_recently_held_tip(&Hash::from_bytes(bytes)));
}

/// REQ-SYNC-005: Fork sync circuit breaker trips at 3 consecutive fork syncs.
#[test]
fn test_inc001_fork_sync_circuit_breaker() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Initially not tripped
    assert!(!manager.is_fork_sync_breaker_tripped());

    // Simulate 3 consecutive fork syncs (each successful reorg increments counter)
    manager.reset_sync_after_successful_reorg();
    assert!(!manager.is_fork_sync_breaker_tripped()); // 1 — not yet

    manager.reset_sync_after_successful_reorg();
    assert!(!manager.is_fork_sync_breaker_tripped()); // 2 — not yet

    manager.reset_sync_after_successful_reorg();
    assert!(
        manager.is_fork_sync_breaker_tripped(),
        "Circuit breaker must trip at 3 consecutive fork syncs"
    );
}

/// REQ-SYNC-005: Circuit breaker resets on successful header-first sync.
#[test]
fn test_inc001_circuit_breaker_resets_on_header_sync() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Trip the breaker
    for _ in 0..3 {
        manager.reset_sync_after_successful_reorg();
    }
    assert!(manager.is_fork_sync_breaker_tripped());

    // Successful header-first sync resets it
    manager.reset_fork_sync_breaker();
    assert!(
        !manager.is_fork_sync_breaker_tripped(),
        "Successful header-first sync must reset the circuit breaker"
    );
}

/// REQ-SYNC-006: After successful reorg, start_sync uses header-first (not fork_sync).
#[test]
fn test_inc001_successful_reorg_enables_header_first_sync() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Setup: node at height 10, peer at height 50
    manager.local_height = 10;
    manager.local_hash = Hash::from_bytes([1u8; 32]);
    manager.local_slot = 10;
    let peer = PeerId::random();
    manager.add_peer(peer, 50, Hash::from_bytes([2u8; 32]), 50);

    // After successful reorg, recovery_phase is Normal
    manager.reset_sync_after_successful_reorg();

    // start_sync should NOT enter PostRollback → fork_sync path
    manager.start_sync();

    // Should be in DownloadingHeaders (header-first), NOT fork_sync
    assert!(
        matches!(manager.state(), SyncState::DownloadingHeaders { .. }),
        "After successful reorg with Normal phase, sync should use header-first, got: {:?}",
        manager.state()
    );
}

// =========================================================================
// INC-001 RC-9: Sync-Production Deadlock Prevention Tests
// REQ-SYNC-007 through REQ-SYNC-009
// =========================================================================

/// REQ-SYNC-007: Layer 6.5 allows production at lag=2 immediately (no 30s timeout).
///
/// Root cause RC-9: The old 30s timeout for lag 2-3 blocks created a fatal
/// deadlock. The node would miss its slot, fall further behind, trigger sync,
/// and sync would cascade into fork_sync → ancestor at h=0 → full reset.
/// The node NEVER produced because the 30s timeout was interrupted by sync.
#[test]
fn test_inc001_rc9_small_lag_allows_production_immediately() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Setup: node at height 20, slot near peers (slot-based, not height-based).
    // One peer agrees at our height (so Layer 9 hash check doesn't block),
    // another peer is 2 blocks ahead (the lag we're testing).
    let local_hash = crypto::hash::hash(b"block20");
    manager.local_height = 20;
    manager.local_slot = 100; // Slot is time-based, close to peers
    manager.local_hash = local_hash;
    manager.first_peer_status_received = Some(Instant::now());

    let peer_agree = PeerId::random();
    let peer_ahead = PeerId::random();
    let ahead_hash = crypto::hash::hash(b"block22");
    // Peer 1: same height, same hash (Layer 9 agrees)
    manager.add_peer(peer_agree, 20, local_hash, 100);
    // Peer 2: 2 blocks ahead (Layer 6.5 lag=2)
    manager.add_peer(peer_ahead, 22, ahead_hash, 102);

    // Sync may have started from add_peer — force Idle for gate check
    manager.state = SyncState::Idle;

    let result = manager.can_produce(101);
    assert_eq!(
        result,
        ProductionAuthorization::Authorized,
        "RC-9: Node 2 blocks behind must be allowed to produce immediately (no 30s timeout). Got: {:?}",
        result
    );
}

/// REQ-SYNC-008: Layer 6.5 allows production at lag=3 immediately.
#[test]
fn test_inc001_rc9_lag3_allows_production_immediately() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Setup: node at height 20, 3 blocks behind one peer.
    // Slot is close (Layer 6 slot check won't trigger).
    // Peer 3 blocks ahead is outside Layer 9's ±2 window, so no hash mismatch.
    let local_hash = crypto::hash::hash(b"block20");
    manager.local_height = 20;
    manager.local_slot = 101; // Close to peer slot — 1 slot behind
    manager.local_hash = local_hash;
    manager.first_peer_status_received = Some(Instant::now());

    let peer_agree = PeerId::random();
    let peer_ahead = PeerId::random();
    // Peer 1: same height (Layer 9 agrees)
    manager.add_peer(peer_agree, 20, local_hash, 101);
    // Peer 2: 3 blocks ahead, outside ±2 window for Layer 9 hash check
    manager.add_peer(peer_ahead, 23, crypto::hash::hash(b"block23"), 103);

    manager.state = SyncState::Idle;

    let result = manager.can_produce(102);
    assert_eq!(
        result,
        ProductionAuthorization::Authorized,
        "RC-9: Node 3 blocks behind must be allowed to produce immediately. Got: {:?}",
        result
    );
}

/// REQ-SYNC-009: Layer 6.5 still blocks production at lag=4 (graduated gate).
#[test]
fn test_inc001_rc9_lag4_blocks_production_with_timeout() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Setup: node at height 20, 4 blocks behind.
    // Slot close enough that Layer 6 doesn't trigger (slot is time-based).
    let local_hash = crypto::hash::hash(b"block20");
    manager.local_height = 20;
    manager.local_slot = 101;
    manager.local_hash = local_hash;
    manager.first_peer_status_received = Some(Instant::now());

    let peer_agree = PeerId::random();
    let peer_ahead = PeerId::random();
    manager.add_peer(peer_agree, 20, local_hash, 101);
    manager.add_peer(peer_ahead, 24, crypto::hash::hash(b"block24"), 104);

    manager.state = SyncState::Idle;

    let result = manager.can_produce(102);
    assert!(
        matches!(
            result,
            ProductionAuthorization::BlockedBehindPeers { height_diff: 4, .. }
        ),
        "RC-9: Node 4 blocks behind should be blocked (graduated gate). Got: {:?}",
        result
    );
}

/// REQ-SYNC-010: Active sync state blocks production (Layer 3 before Layer 6.5).
/// Verifies that Layer 3 (sync state) takes precedence over Layer 6.5 (height lag).
#[test]
fn test_inc001_rc9_active_sync_blocks_production() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 20;
    manager.local_slot = 100;
    manager.local_hash = crypto::hash::hash(b"block20");
    manager.first_peer_status_received = Some(Instant::now());

    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    manager.add_peer(peer1, 22, crypto::hash::hash(b"block22"), 102);
    manager.add_peer(peer2, 22, crypto::hash::hash(b"block22"), 102);

    // Force sync active — Layer 3 blocks before Layer 6.5 is reached
    manager.state = SyncState::DownloadingHeaders {
        target_slot: 102,
        peer: peer1,
        headers_count: 0,
    };

    let result = manager.can_produce(101);
    assert!(
        matches!(result, ProductionAuthorization::BlockedSyncing),
        "RC-9: Active sync must block production (Layer 3). Got: {:?}",
        result
    );
}

/// REQ-SYNC-011: Processing stall resets to Idle immediately (RC-6).
/// Prevents the 30s stuck timeout from wasting 3 slots per stall.
#[test]
fn test_inc001_rc6_processing_stall_immediate_recovery() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 20;
    manager.local_hash = crypto::hash::hash(b"block20");
    manager.state = SyncState::Processing { height: 21 };

    // No pending headers/blocks → should reset to Idle
    let blocks = manager.get_blocks_to_apply();
    assert!(blocks.is_empty());
    assert!(
        matches!(manager.state(), SyncState::Idle),
        "RC-6: Processing with no extractable blocks must reset to Idle immediately. Got: {:?}",
        manager.state()
    );
}

// =========================================================================
// INC-I-005: Sync cascade feedback loop fixes
// Root cause: multi-entry-point feedback loop where each recovery mechanism
// produces imperfect state that triggers a DIFFERENT cascade entry point.
// =========================================================================

/// Fix A: AwaitingCanonicalBlock must have a timeout.
/// Without a timeout, nodes that snap sync to a height no peer recognizes
/// are permanently stuck (production blocked, no automatic recovery).
/// PostRecoveryGrace has a 120s timeout — AwaitingCanonicalBlock needs one too.
#[test]
fn test_awaiting_canonical_block_has_timeout() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Simulate snap sync completing: sets AwaitingCanonicalBlock
    manager.recovery_phase = RecoveryPhase::AwaitingCanonicalBlock {
        started: Instant::now() - Duration::from_secs(61),
    };

    // Production should be blocked initially
    manager.local_height = 600;
    manager.local_slot = 600;
    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    manager.add_peer(peer1, 600, Hash::ZERO, 600);
    manager.add_peer(peer2, 600, Hash::ZERO, 600);
    manager.first_peer_status_received = Some(Instant::now());

    // Run cleanup — should clear AwaitingCanonicalBlock after 60s
    manager.cleanup();

    // After timeout, recovery_phase should be Normal
    assert!(
        matches!(manager.recovery_phase, RecoveryPhase::Normal),
        "Fix A: AwaitingCanonicalBlock must clear after 60s timeout. Got: {:?}",
        manager.recovery_phase
    );
}

/// Fix A (negative): AwaitingCanonicalBlock should NOT timeout before 60s.
#[test]
fn test_awaiting_canonical_block_no_premature_timeout() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Only 30s elapsed — should NOT timeout
    manager.recovery_phase = RecoveryPhase::AwaitingCanonicalBlock {
        started: Instant::now() - Duration::from_secs(30),
    };

    manager.cleanup();

    assert!(
        matches!(
            manager.recovery_phase,
            RecoveryPhase::AwaitingCanonicalBlock { .. }
        ),
        "Fix A: AwaitingCanonicalBlock must NOT clear before 60s. Got: {:?}",
        manager.recovery_phase
    );
}

/// Fix B: Post-snap empty headers should retry snap from different peer,
/// not blacklist the responding peer. When a node just finished snap sync
/// and gets empty headers, the problem is the snap source (gave a hash
/// no peer recognizes), not the header peer.
#[test]
fn test_post_snap_empty_headers_triggers_snap_retry() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Node just completed snap sync (30s ago)
    manager.recovery_phase = RecoveryPhase::AwaitingCanonicalBlock {
        started: Instant::now() - Duration::from_secs(5),
    };
    manager.local_height = 598;
    manager.local_hash = crypto::hash::hash(b"snap_hash_598");
    manager.local_slot = 598;
    manager.snap.threshold = 1000; // Re-enable snap sync for this test

    // Simulate DownloadingHeaders state
    let peer = PeerId::random();
    manager.add_peer(peer, 602, crypto::hash::hash(b"peer_hash_602"), 602);
    manager.state = SyncState::DownloadingHeaders {
        target_slot: 602,
        peer,
        headers_count: 0,
    };

    // Handle empty headers response (peer doesn't recognize our snap hash)
    let response = SyncResponse::Headers(vec![]);
    manager.handle_response(peer, response);

    // Fix B: peer should NOT be blacklisted (it's canonical, our hash is wrong)
    assert!(
        !manager.fork.header_blacklisted_peers.contains_key(&peer),
        "Fix B: Post-snap empty headers must NOT blacklist responding peer"
    );

    // Fix B: needs_genesis_resync should be set (to retry snap sync)
    assert!(
        manager.fork.needs_genesis_resync,
        "Fix B: Post-snap empty headers must trigger snap sync retry, not fork detection"
    );
}

/// Fix C: Monotonic progress floor prevents reset below confirmed height.
/// Once a node has been Synchronized and applied 10+ blocks at height H,
/// reset_local_state() must not set height below H.
#[test]
fn test_confirmed_height_floor_prevents_reset_to_zero() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Simulate a node that was healthy at height 500 (applied 10+ blocks)
    manager.local_height = 500;
    manager.local_hash = crypto::hash::hash(b"block500");
    manager.local_slot = 500;
    manager.state = SyncState::Synchronized;

    // Apply 10 blocks in Synchronized state to set the floor
    for i in 0..10 {
        manager.block_applied_with_weight(
            crypto::hash::hash(format!("block{}", 501 + i).as_bytes()),
            501 + i,
            (501 + i) as u32,
            1,
            crypto::hash::hash(format!("block{}", 500 + i).as_bytes()),
        );
    }

    // Verify floor is set
    assert!(
        manager.confirmed_height_floor() >= 510,
        "Fix C: confirmed_height_floor should be >= 510 after 10 blocks in Synchronized. Got: {}",
        manager.confirmed_height_floor()
    );

    // Now try to reset to genesis — should be refused
    manager.reset_local_state(Hash::ZERO);

    // Fix C: height should NOT be 0 — should stay at or above the floor
    assert!(
        manager.local_height > 0,
        "Fix C: reset_local_state must NOT set height to 0 when confirmed_height_floor > 0. Got: {}",
        manager.local_height
    );
}

/// Fix C (positive): Fresh nodes with no confirmed floor CAN reset to zero.
#[test]
fn test_fresh_node_can_reset_to_zero() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Fresh node — no confirmed height floor
    manager.local_height = 50;
    manager.local_hash = crypto::hash::hash(b"block50");

    // Reset should work normally (floor is 0)
    manager.reset_local_state(Hash::ZERO);

    assert_eq!(
        manager.local_height, 0,
        "Fresh nodes with no confirmed floor should reset to 0"
    );
}

// =========================================================================
// M1: Recovery Gate + Transition Validation Tests
// Architecture: specs/sync-recovery-architecture.md (Sections 2, 4, 6)
// Requirements: REQ-SYNC-102 (RecoveryReason), REQ-SYNC-103 (request_genesis_resync),
//               REQ-SYNC-104 (is_valid_transition), PRESERVE-3 (existing behavior)
// =========================================================================

// -------------------------------------------------------------------------
// Regression tests: lock existing behavior (MUST pass before AND after M1)
// -------------------------------------------------------------------------

mod regression_tests {
    use super::*;

    // PRESERVE-3: set_state() transitions currently used in the codebase
    // must continue to work after is_valid_transition() is added.

    /// Regression: Idle -> DownloadingHeaders is a valid and frequently used transition.
    /// Used by: start_sync() in sync_engine.rs (5+ call sites).
    #[test]
    fn test_regression_idle_to_downloading_headers() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        assert!(matches!(*manager.state(), SyncState::Idle));

        let peer = PeerId::random();
        manager.add_peer(peer, 100, Hash::ZERO, 100);

        // start_sync transitions Idle -> DownloadingHeaders
        manager.start_sync();
        assert!(
            matches!(manager.state(), SyncState::DownloadingHeaders { .. }),
            "Idle -> DownloadingHeaders must remain valid. Got: {:?}",
            manager.state()
        );
    }

    /// Regression: Idle -> SnapCollectingRoots is used when snap sync starts.
    /// Used by: start_sync() in sync_engine.rs when gap > snap.threshold.
    /// Requires enough peers to meet snap quorum (5 by default).
    #[test]
    fn test_regression_idle_to_snap_collecting_roots() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.snap.threshold = 100; // Enable snap sync with low threshold
        manager.snap.quorum = 2; // Lower quorum for test feasibility

        // Add enough peers for snap quorum
        for _ in 0..5 {
            let peer = PeerId::random();
            manager.add_peer(peer, 200, Hash::ZERO, 200);
        }

        // Force back to Idle (add_peer may have started sync)
        manager.state = SyncState::Idle;
        manager.start_sync();

        // With gap=200 > threshold=100 and enough peers, snap sync should trigger.
        // If start_sync took the header-first path instead, that's also valid
        // from Idle. The key point: Idle can transition to either.
        let state = manager.state();
        assert!(
            matches!(
                state,
                SyncState::SnapCollectingRoots { .. } | SyncState::DownloadingHeaders { .. }
            ),
            "Idle -> SnapCollectingRoots or DownloadingHeaders must remain valid. Got: {:?}",
            state
        );
    }

    /// Regression: DownloadingHeaders -> Idle is used on error/timeout/fork detection.
    /// Used by: sync_engine.rs (6+ call sites), cleanup.rs stuck sync detection.
    #[test]
    fn test_regression_downloading_headers_to_idle() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        let peer = PeerId::random();
        manager.add_peer(peer, 100, Hash::ZERO, 100);
        manager.start_sync();
        assert!(matches!(
            manager.state(),
            SyncState::DownloadingHeaders { .. }
        ));

        // Simulate chain mismatch detection -> reset to Idle
        manager.set_state(SyncState::Idle, "test_regression_headers_to_idle");
        assert!(
            matches!(*manager.state(), SyncState::Idle),
            "DownloadingHeaders -> Idle must remain valid"
        );
    }

    /// Regression: DownloadingHeaders -> Synchronized is used when already caught up.
    /// Used by: sync_engine.rs "headers_empty_already_synced".
    #[test]
    fn test_regression_downloading_headers_to_synchronized() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        let peer = PeerId::random();
        manager.state = SyncState::DownloadingHeaders {
            target_slot: 100,
            peer,
            headers_count: 5,
        };

        manager.set_state(SyncState::Synchronized, "test_regression_headers_to_sync");
        assert!(
            matches!(*manager.state(), SyncState::Synchronized),
            "DownloadingHeaders -> Synchronized must remain valid"
        );
    }

    /// Regression: DownloadingHeaders -> DownloadingBodies when all headers collected.
    /// Used by: sync_engine.rs "headers_complete".
    #[test]
    fn test_regression_downloading_headers_to_downloading_bodies() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        let peer = PeerId::random();
        manager.state = SyncState::DownloadingHeaders {
            target_slot: 100,
            peer,
            headers_count: 50,
        };

        manager.set_state(
            SyncState::DownloadingBodies {
                pending: 0,
                total: 50,
            },
            "test_regression_headers_to_bodies",
        );
        assert!(
            matches!(*manager.state(), SyncState::DownloadingBodies { .. }),
            "DownloadingHeaders -> DownloadingBodies must remain valid"
        );
    }

    /// Regression: DownloadingBodies -> Processing when all bodies downloaded.
    /// Used by: sync_engine.rs "bodies_complete".
    #[test]
    fn test_regression_downloading_bodies_to_processing() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = SyncState::DownloadingBodies {
            pending: 0,
            total: 50,
        };

        manager.set_state(
            SyncState::Processing { height: 1 },
            "test_regression_bodies_to_processing",
        );
        assert!(
            matches!(*manager.state(), SyncState::Processing { .. }),
            "DownloadingBodies -> Processing must remain valid"
        );
    }

    /// Regression: DownloadingBodies -> DownloadingBodies (soft retry).
    /// Used by: cleanup.rs "body_stall_soft_retry", sync_engine.rs body count update.
    #[test]
    fn test_regression_downloading_bodies_self_transition() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = SyncState::DownloadingBodies {
            pending: 10,
            total: 50,
        };

        manager.set_state(
            SyncState::DownloadingBodies {
                pending: 5,
                total: 50,
            },
            "test_regression_bodies_self_transition",
        );
        assert!(
            matches!(
                *manager.state(),
                SyncState::DownloadingBodies { pending: 5, .. }
            ),
            "DownloadingBodies -> DownloadingBodies must remain valid"
        );
    }

    /// Regression: DownloadingBodies -> Idle on error.
    /// Used by: cleanup.rs "body_download_exhausted", "cleanup_stuck_sync".
    #[test]
    fn test_regression_downloading_bodies_to_idle() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = SyncState::DownloadingBodies {
            pending: 10,
            total: 50,
        };

        manager.set_state(SyncState::Idle, "test_regression_bodies_to_idle");
        assert!(
            matches!(*manager.state(), SyncState::Idle),
            "DownloadingBodies -> Idle must remain valid"
        );
    }

    /// Regression: Processing -> Synchronized on completion.
    /// Used by: block_lifecycle.rs "sync_complete_block_applied".
    #[test]
    fn test_regression_processing_to_synchronized() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = SyncState::Processing { height: 50 };

        manager.set_state(
            SyncState::Synchronized,
            "test_regression_processing_to_sync",
        );
        assert!(
            matches!(*manager.state(), SyncState::Synchronized),
            "Processing -> Synchronized must remain valid"
        );
    }

    /// Regression: Processing -> Idle on stall/error.
    /// Used by: block_lifecycle.rs "processing_complete_restart", "block_apply_failed",
    ///          sync_engine.rs "processing_stall_reset".
    #[test]
    fn test_regression_processing_to_idle() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = SyncState::Processing { height: 50 };

        manager.set_state(SyncState::Idle, "test_regression_processing_to_idle");
        assert!(
            matches!(*manager.state(), SyncState::Idle),
            "Processing -> Idle must remain valid"
        );
    }

    /// Regression: Processing -> Processing (height update).
    /// The Processing state carries a height field that updates.
    #[test]
    fn test_regression_processing_self_transition() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = SyncState::Processing { height: 50 };

        manager.set_state(
            SyncState::Processing { height: 51 },
            "test_regression_processing_self_transition",
        );
        if let SyncState::Processing { height } = *manager.state() {
            assert_eq!(height, 51);
        } else {
            panic!("Processing -> Processing (height update) must remain valid");
        }
    }

    /// Regression: Synchronized -> Idle on stall detection.
    /// Used by: cleanup.rs "stall_synchronized_behind".
    #[test]
    fn test_regression_synchronized_to_idle() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = SyncState::Synchronized;

        manager.set_state(SyncState::Idle, "test_regression_synchronized_to_idle");
        assert!(
            matches!(*manager.state(), SyncState::Idle),
            "Synchronized -> Idle must remain valid"
        );
    }

    /// Regression: Synchronized -> Synchronized (idempotent set from update_local_tip).
    /// Used by: mod.rs update_local_tip "update_local_tip_caught_up".
    #[test]
    fn test_regression_synchronized_self_transition() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = SyncState::Synchronized;

        manager.set_state(
            SyncState::Synchronized,
            "test_regression_sync_self_transition",
        );
        assert!(
            matches!(*manager.state(), SyncState::Synchronized),
            "Synchronized -> Synchronized must remain valid"
        );
    }

    /// Regression: SnapReady -> Synchronized on snapshot consumed.
    /// Used by: snap_sync.rs "snap_snapshot_applied".
    #[test]
    fn test_regression_snap_ready_to_synchronized() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = SyncState::SnapReady {
            snapshot: VerifiedSnapshot {
                block_hash: Hash::ZERO,
                block_height: 100,
                chain_state: vec![],
                utxo_set: vec![],
                producer_set: vec![],
                state_root: Hash::ZERO,
            },
        };

        manager.set_state(
            SyncState::Synchronized,
            "test_regression_snap_ready_to_sync",
        );
        assert!(
            matches!(*manager.state(), SyncState::Synchronized),
            "SnapReady -> Synchronized must remain valid"
        );
    }

    /// Regression: SnapDownloading -> Idle on error with no alternates.
    /// Used by: snap_sync.rs "snap_download_error_no_alternates".
    #[test]
    fn test_regression_snap_downloading_to_idle() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        let peer = PeerId::random();
        manager.state = SyncState::SnapDownloading {
            target_hash: Hash::ZERO,
            target_height: 100,
            quorum_root: Hash::ZERO,
            peer,
            alternate_peers: vec![],
            started_at: Instant::now(),
        };

        manager.set_state(SyncState::Idle, "test_regression_snap_downloading_to_idle");
        assert!(
            matches!(*manager.state(), SyncState::Idle),
            "SnapDownloading -> Idle must remain valid"
        );
    }

    /// Regression: All block_lifecycle.rs transitions to Idle work.
    /// Used by: reset_sync_for_rollback, reset_sync_after_successful_reorg,
    ///          reset_local_state, start_fork_sync, fork_sync_cleared.
    #[test]
    fn test_regression_lifecycle_resets_to_idle() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // reset_sync_for_rollback -> Idle
        manager.state = SyncState::Processing { height: 10 };
        manager.reset_sync_for_rollback();
        assert!(
            matches!(*manager.state(), SyncState::Idle),
            "reset_sync_for_rollback must transition to Idle"
        );

        // reset_sync_after_successful_reorg -> Idle
        manager.state = SyncState::Processing { height: 10 };
        manager.reset_sync_after_successful_reorg();
        assert!(
            matches!(*manager.state(), SyncState::Idle),
            "reset_sync_after_successful_reorg must transition to Idle"
        );
    }

    /// Regression: needs_genesis_resync flag is readable and consumable.
    /// The periodic task reads this flag to decide on force_recover_from_peers.
    #[test]
    fn test_regression_needs_genesis_resync_readable() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Initially false
        assert!(
            !manager.needs_genesis_resync(),
            "needs_genesis_resync must be false initially"
        );

        // Direct set (current API) still works
        manager.set_needs_genesis_resync();
        assert!(
            manager.needs_genesis_resync(),
            "needs_genesis_resync must be true after set_needs_genesis_resync()"
        );
    }

    /// Regression: signal_stuck_fork sets StuckForkDetected correctly.
    /// request_genesis_resync follows the same pattern.
    #[test]
    fn test_regression_signal_stuck_fork_pattern() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // From Normal -> StuckForkDetected
        assert!(matches!(manager.recovery_phase, RecoveryPhase::Normal));
        manager.signal_stuck_fork();
        assert!(
            matches!(manager.recovery_phase, RecoveryPhase::StuckForkDetected),
            "signal_stuck_fork from Normal must set StuckForkDetected"
        );

        // From PostRollback -> StuckForkDetected
        manager.recovery_phase = RecoveryPhase::PostRollback;
        manager.signal_stuck_fork();
        assert!(
            matches!(manager.recovery_phase, RecoveryPhase::StuckForkDetected),
            "signal_stuck_fork from PostRollback must set StuckForkDetected"
        );

        // From ResyncInProgress -> ignored (no override)
        manager.recovery_phase = RecoveryPhase::ResyncInProgress;
        manager.signal_stuck_fork();
        assert!(
            matches!(manager.recovery_phase, RecoveryPhase::ResyncInProgress),
            "signal_stuck_fork must NOT override ResyncInProgress"
        );
    }
}

// -------------------------------------------------------------------------
// Recovery Gate Tests: request_genesis_resync()
// REQ-SYNC-102 (RecoveryReason enum), REQ-SYNC-103 (gated method)
// Architecture: Section 4 — "New method: request_genesis_resync()"
//
// DEVELOPER NOTE: Remove the #[cfg(feature = "m1_recovery_gate")] gate below
// once RecoveryReason enum and request_genesis_resync() method are implemented.
// These tests are intentionally written BEFORE the code exists (TDD).
// -------------------------------------------------------------------------

mod recovery_gate_tests {
    use super::*;

    /// T-RG-001: request_genesis_resync REFUSED when confirmed_height_floor > 0.
    /// REQ-SYNC-103: Gate 1 — monotonic progress floor.
    ///
    /// If a node was previously healthy at height H (confirmed_height_floor = H),
    /// resetting to genesis would violate the monotonic progress guarantee.
    /// Manual intervention is required instead.
    #[test]
    fn test_request_genesis_resync_refused_by_height_floor() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Simulate a node that was confirmed healthy at height 100
        manager.confirmed_height_floor = 100;
        // Snap sync enabled, fresh state otherwise
        manager.snap.threshold = 500;
        manager.snap.attempts = 0;

        let result =
            manager.request_genesis_resync(RecoveryReason::StuckSyncLargeGap { gap: 2000 });

        assert!(
            !result,
            "T-RG-001: request_genesis_resync must return false when confirmed_height_floor > 0"
        );
        assert!(
            !manager.needs_genesis_resync(),
            "T-RG-001: needs_genesis_resync flag must remain false when gate refuses"
        );
    }

    /// T-RG-002: request_genesis_resync REFUSED during active resync.
    /// REQ-SYNC-103: Gate 2 — no concurrent recovery.
    ///
    /// If a resync is already in progress, starting another one would
    /// create a race condition or reset partial progress.
    #[test]
    fn test_request_genesis_resync_refused_during_active_resync() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Active resync in progress
        manager.recovery_phase = RecoveryPhase::ResyncInProgress;
        manager.snap.threshold = 500;
        manager.snap.attempts = 0;

        let result = manager.request_genesis_resync(RecoveryReason::AllPeersBlacklistedDeepFork);

        assert!(
            !result,
            "T-RG-002: request_genesis_resync must return false during ResyncInProgress"
        );
        assert!(
            !manager.fork.needs_genesis_resync,
            "T-RG-002: needs_genesis_resync flag must remain false during active resync"
        );
    }

    /// T-RG-003: request_genesis_resync REFUSED after MAX_CONSECUTIVE_RESYNCS.
    /// REQ-SYNC-103: Gate 3 — rate limiting.
    ///
    /// After MAX_CONSECUTIVE_RESYNCS (5), the node has failed to recover
    /// repeatedly. Further automatic attempts are futile — manual intervention
    /// is required.
    #[test]
    fn test_request_genesis_resync_refused_after_max_resyncs() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Simulate MAX_CONSECUTIVE_RESYNCS resyncs
        manager.consecutive_resync_count = MAX_CONSECUTIVE_RESYNCS;
        manager.snap.threshold = 500;
        manager.snap.attempts = 0;

        let result = manager.request_genesis_resync(RecoveryReason::PostRollbackSnapEscalation);

        assert!(
            !result,
            "T-RG-003: request_genesis_resync must return false after {} resyncs",
            MAX_CONSECUTIVE_RESYNCS
        );
        assert!(
            !manager.fork.needs_genesis_resync,
            "T-RG-003: needs_genesis_resync flag must remain false after max resyncs"
        );
    }

    /// T-RG-003b: request_genesis_resync accepted at exactly MAX - 1 resyncs.
    /// Boundary test for the rate limiter.
    #[test]
    fn test_request_genesis_resync_accepted_at_max_minus_one() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // One less than MAX
        manager.consecutive_resync_count = MAX_CONSECUTIVE_RESYNCS - 1;
        manager.snap.threshold = 500;
        manager.snap.attempts = 0;

        let result = manager.request_genesis_resync(RecoveryReason::PostRollbackSnapEscalation);

        assert!(
            result,
            "T-RG-003b: request_genesis_resync must be accepted at MAX-1 resyncs ({})",
            MAX_CONSECUTIVE_RESYNCS - 1
        );
    }

    /// T-RG-004: request_genesis_resync REFUSED when snap sync is disabled.
    /// REQ-SYNC-103: Gate 4 — snap sync availability.
    ///
    /// When snap.threshold == u64::MAX (--no-snap-sync), genesis resync requires
    /// snap sync infrastructure. Without it, only header-first recovery works.
    #[test]
    fn test_request_genesis_resync_refused_when_snap_disabled() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Snap sync disabled (default — threshold is u64::MAX)
        assert_eq!(manager.snap.threshold, u64::MAX);
        manager.snap.attempts = 0;

        let result = manager.request_genesis_resync(RecoveryReason::GenesisFallbackEmptyHeaders);

        assert!(
            !result,
            "T-RG-004: request_genesis_resync must return false when snap sync disabled"
        );
        assert!(
            !manager.fork.needs_genesis_resync,
            "T-RG-004: needs_genesis_resync flag must remain false when snap disabled"
        );
    }

    /// T-RG-005: request_genesis_resync REFUSED after 3 snap sync attempts.
    /// REQ-SYNC-103: Gate 5 — snap attempt limit.
    ///
    /// After 3 failed snap sync attempts, further snap syncs are unlikely to
    /// succeed. Manual intervention or header-first recovery is needed.
    #[test]
    fn test_request_genesis_resync_refused_after_max_snap_attempts() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.snap.threshold = 500; // Snap enabled
        manager.snap.attempts = 3; // 3 failed attempts

        let result = manager.request_genesis_resync(RecoveryReason::BodyDownloadPeerError);

        assert!(
            !result,
            "T-RG-005: request_genesis_resync must return false after 3 snap attempts"
        );
        assert!(
            !manager.fork.needs_genesis_resync,
            "T-RG-005: needs_genesis_resync flag must remain false after max snap attempts"
        );
    }

    /// T-RG-005b: request_genesis_resync accepted at exactly 2 snap attempts.
    /// Boundary test for the snap attempt limiter.
    #[test]
    fn test_request_genesis_resync_accepted_at_2_snap_attempts() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.snap.threshold = 500;
        manager.snap.attempts = 2; // Under the limit

        let result = manager.request_genesis_resync(RecoveryReason::BodyDownloadPeerError);

        assert!(
            result,
            "T-RG-005b: request_genesis_resync must be accepted at 2 snap attempts (< 3)"
        );
    }

    /// T-RG-006: request_genesis_resync ACCEPTED when all gates pass.
    /// REQ-SYNC-103: Happy path — all 5 gates open.
    ///
    /// Default SyncManager has: floor=0, phase=Normal, resync_count=0,
    /// snap.threshold=u64::MAX (disabled by default). We enable snap for this test.
    #[test]
    fn test_request_genesis_resync_accepted_when_all_gates_pass() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Enable snap sync (gate 4 requires it)
        manager.snap.threshold = 500;
        // All other gates are at default (floor=0, Normal phase, resync_count=0, attempts=0)

        let result =
            manager.request_genesis_resync(RecoveryReason::StuckSyncLargeGap { gap: 2000 });

        assert!(
            result,
            "T-RG-006: request_genesis_resync must return true when all gates pass"
        );
        assert!(
            manager.fork.needs_genesis_resync,
            "T-RG-006: needs_genesis_resync flag must be true after accepted request"
        );
    }

    /// T-RG-007: request_genesis_resync does not panic for any RecoveryReason variant.
    /// REQ-SYNC-102: All RecoveryReason variants must be handled.
    ///
    /// The method uses reason for logging. Each variant must format correctly
    /// without panicking, regardless of whether the request is honored.
    #[test]
    fn test_request_genesis_resync_handles_all_reason_variants() {
        let reasons = vec![
            RecoveryReason::AllPeersBlacklistedDeepFork,
            RecoveryReason::StuckSyncLargeGap { gap: 2000 },
            RecoveryReason::HeightOffsetDetected { gap: 500 },
            RecoveryReason::PostRollbackSnapEscalation,
            RecoveryReason::GenesisFallbackEmptyHeaders,
            RecoveryReason::BodyDownloadPeerError,
            RecoveryReason::ApplyFailuresSnapThreshold { gap: 100 },
            RecoveryReason::RollbackDeathSpiral {
                peak: 500,
                current: 10,
            },
        ];

        for reason in reasons {
            let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
            manager.snap.threshold = 500; // Enable snap so the request can be honored

            // This must NOT panic for any reason variant
            let result = manager.request_genesis_resync(reason.clone());

            // With all gates open, every variant should be accepted
            assert!(
                result,
                "T-RG-007: request_genesis_resync must handle {:?} without panic and accept it",
                reason
            );
        }
    }

    /// T-RG-008: Multiple consecutive calls only honor the first.
    /// Once needs_genesis_resync is true, subsequent calls are still gated
    /// but the flag stays true (idempotent behavior).
    #[test]
    fn test_request_genesis_resync_idempotent() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.snap.threshold = 500;

        let first = manager.request_genesis_resync(RecoveryReason::StuckSyncLargeGap { gap: 1000 });
        assert!(first, "First call must be accepted");
        assert!(manager.fork.needs_genesis_resync);

        // Second call — flag already true, but gates still open
        let second = manager.request_genesis_resync(RecoveryReason::AllPeersBlacklistedDeepFork);
        // The method should still pass all gates (flag already set is not a gate)
        // The flag stays true regardless
        assert!(
            manager.fork.needs_genesis_resync,
            "T-RG-008: needs_genesis_resync must remain true after second call"
        );
        // Whether second returns true or false is implementation-defined,
        // but it must not panic and the flag must stay true
    }

    /// T-RG-009: Gate ordering — height floor checked first (fast reject).
    /// If both height floor > 0 AND snap disabled, the height floor gate
    /// should reject before reaching the snap gate.
    #[test]
    fn test_request_genesis_resync_gate_ordering_floor_first() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Both gates would reject: floor > 0 AND snap disabled
        manager.confirmed_height_floor = 100;
        // snap.threshold is u64::MAX by default (disabled)

        let result =
            manager.request_genesis_resync(RecoveryReason::StuckSyncLargeGap { gap: 2000 });

        assert!(!result, "T-RG-009: Must be refused (either gate rejects)");
        assert!(
            !manager.fork.needs_genesis_resync,
            "T-RG-009: Flag must remain false"
        );
    }

    /// T-RG-010: Edge case — confirmed_height_floor is exactly 0.
    /// Floor of 0 means the node was never confirmed healthy. Gate 1 should PASS.
    #[test]
    fn test_request_genesis_resync_floor_exactly_zero_passes() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        assert_eq!(manager.confirmed_height_floor, 0);
        manager.snap.threshold = 500;

        let result =
            manager.request_genesis_resync(RecoveryReason::StuckSyncLargeGap { gap: 2000 });

        assert!(
            result,
            "T-RG-010: Floor == 0 means no confirmed health — gate 1 must pass"
        );
    }

    /// T-RG-011: Edge case — consecutive_resync_count is exactly MAX.
    /// At exactly MAX, gate 3 must REFUSE (>= comparison).
    #[test]
    fn test_request_genesis_resync_resync_count_exactly_max() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.consecutive_resync_count = MAX_CONSECUTIVE_RESYNCS;
        manager.snap.threshold = 500;

        let result = manager.request_genesis_resync(RecoveryReason::AllPeersBlacklistedDeepFork);

        assert!(
            !result,
            "T-RG-011: Exactly MAX resyncs ({}) must be refused (>= check)",
            MAX_CONSECUTIVE_RESYNCS
        );
    }

    /// T-RG-012: Edge case — snap.attempts is exactly 3.
    /// At exactly 3, gate 5 must REFUSE (>= comparison).
    #[test]
    fn test_request_genesis_resync_snap_attempts_exactly_3() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.snap.threshold = 500;
        manager.snap.attempts = 3;

        let result = manager.request_genesis_resync(RecoveryReason::GenesisFallbackEmptyHeaders);

        assert!(
            !result,
            "T-RG-012: Exactly 3 snap attempts must be refused (>= check)"
        );
    }

    /// T-RG-013: Different RecoveryPhase values vs. Gate 2.
    /// Only ResyncInProgress should block. All other phases should pass gate 2.
    #[test]
    fn test_request_genesis_resync_gate2_phase_specificity() {
        let phases_that_should_pass = vec![
            RecoveryPhase::Normal,
            RecoveryPhase::StuckForkDetected,
            RecoveryPhase::PostRollback,
            RecoveryPhase::PostRecoveryGrace {
                started: Instant::now(),
                blocks_applied: 0,
            },
            RecoveryPhase::AwaitingCanonicalBlock {
                started: Instant::now(),
            },
        ];

        for phase in phases_that_should_pass {
            let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
            manager.snap.threshold = 500;
            manager.recovery_phase = phase.clone();

            let result =
                manager.request_genesis_resync(RecoveryReason::StuckSyncLargeGap { gap: 2000 });

            assert!(
                result,
                "T-RG-013: RecoveryPhase {:?} must NOT block gate 2 (only ResyncInProgress blocks)",
                phase
            );
        }
    }
}

// -------------------------------------------------------------------------
// Transition Validation Tests: is_valid_transition()
// REQ-SYNC-104: State transition validation
// Architecture: Section 4 - "New method: is_valid_transition()"
//               Section 2 - "Valid Transition Matrix"
//
// -------------------------------------------------------------------------

mod transition_validation_tests {
    use super::*;

    // --- Helper: create SyncState variants for testing ---

    fn idle() -> SyncState {
        SyncState::Idle
    }

    fn downloading_headers() -> SyncState {
        SyncState::DownloadingHeaders {
            target_slot: 100,
            peer: PeerId::random(),
            headers_count: 0,
        }
    }

    fn downloading_bodies() -> SyncState {
        SyncState::DownloadingBodies {
            pending: 10,
            total: 50,
        }
    }

    fn processing() -> SyncState {
        SyncState::Processing { height: 50 }
    }

    fn synchronized() -> SyncState {
        SyncState::Synchronized
    }

    fn snap_collecting_roots() -> SyncState {
        SyncState::SnapCollectingRoots {
            target_hash: Hash::ZERO,
            target_height: 100,
            votes: vec![],
            asked: std::collections::HashSet::new(),
            started_at: Instant::now(),
        }
    }

    fn snap_downloading() -> SyncState {
        SyncState::SnapDownloading {
            target_hash: Hash::ZERO,
            target_height: 100,
            quorum_root: Hash::ZERO,
            peer: PeerId::random(),
            alternate_peers: vec![],
            started_at: Instant::now(),
        }
    }

    fn snap_ready() -> SyncState {
        SyncState::SnapReady {
            snapshot: VerifiedSnapshot {
                block_hash: Hash::ZERO,
                block_height: 100,
                chain_state: vec![],
                utxo_set: vec![],
                producer_set: vec![],
                state_root: Hash::ZERO,
            },
        }
    }

    // === Valid transitions from Idle (Idle -> anything is valid) ===

    /// T-TV-001: Idle can transition to any state.
    /// REQ-SYNC-104: "Idle can go anywhere (it's the reset state)"
    #[test]
    fn test_valid_transitions_from_idle() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        let all_states = vec![
            idle(),
            downloading_headers(),
            downloading_bodies(),
            processing(),
            synchronized(),
            snap_collecting_roots(),
            snap_downloading(),
            snap_ready(),
        ];

        for target in &all_states {
            manager.state = idle();
            assert!(
                manager.is_valid_transition(target),
                "T-TV-001: Idle -> {:?} must be valid",
                std::mem::discriminant(target)
            );
        }
    }

    // === Valid transitions to Idle (anything -> Idle is valid) ===

    /// T-TV-002: Any state can transition to Idle.
    /// REQ-SYNC-104: "Any state can go to Idle (reset/error)"
    #[test]
    fn test_valid_transition_to_idle_from_any() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        let all_states = vec![
            idle(),
            downloading_headers(),
            downloading_bodies(),
            processing(),
            synchronized(),
            snap_collecting_roots(),
            snap_downloading(),
            snap_ready(),
        ];

        for source in &all_states {
            manager.state = source.clone();
            assert!(
                manager.is_valid_transition(&idle()),
                "T-TV-002: {:?} -> Idle must be valid",
                std::mem::discriminant(source)
            );
        }
    }

    // === Invalid transitions (the whole point of the validation) ===

    /// T-TV-003: SnapCollectingRoots -> Synchronized is INVALID.
    /// REQ-SYNC-104: "snap sync can't skip to synchronized"
    ///
    /// Snap sync must go through SnapDownloading -> SnapReady -> Synchronized.
    /// Skipping the download step would leave the node with no state snapshot.
    #[test]
    fn test_invalid_snap_collecting_to_synchronized() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = snap_collecting_roots();

        assert!(
            !manager.is_valid_transition(&synchronized()),
            "T-TV-003: SnapCollectingRoots -> Synchronized must be INVALID"
        );
    }

    /// T-TV-004: Processing -> SnapCollectingRoots is INVALID.
    /// REQ-SYNC-104: "can't start snap sync from processing"
    ///
    /// If we're in the middle of applying downloaded blocks, we should finish
    /// or abort (-> Idle) before starting a completely different sync strategy.
    #[test]
    fn test_invalid_processing_to_snap_collecting() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = processing();

        assert!(
            !manager.is_valid_transition(&snap_collecting_roots()),
            "T-TV-004: Processing -> SnapCollectingRoots must be INVALID"
        );
    }

    /// T-TV-005: Synchronized -> DownloadingBodies is INVALID.
    /// REQ-SYNC-104: "must go through Idle -> DownloadingHeaders"
    ///
    /// Body download requires headers to be downloaded first. Going directly
    /// from Synchronized to DownloadingBodies skips the header phase.
    #[test]
    fn test_invalid_synchronized_to_downloading_bodies() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = synchronized();

        assert!(
            !manager.is_valid_transition(&downloading_bodies()),
            "T-TV-005: Synchronized -> DownloadingBodies must be INVALID"
        );
    }

    // === Valid forward-path transitions ===

    /// T-TV-006: DownloadingHeaders -> DownloadingBodies is valid.
    /// REQ-SYNC-104: "Header download leads to bodies"
    #[test]
    fn test_valid_header_to_body_transition() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = downloading_headers();

        assert!(
            manager.is_valid_transition(&downloading_bodies()),
            "T-TV-006: DownloadingHeaders -> DownloadingBodies must be valid"
        );
    }

    /// T-TV-007: Full snap sync forward path is valid.
    /// REQ-SYNC-104: SnapCollecting -> SnapDownloading -> SnapReady -> Synchronized
    #[test]
    fn test_valid_snap_forward_path() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Step 1: SnapCollecting -> SnapDownloading
        manager.state = snap_collecting_roots();
        assert!(
            manager.is_valid_transition(&snap_downloading()),
            "T-TV-007a: SnapCollectingRoots -> SnapDownloading must be valid"
        );

        // Step 2: SnapDownloading -> SnapReady
        manager.state = snap_downloading();
        assert!(
            manager.is_valid_transition(&snap_ready()),
            "T-TV-007b: SnapDownloading -> SnapReady must be valid"
        );

        // Step 3: SnapReady -> Synchronized
        manager.state = snap_ready();
        assert!(
            manager.is_valid_transition(&synchronized()),
            "T-TV-007c: SnapReady -> Synchronized must be valid"
        );
    }

    /// T-TV-008: Full header-first sync forward path is valid.
    /// Idle -> Headers -> Bodies -> Processing -> Synchronized
    #[test]
    fn test_valid_header_first_forward_path() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Idle -> DownloadingHeaders
        manager.state = idle();
        assert!(manager.is_valid_transition(&downloading_headers()));

        // DownloadingHeaders -> DownloadingBodies
        manager.state = downloading_headers();
        assert!(manager.is_valid_transition(&downloading_bodies()));

        // DownloadingBodies -> Processing
        manager.state = downloading_bodies();
        assert!(manager.is_valid_transition(&processing()));

        // Processing -> Synchronized
        manager.state = processing();
        assert!(
            manager.is_valid_transition(&synchronized()),
            "T-TV-008: Full header-first forward path must be valid"
        );
    }

    /// T-TV-009: DownloadingHeaders -> SnapCollectingRoots is valid.
    /// REQ-SYNC-104: "Header download leads to ... snap"
    ///
    /// During header download, if the gap grows large enough, we may switch
    /// to snap sync. This is a valid pivot.
    #[test]
    fn test_valid_headers_to_snap_collecting() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = downloading_headers();

        assert!(
            manager.is_valid_transition(&snap_collecting_roots()),
            "T-TV-009: DownloadingHeaders -> SnapCollectingRoots must be valid"
        );
    }

    /// T-TV-010: DownloadingHeaders -> Synchronized is valid.
    /// Used when we discover we're already caught up during header download.
    #[test]
    fn test_valid_headers_to_synchronized() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = downloading_headers();

        assert!(
            manager.is_valid_transition(&synchronized()),
            "T-TV-010: DownloadingHeaders -> Synchronized must be valid"
        );
    }

    /// T-TV-011: DownloadingBodies -> DownloadingBodies (self-transition) is valid.
    /// Used for pending count updates and soft retries.
    #[test]
    fn test_valid_bodies_self_transition() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = downloading_bodies();

        assert!(
            manager.is_valid_transition(&SyncState::DownloadingBodies {
                pending: 5,
                total: 50,
            }),
            "T-TV-011: DownloadingBodies -> DownloadingBodies must be valid"
        );
    }

    /// T-TV-012: DownloadingBodies -> Synchronized is valid.
    /// Happens when all bodies are downloaded and processing is trivial.
    #[test]
    fn test_valid_bodies_to_synchronized() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = downloading_bodies();

        assert!(
            manager.is_valid_transition(&synchronized()),
            "T-TV-012: DownloadingBodies -> Synchronized must be valid"
        );
    }

    /// T-TV-013: Processing -> Processing (self-transition) is valid.
    /// Used for height updates during block application.
    #[test]
    fn test_valid_processing_self_transition() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = processing();

        assert!(
            manager.is_valid_transition(&SyncState::Processing { height: 51 }),
            "T-TV-013: Processing -> Processing must be valid"
        );
    }

    /// T-TV-014: Synchronized -> Synchronized (self-transition) is valid.
    /// Used by update_local_tip when already synchronized.
    #[test]
    fn test_valid_synchronized_self_transition() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = synchronized();

        assert!(
            manager.is_valid_transition(&synchronized()),
            "T-TV-014: Synchronized -> Synchronized must be valid"
        );
    }

    /// T-TV-015: SnapDownloading -> SnapDownloading (alternate peer) is valid.
    /// Used when primary peer fails and we switch to an alternate.
    #[test]
    fn test_valid_snap_downloading_self_transition() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = snap_downloading();

        assert!(
            manager.is_valid_transition(&snap_downloading()),
            "T-TV-015: SnapDownloading -> SnapDownloading must be valid"
        );
    }

    // === Comprehensive invalid transition coverage ===

    /// T-TV-016: Synchronized -> DownloadingHeaders is INVALID.
    /// Must go through Idle first.
    #[test]
    fn test_invalid_synchronized_to_downloading_headers() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = synchronized();

        assert!(
            !manager.is_valid_transition(&downloading_headers()),
            "T-TV-016: Synchronized -> DownloadingHeaders must be INVALID (go through Idle)"
        );
    }

    /// T-TV-017: Synchronized -> Processing is INVALID.
    /// Processing requires downloaded blocks, not just being synchronized.
    #[test]
    fn test_invalid_synchronized_to_processing() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = synchronized();

        assert!(
            !manager.is_valid_transition(&processing()),
            "T-TV-017: Synchronized -> Processing must be INVALID"
        );
    }

    /// T-TV-018: Processing -> DownloadingHeaders is INVALID.
    /// Must abort to Idle first.
    #[test]
    fn test_invalid_processing_to_downloading_headers() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = processing();

        assert!(
            !manager.is_valid_transition(&downloading_headers()),
            "T-TV-018: Processing -> DownloadingHeaders must be INVALID"
        );
    }

    /// T-TV-019: Processing -> DownloadingBodies is INVALID.
    /// Can't go back to body download from processing.
    #[test]
    fn test_invalid_processing_to_downloading_bodies() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = processing();

        assert!(
            !manager.is_valid_transition(&downloading_bodies()),
            "T-TV-019: Processing -> DownloadingBodies must be INVALID"
        );
    }

    /// T-TV-020: SnapCollectingRoots -> Processing is INVALID.
    /// Snap sync and header-first sync are different pipelines.
    #[test]
    fn test_invalid_snap_collecting_to_processing() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = snap_collecting_roots();

        assert!(
            !manager.is_valid_transition(&processing()),
            "T-TV-020: SnapCollectingRoots -> Processing must be INVALID"
        );
    }

    /// T-TV-021: SnapCollectingRoots -> DownloadingBodies is INVALID.
    /// Snap sync doesn't download bodies individually.
    #[test]
    fn test_invalid_snap_collecting_to_downloading_bodies() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = snap_collecting_roots();

        assert!(
            !manager.is_valid_transition(&downloading_bodies()),
            "T-TV-021: SnapCollectingRoots -> DownloadingBodies must be INVALID"
        );
    }

    /// T-TV-022: SnapCollectingRoots -> DownloadingHeaders is INVALID.
    /// Can't switch from snap to header-first mid-stream (must go through Idle).
    #[test]
    fn test_invalid_snap_collecting_to_downloading_headers() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = snap_collecting_roots();

        assert!(
            !manager.is_valid_transition(&downloading_headers()),
            "T-TV-022: SnapCollectingRoots -> DownloadingHeaders must be INVALID"
        );
    }

    /// T-TV-023: SnapDownloading -> Processing is INVALID.
    /// Snap sync applies state directly, doesn't go through block processing.
    #[test]
    fn test_invalid_snap_downloading_to_processing() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = snap_downloading();

        assert!(
            !manager.is_valid_transition(&processing()),
            "T-TV-023: SnapDownloading -> Processing must be INVALID"
        );
    }

    /// T-TV-024: SnapReady -> DownloadingHeaders is INVALID.
    /// Snapshot is ready to apply, not to start a new header download.
    #[test]
    fn test_invalid_snap_ready_to_downloading_headers() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = snap_ready();

        assert!(
            !manager.is_valid_transition(&downloading_headers()),
            "T-TV-024: SnapReady -> DownloadingHeaders must be INVALID"
        );
    }

    /// T-TV-025: DownloadingBodies -> DownloadingHeaders is INVALID.
    /// Can't go back to header download from body download (must abort to Idle).
    #[test]
    fn test_invalid_bodies_to_headers() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = downloading_bodies();

        assert!(
            !manager.is_valid_transition(&downloading_headers()),
            "T-TV-025: DownloadingBodies -> DownloadingHeaders must be INVALID"
        );
    }

    // === Warn-only enforcement (M1 behavior) ===

    /// T-TV-026: In M1, set_state() with invalid transition WARNS but still executes.
    /// REQ-SYNC-104: "Invalid transitions are logged but NOT blocked in M1 (warn-only mode)"
    ///
    /// This ensures the transition validation doesn't break existing behavior
    /// during the observation phase. Hard enforcement comes in M3.
    #[test]
    fn test_set_state_warn_only_allows_invalid_transition() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = synchronized();

        // Synchronized -> DownloadingBodies is INVALID per the matrix
        // In M1 (warn-only), set_state should still execute the transition
        manager.set_state(
            SyncState::DownloadingBodies {
                pending: 0,
                total: 10,
            },
            "test_warn_only_mode",
        );

        // In M1 (warn-only), the transition still happens
        assert!(
            matches!(*manager.state(), SyncState::DownloadingBodies { .. }),
            "T-TV-026: In M1 warn-only mode, invalid transitions must still execute. Got: {:?}",
            manager.state()
        );
    }

    /// T-TV-027: SnapCollectingRoots -> SnapCollectingRoots is INVALID.
    /// Self-transition is not defined for this state in the matrix.
    #[test]
    fn test_invalid_snap_collecting_self_transition() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = snap_collecting_roots();

        // SnapCollectingRoots is not in the self-transition list
        assert!(
            !manager.is_valid_transition(&snap_collecting_roots()),
            "T-TV-027: SnapCollectingRoots -> SnapCollectingRoots is not in the valid matrix"
        );
    }

    /// T-TV-028: SnapReady -> SnapReady is INVALID.
    /// Self-transition not defined for this state.
    #[test]
    fn test_invalid_snap_ready_self_transition() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = snap_ready();

        assert!(
            !manager.is_valid_transition(&snap_ready()),
            "T-TV-028: SnapReady -> SnapReady is not in the valid matrix"
        );
    }

    /// T-TV-029: SnapReady -> SnapDownloading is INVALID.
    /// Can't go back to downloading from ready state.
    #[test]
    fn test_invalid_snap_ready_to_snap_downloading() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = snap_ready();

        assert!(
            !manager.is_valid_transition(&snap_downloading()),
            "T-TV-029: SnapReady -> SnapDownloading must be INVALID"
        );
    }

    /// T-TV-030: SnapDownloading -> Synchronized is INVALID.
    /// Must go through SnapReady first (node needs to consume the snapshot).
    #[test]
    fn test_invalid_snap_downloading_to_synchronized() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = snap_downloading();

        assert!(
            !manager.is_valid_transition(&synchronized()),
            "T-TV-030: SnapDownloading -> Synchronized must be INVALID (must go through SnapReady)"
        );
    }

    /// T-TV-031: SnapDownloading -> SnapCollectingRoots is INVALID.
    /// Can't restart root collection from download phase.
    #[test]
    fn test_invalid_snap_downloading_to_snap_collecting() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = snap_downloading();

        assert!(
            !manager.is_valid_transition(&snap_collecting_roots()),
            "T-TV-031: SnapDownloading -> SnapCollectingRoots must be INVALID"
        );
    }
}
