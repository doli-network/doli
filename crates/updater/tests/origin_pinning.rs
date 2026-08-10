//! INC-I-157 M1 (ORIGIN DE-PINNING) — reproduction + contract for the
//! auto-updater release origin.
//!
//! Symptom (measured, see docs/bugfixes/inc-i-157-installer-integrity-analysis.md
//! REQ-I157-010 / REQ-I157-011, §4 row b2 and §5 rank #3):
//!   `crates/updater/src/constants.rs` pins the release origin to the GitHub
//!   namespace `e-weil`, which nobody owns (`github.com/e-weil` -> 404,
//!   `api.github.com/users/e-weil` -> 404). `api.github.com/repos/e-weil/doli`
//!   currently returns a `301` rename-redirect to the real repo
//!   (`doli-network/doli`, confirmed via `git remote -v`), but a GitHub
//!   rename-redirect is NOT a security boundary — it lapses the moment the
//!   old namespace is reoccupied by an attacker (entry point #3 in the
//!   analysis, "CONDITIONAL — precondition not currently met"). Additionally
//!   `FALLBACK_MIRROR = "https://releases.doli.network"` is NXDOMAIN
//!   (measured via dig/host) — a dangling hostname sitting in the download
//!   fallback chain.
//!
//! Root cause under test: `crates/updater/src/constants.rs:120,123,126,129`.
//!
//! Fix (already decided by the runner, not by this test file):
//!   1. Re-point `GITHUB_REPO` / `GITHUB_API_URL` / `GITHUB_RELEASES_URL` to
//!      the owned namespace `doli-network/doli`.
//!   2. Remove `FALLBACK_MIRROR` entirely (const + both call sites in
//!      `crates/updater/src/download.rs` + the re-export in
//!      `crates/updater/src/lib.rs`). Because the symbol will not exist after
//!      the fix, this test file must NEVER reference `FALLBACK_MIRROR` by
//!      name — its absence is asserted via a source-text scan instead
//!      (see `test_no_nonresolving_fallback_mirror_in_updater_source`).
//!
//! Requirement IDs: REQ-I157-010, REQ-I157-011
//! (docs/bugfixes/inc-i-157-installer-integrity-analysis.md, §2 + §13 Traceability Matrix)

// covers: constants, download, lib, Cargo, docker-compose, docker-compose.devnet,
// docker-compose.testnet, install, install.ps1, publish_release, sign-release,
// releases, running_a_node, producer_node_quickstart, testnet, troubleshooting,
// buy_doli, docker, auto_update_system, gui-architecture, README, SKILL, index,
// network, implementation_distribution, IMPLEMENTATION_PLAN_DISTRIBUTION,
// architecture
//
// The stem list above is the full set of files this M1 milestone may touch
// (source, packaging, CI, and every doc/spec that names the release origin
// or the fallback mirror). It exists to authorize the developer's edit set
// under the test-gate's subject binding, not to imply every stem is a Rust
// module — most are markdown/yaml/shell files that reference the same
// literals asserted here.

