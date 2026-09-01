//! INC-I-204 M5 — the single finality authority: units, boundary, ancestry, and the
//! two ways `block_weights` lies.
//!
//! TDD RED, EXPECTED: does not compile against HEAD (the six-parameter `plan_reorg` /
//! `check_reorg_weighted` and `ForkChoiceFinality` do not exist yet).
//!
//! WHAT DIES HERE. Two authorities compare a height against `last_finality_height`:
//! `check_reorg_weighted` (`mod.rs:377-392`, UNGATED on every network, reading
//! `block_weights…unwrap_or(0)` and taking no height closure at all) and `plan_reorg`'s
//! pre-AH arm (`mod.rs:566-580`). Both compare a PER-PROCESS counter against a REAL
//! chain height. That is INV-SYNC-012's named remaining site and it is what vetoed the
//! reorg at h=77,777.
//!
//! WHAT SURVIVES, DELIBERATELY. The refusal itself (LB-1/LB-2) and its STRICT `<`.
//! Rolling back TO the finalized height preserves the finalized block and is legal
//! (INV-SYNC-008, INC-I-090 AC-1/AC-2). Trap T1 — "while we are in here, relax it to
//! `<=`" — is REJECTED by the design brief and pinned against here. What changes is the
//! UNIT of the comparison and the addition of an ancestry test; the safety property is
//! unchanged.
//!
//! WHY ANCESTRY. A height alone cannot tell two competing blocks at the same height
//! apart. `FinalityCheckpoint` already carries `block_hash` and
//! `production_gate.rs:513-526` throws it away. The ancestry test is the real
//! admissibility rule; the height comparison is retained as a cheap pre-filter (brief
//! S4 item 6).
//!
//! Sibling file: `tests_m5_fork_choice.rs` (the weight half — T9 parity, the writer,
//! derived finality).

// OUTPUT CONTRACT — ENUMERATION-CHECKLIST.
//
//   F1: ReorgHandler::check_reorg_weighted(&self, block, current_tip, weight,
//                                          get_height, finality) -> Option<ReorgResult>
//   F2: ReorgHandler::plan_reorg(&self, current_tip, new_tip, get_parent, get_height,
//                                finality) -> Option<ReorgResult>
//     Both are `&self`: O1 (the returned Option) is the ONLY observable. No mutable
//     params, no receiver mutation, no persistent store writes — `crates/network` has
//     no `doli-storage` dependency (S10), so heights arrive only as closures.
//     Sub-observables of O1 when Some, asserted where the plan's SHAPE matters:
//       O1a rollback   O1b common_ancestor   O1c new_blocks   O1d weight_delta
//
//   PATHS (finality arm only; the weight arm is owned by tests_m5_fork_choice.rs):
//     P1 pre-AH   get_height(ancestor) <  AH  -> VERBATIM legacy: mirror +
//                 block_weights…unwrap_or(0) for F1, the mod.rs:566-580 match for F2
//     P2 post-AH  nothing finalized           -> admit
//     P3 post-AH  ancestor_real >= effective_finality AND branch contains the
//                 finalized hash             -> admit
//     P4 post-AH  ancestor_real <  effective_finality (STRICT)  -> refuse
//     P5 post-AH  branch does NOT contain the finalized hash    -> refuse
//
//   INPUT PARTITIONS (the axis that matters is how `block_weights` disagrees with
//   `get_height`):
//     IP-A ancestor stored REAL           (the honest case)
//     IP-B ancestor stored SYNTHETIC LOW  (INC-I-204: fork-recorded, 1 vs 77_777)
//     IP-C ancestor stored SYNTHETIC HIGH (the reverse; catches "prefer whichever
//                                          answer passes")
//     IP-D ancestor ABSENT / LRU-EVICTED  (INC-I-081 Bug 2: unwrap_or(0))
//   x boundary {real == finality, real == finality - 1}
//   x ancestry {finalized hash on the branch, absent from it, no hash at all}
//   x gate {GATE_DORMANT, GATE_ACTIVE}
//
//   MATRIX: every cell below is claimed by exactly one named test; the cells not
//   claimed are the ones the weight arm owns and are asserted in the sibling file.

use super::tests_m5_common::*;
use super::*;

