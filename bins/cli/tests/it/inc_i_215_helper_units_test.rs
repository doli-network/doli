// INC-I-215 M4 — privileged helper units for the staged upgrade (TDD RED).
//
// The node runs sandboxed (`NoNewPrivileges`, `ProtectSystem=full`) and cannot write
// /usr/bin, so auto-update is dead on every `doli service install` host. M1/M2/M3 made the
// node STAGE a verified release into `{data_dir}/updates/` and drop a `ready` marker. M4
// installs the root-side trigger: a systemd `.path` unit that watches that marker and a
// oneshot `.service` that runs `doli upgrade --from-staged`. Without M4 the staging is a
// write-only dead end.
//
// ===========================================================================
// OUTPUT CONTRACT (.claude/protocols/output-contract.md — ENUMERATION-CHECKLIST)
// ===========================================================================
// Subject: the new pure module `bins/cli/src/cmd_service_helper_units.rs`, plus the
// three wiring call sites in `cmd_service.rs` / `cmd_upgrade.rs`.
//
// --- ENUMERATED OUTPUTS, per function under test -------------------------------------
// fn helper_unit_names(service_name) -> (String, String)
//   O1 return.0 = "{service_name}-upgrade.path"
//   O1 return.1 = "{service_name}-upgrade.service"
//   (no params are mutable, no receiver, no store writes, no stdout)
// fn helper_path_unit_path(service_name) -> PathBuf
//   O2 return = /etc/systemd/system/{service_name}-upgrade.path
// fn helper_service_unit_path(service_name) -> PathBuf
//   O3 return = /etc/systemd/system/{service_name}-upgrade.service
// fn helper_path_unit_content(service_name, data_dir) -> String
//   O4 return = unit text. Facets asserted separately because each is independently
//   breakable: O4a [Unit]+Description=, O4b exact "PathExists={data_dir}/updates/ready",
//   O4c exact "Unit={service_name}-upgrade.service", O4d [Install]+WantedBy=multi-user.target.
// fn helper_service_unit_content(service_name, network, data_dir, doli_cli) -> String
//   O5 return = unit text. O5a [Unit]+Description=, O5b Type=oneshot,
//   O5c the exact ExecStart line (flag ORDER included — `--network` is global and precedes
//   the subcommand), O5d ABSENCE of User=/NoNewPrivileges/ProtectSystem/ReadWritePaths,
//   O5e ABSENCE of an [Install] section.
// fn which_doli_cli() -> PathBuf
//   O6 return = a non-empty path. Host-dependent: only non-emptiness is contracted.
// fn install_helper_units(..) -> Result<()>
//   O7 file bytes at O2, O8 file bytes at O3, O9 side effects (daemon-reload,
//   enable --now the .path). NOT EXECUTED — see DELIBERATELY-NOT-COVERED.
// fn remove_helper_units(..) -> Result<()>
//   O10 removal of O2/O3, O11 side effect (disable --now the .path). NOT EXECUTED.
// fn refresh_helper_units_if_root(..) -> ()
//   O12 conditional file writes, O13 stdout line, O14 systemd side effects. NOT EXECUTED.
// Wiring (source text is the observable):
//   O15 `install_systemd` body calls install_helper_units
//   O16 `cmd_uninstall` body calls remove_helper_units
//   O17 `cmd_upgrade` body calls refresh_helper_units_if_root
//   O18 file line counts (module budget, Rule 19 / REQ-215-016)
//
// --- PATHS (branches) ------------------------------------------------------------------
// The six content/name/path functions are PURE and BRANCHLESS: one path each, output is a
// format! of the parameters. That branchlessness is itself the TB-2 control and is asserted
// structurally by test 6. install/remove/refresh branch on EUID, target OS, and file
// presence — those branches are asserted structurally, never executed.
//
// --- INPUT PARTITIONS ------------------------------------------------------------------
// P1 default-named host:  service_name = "doli-mainnet"  (network "mainnet")
// P2 --name-customised host: service_name = "doli-b"     (network "mainnet")
// P3 non-default data_dir: a tempdir path (proves data_dir is interpolated, not hardcoded)
// P4 repeat render (idempotence): the same inputs rendered twice
// P5 source text of cmd_service.rs / cmd_upgrade.rs / cmd_service_helper_units.rs
//
// --- MATRIX (outputs x partitions) ----------------------------------------------------
//   O1  x P1,P2   -> tests 1, 5
//   O2  x P1,P2   -> tests 1, 5
//   O3  x P1,P2   -> tests 1, 5
//   O4a x P1      -> test 2      O4b x P1,P3 -> test 2      O4c x P1 -> test 2
//   O4d x P1      -> test 2
//   O5a x P1      -> test 3      O5b x P1    -> test 3      O5c x P1,P3 -> test 3
//   O5d x P1      -> test 4      O5e x P1    -> test 4
//   O6  x P1      -> test 1 (non-emptiness only)
//   O7,O8 x P4    -> test 10 (bytes rendered+written by the TEST, not by install_helper_units)
//   O15 x P5      -> test 7      O16 x P5 -> test 8      O17 x P5 -> test 9
//   O18 x P5      -> test 11
//   TB-2 (no staged content may reach a root command line) x P5 -> test 6
//
// --- DELIBERATELY NOT COVERED (and why) ------------------------------------------------
// O9, O11, O12, O13, O14 — the shell-out and /etc side effects of install_helper_units,
// remove_helper_units and refresh_helper_units_if_root. Executing them would write
// /etc/systemd/system and invoke systemctl. This dev host also runs an 18-node launchd
// testnet where `upgrade_restart.rs` kickstarts every label containing "doli", so a test
// that spawns the CLI could bounce the live fleet. These outputs are covered
// STRUCTURALLY (tests 7, 8, 9) and their runtime behaviour is a manual/ops check.
// O6 exact value — host-dependent (`which doli`); contracting it would make the suite
// pass or fail on PATH, which is not a property of the change.
// ===========================================================================

