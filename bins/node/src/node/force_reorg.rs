use std::collections::{HashMap, HashSet};
use std::time::Instant;

use network::ForceReorgPoll;

use super::*;
use crate::metrics::record_force_reorg_outcome;

/// INC-I-204 M4.1 / REQ-FORK-012 — what one evaluation of an armed
/// `forceReorgTo` directive decided. Every variant is counted; `Executed` is
/// reported only when the tip actually moved to the operator's target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForceReorgOutcome {
    Executed,
    Expired,
    Uncorroborated,
    UnknownTarget,
    PlanRefused,
    SameBranch,
    Ineligible,
    ReorgDidNotLand,
}

impl ForceReorgOutcome {
    /// Every variant, for metric zero-initialisation (INC-I-187).
    pub const ALL: [ForceReorgOutcome; 8] = [
        ForceReorgOutcome::Executed,
        ForceReorgOutcome::Expired,
        ForceReorgOutcome::Uncorroborated,
        ForceReorgOutcome::UnknownTarget,
        ForceReorgOutcome::PlanRefused,
        ForceReorgOutcome::SameBranch,
        ForceReorgOutcome::Ineligible,
        ForceReorgOutcome::ReorgDidNotLand,
    ];

    /// Prometheus label value.
    pub fn label(&self) -> &'static str {
        match self {
            ForceReorgOutcome::Executed => "executed",
            ForceReorgOutcome::Expired => "expired",
            ForceReorgOutcome::Uncorroborated => "uncorroborated",
            ForceReorgOutcome::UnknownTarget => "unknown_target",
            ForceReorgOutcome::PlanRefused => "plan_refused",
            ForceReorgOutcome::SameBranch => "same_branch",
            ForceReorgOutcome::Ineligible => "ineligible",
            ForceReorgOutcome::ReorgDidNotLand => "reorg_did_not_land",
        }
    }

    /// A decision the operator must act on. Non-terminal outcomes leave the
    /// directive armed so it can fire when the branch arrives.
    fn is_terminal(&self) -> bool {
        !matches!(self, ForceReorgOutcome::UnknownTarget)
    }
}

impl Node {
    /// Evaluate the armed `forceReorgTo` directive, if any.
    ///
    /// Returns `None` iff nothing is armed. Called from `run_periodic_tasks`.
    pub async fn try_consume_force_reorg(&mut self) -> Option<ForceReorgOutcome> {
        // Hot path: no directive, no write lock.
        self.sync_manager.read().await.force_reorg_target()?;

        let local_height = self.chain_state.read().await.best_height;

        let target = {
            let mut sync = self.sync_manager.write().await;
            match sync.poll_force_reorg(Instant::now(), local_height) {
                ForceReorgPoll::Idle => return None,
                ForceReorgPoll::Expired => {
                    record_force_reorg_outcome(ForceReorgOutcome::Expired.label());
                    return Some(ForceReorgOutcome::Expired);
                }
                ForceReorgPoll::Armed(target) => target,
            }
        };

        let outcome = self.evaluate_force_reorg(target).await;
        record_force_reorg_outcome(outcome.label());

        if outcome.is_terminal() {
            self.sync_manager.write().await.consume_force_reorg();
        }

        info!(
            "[FORCE_REORG] target={} outcome={} terminal={}",
            target,
            outcome.label(),
            outcome.is_terminal()
        );
        Some(outcome)
    }

