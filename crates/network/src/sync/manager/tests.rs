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
    assert!(SyncState::Syncing {
        phase: SyncPhase::DownloadingHeaders,
        started_at: Instant::now(),
    }
    .is_syncing());
    assert!(SyncState::Syncing {
        phase: SyncPhase::DownloadingBodies,
        started_at: Instant::now(),
    }
    .is_syncing());
    assert!(SyncState::Syncing {
        phase: SyncPhase::ProcessingBlocks,
        started_at: Instant::now(),
    }
    .is_syncing());
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

    manager.state = SyncState::Syncing {
        phase: SyncPhase::DownloadingHeaders,
        started_at: Instant::now(),
    };
    manager.pipeline_data = SyncPipelineData::Headers {
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
    manager.state = SyncState::Syncing {
        phase: SyncPhase::ProcessingBlocks,
        started_at: Instant::now(),
    };
    manager.pipeline_data = SyncPipelineData::Processing { height: 1 };
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
        !matches!(
            manager.state,
            SyncState::Syncing {
                phase: SyncPhase::ProcessingBlocks,
                ..
            }
        ),
        "Must not stay stuck in Processing when no pending work remains (state={:?})",
        manager.state
    );
}

#[test]
fn test_processing_stuck_recovery_via_cleanup() {
    // Safety net: even if block_applied doesn't fire, cleanup() detects
    // a stuck Processing state with no pending work.
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.state = SyncState::Syncing {
        phase: SyncPhase::ProcessingBlocks,
        started_at: Instant::now(),
    };
    manager.pipeline_data = SyncPipelineData::Processing { height: 1 };
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
        !matches!(
            manager.state,
            SyncState::Syncing {
                phase: SyncPhase::ProcessingBlocks,
                ..
            }
        ),
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
    manager.disable_snap_sync(); // Test header-first behavior specifically

    let peer = PeerId::random();
    manager.add_peer(peer, 1000, Hash::ZERO, 1000);

    // Trigger sync
    manager.start_sync();
    assert!(matches!(
        manager.state,
        SyncState::Syncing {
            phase: SyncPhase::DownloadingHeaders,
            ..
        }
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
    manager.disable_snap_sync(); // Test header-first behavior specifically

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
        manager.pipeline_data,
        SyncPipelineData::Headers {
            headers_count: 5,
            ..
        }
    ));

    // Now: simulate a stale response (doesn't chain)
    let _ = manager.next_request();
    let wrong_prev = Hash::from_bytes([0xAB; 32]);
    let bad_headers = vec![create_test_header(wrong_prev, 1)];
    let _blocks = manager.handle_response(peer, SyncResponse::Headers(bad_headers));

    // Verify: state STAYS in Syncing:Headers (not reset to Idle)
    assert!(
        matches!(
            manager.state,
            SyncState::Syncing {
                phase: SyncPhase::DownloadingHeaders,
                ..
            }
        ),
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
    manager.pipeline_data = SyncPipelineData::None;

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

// test_pgd002_grace_period_capped removed (M2: grace period layer deleted)

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
    if let SyncPipelineData::Headers { headers_count, .. } = manager.pipeline_data {
        assert_eq!(headers_count, 5, "Should have 5 headers counted");
    } else {
        panic!("Expected Headers pipeline data");
    }

    // Round 2: continuation request
    let req2 = manager.next_request();
    assert!(req2.is_some(), "Should be able to request more headers");

    let batch2 = full_chain[5..10].to_vec();
    let _blocks = manager.handle_response(peer, SyncResponse::Headers(batch2));

    if let SyncPipelineData::Headers { headers_count, .. } = manager.pipeline_data {
        assert_eq!(headers_count, 10, "Should have all 10 headers counted");
    } else {
        panic!("Expected Headers pipeline data");
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
    // Instead, it should use a dedicated signaling mechanism (stuck_fork_signal flag).
    assert!(
        manager.fork.stuck_fork_signal,
        "cleanup() must set stuck_fork_signal instead of forcing consecutive_empty_headers to 3"
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

    // For small gap (5 blocks), cleanup should signal stuck fork
    assert!(
        manager.fork.stuck_fork_signal,
        "Blacklist escalation for small gap must set stuck_fork_signal"
    );
}

// =========================================================================
// INC-001: Sync State Explosion — Rollback Loop Prevention Tests
// REQ-SYNC-001 through REQ-SYNC-006
// =========================================================================

/// REQ-SYNC-001: reset_sync_after_successful_reorg sets Normal.
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

/// REQ-SYNC-001: reset_sync_for_rollback sets Normal recovery phase.
#[test]
fn test_inc001_rollback_sets_normal_recovery() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Simulate a rollback
    manager.reset_sync_for_rollback();

    assert!(
        matches!(manager.recovery_phase, RecoveryPhase::Normal),
        "After rollback, recovery_phase must be Normal, got: {:?}",
        manager.recovery_phase
    );
}

/// REQ-SYNC-006: After successful reorg, start_sync uses header-first.
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

    // start_sync should use header-first sync
    manager.start_sync();

    // Should be in Syncing:Headers (header-first)
    assert!(
        matches!(
            manager.state(),
            SyncState::Syncing {
                phase: SyncPhase::DownloadingHeaders,
                ..
            }
        ),
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
/// and sync would cascade into rollback → ancestor at h=0 → full reset.
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
    manager.pipeline_data = SyncPipelineData::None;

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
    manager.pipeline_data = SyncPipelineData::None;

    let result = manager.can_produce(102);
    assert_eq!(
        result,
        ProductionAuthorization::Authorized,
        "RC-9: Node 3 blocks behind must be allowed to produce immediately. Got: {:?}",
        result
    );
}

// test_inc001_rc9_lag4_blocks_production_with_timeout removed (M2: height lag layer deleted)

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
    manager.state = SyncState::Syncing {
        phase: SyncPhase::DownloadingHeaders,
        started_at: Instant::now(),
    };
    manager.pipeline_data = SyncPipelineData::Headers {
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
    manager.state = SyncState::Syncing {
        phase: SyncPhase::ProcessingBlocks,
        started_at: Instant::now(),
    };
    manager.pipeline_data = SyncPipelineData::Processing { height: 21 };

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
fn test_post_snap_empty_headers_triggers_height_fallback() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // Node just completed snap sync (5s ago)
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
    manager.state = SyncState::Syncing {
        phase: SyncPhase::DownloadingHeaders,
        started_at: Instant::now(),
    };
    manager.pipeline_data = SyncPipelineData::Headers {
        target_slot: 602,
        peer,
        headers_count: 0,
    };

    // Handle empty headers response (peer doesn't recognize our snap hash)
    let response = SyncResponse::Headers(vec![]);
    manager.handle_response(peer, response);

    // INC-I-012 F1: peer should NOT be blacklisted (it's canonical, our hash is wrong)
    assert!(
        !manager.fork.header_blacklisted_peers.contains_key(&peer),
        "F1: Post-snap empty headers must NOT blacklist responding peer"
    );

    // INC-I-012 F1: should NOT trigger genesis resync — use height-based headers instead
    assert!(
        !manager.fork.needs_genesis_resync,
        "F1: Post-snap empty headers must NOT trigger genesis resync"
    );

    // INC-I-012 F1: consecutive_empty_headers should NOT be incremented (not fork evidence)
    assert_eq!(
        manager.fork.consecutive_empty_headers, 0,
        "F1: Post-snap empty headers should not count as fork evidence"
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

    /// Regression: Idle -> Syncing:Headers is a valid and frequently used transition.
    /// Used by: start_sync() in sync_engine.rs (5+ call sites).
    #[test]
    fn test_regression_idle_to_downloading_headers() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.disable_snap_sync(); // Test header-first transition specifically
        assert!(matches!(*manager.state(), SyncState::Idle));

        let peer = PeerId::random();
        manager.add_peer(peer, 100, Hash::ZERO, 100);

        // start_sync transitions Idle -> Syncing:Headers
        manager.start_sync();
        assert!(
            matches!(
                manager.state(),
                SyncState::Syncing {
                    phase: SyncPhase::DownloadingHeaders,
                    ..
                }
            ),
            "Idle -> Syncing:Headers must remain valid. Got: {:?}",
            manager.state()
        );
    }

    /// Regression: Idle -> Syncing:SnapCollecting is used when snap sync starts.
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
        manager.pipeline_data = SyncPipelineData::None;
        manager.start_sync();

        // With gap=200 > threshold=100 and enough peers, snap sync should trigger.
        // If start_sync took the header-first path instead, that's also valid
        // from Idle. The key point: Idle can transition to either.
        assert!(
            matches!(manager.state(), SyncState::Syncing { .. }),
            "Idle -> Syncing (SnapCollecting or Headers) must remain valid. Got: {:?}",
            manager.state()
        );
    }

    /// Regression: Syncing:Headers -> Idle is used on error/timeout/fork detection.
    /// Used by: sync_engine.rs (6+ call sites), cleanup.rs stuck sync detection.
    #[test]
    fn test_regression_downloading_headers_to_idle() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.disable_snap_sync(); // Test header-first transition specifically

        let peer = PeerId::random();
        manager.add_peer(peer, 100, Hash::ZERO, 100);
        manager.start_sync();
        assert!(matches!(
            manager.state(),
            SyncState::Syncing {
                phase: SyncPhase::DownloadingHeaders,
                ..
            }
        ));

        // Simulate chain mismatch detection -> reset to Idle
        manager.set_state(SyncState::Idle, "test_regression_headers_to_idle");
        assert!(
            matches!(*manager.state(), SyncState::Idle),
            "Syncing:Headers -> Idle must remain valid"
        );
    }

    /// Regression: Syncing:Headers -> Synchronized is used when already caught up.
    /// Used by: sync_engine.rs "headers_empty_already_synced".
    #[test]
    fn test_regression_downloading_headers_to_synchronized() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        let peer = PeerId::random();
        manager.state = SyncState::Syncing {
            phase: SyncPhase::DownloadingHeaders,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Headers {
            target_slot: 100,
            peer,
            headers_count: 5,
        };

        manager.set_state(SyncState::Synchronized, "test_regression_headers_to_sync");
        assert!(
            matches!(*manager.state(), SyncState::Synchronized),
            "Syncing:Headers -> Synchronized must remain valid"
        );
    }

    /// Regression: Syncing:Headers -> Syncing:Bodies when all headers collected.
    /// Used by: sync_engine.rs "headers_complete".
    #[test]
    fn test_regression_downloading_headers_to_downloading_bodies() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        let peer = PeerId::random();
        manager.state = SyncState::Syncing {
            phase: SyncPhase::DownloadingHeaders,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Headers {
            target_slot: 100,
            peer,
            headers_count: 50,
        };

        manager.set_syncing(
            SyncPhase::DownloadingBodies,
            SyncPipelineData::Bodies {
                pending: 0,
                total: 50,
            },
            "test_regression_headers_to_bodies",
        );
        assert!(
            matches!(
                *manager.state(),
                SyncState::Syncing {
                    phase: SyncPhase::DownloadingBodies,
                    ..
                }
            ),
            "Syncing:Headers -> Syncing:Bodies must remain valid"
        );
    }

    /// Regression: Syncing:Bodies -> Syncing:Processing when all bodies downloaded.
    /// Used by: sync_engine.rs "bodies_complete".
    #[test]
    fn test_regression_downloading_bodies_to_processing() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = SyncState::Syncing {
            phase: SyncPhase::DownloadingBodies,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Bodies {
            pending: 0,
            total: 50,
        };

        manager.set_syncing(
            SyncPhase::ProcessingBlocks,
            SyncPipelineData::Processing { height: 1 },
            "test_regression_bodies_to_processing",
        );
        assert!(
            matches!(
                *manager.state(),
                SyncState::Syncing {
                    phase: SyncPhase::ProcessingBlocks,
                    ..
                }
            ),
            "Syncing:Bodies -> Syncing:Processing must remain valid"
        );
    }

    /// Regression: Syncing:Bodies -> Syncing:Bodies (soft retry / pipeline data update).
    /// Used by: cleanup.rs "body_stall_soft_retry", sync_engine.rs body count update.
    #[test]
    fn test_regression_downloading_bodies_self_transition() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = SyncState::Syncing {
            phase: SyncPhase::DownloadingBodies,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Bodies {
            pending: 10,
            total: 50,
        };

        manager.set_syncing(
            SyncPhase::DownloadingBodies,
            SyncPipelineData::Bodies {
                pending: 5,
                total: 50,
            },
            "test_regression_bodies_self_transition",
        );
        assert!(
            matches!(
                manager.pipeline_data,
                SyncPipelineData::Bodies { pending: 5, .. }
            ),
            "Syncing:Bodies pipeline data must update on self-transition"
        );
    }

    /// Regression: Syncing:Bodies -> Idle on error.
    /// Used by: cleanup.rs "body_download_exhausted", "cleanup_stuck_sync".
    #[test]
    fn test_regression_downloading_bodies_to_idle() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = SyncState::Syncing {
            phase: SyncPhase::DownloadingBodies,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Bodies {
            pending: 10,
            total: 50,
        };

        manager.set_state(SyncState::Idle, "test_regression_bodies_to_idle");
        assert!(
            matches!(*manager.state(), SyncState::Idle),
            "Syncing:Bodies -> Idle must remain valid"
        );
    }

    /// Regression: Syncing:Processing -> Synchronized on completion.
    /// Used by: block_lifecycle.rs "sync_complete_block_applied".
    #[test]
    fn test_regression_processing_to_synchronized() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = SyncState::Syncing {
            phase: SyncPhase::ProcessingBlocks,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Processing { height: 50 };

        manager.set_state(
            SyncState::Synchronized,
            "test_regression_processing_to_sync",
        );
        assert!(
            matches!(*manager.state(), SyncState::Synchronized),
            "Syncing:Processing -> Synchronized must remain valid"
        );
    }

    /// Regression: Syncing:Processing -> Idle on stall/error.
    /// Used by: block_lifecycle.rs "processing_complete_restart", "block_apply_failed",
    ///          sync_engine.rs "processing_stall_reset".
    #[test]
    fn test_regression_processing_to_idle() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = SyncState::Syncing {
            phase: SyncPhase::ProcessingBlocks,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Processing { height: 50 };

        manager.set_state(SyncState::Idle, "test_regression_processing_to_idle");
        assert!(
            matches!(*manager.state(), SyncState::Idle),
            "Syncing:Processing -> Idle must remain valid"
        );
    }

    /// Regression: Syncing:Processing -> Syncing:Processing (height update).
    /// The Processing pipeline_data carries a height field that updates.
    #[test]
    fn test_regression_processing_self_transition() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = SyncState::Syncing {
            phase: SyncPhase::ProcessingBlocks,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Processing { height: 50 };

        manager.set_syncing(
            SyncPhase::ProcessingBlocks,
            SyncPipelineData::Processing { height: 51 },
            "test_regression_processing_self_transition",
        );
        if let SyncPipelineData::Processing { height } = manager.pipeline_data {
            assert_eq!(height, 51);
        } else {
            panic!("Processing pipeline_data height update must remain valid");
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

    /// Regression: SnapReady pipeline -> Synchronized on snapshot consumed.
    /// Used by: snap_sync.rs "snap_snapshot_applied" via take_snap_snapshot().
    #[test]
    fn test_regression_snap_ready_to_synchronized() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = SyncState::Syncing {
            phase: SyncPhase::SnapDownloading,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::SnapReady {
            snapshot: VerifiedSnapshot {
                block_hash: Hash::ZERO,
                block_height: 100,
                chain_state: vec![],
                utxo_set: vec![],
                producer_set: vec![],
                state_root: Hash::ZERO,
            },
        };

        // take_snap_snapshot transitions to Synchronized
        let snap = manager.take_snap_snapshot();
        assert!(
            snap.is_some(),
            "take_snap_snapshot must return the snapshot"
        );
        assert!(
            matches!(*manager.state(), SyncState::Synchronized),
            "SnapReady -> Synchronized must remain valid"
        );
    }

    /// Regression: Syncing:SnapDownloading -> Idle on error with no alternates.
    /// Used by: snap_sync.rs "snap_download_error_no_alternates".
    #[test]
    fn test_regression_snap_downloading_to_idle() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        let peer = PeerId::random();
        manager.state = SyncState::Syncing {
            phase: SyncPhase::SnapDownloading,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::SnapDownloading {
            target_hash: Hash::ZERO,
            target_height: 100,
            quorum_root: Hash::ZERO,
            peer,
            alternate_peers: vec![],
        };

        manager.set_state(SyncState::Idle, "test_regression_snap_downloading_to_idle");
        assert!(
            matches!(*manager.state(), SyncState::Idle),
            "Syncing:SnapDownloading -> Idle must remain valid"
        );
    }

    /// Regression: All block_lifecycle.rs transitions to Idle work.
    /// Used by: reset_sync_for_rollback, reset_sync_after_successful_reorg,
    ///          reset_local_state.
    #[test]
    fn test_regression_lifecycle_resets_to_idle() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // reset_sync_for_rollback -> Idle
        manager.state = SyncState::Syncing {
            phase: SyncPhase::ProcessingBlocks,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Processing { height: 10 };
        manager.reset_sync_for_rollback();
        assert!(
            matches!(*manager.state(), SyncState::Idle),
            "reset_sync_for_rollback must transition to Idle"
        );

        // reset_sync_after_successful_reorg -> Idle
        manager.state = SyncState::Syncing {
            phase: SyncPhase::ProcessingBlocks,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Processing { height: 10 };
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
        manager.snap.threshold = 500; // Enable snap sync so the gate accepts

        // Initially false
        assert!(
            !manager.needs_genesis_resync(),
            "needs_genesis_resync must be false initially"
        );

        // Gated method sets the flag when gates pass
        let accepted = manager.request_genesis_resync(RecoveryReason::RollbackDeathSpiral {
            peak: 0,
            current: 0,
        });
        assert!(
            accepted,
            "request_genesis_resync must be accepted for fresh node"
        );
        assert!(
            manager.needs_genesis_resync(),
            "needs_genesis_resync must be true after accepted request_genesis_resync()"
        );
    }

    /// Regression: signal_stuck_fork sets stuck_fork_signal correctly.
    #[test]
    fn test_regression_signal_stuck_fork_pattern() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // From Normal -> signal set
        assert!(matches!(manager.recovery_phase, RecoveryPhase::Normal));
        assert!(!manager.fork.stuck_fork_signal);
        manager.signal_stuck_fork();
        assert!(
            manager.fork.stuck_fork_signal,
            "signal_stuck_fork from Normal must set stuck_fork_signal"
        );

        // take_stuck_fork_signal clears it
        assert!(manager.take_stuck_fork_signal());
        assert!(!manager.fork.stuck_fork_signal);

        // From ResyncInProgress -> ignored (no override)
        manager.recovery_phase = RecoveryPhase::ResyncInProgress;
        manager.signal_stuck_fork();
        assert!(
            !manager.fork.stuck_fork_signal,
            "signal_stuck_fork must NOT set signal during ResyncInProgress"
        );
    }
}

// -------------------------------------------------------------------------
// Recovery Gate Tests: request_genesis_resync()
// REQ-SYNC-102 (RecoveryReason enum), REQ-SYNC-103 (gated method)
// Architecture: Section 4 — "New method: request_genesis_resync()"
//
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

    /// T-RG-001b: Deep fork reasons BYPASS the height floor (INC-I-007).
    ///
    /// When multiple peers don't recognize our chain (GenesisFallbackEmptyHeaders),
    /// the node is genuinely on the wrong fork. The floor should not trap it.
    /// Other gates (rate limiting, snap attempt limit) still prevent cascade loops.
    #[test]
    fn test_request_genesis_resync_deep_fork_bypasses_floor() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.confirmed_height_floor = 100;
        manager.snap.threshold = 500;
        manager.snap.attempts = 0;

        // Deep fork reason should bypass the floor
        let result = manager.request_genesis_resync(RecoveryReason::GenesisFallbackEmptyHeaders);

        assert!(
            result,
            "T-RG-001b: GenesisFallbackEmptyHeaders must bypass confirmed_height_floor"
        );
        assert!(
            manager.needs_genesis_resync(),
            "T-RG-001b: needs_genesis_resync flag must be set for deep fork recovery"
        );
    }

    /// T-RG-001c: AllPeersBlacklistedDeepFork also bypasses the height floor.
    #[test]
    fn test_request_genesis_resync_all_peers_blacklisted_bypasses_floor() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.confirmed_height_floor = 100;
        manager.snap.threshold = 500;
        manager.snap.attempts = 0;

        let result = manager.request_genesis_resync(RecoveryReason::AllPeersBlacklistedDeepFork);

        assert!(
            result,
            "T-RG-001c: AllPeersBlacklistedDeepFork must bypass confirmed_height_floor"
        );
    }

    /// T-RG-001d: Non-deep-fork reasons still blocked by floor.
    #[test]
    fn test_request_genesis_resync_non_deep_fork_still_blocked() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.confirmed_height_floor = 100;
        manager.snap.threshold = 500;
        manager.snap.attempts = 0;

        // BodyDownloadPeerError is NOT a deep fork reason
        let result = manager.request_genesis_resync(RecoveryReason::BodyDownloadPeerError);

        assert!(
            !result,
            "T-RG-001d: Non-deep-fork reasons must still be blocked by floor"
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

        let result = manager.request_genesis_resync(RecoveryReason::BodyDownloadPeerError);

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

        let result = manager.request_genesis_resync(RecoveryReason::BodyDownloadPeerError);

        assert!(
            result,
            "T-RG-003b: request_genesis_resync must be accepted at MAX-1 resyncs ({})",
            MAX_CONSECUTIVE_RESYNCS - 1
        );
    }

    /// T-RG-004: Non-emergency reasons REFUSED when snap sync is disabled.
    /// REQ-SYNC-103: Gate 4 — snap sync availability.
    ///
    /// When snap.threshold == u64::MAX (--no-snap-sync), non-emergency reasons
    /// are blocked. Emergency reasons bypass this gate (INC-I-007).
    #[test]
    fn test_request_genesis_resync_refused_when_snap_disabled() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Explicitly disable snap sync (simulates --no-snap-sync)
        manager.disable_snap_sync();
        assert_eq!(manager.snap.threshold, u64::MAX);
        manager.snap.attempts = 0;

        // Non-emergency reason: blocked by snap-disabled gate
        let result =
            manager.request_genesis_resync(RecoveryReason::StuckSyncLargeGap { gap: 2000 });

        assert!(
            !result,
            "T-RG-004: non-emergency reasons must be refused when snap sync disabled"
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

    // --- Helpers: create SyncState variants for testing (3-state model) ---
    // With 3 states, is_valid_transition() always returns true.
    // These tests verify the universal validity of the collapsed state model.

    fn idle() -> SyncState {
        SyncState::Idle
    }

    fn syncing_headers() -> SyncState {
        SyncState::Syncing {
            phase: SyncPhase::DownloadingHeaders,
            started_at: Instant::now(),
        }
    }

    fn syncing_bodies() -> SyncState {
        SyncState::Syncing {
            phase: SyncPhase::DownloadingBodies,
            started_at: Instant::now(),
        }
    }

    fn syncing_processing() -> SyncState {
        SyncState::Syncing {
            phase: SyncPhase::ProcessingBlocks,
            started_at: Instant::now(),
        }
    }

    fn synchronized() -> SyncState {
        SyncState::Synchronized
    }

    fn syncing_snap_collecting() -> SyncState {
        SyncState::Syncing {
            phase: SyncPhase::SnapCollecting,
            started_at: Instant::now(),
        }
    }

    fn syncing_snap_downloading() -> SyncState {
        SyncState::Syncing {
            phase: SyncPhase::SnapDownloading,
            started_at: Instant::now(),
        }
    }

    // === Valid transitions from Idle (Idle -> anything is valid) ===

    /// T-TV-001: Idle can transition to any state (3-state model: all transitions valid).
    #[test]
    fn test_valid_transitions_from_idle() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        let all_states = vec![
            idle(),
            syncing_headers(),
            syncing_bodies(),
            syncing_processing(),
            synchronized(),
            syncing_snap_collecting(),
            syncing_snap_downloading(),
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
    #[test]
    fn test_valid_transition_to_idle_from_any() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        let all_states = vec![
            idle(),
            syncing_headers(),
            syncing_bodies(),
            syncing_processing(),
            synchronized(),
            syncing_snap_collecting(),
            syncing_snap_downloading(),
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

    // === With 3 states, ALL transitions are valid ===

    /// T-TV-003: Syncing -> Synchronized is always valid (3-state model).
    /// Previously SnapCollectingRoots -> Synchronized was "invalid" with 8 variants.
    /// With 3 states, Syncing -> Synchronized is always valid.
    #[test]
    fn test_syncing_to_synchronized_valid() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_snap_collecting();

        assert!(
            manager.is_valid_transition(&synchronized()),
            "T-TV-003: Syncing -> Synchronized must be valid (3-state model)"
        );
    }

    /// T-TV-004: Syncing -> Syncing is always valid (3-state model).
    /// Previously Processing -> SnapCollectingRoots was "invalid".
    /// With 3 states, Syncing -> Syncing (same enum variant) is always valid.
    #[test]
    fn test_syncing_to_syncing_valid() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_processing();

        assert!(
            manager.is_valid_transition(&syncing_snap_collecting()),
            "T-TV-004: Syncing -> Syncing must be valid (3-state model)"
        );
    }

    /// T-TV-005: Synchronized -> Syncing is valid (3-state model).
    /// Previously Synchronized -> DownloadingBodies was "invalid".
    /// With 3 states, Synchronized -> Syncing is always valid.
    #[test]
    fn test_synchronized_to_syncing_valid() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = synchronized();

        assert!(
            manager.is_valid_transition(&syncing_bodies()),
            "T-TV-005: Synchronized -> Syncing must be valid (3-state model)"
        );
    }

    // === Valid forward-path transitions (still valid) ===

    /// T-TV-006: Syncing:Headers -> Syncing:Bodies is valid.
    #[test]
    fn test_valid_header_to_body_transition() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_headers();

        assert!(
            manager.is_valid_transition(&syncing_bodies()),
            "T-TV-006: Syncing:Headers -> Syncing:Bodies must be valid"
        );
    }

    /// T-TV-007: Full snap sync forward path is valid.
    /// SnapCollecting -> SnapDownloading -> Synchronized
    #[test]
    fn test_valid_snap_forward_path() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Step 1: SnapCollecting -> SnapDownloading
        manager.state = syncing_snap_collecting();
        assert!(
            manager.is_valid_transition(&syncing_snap_downloading()),
            "T-TV-007a: SnapCollecting -> SnapDownloading must be valid"
        );

        // Step 2: SnapDownloading -> Synchronized
        manager.state = syncing_snap_downloading();
        assert!(
            manager.is_valid_transition(&synchronized()),
            "T-TV-007b: SnapDownloading -> Synchronized must be valid"
        );
    }

    /// T-TV-008: Full header-first sync forward path is valid.
    /// Idle -> Headers -> Bodies -> Processing -> Synchronized
    #[test]
    fn test_valid_header_first_forward_path() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Idle -> Syncing:Headers
        manager.state = idle();
        assert!(manager.is_valid_transition(&syncing_headers()));

        // Syncing:Headers -> Syncing:Bodies
        manager.state = syncing_headers();
        assert!(manager.is_valid_transition(&syncing_bodies()));

        // Syncing:Bodies -> Syncing:Processing
        manager.state = syncing_bodies();
        assert!(manager.is_valid_transition(&syncing_processing()));

        // Syncing:Processing -> Synchronized
        manager.state = syncing_processing();
        assert!(
            manager.is_valid_transition(&synchronized()),
            "T-TV-008: Full header-first forward path must be valid"
        );
    }

    /// T-TV-009: Syncing:Headers -> Syncing:SnapCollecting is valid.
    #[test]
    fn test_valid_headers_to_snap_collecting() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_headers();

        assert!(
            manager.is_valid_transition(&syncing_snap_collecting()),
            "T-TV-009: Syncing:Headers -> Syncing:SnapCollecting must be valid"
        );
    }

    /// T-TV-010: Syncing:Headers -> Synchronized is valid.
    #[test]
    fn test_valid_headers_to_synchronized() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_headers();

        assert!(
            manager.is_valid_transition(&synchronized()),
            "T-TV-010: Syncing:Headers -> Synchronized must be valid"
        );
    }

    /// T-TV-011: Syncing:Bodies -> Syncing:Bodies (self-transition) is valid.
    #[test]
    fn test_valid_bodies_self_transition() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_bodies();

        assert!(
            manager.is_valid_transition(&syncing_bodies()),
            "T-TV-011: Syncing:Bodies self-transition must be valid"
        );
    }

    /// T-TV-012: Syncing:Bodies -> Synchronized is valid.
    #[test]
    fn test_valid_bodies_to_synchronized() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_bodies();

        assert!(
            manager.is_valid_transition(&synchronized()),
            "T-TV-012: Syncing:Bodies -> Synchronized must be valid"
        );
    }

    /// T-TV-013: Syncing:Processing -> Syncing:Processing (self-transition) is valid.
    #[test]
    fn test_valid_processing_self_transition() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_processing();

        assert!(
            manager.is_valid_transition(&syncing_processing()),
            "T-TV-013: Syncing:Processing self-transition must be valid"
        );
    }

    /// T-TV-014: Synchronized -> Synchronized (self-transition) is valid.
    #[test]
    fn test_valid_synchronized_self_transition() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = synchronized();

        assert!(
            manager.is_valid_transition(&synchronized()),
            "T-TV-014: Synchronized -> Synchronized must be valid"
        );
    }

    /// T-TV-015: Syncing:SnapDownloading -> Syncing:SnapDownloading (alternate peer) is valid.
    #[test]
    fn test_valid_snap_downloading_self_transition() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_snap_downloading();

        assert!(
            manager.is_valid_transition(&syncing_snap_downloading()),
            "T-TV-015: Syncing:SnapDownloading self-transition must be valid"
        );
    }

    // === All transitions valid in 3-state model ===

    /// T-TV-016: Synchronized -> Syncing:Headers is valid.
    #[test]
    fn test_valid_synchronized_to_downloading_headers() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = synchronized();

        assert!(
            manager.is_valid_transition(&syncing_headers()),
            "T-TV-016: Synchronized -> Syncing:Headers must be valid"
        );
    }

    /// T-TV-016b: Synchronized -> Syncing:SnapCollecting is valid.
    #[test]
    fn test_valid_synchronized_to_snap_collecting() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = synchronized();

        assert!(
            manager.is_valid_transition(&syncing_snap_collecting()),
            "T-TV-016b: Synchronized -> Syncing:SnapCollecting must be valid"
        );
    }

    /// T-TV-016c: Syncing:Headers -> Syncing:Headers is valid (self-transition).
    #[test]
    fn test_valid_downloading_headers_self_transition() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_headers();

        assert!(
            manager.is_valid_transition(&syncing_headers()),
            "T-TV-016c: Syncing:Headers self-transition must be valid"
        );
    }

    /// T-TV-017: Synchronized -> Syncing:Processing is valid (3-state model).
    /// Previously "invalid" with 8 variants; now all Syncing transitions are valid.
    #[test]
    fn test_synchronized_to_processing_valid() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = synchronized();

        assert!(
            manager.is_valid_transition(&syncing_processing()),
            "T-TV-017: Synchronized -> Syncing:Processing must be valid (3-state model)"
        );
    }

    /// T-TV-018: Syncing -> Syncing (different phases) is valid (3-state model).
    /// Previously Processing -> DownloadingHeaders was "invalid".
    #[test]
    fn test_syncing_phase_change_valid() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_processing();

        assert!(
            manager.is_valid_transition(&syncing_headers()),
            "T-TV-018: Syncing -> Syncing (phase change) must be valid (3-state model)"
        );
    }

    /// T-TV-019: All 3x3 state transitions are valid.
    /// With 3 states, the full 3x3 matrix (Idle, Syncing, Synchronized) is valid.
    #[test]
    fn test_all_3x3_transitions_valid() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        let states = vec![idle(), syncing_headers(), synchronized()];

        for source in &states {
            for target in &states {
                manager.state = source.clone();
                assert!(
                    manager.is_valid_transition(target),
                    "T-TV-019: {:?} -> {:?} must be valid in 3-state model",
                    std::mem::discriminant(source),
                    std::mem::discriminant(target),
                );
            }
        }
    }

    /// T-TV-020: Syncing:SnapCollecting -> Syncing:Processing is valid (3-state model).
    #[test]
    fn test_snap_collecting_to_processing_valid() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_snap_collecting();

        assert!(
            manager.is_valid_transition(&syncing_processing()),
            "T-TV-020: Syncing -> Syncing must be valid (3-state model)"
        );
    }

    /// T-TV-021: Syncing:SnapCollecting -> Syncing:Bodies is valid (3-state model).
    #[test]
    fn test_snap_collecting_to_bodies_valid() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_snap_collecting();

        assert!(
            manager.is_valid_transition(&syncing_bodies()),
            "T-TV-021: Syncing -> Syncing must be valid (3-state model)"
        );
    }

    /// T-TV-022: Syncing:SnapCollecting -> Syncing:Headers is valid (3-state model).
    #[test]
    fn test_snap_collecting_to_headers_valid() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_snap_collecting();

        assert!(
            manager.is_valid_transition(&syncing_headers()),
            "T-TV-022: Syncing -> Syncing must be valid (3-state model)"
        );
    }

    /// T-TV-023: Syncing:SnapDownloading -> Syncing:Processing is valid (3-state model).
    #[test]
    fn test_snap_downloading_to_processing_valid() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_snap_downloading();

        assert!(
            manager.is_valid_transition(&syncing_processing()),
            "T-TV-023: Syncing -> Syncing must be valid (3-state model)"
        );
    }

    /// T-TV-024: Syncing:SnapDownloading -> Syncing:Headers is valid (3-state model).
    #[test]
    fn test_snap_downloading_to_headers_valid() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_snap_downloading();

        assert!(
            manager.is_valid_transition(&syncing_headers()),
            "T-TV-024: Syncing -> Syncing must be valid (3-state model)"
        );
    }

    /// T-TV-025: Syncing:Bodies -> Syncing:Headers is valid (3-state model).
    #[test]
    fn test_bodies_to_headers_valid() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_bodies();

        assert!(
            manager.is_valid_transition(&syncing_headers()),
            "T-TV-025: Syncing -> Syncing must be valid (3-state model)"
        );
    }

    // === Hard enforcement (M3 behavior) ===

    /// T-TV-026: set_state() always accepts transitions in 3-state model.
    /// With 3 states, is_valid_transition() always returns true.
    /// set_state can transition between any of the 3 states.
    #[test]
    fn test_set_state_accepts_all_transitions() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = synchronized();

        // Synchronized -> Syncing is valid in 3-state model
        manager.set_syncing(
            SyncPhase::DownloadingBodies,
            SyncPipelineData::Bodies {
                pending: 0,
                total: 10,
            },
            "test_set_state_valid",
        );

        assert!(
            matches!(
                *manager.state(),
                SyncState::Syncing {
                    phase: SyncPhase::DownloadingBodies,
                    ..
                }
            ),
            "T-TV-026: set_syncing must accept Synchronized -> Syncing:Bodies. Got: {:?}",
            manager.state()
        );
    }

    /// T-TV-027: Syncing:SnapCollecting self-transition is valid (3-state model).
    #[test]
    fn test_snap_collecting_self_transition_valid() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_snap_collecting();

        assert!(
            manager.is_valid_transition(&syncing_snap_collecting()),
            "T-TV-027: Syncing self-transition must be valid (3-state model)"
        );
    }

    /// T-TV-028: Syncing:SnapDownloading self-transition is valid (3-state model).
    #[test]
    fn test_snap_downloading_repeated_valid() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_snap_downloading();

        assert!(
            manager.is_valid_transition(&syncing_snap_downloading()),
            "T-TV-028: Syncing self-transition must be valid (3-state model)"
        );
    }

    /// T-TV-029: Syncing:SnapDownloading -> Syncing:SnapCollecting is valid (3-state model).
    #[test]
    fn test_snap_downloading_to_snap_collecting_valid() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_snap_downloading();

        assert!(
            manager.is_valid_transition(&syncing_snap_collecting()),
            "T-TV-029: Syncing -> Syncing must be valid (3-state model)"
        );
    }

    /// T-TV-030: Syncing:SnapDownloading -> Synchronized is valid (3-state model).
    #[test]
    fn test_snap_downloading_to_synchronized_valid() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = syncing_snap_downloading();

        assert!(
            manager.is_valid_transition(&synchronized()),
            "T-TV-030: Syncing -> Synchronized must be valid (3-state model)"
        );
    }

    /// T-TV-031: Idle -> Idle self-transition is valid.
    #[test]
    fn test_idle_self_transition_valid() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.state = idle();

        assert!(
            manager.is_valid_transition(&idle()),
            "T-TV-031: Idle -> Idle must be valid (3-state model)"
        );
    }
}

