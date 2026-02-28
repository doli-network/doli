//! Merkle tree implementation for transaction and state commitments.
//!
//! This module provides a binary Merkle tree implementation using BLAKE3 hashing.
//! Merkle trees enable efficient verification of data integrity and support
//! compact proofs of inclusion.
//!
//! ## Features
//!
//! - Efficient O(n) tree construction
//! - O(log n) inclusion proofs
//! - O(log n) proof verification
//! - Handles odd numbers of leaves by duplicating the last leaf
//!
//! ## Security Properties
//!
//! - Second preimage resistance from BLAKE3
//! - Domain separation between leaf and internal nodes
//! - Canonical ordering of proof elements

use serde::{Deserialize, Serialize};

use crate::hash::{Hash, Hasher};

/// Domain separator for leaf nodes.
const LEAF_PREFIX: u8 = 0x00;

/// Domain separator for internal nodes.
const INTERNAL_PREFIX: u8 = 0x01;

/// Compute the Merkle root of a list of items.
///
/// Each item is hashed to create a leaf, then leaves are combined
/// pairwise until a single root hash remains.
///
/// # Returns
///
/// Returns `None` if the input is empty.
///
/// # Example
///
/// ```rust
/// use crypto::merkle::merkle_root;
///
/// let items: Vec<&[u8]> = vec![b"tx1", b"tx2", b"tx3"];
/// let root = merkle_root(&items);
/// assert!(root.is_some());
/// ```
#[must_use]
pub fn merkle_root(items: &[&[u8]]) -> Option<Hash> {
    if items.is_empty() {
        return None;
    }

    // Hash all items to create leaves with domain separation
    let mut hashes: Vec<Hash> = items.iter().map(|item| hash_leaf(item)).collect();

    // Build tree bottom-up
    while hashes.len() > 1 {
        hashes = combine_level(&hashes);
    }

    Some(hashes[0])
}

/// Compute the Merkle root from pre-computed hashes.
///
/// Use this when you already have the hashes of the items
/// (e.g., transaction IDs).
///
/// # Returns
///
/// Returns `None` if the input is empty.
#[must_use]
pub fn merkle_root_from_hashes(hashes: &[Hash]) -> Option<Hash> {
    if hashes.is_empty() {
        return None;
    }

    // Wrap existing hashes as leaves
    let mut current: Vec<Hash> = hashes.iter().map(|h| hash_leaf(h.as_bytes())).collect();

    // Build tree bottom-up
    while current.len() > 1 {
        current = combine_level(&current);
    }

    Some(current[0])
}

/// Hash a leaf node with domain separation.
fn hash_leaf(data: &[u8]) -> Hash {
    let mut hasher = Hasher::new();
    hasher.update(&[LEAF_PREFIX]);
    hasher.update(data);
    hasher.finalize()
}

/// Hash an internal node with domain separation.
fn hash_internal(left: &Hash, right: &Hash) -> Hash {
    let mut hasher = Hasher::new();
    hasher.update(&[INTERNAL_PREFIX]);
    hasher.update(left.as_bytes());
    hasher.update(right.as_bytes());
    hasher.finalize()
}

/// Combine a level of hashes into the next level up.
fn combine_level(hashes: &[Hash]) -> Vec<Hash> {
    let mut result = Vec::with_capacity(hashes.len().div_ceil(2));

    for chunk in hashes.chunks(2) {
        let left = &chunk[0];
        // If odd number, duplicate the last element
        let right = chunk.get(1).unwrap_or(left);
        result.push(hash_internal(left, right));
    }

    result
}

/// A Merkle proof of inclusion for a single item.
///
/// The proof consists of sibling hashes along the path from
/// the leaf to the root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MerkleProof {
    /// The index of the item in the original list.
    pub index: usize,
    /// The total number of items in the tree.
    pub total: usize,
    /// Sibling hashes from leaf to root.
    /// Each element is (hash, `is_left`) where `is_left` indicates
    /// if the sibling is on the left.
    pub siblings: Vec<(Hash, bool)>,
}

impl MerkleProof {
    /// Verify this proof against a root hash and item.
    ///
    /// # Returns
    ///
    /// Returns `true` if the proof is valid.
    #[must_use]
    pub fn verify(&self, root: &Hash, item: &[u8]) -> bool {
        let leaf = hash_leaf(item);
        let computed = self.compute_root(&leaf);
        computed == *root
    }

    /// Verify this proof against a root hash and pre-computed item hash.
    ///
    /// The `item_hash` should be a raw hash (e.g., transaction ID), not
    /// already wrapped as a leaf.
    #[must_use]
    pub fn verify_hash(&self, root: &Hash, item_hash: &Hash) -> bool {
        let leaf = hash_leaf(item_hash.as_bytes());
        let computed = self.compute_root(&leaf);
        computed == *root
    }

