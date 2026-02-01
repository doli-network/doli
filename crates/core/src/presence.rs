//! # Presence Commitment for Blocks
//!
//! This module provides the `PresenceCommitment` structure that records
//! which producers were present during a slot and their bond weights.
//!
//! ## Purpose
//!
//! Each block contains a presence commitment that records:
//! - Which producers submitted valid heartbeats (bitfield)
//! - Bond weights of present producers (for reward calculation)
//! - Merkle root of full heartbeat data (for fraud proofs)
//!
//! ## Weighted Presence Rewards
//!
//! Rewards are distributed proportionally based on bond weight:
//!
//! ```text
//! For each block where producer was present:
//!   producer_reward += block_reward × producer_weight / total_present_weight
//! ```
//!
//! ## Storage Efficiency
//!
//! The bitfield uses 1 bit per registered producer, making presence tracking
//! very compact. For 100 producers, only 13 bytes are needed for the bitfield.

use crypto::Hash;
use serde::{Deserialize, Serialize};

use crate::types::Amount;

// =============================================================================
// PRESENCE COMMITMENT
// =============================================================================

/// Compact presence commitment stored in each block.
///
/// Records which producers were present during this slot and their bond weights.
/// This data is used for calculating weighted presence rewards.
///
/// # Storage Overhead
///
/// | Component | Size |
/// |-----------|------|
/// | Bitfield | ~13 bytes (for 100 producers) |
/// | Merkle root | 32 bytes |
/// | Weights | 8 bytes × present count |
/// | Total weight | 8 bytes |
///
/// Total: ~700 bytes per block (assuming ~80 present producers)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceCommitment {
    /// Bitfield: 1 bit per registered producer (sorted by pubkey).
    /// Bit i = 1 means producer i was present this slot.
    pub bitfield: Vec<u8>,

    /// Merkle root of full heartbeat data (for fraud proofs).
    /// Allows verification of individual heartbeats without storing all data.
    pub merkle_root: Hash,

    /// Bond weights of present producers (in bitfield order).
    /// Only includes weights for producers with bit=1.
    /// Length must equal the popcount of the bitfield.
    pub weights: Vec<Amount>,

    /// Cached sum of all weights (for quick reward calculation).
    /// Invariant: total_weight == weights.iter().sum()
    pub total_weight: Amount,
}

impl Default for PresenceCommitment {
    fn default() -> Self {
        Self::empty()
    }
}

impl PresenceCommitment {
    /// Create an empty presence commitment (no producers present).
    pub fn empty() -> Self {
        Self {
            bitfield: Vec::new(),
            merkle_root: Hash::ZERO,
            weights: Vec::new(),
            total_weight: 0,
        }
    }

    /// Create a new presence commitment.
    ///
    /// # Arguments
    ///
    /// * `producer_count` - Total number of registered producers
    /// * `present_indices` - Indices of producers who were present (must be sorted)
    /// * `weights` - Bond weights of present producers (same order as present_indices)
    /// * `merkle_root` - Merkle root of full heartbeat data
    ///
    /// # Panics
    ///
    /// Panics if `present_indices.len() != weights.len()`.
    pub fn new(
        producer_count: usize,
        present_indices: &[usize],
        weights: Vec<Amount>,
        merkle_root: Hash,
    ) -> Self {
        assert_eq!(
            present_indices.len(),
            weights.len(),
            "present_indices and weights must have same length"
        );

        // Create bitfield with enough bytes for all producers
        let mut bitfield = vec![0u8; producer_count.div_ceil(8)];

        // Set bits for present producers
        for &idx in present_indices {
            if idx < producer_count {
                bitfield[idx / 8] |= 1 << (idx % 8);
            }
        }

        let total_weight = weights.iter().sum();

        Self {
            bitfield,
            merkle_root,
            weights,
            total_weight,
        }
    }

    /// Check if producer at index was present.
    ///
    /// Returns false if the index is out of bounds.
    #[inline]
    pub fn is_present(&self, producer_index: usize) -> bool {
        let byte_idx = producer_index / 8;
        let bit_idx = producer_index % 8;

        if byte_idx >= self.bitfield.len() {
            return false;
        }

        (self.bitfield[byte_idx] & (1 << bit_idx)) != 0
    }

