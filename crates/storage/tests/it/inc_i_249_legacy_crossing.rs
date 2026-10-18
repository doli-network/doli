//! INC-I-249 M1.5 — verified height crosses an intact legacy (INC-I-119) file.
//!
//! covers: crates/storage/src/archiver/catch_up.rs
//!
//! OUTPUT CONTRACT:
//!   fn BlockArchiver::catch_up_bounded(dir, store, tip, &G) -> io::Result<CatchUpReport>
//!     O1 report.verified_height / files_read / files_written
//!     O4a `{k:010}.block` bytes at the legacy height k
//!     O4b `{k:010}.blake3` at k
//!     O4c `manifest.json` key `verified_height`
//!   fn archive_finalized(dir, store, &ArchiveBlock, &G) -> io::Result<CatchUpReport>
//!     O1 report.verified_height; O4a/O4b at k and at the new height; O4c
//! INPUT PARTITIONS (file k fails `deserialize_block_compat`):
//!   L1 sidecar ok, header decodes, genesis G, empty store
//!   L2 sidecar wrong, header genesis G, store has block k
//!   L3 sidecar ok, header genesis G2, store has block k for G
//!   L4 sidecar ok, header genesis G2, empty store
//!   L5 sidecar ok, header does not decode, store has block k
//!   L6 L1, second start
//!   L7 L1 under archive_finalized (H = k-1, new block k+1), store has block k
//! MATRIX (O1 / O4a / O4b / O4c):
//!   L1 ✓/✓/✓/✓  L2 ✓/✓/✓/✓  L3 ✓/✓/✓/✓  L4 ✓/✓/✓/✓
//!   L5 ✓/✓/✓/✓  L6 ✓/✓/✓/✓  L7 ✓/✓/✓/✓

use std::path::Path;

use crypto::Hash;
use doli_core::transaction::legacy::deserialize_block_compat;
use doli_core::BlockHeader;
use storage::archiver::{archive_finalized, ArchiveBlock, BlockArchiver, CatchUpReport};
use storage::BlockStore;
use tempfile::TempDir;

use crate::inc_i_249_archive_catchup::{
    block_at, block_path, genesis_g, genesis_other, manifest_json, read_manifest, store_bytes,
    sum_path, write_manifest, write_pair,
};

const TIP: u64 = 20;
const K: u64 = 10;

/// Header of block `h` for `genesis`, then bytes no block format accepts.
fn legacy_bytes(genesis: Hash, h: u64) -> Vec<u8> {
    let mut data = bincode::serialize(&block_at(genesis, h).header).unwrap();
    data.extend_from_slice(&[0xFF; 64]);
    assert!(
        deserialize_block_compat(&data).is_none(),
        "fixture deserializes"
    );
    let header: BlockHeader = bincode::deserialize(&data).expect("fixture header decodes");
    assert_eq!(header.genesis_hash, genesis);
    data
}

fn valid_bytes(h: u64) -> Vec<u8> {
    bincode::serialize(&block_at(genesis_g(), h)).unwrap()
}

/// Pairs for 1..=last valid for G, file K holding `k_bytes`.
fn archive_with(last: u64, k_bytes: &[u8]) -> TempDir {
    let tmp = TempDir::new().unwrap();
    for h in 1..=last {
        let data = if h == K {
            k_bytes.to_vec()
        } else {
            valid_bytes(h)
        };
        write_pair(tmp.path(), h, &data);
    }
    tmp
}

fn store_with(heights: &[u64]) -> (BlockStore, TempDir) {
    let tmp = TempDir::new().unwrap();
    let store = BlockStore::open(tmp.path()).unwrap();
    for &h in heights {
        store
            .put_block_canonical(&block_at(genesis_g(), h), h)
            .unwrap();
    }
    (store, tmp)
}

fn snapshot(dir: &Path, h: u64) -> (Vec<u8>, Vec<u8>) {
    let block = std::fs::read(block_path(dir, h)).unwrap();
    (block, std::fs::read(sum_path(dir, h)).unwrap())
}

fn bounded(dir: &Path, store: &BlockStore) -> CatchUpReport {
    BlockArchiver::catch_up_bounded(dir, store, TIP, &genesis_g()).unwrap()
}

fn manifest_h(dir: &Path) -> Option<u64> {
    read_manifest(dir)["verified_height"].as_u64()
}

fn assert_sidecar_matches(dir: &Path, h: u64) {
    let got = std::fs::read(block_path(dir, h)).unwrap();
    let sum = std::fs::read_to_string(sum_path(dir, h)).unwrap();
    assert_eq!(
        sum.trim(),
        crypto::hash::hash(&got).to_string(),
        "sidecar {h}"
    );
}

/// H stops below K, nothing is written and the pair at K is unchanged.
fn assert_stops_below_k(dir: &Path, r: &CatchUpReport, before: &(Vec<u8>, Vec<u8>)) {
    assert_eq!(r.verified_height, K - 1, "H stops below file {K}");
    assert_eq!(manifest_h(dir), Some(K - 1), "manifest verified_height");
    assert_eq!(r.files_written, 0, "nothing written");
    assert_eq!(&snapshot(dir, K), before, "file {K} and sidecar untouched");
}