// OUTPUT CONTRACT: const GITHUB_REPO / GITHUB_API_URL / GITHUB_RELEASES_URL
//                  (and the ABSENCE of FALLBACK_MIRROR + the unowned literal)
//
// These are `pub const &str` values plus a source-tree invariant, not a
// function with parameters — the O1 (mutable params) and O2 (receiver/self
// mutation) categories from the output-contract protocol are N/A: there is
// no function under test, no parameter list, and no `self`. There is nothing
// to mutate; the "output" IS the constant's resolved value and the state of
// the source tree.
//
//   O1: N/A — no parameters exist for a `const`.
//   O2: N/A — no receiver/`self` exists for a `const`.
//   O3: return — the const's resolved `&'static str` value, read via
//       `updater::GITHUB_REPO`, `updater::GITHUB_API_URL`,
//       `updater::GITHUB_RELEASES_URL`. Each must equal / contain the owned
//       namespace `doli-network/doli` and must NOT contain the unowned
//       namespace literal.
//   O4: persistent store — the repository working tree is treated as a
//       read-only store being asserted over (source-text scan). O4a: no `.rs`
//       file under `crates/updater/src/` (recursively) contains the
//       unowned-namespace literal. O4b: no `.rs` file under `src/` contains
//       the non-resolving fallback hostname literal `releases.doli.network`.
//       O4c: every path in `ORIGIN_DEFINITION_SITES` EXISTS and is readable
//       (absence is a FAILURE, never a skip — see the test's comment).
//       O4d: no file in `ORIGIN_DEFINITION_SITES` contains the
//       unowned-namespace literal, regardless of language/toolchain.
//
// PATHS:
//   P1: GITHUB_REPO value check
//   P2: GITHUB_API_URL value check (contains-owned AND not-contains-unowned)
//   P3: GITHUB_RELEASES_URL value check (contains-owned AND not-contains-unowned)
//   P4: recursive source scan for the unowned namespace literal (REQ-I157-010)
//   P5: recursive source scan for the non-resolving fallback hostname (REQ-I157-011)
//   P6: explicit-list scan of every origin-DEFINITION site across the whole
//       repo for the unowned namespace literal (F4 — the guard that stops the
//       eight shipped copies of the origin from drifting apart again)
//   P7: existence/readability of every path in that explicit list (F4 — a
//       renamed or deleted entry must turn the guard RED, not shrink it)
//
// INPUT PARTITIONS:
//   P1 has a single partition — `GITHUB_REPO` is one literal, one comparison,
//     no branching relationship to partition on.
//   P2/P3 each have exactly 2 partitions per the checklist's own rule
//     ("If you cannot identify 2+ partitions, state why"): (a) positive
//     assertion — the owned namespace IS present; (b) negative assertion —
//     the unowned namespace literal is ABSENT. These are logically
//     independent (a string could contain both, neither, or either alone),
//     so both must be asserted per URL to close the requirement.
//   P4/P5 partition by file population: (a) the const-definition file itself
//     (`constants.rs`, expected to be clean post-fix), and (b) every other
//     `.rs` file under `src/` recursively (e.g. `download.rs`, which is the
//     other known call site per the analysis doc). The walk does not
//     special-case (a) vs (b) — it is one recursive scan — but the failure
//     message must name the specific file+line so a regression in either
//     population is distinguishable.
//   P6 partitions by file KIND, because kind is what determines who edits the
//     file and with which toolchain — which is precisely why these copies
//     drift independently: (a) Rust source (`crates/updater/src/constants.rs`)
//     — also covered by P4, kept here as the anchor that proves the
//     explicit-list mechanism itself works against a file whose expected value
//     is independently known; (b) POSIX shell installer/publisher scripts
//     (`install.sh`, `publish_release.sh`, `sign-release.sh`); (c) PowerShell
//     installer (`install.ps1`); (d) container manifests (the three
//     `docker-compose*.yml`); (e) crate manifest (`Cargo.toml`). The
//     recurrence proof for this partitioning is commit `48183c0` (2026-04-01),
//     which repaired partition (b) alone and left partition (a) pinned to the
//     unowned namespace for four months.
//   P7 has exactly 2 partitions: (a) the path is present and readable — it
//     contributes to the P6 scan; (b) the path is absent or unreadable — the
//     test MUST fail. There is deliberately no "skip" partition; see the
//     test body's comment for why a skip is the more dangerous outcome.
//
// MATRIX: 2 outputs (O3, O4) x 7 paths = the following required assertions
//   P1:  O3(GITHUB_REPO == "doli-network/doli")                              [x]
//   P2a: O3(GITHUB_API_URL contains "doli-network/doli")                     [x]
//   P2b: O3(GITHUB_API_URL not-contains unowned literal)                     [x]
//   P3a: O3(GITHUB_RELEASES_URL contains "doli-network/doli")                [x]
//   P3b: O3(GITHUB_RELEASES_URL not-contains unowned literal)                [x]
//   P4:  O4a(no src/**/*.rs contains unowned literal, offending file:line    [x]
//            named in panic message)
//   P5:  O4b(no src/**/*.rs contains "releases.doli.network", offending      [x]
//            file:line named in panic message)
//   P6:  O4d(no file in ORIGIN_DEFINITION_SITES contains unowned literal,    [x]
//            every offending file AND its matched line named in the panic
//            message; all 5 kind-partitions a-e represented in the list)
//   P7:  O4c(every path in ORIGIN_DEFINITION_SITES exists and is readable;   [x]
//            a missing entry FAILS the test and is named in the panic
//            message, together with the derived repo root it was resolved
//            against, so a wrong-root failure is distinguishable from a
//            genuinely-renamed file)

use std::fs;
use std::path::{Path, PathBuf};

/// The unowned GitHub namespace literal. Written as a plain string literal —
/// the recursive scan below is scoped to `src/` ONLY (see the comment on
/// `updater_src_dir()`), so this file containing the needle is harmless: it
/// can never self-match.
const UNOWNED_NAMESPACE: &str = "e-weil";

