// TDD RED PHASE — Workflow #346, Milestone M1
// All tests FAIL because production code does not exist yet.
//
// OUTPUT CONTRACT:
//   For DiagnosticLedger::serialize_event(event):
//     O3: return — Result<Vec<u8>, Error>
//     Byte layout: [0x01 format_marker][schema_version u16 LE][bincode payload]
//   For DiagnosticLedger::deserialize_event(bytes):
//     O3: return — Result<DiagnosticEvent, Error>
//     Rejects unknown schema versions with Err (not panic)
//   For DiagnosticLedger::event_key_bytes(event):
//     O3: return — Vec<u8> of exactly 25 bytes [kind_u8][height_u64_be][ulid_16]
//   For bincode round-trip of Classification, ForkType, CorrelationKey, etc.:
//     O3: decoded struct is identical to original
//
// INPUT PARTITIONS:
//   serialize/deserialize: each EventKind variant (12) | schema_version=1 OK |
//     schema_version=99 Err | BlockApplied event | WriterHeartbeat event
//   event_key_bytes: kind=1 height=100 | kind=12 height=0 | same-height ULID ordering
//   type round-trips: CorrelationKey all-None | CorrelationKey all-Some |
//     ForkType::Unknown with evidence | ForkType named variants (8) |
//     Classification with recommended_action | DiagnosticBundle empty events |
//     DiagnosticBundle with classification | BlockProvenance with/without peer
//
// MATRIX: ~30 output cells across 15 tests

mod diagnostic_helpers;

use std::collections::HashMap;
use std::time::Duration;

use diagnostic_helpers::{make_event, now_ms};
use storage::diagnostic_ledger::{
    types::{
        BaselineComparison, BlockProvenance, Classification, CorrelationKey, DiagnosticBundle,
        DiagnosticHealth, EventKind, ForkSummary, ForkType,
    },
    DiagnosticLedger,
};

// -- Key format tests (REQ-FORKOBS-LEDGER-002) --

// Requirement: REQ-FORKOBS-LEDGER-002 (Must)
// Acceptance: key = [event_kind_u8][height_u64_be][ulid_16_bytes] = 25 bytes
#[test]
fn test_event_key_composite_format() {
    let ev = make_event(EventKind::BlockApplied, 100);
    let key = DiagnosticLedger::event_key_bytes(&ev);

    assert_eq!(key.len(), 25, "composite key must be exactly 25 bytes");
    assert_eq!(
        key[0],
        EventKind::BlockApplied as u8,
        "first byte is event kind"
    );

    let height_bytes: [u8; 8] = key[1..9].try_into().unwrap();
    assert_eq!(
        u64::from_be_bytes(height_bytes),
        100,
        "bytes 1..9 = height BE"
    );
    assert_eq!(key[9..].len(), 16, "remaining 16 bytes are ULID");
}

// Requirement: REQ-FORKOBS-LEDGER-002 (Must)
// Acceptance: events of same kind sort by height then ULID
#[test]
fn test_event_key_ordering_within_kind() {
    let ev_low = make_event(EventKind::BlockApplied, 50);
    let ev_high = make_event(EventKind::BlockApplied, 200);
    let key_low = DiagnosticLedger::event_key_bytes(&ev_low);
    let key_high = DiagnosticLedger::event_key_bytes(&ev_high);

    assert!(
        key_low < key_high,
        "lower height key must sort before higher"
    );

    let ev_a = make_event(EventKind::BlockApplied, 100);
    std::thread::sleep(Duration::from_millis(2));
    let ev_b = make_event(EventKind::BlockApplied, 100);
    let key_a = DiagnosticLedger::event_key_bytes(&ev_a);
    let key_b = DiagnosticLedger::event_key_bytes(&ev_b);

    assert!(
        key_a < key_b,
        "earlier ULID must sort before later at same height"
    );
}

// -- Serialization round-trip tests (REQ-FORKOBS-LEDGER-003) --

