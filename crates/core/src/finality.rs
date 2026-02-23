//! Finality gadget for the DOLI protocol.
//!
//! Tracks attestation weight for blocks and determines when a block
//! has reached finality (2/3+ of total weight). Once finalized,
//! blocks cannot be reverted by any fork choice rule.

use std::collections::HashMap;

use crypto::Hash;
use serde::{Deserialize, Serialize};

/// Percentage of total weight required for finality (67%).
pub const FINALITY_THRESHOLD_PCT: u32 = 67;

/// Number of slots to wait before timing out pending finality.
pub const FINALITY_TIMEOUT_SLOTS: u32 = 3;

/// A finalized checkpoint — a block that has received 2/3+ attestation weight.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalityCheckpoint {
    /// Hash of the finalized block.
    pub block_hash: Hash,
    /// Height of the finalized block.
    pub height: u64,
    /// Slot of the finalized block.
    pub slot: u32,
    /// Total attestation weight received.
    pub attestation_weight: u64,
    /// Total network weight at the time of finalization.
    pub total_weight: u64,
}

impl FinalityCheckpoint {
    /// Check if the checkpoint has reached finality threshold.
    pub fn is_finalized(&self) -> bool {
        self.total_weight > 0
            && self.attestation_weight * 100 / self.total_weight >= FINALITY_THRESHOLD_PCT as u64
    }
}

/// Pending block awaiting finality.
#[derive(Clone, Debug)]
struct PendingBlock {
    block_hash: Hash,
    height: u64,
    slot: u32,
    attestation_weight: u64,
    total_weight: u64,
}

/// Maximum number of early attestations to buffer before any block is tracked.
const MAX_EARLY_ATTESTATIONS: usize = 100;

/// Tracks blocks awaiting finality and maintains the last finalized checkpoint.
#[derive(Clone, Debug)]
pub struct FinalityTracker {
    /// Last finalized block (if any).
    pub last_finalized: Option<FinalityCheckpoint>,
    /// Blocks awaiting sufficient attestation weight.
    pending: Vec<PendingBlock>,
    /// Buffered attestation weight for blocks not yet tracked.
    /// When `track_block()` is called, any buffered weight is applied.
    early_attestations: HashMap<Hash, u64>,
}

impl FinalityTracker {
    /// Create a new finality tracker with no finalized blocks.
    pub fn new() -> Self {
        Self {
            last_finalized: None,
            pending: Vec::new(),
            early_attestations: HashMap::new(),
        }
    }

    /// Start tracking a new block for finality.
    pub fn track_block(&mut self, hash: Hash, height: u64, slot: u32, total_weight: u64) {
        // Don't track duplicates
        if self.pending.iter().any(|p| p.block_hash == hash) {
            return;
        }

        // Check for any buffered early attestations
        let early_weight = self.early_attestations.remove(&hash).unwrap_or(0);

        self.pending.push(PendingBlock {
            block_hash: hash,
            height,
            slot,
            attestation_weight: early_weight,
            total_weight,
        });
    }

    /// Add attestation weight to a pending block.
    ///
    /// If the block is not yet tracked, the weight is buffered and will be
    /// applied when `track_block()` is called.
    pub fn add_attestation_weight(&mut self, block_hash: Hash, weight: u64) {
        for pending in &mut self.pending {
            if pending.block_hash == block_hash {
                pending.attestation_weight = pending.attestation_weight.saturating_add(weight);
                return;
            }
        }

        // Block not yet tracked — buffer the attestation
        let entry = self.early_attestations.entry(block_hash).or_insert(0);
        *entry = entry.saturating_add(weight);

        // Evict oldest entry if buffer is full (simple size cap)
        if self.early_attestations.len() > MAX_EARLY_ATTESTATIONS {
            // Remove an arbitrary entry (HashMap iteration order)
            if let Some(&key) = self.early_attestations.keys().next() {
                self.early_attestations.remove(&key);
            }
        }
    }

    /// Check if any pending blocks have reached finality.
    ///
    /// Returns the newly finalized checkpoint if one was found.
    /// Removes all pending blocks at or below the finalized height.
    pub fn check_finality(&mut self) -> Option<FinalityCheckpoint> {
        // Find the highest-height block that meets the threshold
        let mut best: Option<usize> = None;

        for (i, pending) in self.pending.iter().enumerate() {
            if pending.total_weight == 0 {
                continue;
            }
            let pct = pending.attestation_weight * 100 / pending.total_weight;
            if pct >= FINALITY_THRESHOLD_PCT as u64 {
                match best {
                    Some(bi) if pending.height > self.pending[bi].height => {
                        best = Some(i);
                    }
                    None => {
                        best = Some(i);
                    }
                    _ => {}
                }
            }
        }

        if let Some(idx) = best {
            let p = &self.pending[idx];
            let checkpoint = FinalityCheckpoint {
                block_hash: p.block_hash,
                height: p.height,
                slot: p.slot,
                attestation_weight: p.attestation_weight,
                total_weight: p.total_weight,
            };

            // Remove all pending at or below finalized height
            let finalized_height = checkpoint.height;
            self.pending.retain(|p| p.height > finalized_height);
            self.last_finalized = Some(checkpoint.clone());

            Some(checkpoint)
        } else {
            None
        }
    }