// =========================================================================
// M2: Site Migration + Monotonic Floor Extension Tests
// Architecture: specs/sync-recovery-architecture.md (Sections 4, 5)
// Requirements: REQ-SYNC-102 (monotonic floor), REQ-SYNC-103 (gated method),
//               REQ-SYNC-105 (recovery reason logging), PRESERVE-5 (existing tests pass)
//
// M2 replaces all 9 `needs_genesis_resync = true` direct writes with
// `request_genesis_resync(RecoveryReason::...)` calls. When gates block,
// the resync is REFUSED — this is the behavioral change vs. M1.
//
// M2 also extends confirmed_height_floor to reset_sync_for_rollback().
// =========================================================================

// -------------------------------------------------------------------------
// Site Migration Tests: verify that each migrated write site now routes
// through request_genesis_resync() and respects the recovery gates.
//
// Strategy: For each write site, create the conditions that would trigger
// the genesis resync code path, but also set a gate condition that should
// REFUSE the request. Then verify needs_genesis_resync stays false.
//
// REQ-SYNC-103 (Must): needs_genesis_resync set from 1 path, not 9
// -------------------------------------------------------------------------

mod site_migration_tests {
    use super::*;

    // === Helper: create a SyncManager with the height floor gate active ===
    // confirmed_height_floor > 0 means the node was previously healthy.
    // Gate 1 of request_genesis_resync() will refuse all requests.

