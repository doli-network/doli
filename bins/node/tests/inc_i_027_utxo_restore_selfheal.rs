//! INC-I-027 regression test — utxo_store self-heals when it diverges from state_db
//!
//! ## The bug (pre-fix)
//!
//! DOLI's auto-checkpoint guardian snapshots `state_db` + `blocks` only. On restore,
//! the operator typically copies those two directories back and restarts — leaving
//! `utxo_store/` in place because nothing in the guardian procedure told them to
//! move it aside.
//!
//! `bins/node/src/node/init.rs` (pre-fix, line 177) opened the stale `utxo_store/`
//! and used it as-is whenever `!store.is_empty()`, with **no reconciliation** against
//! `state_db`. The node then reported the restored chain height on RPC while silently
//! operating on post-restore UTXOs. On 2026-04-09 during the INC-I-026 mainnet
//! recovery, this made ai1/ai2 vulnerable to bad reorgs: within 15 seconds of start,
//! they reorged forward to the cascade chain. Only ai3 survived because its reorg
//! depth limit rejected the bulk reorg.
//!
//! ## The fix
//!
//! `init_utxo_set` extracts the startup init logic into a testable helper. When
//! `utxo_store/` is non-empty, the helper now compares `store.len()` against
//! `state_db.utxo_len()`. On mismatch it WARNs, clears `utxo_store/`, and
//! re-migrates from `state_db.iter_utxos()` — the same authoritative rebuild loop
//! already used for empty stores. `state_db` is carried inside every guardian
//! checkpoint, so this is a zero-cost self-heal at startup.
//!
//! ## What these tests verify
//!
//! 1. `rebuilt_when_len_mismatches_state_db` — post-restore scenario: state_db has N
//!    authoritative UTXOs, utxo_store has M stale UTXOs (M ≠ N, different contents).
//!    Expected: `init_utxo_set` rebuilds utxo_store to match state_db exactly.
//! 2. `used_as_is_when_matches_state_db` — normal steady-state startup: counts and
//!    contents already match. Expected: no rebuild, store used as-is.
//! 3. `migrated_when_empty` — regression: the existing empty-migration path is
//!    untouched by this fix.

use crypto::hash::hash as crypto_hash;
use doli_core::transaction::Output;
use doli_node::node::init_utxo_set;
use storage::{Outpoint, StateDb, UtxoEntry, UtxoSet};
use tempfile::TempDir;

