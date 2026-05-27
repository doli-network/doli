// TDD RED PHASE -- Workflow #346, Milestone M3
// All tests FAIL because production code (classifier.rs) does not exist yet.
//
// OUTPUT CONTRACT: fn classify(events: &[DiagnosticEvent]) -> Classification
//   O3: return -- Classification { fork_type, confidence, evidence_event_ids,
//                                   recommended_action, recommended_action_args }
//   No O1 (no mutable params), no O2 (no self), no O4 (no I/O), no O5/O6 (pure fn)
//
// PATHS:
//   P1: Rule (a) -- ProducerEquivocation
//   P2: Rule (b) -- EpochBoundaryInvalid
//   P3: Rule (c) -- RollbackLoop (>3 rollbacks in 60s)
//   P4: Rule (c) boundary -- 3 rollbacks NOT a loop
//   P5: Rule (d) -- PostSnapDeadTip (within 300s)
//   P6: Rule (d) boundary -- outside 300s window, NOT PostSnapDeadTip
//   P7: Rule (e) -- TipRaceHighLatency (validation_duration > 2000ms)
//   P8: Rule (f) -- TipRaceNatural (low latency, no other signals)
//   P9: Rule (f) violation -- low latency BUT other signals -> NOT TipRaceNatural
//   P10: Rule (g) -- Unknown (no rules match)
//   P11: First-match-wins priority (a > e)
//   P12: Pure function determinism
//   P13: Empty input -> Unknown
//   P14: Confidence bounded [0.0, 1.0] (cross-cutting)
//   P15: Evidence non-empty for named variants (cross-cutting)
//   P16: Recommended action per named variant (cross-cutting)
//   P17: Unknown carries all evidence event IDs (REQ-FORKOBS-RETRO-003)
//   P18: (M6) RecoveryClassifyCall with divergence_height-only correlation_key
//        groups with ForkBlockReceived that has divergence_height+fork_hash
//
// INPUT PARTITIONS:
//   P1a: 2 BlockApplied same height same producer, different block_hash
//   P2a: BlockRejected at epoch boundary, reason contains "EpochReward"
//   P3a: 4 RollbackStarted within 60s
//   P4a: 3 RollbackStarted within 60s (boundary)
//   P5a: SnapSyncCompleted at T, ForkBlockReceived at T+100s
//   P6a: SnapSyncCompleted at T, ForkBlockReceived at T+400s
//   P7a: ForkBlockReceived with validation_duration_ms=3000
//   P8a: ForkBlockReceived with validation_duration_ms=100, no other signals
//   P9a: ForkBlockReceived(100ms) + RecoveryClassifyCall with action in same corr_key group
//   P10a: events that match no rule (e.g., empty or WriterHeartbeat only)
//   P11a: equivocation + high-latency fork together
//   P12a: same input twice -> same output
//   P13a: empty event slice
//   P18a: (M6) ForkBlockReceived with divergence_height=H + fork_hash=Some("abc")
//         + RecoveryClassifyCall with divergence_height=H + fork_hash=None
//         -> has_other_signals returns true (divergence_height match is sufficient)
//
// MATRIX: 1 output (O3: return) x 18 paths = 18 tests
//   Each test asserts: fork_type, confidence range, evidence_event_ids, recommended_action

mod diagnostic_helpers;

use diagnostic_helpers::{make_event_with_ts, now_ms};
use storage::diagnostic_ledger::types::{
    CorrelationKey, DiagnosticEvent, EventKind, EventPayload, ForkType,
};

// ===========================================================================
// The classifier module must be importable at this path after M3 dev creates it.
// This import will fail at compile time (red phase) until classifier.rs exists.
// ===========================================================================
use storage::diagnostic_ledger::classifier::classify;

fn make_block_applied(height: u64, ts: u64, producer: &str, block_hash: &str) -> DiagnosticEvent {
    DiagnosticEvent {
        event_id: ulid::Ulid::new().to_string(),
        kind: EventKind::BlockApplied,
        timestamp_ms: ts,
        height: Some(height),
        correlation_key: None,
        caused_by_event_id: None,
        is_cascade_origin: false,
        payload: EventPayload::BlockApplied {
            slot: height as u32,
            block_hash: block_hash.to_string(),
            producer_pubkey: producer.to_string(),
            from_peer_id: Some("12D3KooWTest".to_string()),
            received_at_ms: Some(ts.saturating_sub(5)),
            applied_at_ms: ts,
            validation_duration_ms: 10,
            mode: "Full".to_string(),
            tx_count: 1,
        },
    }
}

fn make_block_rejected(height: u64, ts: u64, reason: &str) -> DiagnosticEvent {
    DiagnosticEvent {
        event_id: ulid::Ulid::new().to_string(),
        kind: EventKind::BlockRejected,
        timestamp_ms: ts,
        height: Some(height),
        correlation_key: None,
        caused_by_event_id: None,
        is_cascade_origin: false,
        payload: EventPayload::BlockRejected {
            slot: height as u32,
            block_hash: format!("rejected_hash_{}", height),
            producer_pubkey: "aabbccdd".to_string(),
            from_peer_id: Some("12D3KooWTest".to_string()),
            rejection_reason: reason.to_string(),
            mode: "Full".to_string(),
        },
    }
}

fn make_rollback_started(height: u64, ts: u64) -> DiagnosticEvent {
    DiagnosticEvent {
        event_id: ulid::Ulid::new().to_string(),
        kind: EventKind::RollbackStarted,
        timestamp_ms: ts,
        height: Some(height),
        correlation_key: None,
        caused_by_event_id: None,
        is_cascade_origin: false,
        payload: EventPayload::RollbackStarted {
            from_height: height,
            to_height: height.saturating_sub(1),
            trigger: "shallow_recovery".to_string(),
            cumulative_depth: 1,
        },
    }
}