// Requirement: REQ-FORKOBS-LEDGER-003 (Must)
// Acceptance: bincode round-trip preserves BlockApplied event; format marker present
#[test]
fn test_event_bincode_roundtrip_block_applied() {
    let ev = make_event(EventKind::BlockApplied, 42);
    let bytes = DiagnosticLedger::serialize_event(&ev).expect("serialize");
    let decoded = DiagnosticLedger::deserialize_event(&bytes).expect("deserialize");

    assert_eq!(decoded.event_id, ev.event_id);
    assert_eq!(decoded.kind, ev.kind);
    assert_eq!(decoded.timestamp_ms, ev.timestamp_ms);
    assert_eq!(decoded.height, ev.height);
    assert_eq!(bytes[0], 0x01, "first byte must be format marker 0x01");
}

// Requirement: REQ-FORKOBS-LEDGER-003 (Must)
// Acceptance: every EventKind variant round-trips correctly (12 variants)
#[test]
fn test_event_bincode_roundtrip_all_kinds() {
    let kinds = [
        EventKind::BlockApplied,
        EventKind::BlockRejected,
        EventKind::ForkBlockReceived,
        EventKind::RollbackStarted,
        EventKind::RollbackCompleted,
        EventKind::ReorgExecuted,
        EventKind::RecoveryClassifyCall,
        EventKind::SnapSyncAttempted,
        EventKind::SnapSyncCompleted,
        EventKind::SnapSyncFailed,
        EventKind::ChainBreakDetected,
        EventKind::WriterHeartbeat,
    ];

    for kind in kinds {
        let ev = make_event(kind, 100);
        let bytes =
            DiagnosticLedger::serialize_event(&ev).unwrap_or_else(|e| panic!("{:?}: {}", kind, e));
        let decoded = DiagnosticLedger::deserialize_event(&bytes)
            .unwrap_or_else(|e| panic!("{:?}: {}", kind, e));

        assert_eq!(decoded.event_id, ev.event_id, "{:?} event_id", kind);
        assert_eq!(decoded.kind, ev.kind, "{:?} kind", kind);
        assert_eq!(decoded.height, ev.height, "{:?} height", kind);
    }
}

// -- Schema version tests (REQ-FORKOBS-LEDGER-003 — migration bomb mitigation) --

// Requirement: REQ-FORKOBS-LEDGER-003 (Must)
// Acceptance: format marker 0x01 at byte 0
#[test]
fn test_format_marker_byte_present() {
    let ev = make_event(EventKind::RollbackStarted, 500);
    let bytes = DiagnosticLedger::serialize_event(&ev).expect("serialize");
    assert_eq!(bytes[0], 0x01, "byte 0 must be format-marker 0x01");
}

// Requirement: REQ-FORKOBS-LEDGER-003 (Must)
// Acceptance: schema_version u16 at bytes[1..3] == 1
#[test]
fn test_schema_version_present_and_current() {
    let ev = make_event(EventKind::BlockApplied, 1);
    let bytes = DiagnosticLedger::serialize_event(&ev).expect("serialize");
    assert!(bytes.len() >= 3, "at least 3 bytes (marker + version)");

    let ver = u16::from_le_bytes(bytes[1..3].try_into().unwrap());
    assert_eq!(ver, 1, "current schema version must be 1");
}

// Requirement: REQ-FORKOBS-LEDGER-003 (Must)
// Acceptance: future schema version => Err, not panic
#[test]
fn test_decoder_rejects_unknown_future_schema_version() {
    let ev = make_event(EventKind::BlockApplied, 1);
    let mut bytes = DiagnosticLedger::serialize_event(&ev).expect("serialize");
    bytes[1..3].copy_from_slice(&99u16.to_le_bytes());

    let result = DiagnosticLedger::deserialize_event(&bytes);
    assert!(result.is_err(), "future schema_version must return Err");
}

// Requirement: REQ-FORKOBS-LEDGER-003 (Must)
// Acceptance: current version deserializes successfully
#[test]
fn test_decoder_accepts_current_schema_version() {
    let ev = make_event(EventKind::WriterHeartbeat, 0);
    let bytes = DiagnosticLedger::serialize_event(&ev).expect("serialize");
    let decoded = DiagnosticLedger::deserialize_event(&bytes);
    assert!(decoded.is_ok(), "schema_version=1 must deserialize");
    assert_eq!(decoded.unwrap().kind, EventKind::WriterHeartbeat);
}

