//! INC-I-204 M6 — finality-mirror PINS. The mirror is NOT deleted (design brief §5
//! R1: `inc_i_204_fork_choice_activation_height` is `u64::MAX` on mainnet and
//! testnet, so the below-gate path — which reads the mirror as its finality
//! authority — is the only live path on every real network). These tests make the
//! future deletion a mechanical, test-covered step.
//!
//! OUTPUT CONTRACT — `fn ReorgHandler::set_last_finality_height(&mut self, u64)`
//!   O1 mutable params: none.  O2 receiver mutation: `last_finality_height` — read
//!      back through `last_finality_height()`.  O3 return value: `()`.
//!   O4 persistent store: none.  O5 statics: none.  O6 events: none.
//!   PATHS: S1 above the gate → `max()` (monotone). S2 below the gate → assignment.
//!   INPUT PARTITIONS: (gate active | gate dormant) × (higher | lower) than current.
//!   MATRIX: O2 × {S1, S2} × {higher, lower}.
//!
//! OUTPUT CONTRACT — `fn ReorgHandler::clear_finality_if_below_tip(&mut self, u64)`
//!   O1 mutable params: none.  O2 receiver mutation: `last_finality_height`.
//!   O3 return value: `()`.  O4/O5: none.  O6 events: tracing only.
//!   PATHS: C1 tip below finality → clear. C2 tip at/above finality → no-op.
//!   INPUT PARTITIONS: new_tip ∈ {finality-10, finality, finality+10}.
//!   MATRIX: O2 × {C1, C2}.
//!
//! OUTPUT CONTRACT — `fn SyncManager::add_attestation_weight(&mut self, ...)`
//!   (the production writer that keeps mirror and tracker in step)
//!   O1 mutable params: none.  O2 receiver mutation: `finality_tracker.last_finalized`
//!      (read via `last_finalized_height()`) AND `reorg_handler.last_finality_height`
//!      (read via `reorg_handler().last_finality_height()`).  O3 return value: `()`.
//!   O4 persistent store: none.  O5 statics: none.  O6 events: tracing only.
//!   PATHS: F1 threshold+depth met → checkpoint → both observables move together.
//!          F2 threshold not met → neither moves.
//!   INPUT PARTITIONS: a monotone sequence of finalizing rounds; then the
//!          INV-SYNC-004 rollback that is the ONE permitted divergence.
//!   MATRIX: (O2 tracker, O2 mirror) × {F1, F2, rollback}.
//!
//! All tests here are PASS-LOCKS: green before AND after M6.

use crypto::{Hash, PublicKey};
use doli_core::{Network, NetworkParams};

use super::*;
use crate::sync::manager::{SyncConfig, SyncManager};

/// Total network weight per finality round; 70 of 100 clears the 67% threshold.
const TOTAL_WEIGHT: u64 = 100;
const ATTESTED_WEIGHT: u64 = 70;

fn block_hash(tag: &str, h: u64) -> Hash {
    crypto::hash::hash(format!("{tag}_{h}").as_bytes())
}

fn attester(seed: u8) -> PublicKey {
    PublicKey::from_bytes([seed; 32])
}

/// Drive one finality round: apply a tip two blocks above `h`, track the block at
/// `h`, and attest it past the threshold so `finalize_if_ready` fires.
fn finalize_round(mgr: &mut SyncManager, h: u64, seed: u8) {
    let target = block_hash("finalized", h);
    let tip = block_hash("tip", h + 2);
    mgr.update_local_tip(h + 2, tip, (h + 2) as u32);
    mgr.track_block_for_finality(target, h, h as u32, TOTAL_WEIGHT);
    mgr.add_attestation_weight(&target, attester(seed), ATTESTED_WEIGHT);
}

// ===========================================================================
// The mirror must answer every production read the way the tracker would
// ===========================================================================

/// INV-FINALITY-001 / REQ-FORK-004 — Decision: a failure here reveals that
/// `ReorgHandler.last_finality_height` (the authority on every live network, since
/// both real gates are dormant) has drifted from `FinalityTracker.last_finalized`,
/// which is the precondition the mirror deletion depends on — deleting the mirror
/// while the two disagree would silently move every finality guard.
///
/// Covers O2 (tracker, mirror) on path F1 across a monotone sequence.
#[test]
fn m6_pin_the_mirror_answers_every_read_the_finality_tracker_would() {
    let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    let mut finalized_any = false;
    for (i, h) in [100u64, 140, 180, 220].into_iter().enumerate() {
        finalize_round(&mut mgr, h, 1 + i as u8);

        let tracker = mgr.last_finalized_height();
        let mirror = mgr.reorg_handler().last_finality_height();
        assert_eq!(
            mirror, tracker,
            "INV-FINALITY-001: after finalizing h={h} the mirror \
             (ReorgHandler.last_finality_height={mirror:?}) disagrees with the tracker \
             (FinalityTracker.last_finalized={tracker:?}). Every below-gate finality read \
             — recovery.rs FINALITY_GUARD, reorg/doors.rs, production_gate.rs checkpoint \
             write, block_lifecycle.rs RecoveryContext — goes through the mirror, so a \
             divergence moves them all."
        );
        finalized_any |= tracker.is_some();
    }

    assert!(
        finalized_any,
        "fixture: no round ever finalized, so the equality above is vacuous. \
         check_finality needs BOTH >= 67% weight and an applied descendant at depth \
         >= CONFIRMATION_DEPTH."
    );
    assert_eq!(
        mgr.last_finalized_height(),
        Some(220),
        "fixture: the last round must be the one that finalized"
    );
}

