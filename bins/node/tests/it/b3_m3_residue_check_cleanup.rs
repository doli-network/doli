//! B3 M3 / R-5 (INC-I-261) — `doli-node check-producer-set` leaves no scratch
//! copy behind on any exit, and on start removes scratch dirs of dead check
//! processes under the same temp parent.
//!
// covers: producer_set_check main
//
// OUTPUT CONTRACT: fn `doli-node --network devnet --data-dir <D> --log-level error check-producer-set`
//   O1 exit code: 0 EQUAL | 3 NOT-CHECKABLE | 1 error
//   O2 TMPDIR (std::env::temp_dir()): no `doli-check-producer-set-*` dir of this run left; a stale
//      `doli-check-producer-set-<pid>-<nanos>` dir whose <pid> is not a live process is removed;
//      a dir of a live <pid> and a name that does not parse are kept with their contents
//   O3 data dir: every file byte-identical, none added or removed
// PATHS: P-ERR replay done, persisted best hash differs (bail after the scratch exists)
//        | P-HOLE block store has a height gap (NOT-CHECKABLE after the scratch exists)
//        | P-EQ clean store with a stale dead-pid dir | P-EQ clean store with live-pid / unparsable dirs
// INPUT PARTITIONS:
//   pre-existing TMPDIR entries: none | dead pid (reaped `true` child) | own pid (alive) | `notapid` suffix
// MATRIX:
//   O1,O2,O3 x P-ERR  -> b3_m3_failed_check_leaves_no_scratch_and_data_untouched
//   O1,O2,O3 x P-HOLE -> b3_m3_holed_store_not_checkable_leaves_no_scratch
//   O1,O2 x P-EQ x dead pid -> b3_m3_stale_scratch_swept_on_start
//   O1,O2 x P-EQ x {own pid, notapid} -> b3_m3_live_pid_scratch_not_swept

use std::path::{Path, PathBuf};
use std::process::Command;

use storage::{BlockStore, StateDb};

use super::b3_m2_residue_check::{
    assert_outcome, assert_untouched, build, run_check, tree, Fx, TIP,
};

const PREFIX: &str = "doli-check-producer-set-";

/// Pid of a reaped child process: no live process holds it.
fn dead_pid() -> u32 {
    let mut child = Command::new("true").spawn().expect("FIXTURE: spawn `true`");
    let pid = child.id();
    child.wait().expect("FIXTURE: reap `true`");
    // SAFETY: signal 0 only probes for existence.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    assert_eq!(
        (rc, std::io::Error::last_os_error().raw_os_error()),
        (-1, Some(libc::ESRCH)),
        "PRECONDITION: pid {pid} is not a live process"
    );
    pid
}

/// `<TMPDIR>/<name>/leftover` with known content.
fn plant(fx: &Fx, name: &str) -> PathBuf {
    let dir = fx.scratch_root.join(name);
    std::fs::create_dir(&dir).expect("FIXTURE: create planted dir");
    std::fs::write(dir.join("leftover"), b"chain-size copy").expect("FIXTURE: planted file");
    dir
}

fn scratch_entries(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root)
        .expect("read TMPDIR")
        .map(|e| {
            e.expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|n| n.starts_with(PREFIX))
        .collect();
    names.sort();
    names
}

// REQ-B3-005 — Decision: a failure means a check that errors after its replay leaves a chain-size scratch copy under the temp dir or touches the node's data.
#[tokio::test]
async fn b3_m3_failed_check_leaves_no_scratch_and_data_untouched() {
    let fx = build().await;
    {
        let db = StateDb::open(&fx.data.join("state_db")).expect("FIXTURE: open state_db");
        let mut cs = db.get_chain_state().expect("FIXTURE: chain state");
        cs.best_hash = crypto::Hash::from_bytes([0xEE; 32]);
        db.put_chain_state(&cs)
            .expect("FIXTURE: persist foreign best hash");
    }
    let before = tree(&fx.data);
    let r = run_check(&fx);
    assert_eq!(
        r.code,
        Some(1),
        "a failed check exits 1; stdout:\n{}\nstderr:\n{}",
        r.stdout,
        r.stderr
    );
    assert!(
        r.stderr
            .contains(&format!("replayed block at height {TIP}")),
        "PRECONDITION: the check failed after its replay; stderr:\n{}",
        r.stderr
    );
    assert_untouched(&fx, &before, "failed check");
}

// REQ-B3-005 — Decision: a failure means a holed store gets a verdict, or the not-checkable exit leaks its scratch dir or touches the data.
#[tokio::test]
async fn b3_m3_holed_store_not_checkable_leaves_no_scratch() {
    let fx = build().await;
    {
        let bs = BlockStore::open(&fx.data.join("blocks")).expect("FIXTURE: open blocks");
        assert!(
            bs.delete_block_by_height(2).expect("FIXTURE: delete h=2"),
            "FIXTURE: height 2 existed"
        );
    }
    let before = tree(&fx.data);
    let r = run_check(&fx);
    assert_outcome(
        &r,
        "NOT-CHECKABLE",
        3,
        &[("reason", "holed_block_store".to_string())],
    );
    assert_untouched(&fx, &before, "holed store");
}

// REQ-B3-005 — Decision: a failure means a scratch copy left by an aborted check (panic=abort, SIGKILL) is never removed and the temp disk fills one chain copy per abort.
#[tokio::test]
async fn b3_m3_stale_scratch_swept_on_start() {
    let fx = build().await;
    let stale = plant(&fx, &format!("{PREFIX}{}-1", dead_pid()));
    let r = run_check(&fx);
    assert_outcome(&r, "EQUAL", 0, &[]);
    let left = scratch_entries(&fx.scratch_root);
    eprintln!("METRIC stale_scratch_dirs_left={}", left.len());
    assert!(
        !stale.exists() && left.is_empty(),
        "the stale dead-pid scratch dir must be removed on start; left under TMPDIR: {left:?}"
    );
}

// REQ-B3-005 — Decision: a failure means the sweep deletes the scratch dir of a check still running, or a dir it cannot prove is a check scratch dir.
#[tokio::test]
async fn b3_m3_live_pid_scratch_not_swept() {
    let fx = build().await;
    let live = plant(&fx, &format!("{PREFIX}{}-1", std::process::id()));
    let odd = plant(&fx, &format!("{PREFIX}notapid-1"));
    let r = run_check(&fx);
    assert_outcome(&r, "EQUAL", 0, &[]);
    for dir in [&live, &odd] {
        assert_eq!(
            std::fs::read(dir.join("leftover")).ok().as_deref(),
            Some(&b"chain-size copy"[..]),
            "{} must survive the sweep with its contents",
            dir.display()
        );
    }
    let mut want = vec![
        live.file_name().unwrap().to_string_lossy().into_owned(),
        odd.file_name().unwrap().to_string_lossy().into_owned(),
    ];
    want.sort();
    assert_eq!(
        scratch_entries(&fx.scratch_root),
        want,
        "only the planted dirs remain; the run's own scratch is gone"
    );
}