/// The non-resolving fallback mirror hostname (measured NXDOMAIN via
/// dig/host). Same self-match-safety note as `UNOWNED_NAMESPACE` applies.
const NONRESOLVING_FALLBACK_HOST: &str = "releases.doli.network";

/// Every file in this repository that DEFINES the release origin (the GitHub
/// namespace / repo URL that a node, an installer, or a publisher will
/// actually contact), expressed relative to the repo root.
///
/// Why an explicit hardcoded list instead of a directory walk: the failure
/// this guards against is *divergence between copies*, and a walk cannot tell
/// a copy that was renamed out of the scan from a copy that never existed.
/// An explicit list is auditable — a reader can see in one place exactly which
/// origin definitions are claimed to be covered, and adding a new origin
/// definition without adding it here is a review-visible omission rather than
/// a silent gap.
///
/// Recurrence proof that one guarded copy is not enough: commit `48183c0`
/// (2026-04-01) repointed `scripts/install.sh` to the owned namespace while
/// `crates/updater/src/constants.rs` stayed pinned to the unowned one for four
/// months (INC-I-157; the analysis calls this the "DIVERGENCE MARKER").
///
/// This list is self-match-safe: it does NOT include `tests/`, so this file's
/// own `UNOWNED_NAMESPACE` needle is never scanned.
const ORIGIN_DEFINITION_SITES: &[&str] = &[
    // (a) Rust source — the node's compiled-in origin
    "crates/updater/src/constants.rs",
    // (b) POSIX shell installers / publishers
    "scripts/install.sh",
    "scripts/publish_release.sh",
    "scripts/sign-release.sh",
    // (c) PowerShell installer
    "scripts/install.ps1",
    // (d) container manifests
    "docker/docker-compose.yml",
    "docker/docker-compose.devnet.yml",
    "docker/docker-compose.testnet.yml",
    // (e) crate manifest
    "Cargo.toml",
];

/// Repository root, derived from `CARGO_MANIFEST_DIR` (`<root>/crates/updater`)
/// by ascending exactly two levels.
///
/// The layout assumption is ASSERTED, not trusted. A silently-wrong root would
/// make `test_all_origin_definition_sites_use_owned_namespace` scan nothing,
/// find nothing, and report success — a vacuous pass, which is strictly worse
/// than having no guard at all because it also carries a green check mark.
fn repo_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent() // <root>/crates
        .and_then(Path::parent) // <root>
        .unwrap_or_else(|| {
            panic!(
                "layout assumption broken: CARGO_MANIFEST_DIR {:?} has fewer than \
                 two ancestors, so the repository root cannot be derived by \
                 ascending two levels. Fix repo_root() to match the new crate \
                 layout — do NOT let the scan proceed against a wrong root.",
                manifest
            )
        });

    assert!(
        root.join(".git").exists() || root.join("Cargo.toml").exists(),
        "layout assumption broken: derived repository root {:?} (from \
         CARGO_MANIFEST_DIR {:?} by ascending two levels) contains neither a \
         `.git` entry nor a `Cargo.toml`. Refusing to scan: a wrong root makes \
         every origin-site path resolve to a non-existent file, which would \
         turn this guard into a vacuous pass. Fix repo_root() to match the \
         current crate layout.",
        root,
        manifest
    );

    root.to_path_buf()
}

/// Root of the `updater` crate's SOURCE tree (never `tests/`).
///
/// IMPORTANT: this MUST stay scoped to `src/` and must NEVER be widened to
/// include `tests/` (or the crate root, or any ancestor). This very test
/// file legitimately contains the needle literals (`UNOWNED_NAMESPACE`,
/// `NONRESOLVING_FALLBACK_HOST`) as plain Rust string constants so it can
/// perform the scan and describe the defect in doc comments above. If the
/// walk ever includes `tests/`, these tests would self-match on their own
/// needle and could never pass — even after the developer's fix lands. Do
/// NOT "helpfully" widen this path.
fn updater_src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Recursively collect every `.rs` file under `dir`.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Scan every `.rs` file under `src/` for `needle`. Returns a Vec of
/// human-readable `"path:line: <line content>"` hits so failure messages can
/// name exact offending locations rather than asserting blindly.
fn scan_src_for_literal(needle: &str) -> Vec<String> {
    let src_dir = updater_src_dir();
    let mut files = Vec::new();
    collect_rs_files(&src_dir, &mut files);
    assert!(
        !files.is_empty(),
        "sanity check failed: found zero .rs files under {}. The scan is \
         mis-scoped or the crate layout changed — fix the test before \
         trusting a negative result.",
        src_dir.display()
    );

    let mut hits = Vec::new();
    for file in &files {
        let Ok(contents) = fs::read_to_string(file) else {
            continue;
        };
        for (idx, line) in contents.lines().enumerate() {
            if line.contains(needle) {
                hits.push(format!("{}:{}: {}", file.display(), idx + 1, line.trim()));
            }
        }
    }
    hits
}

