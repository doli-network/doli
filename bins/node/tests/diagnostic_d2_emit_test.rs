//! M4 (D2) Diagnostic Emit Tests — INC-I-090 Workflow #376
//!
//! TDD RED PHASE: Tests verify that SnapSync* and ChainBreakDetected
//! events are emitted at the appropriate code sites. Currently all four
//! EventKind variants have ZERO production emit sites, so these tests
//! MUST FAIL until the developer wires the emit calls.
//!
//! Evidence: [E6] types.rs:51-57 — 4 EventKinds defined but zero emit sites
//!           [E13] Probe 2 on N3 mainnet: snap_events=0, chain_breaks=0
//!
// OUTPUT CONTRACT: fn Node::apply_snap_snapshot(snapshot: VerifiedSnapshot) -> Result<()>
//   Outputs:
//     O1: chain_state updated to snapshot height on success
//     O2: SnapSyncAttempted event emitted at entry
//     O3: SnapSyncCompleted event emitted on success (with result + duration)
//     O4: SnapSyncFailed event emitted on failure (with error + duration)
//   Paths:
//     P1: success — valid snapshot, state root matches, deserialization ok
//     P2: state_root_mismatch — computed root != envelope root → SnapSyncFailed
//     P3: deserialization_fail — chain_state/utxo/producer_set corrupt → SnapSyncFailed
//     P4: envelope_mismatch — hash/height in envelope != deserialized state → SnapSyncFailed
//     P5: recovery_mode — node in recovery → early return (no snap events)
//
// OUTPUT CONTRACT: ChainBreakDetected emit
//   Outputs:
//     O5: ChainBreakDetected event with expected_prev_hash, actual_prev_hash, header_slot,
//         valid_so_far_count, from_peer_id
//   Paths:
//     P6: header chain break — header.prev_hash != expected → ChainBreakDetected emitted
//     P7: valid chain — all headers link correctly → no ChainBreakDetected
//
// INPUT PARTITIONS:
//   P1: valid snapshot with matching state root → SnapSyncAttempted + SnapSyncCompleted
//   P2: snapshot with wrong state root → SnapSyncAttempted + SnapSyncFailed("root mismatch")
//   P5: recovery mode active → no snap events emitted
//   P6: header with mismatched prev_hash → ChainBreakDetected with correct fields
//   P7: headers with correct chain linkage → no ChainBreakDetected

use std::sync::Arc;

use crypto::{Hash, KeyPair};
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Transaction};
use doli_node::node::Node;
use storage::diagnostic_ledger::emitter::{DiagnosticEmitter, MockEmitter};
use storage::diagnostic_ledger::types::{EventKind, EventPayload};

/// Create a test Node with MockEmitter injected.
async fn make_node_with_mock(
    n_producers: usize,
) -> (Node, Vec<KeyPair>, tempfile::TempDir, Arc<MockEmitter>) {
    let temp = tempfile::TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");

    let mock = Arc::new(MockEmitter::new());
    node.set_diagnostic_emitter(mock.clone() as Arc<dyn DiagnosticEmitter>);

    (node, producers, temp, mock)
}

fn events_of_kind(
    mock: &MockEmitter,
    kind: EventKind,
) -> Vec<storage::diagnostic_ledger::types::DiagnosticEvent> {
    mock.events()
        .into_iter()
        .filter(|e| e.kind == kind)
        .collect()
}

// ============================================================
// SUITE A — SnapSync emit tests
// ============================================================

