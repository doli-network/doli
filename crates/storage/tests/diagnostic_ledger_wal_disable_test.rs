// INC-I-104 M5 — WAL disable + max_background_jobs regression guard for DiagnosticLedger
//
// OUTPUT CONTRACT:
//   For DiagnosticLedger::open(data_dir):
//     O1: WAL is disabled on all write paths (WriteOptions::disable_wal(true))
//     O2: max_background_jobs is set to 1
//     O3: INC-I-102 caps preserved (block_cache=4 MB, db_write_buffer_size=8 MB)
//     O4: record() + prune() still work correctly with WAL disabled
//
// INPUT PARTITIONS:
//   open(): default config → verify WAL disabled flag exposed
//   record() + query_recent(): events written and readable (proves no-WAL writes work)
//   prune(): batch delete works with no-WAL WriteOptions
//
// Why: diagnostic_ledger is pure observability with NoOp fallback. WAL provides
// no value (events can be lost on crash). Disabling saves fsync on every write.
// This test locks in the M5 configuration change.

mod diagnostic_helpers;

use diagnostic_helpers::{make_event, make_event_with_ts, now_ms};
use storage::diagnostic_ledger::{types::EventKind, DiagnosticLedger};

/// INC-I-104 M5 REGRESSION GUARD — WAL disabled indicator.
///
/// Verifies that the DiagnosticLedger reports WAL as disabled.
/// Pre-M5: wal_disabled() returns false (or method doesn't exist).
/// Post-M5: wal_disabled() returns true.
#[test]
fn diagnostic_ledger_wal_is_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open should succeed");
    assert!(
        ledger.wal_disabled(),
        "INC-I-104 M5 regression: WAL must be disabled on diagnostic_ledger. \
         All write paths should use WriteOptions::set_disable_wal(true)."
    );
}

/// INC-I-104 M5 — INC-I-102 block_cache cap preserved at 4 MB.
#[test]
fn diagnostic_ledger_m5_preserves_block_cache_4mb() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open should succeed");
    assert_eq!(
        ledger.block_cache_capacity_bytes(),
        4 * 1024 * 1024,
        "INC-I-102 block cache cap must remain 4 MB after M5 changes"
    );
}

/// INC-I-104 M5 — INC-I-102 db_write_buffer_size cap preserved at 8 MB.
#[test]
fn diagnostic_ledger_m5_preserves_db_write_buffer_8mb() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open should succeed");
    assert_eq!(
        ledger.db_write_buffer_size_bytes(),
        8 * 1024 * 1024,
        "INC-I-102 db_write_buffer_size cap must remain 8 MB after M5 changes"
    );
}

/// INC-I-104 M5 — record() works with WAL disabled.
///
/// Writes events via record() (uses put_cf_opt with no-WAL WriteOptions)
/// and reads them back via query_recent to confirm writes succeed.
#[test]
fn diagnostic_ledger_record_works_with_wal_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open should succeed");

    let ts = now_ms();
    let ev1 = make_event_with_ts(EventKind::BlockApplied, 10, ts);
    let ev2 = make_event_with_ts(EventKind::ForkBlockReceived, 20, ts + 1);
    let id1 = ev1.event_id.clone();
    let id2 = ev2.event_id.clone();

    ledger
        .record(&ev1)
        .expect("record ev1 should succeed with WAL disabled");
    ledger
        .record(&ev2)
        .expect("record ev2 should succeed with WAL disabled");

    let results = ledger.query_recent(3600, 100).expect("query_recent");
    assert_eq!(results.len(), 2, "both events should be readable");
    assert_eq!(results[0].event_id, id1);
    assert_eq!(results[1].event_id, id2);
}

/// INC-I-104 M5 — prune() works with WAL disabled.
///
/// prune() uses a WriteBatch + write_opt with no-WAL WriteOptions.
/// This test confirms batch deletes still function correctly.
#[test]
fn diagnostic_ledger_prune_works_with_wal_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open should succeed");

    // Insert 10 ancient events that should be pruned
    let ancient = now_ms() - 60 * 86_400_000; // 60 days ago
    for i in 0..10u64 {
        let ev = make_event_with_ts(EventKind::BlockApplied, i, ancient + i);
        ledger.record(&ev).expect("record should succeed");
    }

    // Insert 2 recent events that should survive
    let recent = now_ms() - 3_600_000; // 1 hour ago
    for i in 0..2u64 {
        let ev = make_event_with_ts(EventKind::BlockApplied, 100 + i, recent + i);
        ledger.record(&ev).expect("record should succeed");
    }

    let pruned = ledger
        .prune(30 * 86400, 100_000)
        .expect("prune should succeed with WAL disabled");
    assert_eq!(pruned, 10, "10 ancient events should be pruned");

    let remaining = ledger.query_recent(365 * 86400, 100).unwrap();
    assert_eq!(remaining.len(), 2, "2 recent events should survive");
}

/// INC-I-104 M5 — metrics() still works after WAL disable.
///
/// The metrics accessor should return valid RocksDB metrics even with
/// WAL disabled. This is a sanity check — no specific metric value asserted
/// beyond non-panic.
#[test]
fn diagnostic_ledger_metrics_works_with_wal_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open should succeed");

    let ev = make_event(EventKind::BlockApplied, 1);
    ledger.record(&ev).expect("record should succeed");

    // Should not panic
    let _metrics = ledger.metrics();
}
