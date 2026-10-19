//! INC-I-249 M1 — bounded archive catch-up, live gap heal and the O(1)
//! stale-archive fence, written against the post-fix API.
//!
//! covers: archiver/mod.rs archiver/manifest.rs archiver/catch_up.rs archiver/write.rs archiver/import.rs archiver/fence.rs
//!
//! OUTPUT CONTRACT:
//!   fn BlockArchiver::catch_up_bounded(dir, store, tip, &G) -> io::Result<CatchUpReport>
//!     O1 report.files_read / files_written / verified_height
//!     O4a `{h:010}.block` + `{h:010}.blake3` pairs in `dir`
//!     O4b `manifest.json` keys `verified_height`, `genesis_hash`, `latest_height`
//!     O4c tmp files: no `*.tmp-archiver` left; repair's `*.tmp` never touched
//!   fn archive_finalized(dir, store, &ArchiveBlock, &G) -> io::Result<CatchUpReport>
//!     O1 report.files_written / verified_height; O4a pairs; O4b manifest keys
//!   fn fence_stale_archive(dir, &G) -> io::Result<Option<PathBuf>>
//!     O1 Some(stale path) / None; O4 `dir` content, stale sibling directory
//! INPUT PARTITIONS:
//!   catch_up_bounded: trusted H < tip (100k) | H == tip | H > tip | no manifest
//!     | legacy manifest | partial-run manifest | gap in archive + store
//!     | undeserializable file | malformed manifest (SEC-001, 11 shapes)
//!     | repair tmp sentinels present | interrupted full check > 1000 heights
//!   archive_finalized: h == H+1 | h > H+1 (fill) | h <= H (duplicate)
//!     | fill blocked by a store gap
//!   fence_stale_archive: other genesis | same genesis | no manifest
//!     | unparsable manifest | manifest without genesis_hash
//! MATRIX: every partition asserts O1 and the O4 cells it can change; the
//!   fence's "no fence" partitions assert O1 = None plus O4 unchanged.

use std::path::{Path, PathBuf};

use crypto::Hash;
use storage::archiver::{
    archive_finalized, fence_stale_archive, ArchiveBlock, BlockArchiver, CatchUpReport,
};
use storage::BlockStore;
use tempfile::TempDir;

use crate::inc_i_249_archive_catchup::{
    archive_from_store, block_at, block_path, file_genesis, genesis_g, genesis_other,
    manifest_json, plant_foreign, read_manifest, store_bytes, store_with_chain, sum_path,
    write_manifest, write_pair, CHAIN,
};

fn small() -> (TempDir, BlockStore, TempDir) {
    let (store, store_tmp) = store_with_chain(genesis_g(), CHAIN);
    let archive = TempDir::new().unwrap();
    archive_from_store(archive.path(), &store, 1..=CHAIN);
    (archive, store, store_tmp)
}

/// Block store with every height in 1..=n except `skip`.
fn store_without(n: u64, skip: u64) -> (BlockStore, TempDir) {
    let tmp = TempDir::new().unwrap();
    let store = BlockStore::open(tmp.path()).unwrap();
    for h in (1..=n).filter(|h| *h != skip) {
        store
            .put_block_canonical(&block_at(genesis_g(), h), h)
            .unwrap();
    }
    (store, tmp)
}

fn bounded(dir: &Path, store: &BlockStore, tip: u64) -> CatchUpReport {
    BlockArchiver::catch_up_bounded(dir, store, tip, &genesis_g()).unwrap()
}

fn manifest_h(dir: &Path) -> Option<u64> {
    read_manifest(dir)["verified_height"].as_u64()
}

fn remove_pair(dir: &Path, h: u64) {
    std::fs::remove_file(block_path(dir, h)).unwrap();
    std::fs::remove_file(sum_path(dir, h)).unwrap();
}