/// D2-SNAP-001: SnapSyncAttempted + SnapSyncCompleted on successful snap sync.
///
/// A valid VerifiedSnapshot applied via apply_snap_snapshot() must emit:
///   1. SnapSyncAttempted (at entry)
///   2. SnapSyncCompleted (at success exit)
///
/// Currently FAILS because fork_recovery.rs has zero diagnostic calls.
#[tokio::test]
async fn test_snap_sync_success_emits_attempted_and_completed() {
    let (mut node, producers, _tmp, mock) = make_node_with_mock(2).await;
    let params = node.params.clone();

    // Build a valid chain state to snapshot from
    let genesis_hash = node.chain_state.read().await.best_hash;
    let block1 = build_test_block(1, 1, genesis_hash, &producers[0], &params);
    node.apply_block(block1.clone(), ValidationMode::Full, None)
        .await
        .unwrap();

    // Capture current state as a "snapshot" (serialize the real state)
    let (cs_bytes, utxo_bytes, ps_bytes, state_root, best_hash, best_height) = {
        let cs = node.chain_state.read().await;
        let utxo = node.utxo_set.read().await;
        let ps = node.producer_set.read().await;
        let cs_bytes = bincode::serialize(&*cs).unwrap();
        let utxo_bytes = utxo.serialize_canonical();
        let ps_bytes = bincode::serialize(&*ps).unwrap();
        let root = storage::compute_state_root(&cs, &utxo, &ps).unwrap();
        (
            cs_bytes,
            utxo_bytes,
            ps_bytes,
            root,
            cs.best_hash,
            cs.best_height,
        )
    };

    // Construct a VerifiedSnapshot matching the real state
    let snapshot = network::VerifiedSnapshot {
        block_height: best_height,
        block_hash: best_hash,
        state_root,
        chain_state: cs_bytes,
        utxo_set: utxo_bytes,
        producer_set: ps_bytes,
        block_header_bytes: None,
        epoch_state_bytes: None,
        epoch_bond_snapshot_bytes: None,
        epoch_accumulators_bytes: None,
    };

    // Clear events from the block apply above
    let pre_snap_count = mock.events().len();

    // Apply the snapshot
    node.apply_snap_snapshot(snapshot).await.unwrap();

    // Filter only post-snap events
    let all_events = mock.events();
    let snap_events: Vec<_> = all_events[pre_snap_count..].to_vec();

    let attempted = snap_events
        .iter()
        .filter(|e| e.kind == EventKind::SnapSyncAttempted)
        .collect::<Vec<_>>();
    let completed = snap_events
        .iter()
        .filter(|e| e.kind == EventKind::SnapSyncCompleted)
        .collect::<Vec<_>>();

    assert_eq!(
        attempted.len(),
        1,
        "D2-SNAP-001: exactly one SnapSyncAttempted event expected on successful snap sync"
    );
    assert_eq!(
        completed.len(),
        1,
        "D2-SNAP-001: exactly one SnapSyncCompleted event expected on successful snap sync"
    );

    // Verify SnapSyncAttempted payload
    match &attempted[0].payload {
        EventPayload::SnapSyncAttempted {
            local_height,
            target_height,
            source_peer_id,
        } => {
            assert!(*target_height > 0, "target_height should be non-zero");
            assert!(
                !source_peer_id.is_empty(),
                "source_peer_id should be populated"
            );
            // local_height should be the pre-snap height
            let _ = local_height; // compile check — field exists
        }
        _ => panic!(
            "expected SnapSyncAttempted payload, got {:?}",
            attempted[0].payload
        ),
    }

    // Verify SnapSyncCompleted payload
    match &completed[0].payload {
        EventPayload::SnapSyncCompleted {
            result,
            duration_ms,
        } => {
            assert!(
                result.contains("success") || result.contains("ok") || result.contains("applied"),
                "result should indicate success, got: {}",
                result
            );
            assert!(
                *duration_ms < 60_000,
                "duration_ms should be reasonable in tests"
            );
        }
        _ => panic!(
            "expected SnapSyncCompleted payload, got {:?}",
            completed[0].payload
        ),
    }
}

/// D2-SNAP-002: SnapSyncAttempted + SnapSyncFailed when state root mismatches.
///
/// When compute_state_root() != envelope state_root, snap sync rejects the
/// snapshot. Must emit SnapSyncAttempted first, then SnapSyncFailed.
#[tokio::test]
async fn test_snap_sync_root_mismatch_emits_attempted_and_failed() {
    let (mut node, _producers, _tmp, mock) = make_node_with_mock(2).await;

    // Build a snapshot with all-zero state (will fail deserialization or root check)
    let snapshot = network::VerifiedSnapshot {
        block_height: 100,
        block_hash: Hash::from_bytes([0x01; 32]),
        state_root: Hash::from_bytes([0x99; 32]), // wrong root
        chain_state: vec![0u8; 64],               // garbage
        utxo_set: vec![0u8; 64],
        producer_set: vec![0u8; 64],
        block_header_bytes: None,
        epoch_state_bytes: None,
        epoch_bond_snapshot_bytes: None,
        epoch_accumulators_bytes: None,
    };

    // apply_snap_snapshot returns Ok(()) even on rejection (logs error, fallbacks)
    let _ = node.apply_snap_snapshot(snapshot).await;

    let attempted = events_of_kind(&mock, EventKind::SnapSyncAttempted);
    let failed = events_of_kind(&mock, EventKind::SnapSyncFailed);

    assert_eq!(
        attempted.len(),
        1,
        "D2-SNAP-002: SnapSyncAttempted must be emitted even when snap sync will fail"
    );
    assert!(
        !failed.is_empty(),
        "D2-SNAP-002: SnapSyncFailed must be emitted when state root mismatches or deserialization fails"
    );

    // Verify SnapSyncFailed has a reason
    match &failed[0].payload {
        EventPayload::SnapSyncFailed { error, duration_ms } => {
            assert!(!error.is_empty(), "error must describe the failure reason");
            let _ = duration_ms; // compile check
        }
        _ => panic!(
            "expected SnapSyncFailed payload, got {:?}",
            failed[0].payload
        ),
    }
}

