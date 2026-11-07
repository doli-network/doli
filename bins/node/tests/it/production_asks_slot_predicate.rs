//! Note: a slot entry with no header is unreachable by the public store API on an undamaged store (every delete path removes the slot entry with the header), so these tests damage the store raw.
//! The predicate behind both slot stops, on a real store, one test per branch.
//!
//! covers: mod gates production_asks
//
// OUTPUT CONTRACT: fn slot_block_on_my_branch(store: &BlockStore, slot: u32, my_tip: &Hash) -> bool
//   O3: return — true iff the slot's block has a parent on my branch. O1, O2, O4, O5, O6: none; reads only.
// PATHS: P1 no usable slot entry | P2 no usable header | P3 parent == my_tip
//   P4 parent indexed in both directions | P5 anything else, any store error
// INPUT PARTITIONS:
//   A: slot empty                                   -> P1 false
//   B: slot value of wrong length (read Err)        -> P1 false
//   C: slot entry, header deleted                   -> P2 false
//   D: slot entry, header bytes garbage (read Err)  -> P2 false
//   E: parent == my_tip, parent not indexed         -> P3 true
//   F: parent canonical, not my_tip                 -> P4 true
//   G: parent known, not canonical                  -> P5 false
//   H: parent unknown                               -> P5 false
//   I: hash_to_height -> h, height h names another  -> P5 false
//   J: hash_to_height -> h, height h empty          -> P5 false
//   K: hash_to_height value damaged (read Err)      -> P5 false
//   L: height index value damaged (read Err)        -> P5 false
// MATRIX: O3 x {A..L}, one test each. In B, C, D, K, L the parent is my tip's child or
//   canonical except for the damage, and each damage is shown as a store read first.
// REQ-PASK-SEC-003 — Decision: a grep hit in the names and messages of this file would reveal a public-detail leak; checked by grep, no runtime test.

use std::path::Path;

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::Block;
use doli_node::node::slot_block_on_my_branch;
use storage::BlockStore;
use tempfile::TempDir;

use super::inc_i_289_common::block;

const CF_SLOT: &str = "slot_index";
const CF_HEADERS: &str = "headers";
const CF_HEIGHT: &str = "height_index";
const CF_H2H: &str = "hash_to_height";
const SLOT: u32 = 1000;
const BAD_LEN: [u8; 3] = [1, 2, 3];

struct Fx {
    store: BlockStore,
    key: KeyPair,
    /// `chain[i]` is canonical at height `i + 1`.
    chain: Vec<Block>,
    my_tip: Hash,
    dir: TempDir,
}

/// A store with canonical heights 1..=3.
fn fx() -> Fx {
    let dir = TempDir::new().unwrap();
    let store = BlockStore::open(dir.path()).expect("fixture: open store");
    let key = KeyPair::generate();
    let mut prev = Hash::ZERO;
    let mut chain = Vec::new();
    for h in 1..=3u64 {
        let b = block(h, h as u32, prev, &key, &ConsensusParams::devnet());
        store.put_block_canonical(&b, h).expect("fixture: chain");
        prev = b.hash();
        chain.push(b);
    }
    Fx {
        store,
        key,
        chain,
        my_tip: prev,
        dir,
    }
}

/// Writes a block for `SLOT` on `parent` by the public API; returns its hash.
fn seed_slot(f: &Fx, parent: Hash) -> Hash {
    let b = block(4, SLOT, parent, &f.key, &ConsensusParams::devnet());
    f.store.put_block(&b, 4).expect("fixture: put_block");
    b.hash()
}

/// Closes the store, applies raw edits, and opens the store again.
fn damage(f: Fx, edit: impl FnOnce(&rocksdb::DB)) -> Fx {
    let Fx {
        store,
        key,
        chain,
        my_tip,
        dir,
    } = f;
    drop(store);
    raw_edit(dir.path(), edit);
    let store = BlockStore::open(dir.path()).expect("fixture: reopen store");
    Fx {
        store,
        key,
        chain,
        my_tip,
        dir,
    }
}

