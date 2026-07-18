//! Tests for StateDb

use super::types::{CF_META, CF_UTXO, META_CHAIN_STATE};
use super::*;
use crypto::hash::hash as crypto_hash;
use crypto::KeyPair;
use doli_core::transaction::{Output, Transaction};
use tempfile::TempDir;

use crate::chain_state::ChainState;
use crate::producer::ProducerSet;
use crate::utxo::{Outpoint, UtxoEntry};

fn create_test_db() -> (StateDb, TempDir) {
    let dir = TempDir::new().unwrap();
    let db = StateDb::open(dir.path()).unwrap();
    (db, dir)
}

fn test_coinbase_tx(amount: u64, pubkey_hash: crypto::Hash) -> Transaction {
    Transaction::new_coinbase(amount, pubkey_hash, 0, 0)
}

#[test]
fn test_open_and_has_state() {
    let (db, _dir) = create_test_db();
    assert!(!db.has_state());

    // Write chain state
    let cs = ChainState::new(crypto::Hash::ZERO);
    db.put_chain_state(&cs).unwrap();
    assert!(db.has_state());
}

#[test]
fn test_utxo_crud() {
    let (db, _dir) = create_test_db();
    let pk_hash = crypto_hash(b"alice");

    let outpoint = Outpoint::new(crypto_hash(b"tx1"), 0);
    let entry = UtxoEntry {
        output: Output::normal(500_000, pk_hash),
        height: 1,
        is_coinbase: true,
        is_epoch_reward: false,
    };

    // Insert
    db.insert_utxo(&outpoint, &entry).unwrap();
    assert_eq!(db.utxo_len(), 1);

    // Get
    let got = db.get_utxo(&outpoint).unwrap();
    assert_eq!(got.output.amount, 500_000);
    assert!(got.is_coinbase);

    // Contains
    assert!(db.contains_utxo(&outpoint));
    assert!(!db.contains_utxo(&Outpoint::new(crypto::Hash::ZERO, 0)));

    // By pubkey
    let utxos = db.get_utxos_by_pubkey(&pk_hash);
    assert_eq!(utxos.len(), 1);

    // Remove
    let removed = db.remove_utxo(&outpoint).unwrap().unwrap();
    assert_eq!(removed.output.amount, 500_000);
    assert_eq!(db.utxo_len(), 0);
}

#[test]
fn test_batch_commit_atomic() {
    let (db, _dir) = create_test_db();
    let pk_hash = crypto_hash(b"alice");

    let outpoint1 = Outpoint::new(crypto_hash(b"tx1"), 0);
    let outpoint2 = Outpoint::new(crypto_hash(b"tx2"), 0);
    let entry1 = UtxoEntry {
        output: Output::normal(100, pk_hash),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    let entry2 = UtxoEntry {
        output: Output::normal(200, pk_hash),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };

    // Batch add two UTXOs + chain state
    let mut batch = db.begin_batch();
    batch.add_utxo(outpoint1, entry1);
    batch.add_utxo(outpoint2, entry2);

    let cs = ChainState::new(crypto::Hash::ZERO);
    batch.put_chain_state(&cs);
    batch.set_last_applied(1, crypto_hash(b"block1"), 1);

    batch.commit().unwrap();

    // Verify all committed
    assert_eq!(db.utxo_len(), 2);
    assert!(db.get_utxo(&outpoint1).is_some());
    assert!(db.get_utxo(&outpoint2).is_some());
    assert!(db.get_chain_state().is_some());
    assert!(db.get_last_applied().is_some());
}

#[test]
fn test_batch_drop_no_commit() {
    let (db, _dir) = create_test_db();
    let pk_hash = crypto_hash(b"alice");

    let outpoint = Outpoint::new(crypto_hash(b"tx1"), 0);
    let entry = UtxoEntry {
        output: Output::normal(100, pk_hash),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };

    // Create batch, add UTXO, but DON'T commit
    {
        let mut batch = db.begin_batch();
        batch.add_utxo(outpoint, entry);
        // Drop without commit
    }

    // Nothing persisted
    assert_eq!(db.utxo_len(), 0);
    assert!(db.get_utxo(&outpoint).is_none());
}

#[test]
fn test_same_block_spend() {
    let (db, _dir) = create_test_db();
    let pk_hash = crypto_hash(b"alice");
    let bob_hash = crypto_hash(b"bob");

    let outpoint = Outpoint::new(crypto_hash(b"tx1"), 0);
    let entry = UtxoEntry {
        output: Output::normal(1000, pk_hash),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };

    // TX1 creates UTXO, TX2 spends it — both in same block
    let mut batch = db.begin_batch();
    batch.add_utxo(outpoint, entry);

    // Same-block spend should work (finds in pending_utxos)
    let spent = batch.spend_utxo(&outpoint).unwrap();
    assert_eq!(spent.output.amount, 1000);

    // Add TX2's output
    let outpoint2 = Outpoint::new(crypto_hash(b"tx2"), 0);
    let entry2 = UtxoEntry {
        output: Output::normal(900, bob_hash),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    batch.add_utxo(outpoint2, entry2);

    batch.commit().unwrap();

    // Only TX2's output should exist
    assert_eq!(db.utxo_len(), 1);
    assert!(db.get_utxo(&outpoint).is_none());
    assert!(db.get_utxo(&outpoint2).is_some());
}

#[test]
fn test_producer_crud() {
    let (db, _dir) = create_test_db();

    let kp = KeyPair::generate();
    let pk = *kp.public_key();
    let pk_hash = crypto_hash(pk.as_bytes());

    let info = crate::producer::ProducerInfo::new_with_bonds(
        pk,
        0,
        1_000_000_000,
        (crypto::Hash::ZERO, 0),
        0,
        1,
    );

    let mut batch = db.begin_batch();
    batch.put_producer(&pk_hash, &info);
    batch.commit().unwrap();

    let got = db.get_producer(&pk_hash).unwrap();
    assert_eq!(got.public_key, pk);
    assert_eq!(got.bond_count, 1);
}

#[test]
fn test_exit_history() {
    let (db, _dir) = create_test_db();
    let pk_hash = crypto_hash(b"producer1");

    let mut batch = db.begin_batch();
    batch.put_exit_history(&pk_hash, 5000);
    batch.commit().unwrap();

    assert_eq!(db.get_exit_height(&pk_hash), Some(5000));

    let history = db.iter_exit_history();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0], (pk_hash, 5000));
}

