// INC-I-253 M2 — targeted archive repair visits only the listed heights.
// covers: archiver/import.rs backfill_heights_from_archive, archiver/mod.rs re-export
//
// OUTPUT CONTRACT: fn backfill_heights_from_archive(dir, store, &genesis, heights)
//   O1: none (no mutable params)
//   O2: none (free fn)
//   O3: Err iff manifest absent or manifest genesis != expected; else Ok(TargetedBackfillReport
//       { imported, already_present, missing_files, rejected, heights_visited, files_read })
//   O4: block store — imported heights served with the archive hash; rejected/absent heights
//       stay None; neighbors, key 0 and unlisted holes unchanged
// OUTPUT CONTRACT: fn backfill_from_archive / force_backfill_from_archive (regression)
//   O3: Ok(imported) or today's Err string; O4: holes filled, divergent blocks replaced
// PATHS: T1 import, T2 already stored, T3 .block absent, T4 bad sidecar, T5 block genesis wrong,
//   T6 h == 0, T7 prev link wrong, T8 next link wrong, T9 manifest absent/wrong, T10 empty list,
//   T11 h == 1 against key 0
// INPUT PARTITIONS:
//   A1 300 blocks, holes {150, 200}, every other .block garbage, list [150]
//   A2 list [149] stored   A3 holes {150, 160}, 150.block absent, list [150, 160]
//   A4 holes {100, 150}, 100.blake3 wrong, list [100, 150]   A5 holes {149, 150, 151}, foreign genesis
//   A6 valid pair planted at 0   A7 holes {150, 151}, prev != hash(149)   A8 hole 150, prev ok, 151 unlinked
//   A9 manifest genesis other / no manifest   A10 list []   A11 hole 1
//   R1 full-range, 100.blake3 wrong   R2 full-range, hole 150   R3 force, fork at 149..=150
// MATRIX: O3 × A1-A11 (whole report), O4 × A1-A11 (listed + neighbor heights); O3/O4 × R1-R3

use std::path::{Path, PathBuf};

use crypto::{Hash, KeyPair};
use doli_core::{Block, BlockHeader};
use storage::archiver::{
    backfill_from_archive, backfill_heights_from_archive, force_backfill_from_archive,
    BlockArchiver, TargetedBackfillReport,
};
use storage::BlockStore;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

