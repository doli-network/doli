// TDD RED PHASE — Workflow #346, Milestone M1
// All tests FAIL because production code does not exist yet.
//
// OUTPUT CONTRACT:
//   For DiagnosticLedger::open(data_dir):
//     O3: return — Result<DiagnosticLedger, Error>
//     O4: filesystem — creates <data_dir>/diagnostics/ RocksDB directory
//   For DiagnosticLedger::record(&event):
//     O3: return — Result<(), Error>
//     O4: RocksDB — event persisted to cf_events
//   For DiagnosticLedger::query_recent(duration_secs, limit):
//     O3: return — Vec<DiagnosticEvent> within time window, ordered oldest-first
//   For DiagnosticLedger::query_range(kind, min_h, max_h, limit):
//     O3: return — Vec<DiagnosticEvent> filtered by kind and height range
//   For DiagnosticLedger::prune(retention_secs, max_events):
//     O3: return — usize (number of events pruned)
//     O4: RocksDB — stale/excess events deleted; cascade-origin pins preserved
//
// INPUT PARTITIONS:
//   open(): valid tempdir | read-only dir (permission denied)
//   record(): valid event at various heights (0, mid, u64::MAX, None)
//   query_recent(): all-within-window | mixed fresh/stale | empty ledger
//   query_range(): matching kind+range | no matches | limit cap
//   prune():
//     P1: all fresh — 0 pruned
//     P2: mix fresh/stale — stale removed
//     P3: exceeds count cap, no pins — oldest removed
//     P4: exceeds count cap WITH cascade-origin pins — pins protected
//     P5: multiple correlation keys — all origins preserved
//     P6: all stale — all removed
//     P7: empty — 0, no error
//
// MATRIX: 7 open/record tests x 2 outputs + 3 query tests x 1 output
//   + 7 prune tests x 2-3 outputs = ~32 assertion cells across 17 tests

mod diagnostic_helpers;

use diagnostic_helpers::{make_event, make_event_with_correlation, make_event_with_ts, now_ms};
use storage::diagnostic_ledger::{
    emitter::{DiagnosticEmitter, NoOpEmitter},
    types::{CorrelationKey, EventKind},
    DiagnosticLedger,
};

// ===========================================================================
// Ledger open / record / query
// ===========================================================================

// Requirement: REQ-FORKOBS-LEDGER-001 (Must)
// Acceptance: open() creates <data_dir>/diagnostics/ as a RocksDB directory
#[test]
fn test_open_creates_rocksdb_directory() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let diag = dir.path().join("diagnostics");
    assert!(diag.exists(), "diagnostics/ must be created");
    assert!(diag.is_dir());
    assert!(
        diag.join("CURRENT").exists() || diag.join("IDENTITY").exists(),
        "must contain RocksDB marker"
    );
    drop(ledger);
}

// Requirement: REQ-FORKOBS-LEDGER-001 (Must)
// Acceptance: separate from state_db
#[test]
fn test_open_is_separate_from_state_db() {
    let dir = tempfile::tempdir().unwrap();
    let sdb = dir.path().join("state_db");
    std::fs::create_dir_all(&sdb).unwrap();

    let _ledger = DiagnosticLedger::open(dir.path()).expect("open");
    assert!(dir.path().join("diagnostics").exists());
    assert!(!sdb.join("diagnostics").exists(), "not nested in state_db");
}

// Requirement: REQ-FORKOBS-LEDGER-009 (Must)
// Acceptance: open() on read-only dir => Err, not panic
#[test]
fn test_open_fails_gracefully_on_permission_error() {
    let dir = tempfile::tempdir().unwrap();
    let ro = dir.path().join("readonly");
    std::fs::create_dir_all(&ro).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o444)).unwrap();
    }

    let result = DiagnosticLedger::open(&ro);
    assert!(result.is_err(), "open on read-only must return Err");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

// Requirement: REQ-FORKOBS-LEDGER-006, LEDGER-008 (Must)
// Acceptance: record 3 events, query_recent returns them ordered
#[test]
fn test_record_and_query_recent_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let ts = now_ms();
    let ev1 = make_event_with_ts(EventKind::BlockApplied, 10, ts);
    let ev2 = make_event_with_ts(EventKind::BlockApplied, 11, ts + 1);
    let ev3 = make_event_with_ts(EventKind::ForkBlockReceived, 11, ts + 2);
    let (id1, id2, id3) = (
        ev1.event_id.clone(),
        ev2.event_id.clone(),
        ev3.event_id.clone(),
    );

    ledger.record(&ev1).unwrap();
    ledger.record(&ev2).unwrap();
    ledger.record(&ev3).unwrap();

    let r = ledger.query_recent(3600, 100).expect("query_recent");
    assert_eq!(r.len(), 3, "all 3 events returned");
    assert_eq!(r[0].event_id, id1);
    assert_eq!(r[1].event_id, id2);
    assert_eq!(r[2].event_id, id3);
}

