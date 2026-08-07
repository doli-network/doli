//! INC-I-156 / AUDIT-P3-103 — `get_rebuild_in_progress` must FAIL CLOSED.
//!
//! The rebuild marker is a fail-CLOSED safety mechanism: it exists so that a node whose
//! `cf_utxo` was emptied by an interrupted rebuild-from-genesis refuses to produce blocks, to
//! serve `GetStateSnapshot` and to serve `GetStateRoot` until an operator resyncs it. Nothing
//! else in the node detects a truncated ledger — `BlockHeader` carries no `state_root`, so a
//! wrong UTXO set is never caught at block acceptance.
//!
//! The READER inside that mechanism used to fail OPEN: both a RocksDB read error and a
//! present-but-undecodable value collapsed into `None`, which every caller reads as "healthy".
//! A single corrupt byte in `cf_meta` therefore un-halted a corrupt node — the mechanism
//! silently disabling itself in the one direction it must never fail.
//!
//! Lives in its own file rather than in `state_db/tests.rs` (already 1302 lines) so the module
//! size budget is not made worse.
//!
//! ==================== OUTPUT CONTRACT ====================
//!
//! OUTPUT CONTRACT: fn StateDb::get_rebuild_in_progress(&self) -> Option<(u64, u64)>
//!                    (`state_db/queries.rs:775`)
//!
//! OUTPUTS — full enumeration (the function has exactly one):
//!   O1: the `Option<(target_height, started_at)>` return value. `None` is a positive claim of
//!       health, consumed by `Node::rebuild_halt_reason` (`state_snapshot_serve.rs:34`) and by
//!       the startup log (`init.rs:333`); those are the only two non-test callers.
//!
//! PATHS through the RocksDB read:
//!   K1: `Ok(None)` — key genuinely ABSENT. The normal state of every healthy node that has
//!       never rebuilt, so this arm MUST stay `None`; failing closed here would halt the whole
//!       fleet. **Control.**
//!   K2: `Ok(Some(bytes))`, `bytes.len() == 16` — the well-formed marker written by
//!       `set_rebuild_in_progress` (`writes.rs:117-128`). Must decode to its payload verbatim.
//!       **Control.**
//!   K3: `Ok(Some(bytes))`, `bytes.len() != 16` — PRESENT but undecodable. Something wrote the
//!       key; the only writer that ever writes it writes 16 bytes, so the value being any
//!       other length means either corruption or a format the running binary does not
//!       understand. Either way the node cannot prove it is healthy. **RED.**
//!   K4: `Err(_)` — the read itself failed. Same conclusion as K3: unproven health. Not
//!       constructible without a fault injector (it needs the `cf_meta` point lookup to fail
//!       on an open DB); named as a KNOWN GAP. K3 and K4 are collapsed into one match arm in
//!       the implementation precisely so that K3's coverage constrains K4's behaviour too.
//!
//! INPUT PARTITIONS (the on-disk value at `cf_meta[b"rebuild_in_progress"]`):
//!   V0 absent                    -> K1 -> `None`
//!   V16 the writer's own 16 bytes -> K2 -> `Some((target, started))`
//!   V1 one byte                  -> K3 -> ARMED
//!   V15 fifteen bytes (truncated write / short read) -> K3 -> ARMED
//!   V17 seventeen bytes (a longer future format)     -> K3 -> ARMED
//!   Vempty zero bytes            -> K3 -> ARMED   (`Ok(Some(b""))`, NOT `Ok(None)` — RocksDB
//!                                 distinguishes an empty value from an absent key, and this
//!                                 partition is what proves the implementation does too)
//!
//! MATRIX — 1 output × 6 partitions:
//!   V0            -> `absent_marker_reads_healthy`                       [control]
//!   V16           -> `well_formed_marker_decodes_to_its_payload`         [control]
//!   V1/V15/V17/Vempty -> `malformed_marker_fails_closed`                 [RED]
//!   V16 -> V1 (survives a reopen) -> `malformed_marker_still_halts_after_reopen` [RED]
//!
//! PRE-FIX VERDICT — MEASURED on this branch, not predicted.

use super::types::{CF_META, META_REBUILD_IN_PROGRESS};
use super::*;
use tempfile::TempDir;

fn create_test_db() -> (StateDb, TempDir) {
    let dir = TempDir::new().unwrap();
    let db = StateDb::open(dir.path()).unwrap();
    (db, dir)
}

/// Write an arbitrary byte string at the marker key, bypassing `set_rebuild_in_progress`.
///
/// This is the production-shaped injection, not a synthetic one: the value on disk is whatever
/// a truncated write, a corrupt SST or a differently-versioned binary left behind, and the
/// reader has no way to ask which.
fn put_raw_marker(db: &StateDb, bytes: &[u8]) {
    let cf = db.db.cf_handle(CF_META).unwrap();
    db.db.put_cf(cf, META_REBUILD_IN_PROGRESS, bytes).unwrap();
}

