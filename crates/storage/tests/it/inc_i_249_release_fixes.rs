// covers: manifest.rs catch_up.rs fence.rs
//! INC-I-249 M5 release fixes: manifest size cap, metadata-only spot-check,
//! fence fallback when the rename fails.
//!
//! OUTPUT CONTRACT:
//!   catch_up_bounded -> report {files_read, files_written, verified_height};
//!     `.block` atime / bytes; manifest.json keys and size
//!   fence_stale_archive -> Ok(Some|None) | Err; dir entries; parent entries
//!   manifest_height / manifest_genesis_hash -> Option
//! INPUT PARTITIONS: manifest cap+1 | == cap | sparse 64 MiB;
//!   pair at H valid | empty .block | empty .blake3;
//!   parent unwritable + removable | parent unwritable + locked subdir.

use std::fs::{File, FileTimes, Permissions};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crypto::Hash;
use storage::archiver::{
    fence_stale_archive, manifest_genesis_hash, manifest_height, BlockArchiver, CatchUpReport,
};
use storage::BlockStore;
use tempfile::TempDir;

use crate::inc_i_249_archive_catchup::{
    archive_from_store, block_at, block_path, file_genesis, genesis_g, genesis_other,
    manifest_json, plant_foreign, read_manifest, store_with_chain, sum_path, write_manifest,
    write_pair, CHAIN,
};

const CAP: usize = 65_536;

fn small() -> (TempDir, BlockStore, TempDir) {
    let (store, store_tmp) = store_with_chain(genesis_g(), CHAIN);
    let archive = TempDir::new().unwrap();
    archive_from_store(archive.path(), &store, 1..=CHAIN);
    (archive, store, store_tmp)
}

fn bounded(dir: &Path, store: &BlockStore, tip: u64) -> CatchUpReport {
    BlockArchiver::catch_up_bounded(dir, store, tip, &genesis_g()).unwrap()
}

/// Valid JSON manifest of exactly `len` bytes (padded with a `pad` key).
fn manifest_of_len(genesis: &Hash, verified: u64, len: usize) -> String {
    let mut v = manifest_json(CHAIN, genesis, Some(verified));
    v["pad"] = serde_json::json!("");
    let base = v.to_string().len();
    v["pad"] = serde_json::json!("x".repeat(len - base));
    let s = v.to_string();
    assert_eq!(s.len(), len, "fixture size");
    s
}

fn put_manifest(dir: &Path, text: &str) {
    std::fs::write(dir.join("manifest.json"), text).unwrap();
}

fn manifest_len(dir: &Path) -> u64 {
    std::fs::metadata(dir.join("manifest.json")).unwrap().len()
}

