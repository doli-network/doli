//! INC-I-204 M5 — the two dormant-window cells the Phase-2 suite left open.
//!
//! Written by the developer, not the test-writer, and named here so the addition is
//! visible: (1) the test plan §3 item 1 lists a `plan_reorg` legacy-oracle parity
//! test that the suite does not contain — without it `legacy_plan_reorg_admits` is an
//! unused transcription and the planning door's dormant window is asserted by nothing;
//! (2) the permanent successor to red witness W4, which measured that
//! `set_last_finality_height` is a bare assignment where INV-FINALITY-001 clause (1)
//! requires `max()`. W4 demanded the fix UNGATED, which would change pre-M5 fork
//! choice on both live networks — the one thing the activation height exists to
//! prevent — so the fix is gated and this pins BOTH sides of the gate.

// OUTPUT CONTRACT — ENUMERATION-CHECKLIST.
//
//   F1: ReorgHandler::plan_reorg(&self, cur, new, get_parent, get_height, finality)
//       O1: return — Some/None. `&self`; no mutable params, no receiver mutation,
//           no persistent store writes (crates/network has no doli-storage dep).
//       PATH under test: P1 pre-AH, compared cell-by-cell against
//           `tests_m5_common::legacy_plan_reorg_admits` (an independent
//           transcription of the pre-M5 source lines).
//       INPUT PARTITIONS: ancestor record {Real, Synthetic(low), Evicted}
//           x mirror finality {None, below ancestor, at ancestor, above ancestor}
//           x inc_i_147 gate {0 (post), u64::MAX (pre)}.
//       MATRIX: 1 output x 3 x 4 x 2 = 24 cells, all driven by the table.
//
//   F2: ReorgHandler::set_last_finality_height(&mut self, height) -> ()
//       O1: return — unit.
//       O2: receiver — last_finality_height, read through `last_finality_height()`.
//           THE observable. No other channel: no store writes.
//       PATHS: P1 height <  AH -> bare assignment (pre-M5, may LOWER)
//              P2 height >= AH -> max()
//       INPUT PARTITIONS: {first write, higher write, lower write, equal write}
//           x {GATE_DORMANT, GATE_ACTIVE}.

use super::tests_m5_common::*;
use super::*;

/// REQ-FORK-006 / T9 — Decision: a failure means the PLANNING door's dormant window
/// is not byte-identical, so a node on the new binary plans a different reorg from a
/// node on the old one below the gate — the mixed-fleet split (trap T9, hard filter
/// C-22), arriving through fork recovery and the wedge escape rather than gossip.
///
/// The oracle is `tests_m5_common::legacy_plan_reorg_admits`, transcribed from the
/// pre-M5 `mod.rs:556-597` ordering rather than by calling the code under test.
#[test]
fn t9_pre_ah_plan_reorg_matches_the_legacy_oracle_over_the_finality_vector() {
    const REAL_H: u64 = 1_000;

    for record in [
        AncestorRecord::Real,
        AncestorRecord::Synthetic(1),
        AncestorRecord::Evicted,
    ] {
        for inc_i_147_gate in [0u64, u64::MAX] {
            for mirror in [None, Some(REAL_H - 1), Some(REAL_H), Some(REAL_H + 1)] {
                let mut f = trunk_fork(REAL_H, record, inc_i_147_gate, GATE_DORMANT);
                if let Some(m) = mirror {
                    f.handler.set_last_finality_height(m);
                }

                let stored = f.stored_ancestor_height();
                let expected =
                    legacy_plan_reorg_admits(mirror, stored, Some(REAL_H), inc_i_147_gate);
                let got = f
                    .handler
                    .plan_reorg(
                        f.our_tip,
                        f.cand_tip,
                        f.get_parent(),
                        f.get_height(),
                        // A decoy: below the gate the new finality argument must not
                        // reach any decision. u64::MAX would refuse every reorg.
                        fin(Some(u64::MAX), Some(f.trunk), REAL_H + 1),
                    )
                    .is_some();

                assert_eq!(
                    got, expected,
                    "T9 dormant window, planning door: record={record:?} \
                     inc_i_147_gate={inc_i_147_gate} mirror={mirror:?} \
                     stored={stored:?} real={REAL_H}. The pre-M5 ordering \
                     (mod.rs:556-597) must survive verbatim, and the M5 `finality` \
                     argument must not leak into it."
                );
            }
        }
    }
}

/// REQ-FORK-006 / T9 — ANTI-VACUITY for the test above: the oracle must not be
/// constant over the vector, or the parity assertion asserts nothing.
#[test]
fn the_legacy_plan_reorg_oracle_answers_both_ways_over_the_vector() {
    const REAL_H: u64 = 1_000;
    let mut admits = 0usize;
    let mut refuses = 0usize;
    for stored in [None, Some(1), Some(REAL_H)] {
        for mirror in [None, Some(REAL_H - 1), Some(REAL_H + 1)] {
            for gate in [0u64, u64::MAX] {
                if legacy_plan_reorg_admits(mirror, stored, Some(REAL_H), gate) {
                    admits += 1;
                } else {
                    refuses += 1;
                }
            }
        }
    }
    assert!(
        admits > 0 && refuses > 0,
        "the oracle answered the same way in all {} cells — parity against it would \
         be vacuous",
        admits + refuses
    );
}