fn make_snap_sync_completed(ts: u64) -> DiagnosticEvent {
    DiagnosticEvent {
        event_id: ulid::Ulid::new().to_string(),
        kind: EventKind::SnapSyncCompleted,
        timestamp_ms: ts,
        height: None,
        correlation_key: None,
        caused_by_event_id: None,
        is_cascade_origin: false,
        payload: EventPayload::SnapSyncCompleted {
            result: "success".to_string(),
            duration_ms: 12000,
        },
    }
}

fn make_fork_block_received(
    height: u64,
    ts: u64,
    _validation_duration_ms: u64,
    producer: &str,
    corr_key: Option<CorrelationKey>,
) -> DiagnosticEvent {
    DiagnosticEvent {
        event_id: ulid::Ulid::new().to_string(),
        kind: EventKind::ForkBlockReceived,
        timestamp_ms: ts,
        height: Some(height),
        correlation_key: corr_key,
        caused_by_event_id: None,
        is_cascade_origin: false,
        payload: EventPayload::ForkBlockReceived {
            block_hash: format!("fork_hash_{}", height),
            block_slot: height as u32,
            block_height_estimate: Some(height),
            producer_pubkey: producer.to_string(),
            from_peer_id: "12D3KooWFork".to_string(),
            classification: "ForkBlock".to_string(),
            fork_kind: Some("HeightOccupied".to_string()),
            local_tip_hash: format!("tip_hash_{}", height),
            local_tip_height: height,
        },
    }
}

fn make_recovery_classify_with_action(
    height: u64,
    ts: u64,
    action: &str,
    corr_key: Option<CorrelationKey>,
) -> DiagnosticEvent {
    DiagnosticEvent {
        event_id: ulid::Ulid::new().to_string(),
        kind: EventKind::RecoveryClassifyCall,
        timestamp_ms: ts,
        height: Some(height),
        correlation_key: corr_key,
        caused_by_event_id: None,
        is_cascade_origin: false,
        payload: EventPayload::RecoveryClassifyCall {
            local_height: height,
            network_tip_height: height + 10,
            peer_count: 8,
            last_applied_secs: 60,
            shallow_rollback_count: 0,
            snap_attempts: 0,
            last_rollback_local_height: None,
            in_grace_period: false,
            last_finality_height: Some(height.saturating_sub(100)),
            action_returned: Some(action.to_string()),
            rule_matched: Some("Rule3_catchall".to_string()),
        },
    }
}

// ===========================================================================
// Rule (a): ProducerEquivocation
// ===========================================================================

// Requirement: REQ-FORKOBS-CLF-005 (Must)
// Acceptance: Two block_applied for same height same producer -> ProducerEquivocation
#[test]
fn test_rule_a_producer_equivocation() {
    let ts = now_ms();

    // Two BlockApplied events at same height, same producer, different block_hash
    let ev1 = make_block_applied(100, ts, "producerAABB", "hash_A");
    let ev2 = make_block_applied(100, ts + 1, "producerAABB", "hash_B");

    let classification = classify(&[ev1.clone(), ev2.clone()]);

    // O3: return -- ProducerEquivocation
    assert!(
        matches!(classification.fork_type, ForkType::ProducerEquivocation),
        "expected ProducerEquivocation, got {:?}",
        classification.fork_type
    );
    assert!(
        classification.confidence >= 0.9,
        "ProducerEquivocation should have high confidence (>=0.9), got {}",
        classification.confidence
    );
    // Evidence should include both event IDs
    assert!(
        classification.evidence_event_ids.contains(&ev1.event_id),
        "evidence must include first event"
    );
    assert!(
        classification.evidence_event_ids.contains(&ev2.event_id),
        "evidence must include second event"
    );
}

// ===========================================================================
// Rule (b): EpochBoundaryInvalid
// ===========================================================================

// Requirement: REQ-FORKOBS-CLF-005 (Must)
// Acceptance: block_rejected at epoch boundary with "EpochReward" -> EpochBoundaryInvalid
#[test]
fn test_rule_b_epoch_boundary_invalid() {
    let ts = now_ms();

    // Epoch boundary at h=360 (blocks_per_epoch=360 is the mainnet default)
    // The classifier should check height % blocks_per_epoch == 0
    let ev = make_block_rejected(360, ts, "missing EpochReward at epoch boundary");

    let classification = classify(std::slice::from_ref(&ev));

    // O3: return -- EpochBoundaryInvalid
    assert!(
        matches!(classification.fork_type, ForkType::EpochBoundaryInvalid),
        "expected EpochBoundaryInvalid, got {:?}",
        classification.fork_type
    );
    assert!(
        classification.recommended_action.as_deref() == Some("investigate_producer"),
        "recommended_action should be 'investigate_producer', got {:?}",
        classification.recommended_action
    );
    assert!(
        classification.evidence_event_ids.contains(&ev.event_id),
        "evidence must include the rejected event"
    );
}

// ===========================================================================
// Rule (c): RollbackLoop
// ===========================================================================

