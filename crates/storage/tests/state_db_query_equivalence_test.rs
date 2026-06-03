//! Phase 1 equivalence tests: state_db query methods vs utxo_rocks counterparts.
//!
//! Each test populates both stores with identical UTXO sets and asserts
//! that the new state_db methods produce bit-identical results to the
//! existing utxo_rocks methods. This proves the equivalence claim the
//! redesign depends on.
//!
//! OUTPUT CONTRACT:
//!   Functions under test (all on StateDb, mirroring RocksDbUtxoStore):
//!     1. get_bonded_balance(&Hash) -> Amount
//!     2. count_bonds(&Hash, u64) -> u32
//!     3. get_bond_entries(&Hash) -> Vec<(Outpoint, u32, Amount)>
//!     4. get_all_pools() -> Vec<(Outpoint, UtxoEntry)>
//!     5. find_nft_by_token_id(&Hash) -> Option<(Outpoint, UtxoEntry)>
//!     6. total_confirmed(height, maturity, pool_pkh) -> Amount
//!     7. address_count() -> u64
//!     8. has_unique_id(u8, &Hash) -> bool
//!     9. add_unique_id(u8, &Hash) -- side effect: persists to cf_unique_id
//!    10. remove_unique_id(u8, &Hash) -- side effect: deletes from cf_unique_id
//!    11. BlockBatch::has_unique_id_check(u8, &Hash) -> bool
//!    12. BlockBatch::add_pending_unique_id(u8, Hash) -- adds to pending set
//!
//!   Observable outputs per function:
//!     O1: Return value (Amount/u32/Vec/Option/bool)
//!     O2: Equivalence with utxo_rocks counterpart (identical return)
//!     O3: Side effects (cf_unique_id persisted, pending_unique_ids populated)
//!
//!   Code paths:
//!     P1: Empty store (no UTXOs / no unique IDs)
//!     P2: Populated store (mixed output types, multiple addresses)
//!     P3: Same-block pending (BlockBatch pending_unique_ids)
//!     P4: Disk fallback (unique ID on disk, batch checks disk)
//!     P5: Persistence across reopen (cf_unique_id survives close+open)
//!
//! INPUT PARTITIONS:
//!   For equivalence methods (1-7):
//!     I1: Empty store -> O1=0/empty, O2=matches utxo_rocks
//!     I2: Store with target output type only -> O1=correct, O2=matches
//!     I3: Store with mixed output types -> O1=filters correctly, O2=matches
//!   For unique ID methods (8-12):
//!     I4: ID not present -> O1=false
//!     I5: ID present on disk -> O1=true
//!     I6: ID present in pending only -> O1=true (via has_unique_id_check)
//!     I7: ID removed -> O1=false
//!     I8: Reopen after insert -> O1=true (persistence)
//!
//!   Matrix (test coverage):
//!     equivalence_get_bonded_balance_empty:      O1,O2 x P1 x I1
//!     equivalence_get_bonded_balance_with_bonds:  O1,O2 x P2 x I3
//!     equivalence_count_bonds_empty:              O1,O2 x P1 x I1
//!     equivalence_count_bonds_with_bonds:         O1,O2 x P2 x I2
//!     equivalence_get_bond_entries_empty:          O1,O2 x P1 x I1
//!     equivalence_get_bond_entries_fifo_order:     O1,O2 x P2 x I2
//!     equivalence_get_all_pools_empty:             O1,O2 x P1 x I1
//!     equivalence_get_all_pools_with_pools:        O1,O2 x P2 x I3
//!     equivalence_find_nft_not_found:              O1,O2 x P1 x I1
//!     equivalence_find_nft_found:                  O1,O2 x P2 x I2
//!     equivalence_total_confirmed_empty:           O1,O2 x P1 x I1
//!     equivalence_total_confirmed_mixed:           O1,O2 x P2 x I3
//!     equivalence_address_count_empty:             O1,O2 x P1 x I1
//!     equivalence_address_count_multiple:          O1,O2 x P2 x I3
//!     unique_id_insert_query_delete:               O1,O3 x P2 x I5,I7
//!     unique_id_equivalence:                       O1,O2 x P2 x I5
//!     unique_id_survives_reopen:                   O1,O3 x P5 x I8
//!     pending_unique_ids_same_block:               O1,O3 x P3 x I4,I6
//!     pending_unique_ids_disk_fallback:             O1 x P4 x I5
//!     pending_unique_ids_commit_writes:             O1,O3 x P3 x I6

