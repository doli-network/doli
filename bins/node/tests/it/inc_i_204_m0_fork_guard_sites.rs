//! INC-I-204 M0 / D1 — every FORK_GUARD refusal site carries its own `site` label.
//!
//! `doli_fork_guard_refusals_total` exists with a `site` label, but only
//! `rewards.rs:1227` (`site="producer_rebuild"`) ever increments it. The sibling
//! refusals in `rollback.rs:188` and `block_handling.rs:629` are silent, so a zero
//! for those sites means "not instrumented", not "no refusals". The metric's own
//! HELP text says so — and that sentence becomes FALSE the moment the sites land,
//! which is how an operator misreads a zero as health.
//!
//! OUTPUT CONTRACT
//!   Functions under test:
//!     F1 `Node::rebuild_producer_set_from_blocks(&self, &mut ProducerSet, u64) -> Result<()>`
//!     F2 `Node::rollback_one_block(&mut self) -> Result<RollbackOutcome>`
//!     F3 `Node::execute_reorg(&mut self, ReorgResult, Block) -> Result<()>`
//!     F4 `metrics::register_metrics()` (the HELP text it publishes)
//!   OBSERVABLE OUTPUTS asserted:
//!     O1: exported series `doli_fork_guard_refusals_total{site="producer_rebuild"}` > 0
//!     O2: exported series `doli_fork_guard_refusals_total{site="rollback_rebuild"}` > 0
//!     O3: exported series `doli_fork_guard_refusals_total{site="reorg_execute"}` > 0
//!     O4: exported HELP text of `doli_fork_guard_refusals_total`
//!     O5: the refusal itself — `Err` / `Ok(RefusedNoMutation)` with the
//!         `[FORK_GUARD_BACKFILL_REQUIRED]` decision UNCHANGED (M0 is observability
//!         only; LB-1 says the refusal is the hero)
//!   CODE PATHS:
//!     P1: `ensure_blocks_present(1, target_height.max(1))` returns Err  -> refusal
//!     P2: same guard returns Ok                                        -> no refusal
//!   INPUT PARTITIONS:
//!     I1: dense block store, rebuild over 1..=TARGET  (P2) — control, no series
//!     I2: HOLED block store, producer rebuild         (P1) — O1, O5
//!     I3: HOLED block store, no undo data, rollback   (P1) — O2, O5
//!     I4: HOLED block store, reorg execute            (P1) — O3, O5
//!   MATRIX:
//!     d1_producer_rebuild_refusal_reaches_the_exported_registry : O1,O5 x P1 x I2 (+P2 x I1)
//!     d1_rollback_rebuild_refusal_reaches_the_exported_registry : O2,O5 x P1 x I3
//!     d1_reorg_execute_refusal_reaches_the_exported_registry    : O3,O5 x P1 x I4
//!     d1_fork_guard_help_must_not_claim_a_single_instrumented_site : O4 x F4
//!
//! Counters are process-global and cargo runs this binary's tests in parallel, so
//! every assertion is a BEFORE/AFTER delta on a monotonic counter. No test asserts
//! an absolute value.

#[path = "../inc_i_156_m1_harness/mod.rs"]
mod harness;

use crypto::Hash;
use doli_core::validation::ValidationMode;
use doli_node::metrics::FORK_GUARD_REFUSALS;
use doli_node::node::Node;
use doli_node::node::RollbackOutcome;
use network::ReorgResult;
use storage::ProducerSet;
use tempfile::TempDir;

use super::inc_i_204_m0_common as reg;
use harness as h;

const FORK_GUARD_METRIC: &str = "doli_fork_guard_refusals_total";

const CHAIN_LEN: u64 = 12;
const TARGET_HEIGHT: u64 = CHAIN_LEN - 1;
/// Inside `1..=TARGET_HEIGHT`, so `ensure_blocks_present` refuses on every path.
const HOLE_HEIGHT: u64 = 5;

// ==================== fixtures ====================

async fn dense_node() -> (Node, Vec<crypto::KeyPair>, TempDir) {
    reg::ensure_registered();
    let (mut node, producers, temp) = h::make_node(3).await;
    let params = node.params.clone();
    h::install_production_utxo_backend(&node).await;
    h::apply_plain_up_to(&mut node, &producers, CHAIN_LEN, &params).await;
    assert_eq!(
        node.chain_state.read().await.best_height,
        CHAIN_LEN,
        "fixture: chain must reach CHAIN_LEN"
    );
    (node, producers, temp)
}

/// Remove a canonical height entry so `ensure_blocks_present(1, TARGET_HEIGHT)` fails.
fn punch_hole(node: &Node, height: u64) {
    let hash = node
        .block_store
        .get_hash_by_height(height)
        .expect("block_store get_hash_by_height failed")
        .unwrap_or_else(|| panic!("fixture: expected a canonical entry at h={height}"));
    node.block_store
        .remove_canonical_entry(height, hash)
        .expect("remove_canonical_entry failed");
    assert!(
        node.block_store
            .get_block_by_height(height)
            .expect("block_store read failed")
            .is_none(),
        "fixture: h={height} must be invisible after the hole is punched"
    );
}

