//! Merkle tree utilities for presence proofs
//!
//! This module provides merkle tree construction and verification for heartbeat
//! inclusion proofs. These proofs allow producers to claim epoch rewards by
//! proving their heartbeat was included in a block's presence commitment.
//!
//! ## Design
//!
//! - Uses BLAKE3 for hashing (consistent with rest of protocol)
//! - Binary merkle tree with left-right ordering based on index
//! - Empty trees have ZERO root
//! - Single-element trees have the element hash as root
//! - Proofs are compact: O(log n) hashes for n leaves

use crypto::{Hash, Hasher};
use serde::{Deserialize, Serialize};

/// Domain separator for merkle tree internal nodes
const MERKLE_INTERNAL_DOMAIN: &[u8] = b"DOLI_MERKLE_INTERNAL";

/// Domain separator for merkle tree leaf nodes
const MERKLE_LEAF_DOMAIN: &[u8] = b"DOLI_MERKLE_LEAF";

/// Merkle proof for inclusion of a leaf in a merkle tree
///
/// The proof consists of sibling hashes from the leaf up to the root.
/// Combined with the leaf hash and index, this allows verification
/// against the merkle root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MerkleProof {
    /// Sibling hashes from leaf to root
    pub path: Vec<Hash>,
    /// Index of the leaf in the tree (determines left/right at each level)
    pub index: usize,
}

impl MerkleProof {
    /// Create a new merkle proof
    pub fn new(path: Vec<Hash>, index: usize) -> Self {
        Self { path, index }
    }

    /// Verify that a leaf hash is included in the tree with the given root
    ///
    /// # Arguments
    /// * `leaf_hash` - The hash of the leaf to verify
    /// * `root` - The expected merkle root
    ///
    /// # Returns
    /// `true` if the proof is valid, `false` otherwise
    pub fn verify(&self, leaf_hash: &Hash, root: &Hash) -> bool {
        // Start with the leaf hash (already domain-separated by caller)
        let mut current = hash_leaf(leaf_hash);
        let mut idx = self.index;

        for sibling in &self.path {
            current = if idx % 2 == 0 {
                // Current is left child, sibling is right
                hash_internal(&current, sibling)
            } else {
                // Current is right child, sibling is left
                hash_internal(sibling, &current)
            };
            idx /= 2;
        }

        &current == root
    }

    /// Get the depth of the proof (number of levels)
    pub fn depth(&self) -> usize {
        self.path.len()
    }

    /// Estimate size in bytes
    pub fn size(&self) -> usize {
        // Each hash is 32 bytes + index (8 bytes) + vec overhead
        self.path.len() * 32 + 8 + 16
    }
}

/// Hash a leaf node with domain separation
fn hash_leaf(data: &Hash) -> Hash {
    let mut hasher = Hasher::new();
    hasher.update(MERKLE_LEAF_DOMAIN);
    hasher.update(data.as_bytes());
    hasher.finalize()
}

/// Hash two child nodes to create parent (internal node)
fn hash_internal(left: &Hash, right: &Hash) -> Hash {
    let mut hasher = Hasher::new();
    hasher.update(MERKLE_INTERNAL_DOMAIN);
    hasher.update(left.as_bytes());
    hasher.update(right.as_bytes());
    hasher.finalize()
}

/// Build a merkle tree from leaf hashes and return the root
///
/// # Arguments
/// * `leaves` - The leaf hashes (should already be hashed by caller)
///
/// # Returns
/// The merkle root, or Hash::ZERO if leaves is empty
pub fn compute_merkle_root(leaves: &[Hash]) -> Hash {
    if leaves.is_empty() {
        return Hash::ZERO;
    }

    if leaves.len() == 1 {
        return hash_leaf(&leaves[0]);
    }

    // Hash all leaves first
    let mut current_level: Vec<Hash> = leaves.iter().map(|h| hash_leaf(h)).collect();

    // Build tree bottom-up
    while current_level.len() > 1 {
        let mut next_level = Vec::with_capacity((current_level.len() + 1) / 2);

        for chunk in current_level.chunks(2) {
            let hash = if chunk.len() == 2 {
                hash_internal(&chunk[0], &chunk[1])
            } else {
                // Odd number of nodes: duplicate the last one
                hash_internal(&chunk[0], &chunk[0])
            };
            next_level.push(hash);
        }

        current_level = next_level;
    }

    current_level[0]
}

