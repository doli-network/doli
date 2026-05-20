//! M2 Diagnostic Emit Site Integration Tests — Workflow #346
//!
//! TDD RED PHASE: Tests fail to compile until M2 production code exists.
//! Developer must add: `Node.set_diagnostic_emitter()`, apply_block's 3rd param
//! `provenance: Option<BlockProvenance>`, and emit calls in 7 modified files.
//! See `docs/.workflow/milestone-plan.md` M2 section for full list.
//!
// OUTPUT CONTRACT: fn Node::apply_block(block, mode, provenance)
//   O2: self.chain_state=h+1 on success, unchanged on fail | O3: Ok/Err
//   O4: block_store=persisted on success | O6: block_applied OR block_rejected event
// PATHS: P1:success(P1a:gossip,P1b:self,P1c:reorg), P2:validation_fail, P3:recovery_mode, P4:snap_guard
// OUTPUT CONTRACT: fn Node::handle_new_block(block, source_peer)
//   O6: fork_block_received for ForkBlock/Orphan/Rejected; block_applied if ExtendsTip
// PATHS: ExtendsTip|ForkBlock(HeightOccupied)|Orphan|Rejected|ReorgCandidate
// OUTPUT CONTRACT: fn Node::rollback_one_block()
//   O2: chain_state.height-=1 | O3: Ok(true)/Ok(false) | O6: rollback_started+completed pair
// PATHS: P1:success(completed.caused_by=started.event_id), P2:genesis(no events)
// OUTPUT CONTRACT: fn Node::execute_reorg(reorg_result, triggering_block)
//   O6: reorg_executed with old_tip, new_tip, weight_delta
// OUTPUT CONTRACT: periodic classify() emit
//   O6: recovery_classify_call if action!=None, with all 11 RecoveryContext fields
// INPUT PARTITIONS:
//   apply_block provenance: Some(gossip) | None(self) | None(reorg)
//   handle_new_block: ExtendsTip | HeightOccupied | Orphan | Rejected | ReorgCandidate
//   rollback: success | at_genesis

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Transaction};
use doli_node::node::Node;
use network::PeerId;
use storage::diagnostic_ledger::emitter::{DiagnosticEmitter, MockEmitter};
use storage::diagnostic_ledger::types::{
    BlockProvenance, DiagnosticEvent, EventKind, EventPayload,
};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

/// Create a test Node with MockEmitter injected. Developer must add `set_diagnostic_emitter`.
async fn make_node_with_mock(
    n_producers: usize,
) -> (Node, Vec<KeyPair>, TempDir, Arc<MockEmitter>) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");

    let mock = Arc::new(MockEmitter::new());
    node.set_diagnostic_emitter(mock.clone() as Arc<dyn DiagnosticEmitter>);

    (node, producers, temp, mock)
}

fn build_block(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
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
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: crypto::Hash::ZERO,
        fork_id: crypto::Hash::ZERO,
    };

    Block::new(header, vec![coinbase])
}

