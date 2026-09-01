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
use tracing::warn;

#[cfg(test)]
use doli_core::Block;

mod doors;
mod fork_choice;
mod observe;
pub use fork_choice::{ForkChoiceFinality, WeightVerdict};
pub use observe::ReorgObservations;
use observe::{bump, ReorgCounters};

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
    /// INC-I-147 D6 activation height (`NetworkParams::inc_i_147_activation_height`).
    ///
    /// At or above this chain height, `plan_reorg` compares the common ancestor's
    /// REAL chain height against `last_finality_height` instead of the per-process
    /// `BlockWeight.height` counter. Below it, behaviour is byte-identical to the
    /// pre-fix ordering established by `e25a9a97`.
    ///
    /// Defaults to `0` (always-on) so that unit tests and devnet exercise the
    /// corrected path; the node overrides it from `NetworkParams` via `SyncConfig`.
    inc_i_147_activation_height: u64,
    /// INC-I-204 M5 fork-choice activation height
    /// (`NetworkParams::inc_i_204_fork_choice_activation_height`).
    ///
    /// At or above it ONE authority decides every branch comparison. Below it every
    /// door keeps its pre-M5 path verbatim. `new()` defaults it to `u64::MAX`
    /// (dormant) so pre-existing tests keep exercising the pre-activation branch.
    inc_i_204_fork_choice_activation_height: u64,
    /// INC-I-204 M0 observation counters. Written alongside decisions, never read
    /// by one; `bins/node` scrapes them into Prometheus.
    counters: ReorgCounters,
}

impl ReorgHandler {
    /// Create a new reorg handler
    pub fn new() -> Self {
        // Seed genesis (Hash::ZERO) so forks sharing only genesis
        // as common ancestor can be detected by check_reorg_weighted
        let mut recent_blocks = HashSet::new();
        recent_blocks.insert(Hash::ZERO);
        let mut lru_order = VecDeque::new();
        lru_order.push_back(Hash::ZERO);

        Self {
            recent_blocks,
            block_parents: HashMap::new(),
            block_weights: HashMap::new(),
            max_tracked: 10000,
            lru_order,
            current_chain_weight: 0,
            last_finality_height: None,
            inc_i_147_activation_height: 0,
            inc_i_204_fork_choice_activation_height: u64::MAX,
            counters: ReorgCounters::default(),
        }
    }

    /// Create a reorg handler with an explicit INC-I-147 activation height.
    ///
    /// The node passes `NetworkParams::inc_i_147_activation_height` through
    /// `SyncConfig`. `new()` keeps `0` (always-on) so existing unit tests are
    /// unaffected — they root their chains at `Hash::ZERO`, where the per-process
    /// offset is `0` and both branches agree anyway.
    pub fn with_activation_height(inc_i_147_activation_height: u64) -> Self {
        Self::with_activation_heights(inc_i_147_activation_height, u64::MAX)
    }

    /// Create a reorg handler with both gate heights (INC-I-204 M5).
    ///
    /// The node passes `NetworkParams::inc_i_147_activation_height` and
    /// `NetworkParams::inc_i_204_fork_choice_activation_height` through `SyncConfig`.
    pub fn with_activation_heights(
        inc_i_147_activation_height: u64,
        inc_i_204_fork_choice_activation_height: u64,
    ) -> Self {
        Self {
            inc_i_147_activation_height,
            inc_i_204_fork_choice_activation_height,
            ..Self::new()
        }
    }

    /// The M5 fork-choice activation height this handler was built with.
    pub fn fork_choice_activation_height(&self) -> u64 {
        self.inc_i_204_fork_choice_activation_height
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
        self.record_block_internal(hash, prev_hash, producer_weight, true, None);
    }

    /// Record an applied block using its REAL chain height (INC-I-147 D6).
    ///
    /// Prefer this over [`record_block_with_weight`] wherever the caller knows the
    /// height. Deriving height as `parent_height + 1` from an empty `block_weights`
    /// map yields a per-process counter (`real_height - init_height`) that is not
    /// comparable with any chain-global height. Storing the real height fixes the unit
    /// at the source, for every consumer of `BlockWeight.height` — including
    /// `check_reorg_weighted`, which reads the same field with no fallback.
    ///
    /// Gated by `inc_i_147_activation_height`: below it, the legacy derived height is
    /// stored so behaviour is byte-identical to the pre-fix binary.
    pub fn record_block_with_height(
        &mut self,
        hash: Hash,
        prev_hash: Hash,
        producer_weight: u64,
        real_height: u64,
    ) {
        let post_activation = real_height >= self.inc_i_147_activation_height;
        if !post_activation {
            bump(&self.counters.pre_activation_record_height);
        }
        let height = post_activation.then_some(real_height);
        self.record_block_internal(hash, prev_hash, producer_weight, true, height);
    }

