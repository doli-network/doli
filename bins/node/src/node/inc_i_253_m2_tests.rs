// INC-I-253 M2 — no loop: single-flight, targeted repair, back-off.
// covers: integrity.rs, import.rs, periodic.rs, mod.rs, init.rs, archiver
//
// OUTPUT CONTRACT: fn should_attempt_repair(digest, backoff, scan_no) -> bool
//   O1: none   O2: none (pure)   O4: none
//   O3: false iff backoff.failed == Some((digest, at)) and scan_no <= at + INTEGRITY_REPAIR_BACKOFF_SCANS
// OUTPUT CONTRACT: fn missing_digest(report) -> Hash
//   O3: a function of (missing_count, missing) only; floor, tip and commitment ignored
// OUTPUT CONTRACT: fn InFlightGuard::try_acquire(flag) -> Option<InFlightGuard>; Drop
//   O3: None while another guard holds the flag
//   O4: flag true while held; false after drop on return, early Err, thread panic, JoinError
// OUTPUT CONTRACT: fn scan_and_repair(store, scan_tip, archive_dir, backoff) -> CycleOutcome
//   O1: backoff.scans += 1; backoff.failed = Some((digest, scan_no)) on imported 0 or Err,
//       None on imported > 0 or missing 0
//   O3: .repair Some iff a repair started; .report = re-scan after an import, else the first scan
//   O4: block store gains the imported heights
// PATHS: C0 missing 0, C1 no archive, C2 backed off, C3 imported > 0, C4 imported 0, C5 Err, C6 > cap
// INPUT PARTITIONS:
//   B1 default backoff   B2 same digest at+1..=at+10   B3 at+11   B4 other digest
//   D1 same set, other floor/tip/commitment   D2 other heights   D3 same list, other count
//   G1 held/drop   G2 thread panic   G3 early Err   G4 JoinError in a spawned task
//   S1 complete 300 + archive   S2 hole 150, archive file absent   S3 S2 + hole 200 at cycle 2
//   S4 hole 150 healed, then re-opened   S5 hole 150 in archive   S6 tip 1_100 over 50 blocks
//   S7 manifest genesis wrong   S8 archive None / nonexistent
// MATRIX: should_attempt O3 × B1-B4; digest O3 × D1-D3; guard O3/O4 × G1-G4;
//   scan_and_repair O1/O3/O4 × S1-S8 (cells asserted per test)
// Source tripwires (REQ-INTEG-005/006): periodic.rs text; the planted violation is today's code.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crypto::Hash;
use tempfile::TempDir;

use super::{block_at, Fx, PERIODIC_SRC};
use crate::node::integrity::{
    missing_digest, scan_and_repair, should_attempt_repair, CycleOutcome, InFlightGuard,
    IntegrityReport, RepairBackoff, INTEGRITY_REPAIR_BACKOFF_SCANS, INTEGRITY_REPAIR_MAX_HEIGHTS,
};

fn report(floor: u64, tip: u64, count: u64, missing: Vec<u64>, seed: &[u8]) -> IntegrityReport {
    IntegrityReport {
        floor,
        tip,
        missing_count: count,
        missing,
        commitment: crypto::hash::hash(seed),
    }
}

fn failed_at(digest: Hash, at: u64) -> RepairBackoff {
    RepairBackoff {
        scans: at,
        failed: Some((digest, at)),
    }
}

fn drop_files(dir: &Path, h: u64) {
    for ext in ["block", "blake3"] {
        std::fs::remove_file(dir.join(format!("{h:010}.{ext}"))).unwrap();
    }
}

/// Archive dir holding only a manifest: every listed height is an absent file.
fn manifest_only(genesis: Hash, latest: u64) -> TempDir {
    let dir = TempDir::new().unwrap();
    let json = format!(
        r#"{{"latest_height":{latest},"latest_hash":"{}","genesis_hash":"{genesis}"}}"#,
        Hash::ZERO
    );
    std::fs::write(dir.path().join("manifest.json"), json).unwrap();
    dir
}

