//! INC-I-204 M4.2 / REQ-FORK-002 — pins on the SHAPE of the new contract.
//! TESTS-FIRST (RED). Split from `inc_i_204_m42_poison_containment.rs` for the
//! 800-line test-file budget; read that file's header for the full output contract.
//!
//! ===========================================================================
//! OUTPUT CONTRACT
//! ===========================================================================
//! UNDER TEST
//!   T1 the variant set of `RollbackAuthority`      (node/rollback_authority.rs)
//!   T2 the variant set of `RollbackOutcome`                  (node/rollback.rs)
//!   T3 `metrics::register_metrics()` zero-initialisation           (metrics.rs)
//!
//! OBSERVABLE OUTPUTS
//!   O1 which authorities are CONSTRUCTIBLE, and that no third one exists — the
//!      wildcard-free `match` in `label` makes the second half a compile error
//!   O2 `RollbackOutcome` discriminants, compared with `assert_ne!`
//!   O11 `doli_poison_containment_total{outcome}`, read back out of the RENDERED
//!      exposition text so "registered" can never pass for "exported"
//!
//! CODE PATHS   P-SHAPE the type declarations · P-ZEROINIT registration alone
//! INPUT PARTITIONS   both authority variants · all three outcome labels
//! MATRIX   O1 x P-SHAPE · O2 x P-SHAPE · O11 x P-ZEROINIT — every cell asserted
//! ANTI-VACUITY   O11 also asserts the label set is EXACTLY the three names, so a
//!   fourth series cannot appear unnoticed; O1's `match` carries no wildcard.

use doli_node::node::{RollbackAuthority, RollbackOutcome};

use super::inc_i_204_m0_common as reg;

const METRIC: &str = "doli_poison_containment_total";

// ===========================================================================
// PINS — the shape of the contract itself.
// ===========================================================================

/// REQ-FORK-002 — Decision: a failure here says someone widened the authority
/// enumeration inside M4.2. The spec reserves `ReorgPlan` and `WedgeEscape` for M6
/// (`specs/fork-lifecycle-architecture.md:286`); adding them now is never-constructed
/// dead code AND is trap T7 — bundling the rollback consolidation with the door
/// removal. The `match` below has no wildcard, so a third variant breaks the build.
#[test]
fn rollback_authority_carries_exactly_the_two_production_call_sites() {
    fn label(a: RollbackAuthority) -> &'static str {
        match a {
            RollbackAuthority::CoordinatorApproved { .. } => "coordinator_approved",
            RollbackAuthority::ProductionSelfApply { .. } => "production_self_apply",
        }
    }

    assert_eq!(
        label(RollbackAuthority::CoordinatorApproved { depth: 1 }),
        "coordinator_approved",
        "the coordinator door (periodic.rs ShallowRollback) must be constructible"
    );
    assert_eq!(
        label(RollbackAuthority::ProductionSelfApply { failed_height: 1 }),
        "production_self_apply",
        "the poison arm (production/mod.rs) must be constructible"
    );
}

/// REQ-FORK-002 — Decision: a failure here says the containment refusal was folded
/// into `RefusedNoMutation`, which makes "the poison arm was contained" indexed by
/// the same value as "at genesis / cap reached / gapped store" — the counter and the
/// log then cannot answer the only question an operator asks during an incident.
#[test]
fn refused_not_authorized_is_distinct_from_refused_no_mutation() {
    assert_ne!(
        RollbackOutcome::RefusedNotAuthorized,
        RollbackOutcome::RefusedNoMutation
    );
    assert_ne!(
        RollbackOutcome::RefusedNotAuthorized,
        RollbackOutcome::RolledBack
    );
}

/// REQ-FORK-002 — Decision: a failure here says the containment counter is registered
/// and never exported, which is the INC-I-187 class this repo already carries for 28
/// of 57 `doli_*` metrics: an alert bound to a series that does not exist has nothing
/// to evaluate, so the first contained poison arm in a live incident is silent.
#[test]
fn poison_containment_outcomes_are_zero_initialised_and_exported() {
    reg::ensure_registered();
    for outcome in ["tip_kept", "rolled_back", "rollback_failed"] {
        assert!(
            reg::exported_value(METRIC, &[("outcome", outcome)]).is_some(),
            "`{METRIC}{{outcome=\"{outcome}\"}}` must EXIST in the exposition text \
             before the first poison arm ever runs"
        );
    }
    let exported = reg::exported_label_values(METRIC, "outcome");
    assert_eq!(
        exported,
        vec![
            "rollback_failed".to_string(),
            "rolled_back".to_string(),
            "tip_kept".to_string()
        ],
        "no fourth label may appear: a typo'd outcome string creates a phantom series \
         no alert rule was ever able to bind"
    );
}
