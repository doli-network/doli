// INC-I-253 M1 — the 100-block integrity scan must agree with the archive repair.
// covers: maintenance.rs height_range, periodic.rs integrity_scan, integrity.rs scan_floor, mod.rs
//
// OUTPUT CONTRACT: fn integrity_scan(block_store, scan_tip, archive_configured) -> IntegrityReport
//   O1: none (store borrowed immutably)
//   O2: none (pure block-store reads)
//   O3a: .missing_count — heights in floor..=scan_tip without a served block (never h=0)
//   O3b: .commitment — fold(prev, hash) over served heights floor..=scan_tip, ascending
//   O4: none (state_db writes stay in the caller)
// OUTPUT CONTRACT: fn archiver::backfill_from_archive(dir, store, Some(&g)) — used by the repair cycle
//   O3: Ok(imported); O4: block store gains the archived block at each hole
// PATHS: P1 store complete over the floor range, P2 store has holes, P3 repair cycle
// INPUT PARTITIONS:
//   P1a key 0 + 1..=300 applied (put_block + set_canonical_chain), tip 300 — old floor 0 -> 1 phantom
//   P1b key 0 + 1..=300, scan_tip 200 < store max — range exactly 1..=200
//   P1c snap anchor index-only at S=300, blocks 301..=600 — floor S+1; old byte floor 512
//   P1d key 0 + pruned below cutoff 300, tip 2300 — floor cutoff; old floor 0 -> 300 missing
//   P2a 1..=300 put_block_canonical, no key 0, hole at 5 — old byte floor 256 hides it
//   P3a key 0 + 1..=300 + archive 1..=300, hole at 150 — scan 1, import 1, rescan 0
// MATRIX: O3a/O3b × {P1a, P1b, P1c, P1d, P2a} = 10 cells; P3a O3a×2 + O3b + backfill O3/O4 = 5 cells
// Source tripwires (REQ-INTEG-004/010): periodic.rs text; the planted violation is today's code.

use std::collections::BTreeMap;
use std::sync::Arc;

use crypto::{Hash, KeyPair};
use doli_core::{Block, BlockHeader};
use storage::archiver::{backfill_from_archive, BlockArchiver};
use storage::BlockStore;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

const PERIODIC_SRC: &str = include_str!("periodic.rs");