// ============================================================================
// Must — REQ-I157-010: release origin MUST be pinned to an owned namespace
// ============================================================================

// Requirement: REQ-I157-010 (Should, per traceability matrix — treated as
// Must for this milestone's exhaustiveness since it is the sole scope item)
// Acceptance: constants.rs GITHUB_REPO names doli-network/doli
// P1 — single partition (see OUTPUT CONTRACT above)
#[test]
fn test_github_repo_pinned_to_owned_namespace() {
    assert_eq!(
        updater::GITHUB_REPO,
        "doli-network/doli",
        "GITHUB_REPO must be pinned to the owned namespace 'doli-network/doli', \
         not an unowned or redirect-dependent namespace. Got: {:?}",
        updater::GITHUB_REPO
    );
}

// Requirement: REQ-I157-010
// Acceptance: GITHUB_API_URL both contains the owned namespace (P2a) and
// does not contain the unowned namespace literal (P2b) — asserted
// independently per the OUTPUT CONTRACT partition rule.
#[test]
fn test_github_api_url_pinned_to_owned_namespace() {
    assert!(
        updater::GITHUB_API_URL.contains("doli-network/doli"),
        "P2a FAILED: GITHUB_API_URL must contain the owned namespace \
         'doli-network/doli'. Got: {:?}",
        updater::GITHUB_API_URL
    );
    assert!(
        !updater::GITHUB_API_URL.contains(UNOWNED_NAMESPACE),
        "P2b FAILED: GITHUB_API_URL must NOT contain the unowned namespace \
         literal {:?} — a GitHub rename-redirect is not a security boundary \
         (REQ-I157-010, entry point #3 in the analysis). Got: {:?}",
        UNOWNED_NAMESPACE,
        updater::GITHUB_API_URL
    );
}

// Requirement: REQ-I157-010
// Acceptance: GITHUB_RELEASES_URL both contains the owned namespace (P3a)
// and does not contain the unowned namespace literal (P3b).
#[test]
fn test_github_releases_url_pinned_to_owned_namespace() {
    assert!(
        updater::GITHUB_RELEASES_URL.contains("doli-network/doli"),
        "P3a FAILED: GITHUB_RELEASES_URL must contain the owned namespace \
         'doli-network/doli'. Got: {:?}",
        updater::GITHUB_RELEASES_URL
    );
    assert!(
        !updater::GITHUB_RELEASES_URL.contains(UNOWNED_NAMESPACE),
        "P3b FAILED: GITHUB_RELEASES_URL must NOT contain the unowned \
         namespace literal {:?}. Got: {:?}",
        UNOWNED_NAMESPACE,
        updater::GITHUB_RELEASES_URL
    );
}

// Requirement: REQ-I157-010
// Acceptance: NO file anywhere under crates/updater/src/ (recursively)
// contains the unowned namespace literal — not just constants.rs. This
// catches any additional hardcoded call site (the analysis doc names
// download.rs as a second consumer of the origin constants).
//
// Scope note: intentionally src/ only — see updater_src_dir() doc comment.
#[test]
fn test_no_unowned_namespace_literal_in_updater_source() {
    let hits = scan_src_for_literal(UNOWNED_NAMESPACE);
    assert!(
        hits.is_empty(),
        "REQ-I157-010 VIOLATION: found {} occurrence(s) of the unowned \
         namespace literal {:?} under crates/updater/src/ (scan is src/ \
         only, tests/ is intentionally excluded — see updater_src_dir() \
         doc comment). Offending location(s):\n{}",
        hits.len(),
        UNOWNED_NAMESPACE,
        hits.join("\n")
    );
}

