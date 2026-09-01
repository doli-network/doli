//! INC-I-204 M0 / D2-D6 — every metric M0 adds must be WRITTEN, not just declared.
//!
//! 28 of 57 `doli_*` metrics in this codebase are registered and never written and
//! read 0 on healthy nodes (INC-I-187). So no test here is satisfiable by
//! registration: each drives the REAL producer of the value — `checkpoint_health`,
//! `classify_wedge_plan`, `record_block_with_height`, `check_reorg_weighted`,
//! `plan_reorg` — and then reads the value back out of the RENDERED exposition text.
//!
//! `crates/network` does not depend on `prometheus` (only `bins/node` does), so the
//! network-side sites expose PLAIN counters and `bins/node` scrapes them. That is
//! the `storage::RocksDbMetrics` -> `metrics::apply_rocksdb_metrics` seam
//! (metrics.rs:681), reused verbatim.
//!
//! OUTPUT CONTRACT
//!   Functions under test:
//!     F1 `metrics::update_unique_chain_tips(usize)`            fed by `SyncManager::checkpoint_health`
//!     F2 `metrics::record_wedge_escape_outcome(&str)`          fed by `node::wedge_outcome::classify_wedge_plan`
//!     F3 `metrics::apply_reorg_observations(&mut ReorgScrapeState, &ReorgObservations)`
//!                                                              fed by `ReorgHandler::observations()`
//!     F4 `node::wedge_outcome::classify_wedge_plan(fork_w, our_w, Option<&ReorgResult>) -> WedgeOutcome`
//!     F5 `ReorgHandler::observations(&self) -> ReorgObservations`
//!   OBSERVABLE OUTPUTS asserted:
//!     O1: exported `doli_unique_chain_tips` (gauge, no labels)
//!     O2: exported `doli_wedge_escape_outcomes_total{reason=..}` — 5 distinct reasons
//!     O3: exported `doli_pre_activation_branch_total{gate=..}`
//!     O4: exported `doli_reorg_finality_probe_total{site=..,outcome=..}`
//!     O5: F4 return value — the reason enum itself
//!     O6: F5 return value — the plain per-handler counters
//!   CODE PATHS:
//!     P1: fleet agrees            -> unique_chain_tips == 1
//!     P2: fleet diverges          -> unique_chain_tips > 1
//!     P3: fork cannot out-weight tip (`fork_w <= our_w`, wedge_escape.rs:130)
//!     P4: plan_reorg returned None (finality veto / no ancestor / depth)
//!     P5: plan present, `weight_delta <= 0`
//!     P6: plan present, `weight_delta > 0`, tip landed
//!     P7: plan present, `weight_delta > 0`, tip did NOT land (missing body)
//!     P8: `record_block_with_height` below the activation height
//!     P9: `record_block_with_height` at/above the activation height
//!     P10: `check_reorg_weighted` reaches the finality comparison and REJECTS
//!     P11: `check_reorg_weighted` reaches the finality comparison and ACCEPTS
//!   INPUT PARTITIONS:
//!     I1: 2 agreeing peers | 2 agreeing + 2 distinct divergent peers
//!     I2: one input per wedge outcome P3..P7
//!     I3: real_height = activation-1 | activation
//!     I4: finality above the synthetic ancestor height | at it
//!   MATRIX:
//!     d2_unique_chain_tips_gauge_carries_what_checkpoint_health_computes : O1 x P2 x I1
//!     d2_unique_chain_tips_gauge_clears_when_the_fleet_agrees            : O1 x P1 x I1
//!     d3_every_wedge_outcome_is_separately_countable                     : O2,O5 x P3..P7 x I2
//!     d3_plan_refused_is_distinguishable_from_not_heavier                : O5 x P4,P5
//!     d3_classification_matches_the_live_branch_boundaries               : O5 x P3,P5,P6 (T9 pin)
//!     d4_pre_activation_canary_fires_below_the_activation_height         : O3,O6 x P8 x I3
//!     d4_pre_activation_canary_is_silent_at_and_above_activation         : O6 x P9 x I3
//!     d5_finality_probe_counts_entries_and_rejects_separately            : O6 x P10,P11 x I4
//!     d5_finality_probe_reaches_the_exported_registry                    : O4 x P10 x I4
//!
//! COMPILE-RED: every symbol below that does not exist yet is named deliberately —
//! it IS the contract. See docs/.workflow/inc-i-204-M0-test-plan.md for signatures.