/// Read the counter handle. Used ONLY to compute a delta; the acceptance assertion
/// always reads the exported text via `reg::assert_exported_nonzero`.
fn refusals_at(site: &str) -> u64 {
    FORK_GUARD_REFUSALS.with_label_values(&[site]).get()
}

// ==================== O4 — the HELP text ====================

/// REQ-FORK-016 — Decision: without a series per site the alert expression has
/// nothing to evaluate on a healthy node and must fall back to `absent()`, which is
/// how a not-yet-fired alarm and a broken exporter become indistinguishable.
///
/// Assertion-RED. A labelled family publishes NO series until a label value is
/// first touched (INC-I-154, metrics.rs:576-587), so `register_metrics` must
/// zero-initialise every known site the way it already does for
/// `ATTESTATION_MISSES_TOTAL`. Writing is proven separately by the three driven
/// tests below — this one only buys alert-evaluability.
#[test]
fn d1_fork_guard_every_known_site_is_exported_before_any_refusal() {
    reg::ensure_registered();
    let sites = reg::exported_label_values(FORK_GUARD_METRIC, "site");
    for expected in ["producer_rebuild", "rollback_rebuild", "reorg_execute"] {
        assert!(
            sites.iter().any(|s| s == expected),
            "`{FORK_GUARD_METRIC}{{site=\"{expected}\"}}` is not exported after \
             register_metrics(). An alert on this site has no series to evaluate. \
             Exported sites: {sites:?}"
        );
    }
}

/// REQ-FORK-016 — Decision: a stale HELP text that still says only one site is
/// instrumented makes an operator read a zero on the new sites as health, which is
/// the exact misreading that let INC-I-204 run for 7 days.
///
/// Assertion-RED: compiles and runs today, fails on the assertion.
#[test]
fn d1_fork_guard_help_must_not_claim_a_single_instrumented_site() {
    // A labelled family is collectable only once a label value exists. Touch the
    // one site that is instrumented today, by +0, so this test judges the HELP
    // sentence and not the INC-I-154 presence problem the test above owns.
    reg::ensure_registered();
    FORK_GUARD_REFUSALS
        .with_label_values(&["producer_rebuild"])
        .inc_by(0);

    let help = reg::help_text(FORK_GUARD_METRIC)
        .unwrap_or_else(|| panic!("{FORK_GUARD_METRIC} publishes no family at all"));

    assert!(
        !help.contains("Only site=\"producer_rebuild\" is instrumented"),
        "O4: the HELP text still claims `producer_rebuild` is the only instrumented \
         site. Once rollback.rs and block_handling.rs increment, that sentence is a \
         lie an operator acts on. Got: {help}"
    );
    assert!(
        !help.contains("not-instrumented"),
        "O4: the HELP text still tells operators a zero may mean not-instrumented. \
         After D1 every refusal site increments, so a zero means no refusals. Got: {help}"
    );
}

// ==================== O1 — control: the one site that works today ====================

/// REQ-FORK-016 — Decision: proves the scrape path itself (real refusal -> counter
/// -> REGISTRY.gather -> exposition text) works, so a failure on the sibling sites
/// below is attributable to missing instrumentation and not to a broken harness.
///
/// GREEN today (control). It must stay green after D1.
#[tokio::test]
async fn d1_producer_rebuild_refusal_reaches_the_exported_registry() {
    let (node, _p, _tmp) = dense_node().await;

    // P2 / I1: a DENSE store must not increment anything — a counter that fires on
    // the healthy path is an alarm that means nothing.
    let dense_before = refusals_at("producer_rebuild");
    {
        let mut producers = node.producer_set.write().await;
        node.rebuild_producer_set_from_blocks(&mut producers, TARGET_HEIGHT)
            .expect("O5: a dense store must NOT be refused");
    }
    assert_eq!(
        refusals_at("producer_rebuild"),
        dense_before,
        "O1/P2: the dense path incremented the refusal counter — the metric would \
         alarm on healthy nodes"
    );

    // P1 / I2.
    punch_hole(&node, HOLE_HEIGHT);
    let result = {
        let mut producers = node.producer_set.write().await;
        node.rebuild_producer_set_from_blocks(&mut producers, TARGET_HEIGHT)
    };
    let err = result.expect_err("O5: a holed store must still be REFUSED (LB-1)");
    assert!(
        format!("{err}").contains("[FORK_GUARD_BACKFILL_REQUIRED]"),
        "O5: the refusal must keep its runbook token. Got: {err}"
    );

    reg::assert_exported_nonzero(
        FORK_GUARD_METRIC,
        &[("site", "producer_rebuild")],
        "O1: producer_rebuild",
    );
}

