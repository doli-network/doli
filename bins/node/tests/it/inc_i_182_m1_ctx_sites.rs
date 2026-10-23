//! INC-I-182 M1 — every production ValidationContext site that sets the
//! unbonding gate also sets the producer-auth gate, the same number of times per
//! file (INC-I-173 both-or-neither: a site left at the default judges the rule off).
//!
// covers: producer_auth transaction types unbonding defaults env_loader network_params pool assembly tx_processing validation_checks withdrawal_holdings genesis register bonds withdrawal exit unbonding_tx core data block rpc_client
//
// OUTPUT CONTRACT: fn site_counts() -> BTreeMap<file, (unbonding builder calls, producer-auth builder calls)>
//   O1 per-file pair over non-test sources of crates/*/src and bins/*/src (read-only fs scan)
// PATHS: P-PLANTED synthetic text | P-KNOWN the six files known today | P-ANY any other file
// MATRIX
//   O1 x P-PLANTED -> scanner flags a missing call and accepts a matched one (non-vacuity)
//   O1 x P-KNOWN -> floor: each file still sets the unbonding gate; total >= 8
//   O1 x P-ANY -> inc_i_182_m1_req_pauth_002_every_unbonding_ctx_site_sets_producer_auth
// INPUT PARTITIONS:
//   file kind: production | test (tests.rs, *_tests.rs, tests_*.rs, tests/ dirs, code after #[cfg(test)])
//   count: unbonding only | producer-auth only | both equal | both unequal

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const UNBOND: &str = ".with_inc_i_257_unbonding(";
const PAUTH: &str = ".with_inc_i_182_producer_auth(";
const KNOWN: [&str; 6] = [
    "crates/mempool/src/pool.rs",
    "crates/mempool/src/unbonding.rs",
    "bins/node/src/node/production/assembly.rs",
    "bins/node/src/node/apply_block/tx_processing.rs",
    "bins/node/src/node/validation_checks/mod.rs",
    "bins/node/src/node/validation_checks/slash_withdrawal.rs",
];
const KNOWN_TOTAL: usize = 8;

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

fn pair_of(text: &str) -> (usize, usize) {
    let prod = text
        .lines()
        .take_while(|l| l.trim() != "#[cfg(test)]")
        .collect::<Vec<_>>()
        .join("\n");
    (prod.matches(UNBOND).count(), prod.matches(PAUTH).count())
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
        let pair = pair_of(&std::fs::read_to_string(&f).expect("read source"));
        if pair != (0, 0) {
            counts.insert(rel.to_string_lossy().into_owned(), pair);
        }
    }
    counts
}

// REQ-PAUTH-002 — Decision: a failure names a production context (mempool, builder, block validation, apply or eviction) that never sets the producer-auth gate, so it judges the rule off while the others enforce it (mempool admits what apply refuses).
#[test]
fn inc_i_182_m1_req_pauth_002_every_unbonding_ctx_site_sets_producer_auth() {
    let planted =
        "let c = ValidationContext::new(p, n, 0, h)\n    .with_inc_i_257_unbonding(g, q);\n";
    assert_eq!(
        pair_of(planted),
        (1, 0),
        "NON-VACUITY: scanner must see the planted site"
    );
    let fixed = planted.replace(";\n", "\n    .with_inc_i_182_producer_auth(a);\n");
    assert_eq!(
        pair_of(&fixed),
        (1, 1),
        "NON-VACUITY: scanner must accept a threaded site"
    );
    let after_tests = format!("#[cfg(test)]\n{planted}");
    assert_eq!(
        pair_of(&after_tests),
        (0, 0),
        "NON-VACUITY: test code is excluded"
    );

    let counts = site_counts();
    for f in KNOWN {
        let (unbond, _) = counts.get(f).copied().unwrap_or((0, 0));
        assert!(
            unbond >= 1,
            "NON-VACUITY: {f} no longer sets the unbonding gate; scan or layout changed"
        );
    }
    let total_unbond: usize = counts.values().map(|c| c.0).sum();
    let total_pauth: usize = counts.values().map(|c| c.1).sum();
    assert!(
        total_unbond >= KNOWN_TOTAL,
        "NON-VACUITY: {total_unbond} unbonding sites, expected >= {KNOWN_TOTAL}"
    );
    let mismatched: Vec<String> = counts
        .iter()
        .filter(|(_, (u, a))| u != a)
        .map(|(f, (u, a))| format!("{f}: unbonding {u}, producer-auth {a}"))
        .collect();
    assert!(
        mismatched.is_empty(),
        "every context site must set both gates (unbonding {total_unbond}, producer-auth {total_pauth}):\n{}",
        mismatched.join("\n")
    );
}