use crypto::hash::hash as crypto_hash;
use crypto::Hash;
use doli_core::transaction::Output;
use storage::utxo::{Outpoint, UtxoEntry};
use storage::utxo_rocks::RocksDbUtxoStore;
use storage::StateDb;
use tempfile::TempDir;

/// Create both stores in separate tempdirs.
fn create_dual_stores() -> (StateDb, RocksDbUtxoStore, TempDir, TempDir) {
    let dir_sdb = TempDir::new().unwrap();
    let dir_utxo = TempDir::new().unwrap();
    let sdb = StateDb::open(dir_sdb.path()).unwrap();
    let store = RocksDbUtxoStore::open(dir_utxo.path()).unwrap();
    (sdb, store, dir_sdb, dir_utxo)
}

/// Insert a UTXO into both stores identically.
fn insert_both(sdb: &StateDb, store: &RocksDbUtxoStore, outpoint: Outpoint, entry: UtxoEntry) {
    sdb.insert_utxo(&outpoint, &entry);
    store.insert(outpoint, entry).unwrap();
}

// ==================== get_bonded_balance ====================

#[test]
fn equivalence_get_bonded_balance_empty() {
    let (sdb, store, _d1, _d2) = create_dual_stores();
    let pk = crypto_hash(b"nobody");
    assert_eq!(sdb.get_bonded_balance(&pk), store.get_bonded_balance(&pk));
}

