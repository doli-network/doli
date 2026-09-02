//! INC-I-204 M0 / D3 — which branch the FORK_GUARD wedge escape took.
//!
//! `retain_sibling_and_try_escape` collapsed `plan_reorg -> None` (a guard
//! STOPPED us) and `Some(weight_delta <= 0)` (the fork simply has not won) into
//! one log line. This classifier names them apart so each is countable. It is
//! pure and decides nothing: the caller runs the same arms in the same order.

use network::ReorgResult;

/// The branch `retain_sibling_and_try_escape` took, one variant per outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WedgeOutcome {
    /// `fork_weight <= our_weight` — the short-circuit, checked first.
    CannotOutweigh,
    /// `plan_reorg` returned `None`: finality veto, unknown ancestor, or depth.
    PlanRefused,
    /// A plan exists but the fork is not strictly heavier.
    NotHeavier,
    /// A plan exists and the fork is strictly heavier — reorg attempted.
    Reorg,
    /// The reorg was attempted but the tip did not move (evicted body).
    ReorgDidNotLand,
}

impl WedgeOutcome {
    /// The `reason` label value for `doli_wedge_escape_outcomes_total`.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::CannotOutweigh => "cannot_outweigh",
            Self::PlanRefused => "plan_refused",
            Self::NotHeavier => "not_heavier",
            Self::Reorg => "reorg",
            Self::ReorgDidNotLand => "reorg_did_not_land",
        }
    }
}

/// Every outcome, so `register_metrics` can zero-initialise one series each.
pub const ALL_WEDGE_OUTCOMES: [WedgeOutcome; 5] = [
    WedgeOutcome::CannotOutweigh,
    WedgeOutcome::PlanRefused,
    WedgeOutcome::NotHeavier,
    WedgeOutcome::Reorg,
    WedgeOutcome::ReorgDidNotLand,
];

/// Reproduce `retain_sibling_and_try_escape`'s branch selection exactly.
///
/// The `<=` short-circuit (T9) and the strict `weight_delta > 0` reorg rule
/// (AUDIT-P2-003) are load-bearing boundaries — M0 counts them, never moves them.
/// `ReorgDidNotLand` is not reachable here: it is only known after `execute_reorg`.
pub fn classify_wedge_plan(
    fork_weight: u64,
    our_weight: u64,
    plan: Option<&ReorgResult>,
) -> WedgeOutcome {
    if fork_weight <= our_weight {
        return WedgeOutcome::CannotOutweigh;
    }
    match plan {
        None => WedgeOutcome::PlanRefused,
        Some(p) if p.weight_delta > 0 => WedgeOutcome::Reorg,
        Some(_) => WedgeOutcome::NotHeavier,
    }
}