/// The gossiped sibling used against `check_reorg_weighted`. Strictly heavier than our
/// branch, so neither the lighter-chain gate (`mod.rs:336`) nor the equal-weight
/// tie-break (`mod.rs:351`) can be the refuser and ONLY the finality arm is under test.
fn heavier_sibling(f: &TrunkFork) -> Block {
    fork_block(f.ancestor, 9)
}

/// The producer weight that makes [`heavier_sibling`] strictly heavier.
const SIBLING_WEIGHT: u64 = INCIDENT_WEIGHT + 10;

// ===========================================================================
// P3/P4 — the incident cell, above the gate. The test the milestone exists for.
// ===========================================================================

/// REQ-FORK-006 / INV-SYNC-012 — Decision: a failure means the unified authority still
/// vetoes the reorg that wedged the node at h=77,777, so M5 shipped without fixing
/// INC-I-204 and the activation height was spent for nothing.
///
/// IP-B x boundary(real == finality) x GATE_ACTIVE. Successor to
/// `tests_m5_red_witness.rs` W3.
#[test]
fn post_ah_the_incident_cell_is_no_longer_vetoed_on_a_synthetic_height() {
    let f = trunk_fork(
        ANCESTOR_REAL_H,
        AncestorRecord::Synthetic(1),
        0,
        GATE_ACTIVE,
    );
    assert_eq!(
        f.stored_ancestor_height(),
        Some(1),
        "fixture premise: the ancestor was recorded through the FORK path, so \
         block_weights holds the per-process counter 1 for a block at real height \
         {ANCESTOR_REAL_H}. If this premise breaks, the test below is measuring \
         something else."
    );

    let finality = fin(Some(FINALIZED_H), Some(f.ancestor), TIP_REAL_H);
    let sibling = heavier_sibling(&f);

    let gossip = f.handler.check_reorg_weighted(
        &sibling,
        f.our_tip,
        SIBLING_WEIGHT,
        f.get_height(),
        finality,
    );
    assert!(
        gossip.is_some(),
        "INC-I-204: above the gate the ancestor's REAL height ({ANCESTOR_REAL_H}) is \
         authoritative and equals the finalized height, so the reorg preserves the \
         finalized block and is legal (INV-SYNC-008, strict `<`). The pre-M5 code \
         compared the synthetic 1 and refused at mod.rs:384."
    );
    let plan = f.handler.plan_reorg(
        f.our_tip,
        f.cand_tip,
        f.get_parent(),
        f.get_height(),
        finality,
    );
    let plan = plan.expect("the planning door must reach the same verdict as the gossip door");
    assert_eq!(plan.common_ancestor, f.ancestor, "O1b");
    assert_eq!(plan.rollback, vec![f.our_tip], "O1a");
    assert_eq!(plan.new_blocks, vec![f.cand_tip], "O1c");
}

/// REQ-FORK-006 / LB-1 — Decision: a failure means M5 removed the sub-finality refusal
/// instead of re-basing its units. The refusal is the hero of this design (brief S1);
/// losing it would let a heavy attacker branch roll back finalized history.
///
/// IP-B x boundary(real == finality - 1) x GATE_ACTIVE. `finalized_hash: None` ISOLATES
/// the numeric pre-filter from the ancestry rule; the ancestry rule has its own tests.
#[test]
fn post_ah_the_unified_authority_still_vetoes_a_genuinely_sub_finality_ancestor() {
    let f = trunk_fork(
        ANCESTOR_REAL_H,
        AncestorRecord::Synthetic(1),
        0,
        GATE_ACTIVE,
    );
    let finality = fin(Some(ANCESTOR_REAL_H + 1), None, TIP_REAL_H);
    let sibling = heavier_sibling(&f);

    assert!(
        f.handler
            .check_reorg_weighted(
                &sibling,
                f.our_tip,
                SIBLING_WEIGHT,
                f.get_height(),
                finality
            )
            .is_none(),
        "LB-1: the ancestor's REAL height {ANCESTOR_REAL_H} is strictly below effective \
         finality {}, so the reorg would undo a finalized block and must be refused",
        ANCESTOR_REAL_H + 1
    );
    assert!(
        f.handler
            .plan_reorg(
                f.our_tip,
                f.cand_tip,
                f.get_parent(),
                f.get_height(),
                finality
            )
            .is_none(),
        "LB-1: the planning door must refuse it too, or fork recovery bypasses finality \
         (the reason mod.rs:525-527 mirrors the check in the first place)"
    );
}

