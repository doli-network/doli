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