    fn manager_with_floor_gate() -> SyncManager {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.confirmed_height_floor = 100;
        // Enable snap sync so it's not the snap-disabled gate that blocks
        manager.snap.threshold = 500;
        manager.snap.attempts = 0;
        manager
    }

    // === Helper: create a SyncManager with the max-resyncs gate active ===
    fn manager_with_resync_count_gate() -> SyncManager {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.consecutive_resync_count = MAX_CONSECUTIVE_RESYNCS;
        manager.snap.threshold = 500;
        manager.snap.attempts = 0;
        manager
    }

    // === Site #7 (cleanup.rs ~344): All peers blacklisted, deep fork ===

    /// T-M2-001: cleanup site "all peers blacklisted deep fork" routes through recovery gate.
    /// REQ-SYNC-103 (Must): When confirmed_height_floor > 0, the gate REFUSES genesis resync.
    ///
    /// Setup: 20+ consecutive empty headers, all peers blacklisted, gap > 12, 3+ peers,
    ///        stuck > 120s. confirmed_height_floor > 0 but deep fork bypasses it (INC-I-007).
    /// Expected: needs_genesis_resync becomes TRUE (deep fork recovery allowed).
    #[test]
    fn test_cleanup_all_blacklisted_uses_recovery_gate() {
        let mut manager = manager_with_floor_gate();

        // Set conditions that trigger site #7:
        // - All peers blacklisted (best_peer() returns None)
        // - should_sync() returns true (network_tip > local)
        // - state == Idle
        // - stuck > 120s
        // - consecutive_empty_headers >= 20
        // - enough_peers (peers.len() >= 3)
        // - gap > 12
        manager.local_height = 100;
        manager.local_hash = crypto::hash::hash(b"block100");
        manager.local_slot = 100;
        manager.network.network_tip_height = 200; // gap = 100 > 12
        manager.network.network_tip_slot = 200;
        manager.state = SyncState::Idle;

        // Add 3+ peers (all will be blacklisted)
        let peers: Vec<PeerId> = (0..3).map(|_| PeerId::random()).collect();
        for &peer in &peers {
            manager.add_peer(peer, 200, Hash::ZERO, 200);
            // Force back to Idle (add_peer may start sync)
        }
        manager.state = SyncState::Idle;

        // Blacklist all peers
        for &peer in &peers {
            manager
                .fork
                .header_blacklisted_peers
                .insert(peer, Instant::now());
        }

        // 20+ consecutive empty headers
        manager.fork.consecutive_empty_headers = 25;

        // Stuck for > 120s
        manager.network.last_block_applied = Instant::now() - Duration::from_secs(130);

        // Run cleanup — this triggers the blacklisted-peers escalation path
        manager.cleanup();

        // INC-I-007: Deep fork reasons (AllPeersBlacklistedDeepFork) bypass the floor.
        // The node is genuinely on the wrong fork — recovery must be allowed.
        assert!(
            manager.fork.needs_genesis_resync,
            "T-M2-001: AllPeersBlacklistedDeepFork must bypass confirmed_height_floor={} \
             for deep fork recovery (INC-I-007).",
            manager.confirmed_height_floor
        );
    }

