//! INC-I-257 M1 — every production ValidationContext site that sets the
//! slashing gate also sets the unbonding gate and period, the same number of
//! times per file (INC-I-173 both-or-neither: a builder that skips the lock
//! makes blocks that apply refuses).
//!
// OUTPUT CONTRACT: fn site_counts() -> BTreeMap<file, (slashing builder calls, unbonding builder calls)>
//   O1 per-file pair over non-test sources of crates/*/src and bins/*/src (read-only fs scan)
// PATHS: P-KNOWN the seven sites known today | P-ANY any other file that calls either builder
// INPUT PARTITIONS:
//   file kind: production module | test module (tests.rs, *_tests.rs, tests_*.rs, code after #[cfg(test)]) excluded
//   count: file calls only the slashing builder | only the unbonding builder | both, equal | both, unequal
// MATRIX:
//   O1 x P-KNOWN -> non-vacuity floor (each known file still has its slashing call)
//   O1 x P-ANY -> inc_i_257_m1_req_unbond_005_every_slashing_ctx_site_sets_unbonding

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const SLASH: &str = ".with_inc_i_255_slashing_activation_height(";
const UNBOND: &str = ".with_inc_i_257_unbonding(";
const KNOWN: [&str; 5] = [
    "crates/mempool/src/pool.rs",
    "bins/node/src/node/production/assembly.rs",
    "bins/node/src/node/apply_block/tx_processing.rs",
    "bins/node/src/node/validation_checks/mod.rs",
    "bins/node/src/node/validation_checks/slash_withdrawal.rs",
];
const KNOWN_TOTAL: usize = 7;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn is_test_file(p: &Path) -> bool {
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name == "tests.rs"
        || name.ends_with("_tests.rs")
        || name.starts_with("tests_")
        || p.components().any(|c| c.as_os_str() == "tests")
}

fn production_text(text: &str) -> String {
    text.lines()
        .take_while(|l| l.trim() != "#[cfg(test)]")
        .collect::<Vec<_>>()
        .join("\n")
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn site_counts() -> BTreeMap<String, (usize, usize)> {
    let root = workspace();
    let mut files = Vec::new();
    for group in ["crates", "bins"] {
        for krate in std::fs::read_dir(root.join(group))
            .expect("workspace group")
            .flatten()
        {
            walk(&krate.path().join("src"), &mut files);
        }
    }
    let mut counts = BTreeMap::new();
    for f in files {
        let rel = f.strip_prefix(&root).unwrap_or(&f).to_path_buf();
        if is_test_file(&rel) {
            continue;
        }
        let text = production_text(&std::fs::read_to_string(&f).expect("read source"));
        let pair = (text.matches(SLASH).count(), text.matches(UNBOND).count());
        if pair != (0, 0) {
            counts.insert(rel.to_string_lossy().into_owned(), pair);
        }
    }
    counts
}

// REQ-UNBOND-005 — Decision: a failure means one production validation context (mempool, builder, block validation or apply) runs without the unbonding gate, so it judges the lock fail-closed at u64::MAX while the others apply it.
#[test]
fn inc_i_257_m1_req_unbond_005_every_slashing_ctx_site_sets_unbonding() {
    let counts = site_counts();
    for f in KNOWN {
        let (slash, _) = counts.get(f).copied().unwrap_or((0, 0));
        assert!(
            slash >= 1,
            "NON-VACUITY: {f} no longer sets the slashing gate; scan or layout changed"
        );
    }
    let total_slash: usize = counts.values().map(|c| c.0).sum();
    let total_unbond: usize = counts.values().map(|c| c.1).sum();
    assert!(
        total_slash >= KNOWN_TOTAL,
        "NON-VACUITY: {total_slash} slashing sites, expected >= {KNOWN_TOTAL}"
    );
    let mismatched: Vec<String> = counts
        .iter()
        .filter(|(_, (s, u))| s != u)
        .map(|(f, (s, u))| format!("{f}: slashing {s}, unbonding {u}"))
        .collect();
    assert!(
        mismatched.is_empty() && total_slash == total_unbond,
        "every context site must set both gates (slashing {total_slash}, unbonding {total_unbond}):\n{}",
        mismatched.join("\n")
    );
}