    /// Check if a given block hash is at or below the last finalized height.
    pub fn is_at_or_below_finalized(&self, height: u64) -> bool {
        self.last_finalized
            .as_ref()
            .is_some_and(|f| height <= f.height)
    }

    /// Prune pending blocks older than a given slot (timeout).
    pub fn prune_old_pending(&mut self, min_slot: u32) {
        self.pending
            .retain(|p| p.slot + FINALITY_TIMEOUT_SLOTS >= min_slot);
    }
}

impl Default for FinalityTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hash(seed: u8) -> Hash {
        crypto::hash::hash(&[seed])
    }

    #[test]
    fn test_finality_at_67_pct() {
        let checkpoint = FinalityCheckpoint {
            block_hash: make_hash(1),
            height: 100,
            slot: 10,
            attestation_weight: 67,
            total_weight: 100,
        };
        assert!(checkpoint.is_finalized());
    }

    #[test]
    fn test_no_finality_at_66_pct() {
        let checkpoint = FinalityCheckpoint {
            block_hash: make_hash(1),
            height: 100,
            slot: 10,
            attestation_weight: 66,
            total_weight: 100,
        };
        assert!(!checkpoint.is_finalized());
    }

    #[test]
    fn test_no_finality_zero_weight() {
        let checkpoint = FinalityCheckpoint {
            block_hash: make_hash(1),
            height: 100,
            slot: 10,
            attestation_weight: 0,
            total_weight: 0,
        };
        assert!(!checkpoint.is_finalized());
    }

    #[test]
    fn test_finality_tracker_progression() {
        let mut tracker = FinalityTracker::new();

        let h1 = make_hash(1);
        let h2 = make_hash(2);

        tracker.track_block(h1, 100, 10, 100);
        tracker.track_block(h2, 101, 11, 100);

        // Block 1 gets 50% weight — not enough
        tracker.add_attestation_weight(h1, 50);
        assert!(tracker.check_finality().is_none());

        // Block 1 reaches 67% — finalized
        tracker.add_attestation_weight(h1, 17);
        let cp = tracker.check_finality();
        assert!(cp.is_some());
        let cp = cp.unwrap();
        assert_eq!(cp.block_hash, h1);
        assert_eq!(cp.height, 100);
        assert!(cp.is_finalized());

        // Block 1 should be pruned, block 2 should still be pending
        assert_eq!(tracker.pending.len(), 1);
        assert_eq!(tracker.pending[0].block_hash, h2);

        // Last finalized should be set
        assert!(tracker.last_finalized.is_some());
        assert_eq!(tracker.last_finalized.as_ref().unwrap().height, 100);
    }

    #[test]
    fn test_is_at_or_below_finalized() {
        let mut tracker = FinalityTracker::new();
        assert!(!tracker.is_at_or_below_finalized(100));

        let h = make_hash(1);
        tracker.track_block(h, 100, 10, 100);
        tracker.add_attestation_weight(h, 70);
        tracker.check_finality();

        assert!(tracker.is_at_or_below_finalized(99));
        assert!(tracker.is_at_or_below_finalized(100));
        assert!(!tracker.is_at_or_below_finalized(101));
    }

    #[test]
    fn test_duplicate_tracking_ignored() {
        let mut tracker = FinalityTracker::new();
        let h = make_hash(1);
        tracker.track_block(h, 100, 10, 100);
        tracker.track_block(h, 100, 10, 100); // duplicate
        assert_eq!(tracker.pending.len(), 1);
    }

    #[test]
    fn test_early_attestation_applied_on_track() {
        let mut tracker = FinalityTracker::new();
        let h = make_hash(1);

        // Attestation arrives before block is tracked
        tracker.add_attestation_weight(h, 70);
        assert!(tracker.pending.is_empty());

        // Now track the block — buffered weight should be applied
        tracker.track_block(h, 100, 10, 100);
        assert_eq!(tracker.pending.len(), 1);
        assert_eq!(tracker.pending[0].attestation_weight, 70);

        // Should reach finality immediately
        let cp = tracker.check_finality();
        assert!(cp.is_some());
        assert_eq!(cp.unwrap().block_hash, h);
    }

    #[test]
    fn test_prune_old_pending() {
        let mut tracker = FinalityTracker::new();
        tracker.track_block(make_hash(1), 100, 5, 100);
        tracker.track_block(make_hash(2), 101, 10, 100);
        tracker.track_block(make_hash(3), 102, 15, 100);

        tracker.prune_old_pending(12); // slot 5 + 3 = 8 < 12, slot 10 + 3 = 13 >= 12
        assert_eq!(tracker.pending.len(), 2);
    }
}
