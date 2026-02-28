//! EpochSnapshot — lightweight summary of producer set state at epoch boundary.
//!
//! Replaces full GSet exchange during initial sync. Nodes exchange snapshots
//! to quickly bootstrap, then use GSet for incremental updates within epochs.

use crypto::{Hash, PublicKey};
use serde::{Deserialize, Serialize};

use crate::consensus::{NUM_REGIONS, SLOTS_PER_EPOCH};

/// Compact snapshot of the producer set at an epoch boundary.
///
/// Built at every epoch boundary from the ProducerGSet and ProducerSet.
/// Used for fast initial sync instead of exchanging the full GSet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochSnapshot {
    /// Epoch number this snapshot covers.
    pub epoch: u64,
    /// Merkle root of the sorted producer set (deterministic).
    pub merkle_root: Hash,
    /// Tier 1 validator set (top N by effective weight).
    pub tier1_set: Vec<PublicKey>,
    /// Tier 2 regional assignment: NUM_REGIONS entries, each a list of producers.
    pub tier2_regions: Vec<Vec<PublicKey>>,
    /// Total number of active producers.
    pub total_producers: u64,
    /// Total effective weight across all active producers.
    pub total_weight: u64,
}

impl EpochSnapshot {
    /// Create a new snapshot from producer set data.
    ///
    /// # Arguments
    /// - `epoch`: The epoch this snapshot represents
    /// - `tier1_set`: Computed Tier 1 validator set
    /// - `all_producers`: All active producer public keys (sorted)
    /// - `total_weight`: Sum of all effective weights
    pub fn new(
        epoch: u64,
        tier1_set: Vec<PublicKey>,
        all_producers: &[PublicKey],
        total_weight: u64,
    ) -> Self {
        let merkle_root = Self::compute_merkle_root(all_producers);
        let tier2_regions = Self::assign_regions(all_producers, &tier1_set);

        Self {
            epoch,
            merkle_root,
            tier1_set,
            tier2_regions,
            total_producers: all_producers.len() as u64,
            total_weight,
        }
    }

    /// Compute a deterministic Merkle root from sorted producer keys.
    pub fn compute_merkle_root(producers: &[PublicKey]) -> Hash {
        if producers.is_empty() {
            return Hash::ZERO;
        }
        // Hash each producer key, then build merkle tree
        let leaves: Vec<Hash> = producers
            .iter()
            .map(|pk| crypto::hash::hash(pk.as_bytes()))
            .collect();

        crypto::merkle::merkle_root_from_hashes(&leaves).unwrap_or(Hash::ZERO)
    }

    /// Assign non-Tier1 producers to regions using deterministic hashing.
    fn assign_regions(all_producers: &[PublicKey], tier1_set: &[PublicKey]) -> Vec<Vec<PublicKey>> {
        let mut regions: Vec<Vec<PublicKey>> = (0..NUM_REGIONS).map(|_| Vec::new()).collect();

        for pk in all_producers {
            // Skip Tier 1 producers (they don't have regional assignment)
            if tier1_set.contains(pk) {
                continue;
            }
            let region = crate::consensus::producer_region(pk);
            if let Some(bucket) = regions.get_mut(region as usize) {
                bucket.push(*pk);
            }
        }

        regions
    }

    /// Compute the epoch number from a block height.
    pub fn epoch_from_height(height: u64) -> u64 {
        height / SLOTS_PER_EPOCH as u64
    }

    /// Check if a height is an epoch boundary (first block of new epoch).
    pub fn is_epoch_boundary(height: u64) -> bool {
        height > 0 && height.is_multiple_of(SLOTS_PER_EPOCH as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_keys(n: usize) -> Vec<PublicKey> {
        (0..n)
            .map(|_| *crypto::KeyPair::generate().public_key())
            .collect()
    }

    #[test]
    fn test_snapshot_merkle_root_deterministic() {
        let keys = make_keys(10);
        let root1 = EpochSnapshot::compute_merkle_root(&keys);
        let root2 = EpochSnapshot::compute_merkle_root(&keys);
        assert_eq!(root1, root2, "Same producers should give same root");
    }

    #[test]
    fn test_snapshot_merkle_root_empty() {
        let root = EpochSnapshot::compute_merkle_root(&[]);
        assert_eq!(root, Hash::ZERO);
    }

    #[test]
    fn test_snapshot_merkle_root_order_matters() {
        let mut keys = make_keys(5);
        let root1 = EpochSnapshot::compute_merkle_root(&keys);
        keys.reverse();
        let root2 = EpochSnapshot::compute_merkle_root(&keys);
        assert_ne!(root1, root2, "Different order should give different root");
    }

    #[test]
    fn test_snapshot_region_assignment() {
        let all = make_keys(20);
        let tier1 = all[0..3].to_vec(); // First 3 are Tier 1

        let snapshot = EpochSnapshot::new(1, tier1.clone(), &all, 100);

        assert_eq!(snapshot.tier2_regions.len(), NUM_REGIONS as usize);
        // Tier 1 producers should NOT appear in any region
        for region in &snapshot.tier2_regions {
            for pk in region {
                assert!(
                    !tier1.contains(pk),
                    "Tier1 producer should not be in regions"
                );
            }
        }
        // Total producers in all regions should be all_producers - tier1
        let total_in_regions: usize = snapshot.tier2_regions.iter().map(|r| r.len()).sum();
        assert_eq!(total_in_regions, 17); // 20 - 3
    }

    #[test]
    fn test_epoch_boundary() {
        assert!(!EpochSnapshot::is_epoch_boundary(0));
        assert!(!EpochSnapshot::is_epoch_boundary(1));
        assert!(EpochSnapshot::is_epoch_boundary(360));
        assert!(!EpochSnapshot::is_epoch_boundary(361));
        assert!(EpochSnapshot::is_epoch_boundary(720));
    }

    #[test]
    fn test_epoch_from_height() {
        assert_eq!(EpochSnapshot::epoch_from_height(0), 0);
        assert_eq!(EpochSnapshot::epoch_from_height(359), 0);
        assert_eq!(EpochSnapshot::epoch_from_height(360), 1);
        assert_eq!(EpochSnapshot::epoch_from_height(719), 1);
        assert_eq!(EpochSnapshot::epoch_from_height(720), 2);
    }
}