fn assert_pair_is_store_block(dir: &Path, store: &BlockStore, h: u64) {
    let got = std::fs::read(block_path(dir, h)).unwrap();
    assert_eq!(
        got,
        store_bytes(store, h),
        "file {h} must hold the block-store block"
    );
    let sum = std::fs::read_to_string(sum_path(dir, h)).unwrap();
    assert_eq!(
        sum.trim(),
        crypto::hash::hash(&got).to_string(),
        "checksum {h}"
    );
}

fn archive_block(store: &BlockStore, h: u64) -> ArchiveBlock {
    let b = store
        .get_block_by_height(h)
        .unwrap()
        .expect("block in store");
    ArchiveBlock {
        height: h,
        hash: b.hash(),
        data: bincode::serialize(&b).unwrap(),
    }
}

fn leftover_tmp_archiver(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp-archiver"))
        .collect()
}

// ---------------------------------------------------------------------------
// Bounded catch-up
// ---------------------------------------------------------------------------

// REQ-ARCH-004 — Decision: a failure means a restart still reads O(chain) files, the cause of the slow RPC start.
#[test]
fn bounded_catch_up_100k_reads_at_most_tip_minus_h() {
    const TIP: u64 = 100_000;
    const H: u64 = 99_000;
    let g = genesis_g();
    let data = bincode::serialize(&block_at(g, 1)).unwrap();
    let archive = TempDir::new().unwrap();
    let dir = archive.path();
    for h in 1..=TIP {
        write_pair(dir, h, &data);
    }
    let mut m = manifest_json(TIP, &g, Some(H));
    m["latest_hash"] = serde_json::json!(block_at(g, 1).hash().to_string());
    write_manifest(dir, &m);
    let store_tmp = TempDir::new().unwrap();
    let store = BlockStore::open(store_tmp.path()).unwrap();

    let r = bounded(dir, &store, TIP);
    println!("FILES_READ={}", r.files_read);

    assert!(
        r.files_read <= TIP - H,
        "files_read={} > tip-H",
        r.files_read
    );
    assert_eq!(r.files_written, 0);
    assert_eq!(r.verified_height, TIP);
    assert_eq!(manifest_h(dir), Some(TIP));
    assert_eq!(read_manifest(dir)["latest_height"].as_u64(), Some(TIP));

    let again = bounded(dir, &store, TIP);
    assert_eq!(again.files_read, 0, "H == tip: no file may be read");
    assert_eq!(again.verified_height, TIP);
}

// REQ-ARCH-004 — Decision: a failure means H can jump over a height that no file or block backs.
#[test]
fn verified_height_stops_at_a_gap_in_archive_and_store() {
    let (store, _s) = store_without(CHAIN, 30);
    let archive = TempDir::new().unwrap();
    let dir = archive.path();
    archive_from_store(dir, &store, 1..=29);
    archive_from_store(dir, &store, 31..=CHAIN);
    write_manifest(dir, &manifest_json(CHAIN, &genesis_g(), Some(20)));

    let r = bounded(dir, &store, CHAIN);

    assert_eq!(r.verified_height, 29);
    assert_eq!(manifest_h(dir), Some(29));
    assert_eq!(r.files_written, 0);
    assert!(!block_path(dir, 30).exists());
}

// REQ-ARCH-004 — Decision: a failure means a missing pair above H is not filled from the block store.
#[test]
fn missing_pair_above_h_is_written_and_h_reaches_tip() {
    let (archive, store, _s) = small();
    let dir = archive.path();
    write_manifest(dir, &manifest_json(CHAIN, &genesis_g(), Some(40)));
    remove_pair(dir, 45);

    let r = bounded(dir, &store, CHAIN);

    assert_pair_is_store_block(dir, &store, 45);
    assert_eq!(r.files_written, 1);
    assert_eq!(r.verified_height, CHAIN);
    assert_eq!(
        r.files_read,
        CHAIN - 41,
        "H=40, 45 missing: only 41..=CHAIN minus 45 may be read"
    );
    assert_eq!(manifest_h(dir), Some(CHAIN));
}

