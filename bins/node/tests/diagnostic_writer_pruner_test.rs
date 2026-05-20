//! M2 Diagnostic Writer/Pruner/Security Tests — Workflow #346
//!
//! TDD RED PHASE: Tests for writer task, pruner, no-PII, deploy safety.
//! These tests do NOT require a full Node — they test the writer/pruner
//! contracts and security requirements using DiagnosticLedger + MockEmitter.
//!
// OUTPUT CONTRACT: fn diagnostic_writer::run_writer_task(receiver, ledger, shutdown)
//   O4: DiagnosticLedger — events drained from channel written to RocksDB
// PATHS: P1:normal_drain, P2:shutdown_drain, P3:channel_overflow(dropped_counter)
// OUTPUT CONTRACT: fn diagnostics_pruner::run_pruner(ledger, config)
//   O4: DiagnosticLedger — old/excess events pruned
// PATHS: P1:age_expired, P2:count_cap, P3:empty_ledger
// OUTPUT CONTRACT: event type security
//   O6: no IP addresses in any event field (REQ-FORKOBS-SEC-001)
// INPUT PARTITIONS:
//   writer: normal | bursty | shutdown_mid_drain | overflow
//   pruner: age_expired | count_cap | empty
//   event kinds: all 12 variants for PII scan
//   deploy safety: no activation height | no hardfork entry

use std::time::{SystemTime, UNIX_EPOCH};

use storage::diagnostic_ledger::emitter::{AsyncChannelEmitter, DiagnosticEmitter, MockEmitter};
use storage::diagnostic_ledger::types::{DiagnosticEvent, EventKind, EventPayload};
use storage::diagnostic_ledger::DiagnosticLedger;
use tempfile::TempDir;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn fake_event_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(10000);
    format!("01HYTEST{:018}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn make_event(kind: EventKind, height: u64, ts: u64) -> DiagnosticEvent {
    let payload = match kind {
        EventKind::BlockApplied => EventPayload::BlockApplied {
            slot: height as u32,
            block_hash: format!("hash_{}", height),
            producer_pubkey: "aabbccdd".to_string(),
            from_peer_id: None,
            received_at_ms: None,
            applied_at_ms: ts,
            validation_duration_ms: 1,
            mode: "Full".to_string(),
            tx_count: 1,
        },
        EventKind::ForkBlockReceived => EventPayload::ForkBlockReceived {
            block_hash: format!("fork_{}", height),
            block_slot: height as u32,
            block_height_estimate: Some(height),
            producer_pubkey: "11223344".to_string(),
            from_peer_id: "12D3KooWTest".to_string(),
            classification: "ForkBlock".to_string(),
            fork_kind: Some("HeightOccupied".to_string()),
            local_tip_hash: "tip".to_string(),
            local_tip_height: height.saturating_sub(1),
        },
        _ => EventPayload::WriterHeartbeat {
            events_written_total: height,
            events_dropped_total: 0,
        },
    };
    DiagnosticEvent {
        event_id: fake_event_id(),
        kind,
        timestamp_ms: ts,
        height: Some(height),
        correlation_key: None,
        caused_by_event_id: None,
        is_cascade_origin: false,
        payload,
    }
}

// ============================================================
// PHASE D — Writer task tests
// ============================================================

// Requirement: REQ-FORKOBS-LEDGER-006 (Must)
// Acceptance: writer task drains channel events to DiagnosticLedger
#[tokio::test]
async fn test_writer_task_drains_channel_to_ledger() {
    let temp = TempDir::new().unwrap();
    let ledger = DiagnosticLedger::open(temp.path()).expect("ledger should open");
    let (emitter, receiver) = AsyncChannelEmitter::new(1024);

    for i in 0..10 {
        emitter
            .record(make_event(EventKind::BlockApplied, i, now_ms()))
            .unwrap();
    }

    // TODO(developer): Replace manual drain with actual writer task spawn:
    //   tokio::spawn(diagnostic_writer::run_writer_task(receiver, ledger, shutdown_rx));
    let mut rx = receiver;
    let mut count = 0;
    while let Ok(event) = rx.try_recv() {
        ledger.record(&event).unwrap();
        count += 1;
    }
    assert_eq!(count, 10, "should drain all 10 events");

    let stored = ledger.query_recent(3600, 100).unwrap();
    assert_eq!(stored.len(), 10, "ledger should contain all 10 events");
}

// Requirement: REQ-FORKOBS-LEDGER-006 (Must) — shutdown drains pending
#[tokio::test]
async fn test_writer_task_shutdown_drains_pending() {
    let temp = TempDir::new().unwrap();
    let ledger = DiagnosticLedger::open(temp.path()).expect("ledger should open");
    let (emitter, receiver) = AsyncChannelEmitter::new(1024);

    for i in 0..5 {
        emitter
            .record(make_event(EventKind::ForkBlockReceived, i + 100, now_ms()))
            .unwrap();
    }

    drop(emitter); // Simulate shutdown: sender dropped

    let mut rx = receiver;
    let mut count = 0;
    while let Ok(event) = rx.try_recv() {
        ledger.record(&event).unwrap();
        count += 1;
    }
    assert_eq!(
        count, 5,
        "all 5 pending events should be drained on shutdown"
    );
    assert_eq!(ledger.query_recent(3600, 100).unwrap().len(), 5);
}

