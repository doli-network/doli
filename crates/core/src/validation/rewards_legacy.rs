use crate::block::Block;
use crate::consensus::{ConsensusParams, RewardMode};
use crate::transaction::Transaction;
use crate::types::{Amount, BlockHeight};
use crypto::PublicKey;

use super::{EpochBlockSource, ValidationContext, ValidationError};

/// Validate the coinbase transaction
///
/// Note: This function is no longer called from validate_block since the automatic
/// reward distribution system was deprecated. It's kept for backward compatibility.
#[allow(dead_code)]
pub(super) fn validate_coinbase(
    tx: &Transaction,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    // Must have no inputs
    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidCoinbase(
            "coinbase must have no inputs".to_string(),
        ));
    }

    // Must have exactly one output
    if tx.outputs.len() != 1 {
        return Err(ValidationError::InvalidCoinbase(
            "coinbase must have exactly one output".to_string(),
        ));
    }

    // Amount must match block reward (+ fees, but we skip fee check here)
    let expected_reward = ctx.params.block_reward(ctx.current_height);
    if tx.outputs[0].amount < expected_reward {
        return Err(ValidationError::InvalidCoinbase(format!(
            "coinbase amount {} less than reward {}",
            tx.outputs[0].amount, expected_reward
        )));
    }

    Ok(())
}

/// Calculate expected epoch rewards for the given blocks.
///
/// # Deprecated
///
/// This function is deprecated as part of Milestone 10 (Remove Old Reward System).
/// The automatic epoch reward distribution system has been replaced with weighted
/// presence rewards using ClaimEpochReward transactions. Use the new
/// `crate::rewards::WeightedRewardCalculator` instead.
///
/// This is the deterministic reward calculation algorithm used by both
/// block producers and validators to ensure consistency. Given the same
/// blocks, any node will calculate exactly the same rewards.
///
/// # Arguments
/// * `epoch` - The epoch number for reward transactions
/// * `blocks` - All blocks produced in the epoch (from BlockStore)
/// * `current_height` - Current block height (for reward halving)
/// * `params` - Consensus parameters
///
/// # Returns
/// Vector of (producer_pubkey, reward_amount) pairs, sorted by pubkey for determinism.
/// Empty vector if no blocks were produced in the epoch.
#[deprecated(
    since = "0.1.0",
    note = "Use crate::rewards::WeightedRewardCalculator instead"
)]
pub fn calculate_expected_epoch_rewards(
    _epoch: u64,
    blocks: &[Block],
    current_height: BlockHeight,
    params: &ConsensusParams,
) -> Vec<(PublicKey, u64)> {
    use std::collections::HashMap;

    // Count blocks per producer (by public key)
    let mut producer_blocks: HashMap<PublicKey, u64> = HashMap::new();
    for block in blocks {
        // Skip null producer (genesis block has zero pubkey)
        if block.header.producer.as_bytes().iter().all(|&b| b == 0) {
            continue;
        }
        *producer_blocks.entry(block.header.producer).or_insert(0) += 1;
    }

    let total_blocks = producer_blocks.values().sum::<u64>();
    if total_blocks == 0 {
        return Vec::new();
    }

    // Calculate block reward (with halving based on current height)
    let block_reward = params.block_reward(current_height);

    // Total pool = blocks_produced × block_reward
    let total_pool = total_blocks.saturating_mul(block_reward);

    // Sort producers by public key for deterministic ordering
    let mut sorted_producers: Vec<_> = producer_blocks.into_iter().collect();
    sorted_producers.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    // Calculate proportional rewards
    let mut rewards = Vec::new();
    let mut distributed = 0u64;

    for (i, (producer_pubkey, blocks_produced)) in sorted_producers.iter().enumerate() {
        let is_last = i == sorted_producers.len() - 1;

        // Calculate proportional share
        let share = if is_last {
            // Last producer gets remainder (rounding dust)
            total_pool.saturating_sub(distributed)
        } else {
            // Proportional: (blocks_produced / total_blocks) * total_pool
            // Use u128 to avoid overflow in intermediate calculation
            let share =
                ((*blocks_produced as u128) * (total_pool as u128)) / (total_blocks as u128);
            share as u64
        };

        if share > 0 {
            rewards.push((*producer_pubkey, share));
            distributed += share;
        }
    }

    rewards
}

