// OUTPUT CONTRACT:
// Functions under test:
//   1. storage::diagnostic_ledger::log_replay::replay_log_file(path) -> Vec<ReplayedEvent>
//   2. storage::diagnostic_ledger::classifier::classify(events) -> Classification
//
// Observable outputs:
//   - replay_log_file: Vec<ReplayedEvent> (length, event kinds, event payloads)
//   - classify: Classification (fork_type, confidence, evidence_event_ids, recommended_action)
//
// Code paths x partitions:
//   - INC-I-083 fixture: chain-break loop pattern -> ChainBreakDetected events -> Unknown or named variant
//   - INC-I-081 fixture: BlockRejected at epoch boundary with EpochReward -> EpochBoundaryInvalid
//
// INPUT PARTITIONS:
//   P1: INC-I-083 real captured log (chain-break loop, HEALTH, SYNC_DEBUG) -> at least ChainBreakDetected events
//   P2: INC-I-081 synthesized log (BlockRejected at h%360==0 with EpochReward) -> EpochBoundaryInvalid
//
// Every cell in the matrix has assertions below.

/// E2E test: replay the INC-I-083 fixture through the parser and classifier.
///
/// The fixture is a captured snippet from ~/testnet/logs/n10.log during the
/// 2026-05-19 incident (stuck sync, chain-break loop, recovery refused).
#[test]
fn test_inc_i_083_replay_produces_actionable_verdict() {
    // Load fixture
    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/storage/tests/fixtures/inc-i-083-n10.fixture");
    assert!(
        fixture_path.exists(),
        "Fixture file not found: {:?}",
        fixture_path
    );

    let replayed = storage::diagnostic_ledger::log_replay::replay_log_file(&fixture_path)
        .expect("replay failed");

    // Extract events and classify
    let events: Vec<_> = replayed.iter().map(|r| r.event.clone()).collect();
    let classification = storage::diagnostic_ledger::classifier::classify(&events);

    // ASSERTION 1: At least one event was parsed (basic smoke)
    assert!(
        !events.is_empty(),
        "fixture should produce at least one event"
    );

    // ASSERTION 2: At least one ChainBreakDetected event (the recovery loop signature)
    let has_chain_break = events.iter().any(|e| {
        matches!(
            e.kind,
            storage::diagnostic_ledger::types::EventKind::ChainBreakDetected
        )
    });
    assert!(
        has_chain_break,
        "INC-I-083 fixture should produce ChainBreakDetected events"
    );

    // ASSERTION 3: At least one HEALTH event (RecoveryClassifyCall)
    let has_health = events.iter().any(|e| {
        matches!(
            e.kind,
            storage::diagnostic_ledger::types::EventKind::RecoveryClassifyCall
        )
    });
    assert!(
        has_health,
        "INC-I-083 fixture should produce RecoveryClassifyCall (HEALTH) events"
    );

    // ASSERTION 4: All replayed events have replayed_from_log=true
    assert!(
        replayed.iter().all(|r| r.replayed_from_log),
        "all replayed events should have replayed_from_log=true"
    );

    // ASSERTION 5: Classification verdict
    // Workflow #349 (Phase 1.5) closed the classifier gap that left this fixture as
    // `Unknown`. Rule (h) `ChainBreakLoop` MUST now fire on the chain-break / recovery-
    // churn pattern (signal_a: chain_break_count > 3). REQ-FORKOBS-CLF-006 acceptance.
    match &classification.fork_type {
        storage::diagnostic_ledger::types::ForkType::ChainBreakLoop {
            chain_break_count,
            recovery_attempts,
            seconds_stuck,
            rollback_count,
        } => {
            assert!(
                *chain_break_count > 3,
                "INC-I-083 fixture must trip rule (h) signal_a (chain_break_count > 3), got {}",
                chain_break_count
            );
            assert_eq!(
                classification.recommended_action.as_deref(),
                Some("restart_with_resync"),
                "ChainBreakLoop must recommend restart_with_resync, not Unknown's None"
            );
            assert!(
                classification.recommended_action_args.is_some(),
                "ChainBreakLoop must include structured wipe-scope args"
            );
            // Evidence must include the chain-break events
            let chain_break_ids: Vec<String> = events
                .iter()
                .filter(|e| {
                    matches!(
                        e.kind,
                        storage::diagnostic_ledger::types::EventKind::ChainBreakDetected
                    )
                })
                .map(|e| e.event_id.clone())
                .collect();
            assert!(
                !chain_break_ids.is_empty(),
                "INC-I-083 fixture should produce ChainBreakDetected events"
            );
            let evidence_has_chain_break = chain_break_ids
                .iter()
                .any(|id| classification.evidence_event_ids.contains(id));
            assert!(
                evidence_has_chain_break,
                "ChainBreakLoop evidence should include at least one ChainBreakDetected event"
            );
            println!(
                "INC-I-083 verdict: ChainBreakLoop {{ chain_breaks={}, recovery={}, rollbacks={}, stuck={}s }}",
                chain_break_count, recovery_attempts, rollback_count, seconds_stuck
            );
        }
        other => {
            panic!(
                "Expected ChainBreakLoop after workflow #349. Got {:?}. \
                 If rule (h) was not deployed, the classifier still has the n6 / INC-I-083 gap.",
                other
            );
        }
    }
}

