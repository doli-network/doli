//! A Normal reorg that refuses a fork block, by its schedule check or by an
//! apply failure, returns the node to the tip it had before the reorg, and the
//! refused block stays refused.

use super::reorg_refusals::Refusal;
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReorgMode {
    Normal,
    /// Re-apply the node's own rolled-back blocks; never refuses, never restores.
    Restore,
}

/// Why a Normal-mode reorg stopped.
pub(super) enum ReorgStop {
    /// A new block was refused before; nothing was rolled back.
    Remembered { branch_tip: Hash },
    /// A new block's body does not match its header; nothing was rolled back.
    JunkBody,
    /// `refusal` was refused after `applied` fork blocks landed.
    Refused {
        refusal: Refusal,
        applied: Vec<Hash>,
        branch_tip: Hash,
    },
}

impl ReorgStop {
    /// `hashes` are the new blocks in apply order; `i` is the refused one.
    pub(super) fn unscheduled(hashes: &[Hash], i: usize) -> Self {
        Self::refused(Refusal::Unscheduled(hashes[i]), hashes, i)
    }

    /// Block `i` failed apply_block; `key` is its content key.
    pub(super) fn apply_failed(key: Hash, hashes: &[Hash], i: usize) -> Self {
        Self::refused(Refusal::ApplyFailed(key), hashes, i)
    }

    fn refused(refusal: Refusal, hashes: &[Hash], i: usize) -> Self {
        ReorgStop::Refused {
            refusal,
            applied: hashes[..i].to_vec(),
            branch_tip: hashes[hashes.len() - 1],
        }
    }
}

impl Node {
    /// Execute a chain reorganization. Not atomic: when the schedule check or
    /// apply_block refuses a fork block the node is restored to its prior tip
    /// and the branch is rejected.
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
            None | Some(ReorgStop::JunkBody) => return Ok(()),
            Some(ReorgStop::Remembered { branch_tip }) => branch_tip,
            Some(ReorgStop::Refused {
                refusal,
                applied,
                branch_tip,
            }) => {
                self.refused_reorg_blocks.insert(refusal);
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
}