/// Build a merkle tree and generate proofs for all leaves
///
/// # Arguments
/// * `leaves` - The leaf hashes
///
/// # Returns
/// A tuple of (merkle_root, Vec<MerkleProof>) where proofs[i] is the proof for leaves[i]
pub fn build_merkle_tree_with_proofs(leaves: &[Hash]) -> (Hash, Vec<MerkleProof>) {
    if leaves.is_empty() {
        return (Hash::ZERO, Vec::new());
    }

    if leaves.len() == 1 {
        return (hash_leaf(&leaves[0]), vec![MerkleProof::new(vec![], 0)]);
    }

    // Build the full tree, storing all levels
    let mut levels: Vec<Vec<Hash>> = Vec::new();

    // Level 0: hashed leaves
    levels.push(leaves.iter().map(|h| hash_leaf(h)).collect());

    // Build subsequent levels
    while levels.last().map_or(0, |l| l.len()) > 1 {
        let current = levels.last().unwrap();
        let mut next_level = Vec::with_capacity((current.len() + 1) / 2);

        for chunk in current.chunks(2) {
            let hash = if chunk.len() == 2 {
                hash_internal(&chunk[0], &chunk[1])
            } else {
                hash_internal(&chunk[0], &chunk[0])
            };
            next_level.push(hash);
        }

        levels.push(next_level);
    }

    let root = levels.last().unwrap()[0];

    // Generate proofs for each leaf
    let mut proofs = Vec::with_capacity(leaves.len());

    for leaf_idx in 0..leaves.len() {
        let mut path = Vec::new();
        let mut idx = leaf_idx;

        for level in &levels[..levels.len() - 1] {
            let sibling_idx = if idx % 2 == 0 { idx + 1 } else { idx - 1 };

            // Get sibling, or duplicate if at end of odd-length level
            let sibling = if sibling_idx < level.len() {
                level[sibling_idx]
            } else {
                level[idx] // Duplicate for odd case
            };

            path.push(sibling);
            idx /= 2;
        }

        proofs.push(MerkleProof::new(path, leaf_idx));
    }

    (root, proofs)
}