/// Determine which epoch (if any) needs reward distribution at the current slot.
///
/// # Deprecated
///
/// This function is deprecated as part of Milestone 10 (Remove Old Reward System).
/// The automatic epoch reward distribution system has been replaced with weighted
/// presence rewards using ClaimEpochReward transactions.
///
/// Returns `Some(epoch)` if rewards for that epoch should be included in a block
/// at the given slot, or `None` if no rewards are due.
///
/// This handles catch-up scenarios where multiple epochs may need rewards.
/// Only one epoch is rewarded per block (oldest first).
///
/// **Empty Epoch Handling**: Epochs with no blocks are skipped. This handles
/// the case where nodes start running after the genesis timestamp, leaving
/// earlier epochs empty. The function scans forward from `last_rewarded + 1`
/// to find the first epoch that actually contains blocks.
#[deprecated(
    since = "0.1.0",
    note = "Automatic epoch rewards replaced by ClaimEpochReward transactions"
)]
pub fn epoch_needing_rewards<S: EpochBlockSource>(
    current_slot: u32,
    params: &ConsensusParams,
    source: &S,
) -> Result<Option<u64>, String> {
    let slots_per_epoch = params.slots_per_reward_epoch as u64;

    // Skip slot 0 (genesis, can't distribute before epoch 0 ends)
    if current_slot == 0 {
        return Ok(None);
    }

    let current_epoch = current_slot as u64 / slots_per_epoch;

    // Get the last epoch that received rewards from BlockStore
    let last_rewarded = source.last_rewarded_epoch()?;

    // Only finished epochs (< current_epoch) are eligible for rewards
    if current_epoch <= last_rewarded {
        return Ok(None);
    }

    // Find the first epoch after last_rewarded that has blocks
    // This skips empty epochs (e.g., when chain starts after genesis timestamp)
    for epoch in (last_rewarded + 1)..current_epoch {
        let start_slot = if epoch == 0 {
            1u32 // Skip genesis slot
        } else {
            (epoch * slots_per_epoch) as u32
        };
        let end_slot = ((epoch + 1) * slots_per_epoch) as u32;

        if source.has_any_block_in_slot_range(start_slot, end_slot)? {
            return Ok(Some(epoch));
        }
    }

    // All epochs between last_rewarded and current are empty - no rewards needed
    Ok(None)
}