fn block_at(genesis: Hash, h: u64, prev: Hash, variant: u64) -> Block {
    let kp = KeyPair::from_seed([12u8; 32]);
    let header = BlockHeader {
        version: 1,
        prev_hash: prev,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: genesis,
        timestamp: 30_000 + h + variant * 1_000_000,
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

fn block_file(dir: &Path, h: u64) -> PathBuf {
    dir.join(format!("{:010}.block", h))
}

fn sum_file(dir: &Path, h: u64) -> PathBuf {
    dir.join(format!("{:010}.blake3", h))
}

/// Writes a `.block` + matching `.blake3` pair, as the archiver does.
fn plant(dir: &Path, h: u64, b: &Block) {
    let data = bincode::serialize(b).unwrap();
    std::fs::write(block_file(dir, h), &data).unwrap();
    std::fs::write(sum_file(dir, h), crypto::hash::hash(&data).to_string()).unwrap();
}

fn report(
    imported: u64,
    already_present: u64,
    missing_files: u64,
    rejected: u64,
    heights_visited: u64,
    files_read: u64,
) -> TargetedBackfillReport {
    TargetedBackfillReport {
        imported,
        already_present,
        missing_files,
        rejected,
        heights_visited,
        files_read,
    }
}

struct Ax {
    store: BlockStore,
    g: Hash,
    /// `hashes[h]`; `hashes[0]` is the genesis.
    hashes: Vec<Hash>,
    archive: TempDir,
    _tmp: TempDir,
}

impl Ax {
    /// Key 0 + 1..=n applied, archive 1..=n written by the production catch-up.
    fn new(n: u64) -> Self {
        let tmp = TempDir::new().unwrap();
        let store = BlockStore::open(tmp.path()).unwrap();
        let g = crypto::hash::hash(b"inc-i-253-m2-genesis");
        let mut hashes = vec![g];
        for h in 1..=n {
            let b = block_at(g, h, hashes[h as usize - 1], 0);
            store.put_block(&b, h).unwrap();
            store.set_canonical_chain(b.hash(), h).unwrap();
            hashes.push(b.hash());
        }
        let archive = TempDir::new().unwrap();
        BlockArchiver::catch_up(archive.path(), &store, n, Some(&g)).unwrap();
        assert!(
            block_file(archive.path(), n).exists(),
            "precondition: archive"
        );
        assert_eq!(
            store.get_hash_by_height(0).unwrap(),
            Some(g),
            "precondition: key 0"
        );
        Self {
            store,
            g,
            hashes,
            archive,
            _tmp: tmp,
        }
    }

    fn dir(&self) -> &Path {
        self.archive.path()
    }

    fn served(&self, h: u64) -> Option<Hash> {
        self.store.get_block_by_height(h).unwrap().map(|b| b.hash())
    }

    fn hole(&self, h: u64) {
        assert!(
            self.store.delete_block_by_height(h).unwrap(),
            "deleted h={h}"
        );
        assert_eq!(self.served(h), None, "precondition: hole at {h}");
    }

    fn assert_original(&self, h: u64) {
        assert_eq!(self.served(h), Some(self.hashes[h as usize]), "h={h}");
    }

    fn run(&self, heights: &[u64]) -> Result<TargetedBackfillReport, String> {
        backfill_heights_from_archive(self.dir(), &self.store, &self.g, heights)
    }
}

// REQ-INTEG-006 — Decision: files_read > 1 or a stored 200 means the repair still walks the whole archive
#[test]
fn inc_i_253_targeted_reads_only_the_listed_height() {
    let ax = Ax::new(300);
    ax.hole(150);
    ax.hole(200);
    for h in (1..=300u64).filter(|&h| h != 150) {
        std::fs::write(block_file(ax.dir(), h), b"garbage").unwrap();
    }

    let got = ax.run(&[150]);

    assert_eq!(got, Ok(report(1, 0, 0, 0, 1, 1)), "observed {got:?}");
    ax.assert_original(150);
    assert_eq!(ax.served(200), None, "200 was never visited");
    ax.assert_original(149);
    ax.assert_original(151);
}

// REQ-INTEG-006 — Decision: a file read for a stored height makes repair cost scale with the chain
#[test]
fn inc_i_253_targeted_already_stored_reads_no_file() {
    let ax = Ax::new(200);

    let got = ax.run(&[149]);

    assert_eq!(got, Ok(report(0, 1, 0, 0, 1, 0)), "observed {got:?}");
    ax.assert_original(149);
}

// REQ-INTEG-006 — Decision: an Err or a stop at 150 means one absent file blocks every other repair
#[test]
fn inc_i_253_targeted_absent_file_counts_and_continues() {
    let ax = Ax::new(200);
    ax.hole(150);
    ax.hole(160);
    std::fs::remove_file(block_file(ax.dir(), 150)).unwrap();

    let r = ax
        .run(&[150, 160])
        .expect("absent file is not a whole-call error");

    assert_eq!(r.missing_files, 1, "observed {r:?}");
    assert_eq!(r.imported, 1, "observed {r:?}");
    assert_eq!(r.rejected, 0, "observed {r:?}");
    assert_eq!(r.already_present, 0, "observed {r:?}");
    assert_eq!(r.heights_visited, 2, "observed {r:?}");
    assert_eq!(ax.served(150), None);
    ax.assert_original(160);
}

// REQ-INTEG-SEC-001 — Decision: an Err or an import at 100 means one tampered sidecar blocks or poisons repair
#[test]
fn inc_i_253_targeted_bad_sidecar_rejects_only_that_height() {
    let ax = Ax::new(200);
    ax.hole(100);
    ax.hole(150);
    std::fs::write(
        sum_file(ax.dir(), 100),
        crypto::hash::hash(b"tampered").to_string(),
    )
    .unwrap();

    let got = ax.run(&[100, 150]);

    assert_eq!(got, Ok(report(1, 0, 0, 1, 2, 2)), "observed {got:?}");
    assert_eq!(ax.served(100), None, "tampered height not imported");
    ax.assert_original(150);
}

// REQ-INTEG-SEC-001 — Decision: Ok here means a wrong-chain archive is accepted as a repair source
#[test]
fn inc_i_253_targeted_manifest_absent_or_wrong_genesis_is_err() {
    let ax = Ax::new(200);
    ax.hole(150);
    let other = crypto::hash::hash(b"inc-i-253-other-chain");

    let wrong = backfill_heights_from_archive(ax.dir(), &ax.store, &other, &[150]);
    assert!(wrong.is_err(), "observed {wrong:?}");
    assert_eq!(ax.served(150), None);

    let empty = TempDir::new().unwrap();
    let absent = backfill_heights_from_archive(empty.path(), &ax.store, &ax.g, &[150]);
    assert!(absent.is_err(), "observed {absent:?}");
    assert_eq!(ax.served(150), None);
}

// REQ-INTEG-SEC-001 — Decision: an import here writes a foreign-chain block into the canonical index
#[test]
fn inc_i_253_targeted_foreign_genesis_block_rejected() {
    let ax = Ax::new(200);
    for h in [149, 150, 151] {
        ax.hole(h);
    }
    let other = crypto::hash::hash(b"inc-i-253-other-chain");
    plant(ax.dir(), 150, &block_at(other, 150, ax.hashes[149], 0));

    let got = ax.run(&[150]);

    assert_eq!(got, Ok(report(0, 0, 0, 1, 1, 1)), "observed {got:?}");
    assert_eq!(ax.served(150), None);
}

// REQ-INTEG-SEC-001 — Decision: an import or a read at 0 overwrites the virtual-genesis key
#[test]
fn inc_i_253_targeted_height_zero_rejected_unread() {
    let ax = Ax::new(200);
    std::fs::copy(block_file(ax.dir(), 1), block_file(ax.dir(), 0)).unwrap();
    std::fs::copy(sum_file(ax.dir(), 1), sum_file(ax.dir(), 0)).unwrap();

    let got = ax.run(&[0]);

    assert_eq!(got, Ok(report(0, 0, 0, 1, 1, 0)), "observed {got:?}");
    assert_eq!(ax.store.get_hash_by_height(0).unwrap(), Some(ax.g));
    ax.assert_original(1);
}

// REQ-INTEG-SEC-002 — Decision: an import here splices another branch under a stored parent
#[test]
fn inc_i_253_targeted_prev_link_mismatch_rejected() {
    let ax = Ax::new(200);
    ax.hole(150);
    ax.hole(151);
    let stray = crypto::hash::hash(b"inc-i-253-other-branch");
    plant(ax.dir(), 150, &block_at(ax.g, 150, stray, 0));

    let got = ax.run(&[150]);

    assert_eq!(got, Ok(report(0, 0, 0, 1, 1, 1)), "observed {got:?}");
    assert_eq!(ax.served(150), None);
    ax.assert_original(149);
    assert_eq!(ax.served(151), None);
}

// REQ-INTEG-SEC-002 — Decision: an import here breaks the stored child's link to its parent
#[test]
fn inc_i_253_targeted_next_link_mismatch_rejected() {
    let ax = Ax::new(200);
    ax.hole(150);
    let fork = block_at(ax.g, 150, ax.hashes[149], 1);
    assert_ne!(fork.hash(), ax.hashes[150], "precondition: other branch");
    plant(ax.dir(), 150, &fork);

    let got = ax.run(&[150]);

    assert_eq!(got, Ok(report(0, 0, 0, 1, 1, 1)), "observed {got:?}");
    assert_eq!(ax.served(150), None);
    ax.assert_original(149);
    ax.assert_original(151);
}

// REQ-INTEG-006 — Decision: any visit here means an empty list falls back to the full range
#[test]
fn inc_i_253_targeted_empty_list_visits_nothing() {
    let ax = Ax::new(200);
    ax.hole(150);

    let got = ax.run(&[]);

    assert_eq!(
        got,
        Ok(TargetedBackfillReport::default()),
        "observed {got:?}"
    );
    assert_eq!(ax.served(150), None);
}

// REQ-INTEG-SEC-002 — Decision: a reject here means the link check treats key 0 as a foreign parent
#[test]
fn inc_i_253_targeted_height_one_links_to_key0() {
    let ax = Ax::new(200);
    ax.hole(1);

    let got = ax.run(&[1]);

    assert_eq!(got, Ok(report(1, 0, 0, 0, 1, 1)), "observed {got:?}");
    ax.assert_original(1);
}

// REQ-INTEG-006 — Decision: Ok here means the shared read path changed the CLI/BRIDGE abort contract
#[test]
fn inc_i_253_full_range_bad_sidecar_still_aborts() {
    let ax = Ax::new(200);
    ax.hole(100);
    std::fs::write(
        sum_file(ax.dir(), 100),
        crypto::hash::hash(b"tampered").to_string(),
    )
    .unwrap();

    let err = backfill_from_archive(ax.dir(), &ax.store, Some(&ax.g))
        .expect_err("full-range pass aborts on a bad sidecar");

    assert!(
        err.starts_with("Checksum mismatch at height 100"),
        "observed {err}"
    );
    assert_eq!(ax.served(100), None);
}

// REQ-INTEG-006 — Decision: a changed count here means the full-range backfill regressed for its callers
#[test]
fn inc_i_253_full_range_complete_archive_unchanged() {
    let ax = Ax::new(200);
    ax.hole(150);

    assert_eq!(
        backfill_from_archive(ax.dir(), &ax.store, Some(&ax.g)),
        Ok(1)
    );
    ax.assert_original(150);
    assert_eq!(
        backfill_from_archive(ax.dir(), &ax.store, Some(&ax.g)),
        Ok(0)
    );
}

// REQ-INTEG-006 — Decision: a failure here means the link check leaked into force mode (INC-I-055 fork repair)
#[test]
fn inc_i_253_force_backfill_still_replaces_divergent_fork() {
    let ax = Ax::new(200);
    let f149 = block_at(ax.g, 149, ax.hashes[148], 1);
    let f150 = block_at(ax.g, 150, f149.hash(), 1);
    ax.store.put_block_canonical(&f149, 149).unwrap();
    ax.store.put_block_canonical(&f150, 150).unwrap();
    assert_eq!(ax.served(149), Some(f149.hash()), "precondition: fork");
    assert_eq!(ax.served(150), Some(f150.hash()), "precondition: fork");

    let got = force_backfill_from_archive(ax.dir(), &ax.store, Some(&ax.g));

    assert_eq!(got, Ok(2), "observed {got:?}");
    ax.assert_original(149);
    ax.assert_original(150);
}
