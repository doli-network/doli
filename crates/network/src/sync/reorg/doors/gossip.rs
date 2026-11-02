//! The gossip door: `check_reorg_supported` and its weight/tie verdict.

use crypto::Hash;
use doli_core::Block;
use tracing::{debug, info, warn};

use super::super::support::{candidate_union, union_weight, BlockSupport};
use super::super::{
    bump, ForkChoiceFinality, ReorgHandler, ReorgResult, WeightVerdict, MAX_REORG_DEPTH,
};

impl ReorgHandler {
    /// Check if a new block triggers a reorganization with weight-based fork choice
    ///
    /// This implements the "heaviest chain wins" rule. A reorg only happens if:
    /// 1. The new block doesn't build on our current tip
    /// 2. We can find a common ancestor
    /// 3. The new chain would be heavier than our current chain
    ///
    /// `get_height` resolves a hash to its REAL chain height (block-store backed);
    /// `finality` is the derived finality view. Both are only consulted at and above
    /// `inc_i_204_fork_choice_activation_height`. `get_support` is read only at/above
    /// the INC-I-288 gate (INC-I-289 K-1); `block` itself falls back to its producer.
    pub fn check_reorg_supported(
        &self,
        block: &Block,
        current_tip: Hash,
        block_producer_weight: u64,
        get_height: impl Fn(&Hash) -> Option<u64>,
        get_support: impl Fn(&Hash) -> Option<BlockSupport>,
        finality: ForkChoiceFinality,
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

        // INC-I-288: at/above the gate the verdict waits for the walk and weighs both
        // sides from the common ancestor (`prev_hash`); below it, before the walk.
        let from_ancestor = self.weighs_from_ancestor(finality.local_tip_height);
        if !from_ancestor
            && self.gossip_refuses(
                self.current_chain_weight,
                new_chain_weight,
                &current_tip,
                &block_hash,
            )
        {
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
                // Finality check: never reorg past the last finalized block. It runs
                // before our side is weighed, so a refused branch costs no support read.
                let ancestor_real = get_height(&current);
                if let Some(ancestor_h) = ancestor_real.filter(|h| self.fork_choice_active(*h)) {
                    bump(&self.counters.check_reorg_finality_entries);
                    bump(&self.counters.fork_choice_unified_entries);
                    let admits = self.fork_choice_admits(ancestor_h, finality, || {
                        finality.finalized_hash.is_some_and(|f| {
                            block_hash == f || self.branch_reaches(prev_hash, f, |_| None)
                        })
                    });
                    if !admits {
                        bump(&self.counters.check_reorg_finality_rejects);
                        bump(&self.counters.fork_choice_unified_rejects);
                        warn!(
                            "FINALITY: Rejecting reorg past effective finality {:?} (ancestor at {})",
                            finality.effective_finality(),
                            ancestor_h
                        );
                        return None;
                    }
                } else if let Some(finality_height) = self.last_finality_height {
                    // Pre-activation: byte-identical to the pre-M5 binary — the
                    // erasable mirror against the per-process `BlockWeight.height`.
                    bump(&self.counters.pre_activation_fork_choice);
                    bump(&self.counters.check_reorg_finality_entries);
                    let ancestor_height = self
                        .block_weights
                        .get(&current)
                        .map(|w| w.height)
                        .unwrap_or(0);
                    if ancestor_height < finality_height {
                        bump(&self.counters.check_reorg_finality_rejects);
                        warn!(
                            "FINALITY: Rejecting reorg past finalized height {} (ancestor at {})",
                            finality_height, ancestor_height
                        );
                        return None;
                    }
                }

                // INC-I-293 S-1: a candidate already on our chain never wins a tie.
                if from_ancestor && to_rollback.contains(&block_hash) {
                    debug!(
                        "Ignoring fork: candidate {} is already canonical",
                        block_hash
                    );
                    return None;
                }

                // INC-I-289: an unsupported own block keeps our tip. The candidate
                // counts when recorded or when its slot advances on its parent.
                let (ours, cand) = if from_ancestor {
                    let ours = union_weight(&to_rollback, &get_support)?;
                    let slot = block.header.slot;
                    let counts = self.block_weights.contains_key(&block_hash)
                        || self.slot_weight(slot, &prev_hash, None, 1) > 0;
                    let own = || {
                        let id = *block.header.producer.as_bytes();
                        BlockSupport::single(id, block_producer_weight)
                    };
                    let cand = if counts {
                        candidate_union(&[block_hash], |h| get_support(h).or_else(|| Some(own())))
                    } else {
                        0
                    };
                    (ours, cand)
                } else {
                    (self.current_chain_weight, new_chain_weight)
                };
                if from_ancestor && self.gossip_refuses(ours, cand, &current_tip, &block_hash) {
                    return None;
                }

                let weight_delta = cand as i64 - ours as i64;

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
            to_rollback.len()
        );

        None
    }

    /// The gossip door's weight/tie verdict: true when it keeps our tip.
    ///
    /// THE shared weight/tie rule (`fork_choice::resolve_weight`). This door's pre-M5
    /// rule already switched to the lower hash on an exact tie (INC-I-012: without
    /// it every equal-weight gossip block falls through to serialized fork recovery
    /// and young networks storm), which is exactly the unified post-activation rule
    /// — so it passes `allow_tie_switch = true` in BOTH regimes.
    fn gossip_refuses(&self, ours: u64, cand: u64, current_tip: &Hash, block_hash: &Hash) -> bool {
        match self.resolve_weight(ours, cand, current_tip, block_hash, true) {
            WeightVerdict::Lighter => {
                debug!(
                    "Ignoring fork: new_weight={} < current_weight={}",
                    cand, ours
                );
                true
            }
            WeightVerdict::TieKeep => {
                debug!(
                    "Ignoring fork: equal weight={}, block hash >= current tip (deterministic tie-break)",
                    cand,
                );
                true
            }
            WeightVerdict::TieSwitch => {
                debug!(
                    "Equal-weight tie-break: block {} < tip {} — switching chain",
                    block_hash, current_tip,
                );
                false
            }
            WeightVerdict::Heavier => false,
        }
    }
}
