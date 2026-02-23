//! Mempool entry type

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use crypto::Hash;
use doli_core::Transaction;

/// An entry in the mempool
#[derive(Clone, Debug)]
pub struct MempoolEntry {
    /// The transaction
    pub tx: Transaction,
    /// Transaction hash
    pub tx_hash: Hash,
    /// Transaction fee
    pub fee: u64,
    /// Fee rate (fee / size in satoshis per byte)
    pub fee_rate: u64,
    /// Transaction size in bytes
    pub size: usize,
    /// Unix timestamp when added
    pub added_time: u64,
    /// Ancestor transaction hashes (in mempool)
    pub ancestors: HashSet<Hash>,
    /// Descendant transaction hashes (in mempool)
    pub descendants: HashSet<Hash>,
    /// Total fee including ancestors
    pub ancestor_fee: u64,
    /// Total size including ancestors
    pub ancestor_size: usize,
}

impl MempoolEntry {
    /// Create a new mempool entry
    pub fn new(tx: Transaction, fee: u64) -> Self {
        let tx_hash = tx.hash();
        let size = tx.size();
        let fee_rate = if size > 0 { fee / size as u64 } else { 0 };

        let added_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Self {
            tx,
            tx_hash,
            fee,
            fee_rate,
            size,
            added_time,
            ancestors: HashSet::new(),
            descendants: HashSet::new(),
            ancestor_fee: fee,
            ancestor_size: size,
        }
    }

    /// Get effective fee rate for CPFP (Child Pays For Parent)
    ///
    /// This considers the ancestor packages: if a transaction has unconfirmed
    /// parents with low fees, the effective fee rate includes those.
    pub fn effective_fee_rate(&self) -> u64 {
        if self.ancestor_size > 0 {
            self.ancestor_fee / self.ancestor_size as u64
        } else {
            self.fee_rate
        }
    }

    /// Add an ancestor
    pub fn add_ancestor(&mut self, hash: Hash, fee: u64, size: usize) {
        if self.ancestors.insert(hash) {
            self.ancestor_fee += fee;
            self.ancestor_size += size;
        }
    }

    /// Add a descendant
    pub fn add_descendant(&mut self, hash: Hash) {
        self.descendants.insert(hash);
    }

    /// Remove an ancestor
    pub fn remove_ancestor(&mut self, hash: &Hash, fee: u64, size: usize) {
        if self.ancestors.remove(hash) {
            self.ancestor_fee = self.ancestor_fee.saturating_sub(fee);
            self.ancestor_size = self.ancestor_size.saturating_sub(size);
        }
    }

    /// Remove a descendant
    pub fn remove_descendant(&mut self, hash: &Hash) {
        self.descendants.remove(hash);
    }

    /// Get age in seconds
    pub fn age(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now.saturating_sub(self.added_time)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use doli_core::transaction::{Input, Output};
    use doli_core::{OutputType, TxType};

    /// Helper: create a minimal transfer transaction
    fn make_tx(num_inputs: usize, num_outputs: usize, amount: u64) -> Transaction {
        let inputs: Vec<Input> = (0..num_inputs)
            .map(|i| Input::new(crypto::hash::hash(&[i as u8]), 0))
            .collect();
        let outputs: Vec<Output> = (0..num_outputs)
            .map(|_| Output::normal(amount, crypto::hash::hash(b"addr")))
            .collect();
        Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs,
            outputs,
            extra_data: Vec::new(),
        }
    }

    #[test]
    fn test_entry_fee_rate_calculation() {
        let tx = make_tx(1, 1, 1000);
        let size = tx.size();
        let entry = MempoolEntry::new(tx, 500);
        assert_eq!(entry.fee, 500);
        assert_eq!(entry.fee_rate, 500 / size as u64);
        assert_eq!(entry.size, size);
    }

    #[test]
    fn test_effective_fee_rate_no_ancestors() {
        let tx = make_tx(1, 1, 1000);
        let entry = MempoolEntry::new(tx, 500);
        // Without ancestors, effective == individual
        assert_eq!(entry.effective_fee_rate(), entry.fee_rate);
    }