use crypto::{Hash, PublicKey};
use doli_core::{Block, BlockHeader};
use doli_node::metrics::{apply_reorg_observations, ReorgScrapeState};
use doli_node::node::wedge_outcome::{classify_wedge_plan, WedgeOutcome};
use network::sync::{ReorgHandler, ReorgResult, SyncConfig, SyncManager};
use network::PeerId;

use super::inc_i_204_m0_common as reg;

const TIPS_GAUGE: &str = "doli_unique_chain_tips";
const WEDGE_OUTCOMES: &str = "doli_wedge_escape_outcomes_total";
const PRE_ACTIVATION: &str = "doli_pre_activation_branch_total";
const FINALITY_PROBE: &str = "doli_reorg_finality_probe_total";

fn h(tag: &[u8]) -> Hash {
    crypto::hash::hash(tag)
}

fn block_on(prev_hash: Hash, slot: u32) -> Block {
    let header = BlockHeader {
        version: 1,
        prev_hash,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot,
        producer: PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![])
}

fn plan(weight_delta: i64) -> ReorgResult {
    ReorgResult {
        rollback: vec![h(b"rb")],
        common_ancestor: h(b"anc"),
        new_blocks: vec![h(b"new")],
        weight_delta,
    }
}

// ==================== D2 — the fleet fork gauge ====================

/// REQ-FORK-016 — Decision: a failure means the number periodic.rs:1076 already
/// computes still dies in a log line, and a fleet fork stays invisible to the
/// alerting stack that would have caught INC-I-204 on day one.
#[test]
fn d2_unique_chain_tips_gauge_carries_what_checkpoint_health_computes() {
    reg::ensure_registered();
    let ours = h(b"d2-our-tip");
    let mut mgr = SyncManager::new(SyncConfig::default(), ours);
    mgr.update_local_tip(20, ours, 20);
    mgr.add_peer(PeerId::random(), 20, ours, 20);
    mgr.add_peer(PeerId::random(), 20, h(b"d2-divergent-1"), 20);
    mgr.add_peer(PeerId::random(), 20, h(b"d2-divergent-2"), 20);

    // The REAL producer of the value, not a literal.
    let (_counted, _agreeing, tips) = mgr.checkpoint_health();
    assert_eq!(tips, 3, "fixture: 1 ours + 2 distinct divergent hashes");

    doli_node::metrics::update_unique_chain_tips(tips);

    assert_eq!(
        reg::exported_value(TIPS_GAUGE, &[]),
        Some(3.0),
        "O1/P2: `{TIPS_GAUGE}` must export the value `checkpoint_health` computed. \
         A gauge that is registered and never set reads 0 on a forked fleet."
    );
}

/// REQ-FORK-016 — Decision: a gauge that only ever climbs cannot distinguish a
/// resolved fork from an ongoing one, so the alert would never clear and operators
/// would learn to ignore it.
#[test]
fn d2_unique_chain_tips_gauge_clears_when_the_fleet_agrees() {
    reg::ensure_registered();
    let ours = h(b"d2b-our-tip");
    let mut mgr = SyncManager::new(SyncConfig::default(), ours);
    mgr.update_local_tip(20, ours, 20);
    mgr.add_peer(PeerId::random(), 20, h(b"d2b-divergent"), 20);
    doli_node::metrics::update_unique_chain_tips(mgr.checkpoint_health().2);

    let mut agreed = SyncManager::new(SyncConfig::default(), ours);
    agreed.update_local_tip(20, ours, 20);
    agreed.add_peer(PeerId::random(), 20, ours, 20);
    let tips = agreed.checkpoint_health().2;
    assert_eq!(tips, 1, "fixture: an agreeing fleet is 1 tip");
    doli_node::metrics::update_unique_chain_tips(tips);

    assert_eq!(
        reg::exported_value(TIPS_GAUGE, &[]),
        Some(1.0),
        "O1/P1: the gauge must fall back to 1 when the fork resolves"
    );
}

// ==================== D3 — the wedge_escape reason split ====================

/// REQ-FORK-016 — Decision: today wedge_escape.rs:178's `_` arm logs ONE message
/// for two unrelated causes; a failure means an operator still cannot tell a
/// finality veto from a merely-lighter sibling, which is the diagnosis INC-I-204
/// needed and did not have.
#[test]
fn d3_plan_refused_is_distinguishable_from_not_heavier() {
    // P4: plan_reorg returned None — finality veto, unknown ancestor, or depth
    // exceeded. The node is being STOPPED by a guard.
    let refused = classify_wedge_plan(10, 5, None);
    // P5: a plan exists but the fork is not strictly heavier. Nothing is stopping
    // us; the fork simply has not won yet.
    let not_heavier = classify_wedge_plan(10, 5, Some(&plan(0)));

    assert_ne!(
        refused, not_heavier,
        "O5: `plan_reorg -> None` and `plan_reorg -> Some(delta <= 0)` are \
         DIFFERENT causes and must classify differently. Collapsing them into one \
         message is the INC-I-204 diagnostic gap."
    );
    assert_eq!(refused, WedgeOutcome::PlanRefused, "O5/P4");
    assert_eq!(not_heavier, WedgeOutcome::NotHeavier, "O5/P5");
}

