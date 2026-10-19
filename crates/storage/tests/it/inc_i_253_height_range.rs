// INC-I-253 M1 — numeric BlockStore::height_range + archive backfill loop shape.
// covers: maintenance.rs height_range, periodic.rs integrity_scan, integrity.rs scan_floor, mod.rs
//
// OUTPUT CONTRACT: fn BlockStore::height_range(&self)
//   O1: none (no mutable params)
//   O2: self — read-only; height_index[0] (virtual genesis) must survive the call
//   O3: return — Ok(Some((min, max))) NUMERIC over indexed heights >= 1; Ok(None) if none
//   O4: none
// OUTPUT CONTRACT: fn archiver::backfill_from_archive(dir, store, Some(&genesis))
//   O3: return — Ok(imported)
//   O4: block store — every h in 1..=manifest served, same hash as before
// PATHS: P1 index has heights >= 1, P2 empty index, P3 index holds only key 0
// INPUT PARTITIONS:
//   P1a key 0 + 1..=300 (put_block + set_canonical_chain) — byte order (0, 255), numeric (1, 300)
//   P1b 1..=256 (put_block_canonical), no key 0 — byte order (256, 255), numeric (1, 256)
//   P2  empty store — None
//   P3  key 0 only (block 1 applied, then deleted) — byte order (0, 0), numeric None
//   P4  backfill on complete key-0 store + archive 1..=300 — Ok(0), store unchanged
// MATRIX: height_range O2/O3 × {P1a, P1b, P2, P3} = 5 cells; backfill O3/O4 × P4 = 2 cells

use crypto::{Hash, KeyPair};
use doli_core::{Block, BlockHeader};
use storage::archiver::{backfill_from_archive, BlockArchiver};
use storage::BlockStore;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

fn genesis() -> Hash {
    crypto::hash::hash(b"inc-i-253-genesis")
}

fn block_at(genesis: Hash, h: u64, prev: Hash) -> Block {
    let kp = KeyPair::from_seed([9u8; 32]);
    let header = BlockHeader {
        version: 1,
        prev_hash: prev,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: genesis,
        timestamp: 10_000 + h,
        slot: h as u32,
        producer: *kp.public_key(),
        vdf_output: VdfOutput { value: Vec::new() },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![])
}

/// Production apply path: put_block + set_canonical_chain, block 1 links to genesis.
fn applied_chain(store: &BlockStore, g: Hash, n: u64) -> Vec<Hash> {
    let mut prev = g;
    let mut hashes = Vec::with_capacity(n as usize);
    for h in 1..=n {
        let b = block_at(g, h, prev);
        let hash = b.hash();
        store.put_block(&b, h).unwrap();
        store.set_canonical_chain(hash, h).unwrap();
        hashes.push(hash);
        prev = hash;
    }
    hashes
}

fn open_store() -> (BlockStore, TempDir) {
    let tmp = TempDir::new().unwrap();
    let store = BlockStore::open(tmp.path()).unwrap();
    (store, tmp)
}

// REQ-INTEG-009 — Decision: a byte-order (0, 255) result means the scan floor and getStorageInfo still read LE keys lexicographically
#[test]
fn inc_i_253_height_range_key0_store_is_numeric_1_to_300() {
    let (store, _tmp) = open_store();
    let g = genesis();
    let hashes = applied_chain(&store, g, 300);

    assert_eq!(
        store.get_hash_by_height(0).unwrap(),
        Some(g),
        "precondition: set_canonical_chain wrote the virtual-genesis key 0"
    );
    for h in 1..=300u64 {
        let b = store.get_block_by_height(h).unwrap();
        assert_eq!(b.map(|b| b.hash()), Some(hashes[(h - 1) as usize]), "h={h}");
    }

    let range = store.height_range().unwrap();
    assert_eq!(range, Some((1, 300)), "observed height_range = {range:?}");
    assert_eq!(
        store.get_hash_by_height(0).unwrap(),
        Some(g),
        "height_range must not touch the virtual-genesis key 0 (REQ-INTEG-013 Won't)"
    );
}

// REQ-INTEG-009 — Decision: (256, 255) here is the blind spot that hides holes below 256 from the 100-block scan
#[test]
fn inc_i_253_height_range_no_key0_256_is_numeric() {
    let (store, _tmp) = open_store();
    let g = genesis();
    let mut prev = g;
    for h in 1..=256u64 {
        let b = block_at(g, h, prev);
        prev = b.hash();
        store.put_block_canonical(&b, h).unwrap();
    }
    assert_eq!(
        store.get_hash_by_height(0).unwrap(),
        None,
        "precondition: no key 0"
    );

    let range = store.height_range().unwrap();
    assert_eq!(range, Some((1, 256)), "observed height_range = {range:?}");
}

// REQ-INTEG-009 — Decision: a Some on an empty store would hand the scan a fabricated floor/tip
#[test]
fn inc_i_253_height_range_empty_store_is_none() {
    let (store, _tmp) = open_store();
    assert_eq!(store.height_range().unwrap(), None);
}

// REQ-INTEG-009 — Decision: Some((0, 0)) means the virtual genesis is still counted as a stored block height
#[test]
fn inc_i_253_height_range_only_key0_is_none() {
    let (store, _tmp) = open_store();
    let g = genesis();
    applied_chain(&store, g, 1);
    assert!(store.delete_block_by_height(1).unwrap(), "block 1 deleted");
    assert_eq!(
        store.get_hash_by_height(0).unwrap(),
        Some(g),
        "precondition: key 0 remains"
    );
    assert_eq!(
        store.get_hash_by_height(1).unwrap(),
        None,
        "precondition: no h >= 1"
    );

    let range = store.height_range().unwrap();
    assert_eq!(range, None, "observed height_range = {range:?}");
}

// REQ-INTEG-001 — Decision: a non-zero import on a complete store means backfill visits a height the scan disagrees on
#[test]
fn inc_i_253_backfill_complete_key0_store_imports_zero() {
    let (store, _tmp) = open_store();
    let g = genesis();
    let hashes = applied_chain(&store, g, 300);
    let archive = TempDir::new().unwrap();
    BlockArchiver::catch_up(archive.path(), &store, 300, Some(&g)).unwrap();
    for h in [1u64, 150, 300] {
        assert!(
            archive.path().join(format!("{:010}.block", h)).exists(),
            "precondition: archive file {h}"
        );
    }
    assert!(archive.path().join("manifest.json").exists());

    let imported = backfill_from_archive(archive.path(), &store, Some(&g));
    assert_eq!(imported, Ok(0), "observed backfill = {imported:?}");

    for h in 1..=300u64 {
        let b = store.get_block_by_height(h).unwrap();
        assert_eq!(b.map(|b| b.hash()), Some(hashes[(h - 1) as usize]), "h={h}");
    }
    assert_eq!(store.get_hash_by_height(0).unwrap(), Some(g));
}