/// REQ-FORK-006 / INV-FINALITY-001 clause (1) — successor to red witness W4.
///
/// Decision: a failure in the POST half means M5 replaced the mirror's erasure with a
/// *lowering* — the same disarmament through a quieter door, and the successor
/// `effective_finality` inherits it because it reads the store this guard mirrors. A
/// failure in the PRE half means the dormant window is not byte-identical: the fix
/// would change which reorgs a live mainnet node accepts, with no height crossed.
#[test]
fn set_last_finality_height_is_monotone_above_the_gate_and_verbatim_below_it() {
    const FINALIZED_H: u64 = 77_777;

    let mut post = ReorgHandler::with_activation_heights(0, GATE_ACTIVE);
    post.set_last_finality_height(FINALIZED_H);
    post.set_last_finality_height(FINALIZED_H - 10);
    assert_eq!(
        post.last_finality_height(),
        Some(FINALIZED_H),
        "above the gate the guard takes max(): a checkpoint that moved DOWN (any \
         reorg, snap or backfill re-applies blocks below the tip) must not re-open \
         every reorg between the two heights"
    );
    post.set_last_finality_height(FINALIZED_H + 1);
    assert_eq!(
        post.last_finality_height(),
        Some(FINALIZED_H + 1),
        "and it still moves UP"
    );

    let mut pre = ReorgHandler::with_activation_heights(0, GATE_DORMANT);
    pre.set_last_finality_height(FINALIZED_H);
    pre.set_last_finality_height(FINALIZED_H - 10);
    assert_eq!(
        pre.last_finality_height(),
        Some(FINALIZED_H - 10),
        "below the gate the pre-M5 bare assignment survives verbatim. This value is \
         the defect W4 measured; it is preserved on purpose, because changing it \
         below the height is a mixed-fleet fork-choice divergence with no gate."
    );
}

/// REQ-FORK-006 / brief S12 — Decision: a failure means the mirror's ERASURE route is
/// still live above the gate, so `clear_finality_if_below_tip` can disarm the guard
/// after a rollback and the field can never be deleted in M6.
#[test]
fn clear_finality_if_below_tip_is_a_no_op_above_the_gate() {
    const FINALIZED_H: u64 = 77_777;

    let mut post = ReorgHandler::with_activation_heights(0, GATE_ACTIVE);
    post.set_last_finality_height(FINALIZED_H);
    post.clear_finality_if_below_tip(FINALIZED_H - 100);
    assert_eq!(
        post.last_finality_height(),
        Some(FINALIZED_H),
        "S12: above the gate the erasure is unreachable — the successor releases a \
         sub-finality node by clamping (effective_finality = min(checkpoint, tip)), \
         not by throwing the guard away"
    );

    let mut pre = ReorgHandler::with_activation_heights(0, GATE_DORMANT);
    pre.set_last_finality_height(FINALIZED_H);
    pre.clear_finality_if_below_tip(FINALIZED_H - 100);
    assert_eq!(
        pre.last_finality_height(),
        None,
        "below the gate the erasure survives verbatim (INC-I-081 Bug 4 backstop)"
    );
}

/// REQ-FORK-006 — Decision: a failure means the planning door and the weight
/// authority disagree about the SHAPE of the same two-branch fixture the T9 table
/// drives, so "one authority" would hold for the verdict and not for the plan the
/// verdict authorises. Exercises the caller-supplied parent closure as the fallback
/// the LRU-bounded in-memory map needs.
#[test]
fn post_ah_the_planning_door_plans_the_two_branch_fixture() {
    let f = two_branch(INCIDENT_WEIGHT, INCIDENT_WEIGHT + 1, true, GATE_ACTIVE);
    let finality = fin(Some(ANCESTOR_REAL_H), Some(f.ancestor), TIP_REAL_H);

    assert_eq!(
        f.handler.weigh_branches(&f.our_tip, &f.cand_tip, finality),
        WeightVerdict::Heavier,
        "premise: the candidate branch is strictly heavier"
    );

    let plan = f
        .handler
        .plan_reorg(
            f.our_tip,
            f.cand_tip,
            |h| f.parents.get(h).copied(),
            |h| f.heights.get(h).copied(),
            finality,
        )
        .expect("the planning door must authorise what the weight authority named");

    assert_eq!(plan.common_ancestor, f.ancestor, "O1b");
    assert_eq!(plan.rollback, vec![f.our_tip], "O1a");
    assert_eq!(plan.new_blocks, vec![f.cand_tip], "O1c");
    assert_eq!(plan.weight_delta, 1, "O1d");
}
