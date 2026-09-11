//! REQ-ROT-010 B3 source scan — the file filter plus the self-check that makes it safe.
//!
//! Included with `#[path]` from `bls_rotation_repro.rs` (never declared in `it/main.rs`)
//! so the tripwire file stays inside the 800-line test-file budget.
//!
//! INV-TEST-001: a scan that decides by filename must assert its own parsing assumption.
//! The old filter excluded exactly `tests.rs`, so ~76 `#[cfg(test)]` files living inside
//! `src/` were read as production. It passed only because none of them happened to write
//! `.bls_pubkey`; the first one that did (a storage wire fixture) failed the tripwire.
//!
//! OUTPUT CONTRACT — helper module for `req_rot_010_b3_registration_is_the_only_writer_*`.
//! `scan_sources`: O1 `SourceScan.production` (rel path + comment-stripped lines, one entry
//!    per non-test `.rs`), O2 `SourceScan.excluded` (rel path + RAW text, one entry per
//!    test-named `.rs`). No mutable params, no receiver, no store writes, no logging.
//! `assert_scan_is_honest`: O1 panic-or-not only; it reads `&SourceScan` and mutates
//!    nothing. Four independent panic causes: too few exclusions, an excluded file that is
//!    neither test-attributed nor declared by a `#[cfg(test)] mod`, a missing MUST_SCAN
//!    file, a MUST_SCAN file read as 0 lines.
//!
//! INPUT PARTITIONS (walker, per directory entry): dir named `tests`/`target` (skipped,
//! not descended) | other dir (descended) | non-`.rs` file (ignored) | `.rs` with a test
//! filename -> O2 | other `.rs` -> O1 | unreadable path (skipped). Per line of an O1 file:
//! whole-line `//` comment (dropped) | any other line (kept verbatim).
//! Self-check partitions: excluded set below bound | excluded entry without a test marker |
//! MUST_SCAN file absent | MUST_SCAN file present but empty | all four satisfied (pass).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// What the scan read (comment-stripped lines) and what it refused (raw text, so the
/// self-check can prove the refused files really are test code).
pub struct SourceScan {
    pub production: Vec<(String, Vec<String>)>,
    pub excluded: Vec<(String, String)>,
}

/// This repo keeps `#[cfg(test)]` code inside `src/` under three names. Everything else
/// under `crates`/`bins` counts as production.
fn is_test_filename(name: &str) -> bool {
    name == "tests.rs" || name.starts_with("tests_") || name.ends_with("_tests.rs")
}

/// Measured 2026-09-11: 76 such files under `crates` + `bins`. A lower bound, because the
/// exact count rots with every new test module.
const MIN_EXCLUDED: usize = 60;

/// Production files the scan MUST reach. Without this, a filter bug that empties the scan
/// reports "no violations found" and REQ-ROT-010 proves nothing.
const MUST_SCAN: [&str; 4] = [
    "crates/storage/src/producer/set_registration.rs",
    "bins/node/src/node/apply_block/genesis_completion.rs",
    "bins/node/src/node/apply_block/tx_processing.rs",
    "bins/node/src/node/rewards.rs",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root must resolve")
}

/// Every non-test `.rs` file under the given roots, with whole-line `//` comments stripped
/// so a tombstone comment cannot satisfy or break a scan. Excluded files are kept, not
/// dropped: `assert_scan_is_honest` has to inspect them.
pub fn scan_sources(roots: &[&str]) -> SourceScan {
    let root = repo_root();
    let mut stack: Vec<PathBuf> = roots.iter().map(|r| root.join(r)).collect();
    let mut scan = SourceScan {
        production: Vec::new(),
        excluded: Vec::new(),
    };
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            if p.is_dir() {
                if name != "tests" && name != "target" {
                    stack.push(p);
                }
                continue;
            }
            if !name.ends_with(".rs") {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&p) else {
                continue;
            };
            let rel = p
                .strip_prefix(&root)
                .unwrap_or(&p)
                .to_string_lossy()
                .to_string();
            if is_test_filename(name) {
                scan.excluded.push((rel, src));
            } else {
                let lines = src
                    .lines()
                    .filter(|l| !l.trim_start().starts_with("//"))
                    .map(|l| l.to_string())
                    .collect();
                scan.production.push((rel, lines));
            }
        }
    }
    scan
}

fn stem(rel: &str) -> &str {
    rel.rsplit('/')
        .next()
        .unwrap_or(rel)
        .trim_end_matches(".rs")
}

/// Module names a production file declares under `#[cfg(test)]`. A pure fixture file like
/// `tests_m5_common.rs` holds no test attribute of its own; its parent's `#[cfg(test)] mod`
/// is the proof that it is test code.
fn cfg_test_modules(production: &[(String, Vec<String>)]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (_, lines) in production {
        for (i, line) in lines.iter().enumerate() {
            let Some(rest) = line.trim().strip_suffix(';') else {
                continue;
            };
            let Some((_, name)) = rest.rsplit_once("mod ") else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
            let lo = i.saturating_sub(3);
            if lines[lo..=i].iter().any(|l| l.contains("#[cfg(test)]")) {
                out.insert(name.to_string());
            }
        }
    }
    out
}

/// The widened filter could hide a real production write site, so make the scan prove what
/// it did: it skipped real test code, it skipped a plausible amount of it, and it still
/// read the production files the tripwire is about.
pub fn assert_scan_is_honest(scan: &SourceScan) {
    assert!(
        scan.excluded.len() >= MIN_EXCLUDED,
        "REQ-ROT-010 B3 scan self-check: only {} in-`src` test files were excluded (>= \
         {MIN_EXCLUDED} measured). The filename convention moved, so `#[cfg(test)]` code is \
         now being read as production and the write-site map will show false positives. \
         Re-measure: find crates bins -name 'tests.rs' -o -name 'tests_*.rs' -o -name \
         '*_tests.rs'",
        scan.excluded.len()
    );

    let markers = ["#[test]", "#[cfg(test)]", "::test]"];
    let cfg_test_mods = cfg_test_modules(&scan.production);
    let not_test: Vec<&str> = scan
        .excluded
        .iter()
        .filter(|(f, src)| {
            !markers.iter().any(|m| src.contains(m)) && !cfg_test_mods.contains(stem(f))
        })
        .map(|(f, _)| f.as_str())
        .collect();
    assert!(
        not_test.is_empty(),
        "REQ-ROT-010 B3 scan self-check: these files were skipped as test code but carry no \
         test attribute and no production file declares them under `#[cfg(test)]`, so the \
         scan is silently ignoring production: {not_test:?}"
    );

    let scanned: BTreeSet<&str> = scan.production.iter().map(|(f, _)| f.as_str()).collect();
    for must in MUST_SCAN {
        assert!(
            scanned.contains(must),
            "REQ-ROT-010 B3 scan self-check: `{must}` was not scanned ({} files read). A scan \
             that misses the registration and write-site files reports 'no violations' for \
             free",
            scan.production.len()
        );
    }
    let empty: Vec<&str> = scan
        .production
        .iter()
        .filter(|(f, lines)| MUST_SCAN.contains(&f.as_str()) && lines.is_empty())
        .map(|(f, _)| f.as_str())
        .collect();
    assert!(
        empty.is_empty(),
        "REQ-ROT-010 B3 scan self-check: read 0 lines from {empty:?}, so matching nothing in \
         them is meaningless"
    );
}
