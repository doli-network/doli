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

    /// Compute a commitment hash for inclusion in the block header.
    ///
    /// This hash commits to all the presence data and ensures that
    /// presence information cannot be modified after block creation.
    pub fn commitment_hash(&self) -> crypto::Hash {
        use crypto::Hasher;

        let mut hasher = Hasher::new();
        hasher.update(b"DOLI_PRESENCE_V1");
        hasher.update(&(self.bitfield.len() as u32).to_le_bytes());
        hasher.update(&self.bitfield);
        hasher.update(self.merkle_root.as_bytes());
        hasher.update(&(self.weights.len() as u32).to_le_bytes());
        for w in &self.weights {
            hasher.update(&w.to_le_bytes());
        }
        hasher.update(&self.total_weight.to_le_bytes());
        hasher.finalize()
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

    /// Ensure a producer is marked as present.
    ///
    /// If the producer is already present, this is a no-op.
    /// If not present, adds them with the given weight.
    ///
    /// This is used to ensure the block producer is always marked as present,
    /// even if their own heartbeat wasn't received via gossip (since gossip
    /// typically doesn't echo messages back to the sender).
    ///
    /// # Arguments
    ///
    /// * `producer_index` - Index of the producer in the sorted producer list
    /// * `weight` - Bond weight of the producer
    /// * `producer_count` - Total number of producers (for bitfield sizing)
    pub fn ensure_present(&mut self, producer_index: usize, weight: Amount, producer_count: usize) {
        // Already present? No-op.
        if self.is_present(producer_index) {
            return;
        }

        // Ensure bitfield is large enough
        let required_bytes = producer_count.div_ceil(8);
        if self.bitfield.len() < required_bytes {
            self.bitfield.resize(required_bytes, 0);
        }

        // Set the bit for this producer
        let byte_idx = producer_index / 8;
        let bit_idx = producer_index % 8;
        if byte_idx < self.bitfield.len() {
            self.bitfield[byte_idx] |= 1 << bit_idx;
        }

        // Insert weight at the correct position (maintaining sorted order by index)
        // Count how many producers with lower indices are present
        let insert_pos = self.count_present_before(producer_index);
        self.weights.insert(insert_pos, weight);
        self.total_weight += weight;
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

// =============================================================================
// PRESENCE COMMITMENT V2 (Compact - 40 bytes)
// =============================================================================

/// Compact presence commitment for V2 blocks (post-activation).
///
/// This is a much more compact representation that only stores:
/// - Merkle root of all heartbeats (for proof verification)
/// - Total weight of all present producers (for reward calculation)
///
/// Claims in V2 must provide merkle proofs to verify presence.
///
/// # Storage Overhead
///
/// | Component | Size |
/// |-----------|------|
/// | heartbeats_root | 32 bytes |
/// | total_weight | 8 bytes |
///
/// Total: **40 bytes per block** (constant, regardless of producer count)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceCommitmentV2 {
    /// Merkle root of all valid heartbeats in this slot.
    ///
    /// Each leaf is computed from CompactHeartbeat::leaf_hash().
    /// Sorted by producer pubkey for deterministic ordering.
    pub heartbeats_root: Hash,

    /// Total bond weight of all present producers.
    ///
    /// Sum of bond_amount for each producer who submitted a valid heartbeat.
    pub total_weight: u64,
}

impl Default for PresenceCommitmentV2 {
    fn default() -> Self {
        Self::empty()
    }
}

impl PresenceCommitmentV2 {
    /// Size in bytes (constant).
    pub const SIZE: usize = 32 + 8; // heartbeats_root + total_weight = 40 bytes

    /// Create an empty V2 presence commitment.
    pub fn empty() -> Self {
        Self {
            heartbeats_root: Hash::ZERO,
            total_weight: 0,
        }
    }

    /// Create a new V2 presence commitment.
    pub fn new(heartbeats_root: Hash, total_weight: u64) -> Self {
        Self {
            heartbeats_root,
            total_weight,
        }
    }

    /// Check if no producers were present.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.total_weight == 0
    }

    /// Get the total weight of all present producers.
    #[inline]
    pub fn total_weight(&self) -> u64 {
        self.total_weight
    }

    /// Compute the commitment hash for inclusion in block header.
    ///
    /// This is what goes into BlockHeader.presence_root for V2 blocks.
    pub fn commitment_hash(&self) -> Hash {
        use crypto::Hasher;

        let mut hasher = Hasher::new();
        hasher.update(b"DOLI_PRESENCE_V2");
        hasher.update(self.heartbeats_root.as_bytes());
        hasher.update(&self.total_weight.to_le_bytes());
        hasher.finalize()
    }

    /// Serialize the V2 presence commitment.
    pub fn serialize(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    /// Deserialize a V2 presence commitment.
    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        bincode::deserialize(bytes).ok()
    }

    /// Get the size in bytes (always 40).
    #[inline]
    pub fn size(&self) -> usize {
        Self::SIZE
    }
}