use std::path::{Path, PathBuf};

use doli_cli::cmd_service_helper_units::{
    helper_path_unit_content, helper_path_unit_path, helper_service_unit_content,
    helper_service_unit_path, helper_unit_names, which_doli_cli,
};

// ---------------------------------------------------------------------------
// Local structural-read helpers (reimplemented, not imported — see
// bins/cli/tests/logrotate_dropin_test.rs for the reference shape).
//
// Sources are read at RUNTIME rather than via include_str! so the RED state is
// behavioural: a stale compiled-in snapshot would keep asserting yesterday's file.
// CARGO_MANIFEST_DIR anchors the path so the CWD does not matter.
// ---------------------------------------------------------------------------

fn read_src(rel: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// Source slice of a top-level fn body: from `sig` to the next top-level item.
fn fn_body<'a>(src: &'a str, sig: &str) -> &'a str {
    let start = src
        .find(sig)
        .unwrap_or_else(|| panic!("fn not found: {sig}"));
    let rest = &src[start..];
    let after_sig = &rest[sig.len()..];
    let end = [
        "\nfn ",
        "\npub fn ",
        "\npub(crate) fn ",
        "\nasync fn ",
        "\npub async fn ",
        "\npub(crate) async fn ",
        "\n#[cfg(test)]",
    ]
    .iter()
    .filter_map(|m| after_sig.find(m))
    .min();
    match end {
        Some(i) => &rest[..sig.len() + i],
        None => rest,
    }
}

/// Exact-line match. Bare `contains` is unsafe here: a prior INC-I-215 milestone found
/// that `contains("ready")` is true of any prose containing "already", so every unit-file
/// line is matched whole and trimmed.
fn has_exact_line(text: &str, expected: &str) -> bool {
    text.lines().any(|l| l.trim() == expected)
}