fn entries(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn g_str() -> String {
    genesis_g().to_string()
}

// ---------------------------------------------------------------------------
// REQ-ARCH-SEC-001: 64 KiB manifest cap
// ---------------------------------------------------------------------------

// REQ-ARCH-SEC-001 — Decision: a failure means a manifest above the 64 KiB cap is parsed and its H is trusted.
#[test]
fn sec_oversized_manifest_forces_full_check() {
    let (archive, store, _s) = small();
    let dir = archive.path();
    put_manifest(dir, &manifest_of_len(&genesis_g(), CHAIN, CAP + 1));
    plant_foreign(dir, 10);

    let r = bounded(dir, &store, CHAIN);

    assert_eq!(
        r.files_read,
        CHAIN,
        "a {}-byte manifest was trusted: no full check",
        CAP + 1
    );
    assert_eq!(file_genesis(dir, 10), genesis_g(), "file 10 not verified");
    assert_eq!(r.verified_height, CHAIN);
    let m = read_manifest(dir);
    assert_eq!(m["verified_height"].as_u64(), Some(CHAIN));
    assert_eq!(m["genesis_hash"].as_str(), Some(g_str().as_str()));
    assert!(
        manifest_len(dir) <= CAP as u64,
        "rewritten manifest above the cap"
    );
}

// REQ-ARCH-SEC-001 REQ-ARCH-005 — Decision: a failure means an oversized manifest still drives the genesis fence and moves the archive.
#[test]
fn sec_oversized_foreign_manifest_is_not_fenced() {
    let parent = TempDir::new().unwrap();
    let dir = parent.path().join("archive");
    std::fs::create_dir_all(&dir).unwrap();
    for h in 1..=3 {
        plant_foreign(&dir, h);
    }
    put_manifest(&dir, &manifest_of_len(&genesis_other(), 3, CAP + 1));
    let before = entries(&dir);

    let out = fence_stale_archive(&dir, &genesis_g()).unwrap();

    assert_eq!(out, None, "an oversized manifest was read by the fence");
    assert_eq!(entries(&dir), before, "archive entries changed");
    assert_eq!(entries(parent.path()), vec!["archive".to_string()]);
}

// REQ-ARCH-SEC-001 — Decision: a failure means the RPC/import readers still parse a manifest above the cap.
#[test]
fn sec_oversized_manifest_public_readers_return_none() {
    let archive = TempDir::new().unwrap();
    let dir = archive.path();
    put_manifest(dir, &manifest_of_len(&genesis_g(), CHAIN, CAP + 1));

    assert_eq!(manifest_height(dir), None, "manifest_height above the cap");
    assert_eq!(
        manifest_genesis_hash(dir),
        None,
        "manifest_genesis_hash above the cap"
    );
}

// REQ-ARCH-SEC-001 — Decision: a failure means the cap is off by one and rejects a manifest of exactly 64 KiB (edge).
#[test]
fn sec_manifest_of_exactly_cap_is_accepted() {
    let (archive, store, _s) = small();
    let dir = archive.path();
    put_manifest(dir, &manifest_of_len(&genesis_g(), CHAIN, CAP));
    plant_foreign(dir, 10);

    assert_eq!(manifest_height(dir), Some(CHAIN));
    assert_eq!(manifest_genesis_hash(dir), Some(g_str()));
    let r = bounded(dir, &store, CHAIN);
    assert!(
        r.files_read < CHAIN,
        "a manifest of exactly the cap was not trusted"
    );
    assert_eq!(
        file_genesis(dir, 10),
        genesis_other(),
        "file below H re-read"
    );
    assert_eq!(r.verified_height, CHAIN);

    let parent = TempDir::new().unwrap();
    let foreign = parent.path().join("archive");
    std::fs::create_dir_all(&foreign).unwrap();
    put_manifest(&foreign, &manifest_of_len(&genesis_other(), CHAIN, CAP));
    let out = fence_stale_archive(&foreign, &genesis_g()).unwrap();
    assert!(
        out.is_some(),
        "a foreign manifest of exactly the cap was not fenced"
    );
}

// REQ-ARCH-SEC-001 — Decision: a failure means a huge sparse manifest panics or is trusted (edge).
#[test]
fn sec_sparse_64mib_manifest_is_none_without_panic() {
    let (archive, store, _s) = small();
    let dir = archive.path();
    File::create(dir.join("manifest.json"))
        .unwrap()
        .set_len(64 << 20)
        .unwrap();

    assert_eq!(manifest_height(dir), None);
    assert_eq!(manifest_genesis_hash(dir), None);
    assert_eq!(fence_stale_archive(dir, &genesis_g()).unwrap(), None);
    let r = bounded(dir, &store, CHAIN);
    assert_eq!(r.verified_height, CHAIN);
    assert!(
        manifest_len(dir) <= CAP as u64,
        "rewritten manifest above the cap"
    );
}

// ---------------------------------------------------------------------------
// F-3 / REQ-ARCH-004: metadata-only spot-check, at most tip − H reads
// ---------------------------------------------------------------------------

fn set_atime(path: &Path, t: SystemTime) {
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(FileTimes::new().set_accessed(t))
        .unwrap();
}

// REQ-ARCH-004 — Decision: a failure means the catch-up still opens the file at H, so a restart reads tip − H + 1 files.
#[test]
fn bounded_catch_up_never_reads_block_at_h_atime() {
    const TIP: u64 = 2_000;
    const H: u64 = 1_000;
    let g = genesis_g();
    let data = bincode::serialize(&block_at(g, 1)).unwrap();
    let archive = TempDir::new().unwrap();
    let dir = archive.path();
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(946_684_800);
    for h in 1..=TIP {
        write_pair(dir, h, &data);
        set_atime(&block_path(dir, h), old);
    }
    write_manifest(dir, &manifest_json(TIP, &g, Some(H)));
    let store_tmp = TempDir::new().unwrap();
    let store = BlockStore::open(store_tmp.path()).unwrap();

    let r = bounded(dir, &store, TIP);

    let threshold = old + Duration::from_secs(86_400);
    let moved: Vec<u64> = (1..=TIP)
        .filter(|h| {
            let meta = std::fs::metadata(block_path(dir, *h)).unwrap();
            meta.accessed().unwrap() > threshold
        })
        .collect();
    assert!(
        moved.contains(&TIP),
        "atime of file {TIP} did not move: this filesystem does not record reads"
    );
    assert!(
        !moved.contains(&H),
        "the .block at H={H} was read: the spot-check must be metadata-only"
    );
    assert!(
        moved.len() as u64 <= TIP - H,
        "{} block files read > tip - H = {}",
        moved.len(),
        TIP - H
    );
    assert_eq!(r.verified_height, TIP);
}

fn assert_full_check_after(prep: impl FnOnce(&Path), want_h: u64) {
    let (archive, store, _s) = small();
    let dir = archive.path();
    write_manifest(dir, &manifest_json(CHAIN, &genesis_g(), Some(40)));
    plant_foreign(dir, 10);
    prep(dir);

    let r = bounded(dir, &store, CHAIN);

    assert_eq!(
        file_genesis(dir, 10),
        genesis_g(),
        "manifest trusted: file 10 not verified"
    );
    assert_eq!(
        r.files_read, CHAIN,
        "a full check reads each file once, with no spot-check read"
    );
    assert_eq!(r.verified_height, want_h);
    assert_eq!(read_manifest(dir)["verified_height"].as_u64(), Some(want_h));
}

// REQ-ARCH-004 REQ-ARCH-SEC-001 — Decision: a failure means a truncated .block at H passes the metadata-only spot-check.
#[test]
fn spot_check_empty_block_at_h_forces_full_check() {
    assert_full_check_after(|dir| std::fs::write(block_path(dir, 40), b"").unwrap(), 39);
}

// REQ-ARCH-004 REQ-ARCH-SEC-001 — Decision: a failure means an empty .blake3 at H passes the metadata-only spot-check.
#[test]
fn spot_check_empty_blake3_at_h_forces_full_check() {
    assert_full_check_after(|dir| std::fs::write(sum_path(dir, 40), b"").unwrap(), CHAIN);
}

// ---------------------------------------------------------------------------
// AUDIT-P2-525: fence fallback when the rename fails
// ---------------------------------------------------------------------------

/// Restores 0755 on drop, so TempDir cleanup works after a failed assert.
struct Unlock(Vec<PathBuf>);

impl Drop for Unlock {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = std::fs::set_permissions(p, Permissions::from_mode(0o755));
        }
    }
}