// Requirement: REQ-FORKOBS-CLF-005 (Must)
// Acceptance: >3 rollback_started in 60s -> RollbackLoop
#[test]
fn test_rule_c_rollback_loop() {
    let ts = now_ms();

    // 4 RollbackStarted events within 60 seconds
    let ev1 = make_rollback_started(100, ts);
    let ev2 = make_rollback_started(101, ts + 10_000);
    let ev3 = make_rollback_started(102, ts + 20_000);
    let ev4 = make_rollback_started(103, ts + 30_000); // 4th within 60s

    let classification = classify(&[ev1, ev2, ev3, ev4]);

    // O3: return -- RollbackLoop
    assert!(
        matches!(classification.fork_type, ForkType::RollbackLoop),
        "expected RollbackLoop, got {:?}",
        classification.fork_type
    );
    assert!(
        classification.recommended_action.as_deref() == Some("investigate_recovery_params"),
        "recommended_action should be 'investigate_recovery_params', got {:?}",
        classification.recommended_action
    );
    assert_eq!(
        classification.evidence_event_ids.len(),
        4,
        "evidence should include all 4 rollback events"
    );
}

// Requirement: REQ-FORKOBS-CLF-005 (Must)
// Acceptance: exactly 3 rollbacks in 60s is NOT a loop (rule is >3)
#[test]
fn test_rule_c_three_rollbacks_not_loop() {
    let ts = now_ms();

    // Only 3 RollbackStarted events within 60 seconds
    let ev1 = make_rollback_started(100, ts);
    let ev2 = make_rollback_started(101, ts + 10_000);
    let ev3 = make_rollback_started(102, ts + 20_000);

    let classification = classify(&[ev1, ev2, ev3]);

    // O3: return -- NOT RollbackLoop (rule is strictly >3)
    assert!(
        !matches!(classification.fork_type, ForkType::RollbackLoop),
        "3 rollbacks should NOT trigger RollbackLoop (requires >3), got {:?}",
        classification.fork_type
    );
}

// ===========================================================================
// Rule (d): PostSnapDeadTip
// ===========================================================================

// Requirement: REQ-FORKOBS-CLF-005 (Must)
// Acceptance: snap_sync_completed followed by fork_block_received within 300s -> PostSnapDeadTip
#[test]
fn test_rule_d_post_snap_dead_tip() {
    let ts = now_ms();

    // SnapSyncCompleted at T, ForkBlockReceived at T+100s (within 300s window)
    let ev_snap = make_snap_sync_completed(ts);
    let ev_fork = make_fork_block_received(200, ts + 100_000, 10, "aabbccdd", None);

    let classification = classify(&[ev_snap.clone(), ev_fork.clone()]);

    // O3: return -- PostSnapDeadTip
    assert!(
        matches!(classification.fork_type, ForkType::PostSnapDeadTip),
        "expected PostSnapDeadTip, got {:?}",
        classification.fork_type
    );
    assert!(
        classification
            .evidence_event_ids
            .contains(&ev_snap.event_id)
            || classification
                .evidence_event_ids
                .contains(&ev_fork.event_id),
        "evidence should include snap or fork event"
    );
}

// Requirement: REQ-FORKOBS-CLF-005 (Must) + O5 decision (300s window)
// Acceptance: snap+fork outside 300s is NOT PostSnapDeadTip
#[test]
fn test_rule_d_outside_300s_window() {
    let ts = now_ms();

    // SnapSyncCompleted at T, ForkBlockReceived at T+400s (OUTSIDE 300s window)
    let ev_snap = make_snap_sync_completed(ts);
    let ev_fork = make_fork_block_received(200, ts + 400_000, 10, "aabbccdd", None);

    let classification = classify(&[ev_snap, ev_fork]);

    // O3: return -- NOT PostSnapDeadTip
    assert!(
        !matches!(classification.fork_type, ForkType::PostSnapDeadTip),
        "snap + fork outside 300s window should NOT be PostSnapDeadTip, got {:?}",
        classification.fork_type
    );
}

// ===========================================================================
// Rule (e): TipRaceHighLatency
// ===========================================================================

// Requirement: REQ-FORKOBS-CLF-005 (Must)
// Acceptance: fork_block_received with validation_duration > 2000ms -> TipRaceHighLatency
#[test]
fn test_rule_e_tip_race_high_latency() {
    let ts = now_ms();

    // ForkBlockReceived + BlockApplied with high latency at same height.
    // ForkBlockReceived payload lacks validation_duration_ms; classifier cross-references
    // with BlockApplied at same height for latency, OR dev adds the field.
    let ev_fork = make_fork_block_received(200, ts, 10, "aabbccdd", None);

    // Also create a BlockApplied with high validation_duration_ms at the same height
    let mut ev_applied = make_block_applied(200, ts + 1, "aabbccdd", "hash_high_lat");
    if let EventPayload::BlockApplied {
        ref mut validation_duration_ms,
        ..
    } = ev_applied.payload
    {
        *validation_duration_ms = 3000; // 3 seconds -- high latency
    }

    let classification = classify(&[ev_fork, ev_applied]);

    // O3: return -- TipRaceHighLatency
    assert!(
        matches!(classification.fork_type, ForkType::TipRaceHighLatency),
        "expected TipRaceHighLatency, got {:?}",
        classification.fork_type
    );
}

// ===========================================================================
// Rule (f): TipRaceNatural
// ===========================================================================

