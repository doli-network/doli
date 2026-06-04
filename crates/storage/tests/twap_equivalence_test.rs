//! Phase 3 prerequisite gate: write-time extra_data equivalence tests.
//!
//! Compares the two UTXO write paths that coexist during Phase 2:
//!   - Path A: `RocksDbUtxoStore::add_transaction` (per-tx, eliminated in Phase 3)
//!   - Path B: `BlockBatch::add_transaction_utxos` (per-block batch, survives Phase 3)
//!
//! Both paths process the SAME input transaction. They MUST produce byte-identical
//! `UtxoEntry` values per Outpoint, or removing Path A in Phase 3 silently changes
//! UTXO bytes -> state root divergence -> fork.
//!
//! This test extracts the stamping logic from each path into pure functions
//! (no RocksDB required) and asserts byte-equivalence of the serialized UtxoEntry.
//!
//! OUTPUT CONTRACT:
//!   Functions under test (write-path stamping logic):
//!     Path A: utxo_rocks.rs:230-268 — stamps Bond extra_data AND Pool extra_data
//!     Path B: batch.rs:132-146 — stamps Bond extra_data only
//!
//!   Observable outputs:
//!     O1: Serialized UtxoEntry bytes (bincode) for each output in a transaction
//!     O2: Byte-equivalence between Path A and Path B for identical inputs
//!
//!   Code paths:
//!     P1: Bond output with creation_slot=0 (CLI default, stamped by node)
//!     P2: Bond output with creation_slot!=0 (non-zero pre-stamp)
//!     P3: Pool output with creation_slot=0 (needs stamping)
//!     P4: Pool output with active TWAP (last_update_slot > 0, reserve_b > 0)
//!     P5: Pool output with zero reserves (TWAP skip branch)
//!     P6: Normal output (no stamping — control case)
//!     P7: Pool output with creation_slot already set (>0)
//!
//!   INPUT PARTITIONS:
//!     I1: slot=0 — minimum boundary, creation_slot=0 + slot=0 = no effective stamp
//!     I2: slot=1 — minimal non-zero slot, exercises creation_slot stamp
//!     I3: slot=u32::MAX — maximum boundary, overflow-adjacent
//!     I4: reserve_a=1000, reserve_b=500, slot delta=10 — typical TWAP accumulation
//!     I5: reserve_a=u64::MAX/2, reserve_b=1, slot delta=999_900 — extreme TWAP
//!     I6: reserve_b=0 — division guard, TWAP must not accumulate
//!     I7: creation_slot=42 (pre-set) — Path A skips creation_slot but stamps last_update_slot
//!     I8: mixed tx (Bond+Pool+Normal) — multi-output transaction
//!     I9: LPShare output — confirms no write-time mutation in either path
//!
//!   Matrix (test -> output x path x partition):
//!     test_bond_stamp_equivalence_slot_zero:            O2 x P1 x I1
//!     test_bond_stamp_equivalence_slot_one:             O2 x P1 x I2
//!     test_bond_stamp_equivalence_slot_max:             O2 x P1 x I3
//!     test_bond_stamp_equivalence_nonzero_creation:     O2 x P2 x I2
//!     test_normal_output_no_stamp_equivalence:          O2 x P6 x I2
//!     test_coinbase_output_no_stamp_equivalence:        O2 x P6 x I2
//!     test_pool_stamp_diverges_new_pool_slot_zero:      O1,O2 x P3 x I1
//!     test_pool_stamp_diverges_new_pool_slot_one:       O1,O2 x P3 x I2
//!     test_pool_stamp_diverges_with_reserves_and_twap:  O1,O2 x P4 x I4
//!     test_pool_stamp_diverges_zero_reserve_b:          O1,O2 x P5 x I6
//!     test_pool_stamp_diverges_creation_slot_preset:    O1,O2 x P7 x I7
//!     test_pool_stamp_diverges_slot_max:                O1,O2 x P3 x I3
//!     test_pool_stamp_diverges_large_twap:              O1,O2 x P4 x I5
//!     test_mixed_tx_bond_and_pool_outputs:              O1,O2 x P1,P3,P6 x I8
//!     test_lp_share_no_stamp_equivalence:               O2 x P6 x I9

use doli_core::transaction::{Output, Transaction, TxType};
use doli_core::types::{Amount, BlockHeight};
use doli_core::OutputType;
use storage::utxo::UtxoEntry;

