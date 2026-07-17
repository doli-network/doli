//! Disk Guardian — M1 "Fail-safe foreground writes" (TDD RED phase).
//!
//! Milestone: M1 (workflow run_id 458). Requirements: REQ-DISK-101..106
//! (see `specs/disk-guardian-requirements.md` "Option 1 Requirements (ACTIVE)").
//! Architecture: `specs/disk-guardian-architecture.md` §D1 change table.
//!
//! These tests are written BEFORE the code change. They intentionally FAIL TO
//! COMPILE against the current tree because the seven `StateDb` direct-write
//! methods still return `()` / `Option<UtxoEntry>` instead of
//! `Result<_, StorageError>`. Once the developer performs the D1 signature
//! conversion (`.expect(...)`/`let _ =` → `?`/`Ok(...)`) the file compiles and
//! every assertion here must pass. This is the RED contract for M1.
//!
//! ---------------------------------------------------------------------------
//! SCOPE BOUNDARY (gate transparency — these IDs are NOT tested here, on purpose)
//!   * REQ-DISK-001..012  — SUPERSEDED by the 2026-07-17 scope pivot (watchdog
//!     scope, never implemented). Not part of M1. Out of scope for this file.
//!   * REQ-DISK-201..205  — Milestone M2 ("Bound log growth", installer
//!     logrotate drop-in). A separate, independent milestone; covered by the
//!     M2 test-writer pass, not here.
//! ---------------------------------------------------------------------------
//!
//! OUTPUT CONTRACT (enumeration checklist — outputs × paths × input partitions)
//! Function under test set: the seven D1 direct-write methods. For EACH the
//! observable outputs are: (R) the returned value; (S) the persistent RocksDB
//! store effect (cf_utxo / cf_utxo_by_pubkey / cf_meta / cf_undo writes); and
//! (C) the in-memory `utxo_count` counter. Paths: {success = writable DB} and
//! {disk-write-failure = read-only DB handle, standing in for ENOSPC}.
//!
//!   insert_utxo(op, e) -> Result<(), StorageError>
//!     success:  R=Ok(())        S=entry+index present   C=+1
//!     failure:  R=Err(Database) S=(write rejected)       C=(unchanged — process no longer aborts)
//!   remove_utxo(op) -> Result<Option<UtxoEntry>, StorageError>
//!     present+success: R=Ok(Some(e))  S=deleted   C=-1
//!     absent+success:  R=Ok(None)     S=no-op     C=0
//!     failure:         R=Err(Database)
//!   import_utxos(iter) -> Result<(), StorageError>
//!     success:  R=Ok(())  S=all present  C=set to len
//!     empty:    R=Ok(())  S=no-op        C=0
//!     failure:  R=Err(Database)
//!   add_transaction(tx,h,cb,slot) -> Result<(), StorageError>
//!     success:  R=Ok(())  S=outputs present  C=+outputs
//!     failure:  R=Err(Database)
//!   clear_and_write_genesis(cs) -> Result<(), StorageError>
//!     success:  R=Ok(())  S=all CFs wiped + genesis chain_state written  C=0
//!     failure:  R=Err(Database)
//!   clear_utxos() -> Result<(), StorageError>
//!     success:  R=Ok(())  S=utxo CFs wiped  C=0
//!     failure:  R=Err(Database)
//!   put_undo(h, undo) -> Result<(), StorageError>
//!     success:  R=Ok(())  S=cf_undo entry written  C=n/a
//!     failure:  R=Err(Database)
//!
//! INPUT PARTITIONS (each distinct class = one required test):
//!   P1  writable DB + valid single entry           → insert/remove success, round-trip
//!   P2  writable DB + present outpoint              → remove_utxo Ok(Some)
//!   P3  writable DB + absent outpoint (edge: empty) → remove_utxo Ok(None)
//!   P4  writable DB + tx with outputs               → add_transaction success
//!   P5  writable DB + non-empty entry iterator      → import_utxos success
//!   P6  writable DB + EMPTY iterator (edge: empty)  → import_utxos Ok noop (no write)
//!   P7  writable DB + pre-populated CFs             → clear_and_write_genesis / clear_utxos wipe
//!   P8  writable DB + undo record                   → put_undo round-trip
//!   P9  two writable DBs + identical write sequence → state-root convergence (bit-identity)
//!   P10 read-only DB handle (ENOSPC stand-in)       → every write method returns Err(Database)
//!   P11 read-only DB + pre-existing entry           → remove/clear read-ok then write-Err
//!   P12 caller with `?` over failing DB             → Err propagates (REQ-DISK-103)
//!   P13 UtxoSet::RocksDb wrapper over failing DB    → insert/remove arms surface Err
//!
//! Failure-path injection: `StateDb::from_readonly_for_test(path)` reopens an
//! existing DB as a READ-ONLY RocksDB handle; any `db.write(batch)`/`db.put_cf`
//! then returns a `rocksdb::Error` → `StorageError::Database`. This proves the
//! `?` plumbing surfaces a `StorageError` end-to-end WITHOUT needing a real
//! full disk, and is portable (the Linux-tmpfs ENOSPC variant from the arch
//! Test Strategy is complementary, not required for local RED evidence).