    /// Compute the root from this proof and a leaf hash.
    fn compute_root(&self, leaf: &Hash) -> Hash {
        let mut current = *leaf;

        for (sibling, is_left) in &self.siblings {
            current = if *is_left {
                hash_internal(sibling, &current)
            } else {
                hash_internal(&current, sibling)
            };
        }

        current
    }

    /// Get the depth of this proof (number of levels in the tree).
    #[must_use]
    pub fn depth(&self) -> usize {
        self.siblings.len()
    }
}

/// A Merkle tree for generating proofs.
///
/// This structure stores the full tree for efficient proof generation.
/// For just computing roots, use `merkle_root` instead.
#[derive(Clone, Debug)]
pub struct MerkleTree {
    /// All nodes in the tree, level by level.
    /// Level 0 contains the leaves, last level contains the root.
    levels: Vec<Vec<Hash>>,
    /// Number of original items.
    count: usize,
}

impl MerkleTree {
    /// Build a new Merkle tree from items.
    ///
    /// Returns `None` if the input is empty.
    #[must_use]
    pub fn new(items: &[&[u8]]) -> Option<Self> {
        if items.is_empty() {
            return None;
        }

        let leaves: Vec<Hash> = items.iter().map(|item| hash_leaf(item)).collect();
        Some(Self::from_leaves(leaves, items.len()))
    }

    /// Build a new Merkle tree from pre-computed hashes.
    ///
    /// Returns `None` if the input is empty.
    #[must_use]
    pub fn from_hashes(hashes: &[Hash]) -> Option<Self> {
        if hashes.is_empty() {
            return None;
        }

        let leaves: Vec<Hash> = hashes.iter().map(|h| hash_leaf(h.as_bytes())).collect();
        Some(Self::from_leaves(leaves, hashes.len()))
    }

    /// Build tree from leaf hashes.
    fn from_leaves(leaves: Vec<Hash>, count: usize) -> Self {
        let mut levels = vec![leaves];

        while levels.last().is_none_or(|l| l.len() > 1) {
            let current = levels.last().expect("levels is not empty");
            levels.push(combine_level(current));
        }

        Self { levels, count }
    }

    /// Get the root hash of the tree.
    #[must_use]
    pub fn root(&self) -> Hash {
        self.levels
            .last()
            .and_then(|l| l.first().copied())
            .unwrap_or(Hash::ZERO)
    }

    /// Get the number of items in the tree.
    #[must_use]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Check if the tree is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Get the depth of the tree (number of levels).
    #[must_use]
    pub fn depth(&self) -> usize {
        self.levels.len().saturating_sub(1)
    }

    /// Generate a proof of inclusion for the item at the given index.
    ///
    /// Returns `None` if the index is out of bounds.
    #[must_use]
    pub fn proof(&self, index: usize) -> Option<MerkleProof> {
        if index >= self.count {
            return None;
        }

        let mut siblings = Vec::new();
        let mut idx = index;

        for level in &self.levels[..self.levels.len().saturating_sub(1)] {
            let sibling_idx = if idx.is_multiple_of(2) {
                idx + 1
            } else {
                idx - 1
            };
            let is_left = idx % 2 == 1;

            // Handle odd-length levels (last element pairs with itself)
            let sibling = if sibling_idx < level.len() {
                level[sibling_idx]
            } else {
                level[idx]
            };

            siblings.push((sibling, is_left));
            idx /= 2;
        }

        Some(MerkleProof {
            index,
            total: self.count,
            siblings,
        })
    }

    /// Verify that an item is at the given index.
    #[must_use]
    pub fn verify(&self, index: usize, item: &[u8]) -> bool {
        self.proof(index)
            .is_some_and(|proof| proof.verify(&self.root(), item))
    }
}