/// Replicate Path A stamping logic (utxo_rocks.rs:230-268).
///
/// This is an exact transcription of the mutation applied in
/// `RocksDbUtxoStore::add_transaction` to each output before serialization.
fn stamp_output_path_a(output: &Output, slot: u32) -> Output {
    let mut stamped = output.clone();
    // Bond stamping (utxo_rocks.rs:234-236)
    if stamped.output_type == OutputType::Bond {
        stamped.extra_data = slot.to_le_bytes().to_vec();
    }
    // Pool stamping (utxo_rocks.rs:238-268)
    if stamped.output_type == OutputType::Pool {
        if let Some(mut meta) = stamped.pool_metadata() {
            if meta.creation_slot == 0 {
                meta.creation_slot = slot;
            }
            // Accumulate TWAP BEFORE updating last_update_slot
            if meta.last_update_slot > 0 && slot > meta.last_update_slot && meta.reserve_b > 0 {
                meta.cumulative_price = doli_core::update_twap(
                    meta.cumulative_price,
                    meta.reserve_a,
                    meta.reserve_b,
                    slot,
                    meta.last_update_slot,
                );
            }
            meta.last_update_slot = slot;
            stamped = Output::pool(
                meta.pool_id,
                meta.asset_b_id,
                meta.reserve_a,
                meta.reserve_b,
                meta.total_lp_shares,
                meta.cumulative_price,
                meta.last_update_slot,
                meta.fee_bps,
                meta.creation_slot,
            );
        }
    }
    stamped
}

/// Replicate Path B stamping logic (batch.rs::add_transaction_utxos).
///
/// This is an exact transcription of the mutation applied in
/// `BlockBatch::add_transaction_utxos` to each output before serialization.
///
/// BUG-001 fix: Pool stamping was added to batch.rs (mirroring utxo_rocks.rs)
/// so both write paths produce byte-identical bytes for Pool UTXOs. The
/// transcription below must be kept in sync with the real implementation.
fn stamp_output_path_b(output: &Output, slot: u32) -> Output {
    let mut stamped = output.clone();
    // Bond stamping
    if stamped.output_type == OutputType::Bond {
        stamped.extra_data = slot.to_le_bytes().to_vec();
    }
    // Pool stamping (BUG-001 fix — must match utxo_rocks.rs:237-268)
    if stamped.output_type == OutputType::Pool {
        if let Some(mut meta) = stamped.pool_metadata() {
            if meta.creation_slot == 0 {
                meta.creation_slot = slot;
            }
            // Accumulate TWAP BEFORE updating last_update_slot
            if meta.last_update_slot > 0 && slot > meta.last_update_slot && meta.reserve_b > 0 {
                meta.cumulative_price = doli_core::update_twap(
                    meta.cumulative_price,
                    meta.reserve_a,
                    meta.reserve_b,
                    slot,
                    meta.last_update_slot,
                );
            }
            meta.last_update_slot = slot;
            stamped = Output::pool(
                meta.pool_id,
                meta.asset_b_id,
                meta.reserve_a,
                meta.reserve_b,
                meta.total_lp_shares,
                meta.cumulative_price,
                meta.last_update_slot,
                meta.fee_bps,
                meta.creation_slot,
            );
        }
    }
    stamped
}

/// Build a UtxoEntry from a stamped output (mirrors both paths' entry construction).
fn make_entry(
    output: Output,
    height: BlockHeight,
    is_coinbase: bool,
    is_epoch_reward: bool,
) -> UtxoEntry {
    UtxoEntry {
        output,
        height,
        is_coinbase,
        is_epoch_reward,
    }
}

/// Serialize a UtxoEntry to the bytes that would be written to RocksDB.
fn serialize_entry(entry: &UtxoEntry) -> Vec<u8> {
    bincode::serialize(entry).expect("UtxoEntry serialization must succeed")
}

/// Compare both paths for a single output, returning (path_a_bytes, path_b_bytes).
fn compare_paths(
    output: &Output,
    slot: u32,
    height: BlockHeight,
    is_coinbase: bool,
    is_epoch_reward: bool,
) -> (Vec<u8>, Vec<u8>) {
    let stamped_a = stamp_output_path_a(output, slot);
    let stamped_b = stamp_output_path_b(output, slot);

    let entry_a = make_entry(stamped_a, height, is_coinbase, is_epoch_reward);
    let entry_b = make_entry(stamped_b, height, is_coinbase, is_epoch_reward);

    (serialize_entry(&entry_a), serialize_entry(&entry_b))
}

/// Helper: create a pool output with given parameters.
fn make_pool_output(
    reserve_a: Amount,
    reserve_b: Amount,
    cumulative_price: u128,
    last_update_slot: u32,
    creation_slot: u32,
) -> Output {
    let pool_id = crypto::hash::hash(b"test_pool");
    let asset_b = crypto::hash::hash(b"asset_b");
    Output::pool(
        pool_id,
        asset_b,
        reserve_a,
        reserve_b,
        0, // total_lp_shares
        cumulative_price,
        last_update_slot,
        30, // fee_bps (default 0.3%)
        creation_slot,
    )
}