    // === Site #8 (cleanup.rs ~483): Stuck-sync large gap ===

    /// T-M2-002: cleanup site "stuck sync large gap" routes through recovery gate.
    /// REQ-SYNC-103 (Must): When confirmed_height_floor > 0, gate REFUSES.
    ///
    /// Setup: gap > 1000, snap.attempts < 3, 3+ peers, stuck > 120s.
    /// Expected: needs_genesis_resync stays FALSE.
    #[test]
    fn test_stuck_sync_large_gap_uses_recovery_gate() {
        let mut manager = manager_with_floor_gate();

        manager.local_height = 100;
        manager.local_hash = crypto::hash::hash(b"block100");
        manager.local_slot = 100;
        manager.network.network_tip_height = 1200; // gap = 1100 > 1000
        manager.network.network_tip_slot = 1200;
        manager.state = SyncState::Idle;

        // Add 3+ peers (not blacklisted)
        for _ in 0..3 {
            let peer = PeerId::random();
            manager.add_peer(peer, 1200, Hash::ZERO, 1200);
        }
        manager.state = SyncState::Idle;

        // Stuck for > 120s
        manager.network.last_block_applied = Instant::now() - Duration::from_secs(130);
        // Ensure the "stuck sync" path is reached, not the fork path
        manager.fork.consecutive_empty_headers = 10; // >= 3, so it won't take the small-gap path

        // Run cleanup
        manager.cleanup();

        assert!(
            !manager.fork.needs_genesis_resync,
            "T-M2-002: cleanup 'stuck sync large gap' site must route through recovery gate. \
             With confirmed_height_floor={}, needs_genesis_resync must stay false.",
            manager.confirmed_height_floor
        );
    }