fn assert_line(text: &str, expected: &str, what: &str) {
    assert!(
        has_exact_line(text, expected),
        "{what}: expected the exact line\n  {expected}\nin unit:\n{text}"
    );
}

fn line_count(rel: &str) -> usize {
    read_src(rel).lines().count()
}

const SERVICE_A: &str = "doli-mainnet";
const SERVICE_B: &str = "doli-b";
const NETWORK: &str = "mainnet";

fn fixture_data_dir() -> PathBuf {
    PathBuf::from("/var/lib/doli/mainnet")
}

fn fixture_cli() -> PathBuf {
    PathBuf::from("/usr/bin/doli")
}

/// The one root command line this milestone may ever produce. `--network` is a GLOBAL
/// clap flag on the `Cli` struct, so it precedes the `upgrade` subcommand; `--yes` is
/// mandatory because a oneshot has no tty to answer a prompt.
fn expected_exec_start(cli: &str, network: &str, data_dir: &str, service_name: &str) -> String {
    format!(
        "ExecStart={cli} --network {network} upgrade \
         --from-staged {data_dir}/updates --data-dir {data_dir} \
         --service {service_name} --yes"
    )
}

// ===========================================================================
// 1
// ===========================================================================
// REQ-215-011 — Decision: a failure means the pure helper API the whole milestone is built
// on is absent or misshapen, so nothing downstream (install wiring, refresh, uninstall)
// can be trusted and the developer must fix the module surface before anything else.
// [O1,O2,O3,O4,O5,O6 x P1]
#[test]
fn helper_unit_content_and_path_helpers_are_defined() {
    let data_dir = fixture_data_dir();
    let cli = fixture_cli();

    let (path_unit, service_unit) = helper_unit_names(SERVICE_A);
    assert_eq!(path_unit, "doli-mainnet-upgrade.path");
    assert_eq!(service_unit, "doli-mainnet-upgrade.service");

    assert_eq!(
        helper_path_unit_path(SERVICE_A),
        PathBuf::from("/etc/systemd/system/doli-mainnet-upgrade.path"),
        "the .path unit must live in /etc/systemd/system so systemd loads it"
    );
    assert_eq!(
        helper_service_unit_path(SERVICE_A),
        PathBuf::from("/etc/systemd/system/doli-mainnet-upgrade.service"),
        "the .service unit must live in /etc/systemd/system so the .path can trigger it"
    );

    let path_unit_text = helper_path_unit_content(SERVICE_A, &data_dir);
    let service_unit_text = helper_service_unit_content(SERVICE_A, NETWORK, &data_dir, &cli);
    assert!(
        path_unit_text.contains("[Unit]"),
        ".path unit needs a [Unit] section"
    );
    assert!(
        service_unit_text.contains("[Unit]"),
        ".service unit needs a [Unit] section"
    );

    // O6: host-dependent value, so only non-emptiness is contracted. An empty path would
    // render an ExecStart that starts with a space and systemd would refuse the unit.
    let cli_path = which_doli_cli();
    assert!(
        !cli_path.as_os_str().is_empty(),
        "which_doli_cli must never return an empty path"
    );
}

