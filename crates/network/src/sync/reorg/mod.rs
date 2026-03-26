//! Chain reorganization handling with weight-based fork choice
//!
//! Detects and handles chain reorganizations when a peer announces
//! a block that doesn't build on our current tip.
//!
//! # Fork Choice Rule
//!
//! When comparing competing chains, we choose the one with the highest
//! **accumulated producer weight**. This prevents trivial fork attacks
//! where an attacker creates many low-weight blocks.
//!
//! Weight is based on the producer's `effective_weight` at the time of
//! block production, which factors in seniority and activity.

use std::collections::{HashMap, HashSet, VecDeque};

use crypto::Hash;
use doli_core::Block;
use tracing::{debug, info, warn};

/// Maximum depth for reorg detection (must handle network partitions up to ~2.7 hours)
const MAX_REORG_DEPTH: usize = 1000;

/// Block metadata for fork choice rule
#[derive(Clone, Debug)]
pub struct BlockWeight {
    /// Previous block hash
    pub prev_hash: Hash,
    /// Producer's effective weight at time of production
    pub producer_weight: u64,
    /// Accumulated chain weight from genesis to this block
    pub accumulated_weight: u64,
    /// Block height (for finality check).
    pub height: u64,
}

/// Reorganization result
#[derive(Clone, Debug)]
pub struct ReorgResult {
    /// Blocks to roll back (in reverse order)
    pub rollback: Vec<Hash>,
    /// Common ancestor hash
    pub common_ancestor: Hash,
    /// New blocks to apply (in order)
    pub new_blocks: Vec<Hash>,
    /// Weight difference (positive means new chain is heavier)
    pub weight_delta: i64,
}

/// Handler for chain reorganizations with weight-based fork choice
pub struct ReorgHandler {
    /// Recently seen block hashes (for quick lookup)
    recent_blocks: HashSet<Hash>,
    /// Block parent mapping (hash -> prev_hash)
    block_parents: HashMap<Hash, Hash>,
    /// Block weight information for fork choice rule
    pub(crate) block_weights: HashMap<Hash, BlockWeight>,
    /// Maximum blocks to track
    pub(crate) max_tracked: usize,
    /// LRU order for eviction
    lru_order: VecDeque<Hash>,
    /// Current chain tip accumulated weight
    current_chain_weight: u64,
    /// Height of the last finalized block (reorgs below this are rejected).
    last_finality_height: Option<u64>,
}

impl ReorgHandler {
    /// Create a new reorg handler
    pub fn new() -> Self {
        Self {
            recent_blocks: HashSet::new(),
            block_parents: HashMap::new(),
            block_weights: HashMap::new(),
            max_tracked: 10000,
            lru_order: VecDeque::new(),
            current_chain_weight: 0,
            last_finality_height: None,
        }
    }

    /// Record a block that we've applied (legacy - uses weight 1)
    pub fn record_block(&mut self, hash: Hash, prev_hash: Hash) {
        self.record_block_with_weight(hash, prev_hash, 1);
    }

    /// Record a block with its producer's effective weight
    ///
    /// This enables the weight-based fork choice rule. The accumulated
    /// weight is computed from the parent's accumulated weight plus
    /// this block's producer weight.
    pub fn record_block_with_weight(&mut self, hash: Hash, prev_hash: Hash, producer_weight: u64) {
        self.record_block_internal(hash, prev_hash, producer_weight, true);
    }

    /// Record a fork block's weight WITHOUT updating current_chain_weight.
    ///
    /// During fork recovery we populate weights for the competing chain's blocks
    /// so that plan_reorg / check_reorg_weighted can compute accumulated weight.
    /// If we updated current_chain_weight here, the subsequent comparison would
    /// compare the fork against itself (delta=0) and always reject the reorg.
    pub fn record_fork_block(&mut self, hash: Hash, prev_hash: Hash, producer_weight: u64) {
        self.record_block_internal(hash, prev_hash, producer_weight, false);
    }

    fn record_block_internal(
        &mut self,
        hash: Hash,
        prev_hash: Hash,
        producer_weight: u64,
        update_current_weight: bool,
    ) {
        // Calculate accumulated weight and height
        let (parent_accumulated, parent_height) = self
            .block_weights
            .get(&prev_hash)
            .map(|w| (w.accumulated_weight, w.height))
            .unwrap_or((0, 0));

        let accumulated_weight = parent_accumulated.saturating_add(producer_weight);
        let height = parent_height + 1;

        // Store block weight info
        self.block_weights.insert(
            hash,
            BlockWeight {
                prev_hash,
                producer_weight,
                accumulated_weight,
                height,
            },
        );

        // Only update current chain weight for blocks on OUR chain, not fork blocks
        if update_current_weight {
            self.current_chain_weight = accumulated_weight;
        }

        // Add to tracking
        self.recent_blocks.insert(hash);
        self.block_parents.insert(hash, prev_hash);
        self.lru_order.push_back(hash);

        // Evict old entries if necessary
        while self.lru_order.len() > self.max_tracked {
            if let Some(old_hash) = self.lru_order.pop_front() {
                self.recent_blocks.remove(&old_hash);
                self.block_parents.remove(&old_hash);
                self.block_weights.remove(&old_hash);
            }
        }
    }