// Requirement: REQ-FORKOBS-CLF-005 (Must)
// Acceptance: fork_block_received with validation_duration < 500ms and no other signals
//   -> TipRaceNatural
#[test]
fn test_rule_f_tip_race_natural() {
    let ts = now_ms();

    // Single ForkBlockReceived with low latency, no other fork/recovery signals
    let ev_fork = make_fork_block_received(200, ts, 10, "aabbccdd", None);

    // Optionally include a low-latency BlockApplied
    let mut ev_applied = make_block_applied(200, ts + 1, "aabbccdd", "hash_natural");
    if let EventPayload::BlockApplied {
        ref mut validation_duration_ms,
        ..
    } = ev_applied.payload
    {
        *validation_duration_ms = 100; // 100ms -- low latency
    }

    let classification = classify(&[ev_fork, ev_applied]);

    // O3: return -- TipRaceNatural
    assert!(
        matches!(classification.fork_type, ForkType::TipRaceNatural),
        "expected TipRaceNatural, got {:?}",
        classification.fork_type
    );
}

// Requirement: REQ-FORKOBS-CLF-005 (Must) + O5 (no other signals definition)
// Acceptance: "no other signals" means no other event with fork-classified kind in same
//   correlation_key group. RecoveryClassifyCall with action != None is a signal.
#[test]
fn test_rule_f_no_other_signals_definition() {
    let ts = now_ms();

    // Shared correlation key
    let corr_key = CorrelationKey {
        divergence_height: Some(200),
        canonical_hash: Some("canonical_abc".to_string()),
        fork_hash: Some("fork_xyz".to_string()),
    };

    // ForkBlockReceived with low latency
    let ev_fork = make_fork_block_received(200, ts, 100, "aabbccdd", Some(corr_key.clone()));

    // RecoveryClassifyCall with action != None in the SAME correlation_key group
    // This constitutes "other signals" per architect O5
    let ev_recovery =
        make_recovery_classify_with_action(200, ts + 5000, "HeaderFirstSync", Some(corr_key));

    let classification = classify(&[ev_fork, ev_recovery]);

    // O3: return -- NOT TipRaceNatural (has other signals in same correlation_key group)
    assert!(
        !matches!(classification.fork_type, ForkType::TipRaceNatural),
        "ForkBlockReceived + RecoveryClassifyCall(action!=None) in same corr_key should NOT be TipRaceNatural, got {:?}",
        classification.fork_type
    );
}

// ===========================================================================
// Rule (g): Unknown
// ===========================================================================

// Requirement: REQ-FORKOBS-CLF-005 (Must) + REQ-FORKOBS-RETRO-003 (Must)
// Acceptance: no rule matches -> Unknown with reason_unknown and evidence_event_ids
#[test]
fn test_rule_g_unknown_with_evidence() {
    let ts = now_ms();

    // WriterHeartbeat events don't match any fork classification rule
    let ev1 = make_event_with_ts(EventKind::WriterHeartbeat, 0, ts);
    let ev2 = make_event_with_ts(EventKind::WriterHeartbeat, 0, ts + 1000);

    let classification = classify(&[ev1.clone(), ev2.clone()]);

    // O3: return -- Unknown with evidence
    match &classification.fork_type {
        ForkType::Unknown {
            reason_unknown,
            evidence_event_ids,
        } => {
            assert!(
                !reason_unknown.is_empty(),
                "reason_unknown must be non-empty"
            );
            // Per REQ-FORKOBS-RETRO-003: evidence_event_ids = all input event_ids
            assert!(
                evidence_event_ids.contains(&ev1.event_id),
                "evidence should contain ev1.event_id"
            );
            assert!(
                evidence_event_ids.contains(&ev2.event_id),
                "evidence should contain ev2.event_id"
            );
        }
        other => panic!("expected ForkType::Unknown, got {:?}", other),
    }
}

// ===========================================================================
// First-match-wins (O5)
// ===========================================================================

// Requirement: REQ-FORKOBS-CLF-005 (Must) + O5
// Acceptance: When rule (a) ProducerEquivocation AND rule (e) high latency both match,
//   ProducerEquivocation wins (rule a is higher priority)
#[test]
fn test_first_match_wins_priority() {
    let ts = now_ms();

    // ProducerEquivocation: same height, same producer, different hash
    let ev1 = make_block_applied(100, ts, "producerAABB", "hash_A");
    let mut ev2 = make_block_applied(100, ts + 1, "producerAABB", "hash_B");

    // Also set high validation_duration_ms to make rule (e) match too
    if let EventPayload::BlockApplied {
        ref mut validation_duration_ms,
        ..
    } = ev2.payload
    {
        *validation_duration_ms = 5000; // 5s -- would trigger TipRaceHighLatency
    }

    // Also add a ForkBlockReceived to make rule (e) eligible
    let ev_fork = make_fork_block_received(100, ts + 2, 5000, "producerAABB", None);

    let classification = classify(&[ev1, ev2, ev_fork]);

    // O3: return -- ProducerEquivocation (rule a wins over rule e)
    assert!(
        matches!(classification.fork_type, ForkType::ProducerEquivocation),
        "first-match-wins: ProducerEquivocation (rule a) should beat TipRaceHighLatency (rule e), got {:?}",
        classification.fork_type
    );
}

// ===========================================================================
// Pure function / determinism
// ===========================================================================

// Requirement: REQ-FORKOBS-CLF-003 (Must)
// Acceptance: classify is a pure function -- same input -> same output
#[test]
fn test_classifier_is_pure_function() {
    let ts = now_ms();

    let ev1 = make_rollback_started(100, ts);
    let ev2 = make_rollback_started(101, ts + 10_000);
    let ev3 = make_rollback_started(102, ts + 20_000);
    let ev4 = make_rollback_started(103, ts + 30_000);

    let events = vec![ev1, ev2, ev3, ev4];

    let result1 = classify(&events);
    let result2 = classify(&events);

    // O3: return -- bitwise-identical on both calls
    assert_eq!(
        result1.fork_type, result2.fork_type,
        "fork_type must be identical"
    );
    assert_eq!(
        result1.confidence, result2.confidence,
        "confidence must be identical"
    );
    assert_eq!(
        result1.evidence_event_ids, result2.evidence_event_ids,
        "evidence_event_ids must be identical"
    );
    assert_eq!(
        result1.recommended_action, result2.recommended_action,
        "recommended_action must be identical"
    );
}