// ===========================================================================
// 2
// ===========================================================================
// REQ-215-011 — Decision: a failure means the .path unit does not watch the marker the
// node actually writes (or names the wrong triggered unit), so a staged release sits
// forever and M2's node preflight keeps WARNing — the whole feature is dead on arrival.
// [O4a,O4b,O4c,O4d x P1,P3]
#[test]
fn path_unit_watches_the_ready_marker_and_names_the_service_unit() {
    let data_dir = fixture_data_dir();
    let unit = helper_path_unit_content(SERVICE_A, &data_dir);

    assert!(unit.contains("[Unit]"), ".path unit needs [Unit]:\n{unit}");
    assert!(
        unit.lines()
            .any(|l| l.trim_start().starts_with("Description=")),
        ".path unit needs a Description= so `systemctl list-units` is readable:\n{unit}"
    );
    assert!(unit.contains("[Path]"), ".path unit needs [Path]:\n{unit}");

    // The exact literal M2's node-side preflight greps /etc/systemd/system/*.path for.
    // An inexact string (trailing slash, a different subdir) means the node keeps WARNing
    // even though the helper is installed.
    assert_line(
        &unit,
        "PathExists=/var/lib/doli/mainnet/updates/ready",
        "path unit must watch the exact staged ready marker",
    );
    assert_line(
        &unit,
        "Unit=doli-mainnet-upgrade.service",
        "path unit must name the helper .service it triggers",
    );

    assert!(
        unit.contains("[Install]"),
        ".path unit needs [Install] or `systemctl enable` has nothing to link:\n{unit}"
    );
    assert_line(
        &unit,
        "WantedBy=multi-user.target",
        "path unit must be wanted by multi-user.target so it survives reboot",
    );

    // P3: a non-default data_dir must be interpolated, not hardcoded.
    let alt = PathBuf::from("/srv/doli/testnet-data");
    let alt_unit = helper_path_unit_content(SERVICE_A, &alt);
    assert_line(
        &alt_unit,
        "PathExists=/srv/doli/testnet-data/updates/ready",
        "the watched marker must follow the given data_dir",
    );
}

// ===========================================================================
// 3
// ===========================================================================
// REQ-215-011 — Decision: a failure means the root oneshot runs the wrong command line —
// wrong flag order (`--network` is GLOBAL and must precede the `upgrade` subcommand, M3's
// shipped shape), wrong staging dir, or a missing `--yes` that hangs the unit on a prompt.
// Any of those installs nothing and the bug survives the fix.
// [O5a,O5b,O5c x P1,P3]
#[test]
fn service_unit_is_a_root_oneshot_running_upgrade_from_staged_with_the_network() {
    let data_dir = fixture_data_dir();
    let cli = fixture_cli();
    let unit = helper_service_unit_content(SERVICE_A, NETWORK, &data_dir, &cli);

    assert!(
        unit.contains("[Unit]"),
        ".service unit needs [Unit]:\n{unit}"
    );
    assert!(
        unit.lines()
            .any(|l| l.trim_start().starts_with("Description=")),
        ".service unit needs a Description=:\n{unit}"
    );
    assert!(
        unit.contains("[Service]"),
        ".service unit needs [Service]:\n{unit}"
    );
    assert_line(
        &unit,
        "Type=oneshot",
        "the helper must be a oneshot, not a long-lived service",
    );

    assert_line(
        &unit,
        &expected_exec_start("/usr/bin/doli", NETWORK, "/var/lib/doli/mainnet", SERVICE_A),
        "ExecStart must be M3's exact shipped invocation, global --network first",
    );

    // P3: staging dir, data dir and service name all follow the parameters.
    let alt_dir = PathBuf::from("/srv/doli/testnet-data");
    let alt_cli = PathBuf::from("/usr/local/bin/doli");
    let alt = helper_service_unit_content(SERVICE_B, "testnet", &alt_dir, &alt_cli);
    assert_line(
        &alt,
        &expected_exec_start(
            "/usr/local/bin/doli",
            "testnet",
            "/srv/doli/testnet-data",
            SERVICE_B,
        ),
        "every ExecStart term must follow its parameter, none may be hardcoded",
    );
}