/// Compute Merkle root for a list of transactions (by their hashes).
///
/// This is a convenience function that handles the common case of
/// computing the transaction root for a block.
#[must_use]
pub fn transaction_root(tx_hashes: &[Hash]) -> Hash {
    merkle_root_from_hashes(tx_hashes).unwrap_or(Hash::ZERO)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::hash::hash;

    #[test]
    fn test_single_item() {
        let items: Vec<&[u8]> = vec![b"hello"];
        let root = merkle_root(&items).expect("should have root");

        let tree = MerkleTree::new(&items).expect("should build tree");
        assert_eq!(tree.root(), root);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree.depth(), 0);
    }

    #[test]
    fn test_two_items() {
        let items: Vec<&[u8]> = vec![b"hello", b"world"];
        let root = merkle_root(&items).expect("should have root");

        let tree = MerkleTree::new(&items).expect("should build tree");
        assert_eq!(tree.root(), root);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree.depth(), 1);

        // Verify proofs
        for (i, &item) in items.iter().enumerate() {
            let proof = tree.proof(i).expect("should have proof");
            assert!(proof.verify(&root, item));
        }
    }

    #[test]
    fn test_power_of_two() {
        let items: Vec<&[u8]> = vec![b"a", b"b", b"c", b"d"];
        let tree = MerkleTree::new(&items).expect("should build tree");
        let root = tree.root();

        assert_eq!(tree.len(), 4);
        assert_eq!(tree.depth(), 2);

        // Verify all proofs
        for (i, item) in items.iter().enumerate() {
            let proof = tree.proof(i).expect("should have proof");
            assert!(proof.verify(&root, item));
        }
    }

    #[test]
    fn test_odd_number() {
        let items: Vec<&[u8]> = vec![b"a", b"b", b"c"];
        let tree = MerkleTree::new(&items).expect("should build tree");
        let root = tree.root();

        assert_eq!(tree.len(), 3);

        // Verify all proofs
        for (i, item) in items.iter().enumerate() {
            let proof = tree.proof(i).expect("should have proof");
            assert!(proof.verify(&root, item));
        }
    }

    #[test]
    fn test_larger_tree() {
        let items: Vec<&[u8]> = vec![b"1", b"2", b"3", b"4", b"5", b"6", b"7"];
        let tree = MerkleTree::new(&items).expect("should build tree");
        let root = tree.root();

        // Verify all proofs
        for (i, item) in items.iter().enumerate() {
            let proof = tree.proof(i).expect("should have proof");
            assert!(proof.verify(&root, item), "proof failed for index {i}");
        }
    }

    #[test]
    fn test_empty_input() {
        let items: Vec<&[u8]> = vec![];
        assert!(merkle_root(&items).is_none());
        assert!(MerkleTree::new(&items).is_none());
    }

    #[test]
    fn test_proof_wrong_item() {
        let items: Vec<&[u8]> = vec![b"a", b"b", b"c", b"d"];
        let tree = MerkleTree::new(&items).expect("should build tree");
        let root = tree.root();

        let proof = tree.proof(0).expect("should have proof");
        assert!(!proof.verify(&root, b"wrong"));
    }

    #[test]
    fn test_proof_wrong_root() {
        let items: Vec<&[u8]> = vec![b"a", b"b", b"c", b"d"];
        let tree = MerkleTree::new(&items).expect("should build tree");

        let proof = tree.proof(0).expect("should have proof");
        let wrong_root = hash(b"wrong");
        assert!(!proof.verify(&wrong_root, items[0]));
    }

    #[test]
    fn test_deterministic() {
        let items: Vec<&[u8]> = vec![b"a", b"b", b"c"];
        let root1 = merkle_root(&items);
        let root2 = merkle_root(&items);
        assert_eq!(root1, root2);
    }

    #[test]
    fn test_order_matters() {
        let items1: Vec<&[u8]> = vec![b"a", b"b"];
        let items2: Vec<&[u8]> = vec![b"b", b"a"];
        let root1 = merkle_root(&items1);
        let root2 = merkle_root(&items2);
        assert_ne!(root1, root2);
    }

    #[test]
    fn test_from_hashes() {
        let hash1 = hash(b"tx1");
        let hash2 = hash(b"tx2");
        let hashes = vec![hash1, hash2];

        let root = merkle_root_from_hashes(&hashes).expect("should have root");
        let tree = MerkleTree::from_hashes(&hashes).expect("should build tree");
        assert_eq!(tree.root(), root);
    }

    #[test]
    fn test_transaction_root() {
        let hashes = vec![hash(b"tx1"), hash(b"tx2"), hash(b"tx3")];
        let root = transaction_root(&hashes);
        assert!(!root.is_zero());

        // Empty produces zero hash
        let empty_root = transaction_root(&[]);
        assert!(empty_root.is_zero());
    }

    #[test]
    fn test_proof_serialization() {
        let items: Vec<&[u8]> = vec![b"a", b"b", b"c", b"d"];
        let tree = MerkleTree::new(&items).expect("should build tree");
        let proof = tree.proof(2).expect("should have proof");

        let json = serde_json::to_string(&proof).expect("serialize");
        let recovered: MerkleProof = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(proof, recovered);
        assert!(recovered.verify(&tree.root(), items[2]));
    }

    // Property-based tests
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn prop_root_is_deterministic(items: Vec<Vec<u8>>) {
            prop_assume!(!items.is_empty());
            let refs: Vec<&[u8]> = items.iter().map(std::vec::Vec::as_slice).collect();
            let root1 = merkle_root(&refs);
            let root2 = merkle_root(&refs);
            prop_assert_eq!(root1, root2);
        }

        #[test]
        fn prop_all_proofs_verify(items: Vec<Vec<u8>>) {
            prop_assume!(!items.is_empty() && items.len() <= 100);
            let refs: Vec<&[u8]> = items.iter().map(std::vec::Vec::as_slice).collect();
            let tree = MerkleTree::new(&refs).unwrap();
            let root = tree.root();

            for (i, item) in refs.iter().enumerate() {
                let proof = tree.proof(i).unwrap();
                prop_assert!(proof.verify(&root, item));
            }
        }
    }
}