use std::sync::Arc;

use crypto::hash::hash as crypto_hash;
use crypto::Hash;
use doli_core::transaction::{Output, Transaction};
use storage::{
    compute_state_root, ChainState, Outpoint, ProducerSet, StateDb, StorageError, UndoData,
    UtxoEntry, UtxoSet,
};
use tempfile::TempDir;

// ============================ Helpers ============================

/// Fresh, writable state_db in a temp dir (the success-path harness).
fn writable_db() -> (StateDb, TempDir) {
    let dir = TempDir::new().unwrap();
    let db = StateDb::open(dir.path()).unwrap();
    (db, dir)
}

/// Open a temp DB, run `seed` against it (writes), close it, then reopen the
/// same path READ-ONLY. Every subsequent write on the returned handle fails
/// with `StorageError::Database` — the deterministic disk-write-failure path.
fn readonly_db_with(seed: impl FnOnce(&StateDb)) -> (StateDb, TempDir) {
    let dir = TempDir::new().unwrap();
    {
        let db = StateDb::open(dir.path()).unwrap();
        seed(&db);
        // `db` drops here → RocksDB releases the exclusive lock.
    }
    let ro = StateDb::from_readonly_for_test(dir.path()).unwrap();
    (ro, dir)
}

fn entry(amount: u64, pk: Hash, height: u64) -> UtxoEntry {
    UtxoEntry {
        output: Output::normal(amount, pk),
        height,
        is_coinbase: false,
        is_epoch_reward: false,
    }
}

fn coinbase(amount: u64, pk: Hash) -> Transaction {
    Transaction::new_coinbase(amount, pk, 0, 0)
}

/// Deterministic state root of everything currently in `db`'s UTXO CF,
/// combined with a fixed ChainState + empty ProducerSet. Used to assert the
/// success path is bit-identical (same UTXO effects → same root).
fn utxo_state_root(db: &StateDb) -> Hash {
    let mut utxo = UtxoSet::new();
    for (op, e) in db.iter_utxos() {
        utxo.insert(op, e).unwrap();
    }
    let cs = ChainState::new(Hash::ZERO);
    let ps = ProducerSet::new();
    compute_state_root(&cs, &utxo, &ps).unwrap()
}

fn undo(marker: u8) -> UndoData {
    UndoData {
        spent_utxos: vec![],
        created_utxos: vec![],
        producer_snapshot: vec![marker],
        epoch_state_snapshot: None,
        chain_commitment: None,
    }
}