// ===========================================================================
// 4
// ===========================================================================
// REQ-215-011 — Decision: a failure means the helper inherited the node unit's sandbox,
// which is EXACTLY the INC-I-215 bug being fixed — the oneshot would be unable to write
// /usr/bin and the whole milestone would ship an inert unit that reproduces the defect.
// An [Install] section would also let an operator boot-enable a unit that must only ever
// be path-triggered. [O5d,O5e x P1]
#[test]
fn service_unit_carries_no_sandbox_directives_from_the_node_unit() {
    let data_dir = fixture_data_dir();
    let cli = fixture_cli();
    let unit = helper_service_unit_content(SERVICE_A, NETWORK, &data_dir, &cli);

    for banned in [
        "User=",
        "Group=",
        "NoNewPrivileges",
        "ProtectSystem",
        "ReadWritePaths",
        "ProtectHome",
        "PrivateTmp",
    ] {
        let offender = unit
            .lines()
            .find(|l| l.trim_start().starts_with(banned))
            .map(|l| l.to_string());
        assert!(
            offender.is_none(),
            "helper .service must run unrestricted as root (it installs into /usr/bin); \
             found sandbox directive {banned:?} in line {:?}\n{unit}",
            offender.unwrap_or_default()
        );
    }

    assert!(
        !unit.contains("[Install]"),
        "helper .service must have NO [Install] — it is triggered by the .path unit, \
         never enabled or booted:\n{unit}"
    );
}

// ===========================================================================
// 5
// ===========================================================================
// REQ-215-011 — Decision: a failure means two nodes on one host share a unit name, so
// installing the second silently overwrites the first's helper and one node's staged
// upgrades are installed with the other node's data_dir. Multi-node hosts are the norm on
// this fleet, so this is a data-destroying collision, not a cosmetic one. [O1,O2,O3 x P1,P2]
#[test]
fn unit_names_derive_from_the_resolved_service_name_so_multi_node_hosts_do_not_collide() {
    let (path_a, service_a) = helper_unit_names(SERVICE_A);
    let (path_b, service_b) = helper_unit_names(SERVICE_B);

    assert_eq!(path_a, "doli-mainnet-upgrade.path");
    assert_eq!(service_a, "doli-mainnet-upgrade.service");
    assert_eq!(path_b, "doli-b-upgrade.path");
    assert_eq!(service_b, "doli-b-upgrade.service");

    // The two pairs must be disjoint — no name shared between hosts.
    let a = [path_a.as_str(), service_a.as_str()];
    let b = [path_b.as_str(), service_b.as_str()];
    for x in a {
        assert!(
            !b.contains(&x),
            "unit name {x:?} is shared between service names {SERVICE_A:?} and {SERVICE_B:?}"
        );
    }

    assert_ne!(
        helper_path_unit_path(SERVICE_A),
        helper_path_unit_path(SERVICE_B),
        ".path unit files must not collide across nodes on one host"
    );
    assert_ne!(
        helper_service_unit_path(SERVICE_A),
        helper_service_unit_path(SERVICE_B),
        ".service unit files must not collide across nodes on one host"
    );
}

// ===========================================================================
// 6
// ===========================================================================
// TB-2 — Decision: a failure means something read at render time from disk or the
// environment can reach a ROOT command line. The staging directory's contents are
// explicitly untrusted (a compromised node can write anything there); the design's whole
// value is that root re-verifies rather than trusts. A content function that reads a file
// or shells out would turn the marker into an injection channel, which is a privilege
// escalation, not a bug. [TB-2 x P5]
#[test]
fn exec_start_interpolates_only_render_time_values() {
    let src = read_src("src/cmd_service_helper_units.rs");

    for sig in [
        "pub fn helper_path_unit_content(",
        "pub fn helper_service_unit_content(",
    ] {
        let body = fn_body(&src, sig);
        for banned in [
            "fs::",
            "read_to_string",
            "Command::new",
            "env::var",
            "env!",
            "std::env",
        ] {
            assert!(
                !body.contains(banned),
                "{sig} must be a pure format! of its parameters — found {banned:?}. \
                 No value read from the staging dir or the environment may reach a root \
                 command line (TB-2).\n{body}"
            );
        }
    }
}