    /// Get weight for producer if present.
    ///
    /// Returns `None` if the producer was not present or index is out of bounds.
    pub fn get_weight(&self, producer_index: usize) -> Option<Amount> {
        if !self.is_present(producer_index) {
            return None;
        }

        // Count set bits before this index to find weight array position
        let weight_idx = self.count_present_before(producer_index);
        self.weights.get(weight_idx).copied()
    }

    /// Count how many producers were present before the given index.
    ///
    /// This is used to map from producer index to weight array index.
    fn count_present_before(&self, producer_index: usize) -> usize {
        let mut count = 0;

        for i in 0..producer_index {
            if self.is_present(i) {
                count += 1;
            }
        }

        count
    }

    /// Get the number of present producers.
    #[inline]
    pub fn present_count(&self) -> usize {
        self.weights.len()
    }

    /// Get the total weight of all present producers.
    #[inline]
    pub fn total_weight(&self) -> Amount {
        self.total_weight
    }

    /// Check if no producers were present.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.weights.is_empty()
    }

    /// Get the maximum producer index that can be represented.
    pub fn max_producer_index(&self) -> usize {
        self.bitfield.len() * 8
    }

    /// Estimate serialized size in bytes.
    pub fn size(&self) -> usize {
        // bitfield length + bitfield + merkle_root + weights + total_weight
        4 + self.bitfield.len() + 32 + (self.weights.len() * 8) + 8
    }

    /// Serialize the presence commitment.
    pub fn serialize(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    /// Deserialize a presence commitment.
    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        bincode::deserialize(bytes).ok()
    }

    /// Verify that the stored total_weight matches the sum of weights.
    pub fn verify_total_weight(&self) -> bool {
        let computed: Amount = self.weights.iter().sum();
        self.total_weight == computed
    }

    /// Verify that the number of set bits in bitfield equals weights.len().
    pub fn verify_bitfield_weight_count(&self) -> bool {
        let set_bits = self
            .bitfield
            .iter()
            .map(|b| b.count_ones() as usize)
            .sum::<usize>();
        set_bits == self.weights.len()
    }

    /// Iterate over present producer indices and their weights.
    pub fn iter_present(&self) -> impl Iterator<Item = (usize, Amount)> + '_ {
        PresenceIterator {
            commitment: self,
            producer_index: 0,
            weight_index: 0,
        }
    }
}

/// Iterator over present producers and their weights.
struct PresenceIterator<'a> {
    commitment: &'a PresenceCommitment,
    producer_index: usize,
    weight_index: usize,
}

impl<'a> Iterator for PresenceIterator<'a> {
    type Item = (usize, Amount);

    fn next(&mut self) -> Option<Self::Item> {
        let max_index = self.commitment.max_producer_index();

        while self.producer_index < max_index {
            let idx = self.producer_index;
            self.producer_index += 1;

            if self.commitment.is_present(idx) {
                if let Some(&weight) = self.commitment.weights.get(self.weight_index) {
                    self.weight_index += 1;
                    return Some((idx, weight));
                }
            }
        }

        None
    }
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_commitment() {
        let commitment = PresenceCommitment::empty();

        assert!(commitment.is_empty());
        assert_eq!(commitment.present_count(), 0);
        assert_eq!(commitment.total_weight(), 0);
        assert!(!commitment.is_present(0));
        assert!(commitment.get_weight(0).is_none());
    }

    #[test]
    fn test_bitfield_encoding() {
        // 10 producers, indices 2, 5, 7 present
        let present = [2, 5, 7];
        let weights = vec![1000, 2000, 3000];
        let commitment = PresenceCommitment::new(10, &present, weights.clone(), Hash::ZERO);

        // Check bitfield size
        assert_eq!(commitment.bitfield.len(), 2); // ceil(10/8) = 2 bytes

        // Check presence
        assert!(!commitment.is_present(0));
        assert!(!commitment.is_present(1));
        assert!(commitment.is_present(2));
        assert!(!commitment.is_present(3));
        assert!(!commitment.is_present(4));
        assert!(commitment.is_present(5));
        assert!(!commitment.is_present(6));
        assert!(commitment.is_present(7));
        assert!(!commitment.is_present(8));
        assert!(!commitment.is_present(9));

        // Out of bounds
        assert!(!commitment.is_present(100));
    }

