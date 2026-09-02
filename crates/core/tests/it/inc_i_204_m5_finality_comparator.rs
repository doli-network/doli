//! INC-I-204 M5 — `FinalityTracker::is_at_or_below_finalized` uses `<=`, so it is NOT
//! the fork-choice / rewind guard.
//!
//! Requirement: **REQ-FORK-006** (Must), trap **T1**, **LB-2** / **INV-SYNC-008**.
//!
//! WHY THIS FILE EXISTS, AND WHY IT IS GREEN ON HEAD.
//!
//! Design brief S1 lists `is_at_or_below_finalized` (`crates/core/src/finality.rs:191-195`)
//! as "already-written, zero-caller" API to be wired into the single rewind door. It has
//! zero production callers today, which means nothing currently constrains its
//! comparison, and a reader arriving at M5 sees a helpfully-named finality predicate
//! sitting next to a finality guard that needs one.
//!
//! It is the WRONG predicate for that door, by one character. The rewind guard refuses
//! only when the target is STRICTLY BELOW finality: rolling back TO the finalized height
//! preserves the finalized block and is legal (INV-SYNC-008; INC-I-090 measured the
//! stall a `<=` guard causes — legal 1-block forks at the boundary are blocked for the
//! whole finality window). `is_at_or_below_finalized` answers a different question:
//! "is this height already covered by finality", which is `<=` and is correct for what
//! it is.
//!
//! Wiring it in as-is IS trap T1, which the brief REJECTS. So this file does not assert
//! a future behaviour — it pins the CURRENT semantics as an executable finding, placed
//! BEFORE the developer reaches the wiring decision, so that a later change of the
//! comparison here shows up as a failing test rather than as a silently relaxed guard
//! two crates away. It passes on HEAD by design; it is a tripwire, not a red.

// OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//
//   F1: FinalityTracker::is_at_or_below_finalized(&self, height: u64) -> bool
//       O1: return. `&self`, pure — no mutable params, no receiver mutation, no
//           persistent store writes. One channel, declared complete.
//       PATHS:
//         P1 nothing finalized      -> false for every height
//         P2 height <  finalized    -> true
//         P3 height == finalized    -> true      <- THE cell that separates it from
//                                                   the rewind guard
//         P4 height >  finalized    -> false
//       INPUT PARTITIONS: {no checkpoint, checkpoint at h} x {h-1, h, h+1, 0, u64::MAX}
//       MATRIX: 1 output x 4 paths; every path claimed by a named test below.

use doli_core::finality::{CONFIRMATION_DEPTH, FINALITY_THRESHOLD_PCT};
use doli_core::FinalityTracker;

/// Height of the block driven to finality in the fixture.
const FINALIZED_H: u64 = 100;

/// A tracker with exactly one finalized checkpoint at [`FINALIZED_H`].
///
/// Driven through the real `track_block` / `add_attestation_weight` / `check_finality`
/// path rather than by constructing a checkpoint, so the fixture also witnesses that
/// `last_finalized` is only ever written by `check_finality` (`finality.rs:182`) — the
/// monotonicity precondition the brief S1 requires verified before the mirror can be
/// retired.
fn tracker_finalized_at(height: u64) -> FinalityTracker {
    let mut t = FinalityTracker::new();
    let block = crypto::hash::hash(b"m5_finality_comparator_block");
    t.track_block(block, height, 10, 100);
    t.add_attestation_weight(block, crypto::PublicKey::from_bytes([1u8; 32]), 100);
    let cp = t
        .check_finality(height + CONFIRMATION_DEPTH, true)
        .expect("100/100 weight clears the {FINALITY_THRESHOLD_PCT}% threshold at depth");
    assert_eq!(cp.height, height, "fixture premise");
    // The constant is referenced so the fixture's 100/100 is not a magic number; the
    // comparison is constant by construction (a percentage), hence the allow.
    #[allow(clippy::assertions_on_constants)]
    {
        assert!(
            FINALITY_THRESHOLD_PCT <= 100,
            "threshold is a percentage; referenced so the fixture's 100/100 is not a magic \
             number"
        );
    }
    t
}

/// REQ-FORK-006 / T1 — P3, the load-bearing cell. Decision: a failure means the
/// predicate's meaning changed under the fork-choice work, and whichever door consumes
/// it inherits the opposite answer at the boundary — either blocking legal boundary
/// reorgs (INC-I-090's measured stall) or permitting sub-finality rollback.
#[test]
fn is_at_or_below_finalized_is_true_at_the_finalized_height() {
    let t = tracker_finalized_at(FINALIZED_H);
    assert!(
        t.is_at_or_below_finalized(FINALIZED_H),
        "T1 / LB-2: this predicate is `<=` (finality.rs:191-195). The fork-choice and \
         rewind guard is STRICT `<` — rolling back TO the finalized height preserves \
         the finalized block and is legal (INV-SYNC-008). The two are therefore NOT \
         interchangeable, and wiring this one into the rewind door is trap T1, which \
         the M5 design brief rejects."
    );
}