    async fn evaluate_force_reorg(&mut self, target: Hash) -> ForceReorgOutcome {
        let current_tip = self.chain_state.read().await.best_hash;
        if target == current_tip || self.is_canonical_hash(&target) {
            return ForceReorgOutcome::SameBranch;
        }

        let Some(target_block) = self.resolve_branch_block(&target).await else {
            return ForceReorgOutcome::UnknownTarget;
        };

        // Parent links from the retained fork bodies; `plan_reorg_inner` consults
        // its own `block_parents` first and the block store through this closure.
        let cache_parents: HashMap<Hash, Hash> = {
            let cache = self.fork_block_cache.read().await;
            cache
                .iter()
                .map(|(hash, block)| (*hash, block.header.prev_hash))
                .collect()
        };

        let plan = {
            let store = &self.block_store;
            let sync = self.sync_manager.read().await;
            sync.reorg_handler().plan_reorg_operator(
                current_tip,
                target,
                |h| {
                    cache_parents
                        .get(h)
                        .copied()
                        .or_else(|| store.get_header(h).ok().flatten().map(|hd| hd.prev_hash))
                },
                |h| store.get_height_by_hash(h).ok().flatten(),
                sync.fork_choice_finality(),
            )
        };
        let Some(plan) = plan else {
            warn!(
                "[FORCE_REORG] plan refused for target={} (no common ancestor, depth bound, or unresolvable ancestor height)",
                target
            );
            return ForceReorgOutcome::PlanRefused;
        };
        if plan.new_blocks.is_empty() {
            return ForceReorgOutcome::SameBranch;
        }

        // Refuse on the first unresolvable body: a body we cannot resolve cannot be
        // corroborated or eligibility-checked, so proceeding would run both checks
        // over a partial branch.
        let mut branch_blocks = Vec::with_capacity(plan.new_blocks.len());
        for hash in &plan.new_blocks {
            let Some(block) = self.resolve_branch_block(hash).await else {
                warn!(
                    "[FORCE_REORG] target={} unresolvable branch body {} — refusing before corroboration",
                    target, hash
                );
                return ForceReorgOutcome::ReorgDidNotLand;
            };
            branch_blocks.push(block);
        }

        if !self.branch_is_corroborated(&branch_blocks).await {
            return ForceReorgOutcome::Uncorroborated;
        }

        // `execute_reorg` applies forward blocks in `ValidationMode::Light` and
        // skips `check_producer_eligibility` (block_handling.rs:1040). Mirror
        // wedge_escape.rs:79 / fork_recovery.rs:210 before handing over the plan.
        for block in &branch_blocks {
            if let Err(e) = self.check_producer_eligibility(block).await {
                warn!(
                    "[FORCE_REORG] refusing target={}: branch block {} at slot {} is ineligible: {}",
                    target,
                    block.hash(),
                    block.header.slot,
                    e
                );
                return ForceReorgOutcome::Ineligible;
            }
        }

        warn!(
            "[FORCE_REORG] executing operator reorg target={} rollback={} apply={} ancestor={}",
            target,
            plan.rollback.len(),
            plan.new_blocks.len(),
            plan.common_ancestor
        );
        if let Err(e) = self.execute_reorg(plan, target_block).await {
            error!("[FORCE_REORG] execute_reorg failed for {}: {}", target, e);
            return ForceReorgOutcome::ReorgDidNotLand;
        }

        if self.chain_state.read().await.best_hash == target {
            ForceReorgOutcome::Executed
        } else {
            warn!(
                "[FORCE_REORG] target={} did not land — execute_reorg returned Ok on a no-op path",
                target
            );
            ForceReorgOutcome::ReorgDidNotLand
        }
    }

    /// True when `hash` is on our own canonical chain (tip or ancestor).
    fn is_canonical_hash(&self, hash: &Hash) -> bool {
        let Some(height) = self.block_store.get_height_by_hash(hash).ok().flatten() else {
            return false;
        };
        self.block_store.get_hash_by_height(height).ok().flatten() == Some(*hash)
    }

    /// Resolve a branch body from the same sources `execute_reorg` uses.
    async fn resolve_branch_block(&self, hash: &Hash) -> Option<Block> {
        if let Some(block) = self.fork_block_cache.read().await.get(hash) {
            return Some(block.clone());
        }
        self.block_store.get_block(hash).ok().flatten()
    }

    /// REQ-FORK-012 §4 — the branch's own DISTINCT block producers must hold at
    /// least two thirds of the LOCAL `ProducerSet` weight.
    ///
    /// Numerator and denominator use the same unit deliberately: the same
    /// `weight(height)` function, the same `is_active()` filter, and the same
    /// height. No peer-advertised quantity enters (filter F7).
    async fn branch_is_corroborated(&self, branch_blocks: &[Block]) -> bool {
        let height = self.chain_state.read().await.best_height;
        let producers = self.producer_set.read().await;
        let total_weight = producers.total_weight(height);

        let mut seen = HashSet::new();
        let mut corroborated_weight: u64 = 0;
        for block in branch_blocks {
            if !seen.insert(block.header.producer) {
                continue;
            }
            if let Some(info) = producers.get_by_pubkey(&block.header.producer) {
                if info.is_active() {
                    corroborated_weight = corroborated_weight.saturating_add(info.weight(height));
                }
            }
        }

        let corroborated = total_weight > 0
            && corroborated_weight.saturating_mul(3) >= total_weight.saturating_mul(2);
        if !corroborated {
            warn!(
                "[FORCE_REORG] uncorroborated branch: corroborated_weight={} total_weight={} distinct_producers={} height={}",
                corroborated_weight,
                total_weight,
                seen.len(),
                height
            );
        }
        corroborated
    }
}
