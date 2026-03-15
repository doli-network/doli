//! Tests for StateDb

use super::types::{CF_META, META_CHAIN_STATE};
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
    Transaction::new_coinbase(amount, pubkey_hash, 0)
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
    db.insert_utxo(&outpoint, &entry);
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
    let removed = db.remove_utxo(&outpoint).unwrap();
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
    db.insert_utxo(&outpoint, &entry);

    let mut cs = ChainState::new(crypto_hash(b"genesis"));
    cs.update(crypto_hash(b"block50"), 50, 100);
    db.put_chain_state(&cs).unwrap();

    assert!(db.has_state());
    assert_eq!(db.utxo_len(), 1);

    // Atomic clear + write genesis
    let genesis_cs = ChainState::new(crypto_hash(b"genesis"));
    db.clear_and_write_genesis(&genesis_cs);

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
    db.insert_utxo(&outpoint, &entry);
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
        db.add_transaction(&tx, i, true, 0);
    }

    let bytes1 = db.serialize_canonical_utxo();
    let bytes2 = db.serialize_canonical_utxo();
    assert_eq!(
        bytes1, bytes2,
        "Canonical serialization must be deterministic"
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
