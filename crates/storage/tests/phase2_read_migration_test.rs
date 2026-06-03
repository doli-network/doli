//! Phase 2 read migration tests: verify that routing UtxoSet reads through
//! state_db produces bit-identical results to routing through utxo_store.
//!
//! These tests exercise the UtxoSet enum dispatch AFTER Phase 2 migration,
//! proving that RPC-facing read methods produce the same results whether
//! routed through state_db or utxo_store.
//!
//! OUTPUT CONTRACT:
//!   Functions under test (on UtxoSet with state_db routing):
//!     1. serialize_canonical() -> Vec<u8>
//!     2. total_value() -> Amount
//!     3. len() -> usize
//!     4. total_confirmed(h, mat, pool_pkh) -> u64
//!     5. address_count() -> u64
//!     6. get_bonded_balance(&Hash) -> Amount
//!     7. count_bonds(&Hash, u64) -> u32
//!     8. get_bond_entries(&Hash) -> Vec<(Outpoint, u32, Amount)>
//!     9. get_all_pools() -> Vec<(Outpoint, UtxoEntry)>
//!    10. find_nft_by_token_id(&Hash) -> Option<(Outpoint, UtxoEntry)>
//!    11. get_balance_with_maturity(&Hash, h, mat) -> Amount
//!    12. get_immature_balance_with_maturity(&Hash, h, mat) -> Amount
//!    13. iter_all() -> Vec<(Outpoint, UtxoEntry)>
//!
//!   Observable outputs:
//!     O1: Return value identical to utxo_store-only routing
//!     O2: serialize_canonical bit-identical (consensus-critical)
//!     O3: iter_all ordering identical
//!
//!   Code paths:
//!     P1: Empty store — both backends return empty/zero
//!     P2: Populated store with mixed output types
//!     P3: state_db-routed reads vs utxo_store-only reads
//!
//! INPUT PARTITIONS:
//!     I1: Empty store -> all methods return 0/empty
//!     I2: Mixed output types (Normal, Bond, Pool, NFT, Coinbase)
//!     I3: Multiple addresses
//!     I4: Maturity-gated entries (coinbase with height gating)

use std::sync::Arc;

use crypto::hash::hash as crypto_hash;
use crypto::Hash;
use doli_core::transaction::Output;
use storage::utxo::{Outpoint, UtxoEntry, UtxoSet};
use storage::StateDb;
use tempfile::TempDir;

/// Create a UtxoSet::RocksDb with state_db attached (Phase 2 configuration).
/// Also returns a UtxoSet::RocksDb WITHOUT state_db for comparison.
fn create_phase2_stores() -> (UtxoSet, UtxoSet, Arc<StateDb>, TempDir, TempDir, TempDir) {
    let dir_sdb = TempDir::new().unwrap();
    let dir_utxo_migrated = TempDir::new().unwrap();
    let dir_utxo_baseline = TempDir::new().unwrap();

    let sdb = Arc::new(StateDb::open(dir_sdb.path()).unwrap());

    // Migrated: UtxoSet with state_db attached (Phase 2)
    let mut migrated = UtxoSet::open_rocksdb(dir_utxo_migrated.path()).unwrap();
    migrated.set_state_db(sdb.clone());

    // Baseline: UtxoSet without state_db (pre-Phase 2, reads from utxo_store)
    let baseline = UtxoSet::open_rocksdb(dir_utxo_baseline.path()).unwrap();

    (
        migrated,
        baseline,
        sdb,
        dir_sdb,
        dir_utxo_migrated,
        dir_utxo_baseline,
    )
}

/// Insert a UTXO into a UtxoSet AND into state_db (to simulate dual-write).
fn insert_all(utxo: &mut UtxoSet, sdb: &StateDb, outpoint: Outpoint, entry: UtxoEntry) {
    utxo.insert(outpoint, entry.clone()).unwrap();
    sdb.insert_utxo(&outpoint, &entry);
}