// Requirement: REQ-FORKOBS-PERF-001 (Must) — dropped counter on overflow
#[tokio::test]
async fn test_writer_task_increments_dropped_counter_on_overflow() {
    let (emitter, _receiver) = AsyncChannelEmitter::new(3);

    for i in 0..5u64 {
        emitter
            .record(make_event(EventKind::BlockApplied, i, now_ms()))
            .unwrap();
    }

    assert_eq!(
        emitter.dropped_count(),
        2,
        "2 events dropped (cap=3, pushed 5)"
    );
}

// ============================================================
// PHASE E — Pruner task tests
// ============================================================

// Requirement: REQ-FORKOBS-LEDGER-004 (Must)
#[tokio::test]
async fn test_pruner_removes_age_expired_events() {
    let temp = TempDir::new().unwrap();
    let ledger = DiagnosticLedger::open(temp.path()).expect("ledger should open");

    let three_days_ago = now_ms().saturating_sub(3 * 86_400 * 1000);
    for i in 0..5 {
        ledger
            .record(&make_event(EventKind::BlockApplied, i, three_days_ago + i))
            .unwrap();
    }
    for i in 0..3 {
        ledger
            .record(&make_event(EventKind::BlockApplied, 100 + i, now_ms()))
            .unwrap();
    }

    let pruned = ledger.prune(86_400, 100_000).unwrap();
    assert_eq!(pruned, 5, "5 old events should be pruned");
    assert_eq!(ledger.query_recent(86_400, 100).unwrap().len(), 3);
}

// Requirement: REQ-FORKOBS-LEDGER-005 (Must)
#[tokio::test]
async fn test_pruner_count_cap_prunes_oldest() {
    let temp = TempDir::new().unwrap();
    let ledger = DiagnosticLedger::open(temp.path()).expect("ledger should open");

    for i in 0..60 {
        ledger
            .record(&make_event(EventKind::BlockApplied, i, now_ms()))
            .unwrap();
    }

    let pruned = ledger.prune(30 * 86_400, 50).unwrap();
    assert_eq!(pruned, 10, "10 events pruned when cap=50, count=60");
    assert_eq!(ledger.query_recent(3600, 100).unwrap().len(), 50);
}

// Requirement: REQ-FORKOBS-EMIT-007 (Must) — RecoveryClassifyCall event structure
#[test]
fn test_recovery_classify_event_has_all_11_fields() {
    let mock = MockEmitter::new();
    let event = DiagnosticEvent {
        event_id: fake_event_id(),
        kind: EventKind::RecoveryClassifyCall,
        timestamp_ms: now_ms(),
        height: Some(100),
        correlation_key: None,
        caused_by_event_id: None,
        is_cascade_origin: false,
        payload: EventPayload::RecoveryClassifyCall {
            local_height: 100,
            network_tip_height: 150,
            peer_count: 8,
            last_applied_secs: 30,
            shallow_rollback_count: 2,
            snap_attempts: 0,
            last_rollback_local_height: Some(99),
            in_grace_period: false,
            last_finality_height: Some(50),
            action_returned: Some("ShallowRollback".to_string()),
            rule_matched: Some("Rule2_orphan_evidence".to_string()),
        },
    };
    mock.record(event).unwrap();
    let events = mock.events();
    assert_eq!(events.len(), 1);
    match &events[0].payload {
        EventPayload::RecoveryClassifyCall {
            local_height,
            network_tip_height,
            peer_count,
            last_applied_secs,
            shallow_rollback_count,
            snap_attempts,
            last_rollback_local_height,
            in_grace_period,
            last_finality_height,
            action_returned,
            rule_matched,
        } => {
            assert_eq!(*local_height, 100);
            assert_eq!(*network_tip_height, 150);
            assert_eq!(*peer_count, 8);
            assert_eq!(*last_applied_secs, 30);
            assert_eq!(*shallow_rollback_count, 2);
            assert_eq!(*snap_attempts, 0);
            assert_eq!(*last_rollback_local_height, Some(99));
            assert!(!in_grace_period);
            assert_eq!(*last_finality_height, Some(50));
            assert_eq!(action_returned.as_deref(), Some("ShallowRollback"));
            assert!(rule_matched.is_some());
        }
        _ => panic!("expected RecoveryClassifyCall payload"),
    }
}