    // === Site #9 (cleanup.rs ~524): Height offset detection ===

    /// T-M2-003: cleanup site "height offset detected" routes through recovery gate.
    /// REQ-SYNC-103 (Must): When consecutive_resync_count >= MAX, gate REFUSES.
    ///
    /// Setup: stable gap for > 120s, blocks recently applied, gap >= 2.
    /// Expected: needs_genesis_resync stays FALSE.
    #[test]
    fn test_height_offset_uses_recovery_gate() {
        let mut manager = manager_with_resync_count_gate();

        manager.local_height = 100;
        manager.local_hash = crypto::hash::hash(b"block100");
        manager.local_slot = 100;
        manager.network.network_tip_height = 110; // gap = 10 >= 2
        manager.network.network_tip_slot = 110;
        manager.state = SyncState::Synchronized; // not Idle, should_sync() still true due to gap

        // Add a peer so should_sync() returns true
        let peer = PeerId::random();
        manager.add_peer(peer, 110, Hash::ZERO, 110);
        manager.state = SyncState::Synchronized;

        // Blocks recently applied (within 30s)
        manager.network.last_block_applied = Instant::now() - Duration::from_secs(10);

        // Stable gap since > 120s ago
        manager.fork.stable_gap_since = Some((10, Instant::now() - Duration::from_secs(130)));

        // Run cleanup
        manager.cleanup();

        assert!(
            !manager.fork.needs_genesis_resync,
            "T-M2-003: cleanup 'height offset' site must route through recovery gate. \
             With consecutive_resync_count={}, needs_genesis_resync must stay false.",
            manager.consecutive_resync_count
        );
    }

    // === Site #4 (sync_engine.rs ~274): Post-rollback snap escalation ===

    // === Site #6 (block_lifecycle.rs ~226): Apply failures, large gap, snap available ===

    /// T-M2-004b: block_lifecycle "apply failures large gap" triggers emergency recovery
    /// even with confirmed_height_floor > 0 (INC-I-007).
    ///
    /// Setup: 3+ consecutive apply failures, gap > 50, snap enabled + attempts < 3.
    /// ApplyFailuresSnapThreshold is emergency — bypasses floor gate.
    #[test]
    fn test_apply_failures_large_gap_uses_recovery_gate() {
        let mut manager = manager_with_floor_gate();

        manager.local_height = 100;
        manager.local_hash = crypto::hash::hash(b"block100");
        manager.local_slot = 100;
        manager.network.network_tip_height = 200; // gap = 100 > 50

        // Set up 2 prior failures so the 3rd triggers escalation
        manager.fork.consecutive_apply_failures = 2;

        // Call block_apply_failed() — the 3rd failure triggers emergency recovery
        manager.block_apply_failed();

        // INC-I-007: ApplyFailuresSnapThreshold is emergency — bypasses floor
        assert!(
            manager.fork.needs_genesis_resync,
            "T-M2-004b: ApplyFailuresSnapThreshold must trigger emergency recovery \
             even with confirmed_height_floor={} (INC-I-007).",
            manager.confirmed_height_floor
        );
    }

    // === Site #6b (block_lifecycle.rs ~232): Apply failures, else branch ===

    /// T-M2-004c: block_lifecycle "apply failures" triggers emergency recovery
    /// even when snap disabled and floor > 0 (INC-I-007).
    ///
    /// Setup: 3+ consecutive apply failures, gap > 50, snap disabled, floor > 0.
    /// ApplyFailuresSnapThreshold is an emergency reason — bypasses both gates.
    #[test]
    fn test_apply_failures_snap_disabled_uses_recovery_gate() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.confirmed_height_floor = 100;
        manager.disable_snap_sync(); // Simulate --no-snap-sync

        manager.local_height = 100;
        manager.local_hash = crypto::hash::hash(b"block100");
        manager.local_slot = 100;
        manager.network.network_tip_height = 200; // gap = 100 > 50

        manager.fork.consecutive_apply_failures = 2;

        manager.block_apply_failed();

