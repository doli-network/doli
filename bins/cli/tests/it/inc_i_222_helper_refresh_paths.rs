// INC-I-222 — every install path gives the host the staged-upgrade helper units (TDD RED).
//
// OUTPUT CONTRACT (.claude/protocols/output-contract.md)
// Subject: `doli service refresh-helpers` and its three callers.
// OUTPUTS:
//   O1 exit code of the subcommand       O2 stderr carries no clap error
//   O3 dispatch arm -> cmd_refresh_helpers -> refresh_helper_units_if_root (one template)
//   O4 scripts/install.sh calls it: after the new CLI is installed, root + systemd guard,
//      both networks, failure-tolerant
//   O5 bins/node/postinst (.deb) and O6 bins/node/postinst.sh (.rpm): both networks,
//      failure-tolerant (the scripts run under `set -e`)
// PATHS: A) subcommand parse -> dispatch -> refresh (returns early off Linux/non-root);
//        B) caller scripts (source text is the observable).
// INPUT PARTITIONS:
//   P1 defaults (no --name, no --data-dir)       -> path A
//   P2 explicit --name + --data-dir, testnet     -> path A
//   P3 source text of cmd_service.rs / install.sh / postinst / postinst.sh -> path B
// MATRIX: O1,O2 x P1 -> test 1; O1,O2 x P2 -> test 2; O3 x P3 -> test 3;
//         O4 x P3 -> test 4; O5,O6 x P3 -> test 5.
// NOT COVERED: execution as root on Linux (writes /etc/systemd/system, runs systemctl) — a
// manual/ops check. Off Linux, or when not root, refresh_helper_units_if_root returns before
// any side effect, which is what makes spawning the binary here safe.

use std::path::Path;
use std::process::{Command, Output};

fn doli(args: &[&str], home: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_doli"))
        .args(args)
        .env("HOME", home)
        .output()
        .expect("failed to run doli binary")
}

fn read_src(rel: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

fn line_with<'a>(text: &'a str, needle: &str, file: &str) -> &'a str {
    text.lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("{file} never contains `{needle}`"))
}

const CALL: &str = "service refresh-helpers";

#[test]
fn t1_refresh_helpers_exists_and_is_a_safe_noop_here() {
    let home = tempfile::tempdir().unwrap();
    let out = doli(
        &["--network", "mainnet", "service", "refresh-helpers"],
        home.path(),
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !err.contains("unrecognized subcommand"),
        "O2/P1: `doli service refresh-helpers` must exist. stderr:\n{err}"
    );
    assert!(
        out.status.success(),
        "O1/P1: off Linux or when not root the refresh is a no-op and must exit 0; got {:?}. stderr:\n{err}",
        out.status.code()
    );
}

#[test]
fn t2_refresh_helpers_accepts_name_and_data_dir() {
    let home = tempfile::tempdir().unwrap();
    let dd = home.path().join("data");
    let out = doli(
        &[
            "--network",
            "testnet",
            "service",
            "refresh-helpers",
            "--name",
            "doli-t2",
            "--data-dir",
            dd.to_str().unwrap(),
        ],
        home.path(),
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "O1/P2: --name and --data-dir must parse and the command must exit 0; got {:?}. stderr:\n{err}",
        out.status.code()
    );
}

#[test]
fn t3_dispatch_reaches_the_shared_refresh() {
    let src = read_src("src/cmd_service.rs");
    let at = src
        .find("ServiceCommand::RefreshHelpers")
        .expect("O3: cmd_service.rs has no ServiceCommand::RefreshHelpers arm");
    let arm = &src[at..(at + 200).min(src.len())];
    assert!(
        arm.contains("helper_units::cmd_refresh_helpers("),
        "O3: the arm must delegate to cmd_service_helper_units.rs:\n{arm}"
    );
    let helpers = read_src("src/cmd_service_helper_units.rs");
    let body_at = helpers
        .find("pub fn cmd_refresh_helpers(")
        .expect("O3: cmd_refresh_helpers is missing from cmd_service_helper_units.rs");
    let body = &helpers[body_at..(body_at + 400).min(helpers.len())];
    assert!(
        body.contains("refresh_helper_units_if_root("),
        "O3: cmd_refresh_helpers must call the one shared refresh, not a second unit template:\n{body}"
    );
}

#[test]
fn t4_install_sh_refreshes_after_the_new_cli_is_installed() {
    let file = "../../scripts/install.sh";
    let sh = read_src(file);
    let call_at = sh
        .find(CALL)
        .unwrap_or_else(|| panic!("O4: {file} never runs `{CALL}`"));
    let cli_at = sh
        .find("install -m 755 \"${DIR}/doli\"")
        .expect("O4: install.sh no longer installs the CLI where this test expects");
    assert!(
        call_at > cli_at,
        "O4: the refresh must run the NEW CLI, so it has to come after the CLI install"
    );
    let line = line_with(&sh, CALL, file);
    assert!(
        line.trim_end().ends_with("|| true"),
        "O4: an older CLI without the subcommand must not fail the install: `{line}`"
    );
    let guard_at = sh[..call_at]
        .rfind("\nif [")
        .expect("O4: the call is not inside a guard");
    let guard = sh[guard_at + 1..].lines().next().unwrap();
    assert!(
        guard.contains("id -u") && guard.contains("systemctl"),
        "O4: the call must be guarded by root and systemd: `{guard}`"
    );
    assert!(
        sh[guard_at..call_at].contains("for net in mainnet testnet"),
        "O4: both networks must be refreshed"
    );
}

#[test]
fn t5_both_package_postinst_scripts_refresh() {
    for file in ["../node/postinst", "../node/postinst.sh"] {
        let s = read_src(file);
        let line = line_with(&s, CALL, file);
        assert!(
            line.trim_end().ends_with("|| true"),
            "O5/O6: {file} runs under `set -e`; a failed refresh must not abort the package install: `{line}`"
        );
        assert!(
            s.contains("for net in mainnet testnet"),
            "O5/O6: {file} must refresh both networks"
        );
    }
}