/// Populate both stores with a standard mixed UTXO set for testing.
/// Returns (alice_pkh, bob_pkh, pool_pkh).
fn populate_mixed(
    migrated: &mut UtxoSet,
    baseline: &mut UtxoSet,
    sdb: &StateDb,
) -> (Hash, Hash, Hash) {
    let alice = crypto_hash(b"alice");
    let bob = crypto_hash(b"bob");
    let pool_hash = crypto_hash(b"pool_address");

    // Alice: 2 normal UTXOs
    for i in 0..2u32 {
        let op = Outpoint::new(crypto_hash(format!("alice_normal_{}", i).as_bytes()), 0);
        let entry = UtxoEntry {
            output: Output::normal(500_000 * (i as u64 + 1), alice),
            height: 10,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        insert_all(migrated, sdb, op, entry.clone());
        baseline.insert(op, entry).unwrap();
    }

    // Alice: 3 bond UTXOs
    for i in 0..3u32 {
        let op = Outpoint::new(crypto_hash(format!("alice_bond_{}", i).as_bytes()), 0);
        let entry = UtxoEntry {
            output: Output::bond(1_000_000_000, alice, u64::MAX, i * 10),
            height: 5,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        insert_all(migrated, sdb, op, entry.clone());
        baseline.insert(op, entry).unwrap();
    }

    // Bob: 1 coinbase UTXO (immature at height 10, mature at height 200)
    {
        let op = Outpoint::new(crypto_hash(b"bob_coinbase"), 0);
        let entry = UtxoEntry {
            output: Output::normal(250_000, bob),
            height: 5,
            is_coinbase: true,
            is_epoch_reward: false,
        };
        insert_all(migrated, sdb, op, entry.clone());
        baseline.insert(op, entry).unwrap();
    }

    // Bob: 1 normal UTXO
    {
        let op = Outpoint::new(crypto_hash(b"bob_normal"), 0);
        let entry = UtxoEntry {
            output: Output::normal(100_000, bob),
            height: 8,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        insert_all(migrated, sdb, op, entry.clone());
        baseline.insert(op, entry).unwrap();
    }

    // Pool UTXO
    {
        let pool_output = Output {
            output_type: doli_core::OutputType::Pool,
            amount: 0,
            pubkey_hash: pool_hash,
            lock_until: 0,
            extra_data: vec![0u8; 100],
        };
        let op = Outpoint::new(crypto_hash(b"pool_tx"), 0);
        let entry = UtxoEntry {
            output: pool_output,
            height: 3,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        insert_all(migrated, sdb, op, entry.clone());
        baseline.insert(op, entry).unwrap();
    }

    (alice, bob, pool_hash)
}

// ==================== serialize_canonical bit-identity ====================

#[test]
fn phase2_serialize_canonical_bit_identical() {
    let (mut migrated, mut baseline, sdb, _d1, _d2, _d3) = create_phase2_stores();
    let (_alice, _bob, _pool) = populate_mixed(&mut migrated, &mut baseline, &sdb);

    let migrated_bytes = migrated.serialize_canonical();
    let baseline_bytes = baseline.serialize_canonical();

    assert_eq!(
        migrated_bytes.len(),
        baseline_bytes.len(),
        "canonical byte lengths differ"
    );
    assert_eq!(
        migrated_bytes, baseline_bytes,
        "serialize_canonical NOT bit-identical between state_db and utxo_store routing"
    );
}

#[test]
fn phase2_serialize_canonical_empty() {
    let (migrated, baseline, _sdb, _d1, _d2, _d3) = create_phase2_stores();
    let m = migrated.serialize_canonical();
    let b = baseline.serialize_canonical();
    assert_eq!(m, b);
    // Empty store: 8-byte count (0) only
    assert_eq!(m.len(), 8);
    assert_eq!(u64::from_le_bytes(m[..8].try_into().unwrap()), 0);
}

// ==================== total_value ====================

#[test]
fn phase2_total_value_identical() {
    let (mut migrated, mut baseline, sdb, _d1, _d2, _d3) = create_phase2_stores();
    populate_mixed(&mut migrated, &mut baseline, &sdb);

    assert_eq!(migrated.total_value(), baseline.total_value());
    assert!(migrated.total_value() > 0);
}

// ==================== len ====================

#[test]
fn phase2_len_identical() {
    let (mut migrated, mut baseline, sdb, _d1, _d2, _d3) = create_phase2_stores();
    populate_mixed(&mut migrated, &mut baseline, &sdb);

    assert_eq!(migrated.len(), baseline.len());
    assert_eq!(migrated.len(), 8); // 2 normal + 3 bond + 1 coinbase + 1 normal + 1 pool
}

// ==================== total_confirmed ====================

#[test]
fn phase2_total_confirmed_identical() {
    let (mut migrated, mut baseline, sdb, _d1, _d2, _d3) = create_phase2_stores();
    let (_alice, _bob, pool) = populate_mixed(&mut migrated, &mut baseline, &sdb);

    let height = 200;
    let maturity = 100;
    let pool_pkh = pool.as_bytes();

    let m = migrated.total_confirmed(height, maturity, pool_pkh);
    let b = baseline.total_confirmed(height, maturity, pool_pkh);
    assert_eq!(m, b);
    assert!(m > 0);
}

// ==================== address_count ====================

#[test]
fn phase2_address_count_identical() {
    let (mut migrated, mut baseline, sdb, _d1, _d2, _d3) = create_phase2_stores();
    populate_mixed(&mut migrated, &mut baseline, &sdb);

    assert_eq!(migrated.address_count(), baseline.address_count());
    assert_eq!(migrated.address_count(), 3); // alice, bob, pool_hash
}

// ==================== get_bonded_balance ====================

#[test]
fn phase2_get_bonded_balance_identical() {
    let (mut migrated, mut baseline, sdb, _d1, _d2, _d3) = create_phase2_stores();
    let (alice, _bob, _pool) = populate_mixed(&mut migrated, &mut baseline, &sdb);

    assert_eq!(
        migrated.get_bonded_balance(&alice),
        baseline.get_bonded_balance(&alice)
    );
    assert_eq!(migrated.get_bonded_balance(&alice), 3_000_000_000);
}

// ==================== count_bonds ====================

#[test]
fn phase2_count_bonds_identical() {
    let (mut migrated, mut baseline, sdb, _d1, _d2, _d3) = create_phase2_stores();
    let (alice, _bob, _pool) = populate_mixed(&mut migrated, &mut baseline, &sdb);
    let bond_unit = 1_000_000_000u64;

    assert_eq!(
        migrated.count_bonds(&alice, bond_unit),
        baseline.count_bonds(&alice, bond_unit)
    );
    assert_eq!(migrated.count_bonds(&alice, bond_unit), 3);
}

// ==================== get_bond_entries ====================

#[test]
fn phase2_get_bond_entries_identical() {
    let (mut migrated, mut baseline, sdb, _d1, _d2, _d3) = create_phase2_stores();
    let (alice, _bob, _pool) = populate_mixed(&mut migrated, &mut baseline, &sdb);

    let m = migrated.get_bond_entries(&alice);
    let b = baseline.get_bond_entries(&alice);
    assert_eq!(m.len(), b.len());
    for (me, be) in m.iter().zip(b.iter()) {
        assert_eq!(me.1, be.1, "creation_slot mismatch");
        assert_eq!(me.2, be.2, "amount mismatch");
    }
}

// ==================== get_all_pools ====================

#[test]
fn phase2_get_all_pools_identical() {
    let (mut migrated, mut baseline, sdb, _d1, _d2, _d3) = create_phase2_stores();
    populate_mixed(&mut migrated, &mut baseline, &sdb);

    let m = migrated.get_all_pools();
    let b = baseline.get_all_pools();
    assert_eq!(m.len(), b.len());
    assert_eq!(m.len(), 1);
}

// ==================== find_nft_by_token_id ====================

#[test]
fn phase2_find_nft_by_token_id_not_found() {
    let (mut migrated, mut baseline, sdb, _d1, _d2, _d3) = create_phase2_stores();
    populate_mixed(&mut migrated, &mut baseline, &sdb);

    let tid = crypto_hash(b"nonexistent_token");
    assert!(migrated.find_nft_by_token_id(&tid).is_none());
    assert!(baseline.find_nft_by_token_id(&tid).is_none());
}

#[test]
fn phase2_find_nft_by_token_id_found() {
    let (mut migrated, mut baseline, sdb, _d1, _d2, _d3) = create_phase2_stores();
    let alice = crypto_hash(b"alice");
    let token_id = crypto_hash(b"unique_nft_token");

    // Use the proper Output::nft() constructor which encodes condition + metadata
    let content_hash = crypto_hash(b"content");
    let condition = doli_core::conditions::Condition::Timelock(0); // trivially satisfied
    let nft_output = Output::nft(0, alice, token_id, content_hash.as_bytes(), &condition)
        .expect("NFT output construction");

    let op = Outpoint::new(crypto_hash(b"nft_tx"), 0);
    let entry = UtxoEntry {
        output: nft_output,
        height: 10,
        is_coinbase: false,
        is_epoch_reward: false,
    };

    // Insert in both stores + register unique_id
    insert_all(&mut migrated, &sdb, op, entry.clone());
    baseline.insert(op, entry).unwrap();

    // Register unique_id in state_db (migrated path checks state_db for unique_id)
    sdb.add_unique_id(storage::UID_PREFIX_NFT, &token_id);

    let m = migrated.find_nft_by_token_id(&token_id);
    assert!(m.is_some(), "migrated should find NFT via state_db");

    // Verify outpoint and type match
    let (m_op, m_entry) = m.unwrap();
    assert_eq!(m_op, op);
    assert_eq!(m_entry.output.output_type, doli_core::OutputType::NFT);
}

// ==================== get_balance_with_maturity ====================

#[test]
fn phase2_get_balance_with_maturity_identical() {
    let (mut migrated, mut baseline, sdb, _d1, _d2, _d3) = create_phase2_stores();
    let (alice, bob, _pool) = populate_mixed(&mut migrated, &mut baseline, &sdb);

    // At height 200 with maturity 100, all outputs should be mature
    let m_alice = migrated.get_balance_with_maturity(&alice, 200, 100);
    let b_alice = baseline.get_balance_with_maturity(&alice, 200, 100);
    assert_eq!(m_alice, b_alice);

    let m_bob = migrated.get_balance_with_maturity(&bob, 200, 100);
    let b_bob = baseline.get_balance_with_maturity(&bob, 200, 100);
    assert_eq!(m_bob, b_bob);
}

// ==================== get_immature_balance_with_maturity ====================

#[test]
fn phase2_get_immature_balance_with_maturity_identical() {
    let (mut migrated, mut baseline, sdb, _d1, _d2, _d3) = create_phase2_stores();
    let (_alice, bob, _pool) = populate_mixed(&mut migrated, &mut baseline, &sdb);

    // At height 10 with maturity 100, bob's coinbase (height=5) is immature
    let m = migrated.get_immature_balance_with_maturity(&bob, 10, 100);
    let b = baseline.get_immature_balance_with_maturity(&bob, 10, 100);
    assert_eq!(m, b);
    assert_eq!(m, 250_000); // coinbase at height 5, not yet mature at height 10
}

// ==================== iter_all ordering ====================

#[test]
fn phase2_iter_all_ordering_identical() {
    let (mut migrated, mut baseline, sdb, _d1, _d2, _d3) = create_phase2_stores();
    populate_mixed(&mut migrated, &mut baseline, &sdb);

    let m = migrated.iter_all();
    let b = baseline.iter_all();

    assert_eq!(m.len(), b.len(), "iter_all lengths differ");
    for (i, ((m_op, m_entry), (b_op, b_entry))) in m.iter().zip(b.iter()).enumerate() {
        assert_eq!(m_op, b_op, "outpoint mismatch at index {}", i);
        assert_eq!(
            m_entry.output.amount, b_entry.output.amount,
            "amount mismatch at index {}",
            i
        );
        assert_eq!(
            m_entry.output.pubkey_hash, b_entry.output.pubkey_hash,
            "pubkey_hash mismatch at index {}",
            i
        );
    }
}

// ==================== apply_block critical reads still work ====================
// These methods stay on utxo_store to support same-block-spend.

#[test]
fn phase2_get_still_works_on_utxo_store() {
    let (mut migrated, _baseline, sdb, _d1, _d2, _d3) = create_phase2_stores();
    let alice = crypto_hash(b"alice");
    let op = Outpoint::new(crypto_hash(b"test_get"), 0);
    let entry = UtxoEntry {
        output: Output::normal(100_000, alice),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };

    insert_all(&mut migrated, &sdb, op, entry.clone());

    // get() should find the UTXO (routed through utxo_store for apply_block safety)
    let result = migrated.get(&op);
    assert!(result.is_some());
    assert_eq!(result.unwrap().output.amount, 100_000);
}

#[test]
fn phase2_contains_still_works_on_utxo_store() {
    let (mut migrated, _baseline, sdb, _d1, _d2, _d3) = create_phase2_stores();
    let alice = crypto_hash(b"alice");
    let op = Outpoint::new(crypto_hash(b"test_contains"), 0);
    let entry = UtxoEntry {
        output: Output::normal(100_000, alice),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };

    insert_all(&mut migrated, &sdb, op, entry);

    assert!(migrated.contains(&op));
}

#[test]
fn phase2_get_by_pubkey_hash_still_works_on_utxo_store() {
    let (mut migrated, _baseline, sdb, _d1, _d2, _d3) = create_phase2_stores();
    let alice = crypto_hash(b"alice");
    let op = Outpoint::new(crypto_hash(b"test_pubkey"), 0);
    let entry = UtxoEntry {
        output: Output::normal(100_000, alice),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };

    insert_all(&mut migrated, &sdb, op, entry);

    let results = migrated.get_by_pubkey_hash(&alice);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, op);
}

#[test]
fn phase2_has_unique_id_still_works_on_utxo_store() {
    let (migrated, _baseline, sdb, _d1, _d2, _d3) = create_phase2_stores();

    let token_id = crypto_hash(b"test_uid");

    // unique_id is NOT in utxo_store or state_db
    assert!(!migrated.has_unique_id(storage::UID_PREFIX_NFT, &token_id));

    // Add to state_db but NOT to utxo_store — has_unique_id should still be false
    // because during apply_block it routes through utxo_store
    sdb.add_unique_id(storage::UID_PREFIX_NFT, &token_id);

    // has_unique_id on UtxoSet routes through utxo_store for apply_block safety
    // So it should NOT find the one we added to state_db only
    // (This test verifies the routing stays on utxo_store, not state_db)
    // Actually — we keep has_unique_id on utxo_store to preserve same-block behavior.
    // If has_unique_id was routed to state_db, it WOULD find it. But we specifically
    // keep it on utxo_store.
    assert!(
        !migrated.has_unique_id(storage::UID_PREFIX_NFT, &token_id),
        "has_unique_id should route through utxo_store, not state_db"
    );
}