/// INV-SYNC-004 / REQ-FORK-004 — Decision: a failure here reveals a SECOND way for
/// the mirror and the tracker to diverge. Exactly one is known and intended (the
/// stale-clearing backstop after a rollback below the finalized height); any other
/// divergence is unaccounted state that the mirror deletion would silently drop.
#[test]
fn m6_pin_the_only_mirror_tracker_divergence_is_the_inv_sync_004_backstop() {
    let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    finalize_round(&mut mgr, 100, 1);
    assert_eq!(
        mgr.reorg_handler().last_finality_height(),
        Some(100),
        "precondition: the mirror holds the finalized height"
    );

    mgr.clear_finality_if_below_tip(90);

    assert_eq!(
        mgr.reorg_handler().last_finality_height(),
        None,
        "INV-SYNC-004: after a rollback to a tip below the finalized height the mirror \
         MUST be cleared, or the finality guard permanently blocks every future reorg"
    );
    assert_eq!(
        mgr.last_finalized_height(),
        Some(100),
        "INV-SYNC-004 (the ONE intended divergence): the tracker keeps its checkpoint \
         while the mirror is cleared. Whoever deletes the mirror must reproduce this \
         release through ForkChoiceFinality::effective_finality, which clamps to the \
         local tip."
    );
}

// ===========================================================================
// Monotonicity of the setter (M5), pinned so the deletion cannot regress it
// ===========================================================================

/// INV-FINALITY-001 clause (1) — Decision: a failure here reveals that the M5
/// monotone setter regressed to a bare assignment, so a checkpoint that moved DOWN
/// (reachable whenever a reorg, snap or backfill re-applies blocks below the tip)
/// can lower the finality guard again.
///
/// Covers O2 on paths S1 (gate active → `max()`) and S2 (gate dormant → the
/// deliberately preserved pre-M5 assignment).
#[test]
fn m6_pin_set_last_finality_height_is_monotone_above_the_gate() {
    let mut active = ReorgHandler::with_activation_heights(u64::MAX, 0);
    active.set_last_finality_height(100);
    active.set_last_finality_height(50);
    assert_eq!(
        active.last_finality_height(),
        Some(100),
        "INV-FINALITY-001 (1): above the fork-choice gate set_last_finality_height must \
         take max() — finality may never move DOWN"
    );
    active.set_last_finality_height(150);
    assert_eq!(
        active.last_finality_height(),
        Some(150),
        "INV-FINALITY-001 (1): the monotone setter must still accept a HIGHER checkpoint"
    );

    let mut dormant = ReorgHandler::with_activation_heights(u64::MAX, u64::MAX);
    dormant.set_last_finality_height(100);
    dormant.set_last_finality_height(50);
    assert_eq!(
        dormant.last_finality_height(),
        Some(50),
        "M5 dormant window: below the gate the pre-M5 bare assignment is preserved \
         byte-for-byte. This is the LIVE path on mainnet and testnet — the mirror \
         deletion must carry the monotone rule down with it, not assume it already holds."
    );
}

/// INV-SYNC-004 — Decision: a failure here reveals that the FINALITY_GUARD
/// stale-clearing backstop stopped firing, which is INC-I-081 Bug 4: a stale
/// finality height after a rollback permanently blocks every future reorg.
///
/// Covers O2 on paths C1 (clear) and C2 (no-op at and above the boundary).
#[test]
fn m6_pin_finality_guard_backstop_still_clears_after_a_rollback_below_tip() {
    let mut handler = ReorgHandler::with_activation_heights(u64::MAX, u64::MAX);
    handler.set_last_finality_height(100);
    handler.clear_finality_if_below_tip(99);
    assert_eq!(
        handler.last_finality_height(),
        None,
        "INV-SYNC-004: tip 99 is below finality 100 — the stale marker MUST be cleared"
    );

    let mut at_boundary = ReorgHandler::with_activation_heights(u64::MAX, u64::MAX);
    at_boundary.set_last_finality_height(100);
    at_boundary.clear_finality_if_below_tip(100);
    assert_eq!(
        at_boundary.last_finality_height(),
        Some(100),
        "INV-SYNC-004 fencepost: tip == finality is NOT below it — no clear"
    );

    let mut above = ReorgHandler::with_activation_heights(u64::MAX, u64::MAX);
    above.set_last_finality_height(100);
    above.clear_finality_if_below_tip(110);
    assert_eq!(
        above.last_finality_height(),
        Some(100),
        "INV-SYNC-004: a tip above finality must leave the marker alone"
    );
}

// ===========================================================================
// The precondition that makes the mirror deletion premature
// ===========================================================================

/// REQ-FORK-004 (brief §5 R1) — Decision: a failure here reveals that a real
/// fork-choice activation height has been pinned, which is the exact condition
/// under which the finality-mirror bytes, `with_activation_height` (singular) and
/// `should_reorg_by_weight_with_tiebreak` become deletable. Until then the
/// below-gate path is the only live path and deleting the mirror would leave every
/// mainnet and testnet node with no finality height at all (trap T12).
#[test]
fn m6_pin_fork_choice_gate_is_still_dormant_on_mainnet_and_testnet() {
    for network in [Network::Mainnet, Network::Testnet] {
        let params = NetworkParams::defaults(network);
        assert_eq!(
            params.inc_i_204_fork_choice_activation_height,
            u64::MAX,
            "brief §5 R1: {network:?} now pins a real inc_i_204_fork_choice_activation_height \
             ({}). The mirror-deletion precondition is met — delete \
             ReorgHandler.last_finality_height, with_activation_height (singular) and \
             should_reorg_by_weight_with_tiebreak, and move the pins in this file onto \
             ForkChoiceFinality.",
            params.inc_i_204_fork_choice_activation_height
        );
    }
}