// -- Type-level round-trip tests --

// Requirement: REQ-FORKOBS-RETRO-003 (Must)
// Acceptance: Unknown variant carries reason_unknown + evidence_event_ids
#[test]
fn test_classification_unknown_variant_carries_evidence() {
    let clf = Classification {
        fork_type: ForkType::Unknown {
            reason_unknown: "recovery repeating HeaderFirstSync".to_string(),
            evidence_event_ids: vec!["ev1".into(), "ev2".into(), "ev3".into()],
        },
        confidence: 0.3,
        evidence_event_ids: vec!["ev1".into()],
        recommended_action: None,
        recommended_action_args: None,
    };

    let bytes = bincode::serialize(&clf).expect("serialize");
    let decoded: Classification = bincode::deserialize(&bytes).expect("deserialize");

    match &decoded.fork_type {
        ForkType::Unknown {
            reason_unknown,
            evidence_event_ids,
        } => {
            assert_eq!(reason_unknown, "recovery repeating HeaderFirstSync");
            assert_eq!(evidence_event_ids.len(), 3);
            assert_eq!(evidence_event_ids[2], "ev3");
        }
        other => panic!("expected Unknown, got {:?}", std::mem::discriminant(other)),
    }
    assert!((decoded.confidence - 0.3).abs() < f64::EPSILON);
}

// Requirement: REQ-FORKOBS-EMIT-010 (Should)
// Acceptance: canonical block has all-None correlation key
#[test]
fn test_correlation_key_canonical_block_has_all_none() {
    let corr = CorrelationKey {
        divergence_height: None,
        canonical_hash: None,
        fork_hash: None,
    };
    assert!(corr.divergence_height.is_none());
    assert!(corr.canonical_hash.is_none());
    assert!(corr.fork_hash.is_none());

    let bytes = bincode::serialize(&corr).expect("serialize");
    let decoded: CorrelationKey = bincode::deserialize(&bytes).expect("deserialize");
    assert!(decoded.divergence_height.is_none());
    assert!(decoded.canonical_hash.is_none());
    assert!(decoded.fork_hash.is_none());
}

// Requirement: REQ-FORKOBS-EMIT-010
// Acceptance: populated CorrelationKey round-trips
#[test]
fn test_correlation_key_full_roundtrip() {
    let key = CorrelationKey {
        divergence_height: Some(12345),
        canonical_hash: Some("aabb00112233".into()),
        fork_hash: Some("ccdd44556677".into()),
    };
    let bytes = bincode::serialize(&key).expect("serialize");
    let decoded: CorrelationKey = bincode::deserialize(&bytes).expect("deserialize");
    assert_eq!(decoded.divergence_height, Some(12345));
    assert_eq!(decoded.canonical_hash, Some("aabb00112233".into()));
    assert_eq!(decoded.fork_hash, Some("ccdd44556677".into()));
}

// Requirement: REQ-FORKOBS-EMIT-001 — BlockProvenance round-trip
#[test]
fn test_block_provenance_roundtrip() {
    let with_peer = BlockProvenance {
        from_peer_id: Some("12D3KooW".into()),
        received_at_ms: 17160,
    };
    let without = BlockProvenance {
        from_peer_id: None,
        received_at_ms: 17160,
    };

    let d: BlockProvenance =
        bincode::deserialize(&bincode::serialize(&with_peer).unwrap()).unwrap();
    assert_eq!(d.from_peer_id, Some("12D3KooW".into()));
    assert_eq!(d.received_at_ms, 17160);

    let d: BlockProvenance = bincode::deserialize(&bincode::serialize(&without).unwrap()).unwrap();
    assert!(d.from_peer_id.is_none());
}