/// REQ-FORK-006 / brief S12 — Decision: a failure means the erasable mirror is still an
/// AUTHORITY above the gate, so `clear_finality_if_below_tip` can still disarm the
/// guard (INV-FINALITY-001's measured erasure) and the mirror can never be physically
/// deleted in M6.
///
/// The brief resolves S12 by removing the mirror as an authority, not as bytes. This is
/// the executable form of that resolution: above the gate, a wildly wrong mirror value
/// must change nothing.
#[test]
fn post_ah_the_finality_mirror_is_no_longer_read() {
    let mut f = trunk_fork(ANCESTOR_REAL_H, AncestorRecord::Real, 0, GATE_ACTIVE);
    let finality = fin(Some(FINALIZED_H), Some(f.ancestor), TIP_REAL_H);
    let sibling = heavier_sibling(&f);

    let without_mirror = f
        .handler
        .check_reorg_weighted(
            &sibling,
            f.our_tip,
            SIBLING_WEIGHT,
            f.get_height(),
            finality,
        )
        .is_some();

    // A value that would refuse EVERY reorg if the mirror were still consulted.
    f.handler.set_last_finality_height(u64::MAX);
    let with_absurd_mirror = f
        .handler
        .check_reorg_weighted(
            &sibling,
            f.our_tip,
            SIBLING_WEIGHT,
            f.get_height(),
            finality,
        )
        .is_some();

    assert!(
        without_mirror,
        "fixture premise: admissible before the mirror is poisoned"
    );
    assert_eq!(
        without_mirror, with_absurd_mirror,
        "brief S12: above the gate NOTHING reads last_finality_height. Setting it to \
         u64::MAX must not change the verdict. While it can, the erasure route \
         (clear_finality_if_below_tip) is still reachable above the gate and the mirror \
         cannot be deleted in M6."
    );
}

// ===========================================================================
// T1 — the boundary. STRICT `<`, both regimes, both doors.
// ===========================================================================

/// REQ-FORK-006 / T1 / INV-SYNC-008 / LB-2 — Decision: a failure in the ADMIT half
/// means the fencepost was relaxed the wrong way and legal 1-block forks at the finality
/// boundary are blocked for the whole finality window (INC-I-090, the measured stall);
/// a failure in the REFUSE half means the guard was relaxed to `<=`'s complement and
/// finalized history can be undone. Both directions are pinned because trap T1 is a
/// tempting one-character edit in exactly this code.
#[test]
fn t1_post_ah_the_refusal_is_strict_less_than() {
    // AT the finalized height: the ancestor IS the finalized block, so the reorg
    // preserves it. ADMITTED. Production-shaped: both height and hash present.
    let at = trunk_fork(ANCESTOR_REAL_H, AncestorRecord::Real, 0, GATE_ACTIVE);
    let at_fin = fin(Some(ANCESTOR_REAL_H), Some(at.ancestor), TIP_REAL_H);
    assert!(
        at.handler
            .check_reorg_weighted(
                &heavier_sibling(&at),
                at.our_tip,
                SIBLING_WEIGHT,
                at.get_height(),
                at_fin
            )
            .is_some(),
        "T1/AC-2: ancestor_height == finality_height must be ADMITTED. A `<=` guard here \
         is the fencepost error INV-SYNC-008 names."
    );
    assert!(
        at.handler
            .plan_reorg(
                at.our_tip,
                at.cand_tip,
                at.get_parent(),
                at.get_height(),
                at_fin
            )
            .is_some(),
        "T1/AC-1: same boundary, planning door"
    );

    // ONE BLOCK BELOW: the reorg would undo the finalized block. REFUSED.
    // `finalized_hash: None` isolates the numeric pre-filter — see the note in
    // `ancestry_...` below for why the two rules are otherwise coupled here.
    let below = trunk_fork(ANCESTOR_REAL_H - 1, AncestorRecord::Real, 0, GATE_ACTIVE);
    let below_fin = fin(Some(ANCESTOR_REAL_H), None, ANCESTOR_REAL_H);
    assert!(
        below
            .handler
            .check_reorg_weighted(
                &heavier_sibling(&below),
                below.our_tip,
                SIBLING_WEIGHT,
                below.get_height(),
                below_fin
            )
            .is_none(),
        "T1: one block below finality must be REFUSED — the refusal is the hero, only \
         its unit changed"
    );
    assert!(
        below
            .handler
            .plan_reorg(
                below.our_tip,
                below.cand_tip,
                below.get_parent(),
                below.get_height(),
                below_fin
            )
            .is_none(),
        "T1: same, planning door"
    );
}

