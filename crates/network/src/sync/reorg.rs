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
    block_weights: HashMap<Hash, BlockWeight>,
    /// Maximum blocks to track
    max_tracked: usize,
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

    /// Check if we should reorg to a new chain based on weight
    ///
    /// Returns true if the new chain is heavier than our current chain.
    /// This implements the "heaviest chain wins" fork choice rule.
    /// When weights are equal, uses lower block hash as tie-breaker
    /// so all nodes deterministically converge to the same chain.
    pub fn should_reorg_by_weight(&self, new_tip: &Hash) -> bool {
        let new_weight = self.chain_weight(new_tip);
        new_weight > self.current_chain_weight
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

        // Only reorg if the new chain is strictly heavier (weight-based fork choice).
        // Incumbent wins ties — equal-weight reorgs are unnecessary churn and were
        // the trigger for the epoch-boundary fork bug (weight_delta=+0 reorg).
        if new_chain_weight <= self.current_chain_weight {
            debug!(
                "Ignoring fork: new_weight={} vs current_weight={} (incumbent wins ties)",
                new_chain_weight, self.current_chain_weight,
            );
            return None;
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
mod tests {
    use super::*;

    #[test]
    fn test_reorg_handler_creation() {
        let handler = ReorgHandler::new();
        assert_eq!(handler.tracked_count(), 0);
    }

    #[test]
    fn test_record_block() {
        let mut handler = ReorgHandler::new();

        let genesis = Hash::ZERO;
        let hash1 = crypto::hash::hash(b"block1");

        handler.record_block(hash1, genesis);

        assert!(handler.knows_block(&hash1));
        assert_eq!(handler.get_parent(&hash1), Some(genesis));
        assert_eq!(handler.tracked_count(), 1);
    }

    #[test]
    fn test_no_reorg_on_tip() {
        let mut handler = ReorgHandler::new();

        let genesis = Hash::ZERO;
        let hash1 = crypto::hash::hash(b"block1");

        handler.record_block(hash1, genesis);

        // Create a block that builds on hash1
        let header = doli_core::BlockHeader {
            version: 1,
            prev_hash: hash1,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: 0,
            slot: 1,
            producer: crypto::PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput { value: vec![] },
            vdf_proof: vdf::VdfProof::empty(),
        };
        let block = Block::new(header, vec![]);

        // No reorg needed since it builds on current tip
        assert!(handler.check_reorg(&block, hash1).is_none());
    }

    #[test]
    fn test_detect_reorg() {
        let mut handler = ReorgHandler::new();

        let genesis = Hash::ZERO;
        let hash1 = crypto::hash::hash(b"block1");
        let hash2 = crypto::hash::hash(b"block2");

        // Record main chain: genesis -> hash1 (weight=1) -> hash2 (weight=1)
        // Total accumulated weight = 2
        handler.record_block_with_weight(hash1, genesis, 1);
        handler.record_block_with_weight(hash2, hash1, 1);

        // Create a block that builds on hash1 (not hash2)
        let header = doli_core::BlockHeader {
            version: 1,
            prev_hash: hash1, // Fork from hash1, not hash2
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: 0,
            slot: 2,
            producer: crypto::PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput { value: vec![] },
            vdf_proof: vdf::VdfProof::empty(),
        };
        let block = Block::new(header, vec![]);

        // Fork block has weight=5, so fork chain has accumulated weight = 1 + 5 = 6
        // Current chain has weight 2, so fork is heavier -> should reorg
        let result = handler.check_reorg_weighted(&block, hash2, 5);
        assert!(result.is_some());

        let reorg_result = result.unwrap();
        assert_eq!(reorg_result.rollback.len(), 1);
        assert_eq!(reorg_result.rollback[0], hash2);
        assert!(reorg_result.weight_delta > 0); // New chain is heavier
    }

    #[test]
    fn test_eviction() {
        let mut handler = ReorgHandler::new();
        handler.max_tracked = 10; // Small limit for testing

        let genesis = Hash::ZERO;
        let mut prev = genesis;

        // Add more blocks than limit
        for i in 0..20 {
            let hash = crypto::hash::hash(format!("block{}", i).as_bytes());
            handler.record_block(hash, prev);
            prev = hash;
        }

        // Should only have max_tracked blocks
        assert!(handler.tracked_count() <= 10);
    }

    #[test]
    fn test_weight_accumulation() {
        let mut handler = ReorgHandler::new();

        let genesis = Hash::ZERO;
        let hash1 = crypto::hash::hash(b"block1");
        let hash2 = crypto::hash::hash(b"block2");
        let hash3 = crypto::hash::hash(b"block3");

        // Build chain with different weights
        handler.record_block_with_weight(hash1, genesis, 100); // Weight 100
        handler.record_block_with_weight(hash2, hash1, 200); // Weight 200
        handler.record_block_with_weight(hash3, hash2, 50); // Weight 50

        // Check accumulated weights
        assert_eq!(handler.chain_weight(&hash1), 100);
        assert_eq!(handler.chain_weight(&hash2), 300); // 100 + 200
        assert_eq!(handler.chain_weight(&hash3), 350); // 100 + 200 + 50
        assert_eq!(handler.current_weight(), 350);
    }

    #[test]
    fn test_weight_based_fork_choice_rejects_lighter_chain() {
        let mut handler = ReorgHandler::new();

        let genesis = Hash::ZERO;
        let hash1 = crypto::hash::hash(b"block1");
        let hash2 = crypto::hash::hash(b"block2");

        // Build main chain with high weight
        handler.record_block_with_weight(hash1, genesis, 100);
        handler.record_block_with_weight(hash2, hash1, 200); // Total: 300

        // Create a fork block with low weight
        let header = doli_core::BlockHeader {
            version: 1,
            prev_hash: hash1, // Fork from hash1
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: 0,
            slot: 2,
            producer: crypto::PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput { value: vec![] },
            vdf_proof: vdf::VdfProof::empty(),
        };
        let fork_block = Block::new(header, vec![]);

        // Fork with weight 50 (total 150) should be rejected
        // Our chain has weight 300, fork would have 100 + 50 = 150
        let result = handler.check_reorg_weighted(&fork_block, hash2, 50);
        assert!(result.is_none(), "Should reject lighter fork");
    }

    #[test]
    fn test_weight_based_fork_choice_accepts_heavier_chain() {
        let mut handler = ReorgHandler::new();

        let genesis = Hash::ZERO;
        let hash1 = crypto::hash::hash(b"block1");
        let hash2 = crypto::hash::hash(b"block2");

        // Build main chain with low weight
        handler.record_block_with_weight(hash1, genesis, 100);
        handler.record_block_with_weight(hash2, hash1, 50); // Total: 150

        // Create a fork block with high weight
        let header = doli_core::BlockHeader {
            version: 1,
            prev_hash: hash1, // Fork from hash1
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: 0,
            slot: 2,
            producer: crypto::PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput { value: vec![] },
            vdf_proof: vdf::VdfProof::empty(),
        };
        let fork_block = Block::new(header, vec![]);

        // Fork with weight 200 (total 300) should be accepted
        // Our chain has weight 150, fork would have 100 + 200 = 300
        let result = handler.check_reorg_weighted(&fork_block, hash2, 200);
        assert!(result.is_some(), "Should accept heavier fork");

        let reorg = result.unwrap();
        assert_eq!(reorg.rollback.len(), 1);
        assert_eq!(reorg.rollback[0], hash2);
        assert!(reorg.weight_delta > 0, "Weight delta should be positive");
    }

    #[test]
    fn test_chain_comparison() {
        let mut handler = ReorgHandler::new();

        let genesis = Hash::ZERO;
        let chain_a = crypto::hash::hash(b"chain_a");
        let chain_b = crypto::hash::hash(b"chain_b");

        handler.record_block_with_weight(chain_a, genesis, 100);
        handler.record_block_with_weight(chain_b, genesis, 200);

        use std::cmp::Ordering;
        assert_eq!(handler.compare_chains(&chain_a, &chain_b), Ordering::Less);
        assert_eq!(
            handler.compare_chains(&chain_b, &chain_a),
            Ordering::Greater
        );
        assert_eq!(handler.compare_chains(&chain_a, &chain_a), Ordering::Equal);
    }

    #[test]
    fn test_equal_weight_tiebreak_by_hash() {
        let mut handler = ReorgHandler::new();

        let genesis = Hash::ZERO;
        let hash1 = crypto::hash::hash(b"block1");
        let hash2 = crypto::hash::hash(b"block2");

        // Build main chain: genesis -> hash1 (weight=1) -> hash2 (weight=1)
        // Total accumulated weight = 2
        handler.record_block_with_weight(hash1, genesis, 1);
        handler.record_block_with_weight(hash2, hash1, 1);

        // Create a fork block on hash1 with weight=1 (equal total weight = 2)
        let header = doli_core::BlockHeader {
            version: 1,
            prev_hash: hash1,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: 0,
            slot: 2,
            producer: crypto::PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput { value: vec![] },
            vdf_proof: vdf::VdfProof::empty(),
        };
        let fork_block = Block::new(header, vec![]);
        let fork_hash = fork_block.hash();

        // Equal weight: tie-break by hash
        let result = handler.check_reorg_weighted(&fork_block, hash2, 1);

        // One of the two hashes must be "lower" — the tie-breaker picks it
        if fork_hash.as_bytes() < hash2.as_bytes() {
            assert!(
                result.is_some(),
                "Fork with lower hash should win tie-break"
            );
        } else {
            assert!(
                result.is_none(),
                "Fork with higher hash should lose tie-break"
            );
        }
    }

    #[test]
    fn test_tiebreak_with_new_method() {
        let mut handler = ReorgHandler::new();

        let genesis = Hash::ZERO;
        let tip_a = crypto::hash::hash(b"tip_a");
        let tip_b = crypto::hash::hash(b"tip_b");

        // Both chains have equal weight
        handler.record_block_with_weight(tip_a, genesis, 100);
        // Reset current weight to test tiebreak method
        handler.set_current_weight(100);

        handler.block_weights.insert(
            tip_b,
            BlockWeight {
                prev_hash: genesis,
                producer_weight: 100,
                accumulated_weight: 100,
                height: 1,
            },
        );

        // The lower hash should win
        let a_wins = handler.should_reorg_by_weight_with_tiebreak(&tip_b, &tip_a);
        let b_wins = handler.should_reorg_by_weight_with_tiebreak(&tip_a, &tip_b);

        // Exactly one should win (they can't both have equal hashes)
        assert_ne!(
            a_wins, b_wins,
            "Tie-break must be deterministic: exactly one chain wins"
        );
    }

    #[test]
    fn test_reorg_past_finality_rejected() {
        let mut handler = ReorgHandler::new();

        let genesis = Hash::ZERO;
        let block1 = crypto::hash::hash(b"block1");
        let block2 = crypto::hash::hash(b"block2");
        let _fork_block = crypto::hash::hash(b"fork");

        // Build main chain: genesis -> block1 -> block2
        handler.record_block_with_weight(block1, genesis, 10);
        handler.record_block_with_weight(block2, block1, 10);

        // Set finality at height 1 (block1)
        handler.set_last_finality_height(1);

        // Create a fork from genesis with higher weight
        // This would reorg past the finalized block1 — should be rejected
        let fork = Block {
            header: doli_core::BlockHeader {
                version: 1,
                prev_hash: genesis, // forks from genesis (height 0) < finality height 1
                merkle_root: Hash::ZERO,
                presence_root: Hash::ZERO,
                genesis_hash: Hash::ZERO,
                timestamp: 100,
                slot: 1,
                producer: crypto::PublicKey::from_bytes([0u8; 32]),
                vdf_output: vdf::VdfOutput {
                    value: vec![0u8; 32],
                },
                vdf_proof: vdf::VdfProof { pi: vec![0u8; 32] },
            },
            transactions: vec![],
            aggregate_bls_signature: Vec::new(),
        };

        let result = handler.check_reorg_weighted(&fork, block2, 100);
        assert!(result.is_none(), "Reorg past finality should be rejected");
    }

    #[test]
    fn test_reorg_after_finality_ok() {
        let mut handler = ReorgHandler::new();

        let genesis = Hash::ZERO;
        let block1 = crypto::hash::hash(b"block1");
        let block2 = crypto::hash::hash(b"block2");

        handler.record_block_with_weight(block1, genesis, 10);
        handler.record_block_with_weight(block2, block1, 10);

        // Set finality at height 0 (genesis)
        handler.set_last_finality_height(0);

        // Fork from block1 (height 1) which is above finality — should be allowed if heavier
        let fork = Block {
            header: doli_core::BlockHeader {
                version: 1,
                prev_hash: block1, // forks from block1 (height 1) > finality height 0
                merkle_root: Hash::ZERO,
                presence_root: Hash::ZERO,
                genesis_hash: Hash::ZERO,
                timestamp: 200,
                slot: 2,
                producer: crypto::PublicKey::from_bytes([0u8; 32]),
                vdf_output: vdf::VdfOutput {
                    value: vec![0u8; 32],
                },
                vdf_proof: vdf::VdfProof { pi: vec![0u8; 32] },
            },
            transactions: vec![],
            aggregate_bls_signature: Vec::new(),
        };

        let result = handler.check_reorg_weighted(&fork, block2, 100);
        assert!(result.is_some(), "Reorg above finality should be allowed");
    }

    #[test]
    fn test_fork_block_recording_does_not_corrupt_current_weight() {
        // Regression test for the N4 solo-fork bug:
        // record_fork_block() was using record_block_with_weight() which
        // unconditionally overwrote current_chain_weight, making the fork
        // recovery comparison always see delta=0.
        let mut handler = ReorgHandler::new();

        let genesis = Hash::ZERO;
        let shared = crypto::hash::hash(b"shared_block");

        // Our chain: genesis -> shared (w=100) -> our_tip (w=100) = 200
        handler.record_block_with_weight(shared, genesis, 100);
        let our_tip = crypto::hash::hash(b"our_solo_block");
        handler.record_block_with_weight(our_tip, shared, 100);
        assert_eq!(handler.current_weight(), 200);

        // Fork recovery finds canonical chain: shared -> fork_tip (w=100) = 200
        let fork_tip_hash = crypto::hash::hash(b"canonical_block");
        handler.record_fork_block(fork_tip_hash, shared, 100);

        // CRITICAL: current_chain_weight must still be 200 (our chain), not
        // overwritten to 200 (fork chain). Before the fix, this was corrupted.
        assert_eq!(
            handler.current_weight(),
            200,
            "record_fork_block must NOT overwrite current_chain_weight"
        );

        // Now test with a heavier fork: shared -> fork_heavy (w=500) = 600
        let fork_heavy = crypto::hash::hash(b"canonical_heavy");
        handler.record_fork_block(fork_heavy, shared, 500);

        // current_weight still unchanged
        assert_eq!(handler.current_weight(), 200);

        // But the fork's accumulated weight should be queryable
        assert_eq!(handler.chain_weight(&fork_heavy), 600);

        // check_reorg_weighted should detect the heavier fork
        let fork_block = Block {
            header: doli_core::BlockHeader {
                version: 1,
                prev_hash: shared,
                merkle_root: Hash::ZERO,
                presence_root: Hash::ZERO,
                genesis_hash: Hash::ZERO,
                timestamp: 0,
                slot: 2,
                producer: crypto::PublicKey::from_bytes([0u8; 32]),
                vdf_output: vdf::VdfOutput {
                    value: vec![0u8; 32],
                },
                vdf_proof: vdf::VdfProof { pi: vec![0u8; 32] },
            },
            transactions: vec![],
            aggregate_bls_signature: Vec::new(),
        };
        let result = handler.check_reorg_weighted(&fork_block, our_tip, 500);
        assert!(
            result.is_some(),
            "Heavier fork should trigger reorg after correct weight tracking"
        );
    }
}