// ===========================================================================
// Edge cases
// ===========================================================================

// Requirement: REQ-FORKOBS-CLF-005 (Must)
// Acceptance: empty input -> Unknown
#[test]
fn test_classifier_empty_input() {
    let classification = classify(&[]);

    // O3: return -- Unknown with reason for empty
    match &classification.fork_type {
        ForkType::Unknown {
            reason_unknown,
            evidence_event_ids,
        } => {
            assert!(
                !reason_unknown.is_empty(),
                "reason_unknown must describe why (e.g., 'no events to classify')"
            );
            assert!(
                evidence_event_ids.is_empty(),
                "empty input should produce empty evidence list"
            );
        }
        other => panic!("empty input should produce Unknown, got {:?}", other),
    }
}

// Requirement: REQ-FORKOBS-CLF-001 (Must)
// Acceptance: confidence in [0.0, 1.0] for every classification
#[test]
fn test_classifier_confidence_bounded() {
    let ts = now_ms();

    // Test several scenarios and assert confidence bounds
    let test_cases: Vec<Vec<DiagnosticEvent>> = vec![
        // ProducerEquivocation
        vec![
            make_block_applied(100, ts, "prod", "hash_A"),
            make_block_applied(100, ts + 1, "prod", "hash_B"),
        ],
        // RollbackLoop
        vec![
            make_rollback_started(10, ts),
            make_rollback_started(11, ts + 10_000),
            make_rollback_started(12, ts + 20_000),
            make_rollback_started(13, ts + 30_000),
        ],
        // PostSnapDeadTip
        vec![
            make_snap_sync_completed(ts),
            make_fork_block_received(200, ts + 100_000, 10, "aabbccdd", None),
        ],
        // Unknown (empty)
        vec![],
        // Unknown (non-matching)
        vec![make_event_with_ts(EventKind::WriterHeartbeat, 0, ts)],
    ];

    for (i, events) in test_cases.iter().enumerate() {
        let c = classify(events);
        assert!(
            c.confidence >= 0.0 && c.confidence <= 1.0,
            "test case {}: confidence {} not in [0.0, 1.0]",
            i,
            c.confidence
        );
    }
}

// Requirement: REQ-FORKOBS-CLF-001 (Must)
// Acceptance: when fork_type is a named variant (not Unknown), evidence_event_ids is non-empty
#[test]
fn test_classifier_evidence_event_ids_non_empty_for_specific_variant() {
    let ts = now_ms();

    // ProducerEquivocation: definitely a named variant
    let ev1 = make_block_applied(100, ts, "prod", "hash_A");
    let ev2 = make_block_applied(100, ts + 1, "prod", "hash_B");

    let c = classify(&[ev1, ev2]);

    if !matches!(c.fork_type, ForkType::Unknown { .. }) {
        assert!(
            !c.evidence_event_ids.is_empty(),
            "named variant {:?} must have non-empty evidence_event_ids",
            c.fork_type
        );
    }

    // RollbackLoop
    let events: Vec<_> = (0..4)
        .map(|i| make_rollback_started(100 + i, ts + i * 10_000))
        .collect();
    let c = classify(&events);
    if !matches!(c.fork_type, ForkType::Unknown { .. }) {
        assert!(
            !c.evidence_event_ids.is_empty(),
            "RollbackLoop must have non-empty evidence_event_ids"
        );
    }
}

// Requirement: REQ-FORKOBS-CLF-004 (Should)
// Acceptance: each named variant has a specific recommended_action string
#[test]
fn test_classifier_recommended_action_for_each_named_variant() {
    let ts = now_ms();

    // Table-driven: each variant's fixture + expected recommended_action
    let cases: Vec<(Vec<DiagnosticEvent>, &str, &str)> = vec![
        // (events, expected_variant_name, expected_recommended_action)
        (
            vec![
                make_block_applied(100, ts, "prod", "hash_A"),
                make_block_applied(100, ts + 1, "prod", "hash_B"),
            ],
            "ProducerEquivocation",
            "investigate_producer",
        ),
        (
            vec![make_block_rejected(
                360,
                ts,
                "missing EpochReward at epoch boundary",
            )],
            "EpochBoundaryInvalid",
            "investigate_producer",
        ),
        (
            (0..4)
                .map(|i| make_rollback_started(100 + i, ts + i * 10_000))
                .collect(),
            "RollbackLoop",
            "investigate_recovery_params",
        ),
        (
            vec![
                make_snap_sync_completed(ts),
                make_fork_block_received(200, ts + 100_000, 10, "aabbccdd", None),
            ],
            "PostSnapDeadTip",
            "investigate_snap_sync",
        ),
    ];

    for (events, variant_name, expected_action) in cases {
        let c = classify(&events);
        assert_eq!(
            c.recommended_action.as_deref(),
            Some(expected_action),
            "variant {} should have recommended_action='{}', got {:?}",
            variant_name,
            expected_action,
            c.recommended_action
        );
    }
}

