// INC-I-215 M5 — REQ-215-018 — Decision: a failure tells the reader that the shipped staged-upgrade
// handoff is undocumented on that exact surface, so an operator on a sandboxed host cannot find
// `--from-staged`, the helper units, or the EROFS recovery — and the docs must be written or fixed.
//
// OUTPUT CONTRACT (.claude/protocols/output-contract.md)
// Subject: the shipped documentation set, read from disk. Pure file I/O, no process spawn.
// ENUMERATED OUTPUTS (per surface, one test each — a drift in one must not mask another):
//   O1 docs/cli.md            : "--from-staged", "-upgrade.path", "-upgrade.service"
//   O2 docs/troubleshooting.md: "UPDATE_TARGET_NOT_WRITABLE", "Read-only file system (os error 30)",
//                               "sudo doli service install"
//   O3 specs/security_model.md: "--from-staged", "staging", "trust boundary" (ci), "3-of-5"|"threshold"
//   O4 .claude/skills/SKILLS-INDEX.md: "from-staged", "UPDATE_TARGET_NOT_WRITABLE"
//   O5 docs/releases.md       : "--from-staged", "staged"
//   O6 code literals the docs describe: preflight.rs "UPDATE_TARGET_NOT_WRITABLE",
//                                       cmd_service_helper_units.rs "PathExists="
// covers: scripts/gauntlet-gs020.sh (same two O6 literals, asserted offline by GS-020)
// PATHS x INPUT PARTITIONS: file absent -> read helper panics naming the path;
//   file present + literal absent -> assert fails naming file + literal; present -> pass.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read_doc(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("INC-I-215 REQ-215-018: cannot read {}: {e}", path.display()))
}

fn assert_contains(rel: &str, haystack: &str, needle: &str) {
    assert!(
        haystack.contains(needle),
        "INC-I-215 REQ-215-018: {rel} does not document the literal {needle:?}"
    );
}

#[test]
fn inc_i_215_docs_cli_documents_from_staged_and_helper_units() {
    let rel = "docs/cli.md";
    let doc = read_doc(rel);
    assert_contains(rel, &doc, "--from-staged");
    assert_contains(rel, &doc, "-upgrade.path");
    assert_contains(rel, &doc, "-upgrade.service");
}

#[test]
fn inc_i_215_docs_troubleshooting_documents_erofs_recovery() {
    let rel = "docs/troubleshooting.md";
    let doc = read_doc(rel);
    assert_contains(rel, &doc, "UPDATE_TARGET_NOT_WRITABLE");
    assert_contains(rel, &doc, "Read-only file system (os error 30)");
    assert_contains(rel, &doc, "sudo doli service install");
}

#[test]
fn inc_i_215_docs_security_model_documents_staging_trust_boundary() {
    let rel = "specs/security_model.md";
    let doc = read_doc(rel);
    assert_contains(rel, &doc, "--from-staged");
    assert_contains(rel, &doc, "staging");
    assert!(
        doc.to_lowercase().contains("trust boundary"),
        "INC-I-215 REQ-215-018: {rel} does not name the staging directory a \"trust boundary\""
    );
    assert!(
        doc.contains("3-of-5") || doc.contains("threshold"),
        "INC-I-215 REQ-215-018: {rel} does not document the re-verified signature threshold \
         (expected \"3-of-5\" or \"threshold\")"
    );
}

#[test]
fn inc_i_215_docs_skills_index_documents_from_staged_keywords() {
    let rel = ".claude/skills/SKILLS-INDEX.md";
    let doc = read_doc(rel);
    assert_contains(rel, &doc, "from-staged");
    assert_contains(rel, &doc, "UPDATE_TARGET_NOT_WRITABLE");
}

#[test]
fn inc_i_215_docs_releases_documents_staged_delivery() {
    let rel = "docs/releases.md";
    let doc = read_doc(rel);
    assert_contains(rel, &doc, "--from-staged");
    assert_contains(rel, &doc, "staged");
}

#[test]
fn inc_i_215_docs_described_code_literals_still_exist() {
    let preflight_rel = "bins/node/src/updater/preflight.rs";
    let preflight = read_doc(preflight_rel);
    assert_contains(preflight_rel, &preflight, "UPDATE_TARGET_NOT_WRITABLE");

    let units_rel = "bins/cli/src/cmd_service_helper_units.rs";
    let units = read_doc(units_rel);
    assert_contains(units_rel, &units, "PathExists=");
}