fn cycle(fx: &Fx, dir: &Path, backoff: &mut RepairBackoff) -> CycleOutcome {
    scan_and_repair(&fx.store, 300, Some(dir), backoff)
}

fn held(flag: &AtomicBool) -> bool {
    flag.load(Ordering::SeqCst)
}

// REQ-INTEG-007 — Decision: false here means a node with no prior failure never repairs a real hole
#[test]
fn inc_i_253_m2_backoff_without_failure_attempts() {
    let d = crypto::hash::hash(b"s1");
    let b = RepairBackoff::default();
    for scan in [1u64, 2, 11, 12, 1_000] {
        assert!(should_attempt_repair(&d, &b, scan), "scan {scan}");
    }
}

// REQ-INTEG-007 — Decision: true inside the window is the 100-block retry loop; false at at+11 is a permanent give-up
#[test]
fn inc_i_253_m2_backoff_same_digest_skips_ten_then_retries() {
    assert_eq!(INTEGRITY_REPAIR_BACKOFF_SCANS, 10);
    let d = crypto::hash::hash(b"s1");
    let at = 7;
    let b = failed_at(d, at);
    for scan in at + 1..=at + 10 {
        assert!(!should_attempt_repair(&d, &b, scan), "scan {scan}");
    }
    assert!(should_attempt_repair(&d, &b, at + 11), "scan {}", at + 11);
}

// REQ-INTEG-007 — Decision: false here leaves a newly appeared hole unrepaired for up to 10 scans
#[test]
fn inc_i_253_m2_backoff_changed_digest_attempts_at_once() {
    let b = failed_at(crypto::hash::hash(b"s1"), 7);
    let d2 = crypto::hash::hash(b"s2");
    for scan in [8u64, 12, 17] {
        assert!(should_attempt_repair(&d2, &b, scan), "scan {scan}");
    }
}

// REQ-INTEG-007 — Decision: a digest that moves with the tip resets the back-off every scan
#[test]
fn inc_i_253_m2_digest_ignores_floor_tip_commitment() {
    let a = missing_digest(&report(1, 300, 1, vec![150], b"a"));
    let b = missing_digest(&report(1, 400, 1, vec![150], b"b"));
    let c = missing_digest(&report(5, 500, 1, vec![150], b"c"));
    assert_eq!(a, b);
    assert_eq!(a, c);
}

// REQ-INTEG-007 — Decision: an equal digest for a changed set holds back a repair the new hole needs
#[test]
fn inc_i_253_m2_digest_changes_with_the_set() {
    let base = missing_digest(&report(1, 300, 1, vec![150], b"x"));
    let added = missing_digest(&report(1, 300, 2, vec![150, 200], b"x"));
    let moved = missing_digest(&report(1, 300, 1, vec![151], b"x"));
    assert_ne!(base, added);
    assert_ne!(base, moved);
    assert_ne!(added, moved);
    let list: Vec<u64> = (1..=1_000).collect();
    let capped = missing_digest(&report(1, 2_000, 1_000, list.clone(), b"x"));
    let grown = missing_digest(&report(1, 2_000, 1_050, list, b"x"));
    assert_ne!(capped, grown, "count beyond the list cap is part of S");
}

// REQ-INTEG-005 — Decision: Some while held means two scans run and repair concurrently
#[test]
fn inc_i_253_m2_guard_single_flight_until_drop() {
    let flag = Arc::new(AtomicBool::new(false));
    let g = InFlightGuard::try_acquire(&flag).expect("free flag");
    assert!(held(&flag));
    assert!(InFlightGuard::try_acquire(&flag).is_none(), "second scan");
    drop(g);
    assert!(!held(&flag), "released on drop");
    let again = InFlightGuard::try_acquire(&flag);
    assert!(again.is_some(), "next scan acquires");
}