fn raw_edit(path: &Path, edit: impl FnOnce(&rocksdb::DB)) {
    let opts = rocksdb::Options::default();
    let names = rocksdb::DB::list_cf(&opts, path).expect("fixture: list column families");
    for cf in [CF_SLOT, CF_HEADERS, CF_HEIGHT, CF_H2H] {
        assert!(names.iter().any(|n| n == cf), "fixture: column family {cf}");
    }
    let db = rocksdb::DB::open_cf(&opts, path, &names).expect("fixture: raw open");
    edit(&db);
}

fn raw_put(db: &rocksdb::DB, cf: &str, key: &[u8], value: &[u8]) {
    let handle = db.cf_handle(cf).expect("fixture: column family");
    db.put_cf(handle, key, value).expect("fixture: raw put");
}

fn raw_delete(db: &rocksdb::DB, cf: &str, key: &[u8]) {
    let handle = db.cf_handle(cf).expect("fixture: column family");
    db.delete_cf(handle, key).expect("fixture: raw delete");
}

fn on_my_branch(f: &Fx) -> bool {
    slot_block_on_my_branch(&f.store, SLOT, &f.my_tip)
}

// REQ-PASK-005 — Decision: a failure means an empty slot stops a build.
#[test]
fn slot_empty_is_not_on_my_branch() {
    let f = fx();
    assert!(!on_my_branch(&f), "A: empty slot must return false");
}

// REQ-PASK-005 — Decision: a failure means a damaged slot index value stops every build in that slot.
#[test]
fn slot_value_of_wrong_length_is_not_on_my_branch() {
    let f = damage(fx(), |db| {
        raw_put(db, CF_SLOT, &SLOT.to_le_bytes(), &BAD_LEN)
    });
    assert!(
        f.store.get_hash_by_slot(SLOT).is_err(),
        "B: precondition: the slot read errors"
    );
    assert!(!on_my_branch(&f), "B: slot read error must return false");
}

// REQ-PASK-005 — Decision: a failure means a slot entry with no header stops a build, so a missing header fails closed.
#[test]
fn slot_entry_with_deleted_header_is_not_on_my_branch() {
    let f = fx();
    let tip = f.my_tip;
    let sibling = seed_slot(&f, tip);
    let f = damage(f, |db| raw_delete(db, CF_HEADERS, sibling.as_bytes()));
    assert_eq!(
        f.store.get_hash_by_slot(SLOT).expect("store read"),
        Some(sibling),
        "C: precondition: the slot entry stays"
    );
    assert!(
        f.store.get_header(&sibling).expect("store read").is_none(),
        "C: precondition: the header is gone"
    );
    assert!(!on_my_branch(&f), "C: missing header must return false");
}

// REQ-PASK-005 — Decision: a failure means undecodable header bytes stop a build, so a header read error fails closed.
#[test]
fn slot_entry_with_garbage_header_is_not_on_my_branch() {
    let f = fx();
    let tip = f.my_tip;
    let sibling = seed_slot(&f, tip);
    let f = damage(f, |db| {
        raw_put(db, CF_HEADERS, sibling.as_bytes(), &BAD_LEN)
    });
    assert!(
        f.store.get_header(&sibling).is_err(),
        "D: precondition: the header read errors"
    );
    assert!(!on_my_branch(&f), "D: header read error must return false");
}

// REQ-PASK-005 — Decision: a failure means a sibling on my tip no longer stops a build unless the tip is also indexed.
#[test]
fn parent_equal_to_my_tip_is_on_my_branch() {
    let mut f = fx();
    let tip = crypto::hash::hash(b"tip with no index entry");
    f.my_tip = tip;
    seed_slot(&f, tip);
    assert_eq!(
        f.store.get_height_by_hash(&tip).expect("store read"),
        None,
        "E: precondition: only the tip compare can answer"
    );
    assert!(on_my_branch(&f), "E: parent == my_tip must return true");
}

