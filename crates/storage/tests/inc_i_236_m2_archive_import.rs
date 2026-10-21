//! INC-I-236 M2 — archive import paths verify the block signature at h >= AH (REQ-AUTH-019, census P14).
//!
// covers: import archiver integrity guardian restore validation block
// covers: crates/storage/src/archiver/import.rs crates/storage/src/archiver/mod.rs crates/rpc/src/methods/guardian.rs bins/node/src/node/integrity.rs bins/node/src/operations/restore.rs crates/core/src/validation/block.rs crates/core/src/validation/mod.rs
// covers: crates/storage/tests/it/inc_i_253_targeted_repair.rs crates/storage/tests/it/inc_i_253_height_range.rs bins/node/tests/it/inc_i_248_hole_backfill_order.rs bins/node/src/node/inc_i_253_integrity_tests.rs
// NEEDS-API: storage::archiver::{restore_from_archive, backfill_from_archive, force_backfill_from_archive}(archive_dir, block_store, expected_genesis_hash, activation_height: u64) and backfill_heights_from_archive(archive_dir, block_store, expected_genesis_hash, heights, activation_height: u64) — new trailing u64, checked with doli_core::validation::verify_block_signature_at before put_block_canonical
//
// OUTPUT CONTRACT: fn restore_from_archive / backfill_from_archive / force_backfill_from_archive
//   O1 return Result<u64 imported, String>; O2 block store writes (header, body, height index) per height
//   PATHS: P-BELOW h < AH (pi not read) | P-ARMED h >= AH valid | P-ARMED forged or unsigned (skip or abort, never write)
//     | P-FORCE existing block replaced only by a signed archive copy
// OUTPUT CONTRACT: fn backfill_heights_from_archive
//   O1 return Result<TargetedBackfillReport, String>; O2 block store writes per listed height
//   PATHS: same; a refused height counts in `rejected`, never in `imported`
// INPUT PARTITIONS
//   IP-SIG h4 in {forged (producer named, attacker pi), unsigned}; h1 unsigned below AH; h2 valid below AH; h3 valid at AH
//   IP-SWAP force mode: archive h4 = same-hash pi-variant of the stored signed h4 | a different forged block
//   IP-AH AH = 3 (armed at h3) | AH = u64::MAX (mainnet/testnet today)
// MATRIX
//   O1,O2 x {restore, backfill} x IP-SIG          -> req_auth_019_p14_archive_import_forged_not_imported
//   O1,O2 x targeted x IP-SIG                     -> req_auth_019_p14_archive_backfill_heights_forged_not_written
//   O1,O2 x force x IP-SWAP x IP-SIG              -> req_auth_019_p14_archive_force_backfill_does_not_replace_with_forged
//   O1,O2 x all four entries x AH = u64::MAX      -> req_auth_019_p14_archive_import_unsigned_history_below_ah_unchanged

use std::path::Path;

use crypto::{Hash, KeyPair};
use doli_core::validation::verify_block_signature;
use doli_core::{Block, BlockHeader};
use storage::archiver::{
    backfill_from_archive, backfill_heights_from_archive, force_backfill_from_archive,
    restore_from_archive, TargetedBackfillReport,
};
use storage::BlockStore;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

const AH: u64 = 3;
const TIP: u64 = 4;

type Entry = fn(&Path, &BlockStore, &Hash, u64) -> Result<u64, String>;

fn restore(dir: &Path, s: &BlockStore, g: &Hash, ah: u64) -> Result<u64, String> {
    restore_from_archive(dir, s, Some(g), ah)
}

fn backfill(dir: &Path, s: &BlockStore, g: &Hash, ah: u64) -> Result<u64, String> {
    backfill_from_archive(dir, s, Some(g), ah)
}

fn force(dir: &Path, s: &BlockStore, g: &Hash, ah: u64) -> Result<u64, String> {
    force_backfill_from_archive(dir, s, Some(g), ah)
}