// Requirement: REQ-FORKOBS-LEDGER-007 (Must)
// Acceptance: query_range filters by kind and height range
#[test]
fn test_record_and_query_range_by_kind() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let ts = now_ms();
    ledger
        .record(&make_event_with_ts(EventKind::BlockApplied, 50, ts))
        .unwrap();
    ledger
        .record(&make_event_with_ts(EventKind::BlockRejected, 60, ts + 1))
        .unwrap();
    ledger
        .record(&make_event_with_ts(EventKind::BlockApplied, 70, ts + 2))
        .unwrap();
    ledger
        .record(&make_event_with_ts(EventKind::BlockApplied, 150, ts + 3))
        .unwrap();

    let r = ledger
        .query_range(Some(EventKind::BlockApplied), 40, 80, 100)
        .expect("query");
    assert_eq!(r.len(), 2, "only 2 BlockApplied in [40,80]");
    for ev in &r {
        assert_eq!(ev.kind, EventKind::BlockApplied);
        let h = ev.height.unwrap();
        assert!((40..=80).contains(&h), "height {} out of range", h);
    }
}

// Requirement: REQ-FORKOBS-LEDGER-008 (Must)
// Acceptance: query_recent respects time window
#[test]
fn test_query_recent_respects_window() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let now = now_ms();
    let ev_recent = make_event_with_ts(EventKind::BlockApplied, 100, now - 30 * 60_000);
    let ev_old = make_event_with_ts(EventKind::BlockApplied, 50, now - 2 * 3_600_000);
    let recent_id = ev_recent.event_id.clone();

    ledger.record(&ev_old).unwrap();
    ledger.record(&ev_recent).unwrap();

    let r = ledger.query_recent(3600, 100).expect("query");
    assert_eq!(r.len(), 1, "only recent event in 1h window");
    assert_eq!(r[0].event_id, recent_id);
}

// Requirement: REQ-FORKOBS-SEC-003 prerequisite (Must)
// Acceptance: query_range limit caps results
#[test]
fn test_query_range_respects_limit_cap() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let ts = now_ms();
    for i in 0..100u64 {
        ledger
            .record(&make_event_with_ts(EventKind::BlockApplied, i, ts + i))
            .unwrap();
    }

    let r = ledger.query_range(None, 0, 200, 10).expect("query");
    assert_eq!(r.len(), 10, "limit=10 must cap results");
}

// Requirement: REQ-FORKOBS-LEDGER-009 (Must)
// Acceptance: NoOpEmitter::record returns Ok, stores nothing
#[test]
fn test_record_to_degraded_ledger_is_noop() {
    let emitter = NoOpEmitter;
    let ev = make_event(EventKind::BlockApplied, 1);
    assert!(emitter.record(ev).is_ok(), "NoOpEmitter must return Ok");
}

// -- Edge cases for record/query --

// REQ-FORKOBS-LEDGER-006: height=0 event
#[test]
fn test_record_event_at_height_zero() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let ev = make_event(EventKind::BlockApplied, 0);
    let eid = ev.event_id.clone();
    ledger.record(&ev).unwrap();

    let r = ledger
        .query_range(Some(EventKind::BlockApplied), 0, 0, 10)
        .unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].height, Some(0));
    assert_eq!(r[0].event_id, eid);
}

// REQ-FORKOBS-LEDGER-002: height=u64::MAX in BE key encoding
#[test]
fn test_record_event_at_max_height() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let ev = make_event(EventKind::BlockApplied, u64::MAX);
    ledger.record(&ev).unwrap();

    let r = ledger
        .query_range(Some(EventKind::BlockApplied), u64::MAX, u64::MAX, 10)
        .unwrap();
    assert_eq!(r.len(), 1);
}

// REQ-FORKOBS-LEDGER-006: height=None (WriterHeartbeat)
#[test]
fn test_record_event_with_none_height() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let mut ev = make_event(EventKind::WriterHeartbeat, 0);
    ev.height = None;
    ledger.record(&ev).unwrap();

    let r = ledger.query_recent(3600, 100).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].height, None);
}

// REQ-FORKOBS-LEDGER-001: persistence across close/reopen
#[test]
fn test_ledger_persistence_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let eid;
    {
        let ledger = DiagnosticLedger::open(dir.path()).unwrap();
        let ev = make_event(EventKind::BlockApplied, 42);
        eid = ev.event_id.clone();
        ledger.record(&ev).unwrap();
    }
    {
        let ledger = DiagnosticLedger::open(dir.path()).unwrap();
        let r = ledger.query_recent(3600, 100).unwrap();
        assert_eq!(r.len(), 1, "event survives close/reopen");
        assert_eq!(r[0].event_id, eid);
    }
}

// ===========================================================================
// Pruner
// ===========================================================================

