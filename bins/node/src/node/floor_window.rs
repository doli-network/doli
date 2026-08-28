//! Lifetime bound for the INC-I-190 floor-fallback Light-validation window.
//!
//! The window covers a producer-list divergence that the floor fallback itself
//! pins, so it cannot expire on a fixed schedule (AUDIT-P1-501). It also cannot
//! run forever: `Light` skips producer eligibility AND the VDF, so an attestation
//! blackout that never ends would leave the node accepting blocks with no
//! proof-of-elapsed-time. Bounded validation against a possibly-divergent list is
//! the safer end of that trade — the mismatch it leaves is node-local, is not in
//! the state root, and self-heals at the next non-floor boundary.

use doli_core::consensus::FLOOR_FALLBACK_WINDOW_MAX_BOUNDARIES;
use doli_core::epoch_state::FloorOutcome;
use tracing::{error, info, warn};

use super::Node;

/// What an epoch-boundary derivation does to an armed window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloorWindowAction {
    /// The derivation no longer consumes `prev.producer_list`, so it cannot carry
    /// our divergence forward. Drop back to Full.
    Converged,
    /// Preference (a) is still pinning our list. Hold, carrying the new count.
    Hold(u8),
    /// The bound is spent while the fallback is still firing. Force Full anyway.
    ForceExpire,
}

/// Pure transition for one epoch boundary observed by an ARMED window.
///
/// `PreviousEpochList` is the only floor exit that reads `prev.producer_list`
/// (`floor.rs:159-175`); `BoundedActiveSet` and `NotTriggered` are computed from
/// `active_producers`, `registered_at` and `is_ghost` alone, so either one means
/// the live derivation has converged onto the same inputs the armed node used.
pub fn on_boundary(boundaries: u8, outcome: FloorOutcome) -> FloorWindowAction {
    if outcome != FloorOutcome::PreviousEpochList {
        return FloorWindowAction::Converged;
    }
    let next = boundaries.saturating_add(1);
    if next >= FLOOR_FALLBACK_WINDOW_MAX_BOUNDARIES {
        FloorWindowAction::ForceExpire
    } else {
        FloorWindowAction::Hold(next)
    }
}

impl Node {
    /// Advance the floor-fallback window at an epoch boundary. No-op when disarmed.
    pub(crate) fn advance_floor_window(&mut self, outcome: FloorOutcome, epoch: u64, height: u64) {
        if !self.floor_fallback_window {
            return;
        }
        match on_boundary(self.floor_fallback_boundaries, outcome) {
            FloorWindowAction::Converged => {
                info!(
                    "[FLOOR_BOUND] Derivation at epoch {} no longer consumes the previous producer_list (branch={:?}) — switching gossip validation to Full mode",
                    epoch, outcome
                );
                self.floor_fallback_window = false;
                self.floor_fallback_boundaries = 0;
            }
            FloorWindowAction::Hold(next) => {
                self.floor_fallback_boundaries = next;
                warn!(
                    "[FLOOR_BOUND] Light-validation window held at epoch {} h={} — floor fallback still pinning our producer_list ({}/{} boundaries)",
                    epoch, height, next, FLOOR_FALLBACK_WINDOW_MAX_BOUNDARIES
                );
            }
            FloorWindowAction::ForceExpire => {
                error!(
                    "[FLOOR_BOUND] Light-validation window EXPIRED at epoch {} h={} after {} consecutive floor-fallback boundaries — forcing Full validation. Our producer_list may still differ from the fleet's and this node may reject valid gossip on eligibility until the attestation blackout ends. Operator: check attestation coverage across the fleet.",
                    epoch, height, FLOOR_FALLBACK_WINDOW_MAX_BOUNDARIES
                );
                self.floor_fallback_window = false;
                self.floor_fallback_boundaries = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // OUTPUT CONTRACT
    //   O1 FloorWindowAction returned by on_boundary
    // PATHS   Converged | Hold | ForceExpire
    // INPUT PARTITIONS  outcome in {NotTriggered, BoundedActiveSet, LegacyUnbounded,
    //   PreviousEpochList} x boundaries in {0, 1, MAX-1, u8::MAX}

    /// REV-I190-M4-F7: BoundedActiveSet never reads `prev`, so it disarms exactly
    /// as NotTriggered does. RED before the fix (only NotTriggered disarmed).
    #[test]
    fn floor_window_disarms_on_bounded_active_set() {
        assert_eq!(
            on_boundary(0, FloorOutcome::BoundedActiveSet),
            FloorWindowAction::Converged
        );
        assert_eq!(
            on_boundary(1, FloorOutcome::BoundedActiveSet),
            FloorWindowAction::Converged
        );
    }

    #[test]
    fn floor_window_disarms_on_not_triggered() {
        assert_eq!(
            on_boundary(0, FloorOutcome::NotTriggered),
            FloorWindowAction::Converged
        );
        assert_eq!(
            on_boundary(0, FloorOutcome::LegacyUnbounded),
            FloorWindowAction::Converged
        );
    }

    /// AUDIT-P1-501: a sustained blackout must not hold Light open forever.
    #[test]
    fn floor_window_force_expires_after_max_consecutive_floor_boundaries() {
        assert_eq!(FLOOR_FALLBACK_WINDOW_MAX_BOUNDARIES, 2);

        let mut count = 0u8;
        let mut survived = 0u32;
        loop {
            match on_boundary(count, FloorOutcome::PreviousEpochList) {
                FloorWindowAction::Hold(next) => {
                    count = next;
                    survived += 1;
                    assert!(survived < 10, "window never expired — unbounded");
                }
                FloorWindowAction::ForceExpire => break,
                FloorWindowAction::Converged => panic!("PreviousEpochList must not converge"),
            }
        }
        assert_eq!(
            survived,
            u32::from(FLOOR_FALLBACK_WINDOW_MAX_BOUNDARIES) - 1,
            "the window must survive exactly MAX-1 holds before expiring"
        );
    }

    /// The counter must never wrap back under the bound.
    #[test]
    fn floor_window_counter_saturates() {
        assert_eq!(
            on_boundary(u8::MAX, FloorOutcome::PreviousEpochList),
            FloorWindowAction::ForceExpire
        );
    }
}