        // INC-I-007: ApplyFailuresSnapThreshold is emergency — bypasses floor + snap-disabled
        assert!(
            manager.fork.needs_genesis_resync,
            "T-M2-004c: ApplyFailuresSnapThreshold must trigger emergency recovery \
             even with floor={} and snap disabled (INC-I-007).",
            manager.confirmed_height_floor
        );
    }

    // === Site #5 (sync_engine.rs ~415): Genesis fallback, empty headers ===

    /// T-M2-005a: sync_engine "genesis fallback empty headers" routes through recovery gate.
    /// REQ-SYNC-103 (Must): When snap sync disabled, gate REFUSES.
    ///
    /// This site fires when 10+ consecutive empty headers are received during header download,
    /// with gap > 12 (large gap path). After M2, this calls request_genesis_resync().
    ///
    /// NOTE: This path is complex to trigger through handle_response() because it requires
    /// the node to be in DownloadingHeaders state with a pending request. We test the gate
    /// behavior through request_genesis_resync() directly with the appropriate reason.
    #[test]
    fn test_genesis_fallback_empty_headers_gate_bypasses_snap_disabled() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        // Explicitly disable snap sync — emergency reasons should still bypass gate 4
        manager.disable_snap_sync();
        assert_eq!(manager.snap.threshold, u64::MAX);

        let result = manager.request_genesis_resync(RecoveryReason::GenesisFallbackEmptyHeaders);

        // INC-I-007: Emergency reasons bypass snap-disabled gate
        assert!(
            result,
            "T-M2-005a: GenesisFallbackEmptyHeaders must bypass snap-disabled for emergency recovery"
        );
        assert!(
            manager.fork.needs_genesis_resync,
            "T-M2-005a: needs_genesis_resync must be true for emergency recovery"
        );
    }

    // === Site #6 (sync_engine.rs ~767): Body download peer error ===

    /// T-M2-005b: sync_engine "body download peer error" routes through recovery gate.
    /// REQ-SYNC-103 (Must): When snap attempts exhausted, gate REFUSES.
    #[test]
    fn test_body_download_peer_error_gate_refuses_snap_exhausted() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.snap.threshold = 500; // Snap enabled
        manager.snap.attempts = 3; // Exhausted

        let result = manager.request_genesis_resync(RecoveryReason::BodyDownloadPeerError);

        assert!(
            !result,
            "T-M2-005b: BodyDownloadPeerError must be refused when snap attempts exhausted (3/3)"
        );
        assert!(
            !manager.fork.needs_genesis_resync,
            "T-M2-005b: needs_genesis_resync must stay false"
        );
    }

    // === Site #1 (production_gate.rs ~1087): set_needs_genesis_resync (death spiral) ===

    /// T-M2-006: production_gate set_needs_genesis_resync routes through recovery gate.
    /// REQ-SYNC-103 (Must): After M2, set_needs_genesis_resync() is replaced by
    /// request_genesis_resync(RecoveryReason::RollbackDeathSpiral).
    ///
    /// Verify that the RollbackDeathSpiral reason is refused when floor > 0.
    #[test]
    fn test_set_needs_genesis_resync_replaced_by_gate() {
        let mut manager = manager_with_floor_gate();

        let result = manager.request_genesis_resync(RecoveryReason::RollbackDeathSpiral {
            peak: 500,
            current: 10,
        });

        assert!(
            !result,
            "T-M2-006: RollbackDeathSpiral must be refused when confirmed_height_floor > 0"
        );
        assert!(
            !manager.fork.needs_genesis_resync,
            "T-M2-006: needs_genesis_resync must stay false after gate refusal"
        );
    }

    // === Positive path: sites still work for fresh nodes ===

    /// T-M2-007: apply failures large gap STILL triggers genesis resync for fresh nodes.
    /// REQ-SYNC-103 + PRESERVE-5: The gate should ACCEPT for fresh nodes (floor=0).
    ///
    /// Setup: Fresh node (floor=0), snap enabled, 3+ apply failures, large gap.
    /// Expected: needs_genesis_resync becomes TRUE (gate accepts).
    #[test]
    fn test_apply_failures_still_triggers_for_fresh_nodes() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        // Enable snap sync (floor=0 by default, fresh node)
        manager.snap.threshold = 500;
        manager.snap.attempts = 0;

        manager.local_height = 10;
        manager.local_hash = crypto::hash::hash(b"block10");
        manager.local_slot = 10;
        manager.network.network_tip_height = 200; // gap = 190 > 50

        // Set up 2 prior failures so the 3rd triggers escalation
        manager.fork.consecutive_apply_failures = 2;

        manager.block_apply_failed();

        assert!(
            manager.fork.needs_genesis_resync,
            "T-M2-007: Apply failures on fresh node (floor=0) must still trigger genesis resync. \
             needs_genesis_resync should be true."
        );
    }

    // === Gate specificity: each gate blocks independently ===

    /// T-M2-008: Recovery gate refuses ApplyFailuresSnapThreshold when ResyncInProgress.
    /// REQ-SYNC-103: Gate 2 — no concurrent recovery.
    #[test]
    fn test_apply_failures_gate_refuses_during_resync() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.snap.threshold = 500;
        manager.snap.attempts = 0;
        manager.recovery_phase = RecoveryPhase::ResyncInProgress;

        let result =
            manager.request_genesis_resync(RecoveryReason::ApplyFailuresSnapThreshold { gap: 200 });

        assert!(
            !result,
            "T-M2-008: ApplyFailuresSnapThreshold must be refused during ResyncInProgress"
        );
    }

    // === Comprehensive: verify ALL 8 RecoveryReason variants that M2 uses ===

    /// T-M2-009: Non-deep-fork reasons refused when floor is active.
    /// REQ-SYNC-103 (Must): Gate 1 blocks non-deep-fork reasons.
    ///
    /// Deep fork reasons (GenesisFallbackEmptyHeaders, AllPeersBlacklistedDeepFork)
    /// bypass the floor — see T-RG-001b/001c (INC-I-007).
    #[test]
    fn test_all_m2_reasons_refused_by_floor_gate() {
        // Non-emergency reasons: still blocked by floor
        let blocked_reasons = vec![
            RecoveryReason::StuckSyncLargeGap { gap: 2000 },
            RecoveryReason::HeightOffsetDetected { gap: 500 },
            RecoveryReason::BodyDownloadPeerError,
            RecoveryReason::RollbackDeathSpiral {
                peak: 500,
                current: 10,
            },
        ];

        for reason in blocked_reasons {
            let mut manager = manager_with_floor_gate();

            let result = manager.request_genesis_resync(reason.clone());

            assert!(
                !result,
                "T-M2-009: {:?} must be refused when confirmed_height_floor > 0",
                reason
            );
            assert!(
                !manager.fork.needs_genesis_resync,
                "T-M2-009: needs_genesis_resync must stay false for {:?}",
                reason
            );
        }

        // Emergency reasons: bypass floor AND snap-disabled gate (INC-I-007)
        let bypass_reasons = vec![
            RecoveryReason::GenesisFallbackEmptyHeaders,
            RecoveryReason::AllPeersBlacklistedDeepFork,
            RecoveryReason::ApplyFailuresSnapThreshold { gap: 100 },
        ];

        for reason in bypass_reasons {
            let mut manager = manager_with_floor_gate();

            let result = manager.request_genesis_resync(reason.clone());

            assert!(
                result,
                "T-M2-009: {:?} must BYPASS confirmed_height_floor for deep fork recovery",
                reason
            );
            assert!(
                manager.fork.needs_genesis_resync,
                "T-M2-009: needs_genesis_resync must be true for {:?}",
                reason
            );
        }
    }

    /// T-M2-009b: All 8 RecoveryReason variants used by M2 are ACCEPTED for fresh nodes.
    /// PRESERVE-5: Fresh node recovery must not be broken.
    #[test]
    fn test_all_m2_reasons_accepted_for_fresh_nodes() {
        let reasons = vec![
            RecoveryReason::AllPeersBlacklistedDeepFork,
            RecoveryReason::StuckSyncLargeGap { gap: 2000 },
            RecoveryReason::HeightOffsetDetected { gap: 500 },
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
            manager.snap.threshold = 500; // Enable snap so gate 4 passes

            let result = manager.request_genesis_resync(reason.clone());

            assert!(
                result,
                "T-M2-009b: {:?} must be ACCEPTED for fresh node (floor=0, snap enabled)",
                reason
            );
        }
    }
}

// -------------------------------------------------------------------------
// Floor Extension Tests: confirmed_height_floor in reset_sync_for_rollback()
//
// REQ-SYNC-102 (Must): No node resets below floor via any path
// Architecture Section 4: "Extended checks: confirmed_height_floor in rollback paths"
//
// M2 adds a floor check at the top of reset_sync_for_rollback():
//   if self.local_height > 0 && self.local_height <= self.confirmed_height_floor {
//       warn!("... REFUSED ...");
//       return;
//   }
// -------------------------------------------------------------------------

mod floor_extension_tests {
    use super::*;

    /// T-M2-010: reset_sync_for_rollback REFUSED when height at floor.
    /// REQ-SYNC-102 (Must): Monotonic progress floor prevents rollback below confirmed height.
    ///
    /// Setup: confirmed_height_floor = 50, local_height = 50 (at floor exactly).
    /// Expected: Returns early. State unchanged. recovery_phase stays Normal.
    #[test]
    fn test_reset_sync_for_rollback_refused_at_floor() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.confirmed_height_floor = 50;
        manager.local_height = 50;
        manager.local_hash = crypto::hash::hash(b"block50");
        manager.local_slot = 50;
        manager.state = SyncState::Syncing {
            phase: SyncPhase::ProcessingBlocks,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Processing { height: 50 };
        manager.recovery_phase = RecoveryPhase::Normal;

        manager.reset_sync_for_rollback();

        // Should remain Normal (the early return skips the phase change)
        assert!(
            matches!(manager.recovery_phase, RecoveryPhase::Normal),
            "T-M2-010: recovery_phase must remain Normal when reset_sync_for_rollback is refused. \
             Got: {:?}",
            manager.recovery_phase
        );
        // State should NOT have been reset to Idle
        assert!(
            matches!(
                *manager.state(),
                SyncState::Syncing {
                    phase: SyncPhase::ProcessingBlocks,
                    ..
                }
            ),
            "T-M2-010: state must remain Syncing:Processing when floor blocks rollback. Got: {:?}",
            manager.state()
        );
    }

    /// T-M2-010b: reset_sync_for_rollback REFUSED when height below floor.
    /// REQ-SYNC-102 (Must): Height below floor is also blocked.
    ///
    /// Setup: confirmed_height_floor = 100, local_height = 50 (below floor).
    /// Expected: Returns early. State unchanged.
    #[test]
    fn test_reset_sync_for_rollback_refused_below_floor() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.confirmed_height_floor = 100;
        manager.local_height = 50;
        manager.local_hash = crypto::hash::hash(b"block50");
        manager.local_slot = 50;
        manager.state = SyncState::Syncing {
            phase: SyncPhase::ProcessingBlocks,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Processing { height: 50 };
        manager.recovery_phase = RecoveryPhase::Normal;

        manager.reset_sync_for_rollback();

        assert!(
            matches!(manager.recovery_phase, RecoveryPhase::Normal),
            "T-M2-010b: recovery_phase must remain Normal when height ({}) < floor ({}). Got: {:?}",
            manager.local_height,
            manager.confirmed_height_floor,
            manager.recovery_phase
        );
        assert!(
            matches!(
                *manager.state(),
                SyncState::Syncing {
                    phase: SyncPhase::ProcessingBlocks,
                    ..
                }
            ),
            "T-M2-010b: state must remain Syncing:Processing. Got: {:?}",
            manager.state()
        );
    }

    /// T-M2-011: reset_sync_for_rollback ALLOWED when height above floor.
    /// REQ-SYNC-102 (Must): Heights above the floor can still rollback normally.
    ///
    /// Setup: confirmed_height_floor = 50, local_height = 100 (above floor).
    /// Expected: Proceeds normally. State set to Idle. recovery_phase set to Normal.
    #[test]
    fn test_reset_sync_for_rollback_allowed_above_floor() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.confirmed_height_floor = 50;
        manager.local_height = 100;
        manager.local_hash = crypto::hash::hash(b"block100");
        manager.local_slot = 100;
        manager.state = SyncState::Syncing {
            phase: SyncPhase::ProcessingBlocks,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Processing { height: 100 };
        manager.recovery_phase = RecoveryPhase::Normal;

        manager.reset_sync_for_rollback();

        // Should proceed normally
        assert!(
            matches!(manager.recovery_phase, RecoveryPhase::Normal),
            "T-M2-011: recovery_phase must be Normal when height ({}) > floor ({}). Got: {:?}",
            100,
            50,
            manager.recovery_phase
        );
        assert!(
            matches!(*manager.state(), SyncState::Idle),
            "T-M2-011: state must be Idle after allowed rollback. Got: {:?}",
            manager.state()
        );
    }