#[test]
fn test_chain_state_roundtrip() {
    let (db, _dir) = create_test_db();

    let mut cs = ChainState::new(crypto_hash(b"genesis"));
    cs.update(crypto_hash(b"block42"), 42, 100);
    cs.total_minted = 999_000_000;

    db.put_chain_state(&cs).unwrap();

    let loaded = db.get_chain_state().unwrap();
    assert_eq!(loaded.best_height, 42);
    assert_eq!(loaded.best_slot, 100);
    assert_eq!(loaded.total_minted, 999_000_000);
}

#[test]
fn test_last_applied_roundtrip() {
    let (db, _dir) = create_test_db();

    let mut batch = db.begin_batch();
    let hash = crypto_hash(b"block100");
    batch.set_last_applied(100, hash, 200);
    batch.commit().unwrap();

    let la = db.get_last_applied().unwrap();
    assert_eq!(la.height, 100);
    assert_eq!(la.hash, hash);
    assert_eq!(la.slot, 200);
}

#[test]
fn test_clear_and_write_genesis() {
    let (db, _dir) = create_test_db();
    let pk_hash = crypto_hash(b"alice");

    // Add some state
    let outpoint = Outpoint::new(crypto_hash(b"tx1"), 0);
    let entry = UtxoEntry {
        output: Output::normal(100, pk_hash),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    db.insert_utxo(&outpoint, &entry).unwrap();

    let mut cs = ChainState::new(crypto_hash(b"genesis"));
    cs.update(crypto_hash(b"block50"), 50, 100);
    db.put_chain_state(&cs).unwrap();

    assert!(db.has_state());
    assert_eq!(db.utxo_len(), 1);

    // Atomic clear + write genesis
    let genesis_cs = ChainState::new(crypto_hash(b"genesis"));
    db.clear_and_write_genesis(&genesis_cs).unwrap();

    // DB is never empty — genesis state is there
    assert!(db.has_state());
    let loaded = db.get_chain_state().unwrap();
    assert_eq!(loaded.best_height, 0);

    // UTXOs gone
    assert_eq!(db.utxo_len(), 0);
    assert!(db.get_utxo(&outpoint).is_none());

    // last_applied is set to genesis
    let la = db.get_last_applied().unwrap();
    assert_eq!(la.height, 0);
}

#[test]
fn test_atomic_replace() {
    let (db, _dir) = create_test_db();
    let pk_hash = crypto_hash(b"alice");

    // Seed initial state
    let outpoint = Outpoint::new(crypto_hash(b"tx_old"), 0);
    let entry = UtxoEntry {
        output: Output::normal(100, pk_hash),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    db.insert_utxo(&outpoint, &entry).unwrap();
    db.put_chain_state(&ChainState::new(crypto::Hash::ZERO))
        .unwrap();

    // New state after reorg
    let mut new_cs = ChainState::new(crypto::Hash::ZERO);
    new_cs.update(crypto_hash(b"new_tip"), 100, 200);
    let new_ps = ProducerSet::new();

    let bob_hash = crypto_hash(b"bob");
    let new_outpoint = Outpoint::new(crypto_hash(b"tx_new"), 0);
    let new_entry = UtxoEntry {
        output: Output::normal(999, bob_hash),
        height: 100,
        is_coinbase: false,
        is_epoch_reward: false,
    };

    db.atomic_replace(&new_cs, &new_ps, std::iter::once((new_outpoint, new_entry)))
        .unwrap();

    // Old UTXO gone
    assert!(db.get_utxo(&outpoint).is_none());
    // New UTXO present
    assert_eq!(db.get_utxo(&new_outpoint).unwrap().output.amount, 999);
    assert_eq!(db.utxo_len(), 1);
    // Chain state is new
    assert_eq!(db.get_chain_state().unwrap().best_height, 100);
}

#[test]
fn test_write_dirty_producers() {
    let (db, _dir) = create_test_db();

    // Seed 3 producers via full write
    let mut ps = ProducerSet::new();
    let kp1 = KeyPair::generate();
    let kp2 = KeyPair::generate();
    let kp3 = KeyPair::generate();
    let pk1 = *kp1.public_key();
    let pk2 = *kp2.public_key();
    let pk3 = *kp3.public_key();
    let _ = ps.register_genesis_producer(pk1, 1, 1_000_000_000);
    let _ = ps.register_genesis_producer(pk2, 1, 1_000_000_000);
    let _ = ps.register_genesis_producer(pk3, 1, 1_000_000_000);
    db.write_producer_set(&ps).unwrap();
    assert_eq!(db.iter_producers().len(), 3);

    // Simulate: only pk2 changed (e.g., added bond)
    let pk2_hash = crypto_hash(pk2.as_bytes());
    let dirty = std::collections::HashSet::from([pk2_hash]);
    let removed = std::collections::HashSet::new();
    let dirty_exits = std::collections::HashSet::new();

    // Modify pk2 in-memory
    if let Some(info) = ps.get_by_pubkey_mut(&pk2) {
        info.bond_count = 5;
    }

    let mut batch = db.begin_batch();
    batch.write_dirty_producers(&ps, &dirty, &removed, &dirty_exits);
    batch.commit().unwrap();

    // pk2 updated
    let got = db.get_producer(&pk2_hash).unwrap();
    assert_eq!(got.bond_count, 5);
    // pk1 unchanged
    let pk1_hash = crypto_hash(pk1.as_bytes());
    let got1 = db.get_producer(&pk1_hash).unwrap();
    assert_eq!(got1.bond_count, 1);
    // Still 3 producers total
    assert_eq!(db.iter_producers().len(), 3);
}

#[test]
fn test_serialize_canonical_utxo_deterministic() {
    let (db, _dir) = create_test_db();
    let pk_hash = crypto_hash(b"alice");

    for i in 0..5u64 {
        let tx = test_coinbase_tx(100_000 * (i + 1), pk_hash);
        db.add_transaction(&tx, i, true, 0).unwrap();
    }

    let bytes1 = db.serialize_canonical_utxo();
    let bytes2 = db.serialize_canonical_utxo();
    assert_eq!(
        bytes1, bytes2,
        "Canonical serialization must be deterministic"
    );
}

// OUTPUT CONTRACT: fn test_serialize_canonical_round_trip_with_corrupted_entry
// O1: serialize_canonical_utxo() produces bytes whose header LE-count equals the
//     number of entries actually present in the body (header/body coherence)
// O2: UtxoSet::deserialize_canonical(serialize_canonical_utxo()) parses without
//     STOR028/STOR030/STOR031 truncation errors, for any subset of CF_UTXO that
//     contains undecodable bincode (RocksDB iter failure, value corruption,
//     schema drift) — silently-dropped entries MUST also be dropped from header
// PATHS: happy (all entries valid), corrupted (one value corrupted)
// MATRIX: O1 × O2 across (happy, corrupted) — four cells, four assertions
//
// Regression for STOR028 snap-sync truncation:
// Mainnet seeds emit canonical bytes whose header advertised N entries but body
// emitted ≤N because serialize_canonical_utxo counted via the live utxo_len()
// atomic while the iterator silently dropped failed entries via `.flatten()` +
// `if let Ok(entry) = bincode::deserialize`. Every external node hit STOR028
// at snap and fell back to header-first sync from genesis.
#[test]
fn test_serialize_canonical_round_trip_with_corrupted_entry() {
    use crate::utxo::UtxoSet;

    // ----- happy path -----
    {
        let (db, _dir) = create_test_db();
        let pk_hash = crypto_hash(b"alice");
        for i in 0..5u64 {
            let tx = test_coinbase_tx(100_000 * (i + 1), pk_hash);
            db.add_transaction(&tx, i, true, 0).unwrap();
        }

        let bytes = db.serialize_canonical_utxo();
        let header_count = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        assert_eq!(
            header_count, 5,
            "happy O1: header count must equal body entry count"
        );
        UtxoSet::deserialize_canonical(&bytes)
            .expect("happy O2: round-trip must parse without truncation error");
    }

    // ----- corrupted-value path -----
    let (db, _dir) = create_test_db();
    let pk_hash = crypto_hash(b"alice");
    for i in 0..5u64 {
        let tx = test_coinbase_tx(100_000 * (i + 1), pk_hash);
        db.add_transaction(&tx, i, true, 0).unwrap();
    }
    assert_eq!(db.utxo_len(), 5);

    // Reach into RocksDB and overwrite one CF_UTXO value with bytes that
    // bincode::deserialize::<UtxoEntry> will reject. This simulates the
    // production failure mode: a value present in CF_UTXO whose decode fails
    // (RocksDB read transient failure, schema drift, or actual byte corruption).
    let cf = db.db.cf_handle(CF_UTXO).unwrap();
    let first_key = db
        .db
        .iterator_cf(cf, rocksdb::IteratorMode::Start)
        .next()
        .expect("at least one UTXO present")
        .expect("iter ok")
        .0
        .to_vec();
    db.db
        .put_cf(cf, &first_key, [0xFFu8; 4])
        .expect("overwrite with garbage");

    let bytes = db.serialize_canonical_utxo();
    let header_count = u64::from_le_bytes(bytes[0..8].try_into().unwrap());

    // Pre-fix: header=5 (utxo_len atomic), body=4 (one entry silently dropped).
    // Post-fix: header=4, body=4. Either way header MUST match body.
    assert_eq!(
        header_count, 4,
        "corrupted O1: header count must reflect body count, not the live atomic. \
         (Pre-fix observes header=5; body emitted only 4. STOR028 follows.)"
    );

    // The wire-level invariant: the serializer's output MUST round-trip through
    // the snap-sync deserializer without truncation errors, even when CF_UTXO
    // contains undecodable values.
    UtxoSet::deserialize_canonical(&bytes).expect(
        "corrupted O2: round-trip must parse — header/body coherence is the snap-sync contract",
    );
}

#[test]
fn test_load_producer_set_roundtrip() {
    let (db, _dir) = create_test_db();

    let mut ps = ProducerSet::new();
    let kp1 = KeyPair::generate();
    let kp2 = KeyPair::generate();
    ps.register_genesis_producer(*kp1.public_key(), 1, 1_000_000_000)
        .expect("register pk1");
    ps.register_genesis_producer(*kp2.public_key(), 1, 1_000_000_000)
        .expect("register pk2");
    assert_eq!(ps.active_count(), 2);

    db.write_producer_set(&ps).unwrap();

    let raw = db.iter_producers();
    assert_eq!(raw.len(), 2, "DB should have 2 producer entries");

    let loaded = db.load_producer_set();
    assert_eq!(loaded.active_count(), 2);
}

// ==================== Format Version Tests ====================

#[test]
fn test_chain_state_versioned_roundtrip() {
    // Write with versioned format (0x01 prefix), read back
    let (db, _dir) = create_test_db();
    let mut cs = ChainState::new(crypto_hash(b"genesis"));
    cs.update(crypto_hash(b"block42"), 42, 100);
    cs.total_minted = 999_000_000;
    cs.active_protocol_version = 2;
    cs.pending_protocol_activation = Some((3, 500));

    db.put_chain_state(&cs).unwrap();
    let loaded = db.get_chain_state().unwrap();

    assert_eq!(loaded.best_height, 42);
    assert_eq!(loaded.best_slot, 100);
    assert_eq!(loaded.total_minted, 999_000_000);
    assert_eq!(loaded.active_protocol_version, 2);
    assert_eq!(loaded.pending_protocol_activation, Some((3, 500)));
}

#[test]
fn test_chain_state_legacy_unversioned_migration() {
    // Simulate v1.0.29 data (raw bincode, no prefix) — must still load
    let (db, _dir) = create_test_db();
    let cs = ChainState::new(crypto_hash(b"genesis"));
    // Write raw bincode WITHOUT the 0x01 prefix (legacy format)
    let raw = bincode::serialize(&cs).unwrap();
    let cf = db.db.cf_handle(CF_META).unwrap();
    db.db.put_cf(cf, META_CHAIN_STATE, &raw).unwrap();

    let loaded = db.get_chain_state().unwrap();
    assert_eq!(loaded.best_height, 0);
    assert_eq!(loaded.active_protocol_version, 1);
}

#[test]
#[should_panic(expected = "failed to deserialize")]
fn test_chain_state_corrupt_bytes_panics() {
    // Corrupt data must panic, never silently return None
    let (db, _dir) = create_test_db();
    let cf = db.db.cf_handle(CF_META).unwrap();
    db.db
        .put_cf(cf, META_CHAIN_STATE, b"garbage bytes that are not valid")
        .unwrap();

    // This must panic — not return None
    let _ = db.get_chain_state();
}

#[test]
fn test_chain_state_fixture_backward_compat() {
    // Frozen bincode bytes from v1.0.30 ChainState::new(Hash::ZERO).
    // If this test fails, someone added/removed/reordered a field
    // without updating the migration in deserialize_chain_state().
    let cs = ChainState::new(crypto::Hash::ZERO);
    let bytes = bincode::serialize(&cs).unwrap();

    // Store the length as a canary — any field change alters it
    assert_eq!(
        bytes.len(),
        170,
        "ChainState bincode size changed! A field was added/removed. \
         Update deserialize_chain_state() with a migration path BEFORE deploying."
    );

    // Must roundtrip
    let loaded: ChainState = bincode::deserialize(&bytes).unwrap();
    assert_eq!(loaded.best_height, 0);
    assert_eq!(loaded.active_protocol_version, 1);
    assert!(loaded.pending_protocol_activation.is_none());
}

#[test]
fn test_versioned_write_has_prefix() {
    // Verify the 0x01 prefix is actually written
    let (db, _dir) = create_test_db();
    let cs = ChainState::new(crypto::Hash::ZERO);
    db.put_chain_state(&cs).unwrap();

    let cf = db.db.cf_handle(CF_META).unwrap();
    let raw = db.db.get_cf(cf, META_CHAIN_STATE).unwrap().unwrap();
    assert_eq!(raw[0], 0x01, "First byte must be format version 0x01");
    // Rest is bincode of ChainState
    let payload = &raw[1..];
    let loaded: ChainState = bincode::deserialize(payload).unwrap();
    assert_eq!(loaded.best_height, 0);
}

// ==================== INC-I-074: prune_undo_below ====================

// OUTPUT CONTRACT: fn prune_undo_below(&self, keep_height: BlockHeight) -> u64
//
// Observable outputs (O1..On):
//   O1: return value (u64 — number of entries deleted)
//   O2: cf_undo contents after call
//       (entries with height < keep_height MUST be absent;
//        entries with height >= keep_height MUST remain unchanged)
//   O3: no panic, no error propagation
//       (method is infallible by design — mirrors prune_undo_above which
//        uses `let _ = self.db.write(batch)`)
//
// Paths:
//   PATH-A: keep_height == 0  -> early no-op return (matches prune_undo_before semantics)
//   PATH-B: keep_height > 0   -> iterate cf_undo, batch-delete keys with height < keep_height
//
// INPUT PARTITIONS:
//   PATH-A:
//     P3: keep_height == 0, cf_undo has entries
//         -> O1 = 0, O2 unchanged, O3 no panic
//   PATH-B:
//     P1: cf_undo has entries below AND at/above horizon (stranded scenario)
//         -> O1 = count below, O2: entries < horizon gone, entries >= horizon kept
//     P2: cf_undo has zero entries below the horizon (idempotent re-run scenario)
//         -> O1 = 0, O2 unchanged, O3 no panic
//
// Matrix (O1 × O2 × O3) × (PATH-A×P3, PATH-B×P1, PATH-B×P2) = 9 assertions covered below.

fn make_undo(marker: u8) -> super::types::UndoData {
    super::types::UndoData {
        spent_utxos: vec![],
        created_utxos: vec![],
        producer_snapshot: vec![marker],
        epoch_state_snapshot: None,
        chain_commitment: None,
    }
}

#[test]
fn prune_undo_below_bulk_deletes_stranded_entries() {
    // PATH-B × P1: stranded scenario — most important partition.
    let (db, _dir) = create_test_db();
    for h in 0u64..=10 {
        db.put_undo(h, &make_undo(h as u8)).unwrap();
    }
    // Setup sanity: all 11 entries present.
    for h in 0u64..=10 {
        assert!(db.get_undo(h).is_some(), "setup: entry {} missing", h);
    }

    let deleted = db.prune_undo_below(5);

    // O1: return value must equal count of deleted entries.
    assert_eq!(deleted, 5, "P1: should delete heights 0..=4 (5 entries)");

    // O2: cf_undo contents — entries below horizon gone.
    for h in 0u64..5 {
        assert!(
            db.get_undo(h).is_none(),
            "P1: entry {} should be deleted (below horizon=5)",
            h
        );
    }
    // O2: cf_undo contents — entries at/above horizon untouched.
    for h in 5u64..=10 {
        let entry = db.get_undo(h);
        assert!(
            entry.is_some(),
            "P1: entry {} should be retained (>= horizon=5)",
            h
        );
        assert_eq!(
            entry.unwrap().producer_snapshot,
            vec![h as u8],
            "P1: entry {} producer_snapshot must be unmodified",
            h
        );
    }
    // O3: no panic reached by getting here.
}

#[test]
fn prune_undo_below_idempotent_when_already_clean() {
    // PATH-B × P2: idempotent re-run scenario.
    let (db, _dir) = create_test_db();
    // Only insert entries at/above horizon — nothing to delete.
    for h in 5u64..=10 {
        db.put_undo(h, &make_undo(h as u8)).unwrap();
    }

    let deleted = db.prune_undo_below(5);

    // O1: zero entries below horizon -> zero deletions.
    assert_eq!(
        deleted, 0,
        "P2: zero entries below horizon should yield zero deletions"
    );

    // O2: cf_undo contents unchanged.
    for h in 5u64..=10 {
        let entry = db.get_undo(h);
        assert!(entry.is_some(), "P2: entry {} should be retained", h);
        assert_eq!(
            entry.unwrap().producer_snapshot,
            vec![h as u8],
            "P2: entry {} must be unmodified",
            h
        );
    }

    // O1+O3: re-run must also be a no-op.
    let deleted_again = db.prune_undo_below(5);
    assert_eq!(deleted_again, 0, "P2: re-run must also yield 0");
}

#[test]
fn prune_undo_below_zero_keep_height_is_noop() {
    // PATH-A × P3: keep_height == 0 must be a no-op
    // (matches prune_undo_before semantics — see undo.rs:40-42).
    let (db, _dir) = create_test_db();
    for h in 0u64..=3 {
        db.put_undo(h, &make_undo(h as u8)).unwrap();
    }

    let deleted = db.prune_undo_below(0);

    // O1: keep_height=0 must yield 0 deletions.
    assert_eq!(deleted, 0, "P3: keep_height=0 must be a no-op");

    // O2: all original entries still present.
    for h in 0u64..=3 {
        assert!(
            db.get_undo(h).is_some(),
            "P3: entry {} must be retained when keep_height=0",
            h
        );
    }
    // O3: no panic reached by getting here.
}

// ==================== INC-I-136 M1: insert_utxo counter idempotency ====================
//
// OUTPUT CONTRACT: fn insert_utxo(&self, outpoint: &Outpoint, entry: &UtxoEntry)
// Outputs:
//   O1: self.utxo_count (AtomicU64) — incremented by 1 only when the key is NEW
//   O2: cf_utxo (RocksDB) — key=outpoint.to_bytes(), value=bincode(entry), upsert semantics
//   O3: cf_utxo_by_pubkey (RocksDB) — secondary index updated (pubkey_hash ++ outpoint -> 0x00)
//   (no return value — fn returns ())
//
// PATHS:
//   P1: new key — outpoint does not exist in cf_utxo before call
//   P2: existing key (upsert) — outpoint already exists in cf_utxo before call
//
// INPUT PARTITIONS:
//   P1a: single new key insert (N=1 from empty)
//   P1b: multiple distinct new keys (N=3 from empty, exercises accumulation)
//   P2a: re-insert same outpoint+entry (identical data, exercises the upsert-counter bug)
//   P2b: re-insert same outpoint with different entry data (value changes, key unchanged)
//   P2c: bulk re-insert all existing keys (rebuild scenario — exercises N doublings)
//
// MATRIX: 3 outputs x 5 partitions = 15 cells
//   P1a: O1(count=1)  O2(key present, correct value)  O3(index present)
//   P1b: O1(count=N)  O2(all N keys present)           O3(all N indexed)
//   P2a: O1(count=N, NOT N+1)  O2(data unchanged)      O3(index unchanged)
//   P2b: O1(count=N, NOT N+1)  O2(data updated)        O3(index present)
//   P2c: O1(count=N, NOT 2N)   O2(all N keys present)  O3(all N indexed)
//
// Edge case: remove-then-reinsert — counter goes N -> N-1 -> N (not N+1)
//
// Requirement: REQ-GUARD-001 (Must)
// Acceptance: insert_utxo on an existing key does not change utxo_count

#[test]
fn test_m1_counter_insert_new_key_increments() {
    // P1a + P1b: Baseline — inserting N distinct keys must yield utxo_len() == N.
    // This test establishes the correct behavior that must remain true after the fix.
    let (db, _dir) = create_test_db();
    let pk_hash = crypto_hash(b"alice");

    // P1a: single insert
    let op1 = Outpoint::new(crypto_hash(b"tx_m1_1"), 0);
    let entry1 = UtxoEntry {
        output: Output::normal(100_000, pk_hash),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    db.insert_utxo(&op1, &entry1).unwrap();

    // O1: count must be 1
    assert_eq!(db.utxo_len(), 1, "P1a O1: single insert must yield count=1");
    // O2: data present
    assert!(db.get_utxo(&op1).is_some(), "P1a O2: key must exist");
    assert_eq!(
        db.get_utxo(&op1).unwrap().output.amount,
        100_000,
        "P1a O2: value must match"
    );
    // O3: index present
    let by_pk = db.get_utxos_by_pubkey(&pk_hash);
    assert_eq!(by_pk.len(), 1, "P1a O3: pubkey index must have 1 entry");

    // P1b: insert 2 more distinct keys (total N=3)
    let op2 = Outpoint::new(crypto_hash(b"tx_m1_2"), 0);
    let entry2 = UtxoEntry {
        output: Output::normal(200_000, pk_hash),
        height: 2,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    let op3 = Outpoint::new(crypto_hash(b"tx_m1_3"), 0);
    let entry3 = UtxoEntry {
        output: Output::normal(300_000, pk_hash),
        height: 3,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    db.insert_utxo(&op2, &entry2).unwrap();
    db.insert_utxo(&op3, &entry3).unwrap();

    // O1: count must be 3
    assert_eq!(
        db.utxo_len(),
        3,
        "P1b O1: three distinct inserts must yield count=3"
    );
    // O2: all keys present
    assert!(db.get_utxo(&op2).is_some(), "P1b O2: key 2 must exist");
    assert!(db.get_utxo(&op3).is_some(), "P1b O2: key 3 must exist");
    // O3: index has all 3
    let by_pk = db.get_utxos_by_pubkey(&pk_hash);
    assert_eq!(by_pk.len(), 3, "P1b O3: pubkey index must have 3 entries");
}

#[test]
fn test_m1_counter_reinsert_same_key_does_not_increment() {
    // P2a: ROOT CAUSE TEST — re-inserting an existing outpoint with identical data
    // must NOT increment utxo_count. This is the primary FAIL->PASS test.
    //
    // Requirement: REQ-GUARD-001 (Must)
    // Acceptance: insert_utxo on an existing key does not change utxo_count
    let (db, _dir) = create_test_db();
    let pk_hash = crypto_hash(b"alice");

    // Insert N=3 distinct UTXOs
    let op1 = Outpoint::new(crypto_hash(b"tx_m1_dup_1"), 0);
    let op2 = Outpoint::new(crypto_hash(b"tx_m1_dup_2"), 0);
    let op3 = Outpoint::new(crypto_hash(b"tx_m1_dup_3"), 0);
    let entry1 = UtxoEntry {
        output: Output::normal(100_000, pk_hash),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    let entry2 = UtxoEntry {
        output: Output::normal(200_000, pk_hash),
        height: 2,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    let entry3 = UtxoEntry {
        output: Output::normal(300_000, pk_hash),
        height: 3,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    db.insert_utxo(&op1, &entry1).unwrap();
    db.insert_utxo(&op2, &entry2).unwrap();
    db.insert_utxo(&op3, &entry3).unwrap();
    assert_eq!(db.utxo_len(), 3, "setup: must have 3 UTXOs");

    // Re-insert op1 with the SAME entry data
    db.insert_utxo(&op1, &entry1).unwrap();

    // O1: count must still be 3, NOT 4
    assert_eq!(
        db.utxo_len(),
        3,
        "P2a O1: re-inserting existing key must NOT increment count (got {} instead of 3)",
        db.utxo_len()
    );
    // O2: data unchanged
    let got = db.get_utxo(&op1).unwrap();
    assert_eq!(
        got.output.amount, 100_000,
        "P2a O2: data must be unchanged after same-data re-insert"
    );
    // O3: index unchanged — still 3 entries for this pubkey
    let by_pk = db.get_utxos_by_pubkey(&pk_hash);
    assert_eq!(
        by_pk.len(),
        3,
        "P2a O3: pubkey index must still have 3 entries"
    );
}

#[test]
fn test_m1_counter_reinsert_same_key_different_value() {
    // P2b: Re-insert same outpoint with DIFFERENT entry data.
    // The counter must not increment; the data must be updated.
    //
    // Requirement: REQ-GUARD-001 (Must)
    let (db, _dir) = create_test_db();
    let pk_hash = crypto_hash(b"alice");

    let op = Outpoint::new(crypto_hash(b"tx_m1_diffval"), 0);
    let entry_v1 = UtxoEntry {
        output: Output::normal(100_000, pk_hash),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    db.insert_utxo(&op, &entry_v1).unwrap();
    assert_eq!(db.utxo_len(), 1, "setup: must have 1 UTXO");

    // Re-insert with different amount (same outpoint key)
    let entry_v2 = UtxoEntry {
        output: Output::normal(999_000, pk_hash),
        height: 5,
        is_coinbase: true,
        is_epoch_reward: false,
    };
    db.insert_utxo(&op, &entry_v2).unwrap();

    // O1: count must still be 1, NOT 2
    assert_eq!(
        db.utxo_len(),
        1,
        "P2b O1: re-inserting existing key with new value must NOT increment count (got {} instead of 1)",
        db.utxo_len()
    );
    // O2: data must be the new value (RocksDB upsert semantics)
    let got = db.get_utxo(&op).unwrap();
    assert_eq!(
        got.output.amount, 999_000,
        "P2b O2: data must reflect the updated value"
    );
    assert_eq!(got.height, 5, "P2b O2: height must reflect the update");
    // O3: index must still be present (1 entry for this pubkey)
    let by_pk = db.get_utxos_by_pubkey(&pk_hash);
    assert_eq!(
        by_pk.len(),
        1,
        "P2b O3: pubkey index must still have 1 entry"
    );
}

#[test]
fn test_m1_rebuild_reinsert_all_existing_does_not_double_count() {
    // P2c: REBUILD SCENARIO MIRROR — the exact pattern that triggers in
    // recover_body_gaps() at init.rs:107-109.
    //
    // Open a StateDb, insert N distinct UTXOs, snapshot the count, then
    // iterate all UTXOs and re-insert each via insert_utxo (the same call
    // path used by recover_body_gaps). Assert count == before, NOT 2N.
    //
    // Requirement: REQ-GUARD-001 (Must)
    // Acceptance: utxoCount == persisted count after rebuild
    let (db, _dir) = create_test_db();
    let pk_hash = crypto_hash(b"alice");
    let bob_hash = crypto_hash(b"bob");

    // Insert N=5 distinct UTXOs across two addresses
    let ops_entries: Vec<(Outpoint, UtxoEntry)> = vec![
        (
            Outpoint::new(crypto_hash(b"tx_rb_1"), 0),
            UtxoEntry {
                output: Output::normal(100_000, pk_hash),
                height: 1,
                is_coinbase: false,
                is_epoch_reward: false,
            },
        ),
        (
            Outpoint::new(crypto_hash(b"tx_rb_2"), 0),
            UtxoEntry {
                output: Output::normal(200_000, pk_hash),
                height: 2,
                is_coinbase: true,
                is_epoch_reward: false,
            },
        ),
        (
            Outpoint::new(crypto_hash(b"tx_rb_3"), 0),
            UtxoEntry {
                output: Output::normal(300_000, bob_hash),
                height: 3,
                is_coinbase: false,
                is_epoch_reward: false,
            },
        ),
        (
            Outpoint::new(crypto_hash(b"tx_rb_4"), 0),
            UtxoEntry {
                output: Output::normal(400_000, pk_hash),
                height: 4,
                is_coinbase: false,
                is_epoch_reward: true,
            },
        ),
        (
            Outpoint::new(crypto_hash(b"tx_rb_5"), 0),
            UtxoEntry {
                output: Output::normal(500_000, bob_hash),
                height: 5,
                is_coinbase: false,
                is_epoch_reward: false,
            },
        ),
    ];

    for (op, entry) in &ops_entries {
        db.insert_utxo(op, entry).unwrap();
    }
    let before = db.utxo_len();
    assert_eq!(before, 5, "setup: must have 5 UTXOs");

    // Reproduce the rebuild loop from recover_body_gaps():
    //   for (outpoint, entry) in state_db.iter_utxos() {
    //       let _ = utxo_set.insert(outpoint, entry);  // -> sdb.insert_utxo()
    //   }
    // Collect first to avoid borrow issues (same as the real code path).
    let all_utxos: Vec<(Outpoint, UtxoEntry)> = db.iter_utxos();
    assert_eq!(all_utxos.len(), 5, "setup: iter_utxos must return 5");

    for (outpoint, entry) in &all_utxos {
        db.insert_utxo(outpoint, entry).unwrap();
    }

    // O1: count must still be 5, NOT 10 (2x)
    assert_eq!(
        db.utxo_len(),
        before,
        "P2c O1: rebuild re-insert must NOT double the count. \
         Expected {} but got {} (exactly {}x — the INC-I-136 bug).",
        before,
        db.utxo_len(),
        db.utxo_len() / before
    );

    // O2: all keys still present with correct values
    for (op, entry) in &ops_entries {
        let got = db
            .get_utxo(op)
            .expect("P2c O2: key must still exist after rebuild");
        assert_eq!(
            got.output.amount, entry.output.amount,
            "P2c O2: value must be unchanged after rebuild"
        );
    }

    // O3: pubkey indexes correct — alice has 3, bob has 2
    let alice_utxos = db.get_utxos_by_pubkey(&pk_hash);
    assert_eq!(
        alice_utxos.len(),
        3,
        "P2c O3: alice pubkey index must have 3 entries"
    );
    let bob_utxos = db.get_utxos_by_pubkey(&bob_hash);
    assert_eq!(
        bob_utxos.len(),
        2,
        "P2c O3: bob pubkey index must have 2 entries"
    );
}

#[test]
fn test_m1_rebuild_via_utxoset_rocksdb_does_not_double_count() {
    // Same rebuild scenario but through the UtxoSet::RocksDb wrapper —
    // the exact call path used in init.rs:109 (`utxo_set.insert()`).
    //
    // Requirement: REQ-GUARD-001 (Must)
    // Acceptance: utxoCount == persisted count after rebuild via UtxoSet
    use crate::utxo::UtxoSet;
    use std::sync::Arc;

    let (db, _dir) = create_test_db();
    let pk_hash = crypto_hash(b"carol");

    // Insert N=4 distinct UTXOs via the StateDb directly
    let ops: Vec<(Outpoint, UtxoEntry)> = (0..4)
        .map(|i| {
            let op = Outpoint::new(crypto_hash(format!("tx_us_{}", i).as_bytes()), 0);
            let entry = UtxoEntry {
                output: Output::normal((i as u64 + 1) * 100_000, pk_hash),
                height: i as u64 + 1,
                is_coinbase: false,
                is_epoch_reward: false,
            };
            (op, entry)
        })
        .collect();

    for (op, entry) in &ops {
        db.insert_utxo(op, entry).unwrap();
    }
    assert_eq!(db.utxo_len(), 4, "setup: must have 4 UTXOs");

    // Wrap in UtxoSet::RocksDb — the production path
    let db_arc = Arc::new(db);
    let mut utxo_set = UtxoSet::from_state_db(db_arc.clone());

    let before = utxo_set.len();
    assert_eq!(before, 4, "setup: UtxoSet.len() must be 4");

    // Reproduce recover_body_gaps() rebuild loop through UtxoSet
    let all_utxos: Vec<(Outpoint, UtxoEntry)> = db_arc.iter_utxos();
    for (outpoint, entry) in all_utxos {
        utxo_set
            .insert(outpoint, entry)
            .expect("insert must not error");
    }

    // O1: count must still be 4, NOT 8
    assert_eq!(
        utxo_set.len(),
        before,
        "P2c via UtxoSet: rebuild re-insert must NOT double the count. \
         Expected {} but got {} (the INC-I-136 bug via UtxoSet::insert).",
        before,
        utxo_set.len()
    );
}

#[test]
fn test_m1_counter_remove_then_reinsert_increments_correctly() {
    // Edge case: remove a UTXO then re-insert it. The counter SHOULD
    // go N -> N-1 -> N (not N -> N-1 -> N+1 or other).
    // This confirms the fix does not break the legitimate new-key path
    // after a key has been removed.
    //
    // Requirement: REQ-GUARD-001 (Must)
    let (db, _dir) = create_test_db();
    let pk_hash = crypto_hash(b"eve");

    let op = Outpoint::new(crypto_hash(b"tx_m1_rmins"), 0);
    let entry = UtxoEntry {
        output: Output::normal(500_000, pk_hash),
        height: 10,
        is_coinbase: false,
        is_epoch_reward: false,
    };

    // Insert -> count = 1
    db.insert_utxo(&op, &entry).unwrap();
    assert_eq!(db.utxo_len(), 1, "after insert: count must be 1");

    // Remove -> count = 0
    let removed = db.remove_utxo(&op).unwrap();
    assert!(removed.is_some(), "remove must return the entry");
    assert_eq!(db.utxo_len(), 0, "after remove: count must be 0");
    assert!(
        db.get_utxo(&op).is_none(),
        "after remove: key must not exist"
    );

    // Re-insert the same outpoint (now a genuinely new key)
    db.insert_utxo(&op, &entry).unwrap();

    // O1: count must be 1 (not 0, not 2)
    assert_eq!(
        db.utxo_len(),
        1,
        "after remove+reinsert: count must be 1 (key is genuinely new again)"
    );
    // O2: data present
    let got = db.get_utxo(&op).unwrap();
    assert_eq!(
        got.output.amount, 500_000,
        "after remove+reinsert: data must match"
    );
}

// ============================================================
// INC-I-136 M2: has_undo_data (undo-data availability check)
// ============================================================
//
// OUTPUT CONTRACT: fn has_undo_data(&self, from: u64, to: u64) -> bool
// Outputs:
//   O1: return (bool) — true iff get_undo(h) is Some for all h in [from,to]
//
// Paths:
//   P1: all present — every height in [from,to] has undo data
//   P2: missing interior — at least one height in (from,to) has no undo data
//   P3: empty range — from > to
//   P4: single height
//
// INPUT PARTITIONS:
//   P1a: dense range [5..10], all heights have undo data (happy path)
//   P2a: range [5..10], height 7 missing (interior gap)
//   P2b: range [5..10], height 5 missing (leading gap — first in range)
//   P2c: range [5..10], height 10 missing (trailing gap — last in range)
//   P3a: from=10, to=5 (inverted range → empty → true)
//   P4a: from=7, to=7, undo present → true
//   P4b: from=7, to=7, undo absent → false
//
// MATRIX: 1 output x 7 partitions = 7 cells
//   P1a: O1(true)  ✓
//   P2a: O1(false) ✓
//   P2b: O1(false) ✓
//   P2c: O1(false) ✓
//   P3a: O1(true)  ✓
//   P4a: O1(true)  ✓
//   P4b: O1(false) ✓
//
// Requirement: REQ-GUARD-003 (Must) — F4
// Acceptance: No healthy checkpoint with missing undo data in rollback window

#[test]
fn test_m2_undo_data_full_range_present() {
    // P1a: all heights [5..10] have undo data → true
    // Requirement: REQ-GUARD-003 (Must)
    let (db, _dir) = create_test_db();

    for h in 5u64..=10 {
        db.put_undo(h, &make_undo(h as u8)).unwrap();
    }

    assert!(
        db.has_undo_data(5, 10),
        "P1a O1: all undo data present in [5..10] must return true"
    );
}

#[test]
fn test_m2_undo_data_interior_gap() {
    // P2a: heights 5-10 have undo data except height 7 → false
    // This is the core failure mode: a checkpoint whose state can't be
    // rolled back because undo data is missing mid-window.
    // Requirement: REQ-GUARD-003 (Must)
    let (db, _dir) = create_test_db();

    for h in 5u64..=10 {
        if h != 7 {
            db.put_undo(h, &make_undo(h as u8)).unwrap();
        }
    }

    assert!(
        !db.has_undo_data(5, 10),
        "P2a O1: missing undo at height 7 must return false"
    );
}

#[test]
fn test_m2_undo_data_leading_gap() {
    // P2b: heights 6-10 have undo data but height 5 missing → false over [5..10]
    // Requirement: REQ-GUARD-003 (Must)
    let (db, _dir) = create_test_db();

    for h in 6u64..=10 {
        db.put_undo(h, &make_undo(h as u8)).unwrap();
    }

    assert!(
        !db.has_undo_data(5, 10),
        "P2b O1: missing leading undo at height 5 must return false"
    );
}

#[test]
fn test_m2_undo_data_trailing_gap() {
    // P2c: heights 5-9 have undo data but height 10 missing → false over [5..10]
    // Requirement: REQ-GUARD-003 (Must)
    let (db, _dir) = create_test_db();

    for h in 5u64..=9 {
        db.put_undo(h, &make_undo(h as u8)).unwrap();
    }

    assert!(
        !db.has_undo_data(5, 10),
        "P2c O1: missing trailing undo at height 10 must return false"
    );
}

#[test]
fn test_m2_undo_data_empty_range() {
    // P3a: from > to (inverted/empty range) → true
    // Requirement: REQ-GUARD-003 (Must)
    let (db, _dir) = create_test_db();

    assert!(
        db.has_undo_data(10, 5),
        "P3a O1: empty range (from > to) must return true"
    );
}

#[test]
fn test_m2_undo_data_single_present() {
    // P4a: from==to==7, undo present → true
    // Requirement: REQ-GUARD-003 (Must)
    let (db, _dir) = create_test_db();

    db.put_undo(7, &make_undo(7)).unwrap();

    assert!(
        db.has_undo_data(7, 7),
        "P4a O1: single height with undo present must return true"
    );
}

#[test]
fn test_m2_undo_data_single_absent() {
    // P4b: from==to==7, no undo → false
    // Requirement: REQ-GUARD-003 (Must)
    let (db, _dir) = create_test_db();

    assert!(
        !db.has_undo_data(7, 7),
        "P4b O1: single height with no undo must return false"
    );
}