// ===========================================================================
// 7
// ===========================================================================
// REQ-215-011 — Decision: a failure means the helper module exists but nothing installs
// it, so every freshly installed host still has no privileged path to /usr/bin. This is
// the wiring the analysis calls out as the difference between a written module and a
// working fix. [O15 x P5]
#[test]
fn install_systemd_writes_and_enables_the_path_unit() {
    let service_src = read_src("src/cmd_service.rs");
    let install_body = fn_body(&service_src, "fn install_systemd(");
    assert!(
        install_body.contains("install_helper_units("),
        "install_systemd must call install_helper_units(..) after writing the node unit"
    );

    let helper_src = read_src("src/cmd_service_helper_units.rs");
    let body = fn_body(&helper_src, "pub fn install_helper_units(");

    for needed in [
        "helper_path_unit_path",
        "helper_service_unit_path",
        "helper_path_unit_content",
        "helper_service_unit_content",
        "daemon-reload",
        "enable",
        "--now",
    ] {
        assert!(
            body.contains(needed),
            "install_helper_units must reference {needed:?} (write both units, reload the \
             daemon, then enable --now the watcher)\n{body}"
        );
    }

    // The .path is the only unit that may be enabled. The .service has no [Install]
    // (test 4), so enabling it is meaningless; hard-coding its NAME into an enable call
    // is the mistake this guards against.
    assert!(
        !body.contains("-upgrade.service"),
        "install_helper_units must not name the .service unit for enabling — it is \
         triggered by the .path unit, never enabled\n{body}"
    );
}

// ===========================================================================
// 8
// ===========================================================================
// REQ-215-013 — Decision: a failure means uninstall leaves an orphan root oneshot on the
// host pointing at a data dir that no longer belongs to any service. A stale root unit
// that installs binaries is a durable security and operations liability. [O16 x P5]
#[test]
fn cmd_uninstall_removes_both_helper_units() {
    let service_src = read_src("src/cmd_service.rs");
    let uninstall_body = fn_body(&service_src, "fn cmd_uninstall(");
    assert!(
        uninstall_body.contains("remove_helper_units("),
        "cmd_uninstall must call remove_helper_units(&service_name)"
    );

    let helper_src = read_src("src/cmd_service_helper_units.rs");
    let body = fn_body(&helper_src, "pub fn remove_helper_units(");

    assert!(
        body.contains("disable"),
        "remove_helper_units must disable the .path unit before removing it\n{body}"
    );
    for needed in ["helper_path_unit_path", "helper_service_unit_path"] {
        assert!(
            body.contains(needed),
            "remove_helper_units must resolve {needed:?} so BOTH unit files are removed\n{body}"
        );
    }
    assert!(
        body.contains("remove_file"),
        "remove_helper_units must remove the unit files\n{body}"
    );
}

// ===========================================================================
// 9
// ===========================================================================
// REQ-215-012 — Decision: a failure means already-deployed hosts never gain the helper
// (there is no re-install step in the fleet's routine), or the refresh runs as a
// non-root user and turns a working upgrade into a hard failure. Both are silent: the
// first leaves the bug in place forever, the second breaks the manual escape hatch
// operators depend on today. [O17 x P5]
#[test]
fn cmd_upgrade_refreshes_the_helper_units_when_root() {
    let upgrade_src = read_src("src/cmd_upgrade.rs");
    let body = fn_body(&upgrade_src, "fn cmd_upgrade(");
    assert!(
        body.contains("refresh_helper_units_if_root("),
        "cmd_upgrade must call refresh_helper_units_if_root(..) after a successful install"
    );

    let helper_src = read_src("src/cmd_service_helper_units.rs");
    let refresh = fn_body(&helper_src, "pub fn refresh_helper_units_if_root(");

    // Root check: the crate's own convention is an EUID probe (`id -u`, cmd_service.rs
    // check_sudo), so any of these tokens satisfies the contract.
    let root_tokens = ["geteuid", "getuid", "euid", "EUID", "check_sudo", "is_root"];
    assert!(
        root_tokens.iter().any(|t| refresh.contains(t)),
        "refresh_helper_units_if_root must test the effective UID before touching \
         /etc/systemd/system; expected one of {root_tokens:?}\n{refresh}"
    );

    assert!(
        refresh.contains("/etc/systemd/system/"),
        "refresh must look for the node unit under /etc/systemd/system/\n{refresh}"
    );
    assert!(
        refresh.contains("exists()"),
        "refresh must confirm the node unit EXISTS before writing helper units — a host \
         with no systemd node service must get no units\n{refresh}"
    );
}