// REQ-PASK-005 — Decision: a failure means a sibling on an earlier block of my branch no longer stops a build.
#[test]
fn canonical_parent_is_on_my_branch() {
    let f = fx();
    let parent = f.chain[0].hash();
    assert_ne!(
        parent, f.my_tip,
        "F: precondition: the parent is not my tip"
    );
    seed_slot(&f, parent);
    assert!(on_my_branch(&f), "F: canonical parent must return true");
}

// REQ-PASK-005 — Decision: a failure means a known block of another branch stops a build, which is the behaviour this predicate removes.
#[test]
fn known_parent_off_my_branch_is_not_on_my_branch() {
    let f = fx();
    let off = block(2, 900, Hash::ZERO, &f.key, &ConsensusParams::devnet());
    f.store.put_block(&off, 2).expect("fixture: put_block");
    let parent = off.hash();
    assert_eq!(
        f.store.get_height_by_hash(&parent).expect("store read"),
        None,
        "G: precondition: the parent is not canonical"
    );
    seed_slot(&f, parent);
    assert!(
        !on_my_branch(&f),
        "G: known non-canonical parent must return false"
    );
}

// REQ-PASK-005 — Decision: a failure means a block on a parent this node has never seen stops a build.
#[test]
fn unknown_parent_is_not_on_my_branch() {
    let f = fx();
    seed_slot(&f, crypto::hash::hash(b"parent the store has never seen"));
    assert!(!on_my_branch(&f), "H: unknown parent must return false");
}

// REQ-PASK-005 — Decision: a failure means a stale reverse index entry alone puts a parent on my branch, so the height index is not read back.
#[test]
fn parent_with_stale_reverse_index_is_not_on_my_branch() {
    let f = fx();
    let parent = crypto::hash::hash(b"parent with a stale reverse entry");
    let current = crypto::hash::hash(b"current hash at that height");
    let store = &f.store;
    store
        .seed_canonical_index(parent, 7)
        .expect("fixture: seed");
    store
        .seed_canonical_index(current, 7)
        .expect("fixture: seed");
    assert_eq!(
        store.get_height_by_hash(&parent).expect("store read"),
        Some(7),
        "I: precondition: the reverse entry stays"
    );
    assert_eq!(
        store.get_hash_by_height(7).expect("store read"),
        Some(current),
        "I: precondition: the height names another hash"
    );
    seed_slot(&f, parent);
    assert!(
        !on_my_branch(&f),
        "I: height naming another hash must return false"
    );
}

// REQ-PASK-005 — Decision: a failure means a reverse entry with an empty height puts a parent on my branch.
#[test]
fn parent_with_empty_height_is_not_on_my_branch() {
    let f = fx();
    let parent = f.chain[0].hash();
    seed_slot(&f, parent);
    let f = damage(f, |db| raw_delete(db, CF_HEIGHT, &1u64.to_le_bytes()));
    assert_eq!(
        f.store.get_hash_by_height(1).expect("store read"),
        None,
        "J: precondition: the height is empty"
    );
    assert!(!on_my_branch(&f), "J: empty height must return false");
}

// REQ-PASK-005 — Decision: a failure means a reverse index read error stops a build, so the error fails closed.
#[test]
fn damaged_reverse_index_value_is_not_on_my_branch() {
    let f = fx();
    let parent = f.chain[0].hash();
    seed_slot(&f, parent);
    let f = damage(f, |db| raw_put(db, CF_H2H, parent.as_bytes(), &BAD_LEN));
    assert!(
        f.store.get_height_by_hash(&parent).is_err(),
        "K: precondition: the reverse read errors"
    );
    assert!(!on_my_branch(&f), "K: reverse read error must return false");
}

// REQ-PASK-005 — Decision: a failure means a height index read error stops a build, so the error fails closed.
#[test]
fn damaged_height_index_value_is_not_on_my_branch() {
    let f = fx();
    let parent = f.chain[0].hash();
    seed_slot(&f, parent);
    let f = damage(f, |db| {
        raw_put(db, CF_HEIGHT, &1u64.to_le_bytes(), &BAD_LEN)
    });
    assert!(
        f.store.get_hash_by_height(1).is_err(),
        "L: precondition: the height read errors"
    );
    assert!(!on_my_branch(&f), "L: height read error must return false");
}