// REQ-INTEG-005 — Decision: a flag left true after a panic disables every later integrity scan
#[test]
fn inc_i_253_m2_guard_released_when_holder_panics() {
    let flag = Arc::new(AtomicBool::new(false));
    let f = flag.clone();
    let joined = std::thread::spawn(move || -> u8 {
        let _g = InFlightGuard::try_acquire(&f).expect("free flag");
        panic!("repair stub panicked");
    })
    .join();
    assert!(joined.is_err(), "holder panicked");
    assert!(!held(&flag), "released by unwinding");
    assert!(InFlightGuard::try_acquire(&flag).is_some());
}

fn guarded_step(flag: &Arc<AtomicBool>, fail: bool) -> Result<u64, String> {
    let _g = InFlightGuard::try_acquire(flag).ok_or("busy")?;
    if fail {
        return Err("repair failed".to_string());
    }
    Ok(1)
}

// REQ-INTEG-005 — Decision: a flag left true after a repair error stops every later scan
#[test]
fn inc_i_253_m2_guard_released_on_error_return() {
    let flag = Arc::new(AtomicBool::new(false));
    assert_eq!(guarded_step(&flag, true), Err("repair failed".to_string()));
    assert!(!held(&flag), "released on the Err path");
    assert_eq!(guarded_step(&flag, false), Ok(1));
    assert!(!held(&flag), "released on the Ok path");
}

// REQ-INTEG-005 — Decision: a guard leaked on JoinError stops all later scans after one repair panic
#[tokio::test]
async fn inc_i_253_m2_guard_released_on_join_error() {
    let flag = Arc::new(AtomicBool::new(false));
    let guard = InFlightGuard::try_acquire(&flag).expect("free flag");
    assert!(
        InFlightGuard::try_acquire(&flag).is_none(),
        "held by the task"
    );
    let task = tokio::spawn(async move {
        let _g = guard;
        tokio::task::spawn_blocking(|| -> u64 { panic!("repair stub panicked") })
            .await
            .is_err()
    });
    assert!(
        task.await.expect("task returns"),
        "spawn_blocking JoinError"
    );
    assert!(!held(&flag), "released after JoinError");
}

// REQ-INTEG-001 — Decision: any repair start on a complete store is the D-1 phantom loop
#[test]
fn inc_i_253_m2_complete_store_starts_no_repair() {
    let fx = Fx::applied(300);
    let archive = fx.archive(300);
    let mut b = RepairBackoff::default();
    for n in 1..=36u64 {
        let out = cycle(&fx, archive.path(), &mut b);
        assert!(out.repair.is_none(), "cycle {n} started a repair");
        assert_eq!(out.report.missing_count, 0, "cycle {n}");
    }
    assert_eq!(
        b,
        RepairBackoff {
            scans: 36,
            failed: None
        }
    );
}

// REQ-INTEG-007 — Decision: more than 4 starts in 36 scans is the 100-block retry loop on a hole the archive lacks
#[test]
fn inc_i_253_m2_unrepairable_hole_backs_off_ten_scans() {
    let fx = Fx::applied(300);
    let archive = fx.archive(300);
    fx.delete(150);
    drop_files(archive.path(), 150);
    let mut b = RepairBackoff::default();
    let mut started = Vec::new();
    let mut last = None;
    for n in 1..=36u64 {
        let out = cycle(&fx, archive.path(), &mut b);
        assert_eq!(out.report.missing, vec![150], "cycle {n} report");
        assert_eq!(out.report.missing_count, 1, "cycle {n}");
        if let Some(r) = out.repair {
            let r = r.expect("targeted repair is Ok");
            assert_eq!(
                (r.imported, r.missing_files, r.heights_visited),
                (0, 1, 1),
                "cycle {n}"
            );
            started.push(n);
        }
        last = Some(out.report);
    }
    assert_eq!(started, vec![1, 12, 23, 34]);
    let d = missing_digest(&last.unwrap());
    assert_eq!(b.failed, Some((d, 34)));
    assert_eq!(b.scans, 36);
}