// =====================================================================
//  MUST — SUCCESS PATH IS BIT-IDENTICAL (REQ-DISK-102)
//  Only the FAILURE path changes; the success path must return Ok with the
//  exact same UTXO effects, counter, and state root as before.
// =====================================================================

// Requirement: REQ-DISK-101 / REQ-DISK-102 (Must) — partition P1
// Acceptance: insert_utxo returns Ok(()) and the stored entry round-trips
// bit-for-bit (amount, pubkey, height, flags), counter +1.
#[test]
fn insert_utxo_success_is_ok_and_roundtrip_bit_identical() {
    let (db, _dir) = writable_db();
    let pk = crypto_hash(b"alice");
    let op = Outpoint::new(crypto_hash(b"tx1"), 0);
    let e = UtxoEntry {
        output: Output::normal(500_000, pk),
        height: 7,
        is_coinbase: true,
        is_epoch_reward: false,
    };

    // R: Ok
    assert!(db.insert_utxo(&op, &e).is_ok());

    // S + C: exact round-trip and counter.
    let got = db.get_utxo(&op).expect("utxo must be present");
    assert_eq!(got.output.amount, 500_000);
    assert_eq!(got.output.pubkey_hash, pk);
    assert_eq!(got.height, 7);
    assert!(got.is_coinbase);
    assert!(!got.is_epoch_reward);
    assert_eq!(db.utxo_len(), 1);
    assert_eq!(db.get_utxos_by_pubkey(&pk).len(), 1);
}

// Requirement: REQ-DISK-101 / REQ-DISK-102 (Must) — partition P2
// Acceptance: remove_utxo of a present outpoint returns Ok(Some(entry)) with the
// exact removed value, deletes it, counter -1.
#[test]
fn remove_utxo_present_returns_ok_some_and_deletes() {
    let (db, _dir) = writable_db();
    let pk = crypto_hash(b"bob");
    let op = Outpoint::new(crypto_hash(b"tx2"), 0);
    db.insert_utxo(&op, &entry(900, pk, 3)).unwrap();

    let removed = db.remove_utxo(&op);
    assert_eq!(removed, Ok(Some(entry(900, pk, 3))));
    assert!(db.get_utxo(&op).is_none());
    assert_eq!(db.utxo_len(), 0);
}

// Requirement: REQ-DISK-102 (Must) — partition P3 (edge: absent / empty)
// Acceptance: remove_utxo of an absent outpoint returns Ok(None) (no write, no
// counter change).
#[test]
fn remove_utxo_absent_returns_ok_none() {
    let (db, _dir) = writable_db();
    let op = Outpoint::new(crypto_hash(b"never"), 0);
    assert_eq!(db.remove_utxo(&op), Ok(None));
    assert_eq!(db.utxo_len(), 0);
}

// Requirement: REQ-DISK-101 / REQ-DISK-102 (Must) — partition P4
// Acceptance: add_transaction returns Ok(()) and all tx outputs become UTXOs.
#[test]
fn add_transaction_success_is_ok_and_outputs_present() {
    let (db, _dir) = writable_db();
    let pk = crypto_hash(b"carol");
    let tx = coinbase(1_000, pk);
    let tx_hash = tx.hash();
    let n_outputs = tx.outputs.len() as u64;

    assert!(db.add_transaction(&tx, 5, true, 0).is_ok());

    for index in 0..n_outputs {
        let op = Outpoint::new(tx_hash, index as u32);
        assert!(db.get_utxo(&op).is_some(), "output {index} must be present");
    }
    assert_eq!(db.utxo_len() as u64, n_outputs);
}