    /// T-M2-012: reset_sync_for_rollback ALLOWED with zero floor (default).
    /// REQ-SYNC-102 (Must): Floor = 0 means unconstrained — fresh nodes can rollback.
    ///
    /// Setup: confirmed_height_floor = 0 (default), local_height = 10.
    /// Expected: Proceeds normally.
    #[test]
    fn test_reset_sync_for_rollback_allowed_with_zero_floor() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Floor is 0 by default
        assert_eq!(manager.confirmed_height_floor, 0);
        manager.local_height = 10;
        manager.local_hash = crypto::hash::hash(b"block10");
        manager.local_slot = 10;
        manager.state = SyncState::Syncing {
            phase: SyncPhase::ProcessingBlocks,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Processing { height: 10 };

        manager.reset_sync_for_rollback();

        assert!(
            matches!(manager.recovery_phase, RecoveryPhase::Normal),
            "T-M2-012: Floor=0 must not constrain rollback. recovery_phase should be Normal. \
             Got: {:?}",
            manager.recovery_phase
        );
        assert!(
            matches!(*manager.state(), SyncState::Idle),
            "T-M2-012: state must be Idle after allowed rollback. Got: {:?}",
            manager.state()
        );
    }

    /// T-M2-012b: reset_sync_for_rollback ALLOWED when local_height is 0.
    /// Edge case: The condition checks `self.local_height > 0` first, so height=0
    /// is always allowed regardless of floor value. This prevents blocking at genesis.
    #[test]
    fn test_reset_sync_for_rollback_allowed_at_height_zero() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.confirmed_height_floor = 100; // Floor > 0 but height = 0
        manager.local_height = 0;
        manager.local_hash = Hash::ZERO;
        manager.local_slot = 0;
        manager.state = SyncState::Syncing {
            phase: SyncPhase::ProcessingBlocks,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Processing { height: 0 };

        manager.reset_sync_for_rollback();

        // Height=0 bypasses the floor check (local_height > 0 is false)
        assert!(
            matches!(manager.recovery_phase, RecoveryPhase::Normal),
            "T-M2-012b: Height=0 must bypass floor check. Got: {:?}",
            manager.recovery_phase
        );
    }

    /// T-M2-013: reset_sync_for_rollback floor check is exact boundary.
    /// Edge case: local_height = floor + 1 should be ALLOWED (just above floor).
    #[test]
    fn test_reset_sync_for_rollback_boundary_floor_plus_one() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.confirmed_height_floor = 50;
        manager.local_height = 51; // One above floor
        manager.local_hash = crypto::hash::hash(b"block51");
        manager.local_slot = 51;
        manager.state = SyncState::Syncing {
            phase: SyncPhase::ProcessingBlocks,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Processing { height: 51 };

        manager.reset_sync_for_rollback();

        assert!(
            matches!(manager.recovery_phase, RecoveryPhase::Normal),
            "T-M2-013: Height 51 (floor+1) must be allowed to rollback. Got: {:?}",
            manager.recovery_phase
        );
    }

    /// T-M2-014: reset_sync_for_rollback does NOT affect reset_sync_after_successful_reorg.
    /// The floor check is ONLY in reset_sync_for_rollback (rejected/unknown reorgs),
    /// NOT in reset_sync_after_successful_reorg (which is called for accepted reorgs).
    ///
    /// Rationale: A successful reorg means we validated the new chain and accepted it.
    /// The floor should not prevent a successful reorg — the new chain IS canonical.
    #[test]
    fn test_successful_reorg_not_blocked_by_floor() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.confirmed_height_floor = 100;
        manager.local_height = 50; // Below floor
        manager.local_hash = crypto::hash::hash(b"block50");
        manager.local_slot = 50;
        manager.state = SyncState::Syncing {
            phase: SyncPhase::ProcessingBlocks,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Processing { height: 50 };

        // Successful reorg should NOT be blocked by the floor
        manager.reset_sync_after_successful_reorg();

        assert!(
            matches!(manager.recovery_phase, RecoveryPhase::Normal),
            "T-M2-014: reset_sync_after_successful_reorg must NOT be blocked by floor. Got: {:?}",
            manager.recovery_phase
        );
        assert!(
            matches!(*manager.state(), SyncState::Idle),
            "T-M2-014: state must be Idle after successful reorg. Got: {:?}",
            manager.state()
        );
    }

    /// T-M2-015: Floor check interacts correctly with existing reset_local_state floor.
    /// Both reset_local_state() AND reset_sync_for_rollback() should refuse when at floor.
    /// This ensures the monotonic progress guarantee covers BOTH reset paths.
    #[test]
    fn test_both_reset_paths_respect_floor() {
        // Path 1: reset_local_state
        let mut manager1 = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager1.confirmed_height_floor = 50;
        manager1.local_height = 50;
        manager1.local_hash = crypto::hash::hash(b"block50");
        manager1.local_slot = 50;
        manager1.state = SyncState::Synchronized;

        manager1.reset_local_state(Hash::ZERO);

        assert!(
            manager1.local_height > 0,
            "T-M2-015a: reset_local_state must not reduce height to 0 when floor=50. Got: {}",
            manager1.local_height
        );

        // Path 2: reset_sync_for_rollback
        let mut manager2 = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager2.confirmed_height_floor = 50;
        manager2.local_height = 50;
        manager2.local_hash = crypto::hash::hash(b"block50");
        manager2.local_slot = 50;
        manager2.state = SyncState::Syncing {
            phase: SyncPhase::ProcessingBlocks,
            started_at: Instant::now(),
        };
        manager2.pipeline_data = SyncPipelineData::Processing { height: 50 };
        manager2.recovery_phase = RecoveryPhase::Normal;

        manager2.reset_sync_for_rollback();

        assert!(
            matches!(manager2.recovery_phase, RecoveryPhase::Normal),
            "T-M2-015b: reset_sync_for_rollback must refuse when at floor. Got: {:?}",
            manager2.recovery_phase
        );
    }
}

// -------------------------------------------------------------------------
// M2 Regression Tests: Ensure existing behavior is preserved.
//
// PRESERVE-5: All existing tests must pass after M2 changes.
// -------------------------------------------------------------------------

mod m2_regression_tests {
    use super::*;

    /// T-M2-020: Fresh nodes (floor=0, no prior sync) can still trigger genesis resync.
    /// PRESERVE-5: The gate must not break new node onboarding.
    ///
    /// Fresh nodes have floor=0, consecutive_resync_count=0, snap enabled.
    /// All 5 gates should pass, allowing genesis resync.
    #[test]
    fn test_genesis_resync_still_works_for_fresh_nodes() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Enable snap sync (fresh node with snap configured)
        manager.snap.threshold = 500;

        // Verify all gate prerequisites are at fresh-node defaults
        assert_eq!(
            manager.confirmed_height_floor, 0,
            "Fresh node floor must be 0"
        );
        assert!(
            matches!(manager.recovery_phase, RecoveryPhase::Normal),
            "Fresh node recovery_phase must be Normal"
        );
        assert_eq!(
            manager.consecutive_resync_count, 0,
            "Fresh node resync_count must be 0"
        );
        assert_eq!(
            manager.snap.attempts, 0,
            "Fresh node snap attempts must be 0"
        );

