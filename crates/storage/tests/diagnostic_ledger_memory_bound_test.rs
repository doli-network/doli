// INC-I-102 — Memory bound regression guard for DiagnosticLedger::open()
//
// OUTPUT CONTRACT:
//   For DiagnosticLedger::open(data_dir):
//     O1: return — Result<DiagnosticLedger, Error>
//     O2: RocksDB block_cache — must be capped at <= 8 MB (INC-I-102 fix)
//     O3: RocksDB db_write_buffer_size — must be capped at <= 16 MB (INC-I-102 fix)
//
// INPUT PARTITIONS:
//   open(): default config (the only callable input on this path)
//
// MATRIX: O1×P1=Result/Ok, O2×P1=cache<=8MB, O3×P1=write_buf<=16MB
//
// Why this test: INC-I-102 (mainnet OOM cascade on ai5 2026-05-29..30) was
// caused by DiagnosticLedger::open() using rocksdb::Options::default() which
// reserves 32 MB block cache per CF + 128 MB memtable budget. This regression
// guard locks in explicit small caps so the diagnostic_ledger DB instance can
// never silently inflate per-node memory again.
//
// Pre-fix this test FAILS:
//   - rocksdb.block-cache-capacity == 33_554_432 (32 MB default)
// Post-fix this test PASSES:
//   - rocksdb.block-cache-capacity == 4_194_304 (4 MB)

use storage::diagnostic_ledger::DiagnosticLedger;

const MAX_BLOCK_CACHE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_DB_WRITE_BUFFER_BYTES: u64 = 16 * 1024 * 1024;

/// REGRESSION GUARD — INC-I-102
///
/// Opens a DiagnosticLedger and asserts the RocksDB block cache capacity
/// is bounded. The default `Options::default()` reserves 32 MB per CF;
/// this test FAILS until `open()` calls `set_block_cache()` explicitly.
#[test]
fn diagnostic_ledger_caps_block_cache_at_or_below_8mb() {
    let dir = tempfile::tempdir().unwrap();
    let capacity = {
        let ledger = DiagnosticLedger::open(dir.path()).expect("open should succeed");
        // Read the block-cache capacity via the public accessor.
        ledger.block_cache_capacity_bytes()
    };

    assert!(
        capacity <= MAX_BLOCK_CACHE_BYTES,
        "INC-I-102 regression: block cache capacity {} bytes exceeds {} bytes cap. \
         DiagnosticLedger::open() must call set_block_cache() with a small cache.",
        capacity,
        MAX_BLOCK_CACHE_BYTES
    );
}

/// REGRESSION GUARD — INC-I-102
///
/// Asserts memtable budget is bounded. Default is 64 MB × 2 = 128 MB per CF.
#[test]
fn diagnostic_ledger_caps_db_write_buffer_at_or_below_16mb() {
    let dir = tempfile::tempdir().unwrap();
    let budget = {
        let ledger = DiagnosticLedger::open(dir.path()).expect("open should succeed");
        ledger.db_write_buffer_size_bytes()
    };

    assert!(
        budget > 0,
        "INC-I-102 regression: db_write_buffer_size is 0 (unset = unbounded per-CF \
         default of 128 MB). DiagnosticLedger::open() must call set_db_write_buffer_size()."
    );
    assert!(
        budget <= MAX_DB_WRITE_BUFFER_BYTES,
        "INC-I-102 regression: db_write_buffer_size {} bytes exceeds {} bytes cap. \
         DiagnosticLedger::open() must call set_db_write_buffer_size() with a small budget.",
        budget,
        MAX_DB_WRITE_BUFFER_BYTES
    );
}
