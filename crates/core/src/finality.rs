//! Finality gadget for the DOLI protocol.
//!
//! Tracks attestation weight for blocks and determines when a block
//! has reached finality (2/3+ of total weight). Once finalized,
//! blocks cannot be reverted by any fork choice rule.

use std::collections::HashMap;

use crypto::{Hash, PublicKey};
use serde::{Deserialize, Serialize};

/// Percentage of total weight required for finality (67%).
pub const FINALITY_THRESHOLD_PCT: u32 = 67;

/// Minimum locally-applied descendant depth before a pending block may finalize
/// (INC-I-190 D1 [F2]). A block at height `h` finalizes only once the node has
/// applied a descendant at height `>= h + CONFIRMATION_DEPTH`, eliminating the
/// depth-0 instant self-finality that made the wedge irreversible.
pub const CONFIRMATION_DEPTH: u64 = 2;

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
    attesters: HashMap<PublicKey, u64>,
    total_weight: u64,
}

impl PendingBlock {
    fn numerator(&self) -> u64 {
        self.attesters.values().copied().sum()
    }
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
    /// Buffered attestation weight for blocks not yet tracked, keyed by attester.
    /// When `track_block()` is called, any buffered per-attester weight is applied.
    early_attestations: HashMap<Hash, HashMap<PublicKey, u64>>,
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
        let early = self.early_attestations.remove(&hash).unwrap_or_default();