// REQ-ARCH-004 — Decision: a failure means a node that rolled back below H loses or underflows H.
#[test]
fn h_above_tip_does_nothing_and_keeps_h() {
    let (archive, store, _s) = small();
    let dir = archive.path();
    write_manifest(dir, &manifest_json(CHAIN, &genesis_g(), Some(CHAIN)));

    let r = bounded(dir, &store, 30);

    assert_eq!(r.files_written, 0);
    assert_eq!(r.files_read, 0, "H above tip: no file may be read");
    assert_eq!(r.verified_height, CHAIN);
    assert_eq!(manifest_h(dir), Some(CHAIN));
}

// ---------------------------------------------------------------------------
// Full check: no manifest, legacy manifest, resume, resumable save
// ---------------------------------------------------------------------------

// REQ-ARCH-005 — Decision: a failure means an archive without the key is never verified once, or every start stays a full scan.
#[test]
fn no_manifest_full_check_reads_every_file_once_then_next_call_is_bounded() {
    let (archive, store, _s) = small();
    let dir = archive.path();

    let first = bounded(dir, &store, CHAIN);
    assert_eq!(first.files_read, CHAIN, "full check reads each file once");
    assert_eq!(first.files_written, 0);
    assert_eq!(first.verified_height, CHAIN);
    let m = read_manifest(dir);
    assert_eq!(m["verified_height"].as_u64(), Some(CHAIN));
    assert_eq!(m["latest_height"].as_u64(), Some(CHAIN));
    assert_eq!(
        m["genesis_hash"].as_str(),
        Some(genesis_g().to_string().as_str())
    );

    let second = bounded(dir, &store, CHAIN);
    assert_eq!(
        second.files_read, 0,
        "second call: H == tip, no file may be read"
    );
    assert_eq!(second.verified_height, CHAIN);
}

// REQ-ARCH-005 — Decision: a failure means a manifest from an older binary is trusted without the key.
#[test]
fn legacy_manifest_without_key_gets_one_full_check() {
    let (archive, store, _s) = small();
    let dir = archive.path();
    write_manifest(dir, &manifest_json(CHAIN, &genesis_g(), None));

    let r = bounded(dir, &store, CHAIN);

    assert_eq!(r.files_read, CHAIN);
    assert_eq!(r.verified_height, CHAIN);
    assert_eq!(manifest_h(dir), Some(CHAIN));
    assert_eq!(read_manifest(dir)["latest_height"].as_u64(), Some(CHAIN));
}

// REQ-ARCH-005 — Decision: a failure means a restart after a partial run starts the full check again.
#[test]
fn partial_run_manifest_resumes_from_saved_h() {
    let (store, _s) = store_without(CHAIN, 30);
    let archive = TempDir::new().unwrap();
    let dir = archive.path();
    archive_from_store(dir, &store, 1..=29);
    archive_from_store(dir, &store, 31..=CHAIN);
    let first = bounded(dir, &store, CHAIN);
    assert_eq!(first.verified_height, 29);

    store
        .put_block_canonical(&block_at(genesis_g(), 30), 30)
        .unwrap();
    let second = bounded(dir, &store, CHAIN);

    assert_eq!(
        second.files_read,
        CHAIN - 30,
        "H=29, 30 missing: only 31..=CHAIN may be read"
    );
    assert_eq!(second.files_written, 1);
    assert_pair_is_store_block(dir, &store, 30);
    assert_eq!(second.verified_height, CHAIN);
    assert_eq!(manifest_h(dir), Some(CHAIN));
}