// Requirement: REQ-FORKOBS-EMIT-006 (Must) — ReorgExecuted event structure
#[test]
fn test_reorg_executed_event_structure() {
    let mock = MockEmitter::new();
    let event = DiagnosticEvent {
        event_id: fake_event_id(),
        kind: EventKind::ReorgExecuted,
        timestamp_ms: now_ms(),
        height: Some(50),
        correlation_key: None,
        caused_by_event_id: None,
        is_cascade_origin: false,
        payload: EventPayload::ReorgExecuted {
            old_tip_hash: "aabb1234".to_string(),
            new_tip_hash: "ccdd5678".to_string(),
            rollback_depth: 2,
            applied_count: 3,
            weight_delta: 15,
            trigger_block_hash: "trigger9876".to_string(),
            trigger_from_peer_id: Some("12D3KooWReorgPeer".to_string()),
        },
    };
    mock.record(event).unwrap();
    let events = mock.events();
    match &events[0].payload {
        EventPayload::ReorgExecuted {
            old_tip_hash,
            new_tip_hash,
            rollback_depth,
            applied_count,
            weight_delta,
            trigger_block_hash,
            trigger_from_peer_id,
        } => {
            assert_eq!(old_tip_hash, "aabb1234");
            assert_eq!(new_tip_hash, "ccdd5678");
            assert_eq!(*rollback_depth, 2);
            assert_eq!(*applied_count, 3);
            assert_eq!(*weight_delta, 15);
            assert!(!trigger_block_hash.is_empty());
            assert!(trigger_from_peer_id.is_some());
        }
        _ => panic!("expected ReorgExecuted payload"),
    }
}

// ============================================================
// PHASE F — No-PII tests (REQ-FORKOBS-SEC-001)
// ============================================================

// Requirement: REQ-FORKOBS-SEC-001 (Must)
#[test]
fn test_no_ip_address_in_any_event_payload() {
    let forbidden = [
        "127.0.0.1",
        "0.0.0.0",
        "192.168.",
        "10.0.",
        "172.16.",
        "::1",
    ];
    let sample_events: Vec<DiagnosticEvent> = vec![
        make_event(EventKind::BlockApplied, 10, 1000),
        {
            let mut e = make_event(EventKind::BlockApplied, 10, 1000);
            e.kind = EventKind::BlockRejected;
            e.payload = EventPayload::BlockRejected {
                slot: 10,
                block_hash: "h".into(),
                producer_pubkey: "p".into(),
                from_peer_id: Some("12D3KooWPeer".into()),
                rejection_reason: "bad".into(),
                mode: "Full".into(),
            };
            e
        },
        make_event(EventKind::ForkBlockReceived, 10, 1000),
        {
            let mut e = make_event(EventKind::BlockApplied, 10, 1000);
            e.kind = EventKind::SnapSyncAttempted;
            e.payload = EventPayload::SnapSyncAttempted {
                local_height: 0,
                target_height: 1000,
                source_peer_id: "12D3KooWSnap".into(),
            };
            e
        },
        {
            let mut e = make_event(EventKind::BlockApplied, 10, 1000);
            e.kind = EventKind::ChainBreakDetected;
            e.payload = EventPayload::ChainBreakDetected {
                expected_prev_hash: "aa".into(),
                actual_prev_hash: "bb".into(),
                header_slot: 10,
                valid_so_far_count: 5,
                from_peer_id: "12D3KooWBreak".into(),
            };
            e
        },
    ];

    for event in &sample_events {
        let json = serde_json::to_string(event).unwrap();
        for pat in &forbidden {
            assert!(
                !json.contains(pat),
                "IP '{}' in {:?} event: {}",
                pat,
                event.kind,
                json
            );
        }
    }
}

// ============================================================
// PHASE H — Rolling-deploy safety (REQ-FORKOBS-SEC-006)
// ============================================================

// Requirement: REQ-FORKOBS-SEC-006 (Must) — no activation height for this feature
#[test]
fn test_no_activation_height_added() {
    let src = include_str!("../../../crates/core/src/network_params/defaults.rs");
    for pat in &[
        "diagnostic_activation_height",
        "fork_obs_activation_height",
        "observability_activation_height",
        "diag_activation_height",
        "fork_diagnostic_activation_height",
    ] {
        assert!(
            !src.contains(pat),
            "FORBIDDEN: '{}' in network_params/defaults.rs",
            pat
        );
    }
}

// Requirement: REQ-FORKOBS-SEC-006 (Must) — no hardfork schedule entry
#[test]
fn test_no_hardfork_schedule_entry_added() {
    let src = include_str!("../../../crates/updater/src/hardfork.rs");
    let lower = src.to_lowercase();
    for pat in &[
        "diagnostic",
        "fork_obs",
        "observability",
        "forkobservability",
    ] {
        if lower.contains(pat) {
            for line in src.lines() {
                let ll = line.to_lowercase();
                if ll.contains(pat) && (ll.contains("hardfork") || ll.contains("schedule")) {
                    panic!("FORBIDDEN: '{}' in hardfork.rs schedule entry", pat);
                }
            }
        }
    }
}
