// OUTPUT CONTRACT: N/A — fixture/helper file (no tests, only shared constructors)
// INPUT PARTITIONS: N/A — fixture/helper file
//
// Shared test helpers for diagnostic_ledger test suite — Workflow #346, M1
//
// TDD RED PHASE: These helpers reference production types that do not exist yet.
// All tests using these helpers will fail to compile until the developer creates:
//   - crates/storage/src/diagnostic_ledger/types.rs
//   - crates/storage/src/diagnostic_ledger/mod.rs
//   - crates/storage/src/diagnostic_ledger/emitter.rs

use std::time::{SystemTime, UNIX_EPOCH};

use storage::diagnostic_ledger::types::{CorrelationKey, DiagnosticEvent, EventKind, EventPayload};

/// Current wall-clock in milliseconds.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Build a minimal DiagnosticEvent with the given kind and height.
#[allow(dead_code)]
pub fn make_event(kind: EventKind, height: u64) -> DiagnosticEvent {
    make_event_with_ts(kind, height, now_ms())
}

/// Build a DiagnosticEvent with explicit timestamp (for retention tests).
pub fn make_event_with_ts(kind: EventKind, height: u64, timestamp_ms: u64) -> DiagnosticEvent {
    let payload = match kind {
        EventKind::BlockApplied => EventPayload::BlockApplied {
            slot: height as u32,
            block_hash: format!("hash_{}", height),
            producer_pubkey: "aabbccdd".to_string(),
            from_peer_id: Some("12D3KooWTest".to_string()),
            received_at_ms: Some(timestamp_ms.saturating_sub(10)),
            applied_at_ms: timestamp_ms,
            validation_duration_ms: 5,
            mode: "Full".to_string(),
            tx_count: 1,
        },
        EventKind::BlockRejected => EventPayload::BlockRejected {
            slot: height as u32,
            block_hash: format!("hash_{}", height),
            producer_pubkey: "aabbccdd".to_string(),
            from_peer_id: Some("12D3KooWTest".to_string()),
            rejection_reason: "invalid signature".to_string(),
            mode: "Full".to_string(),
        },
        EventKind::ForkBlockReceived => EventPayload::ForkBlockReceived {
            block_hash: format!("fork_hash_{}", height),
            block_slot: height as u32,
            block_height_estimate: Some(height),
            producer_pubkey: "aabbccdd".to_string(),
            from_peer_id: "12D3KooWFork".to_string(),
            classification: "ForkBlock".to_string(),
            fork_kind: Some("HeightOccupied".to_string()),
            local_tip_hash: format!("tip_hash_{}", height),
            local_tip_height: height,
        },
        EventKind::RollbackStarted => EventPayload::RollbackStarted {
            from_height: height,
            to_height: height.saturating_sub(1),
            trigger: "shallow_recovery".to_string(),
            cumulative_depth: 1,
        },
        EventKind::RollbackCompleted => EventPayload::RollbackCompleted {
            from_height: height,
            to_height: height.saturating_sub(1),
            duration_ms: 42,
            success: true,
        },
        EventKind::ReorgExecuted => EventPayload::ReorgExecuted {
            old_tip_hash: format!("old_{}", height),
            new_tip_hash: format!("new_{}", height),
            rollback_depth: 1,
            applied_count: 2,
            weight_delta: 10,
            trigger_block_hash: format!("trigger_{}", height),
            trigger_from_peer_id: Some("12D3KooWReorg".to_string()),
        },
        EventKind::RecoveryClassifyCall => EventPayload::RecoveryClassifyCall {
            local_height: height,
            network_tip_height: height + 10,
            peer_count: 8,
            last_applied_secs: 30,
            shallow_rollback_count: 0,
            snap_attempts: 0,
            last_rollback_local_height: None,
            in_grace_period: false,
            last_finality_height: Some(height.saturating_sub(100)),
            action_returned: None,
            rule_matched: None,
        },
        EventKind::SnapSyncAttempted => EventPayload::SnapSyncAttempted {
            local_height: height,
            target_height: height + 500,
            source_peer_id: "12D3KooWSnap".to_string(),
        },
        EventKind::SnapSyncCompleted => EventPayload::SnapSyncCompleted {
            result: "success".to_string(),
            duration_ms: 12000,
        },
        EventKind::SnapSyncFailed => EventPayload::SnapSyncFailed {
            error: "peer disconnected".to_string(),
            duration_ms: 5000,
        },
        EventKind::ChainBreakDetected => EventPayload::ChainBreakDetected {
            expected_prev_hash: "expected_aabb".to_string(),
            actual_prev_hash: "actual_ccdd".to_string(),
            header_slot: height as u32,
            valid_so_far_count: 50,
            from_peer_id: "12D3KooWBreak".to_string(),
        },
        EventKind::WriterHeartbeat => EventPayload::WriterHeartbeat {
            events_written_total: 100,
            events_dropped_total: 0,
        },
    };

    DiagnosticEvent {
        event_id: ulid::Ulid::new().to_string(),
        kind,
        timestamp_ms,
        height: Some(height),
        correlation_key: None,
        caused_by_event_id: None,
        is_cascade_origin: false,
        payload,
    }
}

/// Build a DiagnosticEvent with a specific correlation key.
#[allow(dead_code)]
pub fn make_event_with_correlation(
    kind: EventKind,
    height: u64,
    timestamp_ms: u64,
    corr_key: CorrelationKey,
) -> DiagnosticEvent {
    let mut ev = make_event_with_ts(kind, height, timestamp_ms);
    ev.correlation_key = Some(corr_key);
    ev
}