/// Build a deterministic test UTXO keyed by `tag`.
fn make_utxo(tag: &str, amount: u64) -> (Outpoint, UtxoEntry) {
    let pk_hash = crypto_hash(b"inc_i_027_test_owner");
    let outpoint = Outpoint::new(crypto_hash(tag.as_bytes()), 0);
    let entry = UtxoEntry {
        output: Output::normal(amount, pk_hash),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    (outpoint, entry)
}

/// Seed `state_db` with `n` UTXOs tagged `<prefix>-<i>`.
fn seed_state_db(state_db: &StateDb, n: usize, prefix: &str) {
    for i in 0..n {
        let (o, e) = make_utxo(&format!("{}-{}", prefix, i), 1000 + i as u64);
        state_db.insert_utxo(&o, &e);
    }
}

/// Seed an on-disk `utxo_store` with `n` entries tagged `<prefix>-<i>`, then close it.
fn seed_utxo_store_and_close(path: &std::path::Path, n: usize, prefix: &str) {
    let mut store = UtxoSet::open_rocksdb(path).expect("open utxo_store for seeding");
    for i in 0..n {
        let (o, e) = make_utxo(&format!("{}-{}", prefix, i), 9000 + i as u64);
        store.insert(o, e).expect("insert into utxo_store");
    }
    drop(store);
}

#[test]
fn inc_i_027_utxo_store_rebuilt_when_len_mismatches_state_db() {
    // Guardian-restore scenario: operator restored state_db from a checkpoint
    // (5 authoritative UTXOs) but left utxo_store from the pre-restore era
    // (8 stale UTXOs with completely different outpoints).
    //
    // Without the fix, `init_utxo_set` would return a store with 8 stale entries.
    // With the fix, it rebuilds utxo_store from state_db and returns 5 authoritative
    // entries.

    let temp = TempDir::new().unwrap();
    let data_dir = temp.path();

    // Step 1: simulate restored state_db — authoritative source with 5 UTXOs.
    let state_db = StateDb::open(&data_dir.join("state_db")).expect("open state_db");
    seed_state_db(&state_db, 5, "authoritative");
    assert_eq!(
        state_db.utxo_len(),
        5,
        "state_db should have 5 authoritative UTXOs after seeding"
    );

    // Step 2: simulate stale utxo_store from pre-restore era — 8 DIFFERENT UTXOs.
    seed_utxo_store_and_close(&data_dir.join("utxo_store"), 8, "stale");

    // Step 3: run the startup init logic the fix lives in.
    let utxo_set = init_utxo_set(data_dir, &state_db);

    // Step 4: count should match state_db, not the stale store.
    assert_eq!(
        utxo_set.len(),
        5,
        "INC-I-027: utxo_store should be rebuilt to match state_db (expected 5, got {})",
        utxo_set.len()
    );

    // Step 5: contents — every authoritative outpoint must be present.
    for (outpoint, _) in state_db.iter_utxos() {
        assert!(
            utxo_set.contains(&outpoint),
            "INC-I-027: rebuilt utxo_store is missing authoritative outpoint {:?}",
            outpoint
        );
    }

    // Step 6: NONE of the stale outpoints may survive the rebuild.
    for i in 0..8 {
        let (stale_outpoint, _) = make_utxo(&format!("stale-{}", i), 0);
        assert!(
            !utxo_set.contains(&stale_outpoint),
            "INC-I-027: rebuilt utxo_store still contains stale outpoint {:?} — self-heal failed",
            stale_outpoint
        );
    }
}

#[test]
fn inc_i_027_utxo_store_used_as_is_when_matches_state_db() {
    // Normal steady-state startup: state_db and utxo_store are already consistent.
    // Expected: init_utxo_set takes the no-op path and returns the existing store.

    let temp = TempDir::new().unwrap();
    let data_dir = temp.path();

    let state_db = StateDb::open(&data_dir.join("state_db")).expect("open state_db");

    // Seed state_db AND utxo_store with the SAME 5 UTXOs.
    {
        let mut store =
            UtxoSet::open_rocksdb(&data_dir.join("utxo_store")).expect("open utxo_store");
        for i in 0..5 {
            let (o, e) = make_utxo(&format!("match-{}", i), 1000 + i as u64);
            state_db.insert_utxo(&o, &e);
            store.insert(o, e).expect("insert");
        }
        drop(store);
    }
    assert_eq!(state_db.utxo_len(), 5);

    // Run init — should detect match and leave the store alone.
    let utxo_set = init_utxo_set(data_dir, &state_db);

    assert_eq!(
        utxo_set.len(),
        5,
        "INC-I-027: matching stores should be used as-is"
    );
    for i in 0..5 {
        let (o, _) = make_utxo(&format!("match-{}", i), 0);
        assert!(
            utxo_set.contains(&o),
            "INC-I-027: outpoint match-{} should be present after no-op init",
            i
        );
    }
}

#[test]
fn inc_i_027_utxo_store_migrated_when_empty() {
    // Regression: the existing empty-migration path must still work.
    // Scenario: state_db has 5 UTXOs, utxo_store/ does not exist yet (fresh upgrade).
    // Expected: init_utxo_set creates utxo_store and migrates from state_db.

    let temp = TempDir::new().unwrap();
    let data_dir = temp.path();

    let state_db = StateDb::open(&data_dir.join("state_db")).expect("open state_db");
    seed_state_db(&state_db, 5, "migrate");
    assert_eq!(state_db.utxo_len(), 5);

    // utxo_store/ does not exist — init_utxo_set will create and migrate.
    let utxo_set = init_utxo_set(data_dir, &state_db);

    assert_eq!(
        utxo_set.len(),
        5,
        "INC-I-027 regression: empty utxo_store should be populated from state_db"
    );
    for i in 0..5 {
        let (o, _) = make_utxo(&format!("migrate-{}", i), 0);
        assert!(
            utxo_set.contains(&o),
            "INC-I-027 regression: migrate-{} missing after empty-store migration",
            i
        );
    }
}