/// D2-SNAP-003: No snap events when node is in recovery mode.
///
/// The recovery mode gate at the top of apply_snap_snapshot() returns early.
/// No SnapSync* events should be emitted.
#[tokio::test]
async fn test_snap_sync_recovery_mode_emits_nothing() {
    let (mut node, _producers, _tmp, mock) = make_node_with_mock(2).await;

    // Activate recovery mode
    node.recovery_mode
        .store(true, std::sync::atomic::Ordering::Relaxed);

    let snapshot = network::VerifiedSnapshot {
        block_height: 100,
        block_hash: Hash::from_bytes([0x01; 32]),
        state_root: Hash::from_bytes([0x02; 32]),
        chain_state: vec![],
        utxo_set: vec![],
        producer_set: vec![],
        block_header_bytes: None,
        epoch_state_bytes: None,
        epoch_bond_snapshot_bytes: None,
        epoch_accumulators_bytes: None,
    };

    let _ = node.apply_snap_snapshot(snapshot).await;

    let attempted = events_of_kind(&mock, EventKind::SnapSyncAttempted);
    let completed = events_of_kind(&mock, EventKind::SnapSyncCompleted);
    let failed = events_of_kind(&mock, EventKind::SnapSyncFailed);

    assert!(
        attempted.is_empty(),
        "D2-SNAP-003: no SnapSyncAttempted in recovery mode"
    );
    assert!(
        completed.is_empty(),
        "D2-SNAP-003: no SnapSyncCompleted in recovery mode"
    );
    assert!(
        failed.is_empty(),
        "D2-SNAP-003: no SnapSyncFailed in recovery mode"
    );
}

// ============================================================
// SUITE B — ChainBreakDetected emit tests
// ============================================================

/// D2-CHAIN-001: ChainBreakDetected emitted when header chain breaks.
///
/// When process_headers encounters a header whose prev_hash does not match
/// the expected hash, a ChainBreakDetected event must eventually be emitted
/// by the node layer (via the SyncManager drain pattern).
///
/// Currently FAILS because zero emit sites exist for ChainBreakDetected.
#[tokio::test]
async fn test_chain_break_detection_emits_event() {
    let (mut node, producers, _tmp, mock) = make_node_with_mock(2).await;
    let params = node.params.clone();

    // Apply a block so we have a non-genesis tip
    let genesis_hash = node.chain_state.read().await.best_hash;
    let block1 = build_test_block(1, 1, genesis_hash, &producers[0], &params);
    node.apply_block(block1.clone(), ValidationMode::Full, None)
        .await
        .unwrap();

    let local_tip_hash = node.chain_state.read().await.best_hash;

    // Simulate a chain break by feeding headers through the sync manager
    // where the first header's prev_hash doesn't match our tip
    let wrong_parent = Hash::from_bytes([0xDE; 32]);
    let break_header = make_header(wrong_parent, 2);

    // Feed the mismatched header through the sync manager
    {
        let mut sync = node.sync_manager.write().await;
        // Add a peer so the break can be attributed
        let fake_peer = network::PeerId::random();
        let peer_hash = Hash::from_bytes([0x01; 32]);
        sync.add_peer(fake_peer, 10, peer_hash, 10);
        sync.update_network_tip_height(10);

        // Feed headers that will break the chain
        sync.feed_headers_for_chain_break_detection(fake_peer, &[break_header], local_tip_hash);
    }

    // Run periodic tasks to drain chain break events
    let _ = node.run_periodic_tasks().await;

    let chain_breaks = events_of_kind(&mock, EventKind::ChainBreakDetected);
    assert!(
        !chain_breaks.is_empty(),
        "D2-CHAIN-001: at least one ChainBreakDetected event expected when header chain breaks"
    );

    // Verify payload fields
    match &chain_breaks[0].payload {
        EventPayload::ChainBreakDetected {
            expected_prev_hash,
            actual_prev_hash,
            header_slot,
            valid_so_far_count,
            from_peer_id,
        } => {
            assert_ne!(
                expected_prev_hash, actual_prev_hash,
                "expected and actual prev_hash must differ (that's the break)"
            );
            assert!(*header_slot > 0, "header_slot should be non-zero");
            let _ = valid_so_far_count;
            assert!(
                !from_peer_id.is_empty(),
                "from_peer_id should identify the source peer"
            );
        }
        _ => panic!(
            "expected ChainBreakDetected payload, got {:?}",
            chain_breaks[0].payload
        ),
    }
}

// ============================================================
// Helpers
// ============================================================

fn build_test_block(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &doli_core::consensus::ConsensusParams,
) -> Block {
    let reward = params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase));
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash,
        timestamp,
        slot,
        producer: *producer.public_key(),
        vdf_output: vdf::VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: crypto::Hash::ZERO,
        fork_id: crypto::Hash::ZERO,
    };

    Block::new(header, vec![coinbase])
}

fn make_header(prev_hash: Hash, slot: u32) -> doli_core::BlockHeader {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    doli_core::BlockHeader {
        version: 2,
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