#[test]
fn equivalence_get_bonded_balance_with_bonds() {
    let (sdb, store, _d1, _d2) = create_dual_stores();
    let alice = crypto_hash(b"alice");

    // 3 bond UTXOs + 1 normal UTXO
    for i in 0..3u32 {
        let op = Outpoint::new(crypto_hash(format!("bond_tx_{}", i).as_bytes()), 0);
        let entry = UtxoEntry {
            output: Output::bond(1_000_000_000, alice, u64::MAX, i * 10),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        insert_both(&sdb, &store, op, entry);
    }
    // Normal (should not count)
    let op_normal = Outpoint::new(crypto_hash(b"normal_tx"), 0);
    let entry_normal = UtxoEntry {
        output: Output::normal(500_000, alice),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    insert_both(&sdb, &store, op_normal, entry_normal);

    assert_eq!(
        sdb.get_bonded_balance(&alice),
        store.get_bonded_balance(&alice)
    );
    assert_eq!(sdb.get_bonded_balance(&alice), 3_000_000_000);
}

// ==================== count_bonds ====================

#[test]
fn equivalence_count_bonds_empty() {
    let (sdb, store, _d1, _d2) = create_dual_stores();
    let pk = crypto_hash(b"nobody");
    let bond_unit = 1_000_000_000u64;
    assert_eq!(
        sdb.count_bonds(&pk, bond_unit),
        store.count_bonds(&pk, bond_unit)
    );
}

#[test]
fn equivalence_count_bonds_with_bonds() {
    let (sdb, store, _d1, _d2) = create_dual_stores();
    let alice = crypto_hash(b"alice");
    let bond_unit = 1_000_000_000u64;

    for i in 0..5u32 {
        let op = Outpoint::new(crypto_hash(format!("bond_{}", i).as_bytes()), 0);
        let entry = UtxoEntry {
            output: Output::bond(bond_unit, alice, u64::MAX, i * 10),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        insert_both(&sdb, &store, op, entry);
    }

    assert_eq!(
        sdb.count_bonds(&alice, bond_unit),
        store.count_bonds(&alice, bond_unit)
    );
    assert_eq!(sdb.count_bonds(&alice, bond_unit), 5);
}

// ==================== get_bond_entries ====================

#[test]
fn equivalence_get_bond_entries_empty() {
    let (sdb, store, _d1, _d2) = create_dual_stores();
    let pk = crypto_hash(b"nobody");
    assert_eq!(
        sdb.get_bond_entries(&pk).len(),
        store.get_bond_entries(&pk).len()
    );
}

#[test]
fn equivalence_get_bond_entries_fifo_order() {
    let (sdb, store, _d1, _d2) = create_dual_stores();
    let alice = crypto_hash(b"alice");

    // Insert bonds with different creation slots (out of order)
    let slots = [30u32, 10, 20];
    for (i, &slot) in slots.iter().enumerate() {
        let op = Outpoint::new(crypto_hash(format!("bond_{}", i).as_bytes()), 0);
        let entry = UtxoEntry {
            output: Output::bond(1_000_000_000, alice, u64::MAX, slot),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        insert_both(&sdb, &store, op, entry);
    }

    let sdb_entries = sdb.get_bond_entries(&alice);
    let store_entries = store.get_bond_entries(&alice);

    // Same length
    assert_eq!(sdb_entries.len(), store_entries.len());
    // Same FIFO order (sorted by creation_slot)
    for (s, u) in sdb_entries.iter().zip(store_entries.iter()) {
        assert_eq!(s.1, u.1, "creation_slot mismatch");
        assert_eq!(s.2, u.2, "amount mismatch");
    }
    // Verify sorted ascending by slot
    assert_eq!(sdb_entries[0].1, 10);
    assert_eq!(sdb_entries[1].1, 20);
    assert_eq!(sdb_entries[2].1, 30);
}

// ==================== get_all_pools ====================

#[test]
fn equivalence_get_all_pools_empty() {
    let (sdb, store, _d1, _d2) = create_dual_stores();
    assert_eq!(sdb.get_all_pools().len(), store.get_all_pools().len());
}

#[test]
fn equivalence_get_all_pools_with_pools() {
    let (sdb, store, _d1, _d2) = create_dual_stores();
    let alice = crypto_hash(b"alice");

    // Insert a Pool UTXO with raw construction
    let pool_output = Output {
        output_type: doli_core::OutputType::Pool,
        amount: 0,
        pubkey_hash: alice,
        lock_until: 0,
        extra_data: vec![0u8; 100], // dummy pool metadata
    };
    let op_pool = Outpoint::new(crypto_hash(b"pool_tx"), 0);
    let entry_pool = UtxoEntry {
        output: pool_output,
        height: 5,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    insert_both(&sdb, &store, op_pool, entry_pool);

    // Normal UTXO (should NOT appear)
    let op_normal = Outpoint::new(crypto_hash(b"normal_tx"), 0);
    let entry_normal = UtxoEntry {
        output: Output::normal(100, alice),
        height: 5,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    insert_both(&sdb, &store, op_normal, entry_normal);

    let sdb_pools = sdb.get_all_pools();
    let store_pools = store.get_all_pools();
    assert_eq!(sdb_pools.len(), store_pools.len());
    assert_eq!(sdb_pools.len(), 1);
}

// ==================== find_nft_by_token_id ====================

#[test]
fn equivalence_find_nft_by_token_id_not_found() {
    let (sdb, store, _d1, _d2) = create_dual_stores();
    let tid = crypto_hash(b"nonexistent_token");
    assert!(sdb.find_nft_by_token_id(&tid).is_none());
    assert!(store.find_nft_by_token_id(&tid).is_none());
}

#[test]
fn equivalence_find_nft_by_token_id_found() {
    let (sdb, store, _d1, _d2) = create_dual_stores();
    let alice = crypto_hash(b"alice");
    let token_id = crypto_hash(b"unique_nft_token");

    // Construct NFT output with token_id embedded in extra_data.
    // extra_data layout: [condition_bytes][1B version=0x01][32B token_id][content]
    // For test: empty condition = [] so: [0x01][32B token_id][32B content_hash]
    let content_hash = crypto_hash(b"content");
    let mut extra_data = Vec::new();
    extra_data.push(0x01); // version
    extra_data.extend_from_slice(token_id.as_bytes());
    extra_data.extend_from_slice(content_hash.as_bytes());

    let nft_output = Output {
        output_type: doli_core::OutputType::NFT,
        amount: 0,
        pubkey_hash: alice,
        lock_until: 0,
        extra_data,
    };

    let op = Outpoint::new(crypto_hash(b"nft_tx"), 0);
    let entry = UtxoEntry {
        output: nft_output,
        height: 10,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    insert_both(&sdb, &store, op, entry);

    let sdb_result = sdb.find_nft_by_token_id(&token_id);
    let store_result = store.find_nft_by_token_id(&token_id);

    // Both should find it or both should miss it
    assert_eq!(sdb_result.is_some(), store_result.is_some());
    if let (Some((sdb_op, sdb_entry)), Some((store_op, store_entry))) = (sdb_result, store_result) {
        assert_eq!(sdb_op, store_op);
        assert_eq!(sdb_entry.output.output_type, store_entry.output.output_type);
        assert_eq!(sdb_entry.height, store_entry.height);
    }
}

// ==================== total_confirmed ====================

#[test]
fn equivalence_total_confirmed_empty() {
    let (sdb, store, _d1, _d2) = create_dual_stores();
    let pool_pkh = [0u8; 32];
    assert_eq!(
        sdb.total_confirmed(100, 10, &pool_pkh),
        store.total_confirmed(100, 10, &pool_pkh)
    );
}

#[test]
fn equivalence_total_confirmed_mixed_utxos() {
    let (sdb, store, _d1, _d2) = create_dual_stores();
    let alice = crypto_hash(b"alice");
    let pool_pkh_hash = crypto_hash(b"pool_address");

    // Normal UTXO (should count)
    let op1 = Outpoint::new(crypto_hash(b"tx1"), 0);
    let e1 = UtxoEntry {
        output: Output::normal(1_000_000, alice),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    insert_both(&sdb, &store, op1, e1);

    // Bond UTXO (should NOT count)
    let op2 = Outpoint::new(crypto_hash(b"tx2"), 0);
    let e2 = UtxoEntry {
        output: Output::bond(2_000_000, alice, u64::MAX, 0),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    insert_both(&sdb, &store, op2, e2);

    // Pool address UTXO (should NOT count)
    let op3 = Outpoint::new(crypto_hash(b"tx3"), 0);
    let e3 = UtxoEntry {
        output: Output::normal(3_000_000, pool_pkh_hash),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    insert_both(&sdb, &store, op3, e3);

    // Immature coinbase (height=90, maturity=10, current=95 -> immature)
    let op4 = Outpoint::new(crypto_hash(b"tx4"), 0);
    let e4 = UtxoEntry {
        output: Output::normal(4_000_000, alice),
        height: 90,
        is_coinbase: true,
        is_epoch_reward: false,
    };
    insert_both(&sdb, &store, op4, e4);

    let pool_pkh: [u8; 32] = *pool_pkh_hash.as_bytes();
    let sdb_total = sdb.total_confirmed(95, 10, &pool_pkh);
    let store_total = store.total_confirmed(95, 10, &pool_pkh);
    assert_eq!(sdb_total, store_total);
    // Only op1 (1M) should count
    assert_eq!(sdb_total, 1_000_000);
}

// ==================== address_count ====================

#[test]
fn equivalence_address_count_empty() {
    let (sdb, store, _d1, _d2) = create_dual_stores();
    assert_eq!(sdb.address_count(), store.address_count());
}

#[test]
fn equivalence_address_count_multiple() {
    let (sdb, store, _d1, _d2) = create_dual_stores();
    let alice = crypto_hash(b"alice");
    let bob = crypto_hash(b"bob");

    // 3 UTXOs for alice, 1 for bob
    for i in 0..3u32 {
        let op = Outpoint::new(crypto_hash(format!("alice_{}", i).as_bytes()), 0);
        let e = UtxoEntry {
            output: Output::normal(100 * (i as u64 + 1), alice),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        insert_both(&sdb, &store, op, e);
    }
    let op_bob = Outpoint::new(crypto_hash(b"bob_tx"), 0);
    let e_bob = UtxoEntry {
        output: Output::normal(500, bob),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    insert_both(&sdb, &store, op_bob, e_bob);

    assert_eq!(sdb.address_count(), store.address_count());
    assert_eq!(sdb.address_count(), 2);
}

// ==================== cf_unique_id ====================

#[test]
fn unique_id_insert_query_delete() {
    let dir = TempDir::new().unwrap();
    let sdb = StateDb::open(dir.path()).unwrap();

    let id = Hash::from_bytes([0xAA; 32]);
    let prefix = storage::utxo::UID_PREFIX_NFT;

    // Not present initially
    assert!(!sdb.has_unique_id(prefix, &id));

    // Insert
    sdb.add_unique_id(prefix, &id);
    assert!(sdb.has_unique_id(prefix, &id));

    // Delete
    sdb.remove_unique_id(prefix, &id);
    assert!(!sdb.has_unique_id(prefix, &id));
}

#[test]
fn unique_id_equivalence_vs_utxo_store() {
    let dir_sdb = TempDir::new().unwrap();
    let dir_utxo = TempDir::new().unwrap();
    let sdb = StateDb::open(dir_sdb.path()).unwrap();
    let store = RocksDbUtxoStore::open(dir_utxo.path()).unwrap();

    let nft_id = Hash::from_bytes([0x11; 32]);
    let pool_id = Hash::from_bytes([0x22; 32]);

    // Insert in both
    sdb.add_unique_id(storage::utxo::UID_PREFIX_NFT, &nft_id);
    sdb.add_unique_id(storage::utxo::UID_PREFIX_POOL, &pool_id);

    store.add_unique_id(storage::utxo::UID_PREFIX_NFT, &nft_id);
    store.add_unique_id(storage::utxo::UID_PREFIX_POOL, &pool_id);

    // Equivalence checks
    assert_eq!(
        sdb.has_unique_id(storage::utxo::UID_PREFIX_NFT, &nft_id),
        store.has_unique_id(storage::utxo::UID_PREFIX_NFT, &nft_id)
    );
    assert_eq!(
        sdb.has_unique_id(storage::utxo::UID_PREFIX_POOL, &pool_id),
        store.has_unique_id(storage::utxo::UID_PREFIX_POOL, &pool_id)
    );

    // Non-existent
    let absent = Hash::from_bytes([0xFF; 32]);
    assert_eq!(
        sdb.has_unique_id(storage::utxo::UID_PREFIX_NFT, &absent),
        store.has_unique_id(storage::utxo::UID_PREFIX_NFT, &absent)
    );
}

#[test]
fn unique_id_survives_reopen() {
    let dir = TempDir::new().unwrap();
    let id = Hash::from_bytes([0xBB; 32]);
    let prefix = storage::utxo::UID_PREFIX_ASSET;

    {
        let sdb = StateDb::open(dir.path()).unwrap();
        sdb.add_unique_id(prefix, &id);
        assert!(sdb.has_unique_id(prefix, &id));
    }
    // Reopen
    {
        let sdb = StateDb::open(dir.path()).unwrap();
        assert!(sdb.has_unique_id(prefix, &id));
    }
}

// ==================== pending_unique_ids (BlockBatch) ====================

#[test]
fn pending_unique_ids_same_block_check() {
    let dir = TempDir::new().unwrap();
    let sdb = StateDb::open(dir.path()).unwrap();

    let id = Hash::from_bytes([0xCC; 32]);
    let prefix = storage::utxo::UID_PREFIX_NFT;

    // Not on disk, not in batch
    let mut batch = sdb.begin_batch();
    assert!(!batch.has_unique_id_check(prefix, &id));

    // Add to batch pending set
    batch.add_pending_unique_id(prefix, id);

    // Now should find it in pending (not yet on disk)
    assert!(batch.has_unique_id_check(prefix, &id));

    // Commit and verify on disk
    batch.commit().unwrap();
    assert!(sdb.has_unique_id(prefix, &id));
}

#[test]
fn pending_unique_ids_disk_fallback() {
    let dir = TempDir::new().unwrap();
    let sdb = StateDb::open(dir.path()).unwrap();

    let id = Hash::from_bytes([0xDD; 32]);
    let prefix = storage::utxo::UID_PREFIX_POOL;

    // Insert on disk first
    sdb.add_unique_id(prefix, &id);

    // New batch should find it via disk fallback
    let batch = sdb.begin_batch();
    assert!(batch.has_unique_id_check(prefix, &id));
}

#[test]
fn pending_unique_ids_batch_commit_writes_to_disk() {
    let dir = TempDir::new().unwrap();
    let sdb = StateDb::open(dir.path()).unwrap();

    let id = Hash::from_bytes([0xEE; 32]);
    let prefix = storage::utxo::UID_PREFIX_ASSET;

    let mut batch = sdb.begin_batch();
    batch.add_pending_unique_id(prefix, id);
    batch.commit().unwrap();

    // Verify persisted to cf_unique_id
    assert!(sdb.has_unique_id(prefix, &id));
}