fn lock(paths: &[&Path]) -> Unlock {
    for p in paths {
        std::fs::set_permissions(p, Permissions::from_mode(0o555)).unwrap();
    }
    Unlock(paths.iter().map(|p| p.to_path_buf()).collect())
}

/// True when a 0555 dir still accepts a new file (running as root).
fn perms_do_not_bite(dir: &Path) -> bool {
    let probe = dir.join(".perm-probe");
    let created = File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    created
}

fn foreign_archive(parent: &Path) -> PathBuf {
    let dir = parent.join("archive");
    std::fs::create_dir_all(dir.join("sub/deeper")).unwrap();
    std::fs::write(dir.join("sub/f"), b"x").unwrap();
    std::fs::write(dir.join("sub/deeper/f"), b"x").unwrap();
    for h in 1..=3 {
        plant_foreign(&dir, h);
    }
    write_manifest(&dir, &manifest_json(3, &genesis_other(), Some(3)));
    dir
}

// REQ-ARCH-005 REQ-ARCH-002 — Decision: a failure means an unrenamable foreign archive is kept and later stamped with our genesis (AUDIT-P2-525).
#[test]
fn fence_rename_fails_empties_dir_then_catch_up_writes_our_genesis() {
    let parent = TempDir::new().unwrap();
    let dir = foreign_archive(parent.path());
    let _unlock = lock(&[parent.path()]);
    if perms_do_not_bite(parent.path()) {
        println!("SKIP: a 0555 parent is writable here (root); the rename cannot be made to fail");
        return;
    }

    let out = fence_stale_archive(&dir, &genesis_g());

    assert!(
        matches!(out, Ok(None)),
        "fence with an unwritable parent -> {out:?}, want Ok(None) after emptying the dir"
    );
    assert!(dir.is_dir(), "the archive dir itself was removed");
    assert_eq!(
        entries(&dir),
        Vec::<String>::new(),
        "foreign entries left inside the dir"
    );
    assert_eq!(entries(parent.path()), vec!["archive".to_string()]);

    let (store, _s) = store_with_chain(genesis_g(), 5);
    let r = BlockArchiver::catch_up_bounded(&dir, &store, 5, &genesis_g()).unwrap();
    assert_eq!(r.verified_height, 5);
    assert_eq!(
        read_manifest(&dir)["genesis_hash"].as_str(),
        Some(g_str().as_str())
    );
    for h in 1..=5 {
        assert_eq!(file_genesis(&dir, h), genesis_g(), "file {h}");
    }
}

// REQ-ARCH-005 REQ-ARCH-002 — Decision: a failure means the fence reports success while foreign entries it could not remove remain (edge).
#[test]
fn fence_rename_and_removal_fail_returns_err() {
    let parent = TempDir::new().unwrap();
    let dir = foreign_archive(parent.path());
    let locked = dir.join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::write(locked.join("f"), b"x").unwrap();
    let _unlock = lock(&[&locked, parent.path()]);
    if perms_do_not_bite(&locked) {
        println!("SKIP: a 0555 dir is writable here (root); the removal cannot be made to fail");
        return;
    }

    let out = fence_stale_archive(&dir, &genesis_g());

    assert!(
        out.is_err(),
        "fence that can neither rename nor empty the dir -> {out:?}, want Err"
    );
    assert_eq!(entries(parent.path()), vec!["archive".to_string()]);
}