/// E2E test: replay the INC-I-081 fixture through the parser and classifier.
///
/// The fixture is synthesized to production format matching INC-I-081 (mainnet
/// incident: broken producer emitted invalid epoch-boundary block missing
/// EpochReward transaction). The [BLOCK] REJECT format requires the M4-added
/// parse_block_reject_structured parser.
#[test]
fn test_inc_i_081_replay_produces_epoch_boundary_invalid() {
    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/storage/tests/fixtures/inc-i-081-broken-producer.fixture");
    assert!(
        fixture_path.exists(),
        "Fixture file not found: {:?}",
        fixture_path
    );

    let replayed = storage::diagnostic_ledger::log_replay::replay_log_file(&fixture_path)
        .expect("replay failed");

    let events: Vec<_> = replayed.iter().map(|r| r.event.clone()).collect();
    let classification = storage::diagnostic_ledger::classifier::classify(&events);

    // ASSERTION 1: The fixture produced BlockRejected events
    let rejected_events: Vec<_> = events
        .iter()
        .filter(|e| {
            matches!(
                e.kind,
                storage::diagnostic_ledger::types::EventKind::BlockRejected
            )
        })
        .collect();
    assert!(
        !rejected_events.is_empty(),
        "INC-I-081 fixture should produce BlockRejected events"
    );

    // ASSERTION 2: BlockRejected at h=110160 (epoch boundary, 110160 % 360 == 0)
    let has_epoch_boundary_reject = rejected_events.iter().any(|e| {
        if let Some(h) = e.height {
            h % 360 == 0
        } else {
            false
        }
    });
    assert!(
        has_epoch_boundary_reject,
        "INC-I-081 fixture should have BlockRejected at an epoch boundary height"
    );

    // ASSERTION 3: Rejection reason contains "EpochReward"
    let has_epoch_reward_reason = rejected_events.iter().any(|e| {
        if let storage::diagnostic_ledger::types::EventPayload::BlockRejected {
            ref rejection_reason,
            ..
        } = e.payload
        {
            rejection_reason.contains("EpochReward")
        } else {
            false
        }
    });
    assert!(
        has_epoch_reward_reason,
        "INC-I-081 fixture should have EpochReward in rejection reason"
    );

    // ASSERTION 4: Classification is EpochBoundaryInvalid (rule b)
    match &classification.fork_type {
        storage::diagnostic_ledger::types::ForkType::EpochBoundaryInvalid => {
            assert_eq!(
                classification.recommended_action.as_deref(),
                Some("investigate_producer"),
                "EpochBoundaryInvalid should recommend investigate_producer"
            );
            println!("INC-I-081 verdict: EpochBoundaryInvalid (correct)");
        }
        other => {
            panic!(
                "Expected EpochBoundaryInvalid, got {:?}. Classifier rule (b) coverage hole?",
                other
            );
        }
    }
}
