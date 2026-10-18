//! INC-I-249 M1 — archive catch-up: RED reproduction + genesis-guard
//! characterization. Uses ONLY the pre-fix `BlockArchiver::catch_up`
//! signature, so it compiles on the base commit and after the fix.
//!
//! covers: archiver/mod.rs archiver/manifest.rs archiver/catch_up.rs archiver/write.rs archiver/import.rs archiver/fence.rs
//!
//! OUTPUT CONTRACT: fn BlockArchiver::catch_up(dir, store, tip, Some(&G))
//!   O1 return: Ok(n) = archive files written (new or overwritten) / Err(io)
//!   O4a `{h:010}.block` bytes in `dir`
//!   O4b `{h:010}.blake3` hex checksum in `dir` (must equal blake3 of O4a)
//!   O4c `manifest.json` keys `latest_height`, `genesis_hash`
//!   (`verified_height` is asserted only in the RED test; the old code has none)
//!   The block store is read-only for catch-up (no output).
//! INPUT PARTITIONS:
//!   P1 trusted manifest (genesis = G, verified_height = H), foreign file at k <= H
//!   P2 trusted manifest, foreign file at k > H
//!   P3 manifest for another genesis, foreign file at k
//!   P4 no manifest, foreign file at k
//!   P5 legacy manifest (no verified_height), foreign file at k
//!   P6 undeserializable file at k (INC-I-119 legacy shape)
//!   P7 missing pair at k, block k in the block store
//! MATRIX (O1 / O4a / O4b / O4c):
//!   P1 ✓/✓/–/✓   P2 ✓/✓/✓/✓   P3 ✓/✓/✓/✓   P4 ✓/✓/✓/✓
//!   P5 ✓/✓/✓/✓   P6 ✓/✓/✓/–   P7 ✓/✓/✓/✓

use std::path::Path;

use crypto::{Hash, KeyPair};
use doli_core::{Block, BlockHeader};
use storage::archiver::BlockArchiver;
use storage::BlockStore;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

pub(crate) const CHAIN: u64 = 50;

pub(crate) fn genesis_g() -> Hash {
    crypto::hash::hash(b"inc-i-249-genesis-current")
}

pub(crate) fn genesis_other() -> Hash {
    crypto::hash::hash(b"inc-i-249-genesis-old-chain")
}

