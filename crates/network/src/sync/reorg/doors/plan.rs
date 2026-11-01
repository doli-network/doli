//! The planning door: `plan_reorg`, its support and operator variants, and the
//! shared `plan_reorg_inner`.

use std::collections::HashSet;

use crypto::Hash;
use tracing::warn;

use super::super::support::{candidate_union, union_weight, BlockSupport};
use super::super::{bump, ForkChoiceFinality, ReorgHandler, ReorgResult, MAX_REORG_DEPTH};

impl ReorgHandler {
    /// Plan a reorganization from current chain to new chain
    pub fn plan_reorg(
        &self,
        current_tip: Hash,
        new_tip: Hash,
        get_parent: impl Fn(&Hash) -> Option<Hash>,
        get_height: impl Fn(&Hash) -> Option<u64>,
        finality: ForkChoiceFinality,
    ) -> Option<ReorgResult> {
        self.plan_reorg_with_weights(
            current_tip,
            new_tip,
            get_parent,
            get_height,
            |_| None,
            finality,
        )
    }

    /// [`Self::plan_reorg`] with each block's supporters (INC-I-289 K-1: read only
    /// at/above the INC-I-288 gate, by `union_weight`).
    pub fn plan_reorg_with_support(
        &self,
        current_tip: Hash,
        new_tip: Hash,
        get_parent: impl Fn(&Hash) -> Option<Hash>,
        get_height: impl Fn(&Hash) -> Option<u64>,
        get_support: impl Fn(&Hash) -> Option<BlockSupport>,
        finality: ForkChoiceFinality,
    ) -> Option<ReorgResult> {
        self.plan_reorg_inner(
            current_tip,
            new_tip,
            get_parent,
            get_height,
            get_support,
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
        // The operator door does not weigh: every untracked block weighs 0.
        self.plan_reorg_inner(
            current_tip,
            new_tip,
            get_parent,
            get_height,
            |h| self.per_block_support(h, |_| Some(0)),
            finality,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn plan_reorg_inner(
        &self,
        current_tip: Hash,
        new_tip: Hash,
        get_parent: impl Fn(&Hash) -> Option<Hash>,
        get_height: impl Fn(&Hash) -> Option<u64>,
        get_support: impl Fn(&Hash) -> Option<BlockSupport>,
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

        // INC-I-288: at/above the gate each side is weighed from the common ancestor
        // (our side unknown: no plan, our tip is kept); below it, the base-relative
        // accumulated weights of a lossy cache.
        let (current_weight, new_weight) = if self.weighs_from_ancestor(finality.local_tip_height) {
            // INC-I-293 S-1: a new tip already on our chain is no candidate.
            if new_chain.is_empty() {
                return None;
            }
            (
                union_weight(&rollback, &get_support)?,
                candidate_union(&new_chain, &get_support),
            )
        } else {
            (self.chain_weight(&current_tip), self.chain_weight(&new_tip))
        };
        let weight_delta = new_weight as i64 - current_weight as i64;

        Some(ReorgResult {
            rollback,
            common_ancestor,
            new_blocks: new_chain,
            weight_delta,
        })
    }
}
