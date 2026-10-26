//! B3 M2 / R-6 (INC-I-261) — the cargo-visible wrapper of the static shell
//! test that proves gauntlet opt-in flags combine in one call.
//!
// covers: gauntlet.sh README.md test_gauntlet_optin_combine.sh
//
// OUTPUT CONTRACT: fn `bash scripts/test_gauntlet_optin_combine.sh` (no args, reads scripts only)
//   O3 exit code: 0 iff every check passes
//   O6 stdout: one `PASS|FAIL <check>` line per check, then `SUMMARY fails=<n>`
// PATHS: P1 every check passes | P2 at least one check fails
// INPUT PARTITIONS: dispatch = one independent `if` block per opt-in flag (P1) | an if/elif ladder (P2)
//   banner = reads every opt-in flag (P1) | reads CHAOS only (P2)
// MATRIX: O3+O6 x {P1, P2} = 1 test (RED on the ladder, GREEN after the fix)

use std::path::PathBuf;
use std::process::Command;

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test_gauntlet_optin_combine.sh")
}

// REQ-B3-006 — Decision: a failure means two armed opt-in flags in one gauntlet call still run only the first scenario, or the banner hides an armed mode.
#[test]
fn b3_m2_gauntlet_optins_combine() {
    let path = script();
    assert!(path.is_file(), "missing {}", path.display());
    let out = Command::new("bash").arg(&path).output().expect("run bash");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let fails: Vec<&str> = stdout.lines().filter(|l| l.starts_with("FAIL ")).collect();
    assert!(
        stdout.lines().any(|l| l == "SUMMARY fails=0"),
        "shell test failed:\n{}\nstderr:\n{}",
        fails.join("\n"),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(0), "exit code; stdout:\n{stdout}");
}