// Requirement: REQ-FORKOBS-RETRO-003 (Must)
// Acceptance: Unknown carries all input event_ids as evidence for human audit
#[test]
fn test_classifier_unknown_carries_evidence() {
    let ts = now_ms();

    // Mix of events that don't match any specific rule
    let ev1 = make_event_with_ts(EventKind::SnapSyncFailed, 0, ts);
    let ev2 = make_event_with_ts(EventKind::ChainBreakDetected, 50, ts + 1000);
    let ev3 = make_event_with_ts(EventKind::WriterHeartbeat, 0, ts + 2000);

    let classification = classify(&[ev1.clone(), ev2.clone(), ev3.clone()]);

    match &classification.fork_type {
        ForkType::Unknown {
            reason_unknown: _,
            evidence_event_ids,
        } => {
            // All input event IDs must be in the evidence for human audit
            assert!(
                evidence_event_ids.contains(&ev1.event_id),
                "evidence must contain ev1"
            );
            assert!(
                evidence_event_ids.contains(&ev2.event_id),
                "evidence must contain ev2"
            );
            assert!(
                evidence_event_ids.contains(&ev3.event_id),
                "evidence must contain ev3"
            );
            assert_eq!(
                evidence_event_ids.len(),
                3,
                "evidence should contain exactly all 3 input event IDs"
            );
        }
        other => panic!("expected Unknown for non-matching events, got {:?}", other),
    }
}

// ===========================================================================
// Rule (h): ChainBreakLoop -- Workflow #349 (Phase 1.5)
// ===========================================================================

fn make_chain_break_detected(slot: u32, ts: u64) -> DiagnosticEvent {
    DiagnosticEvent {
        event_id: ulid::Ulid::new().to_string(),
        kind: EventKind::ChainBreakDetected,
        timestamp_ms: ts,
        height: None,
        correlation_key: None,
        caused_by_event_id: None,
        is_cascade_origin: false,
        payload: EventPayload::ChainBreakDetected {
            expected_prev_hash: format!("expected_{}", slot),
            actual_prev_hash: format!("actual_{}", slot),
            header_slot: slot,
            valid_so_far_count: 0,
            from_peer_id: "12D3KooWChainBreak".to_string(),
        },
    }
}

// Requirement: REQ-FORKOBS-CLF-006 (Must) — signal (a) ChainBreakDetected > 3
// Acceptance: 5 ChainBreakDetected events -> ChainBreakLoop, NOT Unknown.
#[test]
fn test_rule_h_chain_break_loop_signal_a_chain_breaks() {
    let ts = now_ms();

    // Five ChainBreakDetected events spread across 30 minutes -- signal (a) > 3.
    let events: Vec<DiagnosticEvent> = (0..5)
        .map(|i| make_chain_break_detected(218900 + i, ts + i as u64 * 60_000))
        .collect();

    let classification = classify(&events);

    match &classification.fork_type {
        ForkType::ChainBreakLoop {
            chain_break_count, ..
        } => {
            assert_eq!(
                *chain_break_count, 5,
                "rule (h) must count all 5 ChainBreakDetected events"
            );
        }
        other => panic!(
            "expected ChainBreakLoop, got {:?} (rule h signal (a) should fire on 5 chain breaks)",
            other
        ),
    }
    assert_eq!(
        classification.recommended_action.as_deref(),
        Some("restart_with_resync"),
        "ChainBreakLoop must recommend restart_with_resync"
    );
    assert!(
        (classification.confidence - 0.85).abs() < 1e-9,
        "ChainBreakLoop confidence should be 0.85, got {}",
        classification.confidence
    );
    assert!(
        !classification.evidence_event_ids.is_empty(),
        "ChainBreakLoop must carry evidence event IDs"
    );
    assert!(
        classification.recommended_action_args.is_some(),
        "ChainBreakLoop must include structured recommended_action_args"
    );
}

// Requirement: REQ-FORKOBS-CLF-006 (Must) — signal (b) fork/applied ratio
// Acceptance: 150 ForkBlockReceived + 5 BlockApplied (ratio 30:1) -> ChainBreakLoop.
//   This is the n6 pattern in miniature: many fork events, few applies, no chain breaks.
#[test]
fn test_rule_h_chain_break_loop_signal_b_fork_ratio() {
    let ts = now_ms();

    let mut events: Vec<DiagnosticEvent> = Vec::new();
    // 150 ForkBlockReceived events at sparse heights, low validation latency
    for i in 0..150u64 {
        events.push(make_fork_block_received(
            115_000 + i,
            ts + i * 10_000,
            10,
            "stuck_node_producer",
            None,
        ));
    }
    // 5 BlockApplied events (the node is barely advancing)
    for i in 0..5u64 {
        let mut ev = make_block_applied(115_000 + i, ts + i * 600_000, "any_prod", "hash_x");
        if let EventPayload::BlockApplied {
            ref mut validation_duration_ms,
            ..
        } = ev.payload
        {
            *validation_duration_ms = 50;
        }
        events.push(ev);
    }

    let classification = classify(&events);

    assert!(
        matches!(classification.fork_type, ForkType::ChainBreakLoop { .. }),
        "fork/applied ratio 30:1 with 150 fork events should classify as ChainBreakLoop, got {:?}",
        classification.fork_type
    );
    assert_eq!(
        classification.recommended_action.as_deref(),
        Some("restart_with_resync"),
    );
}