    /// Get the accumulated weight of a chain ending at the given hash
    pub fn chain_weight(&self, hash: &Hash) -> u64 {
        self.block_weights
            .get(hash)
            .map(|w| w.accumulated_weight)
            .unwrap_or(0)
    }

    /// Get current chain tip weight
    pub fn current_weight(&self) -> u64 {
        self.current_chain_weight
    }

    /// Update the last finality height. Reorgs below this height are rejected.
    pub fn set_last_finality_height(&mut self, height: u64) {
        self.last_finality_height = Some(height);
    }

    /// Get the last finality height (for defense-in-depth checks in execute_reorg).
    pub fn last_finality_height(&self) -> Option<u64> {
        self.last_finality_height
    }

    /// Compare two chains and return which is heavier
    ///
    /// Returns:
    /// - `Ordering::Greater` if chain A is heavier
    /// - `Ordering::Less` if chain B is heavier
    /// - `Ordering::Equal` if they have equal weight
    pub fn compare_chains(&self, chain_a_tip: &Hash, chain_b_tip: &Hash) -> std::cmp::Ordering {
        let weight_a = self.chain_weight(chain_a_tip);
        let weight_b = self.chain_weight(chain_b_tip);
        weight_a.cmp(&weight_b)
    }

    /// Check if we should reorg to a new chain based on weight,
    /// with deterministic tie-breaking by block hash.
    ///
    /// When two chains have equal weight (common on devnet where all
    /// producers have weight=1), uses the lower block hash as a
    /// tie-breaker to ensure all nodes converge to the same chain.
    pub fn should_reorg_by_weight_with_tiebreak(&self, new_tip: &Hash, current_tip: &Hash) -> bool {
        let new_weight = self.chain_weight(new_tip);
        if new_weight > self.current_chain_weight {
            return true;
        }
        if new_weight == self.current_chain_weight && new_weight > 0 {
            // Deterministic tie-break: lower hash wins
            return new_tip.as_bytes() < current_tip.as_bytes();
        }
        false
    }

    /// Check if a new block triggers a reorganization (legacy - no weight check)
    ///
    /// Prefer `check_reorg_weighted` for weight-based fork choice.
    pub fn check_reorg(&self, block: &Block, current_tip: Hash) -> Option<Vec<Hash>> {
        self.check_reorg_weighted(block, current_tip, 1)
            .map(|result| result.rollback)
    }

    /// Check if a new block triggers a reorganization with weight-based fork choice
    ///
    /// This implements the "heaviest chain wins" rule. A reorg only happens if:
    /// 1. The new block doesn't build on our current tip
    /// 2. We can find a common ancestor
    /// 3. The new chain would be heavier than our current chain
    ///
    /// # Arguments
    /// * `block` - The new block being considered
    /// * `current_tip` - Our current chain tip hash
    /// * `block_producer_weight` - The producer's effective weight for this block
    ///
    /// # Returns
    /// `Some(ReorgResult)` if we should reorg, `None` otherwise
    pub fn check_reorg_weighted(
        &self,
        block: &Block,
        current_tip: Hash,
        block_producer_weight: u64,
    ) -> Option<ReorgResult> {
        let prev_hash = block.header.prev_hash;
        let block_hash = block.hash();

        // If block builds on current tip, no reorg needed
        if prev_hash == current_tip {
            return None;
        }

        // If we haven't seen the parent, we can't detect reorg
        if !self.recent_blocks.contains(&prev_hash) {
            debug!("Unknown parent {} for block {}", prev_hash, block_hash);
            return None;
        }

        // Calculate the weight of the new chain (parent weight + this block)
        let parent_weight = self.chain_weight(&prev_hash);
        let new_chain_weight = parent_weight.saturating_add(block_producer_weight);

        // Weight-based fork choice with deterministic tie-breaking.
        // Strictly lighter chains are always rejected.
        if new_chain_weight < self.current_chain_weight {
            debug!(
                "Ignoring fork: new_weight={} < current_weight={}",
                new_chain_weight, self.current_chain_weight,
            );
            return None;
        }

        // Equal weight: deterministic tie-break by block hash (lower wins).
        // INC-I-012: Without this, ALL equal-weight gossip blocks are rejected
        // and fall through to slow fork recovery (serialized, 10s/tick). On
        // young networks where all producers have weight=1, this creates fork
        // storms that accumulate faster than recovery can resolve them.
        // Deterministic hash tie-breaking ensures all nodes converge to the
        // same chain without any recovery overhead.
        if new_chain_weight == self.current_chain_weight {
            if block_hash.as_bytes() >= current_tip.as_bytes() {
                debug!(
                    "Ignoring fork: equal weight={}, block hash >= current tip (deterministic tie-break)",
                    new_chain_weight,
                );
                return None;
            }
            debug!(
                "Equal-weight tie-break: block {} < tip {} — switching chain",
                block_hash, current_tip,
            );
        }

        // Find common ancestor and build rollback list
        let mut to_rollback = Vec::new();
        let mut current = current_tip;

        for _ in 0..MAX_REORG_DEPTH {
            if current == prev_hash {
                // Found common ancestor
                if to_rollback.is_empty() {
                    return None; // No reorg needed
                }

                // Finality check: never reorg past the last finalized block
                if let Some(finality_height) = self.last_finality_height {
                    let ancestor_height = self
                        .block_weights
                        .get(&current)
                        .map(|w| w.height)
                        .unwrap_or(0);
                    if ancestor_height <= finality_height {
                        warn!(
                            "FINALITY: Rejecting reorg past finalized height {} (ancestor at {})",
                            finality_height, ancestor_height
                        );
                        return None;
                    }
                }

                let weight_delta = new_chain_weight as i64 - self.current_chain_weight as i64;

                info!(
                    "Reorg to heavier chain: rolling back {} blocks, weight_delta=+{}",
                    to_rollback.len(),
                    weight_delta
                );

                return Some(ReorgResult {
                    rollback: to_rollback,
                    common_ancestor: current,
                    new_blocks: vec![block_hash],
                    weight_delta,
                });
            }

            if let Some(&parent) = self.block_parents.get(&current) {
                to_rollback.push(current);
                current = parent;
            } else {
                // Can't trace back further
                break;
            }
        }

        warn!(
            "Could not find common ancestor for block {} (searched {} blocks)",
            block.hash(),
            MAX_REORG_DEPTH
        );

        None
    }

