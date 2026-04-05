//! Parallel transaction validation via dependency graph.
//!
//! Transactions that don't share inputs/outputs can be validated in parallel.
//! This module builds a dependency graph from a block's transactions and
//! identifies independent groups that can be validated concurrently.
//!
//! Signature verification (Ed25519) is the most expensive operation per tx
//! and is embarrassingly parallel. BLS aggregate verification is already O(1).

use std::collections::{HashMap, HashSet};

use crypto::Hash;

use crate::transaction::Transaction;

/// A group of transactions that can be validated in parallel.
/// Transactions within a group share no inputs or outputs with each other.
#[derive(Debug)]
pub struct IndependentGroup {
    /// Indices into the original transaction list
    pub tx_indices: Vec<usize>,
}

/// Dependency analysis result for a block's transactions.
#[derive(Debug)]
pub struct DependencyGraph {
    /// Groups of independent transactions (can be validated in parallel)
    pub groups: Vec<IndependentGroup>,
    /// Total number of transactions analyzed
    pub total_txs: usize,
}

impl DependencyGraph {
    /// Percentage of transactions that can be parallelized
    pub fn parallelism_ratio(&self) -> f64 {
        if self.total_txs == 0 {
            return 0.0;
        }
        let max_group = self
            .groups
            .iter()
            .map(|g| g.tx_indices.len())
            .max()
            .unwrap_or(0);
        max_group as f64 / self.total_txs as f64
    }
}

/// Build a dependency graph from a list of transactions.
///
/// Two transactions are dependent if they share any input outpoint (double-spend)
/// or if one transaction's output is another's input (chained spend).
///
/// Returns groups of independent transactions that can be validated in parallel.
pub fn build_dependency_graph(transactions: &[Transaction]) -> DependencyGraph {
    let total_txs = transactions.len();

    if total_txs <= 1 {
        return DependencyGraph {
            groups: vec![IndependentGroup {
                tx_indices: (0..total_txs).collect(),
            }],
            total_txs,
        };
    }

    // Map each outpoint to the transaction index that uses it (as input or output)
    let mut outpoint_to_tx: HashMap<(Hash, u32), Vec<usize>> = HashMap::new();

    for (i, tx) in transactions.iter().enumerate() {
        // Inputs: this tx consumes these outpoints
        for input in &tx.inputs {
            outpoint_to_tx
                .entry((input.prev_tx_hash, input.output_index))
                .or_default()
                .push(i);
        }
        // Outputs: this tx creates outpoints that other txs might reference
        let tx_hash = tx.hash();
        for (idx, _) in tx.outputs.iter().enumerate() {
            outpoint_to_tx
                .entry((tx_hash, idx as u32))
                .or_default()
                .push(i);
        }
    }

    // Build adjacency: two txs are dependent if they share an outpoint
    let mut adj: Vec<HashSet<usize>> = vec![HashSet::new(); total_txs];
    for indices in outpoint_to_tx.values() {
        if indices.len() > 1 {
            for &i in indices {
                for &j in indices {
                    if i != j {
                        adj[i].insert(j);
                        adj[j].insert(i);
                    }
                }
            }
        }
    }

    // Graph coloring: greedy independent set extraction
    let mut assigned = vec![false; total_txs];
    let mut groups = Vec::new();

    while assigned.iter().any(|&a| !a) {
        let mut group = Vec::new();
        let mut group_neighbors: HashSet<usize> = HashSet::new();

        for i in 0..total_txs {
            if assigned[i] || group_neighbors.contains(&i) {
                continue;
            }
            group.push(i);
            assigned[i] = true;
            group_neighbors.extend(&adj[i]);
        }

        if !group.is_empty() {
            groups.push(IndependentGroup { tx_indices: group });
        }
    }

    DependencyGraph { groups, total_txs }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::{Input, Output, Transaction, TxType};

    fn make_tx(inputs: Vec<(Hash, u32)>, output_count: usize) -> Transaction {
        Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: inputs.into_iter().map(|(h, i)| Input::new(h, i)).collect(),
            outputs: (0..output_count)
                .map(|_| Output::normal(100, Hash::ZERO))
                .collect(),
            extra_data: Vec::new(),
        }
    }

    #[test]
    fn test_empty_block() {
        let graph = build_dependency_graph(&[]);
        assert_eq!(graph.total_txs, 0);
        assert_eq!(graph.groups.len(), 1);
    }

    #[test]
    fn test_single_tx() {
        let tx = make_tx(vec![(Hash::ZERO, 0)], 1);
        let graph = build_dependency_graph(&[tx]);
        assert_eq!(graph.total_txs, 1);
        assert_eq!(graph.groups.len(), 1);
        assert_eq!(graph.groups[0].tx_indices.len(), 1);
    }

    #[test]
    fn test_independent_txs() {
        // 3 txs with completely different inputs — all independent
        let h1 = crypto::hash::hash(b"tx1");
        let h2 = crypto::hash::hash(b"tx2");
        let h3 = crypto::hash::hash(b"tx3");

        let tx1 = make_tx(vec![(h1, 0)], 1);
        let tx2 = make_tx(vec![(h2, 0)], 1);
        let tx3 = make_tx(vec![(h3, 0)], 1);

        let graph = build_dependency_graph(&[tx1, tx2, tx3]);
        assert_eq!(graph.total_txs, 3);
        // All 3 should be in the same group (independent)
        assert_eq!(graph.groups.len(), 1);
        assert_eq!(graph.groups[0].tx_indices.len(), 3);
    }

    #[test]
    fn test_dependent_txs() {
        // 2 txs spending the same input — dependent
        let shared = crypto::hash::hash(b"shared");

        let tx1 = make_tx(vec![(shared, 0)], 1);
        let tx2 = make_tx(vec![(shared, 0)], 1);

        let graph = build_dependency_graph(&[tx1, tx2]);
        assert_eq!(graph.total_txs, 2);
        // Should be in different groups (dependent)
        assert_eq!(graph.groups.len(), 2);
    }

    #[test]
    fn test_mixed_independent_dependent() {
        let h1 = crypto::hash::hash(b"input1");
        let h2 = crypto::hash::hash(b"input2");
        let shared = crypto::hash::hash(b"shared");

        // tx0 and tx1 share an input (dependent)
        // tx2 is independent
        let tx0 = make_tx(vec![(shared, 0)], 1);
        let tx1 = make_tx(vec![(shared, 0)], 1);
        let tx2 = make_tx(vec![(h1, 0)], 1);

        let graph = build_dependency_graph(&[tx0, tx1, tx2]);
        assert_eq!(graph.total_txs, 3);
        // tx0 and tx2 in group 1, tx1 in group 2
        assert!(graph.groups.len() >= 2);
        let total_in_groups: usize = graph.groups.iter().map(|g| g.tx_indices.len()).sum();
        assert_eq!(total_in_groups, 3);
    }

    #[test]
    fn test_parallelism_ratio() {
        let h1 = crypto::hash::hash(b"a");
        let h2 = crypto::hash::hash(b"b");
        let h3 = crypto::hash::hash(b"c");

        let tx1 = make_tx(vec![(h1, 0)], 1);
        let tx2 = make_tx(vec![(h2, 0)], 1);
        let tx3 = make_tx(vec![(h3, 0)], 1);

        let graph = build_dependency_graph(&[tx1, tx2, tx3]);
        assert!((graph.parallelism_ratio() - 1.0).abs() < 0.01); // 100% parallel
    }
}
