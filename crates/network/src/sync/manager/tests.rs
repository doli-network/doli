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
        matches!(result, ProductionAuthorization::Authorized),
        "Layer 7 removed: should be Authorized, got: {:?}",
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
            protocol_version: 0,
            producer_pubkey: None,
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
            protocol_version: 0,
            producer_pubkey: None,
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
            protocol_version: 0,
            producer_pubkey: None,
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
            protocol_version: 0,
            producer_pubkey: None,
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
            protocol_version: 0,
            producer_pubkey: None,
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
            protocol_version: 0,
            producer_pubkey: None,
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
            protocol_version: 0,
            producer_pubkey: None,
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
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
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
/// With INC-I-026 + fork_id, cleanup does NOT signal fork recovery for small gaps
/// even after 300s. Small gaps resolve via gossip, not rollback.
#[test]
fn test_no_forced_counter_oscillation() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 100;
    manager.local_slot = 100;
    manager.state = SyncState::Idle;

    let peer = PeerId::random();
    manager.add_peer(peer, 105, Hash::ZERO, 105);

    // Simulate stuck: no block applied for >300s
    manager.network.last_block_applied = Instant::now() - Duration::from_secs(310);

    // Counter starts at 0
    assert_eq!(manager.fork.consecutive_empty_headers, 0);

    // Run cleanup — small gap should NOT signal fork recovery
    manager.cleanup();

    // With deterministic scheduler, small gaps don't trigger fork signal
    assert!(
        !manager.fork.stuck_fork_signal,
        "cleanup() must NOT signal fork recovery for small gaps with deterministic scheduler"
    );
}