/// REQ-FORK-006 / T1 / T9 — Decision: a failure means the boundary moved BELOW the
/// gate, i.e. the dormant window is not byte-identical at the one input INC-I-090
/// already cost a stall to get right. The pre-M5 boundary tests
/// (`tests.rs::plan_reorg_permits_reorg_to_finality`,
/// `check_reorg_weighted_permits_reorg_to_finality`) must keep their answers.
#[test]
fn t1_pre_ah_the_legacy_boundary_is_unchanged() {
    // Below the gate the comparison reads the MIRROR and the STORED height, so the
    // fixture roots the stored heights at the real ones (inc_i_147 gate 0, as on HEAD).
    let mut at = trunk_fork(ANCESTOR_REAL_H, AncestorRecord::Real, 0, GATE_DORMANT);
    assert_eq!(
        at.stored_ancestor_height(),
        Some(ANCESTOR_REAL_H),
        "fixture premise: pre-AH the stored height is what the legacy comparison reads"
    );
    at.handler.set_last_finality_height(ANCESTOR_REAL_H);
    // The `finality` argument is supplied and must be IGNORED below the gate; giving it
    // a value that would refuse proves the legacy arm is genuinely in control.
    let decoy = fin(Some(u64::MAX), None, TIP_REAL_H);
    assert!(
        at.handler
            .check_reorg_weighted(
                &heavier_sibling(&at),
                at.our_tip,
                SIBLING_WEIGHT,
                at.get_height(),
                decoy
            )
            .is_some(),
        "T9: below the gate the mirror decides and reorging TO the finalized height is \
         admitted, exactly as on HEAD. The new `finality` argument must not leak in."
    );

    let mut below = trunk_fork(ANCESTOR_REAL_H - 1, AncestorRecord::Real, 0, GATE_DORMANT);
    below.handler.set_last_finality_height(ANCESTOR_REAL_H);
    assert!(
        below
            .handler
            .check_reorg_weighted(
                &heavier_sibling(&below),
                below.our_tip,
                SIBLING_WEIGHT,
                below.get_height(),
                fin(None, None, TIP_REAL_H)
            )
            .is_none(),
        "T9: and one below is still refused, from the mirror, with strict `<`"
    );
}

// ===========================================================================
// P5 — ancestry. The rule the height comparison was always standing in for.
// ===========================================================================

/// REQ-FORK-006 — Decision: a failure means admissibility is still decided by a NUMBER,
/// which cannot distinguish two competing blocks at the same height; a node holding a
/// stale checkpoint from an abandoned branch would reorg away from the block it
/// finalized while every numeric guard reported "fine".
///
/// WHY THIS CELL IS CONSTRUCTIBLE. On one truthful chain it is not: a branch that omits
/// the finalized block always has a common ancestor strictly below the finalized height,
/// so the numeric pre-filter refuses first. It becomes reachable exactly when the local
/// checkpoint names a block that is NOT on the chain being compared — a checkpoint from
/// a branch the node has since abandoned, or an equivocating producer's second block at
/// the finalized height. That is the state the mirror's erasure existed to paper over.
#[test]
fn post_ah_a_branch_that_omits_the_finalized_hash_is_refused() {
    let f = trunk_fork(ANCESTOR_REAL_H, AncestorRecord::Real, 0, GATE_ACTIVE);

    // The checkpoint names a block at a height the ancestor clears, but that block is
    // on neither branch.
    let abandoned = crypto::hash::hash(b"m5_finalized_on_an_abandoned_branch");
    let stale = fin(Some(ANCESTOR_REAL_H - 1), Some(abandoned), TIP_REAL_H);

    assert!(
        f.handler
            .check_reorg_weighted(
                &heavier_sibling(&f),
                f.our_tip,
                SIBLING_WEIGHT,
                f.get_height(),
                stale
            )
            .is_none(),
        "the numeric pre-filter PASSES here (ancestor {ANCESTOR_REAL_H} >= effective \
         finality {}), so only the ancestry test can refuse. Height alone cannot tell \
         two blocks at the same height apart; FinalityCheckpoint.block_hash exists \
         precisely for this and production_gate.rs:513-526 currently discards it.",
        ANCESTOR_REAL_H - 1
    );
    assert!(
        f.handler
            .plan_reorg(f.our_tip, f.cand_tip, f.get_parent(), f.get_height(), stale)
            .is_none(),
        "same, planning door"
    );
}