// REQ-INTEG-007 — Decision: no start at cycle 2 leaves a new hole unrepaired for 10 scans
#[test]
fn inc_i_253_m2_changed_set_repairs_during_backoff() {
    let fx = Fx::applied(300);
    let archive = fx.archive(300);
    fx.delete(150);
    drop_files(archive.path(), 150);
    let mut b = RepairBackoff::default();
    assert!(
        cycle(&fx, archive.path(), &mut b).repair.is_some(),
        "cycle 1"
    );
    fx.delete(200);
    drop_files(archive.path(), 200);

    let out = cycle(&fx, archive.path(), &mut b);

    assert_eq!(out.report.missing, vec![150, 200]);
    let r = out.repair.expect("cycle 2 starts: S changed").expect("Ok");
    assert_eq!((r.heights_visited, r.imported), (2, 0));
    assert_eq!(b.failed, Some((missing_digest(&out.report), 2)));
    let third = cycle(&fx, archive.path(), &mut b);
    assert!(third.repair.is_none(), "cycle 3 backs off the new S");
}

// REQ-INTEG-007 — Decision: a back-off that survives a clean scan delays the repair of a recurring hole
#[test]
fn inc_i_253_m2_clean_scan_clears_backoff() {
    let fx = Fx::applied(300);
    let archive = manifest_only(fx.g, 300);
    fx.delete(150);
    let mut b = RepairBackoff::default();
    assert!(
        cycle(&fx, archive.path(), &mut b).repair.is_some(),
        "cycle 1"
    );
    assert!(b.failed.is_some(), "imported 0 sets the back-off");

    let healed = block_at(fx.g, 150, fx.hashes[&149]);
    assert_eq!(healed.hash(), fx.hashes[&150], "precondition: same block");
    fx.store.put_block_canonical(&healed, 150).unwrap();
    let out = cycle(&fx, archive.path(), &mut b);
    assert!(out.repair.is_none(), "cycle 2 is clean");
    assert_eq!(out.report.missing_count, 0);
    assert_eq!(b.failed, None, "clean scan clears the back-off");

    fx.delete(150);
    let out = cycle(&fx, archive.path(), &mut b);
    assert!(out.repair.is_some(), "recurring hole repairs at once");
}

// REQ-INTEG-006 — Decision: heights_visited > 1 is the full-range pass; failed Some after an import delays the next repair
#[test]
fn inc_i_253_m2_repairable_hole_one_targeted_pass() {
    let fx = Fx::applied(300);
    let archive = fx.archive(300);
    fx.delete(150);
    let mut b = RepairBackoff::default();

    let out = cycle(&fx, archive.path(), &mut b);

    let r = out.repair.expect("repair starts").expect("Ok");
    assert_eq!(
        (r.imported, r.heights_visited, r.files_read, r.rejected),
        (1, 1, 1, 0)
    );
    assert_eq!(out.report.missing_count, 0, "re-scan after the import");
    assert_eq!(out.report.commitment, fx.commitment(1..=300, &[]));
    assert_eq!(
        b,
        RepairBackoff {
            scans: 1,
            failed: None
        }
    );
    fx.assert_served(1..=300);
    assert!(cycle(&fx, archive.path(), &mut b).repair.is_none());
}

// REQ-INTEG-006 — Decision: more than 1_000 heights per pass makes one cycle's cost unbounded
#[test]
fn inc_i_253_m2_repair_caps_heights_per_pass() {
    assert_eq!(INTEGRITY_REPAIR_MAX_HEIGHTS, 1_000);
    let fx = Fx::applied(50);
    let archive = manifest_only(fx.g, 1_100);
    let mut b = RepairBackoff::default();

    let out = scan_and_repair(&fx.store, 1_100, Some(archive.path()), &mut b);

    assert_eq!(out.report.missing_count, 1_050);
    let r = out.repair.expect("repair starts").expect("Ok");
    assert_eq!((r.heights_visited, r.missing_files), (1_000, 1_000));
}