/// Validate block reward transactions for epoch distribution mode.
///
/// # Deprecated
///
/// This function is deprecated as part of Milestone 10 (Remove Old Reward System).
/// The automatic epoch reward distribution system has been replaced with weighted
/// presence rewards using ClaimEpochReward transactions.
///
/// In EpochPool mode:
/// - At epoch boundary: expect EpochReward txs, NO coinbase
/// - Non-boundary: NO rewards at all (pool accumulates)
///
/// In DirectCoinbase mode: this function does nothing (handled by validate_coinbase).
#[allow(dead_code)]
pub(super) fn validate_block_rewards(
    block: &Block,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    // Only applies to EpochPool mode
    if ctx.params.reward_mode != RewardMode::EpochPool {
        return Ok(());
    }

    let slot = block.header.slot;
    let is_boundary = ctx.params.is_reward_epoch_boundary(slot);

    if is_boundary {
        // Epoch boundary: expect EpochReward transactions, NO coinbase
        let epoch_rewards: Vec<_> = block
            .transactions
            .iter()
            .filter(|tx| tx.is_epoch_reward())
            .collect();

        // Verify no coinbase in epoch boundary blocks
        if block.transactions.iter().any(|tx| tx.is_coinbase()) {
            return Err(ValidationError::InvalidBlock(
                "epoch boundary block cannot have coinbase".to_string(),
            ));
        }

        // Calculate total distributed across all epoch reward outputs
        let total_distributed: Amount = epoch_rewards
            .iter()
            .flat_map(|tx| tx.outputs.iter())
            .map(|o| o.amount)
            .sum();

        // Expected pool for this epoch
        let expected_pool = ctx.params.total_epoch_reward(ctx.current_height);

        // With attestation qualification, total_distributed ≤ pool:
        // - All qualified: distributed == pool (remainder dust to first qualifier)
        // - Some disqualified: distributed < pool (non-qualifier share burned)
        // - None qualified: distributed == 0 (entire pool burned)
        if total_distributed > expected_pool {
            return Err(ValidationError::InvalidBlock(format!(
                "epoch rewards {} > expected pool {}",
                total_distributed, expected_pool
            )));
        }

        // Validate each epoch reward transaction
        for tx in &epoch_rewards {
            // Verify epoch number matches
            if let Some(data) = tx.epoch_reward_data() {
                let expected_epoch = ctx.params.slot_to_reward_epoch(slot);
                if data.epoch != expected_epoch as u64 {
                    return Err(ValidationError::InvalidEpochReward(format!(
                        "epoch mismatch: {} vs expected {}",
                        data.epoch, expected_epoch
                    )));
                }
            }
        }
    } else {
        // Non-boundary: NO coinbase, NO epoch rewards (pool accumulates)
        if block.transactions.iter().any(|tx| tx.is_coinbase()) {
            return Err(ValidationError::InvalidBlock(
                "coinbase transactions not allowed — rewards distributed at epoch boundary"
                    .to_string(),
            ));
        }
        if block.transactions.iter().any(|tx| tx.is_epoch_reward()) {
            return Err(ValidationError::InvalidBlock(
                "epoch rewards only distributed at epoch boundary".to_string(),
            ));
        }
    }

    Ok(())
}