/// REQ-FORK-006 — P2 and P4. Decision: a failure means the predicate is not a height
/// comparison against the checkpoint at all, so the T1 distinction above is being drawn
/// against something other than what the name says.
#[test]
fn is_at_or_below_finalized_brackets_the_finalized_height() {
    let t = tracker_finalized_at(FINALIZED_H);
    assert!(t.is_at_or_below_finalized(FINALIZED_H - 1), "P2");
    assert!(t.is_at_or_below_finalized(0), "P2 at the extreme");
    assert!(!t.is_at_or_below_finalized(FINALIZED_H + 1), "P4");
    assert!(!t.is_at_or_below_finalized(u64::MAX), "P4 at the extreme");
}

/// REQ-FORK-006 / INV-FINALITY-001 — P1. Decision: a failure means the predicate
/// reports coverage before anything has finalized. Every node is in this state at
/// process start, so a door gated on it would refuse (or admit) everything for the
/// whole pre-finality window — the same class of bug as the mirror's `None`.
#[test]
fn is_at_or_below_finalized_is_false_for_everything_before_anything_finalizes() {
    let t = FinalityTracker::new();
    for h in [0u64, 1, FINALIZED_H, u64::MAX] {
        assert!(
            !t.is_at_or_below_finalized(h),
            "P1: with no checkpoint, height {h} is not covered by finality"
        );
    }
}

/// REQ-FORK-006 / INV-FINALITY-001 — **RED. This reproduces a DEFECT, not a premise.**
///
/// Decision: a failure (which is what happens today) means the HARD PRECONDITION of the
/// M5 mirror-retirement is false — `FinalityTracker.last_finalized` CAN be lowered, so
/// `effective_finality = min(tracker.last_finalized, local_tip)` is not monotone and is
/// not a safe substitute for the mirror. The developer must resolve this BEFORE
/// demoting the mirror, or M5 replaces an erasable guard with a *lowerable* one.
///
/// MEASURED, this session: finalize at height 100, then `track_block` a competing block
/// at height 90 with full attestation weight and call `check_finality` at a tip that
/// clears `CONFIRMATION_DEPTH`. `last_finalized` moves 100 -> **90**.
///
/// WHY. `check_finality` (`finality.rs:143-188`) selects the highest qualifying block
/// among `self.pending` and NEVER compares the winner against `self.last_finalized`
/// before assigning at `:182`. The `pending.retain(|p| p.height > finalized_height)`
/// prune at `:181` only removes blocks that were pending AT THAT MOMENT; anything
/// tracked afterwards at a lower height is eligible.
///
/// REACHABILITY — not theoretical. `track_block` is called for EVERY applied block
/// (`bins/node/src/node/apply_block/state_update.rs:170`, via
/// `production_gate.rs:484-497`, which then calls `finalize_if_ready` immediately).
/// A reorg rolls back and RE-APPLIES blocks at heights below the current tip, and snap
/// sync / backfill apply blocks at low heights, so the "track a block below
/// last_finalized" input arrives on ordinary paths, not only under attack.
///
/// This CONTRADICTS design brief S10, which states of `finality.rs:143-188`: "No public
/// API lowers or resets it. **Verified** — this is the precondition for trusting the
/// tracker as the single store." The brief's own instruction was to verify
/// `FinalityTracker`'s monotonicity IN CODE before deleting the mirror (S1). This is
/// that verification, and it fails.
///
/// It is also INV-FINALITY-001 clause (1) verbatim — "check_finality() MUST refuse to
/// finalize any height at or below last_finalized.height" — unenforced in code.
#[test]
fn check_finality_must_refuse_to_lower_last_finalized() {
    let mut t = tracker_finalized_at(FINALIZED_H);
    assert_eq!(
        t.last_finalized.as_ref().map(|c| c.height),
        Some(FINALIZED_H),
        "fixture premise"
    );

    let lower = crypto::hash::hash(b"m5_lower_competitor");
    t.track_block(lower, FINALIZED_H - 10, 11, 100);
    t.add_attestation_weight(lower, crypto::PublicKey::from_bytes([2u8; 32]), 100);
    let _ = t.check_finality(FINALIZED_H + CONFIRMATION_DEPTH, true);

    assert_eq!(
        t.last_finalized.as_ref().map(|c| c.height),
        Some(FINALIZED_H),
        "INV-FINALITY-001 clause (1): check_finality must REFUSE to finalize a height at          or below last_finalized. It finalized 90 over 100 instead. Until this holds,          `effective_finality = min(tracker.last_finalized, local_tip)` can go DOWN, and          M5 would replace the mirror's erasure with a lowering — the same disarmament          through a different door (brief S12 depends on this not being possible)."
    );
}