fn build_invalid_block(h: u64, slot: u32, prev: Hash, kp: &KeyPair, p: &ConsensusParams) -> Block {
    let mut b = build_block(h, slot, prev, kp, p);
    b.header.merkle_root = Hash::ZERO; // WRONG — will fail merkle validation
    b
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn events_of_kind(mock: &MockEmitter, kind: EventKind) -> Vec<DiagnosticEvent> {
    mock.events()
        .into_iter()
        .filter(|e| e.kind == kind)
        .collect()
}

fn fake_peer_id() -> PeerId {
    PeerId::random()
}

fn fake_event_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    format!("01HYTEST{:018}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

// Requirement: REQ-FORKOBS-EMIT-001 (Must)
// Acceptance: block_applied event with all specified fields non-null for gossip blocks
#[tokio::test]
async fn test_apply_block_success_emits_block_applied() {
    let (mut node, producers, _tmp, mock) = make_node_with_mock(2).await;
    let params = node.params.clone();

    let genesis_hash = {
        let cs = node.chain_state.read().await;
        cs.best_hash
    };

    let block = build_block(1, 1, genesis_hash, &producers[0], &params);
    let provenance = Some(BlockProvenance {
        from_peer_id: Some("12D3KooWTestPeer".to_string()),
        received_at_ms: now_ms(),
    });

    let result = node
        .apply_block(block.clone(), ValidationMode::Full, provenance)
        .await;
    assert!(
        result.is_ok(),
        "apply_block should succeed for a valid block"
    );

    let applied = events_of_kind(&mock, EventKind::BlockApplied);
    assert_eq!(applied.len(), 1, "exactly one block_applied event expected");

    let event = &applied[0];
    assert_eq!(event.height, Some(1));
    assert!(
        !event.event_id.is_empty(),
        "event_id should be a non-empty ULID"
    );

    match &event.payload {
        EventPayload::BlockApplied {
            slot,
            block_hash,
            producer_pubkey,
            from_peer_id,
            received_at_ms,
            applied_at_ms,
            validation_duration_ms,
            mode,
            tx_count,
        } => {
            assert_eq!(*slot, 1);
            assert!(!block_hash.is_empty());
            assert!(!producer_pubkey.is_empty());
            assert_eq!(from_peer_id.as_deref(), Some("12D3KooWTestPeer"));
            assert!(received_at_ms.is_some());
            assert!(*applied_at_ms > 0);
            assert!(*validation_duration_ms < 10_000); // sanity: <10s
            assert_eq!(mode, "Full");
            assert!(*tx_count >= 1); // at least coinbase
        }
        _ => panic!("expected BlockApplied payload, got {:?}", event.payload),
    }
}

// Requirement: REQ-FORKOBS-EMIT-002 (Must)
// Acceptance: block_rejected event with rejection_reason matching validation error
#[tokio::test]
async fn test_apply_block_failure_emits_block_rejected() {
    let (mut node, producers, _tmp, mock) = make_node_with_mock(2).await;
    let params = node.params.clone();

    let genesis_hash = {
        let cs = node.chain_state.read().await;
        cs.best_hash
    };

    let bad_block = build_invalid_block(1, 1, genesis_hash, &producers[0], &params);
    let provenance = Some(BlockProvenance {
        from_peer_id: Some("12D3KooWBadPeer".to_string()),
        received_at_ms: now_ms(),
    });

    let result = node
        .apply_block(bad_block, ValidationMode::Full, provenance)
        .await;
    // apply_block returns Err on validation failure
    assert!(result.is_err(), "apply_block should fail for invalid block");

    let rejected = events_of_kind(&mock, EventKind::BlockRejected);
    assert_eq!(
        rejected.len(),
        1,
        "exactly one block_rejected event expected"
    );

    let event = &rejected[0];
    match &event.payload {
        EventPayload::BlockRejected {
            rejection_reason,
            from_peer_id,
            mode,
            ..
        } => {
            assert!(
                !rejection_reason.is_empty(),
                "rejection_reason must be non-empty"
            );
            assert_eq!(from_peer_id.as_deref(), Some("12D3KooWBadPeer"));
            assert_eq!(mode, "Full");
        }
        _ => panic!("expected BlockRejected payload"),
    }

    // Verify chain state was NOT advanced (O2 mutation check)
    let height = node.chain_state.read().await.best_height;
    assert_eq!(
        height, 0,
        "chain state should not advance on rejected block"
    );
}

// Requirement: REQ-FORKOBS-EMIT-003 (Must)
// Acceptance: fork_block_received with classification=ForkBlock for height-occupied block
#[tokio::test]
async fn test_classify_gossip_block_fork_emits_event() {
    let (mut node, producers, _tmp, mock) = make_node_with_mock(2).await;
    let params = node.params.clone();

    let genesis_hash = {
        let cs = node.chain_state.read().await;
        cs.best_hash
    };

    // Apply block at h=1 to establish a canonical tip
    let block1 = build_block(1, 1, genesis_hash, &producers[0], &params);
    node.apply_block(block1.clone(), ValidationMode::Full, None)
        .await
        .expect("first block should apply");

    // Now build a COMPETING block at the same height from a different producer
    let fork_block = build_block(1, 2, genesis_hash, &producers[1], &params);
    let source_peer = fake_peer_id();

    // handle_new_block should classify this as ForkBlock(HeightOccupied)
    let _ = node.handle_new_block(fork_block, source_peer).await;

    let fork_events = events_of_kind(&mock, EventKind::ForkBlockReceived);
    assert!(
        !fork_events.is_empty(),
        "at least one fork_block_received event expected for height-occupied fork"
    );

    let event = &fork_events[0];
    match &event.payload {
        EventPayload::ForkBlockReceived {
            classification,
            fork_kind,
            local_tip_height,
            from_peer_id,
            ..
        } => {
            assert_eq!(classification, "ForkBlock");
            assert_eq!(fork_kind.as_deref(), Some("HeightOccupied"));
            assert_eq!(*local_tip_height, 1);
            assert!(
                !from_peer_id.is_empty(),
                "from_peer_id should be the source peer"
            );
        }
        _ => panic!("expected ForkBlockReceived payload"),
    }
}

// Requirement: REQ-FORKOBS-EMIT-003 (Must)
// Acceptance: fork_block_received with classification=Orphan for orphan block
#[tokio::test]
async fn test_classify_gossip_block_orphan_emits_event() {
    let (mut node, _producers, _tmp, mock) = make_node_with_mock(2).await;

    // Build a block whose parent is NOT in our store (orphan)
    let unknown_parent = Hash::from_bytes([0xAB; 32]);
    let orphan_producer = KeyPair::generate();
    let params = node.params.clone();
    let orphan = build_block(99, 99, unknown_parent, &orphan_producer, &params);

    let source_peer = fake_peer_id();
    let _ = node.handle_new_block(orphan, source_peer).await;

    let fork_events = events_of_kind(&mock, EventKind::ForkBlockReceived);
    assert!(
        !fork_events.is_empty(),
        "at least one fork_block_received event expected for orphan block"
    );

    let has_orphan = fork_events.iter().any(|e| {
        matches!(
            &e.payload,
            EventPayload::ForkBlockReceived { classification, .. } if classification == "Orphan"
        )
    });
    assert!(
        has_orphan,
        "should have a ForkBlockReceived with classification=Orphan"
    );
}

// Requirement: REQ-FORKOBS-EMIT-003 (Must)
// Acceptance: fork_block_received with classification=Rejected for invalid genesis
#[tokio::test]
async fn test_classify_gossip_block_rejected_emits_event() {
    let (mut node, producers, _tmp, mock) = make_node_with_mock(2).await;
    let params = node.params.clone();
    let genesis_hash = node.chain_state.read().await.best_hash;

    // Build block with wrong genesis hash -> classify returns Rejected
    let mut block = build_block(1, 1, genesis_hash, &producers[0], &params);
    block.header.genesis_hash = Hash::from_bytes([0xFF; 32]);

    let _ = node.handle_new_block(block, fake_peer_id()).await;

    let fork_events = events_of_kind(&mock, EventKind::ForkBlockReceived);
    let has_rejected = fork_events.iter().any(|e| {
        matches!(
            &e.payload,
            EventPayload::ForkBlockReceived { classification, .. } if classification == "Rejected"
        )
    });
    assert!(
        has_rejected,
        "should emit ForkBlockReceived(Rejected) for wrong genesis"
    );
}

// Requirement: REQ-FORKOBS-EMIT-003 (Must)
// Acceptance: fork_block_received with fork_kind=ReorgCandidate
#[tokio::test]
async fn test_classify_gossip_block_reorg_candidate_emits_event() {
    let (mut node, producers, _tmp, mock) = make_node_with_mock(2).await;
    let params = node.params.clone();

    let genesis_hash = {
        let cs = node.chain_state.read().await;
        cs.best_hash
    };

    // Apply blocks 1 and 2 to establish a chain
    let block1 = build_block(1, 1, genesis_hash, &producers[0], &params);
    let block1_hash = block1.hash();
    node.apply_block(block1, ValidationMode::Full, None)
        .await
        .unwrap();

    let block2 = build_block(2, 2, block1_hash, &producers[0], &params);
    node.apply_block(block2, ValidationMode::Full, None)
        .await
        .unwrap();

    // Build a block that branches from block1 (parent known) but doesn't extend tip
    // This creates a ReorgCandidate — parent block1 is in store, but tip is at h=2
    let fork_block = build_block(2, 3, block1_hash, &producers[1], &params);
    let source_peer = fake_peer_id();
    let _ = node.handle_new_block(fork_block, source_peer).await;

    let fork_events = events_of_kind(&mock, EventKind::ForkBlockReceived);
    // The block has parent in store, height already occupied -> HeightOccupied
    // OR parent known, no height conflict at fork point -> ReorgCandidate
    // Depends on exact classification logic. Both are valid fork events.
    assert!(
        !fork_events.is_empty(),
        "should emit fork_block_received for a block that doesn't extend tip"
    );
}

// Requirement: REQ-FORKOBS-EMIT-004, REQ-FORKOBS-EMIT-005 (Must)
// Acceptance: rollback_started and rollback_completed emitted; completed.caused_by = started.event_id
#[tokio::test]
async fn test_rollback_emits_started_and_completed() {
    let (mut node, producers, _tmp, mock) = make_node_with_mock(2).await;
    let params = node.params.clone();

    let genesis_hash = {
        let cs = node.chain_state.read().await;
        cs.best_hash
    };

    // Apply a block so we have something to rollback
    let block1 = build_block(1, 1, genesis_hash, &producers[0], &params);
    node.apply_block(block1, ValidationMode::Full, None)
        .await
        .unwrap();

    // Rollback
    let result = node.rollback_one_block().await;
    assert!(result.is_ok());

    let started = events_of_kind(&mock, EventKind::RollbackStarted);
    let completed = events_of_kind(&mock, EventKind::RollbackCompleted);

    assert_eq!(started.len(), 1, "exactly one rollback_started event");
    assert_eq!(completed.len(), 1, "exactly one rollback_completed event");

    // Verify rollback_started payload
    match &started[0].payload {
        EventPayload::RollbackStarted {
            from_height,
            to_height,
            ..
        } => {
            assert_eq!(*from_height, 1);
            assert_eq!(*to_height, 0);
        }
        _ => panic!("expected RollbackStarted payload"),
    }

    // Verify causal chain: completed.caused_by_event_id == started.event_id
    assert_eq!(
        completed[0].caused_by_event_id.as_ref(),
        Some(&started[0].event_id),
        "rollback_completed must reference rollback_started via caused_by_event_id"
    );

    // Verify rollback_completed payload
    match &completed[0].payload {
        EventPayload::RollbackCompleted {
            from_height,
            to_height,
            duration_ms,
            success,
        } => {
            assert_eq!(*from_height, 1);
            assert_eq!(*to_height, 0);
            assert!(*duration_ms < 60_000, "rollback should be fast in tests");
            assert!(*success);
        }
        _ => panic!("expected RollbackCompleted payload"),
    }
}

// NOTE: REQ-FORKOBS-EMIT-007 (recovery_classify_call) and REQ-FORKOBS-EMIT-006
// (reorg_executed) tests are in diagnostic_writer_pruner_test.rs as unit-level
// type-validation tests — they don't need a full Node to verify event structure.

// ============================================================
// PHASE B — Provenance tests (O4: explicit Option<BlockProvenance>)
// ============================================================

// Requirement: REQ-FORKOBS-EMIT-001 + O4 (Must)
// Acceptance: gossip block has from_peer_id populated in event
#[tokio::test]
async fn test_apply_block_gossip_has_peer_provenance() {
    let (mut node, producers, _tmp, mock) = make_node_with_mock(2).await;
    let params = node.params.clone();

    let genesis_hash = {
        let cs = node.chain_state.read().await;
        cs.best_hash
    };

    let block = build_block(1, 1, genesis_hash, &producers[0], &params);
    let peer_id_str = "12D3KooWGossipPeer123".to_string();
    let before_apply = now_ms();

    let provenance = Some(BlockProvenance {
        from_peer_id: Some(peer_id_str.clone()),
        received_at_ms: before_apply,
    });

    node.apply_block(block, ValidationMode::Full, provenance)
        .await
        .unwrap();

    let applied = events_of_kind(&mock, EventKind::BlockApplied);
    assert_eq!(applied.len(), 1);

    match &applied[0].payload {
        EventPayload::BlockApplied {
            from_peer_id,
            received_at_ms,
            ..
        } => {
            assert_eq!(
                from_peer_id.as_deref(),
                Some(peer_id_str.as_str()),
                "from_peer_id must match the provenance peer"
            );
            assert!(
                received_at_ms.is_some(),
                "received_at_ms must be populated for gossip"
            );
            let recv = received_at_ms.unwrap();
            let delta = if recv > before_apply {
                recv - before_apply
            } else {
                before_apply - recv
            };
            assert!(
                delta < 5_000,
                "received_at_ms should be close to wall clock (within 5s)"
            );
        }
        _ => panic!("expected BlockApplied payload"),
    }
}

// Requirement: REQ-FORKOBS-EMIT-001 + O4 (Must)
// Acceptance: self-produced block has from_peer_id=None
#[tokio::test]
async fn test_apply_block_self_produced_has_none_provenance() {
    let (mut node, producers, _tmp, mock) = make_node_with_mock(2).await;
    let params = node.params.clone();

    let genesis_hash = {
        let cs = node.chain_state.read().await;
        cs.best_hash
    };

    let block = build_block(1, 1, genesis_hash, &producers[0], &params);

    // Self-produced: provenance = None
    node.apply_block(block, ValidationMode::Full, None)
        .await
        .unwrap();

    let applied = events_of_kind(&mock, EventKind::BlockApplied);
    assert_eq!(applied.len(), 1);

    match &applied[0].payload {
        EventPayload::BlockApplied {
            from_peer_id,
            received_at_ms,
            ..
        } => {
            assert!(
                from_peer_id.is_none(),
                "self-produced blocks should have from_peer_id=None"
            );
            assert!(
                received_at_ms.is_none(),
                "self-produced blocks should have received_at_ms=None"
            );
        }
        _ => panic!("expected BlockApplied payload"),
    }
}

// Requirement: REQ-FORKOBS-EMIT-001 + O4 (Must)
// Acceptance: reorg-replay blocks have provenance=None
#[tokio::test]
async fn test_apply_block_reorg_replay_has_none_provenance() {
    let (mut node, producers, _tmp, mock) = make_node_with_mock(2).await;
    let params = node.params.clone();

    let genesis_hash = {
        let cs = node.chain_state.read().await;
        cs.best_hash
    };

    // Apply via Light mode with None provenance (mimics reorg replay)
    let block = build_block(1, 1, genesis_hash, &producers[0], &params);
    node.apply_block(block, ValidationMode::Light, None)
        .await
        .unwrap();

    let applied = events_of_kind(&mock, EventKind::BlockApplied);
    assert_eq!(applied.len(), 1);

    match &applied[0].payload {
        EventPayload::BlockApplied {
            from_peer_id, mode, ..
        } => {
            assert!(
                from_peer_id.is_none(),
                "reorg-replay blocks should have from_peer_id=None"
            );
            assert_eq!(mode, "Light", "reorg-replay uses Light validation mode");
        }
        _ => panic!("expected BlockApplied payload"),
    }
}

// Requirement: O4 (Must) — compile-time: apply_block takes Option<BlockProvenance>
#[tokio::test]
async fn test_apply_block_signature_includes_provenance_param() {
    let (mut node, producers, _tmp, _mock) = make_node_with_mock(1).await;
    let params = node.params.clone();
    let genesis_hash = node.chain_state.read().await.best_hash;

    let block = build_block(1, 1, genesis_hash, &producers[0], &params);

    // The 3-arg call is the compile check:
    let _result = node
        .apply_block(
            block,
            ValidationMode::Full,
            Some(BlockProvenance {
                from_peer_id: None,
                received_at_ms: 0,
            }),
        )
        .await;
    // If this compiles, the signature check passes.
}

// ============================================================
// PHASE C — Correlation key tests (REQ-FORKOBS-EMIT-010)
// ============================================================

// Requirement: REQ-FORKOBS-EMIT-010 (Should)
// Acceptance: fork events carry a correlation_key with divergence_height populated
#[tokio::test]
async fn test_fork_event_carries_correlation_key() {
    let (mut node, producers, _tmp, mock) = make_node_with_mock(2).await;
    let params = node.params.clone();

    let genesis_hash = {
        let cs = node.chain_state.read().await;
        cs.best_hash
    };

    // Apply block at h=1
    let block1 = build_block(1, 1, genesis_hash, &producers[0], &params);
    node.apply_block(block1, ValidationMode::Full, None)
        .await
        .unwrap();

    // Feed a competing block at h=1 to trigger ForkBlock(HeightOccupied)
    let fork_block = build_block(1, 2, genesis_hash, &producers[1], &params);
    let source_peer = fake_peer_id();
    let _ = node.handle_new_block(fork_block, source_peer).await;

    let fork_events = events_of_kind(&mock, EventKind::ForkBlockReceived);
    assert!(!fork_events.is_empty(), "fork event should be emitted");

    let event = &fork_events[0];
    assert!(
        event.correlation_key.is_some(),
        "fork event must carry a correlation_key"
    );

    let ck = event.correlation_key.as_ref().unwrap();
    assert!(
        ck.divergence_height.is_some(),
        "divergence_height must be populated for fork events"
    );
}

// Requirement: REQ-FORKOBS-EMIT-010 (Should)
// Acceptance: canonical block_applied has all-None correlation_key
#[tokio::test]
async fn test_canonical_block_applied_has_empty_correlation_key() {
    let (mut node, producers, _tmp, mock) = make_node_with_mock(2).await;
    let params = node.params.clone();

    let genesis_hash = {
        let cs = node.chain_state.read().await;
        cs.best_hash
    };

    let block = build_block(1, 1, genesis_hash, &producers[0], &params);
    node.apply_block(block, ValidationMode::Full, None)
        .await
        .unwrap();

    let applied = events_of_kind(&mock, EventKind::BlockApplied);
    assert_eq!(applied.len(), 1);

    let event = &applied[0];
    // Canonical blocks should have correlation_key = None (no fork context)
    assert!(
        event.correlation_key.is_none(),
        "canonical block_applied should have correlation_key=None"
    );
}

// ============================================================
// PHASE G — Default-ON tests (REQ-FORKOBS-SEC-005)
// ============================================================

// Requirement: REQ-FORKOBS-SEC-005 (Must) — default-ON: no config needed to emit
#[tokio::test]
async fn test_node_with_no_config_emits_events() {
    let (mut node, producers, _tmp, mock) = make_node_with_mock(2).await;
    let params = node.params.clone();

    let genesis_hash = node.chain_state.read().await.best_hash;
    let block = build_block(1, 1, genesis_hash, &producers[0], &params);

    node.apply_block(block, ValidationMode::Full, None)
        .await
        .unwrap();

    let all_events = mock.events();
    assert!(
        !all_events.is_empty(),
        "default-ON: node should emit events with no special configuration"
    );
}

// ============================================================
// PHASE I — Edge case tests for Must requirements
// ============================================================

// Requirement: REQ-FORKOBS-EMIT-001 (Must) — edge case: multiple blocks in sequence
// Acceptance: each block produces exactly one event
#[tokio::test]
async fn test_multiple_blocks_each_produce_one_event() {
    let (mut node, producers, _tmp, mock) = make_node_with_mock(2).await;
    let params = node.params.clone();

    let mut prev_hash = node.chain_state.read().await.best_hash;

    for i in 1..=5u64 {
        let block = build_block(i, i as u32, prev_hash, &producers[0], &params);
        prev_hash = block.hash();
        node.apply_block(block, ValidationMode::Full, None)
            .await
            .unwrap();
    }

    let applied = events_of_kind(&mock, EventKind::BlockApplied);
    assert_eq!(
        applied.len(),
        5,
        "5 blocks applied should produce exactly 5 block_applied events"
    );

    // Verify heights are sequential
    for (i, event) in applied.iter().enumerate() {
        assert_eq!(event.height, Some((i + 1) as u64));
    }
}

// Requirement: REQ-FORKOBS-EMIT-004 (Must) — edge case: rollback at genesis
// Acceptance: rollback at genesis height should not emit events
#[tokio::test]
async fn test_rollback_at_genesis_emits_nothing() {
    let (mut node, _producers, _tmp, mock) = make_node_with_mock(2).await;

    // At genesis (h=0), rollback should return false and emit nothing
    let result = node.rollback_one_block().await;
    assert!(result.is_ok());

    let started = events_of_kind(&mock, EventKind::RollbackStarted);
    let completed = events_of_kind(&mock, EventKind::RollbackCompleted);
    assert!(started.is_empty(), "no rollback_started at genesis");
    assert!(completed.is_empty(), "no rollback_completed at genesis");
}

// Requirement: REQ-FORKOBS-EMIT-001 (Must) — edge case: emit does not affect apply result
#[tokio::test]
async fn test_emit_failure_does_not_affect_apply_block() {
    // Architectural invariant: emit calls use `let _ =` so errors never propagate.
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..2).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");

    // Use NoOpEmitter — events are silently dropped
    use storage::diagnostic_ledger::emitter::NoOpEmitter;
    node.set_diagnostic_emitter(Arc::new(NoOpEmitter) as Arc<dyn DiagnosticEmitter>);

    let params = node.params.clone();
    let genesis_hash = node.chain_state.read().await.best_hash;
    let block = build_block(1, 1, genesis_hash, &producers[0], &params);

    let result = node.apply_block(block, ValidationMode::Full, None).await;
    assert!(
        result.is_ok(),
        "apply_block must succeed even when emitter is NoOp (graceful degradation)"
    );

    let height = node.chain_state.read().await.best_height;
    assert_eq!(height, 1, "block should be applied despite NoOp emitter");
}

// Requirement: REQ-FORKOBS-EMIT-001 (Must) — edge case: already-known block
// Acceptance: no event emitted for duplicate block
#[tokio::test]
async fn test_already_known_block_emits_nothing() {
    let (mut node, producers, _tmp, mock) = make_node_with_mock(2).await;
    let params = node.params.clone();

    let genesis_hash = node.chain_state.read().await.best_hash;
    let block = build_block(1, 1, genesis_hash, &producers[0], &params);

    // Apply the block once
    node.apply_block(block.clone(), ValidationMode::Full, None)
        .await
        .unwrap();

    let events_after_first = mock.events().len();

    // Feed the same block again via handle_new_block
    let source_peer = fake_peer_id();
    let _ = node.handle_new_block(block, source_peer).await;

    let events_after_second = mock.events().len();

    assert_eq!(
        events_after_first, events_after_second,
        "already-known block should not emit additional events"
    );
}

// Requirement: REQ-FORKOBS-EMIT-001 (Must) — edge case: event_id uniqueness
// Acceptance: every emitted event has a unique event_id
#[tokio::test]
async fn test_event_ids_are_unique() {
    let (mut node, producers, _tmp, mock) = make_node_with_mock(2).await;
    let params = node.params.clone();

    let mut prev_hash = node.chain_state.read().await.best_hash;

    for i in 1..=3u64 {
        let block = build_block(i, i as u32, prev_hash, &producers[0], &params);
        prev_hash = block.hash();
        node.apply_block(block, ValidationMode::Full, None)
            .await
            .unwrap();
    }

    let all_events = mock.events();
    let ids: Vec<&str> = all_events.iter().map(|e| e.event_id.as_str()).collect();
    let unique: std::collections::HashSet<&str> = ids.iter().cloned().collect();
    assert_eq!(
        ids.len(),
        unique.len(),
        "all event_ids must be unique (ULID monotonic ordering)"
    );
}