// Requirement: REQ-FORKOBS-CLF-006 (Must) — signal (c) rollback_count > 10 in window
// Acceptance: 11 rollbacks spread across 30 minutes (NOT triggering rule (c) 60s loop)
//   -> ChainBreakLoop.
#[test]
fn test_rule_h_chain_break_loop_signal_c_rollback_spread() {
    let ts = now_ms();

    // 11 rollbacks spread across 30 minutes -- well outside the 60s window of rule (c).
    // Rule (c) needs >3 within 60s; rule (h) signal (c) needs >10 anywhere in the hour window.
    let events: Vec<DiagnosticEvent> = (0..11u64)
        .map(|i| make_rollback_started(100 + i, ts + i * 180_000))
        .collect();

    let classification = classify(&events);

    assert!(
        matches!(classification.fork_type, ForkType::ChainBreakLoop { .. }),
        "11 rollbacks spread across 30min should classify as ChainBreakLoop (rule h sig c), got {:?}",
        classification.fork_type
    );
}

// Requirement: REQ-FORKOBS-CLF-006 (Must) — signal (d) recovery_attempts > 20
// Acceptance: 21 RecoveryClassifyCall events -> ChainBreakLoop.
#[test]
fn test_rule_h_chain_break_loop_signal_d_recovery_churn() {
    let ts = now_ms();

    // 21 RecoveryClassifyCall events -- recovery state machine churning.
    let events: Vec<DiagnosticEvent> = (0..21u64)
        .map(|i| {
            make_recovery_classify_with_action(110_367, ts + i * 1_000, "HeaderFirstSync", None)
        })
        .collect();

    let classification = classify(&events);

    match &classification.fork_type {
        ForkType::ChainBreakLoop {
            recovery_attempts, ..
        } => {
            assert_eq!(
                *recovery_attempts, 21,
                "must count all 21 RecoveryClassifyCall events"
            );
        }
        other => panic!("expected ChainBreakLoop, got {:?}", other),
    }
}

// Requirement: REQ-FORKOBS-CLF-006 (Must) — precedence after (d), before (e)/(f)
// Acceptance: When PostSnapDeadTip (rule d) AND ChainBreakLoop (rule h) both match,
//   rule (d) wins. When TipRaceNatural (rule f) AND ChainBreakLoop (rule h) both match,
//   rule (h) wins.
#[test]
fn test_rule_h_precedence_after_d_before_ef() {
    let ts = now_ms();

    // Case 1: SnapSyncCompleted + recent ForkBlockReceived (rule d) + 5 chain breaks (rule h sig a)
    //   -> rule (d) PostSnapDeadTip wins (precedence)
    let mut events_d_wins: Vec<DiagnosticEvent> = vec![
        make_snap_sync_completed(ts),
        make_fork_block_received(200, ts + 100_000, 10, "aabbccdd", None),
    ];
    for i in 0..5u64 {
        events_d_wins.push(make_chain_break_detected(
            218_900 + i as u32,
            ts + i * 1_000,
        ));
    }
    let c = classify(&events_d_wins);
    assert!(
        matches!(c.fork_type, ForkType::PostSnapDeadTip),
        "PostSnapDeadTip (rule d) must win over ChainBreakLoop (rule h), got {:?}",
        c.fork_type
    );

    // Case 2: Single fast-race fork event (would match rule f) + 5 chain breaks (rule h sig a)
    //   -> rule (h) ChainBreakLoop wins (precedence over f)
    let mut events_h_wins: Vec<DiagnosticEvent> = vec![
        make_fork_block_received(200, ts, 100, "fast_producer", None),
        make_block_applied(200, ts + 1, "fast_producer", "applied_hash"),
    ];
    if let EventPayload::BlockApplied {
        ref mut validation_duration_ms,
        ..
    } = events_h_wins[1].payload
    {
        *validation_duration_ms = 100;
    }
    for i in 0..5u64 {
        events_h_wins.push(make_chain_break_detected(
            218_900 + i as u32,
            ts + i * 1_000,
        ));
    }
    let c = classify(&events_h_wins);
    assert!(
        matches!(c.fork_type, ForkType::ChainBreakLoop { .. }),
        "ChainBreakLoop (rule h) must win over TipRaceNatural (rule f), got {:?}",
        c.fork_type
    );
}

