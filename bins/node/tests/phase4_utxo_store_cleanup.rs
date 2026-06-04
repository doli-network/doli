//! Phase 4 regression tests: orphaned `utxo_store/` directory cleanup.
//!
//! Phase 4 of the UTXO storage consolidation deletes the separate `utxo_store/`
//! RocksDB instance that was used prior to Phase 2. The node now constructs
//! `UtxoSet::from_state_db()` directly. On first boot after upgrade, any
//! leftover `utxo_store/` directory is removed to reclaim disk space.
//!
//! OUTPUT CONTRACT:
//!   Outputs: directory presence/absence after cleanup
//!   Paths:
//!     P1: utxo_store/ dir present -> removed after cleanup
//!     P2: utxo_store/ dir absent -> no error (idempotent)
//!     P3: utxo_store_secondary_* dirs present -> removed after cleanup
//!
//!   INPUT PARTITIONS:
//!     I1: directory exists with files inside (realistic orphan)
//!     I2: directory does not exist (clean install or post-cleanup boot)
//!     I3: secondary directories from migration tools

use doli_node::node::cleanup_orphan_utxo_store;

/// P1: An orphaned `utxo_store/` directory is removed on cleanup.
#[test]
fn phase4_cleanup_removes_orphaned_utxo_store() {
    let dir = tempfile::TempDir::new().unwrap();
    let data_dir = dir.path();

    // Simulate an orphaned utxo_store with some files inside
    let utxo_store_path = data_dir.join("utxo_store");
    std::fs::create_dir_all(&utxo_store_path).unwrap();
    std::fs::write(utxo_store_path.join("CURRENT"), b"mock").unwrap();
    std::fs::write(utxo_store_path.join("000001.sst"), b"mock-data").unwrap();

    assert!(utxo_store_path.exists(), "precondition: utxo_store exists");

    cleanup_orphan_utxo_store(data_dir);

    assert!(
        !utxo_store_path.exists(),
        "utxo_store directory should be removed after Phase 4 cleanup"
    );
}

/// P2: Cleanup is idempotent — no error when utxo_store/ is already absent.
#[test]
fn phase4_cleanup_noop_when_no_utxo_store() {
    let dir = tempfile::TempDir::new().unwrap();
    let data_dir = dir.path();

    // No utxo_store directory exists — cleanup should be a no-op
    assert!(!data_dir.join("utxo_store").exists());

    // Should not panic or error
    cleanup_orphan_utxo_store(data_dir);

    assert!(!data_dir.join("utxo_store").exists());
}

/// P3: Secondary utxo_store directories from migration tools are also cleaned up.
#[test]
fn phase4_cleanup_removes_secondary_utxo_store_dirs() {
    let dir = tempfile::TempDir::new().unwrap();
    let data_dir = dir.path();

    // Simulate secondary directories from pool_byte_diff / pool_backfill tools
    let secondary1 = data_dir.join("utxo_store_secondary_1");
    let secondary2 = data_dir.join("utxo_store_secondary_readonly");
    std::fs::create_dir_all(&secondary1).unwrap();
    std::fs::create_dir_all(&secondary2).unwrap();
    std::fs::write(secondary1.join("CURRENT"), b"mock").unwrap();

    assert!(secondary1.exists());
    assert!(secondary2.exists());

    cleanup_orphan_utxo_store(data_dir);

    assert!(
        !secondary1.exists(),
        "utxo_store_secondary_1 should be removed"
    );
    assert!(
        !secondary2.exists(),
        "utxo_store_secondary_readonly should be removed"
    );
}