pub(crate) fn block_at(genesis: Hash, h: u64) -> Block {
    let kp = KeyPair::from_seed([7u8; 32]);
    let header = BlockHeader {
        version: 1,
        prev_hash: Hash::ZERO,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: genesis,
        timestamp: 1000 + h,
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

/// Block store holding heights 1..=n, all with genesis `genesis`.
pub(crate) fn store_with_chain(genesis: Hash, n: u64) -> (BlockStore, TempDir) {
    let tmp = TempDir::new().unwrap();
    let store = BlockStore::open(tmp.path()).unwrap();
    for h in 1..=n {
        store.put_block_canonical(&block_at(genesis, h), h).unwrap();
    }
    (store, tmp)
}

pub(crate) fn store_bytes(store: &BlockStore, h: u64) -> Vec<u8> {
    let b = store
        .get_block_by_height(h)
        .unwrap()
        .expect("block in store");
    bincode::serialize(&b).unwrap()
}

pub(crate) fn block_path(dir: &Path, h: u64) -> std::path::PathBuf {
    dir.join(format!("{:010}.block", h))
}

pub(crate) fn sum_path(dir: &Path, h: u64) -> std::path::PathBuf {
    dir.join(format!("{:010}.blake3", h))
}

pub(crate) fn write_pair(dir: &Path, h: u64, data: &[u8]) {
    std::fs::write(block_path(dir, h), data).unwrap();
    std::fs::write(sum_path(dir, h), crypto::hash::hash(data).to_string()).unwrap();
}

/// Archive pairs for `range`, copied byte-for-byte from the block store.
pub(crate) fn archive_from_store(
    dir: &Path,
    store: &BlockStore,
    range: std::ops::RangeInclusive<u64>,
) {
    for h in range {
        write_pair(dir, h, &store_bytes(store, h));
    }
}

pub(crate) fn write_manifest(dir: &Path, v: &serde_json::Value) {
    std::fs::write(dir.join("manifest.json"), v.to_string()).unwrap();
}

pub(crate) fn manifest_json(
    latest: u64,
    genesis: &Hash,
    verified: Option<u64>,
) -> serde_json::Value {
    let mut v = serde_json::json!({
        "latest_height": latest,
        "latest_hash": Hash::ZERO.to_string(),
        "genesis_hash": genesis.to_string(),
    });
    if let Some(h) = verified {
        v["verified_height"] = serde_json::json!(h);
    }
    v
}

pub(crate) fn read_manifest(dir: &Path) -> serde_json::Value {
    let s = std::fs::read_to_string(dir.join("manifest.json")).expect("manifest.json present");
    serde_json::from_str(&s).expect("manifest.json parses")
}

/// Plant a block from another chain at `h` (with a matching checksum).
pub(crate) fn plant_foreign(dir: &Path, h: u64) -> Vec<u8> {
    let data = bincode::serialize(&block_at(genesis_other(), h)).unwrap();
    write_pair(dir, h, &data);
    data
}

pub(crate) fn file_genesis(dir: &Path, h: u64) -> Hash {
    let data = std::fs::read(block_path(dir, h)).unwrap();
    doli_core::transaction::legacy::deserialize_block_compat(&data)
        .expect("archived file deserializes")
        .header
        .genesis_hash
}

/// O4a + O4b: the pair at `h` is the block-store block, checksum consistent.
fn assert_pair_is_store_block(dir: &Path, store: &BlockStore, h: u64) {
    let expected = store_bytes(store, h);
    let got = std::fs::read(block_path(dir, h)).unwrap();
    assert_eq!(got, expected, "file {h} must hold the block-store block");
    assert_eq!(file_genesis(dir, h), genesis_g(), "file {h} genesis");
    let sum = std::fs::read_to_string(sum_path(dir, h)).unwrap();
    assert_eq!(
        sum.trim(),
        crypto::hash::hash(&got).to_string(),
        "checksum {h}"
    );
}

/// O4c: the keys every reader depends on keep their meaning.
fn assert_manifest_keys(dir: &Path) {
    let m = read_manifest(dir);
    assert_eq!(m["latest_height"].as_u64(), Some(CHAIN), "latest_height");
    assert_eq!(
        m["genesis_hash"].as_str(),
        Some(genesis_g().to_string().as_str())
    );
}

fn fixture() -> (TempDir, BlockStore, TempDir) {
    let (store, store_tmp) = store_with_chain(genesis_g(), CHAIN);
    let archive = TempDir::new().unwrap();
    archive_from_store(archive.path(), &store, 1..=CHAIN);
    (archive, store, store_tmp)
}

// REQ-ARCH-001 — Decision: a failure means catch-up still reads every file below the verified height, so a restart costs O(chain) reads.
// REQ-ARCH-004 — Decision: a failure means the bounded start (H+1..=tip) is not in effect.
#[test]
fn repro_catch_up_does_not_read_files_below_verified_height() {
    let (archive, store, _s) = fixture();
    let dir = archive.path();
    let g = genesis_g();
    write_manifest(dir, &manifest_json(CHAIN, &g, Some(40)));
    // Height 10 is in the block store, so the old loop rewrites it (no `break`).
    let planted = plant_foreign(dir, 10);

    let written = BlockArchiver::catch_up(dir, &store, CHAIN, Some(&g)).unwrap();

    let after = std::fs::read(block_path(dir, 10)).unwrap();
    assert!(
        after == planted,
        "file 10 (< verified_height 40) was rewritten: catch-up read a file below H"
    );
    assert_eq!(
        written, 0,
        "every height in 41..=50 is valid: nothing to write"
    );
    assert_manifest_keys(dir);
    assert_eq!(read_manifest(dir)["verified_height"].as_u64(), Some(CHAIN));
}

// REQ-ARCH-002 — Decision: a failure means an old-chain file above H survives catch-up (INV-ARCHIVER-001 broken).
#[test]
fn characterize_old_chain_file_above_verified_height_is_rewritten() {
    let (archive, store, _s) = fixture();
    let dir = archive.path();
    let g = genesis_g();
    write_manifest(dir, &manifest_json(CHAIN, &g, Some(40)));
    plant_foreign(dir, 45);

    let written = BlockArchiver::catch_up(dir, &store, CHAIN, Some(&g)).unwrap();

    assert_pair_is_store_block(dir, &store, 45);
    assert_eq!(written, 1, "exactly the stale file at 45 is rewritten");
    assert_manifest_keys(dir);
}

// REQ-ARCH-002 — Decision: a failure means a manifest from another chain is trusted and its old-chain files are kept.
#[test]
fn characterize_manifest_with_other_genesis_is_not_trusted() {
    let (archive, store, _s) = fixture();
    let dir = archive.path();
    let g = genesis_g();
    write_manifest(dir, &manifest_json(CHAIN, &genesis_other(), Some(CHAIN)));
    plant_foreign(dir, 10);

    let written = BlockArchiver::catch_up(dir, &store, CHAIN, Some(&g)).unwrap();

    assert_pair_is_store_block(dir, &store, 10);
    assert_eq!(written, 1, "the stale file at 10 is rewritten");
    assert_manifest_keys(dir);
}

// REQ-ARCH-002 — Decision: a failure means an archive with no manifest keeps old-chain files.
#[test]
fn characterize_no_manifest_old_chain_file_is_rewritten() {
    let (archive, store, _s) = fixture();
    let dir = archive.path();
    plant_foreign(dir, 10);

    let written = BlockArchiver::catch_up(dir, &store, CHAIN, Some(&genesis_g())).unwrap();

    assert_pair_is_store_block(dir, &store, 10);
    assert_eq!(written, 1);
    assert_manifest_keys(dir);
}

// REQ-ARCH-002 — Decision: a failure means a legacy manifest (no verified_height) skips the per-file genesis check.
#[test]
fn characterize_legacy_manifest_old_chain_file_is_rewritten() {
    let (archive, store, _s) = fixture();
    let dir = archive.path();
    let g = genesis_g();
    write_manifest(dir, &manifest_json(CHAIN, &g, None));
    plant_foreign(dir, 10);

    let written = BlockArchiver::catch_up(dir, &store, CHAIN, Some(&g)).unwrap();

    assert_pair_is_store_block(dir, &store, 10);
    assert_eq!(written, 1);
    assert_manifest_keys(dir);
}

// REQ-ARCH-002 — Decision: a failure means catch-up rewrites an undeserializable (INC-I-119 legacy) file.
#[test]
fn characterize_undeserializable_file_is_not_rewritten() {
    let (archive, store, _s) = fixture();
    let dir = archive.path();
    let garbage = vec![0xFFu8; 64];
    write_pair(dir, 10, &garbage);
    let sum_before = std::fs::read_to_string(sum_path(dir, 10)).unwrap();

    let written = BlockArchiver::catch_up(dir, &store, CHAIN, Some(&genesis_g())).unwrap();

    assert_eq!(std::fs::read(block_path(dir, 10)).unwrap(), garbage);
    assert_eq!(
        std::fs::read_to_string(sum_path(dir, 10)).unwrap(),
        sum_before
    );
    assert_eq!(
        written, 0,
        "no file is written for an undeserializable height"
    );
}

// REQ-ARCH-002 — Decision: a failure means catch-up no longer fills a missing pair from the block store.
// REQ-ARCH-012 — Decision: a compile failure here means the module split changed a public storage::archiver path.
#[test]
fn characterize_missing_pair_is_filled_from_block_store() {
    let (archive, store, _s) = fixture();
    let dir = archive.path();
    std::fs::remove_file(block_path(dir, 20)).unwrap();
    std::fs::remove_file(sum_path(dir, 20)).unwrap();

    let written = BlockArchiver::catch_up(dir, &store, CHAIN, Some(&genesis_g())).unwrap();

    assert_pair_is_store_block(dir, &store, 20);
    assert_eq!(written, 1);
    assert_manifest_keys(dir);
}