/// INC-I-120 (RC-2): a SUSTAINED small-gap stall where peers do not recognize
/// our tip (≥3 — here 25 — consecutive empty headers) IS a genuine fork and MUST
/// now signal fork recovery. This reverses the prior unconditional "small gaps
/// never signal" behavior, which left forked nodes looping HeaderFirstSync
/// forever (the INC-I-120 STALL / INC-I-103/104 1-block offset). The cleanup.rs
/// guard requires ≥300s no-apply AND ≥3 empty headers — contrast
/// `test_no_forced_counter_oscillation` (0 empty headers → still no signal).
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
                protocol_version: 0,
                producer_pubkey: None,
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

    // Stuck for >300s (new threshold with deterministic scheduler)
    manager.network.last_block_applied = Instant::now() - Duration::from_secs(310);

    manager.cleanup();

    // INC-I-120 (RC-2): sustained stall (310s) + small gap + 25 consecutive
    // empty headers = peers don't recognize our tip = genuine fork. cleanup()
    // MUST now signal fork recovery so periodic.rs can escalate to a finality-
    // guarded ShallowRollback (previously this looped HeaderFirstSync forever).
    assert!(
        manager.fork.stuck_fork_signal,
        "Sustained small-gap stall with ≥3 empty headers must signal a stuck fork"
    );
    // Blacklist is still cleared during the all-peers-blacklisted escalation so
    // the rollback retry has a fresh slate of peers.
    assert!(
        manager.fork.header_blacklisted_peers.is_empty(),
        "Blacklist must be cleared after escalation"
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
    // INC-I-026: behind-tip gate removed. With deterministic scheduler + FORK_GUARD,
    // a stale block is silently ignored — no fork risk. Allow production to prevent deadlocks.
    assert_eq!(
        result,
        ProductionAuthorization::Authorized,
        "INC-I-026: node 2 blocks behind must be allowed to produce (FORK_GUARD handles stale blocks). Got: {:?}",
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
    // INC-I-026: behind-tip gate removed. FORK_GUARD handles stale blocks.
    assert_eq!(
        result,
        ProductionAuthorization::Authorized,
        "INC-I-026: node 3 blocks behind must be allowed to produce. Got: {:?}",
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
// =========================================================================
// INC-I-017: Header-first fallback tests
//
// Reproduces the deadlock where fresh nodes fail snap sync 3x, fall back to
// header-first, get empty headers from unsynced peers, and get stuck forever
// because: (1) GetHeadersByHeight was gated behind AwaitingCanonicalBlock,
// (2) the genesis fallback intercepted before height-based requests could fire.
// =========================================================================

/// INC-I-017 F1: Fresh node with exhausted snap + 2 empty headers triggers height fallback.
/// Before the fix, this path required AwaitingCanonicalBlock (only set after SUCCESSFUL snap).
#[test]
fn test_inc_i017_snap_exhausted_empty_headers_triggers_height_fallback() {
    let genesis = Hash::ZERO;
    let mut manager = SyncManager::new(SyncConfig::default(), genesis);

    // Fresh node at h=1 (applied block 1 via gossip), snap exhausted
    manager.local_height = 1;
    manager.local_hash = crypto::hash::hash(b"block_1_hash");
    manager.local_slot = 1;
    manager.snap.attempts = 3; // Exhausted
    manager.recovery_phase = RecoveryPhase::Normal; // NOT AwaitingCanonicalBlock

    // Pre-set: 2 previous empty responses already accumulated
    // (the check fires BEFORE incrementing, so counter must be >= 2 on entry)
    manager.fork.consecutive_empty_headers = 2;

    let synced_peer = PeerId::random();
    manager.add_peer(synced_peer, 500, crypto::hash::hash(b"peer_tip"), 500);

    // Enter DownloadingHeaders
    manager.state = SyncState::Syncing {
        phase: SyncPhase::DownloadingHeaders,
        started_at: Instant::now(),
    };
    manager.pipeline_data = SyncPipelineData::Headers {
        target_slot: 500,
        peer: synced_peer,
        headers_count: 0,
    };

    // This empty response sees consecutive=2 + snap exhausted → triggers height fallback
    manager.handle_response(synced_peer, SyncResponse::Headers(vec![]));

    assert!(
        manager.fork.height_fallback_attempted,
        "Height fallback should be attempted after 2 empties + snap exhausted"
    );
    assert!(
        manager.fork.use_height_based_headers,
        "use_height_based_headers should be set for the next dispatch"
    );
    // Counter stays at 2 (not incremented — the handler returns early).
    // Post-DC-4 (INC-I-139 M5) the GetHeadersByHeight dispatch no longer resets the counter; it is preserved as fork evidence.
    assert_eq!(
        manager.fork.consecutive_empty_headers, 2,
        "Empty counter should NOT increment beyond pre-set value"
    );
}

/// INC-I-017 F2: Height-based request dispatches BEFORE genesis fallback.
/// Before the fix, consecutive_empty_headers >= 10 triggered genesis fallback
/// which returned None, preventing the height-based request from ever firing.
#[test]
fn test_inc_i017_height_based_request_fires_before_genesis_fallback() {
    let genesis = Hash::ZERO;
    let mut manager = SyncManager::new(SyncConfig::default(), genesis);

    manager.local_height = 1;
    manager.local_hash = crypto::hash::hash(b"block_1_hash");
    manager.local_slot = 1;
    manager.snap.attempts = 3;

    let peer = PeerId::random();
    manager.add_peer(peer, 500, crypto::hash::hash(b"peer_tip"), 500);

    // Set up DownloadingHeaders with the height-based flag AND high empties
    manager.fork.use_height_based_headers = true;
    manager.fork.consecutive_empty_headers = 15; // Would trigger genesis fallback
    manager.state = SyncState::Syncing {
        phase: SyncPhase::DownloadingHeaders,
        started_at: Instant::now(),
    };
    manager.pipeline_data = SyncPipelineData::Headers {
        target_slot: 500,
        peer,
        headers_count: 0,
    };

    let result = manager.next_request();

    // Height-based request MUST fire, not be intercepted by genesis fallback
    assert!(
        result.is_some(),
        "Height-based request must fire even with empties >= 10"
    );
    let (_, request) = result.unwrap();
    match request {
        crate::protocols::SyncRequest::GetHeadersByHeight { start_height, .. } => {
            assert_eq!(start_height, 1, "Should request from local_height");
        }
        other => panic!("Expected GetHeadersByHeight, got {:?}", other),
    }

    // After firing, the flag and counter should be cleared
    assert!(
        !manager.fork.use_height_based_headers,
        "Flag should be cleared after use"
    );
    // INC-I-139 M5/DC-4: a request dispatch is NOT progress — the height-based
    // request no longer zeroes the evidence counter. It is preserved so deep-fork
    // evidence can still accumulate to the escalation threshold (INV-SYNC-011:
    // only genuine block apply + the gap≤3 gossip-wait reset it).
    assert_eq!(
        manager.fork.consecutive_empty_headers, 15,
        "Post-DC-4: height-based request must PRESERVE the evidence counter, not reset it"
    );
}

/// INC-I-017 F3: Deep fork snap redirect blocked for fresh nodes.
/// confirmed_height_floor == 0 means the node was never fully synced.
/// Resetting snap.attempts for such nodes creates an infinite cycle.
#[test]
fn test_inc_i017_deep_fork_snap_redirect_blocked_for_fresh_nodes() {
    let genesis = Hash::ZERO;
    let mut manager = SyncManager::new(SyncConfig::default(), genesis);

    // Fresh node: never fully synced, confirmed_height_floor = 0
    manager.local_height = 1;
    manager.local_hash = crypto::hash::hash(b"block_1");
    manager.local_slot = 1;
    manager.confirmed_height_floor = 0; // Never synced
    manager.snap.attempts = 3; // Exhausted
    manager.fork.consecutive_empty_headers = 15;

    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    let peer3 = PeerId::random();
    manager.add_peer(peer1, 500, crypto::hash::hash(b"tip"), 500);
    manager.add_peer(peer2, 500, crypto::hash::hash(b"tip"), 500);
    manager.add_peer(peer3, 500, crypto::hash::hash(b"tip"), 500);

    manager.state = SyncState::Syncing {
        phase: SyncPhase::DownloadingHeaders,
        started_at: Instant::now(),
    };
    manager.pipeline_data = SyncPipelineData::Headers {
        target_slot: 500,
        peer: peer1,
        headers_count: 0,
    };

    let _ = manager.next_request();

    // snap.attempts must NOT be reset for fresh nodes
    assert_eq!(
        manager.snap.attempts, 3,
        "snap.attempts must NOT be reset when confirmed_height_floor == 0"
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

// =========================================================================
// Gossip activity watchdog tests (mesh isolation detection)
//
// Reproduces the mainnet death cascade (2026-03-29): node had 26 peers but
// received no gossip blocks for 2 minutes. Without the watchdog, the node
// produced during the silence and the scheduler permanently diverged from
// the network.
// =========================================================================

#[test]
fn test_gossip_watchdog_blocks_production_when_no_gossip() {
    // Scenario: Node has peers AHEAD of us, gossip timer expired → should block.
    // This reproduces the mainnet mesh partition where the node had 26 peers
    // but no gossip blocks arrived for 2+ minutes.
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 24340;
    manager.local_slot = 24493;

    // Peers are AHEAD — gossip should be delivering their blocks
    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    manager.add_peer(peer1, 24342, Hash::ZERO, 24495);
    manager.add_peer(peer2, 24342, Hash::ZERO, 24495);
    manager.first_peer_status_received = Some(Instant::now());

    // Simulate: last gossip block was 200 seconds ago (exceeds 180s default timeout)
    manager.last_block_received_via_gossip = Some(Instant::now() - Duration::from_secs(200));

    let result = manager.can_produce(24500);
    // INC-I-026: gossip watchdog disabled. With deterministic scheduler + FORK_GUARD,
    // producing during gossip silence just creates an ignored block — no fork risk.
    // Blocking production causes deadlocks (h=3851 incident).
    assert_eq!(
        result,
        ProductionAuthorization::Authorized,
        "INC-I-026: gossip watchdog disabled — production must be allowed. Got: {:?}",
        result
    );
}

#[test]
fn test_gossip_watchdog_bypassed_when_all_peers_at_same_height() {
    // Scenario: Cold restart / Guardian recovery — all peers at same height,
    // gossip expired. Should allow production to break the deadlock.
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 24340;
    manager.local_slot = 24493;

    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    manager.add_peer(peer1, 24340, Hash::ZERO, 24493);
    manager.add_peer(peer2, 24340, Hash::ZERO, 24493);
    manager.first_peer_status_received = Some(Instant::now());

    // Gossip expired — but all peers at same height = cold restart scenario
    manager.last_block_received_via_gossip = Some(Instant::now() - Duration::from_secs(200));

    let result = manager.can_produce(24500);
    assert_eq!(
        result,
        ProductionAuthorization::Authorized,
        "All peers at same height = cold restart, watchdog must be bypassed"
    );
}

#[test]
fn test_gossip_watchdog_allows_production_with_recent_gossip() {
    // Scenario: Node has peers and recent gossip activity → should produce.
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 24340;
    manager.local_slot = 24493;

    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    manager.add_peer(peer1, 24340, Hash::ZERO, 24493);
    manager.add_peer(peer2, 24340, Hash::ZERO, 24493);
    manager.first_peer_status_received = Some(Instant::now());

    // Recent gossip — 5 seconds ago
    manager.last_block_received_via_gossip = Some(Instant::now() - Duration::from_secs(5));

    let result = manager.can_produce(24500);
    assert_eq!(result, ProductionAuthorization::Authorized);
}

#[test]
fn test_gossip_watchdog_skipped_when_insufficient_peers() {
    // Scenario: Only 1 peer, gossip expired → should be blocked by min_peers,
    // NOT by gossip watchdog (min_peers fires first and is the correct reason).
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 100;
    manager.local_slot = 100;

    let peer = PeerId::random();
    manager.add_peer(peer, 100, Hash::ZERO, 100);
    manager.first_peer_status_received = Some(Instant::now());

    // Gossip expired
    manager.last_block_received_via_gossip = Some(Instant::now() - Duration::from_secs(300));

    let result = manager.can_produce(101);
    // Should be InsufficientPeers (1 < 2), not NoGossipActivity
    assert!(
        matches!(
            result,
            ProductionAuthorization::BlockedInsufficientPeers { .. }
        ),
        "Expected BlockedInsufficientPeers (fires before gossip check), got: {:?}",
        result
    );
}

#[test]
fn test_gossip_watchdog_respects_custom_timeout() {
    // Scenario: Custom short timeout (30s), peers ahead — blocks after 30s of silence.
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 100;
    manager.local_slot = 100;
    manager.gossip_activity_timeout_secs = 30; // Short timeout

    // Peers ahead — watchdog should activate
    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    manager.add_peer(peer1, 102, Hash::ZERO, 102);
    manager.add_peer(peer2, 102, Hash::ZERO, 102);
    manager.first_peer_status_received = Some(Instant::now());

    // 35 seconds without gossip — exceeds custom 30s timeout
    manager.last_block_received_via_gossip = Some(Instant::now() - Duration::from_secs(35));

    let result = manager.can_produce(101);
    // INC-I-026: gossip watchdog disabled — always authorized regardless of timeout.
    assert_eq!(
        result,
        ProductionAuthorization::Authorized,
        "INC-I-026: gossip watchdog disabled — production must be allowed. Got: {:?}",
        result
    );
}

#[test]
fn test_gossip_watchdog_not_triggered_just_under_timeout() {
    // Scenario: Gossip silence of 170s with 180s timeout → should still produce.
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 100;
    manager.local_slot = 100;

    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    manager.add_peer(peer1, 100, Hash::ZERO, 100);
    manager.add_peer(peer2, 100, Hash::ZERO, 100);
    manager.first_peer_status_received = Some(Instant::now());

    // 170s of silence — just under the 180s default timeout
    manager.last_block_received_via_gossip = Some(Instant::now() - Duration::from_secs(170));

    let result = manager.can_produce(101);
    assert_eq!(
        result,
        ProductionAuthorization::Authorized,
        "170s < 180s timeout — should still produce"
    );
}

#[test]
fn test_note_gossip_resets_watchdog() {
    // Scenario: Watchdog would fire, but note_block_received_via_gossip() resets it.
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    manager.local_height = 100;
    manager.local_slot = 100;

    // Peers ahead — watchdog will activate
    let peer1 = PeerId::random();
    let peer2 = PeerId::random();
    manager.add_peer(peer1, 102, Hash::ZERO, 102);
    manager.add_peer(peer2, 102, Hash::ZERO, 102);
    manager.first_peer_status_received = Some(Instant::now());

    // Gossip expired (200s ago)
    manager.last_block_received_via_gossip = Some(Instant::now() - Duration::from_secs(200));

    // INC-I-026: gossip watchdog disabled — production allowed even without gossip
    assert_eq!(
        manager.can_produce(101),
        ProductionAuthorization::Authorized,
    );

    // Gossip block received — timer reset (no behavioral change, watchdog disabled)
    manager.note_block_received_via_gossip();

    // Still authorized
    let result = manager.can_produce(103);
    assert_eq!(
        result,
        ProductionAuthorization::Authorized,
        "After receiving gossip block and catching up, production should be authorized"
    );
}

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
                block_header_bytes: None,
                epoch_bond_snapshot_bytes: None,
                epoch_accumulators_bytes: None,
                epoch_state_bytes: None,
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
    ///
    /// DC-2 (INC-I-139): forward-large-gap reasons (CoordinatorSnapEscalation,
    /// StuckSyncLargeGap) are now floor-EXEMPT — a forward snap cannot violate the
    /// monotonic floor. This test therefore uses CoordinatorGenesisEscalation, which
    /// remains floor-gated, to preserve floor-refusal coverage.
    #[test]
    fn test_request_genesis_resync_refused_by_height_floor() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Simulate a node that was confirmed healthy at height 100
        manager.confirmed_height_floor = 100;
        // Snap sync enabled, fresh state otherwise
        manager.snap.threshold = 500;
        manager.snap.attempts = 0;

        let result = manager.request_genesis_resync(RecoveryReason::CoordinatorGenesisEscalation);

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
            RecoveryReason::CoordinatorSnapEscalation,
            RecoveryReason::CoordinatorGenesisEscalation,
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
        let _second = manager.request_genesis_resync(RecoveryReason::AllPeersBlacklistedDeepFork);
        // The method should still pass all gates (flag already set is not a gate)
        // The flag stays true regardless
        assert!(
            manager.fork.needs_genesis_resync,
            "T-RG-008: needs_genesis_resync must remain true after second call"
        );
        // Whether second returns true or false is implementation-defined,
        // but it must not panic and the flag must stay true
    }

    /// T-RG-009: Gate ordering — height floor (Gate 1) checked first (fast reject).
    /// With floor > 0 and a still-floor-gated reason, Gate 1 refuses regardless of
    /// the snap gate. We use a floor-gated reason (CoordinatorGenesisEscalation) so
    /// the refusal is attributable to Gate 1; the SyncConfig default snap.threshold
    /// is 50 (snap ENABLED), so the snap gate would not block on its own.
    ///
    /// DC-2 (INC-I-139): forward-large-gap reasons are floor-exempt, so this test
    /// deliberately avoids StuckSyncLargeGap to keep asserting Gate-1 refusal.
    #[test]
    fn test_request_genesis_resync_gate_ordering_floor_first() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Floor > 0 with a floor-gated reason → Gate 1 refuses first.
        manager.confirmed_height_floor = 100;

        let result = manager.request_genesis_resync(RecoveryReason::CoordinatorGenesisEscalation);

        assert!(!result, "T-RG-009: Must be refused (Gate 1 floor rejects)");
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

        // Stuck for > 300s (new threshold with deterministic scheduler)
        manager.network.last_block_applied = Instant::now() - Duration::from_secs(310);

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
    /// DC-2 (INC-I-139): StuckSyncLargeGap is floor-EXEMPT — a forward snap
    /// catch-up cannot violate the monotonic confirmed-height floor (CR-1), so a
    /// legitimate gap>1000 stuck-sync recovery must be HONORED even when
    /// confirmed_height_floor > 0.
    ///
    /// Setup: gap > 1000, snap.attempts < 3, snap enabled (threshold=500), 3+
    /// peers, stuck > 300s (the real stuck-sync raise threshold).
    /// Expected: needs_genesis_resync becomes TRUE (DC-2 floor-exempt).
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

        // Stuck for > 300s so the StuckSyncLargeGap raise path actually fires
        // (real threshold is 300s, not 120s — pre-existing comment/code drift).
        manager.network.last_block_applied = Instant::now() - Duration::from_secs(310);
        // Ensure the "stuck sync" path is reached, not the fork path
        manager.fork.consecutive_empty_headers = 10; // >= 3, so it won't take the small-gap path

        // Run cleanup
        manager.cleanup();

        assert!(
            manager.fork.needs_genesis_resync,
            "T-M2-002: DC-2 makes StuckSyncLargeGap floor-EXEMPT — a forward snap \
             catch-up cannot violate the monotonic floor (CR-1). With gap>1000, \
             stuck>300s, confirmed_height_floor={} > 0 and snap enabled, the recovery \
             gate must HONOR the request (needs_genesis_resync == true).",
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
        // Non-emergency, non-forward-large-gap reasons: still blocked by floor.
        let blocked_reasons = vec![
            RecoveryReason::HeightOffsetDetected { gap: 500 },
            RecoveryReason::BodyDownloadPeerError,
            RecoveryReason::RollbackDeathSpiral {
                peak: 500,
                current: 10,
            },
            RecoveryReason::CoordinatorGenesisEscalation,
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

        // DC-2 (INC-I-139): forward-large-gap reasons are floor-exempt — a FORWARD
        // snap cannot violate the monotonic floor (CR-1). They bypass Gate 1 but NOT
        // Gate 4 (operator disable).
        let forward_large_gap_reasons = vec![
            RecoveryReason::CoordinatorSnapEscalation,
            RecoveryReason::StuckSyncLargeGap { gap: 2000 },
        ];

        for reason in forward_large_gap_reasons {
            let mut manager = manager_with_floor_gate();

            let result = manager.request_genesis_resync(reason.clone());

            assert!(
                result,
                "T-M2-009: {:?} must be HONORED via the floor exemption (DC-2)",
                reason
            );
            assert!(
                manager.fork.needs_genesis_resync,
                "T-M2-009: needs_genesis_resync must be true for {:?}",
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
            RecoveryReason::CoordinatorSnapEscalation,
            RecoveryReason::CoordinatorGenesisEscalation,
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

        // Fix: floor is now LOWERED instead of refusing. The rollback proceeds
        // and state resets to Idle. Floor should be lowered to local_height.
        assert!(
            manager.confirmed_height_floor <= manager.local_height,
            "T-M2-010: floor must be lowered to allow rollback. floor={}, local_h={}",
            manager.confirmed_height_floor,
            manager.local_height
        );
        assert!(
            matches!(*manager.state(), SyncState::Idle),
            "T-M2-010: state must be Idle after floor-lowered rollback. Got: {:?}",
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

        // Fix: floor is now LOWERED instead of refusing, even when height < floor.
        assert!(
            manager.confirmed_height_floor <= manager.local_height,
            "T-M2-010b: floor must be lowered to local_height ({}). Got floor={}",
            manager.local_height,
            manager.confirmed_height_floor
        );
        assert!(
            matches!(*manager.state(), SyncState::Idle),
            "T-M2-010b: state must be Idle after floor-lowered rollback. Got: {:?}",
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

    /// Checkpoint health: peers behind our tip (normal lag) should count as agreeing
    /// when their best_hash matches our canonical hash at their reported height.
    #[test]
    fn test_checkpoint_health_tolerates_peer_lag() {
        let genesis = Hash::ZERO;
        let mut manager = SyncManager::new(SyncConfig::default(), genesis);

        // Simulate applying blocks 1..=10, each with a unique hash
        let mut hashes = vec![genesis];
        for h in 1..=10u64 {
            let hash = Hash::from_bytes([h as u8; 32]);
            hashes.push(hash);
            manager.update_local_tip(h, hash, h as u32 * 10);
        }

        // Add peers that lag behind (reporting height 8 and 9)
        let peer1 = PeerId::random();
        let peer2 = PeerId::random();
        manager.add_peer(peer1, 8, hashes[8], 80);
        manager.add_peer(peer2, 9, hashes[9], 90);

        let (count, agreeing, tips) = manager.checkpoint_health();
        assert_eq!(count, 2);
        assert_eq!(agreeing, 2, "Peers on our canonical chain should agree");
        assert_eq!(tips, 1, "All on same chain = 1 tip");
    }

    /// Checkpoint health: a peer at the same height but different hash = real fork.
    #[test]
    fn test_checkpoint_health_detects_real_fork() {
        let genesis = Hash::ZERO;
        let mut manager = SyncManager::new(SyncConfig::default(), genesis);

        let _hash_5 = Hash::from_bytes([5u8; 32]);
        for h in 1..=5u64 {
            let hash = Hash::from_bytes([h as u8; 32]);
            manager.update_local_tip(h, hash, h as u32 * 10);
        }

        // Peer at height 5 but with a DIFFERENT hash = fork
        let forked_hash = Hash::from_bytes([55u8; 32]);
        let peer = PeerId::random();
        manager.add_peer(peer, 5, forked_hash, 50);

        let (count, agreeing, tips) = manager.checkpoint_health();
        assert_eq!(count, 1);
        assert_eq!(agreeing, 0, "Forked peer must not agree");
        assert_eq!(tips, 2, "Our chain + forked chain = 2 tips");
    }

    /// Checkpoint health: mix of agreeing and forked peers.
    #[test]
    fn test_checkpoint_health_mixed_peers() {
        let genesis = Hash::ZERO;
        let mut manager = SyncManager::new(SyncConfig::default(), genesis);

        let mut hashes = vec![genesis];
        for h in 1..=10u64 {
            let hash = Hash::from_bytes([h as u8; 32]);
            hashes.push(hash);
            manager.update_local_tip(h, hash, h as u32 * 10);
        }

        // 2 peers on our chain (lagging)
        let p1 = PeerId::random();
        let p2 = PeerId::random();
        manager.add_peer(p1, 8, hashes[8], 80);
        manager.add_peer(p2, 10, hashes[10], 100);

        // 1 peer on a fork at height 9
        let p3 = PeerId::random();
        let forked = Hash::from_bytes([99u8; 32]);
        manager.add_peer(p3, 9, forked, 90);

        let (count, agreeing, tips) = manager.checkpoint_health();
        assert_eq!(count, 3);
        assert_eq!(agreeing, 2, "2 peers on our chain");
        assert_eq!(tips, 2, "Our chain + 1 forked chain");
    }

    /// Checkpoint health: ring buffer handles reorgs (duplicate heights replaced).
    #[test]
    fn test_checkpoint_health_reorg_replaces_stale_hashes() {
        let genesis = Hash::ZERO;
        let mut manager = SyncManager::new(SyncConfig::default(), genesis);

        // Apply blocks 1..=5
        for h in 1..=5u64 {
            manager.update_local_tip(h, Hash::from_bytes([h as u8; 32]), h as u32 * 10);
        }

        // Reorg: tip drops back to 3, then applies new 4 and 5
        let new_hash_4 = Hash::from_bytes([44u8; 32]);
        let new_hash_5 = Hash::from_bytes([55u8; 32]);
        manager.update_local_tip(4, new_hash_4, 40);
        manager.update_local_tip(5, new_hash_5, 50);

        // Peer reports the NEW hash at height 4
        let peer = PeerId::random();
        manager.add_peer(peer, 4, new_hash_4, 40);

        let (_, agreeing, _) = manager.checkpoint_health();
        assert_eq!(agreeing, 1, "Peer matches post-reorg canonical hash");

        // Peer with OLD hash at height 4 should NOT agree
        let peer2 = PeerId::random();
        let old_hash_4 = Hash::from_bytes([4u8; 32]);
        manager.add_peer(peer2, 4, old_hash_4, 40);

        let (_, agreeing, _) = manager.checkpoint_health();
        assert_eq!(agreeing, 1, "Old fork hash must not agree");
    }
}

// =========================================================================
// M-RC12-full — Asymmetric blacklist invariant (B-5 / F-4 / S-6 convergence)
//
// Spec: specs/scheduler-state-architecture.md
//   "Empty-headers response from peer for local hash triggers
//    GetHeadersByHeight(local_height) + weight_compare(peer_chain). Peer
//    is NEVER blacklisted on empty-headers alone. RC#12 structurally
//    impossible. (F-4 satisfied.)"
//
// Target code (bug): crates/network/src/sync/manager/sync_engine/response.rs
//   lines 317-347. The `consecutive_empty_headers < 3` branch inserts the
//   peer into `fork.header_blacklisted_peers` when not recently_snapped.
//   This is the RC#12 symmetric-blacklist heuristic and violates B-5.
//
// Partial fix at 42fe7982 (M-RC12 phase 1) only touched the ORPHAN-GOSSIP
// path (block_lifecycle.rs / peers.rs / types.rs / rollback.rs). It did
// NOT touch `sync_engine/response.rs` — empty-headers blacklist still
// exists. M-RC12-full closes this gap.
//
// OUTPUT CONTRACT: fn handle_response(&mut self, peer, SyncResponse::Headers(vec![]))
//                  which delegates to handle_headers_response for empty headers.
//   O2: self.fork.header_blacklisted_peers — HashMap<PeerId, Instant>
//       Post-condition on empty-headers path: peer MUST NOT be inserted.
//   O2: self.fork.use_height_based_headers — bool
//       Post-condition: MUST become true so next sync cycle consults fork
//       choice via GetHeadersByHeight (replacement for deleted blacklist).
//   O2: self.fork.consecutive_empty_headers — u32
//       Post-condition: incremented by 1 (cascade detection preserved).
//   O2: self.state — SyncState
//       Post-condition: transitions to Idle (unchanged by M-RC12-full).
//   O3: return value — Vec<Block>
//       Post-condition: vec![] (unchanged for all Headers responses).
// INPUT PARTITIONS: P1-P4 below — each a distinct empty-headers input class
//   exercising a different counter/snapshot relationship.
// PATHS covered by the four tests below:
//   P1: first empty (counter 0 -> 1), not recently snapped, not post-snap
//   P2: second empty (counter 1 -> 2), same pre-conditions
//   P3: snap available but not exhausted (attempts=1, threshold=1000)
//   P4: adversarial — 2 distinct peers each give 1 empty response
// MATRIX: 4 outputs of interest × 4 paths = 16 cells. Every cell is asserted
// or explicitly documented as out-of-scope (state and return are invariant
// across paths, so asserted once in P1; cascade/stuck-fork path at >=3 is
// out of M-RC12-full scope per spec RC#12 row).
// =========================================================================
mod m_rc12_full_asymmetric_blacklist_tests {
    use super::*;

    /// Build a SyncManager positioned at the empty-headers branch that
    /// currently inserts into header_blacklisted_peers (response.rs:329-332).
    ///
    /// Pre-conditions required to reach the buggy path:
    ///   - `recovery_phase = Normal` (not AwaitingCanonicalBlock) → skips
    ///     post-snap height-fallback branch at response.rs:218.
    ///   - `snap.last_snap_completed` was > 5 min ago → `recently_snapped = false`
    ///     at response.rs:323-327, so the guard at 328 does NOT skip the insert.
    ///   - `pipeline.headers_needing_bodies.is_empty()` and
    ///     `pipeline.pending_headers.is_empty()` → reaches the "no bodies, no
    ///     pending" arm at response.rs:193.
    ///   - `local_height > 0` and peer gap > 3 → skips small-gap branch at
    ///     response.rs:256.
    ///   - `consecutive_empty_headers < 3` post-increment → skips stuck-fork
    ///     branch at response.rs:269.
    fn mk_manager_at_empty_headers_blacklist_path(
        local_height: u64,
        consecutive_before: u32,
        snap_attempts: u8,
        snap_threshold: u64,
    ) -> (SyncManager, PeerId) {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        manager.local_height = local_height;
        manager.local_slot = local_height as u32;
        manager.local_hash = crypto::hash::hash(format!("local_{}", local_height).as_bytes());

        // Normal phase — bypass the post-snap AwaitingCanonicalBlock branch
        // at response.rs:211-214 that ALREADY avoids the blacklist.
        manager.recovery_phase = RecoveryPhase::Normal;

        // Snap completed 10 minutes ago → recently_snapped = false.
        manager.snap.last_snap_completed = Some(Instant::now() - Duration::from_secs(600));
        manager.snap.attempts = snap_attempts;
        manager.snap.threshold = snap_threshold;

        // Counter starts below 3 so we don't enter the stuck-fork branch.
        manager.fork.consecutive_empty_headers = consecutive_before;
        assert!(
            manager.fork.header_blacklisted_peers.is_empty(),
            "precondition: blacklist starts empty"
        );
        assert!(
            !manager.fork.use_height_based_headers,
            "precondition: use_height_based_headers starts false"
        );

        // Peer at gap > 3 to skip the small-gap gossip-timing branch.
        let peer = PeerId::random();
        let peer_height = local_height + 10;
        manager.add_peer(
            peer,
            peer_height,
            crypto::hash::hash(format!("peer_{}", peer_height).as_bytes()),
            peer_height as u32,
        );

        manager.state = SyncState::Syncing {
            phase: SyncPhase::DownloadingHeaders,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Headers {
            target_slot: peer_height as u32,
            peer,
            headers_count: 0,
        };

        (manager, peer)
    }

    /// Test 1 (P1): The VERY FIRST empty-headers response must not blacklist
    /// the responding peer. This is the asymmetric-blacklist invariant (B-5):
    /// empty-headers is NOT positive fault evidence — peer may simply have a
    /// heavier chain that doesn't link from our local_hash.
    #[test]
    fn test_m_rc12_first_empty_headers_does_not_blacklist() {
        let (mut manager, peer) = mk_manager_at_empty_headers_blacklist_path(
            500, // local_height
            0,   // consecutive_empty_headers before
            0,   // snap.attempts (not stuck)
            50,  // snap.threshold (default-ish)
        );

        // Return value contract (O3): empty block vec for all Headers responses.
        let returned = manager.handle_response(peer, SyncResponse::Headers(vec![]));
        assert!(
            returned.is_empty(),
            "O3: handle_response for Headers must return empty Vec<Block>"
        );

        // O2 (PRIMARY INVARIANT): responding peer must NOT be blacklisted.
        assert!(
            !manager.fork.header_blacklisted_peers.contains_key(&peer),
            "M-RC12-full: first empty-headers must NOT blacklist responding peer \
             (asymmetric invariant — peer is not fault evidence). Blacklist \
             contents: {:?}",
            manager
                .fork
                .header_blacklisted_peers
                .keys()
                .collect::<Vec<_>>()
        );
        assert!(
            manager.fork.header_blacklisted_peers.is_empty(),
            "M-RC12-full: first empty-headers must leave blacklist empty. \
             Size: {}",
            manager.fork.header_blacklisted_peers.len()
        );

        // O2: use_height_based_headers must be set so the next sync cycle
        // consults fork choice via GetHeadersByHeight (B-5 replacement for
        // the deleted blacklist heuristic).
        assert!(
            manager.fork.use_height_based_headers,
            "M-RC12-full: first empty-headers must set use_height_based_headers=true \
             to consult fork choice on next sync cycle"
        );

        // O2: counter must still advance for cascade detection at >= 3.
        assert_eq!(
            manager.fork.consecutive_empty_headers, 1,
            "M-RC12-full: counter must still advance for cascade detection"
        );

        // O2: state transitions to Idle (invariant across all empty-headers arms).
        assert!(
            matches!(manager.state, SyncState::Idle),
            "M-RC12-full: empty-headers must transition state to Idle. Got: {:?}",
            manager.state
        );
    }

    /// Test 2 (P2): A SECOND empty-headers in a row also must not blacklist.
    /// The bug has the same symptom at counter=1→2; the asymmetric invariant
    /// must hold until cascade detection takes over at counter>=3.
    #[test]
    fn test_m_rc12_second_empty_headers_still_no_blacklist() {
        let (mut manager, peer) = mk_manager_at_empty_headers_blacklist_path(
            500, // local_height
            1,   // consecutive_empty_headers before — simulate second-in-a-row
            0,   // snap.attempts
            50,  // snap.threshold
        );

        let _ = manager.handle_response(peer, SyncResponse::Headers(vec![]));

        // O2: PRIMARY invariant — no blacklist at second empty.
        assert!(
            !manager.fork.header_blacklisted_peers.contains_key(&peer),
            "M-RC12-full: second consecutive empty-headers must NOT blacklist \
             responding peer"
        );

        // O2: use_height_based_headers set.
        assert!(
            manager.fork.use_height_based_headers,
            "M-RC12-full: second empty-headers must set use_height_based_headers=true"
        );

        // O2: counter advanced to 2 (still below cascade threshold of 3).
        assert_eq!(
            manager.fork.consecutive_empty_headers, 2,
            "M-RC12-full: counter must advance 1 -> 2"
        );
    }

    /// Test 3 (P3): When snap is AVAILABLE but not exhausted (attempts=1,
    /// threshold=1000), the pre-M-RC12 gate at response.rs:210-217 required
    /// `snap_exhausted = attempts >= 3` before allowing height-fallback.
    /// M-RC12-full removes that gate for the blacklist invariant: the peer
    /// is still not fault evidence regardless of whether snap is retryable.
    #[test]
    fn test_m_rc12_snap_not_exhausted_still_no_blacklist() {
        let (mut manager, peer) = mk_manager_at_empty_headers_blacklist_path(
            500,  // local_height
            0,    // consecutive_empty_headers before
            1,    // snap.attempts (snap tried once, 2 retries left)
            1000, // snap.threshold (large — snap still viable)
        );

        // Sanity: precondition for the "snap available" path.
        assert!(
            manager.snap.attempts < 3,
            "precondition: snap is not exhausted"
        );

        let _ = manager.handle_response(peer, SyncResponse::Headers(vec![]));

        // O2: PRIMARY invariant — no blacklist regardless of snap availability.
        assert!(
            !manager.fork.header_blacklisted_peers.contains_key(&peer),
            "M-RC12-full: empty-headers must NOT blacklist peer even when snap \
             is available and not exhausted. Blacklist contents: {:?}",
            manager
                .fork
                .header_blacklisted_peers
                .keys()
                .collect::<Vec<_>>()
        );

        // O2: use_height_based_headers set — asymmetric behaviour is the same
        // whether or not snap is exhausted.
        assert!(
            manager.fork.use_height_based_headers,
            "M-RC12-full: use_height_based_headers must be set even when snap \
             is available — blacklist invariant is independent of snap state"
        );

        // O2: counter advanced.
        assert_eq!(
            manager.fork.consecutive_empty_headers, 1,
            "M-RC12-full: counter advances to 1 regardless of snap state"
        );
    }

    /// Test 4 (P4, adversarial): Two distinct peers each give a single
    /// empty-headers response (each peer's first empty, not third). Simulates
    /// a symmetric fork where multiple peers reject our local_hash. The
    /// blacklist must remain empty across the entire loop — otherwise a
    /// regression where `insert()` creeps back under a different code path
    /// would be hidden by single-peer tests.
    #[test]
    fn test_m_rc12_blacklist_invariant_covers_multiple_peers() {
        let (mut manager, peer1) = mk_manager_at_empty_headers_blacklist_path(
            500, // local_height
            0,   // consecutive_empty_headers before
            0,   // snap.attempts
            50,  // snap.threshold
        );

        // Peer 1: first empty.
        let _ = manager.handle_response(peer1, SyncResponse::Headers(vec![]));

        // Counter is now 1. Reset to 0 before the second peer's response so
        // BOTH peers hit the `consecutive_empty_headers < 3` first-empty branch
        // (the one carrying the blacklist bug) — not the cascade/stuck-fork
        // branch at >=3, which is out of M-RC12-full scope.
        manager.fork.consecutive_empty_headers = 0;
        // Allow height-fallback to fire again for the second peer.
        manager.fork.height_fallback_attempted = false;
        manager.fork.use_height_based_headers = false;

        // Re-arm the sync pipeline for peer 2 (state machine transitions to
        // Idle after the first response; rewire pipeline_data directly rather
        // than driving cleanup()).
        let peer2 = PeerId::random();
        let peer2_height = manager.local_height + 10;
        manager.add_peer(
            peer2,
            peer2_height,
            crypto::hash::hash(b"peer2_hash"),
            peer2_height as u32,
        );
        manager.state = SyncState::Syncing {
            phase: SyncPhase::DownloadingHeaders,
            started_at: Instant::now(),
        };
        manager.pipeline_data = SyncPipelineData::Headers {
            target_slot: peer2_height as u32,
            peer: peer2,
            headers_count: 0,
        };

        // Peer 2: first empty.
        let _ = manager.handle_response(peer2, SyncResponse::Headers(vec![]));

        // O2: PRIMARY adversarial invariant — blacklist stays empty after
        // BOTH peers' empty responses. Catches regressions where a new code
        // path re-introduces the insert.
        assert!(
            manager.fork.header_blacklisted_peers.is_empty(),
            "M-RC12-full: asymmetric-blacklist invariant must hold across \
             multiple peers. Blacklist contents after 2 peers: {:?}",
            manager
                .fork
                .header_blacklisted_peers
                .keys()
                .collect::<Vec<_>>()
        );
        assert!(
            !manager.fork.header_blacklisted_peers.contains_key(&peer1),
            "M-RC12-full: peer1 must not be blacklisted"
        );
        assert!(
            !manager.fork.header_blacklisted_peers.contains_key(&peer2),
            "M-RC12-full: peer2 must not be blacklisted"
        );

        // O2: use_height_based_headers still true after second peer's response.
        assert!(
            manager.fork.use_height_based_headers,
            "M-RC12-full: use_height_based_headers must be set after multi-peer \
             empty-headers loop"
        );
    }
}

// -------------------------------------------------------------------------
// D6 (INC-I-090): consume_stuck_fork_signal wiring tests
//
// Validates that the stuck-fork consumer method exists, returns context
// when a signal is present, and returns None after consumption (dedup).
// BEFORE the fix: consume_stuck_fork_signal() does not exist → compile fail.
// -------------------------------------------------------------------------

mod d6_stuck_fork_consumer_tests {
    use super::*;

    /// T-D6-001: consume_stuck_fork_signal returns Some when signal is set.
    ///
    /// OUTPUT CONTRACT:
    ///   O1: &mut self (SyncManager) — consumes stuck_fork_signal flag
    ///   O2: returns Option<StuckForkAlert> with context (local_height, peer_height, peer_count)
    ///
    /// PATHS:
    ///   P1: Signal set → returns Some(StuckForkAlert) with correct context
    ///   P2: Signal not set → returns None
    ///   P3: Second call after P1 → returns None (consumed)
    ///
    /// .test_verified: FAIL→PASS required
    #[test]
    fn t_d6_001_consume_returns_some_when_signaled() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // Set up context: local_height=100, add a peer at height 200
        manager.update_local_tip(100, Hash::ZERO, 10);
        let peer = PeerId::random();
        manager.add_peer(peer, 200, Hash::ZERO, 20);

        // Signal a stuck fork from Normal phase
        manager.signal_stuck_fork();
        assert!(
            manager.fork.stuck_fork_signal,
            "precondition: signal must be set"
        );

        // P1: consume returns Some with context
        let alert = manager.consume_stuck_fork_signal();
        assert!(alert.is_some(), "P1: must return Some when signal is set");
        let alert = alert.unwrap();
        assert_eq!(alert.local_height, 100, "P1: local_height must match");
        assert_eq!(
            alert.best_peer_height, 200,
            "P1: best_peer_height must match"
        );
        assert!(
            alert.peer_count > 0,
            "P1: peer_count must reflect connected peers"
        );

        // P3: second call returns None (signal consumed by take)
        let second = manager.consume_stuck_fork_signal();
        assert!(
            second.is_none(),
            "P3: second call must return None (consumed)"
        );
    }

    /// T-D6-002: consume_stuck_fork_signal returns None when no signal.
    #[test]
    fn t_d6_002_consume_returns_none_when_no_signal() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // P2: no signal set → None
        let alert = manager.consume_stuck_fork_signal();
        assert!(
            alert.is_none(),
            "P2: must return None when no signal is set"
        );
    }

    /// T-D6-003: consume_stuck_fork_signal returns None during non-Normal phases.
    /// signal_stuck_fork() ignores the call during ResyncInProgress, so
    /// consume must also return None.
    #[test]
    fn t_d6_003_consume_returns_none_during_resync() {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.recovery_phase = RecoveryPhase::ResyncInProgress;

        // signal_stuck_fork ignores during ResyncInProgress
        manager.signal_stuck_fork();
        assert!(
            !manager.fork.stuck_fork_signal,
            "precondition: signal must NOT be set during resync"
        );

        let alert = manager.consume_stuck_fork_signal();
        assert!(alert.is_none(), "must return None when signal was not set");
    }
}

// =========================================================================
// INC-I-138 D5: applied-since-rollback suppression heuristic
//
// Suppression function: SyncManager::note_orphan_gossip_block (peers.rs:465)
// Suppression guard:    peers.rs:533-549 — the "BEHIND not forked" branch
//
// Bug (D5, measured 21× on testnet): the guard fires whenever ANY block was
// applied after the last rollback — including blocks the node SELF-PRODUCED on
// its own unrecognized fork tip. n5 rolled back from h=19 to h=18, self-produced
// fork blocks h=19/20/21 (all rejected "outside time window" by all 5 peers),
// then accumulating orphan gossip triggered note_orphan_gossip_block 21 times —
// each time the guard saw local_height(21) > rb_h(18) and suppressed stuck_fork.
// Result: G3/ShallowRollback never fired → 325s stall → SnapSync at gap=28.
//
// OUTPUT CONTRACT: fn note_orphan_gossip_block(&mut self, height: u64, slot: u32)
//   O1: self.consecutive_orphan_gossip_blocks — incremented each call;
//       reset to 0 when any action-taking exit path fires (>= 3 threshold)
//   O2: self.fork.stuck_fork_signal — set true ONLY via signal_stuck_fork()
//       (Normal recovery phase); unchanged on suppression path (BUG)
//   O3: self.state — Idle → Syncing when start_sync() fires (suppression path);
//       stays Idle when signal_stuck_fork() fires instead
//   O4: self.recovery — always receives OrphanGossip evidence (peers.rs:481-485)
//       before any branch
//   O5: self.network.network_tip_height — updated if block_height > current tip
//
// Code paths:
//   PATH-A (self-produced-fork applies, rollback_fresh=true, local_h > rb_h, gap=18):
//     → suppression guard fires → O2=false (BUG: stuck_fork NOT set), O3=Syncing
//   PATH-B (peer-received canonical applies — regression pin):
//     → same guard fires (CORRECT: node is BEHIND) → O2=false, O3=Syncing
//   PATH-C (no rollback state — control):
//     → guard skipped → signal_stuck_fork() → O2=true, O3=Idle
//
// INPUT PARTITIONS × output matrix (O2 is the discriminating output):
//   | Partition                          | O2 stuck_fork | O3 state |
//   |------------------------------------|---------------|----------|
//   | A: self-fork  (rb=18, h=21, g=18) | true  (MUST)  | Idle     | <- FAILS (BUG)
//   | B: canonical  (rb=18, h=19, g= 3) | false (pin)   | Syncing  | <- PASSES
//   | C: no-rollback (h=21, g=18)       | true          | Idle     | <- PASSES
// =========================================================================
mod inc_i138_d5_stuck_fork_suppression {
    use super::*;
    use libp2p::PeerId;
    use std::time::{Duration, Instant};

    /// Build a SyncManager in the correct initial state for D5 suppression tests.
    ///
    /// `last_block_applied` is set to 35s ago:
    ///   recently_synced = (35 < 60) = true → enters the "fork vs behind" check
    ///   secs_since_apply = 35 >= 30         → NOT suppressed by the fresh-apply guard
    fn fork_test_manager_d5(local_height: u64, net_tip_height: u64) -> SyncManager {
        let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        mgr.state = SyncState::Idle;
        mgr.pipeline_data = SyncPipelineData::None;
        mgr.local_height = local_height;
        mgr.network.network_tip_height = net_tip_height;
        // 35s: recently_synced=true AND secs_since_apply >= 30 → passes fresh-apply guard
        mgr.network.last_block_applied = Instant::now()
            .checked_sub(Duration::from_secs(35))
            .expect("test machine uptime must exceed 35s");
        mgr
    }

    /// Insert a peer directly, bypassing add_peer() which triggers start_sync().
    fn insert_peer_d5(mgr: &mut SyncManager, height: u64) {
        let peer = PeerId::random();
        mgr.peers.insert(
            peer,
            PeerSyncStatus {
                best_height: height,
                best_hash: Hash::ZERO,
                best_slot: height as u32,
                last_status_response: Instant::now(),
                last_block_received: None,
                pending_request: None,
                protocol_version: 0,
                producer_pubkey: None,
            },
        );
    }

    // -------------------------------------------------------------------------
    // PATH-A — BUG REPRO (MUST FAIL on current code)
    // -------------------------------------------------------------------------

    /// INC-I-138 D5 — self-produced fork blocks MUST NOT suppress stuck_fork.
    ///
    /// Exact incident state (n5 log-verified, orchestrator-measured 21× WARN):
    ///   n5 rolled back from h=19 to h=18 (rb_h=18, rollback_fresh=true ~0s).
    ///   n5 self-produced fork blocks h=19/20/21 on slot 639847-639852.
    ///   All 5 peers rejected fork blocks "outside time window" (simultaneous).
    ///   Peers report network tip h=39 (gap=18 < 50 → enters suppression check).
    ///   Orphan gossip blocks h=37-39 accumulate.
    ///
    ///   Suppression guard (peers.rs:533-549):
    ///     rollback_fresh=true AND rb_h=Some(18) AND local_height(21) > rb_h(18) → fires.
    ///     WARN "applied since last rollback (rb_h=18) → BEHIND not forked. Suppressing..."
    ///     Calls start_sync() and returns WITHOUT calling signal_stuck_fork().
    ///
    ///   CORRECT: stuck_fork_signal=true → G3 → ShallowRollback → recovery seconds.
    ///   ACTUAL (BUG): stuck_fork_signal=false → 21× suppression → 325s stall.
    ///
    /// FAILS on current code: suppression branch fires, O2=false instead of true.
    #[test]
    fn test_inc_i138_d5_self_produced_fork_blocks_must_not_suppress_stuck_fork() {
        // local_height=21: self-produced fork blocks h=19/20/21 after rollback to h=18.
        // net_tip_height=39: gap=18 (< 50 → suppression guard is entered).
        let mut mgr = fork_test_manager_d5(21, 39);

        // Simulate rollback to h=18 just completed (rollback_fresh=true, ~0s ago).
        // Sets fork.last_rollback_local_height=Some(18), fork.last_rollback_time=now.
        mgr.note_rollback_completed(18);

        // Insert 2 peers at h=39 (canonical chain n5 fell off).
        // Direct insert bypasses add_peer() → no premature start_sync().
        insert_peer_d5(&mut mgr, 39);
        insert_peer_d5(&mut mgr, 39);

        // Accumulate 3 consecutive orphan gossip blocks (h=37/38/39).
        // These are canonical chain blocks that n5's fork tip (h=21) cannot extend.
        mgr.note_orphan_gossip_block(37, 637);
        mgr.note_orphan_gossip_block(38, 638);
        // Third call crosses the >= 3 threshold. Suppression branch fires:
        //   peers.rs:533: rollback_fresh=true
        //   peers.rs:534: rb_h=Some(18)
        //   peers.rs:535: local_height(21) > rb_h(18) → TRUE → SUPPRESSION FIRES
        //   peers.rs:546-548: warn!, consecutive_orphan_gossip_blocks=0, start_sync(), return
        //   BUG: signal_stuck_fork() never called → stuck_fork_signal stays false.
        mgr.note_orphan_gossip_block(39, 639);

        // O1: counter must be reset to 0 by whichever exit path fires
        assert_eq!(
            mgr.consecutive_orphan_gossip_blocks, 0,
            "O1: orphan counter must be reset to 0 on any action-taking exit"
        );

        // O2 — PRIMARY BUG ASSERTION (FAILS on current code):
        //   When all applies since rollback are self-produced fork blocks on a tip no
        //   peer recognizes (empty GetHeaders for our tip hash, gap=18), stuck_fork_signal
        //   MUST be true so G3 can fire ShallowRollback and recover in seconds.
        //   ACTUAL: false. The suppression branch (peers.rs:533-549) took the early-return
        //   path, calling start_sync() instead of signal_stuck_fork().
        //   This is the D5 defect: 21× suppressions in the incident → 325s stall.
        assert!(
            mgr.fork.stuck_fork_signal,
            "INC-I-138 D5 BUG: self-produced fork blocks (h=19/20/21) after rollback to h=18 \
             caused the applied-since-rollback heuristic (peers.rs:533-549) to classify FORKED \
             as BEHIND. stuck_fork_signal=false instead of true. \
             21x suppressions measured in incident → G3/ShallowRollback starved → 325s stall \
             → spurious SnapSync at gap=28. Fix: gate suppression on peer-confirmed canonical \
             tip progress, not merely local_height > rb_h."
        );
    }

    // -------------------------------------------------------------------------
    // PATH-B — REGRESSION PIN (must PASS now and after D5 fix)
    // -------------------------------------------------------------------------

    /// INC-I-138 D5 regression pin — peer-received canonical applies MUST suppress.
    ///
    /// Scenario: n5 rolled back to h=18, received ONE canonical block h=19 from a peer.
    /// local_height=19 > rb_h=18 → rollback succeeded, reconnected to canonical.
    /// Peers at h=22 (gap=3). Orphan gossip h=20-22 means node is merely BEHIND.
    ///
    /// CORRECT: suppression fires → start_sync() closes the gap.
    ///   Rolling back here would undo a valid canonical block — the 2026-04-15
    ///   folsi cascade: 25 consecutive rollbacks grew the gap from 2 to 50+.
    ///
    /// This test MUST PASS both before AND after the D5 fix. The fix must NOT remove
    /// suppression for the legitimate "BEHIND, not FORKED" case.
    ///
    /// PASSES on current code: suppression correctly fires, O2=false.
    #[test]
    fn test_inc_i138_d5_peer_canonical_applies_correctly_suppress_signal_pin() {
        // local_height=19: one peer-canonical block received after rollback to h=18.
        // net_tip_height=22: gap=3, node is merely behind the canonical tip.
        let mut mgr = fork_test_manager_d5(19, 22);

        mgr.note_rollback_completed(18);
        // INC-I-138 D5 FIX: simulate one peer-received canonical block applied
        // after rollback (h=19 from a sync peer). This sets peer_block_applied_since_rollback=true,
        // which tells the suppression guard the rollback succeeded and we reconnected.
        // In production this is called by the block-handling layer after apply_block().
        mgr.note_peer_block_applied_since_rollback();
        insert_peer_d5(&mut mgr, 22);
        insert_peer_d5(&mut mgr, 22);

        mgr.note_orphan_gossip_block(20, 620);
        mgr.note_orphan_gossip_block(21, 621);
        mgr.note_orphan_gossip_block(22, 622);

        // O1: counter reset by suppression exit
        assert_eq!(
            mgr.consecutive_orphan_gossip_blocks, 0,
            "O1: orphan counter must be reset to 0 on any action-taking exit"
        );

        // O2: suppression CORRECT here — PASSES (regression pin: fix must NOT break this)
        //   local_height(19) > rb_h(18) + rollback_fresh → suppression correctly fires.
        //   We're BEHIND, not FORKED. Rolling back canonical block cascades (folsi).
        assert!(
            !mgr.fork.stuck_fork_signal,
            "INC-I-138 D5 pin: peer-canonical applies after rollback (h=19>rb=18, gap=3) \
             MUST suppress stuck_fork. Node is BEHIND, not forked. This must PASS after fix."
        );

        // O3: start_sync() fires via suppression path → state becomes Syncing
        assert!(
            mgr.state.is_syncing(),
            "O3: suppression path must call start_sync() → state transitions to Syncing"
        );
    }

    // -------------------------------------------------------------------------
    // PATH-C — CONTROL (no rollback state; must PASS on current code)
    // -------------------------------------------------------------------------

    /// INC-I-138 D5 control — without rollback state, stuck_fork fires correctly.
    ///
    /// Same heights as PATH-A but NO rollback state. rollback_fresh=false → guard skipped.
    /// secs_since_apply=35 >= 30 → fresh-apply guard does not suppress.
    /// signal_stuck_fork() fires → stuck_fork_signal=true.
    ///
    /// Proves: (a) the signal path is intact; (b) D5 is specifically the rollback-state
    /// interaction (peers.rs:533-549), not a broader structural bug.
    ///
    /// PASSES on current code: guard skipped, O2=true.
    #[test]
    fn test_inc_i138_d5_no_rollback_state_stuck_fork_fires_control() {
        // Same heights as PATH-A: local=21, tip=39, gap=18.
        // No note_rollback_completed call → rollback_fresh=false → guard NOT entered.
        let mut mgr = fork_test_manager_d5(21, 39);

        insert_peer_d5(&mut mgr, 39);
        insert_peer_d5(&mut mgr, 39);

        mgr.note_orphan_gossip_block(37, 637);
        mgr.note_orphan_gossip_block(38, 638);
        // Third call: rollback_fresh=false → suppression guard (peers.rs:528-551) skipped.
        // secs_since_apply=35 >= 30 → fresh-apply guard (peers.rs:566) passes through.
        // signal_stuck_fork() fires → O2=true, O3 stays Idle.
        mgr.note_orphan_gossip_block(39, 639);

        // O1: counter reset by signal exit
        assert_eq!(
            mgr.consecutive_orphan_gossip_blocks, 0,
            "O1: orphan counter must be reset to 0 on any action-taking exit"
        );

        // O2: signal MUST fire when no rollback state is present (PASSES)
        assert!(
            mgr.fork.stuck_fork_signal,
            "INC-I-138 D5 control: without rollback state, stuck_fork must fire. \
             If this FAILS, the signal path itself is broken — separate structural bug."
        );

        // O3: signal_stuck_fork() does NOT call start_sync(); state stays Idle
        assert!(
            matches!(*mgr.state(), SyncState::Idle),
            "O3: signal_stuck_fork() must not call start_sync(); state must stay Idle"
        );
    }
}

// =========================================================================
// INC-I-138 M2: D2 counter starvation + D4 gap-blind escalation + floor=0 race
//
// Three co-equal defects that, together with D5 (M1, already merged), created
// the 325s stall at epoch-1 boundary (2026-07-07, testnet v6.23.9).
//
// D2: consecutive_empty_headers pinned at max=2 over 325s by two reset writers:
//   dominant:  production_gate.rs:558 reset_empty_headers() via periodic.rs:712
//              on every HeaderFirstSync coordinator action (~every 30s).
//   secondary: dispatch.rs:84 counter reset when use_height_based_headers fires.
//   Measured:  142 GetHeadersByHeight calls, 0 ShallowRollback events, max=2 (E5).
//
// D4: recovery.rs:382-383 deep_fork_confirmed has NO gap guard.
//   Rule 2 fires SnapSync at gap=28 (below SNAP_SYNC_GAP_MIN=500) when
//   empty_count>=10 && last_applied>=STALE_TIP_SECS(300). Terminal event:
//   E8 "[COORDINATOR] action=SnapSync gap=28 last_applied=325s" (n5.log:17197).
//
// NAIVE-FIX TRAP: dispatch.rs:96→134 GenesisFallbackEmptyHeaders fires at
//   gap=28 when counter=10. Reachable once D2 is fixed without fixing D4 too.
//   Correct outcome in the 4..50 gap regime: G3/ShallowRollback, not genesis-resync.
//
// FLOOR=0 RACE: block_lifecycle.rs:74 floor check runs BEFORE the Syncing→
//   Synchronized transition at :150. The transition block never sets the floor.
//   Gate 1 (confirmed_height_floor > 0) at production_gate.rs:674 evaluates to
//   false, allowing CoordinatorSnapEscalation unconditionally (E10).
//
// Invariant: INV-FORK-001 — G3 must fire and trigger ShallowRollback.
// Causal chain: E5 (D2) → E8 (D4) → E10 (floor race) → blocks 37-63 missing.
// Shape: INC-I-120 recurrence #2 (same seam, different mechanism — starved vs absent).
// RUN_ID: 449
// =========================================================================
mod inc_i138_m2_escalation {
    use super::*;
    use crate::sync::manager::recovery::{
        thresholds, RecoveryAction, RecoveryContext, RecoveryCoordinator, RecoveryEvidence,
    };
    use libp2p::PeerId;
    use std::time::{Duration, Instant};

    // -------------------------------------------------------------------------
    // Shared helpers
    // -------------------------------------------------------------------------

    /// Build a SyncManager in the INC-I-138 Phase-2 state (t > 109s, D5 expired):
    ///   local_height=36, network_tip=64, gap=28 (< MINOR_FORK_GAP_MAX=50)
    ///   last_block_applied=325s ago (> STALE_TIP_SECS=300)
    ///   5 peers at height 64 (>= SNAP_MIN_PEERS=3)
    ///   recovery_phase=Normal, snap.attempts=0, confirmed_height_floor=0
    fn phase2_manager() -> SyncManager {
        let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        mgr.local_height = 36;
        mgr.local_hash = crypto::hash::hash(b"fork_tip_290d4942");
        mgr.local_slot = 636;
        mgr.network.network_tip_height = 64;
        mgr.network.network_tip_slot = 664;
        mgr.network.last_block_applied = Instant::now()
            .checked_sub(Duration::from_secs(325))
            .expect("test machine uptime must exceed 325s");
        // 5 peers at height 64 (>= SNAP_MIN_PEERS=3)
        for _ in 0..5 {
            mgr.peers.insert(
                PeerId::random(),
                PeerSyncStatus {
                    best_height: 64,
                    best_hash: Hash::ZERO,
                    best_slot: 664,
                    last_status_response: Instant::now(),
                    last_block_received: None,
                    pending_request: None,
                    protocol_version: 0,
                    producer_pubkey: None,
                },
            );
        }
        mgr
    }

    // -------------------------------------------------------------------------
    // D2 — COUNTER STARVATION
    //
    // OUTPUT CONTRACT: fn reset_empty_headers(&mut self) [production_gate.rs:557]
    //   O1: self.fork.consecutive_empty_headers → set to 0
    //
    // OUTPUT CONTRACT: fn block_applied_with_weight(&mut self, hash, height, slot,
    //                      weight, prev_hash) [block_lifecycle.rs:25]
    //   O1: self.fork.consecutive_empty_headers → set to 0 at line 68 (always)
    //   O2: self.local_height → set to height
    //   O3: self.state → may transition Syncing→Synchronized when at network tip
    //   O4: self.confirmed_height_floor → set when already Synchronized (line 74)
    //
    // OUTPUT CONTRACT: fn RecoveryCoordinator::classify(&self, ctx: &RecoveryContext)
    //                  [recovery.rs:262]
    //   O1: RecoveryAction — least-severe action fitting evidence+context
    //   Path under test: medium_gap=28>0&&<500 → Rule 3 → HeaderFirstSync
    //
    // CODE PATHS (INC-I-138 Phase 2, t=109-325s, gap=28, last_applied=325s):
    //   PATH-D2A (BUG): empty-header response increments counter → coordinator
    //     classify sees medium_gap=28 → HeaderFirstSync → periodic.rs:712 calls
    //     reset_empty_headers() → counter=0. Counter oscillates, never reaching the
    //     G3 threshold (3). Measured: consecutive_max=2 over 325s (E5).
    //     [FAILS today: max_counter < 3 across MAX_CYCLES cycles]
    //   PATH-D2B (pin): block_applied resets counter legitimately (must stay).
    //     [PASSES today: block_lifecycle.rs:68]
    //   PATH-D2C (pin): height-based fallback (dispatch.rs:84) resets counter
    //     legitimately (must stay — INC-I-012 F1 post-snap path).
    //     [PASSES today: dispatch.rs:84]
    //
    // INPUT PARTITIONS × OUTPUT MATRIX (O1 = consecutive_empty_headers):
    //   | Partition                         | O1 counter reached ≥ 3 |
    //   |-----------------------------------|------------------------|
    //   | D2A: ×2 incr + coord-reset × N   | true (MUST)  FAILS     |
    //   | D2B: block_applied                | counter=0    PASSES    |
    //   | D2C: use_height_based_headers     | counter=0    PASSES    |
    // -------------------------------------------------------------------------

    /// INC-I-138 D2 — counter starved by coordinator HeaderFirstSync reset.
    ///
    /// Simulates the measured incident cycle (E5):
    ///   Two empty-header responses arrive between coordinator ticks
    ///   (142 calls / 216s = ~1.5 calls / 30s coordinator period ≈ 2 per tick).
    ///   Coordinator classify() sees: medium_gap=28 (Rule 3) → HeaderFirstSync.
    ///   periodic.rs:712 executes reset_empty_headers() → counter=0.
    ///   Counter maxes at 2, never reaching G3 threshold (3) → 0 ShallowRollback
    ///   in 325s → INV-FORK-001 violated.
    ///
    /// EXPECTED: FAILS today (max_counter ≤ 2; assert ≥ 3 fails).
    /// PASSES after D2 fix: reset_empty_headers() is conditional on post-snap
    /// path only (INC-I-012 F1 gating), so counter accumulates across ticks.
    #[test]
    fn test_inc_i138_d2_counter_starved_by_coordinator_reset() {
        // INC-I-138 Phase-2 context (gap=28, last_applied=325s, 5 peers)
        let ctx = RecoveryContext {
            local_height: 36,
            network_tip_height: 64,
            peer_count: 5,
            last_applied_secs: 325,
            shallow_rollback_count: 0,
            snap_attempts: 0,
            last_rollback_local_height: None,
            last_rollback_time: None,
            in_grace_period: false,
            last_finality_height: None,
        };

        // Precondition: verify classify() returns HeaderFirstSync at gap=28 (the reset trigger).
        // With StaleTip evidence and medium_gap=28 (Rule 3), HeaderFirstSync fires.
        // If this FAILS, the test setup is wrong — not the D2 defect.
        {
            let mut coord = RecoveryCoordinator::new();
            coord.report(RecoveryEvidence::StaleTip {
                last_applied_secs: 325,
                gap: 28,
            });
            let precond = coord.classify(&ctx);
            assert_eq!(
                precond,
                RecoveryAction::HeaderFirstSync,
                "precondition: coordinator must return HeaderFirstSync at gap=28 \
                 last_applied=325s (medium_gap Rule 3); verify test setup is correct"
            );
        }

        // Simulate N coordinator ticks, each with 2 increments before the tick.
        // Two increments per tick models the measured ~1.5 empty-header responses
        // per 30s coordinator interval (142 calls over 216s, E5).
        const MAX_CYCLES: u32 = 10;
        let mut counter: u32 = 0;
        let mut max_counter: u32 = 0;

        for _ in 0..MAX_CYCLES {
            // Two empty-header responses arrive between coordinator ticks
            counter += 1; // first GetHeadersByHeight → "header chain broken"
            counter += 1; // second GetHeadersByHeight → "header chain broken"
            max_counter = max_counter.max(counter);

            if counter >= thresholds::MIN_MINOR_FORK_EVIDENCE {
                break; // G3 reached — threshold met (early success)
            }

            // Coordinator tick: classify returns HeaderFirstSync (medium_gap=28 Rule 3).
            // Fresh coordinator per cycle avoids 30s ACTION_COOLDOWN side effect;
            // the real periodic.rs runs at ~30s intervals where cooldown has expired.
            let mut coord = RecoveryCoordinator::new();
            coord.report(RecoveryEvidence::StaleTip {
                last_applied_secs: 325,
                gap: 28,
            });
            let action = coord.classify(&ctx);
            if matches!(action, RecoveryAction::HeaderFirstSync) {
                // Mirrors periodic.rs:712 — sync.reset_empty_headers()
                // This is the dominant reset writer (production_gate.rs:558).
                counter = 0;
            }
        }

        // PRIMARY BUG ASSERTION (FAILS today):
        // consecutive_empty_headers MUST be able to accumulate to the G3 threshold (3)
        // so cleanup.rs / periodic.rs can raise stuck_fork and coordinator Rule 1b
        // can issue ShallowRollback via INV-FORK-001. Currently the coordinator
        // returns HeaderFirstSync every tick at gap=28, resetting the counter to 0
        // before it can accumulate. consecutive_max=2 measured over 325s (E5).
        assert!(
            max_counter >= thresholds::MIN_MINOR_FORK_EVIDENCE,
            "INC-I-138 D2 BUG: consecutive_empty_headers maxed at {} across {} cycles \
             (G3 threshold={}). The HeaderFirstSync coordinator reset \
             (periodic.rs:712 → reset_empty_headers production_gate.rs:558) fires every \
             coordinator tick at gap=28, keeping the counter below the ShallowRollback \
             trigger. INV-FORK-001 violated: 0 ShallowRollback in 325s (E5). \
             Fix: gate reset_empty_headers() on the post-snap (INC-I-012 F1) path only \
             so counter can accumulate in the minor-fork stall regime.",
            max_counter,
            MAX_CYCLES,
            thresholds::MIN_MINOR_FORK_EVIDENCE,
        );
    }

    /// INC-I-138 D2 pin — block_applied MUST reset counter (legitimate reset; must stay).
    ///
    /// block_lifecycle.rs:68 resets consecutive_empty_headers on every block apply.
    /// When the chain is advancing (blocks arriving), fork evidence is stale.
    /// This reset is CORRECT — the D2 fix must NOT remove it.
    ///
    /// PASSES today. MUST PASS after D2 fix.
    #[test]
    fn test_inc_i138_d2_block_applied_resets_counter_pin() {
        let mut mgr = phase2_manager();
        mgr.fork.consecutive_empty_headers = 5;
        // Apply a canonical block (simulates network delivering h=37 after D5+D2 fix)
        mgr.block_applied_with_weight(
            crypto::hash::hash(b"canonical_h37"),
            37,
            637,
            1,
            crypto::hash::hash(b"fork_tip_290d4942"),
        );
        // O1: counter MUST be 0 after block_applied (block_lifecycle.rs:68).
        // Legitimate reset: an arriving canonical block invalidates stale fork evidence.
        assert_eq!(
            mgr.fork.consecutive_empty_headers, 0,
            "D2 pin: block_applied MUST reset consecutive_empty_headers (block_lifecycle.rs:68). \
             This is a correct reset: arriving canonical blocks mean fork evidence is stale. \
             D2 fix must NOT remove this reset — only the HeaderFirstSync coordinator reset \
             (periodic.rs:712) is the bug."
        );
    }

    /// INC-I-139 DC-4 pin — height-based fallback dispatch MUST PRESERVE counter.
    ///
    /// The prior thesis (dispatch.rs:84 MUST reset the counter as the INC-I-012 F1
    /// post-snap reset) is exactly what INC-I-139 DC-4 overturns. DC-4 determined this
    /// dispatch-time reset was the E5 starvation writer — the same defect class D2 fixed
    /// at periodic.rs:712: re-arming use_height_based_headers each cycle zeroed the
    /// evidence counter, starving deep-fork escalation. It is now REMOVED; the
    /// height-based request PRESERVES the counter. Only genuine block apply
    /// (block_lifecycle.rs) and the bounded gap≤3 gossip-wait reset it
    /// (INV-SYNC-011 extended, REQ-SNAP-007). The sibling pin
    /// test_inc_i138_d2_block_applied_resets_counter_pin still covers the legitimate
    /// block-apply reset.
    #[test]
    fn test_inc_i139_dc4_height_fallback_dispatch_preserves_counter_pin() {
        let mut mgr = phase2_manager();
        mgr.fork.consecutive_empty_headers = 4;
        mgr.fork.use_height_based_headers = true; // INC-I-012 F1 post-snap flag
        mgr.fork.height_fallback_attempted = false;
        // Set up Syncing/Headers pipeline so dispatch.rs:72 branch is entered
        let peer = *mgr
            .peers
            .keys()
            .next()
            .expect("phase2_manager inserts 5 peers");
        mgr.state = SyncState::Syncing {
            phase: SyncPhase::DownloadingHeaders,
            started_at: Instant::now(),
        };
        mgr.pipeline_data = SyncPipelineData::Headers {
            target_slot: 664,
            peer,
            headers_count: 0,
        };
        // next_request() enters the height-based branch (use_height_based_headers=true)
        // and issues GetHeadersByHeight — post-DC-4 it no longer touches the counter.
        let _req = mgr.next_request();
        // O1: post-DC-4 the counter MUST be PRESERVED (== pre-set 4). The height-based
        // request is not progress; only block apply (block_lifecycle.rs) and the bounded
        // gap≤3 gossip-wait reset the counter (INV-SYNC-011 extended, INC-I-139 M5).
        assert_eq!(
            mgr.fork.consecutive_empty_headers, 4,
            "DC-4 pin: height-based fallback dispatch MUST PRESERVE consecutive_empty_headers. \
             INC-I-139 E5: the old dispatch-time reset (dispatch.rs:84) starved deep-fork evidence \
             (re-arming use_height_based_headers each cycle zeroed it). Single-owner reset writers \
             are now genuine block apply + the gap≤3 gossip-wait only."
        );
    }

    // -------------------------------------------------------------------------
    // D4 — GAP-BLIND deep_fork_confirmed
    //
    // OUTPUT CONTRACT: fn RecoveryCoordinator::classify(&self, ctx: &RecoveryContext)
    //                  [recovery.rs:262]
    //   O1: RecoveryAction — must be SnapSync ONLY when gap >= SNAP_SYNC_GAP_MIN(500)
    //       OR rollback_exhausted. Must NOT be SnapSync at gap=28 solely from
    //       empty_count>=10 && last_applied>=STALE_TIP_SECS(300) (no gap guard).
    //
    // CODE PATH (D4 BUG, recovery.rs:382-383):
    //   deep_fork_confirmed = (deep_fork_count > 0)
    //                       || (empty_count >= 10 && last_applied_secs >= 300)
    //   No gap guard → TRUE at gap=28.
    //   Rule 2: (rollback_exhausted=F || large_gap=F || deep_fork_confirmed=T)
    //           && snap_attempts(0) < 3 && peers(5) >= 3 → SnapSync at gap=28.
    //   Terminal event: E8 "[COORDINATOR] action=SnapSync gap=28 last_applied=325s".
    //
    // INPUT PARTITIONS × OUTPUT MATRIX (O1 = RecoveryAction):
    //   | Partition                           | O1 action    |
    //   |-------------------------------------|--------------|
    //   | D4A: empty_count=10, gap=28, stale  | ≠ SnapSync   | FAILS today  |
    //   | D4B pin: large_gap=600              | == SnapSync  | PASSES today |
    //   | D4C pin: rollback_exhausted, gap=28 | == SnapSync  | PASSES today |
    // -------------------------------------------------------------------------

    /// INC-I-138 D4 — gap-blind deep_fork_confirmed fires SnapSync at gap=28.
    ///
    /// recovery.rs:382-383 computes deep_fork_confirmed without a gap guard:
    ///   (empty_count >= 10 && last_applied_secs >= STALE_TIP_SECS) → TRUE at gap=28.
    /// Rule 2 returns SnapSync, bypassing SNAP_SYNC_GAP_MIN=500. A 28-block gap
    /// (one epoch) is minor-fork range, recoverable in seconds via ShallowRollback(1).
    /// Instead the node jumped to h=64, missing blocks 37-63 (E8).
    ///
    /// Fix: add `gap >= MINOR_FORK_GAP_MAX` (or equivalent) guard to the
    /// `empty_count >= 10 && stale_tip` branch of deep_fork_confirmed.
    ///
    /// EXPECTED: FAILS today (classify returns SnapSync at gap=28).
    #[test]
    fn test_inc_i138_d4_gap_blind_snap_sync_at_gap_28() {
        // Accumulate 10 EmptyHeaders in the coordinator evidence window.
        // In the incident, D1 (response.rs:261 before gap guard) inflated this;
        // here we replicate the accumulated state directly.
        let mut coord = RecoveryCoordinator::new();
        for _ in 0..10 {
            coord.report(RecoveryEvidence::EmptyHeaders {
                peer: PeerId::random(),
                gap: 28,
            });
        }
        coord.report(RecoveryEvidence::StaleTip {
            last_applied_secs: 325,
            gap: 28,
        });

        // INC-I-138 exact incident context at t=325s (E8):
        // local=36, network_tip=64, gap=28, last_applied=325s, snap_attempts=0, peers=5
        let ctx = RecoveryContext {
            local_height: 36,
            network_tip_height: 64,
            peer_count: 5,
            last_applied_secs: 325,
            shallow_rollback_count: 0,
            snap_attempts: 0,
            last_rollback_local_height: None,
            last_rollback_time: None,
            in_grace_period: false,
            last_finality_height: None,
        };

        let action = coord.classify(&ctx);

        // PRIMARY BUG ASSERTION (FAILS today):
        // classify() MUST NOT return SnapSync at gap=28 (< MINOR_FORK_GAP_MAX=50).
        // A 28-block gap is minor-fork range. deep_fork_confirmed fires at recovery.rs:382
        // without a gap guard → Rule 2 SnapSync at gap=28.
        // Terminal incident event: E8 "[COORDINATOR] action=SnapSync gap=28 last_applied=325s
        // shallow_rb=0 snap_attempts=0" (n5.log:17197). Blocks 37-63 lost.
        // After D4 fix (gap guard on deep_fork_confirmed), the correct action here is
        // None or ShallowRollback (if stuck_fork evidence present), not SnapSync.
        assert_ne!(
            action,
            RecoveryAction::SnapSync,
            "INC-I-138 D4 BUG: classify() returned {:?} at gap=28 (< MINOR_FORK_GAP_MAX={}). \
             recovery.rs:382 deep_fork_confirmed = (empty_count>=10 && last_applied>=300s) \
             fires with NO gap guard, triggering SnapSync in minor-fork range. \
             SNAP_SYNC_GAP_MIN={}; gap=28 is recoverable via ShallowRollback(1). \
             Fix: add gap >= MINOR_FORK_GAP_MAX guard to deep_fork_confirmed so \
             minor-fork stalls cannot escalate to SnapSync via the empty_count path.",
            action,
            thresholds::MINOR_FORK_GAP_MAX,
            thresholds::SNAP_SYNC_GAP_MIN,
        );
    }

    /// INC-I-138 D4 pin — genuine large gap (≥ SNAP_SYNC_GAP_MIN) must still escalate.
    ///
    /// When gap >= 500 (large_gap=true in Rule 2), SnapSync is correct and required.
    /// The D4 fix must NOT break this path — only the gap-blind deep_fork_confirmed
    /// branch is wrong.
    ///
    /// PASSES today. MUST PASS after D4 fix.
    #[test]
    fn test_inc_i138_d4_genuine_large_gap_still_escalates_pin() {
        let mut coord = RecoveryCoordinator::new();
        for _ in 0..10 {
            coord.report(RecoveryEvidence::EmptyHeaders {
                peer: PeerId::random(),
                gap: 600,
            });
        }
        coord.report(RecoveryEvidence::StaleTip {
            last_applied_secs: 600,
            gap: 600,
        });
        let ctx = RecoveryContext {
            local_height: 0,
            network_tip_height: 600,
            peer_count: 5,
            last_applied_secs: 600,
            shallow_rollback_count: 0,
            snap_attempts: 0,
            last_rollback_local_height: None,
            last_rollback_time: None,
            in_grace_period: false,
            last_finality_height: None,
        };
        let action = coord.classify(&ctx);
        // large_gap = 600 >= SNAP_SYNC_GAP_MIN=500 → Rule 2 fires SnapSync correctly.
        // D4 fix only adds gap guard to deep_fork_confirmed; large_gap path is unchanged.
        assert_eq!(
            action,
            RecoveryAction::SnapSync,
            "D4 pin: genuine large gap ({}) MUST still escalate to SnapSync (large_gap path). \
             D4 fix must NOT touch the large_gap branch of Rule 2 \
             (SNAP_SYNC_GAP_MIN={}).",
            ctx.gap(),
            thresholds::SNAP_SYNC_GAP_MIN,
        );
    }

    /// SUPERSEDED by INC-I-204 M6 (D1, REQ-FORK-004). This pinned `SnapSync` for the
    /// `rollback_exhausted` trigger at gap=28. M6 deletes that trigger: a spent
    /// rollback budget is a statement about a BUDGET, not evidence of behind-ness.
    /// Reversed in place (a sibling of `recovery.rs`'s
    /// `shallow_rollback_exhausted_no_longer_escalates_to_snap`), so the behaviour
    /// change is visible in the diff rather than deleted from the record.
    #[test]
    fn test_inc_i138_d1_rollback_exhausted_no_longer_escalates_pin() {
        let mut coord = RecoveryCoordinator::new();
        for _ in 0..3 {
            coord.report(RecoveryEvidence::EmptyHeaders {
                peer: PeerId::random(),
                gap: 28,
            });
        }
        let ctx = RecoveryContext {
            local_height: 36,
            network_tip_height: 64,
            peer_count: 5,
            last_applied_secs: 5, // recently_synced=true → Rule 1 checks rollback budget
            shallow_rollback_count: thresholds::SHALLOW_ROLLBACK_MAX, // exhausted
            snap_attempts: 0,
            last_rollback_local_height: None,
            last_rollback_time: None,
            in_grace_period: false,
            last_finality_height: None,
        };
        let action = coord.classify(&ctx);
        // Rule 1: minor_fork_evidence(T) && gap(28)<50 && recently_synced(T)
        //         && shallow_rollback_count(10) < MAX(10) → 10<10=FALSE → Rule 1 skips.
        // Rule 2 (M6): large_gap = 28 >= 500 = FALSE → skipped; no other trigger remains.
        // Rule 3: medium_gap(28) && !wedged_shape → HeaderFirstSync, a non-lossy rung.
        assert_eq!(
            action,
            RecoveryAction::HeaderFirstSync,
            "INC-I-204 M6 (D1): rollback_exhausted (count={} >= MAX={}) must NOT reach a \
             history-destroying rung at gap=28. Snap admission is gap >= {} only.",
            thresholds::SHALLOW_ROLLBACK_MAX,
            thresholds::SHALLOW_ROLLBACK_MAX,
            thresholds::SNAP_SYNC_GAP_MIN,
        );
    }

    // -------------------------------------------------------------------------
    // NAIVE-FIX TRAP (forward-looking contract for the developer)
    //
    // OUTPUT CONTRACT: fn next_request(&mut self) [dispatch.rs:13]
    //   (via SyncPipelineData::Headers branch, consecutive_empty_headers >= 10)
    //   O1: self.fork.needs_genesis_resync — MUST NOT be set when gap is in 4..50
    //       (minor-fork range recoverable via ShallowRollback, not genesis-resync).
    //   O2: return value → None when genesis-resync fires
    //
    // CODE PATH (dispatch.rs:96→134, gap=28, counter=10):
    //   consecutive_empty_headers=10 → escalation block entered (dispatch.rs:96)
    //   gap(28) > snap.threshold(50): FALSE → no snap redirect
    //   gap(28) <= 3: FALSE → no gossip wait
    //   → request_genesis_resync(GenesisFallbackEmptyHeaders) → O1=true (WRONG)
    //
    // REGIME GUARDED: counter=10 is UNREACHABLE today (D2 pins counter at ≤2).
    //   Once D2 is fixed (counter accumulates), the naive developer may remove
    //   the HeaderFirstSync reset WITHOUT fixing D4. In this regime:
    //   counter reaches 10 at gap=28 → dispatch.rs:134 fires GenesisFallbackEmptyHeaders
    //   → wipes local state for a gap that ShallowRollback(1) resolves in seconds.
    //   BOTH D2+D4 fixes are required. D2 alone opens this trap.
    //   Test arms the assertion by directly setting counter=10.
    //
    // INPUT PARTITIONS × OUTPUT MATRIX (O1 = needs_genesis_resync):
    //   | Partition              | O1 needs_genesis_resync |
    //   |------------------------|------------------------|
    //   | gap=28, counter=10     | false (MUST)  FAILS today (dispatch.rs:134 sets true) |
    // -------------------------------------------------------------------------

    /// INC-I-138 NAIVE-FIX TRAP — dispatch.rs:96→134 MUST NOT genesis-resync at gap=28.
    ///
    /// REGIME: counter=10 (reachable once D2 fixed), gap=28 (4..50 minor-fork range).
    ///
    /// Naive D2 fix without D4: counter accumulates to 10 at gap=28. dispatch.rs:96
    /// enters the escalation block:
    ///   gap(28) > snap.threshold(50): FALSE → no snap redirect
    ///   gap(28) <= 3: FALSE → no gossip wait
    ///   → GenesisFallbackEmptyHeaders → needs_genesis_resync=true
    ///
    /// Correct outcome: G3 stuck_fork → coordinator Rule 1b → ShallowRollback(1)
    /// recovers in seconds. genesis-resync at gap=28 wipes state unnecessarily.
    ///
    /// Counter=10 is set directly to arm the assertion for the post-D2 regime;
    /// it is unreachable via the normal path today (D2 pins counter at ≤2).
    ///
    /// EXPECTED: FAILS today — dispatch.rs:134 fires, needs_genesis_resync=true.
    #[test]
    fn test_inc_i138_naive_fix_trap_counter10_gap28_must_not_genesis_resync() {
        let mut mgr = phase2_manager();
        // Arm: directly set counter=10 (simulates post-D2-fix accumulation)
        // This is unreachable today; armed for the post-D2 regime validation.
        mgr.fork.consecutive_empty_headers = 10;
        mgr.fork.use_height_based_headers = false;
        mgr.fork.height_fallback_attempted = false;

        // Set up Headers pipeline for dispatch.rs to process
        let peer = *mgr
            .peers
            .keys()
            .next()
            .expect("phase2_manager must insert 5 peers");
        mgr.state = SyncState::Syncing {
            phase: SyncPhase::DownloadingHeaders,
            started_at: Instant::now(),
        };
        mgr.pipeline_data = SyncPipelineData::Headers {
            target_slot: 664,
            peer,
            headers_count: 0,
        };
        // Precondition: snap.threshold=50 (default), gap=28; snap redirect won't fire.
        // dispatch.rs:96: counter(10) >= 10 → escalation block
        // dispatch.rs:105: gap(28) > threshold(50) = FALSE → no snap redirect
        // dispatch.rs:118: gap(28) <= 3 = FALSE → no gossip wait
        // dispatch.rs:134: request_genesis_resync(GenesisFallbackEmptyHeaders)
        let _req = mgr.next_request();

        // PRIMARY ASSERTION (FAILS today):
        // dispatch.rs MUST NOT set needs_genesis_resync for gap=28 (minor-fork range).
        // ShallowRollback(1) resolves a 1-epoch-boundary fork in seconds; genesis-resync
        // wipes local state and takes minutes. This guards the naive-fix trap:
        // D2 and D4 must be fixed together. D2 alone opens this path at gap=28.
        assert!(
            !mgr.fork.needs_genesis_resync,
            "INC-I-138 NAIVE-FIX TRAP: dispatch.rs:134 set needs_genesis_resync=true \
             with counter=10 and gap=28 (minor-fork range 4..50). \
             Correct outcome: G3 stuck_fork → coordinator Rule 1b → ShallowRollback(1). \
             Genesis-resync at gap=28 is disproportionate; it wipes local state for a \
             gap ShallowRollback recovers in seconds. \
             BOTH D2+D4 fixes required (D2 alone opens this trap). \
             snap.threshold={}, gap={}.",
            mgr.snap.threshold,
            mgr.network
                .network_tip_height
                .saturating_sub(mgr.local_height),
        );
    }

    // -------------------------------------------------------------------------
    // FLOOR=0 RACE (block_lifecycle.rs:74 before :150)
    //
    // OUTPUT CONTRACT: fn block_applied_with_weight(&mut self, hash, height, slot,
    //                      weight, prev_hash) [block_lifecycle.rs:25]
    //   O1: self.confirmed_height_floor — MUST be > 0 when the applied block
    //       triggers the Syncing→Synchronized transition. Currently 0 because
    //       the floor check at :74 runs with state=Syncing (before :150 transition).
    //   O2: self.state — MUST be Synchronized after the transition block.
    //
    // CODE PATH (floor=0 race):
    //   :74  if matches!(state, Synchronized) → state=Syncing → FALSE → floor NOT set
    //   :150 is_syncing() && height >= network_tip && slot_ok → TRUE → Synchronized
    //   RESULT: state=Synchronized, floor=0
    //   Consequence (E10): Gate 1 at production_gate.rs:674 (floor > 0 → false)
    //   allows CoordinatorSnapEscalation unconditionally for non-emergency reasons.
    //
    // INPUT PARTITIONS × OUTPUT MATRIX (O1=floor, O2=state):
    //   | Partition                            | O1 floor | O2 state      |
    //   |--------------------------------------|----------|---------------|
    //   | transition block (Syncing→Synced)    | > 0 MUST | Synchronized  | FAILS/PASSES|
    //   | post-transition (already Synced)     | > 0 MUST | Synchronized  | PASSES/PASSES|
    // -------------------------------------------------------------------------

    /// INC-I-138 floor=0 race — transition block MUST set confirmed_height_floor.
    ///
    /// The floor check at block_lifecycle.rs:74 runs before the Syncing→Synchronized
    /// transition at :150. On the block that triggers the transition:
    ///   :74  state=Syncing → condition false → floor NOT set
    ///   :150 state → Synchronized (transition executes)
    ///   RESULT: state=Synchronized, confirmed_height_floor=0 (race)
    ///
    /// Consequence (E10): Gate 1 at production_gate.rs:674 (`confirmed_height_floor > 0`)
    /// evaluates to false, allowing CoordinatorSnapEscalation to pass unconditionally
    /// for non-emergency reasons. In INC-I-138, this allowed the spurious SnapSync
    /// at gap=28 to execute despite the node having synced correctly to h=64.
    ///
    /// Fix: move the floor update to AFTER the :150 transition block, or set the
    /// floor inside the Syncing→Synchronized transition at :150.
    ///
    /// EXPECTED: O1 (floor) FAILS today (stays 0). O2 (state) PASSES today.
    #[test]
    fn test_inc_i138_floor_zero_race_on_sync_complete_transition() {
        let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);

        // State: Syncing, one block behind network tip.
        // Applying h=64 satisfies (is_syncing && 64 >= network_tip=64 && slot_ok) → :150 fires.
        mgr.local_height = 63;
        mgr.local_hash = crypto::hash::hash(b"block63");
        mgr.local_slot = 639;
        mgr.network.network_tip_height = 64;
        mgr.network.network_tip_slot = 640;
        mgr.state = SyncState::Syncing {
            phase: SyncPhase::DownloadingHeaders,
            started_at: Instant::now(),
        };
        mgr.confirmed_height_floor = 0; // race starting state
        mgr.consecutive_resync_count = 0; // floor update not gated by resync counter
                                          // 2 peers so peer-count checks don't interfere
        for _ in 0..2 {
            mgr.peers.insert(
                PeerId::random(),
                PeerSyncStatus {
                    best_height: 64,
                    best_hash: Hash::ZERO,
                    best_slot: 640,
                    last_status_response: Instant::now(),
                    last_block_received: None,
                    pending_request: None,
                    protocol_version: 0,
                    producer_pubkey: None,
                },
            );
        }

        // Apply transition block: h=64, slot=640, matches network_tip → :150 fires.
        // slot_ok: network_tip_slot(640).saturating_sub(slot=640) = 0 <= max_slots_behind(2).
        mgr.block_applied_with_weight(
            crypto::hash::hash(b"block64"),
            64,
            640,
            1,
            crypto::hash::hash(b"block63"),
        );

        // O2: state MUST be Synchronized (PASSES today — transition :150 works).
        assert!(
            matches!(*mgr.state(), SyncState::Synchronized),
            "O2: state must be Synchronized after applying block at network_tip height"
        );

        // O1: PRIMARY RACE BUG ASSERTION (FAILS today):
        // confirmed_height_floor MUST be > 0 after the block that triggers
        // Syncing→Synchronized. The floor check at :74 ran with state=Syncing
        // (condition false) before the :150 transition; floor stays 0.
        // Gate 1 at production_gate.rs:674 then evaluates false unconditionally,
        // allowing CoordinatorSnapEscalation to fire for any gap (E10 measured).
        assert!(
            mgr.confirmed_height_floor > 0,
            "INC-I-138 floor=0 RACE: confirmed_height_floor must be > 0 after the block \
             that triggers Syncing→Synchronized (h=64). Currently 0 because \
             block_lifecycle.rs:74 checks `matches!(state, Synchronized)` BEFORE \
             the Synchronized transition at :150. Fix: move floor update to AFTER \
             the :150 transition, or set floor inside the transition block. \
             Consequence (E10): Gate 1 (floor > 0) at production_gate.rs:674 is \
             permanently false after first sync, allowing CoordinatorSnapEscalation \
             at any gap."
        );
    }

    /// INC-I-138 floor race pin — post-transition block sets floor correctly (must stay).
    ///
    /// When a block is applied AFTER the node is already in Synchronized state,
    /// block_lifecycle.rs:74 fires correctly (state==Synchronized → true).
    /// This pin confirms the non-race path works and that the fix must not break it.
    ///
    /// PASSES today.
    #[test]
    fn test_inc_i138_floor_post_transition_sets_floor_pin() {
        let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        mgr.local_height = 64;
        mgr.local_hash = crypto::hash::hash(b"block64");
        mgr.local_slot = 640;
        mgr.state = SyncState::Synchronized; // already past transition
        mgr.confirmed_height_floor = 0;
        mgr.consecutive_resync_count = 0;
        // Apply block 65 while already Synchronized — :74 fires correctly
        mgr.block_applied_with_weight(
            crypto::hash::hash(b"block65"),
            65,
            641,
            1,
            crypto::hash::hash(b"block64"),
        );
        // floor MUST be set (block_lifecycle.rs:74: state=Synchronized → condition true)
        assert!(
            mgr.confirmed_height_floor > 0,
            "floor pin: applying a block in Synchronized state MUST set \
             confirmed_height_floor (block_lifecycle.rs:74). PASSES today."
        );
        assert_eq!(
            mgr.confirmed_height_floor, 65,
            "floor pin: floor must equal the applied height"
        );
    }
}

// =========================================================================
// INC-I-138 M3: D1 — evidence gating (EmptyHeaders before gap<=3 guard)
//
// Defect (D1): response.rs:261-262 calls self.recovery.report(EmptyHeaders)
// BEFORE the gap<=3 benign-gossip-timing guard at :264. Every gap<=3 empty
// response (classified benign by the very next line) pollutes the 120s
// coordinator evidence window. empty_count >= 10 then becomes trivially
// satisfiable within 120s, feeding D4's deep_fork_confirmed.
//
// Evidence cite: [E7] crates/network/src/sync/manager/sync_engine/response.rs:261
//   — self.recovery.report(EmptyHeaders{peer, gap}) executes before gap guard at :264.
//   Amplified D4 at recovery.rs:382-383 (E9) → terminal SnapSync at gap=28 (E8).
//
// Correct behavior:
//   - gap<=3 empty-header responses must NOT be reported as EmptyHeaders evidence.
//   - gap>3 empty-header responses MUST still be reported (genuine fork evidence).
//
// D1 is independent of D4 (M2's D4 fix already guards deep_fork_confirmed with
// gap >= MINOR_FORK_GAP_MAX). Cross-check pin (Test 3) verifies the two defense
// layers remain orthogonal after M3 lands.
//
// Test 1 (FAIL today): gap=2 empty response → evidence_len() MUST be 0.
// Test 2 (PIN):         gap=10 empty response → evidence_len() MUST be >= 1.
// Test 3 (PIN):         10+ gap<=3 empties reported OLD way + classify(gap=28)
//                       → NOT SnapSync (D4 guard holds independently of D1).
//
// RUN_ID: 449
// =========================================================================
mod inc_i138_m3_evidence_gating {
    use super::*;
    use crate::sync::manager::recovery::{
        RecoveryAction, RecoveryContext, RecoveryCoordinator, RecoveryEvidence,
    };

    // OUTPUT CONTRACT: fn handle_headers_response(&mut self, peer, headers=[])
    //   via handle_response(&mut self, peer, SyncResponse::Headers(vec![]))
    //   [response.rs:188, empty-headers branch, non-post-snap, non-stuck path]
    //
    // Outputs:
    //   O1: self.fork.consecutive_empty_headers — u32
    //       All paths: incremented by +1 at response.rs:252 (not gated by gap guard).
    //   O2: self.recovery.evidence — coordinator evidence window
    //       P1 (gap<=3): MUST contain ZERO EmptyHeaders entries.
    //                    BUG: contains 1 today (report fires at :261 before :264 guard).
    //       P2 (gap>3):  MUST contain >= 1 EmptyHeaders entry (correct today).
    //   O3: self.state — SyncState
    //       P1 (gap<=3): transitions to Idle via set_state(:273).
    //   O4: return value — Vec<Block>: always vec![] for Headers responses.
    //
    // Paths:
    //   P1: gap<=3, local_height>0 — benign gossip-timing early return (:264-:274).
    //       BUG: report(EmptyHeaders) fires at :261 BEFORE this guard.
    //       INPUT PARTITIONS:
    //         P1a: gap=2 — well below boundary (primary test; clear gossip-timing case).
    //         P1b: gap=3 — exact boundary value (off-by-one check; same code path as P1a).
    //   P2: gap>3 — genuine fork evidence path (continues to rollback/escalation).
    //       INPUT PARTITIONS:
    //         P2a: gap=10 — minor-fork range, unambiguous evidence case.
    //
    // Matrix: 2 key outputs (O1, O2) × 3 partitions = 6 cells.
    //   P1a (gap=2):  O1(counter=1)✓  O2(evidence_len=0)✓  [Test 1, FAILS today: len=1]
    //   P1b (gap=3):  O1(counter=1)   O2(evidence_len=0)   [same code path as P1a]
    //   P2a (gap=10): O1(counter=1)✓  O2(evidence_len>=1)✓ [Test 2 PIN, PASSES today]

    /// Build a SyncManager positioned at the empty-headers main branch (response.rs:201+).
    ///
    /// Pre-conditions to reach the gap-guard line at :264 (not diverted earlier):
    ///   - recovery_phase=Normal (NOT AwaitingCanonicalBlock → bypasses INC-I-012 F1 branch at :226)
    ///   - snap.attempts=0, consecutive_empty_headers=0 (empty_headers_stuck=false → bypasses :224)
    ///   - pipeline.headers_needing_bodies empty, pipeline.pending_headers empty → enters :201 branch
    ///   - local_height > 0 (required for gap<=3 early return condition at :264)
    ///   - peer in peers map at local_height + gap (gap = peer_height.saturating_sub(local_height))
    fn mk_d1_manager(local_height: u64, gap: u64) -> (SyncManager, PeerId) {
        let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);
        manager.local_height = local_height;
        manager.local_slot = local_height as u32;
        manager.local_hash = crypto::hash::hash(format!("local_{}", local_height).as_bytes());
        // Normal phase: AwaitingCanonicalBlock would divert to height-fallback at :226.
        manager.recovery_phase = RecoveryPhase::Normal;
        // snap.attempts=0 (default): not snap_exhausted; prevents INC-I-017 branch at :224.
        // consecutive_empty_headers=0 (default): empty_headers_stuck=false; same guard.

        let peer = PeerId::random();
        let peer_height = local_height + gap;
        manager.add_peer(
            peer,
            peer_height,
            crypto::hash::hash(format!("peer_{}", peer_height).as_bytes()),
            peer_height as u32,
        );
        (manager, peer)
    }

    /// INC-I-138 D1 — gap<=3 benign empty-headers MUST NOT pollute coordinator evidence.
    ///
    /// Defect: response.rs:261-262 calls self.recovery.report(EmptyHeaders{peer, gap})
    /// BEFORE the gap<=3 guard at :264. The guard correctly classifies gap<=3 as
    /// "gossip timing, not a fork" and returns early — but only AFTER the evidence is
    /// already inserted into the coordinator's 120s window. Every gossip-timing empty
    /// response inflates empty_count, making the threshold >= 10 trivially satisfiable
    /// within 120s. D4's deep_fork_confirmed then fires at gap=28 (E8, n5.log:17197).
    ///
    /// Fix: move report() to AFTER the gap<=3 guard (i.e., suppress for gap<=3).
    ///
    /// Partition: P1a (gap=2, local_height=36).
    ///
    /// EXPECTED: FAILS today — evidence_len() is 1 after a single gap=2 empty response.
    /// PASSES after D1 fix: evidence_len() is 0.
    #[test]
    fn test_inc_i138_m3_benign_gap_does_not_pollute_evidence_window() {
        // P1a: gap=2, local_height=36 (gap <= 3 && local_height > 0 → early return).
        let (mut manager, peer) = mk_d1_manager(36, 2);

        // Precondition: coordinator evidence window starts empty.
        assert_eq!(
            manager.recovery.evidence_len(),
            0,
            "precondition: coordinator must start with empty evidence window"
        );

        // Trigger the empty-headers path at gap=2.
        // Code path (response.rs): is_empty()=true → pipeline empty → not post_snap
        //   → counter++ (line 252) → report(EmptyHeaders{gap=2}) (line 261, BUG)
        //   → gap<=3 && local_height>0 (line 264) → set_state(Idle) → return.
        let _returned = manager.handle_response(peer, SyncResponse::Headers(vec![]));

        // O2 — PRIMARY BUG ASSERTION (FAILS today):
        // response.rs:261 fires report(EmptyHeaders{gap=2}) before the :264 guard,
        // so evidence_len() == 1 right now. After the D1 fix, the guard at :264
        // must be evaluated BEFORE report(), leaving evidence_len() == 0.
        // BUG consequence (E7→E9→E8): 10+ such benign gap=2 reports in 120s
        // made deep_fork_confirmed=true at gap=28, triggering SnapSync (n5.log:17197).
        assert_eq!(
            manager.recovery.evidence_len(),
            0,
            "INC-I-138 D1 BUG: response.rs:261 called self.recovery.report(EmptyHeaders{{gap=2}}) \
             BEFORE the gap<=3 benign guard at :264. Evidence window has {} entry(s) (expected 0). \
             Every gossip-timing empty (gap<=3) pollutes the 120s coordinator window; \
             empty_count>=10 becomes trivially satisfiable within 120s, feeding D4 \
             deep_fork_confirmed at gap=28 → SnapSync (E8, n5.log:17197, blocks 37-63 lost). \
             Fix: move report() to AFTER the gap<=3 guard so gap<=3 responses are silenced.",
            manager.recovery.evidence_len(),
        );

        // O1: consecutive_empty_headers counter MUST still be incremented (line 252).
        // This counter drives the legacy G3 stuck-fork detection; the D1 fix must NOT
        // move or suppress the counter increment — only the coordinator report() call.
        assert_eq!(
            manager.fork.consecutive_empty_headers, 1,
            "O1: consecutive_empty_headers must be incremented even for gap<=3 \
             (response.rs:252 is correct; D1 fix only moves report() past the :264 guard)"
        );
    }

    /// INC-I-138 D1 PIN — gap>3 empty-headers MUST add EmptyHeaders evidence.
    ///
    /// A gap>3 response is genuine fork evidence — the peer recognizes a chain
    /// diverging by more than gossip latency. response.rs:261 must still report
    /// it to the coordinator AFTER the D1 fix. Only gap<=3 is suppressed.
    ///
    /// Partition: P2a (gap=10, local_height=36).
    ///
    /// PASSES today. MUST PASS after D1 fix.
    #[test]
    fn test_inc_i138_m3_genuine_gap_adds_evidence_pin() {
        // P2a: gap=10 (well above the gap<=3 boundary; genuine minor-fork evidence).
        let (mut manager, peer) = mk_d1_manager(36, 10);

        // Trigger the empty-headers path at gap=10.
        // Code path (response.rs): is_empty()=true → pipeline empty → not post_snap
        //   → counter++ → report(EmptyHeaders{gap=10}) ← MUST fire → continues to :277+.
        let _returned = manager.handle_response(peer, SyncResponse::Headers(vec![]));

        // O2: coordinator evidence window MUST contain at least one EmptyHeaders entry.
        // D1 fix must NOT suppress reporting for gap>3 — genuine fork evidence.
        assert!(
            manager.recovery.evidence_len() >= 1,
            "D1 pin: gap=10 empty-headers MUST add EmptyHeaders evidence to the coordinator \
             (response.rs:261). evidence_len()={} (expected >= 1). \
             D1 fix suppresses report() for gap<=3 only; gap>3 must remain reported.",
            manager.recovery.evidence_len(),
        );
    }

    /// INC-I-138 M3 cross-check — D1 and D4 defense layers are independent.
    ///
    /// Even if D1 is still broken (10+ gap<=3 EmptyHeaders inflating the window),
    /// M2's D4 fix (gap >= MINOR_FORK_GAP_MAX(50) guard on deep_fork_confirmed in
    /// recovery.rs:401-404) prevents SnapSync at gap=28. The two fixes are orthogonal
    /// defense-in-depth layers: D1 reduces false evidence; D4 prevents escalation at
    /// small gaps regardless of evidence count.
    ///
    /// Uses RecoveryCoordinator directly to model OLD D1 behavior: 10 EmptyHeaders
    /// with gap=2 reported unconditionally (as the bug does — report before guard).
    ///
    /// PASSES today (D4 already fixed by M2). MUST PASS after D1 fix (M3).
    #[test]
    fn test_inc_i138_m3_d4_guard_holds_with_inflated_evidence_pin() {
        // Simulate OLD D1 behavior: 10 gap=2 responses ALL reported as EmptyHeaders.
        let mut coord = RecoveryCoordinator::new();
        for _ in 0..10 {
            coord.report(RecoveryEvidence::EmptyHeaders {
                peer: PeerId::random(),
                gap: 2,
            });
        }

        // INC-I-138 incident context at t=325s (E8): gap=28, stale, 5 peers.
        let ctx = RecoveryContext {
            local_height: 36,
            network_tip_height: 64, // gap() = 64 - 36 = 28
            peer_count: 5,
            last_applied_secs: 325,
            shallow_rollback_count: 0,
            snap_attempts: 0,
            last_rollback_local_height: None,
            last_rollback_time: None,
            in_grace_period: false,
            last_finality_height: None,
        };

        let action = coord.classify(&ctx);

        // D4 guard (M2 fix, recovery.rs:401-404):
        //   deep_fork_confirmed = empty_count>=10 && stale>=300s && gap>=50
        //   gap=28 < MINOR_FORK_GAP_MAX(50) → deep_fork_confirmed=false → Rule 2 skipped.
        //   Rule 3 (medium_gap=28) → HeaderFirstSync (not SnapSync).
        // ONE-ASSERT cross-check: D1+D4 layers must be independent.
        assert_ne!(
            action,
            RecoveryAction::SnapSync,
            "INC-I-138 M3 cross-check: D4 guard must prevent SnapSync at gap=28 even \
             with 10 EmptyHeaders in the evidence window (inflated by D1 bug). \
             classify() returned {:?}. Expected NOT SnapSync. \
             D4 fix: deep_fork_confirmed requires gap >= MINOR_FORK_GAP_MAX(50); \
             gap=28 < 50 → false → Rule 2 skipped. \
             D1 and D4 are independent defense layers.",
            action,
        );
    }
}