/// REQ-FORK-006 — ANTI-VACUITY for the test above. Decision: a failure means the
/// ancestry rule refuses EVERYTHING, which would wedge every node permanently — a
/// strictly worse outcome than the incident, and one that a refuse-only test cannot
/// distinguish from a correct implementation.
#[test]
fn post_ah_a_branch_that_contains_the_finalized_hash_is_admitted() {
    let f = trunk_fork(ANCESTOR_REAL_H, AncestorRecord::Real, 0, GATE_ACTIVE);
    // The finalized block is the TRUNK, one below the common ancestor and therefore on
    // BOTH branches.
    let onbranch = fin(Some(ANCESTOR_REAL_H - 1), Some(f.trunk), TIP_REAL_H);

    assert!(
        f.handler
            .check_reorg_weighted(
                &heavier_sibling(&f),
                f.our_tip,
                SIBLING_WEIGHT,
                f.get_height(),
                onbranch
            )
            .is_some(),
        "the candidate branch descends from the finalized trunk block, so the \
         finalized block survives the reorg and it must be admitted"
    );
    assert!(
        f.handler
            .plan_reorg(
                f.our_tip,
                f.cand_tip,
                f.get_parent(),
                f.get_height(),
                onbranch
            )
            .is_some(),
        "same, planning door"
    );
}

// ===========================================================================
// INV-SYNC-012 — block_weights is never a height source above the gate.
// ===========================================================================

/// REQ-FORK-006 / INV-SYNC-012 — Decision: a failure in the FIRST half means the wedge
/// survives; a failure in the SECOND means the implementation consults both sources and
/// takes whichever admits, which is not a rule at all and would let a node with a stale
/// in-memory map reorg past finality. The second half is the discriminator: an
/// implementation that simply prefers the passing answer is GREEN on every other test
/// in this file.
#[test]
fn post_ah_the_ancestor_height_comes_from_get_height_and_never_from_block_weights() {
    // IP-B: stored LOW (1), real at finality. block_weights would refuse; get_height admits.
    let low = trunk_fork(
        ANCESTOR_REAL_H,
        AncestorRecord::Synthetic(1),
        0,
        GATE_ACTIVE,
    );
    assert_eq!(
        low.stored_ancestor_height(),
        Some(1),
        "fixture premise IP-B"
    );
    assert!(
        low.handler
            .plan_reorg(
                low.our_tip,
                low.cand_tip,
                low.get_parent(),
                low.get_height(),
                fin(Some(FINALIZED_H), Some(low.ancestor), TIP_REAL_H)
            )
            .is_some(),
        "INV-SYNC-012: block_weights says 1 and would refuse; get_height says \
         {ANCESTOR_REAL_H} and admits. The chain-derived height wins."
    );

    // IP-C: stored HIGH (900_000), real BELOW finality. block_weights would admit;
    // get_height refuses. THE DISCRIMINATOR.
    const REAL_H: u64 = 1_000;
    const STORED_H: u64 = 900_000;
    let high = trunk_fork(REAL_H, AncestorRecord::Synthetic(STORED_H), 0, GATE_ACTIVE);
    assert_eq!(
        high.stored_ancestor_height(),
        Some(STORED_H),
        "fixture premise IP-C: block_weights must hold the inflated counter"
    );
    assert!(
        high.handler
            .plan_reorg(
                high.our_tip,
                high.cand_tip,
                high.get_parent(),
                high.get_height(),
                fin(Some(REAL_H + 1), None, REAL_H + 1)
            )
            .is_none(),
        "INV-SYNC-012 DISCRIMINATOR: block_weights says {STORED_H} and would admit; \
         get_height says {REAL_H}, which is strictly below effective finality {}, and \
         must refuse. An implementation that consults both and takes whichever passes \
         is green on every other cell and fails only here.",
        REAL_H + 1
    );
}