// Requirement: REQ-DISK-101 / REQ-DISK-102 (Must) — partition P5
// Acceptance: bulk import_utxos returns Ok(()) and every imported entry is
// present, counter set to the imported count.
#[test]
fn import_utxos_success_is_ok_and_all_present() {
    let (db, _dir) = writable_db();
    let pk = crypto_hash(b"dave");
    let ops: Vec<Outpoint> = (0..64u32)
        .map(|i| Outpoint::new(crypto_hash(format!("imp{i}").as_bytes()), 0))
        .collect();
    let entries: Vec<UtxoEntry> = (0..64u64).map(|i| entry(100 + i, pk, i)).collect();
    let pairs: Vec<(&Outpoint, &UtxoEntry)> = ops.iter().zip(entries.iter()).collect();

    assert!(db.import_utxos(pairs.into_iter()).is_ok());

    assert_eq!(db.utxo_len(), 64);
    for op in &ops {
        assert!(db.get_utxo(op).is_some());
    }
}

// Requirement: REQ-DISK-102 (Must) — partition P6 (edge: empty input, worst-case #1)
// Acceptance: import_utxos over an empty iterator returns Ok(()) and writes
// nothing (must not be an Err — no write op is issued).
#[test]
fn import_utxos_empty_iterator_is_ok_noop() {
    let (db, _dir) = writable_db();
    let empty: Vec<(&Outpoint, &UtxoEntry)> = vec![];
    assert!(db.import_utxos(empty.into_iter()).is_ok());
    assert_eq!(db.utxo_len(), 0);
}

// Requirement: REQ-DISK-101 / REQ-DISK-102 (Must) — partition P7
// Acceptance: clear_and_write_genesis returns Ok(()), wipes prior UTXOs, writes
// the genesis chain state, counter 0.
#[test]
fn clear_and_write_genesis_success_is_ok_wipes_and_writes() {
    let (db, _dir) = writable_db();
    let pk = crypto_hash(b"eve");
    db.insert_utxo(&Outpoint::new(crypto_hash(b"old"), 0), &entry(42, pk, 1))
        .unwrap();

    let genesis = ChainState::new(crypto_hash(b"genesis"));
    assert!(db.clear_and_write_genesis(&genesis).is_ok());

    assert_eq!(db.utxo_len(), 0);
    assert!(db.has_state());
    let cs = db.get_chain_state().expect("genesis chain state present");
    assert_eq!(cs.best_hash, crypto_hash(b"genesis"));
    assert_eq!(cs.best_height, 0);
}

// Requirement: REQ-DISK-105 (Should) — partition P7
// Acceptance: clear_utxos returns Ok(()) (no longer a silent `let _ =` swallow)
// and empties the UTXO set.
#[test]
fn clear_utxos_success_is_ok_and_wipes() {
    let (db, _dir) = writable_db();
    let pk = crypto_hash(b"frank");
    for i in 0..5u32 {
        db.insert_utxo(
            &Outpoint::new(crypto_hash(format!("c{i}").as_bytes()), 0),
            &entry(10, pk, i as u64),
        )
        .unwrap();
    }
    assert_eq!(db.utxo_len(), 5);

    assert!(db.clear_utxos().is_ok());
    assert_eq!(db.utxo_len(), 0);
}

// Requirement: REQ-DISK-101 / REQ-DISK-102 (Must) — partition P8
// Acceptance: put_undo returns Ok(()) and the undo record round-trips.
#[test]
fn put_undo_success_is_ok_and_roundtrip() {
    let (db, _dir) = writable_db();
    assert!(db.put_undo(9, &undo(0xAB)).is_ok());
    let got = db.get_undo(9).expect("undo present");
    assert_eq!(got.producer_snapshot, vec![0xAB]);
}