// REQ-ARCH-004 — Decision: a failure means H stops below an intact legacy file and every start re-reads the archive above it.
// REQ-ARCH-002 — Decision: a failure means an intact legacy file is rewritten (INC-I-119 skip lost).
#[test]
fn repro_h_advances_over_intact_legacy_file() {
    let archive = archive_with(TIP, &legacy_bytes(genesis_g(), K));
    let dir = archive.path();
    let before = snapshot(dir, K);
    let (store, _s) = store_with(&[]);

    let r = bounded(dir, &store);

    assert_eq!(
        r.verified_height, TIP,
        "H must cross the intact legacy file at {K}"
    );
    assert_eq!(manifest_h(dir), Some(TIP), "manifest verified_height");
    assert_eq!(r.files_written, 0, "nothing written");
    assert_eq!(
        snapshot(dir, K),
        before,
        "legacy file and sidecar never rewritten"
    );
}

// REQ-ARCH-002 — Decision: a failure means a corrupt file (bad sidecar) is trusted or rewritten like a legacy one.
#[test]
fn legacy_file_with_wrong_blake3_stops_h_below_it() {
    let archive = archive_with(TIP, &legacy_bytes(genesis_g(), K));
    let dir = archive.path();
    let wrong = crypto::hash::hash(b"other bytes").to_string();
    std::fs::write(sum_path(dir, K), wrong).unwrap();
    let before = snapshot(dir, K);
    let (store, _s) = store_with(&[K]);

    let r = bounded(dir, &store);

    assert_stops_below_k(dir, &r, &before);
}

// REQ-ARCH-002 — Decision: a failure means a stale old-chain file with an unreadable body survives a store-backed rewrite.
#[test]
fn legacy_file_with_other_genesis_is_rewritten_from_store() {
    let archive = archive_with(TIP, &legacy_bytes(genesis_other(), K));
    let dir = archive.path();
    let (store, _s) = store_with(&[K]);

    let r = bounded(dir, &store);

    assert!(
        std::fs::read(block_path(dir, K)).unwrap() == store_bytes(&store, K),
        "file {K} rewritten from the block store"
    );
    assert_sidecar_matches(dir, K);
    assert_eq!(r.files_written, 1, "one pair rewritten");
    assert_eq!(r.verified_height, TIP);
    assert_eq!(manifest_h(dir), Some(TIP), "manifest verified_height");
}

// REQ-ARCH-004 — Decision: a failure means H advances over an old-chain file that no block backs.
#[test]
fn legacy_file_with_other_genesis_and_no_store_block_stops_h() {
    let archive = archive_with(TIP, &legacy_bytes(genesis_other(), K));
    let dir = archive.path();
    let before = snapshot(dir, K);
    let (store, _s) = store_with(&[]);

    let r = bounded(dir, &store);

    assert_stops_below_k(dir, &r, &before);
}

// REQ-ARCH-002 — Decision: a failure means a file with no readable header is trusted or rewritten.
#[test]
fn undecodable_header_stops_h_below_it() {
    let junk = vec![0xABu8; 8];
    assert!(
        deserialize_block_compat(&junk).is_none(),
        "fixture deserializes"
    );
    assert!(
        bincode::deserialize::<BlockHeader>(&junk).is_err(),
        "fixture header decodes"
    );
    let archive = archive_with(TIP, &junk);
    let dir = archive.path();
    let before = snapshot(dir, K);
    let (store, _s) = store_with(&[K]);

    let r = bounded(dir, &store);

    assert_stops_below_k(dir, &r, &before);
}

// REQ-ARCH-004 — Decision: a failure means the next start still re-reads the files above the legacy height.
#[test]
fn next_start_trusts_h_over_legacy_file() {
    let archive = archive_with(TIP, &legacy_bytes(genesis_g(), K));
    let dir = archive.path();
    let before = snapshot(dir, K);
    let (store, _s) = store_with(&[]);
    bounded(dir, &store);

    let again = bounded(dir, &store);

    assert_eq!(
        again.files_read, 0,
        "next start: H == tip, no file may be read"
    );
    assert_eq!(again.verified_height, TIP);
    assert_eq!(manifest_h(dir), Some(TIP), "manifest verified_height");
    assert_eq!(
        snapshot(dir, K),
        before,
        "legacy file and sidecar never rewritten"
    );
}

// REQ-ARCH-004 — Decision: a failure means the live path stalls H at an intact legacy file.
// REQ-ARCH-002 — Decision: a failure means the live fill rewrites an intact legacy file from the block store.
#[test]
fn archive_finalized_crosses_intact_legacy_file() {
    let archive = archive_with(K, &legacy_bytes(genesis_g(), K));
    let dir = archive.path();
    write_manifest(dir, &manifest_json(K - 1, &genesis_g(), Some(K - 1)));
    let before = snapshot(dir, K);
    let (store, _s) = store_with(&[K]);
    let next = block_at(genesis_g(), K + 1);
    let block = ArchiveBlock {
        height: K + 1,
        hash: next.hash(),
        data: bincode::serialize(&next).unwrap(),
    };

    let r = archive_finalized(dir, &store, &block, &genesis_g()).unwrap();

    assert_eq!(
        r.verified_height,
        K + 1,
        "live path must cross the legacy file at {K}"
    );
    assert_eq!(manifest_h(dir), Some(K + 1), "manifest verified_height");
    assert_eq!(
        snapshot(dir, K),
        before,
        "legacy file and sidecar never rewritten"
    );
    assert_eq!(std::fs::read(block_path(dir, K + 1)).unwrap(), block.data);
    assert_sidecar_matches(dir, K + 1);
    assert_eq!(r.files_written, 1, "only the new block written");
}