// REQ-ARCH-005 — Decision: a failure means a gap stops the scan and old-chain files above it survive.
#[test]
fn full_check_continues_past_a_gap_and_h_stops_before_it() {
    const N: u64 = 3_000;
    let (store, _s) = store_without(N, 2_500);
    let archive = TempDir::new().unwrap();
    let dir = archive.path();
    archive_from_store(dir, &store, 1..=2_499);
    archive_from_store(dir, &store, 2_501..=N);
    plant_foreign(dir, 2_700);

    let r = bounded(dir, &store, N);

    assert_eq!(r.verified_height, 2_499);
    assert_eq!(manifest_h(dir), Some(2_499));
    assert_eq!(r.files_read, N - 1, "every present file is read once");
    assert_eq!(
        file_genesis(dir, 2_700),
        genesis_g(),
        "old-chain file above the gap"
    );
    assert_eq!(r.files_written, 1);
}

// REQ-ARCH-005 — Decision: a failure means an interrupted full check saves no H and the restart rescans from 1.
#[test]
fn interrupted_full_check_has_saved_h_at_least_every_1000_heights() {
    const N: u64 = 3_000;
    let (store, _s) = store_with_chain(genesis_g(), N);
    let archive = TempDir::new().unwrap();
    let dir = archive.path();
    archive_from_store(dir, &store, 1..=N);
    remove_pair(dir, 2_500);
    // A directory at the writer's tmp name makes the write of 2500 fail.
    std::fs::create_dir(dir.join(format!("{:010}.block.tmp-archiver", 2_500))).unwrap();

    let result = BlockArchiver::catch_up_bounded(dir, &store, N, &genesis_g());

    let saved = manifest_h(dir).expect("H saved before the failing height");
    assert!((2_000..=2_499).contains(&saved), "saved H = {saved}");
    if let Ok(r) = result {
        assert_eq!(r.verified_height, saved);
    }
}

// ---------------------------------------------------------------------------
// REQ-ARCH-SEC-001: malformed manifests never skip verification, never panic
// ---------------------------------------------------------------------------

fn assert_untrusted(manifest: &str, prep: impl FnOnce(&Path)) {
    let (archive, store, _s) = small();
    let dir = archive.path();
    std::fs::write(dir.join("manifest.json"), manifest).unwrap();
    plant_foreign(dir, 10);
    prep(dir);

    let r = bounded(dir, &store, CHAIN);

    assert_eq!(
        file_genesis(dir, 10),
        genesis_g(),
        "manifest {manifest:?} was trusted: file 10 not verified"
    );
    assert_pair_is_store_block(dir, &store, 10);
    assert_eq!(r.verified_height, CHAIN, "H recomputed by the full check");
    let m = read_manifest(dir);
    assert_eq!(m["verified_height"].as_u64(), Some(CHAIN));
    assert_eq!(
        m["genesis_hash"].as_str(),
        Some(genesis_g().to_string().as_str())
    );
}

fn manifest_text(
    latest: serde_json::Value,
    genesis: serde_json::Value,
    h: serde_json::Value,
) -> String {
    serde_json::json!({
        "latest_height": latest,
        "latest_hash": Hash::ZERO.to_string(),
        "genesis_hash": genesis,
        "verified_height": h,
    })
    .to_string()
}

fn g_json() -> serde_json::Value {
    serde_json::json!(genesis_g().to_string())
}

// REQ-ARCH-SEC-001 REQ-ARCH-005 — Decision: a failure means unparsable JSON either panics or is read as "verified".
#[test]
fn sec_bad_json_is_not_trusted() {
    assert_untrusted("{not json", |_| {});
    assert_untrusted("", |_| {});
}

// REQ-ARCH-SEC-001 REQ-ARCH-005 — Decision: a failure means a wrongly typed key is coerced into a trusted H.
#[test]
fn sec_verified_height_wrong_type_is_not_trusted() {
    let l = serde_json::json!(CHAIN);
    assert_untrusted(
        &manifest_text(l.clone(), g_json(), serde_json::json!("40")),
        |_| {},
    );
    assert_untrusted(
        &manifest_text(l.clone(), g_json(), serde_json::json!(-1)),
        |_| {},
    );
    assert_untrusted(&manifest_text(l, g_json(), serde_json::json!(40.5)), |_| {});
}