// Requirement: REQ-FORKOBS-CLF-002 — all ForkType variants round-trip
#[test]
fn test_fork_type_all_variants_roundtrip() {
    let variants: Vec<ForkType> = vec![
        ForkType::TipRaceNatural,
        ForkType::TipRaceHighLatency,
        ForkType::ProducerEquivocation,
        ForkType::EpochBoundaryInvalid,
        ForkType::PostSnapDeadTip,
        ForkType::ValidationDisagreement,
        ForkType::RollbackLoop,
        ForkType::SnapSyncToMinorityFork,
        ForkType::Unknown {
            reason_unknown: "test".into(),
            evidence_event_ids: vec!["e".into()],
        },
    ];
    for v in &variants {
        let bytes = bincode::serialize(v).expect("serialize");
        let d: ForkType = bincode::deserialize(&bytes).expect("deserialize");
        assert_eq!(std::mem::discriminant(v), std::mem::discriminant(&d));
    }
}

// Requirement: REQ-FORKOBS-LEDGER-003 — DiagnosticBundle empty round-trip
#[test]
fn test_diagnostic_bundle_roundtrip_empty_events() {
    let bundle = DiagnosticBundle {
        schema_version: 1,
        node_peer_id: "12D3KooWTest".into(),
        query_timestamp_ms: now_ms(),
        events: vec![],
        fork_summary: ForkSummary {
            fork_events_in_window: 0,
            by_producer: HashMap::new(),
            by_event_kind: HashMap::new(),
            first_fork_height: None,
            last_fork_height: None,
        },
        classification: None,
        baseline: BaselineComparison {
            fork_events_per_hour_current: 0.0,
            fork_events_per_hour_24h_avg: 0.0,
            delta_pct: 0.0,
        },
        health: DiagnosticHealth {
            ledger_available: true,
            events_written_total: 0,
            events_dropped_total: 0,
            last_heartbeat_ms: None,
        },
    };
    let d: DiagnosticBundle = bincode::deserialize(&bincode::serialize(&bundle).unwrap()).unwrap();
    assert_eq!(d.schema_version, 1);
    assert!(d.events.is_empty());
    assert!(d.classification.is_none());
    assert!(d.health.last_heartbeat_ms.is_none());
}

// Requirement: REQ-FORKOBS-LEDGER-003 — DiagnosticBundle with classification round-trip
#[test]
fn test_diagnostic_bundle_roundtrip_with_classification() {
    let bundle = DiagnosticBundle {
        schema_version: 1,
        node_peer_id: "12D3KooWProd".into(),
        query_timestamp_ms: now_ms(),
        events: vec![make_event(EventKind::BlockApplied, 100)],
        fork_summary: ForkSummary {
            fork_events_in_window: 1,
            by_producer: {
                let mut m = HashMap::new();
                m.insert("aabb".into(), 1);
                m
            },
            by_event_kind: {
                let mut m = HashMap::new();
                m.insert("BlockApplied".into(), 1);
                m
            },
            first_fork_height: Some(100),
            last_fork_height: Some(100),
        },
        classification: Some(Classification {
            fork_type: ForkType::TipRaceNatural,
            confidence: 0.85,
            evidence_event_ids: vec!["ev1".into()],
            recommended_action: Some("normal_operation".into()),
            recommended_action_args: None,
        }),
        baseline: BaselineComparison {
            fork_events_per_hour_current: 2.5,
            fork_events_per_hour_24h_avg: 1.0,
            delta_pct: 150.0,
        },
        health: DiagnosticHealth {
            ledger_available: true,
            events_written_total: 42,
            events_dropped_total: 0,
            last_heartbeat_ms: Some(now_ms()),
        },
    };
    let d: DiagnosticBundle = bincode::deserialize(&bincode::serialize(&bundle).unwrap()).unwrap();
    assert_eq!(d.events.len(), 1);
    let clf = d.classification.unwrap();
    assert!(matches!(clf.fork_type, ForkType::TipRaceNatural));
    assert!((clf.confidence - 0.85).abs() < f64::EPSILON);
    assert_eq!(d.fork_summary.fork_events_in_window, 1);
    assert_eq!(d.health.events_written_total, 42);
}