// Requirement: REQ-FORKOBS-CLF-006 (Must) — n6 live fixture acceptance criterion A
// Acceptance: The live-captured n6 DiagnosticBundle (workflow #349, 2026-05-20) — the same
//   data the broken classifier mis-labelled as TipRaceNatural — must classify as ChainBreakLoop.
//
//   Bundle stats (verified via jq):
//   - ForkBlockReceived: 1299, BlockApplied: 30  (ratio 43:1, fires signal b)
//   - RollbackStarted: 42                         (>10, fires signal c)
//   - RecoveryClassifyCall: 241                   (>20, fires signal d)
//   - ChainBreakDetected: 0                       (does not fire signal a -- emitter
//                                                  does not yet emit on chain-break path;
//                                                  the historical log-replay fixture does)
#[test]
fn test_rule_h_chain_break_loop_n6_live_fixture() {
    // Fixture captured from live testnet n6 (workflow #349, 2026-05-20).
    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/inc-n6-chain-break-loop.json");
    assert!(
        fixture_path.exists(),
        "n6 fixture file not found: {:?}",
        fixture_path
    );

    let bundle_json = std::fs::read_to_string(&fixture_path).expect("read n6 fixture");
    let bundle: storage::diagnostic_ledger::types::DiagnosticBundle =
        serde_json::from_str(&bundle_json).expect("n6 fixture must parse as DiagnosticBundle");

    let classification = classify(&bundle.events);

    // Must classify as ChainBreakLoop, NOT TipRaceNatural (the bug we are fixing)
    match &classification.fork_type {
        ForkType::ChainBreakLoop {
            chain_break_count,
            recovery_attempts,
            rollback_count,
            seconds_stuck,
        } => {
            // n6 has zero ChainBreakDetected events in the bundle (emitter does not yet emit
            // on chain-break path). Rule (h) fires via signals b/c/d.
            assert_eq!(
                *chain_break_count, 0,
                "n6 bundle has no ChainBreakDetected events"
            );
            // Recovery churn: 241 in the original bundle, but rule (h) only counts events
            // within the last hour. The bundle may span >1 hour, so the value can be lower.
            assert!(
                *recovery_attempts >= 21,
                "n6 bundle must have >=21 RecoveryClassifyCall events in the window, got {}",
                recovery_attempts
            );
            // Rollback count similarly bounded by window
            assert!(
                *rollback_count >= 11,
                "n6 bundle must have >=11 RollbackStarted events in the window, got {}",
                rollback_count
            );
            // Seconds_stuck is bounded by window (<=3600). Value depends on event spacing.
            assert!(
                *seconds_stuck <= 3600,
                "seconds_stuck capped by 1h window, got {}",
                seconds_stuck
            );
        }
        other => panic!(
            "n6 fixture must classify as ChainBreakLoop, got {:?} (the bug being fixed)",
            other
        ),
    }

    assert_eq!(
        classification.recommended_action.as_deref(),
        Some("restart_with_resync"),
        "ChainBreakLoop must recommend restart_with_resync, not normal_operation"
    );
    assert!(
        !classification.evidence_event_ids.is_empty(),
        "ChainBreakLoop must carry evidence event IDs"
    );
    assert!(
        classification.recommended_action_args.is_some(),
        "recommended_action_args must include the wipe scope"
    );
    // Confidence bounded
    assert!(
        classification.confidence >= 0.0 && classification.confidence <= 1.0,
        "confidence in [0,1], got {}",
        classification.confidence
    );
}

// ===========================================================================
// M6 (D3): RecoveryClassifyCall correlation_key grouping — INC-I-090
// ===========================================================================

// OUTPUT CONTRACT: fn has_other_signals(events, fork_ev) -> bool
//   When a ForkBlockReceived carries correlation_key with divergence_height=H
//   and a RecoveryClassifyCall carries correlation_key with divergence_height=H
//   (but different fork_hash / canonical_hash — recovery doesn't know hashes),
//   has_other_signals MUST return true. The grouping is by divergence_height,
//   not by exact CorrelationKey equality.
//
// Path P18: divergence_height-only correlation match
// Input partition P18a: ForkBlockReceived at h=284677 with
//   divergence_height=Some(284677), fork_hash=Some("150b4a7b")
//   + RecoveryClassifyCall at h=284677 with
//   divergence_height=Some(284677), fork_hash=None
//   -> classify does NOT return TipRaceNatural (has_other_signals returns true)

// Requirement: INC-I-090 D3 (Must)
// Acceptance: RecoveryClassifyCall with divergence_height-only correlation_key
//   groups with ForkBlockReceived that has divergence_height+fork_hash.
//   The classifier must NOT return TipRaceNatural when recovery activity
//   at the same divergence_height exists.
#[test]
fn test_m6_d3_recovery_correlation_key_groups_by_divergence_height() {
    let ts = now_ms();
    let div_height: u64 = 284_677;

    // ForkBlockReceived with full correlation_key (as emitted by block_handling.rs:209)
    let fork_corr_key = CorrelationKey {
        divergence_height: Some(div_height),
        canonical_hash: None,
        fork_hash: Some("150b4a7b".to_string()),
    };
    let ev_fork = make_fork_block_received(
        div_height,
        ts,
        100, // low validation latency
        "aabbccdd",
        Some(fork_corr_key),
    );

    // RecoveryClassifyCall with divergence_height-only correlation_key
    // (as should be emitted by periodic.rs:624 after M6 fix — recovery
    // coordinator doesn't know fork_hash or canonical_hash)
    let recovery_corr_key = CorrelationKey {
        divergence_height: Some(div_height),
        canonical_hash: None,
        fork_hash: None,
    };
    let ev_recovery = make_recovery_classify_with_action(
        div_height,
        ts + 5_000,
        "HeaderFirstSync",
        Some(recovery_corr_key),
    );

    // Also include a low-latency BlockApplied so rule (f) would match if
    // has_other_signals returned false (i.e., without the fix this would be
    // classified as TipRaceNatural)
    let mut ev_applied = make_block_applied(div_height, ts + 1, "aabbccdd", "canonical_hash");
    if let EventPayload::BlockApplied {
        ref mut validation_duration_ms,
        ..
    } = ev_applied.payload
    {
        *validation_duration_ms = 100; // low latency
    }

    let classification = classify(&[ev_fork, ev_recovery, ev_applied]);

    // OUTPUT CONTRACT CHECK:
    // With the fix, has_other_signals matches on divergence_height alone,
    // so the RecoveryClassifyCall is seen as a signal in the fork's correlation
    // group. Rule (f) TipRaceNatural requires NO other signals, so it must NOT
    // match. The result should be Unknown (rule g) since no other rule fits
    // this 3-event set.
    assert!(
        !matches!(classification.fork_type, ForkType::TipRaceNatural),
        "D3 fix: ForkBlockReceived + RecoveryClassifyCall at same divergence_height \
         (but different fork_hash) must NOT classify as TipRaceNatural. \
         has_other_signals must match on divergence_height, not exact CorrelationKey. \
         Got {:?}",
        classification.fork_type
    );
}