// =============================================================================
// PRESENCE ITERATOR
// =============================================================================

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

    #[test]
    fn test_ensure_present_adds_missing_producer() {
        // Start with producers 1 and 3 present
        let present = [1, 3];
        let weights = vec![100, 300];
        let mut commitment = PresenceCommitment::new(5, &present, weights, Hash::ZERO);

        assert_eq!(commitment.present_count(), 2);
        assert_eq!(commitment.total_weight(), 400);
        assert!(!commitment.is_present(2));

        // Add producer 2 (missing)
        commitment.ensure_present(2, 200, 5);

        assert_eq!(commitment.present_count(), 3);
        assert_eq!(commitment.total_weight(), 600);
        assert!(commitment.is_present(1));
        assert!(commitment.is_present(2));
        assert!(commitment.is_present(3));

        // Check weights are in correct order
        assert_eq!(commitment.get_weight(1), Some(100));
        assert_eq!(commitment.get_weight(2), Some(200));
        assert_eq!(commitment.get_weight(3), Some(300));
    }

    #[test]
    fn test_ensure_present_noop_if_already_present() {
        let present = [0, 2, 4];
        let weights = vec![100, 200, 300];
        let mut commitment = PresenceCommitment::new(5, &present, weights, Hash::ZERO);

        let original_count = commitment.present_count();
        let original_weight = commitment.total_weight();

        // Try to add producer 2 who is already present
        commitment.ensure_present(2, 999, 5);

        // Should be unchanged
        assert_eq!(commitment.present_count(), original_count);
        assert_eq!(commitment.total_weight(), original_weight);
        assert_eq!(commitment.get_weight(2), Some(200)); // Original weight preserved
    }

    #[test]
    fn test_ensure_present_at_beginning() {
        // Start with producers 2, 3, 4 present
        let present = [2, 3, 4];
        let weights = vec![200, 300, 400];
        let mut commitment = PresenceCommitment::new(5, &present, weights, Hash::ZERO);

        // Add producer 0 (at the beginning)
        commitment.ensure_present(0, 50, 5);

        assert_eq!(commitment.present_count(), 4);
        assert!(commitment.is_present(0));
        assert_eq!(commitment.get_weight(0), Some(50));
        assert_eq!(commitment.get_weight(2), Some(200));
        assert_eq!(commitment.get_weight(3), Some(300));
        assert_eq!(commitment.get_weight(4), Some(400));
    }

    #[test]
    fn test_ensure_present_at_end() {
        // Start with producers 0, 1, 2 present
        let present = [0, 1, 2];
        let weights = vec![100, 200, 300];
        let mut commitment = PresenceCommitment::new(5, &present, weights, Hash::ZERO);

        // Add producer 4 (at the end)
        commitment.ensure_present(4, 500, 5);

        assert_eq!(commitment.present_count(), 4);
        assert!(commitment.is_present(4));
        assert_eq!(commitment.get_weight(0), Some(100));
        assert_eq!(commitment.get_weight(1), Some(200));
        assert_eq!(commitment.get_weight(2), Some(300));
        assert_eq!(commitment.get_weight(4), Some(500));
    }

    #[test]
    fn test_ensure_present_on_empty() {
        let mut commitment = PresenceCommitment::empty();

        commitment.ensure_present(3, 1000, 5);

        assert_eq!(commitment.present_count(), 1);
        assert_eq!(commitment.total_weight(), 1000);
        assert!(commitment.is_present(3));
        assert_eq!(commitment.get_weight(3), Some(1000));
    }

    #[test]
    fn test_ensure_present_expands_bitfield() {
        // Start with small bitfield (8 producers)
        let present = [0, 1];
        let weights = vec![100, 200];
        let mut commitment = PresenceCommitment::new(8, &present, weights, Hash::ZERO);

        assert_eq!(commitment.bitfield.len(), 1); // 8 bits = 1 byte

        // Add producer at index 15 (requires 2 bytes)
        commitment.ensure_present(15, 500, 16);

        assert_eq!(commitment.bitfield.len(), 2); // Now needs 16 bits = 2 bytes
        assert!(commitment.is_present(15));
        assert_eq!(commitment.get_weight(15), Some(500));
    }

    #[test]
    fn test_commitment_hash_deterministic() {
        let present = [1, 3, 5];
        let weights = vec![100, 200, 300];
        let merkle = Hash::from_bytes([42u8; 32]);
        let commitment = PresenceCommitment::new(8, &present, weights, merkle);

        let hash1 = commitment.commitment_hash();
        let hash2 = commitment.commitment_hash();

        assert_eq!(hash1, hash2);
        assert!(!hash1.is_zero());
    }

    #[test]
    fn test_commitment_hash_different_for_different_data() {
        let present = [1, 3];
        let weights = vec![100, 200];

        let commitment1 = PresenceCommitment::new(8, &present, weights.clone(), Hash::ZERO);
        let commitment2 = PresenceCommitment::new(8, &[1, 4], weights, Hash::ZERO); // Different indices

        assert_ne!(commitment1.commitment_hash(), commitment2.commitment_hash());
    }

    // =========================================================================
    // PresenceCommitmentV2 Tests
    // =========================================================================

    #[test]
    fn test_v2_empty() {
        let commitment = PresenceCommitmentV2::empty();

        assert!(commitment.is_empty());
        assert_eq!(commitment.total_weight(), 0);
        assert_eq!(commitment.heartbeats_root, Hash::ZERO);
    }

    #[test]
    fn test_v2_size_is_constant() {
        let commitment1 = PresenceCommitmentV2::empty();
        let commitment2 = PresenceCommitmentV2::new(Hash::from_bytes([42u8; 32]), 1_000_000_000);

        assert_eq!(commitment1.size(), 40);
        assert_eq!(commitment2.size(), 40);
        assert_eq!(PresenceCommitmentV2::SIZE, 40);
    }

    #[test]
    fn test_v2_commitment_hash_deterministic() {
        let root = Hash::from_bytes([1u8; 32]);
        let commitment = PresenceCommitmentV2::new(root, 5_000_000_000_000);

        let hash1 = commitment.commitment_hash();
        let hash2 = commitment.commitment_hash();

        assert_eq!(hash1, hash2);
        assert!(!hash1.is_zero());
    }

    #[test]
    fn test_v2_commitment_hash_different_for_different_data() {
        let root = Hash::from_bytes([1u8; 32]);

        let commitment1 = PresenceCommitmentV2::new(root, 5_000_000_000_000);
        let commitment2 = PresenceCommitmentV2::new(root, 5_000_000_000_001); // Different weight

        assert_ne!(commitment1.commitment_hash(), commitment2.commitment_hash());

        let commitment3 = PresenceCommitmentV2::new(Hash::from_bytes([2u8; 32]), 5_000_000_000_000);
        assert_ne!(commitment1.commitment_hash(), commitment3.commitment_hash());
    }

    #[test]
    fn test_v2_serialization_roundtrip() {
        let root = Hash::from_bytes([42u8; 32]);
        let commitment = PresenceCommitmentV2::new(root, 123_456_789_000);

        let serialized = commitment.serialize();
        let deserialized = PresenceCommitmentV2::deserialize(&serialized).unwrap();

        assert_eq!(commitment, deserialized);
    }

    #[test]
    fn test_v2_default() {
        let commitment = PresenceCommitmentV2::default();
        assert!(commitment.is_empty());
        assert_eq!(commitment, PresenceCommitmentV2::empty());
    }
}