    #[test]
    fn test_get_weight() {
        // 5 producers, indices 1, 3 present with weights 100, 200
        let present = [1, 3];
        let weights = vec![100, 200];
        let commitment = PresenceCommitment::new(5, &present, weights, Hash::ZERO);

        assert_eq!(commitment.get_weight(0), None);
        assert_eq!(commitment.get_weight(1), Some(100));
        assert_eq!(commitment.get_weight(2), None);
        assert_eq!(commitment.get_weight(3), Some(200));
        assert_eq!(commitment.get_weight(4), None);
    }

    #[test]
    fn test_total_weight() {
        let present = [0, 2, 4];
        let weights = vec![1000, 2000, 3000];
        let commitment = PresenceCommitment::new(5, &present, weights, Hash::ZERO);

        assert_eq!(commitment.total_weight(), 6000);
        assert_eq!(commitment.present_count(), 3);
    }

    #[test]
    fn test_verify_total_weight() {
        let present = [0, 1];
        let weights = vec![100, 200];
        let commitment = PresenceCommitment::new(10, &present, weights, Hash::ZERO);

        assert!(commitment.verify_total_weight());
    }

    #[test]
    fn test_verify_bitfield_weight_count() {
        let present = [0, 2, 4, 6];
        let weights = vec![10, 20, 30, 40];
        let commitment = PresenceCommitment::new(10, &present, weights, Hash::ZERO);

        assert!(commitment.verify_bitfield_weight_count());
    }

    #[test]
    fn test_iter_present() {
        let present = [1, 3, 5];
        let weights = vec![100, 200, 300];
        let commitment = PresenceCommitment::new(8, &present, weights, Hash::ZERO);

        let collected: Vec<_> = commitment.iter_present().collect();
        assert_eq!(collected, vec![(1, 100), (3, 200), (5, 300)]);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let present = [0, 5, 10];
        let weights = vec![1000, 2000, 3000];
        let merkle = Hash::from_bytes([42u8; 32]);
        let commitment = PresenceCommitment::new(16, &present, weights, merkle);

        let serialized = commitment.serialize();
        let deserialized = PresenceCommitment::deserialize(&serialized).unwrap();

        assert_eq!(commitment, deserialized);
    }

    #[test]
    fn test_large_producer_set() {
        // 100 producers, every 5th one present
        let present: Vec<usize> = (0..100).filter(|i| i % 5 == 0).collect();
        let weights: Vec<Amount> = present.iter().map(|i| (*i as Amount + 1) * 1000).collect();
        let commitment = PresenceCommitment::new(100, &present, weights.clone(), Hash::ZERO);

        // Bitfield size
        assert_eq!(commitment.bitfield.len(), 13); // ceil(100/8) = 13 bytes

        // Present count
        assert_eq!(commitment.present_count(), 20);

        // Check some values
        assert!(commitment.is_present(0));
        assert!(!commitment.is_present(1));
        assert!(commitment.is_present(5));
        assert!(commitment.is_present(95));
        assert!(!commitment.is_present(99));

        // Weights
        assert_eq!(commitment.get_weight(0), Some(1000));
        assert_eq!(commitment.get_weight(5), Some(6000));
        assert_eq!(commitment.get_weight(95), Some(96000));
    }

    #[test]
    fn test_size_estimation() {
        let present = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]; // 10 present
        let weights = vec![1000; 10];
        let commitment = PresenceCommitment::new(100, &present, weights, Hash::ZERO);

        // Size should be: 4 (len prefix) + 13 (bitfield) + 32 (merkle) + 80 (10*8 weights) + 8 (total)
        // = 137 bytes
        let size = commitment.size();
        assert!(size > 100);
        assert!(size < 200);
    }

    #[test]
    #[should_panic(expected = "present_indices and weights must have same length")]
    fn test_mismatched_lengths_panics() {
        let present = [0, 1, 2];
        let weights = vec![100, 200]; // Wrong length!
        PresenceCommitment::new(10, &present, weights, Hash::ZERO);
    }

    #[test]
    fn test_default() {
        let commitment = PresenceCommitment::default();
        assert!(commitment.is_empty());
        assert_eq!(commitment, PresenceCommitment::empty());
    }
}