// Requirement: REQ-I157-010 (F4 — reviewer finding, INC-I-157 M1 iteration 1)
// Acceptance: EVERY file that defines the release origin — not just the Rust
// constants — is pinned to the owned namespace, AND every such file still
// exists at the path this guard claims to cover.
//
// The four preceding tests all read either the three `pub const` values or the
// `crates/updater/src/` subtree. That covers ONE of the nine shipped origin
// definitions. The other eight (both installers, both release-publishing
// scripts, all three compose files, the crate manifest) were repointed by this
// same milestone and had zero regression guard — which is the exact shape of
// the drift that produced INC-I-157 in the first place (see
// ORIGIN_DEFINITION_SITES for the recurrence proof).
//
// P6 + P7 — see the OUTPUT CONTRACT block at the top of this file.
#[test]
fn test_all_origin_definition_sites_use_owned_namespace() {
    let root = repo_root();

    let mut violations: Vec<String> = Vec::new();
    let mut unreadable: Vec<String> = Vec::new();

    for rel in ORIGIN_DEFINITION_SITES {
        let path = root.join(rel);

        // A MISSING FILE MUST FAIL THIS TEST — IT MUST NEVER BE SKIPPED.
        //
        // If one of these paths is renamed, moved or deleted, the guard has to
        // go RED so a human re-points it at the file's new home. The tempting
        // alternative (`continue` on a missing path) lets the guard silently
        // shrink its own coverage: the origin definition would still exist
        // somewhere in the tree, still be shipped, and simply stop being
        // checked — while the suite keeps reporting green. That is how the
        // original four-month divergence went unnoticed, and it is strictly
        // worse than a loud failure on a file rename.
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(err) => {
                unreadable.push(format!("{} -> {} ({})", rel, path.display(), err));
                continue;
            }
        };

        for (idx, line) in contents.lines().enumerate() {
            if line.contains(UNOWNED_NAMESPACE) {
                violations.push(format!("{}:{}: {}", rel, idx + 1, line.trim()));
            }
        }
    }

    // P7 first: an unreadable site means the P6 result below is incomplete, so
    // report the coverage hole before reporting a (possibly vacuous) clean scan.
    assert!(
        unreadable.is_empty(),
        "F4 COVERAGE HOLE: {} of {} origin-definition site(s) could not be read, \
         so this guard did NOT check them. A missing file is a FAILURE, not a \
         skip — if the file was renamed or moved, update ORIGIN_DEFINITION_SITES \
         to its new path; if it was deleted, remove the entry deliberately and \
         say so in the commit. Derived repo root: {}. Unreadable site(s):\n{}",
        unreadable.len(),
        ORIGIN_DEFINITION_SITES.len(),
        root.display(),
        unreadable.join("\n")
    );

    // P6: with every site confirmed readable, a clean scan is meaningful.
    assert!(
        violations.is_empty(),
        "REQ-I157-010 VIOLATION (F4): found {} occurrence(s) of the unowned \
         namespace literal {:?} across {} origin-definition site(s). The release \
         origin must be pinned to the owned namespace 'doli-network/doli' in \
         EVERY location that defines it — a GitHub rename-redirect is not a \
         security boundary, and a namespace pinned in one copy but not another \
         is exactly the divergence that caused INC-I-157. Offending \
         location(s):\n{}",
        violations.len(),
        UNOWNED_NAMESPACE,
        ORIGIN_DEFINITION_SITES.len(),
        violations.join("\n")
    );
}

// ============================================================================
// Must — REQ-I157-011: download fallback chain MUST NOT contain a
// non-resolving hostname
// ============================================================================

// Requirement: REQ-I157-011
// Acceptance: NO file under crates/updater/src/ (recursively) contains the
// non-resolving fallback mirror hostname literal. The FALLBACK_MIRROR const
// itself is being REMOVED as part of this fix (decided by the runner), so
// this test deliberately does NOT reference `updater::FALLBACK_MIRROR` by
// symbol — referencing a symbol slated for deletion would make this test
// file fail to compile once the fix lands, defeating its purpose as a
// red-then-green reproduction test. Instead it asserts the literal's
// absence from the source text, which remains valid before AND after the
// symbol is deleted.
//
// Scope note: intentionally src/ only — see updater_src_dir() doc comment.
#[test]
fn test_no_nonresolving_fallback_mirror_in_updater_source() {
    let hits = scan_src_for_literal(NONRESOLVING_FALLBACK_HOST);
    assert!(
        hits.is_empty(),
        "REQ-I157-011 VIOLATION: found {} occurrence(s) of the non-resolving \
         fallback mirror hostname {:?} (measured NXDOMAIN) under \
         crates/updater/src/ (scan is src/ only, tests/ is intentionally \
         excluded — see updater_src_dir() doc comment). Offending \
         location(s):\n{}",
        hits.len(),
        NONRESOLVING_FALLBACK_HOST,
        hits.join("\n")
    );
}