fn targeted(
    dir: &Path,
    s: &BlockStore,
    g: &Hash,
    heights: &[u64],
    ah: u64,
) -> Result<TargetedBackfillReport, String> {
    backfill_heights_from_archive(dir, s, g, heights, ah)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Sig {
    Unsigned,
    Valid,
    Forged,
}

fn producer() -> KeyPair {
    KeyPair::from_seed([0x5A; 32])
}

fn sign_with(header: &mut BlockHeader, kp: &KeyPair) {
    let msg = header.signing_message();
    header.vdf_proof.pi = crypto::signature::sign(msg.as_bytes(), kp.private_key()).to_vec();
}

fn set_sig(header: &mut BlockHeader, sig: Sig) {
    header.vdf_proof = VdfProof::empty();
    match sig {
        Sig::Unsigned => {}
        Sig::Valid => sign_with(header, &producer()),
        Sig::Forged => sign_with(header, &KeyPair::from_seed([0x66; 32])),
    }
    let ok = verify_block_signature(header).is_ok();
    assert_eq!(
        ok,
        sig == Sig::Valid,
        "fixture {sig:?} slot={}",
        header.slot
    );
}

fn block(g: Hash, h: u64, prev: Hash, sig: Sig, variant: u64) -> Block {
    let mut header = BlockHeader {
        version: 2,
        prev_hash: prev,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: g,
        timestamp: 40_000 + h + variant * 1_000_000,
        slot: h as u32,
        producer: *producer().public_key(),
        vdf_output: VdfOutput {
            value: vec![h as u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    set_sig(&mut header, sig);
    Block::new(header, vec![])
}

fn genesis() -> Hash {
    crypto::hash::hash(b"inc-i-236-m2-archive-genesis")
}

/// h1 unsigned (< AH), h2 valid (< AH), h3 valid (== AH), h4 = `last` (> AH).
fn chain(g: Hash, last: Sig) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    for (h, sig) in [
        (1, Sig::Unsigned),
        (2, Sig::Valid),
        (3, Sig::Valid),
        (4, last),
    ] {
        let prev = out.last().map(|b| b.hash()).unwrap_or(g);
        out.push(block(g, h, prev, sig, 0));
    }
    out
}

fn archive(g: Hash, blocks: &[Block]) -> TempDir {
    let dir = TempDir::new().unwrap();
    let manifest = serde_json::json!({ "latest_height": TIP, "genesis_hash": g.to_string() });
    std::fs::write(dir.path().join("manifest.json"), manifest.to_string()).unwrap();
    for (i, b) in blocks.iter().enumerate() {
        plant(dir.path(), i as u64 + 1, b);
    }
    dir
}

fn plant(dir: &Path, h: u64, b: &Block) {
    let data = bincode::serialize(b).unwrap();
    std::fs::write(dir.join(format!("{h:010}.block")), &data).unwrap();
    let sum = crypto::hash::hash(&data).to_string();
    std::fs::write(dir.join(format!("{h:010}.blake3")), sum).unwrap();
}

fn store() -> (TempDir, BlockStore) {
    let tmp = TempDir::new().unwrap();
    let s = BlockStore::open(tmp.path()).unwrap();
    (tmp, s)
}

fn served(s: &BlockStore, h: u64) -> Option<Block> {
    s.get_block_by_height(h).unwrap()
}

fn assert_prefix_imported(s: &BlockStore, blocks: &[Block], what: &str) {
    for h in 1..=3u64 {
        let got = served(s, h).map(|b| b.hash());
        assert_eq!(got, Some(blocks[h as usize - 1].hash()), "{what}: h={h}");
    }
}

// REQ-AUTH-019 — Decision: a failure means bridgeFromArchive or `restore --from` writes a forged or unsigned post-height block into the canonical store.
#[test]
fn req_auth_019_p14_archive_import_forged_not_imported() {
    let g = genesis();
    for last in [Sig::Forged, Sig::Unsigned] {
        let blocks = chain(g, last);
        for (entry, run) in [("restore", restore as Entry), ("backfill", backfill)] {
            let what = format!("{entry} with h4 {last:?}");
            let ax = archive(g, &blocks);
            let (_tmp, s) = store();
            let r = run(ax.path(), &s, &g, AH);
            assert_prefix_imported(&s, &blocks, &what);
            let h4 = served(&s, TIP).map(|b| b.header.vdf_proof.pi);
            assert_eq!(h4, None, "{what}: h4 stored (pi shown), result {r:?}");
            assert!(matches!(r, Ok(3) | Err(_)), "{what}: result {r:?}");
        }
    }
}

// REQ-AUTH-019 — Decision: a failure means the automatic integrity repair writes a forged or unsigned post-height block from the archive.
#[test]
fn req_auth_019_p14_archive_backfill_heights_forged_not_written() {
    let g = genesis();
    for last in [Sig::Forged, Sig::Unsigned] {
        let blocks = chain(g, last);
        let ax = archive(g, &blocks);
        let (_tmp, s) = store();
        let r = targeted(ax.path(), &s, &g, &[1, 2, 3, 4], AH);
        let what = format!("targeted with h4 {last:?}");
        assert_prefix_imported(&s, &blocks, &what);
        assert_eq!(served(&s, TIP).map(|b| b.hash()), None, "{what}: h4 stored");
        let want = TargetedBackfillReport {
            imported: 3,
            already_present: 0,
            missing_files: 0,
            rejected: 1,
            heights_visited: 4,
            files_read: 4,
        };
        assert_eq!(r, Ok(want), "{what}: report");
    }
}

// REQ-AUTH-019 — Decision: a failure means force backfill swaps a stored signed block for a forged copy, including a same-hash pi swap the hash check cannot see.
#[test]
fn req_auth_019_p14_archive_force_backfill_does_not_replace_with_forged() {
    let g = genesis();
    let honest = chain(g, Sig::Valid);
    let stored_h4 = honest[3].clone();
    for bad in [Sig::Forged, Sig::Unsigned] {
        let mut swap = stored_h4.clone();
        set_sig(&mut swap.header, bad);
        assert_eq!(swap.hash(), stored_h4.hash(), "pi swap keeps hash()");
        let other = block(g, TIP, honest[2].hash(), bad, 1);
        assert_ne!(other.hash(), stored_h4.hash(), "other block differs");
        for (kind, replacement) in [("pi-swap", swap), ("other-block", other)] {
            let what = format!("force {kind} {bad:?}");
            let (_tmp, s) = store();
            for (i, b) in honest.iter().enumerate() {
                s.put_block_canonical(b, i as u64 + 1).unwrap();
            }
            let baseline = force(archive(g, &honest).path(), &s, &g, AH);
            let mut archived = honest[..3].to_vec();
            archived.push(replacement);
            let ax = archive(g, &archived);
            let r = force(ax.path(), &s, &g, AH);
            let kept = served(&s, TIP).expect("h4 must stay stored");
            assert_eq!(kept.hash(), stored_h4.hash(), "{what}: hash, result {r:?}");
            assert_eq!(
                kept.header.vdf_proof.pi, stored_h4.header.vdf_proof.pi,
                "{what}: stored pi replaced, result {r:?}"
            );
            assert!(
                r == baseline || r.is_err(),
                "{what}: result {r:?}, honest-archive baseline {baseline:?}"
            );
        }
    }
}

// REQ-AUTH-019 — Decision: a failure means the fix breaks archive restore and integrity repair of the unsigned mainnet/testnet history below the height.
#[test]
fn req_auth_019_p14_archive_import_unsigned_history_below_ah_unchanged() {
    let g = genesis();
    let mut blocks: Vec<Block> = Vec::new();
    for h in 1..=TIP {
        let prev = blocks.last().map(|b| b.hash()).unwrap_or(g);
        let mut b = block(g, h, prev, Sig::Unsigned, 0);
        if h == TIP {
            b.header.vdf_proof.pi = vec![0xEE; 5];
        }
        blocks.push(b);
    }
    for ah in [u64::MAX, TIP + 1] {
        for entry in ["restore", "backfill", "force", "targeted"] {
            let ax = archive(g, &blocks);
            let (_tmp, s) = store();
            let imported = match entry {
                "restore" => restore(ax.path(), &s, &g, ah),
                "backfill" => backfill(ax.path(), &s, &g, ah),
                "force" => force(ax.path(), &s, &g, ah),
                _ => targeted(ax.path(), &s, &g, &[1, 2, 3, 4], ah).map(|r| r.imported),
            };
            assert_eq!(imported, Ok(TIP), "{entry} AH={ah}");
            for (i, b) in blocks.iter().enumerate() {
                let h = i as u64 + 1;
                let got = served(&s, h).map(|x| x.header);
                assert_eq!(
                    got.as_ref().map(|x| x.hash()),
                    Some(b.hash()),
                    "{entry} h={h}"
                );
                assert_eq!(
                    got.map(|x| x.vdf_proof.pi),
                    Some(b.header.vdf_proof.pi.clone())
                );
            }
        }
    }
}