// ===========================================================================
// IP-D — the ancestor is gone from block_weights. INC-I-081 Bug 2's successor.
// ===========================================================================

/// REQ-FORK-006 / INV-SYNC-002 — Decision: a failure means the guard still collapses to
/// `unwrap_or(0)` when the ancestor is not in the LRU map, which silently rejects EVERY
/// reorg on a node whose finality is non-zero — the permanent wedge of INC-I-081 Bug 2,
/// reachable on any long-running node because the map is bounded at 10,000.
///
/// The ancestor is evicted for real: `max_tracked` is shrunk and one more block is
/// recorded, so the LRU pops genesis, the trunk and the ancestor while both tips stay.
#[test]
fn post_ah_an_lru_evicted_ancestor_still_resolves_through_get_height() {
    let mut f = trunk_fork(ANCESTOR_REAL_H, AncestorRecord::Real, 0, GATE_ACTIVE);
    assert_eq!(
        f.stored_ancestor_height(),
        Some(ANCESTOR_REAL_H),
        "fixture premise: the ancestor starts tracked, so the eviction below is a real \
         state change and not a no-op"
    );

    f.handler.max_tracked = 3;
    f.handler
        .record_fork_block(crypto::hash::hash(b"m5_lru_filler"), Hash::ZERO, 0);
    assert_eq!(
        f.stored_ancestor_height(),
        None,
        "fixture premise: the ancestor must actually be gone from block_weights"
    );

    assert!(
        f.handler
            .plan_reorg(
                f.our_tip,
                f.cand_tip,
                f.get_parent(),
                f.get_height(),
                fin(Some(FINALIZED_H), Some(f.ancestor), TIP_REAL_H)
            )
            .is_some(),
        "INV-SYNC-002: with the ancestor evicted, the height must come from get_height \
         ({ANCESTOR_REAL_H}) and the reorg is admissible. Collapsing to unwrap_or(0) \
         gives 0 < {FINALIZED_H} and refuses — that is the permanent wedge."
    );
    assert!(
        f.handler
            .plan_reorg(
                f.our_tip,
                f.cand_tip,
                f.get_parent(),
                f.get_height(),
                fin(Some(ANCESTOR_REAL_H + 1), None, TIP_REAL_H)
            )
            .is_none(),
        "and eviction is not a bypass: a genuinely sub-finality ancestor is still refused"
    );
}

/// REQ-FORK-012 / LB-1 — Decision: a failure means the audited operator door either
/// stopped crossing the marker (M4.1 regressed, and a wedged node has no manual escape)
/// or started crossing more than the marker. M5 must leave `plan_reorg_operator`'s
/// scope exactly where M4.1 put it.
#[test]
fn post_ah_the_operator_door_still_crosses_the_refusal_and_nothing_else() {
    let f = trunk_fork(ANCESTOR_REAL_H, AncestorRecord::Real, 0, GATE_ACTIVE);
    let sub_finality = fin(Some(ANCESTOR_REAL_H + 1), None, TIP_REAL_H);

    assert!(
        f.handler
            .plan_reorg(
                f.our_tip,
                f.cand_tip,
                f.get_parent(),
                f.get_height(),
                sub_finality
            )
            .is_none(),
        "premise: the automatic door refuses this plan"
    );
    assert!(
        f.handler
            .plan_reorg_operator(
                f.our_tip,
                f.cand_tip,
                f.get_parent(),
                f.get_height(),
                sub_finality
            )
            .is_some(),
        "REQ-FORK-012: the operator door crosses the finality refusal for one planned \
         reorg. Automatic callers must never reach this variant."
    );

    // The operator door overrides FINALITY, not the graph. No common ancestor is still
    // no plan.
    let unrelated = crypto::hash::hash(b"m5_unrelated_tip_with_no_ancestor");
    assert!(
        f.handler
            .plan_reorg_operator(f.our_tip, unrelated, |_| None, f.get_height(), sub_finality)
            .is_none(),
        "REQ-FORK-012: the override is scoped to the finality refusal. The \
         no-common-ancestor veto still binds."
    );
}