        self.pending.push(PendingBlock {
            block_hash: hash,
            height,
            slot,
            attesters: early,
            total_weight,
        });
    }

    /// Add attestation weight to a pending block.
    ///
    /// If the block is not yet tracked, the weight is buffered and will be
    /// applied when `track_block()` is called.
    pub fn add_attestation_weight(&mut self, block_hash: Hash, attester: PublicKey, weight: u64) {
        for pending in &mut self.pending {
            if pending.block_hash == block_hash {
                pending.attesters.insert(attester, weight);
                return;
            }
        }

        // Block not yet tracked — buffer the attestation, deduped by attester
        self.early_attestations
            .entry(block_hash)
            .or_default()
            .insert(attester, weight);

        // Evict an arbitrary block-hash entry if buffer is full (size cap on
        // the number of distinct block hashes buffered)
        if self.early_attestations.len() > MAX_EARLY_ATTESTATIONS {
            // Remove an arbitrary entry (HashMap iteration order)
            if let Some(&key) = self.early_attestations.keys().next() {
                self.early_attestations.remove(&key);
            }
        }
    }

    /// Check if any pending blocks have reached finality.
    ///
    /// A block finalizes only when it has BOTH >= `FINALITY_THRESHOLD_PCT` weight
    /// AND a locally-applied descendant at depth >= `CONFIRMATION_DEPTH`
    /// (`applied_tip_height >= block.height + CONFIRMATION_DEPTH`).
    ///
    /// Returns the newly finalized checkpoint if one was found.
    /// Removes all pending blocks at or below the finalized height.
    pub fn check_finality(&mut self, applied_tip_height: u64) -> Option<FinalityCheckpoint> {
        // Find the highest-height block that meets the threshold
        let mut best: Option<usize> = None;

        for (i, pending) in self.pending.iter().enumerate() {
            if pending.total_weight == 0 {
                continue;
            }
            let pct = pending.numerator() * 100 / pending.total_weight;
            // INC-I-190 D1 [F2]: require a locally-applied descendant at depth
            // >= CONFIRMATION_DEPTH — no depth-0 instant self-finality.
            let has_confirmation_depth =
                applied_tip_height >= pending.height.saturating_add(CONFIRMATION_DEPTH);
            if pct >= FINALITY_THRESHOLD_PCT as u64 && has_confirmation_depth {
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
                attestation_weight: p.numerator(),
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
    use crypto::PublicKey;

    fn make_hash(seed: u8) -> Hash {
        crypto::hash::hash(&[seed])
    }

    fn make_pubkey(seed: u8) -> PublicKey {
        PublicKey::from_bytes([seed; 32])
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
        tracker.add_attestation_weight(h1, make_pubkey(101), 50);
        assert!(tracker.check_finality(102).is_none());

        // Block 1 reaches 67% — a SECOND producer adds 17 (50 + 17 = 67).
        tracker.add_attestation_weight(h1, make_pubkey(102), 17);
        let cp = tracker.check_finality(102);
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
        tracker.add_attestation_weight(h, make_pubkey(1), 70);
        tracker.check_finality(102);

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
        tracker.add_attestation_weight(h, make_pubkey(1), 70);
        assert!(tracker.pending.is_empty());

        // Now track the block — buffered weight should be applied
        tracker.track_block(h, 100, 10, 100);
        assert_eq!(tracker.pending.len(), 1);
        assert_eq!(tracker.pending[0].numerator(), 70);

        // Should reach finality once depth-2 is satisfied
        let cp = tracker.check_finality(102);
        assert!(cp.is_some());
        assert_eq!(cp.unwrap().block_hash, h);
    }

    // OUTPUT CONTRACT (per .claude/protocols/output-contract.md):
    //   Output: FinalityTracker::check_finality(applied_tip_height) -> Option<checkpoint>
    //           + self.last_finalized side effect.
    // INPUT PARTITIONS (67% weight held constant):
    //   - applied_tip == block.height        (depth 0) => None
    //   - applied_tip == block.height + 1     (depth 1) => None
    //   - applied_tip >= block.height + 2     (depth 2) => Some(checkpoint)
    #[test]
    fn test_no_depth0_self_finality() {
        // INC-I-190 D1 [F2]: a 67% block must NOT finalize at depth 0/1.
        let mut tracker = FinalityTracker::new();
        let h = make_hash(1);
        tracker.track_block(h, 100, 10, 100);
        tracker.add_attestation_weight(h, make_pubkey(1), 67); // 67% weight, no descendants yet

        assert!(
            tracker.check_finality(100).is_none(),
            "depth-0 must not finalize"
        );
        assert!(
            tracker.check_finality(101).is_none(),
            "depth-1 must not finalize"
        );
        let cp = tracker.check_finality(102);
        assert!(cp.is_some(), "depth-2 must finalize");
        assert_eq!(cp.unwrap().height, 100);
    }

    #[test]
    fn test_normal_finality_at_depth2_no_liveness_regression() {
        // A block with 67% AND >= 2 applied descendants finalizes normally.
        let mut tracker = FinalityTracker::new();
        let h = make_hash(1);
        tracker.track_block(h, 200, 20, 100);
        tracker.add_attestation_weight(h, make_pubkey(1), 67);
        let cp = tracker.check_finality(202); // depth 2
        assert!(
            cp.is_some(),
            "67% + depth 2 must finalize (no liveness stall)"
        );
        assert_eq!(cp.unwrap().height, 200);
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

    // OUTPUT CONTRACT (M3 [F1-dedup], per .claude/protocols/output-contract.md):
    //   Output: PendingBlock::numerator() = attesters.values().sum(); FinalityCheckpoint.attestation_weight.
    // INPUT PARTITIONS (dedup by attester):
    //   - same attester, N deliveries          => numerator = that attester's weight (counted once)
    //   - K distinct attesters, weight w each   => numerator = K*w, always <= total_weight
    //   - echo/re-broadcast of a counted attester => numerator unchanged
    #[test]
    fn test_duplicate_attester_counts_once() {
        // M3 [F1-dedup]: same authenticated attester delivered 3x counts ONCE.
        let mut tracker = FinalityTracker::new();
        let h = make_hash(1);
        let a = make_pubkey(9);
        tracker.track_block(h, 100, 10, 5); // total_weight = 5
        tracker.add_attestation_weight(h, a, 1);
        tracker.add_attestation_weight(h, a, 1);
        tracker.add_attestation_weight(h, a, 1);
        assert_eq!(
            tracker.pending[0].numerator(),
            1,
            "same attester delivered 3x must count once, not 3"
        );
    }

    #[test]
    fn test_numerator_never_exceeds_total() {
        // M3 [F1-dedup]: 5 producers each weight 1 (total 5); one attests twice.
        let mut tracker = FinalityTracker::new();
        let h = make_hash(2);
        tracker.track_block(h, 100, 10, 5);
        let attesters: Vec<PublicKey> = (0..5u8).map(make_pubkey).collect();
        for a in &attesters {
            tracker.add_attestation_weight(h, *a, 1);
        }
        // Echo: one producer attests a second time.
        tracker.add_attestation_weight(h, attesters[0], 1);
        assert_eq!(
            tracker.pending[0].numerator(),
            5,
            "numerator must equal distinct producer weight, never 6"
        );
        let cp = tracker
            .check_finality(102)
            .expect("100% weight + depth 2 must finalize");
        assert!(
            cp.attestation_weight <= cp.total_weight,
            "numerator must never exceed total_weight"
        );
        assert_eq!(cp.attestation_weight, 5);
    }
}