// REQ-ARCH-SEC-001 REQ-ARCH-004 — Decision: a failure means a key above latest_height (or u64::MAX) is trusted or overflows.
#[test]
fn sec_verified_height_above_latest_is_not_trusted() {
    assert_untrusted(
        &manifest_text(serde_json::json!(30), g_json(), serde_json::json!(40)),
        |_| {},
    );
    assert_untrusted(
        &manifest_text(
            serde_json::json!(CHAIN),
            g_json(),
            serde_json::json!(u64::MAX),
        ),
        |_| {},
    );
}

// REQ-ARCH-SEC-001 REQ-ARCH-005 — Decision: a failure means H is trusted without a genesis binding.
#[test]
fn sec_genesis_hash_missing_or_not_a_string_is_not_trusted() {
    let no_genesis = serde_json::json!({
        "latest_height": CHAIN,
        "latest_hash": Hash::ZERO.to_string(),
        "verified_height": 40,
    })
    .to_string();
    assert_untrusted(&no_genesis, |_| {});
    let l = serde_json::json!(CHAIN);
    let h = serde_json::json!(40);
    assert_untrusted(
        &manifest_text(l.clone(), serde_json::json!(123), h.clone()),
        |_| {},
    );
    assert_untrusted(&manifest_text(l, serde_json::json!("zz"), h), |_| {});
}

// REQ-ARCH-SEC-001 REQ-ARCH-005 — Decision: a failure means a manifest copied from another archive is trusted.
#[test]
fn sec_spot_check_file_missing_is_not_trusted() {
    let m = manifest_json(CHAIN, &genesis_g(), Some(40)).to_string();
    assert_untrusted(&m, |dir| remove_pair(dir, 40));
}

// REQ-ARCH-004 REQ-ARCH-SEC-001 — Decision: a failure means the spot-check still opens file H, so a restart reads tip − H + 1 files.
#[test]
fn sec_spot_check_is_metadata_only_foreign_pair_at_h_keeps_h() {
    let (archive, store, _s) = small();
    let dir = archive.path();
    write_manifest(dir, &manifest_json(CHAIN, &genesis_g(), Some(40)));
    let at_h = plant_foreign(dir, 40);
    plant_foreign(dir, 10);

    let r = bounded(dir, &store, CHAIN);

    assert_eq!(r.files_read, CHAIN - 40, "only 41..=CHAIN may be read");
    assert_eq!(
        std::fs::read(block_path(dir, 40)).unwrap(),
        at_h,
        "file at H re-verified"
    );
    assert_eq!(
        file_genesis(dir, 10),
        genesis_other(),
        "file below H re-read"
    );
    assert_eq!(r.verified_height, CHAIN);
    assert_eq!(manifest_h(dir), Some(CHAIN));
}

// ---------------------------------------------------------------------------
// Durability and tmp names
// ---------------------------------------------------------------------------

// REQ-ARCH-003 — Decision: a failure means the catch-up writer shares a tmp name with the repair RPC or leaves tmp files.
#[test]
fn writer_never_touches_repair_tmp_names_and_leaves_no_tmp() {
    let (archive, store, _s) = small();
    let dir = archive.path();
    write_manifest(dir, &manifest_json(CHAIN, &genesis_g(), Some(40)));
    remove_pair(dir, 45);
    let block_tmp = dir.join(format!("{:010}.block.tmp", 45));
    let sum_tmp = dir.join(format!("{:010}.blake3.tmp", 45));
    std::fs::write(&block_tmp, b"repair-sentinel-block").unwrap();
    std::fs::write(&sum_tmp, b"repair-sentinel-sum").unwrap();

    let r = bounded(dir, &store, CHAIN);

    assert_eq!(std::fs::read(&block_tmp).unwrap(), b"repair-sentinel-block");
    assert_eq!(std::fs::read(&sum_tmp).unwrap(), b"repair-sentinel-sum");
    assert_pair_is_store_block(dir, &store, 45);
    assert_eq!(r.files_written, 1);
    assert_eq!(leftover_tmp_archiver(dir), Vec::<String>::new());
    assert!(!dir.join("manifest.json.tmp").exists());
}