fn block_at(genesis: Hash, h: u64, prev: Hash) -> Block {
    let kp = KeyPair::from_seed([11u8; 32]);
    let header = BlockHeader {
        version: 1,
        prev_hash: prev,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: genesis,
        timestamp: 20_000 + h,
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

fn fold<'a>(hashes: impl IntoIterator<Item = &'a Hash>) -> Hash {
    let mut c = Hash::default();
    for h in hashes {
        let mut hasher = crypto::Hasher::new();
        hasher.update(c.as_bytes());
        hasher.update(h.as_bytes());
        c = hasher.finalize();
    }
    c
}

pub(super) struct Fx {
    pub store: Arc<BlockStore>,
    pub g: Hash,
    /// Every block ever stored, by height (holes are tracked by the test).
    pub hashes: BTreeMap<u64, Hash>,
    _tmp: TempDir,
}

impl Fx {
    fn empty() -> Self {
        let tmp = TempDir::new().unwrap();
        let store = Arc::new(BlockStore::open(tmp.path()).unwrap());
        Self {
            store,
            g: crypto::hash::hash(b"inc-i-253-node-genesis"),
            hashes: BTreeMap::new(),
            _tmp: tmp,
        }
    }

    /// Production apply path from genesis: writes height_index[0] = genesis.
    fn apply_from(&mut self, from: u64, to: u64, mut prev: Hash) {
        for h in from..=to {
            let b = block_at(self.g, h, prev);
            prev = b.hash();
            self.store.put_block(&b, h).unwrap();
            self.store.set_canonical_chain(prev, h).unwrap();
            self.hashes.insert(h, prev);
        }
    }

    pub(super) fn applied(n: u64) -> Self {
        let mut fx = Self::empty();
        let g = fx.g;
        fx.apply_from(1, n, g);
        assert_eq!(
            fx.store.get_hash_by_height(0).unwrap(),
            Some(g),
            "precondition: key 0"
        );
        fx
    }

    pub(super) fn canonical_no_key0(n: u64) -> Self {
        let mut fx = Self::empty();
        let mut prev = fx.g;
        for h in 1..=n {
            let b = block_at(fx.g, h, prev);
            prev = b.hash();
            fx.store.put_block_canonical(&b, h).unwrap();
            fx.hashes.insert(h, prev);
        }
        assert_eq!(
            fx.store.get_hash_by_height(0).unwrap(),
            None,
            "precondition: no key 0"
        );
        fx
    }

    /// Snap-synced shape: anchor at `s` is index-only, blocks s+1..=tip applied.
    pub(super) fn snap_synced(s: u64, tip: u64) -> Self {
        let mut fx = Self::empty();
        let anchor = crypto::hash::hash(b"inc-i-253-snap-anchor");
        fx.store.seed_canonical_index(anchor, s).unwrap();
        fx.apply_from(s + 1, tip, anchor);
        assert_eq!(fx.store.get_hash_by_height(s).unwrap(), Some(anchor));
        assert!(
            fx.store.get_block_by_height(s).unwrap().is_none(),
            "anchor has no block"
        );
        assert_eq!(fx.store.get_hash_by_height(0).unwrap(), None);
        fx
    }

    /// Key 0 + 1..=tip applied, then pruned below `cutoff` by the production pruner.
    pub(super) fn pruned(cutoff: u64, tip: u64) -> Self {
        let mut fx = Self::applied(tip);
        let out = fx.store.prune_blocks_below(cutoff, tip).unwrap();
        assert_eq!(
            out,
            (cutoff - 1, cutoff),
            "precondition: pruner removed 1..cutoff"
        );
        fx.hashes.retain(|h, _| *h >= cutoff);
        assert_eq!(
            fx.store.get_hash_by_height(0).unwrap(),
            Some(fx.g),
            "pruner keeps key 0"
        );
        fx
    }

    pub(super) fn delete(&self, h: u64) {
        assert!(
            self.store.delete_block_by_height(h).unwrap(),
            "deleted h={h}"
        );
        assert!(self.store.get_block_by_height(h).unwrap().is_none());
    }

    pub(super) fn archive(&self, tip: u64) -> TempDir {
        let dir = TempDir::new().unwrap();
        BlockArchiver::catch_up(dir.path(), &self.store, tip, Some(&self.g)).unwrap();
        assert!(dir.path().join(format!("{:010}.block", tip)).exists());
        dir
    }

    pub(super) fn assert_served(&self, range: std::ops::RangeInclusive<u64>) {
        for h in range {
            let got = self.store.get_block_by_height(h).unwrap().map(|b| b.hash());
            assert_eq!(
                got,
                self.hashes.get(&h).copied(),
                "precondition: served h={h}"
            );
        }
    }

    /// Commitment over stored heights in `range`, skipping `holes`.
    pub(super) fn commitment(&self, range: std::ops::RangeInclusive<u64>, holes: &[u64]) -> Hash {
        fold(
            self.hashes
                .range(range)
                .filter(|(h, _)| !holes.contains(h))
                .map(|(_, v)| v),
        )
    }
}

// REQ-INTEG-001 — Decision: a non-zero count here is the phantom that triggers a full backfill every 100 blocks
#[test]
fn inc_i_253_repro_complete_store_reports_zero_missing() {
    let fx = Fx::applied(300);
    fx.assert_served(1..=300);

    let r = super::integrity_scan(&fx.store, 300, true);
    let (missing, commitment) = (r.missing_count, r.commitment);
    assert_eq!(
        missing, 0,
        "observed missing = {missing} on a store that serves every h in 1..=300"
    );
    assert_eq!(
        commitment,
        fx.commitment(1..=300, &[]),
        "commitment covers 1..=300"
    );
}

// REQ-INTEG-001 — Decision: 0 here means the byte-order floor 256 still hides real holes below it
#[test]
fn inc_i_253_blind_spot_hole_at_5_is_reported() {
    let fx = Fx::canonical_no_key0(300);
    fx.delete(5);

    let r = super::integrity_scan(&fx.store, 300, false);
    let (missing, commitment) = (r.missing_count, r.commitment);
    assert_eq!(
        missing, 1,
        "observed missing = {missing}; hole at 5 must be seen"
    );
    assert_eq!(commitment, fx.commitment(1..=300, &[5]));
}

// REQ-INTEG-003 — Decision: a count != 1 or import != 1 means scan and repair still disagree on the range
#[test]
fn inc_i_253_real_hole_repaired_once_then_zero() {
    let fx = Fx::applied(300);
    let archive = fx.archive(300);
    let full = fx.commitment(1..=300, &[]);
    fx.delete(150);

    let first = super::integrity_scan(&fx.store, 300, true).missing_count;
    assert_eq!(
        first, 1,
        "observed first-scan missing = {first}; only h=150 is absent"
    );

    let imported = backfill_from_archive(archive.path(), &fx.store, Some(&fx.g), u64::MAX);
    assert_eq!(imported, Ok(1), "observed backfill = {imported:?}");
    fx.assert_served(1..=300);

    let r = super::integrity_scan(&fx.store, 300, true);
    let (after, commitment) = (r.missing_count, r.commitment);
    assert_eq!(after, 0, "observed rescan missing = {after}");
    assert_eq!(
        commitment, full,
        "repaired chain commits like the intact chain"
    );
}

// REQ-INTEG-002 — Decision: a non-zero count means the scan range is not exactly 1..=scan_tip on key-0 stores
#[test]
fn inc_i_253_floor_key0_scan_tip_below_store_max() {
    let fx = Fx::applied(300);

    let r = super::integrity_scan(&fx.store, 200, false);
    let (missing, commitment) = (r.missing_count, r.commitment);
    assert_eq!(missing, 0, "observed missing = {missing} for scan 1..=200");
    assert_eq!(commitment, fx.commitment(1..=200, &[]));
}

// REQ-INTEG-002 — Decision: a WARN or a 512-based commitment here means snap-synced producers get a wrong floor
#[test]
fn inc_i_253_floor_snap_synced_non_archive_zero_missing() {
    let fx = Fx::snap_synced(300, 600);
    fx.assert_served(301..=600);

    let r = super::integrity_scan(&fx.store, 600, false);
    let (missing, commitment) = (r.missing_count, r.commitment);
    assert_eq!(missing, 0, "observed missing = {missing}");
    assert_eq!(
        commitment,
        fx.commitment(301..=600, &[]),
        "commitment must cover S+1..=tip (301..=600), not a byte-order floor"
    );
}

// REQ-INTEG-002 — Decision: a non-zero count means pruned non-archive nodes report pruned heights as missing
#[test]
fn inc_i_253_floor_pruned_non_archive_zero_missing() {
    let fx = Fx::pruned(300, 2300);
    fx.assert_served(300..=2300);

    let r = super::integrity_scan(&fx.store, 2300, false);
    let (missing, commitment) = (r.missing_count, r.commitment);
    assert_eq!(
        missing, 0,
        "observed missing = {missing} on key 0 + 300..=2300"
    );
    assert_eq!(commitment, fx.commitment(300..=2300, &[]));
}

// REQ-INTEG-010 — Decision: a height_range() floor left in periodic.rs means the two integrity paths can diverge again
#[test]
fn inc_i_253_periodic_has_no_height_range_floor() {
    assert!(
        PERIODIC_SRC.contains("fn maybe_run_integrity_check"),
        "real periodic.rs"
    );
    let n = PERIODIC_SRC.matches(".height_range()").count();
    assert_eq!(
        n, 0,
        "observed {n} height_range() floor derivations in periodic.rs"
    );
}

// REQ-INTEG-004 — Decision: a hardcoded `1..=` in a missing-blocks line hides the true scanned range from operators
#[test]
fn inc_i_253_periodic_missing_lines_do_not_hardcode_floor_1() {
    assert!(PERIODIC_SRC.contains("[INTEGRITY]"), "real periodic.rs");
    for lit in [
        "missing blocks in 1..={}",
        "in 1..={} (archive may be incomplete)",
    ] {
        assert!(
            !PERIODIC_SRC.contains(lit),
            "periodic.rs still logs `{lit}`"
        );
    }
}

// REQ-INTEG-001 — Decision: outcome metric only (asserts nothing); feeds .claude/scripts/inc-i-253-m1-probe.sh
#[test]
fn inc_i_253_outcome_probe() {
    let fx = Fx::applied(300);
    let archive = fx.archive(300);
    let mut first_scan_missing = None;
    let mut passes = 0u64;
    for _tick in 0..36 {
        let missing = super::integrity_scan(&fx.store, 300, true).missing_count;
        first_scan_missing.get_or_insert(missing);
        if missing > 0 {
            let _ = backfill_from_archive(archive.path(), &fx.store, Some(&fx.g), u64::MAX);
            passes += 1;
            let _ = super::integrity_scan(&fx.store, 300, true);
        }
    }
    println!(
        "INC253_OUTCOME phantom_missing={} backfill_passes_per_hour={:.1}",
        first_scan_missing.unwrap_or(0),
        passes as f64 / 10.0
    );
}

// Target API: red (compile error) until bins/node/src/node/integrity.rs exists.
// OUTPUT CONTRACT: fn scan_floor(store, archive_configured) -> u64
//   O3: floor >= 1; 1 if archive_configured or block 1 stored; else lowest stored height >= 1
// OUTPUT CONTRACT: fn integrity_scan(store, scan_tip, archive_configured) -> IntegrityReport
//   O3: floor, tip, missing_count, missing (ascending, <= INTEGRITY_MISSING_LIST_CAP), commitment
// OUTPUT CONTRACT: fn IntegrityReport::missing_summary(&self) -> String
//   O3: "{count} missing blocks in {floor}..={tip}: [first <= 10]" + " (+{count-10} more)" if count > 10
// INPUT PARTITIONS:
//   F1 key 0 + full 1..=300 -> 1 (both flags)    F2 block 1 absent: archive -> 1, no archive -> 2
//   F3 snap S=300: no archive -> 301, archive -> 1   F4 pruned cutoff 300: no archive -> 300, archive -> 1
//   F5 no key 0, full 1..=300 -> 1
//   F6 empty store, no archive -> 1   F7 snap anchor S=300 only (no block yet), no archive -> 301, empty scan
//   R1 complete -> 0 missing   R2 holes [5, 150]   R3 12 holes   R4 holes > cap (list capped, count exact)
//   R5 snap no-archive -> floor 301, 0 missing   R6 hole 150 -> report [150], repair, clean report
//   S1 count 2   S2 count == 10 (no suffix)   S3 floor != 1   S4 count > list cap (suffix uses count)
// MATRIX: scan_floor O3 × 9 cells; report O3 (5 fields) × R1-R6; summary O3 × S1-S4
mod target_api {
    use super::{block_at, Fx, PERIODIC_SRC};
    use crate::node::integrity::{
        self, IntegrityReport, INTEGRITY_LOG_HEIGHTS, INTEGRITY_MISSING_LIST_CAP,
    };
    use crypto::Hash;
    use storage::archiver::backfill_from_archive;

    const INTEGRITY_SRC: &str = include_str!("integrity.rs");

    fn report(floor: u64, tip: u64, count: u64, missing: Vec<u64>) -> IntegrityReport {
        IntegrityReport {
            floor,
            tip,
            missing_count: count,
            missing,
            commitment: Hash::ZERO,
        }
    }

    // REQ-INTEG-002 — Decision: a floor of 0 or 256 here means the key-0 / byte-order defect is back
    #[test]
    fn target_scan_floor_key0_full_store_is_1() {
        let fx = Fx::applied(300);
        assert_eq!(integrity::scan_floor(&fx.store, false), 1);
        assert_eq!(integrity::scan_floor(&fx.store, true), 1);
    }

    // REQ-INTEG-002 — Decision: floor 2 with an archive means a lost block 1 is never repaired
    #[test]
    fn target_scan_floor_block1_absent_depends_on_archive() {
        let fx = Fx::applied(300);
        fx.delete(1);
        assert_eq!(integrity::scan_floor(&fx.store, true), 1);
        assert_eq!(integrity::scan_floor(&fx.store, false), 2);
    }

    // REQ-INTEG-002 — Decision: a floor at or below S makes every snap-synced producer WARN forever
    #[test]
    fn target_scan_floor_snap_synced_is_anchor_plus_1() {
        let fx = Fx::snap_synced(300, 600);
        assert_eq!(integrity::scan_floor(&fx.store, false), 301);
        assert_eq!(integrity::scan_floor(&fx.store, true), 1);
    }

    // REQ-INTEG-002 — Decision: a floor below the cutoff makes pruned non-archive nodes WARN forever
    #[test]
    fn target_scan_floor_pruned_is_cutoff() {
        let fx = Fx::pruned(300, 2300);
        assert_eq!(integrity::scan_floor(&fx.store, false), 300);
        assert_eq!(integrity::scan_floor(&fx.store, true), 1);
    }

    // REQ-INTEG-002 — Decision: 256 here means the floor still depends on RocksDB key byte order
    #[test]
    fn target_scan_floor_no_key0_full_store_is_1() {
        let fx = Fx::canonical_no_key0(300);
        assert_eq!(integrity::scan_floor(&fx.store, false), 1);
    }

    // REQ-INTEG-002 — Decision: a floor other than 1 on an empty store invents a history floor
    #[test]
    fn target_scan_floor_empty_store_is_1() {
        let fx = Fx::empty();
        assert_eq!(integrity::scan_floor(&fx.store, false), 1);
    }

    // REQ-INTEG-002 — Decision: a floor <= S right after a snap makes the node count the anchor as missing
    #[test]
    fn target_scan_floor_snap_anchor_only_is_anchor_plus_1() {
        let fx = Fx::empty();
        let anchor = crypto::hash::hash(b"inc-i-253-anchor-only");
        fx.store.seed_canonical_index(anchor, 300).unwrap();
        assert_eq!(integrity::scan_floor(&fx.store, false), 301);
        let got = integrity::integrity_scan(&fx.store, 300, false);
        let want = IntegrityReport {
            commitment: Hash::default(),
            ..report(301, 300, 0, vec![])
        };
        assert_eq!(got, want);
    }

    // REQ-INTEG-001 — Decision: any field off here means the clean-chain report still carries the phantom
    #[test]
    fn target_report_complete_store() {
        let fx = Fx::applied(300);
        let got = integrity::integrity_scan(&fx.store, 300, true);
        let want = IntegrityReport {
            commitment: fx.commitment(1..=300, &[]),
            ..report(1, 300, 0, vec![])
        };
        assert_eq!(got, want);
    }

    // REQ-INTEG-004 — Decision: a wrong list or summary means operators cannot see which heights are missing
    #[test]
    fn target_report_two_holes_names_them() {
        let fx = Fx::applied(300);
        fx.delete(5);
        fx.delete(150);
        let got = integrity::integrity_scan(&fx.store, 300, true);
        let want = IntegrityReport {
            commitment: fx.commitment(1..=300, &[5, 150]),
            ..report(1, 300, 2, vec![5, 150])
        };
        assert_eq!(got, want);
        assert_eq!(
            got.missing_summary(),
            "2 missing blocks in 1..=300: [5, 150]"
        );
    }

    // REQ-INTEG-004 — Decision: a missing `(+N more)` suffix hides how many heights are absent beyond the first 10
    #[test]
    fn target_report_more_than_10_holes_summary_suffix() {
        let fx = Fx::applied(300);
        for h in 1..=12 {
            fx.delete(h);
        }
        let got = integrity::integrity_scan(&fx.store, 300, true);
        assert_eq!(got.floor, 1, "archive configured keeps floor 1");
        assert_eq!(got.missing_count, 12);
        assert_eq!(got.missing, (1..=12).collect::<Vec<u64>>());
        let holes: Vec<u64> = (1..=12).collect();
        assert_eq!(got.commitment, fx.commitment(1..=300, &holes));
        assert_eq!(
            got.missing_summary(),
            "12 missing blocks in 1..=300: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10] (+2 more)"
        );
    }

    // REQ-INTEG-004 — Decision: an uncapped list lets a huge hole allocate one entry per missing height
    #[test]
    fn target_report_missing_list_is_capped_count_is_exact() {
        let mut fx = Fx::empty();
        let holes = INTEGRITY_MISSING_LIST_CAP as u64 + 104;
        let tip = holes + 1;
        let b = block_at(fx.g, tip, Hash::ZERO);
        fx.store.put_block_canonical(&b, tip).unwrap();
        fx.hashes.insert(tip, b.hash());

        let got = integrity::integrity_scan(&fx.store, tip, true);
        assert_eq!((got.floor, got.tip, got.missing_count), (1, tip, holes));
        assert_eq!(
            got.missing,
            (1..=INTEGRITY_MISSING_LIST_CAP as u64).collect::<Vec<u64>>()
        );
        assert_eq!(got.commitment, fx.commitment(1..=tip, &[]));
        let first: Vec<String> = (1..=INTEGRITY_LOG_HEIGHTS as u64)
            .map(|h| h.to_string())
            .collect();
        assert_eq!(
            got.missing_summary(),
            format!(
                "{holes} missing blocks in 1..={tip}: [{}] (+{} more)",
                first.join(", "),
                holes - INTEGRITY_LOG_HEIGHTS as u64
            )
        );
    }

    // REQ-INTEG-002 — Decision: a non-zero count or floor <= 300 means snap-synced producers get a new WARN
    #[test]
    fn target_report_snap_synced_non_archive_is_clean() {
        let fx = Fx::snap_synced(300, 600);
        let got = integrity::integrity_scan(&fx.store, 600, false);
        let want = IntegrityReport {
            commitment: fx.commitment(301..=600, &[]),
            ..report(301, 600, 0, vec![])
        };
        assert_eq!(got, want);
    }

    // REQ-INTEG-003 — Decision: a report that does not name 150, or is dirty after one import, breaks repair agreement
    #[test]
    fn target_report_hole_then_clean_after_repair() {
        let fx = Fx::applied(300);
        let archive = fx.archive(300);
        let full = fx.commitment(1..=300, &[]);
        fx.delete(150);

        let first = integrity::integrity_scan(&fx.store, 300, true);
        assert_eq!((first.missing_count, first.missing.clone()), (1, vec![150]));
        assert_eq!(
            backfill_from_archive(archive.path(), &fx.store, Some(&fx.g), u64::MAX),
            Ok(1)
        );
        let after = integrity::integrity_scan(&fx.store, 300, true);
        assert_eq!(
            after,
            IntegrityReport {
                commitment: full,
                ..report(1, 300, 0, vec![])
            }
        );
    }

    // REQ-INTEG-004 — Decision: a format drift here breaks the log lines operators and GS-025 grep for
    #[test]
    fn target_summary_exact_strings() {
        assert_eq!(
            report(1, 300, 2, vec![5, 150]).missing_summary(),
            "2 missing blocks in 1..=300: [5, 150]"
        );
        assert_eq!(
            report(1, 300, 10, (1..=10).collect()).missing_summary(),
            "10 missing blocks in 1..=300: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]"
        );
        assert_eq!(
            report(301, 600, 3, vec![301, 450, 600]).missing_summary(),
            "3 missing blocks in 301..=600: [301, 450, 600]"
        );
        assert_eq!(
            report(1, 2000, 1500, (1..=1000).collect()).missing_summary(),
            "1500 missing blocks in 1..=2000: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10] (+1490 more)"
        );
    }

    // REQ-INTEG-010 — Decision: a second floor function or a caller that bypasses it splits the two integrity paths
    #[test]
    fn target_one_floor_helper_two_callers() {
        assert!(
            INTEGRITY_SRC.contains("fn integrity_scan("),
            "real integrity.rs"
        );
        assert_eq!(INTEGRITY_SRC.matches("fn scan_floor(").count(), 1);
        assert!(
            INTEGRITY_SRC.matches("scan_floor(").count() >= 2,
            "integrity_scan calls scan_floor"
        );
        assert!(
            PERIODIC_SRC.contains("scan_floor("),
            "maybe_run_integrity_check calls scan_floor"
        );
    }
}

// INC-I-253 M2 tests (separate file: this one is near the 800-line test budget).
#[path = "inc_i_253_m2_tests.rs"]
mod m2;