    /// Record a fork block's weight WITHOUT updating current_chain_weight.
    ///
    /// During fork recovery we populate weights for the competing chain's blocks
    /// so that plan_reorg / check_reorg_weighted can compute accumulated weight.
    /// If we updated current_chain_weight here, the subsequent comparison would
    /// compare the fork against itself (delta=0) and always reject the reorg.
    pub fn record_fork_block(&mut self, hash: Hash, prev_hash: Hash, producer_weight: u64) {
        self.record_block_internal(hash, prev_hash, producer_weight, false, None);
    }

    /// Record a fork block using its REAL chain height (INC-I-204 M5, Contradiction 2).
    ///
    /// Gated by `inc_i_204_fork_choice_activation_height` exactly as
    /// [`Self::record_block_with_height`] is gated by INC-I-147: the gate input is the
    /// REAL height argument, and below the gate the legacy derived height is stored so
    /// the write is indistinguishable from [`Self::record_fork_block`].
    pub fn record_fork_block_with_height(
        &mut self,
        hash: Hash,
        prev_hash: Hash,
        producer_weight: u64,
        real_height: u64,
    ) {
        let post_activation = self.fork_choice_active(real_height);
        if post_activation {
            bump(&self.counters.record_fork_block_real_height);
        } else {
            bump(&self.counters.pre_activation_fork_choice);
        }
        let height = post_activation.then_some(real_height);
        self.record_block_internal(hash, prev_hash, producer_weight, false, height);
    }

    /// `real_height`: the block's true chain height when the caller knows it
    /// (INC-I-147 D6). When `None`, height is derived as `parent_height + 1`, which is
    /// only correct if the parent is itself recorded — at process start `block_weights`
    /// is EMPTY, so the first block silently becomes height 1 and every later block
    /// counts from there, producing `H_syn = H_real - init_height`.
    fn record_block_internal(
        &mut self,
        hash: Hash,
        prev_hash: Hash,
        producer_weight: u64,
        update_current_weight: bool,
        real_height: Option<u64>,
    ) {
        // Calculate accumulated weight and height
        let (parent_accumulated, parent_height) = self
            .block_weights
            .get(&prev_hash)
            .map(|w| (w.accumulated_weight, w.height))
            .unwrap_or((0, 0));

        let accumulated_weight = parent_accumulated.saturating_add(producer_weight);
        // INC-I-147 D6: prefer the caller's real chain height; fall back to deriving
        // from the parent (correct only when the parent is already recorded).
        let height = real_height.unwrap_or(parent_height + 1);

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
    ///
    /// INC-I-204 M5 / INV-FINALITY-001 clause (1): at and above the fork-choice gate
    /// this takes `max()`, so a checkpoint that moved DOWN (reachable whenever a
    /// reorg, snap or backfill re-applies blocks below the tip) can no longer lower
    /// the guard. Below the gate it stays the pre-M5 bare assignment.
    pub fn set_last_finality_height(&mut self, height: u64) {
        if self.fork_choice_active(height) {
            let next = self
                .last_finality_height
                .map_or(height, |cur| cur.max(height));
            self.last_finality_height = Some(next);
            return;
        }
        self.last_finality_height = Some(height);
    }

    /// Get the last finality height (for defense-in-depth checks in execute_reorg).
    pub fn last_finality_height(&self) -> Option<u64> {
        self.last_finality_height
    }

    /// If the local tip has dropped below the cached finality height, clear the
    /// finality marker so sync can recover. Backstop for INV-SYNC-001 violations
    /// (INC-I-081 Bug 4). The new_tip_height argument is the height of the
    /// post-rollback local tip.
    ///
    /// INC-I-204 M5 brief S12: above the fork-choice gate the mirror is no longer an
    /// authority and this erasure route is a no-op — the successor
    /// `ForkChoiceFinality::effective_finality` performs the same release by clamping
    /// to the local tip, with no erasable state. The field and this method survive as
    /// BYTES so the dormant window stays byte-identical; M6 deletes them.
    pub fn clear_finality_if_below_tip(&mut self, new_tip_height: u64) {
        if self.fork_choice_active(new_tip_height) {
            return;
        }
        if let Some(finality) = self.last_finality_height {
            if new_tip_height < finality {
                warn!(
                    "[FINALITY_GUARD] clearing stale last_finality_height={} after rollback to tip_h={} (INV-SYNC-004 backstop)",
                    finality, new_tip_height
                );
                self.last_finality_height = None;
            }
        }
    }

    /// Clear all tracked blocks
    pub fn clear(&mut self) {
        self.recent_blocks.clear();
        self.block_parents.clear();
        self.block_weights.clear();
        self.lru_order.clear();
        // Re-seed genesis after clear
        self.recent_blocks.insert(Hash::ZERO);
        self.lru_order.push_back(Hash::ZERO);
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

    /// INC-I-204 M0: snapshot of this handler's observation counters.
    pub fn observations(&self) -> ReorgObservations {
        self.counters.snapshot()
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

#[cfg(test)]
mod tests_inc_i_147;

#[cfg(test)]
mod tests_m5_common;

#[cfg(test)]
mod tests_m5_fork_choice;

#[cfg(test)]
mod tests_m5_finality_authority;

#[cfg(test)]
mod tests_m5_dormant_window;