// ==================== Controls ====================

#[test]
fn absent_marker_reads_healthy() {
    let (db, _dir) = create_test_db();
    assert_eq!(
        db.get_rebuild_in_progress(),
        None,
        "[AUDIT-P3-103] / K1: an ABSENT key must stay healthy. This is the state of every node \
         that has never rebuilt, so failing closed here would halt the entire fleet — only a \
         read ERROR or a PRESENT-but-undecodable value may arm the halt."
    );
}

#[test]
fn well_formed_marker_decodes_to_its_payload() {
    let (db, _dir) = create_test_db();
    db.set_rebuild_in_progress(4321).unwrap();
    let (target, started) = db
        .get_rebuild_in_progress()
        .expect("[AUDIT-P3-103] / K2: a well-formed marker must read as armed");
    assert_eq!(
        target, 4321,
        "[AUDIT-P3-103] / K2: the target height must round-trip verbatim — the operator message \
         names it (state_snapshot_serve.rs:36)"
    );
    assert!(
        started > 0,
        "[AUDIT-P3-103] / K2: the wall-clock start timestamp must round-trip"
    );
    assert_ne!(
        target,
        StateDb::REBUILD_TARGET_UNKNOWN,
        "[AUDIT-P3-103] / K2: a real target height must never collide with the unknown-payload \
         sentinel, otherwise a genuine rebuild would be reported as a corrupt marker"
    );
}

// ==================== RED ====================

/// **AUDIT-P3-103.** Every PRESENT-but-undecodable value must arm the halt.
///
/// Pre-fix all four of these read as `None`, i.e. as a positive claim that the ledger is
/// intact — a fail-OPEN read at the centre of a fail-CLOSED mechanism. The `Vempty` partition
/// is the sharpest: RocksDB returns `Ok(Some(b""))` for a stored empty value and `Ok(None)`
/// only for a genuinely absent key, so an implementation that conflated "no bytes" with "no
/// key" would pass every other partition and still fail this one.
#[test]
fn malformed_marker_fails_closed() {
    for bytes in [
        vec![0u8; 1],  // V1
        vec![0u8; 15], // V15 — a truncated write of the 16-byte record
        vec![0u8; 17], // V17 — a longer format this binary does not understand
        Vec::new(),    // Vempty
    ] {
        let (db, _dir) = create_test_db();
        put_raw_marker(&db, &bytes);

        let marker = db.get_rebuild_in_progress().unwrap_or_else(|| {
            panic!(
                "[AUDIT-P3-103] / K3: a {}-byte marker value is PRESENT but undecodable, so the \
                 node cannot prove its UTXO set is intact and must stay HALTED. Returning None \
                 reports it as healthy: one corrupt byte in cf_meta silently un-halts a node \
                 whose cf_utxo is truncated, and nothing downstream re-detects that \
                 (BlockHeader carries no state_root). The only writer of this key writes 16 \
                 bytes (writes.rs:117-128), so any other length is corruption or an \
                 unrecognised format — never health.",
                bytes.len()
            )
        });
        assert_eq!(
            marker.0,
            StateDb::REBUILD_TARGET_UNKNOWN,
            "[AUDIT-P3-103] / K3: an undecodable marker must report the UNKNOWN sentinel as its \
             target height, not a fabricated one — the payload was not readable, and \
             `rebuild_halt_reason` renders this case as 'UNKNOWN (marker unreadable)' rather \
             than inventing a height the marker never claimed"
        );
    }
}

/// The halt's entire value is that it survives the process death that created the window it
/// describes. A corrupt marker must therefore keep halting across a reopen — this is the same
/// restart leg the P1 file asserts for the well-formed marker, applied to the corrupt one.
#[test]
fn malformed_marker_still_halts_after_reopen() {
    let dir = TempDir::new().unwrap();
    {
        let db = StateDb::open(dir.path()).unwrap();
        db.set_rebuild_in_progress(99).unwrap();
        // Corrupt the value in place — the shape a partial overwrite leaves behind.
        put_raw_marker(&db, &[7u8; 3]);
    }
    let reopened = StateDb::open(dir.path()).unwrap();
    assert!(
        reopened.get_rebuild_in_progress().is_some(),
        "[AUDIT-P3-103] / K3: the fail-closed read must survive a restart. `Restart=always` \
         otherwise returns a node with a corrupt marker AND a possibly-truncated ledger \
         straight back to the fleet, with its halt silently disabled."
    );
}