/// REQ-FORK-016 — Decision: a failure means at least one wedge outcome has no
/// series of its own, so the ratio that identifies WHICH wedge a node is in cannot
/// be computed from the metrics alone.
#[test]
fn d3_every_wedge_outcome_is_separately_countable() {
    reg::ensure_registered();

    let cases = [
        // P3: wedge_escape.rs:130 — fork_weight <= our_weight short-circuit.
        (
            classify_wedge_plan(5, 5, None),
            WedgeOutcome::CannotOutweigh,
        ),
        // P4 / P5.
        (classify_wedge_plan(10, 5, None), WedgeOutcome::PlanRefused),
        (
            classify_wedge_plan(10, 5, Some(&plan(0))),
            WedgeOutcome::NotHeavier,
        ),
        // P6: the reorg arm.
        (
            classify_wedge_plan(10, 5, Some(&plan(3))),
            WedgeOutcome::Reorg,
        ),
    ];
    for (got, want) in cases {
        assert_eq!(got, want, "O5: classification");
        doli_node::metrics::record_wedge_escape_outcome(got.reason());
    }
    // P7: execute_reorg no-ops when an intermediate body was evicted
    // (wedge_escape.rs:171). Post-execute, so it is recorded directly.
    doli_node::metrics::record_wedge_escape_outcome(WedgeOutcome::ReorgDidNotLand.reason());

    for reason in [
        "cannot_outweigh",
        "plan_refused",
        "not_heavier",
        "reorg",
        "reorg_did_not_land",
    ] {
        reg::assert_exported_nonzero(WEDGE_OUTCOMES, &[("reason", reason)], "O2: wedge outcome");
    }
}

/// T9 / REQ-FORK-014 — Decision: a failure means extracting the classifier moved a
/// fork-choice boundary; trap T9 (changing `fork_w <= our_w` ungated) is REJECTED,
/// and M0 may only COUNT these branches.
#[test]
fn d3_classification_matches_the_live_branch_boundaries() {
    // wedge_escape.rs:130 is `fork_weight <= our_weight` — EQUAL takes the
    // short-circuit. One-off either way is the whole trap.
    assert_eq!(
        classify_wedge_plan(5, 5, Some(&plan(9))),
        WedgeOutcome::CannotOutweigh,
        "T9: equal weight must short-circuit BEFORE plan_reorg is consulted"
    );
    assert_ne!(
        classify_wedge_plan(6, 5, Some(&plan(9))),
        WedgeOutcome::CannotOutweigh,
        "T9: one unit heavier must NOT short-circuit"
    );
    // wedge_escape.rs:159 is `result.weight_delta > 0` — STRICT (AUDIT-P2-003).
    assert_eq!(
        classify_wedge_plan(10, 5, Some(&plan(0))),
        WedgeOutcome::NotHeavier,
        "T9: weight_delta == 0 must NOT reorg — the equal-weight tie-break stays \
         out of the unsolicited-gossip path"
    );
    assert_eq!(
        classify_wedge_plan(10, 5, Some(&plan(1))),
        WedgeOutcome::Reorg,
        "T9: weight_delta == 1 reorgs"
    );
}

// ==================== D4 — the INV-SYNC-012 canary ====================

/// REQ-FORK-014 / INV-SYNC-012 — Decision: a failure means a node running below an
/// activation height on a live network is silent about it, which is exactly the
/// 7-day unmonitored dormant window INC-I-204 sat in.
#[test]
fn d4_pre_activation_canary_fires_below_the_activation_height() {
    reg::ensure_registered();
    const ACTIVATION: u64 = 1_000;

    let mut handler = ReorgHandler::with_activation_height(ACTIVATION);
    handler.record_block_with_height(h(b"d4-below"), Hash::ZERO, 1, ACTIVATION - 1);

    let obs = handler.observations();
    assert_eq!(
        obs.pre_activation_record_height, 1,
        "O6/P8: executing the pre-activation branch must be counted per handler"
    );

    let mut state = ReorgScrapeState::new();
    apply_reorg_observations(&mut state, &obs);
    reg::assert_exported_nonzero(
        PRE_ACTIVATION,
        &[("gate", "inc_i_147_record_height")],
        "O3/P8: pre-activation canary",
    );
}