    #[test]
    fn test_cpfp_effective_fee_rate() {
        // Parent: low fee (10 units)
        let parent_tx = make_tx(1, 1, 1000);
        let parent_fee = 10u64;
        let parent_size = parent_tx.size();
        let parent = MempoolEntry::new(parent_tx, parent_fee);

        // Child: high fee (1000 units) — "pays for parent"
        let child_tx = make_tx(1, 1, 500);
        let child_fee = 1000u64;
        let mut child = MempoolEntry::new(child_tx, child_fee);

        child.add_ancestor(parent.tx_hash, parent_fee, parent_size);

        // Package rate = (child_fee + parent_fee) / (child_size + parent_size)
        let expected = (child_fee + parent_fee) / (child.size + parent_size) as u64;
        assert_eq!(child.effective_fee_rate(), expected);

        // CPFP: package rate must be higher than parent's individual rate
        assert!(child.effective_fee_rate() > parent.fee_rate);
    }

    #[test]
    fn test_add_ancestor_idempotent() {
        let tx = make_tx(1, 1, 100);
        let mut entry = MempoolEntry::new(tx, 50);
        let initial_fee = entry.ancestor_fee;
        let initial_size = entry.ancestor_size;

        let hash = crypto::hash::hash(b"ancestor");
        entry.add_ancestor(hash, 30, 200);
        assert_eq!(entry.ancestors.len(), 1);
        assert_eq!(entry.ancestor_fee, initial_fee + 30);
        assert_eq!(entry.ancestor_size, initial_size + 200);

        // Second insert of same hash — must be a no-op
        entry.add_ancestor(hash, 30, 200);
        assert_eq!(entry.ancestors.len(), 1);
        assert_eq!(entry.ancestor_fee, initial_fee + 30); // NOT doubled
    }

    #[test]
    fn test_remove_ancestor() {
        let tx = make_tx(1, 1, 100);
        let mut entry = MempoolEntry::new(tx, 50);
        let base_fee = entry.ancestor_fee;
        let base_size = entry.ancestor_size;

        let hash = crypto::hash::hash(b"anc");
        entry.add_ancestor(hash, 40, 300);
        entry.remove_ancestor(&hash, 40, 300);
        assert!(entry.ancestors.is_empty());
        assert_eq!(entry.ancestor_fee, base_fee);
        assert_eq!(entry.ancestor_size, base_size);
    }

    #[test]
    fn test_remove_nonexistent_ancestor_is_noop() {
        let tx = make_tx(1, 1, 100);
        let mut entry = MempoolEntry::new(tx, 50);
        let fee_before = entry.ancestor_fee;

        let hash = crypto::hash::hash(b"ghost");
        entry.remove_ancestor(&hash, 999, 999);
        assert_eq!(entry.ancestor_fee, fee_before); // Unchanged
    }

    #[test]
    fn test_descendants_add_remove() {
        let tx = make_tx(1, 1, 100);
        let mut entry = MempoolEntry::new(tx, 50);

        let d1 = crypto::hash::hash(b"d1");
        let d2 = crypto::hash::hash(b"d2");

        entry.add_descendant(d1);
        entry.add_descendant(d2);
        assert_eq!(entry.descendants.len(), 2);

        entry.remove_descendant(&d1);
        assert_eq!(entry.descendants.len(), 1);
        assert!(entry.descendants.contains(&d2));
    }

    #[test]
    fn test_zero_fee_entry() {
        let tx = make_tx(0, 1, 100);
        let entry = MempoolEntry::new(tx, 0);
        assert_eq!(entry.fee, 0);
        assert_eq!(entry.fee_rate, 0);
        assert_eq!(entry.effective_fee_rate(), 0);
    }

    #[test]
    fn test_age_is_non_negative() {
        let tx = make_tx(1, 1, 100);
        let entry = MempoolEntry::new(tx, 10);
        // Just created — age should be very small
        assert!(entry.age() < 2);
    }
}
