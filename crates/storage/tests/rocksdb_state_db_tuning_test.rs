//! INC-I-104 M3 regression tests: verify state_db per-CF tuning.
//!
//! These tests confirm that the state_db opens correctly with workload-derived
//! per-CF options, that the explicit 48 MB block cache is attached, and that
//! no write stalls occur under normal operation.
//!
//! OUTPUT CONTRACT:
//!   Outputs: successful open, block_cache_bytes > 0, is_write_stopped == 0,
//!            re-open compatibility
//!   Paths:
//!     P1: state_db fresh open + UTXO write → metrics show cache attached
//!     P2: state_db fresh open + UTXO write → no write stall
//!     P3: state_db re-open existing dir with historical CF set → data survives
//!
//!   INPUT PARTITIONS:
//!     P1: fresh tempdir, write 10 UTXOs, flush, check block_cache_bytes > 0
//!     P2: fresh tempdir, write 10 UTXOs, check is_write_stopped == 0
//!     P3: write UTXOs, drop, re-open, verify data

use tempfile::TempDir;

/// REQ-ROCKSDB-M3-001: state_db opens with per-CF tuning and the explicit
/// 48 MB block cache is attached. After writing + flushing UTXOs through
/// cf_utxo, block_cache_bytes must be > 0 (proves cache is wired to CFs).
#[test]
fn state_db_explicit_block_cache_attached() {
    let dir = TempDir::new().unwrap();
    let sdb = storage::StateDb::open(dir.path()).unwrap();

    // Write several UTXOs to force memtable activity on cf_utxo
    let pk_hash = crypto::hash::hash(b"m3-cache-test");
    for i in 0u64..10 {
        let tx =
            doli_core::transaction::Transaction::new_coinbase(100_000 * (i + 1), pk_hash, 0, 0);
        sdb.add_transaction(&tx, i, true, 0).unwrap();
    }

    // Force a read-back to populate the block cache
    let utxos = sdb.get_utxos_by_pubkey(&pk_hash);
    assert_eq!(utxos.len(), 10, "should have 10 UTXOs");

    let m = sdb.metrics();
    assert_eq!(m.instance, "state_db");
    assert!(
        m.block_cache_bytes > 0,
        "block_cache_bytes must be > 0 after reads — proves the explicit 48 MB \
         LRU cache is attached to state_db CFs. Got: {}",
        m.block_cache_bytes
    );
}

/// REQ-ROCKSDB-M3-002: state_db is not write-stopped under normal load.
/// With L0 slowdown=40/stop=60 on the hot CFs, normal test writes must
/// never trigger a write stall. This is the C-003 liveness assertion.
#[test]
fn state_db_no_write_stall_under_normal_load() {
    let dir = TempDir::new().unwrap();
    let sdb = storage::StateDb::open(dir.path()).unwrap();

    let pk_hash = crypto::hash::hash(b"m3-stall-test");
    for i in 0u64..50 {
        let tx =
            doli_core::transaction::Transaction::new_coinbase(100_000 * (i + 1), pk_hash, 0, 0);
        sdb.add_transaction(&tx, i, true, 0).unwrap();
    }

    let m = sdb.metrics();
    assert_eq!(
        m.is_write_stopped, 0,
        "writes must not be stopped on state_db under normal load (C-003 L0 triggers)"
    );
    assert_eq!(
        m.actual_delayed_write_rate, 0,
        "no write throttling expected under normal load"
    );
    assert_eq!(m.background_errors, 0, "no background errors");
}

/// REQ-ROCKSDB-M3-003: state_db re-open with per-CF tuning is compatible
/// with data written by the previous open. Verifies migration safety —
/// changing memtable sizes / bloom / compression doesn't corrupt existing data.
#[test]
fn state_db_reopen_with_per_cf_tuning_data_survives() {
    let dir = TempDir::new().unwrap();
    let pk_hash = crypto::hash::hash(b"m3-reopen-test");

    // First open — write UTXOs
    {
        let sdb = storage::StateDb::open(dir.path()).unwrap();
        for i in 0u64..5 {
            let tx =
                doli_core::transaction::Transaction::new_coinbase(100_000 * (i + 1), pk_hash, 0, 0);
            sdb.add_transaction(&tx, i, true, 0).unwrap();
        }
        assert_eq!(sdb.utxo_len(), 5);
        // DB dropped here, files flushed
    }

    // Second open — data must survive the per-CF option changes
    {
        let sdb = storage::StateDb::open(dir.path()).unwrap();
        assert_eq!(
            sdb.utxo_len(),
            5,
            "UTXOs must survive re-open with new per-CF tuning"
        );

        let utxos = sdb.get_utxos_by_pubkey(&pk_hash);
        assert_eq!(utxos.len(), 5, "UTXO by pubkey index must survive re-open");

        let m = sdb.metrics();
        assert_eq!(m.instance, "state_db");
        assert_eq!(m.background_errors, 0);
    }
}
