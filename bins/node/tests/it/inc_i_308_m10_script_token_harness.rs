//! INC-I-308 — cargo-visible wrapper of `scripts/tests/test-rpc-admin-token.sh`.
// OUTPUT CONTRACT: exit code 0 iff every check passes; stdout has one `PASS|FAIL <check>` line per check.

use std::path::PathBuf;
use std::process::Command;

// INC-I-308 P2-a — Decision: a failure means an operator shell or Python helper sends a token the node ignores, or omits it.
#[test]
fn inc_i_308_script_token_harness_passes() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/tests/test-rpc-admin-token.sh");
    assert!(path.is_file(), "missing {}", path.display());
    let out = Command::new("bash").arg(&path).output().expect("run bash");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let fails: Vec<&str> = stdout.lines().filter(|l| l.starts_with("FAIL ")).collect();
    assert!(fails.is_empty(), "shell test failed:\n{}", fails.join("\n"));
    assert_eq!(out.status.code(), Some(0), "exit code; stdout:\n{stdout}");
}