// ---------------------------------------------------------------------------
// Live path: archive_finalized
// ---------------------------------------------------------------------------

// REQ-ARCH-004 — Decision: a failure means a dropped live block leaves H stalled and the next start reads O(uptime) files.
#[test]
fn archive_finalized_heals_gap_then_advances_by_one() {
    let (store, _s) = store_with_chain(genesis_g(), CHAIN);
    let archive = TempDir::new().unwrap();
    let dir = archive.path();
    archive_from_store(dir, &store, 1..=40);
    write_manifest(dir, &manifest_json(40, &genesis_g(), Some(40)));

    let r = archive_finalized(dir, &store, &archive_block(&store, 45), &genesis_g()).unwrap();
    for h in 41..=45 {
        assert_pair_is_store_block(dir, &store, h);
    }
    assert_eq!(r.files_written, 5);
    assert_eq!(r.verified_height, 45);
    let m = read_manifest(dir);
    assert_eq!(m["verified_height"].as_u64(), Some(45));
    assert_eq!(m["latest_height"].as_u64(), Some(45));
    assert_eq!(
        m["genesis_hash"].as_str(),
        Some(genesis_g().to_string().as_str())
    );

    let r = archive_finalized(dir, &store, &archive_block(&store, 46), &genesis_g()).unwrap();
    assert_pair_is_store_block(dir, &store, 46);
    assert_eq!(r.files_written, 1);
    assert_eq!(r.verified_height, 46);
    assert_eq!(manifest_h(dir), Some(46));
    assert_eq!(leftover_tmp_archiver(dir), Vec::<String>::new());
}

// REQ-ARCH-004 — Decision: a failure means a duplicate live block rewrites a file or drops the key.
#[test]
fn archive_finalized_duplicate_keeps_h_and_key() {
    let (archive, store, _s) = small();
    let dir = archive.path();
    write_manifest(dir, &manifest_json(CHAIN, &genesis_g(), Some(CHAIN)));

    let r = archive_finalized(dir, &store, &archive_block(&store, 45), &genesis_g()).unwrap();

    assert_eq!(r.files_written, 0);
    assert_eq!(r.verified_height, CHAIN);
    assert_eq!(manifest_h(dir), Some(CHAIN));
    assert_eq!(read_manifest(dir)["latest_height"].as_u64(), Some(CHAIN));
}

// REQ-ARCH-004 — Decision: a failure means the live path advances H over a height it could not fill.
#[test]
fn archive_finalized_store_gap_writes_block_but_h_stops() {
    let (store, _s) = store_without(CHAIN, 43);
    let archive = TempDir::new().unwrap();
    let dir = archive.path();
    archive_from_store(dir, &store, 1..=40);
    write_manifest(dir, &manifest_json(40, &genesis_g(), Some(40)));

    let r = archive_finalized(dir, &store, &archive_block(&store, 45), &genesis_g()).unwrap();

    assert_pair_is_store_block(dir, &store, 41);
    assert_pair_is_store_block(dir, &store, 42);
    assert!(!block_path(dir, 43).exists());
    assert_pair_is_store_block(dir, &store, 45);
    assert_eq!(r.verified_height, 42);
    assert_eq!(manifest_h(dir), Some(42));
}

// REQ-ARCH-004 — Decision: a failure means a fresh archive never gains the key from the live path.
#[test]
fn archive_finalized_first_block_on_empty_archive_sets_h_one() {
    let (store, _s) = store_with_chain(genesis_g(), 1);
    let archive = TempDir::new().unwrap();
    let dir = archive.path();

    let r = archive_finalized(dir, &store, &archive_block(&store, 1), &genesis_g()).unwrap();

    assert_pair_is_store_block(dir, &store, 1);
    assert_eq!(r.files_written, 1);
    assert_eq!(r.verified_height, 1);
    assert_eq!(manifest_h(dir), Some(1));
}

