//! Merkle Mountain Range (MMR) for incremental state root computation.
//!
//! Instead of hashing all UTXOs every block (O(n)), the MMR maintains a rolling
//! commitment that updates in O(log n) per block:
//! - New UTXOs: append to MMR (O(log n) hashes)
//! - Spent UTXOs: mark as spent in a separate set (O(1))
//! - State root: combine MMR root + spent set hash + chain_state + producer_set
//!
//! The MMR root commits to all UTXOs ever created. Combined with the spent set,
//! it implicitly commits to the live UTXO set without iterating it.

use crypto::{hash::hash, Hash, Hasher};
use serde::{Deserialize, Serialize};

/// Compact MMR state — only stores the peaks (rightmost nodes at each level).
/// Full MMR would store all nodes; we only need peaks for root computation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompactMmr {
    /// Peaks at each level (index 0 = leaves). Only non-None entries matter.
    peaks: Vec<Option<Hash>>,
    /// Total number of leaves appended
    leaf_count: u64,
}

impl CompactMmr {
    /// Create an empty MMR
    pub fn new() -> Self {
        Self {
            peaks: Vec::new(),
            leaf_count: 0,
        }
    }

    /// Append a leaf (new UTXO hash) to the MMR. O(log n).
    pub fn append(&mut self, leaf_hash: Hash) {
        self.leaf_count += 1;
        let mut current = leaf_hash;
        let mut level = 0;

        loop {
            // Ensure peaks vector is large enough
            while self.peaks.len() <= level {
                self.peaks.push(None);
            }

            match self.peaks[level].take() {
                None => {
                    // No peak at this level — place current here
                    self.peaks[level] = Some(current);
                    break;
                }
                Some(existing) => {
                    // Merge with existing peak: hash(existing || current)
                    let mut hasher = Hasher::new();
                    hasher.update(existing.as_bytes());
                    hasher.update(current.as_bytes());
                    current = hasher.finalize();
                    level += 1;
                    // Continue to next level
                }
            }
        }
    }

    /// Compute the MMR root by hashing all peaks right-to-left. O(log n).
    pub fn root(&self) -> Hash {
        if self.peaks.is_empty() {
            return Hash::ZERO;
        }

        let mut result: Option<Hash> = None;
        for p in self.peaks.iter().rev().flatten() {
            result = Some(match result {
                None => *p,
                Some(r) => {
                    let mut hasher = Hasher::new();
                    hasher.update(p.as_bytes());
                    hasher.update(r.as_bytes());
                    hasher.finalize()
                }
            });
        }

        result.unwrap_or(Hash::ZERO)
    }

    /// Number of leaves in the MMR
    pub fn len(&self) -> u64 {
        self.leaf_count
    }

    /// Check if MMR is empty
    pub fn is_empty(&self) -> bool {
        self.leaf_count == 0
    }
}

impl Default for CompactMmr {
    fn default() -> Self {
        Self::new()
    }
}

/// Incremental state root using MMR for UTXO commitment.
///
/// Instead of serializing and hashing all UTXOs (O(n)), this maintains:
/// - MMR of all UTXO hashes ever created (append-only, O(log n) per block)
/// - Hash set of spent outpoint hashes (tracks what's been consumed)
/// - The state root = BLAKE3(mmr_root || spent_root || cs_hash || ps_hash)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IncrementalStateRoot {
    /// MMR of all UTXO hashes (append-only)
    mmr: CompactMmr,
    /// Accumulated hash of all spent outpoints (XOR-based for O(1) add/remove)
    /// XOR is used because it's commutative and cancels on double-application,
    /// making it suitable for tracking a set without ordering.
    spent_accumulator: [u8; 32],
}

impl IncrementalStateRoot {
    /// Create a new incremental state root tracker
    pub fn new() -> Self {
        Self {
            mmr: CompactMmr::new(),
            spent_accumulator: [0u8; 32],
        }
    }

    /// Record a new UTXO creation. Call once per output in apply_block.
    pub fn add_utxo(&mut self, outpoint_hash: Hash) {
        self.mmr.append(outpoint_hash);
    }

    /// Record a UTXO being spent. Call once per input in apply_block.
    pub fn spend_utxo(&mut self, outpoint_hash: Hash) {
        let bytes = outpoint_hash.as_bytes();
        for (i, b) in bytes.iter().enumerate() {
            self.spent_accumulator[i] ^= b;
        }
    }

    /// Undo a UTXO spend (for rollback). XOR is self-inverse.
    pub fn unspend_utxo(&mut self, outpoint_hash: Hash) {
        // XOR is its own inverse — same operation as spend
        self.spend_utxo(outpoint_hash);
    }

