// M2 disk-guardian — installer/uninstaller WIRING (TDD RED)
// REQ-DISK-201 (install writes drop-in) / REQ-DISK-203 (uninstall removes drop-in)
//
// OUTPUT CONTRACT
// Subject under test: the SOURCE of cmd_service.rs (structural wiring), because
// `doli-cli` is a BIN-ONLY crate (no src/lib.rs) — integration tests cannot call the
// private `install_systemd` / `cmd_uninstall`, and the real writes require root
// (/etc/logrotate.d). We assert wiring via include_str!, matching the existing
// convention in bins/cli/tests/delegation_bond_cap.rs.
//   O1: cmd_service.rs defines `fn logrotate_dropin_content` and `fn logrotate_dropin_path`
//       and contains the path literal "/etc/logrotate.d/doli-".
//   O2: the `install_systemd` fn body references both helpers (writes the generated drop-in).
//   O3: the `cmd_uninstall` fn body references `logrotate_dropin_path` and `remove_file`
//       (removes the drop-in).
// PATHS: string-search over static source (no runtime branches).
// INPUT PARTITIONS:
//   - P_helpers:   whole-file search for helper definitions + path literal (O1).
//   - P_install:   `install_systemd` fn-body slice (O2).
//   - P_uninstall: `cmd_uninstall` fn-body slice (O3).
// MATRIX: O1×P_helpers, O2×P_install, O3×P_uninstall.
//
// These assertions FAIL until the developer wires the drop-in into install_systemd
// and cmd_uninstall — that is the intended RED state.

const SRC: &str = include_str!("../src/cmd_service.rs");

/// Return the source slice of a top-level fn body: from the signature up to the
/// next top-level `\nfn ` (or end of file).
fn fn_body<'a>(src: &'a str, sig: &str) -> &'a str {
    let start = src
        .find(sig)
        .unwrap_or_else(|| panic!("fn not found: {sig}"));
    let rest = &src[start..];
    let after_sig = &rest[sig.len()..];
    match after_sig.find("\nfn ") {
        Some(i) => &rest[..sig.len() + i],
        None => rest,
    }
}

// REQ-DISK-201 (Must): the pure content/path helpers exist in the module. [O1×P_helpers]
#[test]
fn req_disk_201_dropin_helpers_defined() {
    assert!(
        SRC.contains("fn logrotate_dropin_content"),
        "logrotate_dropin_content must be defined in cmd_service.rs"
    );
    assert!(
        SRC.contains("fn logrotate_dropin_path"),
        "logrotate_dropin_path must be defined in cmd_service.rs"
    );
    assert!(
        SRC.contains("/etc/logrotate.d/doli-"),
        "drop-in path literal /etc/logrotate.d/doli- must appear in cmd_service.rs"
    );
}

// REQ-DISK-201 (Must): install_systemd writes the drop-in it generates. [O2×P_install]
#[test]
fn req_disk_201_install_systemd_writes_dropin() {
    let body = fn_body(SRC, "fn install_systemd");
    assert!(
        body.contains("logrotate_dropin_content"),
        "install_systemd must generate content via logrotate_dropin_content"
    );
    assert!(
        body.contains("logrotate_dropin_path"),
        "install_systemd must target the path via logrotate_dropin_path"
    );
}

// REQ-DISK-203 (Should): uninstall removes the drop-in (absent-file tolerated). [O3×P_uninstall]
#[test]
fn req_disk_203_uninstall_removes_dropin() {
    let body = fn_body(SRC, "fn cmd_uninstall");
    assert!(
        body.contains("logrotate_dropin_path"),
        "cmd_uninstall must reference the drop-in path via logrotate_dropin_path"
    );
    assert!(
        body.contains("remove_file"),
        "cmd_uninstall must remove the drop-in file via std::fs::remove_file"
    );
}