// ---------------------------------------------------------------------------
// Undeserializable file (INC-I-119)
// ---------------------------------------------------------------------------

// REQ-ARCH-002 — Decision: a failure means an undeserializable file is rewritten or counted as verified.
#[test]
fn undeserializable_file_is_kept_and_h_stops_below_it() {
    for manifest in [None, Some(20)] {
        let (archive, store, _s) = small();
        let dir = archive.path();
        if let Some(h) = manifest {
            write_manifest(dir, &manifest_json(CHAIN, &genesis_g(), Some(h)));
        }
        let garbage = vec![0xFFu8; 64];
        write_pair(dir, 30, &garbage);

        let r = bounded(dir, &store, CHAIN);

        assert_eq!(
            std::fs::read(block_path(dir, 30)).unwrap(),
            garbage,
            "{manifest:?}"
        );
        assert_eq!(r.files_written, 0, "{manifest:?}");
        assert_eq!(r.verified_height, 29, "{manifest:?}");
        assert_eq!(manifest_h(dir), Some(29), "{manifest:?}");
    }
}

// ---------------------------------------------------------------------------
// O(1) stale-archive fence
// ---------------------------------------------------------------------------

fn fence_fixture(manifest: Option<&str>) -> (TempDir, PathBuf, Vec<u8>) {
    let parent = TempDir::new().unwrap();
    let dir = parent.path().join("archive");
    std::fs::create_dir_all(&dir).unwrap();
    let data = bincode::serialize(&block_at(genesis_other(), 1)).unwrap();
    write_pair(&dir, 1, &data);
    if let Some(m) = manifest {
        std::fs::write(dir.join("manifest.json"), m).unwrap();
    }
    (parent, dir, data)
}

fn entries(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

// REQ-ARCH-005 — Decision: a failure means old-chain files stay importable after a chain reset.
#[test]
fn fence_moves_archive_of_other_genesis_aside() {
    let m = manifest_json(1, &genesis_other(), Some(1)).to_string();
    let (parent, dir, data) = fence_fixture(Some(&m));

    let stale = fence_stale_archive(&dir, &genesis_g())
        .unwrap()
        .expect("fenced");

    let name = stale.file_name().unwrap().to_string_lossy().into_owned();
    let ts = name
        .strip_prefix("archive.stale-")
        .expect("<dir>.stale-<ts>");
    assert!(ts.parse::<u64>().is_ok(), "unix-ts suffix: {name}");
    assert_eq!(stale.parent(), Some(parent.path()));
    assert_eq!(std::fs::read(block_path(&stale, 1)).unwrap(), data);
    assert!(stale.join("manifest.json").exists());
    assert!(dir.is_dir(), "fresh archive dir recreated");
    assert_eq!(entries(&dir), Vec::<String>::new());
}

// REQ-ARCH-005 — Decision: a failure means the fence moves a valid archive and forces a needless rebuild.
#[test]
fn fence_leaves_archive_alone_when_not_provably_foreign() {
    let same = manifest_json(1, &genesis_g(), Some(1)).to_string();
    let no_genesis = serde_json::json!({"latest_height": 1, "latest_hash": "x"}).to_string();
    for manifest in [
        Some(same.as_str()),
        None,
        Some("{not json"),
        Some(no_genesis.as_str()),
    ] {
        let (parent, dir, data) = fence_fixture(manifest);
        let before = entries(&dir);

        let out = fence_stale_archive(&dir, &genesis_g()).unwrap();

        assert_eq!(out, None, "{manifest:?}");
        assert_eq!(entries(&dir), before, "{manifest:?}");
        assert_eq!(std::fs::read(block_path(&dir, 1)).unwrap(), data);
        assert_eq!(
            entries(parent.path()),
            vec!["archive".to_string()],
            "{manifest:?}"
        );
    }
}