// Requirement: REQ-DISK-102 (Must) — partition P9, the CRITICAL bit-identity invariant.
// Acceptance: applying the SAME sequence of direct writes to two independent
// DBs yields an IDENTICAL state root (determinism preserved). Only the failure
// path may differ from pre-change behavior.
#[test]
fn state_root_is_bit_identical_across_equivalent_sequences() {
    let pk = crypto_hash(b"grace");
    let apply = |db: &StateDb| {
        db.insert_utxo(&Outpoint::new(crypto_hash(b"s1"), 0), &entry(111, pk, 1))
            .unwrap();
        db.add_transaction(&coinbase(222, pk), 2, true, 0).unwrap();
        db.insert_utxo(&Outpoint::new(crypto_hash(b"s3"), 1), &entry(333, pk, 3))
            .unwrap();
    };

    let (db_a, _a) = writable_db();
    let (db_b, _b) = writable_db();
    apply(&db_a);
    apply(&db_b);

    let root_a = utxo_state_root(&db_a);
    let root_b = utxo_state_root(&db_b);
    assert_eq!(root_a, root_b, "identical write sequences must converge");

    // Sanity: the root actually depends on contents (a divergent DB differs),
    // so the equality above is meaningful, not vacuous.
    let (db_c, _c) = writable_db();
    db_c.insert_utxo(&Outpoint::new(crypto_hash(b"other"), 0), &entry(999, pk, 9))
        .unwrap();
    assert_ne!(root_a, utxo_state_root(&db_c));
}

// =====================================================================
//  MUST — FAILURE PATH RETURNS Err, NEVER PANICS/ABORTS (REQ-DISK-101, 104)
//  Read-only handle stands in for ENOSPC. The process must survive. (P10/P11)
// =====================================================================

// Requirement: REQ-DISK-101 / REQ-DISK-104 (Must) — partition P10
// Acceptance: a failing underlying write makes insert_utxo return Err (not abort).
#[test]
fn insert_utxo_on_failing_db_returns_err_not_panic() {
    let (ro, _dir) = readonly_db_with(|_| {});
    let pk = crypto_hash(b"h");
    let r = ro.insert_utxo(&Outpoint::new(crypto_hash(b"f1"), 0), &entry(1, pk, 0));
    assert!(matches!(r, Err(StorageError::Database(_))), "got {r:?}");
}

// Requirement: REQ-DISK-101 / REQ-DISK-104 (Must) — partition P11
// Acceptance: remove_utxo of an EXISTING entry on a failing DB returns Err — the
// read of the entry succeeds but the delete-batch write fails. Also exercises
// the rollback delete path that utxo/set.rs drives.
#[test]
fn remove_utxo_on_failing_db_returns_err_not_panic() {
    let pk = crypto_hash(b"i");
    let op = Outpoint::new(crypto_hash(b"f2"), 0);
    let (ro, _dir) = readonly_db_with(|db| {
        db.insert_utxo(&op, &entry(5, pk, 0)).unwrap();
    });
    let r = ro.remove_utxo(&op);
    assert!(matches!(r, Err(StorageError::Database(_))), "got {r:?}");
}

// Requirement: REQ-DISK-101 / REQ-DISK-104 (Must) — partition P10
#[test]
fn add_transaction_on_failing_db_returns_err_not_panic() {
    let (ro, _dir) = readonly_db_with(|_| {});
    let r = ro.add_transaction(&coinbase(10, crypto_hash(b"j")), 1, true, 0);
    assert!(matches!(r, Err(StorageError::Database(_))), "got {r:?}");
}

// Requirement: REQ-DISK-101 / REQ-DISK-104 (Must) — partition P10
#[test]
fn import_utxos_on_failing_db_returns_err_not_panic() {
    let (ro, _dir) = readonly_db_with(|_| {});
    let pk = crypto_hash(b"k");
    let op = Outpoint::new(crypto_hash(b"f3"), 0);
    let e = entry(2, pk, 0);
    let pairs = vec![(&op, &e)];
    let r = ro.import_utxos(pairs.into_iter());
    assert!(matches!(r, Err(StorageError::Database(_))), "got {r:?}");
}