        // Every RecoveryReason that a write site uses must be accepted
        let result = manager.request_genesis_resync(RecoveryReason::AllPeersBlacklistedDeepFork);
        assert!(
            result,
            "T-M2-020: Fresh node must accept genesis resync. Got refused."
        );
        assert!(
            manager.fork.needs_genesis_resync,
            "T-M2-020: needs_genesis_resync must be true for fresh node."
        );
    }

    /// T-M2-021: reset_sync_for_rollback sets Normal for rollback above floor.
    /// PRESERVE-5: The floor extension must not break normal rollback behavior.
    ///
    /// Normal operation: floor is set via confirmed_height_floor from Synchronized state.
    /// If local_height > floor (typical — node advanced past the floor), rollback proceeds.
    #[test]
    fn test_rollback_works_normally_above_floor() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Simulate a node that reached Synchronized and set a floor at 100
        manager.confirmed_height_floor = 100;
        manager.local_height = 150; // 50 blocks above the floor
        manager.local_hash = crypto::hash::hash(b"block150");
        manager.local_slot = 150;
        manager.state = SyncState::Syncing {
            phase: SyncPhase::ProcessingBlocks,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Processing { height: 150 };

        manager.reset_sync_for_rollback();

        assert!(
            matches!(manager.recovery_phase, RecoveryPhase::Normal),
            "T-M2-021: Normal rollback above floor must set Normal. Got: {:?}",
            manager.recovery_phase
        );
        assert!(
            matches!(*manager.state(), SyncState::Idle),
            "T-M2-021: Normal rollback must set state to Idle. Got: {:?}",
            manager.state()
        );
    }

    /// T-M2-022: block_apply_failed still triggers signal_stuck_fork for small gaps.
    /// PRESERVE-5: The small-gap path (gap <= 50) was NOT migrated — it still calls
    /// signal_stuck_fork(), not request_genesis_resync(). This is intentional.
    #[test]
    fn test_apply_failures_small_gap_still_signals_fork() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.local_height = 100;
        manager.local_hash = crypto::hash::hash(b"block100");
        manager.local_slot = 100;
        manager.network.network_tip_height = 130; // gap = 30 <= 50

        // 2 prior failures, 3rd triggers escalation
        manager.fork.consecutive_apply_failures = 2;

        manager.block_apply_failed();

        // Small gap path calls signal_stuck_fork(), NOT request_genesis_resync()
        assert!(
            !manager.fork.needs_genesis_resync,
            "T-M2-022: Small gap (<=50) must NOT trigger genesis resync, \
             should use signal_stuck_fork() instead."
        );
        assert!(
            manager.fork.stuck_fork_signal,
            "T-M2-022: Small gap must set stuck_fork_signal. Got: {:?}",
            manager.fork.stuck_fork_signal
        );
    }

    /// T-M2-023: set_needs_genesis_resync() has been removed.
    /// All callers now use request_genesis_resync(RecoveryReason::RollbackDeathSpiral).
    /// Verify the gate path works for the death spiral case (fresh node, floor=0).
    #[test]
    fn test_death_spiral_uses_gated_recovery() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.snap.threshold = 500; // Enable snap sync so gate 4 passes

        // Fresh node (floor=0): request should be accepted
        let accepted = manager.request_genesis_resync(RecoveryReason::RollbackDeathSpiral {
            peak: 100,
            current: 5,
        });
        assert!(
            accepted,
            "T-M2-023: RollbackDeathSpiral must be accepted for fresh node (floor=0)"
        );
        assert!(
            manager.fork.needs_genesis_resync,
            "T-M2-023: flag must be set after accepted request"
        );
    }

    /// T-M2-024: cleanup still works for non-genesis-resync paths.
    /// PRESERVE-5: cleanup() has 13+ timeout actions. Only 3 write sites are migrated.
    /// All other paths (signal_stuck_fork, blacklist clearing, stall detection) must
    /// continue working unchanged.
    #[test]
    fn test_cleanup_non_resync_paths_unchanged() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.local_height = 100;
        manager.local_hash = crypto::hash::hash(b"block100");
        manager.local_slot = 100;
        manager.network.network_tip_height = 110; // Small gap
        manager.network.network_tip_slot = 110;
        manager.state = SyncState::Idle;

        // Add peers (not blacklisted)
        for _ in 0..3 {
            let peer = PeerId::random();
            manager.add_peer(peer, 110, Hash::ZERO, 110);
        }
        manager.state = SyncState::Idle;

        // Stuck for > 120s with small gap
        manager.network.last_block_applied = Instant::now() - Duration::from_secs(130);
        manager.fork.consecutive_empty_headers = 0; // < 3, takes the signal_stuck_fork path

        manager.cleanup();

        // Small gap + stuck should trigger signal_stuck_fork, not genesis resync
        // (This is the existing behavior that must be preserved)
        assert!(
            !manager.fork.needs_genesis_resync,
            "T-M2-024: Small gap stuck-sync must not trigger genesis resync"
        );
    }

    // =====================================================================
    // ADVERSARIAL TESTS — INC-I-014 / INC-I-010 attack surface
    // =====================================================================

    /// P1: Verify confirmed_height_floor is set when state=Synchronized +
    /// consecutive_resync_count=0. The floor prevents regression via
    /// reset_local_state() (INC-I-005 Fix C).
    #[test]
    fn test_adversarial_confirmed_floor_monotonic() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Directly set state to Synchronized (avoids sync trigger complexity)
        manager.state = SyncState::Synchronized;
        manager.local_height = 100;
        manager.local_slot = 100;
        // consecutive_resync_count is 0 by default

        // Apply 5 blocks in Synchronized state
        for i in 101..=105 {
            let hash = crypto::hash::hash(format!("block_{}", i).as_bytes());
            manager.block_applied_with_weight(hash, i, i as u32, 1, Hash::ZERO);
        }

        let floor = manager.confirmed_height_floor();
        assert!(
            floor > 0,
            "Floor should be established after applying blocks in Synchronized state, got {}",
            floor
        );
        assert_eq!(floor, 105, "Floor should be at latest applied height");

        // Floor must never decrease
        let floor_after = manager.confirmed_height_floor();
        assert!(
            floor_after >= floor,
            "Floor must be monotonically increasing"
        );
    }

    /// P0: Verify production is blocked during active sync.
    #[test]
    fn test_adversarial_production_blocked_during_sync() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        let peer = PeerId::random();
        manager.add_peer(peer, 1000, crypto::hash::hash(b"peer_tip"), 1000);

        let auth = manager.can_produce(1);
        assert!(
            !matches!(auth, ProductionAuthorization::Authorized),
            "Production should be blocked during sync, got: {:?}",
            auth
        );
    }

    /// P1: Verify can_produce with no peers blocks production.
    #[test]
    fn test_adversarial_production_blocked_zero_peers() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.local_height = 100;
        manager.local_slot = 100;
        manager.first_peer_status_received = Some(Instant::now());

        let auth = manager.can_produce(101);
        assert!(
            !matches!(auth, ProductionAuthorization::Authorized),
            "Production should be blocked with 0 peers, got: {:?}",
            auth
        );
    }

    /// P1: Verify block_applied properly resets fork counters.
    #[test]
    fn test_adversarial_block_applied_resets_fork_counters() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        for _ in 0..5 {
            manager.fork.consecutive_empty_headers += 1;
        }
        assert!(manager.fork.consecutive_empty_headers >= 5);

        let hash = crypto::hash::hash(b"block1");
        manager.block_applied_with_weight(hash, 1, 1, 1, Hash::ZERO);

        assert_eq!(
            manager.fork.consecutive_empty_headers, 0,
            "block_applied should reset consecutive_empty_headers"
        );
    }

    /// P1: Adding many peers doesn't cause quadratic behavior.
    #[test]
    fn test_adversarial_many_peers_performance() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        let start = Instant::now();
        for i in 0..1000u64 {
            let peer = PeerId::random();
            let hash = crypto::hash::hash(format!("peer_{}", i).as_bytes());
            manager.add_peer(peer, i, hash, i as u32);
        }
        let add_time = start.elapsed();

        let best = manager.best_peer_for_recovery();
        assert!(best.is_some(), "Should find a best peer");

        assert!(
            add_time < Duration::from_secs(2),
            "Adding 1000 peers took {:?} — too slow",
            add_time
        );
    }

    /// P0: ForkState.recommend_action handles all edge cases.
    #[test]
    fn test_adversarial_fork_action_edge_cases() {
        use super::ForkState;

        // Deep fork (10+ empty headers) -> genesis resync
        let mut fork = ForkState::new();
        fork.consecutive_empty_headers = 10;
        let action = fork.recommend_action(5, 0, 12, Some(PeerId::random()));
        assert!(matches!(action, ForkAction::NeedsGenesisResync));

        // needs_genesis_resync flag overrides everything
        let mut fork2 = ForkState::new();
        fork2.needs_genesis_resync = true;
        let action2 = fork2.recommend_action(0, 0, 12, None);
        assert!(matches!(action2, ForkAction::NeedsGenesisResync));

        // Gap > max_rollback_depth escalates
        let fork3 = ForkState::new();
        let action3 = fork3.recommend_action(100, 0, 12, Some(PeerId::random()));
        assert!(matches!(action3, ForkAction::NeedsGenesisResync));

        // Shallow fork with < 3 empty headers -> None
        let mut fork4 = ForkState::new();
        fork4.consecutive_empty_headers = 2;
        let action4 = fork4.recommend_action(5, 0, 12, Some(PeerId::random()));
        assert!(matches!(action4, ForkAction::None));

        // Shallow fork with >= 3 empty headers and rollbacks < max -> rollback
        let mut fork5 = ForkState::new();
        fork5.consecutive_empty_headers = 3;
        let action5 = fork5.recommend_action(5, 0, 12, Some(PeerId::random()));
        assert!(matches!(action5, ForkAction::RollbackOne));

        // Exhausted rollbacks
        let mut fork6 = ForkState::new();
        fork6.consecutive_empty_headers = 3;
        let action6 = fork6.recommend_action(5, 12, 12, Some(PeerId::random()));
        assert!(matches!(action6, ForkAction::None));
    }

    /// P2: SyncPipelineData.is_snap_syncing for all variants.
    #[test]
    fn test_adversarial_pipeline_data_snap_syncing() {
        assert!(!SyncPipelineData::None.is_snap_syncing());
        assert!(!SyncPipelineData::Headers {
            target_slot: 100,
            peer: PeerId::random(),
            headers_count: 50,
        }
        .is_snap_syncing());
        assert!(!SyncPipelineData::Bodies {
            pending: 10,
            total: 100,
        }
        .is_snap_syncing());
        assert!(!SyncPipelineData::Processing { height: 42 }.is_snap_syncing());
        assert!(SyncPipelineData::SnapCollecting {
            target_hash: Hash::ZERO,
            target_height: 100,
            votes: vec![],
            asked: std::collections::HashSet::new(),
        }
        .is_snap_syncing());
        assert!(SyncPipelineData::SnapDownloading {
            target_hash: Hash::ZERO,
            target_height: 100,
            quorum_root: Hash::ZERO,
            peer: PeerId::random(),
            alternate_peers: vec![],
        }
        .is_snap_syncing());
    }

    /// P1 BUG FOUND: complete_resync() transitions to Normal, NOT
    /// PostRecoveryGrace. The RecoveryPhase enum defines the lifecycle as:
    ///   Normal -> ResyncInProgress -> PostRecoveryGrace -> Normal
    /// But the actual code (production_gate.rs:215) does:
    ///   complete_resync() { self.recovery_phase = Normal; }
    ///
    /// This means there's NO grace period after resync completes — the node
    /// can immediately start producing blocks before it has confirmed
    /// it's on the canonical chain. The PostRecoveryGrace variant exists
    /// in the enum but is never entered via complete_resync().
    ///
    /// IMPACT: After a forced resync, the node may produce blocks before
    /// receiving a canonical gossip block, potentially extending a fork.
    /// The AwaitingCanonicalBlock phase (entered via snap sync) partially
    /// mitigates this, but complete_resync() bypasses that check.
    #[test]
    fn test_adversarial_recovery_phase_lifecycle() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        assert!(matches!(manager.recovery_phase, RecoveryPhase::Normal));

        manager.recovery_phase = RecoveryPhase::ResyncInProgress;
        assert!(manager.is_resync_in_progress());

        manager.complete_resync();

        // Fixed: now correctly transitions to PostRecoveryGrace
        assert!(
            matches!(
                manager.recovery_phase,
                RecoveryPhase::PostRecoveryGrace {
                    blocks_applied: 0,
                    ..
                }
            ),
            "complete_resync() should transition to PostRecoveryGrace, not Normal"
        );

        // Verify last_resync_completed is set
        assert!(manager.last_resync_completed.is_some());
    }

    /// P3: Snap sync state defaults are sane.
    #[test]
    fn test_adversarial_snap_sync_defaults() {
        let manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        assert_eq!(manager.snap.threshold, 50);
        assert_eq!(manager.snap.quorum, 3);
        assert_eq!(manager.snap.attempts, 0);
        assert!(manager.snap.blacklisted_peers.is_empty());
    }
}
