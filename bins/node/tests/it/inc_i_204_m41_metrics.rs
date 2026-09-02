//! INC-I-204 M4.1 / REQ-FORK-012 — the escape's outcome counter must be EXPORTED,
//! not merely declared. TESTS-FIRST (RED).
//!
//! REQ-FORK-012 — Decision: a failure means the audited door has no audit trail —
//! 28 of 57 `doli_*` metrics in this codebase are registered and never written and
//! read 0 on healthy nodes (INC-I-187), and an alert on a series that does not
//! exist has nothing to evaluate, so a refused rescue would be silent.
//!
//! OUTPUT CONTRACT — `fn metrics::register_metrics()` +
//!                   `async fn Node::try_consume_force_reorg(&mut self)`
//!   O1 none.  O2 (node) covered by the sibling modules.
//!   O3 none for `register_metrics`.
//!   O4 none.
//!   O5 GLOBAL/STATIC — the only output asserted here:
//!      `doli_force_reorg_outcomes_total{outcome=..}`, read back out of the
//!      RENDERED exposition text so "registered" can never pass for "exported".
//!   O6 none.
//!   PATHS: P-ZEROINIT registration alone publishes one series per outcome ·
//!          P-WRITE the real escape path moves the series it names.
//!   INPUT PARTITIONS: every value of `ForceReorgOutcome::ALL`; one live refusal.
//!   MATRIX: O5 x {P-ZEROINIT, P-WRITE} — both cells asserted.

use doli_node::node::ForceReorgOutcome;

use super::inc_i_204_m0_common as reg;
use super::inc_i_204_m41_common::wedged_node;

const FAMILY: &str = "doli_force_reorg_outcomes_total";

/// REQ-FORK-012 — Decision: a failure means an operator who has never used the
/// escape sees no series at all, so the first refusal in a live incident produces a
/// metric that appears from nowhere and no alert rule was ever able to bind it.
#[test]
fn every_force_reorg_outcome_is_zero_initialised_and_exported() {
    reg::ensure_registered();

    assert!(
        !ForceReorgOutcome::ALL.is_empty(),
        "the outcome enumeration must be non-empty or this test is vacuous"
    );
    for outcome in ForceReorgOutcome::ALL {
        assert!(
            reg::exported_value(FAMILY, &[("outcome", outcome.label())]).is_some(),
            "INC-I-187: `{FAMILY}{{outcome=\"{}\"}}` must be zero-initialised in \
             register_metrics() so the series EXISTS before the first escape",
            outcome.label()
        );
    }

    // Every exported label value is a real outcome — no typo'd string can create a
    // phantom series that an alert would then have to know about.
    let labels: Vec<String> = ForceReorgOutcome::ALL
        .iter()
        .map(|o| o.label().to_string())
        .collect();
    for exported in reg::exported_label_values(FAMILY, "outcome") {
        assert!(
            labels.contains(&exported),
            "exported label `{exported}` is not a ForceReorgOutcome variant"
        );
    }
}

/// REQ-FORK-012 — Decision: a failure means the counter is decorative: the escape
/// runs, refuses, and the operator's dashboard still reads 0, which is exactly the
/// INC-I-187 defect class this milestone must not add to.
#[tokio::test]
async fn a_real_refusal_moves_the_exported_counter() {
    reg::ensure_registered();
    let label = ForceReorgOutcome::UnknownTarget.label();
    let before = reg::exported_value(FAMILY, &[("outcome", label)])
        .expect("the series must exist before the escape runs (zero-init)");

    let mut w = wedged_node().await;
    w.node
        .sync_manager
        .write()
        .await
        .arm_force_reorg(crypto::hash::hash(b"inc_i204_m41_metric_probe"));
    assert_eq!(
        w.node.try_consume_force_reorg().await,
        Some(ForceReorgOutcome::UnknownTarget),
        "drive the REAL producer of the value, never the counter handle"
    );

    let after = reg::exported_value(FAMILY, &[("outcome", label)])
        .expect("the series must still be exported after the escape");
    // `>=` not `==`: the family is process-global and sibling tests run in parallel,
    // but a counter is monotone so a missed increment still fails.
    assert!(
        after >= before + 1.0,
        "{FAMILY}{{outcome=\"{label}\"}} must advance when the escape reaches that \
         outcome (before={before}, after={after})"
    );
}