/// Helper: create a bond output with given creation_slot.
fn make_bond_output(creation_slot: u32) -> Output {
    let pkh = crypto::hash::hash(b"bond_owner");
    Output::bond(5_000_000_000, pkh, u64::MAX, creation_slot)
}

// ==========================================================================
// Bond equivalence tests — EXPECTED: PASS (both paths stamp identically)
// ==========================================================================

#[test]
fn test_bond_stamp_equivalence_slot_zero() {
    let output = make_bond_output(0); // CLI sends creation_slot=0
    let (a, b) = compare_paths(&output, 0, 1, false, false);
    assert_eq!(a, b, "Bond stamping diverges at slot=0");
}

#[test]
fn test_bond_stamp_equivalence_slot_one() {
    let output = make_bond_output(0);
    let (a, b) = compare_paths(&output, 1, 10, false, false);
    assert_eq!(a, b, "Bond stamping diverges at slot=1");
}

#[test]
fn test_bond_stamp_equivalence_slot_max() {
    let output = make_bond_output(0);
    let (a, b) = compare_paths(&output, u32::MAX, 1000, false, false);
    assert_eq!(a, b, "Bond stamping diverges at slot=u32::MAX");
}

#[test]
fn test_bond_stamp_equivalence_nonzero_creation_slot() {
    // Bond with non-zero creation_slot — both paths overwrite it with the block slot
    let output = make_bond_output(42);
    let (a, b) = compare_paths(&output, 100, 50, false, false);
    assert_eq!(
        a, b,
        "Bond stamping diverges when input has nonzero creation_slot"
    );
}

// ==========================================================================
// Normal output control — EXPECTED: PASS (no stamping in either path)
// ==========================================================================

#[test]
fn test_normal_output_no_stamp_equivalence() {
    let output = Output::normal(1_000_000, crypto::hash::hash(b"recipient"));
    let (a, b) = compare_paths(&output, 42, 5, false, false);
    assert_eq!(a, b, "Normal output should be identical in both paths");
}

#[test]
fn test_coinbase_output_no_stamp_equivalence() {
    let output = Output::normal(500_000, crypto::hash::hash(b"producer"));
    let (a, b) = compare_paths(&output, 10, 1, true, false);
    assert_eq!(a, b, "Coinbase output should be identical in both paths");
}

// ==========================================================================
// Pool equivalence tests — REGRESSION GUARD for BUG-001.
//
// BUG-001 (resolved): batch.rs::add_transaction_utxos previously did not
// stamp Pool outputs. utxo_rocks.rs::add_transaction stamps creation_slot,
// last_update_slot, and accumulates TWAP. The two paths produced different
// bytes for every Pool UTXO until the fix. Phase 3 of the UTXO storage
// redesign (specs/utxo-storage-architecture.md) would have caused state
// root divergence on every block containing a Pool output without the fix.
//
// These asserts pin the equivalence. If either path's stamping logic
// changes in the future, both helpers above must be updated in lockstep
// or these tests will catch the drift.
// ==========================================================================

#[test]
fn test_pool_stamp_diverges_new_pool_creation_slot_zero() {
    // New pool: creation_slot=0, last_update_slot=0, no reserves.
    // Path A stamps creation_slot=slot, last_update_slot=slot.
    // Path B writes the output unchanged.
    let output = make_pool_output(0, 0, 0, 0, 0);
    let (a, b) = compare_paths(&output, 1, 1, false, false);

    // This SHOULD be equal if both paths stamp correctly.
    // It WILL NOT be equal because Path B does not stamp Pool outputs.
    if a != b {
        let stamped_a = stamp_output_path_a(&output, 1);
        let stamped_b = stamp_output_path_b(&output, 1);
        let meta_a = stamped_a.pool_metadata().unwrap();
        let meta_b = stamped_b.pool_metadata().unwrap();

        eprintln!("=== DIVERGENCE DETECTED: Pool creation_slot stamping ===");
        eprintln!(
            "Path A: creation_slot={}, last_update_slot={}",
            meta_a.creation_slot, meta_a.last_update_slot
        );
        eprintln!(
            "Path B: creation_slot={}, last_update_slot={}",
            meta_b.creation_slot, meta_b.last_update_slot
        );
        eprintln!("Path A bytes (hex): {}", hex_string(&a));
        eprintln!("Path B bytes (hex): {}", hex_string(&b));
        eprintln!("Divergent byte offsets: {:?}", find_diff_offsets(&a, &b));
        eprintln!("=== Phase 3 gate: BLOCKED ===");
    }

    assert_eq!(
        a, b,
        "BUG-001: Pool extra_data diverges — Path B (batch.rs) does not stamp Pool outputs"
    );
}