// ==================== O2 — rollback.rs:188 ====================

/// REQ-FORK-016 — Decision: a wedged node refusing rollback rebuilds is invisible
/// to Prometheus today, so the operator learns of it only from a log file nobody
/// tails; a failure here means the wedge stays log-only.
///
/// Assertion-RED: compiles and runs today, fails because rollback.rs:188 never
/// touches the counter.
#[tokio::test]
async fn d1_rollback_rebuild_refusal_reaches_the_exported_registry() {
    let (mut node, _p, _tmp) = dense_node().await;

    // The no-undo legacy fallback (rollback.rs:155) is the only path that reaches
    // the guard at :188. `prune_undo_above(0)` is what a snap-synced node looks like.
    node.state_db.prune_undo_above(0);
    assert!(
        node.state_db.get_undo(CHAIN_LEN).is_none(),
        "fixture: undo data at h={CHAIN_LEN} must be absent so rollback takes the \
         rebuild-from-genesis fallback"
    );
    punch_hole(&node, HOLE_HEIGHT);

    let before = refusals_at("rollback_rebuild");
    let outcome = node
        .rollback_one_block(doli_node::node::RollbackAuthority::CoordinatorApproved { depth: 1 })
        .await
        .expect("rollback_one_block must not error on the refusal path");

    // O5 — the DECISION is pinned: the guard refuses and mutates nothing. M0 counted
    // this branch without changing it; INC-I-204 M3 kept the decision identical and
    // changed only what it REPORTS — `Ok(true)` (indistinguishable from a real
    // rollback) became `RefusedNoMutation`, so the caller stops burning a
    // rollback-budget rung on a no-op.
    assert_eq!(
        outcome,
        RollbackOutcome::RefusedNoMutation,
        "O5: the refusal path must mutate nothing and SAY so"
    );

    assert!(
        refusals_at("rollback_rebuild") > before,
        "O2: `{FORK_GUARD_METRIC}{{site=\"rollback_rebuild\"}}` did not increment. \
         rollback.rs:188 refuses the rebuild and says nothing to Prometheus."
    );
    reg::assert_exported_nonzero(
        FORK_GUARD_METRIC,
        &[("site", "rollback_rebuild")],
        "O2: rollback_rebuild",
    );
}

// ==================== O3 — block_handling.rs:629 ====================

/// REQ-FORK-016 — Decision: the reorg-execute refusal is the site closest to the
/// INC-I-204 wedge itself; a failure here means the fleet-wide refusal count an
/// alert would key on is systematically under-reported.
///
/// Assertion-RED: compiles and runs today, fails because block_handling.rs:629
/// never touches the counter.
#[tokio::test]
async fn d1_reorg_execute_refusal_reaches_the_exported_registry() {
    let (mut node, producers, _tmp) = dense_node().await;
    let params = node.params.clone();

    let tip_hash = node.chain_state.read().await.best_hash;
    let ancestor_hash: Hash = node
        .block_store
        .get_hash_by_height(TARGET_HEIGHT)
        .expect("get_hash_by_height failed")
        .expect("fixture: ancestor must exist");

    // A one-block sibling branch off TARGET_HEIGHT, cached so execute_reorg can find it.
    let sibling = h::build_block_with_txs(
        (CHAIN_LEN + 500) as u32,
        ancestor_hash,
        &producers[0],
        &params,
        vec![h::coinbase_for(
            CHAIN_LEN,
            (CHAIN_LEN + 500) as u32,
            &params,
        )],
    );
    node.fork_block_cache
        .write()
        .await
        .insert(sibling.hash(), sibling.clone());

    punch_hole(&node, HOLE_HEIGHT);

    let plan = ReorgResult {
        rollback: vec![tip_hash],
        common_ancestor: ancestor_hash,
        new_blocks: vec![sibling.hash()],
        weight_delta: 1,
    };

    let before = refusals_at("reorg_execute");
    let err = node
        .execute_reorg(plan, sibling)
        .await
        .expect_err("O5: a holed store must still refuse the reorg (LB-1)");
    assert!(
        format!("{err}").contains("[FORK_GUARD_BACKFILL_REQUIRED]"),
        "O5: the refusal must keep its runbook token. Got: {err}"
    );

    assert!(
        refusals_at("reorg_execute") > before,
        "O3: `{FORK_GUARD_METRIC}{{site=\"reorg_execute\"}}` did not increment. \
         block_handling.rs:629 refuses the reorg and says nothing to Prometheus."
    );
    reg::assert_exported_nonzero(
        FORK_GUARD_METRIC,
        &[("site", "reorg_execute")],
        "O3: reorg_execute",
    );
}

// Silence unused-import lints if a partition is trimmed during review.
#[allow(dead_code)]
fn _touch(_: ValidationMode, _: ProducerSet) {}