/// Validate block rewards with exact distribution verification.
///
/// # Deprecated
///
/// This function is deprecated as part of Milestone 10 (Remove Old Reward System).
/// The automatic epoch reward distribution system has been replaced with weighted
/// presence rewards using ClaimEpochReward transactions.
///
/// This is the strict validation mode that verifies each producer receives
/// exactly their proportional share of rewards based on blocks produced.
///
/// Uses `EpochBlockSource` to access historical blockchain data and
/// recalculate expected rewards deterministically.
///
/// # Arguments
/// * `block` - The block to validate
/// * `ctx` - Validation context with consensus parameters
/// * `source` - BlockStore accessor for epoch data
///
/// # Validation Rules
///
/// At epoch boundary (`current_epoch > last_rewarded`):
/// - Block MUST contain EpochReward transactions
/// - Each transaction MUST have correct epoch number
/// - Recipients MUST match producers who created blocks
/// - Amounts MUST match proportional distribution
/// - Total MUST equal `blocks_produced × block_reward`
///
/// At non-boundary:
/// - Block MUST NOT contain any reward transactions
#[deprecated(
    since = "0.1.0",
    note = "Automatic epoch rewards replaced by ClaimEpochReward transactions"
)]
#[allow(deprecated)]
pub fn validate_block_rewards_exact<S: EpochBlockSource>(
    block: &Block,
    ctx: &ValidationContext,
    source: &S,
) -> Result<(), ValidationError> {
    // Only applies to EpochPool mode
    if ctx.params.reward_mode != RewardMode::EpochPool {
        return Ok(());
    }

    let slot = block.header.slot;
    let _slots_per_epoch = ctx.params.slots_per_reward_epoch as u64;

    // Determine if this block should include epoch rewards
    let epoch_to_reward = match epoch_needing_rewards(slot, &ctx.params, source) {
        Ok(epoch) => epoch,
        Err(e) => {
            return Err(ValidationError::InvalidBlock(format!(
                "failed to determine epoch reward status: {}",
                e
            )));
        }
    };

    // Get the block's epoch reward transactions
    let block_epoch_rewards: Vec<_> = block
        .transactions
        .iter()
        .filter(|tx| tx.is_epoch_reward())
        .collect();

    // Verify no coinbase in EpochPool mode blocks
    if block.transactions.iter().any(|tx| tx.is_coinbase()) {
        return Err(ValidationError::InvalidBlock(
            "coinbase not allowed in EpochPool mode".to_string(),
        ));
    }

    if let Some(epoch) = epoch_to_reward {
        // Epoch 0 (genesis): pool was consumed by genesis bonds, no distribution.
        // Remainder carries to E1. Skip validation for E0.
        if epoch == 0 {
            if !block_epoch_rewards.is_empty() {
                return Err(ValidationError::InvalidEpochReward(
                    "epoch 0 must not distribute rewards (genesis pool used for bonds)".to_string(),
                ));
            }
            return Ok(());
        }

        // Block SHOULD include epoch rewards - validate exact distribution

        if block_epoch_rewards.is_empty() {
            return Err(ValidationError::MissingEpochReward { epoch });
        }

        // Calculate slot range for the epoch being rewarded
        let start_slot = if epoch == 0 {
            1u32 // Skip genesis at slot 0
        } else {
            epoch as u32 * ctx.params.slots_per_reward_epoch
        };
        let end_slot = (epoch + 1) as u32 * ctx.params.slots_per_reward_epoch;

        // Get blocks from source
        let epoch_blocks = source
            .blocks_in_slot_range(start_slot, end_slot)
            .map_err(|e| {
                ValidationError::InvalidBlock(format!("failed to get epoch blocks: {}", e))
            })?;

        // Calculate expected rewards
        let expected_rewards =
            calculate_expected_epoch_rewards(epoch, &epoch_blocks, ctx.current_height, &ctx.params);

        // Verify transaction count matches
        if block_epoch_rewards.len() != expected_rewards.len() {
            return Err(ValidationError::EpochRewardMismatch {
                reason: format!(
                    "expected {} reward transactions, got {}",
                    expected_rewards.len(),
                    block_epoch_rewards.len()
                ),
            });
        }

        // Build a map of expected rewards for comparison
        let mut expected_map: std::collections::HashMap<&PublicKey, u64> =
            std::collections::HashMap::new();
        for (pubkey, amount) in &expected_rewards {
            expected_map.insert(pubkey, *amount);
        }

        // Verify each epoch reward transaction
        for tx in &block_epoch_rewards {
            let data = tx.epoch_reward_data().ok_or_else(|| {
                ValidationError::InvalidEpochReward("missing epoch reward data".to_string())
            })?;

            // Verify epoch number
            if data.epoch != epoch {
                return Err(ValidationError::InvalidEpochReward(format!(
                    "epoch mismatch: got {}, expected {}",
                    data.epoch, epoch
                )));
            }

            // Verify recipient is expected
            let expected_amount = expected_map.get(&data.recipient).ok_or_else(|| {
                ValidationError::EpochRewardMismatch {
                    reason: format!(
                        "unexpected recipient: {}",
                        &crypto::hash::hash(data.recipient.as_bytes()).to_hex()[..16]
                    ),
                }
            })?;

            // Verify amount matches
            let actual_amount = tx.outputs.first().map(|o| o.amount).unwrap_or(0);
            if actual_amount != *expected_amount {
                return Err(ValidationError::EpochRewardMismatch {
                    reason: format!(
                        "amount mismatch for producer: expected {}, got {}",
                        expected_amount, actual_amount
                    ),
                });
            }
        }
    } else {
        // Block should NOT include epoch rewards
        if !block_epoch_rewards.is_empty() {
            return Err(ValidationError::UnexpectedEpochReward);
        }
    }

    Ok(())
}