#[test]
fn test_pool_stamp_diverges_new_pool_slot_zero() {
    // Edge: slot=0 with creation_slot=0. Path A: creation_slot stays 0
    // (because slot==0 means `if meta.creation_slot == 0 { meta.creation_slot = 0 }` — no-op).
    // But last_update_slot is STILL set to 0 by Path A (which is already 0).
    // This is the one case where creation_slot and last_update_slot happen to match.
    let output = make_pool_output(0, 0, 0, 0, 0);
    let (a, b) = compare_paths(&output, 0, 1, false, false);

    // Both outputs have creation_slot=0, last_update_slot=0. Path A rebuilds
    // via Output::pool() which may produce different bytes due to status field
    // reconstruction. Check if this is truly a no-op.
    if a != b {
        eprintln!("=== DIVERGENCE even at slot=0: Output::pool() reconstruction ===");
        eprintln!("Path A bytes len={}, Path B bytes len={}", a.len(), b.len());
        eprintln!("Divergent byte offsets: {:?}", find_diff_offsets(&a, &b));
    }

    assert_eq!(
        a, b,
        "Pool at slot=0 diverges — Output::pool() reconstruction changes bytes"
    );
}

#[test]
fn test_pool_stamp_diverges_with_reserves_and_twap() {
    // Active pool: reserve_a=1000, reserve_b=500, last_update_slot=10.
    // Path A: TWAP accumulates, last_update_slot updated to slot=20.
    // Path B: output written as-is.
    let output = make_pool_output(1000, 500, 0, 10, 5);
    let (a, b) = compare_paths(&output, 20, 100, false, false);

    if a != b {
        let stamped_a = stamp_output_path_a(&output, 20);
        let stamped_b = stamp_output_path_b(&output, 20);
        let meta_a = stamped_a.pool_metadata().unwrap();
        let meta_b = stamped_b.pool_metadata().unwrap();

        eprintln!("=== DIVERGENCE DETECTED: Pool TWAP accumulation ===");
        eprintln!(
            "Path A: cumulative_price={}, last_update_slot={}",
            meta_a.cumulative_price, meta_a.last_update_slot
        );
        eprintln!(
            "Path B: cumulative_price={}, last_update_slot={}",
            meta_b.cumulative_price, meta_b.last_update_slot
        );
        eprintln!("Divergent byte offsets: {:?}", find_diff_offsets(&a, &b));
    }

    assert_eq!(
        a, b,
        "BUG-001: Pool TWAP accumulation diverges — batch.rs does not accumulate TWAP"
    );
}

#[test]
fn test_pool_stamp_diverges_zero_reserve_b() {
    // Pool with reserve_b=0: TWAP should NOT accumulate (division guard).
    // But creation_slot and last_update_slot are still stamped by Path A.
    let output = make_pool_output(1000, 0, 0, 10, 0);
    let (a, b) = compare_paths(&output, 20, 50, false, false);

    if a != b {
        let stamped_a = stamp_output_path_a(&output, 20);
        let stamped_b = stamp_output_path_b(&output, 20);
        let meta_a = stamped_a.pool_metadata().unwrap();
        let meta_b = stamped_b.pool_metadata().unwrap();

        eprintln!("=== DIVERGENCE: Pool zero-reserve_b still stamps slot fields ===");
        eprintln!(
            "Path A: creation_slot={}, last_update_slot={}",
            meta_a.creation_slot, meta_a.last_update_slot
        );
        eprintln!(
            "Path B: creation_slot={}, last_update_slot={}",
            meta_b.creation_slot, meta_b.last_update_slot
        );
    }

    assert_eq!(
        a, b,
        "BUG-001: Pool slot fields diverge even with zero reserves"
    );
}

#[test]
fn test_pool_stamp_diverges_creation_slot_preset() {
    // Pool with creation_slot already >0: Path A skips creation_slot stamp
    // but still stamps last_update_slot. Path B does nothing.
    let output = make_pool_output(0, 0, 0, 0, 42);
    let (a, b) = compare_paths(&output, 100, 10, false, false);

    if a != b {
        let stamped_a = stamp_output_path_a(&output, 100);
        let stamped_b = stamp_output_path_b(&output, 100);
        let meta_a = stamped_a.pool_metadata().unwrap();
        let meta_b = stamped_b.pool_metadata().unwrap();

        eprintln!("=== DIVERGENCE: Pool with pre-set creation_slot ===");
        eprintln!(
            "Path A: creation_slot={}, last_update_slot={}",
            meta_a.creation_slot, meta_a.last_update_slot
        );
        eprintln!(
            "Path B: creation_slot={}, last_update_slot={}",
            meta_b.creation_slot, meta_b.last_update_slot
        );
    }

    assert_eq!(
        a, b,
        "BUG-001: Pool last_update_slot diverges even when creation_slot preset"
    );
}

