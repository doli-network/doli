//! INC-I-248 x INC-I-235 — the hole-repair backfill and the start-time provenance check.
//!
// covers: bins/node/src/node/maintainer_sanitize.rs, crates/storage/src/archiver/import.rs, docs/archiver.md, docs/troubleshooting.md, docs/auto_update_system.md
//!
// OUTPUT CONTRACT: Node::sanitize_retired_maintainer_set(&self) -> bool (async, start-time entry)
//   after storage::archiver::backfill_from_archive fills the node's snap hole; doc text.
//   O1: none
//   O2: maintainer_state.set / last_derived_height; REL-M1 eligibility; maintainer_probe_phase()
//   O3: true iff it reset
//   O4: maintainer_state.bin; file text of the three docs (read-only)
//   O5: none   O6: none
// PATHS: B1 first start, then backfill, then restart -> reset once, root stays EMPTY and
//   eligible; B2 backfill before the first start -> block D+1 present, the stale root is kept
//   (the residual the docs warn about); D1 the docs order the first start before any backfill.
// INPUT PARTITIONS: order {start-then-backfill, backfill-then-start}; doc {archiver §5.3,
//   troubleshooting §1.12, troubleshooting §5.4, auto_update_system §2.6.1 Provenance}.

use std::path::Path;

use crate::inc_i_248_rel_m2_fixture as m2;
use crate::inc_i_249_archive_fixture as archive;
use crypto::Hash;
use doli_node::node::Node;
use tempfile::TempDir;

/// Fills the node's hole 2..=S from an archive of 1..=S, as `restore --backfill` does.
fn backfill_the_hole(node: &Node, temp: &TempDir) {
    let dir = temp.path().join("archive");
    archive::build_legacy_archive(&dir, m2::S, Hash::ZERO);
    let imported =
        storage::archiver::backfill_from_archive(&dir, &node.block_store, None, u64::MAX)
            .expect("fixture: backfill_from_archive");
    assert_eq!(imported, m2::S - 1, "fixture: backfill imports 2..=S only");
    assert!(
        m2::has_block(node, 2) && m2::has_block(node, m2::S) && m2::has_block(node, m2::S + 1),
        "fixture: store = 1..=TIP after the backfill"
    );
}

fn read_repo_file(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// The lines from the first line that starts with `start` up to the next line that
/// starts with any of `ends`.
fn slice(text: &str, start: &str, ends: &[&str]) -> Option<String> {
    let mut lines = text.lines().skip_while(|l| !l.starts_with(start));
    let first = lines.next()?;
    let rest = lines.take_while(|l| !ends.iter().any(|e| l.starts_with(e)));
    Some(
        std::iter::once(first)
            .chain(rest)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn lacks(rel: &str, section: Option<String>, name: &str, phrases: &[&str]) -> Vec<String> {
    let Some(text) = section else {
        return vec![format!("{rel}: no {name}")];
    };
    let lower = text.to_lowercase();
    phrases
        .iter()
        .filter(|p| !lower.contains(&p.to_lowercase()))
        .map(|p| format!("{rel} {name}: no `{p}`"))
        .collect()
}

// INC-I-248 REL-M2 x INC-I-235 — Decision: a failure means the documented order (fixed binary first, then the hole repair) does not leave the root open to the peer quorum, so the runbook's precondition is wrong.
#[tokio::test]
async fn first_start_then_backfill_keeps_the_root_repairable() {
    let (node, temp) = m2::stale_snap_node().await;
    assert!(
        node.sanitize_retired_maintainer_set().await,
        "first start on the fixed binary: the stale root is reset"
    );
    assert!(m2::eligible(&node).await, "after the reset: root eligible");
    backfill_the_hole(&node, &temp);
    m2::assert_start_keeps(&node, "restart after the backfill").await;
    assert!(
        m2::eligible(&node).await,
        "restart after the backfill: the EMPTY root stays open to the peer quorum"
    );
}

// INC-I-248 REL-M2 x INC-I-235 — Decision: a failure means a backfill before the first start no longer hides the hole from the start-time check; the warning in archiver §5.3 and troubleshooting §5.4 is then stale and must change with the code.
#[tokio::test]
async fn backfill_before_the_first_start_hides_the_hole_and_keeps_the_stale_root() {
    let (node, temp) = m2::stale_snap_node().await;
    backfill_the_hole(&node, &temp);
    m2::assert_start_keeps(&node, "first start after a backfill").await;
}

// INC-I-248 x INC-I-235 — Decision: a failure means an operator who follows main's hole-repair runbook on a node that was once snap-synced can fill the hole before the INC-I-248 start check runs, and keep a stale release trust root with no warning.
#[test]
fn hole_repair_docs_order_the_trust_root_start_first() {
    let archiver = read_repo_file("docs/archiver.md");
    let trouble = read_repo_file("docs/troubleshooting.md");
    let auto = read_repo_file("docs/auto_update_system.md");
    let mut bad = Vec::new();
    bad.extend(lacks(
        "docs/archiver.md",
        slice(&archiver, "### 5.3.", &["### ", "## "]),
        "§5.3",
        &[
            "INC-I-248",
            "before step 2",
            "getMaintainerSet",
            "maintainer_set_digest",
        ],
    ));
    bad.extend(lacks(
        "docs/troubleshooting.md",
        slice(&trouble, "### 1.12.", &["### ", "## ", "---"]),
        "§1.12",
        &["INC-I-248", "before the backfill"],
    ));
    bad.extend(lacks(
        "docs/troubleshooting.md",
        slice(&trouble, "### 5.4.", &["### ", "## "]),
        "§5.4",
        &["backfill", "full resync from genesis"],
    ));
    bad.extend(lacks(
        "docs/auto_update_system.md",
        slice(&auto, "- **Provenance.**", &["- **"]),
        "§2.6.1 Provenance",
        &["backfill", "restore"],
    ));
    assert!(bad.is_empty(), "hole-repair docs vs INC-I-248: {bad:#?}");
}