    /// Compute the incremental state root.
    /// Combines MMR root (created UTXOs) + spent accumulator + chain state + producers.
    pub fn compute_root(&self, cs_hash: Hash, ps_hash: Hash) -> Hash {
        let mmr_root = self.mmr.root();
        let spent_hash = hash(&self.spent_accumulator);

        let mut combined = Vec::with_capacity(128);
        combined.extend_from_slice(mmr_root.as_bytes());
        combined.extend_from_slice(spent_hash.as_bytes());
        combined.extend_from_slice(cs_hash.as_bytes());
        combined.extend_from_slice(ps_hash.as_bytes());

        hash(&combined)
    }

    /// Number of UTXOs ever created (not necessarily live)
    pub fn total_created(&self) -> u64 {
        self.mmr.len()
    }
}

impl Default for IncrementalStateRoot {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mmr_empty_root() {
        let mmr = CompactMmr::new();
        assert_eq!(mmr.root(), Hash::ZERO);
        assert_eq!(mmr.len(), 0);
    }

    #[test]
    fn test_mmr_single_leaf() {
        let mut mmr = CompactMmr::new();
        let leaf = hash(b"utxo1");
        mmr.append(leaf);
        assert_eq!(mmr.len(), 1);
        assert_eq!(mmr.root(), leaf); // Single leaf = root is the leaf itself
    }

    #[test]
    fn test_mmr_two_leaves() {
        let mut mmr = CompactMmr::new();
        let l1 = hash(b"utxo1");
        let l2 = hash(b"utxo2");
        mmr.append(l1);
        mmr.append(l2);
        assert_eq!(mmr.len(), 2);

        // Root should be hash(l1 || l2)
        let mut hasher = Hasher::new();
        hasher.update(l1.as_bytes());
        hasher.update(l2.as_bytes());
        let expected = hasher.finalize();
        assert_eq!(mmr.root(), expected);
    }

    #[test]
    fn test_mmr_deterministic() {
        let mut mmr1 = CompactMmr::new();
        let mut mmr2 = CompactMmr::new();

        for i in 0..100u64 {
            let leaf = hash(&i.to_le_bytes());
            mmr1.append(leaf);
            mmr2.append(leaf);
        }

        assert_eq!(mmr1.root(), mmr2.root());
        assert_eq!(mmr1.len(), 100);
    }

    #[test]
    fn test_mmr_different_order_different_root() {
        let mut mmr1 = CompactMmr::new();
        let mut mmr2 = CompactMmr::new();

        let l1 = hash(b"a");
        let l2 = hash(b"b");

        mmr1.append(l1);
        mmr1.append(l2);

        mmr2.append(l2);
        mmr2.append(l1);

        assert_ne!(mmr1.root(), mmr2.root());
    }

    #[test]
    fn test_incremental_state_root() {
        let mut isr = IncrementalStateRoot::new();

        // Add some UTXOs
        let u1 = hash(b"outpoint1");
        let u2 = hash(b"outpoint2");
        let u3 = hash(b"outpoint3");

        isr.add_utxo(u1);
        isr.add_utxo(u2);
        isr.add_utxo(u3);

        // Spend one
        isr.spend_utxo(u2);

        let cs = hash(b"chain_state");
        let ps = hash(b"producer_set");
        let root1 = isr.compute_root(cs, ps);

        // Same operations should produce same root
        let mut isr2 = IncrementalStateRoot::new();
        isr2.add_utxo(u1);
        isr2.add_utxo(u2);
        isr2.add_utxo(u3);
        isr2.spend_utxo(u2);
        let root2 = isr2.compute_root(cs, ps);

        assert_eq!(root1, root2);
    }

    #[test]
    fn test_spend_unspend_cancels() {
        let mut isr = IncrementalStateRoot::new();
        let u1 = hash(b"outpoint1");
        isr.add_utxo(u1);

        let cs = hash(b"cs");
        let ps = hash(b"ps");
        let root_before = isr.compute_root(cs, ps);

        isr.spend_utxo(u1);
        let root_spent = isr.compute_root(cs, ps);
        assert_ne!(root_before, root_spent);

        isr.unspend_utxo(u1);
        let root_after = isr.compute_root(cs, ps);
        assert_eq!(root_before, root_after);
    }

    #[test]
    fn test_mmr_large_scale() {
        let mut mmr = CompactMmr::new();
        for i in 0..10_000u64 {
            mmr.append(hash(&i.to_le_bytes()));
        }
        assert_eq!(mmr.len(), 10_000);
        // Root should be non-zero and deterministic
        let root = mmr.root();
        assert_ne!(root, Hash::ZERO);

        // Verify deterministic
        let mut mmr2 = CompactMmr::new();
        for i in 0..10_000u64 {
            mmr2.append(hash(&i.to_le_bytes()));
        }
        assert_eq!(mmr.root(), mmr2.root());
    }
}
