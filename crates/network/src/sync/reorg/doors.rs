//! INC-I-204 M5 — the two reorg doors: the gossip door and the planning door.
//!
//! Split out of `reorg/mod.rs` for the module-size budget. Both doors reach their
//! weight verdict through `fork_choice::resolve_weight` and their finality verdict
//! through `ReorgHandler::fork_choice_admits`, gated on
//! `inc_i_204_fork_choice_activation_height`. Below the gate each keeps its own
//! pre-M5 code path verbatim.

use std::collections::HashSet;

use crypto::Hash;
use doli_core::Block;
use tracing::{debug, info, warn};

use super::{bump, ForkChoiceFinality, ReorgHandler, ReorgResult, WeightVerdict, MAX_REORG_DEPTH};

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
    /// `inc_i_204_fork_choice_activation_height`.
    pub fn check_reorg_weighted(
        &self,
        block: &Block,
        current_tip: Hash,
        block_producer_weight: u64,
        get_height: impl Fn(&Hash) -> Option<u64>,
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

        // THE shared weight/tie rule (`fork_choice::resolve_weight`). This door's
        // pre-M5 rule already switched to the lower hash on an exact tie (INC-I-012:
        // without it every equal-weight gossip block falls through to serialized
        // fork recovery and young networks storm), which is exactly the unified
        // post-activation rule — so it passes `allow_tie_switch = true` in BOTH
        // regimes and is byte-identical below the gate.
        match self.resolve_weight(
            self.current_chain_weight,
            new_chain_weight,
            &current_tip,
            &block_hash,
            true,
        ) {
            WeightVerdict::Lighter => {
                debug!(
                    "Ignoring fork: new_weight={} < current_weight={}",
                    new_chain_weight, self.current_chain_weight,
                );
                return None;
            }
            WeightVerdict::TieKeep => {
                debug!(
                    "Ignoring fork: equal weight={}, block hash >= current tip (deterministic tie-break)",
                    new_chain_weight,
                );
                return None;
            }
            WeightVerdict::TieSwitch => {
                debug!(
                    "Equal-weight tie-break: block {} < tip {} — switching chain",
                    block_hash, current_tip,
                );
            }
            WeightVerdict::Heavier => {}
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

                // Finality check: never reorg past the last finalized block.
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
        get_height: impl Fn(&Hash) -> Option<u64>,
        finality: ForkChoiceFinality,
    ) -> Option<ReorgResult> {
        self.plan_reorg_inner(
            current_tip,
            new_tip,
            get_parent,
            get_height,
            finality,
            false,
        )
    }

    /// INC-I-204 M4.1 / REQ-FORK-012: the audited operator door.
    ///
    /// Identical to `plan_reorg` except that the finality MARKER on
    /// `last_finality_height` no longer refuses the plan. Every other veto still
    /// binds: `MAX_REORG_DEPTH`, no-common-ancestor, and the unresolvable-ancestor
    /// refusal. Planning stays pure — the marker is read, never written.
    /// Automatic callers must never reach this variant.
    pub fn plan_reorg_operator(
        &self,
        current_tip: Hash,
        new_tip: Hash,
        get_parent: impl Fn(&Hash) -> Option<Hash>,
        get_height: impl Fn(&Hash) -> Option<u64>,
        finality: ForkChoiceFinality,
    ) -> Option<ReorgResult> {
        self.plan_reorg_inner(current_tip, new_tip, get_parent, get_height, finality, true)
    }

    fn plan_reorg_inner(
        &self,
        current_tip: Hash,
        new_tip: Hash,
        get_parent: impl Fn(&Hash) -> Option<Hash>,
        get_height: impl Fn(&Hash) -> Option<u64>,
        finality: ForkChoiceFinality,
        finality_override: bool,
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
                    // Include genesis in ancestor set so forks sharing
                    // only genesis as common ancestor can be resolved
                    current_ancestors.insert(parent);
                    break;
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
                    // Check if genesis is the common ancestor
                    if current_ancestors.contains(&parent) {
                        common_ancestor = Some(parent);
                    }
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
        let m5_real_height = get_height(&common_ancestor);
        if let Some(ancestor_h) = m5_real_height.filter(|h| self.fork_choice_active(*h)) {
            bump(&self.counters.plan_reorg_finality_entries);
            bump(&self.counters.fork_choice_unified_entries);
            let admits = self.fork_choice_admits(ancestor_h, finality, || {
                finality.finalized_hash.is_some_and(|f| {
                    new_chain.contains(&f) || self.branch_reaches(common_ancestor, f, &get_parent)
                })
            });
            if !admits {
                if !finality_override {
                    bump(&self.counters.plan_reorg_finality_rejects);
                    bump(&self.counters.fork_choice_unified_rejects);
                    warn!(
                        "FINALITY: plan_reorg rejecting reorg past effective finality {:?} (ancestor at {})",
                        finality.effective_finality(),
                        ancestor_h
                    );
                    return None;
                }
                warn!(
                    "[FORCE_REORG] operator plan crosses the unified finality rule: effective={:?} ancestor={} new_tip={}",
                    finality.effective_finality(),
                    ancestor_h,
                    new_tip
                );
            }
        } else if let Some(finality_height) = self.last_finality_height {
            bump(&self.counters.pre_activation_fork_choice);
            bump(&self.counters.plan_reorg_finality_entries);
            // INV-SYNC-002 (INC-I-081 Bug 2): block_weights is LRU-bounded and is
            // pruned during rollback. Falling back to height=0 silently rejects
            // every reorg whose finality is non-zero. Consult the caller-provided
            // height lookup (typically backed by block_store.get_height_by_hash)
            // before declining.
            // INC-I-147 D6: `BlockWeight.height` is a PER-PROCESS counter, not a chain
            // height. `block_weights` is empty at process start, so the first block
            // recorded always gets height 1 and every later block counts up from there:
            //
            //     H_syn = H_real - I,  I = (height of first recorded block) - 1
            //
            // `finality_height` below is a REAL chain height (set from `check_finality`).
            // Comparing the two mixes units, and on any restarted or snap-synced node
            // (I > finality lag) the guard can never pass — no reorg is ever approved and
            // the node is permanently wedged on whatever fork it holds. MEASURED
            // 2026-07-31: the same block at real height 57067 was recorded as 267 by the
            // seed (init 56800) and 25897 by n7 (init 31170), 5.6 ms apart.
            //
            // `e25a9a97` guarded only the `None` arm of this lookup; the `Some` arm
            // shadows the correct `get_height` result with the wrong-unit value.
            let real_height = get_height(&common_ancestor);
            let synthetic_height = self.block_weights.get(&common_ancestor).map(|w| w.height);

            // Gate on the ancestor's REAL height — a genuine chain height. Deliberately
            // NOT `finality_height`: the guard only runs when finality is `Some`, but a
            // node that has never finalized would then never evaluate the gate at all.
            let post_activation =
                real_height.is_some_and(|h| h >= self.inc_i_147_activation_height);
            if !post_activation {
                bump(&self.counters.pre_activation_plan_reorg_finality);
            }

            let ancestor_height = if post_activation {
                // Post-activation: the real chain height is authoritative.
                real_height.expect("post_activation implies real_height.is_some()")
            } else {
                // Pre-activation: byte-identical to the `e25a9a97` ordering.
                match synthetic_height {
                    Some(h) => h,
                    None => match real_height {
                        Some(h) => h,
                        None => {
                            warn!(
                                "[ANCESTOR_UNKNOWN] plan_reorg cannot resolve height for common_ancestor={} (absent from block_weights and get_height) — declining reorg",
                                common_ancestor
                            );
                            return None;
                        }
                    },
                }
            };
            if ancestor_height < finality_height {
                if !finality_override {
                    bump(&self.counters.plan_reorg_finality_rejects);
                    warn!(
                        "FINALITY: plan_reorg rejecting reorg past finalized height {} (ancestor at {})",
                        finality_height, ancestor_height
                    );
                    return None;
                }
                // INC-I-204 M4.1 / REQ-FORK-012: the operator door. The marker is
                // crossed for this one planned reorg and left intact.
                warn!(
                    "[FORCE_REORG] operator plan crosses the finality MARKER: finalized={} ancestor={} new_tip={}",
                    finality_height, ancestor_height, new_tip
                );
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
}