/// REQ-FORK-014 — Decision: a canary that fires above the activation height too is
/// noise, and an alert nobody can act on is worse than none. Asserted on the
/// PER-HANDLER counter so a parallel test cannot pollute the zero.
#[test]
fn d4_pre_activation_canary_is_silent_at_and_above_activation() {
    const ACTIVATION: u64 = 1_000;

    let mut at = ReorgHandler::with_activation_height(ACTIVATION);
    at.record_block_with_height(h(b"d4-at"), Hash::ZERO, 1, ACTIVATION);
    assert_eq!(
        at.observations().pre_activation_record_height,
        0,
        "O6/P9: at the activation height the post-activation branch runs — no canary"
    );

    let mut above = ReorgHandler::with_activation_height(ACTIVATION);
    above.record_block_with_height(h(b"d4-above"), Hash::ZERO, 1, ACTIVATION + 5_000);
    assert_eq!(
        above.observations().pre_activation_record_height,
        0,
        "O6/P9: above the activation height there is no canary either"
    );
}

// ==================== D5 — the check_reorg_weighted probe ====================

/// INV-SYNC-012 / REQ-FORK-016 — Decision: the "0 rejects measured on all specimen
/// nodes" premise is what lets M5 delete this site; a failure means the premise
/// stops being measured in the field and M5 would be deleting on a stale claim.
#[test]
fn d5_finality_probe_counts_entries_and_rejects_separately() {
    let a1 = h(b"d5-a1");
    let a2 = h(b"d5-a2");
    let heavy = block_on(a1, 9);

    // P10 / I4: synthetic ancestor height 1 < finality 5 -> ENTRY and REJECT.
    let mut rejecting = ReorgHandler::new();
    rejecting.record_block_with_weight(a1, Hash::ZERO, 10);
    rejecting.record_block_with_weight(a2, a1, 10);
    rejecting.set_last_finality_height(5);
    assert!(
        rejecting.check_reorg_weighted(&heavy, a2, 30).is_none(),
        "decision pin: the reject itself is unchanged (see network decision pins)"
    );
    let obs = rejecting.observations();
    assert_eq!(
        obs.check_reorg_finality_entries, 1,
        "O6/P10: reaching the finality comparison counts as an ENTRY"
    );
    assert_eq!(
        obs.check_reorg_finality_rejects, 1,
        "O6/P10: the refusal counts as a REJECT"
    );

    // P11 / I4: same input, finality at the synthetic height -> ENTRY, no REJECT.
    // Without this cell an implementation that increments both counters on every
    // entry would pass the assertions above.
    let mut accepting = ReorgHandler::new();
    accepting.record_block_with_weight(a1, Hash::ZERO, 10);
    accepting.record_block_with_weight(a2, a1, 10);
    accepting.set_last_finality_height(1);
    assert!(
        accepting.check_reorg_weighted(&heavy, a2, 30).is_some(),
        "decision pin: the approval itself is unchanged"
    );
    let obs = accepting.observations();
    assert_eq!(
        obs.check_reorg_finality_entries, 1,
        "O6/P11: an approved fork still ENTERS the comparison"
    );
    assert_eq!(
        obs.check_reorg_finality_rejects, 0,
        "O6/P11: an approved fork must NOT count as a reject — entries and rejects \
         are independent, or the measured reject RATE is meaningless"
    );
}

/// REQ-FORK-016 — Decision: a failure means the probe's numbers never leave the
/// process, so the field measurement M5 depends on does not exist.
#[test]
fn d5_finality_probe_reaches_the_exported_registry() {
    reg::ensure_registered();
    let a1 = h(b"d5b-a1");
    let a2 = h(b"d5b-a2");
    let heavy = block_on(a1, 9);

    let mut handler = ReorgHandler::new();
    handler.record_block_with_weight(a1, Hash::ZERO, 10);
    handler.record_block_with_weight(a2, a1, 10);
    handler.set_last_finality_height(5);
    let _ = handler.check_reorg_weighted(&heavy, a2, 30);

    let mut state = ReorgScrapeState::new();
    apply_reorg_observations(&mut state, &handler.observations());

    reg::assert_exported_nonzero(
        FINALITY_PROBE,
        &[("site", "check_reorg_weighted"), ("outcome", "entry")],
        "O4: finality probe entry",
    );
    reg::assert_exported_nonzero(
        FINALITY_PROBE,
        &[("site", "check_reorg_weighted"), ("outcome", "reject")],
        "O4: finality probe reject",
    );
}
