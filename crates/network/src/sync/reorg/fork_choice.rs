//! INC-I-204 M5 — the ONE branch-choice authority, behind one activation height.
//!
//! Gated on `NetworkParams::inc_i_204_fork_choice_activation_height`. Below the
//! gate every caller gets the pre-M5 answer bit for bit (mainnet and testnet sit at
//! `u64::MAX` for the whole of M5). At and above it: strictly heavier wins, an exact
//! tie goes to the lower block hash, and the finality guard reads the common
//! ancestor's REAL chain height (never `BlockWeight.height` — INV-SYNC-012), keeps
//! its STRICT `<` (INV-SYNC-008 / LB-2), and requires the candidate branch to
//! CONTAIN the finalized hash. Both gate inputs are chain-derived heights: the local
//! tip for the weight rule, the ancestor's real height for the finality rule.

use crypto::Hash;

use super::{bump, ReorgHandler, MAX_REORG_DEPTH};

/// The finality view a caller hands to the fork-choice authority. Built by
/// `SyncManager::fork_choice_finality` from `FinalityTracker.last_finalized` plus
/// the local tip; replaces `ReorgHandler.last_finality_height` as the AUTHORITY
/// above the gate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ForkChoiceFinality {
    /// Height of the last finalized checkpoint, if anything has finalized.
    pub finalized_height: Option<u64>,
    /// Hash of the last finalized checkpoint — the ancestry guard's input.
    pub finalized_hash: Option<Hash>,
    /// The node's own applied tip height. Also the weight rule's gate input.
    pub local_tip_height: u64,
}

impl ForkChoiceFinality {
    /// The finality height that actually binds: `min(checkpoint, local tip)`.
    ///
    /// Clamping releases a node that rolled back below finality without the
    /// mirror's erasure, so it never returns to `None` once something finalized.
    pub fn effective_finality(&self) -> Option<u64> {
        self.finalized_height.map(|f| f.min(self.local_tip_height))
    }
}

/// The single branch-weight verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeightVerdict {
    /// The candidate branch is strictly heavier — switch.
    Heavier,
    /// Exact tie, candidate hash lower — switch (post-activation only).
    TieSwitch,
    /// Exact tie, our tip keeps it.
    TieKeep,
    /// The candidate branch is strictly lighter — keep our tip.
    Lighter,
}

impl ReorgHandler {
    /// THE branch-weight rule. Every door — gossip, wedge escape, fork recovery —
    /// reaches its verdict through this function.
    pub fn weigh_branches(
        &self,
        our_tip: &Hash,
        cand_tip: &Hash,
        finality: ForkChoiceFinality,
    ) -> WeightVerdict {
        let our_w = self.chain_weight(our_tip);
        let cand_w = self.chain_weight(cand_tip);

        let post = self.fork_choice_active(finality.local_tip_height);
        if post {
            bump(&self.counters.fork_choice_unified_entries);
        } else {
            bump(&self.counters.pre_activation_fork_choice);
        }
        self.resolve_weight(our_w, cand_w, our_tip, cand_tip, post)
    }

    /// The shared weight/tie comparison. `allow_tie_switch` is the ONLY thing that
    /// differs between callers: `weigh_branches` passes the activation gate, the
    /// gossip door passes `true` because its pre-M5 rule already switched on a tie.
    pub(super) fn resolve_weight(
        &self,
        our_w: u64,
        cand_w: u64,
        our_tip: &Hash,
        cand_tip: &Hash,
        allow_tie_switch: bool,
    ) -> WeightVerdict {
        if cand_w > our_w {
            return WeightVerdict::Heavier;
        }
        if cand_w < our_w {
            return WeightVerdict::Lighter;
        }
        if allow_tie_switch && cand_tip.as_bytes() < our_tip.as_bytes() {
            WeightVerdict::TieSwitch
        } else {
            WeightVerdict::TieKeep
        }
    }

    /// `true` once the M5 authority is live at `chain_height`.
    pub(super) fn fork_choice_active(&self, chain_height: u64) -> bool {
        chain_height >= self.inc_i_204_fork_choice_activation_height
    }

    /// The post-activation finality rule, shared by both reorg doors.
    /// `branch_contains_finalized` is lazy: the walk is paid for only after the
    /// numeric pre-filter passes.
    pub(super) fn fork_choice_admits(
        &self,
        ancestor_real_height: u64,
        finality: ForkChoiceFinality,
        branch_contains_finalized: impl FnOnce() -> bool,
    ) -> bool {
        let Some(effective) = finality.effective_finality() else {
            return true;
        };
        if ancestor_real_height < effective {
            return false;
        }
        if finality.finalized_hash.is_none() {
            return true;
        }
        branch_contains_finalized()
    }

    /// Walk `start` and its ancestors looking for `target`, using the in-memory
    /// parent map first and the caller's closure as the fallback.
    pub(super) fn branch_reaches(
        &self,
        start: Hash,
        target: Hash,
        get_parent: impl Fn(&Hash) -> Option<Hash>,
    ) -> bool {
        let mut hash = start;
        for _ in 0..MAX_REORG_DEPTH {
            if hash == target {
                return true;
            }
            match self
                .block_parents
                .get(&hash)
                .copied()
                .or_else(|| get_parent(&hash))
            {
                Some(parent) => hash = parent,
                None => return false,
            }
        }
        false
    }

    /// Compare two chains and return which is heavier.
    pub fn compare_chains(&self, chain_a_tip: &Hash, chain_b_tip: &Hash) -> std::cmp::Ordering {
        let weight_a = self.chain_weight(chain_a_tip);
        let weight_b = self.chain_weight(chain_b_tip);
        weight_a.cmp(&weight_b)
    }

    /// Legacy third tie rule. ZERO production callers (measured run 542); deleted
    /// in M6. Its `new_weight > 0` guard has no counterpart in [`Self::weigh_branches`],
    /// so it stays byte-identical rather than being re-pointed at the new authority.
    pub fn should_reorg_by_weight_with_tiebreak(&self, new_tip: &Hash, current_tip: &Hash) -> bool {
        let new_weight = self.chain_weight(new_tip);
        if new_weight > self.current_weight() {
            return true;
        }
        if new_weight == self.current_weight() && new_weight > 0 {
            return new_tip.as_bytes() < current_tip.as_bytes();
        }
        false
    }
}