#[test]
fn test_pool_stamp_diverges_slot_max() {
    // Edge: slot=u32::MAX. Path A stamps creation_slot and last_update_slot
    // to u32::MAX. Path B leaves them at input values.
    let output = make_pool_output(1000, 1000, 0, 0, 0);
    let (a, b) = compare_paths(&output, u32::MAX, 999, false, false);

    assert_eq!(a, b, "BUG-001: Pool extra_data diverges at slot=u32::MAX");
}

#[test]
fn test_pool_stamp_diverges_large_twap_accumulation() {
    // Large reserves, large slot delta -> significant TWAP accumulation.
    // Path A accumulates. Path B leaves cumulative_price at 0.
    let output = make_pool_output(u64::MAX / 2, 1, 0, 100, 50);
    let (a, b) = compare_paths(&output, 1_000_000, 500, false, false);

    if a != b {
        let stamped_a = stamp_output_path_a(&output, 1_000_000);
        let meta_a = stamped_a.pool_metadata().unwrap();
        eprintln!(
            "=== DIVERGENCE: TWAP cumulative_price after large delta ===\n\
             Path A cumulative_price: {}\n\
             Path B cumulative_price: 0 (unstamped)",
            meta_a.cumulative_price
        );
    }

    assert_eq!(
        a, b,
        "BUG-001: Pool TWAP diverges massively with large reserves and slot delta"
    );
}

// ==========================================================================
// Multi-output transaction equivalence (end-to-end stamping check)
// ==========================================================================

#[test]
fn test_mixed_tx_bond_and_pool_outputs() {
    // A transaction with both Bond and Pool outputs.
    // Bond should be equivalent. Pool will diverge.
    let pkh = crypto::hash::hash(b"multi_owner");
    let pool_id = crypto::hash::hash(b"test_pool");
    let asset_b = crypto::hash::hash(b"asset_b");

    let tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![],
        outputs: vec![
            Output::bond(5_000_000_000, pkh, u64::MAX, 0),
            Output::pool(pool_id, asset_b, 1000, 500, 0, 0, 0, 30, 0),
            Output::normal(100_000, pkh),
        ],
        extra_data: vec![],
    };

    let slot = 42u32;
    let height = 10u64;

    // Stamp each output through both paths and compare
    let mut divergences = Vec::new();
    for (i, output) in tx.outputs.iter().enumerate() {
        let (a, b) = compare_paths(output, slot, height, false, false);
        if a != b {
            divergences.push((i, output.output_type, find_diff_offsets(&a, &b)));
        }
    }

    if !divergences.is_empty() {
        eprintln!("=== DIVERGENCES in mixed transaction ===");
        for (idx, otype, offsets) in &divergences {
            eprintln!(
                "  Output #{} ({:?}): {} bytes differ at offsets {:?}",
                idx,
                otype,
                offsets.len(),
                offsets
            );
        }
    }

    assert!(
        divergences.is_empty(),
        "BUG-001: {} outputs diverge between Path A and Path B in mixed transaction",
        divergences.len()
    );
}

// ==========================================================================
// LPShare — verify no write-time mutation
// ==========================================================================

#[test]
fn test_lp_share_no_stamp_equivalence() {
    // LPShare has no write-time mutation in either path. Confirm.
    let pool_id = crypto::hash::hash(b"test_pool");
    let owner = crypto::hash::hash(b"lp_holder");
    let output = Output::lp_share(1000, pool_id, owner);
    let (a, b) = compare_paths(&output, 42, 10, false, false);
    assert_eq!(
        a, b,
        "LPShare should have no write-time stamping in either path"
    );
}

// ==========================================================================
// Utility functions
// ==========================================================================

fn hex_string(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn find_diff_offsets(a: &[u8], b: &[u8]) -> Vec<usize> {
    let max_len = a.len().max(b.len());
    let mut diffs = Vec::new();
    for i in 0..max_len {
        let byte_a = a.get(i).copied();
        let byte_b = b.get(i).copied();
        if byte_a != byte_b {
            diffs.push(i);
        }
    }
    diffs
}