// Requirement: REQ-DISK-101 / REQ-DISK-104 (Must) — partition P10
#[test]
fn clear_and_write_genesis_on_failing_db_returns_err_not_panic() {
    let (ro, _dir) = readonly_db_with(|_| {});
    let r = ro.clear_and_write_genesis(&ChainState::new(crypto_hash(b"g")));
    assert!(matches!(r, Err(StorageError::Database(_))), "got {r:?}");
}

// Requirement: REQ-DISK-105 (Should) — partition P11 (failure path)
#[test]
fn clear_utxos_on_failing_db_returns_err_not_panic() {
    let pk = crypto_hash(b"l");
    let (ro, _dir) = readonly_db_with(|db| {
        db.insert_utxo(&Outpoint::new(crypto_hash(b"f4"), 0), &entry(1, pk, 0))
            .unwrap();
    });
    let r = ro.clear_utxos();
    assert!(matches!(r, Err(StorageError::Database(_))), "got {r:?}");
}

// Requirement: REQ-DISK-101 / REQ-DISK-104 (Must) — partition P10 (undo write path)
#[test]
fn put_undo_on_failing_db_returns_err_not_panic() {
    let (ro, _dir) = readonly_db_with(|_| {});
    let r = ro.put_undo(3, &undo(0x01));
    assert!(matches!(r, Err(StorageError::Database(_))), "got {r:?}");
}

// =====================================================================
//  MUST — CALLER SURFACING: the `?` propagates end-to-end (REQ-DISK-103)
// =====================================================================

/// A caller that does `state_db.insert_utxo(...)?` — models init.rs / chain.rs.
fn caller_that_propagates(db: &StateDb, op: &Outpoint, e: &UtxoEntry) -> Result<(), StorageError> {
    db.insert_utxo(op, e)?;
    Ok(())
}

// Requirement: REQ-DISK-103 (Must) — partition P12
// Acceptance: a caller using `?` surfaces the Err on failure and Ok on success —
// the error is not swallowed and does not abort.
#[test]
fn caller_propagates_err_via_question_mark() {
    let pk = crypto_hash(b"m");
    let op = Outpoint::new(crypto_hash(b"f5"), 0);
    let e = entry(7, pk, 0);

    let (ok_db, _d1) = writable_db();
    assert!(caller_that_propagates(&ok_db, &op, &e).is_ok());

    let (ro, _d2) = readonly_db_with(|_| {});
    assert!(matches!(
        caller_that_propagates(&ro, &op, &e),
        Err(StorageError::Database(_))
    ));
}

// Requirement: REQ-DISK-103 (Must) — partition P13
// Acceptance: the `UtxoSet::RocksDb` rollback wrapper arms (insert / remove)
// propagate the StateDb error via `?` instead of swallowing it. Success case
// stays Ok; failing DB yields Err.
#[test]
fn rollback_wrapper_insert_and_remove_surface_err() {
    let pk = crypto_hash(b"n");
    let op = Outpoint::new(crypto_hash(b"f6"), 0);
    let e = entry(4, pk, 0);

    // Success path: RocksDb-backed UtxoSet insert/remove return Ok.
    let dir_ok = TempDir::new().unwrap();
    let db_ok = Arc::new(StateDb::open(dir_ok.path()).unwrap());
    let mut set_ok = UtxoSet::from_state_db(db_ok);
    assert!(set_ok.insert(op, e.clone()).is_ok());
    assert!(matches!(set_ok.remove(&op), Ok(Some(_))));

    // Failure path: read-only backing store → both arms return Err.
    let tmp = TempDir::new().unwrap();
    {
        let db = StateDb::open(tmp.path()).unwrap();
        db.insert_utxo(&op, &e).unwrap();
    }
    let ro = Arc::new(StateDb::from_readonly_for_test(tmp.path()).unwrap());
    let mut set_ro = UtxoSet::from_state_db(ro);
    assert!(
        set_ro.insert(op, e.clone()).is_err(),
        "insert arm must surface Err"
    );
    assert!(set_ro.remove(&op).is_err(), "remove arm must surface Err");
}