// ===========================================================================
// 10
// ===========================================================================
// REQ-215-011 — Decision: a failure means re-running `service install` produces different
// unit bytes each time, so the refresh path in test 9 (which writes only when content
// DIFFERS) would rewrite and daemon-reload on every upgrade, and operators could never
// tell a drifted unit from a freshly rendered one. [O7,O8 x P4]
#[test]
fn helper_install_is_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data_dir = dir.path().join("nodedata");
    let cli = fixture_cli();

    // The unit FILE NAMES are what install writes; render into the tempdir under those
    // names so a second round overwrites rather than accumulates. install_helper_units
    // itself is never called: it writes /etc and shells out to systemctl.
    let (path_name, service_name) = helper_unit_names(SERVICE_A);
    let path_file = dir.path().join(&path_name);
    let service_file = dir.path().join(&service_name);

    for _ in 0..2 {
        std::fs::write(
            &path_file,
            helper_path_unit_content(SERVICE_A, &data_dir).as_bytes(),
        )
        .expect("write .path unit");
        std::fs::write(
            &service_file,
            helper_service_unit_content(SERVICE_A, NETWORK, &data_dir, &cli).as_bytes(),
        )
        .expect("write .service unit");
    }

    let round1_path = helper_path_unit_content(SERVICE_A, &data_dir);
    let round2_path = helper_path_unit_content(SERVICE_A, &data_dir);
    assert_eq!(
        round1_path, round2_path,
        ".path unit content must be deterministic for identical inputs"
    );

    let round1_service = helper_service_unit_content(SERVICE_A, NETWORK, &data_dir, &cli);
    let round2_service = helper_service_unit_content(SERVICE_A, NETWORK, &data_dir, &cli);
    assert_eq!(
        round1_service, round2_service,
        ".service unit content must be deterministic for identical inputs"
    );

    assert_eq!(
        std::fs::read(&path_file).expect("read back .path"),
        round1_path.as_bytes(),
        "the second render must produce byte-identical .path bytes"
    );
    assert_eq!(
        std::fs::read(&service_file).expect("read back .service"),
        round1_service.as_bytes(),
        "the second render must produce byte-identical .service bytes"
    );

    let written = std::fs::read_dir(dir.path())
        .expect("read tempdir")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .count();
    assert_eq!(
        written, 2,
        "re-install must overwrite the same two units, never create duplicates"
    );
}

// ===========================================================================
// 11
// ===========================================================================
// REQ-215-016 — Decision: a failure means the milestone paid for the fix by growing two
// already-large files instead of extracting a module, which is the exact Rule 19 drift
// the analysis budgeted against. It is the only signal that catches "wired it inline"
// before review. [O18 x P5]
#[test]
fn cmd_service_rs_and_cmd_upgrade_rs_are_within_the_module_budget() {
    // 918 pre-existing + at most 10 wiring lines in install_systemd / cmd_uninstall.
    let service_lines = line_count("src/cmd_service.rs");
    assert!(
        service_lines <= 928,
        "cmd_service.rs is {service_lines} lines (budget 928 = 918 pre-existing + 10 \
         wiring lines); the helper logic belongs in cmd_service_helper_units.rs"
    );

    let upgrade_lines = line_count("src/cmd_upgrade.rs");
    assert!(
        upgrade_lines <= 500,
        "cmd_upgrade.rs is {upgrade_lines} lines (Rule 19 budget 500); the refresh call \
         site must be <= 3 lines"
    );

    let helper_lines = line_count("src/cmd_service_helper_units.rs");
    assert!(
        helper_lines <= 500,
        "cmd_service_helper_units.rs is {helper_lines} lines (Rule 19 budget 500)"
    );
}