// REQ-INTEG-007 — Decision: an Err repair without back-off retries a wrong archive every 100 blocks
#[test]
fn inc_i_253_m2_repair_err_backs_off() {
    let fx = Fx::applied(300);
    let archive = manifest_only(crypto::hash::hash(b"inc-i-253-other-chain"), 300);
    fx.delete(150);
    let mut b = RepairBackoff::default();

    let out = cycle(&fx, archive.path(), &mut b);

    assert!(
        matches!(out.repair, Some(Err(_))),
        "observed {:?}",
        out.repair
    );
    assert_eq!(b.failed, Some((missing_digest(&out.report), 1)));
    for n in 2..=11u64 {
        let out = cycle(&fx, archive.path(), &mut b);
        assert!(out.repair.is_none(), "cycle {n} backs off");
    }
    let out = cycle(&fx, archive.path(), &mut b);
    assert!(out.repair.is_some(), "cycle 12 retries");
}

// REQ-INTEG-006 — Decision: a repair start without an archive is a pass that can never import
#[test]
fn inc_i_253_m2_no_archive_starts_no_repair() {
    let fx = Fx::applied(300);
    fx.delete(150);
    let gone = TempDir::new().unwrap().path().to_path_buf();
    assert!(!gone.exists(), "precondition: nonexistent dir");
    let mut b = RepairBackoff::default();
    for dir in [None, Some(gone.as_path())] {
        let out = scan_and_repair(&fx.store, 300, dir, &mut b);
        assert!(out.repair.is_none(), "archive {dir:?}");
        assert_eq!(out.report.missing, vec![150], "archive {dir:?}");
    }
    assert_eq!(b.scans, 2);
}

// REQ-INTEG-005 — Decision: no try_acquire or scan_and_repair in periodic.rs means overlapping scans still stack
#[test]
fn inc_i_253_m2_periodic_uses_guard_and_scan_and_repair() {
    assert!(PERIODIC_SRC.contains("[INTEGRITY]"), "real periodic.rs");
    for needle in ["InFlightGuard::try_acquire", "scan_and_repair("] {
        assert!(
            PERIODIC_SRC.contains(needle),
            "periodic.rs lacks `{needle}`"
        );
    }
}

// REQ-INTEG-006 — Decision: a full-range backfill left in periodic.rs re-reads the whole archive per hole
#[test]
fn inc_i_253_m2_periodic_has_no_full_range_backfill() {
    assert!(PERIODIC_SRC.contains("[INTEGRITY]"), "real periodic.rs");
    let n = PERIODIC_SRC.matches("backfill_from_archive(").count();
    assert_eq!(
        n, 0,
        "observed {n} full-range backfill calls in periodic.rs"
    );
}

// REQ-INTEG-006/007 — Decision: outcome metric only (asserts nothing); feeds .claude/scripts/inc-i-253-m2-probe.sh
#[test]
fn inc_i_253_m2_outcome_probe() {
    let complete = Fx::applied(300);
    let complete_archive = complete.archive(300);
    let hole = Fx::applied(300);
    let hole_archive = hole.archive(300);
    hole.delete(150);
    drop_files(hole_archive.path(), 150);

    let (mut cb, mut hb) = (RepairBackoff::default(), RepairBackoff::default());
    let (mut complete_passes, mut hole_passes, mut hole_visited) = (0u64, 0u64, 0u64);
    for _scan in 0..36 {
        if cycle(&complete, complete_archive.path(), &mut cb)
            .repair
            .is_some()
        {
            complete_passes += 1;
        }
        if let Some(r) = cycle(&hole, hole_archive.path(), &mut hb).repair {
            hole_passes += 1;
            hole_visited = hole_visited.max(r.map(|r| r.heights_visited).unwrap_or(0));
        }
    }
    println!(
        "INC253_M2_OUTCOME complete_passes_per_hour={:.1} hole_passes_per_hour={:.1} hole_heights_per_pass={}",
        complete_passes as f64 / 10.0,
        hole_passes as f64 / 10.0,
        hole_visited
    );
}
