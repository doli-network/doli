//! A reorg that the in-reorg schedule check refuses returns the node to the
//! tip it had before the reorg, and the refused block stays refused.

use std::collections::VecDeque;

use super::*;

/// Refused fork blocks remembered (FIFO).
const REFUSED_REORG_CAP: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReorgMode {
    Normal,
    /// Re-apply the node's own rolled-back blocks; never refuses, never restores.
    Restore,
}

/// Why a Normal-mode reorg stopped on a refused block.
pub(super) enum ReorgStop {
    /// A new block was refused before; nothing was rolled back.
    Remembered { branch_tip: Hash },
    /// The schedule check refused `refused` after `applied` fork blocks landed.
    Unscheduled {
        refused: Hash,
        applied: Vec<Hash>,
        branch_tip: Hash,
    },
}

impl ReorgStop {
    /// `hashes` are the new blocks in apply order; `i` is the refused one.
    pub(super) fn unscheduled(hashes: &[Hash], i: usize) -> Self {
        ReorgStop::Unscheduled {
            refused: hashes[i],
            applied: hashes[..i].to_vec(),
            branch_tip: hashes[hashes.len() - 1],
        }
    }
}

/// Hashes of fork blocks the in-reorg schedule check refused.
#[derive(Debug, Default)]
pub struct RefusedReorgBlocks(VecDeque<Hash>);

impl RefusedReorgBlocks {
    fn insert(&mut self, hash: Hash) {
        if self.0.contains(&hash) {
            return;
        }
        self.0.push_back(hash);
        if self.0.len() > REFUSED_REORG_CAP {
            self.0.pop_front();
        }
    }

    fn contains(&self, hash: &Hash) -> bool {
        self.0.contains(hash)
    }
}

impl Node {
    /// Execute a chain reorganization. Not atomic: when the schedule check
    /// refuses a fork block the node is restored to its prior tip and the
    /// branch is rejected; an apply failure leaves the applied prefix.
    pub async fn execute_reorg(
        &mut self,
        reorg_result: ReorgResult,
        triggering_block: Block,
    ) -> Result<()> {
        let tip_before = self.chain_state.read().await.best_hash;
        let old_blocks = reorg_result.rollback.clone();
        let ancestor = reorg_result.common_ancestor;
        let stop = self
            .run_reorg(reorg_result, Some(triggering_block), ReorgMode::Normal)
            .await?;
        let branch_tip = match stop {
            None => return Ok(()),
            Some(ReorgStop::Remembered { branch_tip }) => branch_tip,
            Some(ReorgStop::Unscheduled {
                refused,
                applied,
                branch_tip,
            }) => {
                self.refused_reorg_blocks.insert(refused);
                let plan = ReorgResult {
                    rollback: applied,
                    common_ancestor: ancestor,
                    new_blocks: old_blocks,
                    weight_delta: 0,
                };
                self.restore_tip(plan, tip_before).await;
                branch_tip
            }
        };
        let local_tip = self.chain_state.read().await.best_hash;
        self.sync_manager
            .write()
            .await
            .mark_fork_branch_rejected(branch_tip, local_tip);
        Ok(())
    }

    /// Rolls back the applied fork blocks and re-applies the node's own.
    async fn restore_tip(&mut self, plan: ReorgResult, tip_before: Hash) {
        let result = Box::pin(self.run_reorg(plan, None, ReorgMode::Restore)).await;
        let tip = self.chain_state.read().await.best_hash;
        match result {
            Ok(_) if tip == tip_before => {
                info!("[REORG_RESTORE] restored tip {:.16}", tip);
            }
            other => error!(
                "[REORG_RESTORE] failed: tip {:.16}, not the prior tip {:.16} ({:?}) — header-first sync recovers",
                tip,
                tip_before,
                other.err()
            ),
        }
    }

    /// A Normal reorg whose new blocks include a refused block stops here.
    pub(super) fn remembered_refusal(
        &self,
        new_blocks: &[Block],
        mode: ReorgMode,
    ) -> Option<ReorgStop> {
        if mode != ReorgMode::Normal {
            return None;
        }
        let hit = new_blocks
            .iter()
            .find(|b| self.refused_reorg_blocks.contains(&b.hash()))?;
        let branch_tip = new_blocks.last()?.hash();
        warn!(
            "[REORG] Branch tip {:.16} holds fork block {:.16} that was refused at its parent — reorg refused, nothing changed",
            branch_tip,
            hit.hash()
        );
        Some(ReorgStop::Remembered { branch_tip })
    }
}
