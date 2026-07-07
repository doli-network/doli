//! INC-I-104 M0 regression tests: verify all RocksDB instances have
//! `db_write_buffer_size > 0` (memtable budget cap) and statistics enabled.
//!
//! These tests open each store and verify that `metrics()` returns a valid
//! snapshot with statistics enabled (prerequisite for Prometheus export).
//! The memtable cap itself is enforced by RocksDB internally — our test
//! validates that the configuration code path executes without error and
//! that metrics collection works post-cap.
//!
//! Phase 4: utxo_store test (P3) removed — utxo_store was deleted.
//! state_db is the sole UTXO store.
//!
//! Requirements:
//!   AC-MUST-002 — all instances capped (verified by code review + compile)
//!   AC-MUST-004 — WAL caps on block_store + state_db
//!
//! OUTPUT CONTRACT:
//!   Outputs: metrics snapshot (memtable_bytes, memtable_max_bytes, instance label)
//!   Paths:
//!     P1: block_store open -> metrics snapshot valid, statistics enabled
//!     P2: state_db open -> metrics snapshot valid, statistics enabled
//!
//! INPUT PARTITIONS:
//!   Each path has one partition: fresh tempdir open with default options.
//!   The cap is a compile-time constant — no runtime input variation needed.

use tempfile::TempDir;

/// REQ-ROCKSDB-002: block_store opens successfully with memtable cap and
/// statistics are enabled (prerequisite for Prometheus memtable gauge).
#[test]
fn block_store_metrics_after_memtable_cap() {
    let dir = TempDir::new().unwrap();
    let store = storage::BlockStore::open(dir.path()).unwrap();
    let m = store.metrics();

    assert_eq!(m.instance, "block_store");
    // Statistics enabled -> memtable allocation is tracked.
    // On a fresh DB, RocksDB allocates one write buffer per CF.
    // With 9 CFs and statistics enabled, this should be > 0.
    assert!(
        m.memtable_max_bytes > 0,
        "block_store memtable_max_bytes should be > 0 (statistics enabled)"
    );
    // Sanity: no background errors on fresh open
    assert_eq!(m.background_errors, 0);
}

/// REQ-ROCKSDB-002: state_db opens successfully with memtable cap and
/// statistics are enabled.
#[test]
fn state_db_metrics_after_memtable_cap() {
    let dir = TempDir::new().unwrap();
    let sdb = storage::StateDb::open(dir.path()).unwrap();
    let m = sdb.metrics();

    assert_eq!(m.instance, "state_db");
    assert!(
        m.memtable_max_bytes > 0,
        "state_db memtable_max_bytes should be > 0 (statistics enabled)"
    );
    assert_eq!(m.background_errors, 0);
}