    /// Plan a reorganization from current chain to new chain
    pub fn plan_reorg(
        &self,
        current_tip: Hash,
        new_tip: Hash,
        get_parent: impl Fn(&Hash) -> Option<Hash>,
    ) -> Option<ReorgResult> {
        // Build ancestor chain for current tip
        let mut current_chain = Vec::new();
        let mut hash = current_tip;
        let mut current_ancestors = HashSet::new();

        for _ in 0..MAX_REORG_DEPTH {
            current_ancestors.insert(hash);
            current_chain.push(hash);

            if let Some(parent) = self
                .block_parents
                .get(&hash)
                .copied()
                .or_else(|| get_parent(&hash))
            {
                if parent.is_zero() {
                    break; // Genesis
                }
                hash = parent;
            } else {
                break;
            }
        }

        // Build ancestor chain for new tip and find common ancestor
        let mut new_chain = Vec::new();
        let mut hash = new_tip;
        let mut common_ancestor = None;

        for _ in 0..MAX_REORG_DEPTH {
            if current_ancestors.contains(&hash) {
                common_ancestor = Some(hash);
                break;
            }

            new_chain.push(hash);

            if let Some(parent) = self
                .block_parents
                .get(&hash)
                .copied()
                .or_else(|| get_parent(&hash))
            {
                if parent.is_zero() {
                    break;
                }
                hash = parent;
            } else {
                break;
            }
        }

        let common_ancestor = common_ancestor?;

        // Finality check: never reorg past the last finalized block.
        // This mirrors the check in check_reorg_weighted() — without it,
        // fork recovery falls through to plan_reorg() and bypasses finality.
        if let Some(finality_height) = self.last_finality_height {
            let ancestor_height = self
                .block_weights
                .get(&common_ancestor)
                .map(|w| w.height)
                .unwrap_or(0);
            if ancestor_height <= finality_height {
                warn!(
                    "FINALITY: plan_reorg rejecting reorg past finalized height {} (ancestor at {})",
                    finality_height, ancestor_height
                );
                return None;
            }
        }

        // Find rollback blocks (from current tip to common ancestor)
        let rollback: Vec<Hash> = current_chain
            .into_iter()
            .take_while(|h| h != &common_ancestor)
            .collect();

        // New blocks are in reverse order (need to apply from ancestor to tip)
        new_chain.reverse();

        // Calculate weight delta
        let current_weight = self.chain_weight(&current_tip);
        let new_weight = self.chain_weight(&new_tip);
        let weight_delta = new_weight as i64 - current_weight as i64;

        Some(ReorgResult {
            rollback,
            common_ancestor,
            new_blocks: new_chain,
            weight_delta,
        })
    }

    /// Clear all tracked blocks
    pub fn clear(&mut self) {
        self.recent_blocks.clear();
        self.block_parents.clear();
        self.block_weights.clear();
        self.lru_order.clear();
        self.current_chain_weight = 0;
    }

    /// Update the current chain weight (call after reorg completes)
    pub fn set_current_weight(&mut self, weight: u64) {
        self.current_chain_weight = weight;
    }

    /// Get block weight info
    pub fn get_block_weight(&self, hash: &Hash) -> Option<&BlockWeight> {
        self.block_weights.get(hash)
    }

    /// Check if we know about a block
    pub fn knows_block(&self, hash: &Hash) -> bool {
        self.recent_blocks.contains(hash)
    }

    /// Get parent of a block
    pub fn get_parent(&self, hash: &Hash) -> Option<Hash> {
        self.block_parents.get(hash).copied()
    }

    /// Get number of tracked blocks
    pub fn tracked_count(&self) -> usize {
        self.recent_blocks.len()
    }
}

impl Default for ReorgHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