// Requirement: REQ-FORKOBS-LEDGER-004 (Must)
// INPUT PARTITION P2: mix of fresh/stale; retention=30d
#[test]
fn test_prune_removes_age_expired_events() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let now = now_ms();
    let ev_old = make_event_with_ts(EventKind::BlockApplied, 1, now - 31 * 86_400_000);
    let ev_fresh = make_event_with_ts(EventKind::BlockApplied, 1000, now - 3_600_000);
    let old_id = ev_old.event_id.clone();

    ledger.record(&ev_old).unwrap();
    ledger.record(&ev_fresh).unwrap();

    let pruned = ledger.prune(30 * 86400, 100_000).expect("prune");
    assert_eq!(pruned, 1, "1 age-expired event pruned");

    let rem = ledger.query_recent(365 * 86400, 100).unwrap();
    assert_eq!(rem.len(), 1);
    assert_ne!(rem[0].event_id, old_id);
}

// Requirement: REQ-FORKOBS-LEDGER-005 (Must)
// INPUT PARTITION P3: 105 events, cap=100, no pins
#[test]
fn test_prune_respects_max_events_cap() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let ts = now_ms();
    for i in 0..105u64 {
        ledger
            .record(&make_event_with_ts(EventKind::BlockApplied, i, ts + i))
            .unwrap();
    }

    let pruned = ledger.prune(365 * 86400, 100).expect("prune");
    assert_eq!(pruned, 5, "5 oldest pruned to meet cap=100");

    let rem = ledger.query_recent(365 * 86400, 200).unwrap();
    assert_eq!(rem.len(), 100);
}

// Requirement: REQ-FORKOBS-LEDGER-004/005 + O3 cascade-origin pin (Must)
// INPUT PARTITION P4: 200 events, one correlation_key, cap=50
#[test]
fn test_prune_preserves_cascade_origin_pin() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let ts = now_ms();
    let ck = CorrelationKey {
        divergence_height: Some(100),
        canonical_hash: Some("aa".into()),
        fork_hash: Some("cc".into()),
    };

    let mut first_id = String::new();
    for i in 0..200u64 {
        let ev =
            make_event_with_correlation(EventKind::ForkBlockReceived, 100 + i, ts + i, ck.clone());
        if i == 0 {
            first_id = ev.event_id.clone();
        }
        ledger.record(&ev).unwrap();
    }

    let pruned = ledger.prune(365 * 86400, 50).expect("prune");
    assert!(pruned >= 149, "at least 149 pruned (200 - 50 - origin pin)");

    let rem = ledger.query_recent(365 * 86400, 500).unwrap();
    assert!(
        rem.iter().any(|e| e.event_id == first_id),
        "cascade origin must survive"
    );
}

// Requirement: O3 — multiple correlation keys
// INPUT PARTITION P5: 3 keys x 50 events = 150, cap=50
#[test]
fn test_prune_preserves_multiple_cascade_origins() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let ts = now_ms();
    let mut origin_ids = Vec::new();

    for ki in 0..3u64 {
        let ck = CorrelationKey {
            divergence_height: Some(100 + ki * 100),
            canonical_hash: Some(format!("c{}", ki)),
            fork_hash: Some(format!("f{}", ki)),
        };
        for i in 0..50u64 {
            let ev = make_event_with_correlation(
                EventKind::ForkBlockReceived,
                100 + ki * 100 + i,
                ts + ki * 1000 + i,
                ck.clone(),
            );
            if i == 0 {
                origin_ids.push(ev.event_id.clone());
            }
            ledger.record(&ev).unwrap();
        }
    }

    let pruned = ledger.prune(365 * 86400, 50).expect("prune");
    assert!(pruned >= 97, "at least 97 pruned");

    let rem = ledger.query_recent(365 * 86400, 500).unwrap();
    for oid in &origin_ids {
        assert!(
            rem.iter().any(|e| &e.event_id == oid),
            "origin {} must survive",
            oid
        );
    }
    assert!(rem.len() >= 3, "at least 3 origins remain");
}

// Requirement: REQ-FORKOBS-LEDGER-004 (Must)
// INPUT PARTITION P7: empty ledger
#[test]
fn test_prune_empty_ledger_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");
    assert_eq!(ledger.prune(3600, 100).expect("prune"), 0);
}

// Requirement: REQ-FORKOBS-LEDGER-004 (Must)
// INPUT PARTITION P6: all events stale
#[test]
fn test_prune_all_events_stale() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let ancient = now_ms() - 60 * 86_400_000;
    for i in 0..10u64 {
        ledger
            .record(&make_event_with_ts(EventKind::BlockApplied, i, ancient + i))
            .unwrap();
    }

    let pruned = ledger.prune(30 * 86400, 100_000).expect("prune");
    assert_eq!(pruned, 10, "all 10 stale events pruned");

    let rem = ledger.query_recent(365 * 86400, 100).unwrap();
    assert!(rem.is_empty(), "ledger empty after pruning all stale");
}