/// Heartbeat-specific merkle proof (type alias with context)
pub type HeartbeatMerkleProof = MerkleProof;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_tree() {
        let root = compute_merkle_root(&[]);
        assert_eq!(root, Hash::ZERO);

        let (root2, proofs) = build_merkle_tree_with_proofs(&[]);
        assert_eq!(root2, Hash::ZERO);
        assert!(proofs.is_empty());
    }

    #[test]
    fn test_single_leaf() {
        let leaf = crypto::hash::hash(b"test leaf");
        let root = compute_merkle_root(&[leaf]);

        let (root2, proofs) = build_merkle_tree_with_proofs(&[leaf]);
        assert_eq!(root, root2);
        assert_eq!(proofs.len(), 1);

        // Verify the proof
        assert!(proofs[0].verify(&leaf, &root));
        assert_eq!(proofs[0].depth(), 0);
    }

    #[test]
    fn test_two_leaves() {
        let leaf1 = crypto::hash::hash(b"leaf 1");
        let leaf2 = crypto::hash::hash(b"leaf 2");

        let root = compute_merkle_root(&[leaf1, leaf2]);
        let (root2, proofs) = build_merkle_tree_with_proofs(&[leaf1, leaf2]);

        assert_eq!(root, root2);
        assert_eq!(proofs.len(), 2);

        // Both proofs should verify
        assert!(proofs[0].verify(&leaf1, &root));
        assert!(proofs[1].verify(&leaf2, &root));

        // Cross-verification should fail
        assert!(!proofs[0].verify(&leaf2, &root));
        assert!(!proofs[1].verify(&leaf1, &root));
    }

    #[test]
    fn test_power_of_two_leaves() {
        let leaves: Vec<Hash> = (0..8u8)
            .map(|i| crypto::hash::hash(&[i]))
            .collect();

        let root = compute_merkle_root(&leaves);
        let (root2, proofs) = build_merkle_tree_with_proofs(&leaves);

        assert_eq!(root, root2);
        assert_eq!(proofs.len(), 8);

        // All proofs should verify
        for (i, proof) in proofs.iter().enumerate() {
            assert!(
                proof.verify(&leaves[i], &root),
                "Proof {} failed to verify",
                i
            );
            assert_eq!(proof.depth(), 3); // log2(8) = 3
        }
    }

    #[test]
    fn test_non_power_of_two_leaves() {
        let leaves: Vec<Hash> = (0..5u8)
            .map(|i| crypto::hash::hash(&[i]))
            .collect();

        let root = compute_merkle_root(&leaves);
        let (root2, proofs) = build_merkle_tree_with_proofs(&leaves);

        assert_eq!(root, root2);
        assert_eq!(proofs.len(), 5);

        // All proofs should verify
        for (i, proof) in proofs.iter().enumerate() {
            assert!(
                proof.verify(&leaves[i], &root),
                "Proof {} failed to verify",
                i
            );
        }
    }

    #[test]
    fn test_large_tree() {
        // Simulate ~1000 heartbeats (realistic epoch size)
        let leaves: Vec<Hash> = (0..1000u32)
            .map(|i| crypto::hash::hash(&i.to_le_bytes()))
            .collect();

        let (root, proofs) = build_merkle_tree_with_proofs(&leaves);

        // Verify a sample of proofs
        for i in [0, 1, 499, 500, 998, 999] {
            assert!(
                proofs[i].verify(&leaves[i], &root),
                "Proof {} failed",
                i
            );
        }

        // Depth should be ceil(log2(1000)) = 10
        assert_eq!(proofs[0].depth(), 10);

        // Proof size should be ~10 * 32 = 320 bytes + overhead
        assert!(proofs[0].size() < 400);
    }

    #[test]
    fn test_tampered_proof_fails() {
        let leaves: Vec<Hash> = (0..4u8)
            .map(|i| crypto::hash::hash(&[i]))
            .collect();

        let (root, mut proofs) = build_merkle_tree_with_proofs(&leaves);

        // Tamper with a sibling hash
        if !proofs[0].path.is_empty() {
            proofs[0].path[0] = crypto::hash::hash(b"tampered");
        }

        // Should fail verification
        assert!(!proofs[0].verify(&leaves[0], &root));
    }

    #[test]
    fn test_wrong_index_fails() {
        let leaves: Vec<Hash> = (0..4u8)
            .map(|i| crypto::hash::hash(&[i]))
            .collect();

        let (root, proofs) = build_merkle_tree_with_proofs(&leaves);

        // Use proof for leaf 0 but claim it's for leaf 1
        let mut wrong_proof = proofs[0].clone();
        wrong_proof.index = 1;

        assert!(!wrong_proof.verify(&leaves[0], &root));
    }

    #[test]
    fn test_proof_serialization() {
        let leaves: Vec<Hash> = (0..4u8)
            .map(|i| crypto::hash::hash(&[i]))
            .collect();

        let (root, proofs) = build_merkle_tree_with_proofs(&leaves);

        // Serialize and deserialize
        let serialized = bincode::serialize(&proofs[0]).unwrap();
        let deserialized: MerkleProof = bincode::deserialize(&serialized).unwrap();

        // Should still verify
        assert!(deserialized.verify(&leaves[0], &root));
        assert_eq!(deserialized, proofs[0]);
    }

    #[test]
    fn test_deterministic_root() {
        let leaves: Vec<Hash> = (0..10u8)
            .map(|i| crypto::hash::hash(&[i]))
            .collect();

        // Computing root multiple times should give same result
        let root1 = compute_merkle_root(&leaves);
        let root2 = compute_merkle_root(&leaves);
        let (root3, _) = build_merkle_tree_with_proofs(&leaves);

        assert_eq!(root1, root2);
        assert_eq!(root1, root3);
    }
}
