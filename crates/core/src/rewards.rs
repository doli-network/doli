//! # Weighted Presence Reward Calculation
//!
//! This module provides deterministic calculation of weighted presence rewards
//! for the epoch-based claim system.
//!
//! ## Overview
//!
//! Rewards are distributed proportionally based on bond weight:
//!
//! ```text
//! For each block where producer was present:
//!   producer_reward += block_reward × producer_weight / total_present_weight
//! ```
//!
//! ## Key Components
//!
//! - [`BlockSource`] - Trait for accessing blocks by height
//! - [`WeightedRewardCalculator`] - Main calculator that scans epoch blocks
//! - [`WeightedRewardCalculation`] - Result of reward calculation
//! - [`ClaimableSummary`] - Summary for UI display
//!
//! ## Determinism
//!
//! All calculations use integer arithmetic with u128 for intermediate values
//! to prevent overflow. The same inputs always produce the same outputs,
//! ensuring all nodes calculate identical reward amounts.

use crypto::{Hash, PublicKey};
use serde::{Deserialize, Serialize};

use crate::consensus::{reward_epoch, ConsensusParams};
use crate::types::{Amount, BlockHeight};
use crate::Block;

// =============================================================================
// ERROR TYPES
// =============================================================================

/// Errors that can occur during reward calculation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewardError {
    /// Block not found at expected height.
    BlockNotFound { height: BlockHeight },
    /// Storage error while accessing blocks.
    StorageError(String),
    /// Epoch is not yet complete.
    EpochNotComplete {
        epoch: u64,
        current_height: BlockHeight,
    },
    /// Producer not found in producer set.
    ProducerNotFound { producer: PublicKey },
}

impl std::fmt::Display for RewardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BlockNotFound { height } => {
                write!(f, "block not found at height {}", height)
            }
            Self::StorageError(msg) => {
                write!(f, "storage error: {}", msg)
            }
            Self::EpochNotComplete {
                epoch,
                current_height,
            } => {
                write!(
                    f,
                    "epoch {} is not complete (current height: {})",
                    epoch, current_height
                )
            }
            Self::ProducerNotFound { producer } => {
                write!(f, "producer not found: {:?}", producer)
            }
        }
    }
}

impl std::error::Error for RewardError {}

// =============================================================================
// BLOCK SOURCE TRAIT
// =============================================================================

/// Trait for accessing blocks by height.
///
/// This trait abstracts block storage access, allowing the reward calculator
/// to work with different storage implementations (BlockStore, in-memory, etc.).
///
/// # Implementation Notes
///
/// Implementors should return `None` for heights that don't exist yet.
/// Storage errors should be converted to `RewardError::StorageError`.
pub trait BlockSource {
    /// Get a block by its height.
    ///
    /// Returns `Ok(None)` if no block exists at the given height.
    /// Returns `Err` only for actual storage failures.
    fn get_block_by_height(&self, height: BlockHeight) -> Result<Option<Block>, RewardError>;
}

// =============================================================================
// WEIGHTED REWARD CALCULATION RESULT
// =============================================================================

/// Result of calculating weighted presence rewards for a producer in an epoch.
///
/// Contains all intermediate values for transparency and debugging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeightedRewardCalculation {
    /// Epoch that was calculated.
    pub epoch: u64,
    /// Producer's public key.
    pub producer: PublicKey,
    /// Producer's index in the sorted producer set.
    pub producer_index: usize,
    /// Number of blocks where the producer was present.
    pub blocks_present: u64,
    /// Total blocks in the epoch (may be less than BLOCKS_PER_REWARD_EPOCH
    /// if some blocks are missing).
    pub total_blocks: u64,
    /// Sum of producer's weight across all present blocks.
    pub total_producer_weight: Amount,
    /// Sum of all weights across all blocks where producer was present.
    pub total_all_weights: Amount,
    /// Block reward rate used for calculation.
    pub block_reward: Amount,
    /// Final calculated reward amount.
    pub reward_amount: Amount,
}

impl WeightedRewardCalculation {
    /// Check if the producer earned any reward.
    #[inline]
    pub fn has_reward(&self) -> bool {
        self.reward_amount > 0
    }

    /// Calculate the producer's average weight per block.
    pub fn average_weight(&self) -> Amount {
        if self.blocks_present == 0 {
            0
        } else {
            self.total_producer_weight / self.blocks_present
        }
    }

    /// Calculate the producer's presence rate as a percentage.
    pub fn presence_rate(&self) -> u8 {
        if self.total_blocks == 0 {
            0
        } else {
            ((self.blocks_present * 100) / self.total_blocks).min(100) as u8
        }
    }
}

// =============================================================================
// CLAIMABLE SUMMARY
// =============================================================================

/// Summary of a claimable epoch for UI display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimableSummary {
    /// Epoch number.
    pub epoch: u64,
    /// Number of blocks where producer was present.
    pub blocks_present: u64,
    /// Estimated reward amount.
    pub estimated_reward: Amount,
    /// Whether this epoch has been claimed.
    pub is_claimed: bool,
    /// Transaction hash if claimed.
    pub claim_tx_hash: Option<Hash>,
}

// =============================================================================
// WEIGHTED REWARD CALCULATOR
// =============================================================================

/// Calculator for weighted presence rewards.
///
/// Uses the [`BlockSource`] trait to access blocks and calculate rewards
/// deterministically. Each node will calculate the same reward for the
/// same inputs.
///
/// # Example
///
/// ```rust,ignore
/// use doli_core::rewards::{BlockSource, WeightedRewardCalculator};
/// use doli_core::consensus::ConsensusParams;
///
/// let params = ConsensusParams::mainnet();
/// let calculator = WeightedRewardCalculator::new(&block_store, &params);
///
/// // Calculate reward for producer in epoch 5
/// let result = calculator.calculate_producer_reward(&producer_pubkey, 0, 5)?;
/// println!("Reward: {}", result.reward_amount);
/// ```
pub struct WeightedRewardCalculator<'a, B: BlockSource> {
    block_source: &'a B,
    params: &'a ConsensusParams,
    /// Blocks per reward epoch (network-specific: mainnet/testnet=360, devnet=60)
    blocks_per_epoch: u64,
}

impl<'a, B: BlockSource> WeightedRewardCalculator<'a, B> {
    /// Create a new reward calculator with default blocks per epoch (360).
    ///
    /// # Arguments
    ///
    /// * `block_source` - Source for accessing blocks by height
    /// * `params` - Consensus parameters (for block reward calculation)
    pub fn new(block_source: &'a B, params: &'a ConsensusParams) -> Self {
        Self {
            block_source,
            params,
            blocks_per_epoch: reward_epoch::blocks_per_epoch(), // mainnet default
        }
    }

    /// Create a new reward calculator with custom blocks per epoch.
    ///
    /// Use this for network-specific epoch sizes (e.g., devnet uses 60).
    pub fn with_blocks_per_epoch(
        block_source: &'a B,
        params: &'a ConsensusParams,
        blocks_per_epoch: u64,
    ) -> Self {
        Self {
            block_source,
            params,
            blocks_per_epoch,
        }
    }

    /// Calculate weighted presence reward for a producer in an epoch.
    ///
    /// Scans all blocks in the epoch (exactly BLOCKS_PER_REWARD_EPOCH blocks)
    /// and calculates the producer's share of rewards based on their weight
    /// relative to total weight of all present producers.
    ///
    /// # Formula
    ///
    /// For each block where producer was present:
    /// ```text
    /// reward += block_reward × producer_weight / total_present_weight
    /// ```
    ///
    /// # Arguments
    ///
    /// * `producer` - The producer's public key
    /// * `producer_index` - The producer's index in the sorted producer set
    /// * `epoch` - The epoch to calculate rewards for
    ///
    /// # Returns
    ///
    /// A [`WeightedRewardCalculation`] containing the reward amount and
    /// all intermediate values.
    ///
    /// # Errors
    ///
    /// Returns an error if blocks cannot be accessed or if there's a
    /// storage failure.
    /// Calculate producer reward for an epoch.
    ///
    /// # Deprecation Notice
    ///
    /// This method is deprecated. In the deterministic scheduler model,
    /// 100% of block rewards go directly to producers via coinbase.
    /// There is no separate presence-based reward calculation.
    ///
    /// This method now returns an empty calculation for compatibility.
    #[deprecated(note = "Use coinbase rewards - 100% to producer")]
    pub fn calculate_producer_reward(
        &self,
        producer: &PublicKey,
        producer_index: usize,
        epoch: u64,
    ) -> Result<WeightedRewardCalculation, RewardError> {
        let (start_height, end_height) =
            reward_epoch::boundaries_with(epoch, self.blocks_per_epoch);

        let mut total_blocks: u64 = 0;

        // Just count blocks in the epoch
        for height in start_height..end_height {
            if self.block_source.get_block_by_height(height)?.is_some() {
                total_blocks += 1;
            }
        }

        // In deterministic scheduler model, rewards are via coinbase (100% to producer)
        // This calculation returns 0 as presence-based rewards are deprecated
        Ok(WeightedRewardCalculation {
            epoch,
            producer: *producer,
            producer_index,
            blocks_present: 0,
            total_blocks,
            total_producer_weight: 0,
            total_all_weights: 0,
            block_reward: self.params.block_reward(start_height),
            reward_amount: 0, // No presence-based rewards in deterministic scheduler
        })
    }

    /// Calculate rewards for a producer across multiple epochs.
    ///
    /// This is useful for batch claiming or displaying total unclaimed rewards.
    ///
    /// # Arguments
    ///
    /// * `producer` - The producer's public key
    /// * `producer_index` - The producer's index in the sorted producer set
    /// * `epochs` - Iterator of epoch numbers to calculate
    ///
    /// # Returns
    ///
    /// A vector of calculations, one per epoch. Epochs with zero reward
    /// are included.
    #[allow(deprecated)] // Uses deprecated calculate_producer_reward
    pub fn calculate_multiple_epochs<I: IntoIterator<Item = u64>>(
        &self,
        producer: &PublicKey,
        producer_index: usize,
        epochs: I,
    ) -> Result<Vec<WeightedRewardCalculation>, RewardError> {
        epochs
            .into_iter()
            .map(|epoch| self.calculate_producer_reward(producer, producer_index, epoch))
            .collect()
    }

    /// Get the total claimable reward for a producer across all unclaimed epochs.
    ///
    /// # Arguments
    ///
    /// * `producer` - The producer's public key
    /// * `producer_index` - The producer's index in the sorted producer set
    /// * `unclaimed_epochs` - Iterator of unclaimed epoch numbers
    ///
    /// # Returns
    ///
    /// The total reward amount across all specified epochs.
    pub fn total_claimable_reward<I: IntoIterator<Item = u64>>(
        &self,
        producer: &PublicKey,
        producer_index: usize,
        unclaimed_epochs: I,
    ) -> Result<Amount, RewardError> {
        let calculations =
            self.calculate_multiple_epochs(producer, producer_index, unclaimed_epochs)?;
        Ok(calculations.iter().map(|c| c.reward_amount).sum())
    }
}

// =============================================================================
// HELPER FUNCTIONS
// =============================================================================

/// Calculate the number of complete epochs at a given block height.
/// Uses mainnet default (360 blocks/epoch).
#[inline]
pub fn complete_epochs_at_height(height: BlockHeight) -> u64 {
    reward_epoch::complete_epochs(height)
}

/// Calculate the number of complete epochs at a given block height (network-aware).
#[inline]
pub fn complete_epochs_at_height_with(height: BlockHeight, blocks_per_epoch: u64) -> u64 {
    reward_epoch::complete_epochs_with(height, blocks_per_epoch)
}

/// Get the epoch boundaries for a given epoch number.
/// Uses mainnet default (360 blocks/epoch).
#[inline]
pub fn epoch_boundaries(epoch: u64) -> (BlockHeight, BlockHeight) {
    reward_epoch::boundaries(epoch)
}

/// Get the epoch boundaries for a given epoch number (network-aware).
#[inline]
pub fn epoch_boundaries_with(epoch: u64, blocks_per_epoch: u64) -> (BlockHeight, BlockHeight) {
    reward_epoch::boundaries_with(epoch, blocks_per_epoch)
}

/// Check if an epoch is complete at the given height.
/// Uses mainnet default (360 blocks/epoch).
#[inline]
pub fn is_epoch_complete(epoch: u64, current_height: BlockHeight) -> bool {
    reward_epoch::is_complete(epoch, current_height)
}

/// Check if an epoch is complete at the given height (network-aware).
#[inline]
pub fn is_epoch_complete_with(
    epoch: u64,
    current_height: BlockHeight,
    blocks_per_epoch: u64,
) -> bool {
    reward_epoch::is_complete_with(epoch, current_height, blocks_per_epoch)
}

/// Get all complete epochs up to (but not including) the current epoch.
/// Uses mainnet default (360 blocks/epoch).
pub fn complete_epoch_range(current_height: BlockHeight) -> std::ops::Range<u64> {
    let current_epoch = reward_epoch::from_height(current_height);
    0..current_epoch
}

/// Get all complete epochs up to (but not including) the current epoch (network-aware).
pub fn complete_epoch_range_with(
    current_height: BlockHeight,
    blocks_per_epoch: u64,
) -> std::ops::Range<u64> {
    let current_epoch = reward_epoch::from_height_with(current_height, blocks_per_epoch);
    0..current_epoch
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::BLOCKS_PER_REWARD_EPOCH;
    use crate::BlockHeader;
    use crypto::Hash;
    use std::collections::HashMap;
    use vdf::{VdfOutput, VdfProof};

    /// Mock block source for testing.
    struct MockBlockSource {
        blocks: HashMap<BlockHeight, Block>,
    }

    impl MockBlockSource {
        fn new() -> Self {
            Self {
                blocks: HashMap::new(),
            }
        }

        fn add_block(&mut self, height: BlockHeight, block: Block) {
            self.blocks.insert(height, block);
        }
    }

    impl BlockSource for MockBlockSource {
        fn get_block_by_height(&self, height: BlockHeight) -> Result<Option<Block>, RewardError> {
            Ok(self.blocks.get(&height).cloned())
        }
    }

    /// Create a test block (no presence commitment in deterministic scheduler model).
    fn create_test_block() -> Block {
        let header = BlockHeader {
            version: 1,
            prev_hash: Hash::ZERO,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: 0,
            slot: 0,
            producer: PublicKey::from_bytes([0u8; 32]),
            vdf_output: VdfOutput {
                value: vec![0u8; 32],
            },
            vdf_proof: VdfProof { pi: vec![] },
            missed_producers: Vec::new(),
            data_root: Hash::ZERO,
        };

        // NOTE: Presence commitment removed in deterministic scheduler model
        // Rewards are 100% to block producer via coinbase
        Block::new(header, vec![])
    }

    /// Create test producer public key.
    fn test_producer(id: u8) -> PublicKey {
        let mut bytes = [0u8; 32];
        bytes[0] = id;
        PublicKey::from_bytes(bytes)
    }

    // =========================================================================
    // DEPRECATED PRESENCE-BASED REWARD TESTS
    // =========================================================================
    // The following tests verified the old presence-based weighted reward system.
    // In the deterministic scheduler model, rewards go 100% to block producers
    // via coinbase. The calculate_producer_reward method is deprecated and
    // always returns 0 rewards.
    //
    // Tests removed:
    // - test_single_producer_all_present
    // - test_two_producers_equal_weight
    // - test_two_producers_different_weight
    // - test_producer_absent_some_blocks
    // - test_u128_prevents_overflow
    // - test_multiple_epochs
    // - test_total_claimable_reward
    // - test_devnet_epoch_boundaries
    // - test_devnet_epoch_1_boundaries
    // - test_mainnet_vs_devnet_epoch_isolation
    // =========================================================================

    #[test]
    fn test_deprecated_calculate_producer_reward_returns_zero() {
        // In deterministic scheduler model, calculate_producer_reward is deprecated
        // and always returns 0 rewards (rewards go via coinbase to producer)
        let mut source = MockBlockSource::new();
        let params = ConsensusParams::mainnet();

        let blocks_per_epoch = BLOCKS_PER_REWARD_EPOCH;
        for height in 0..blocks_per_epoch {
            source.add_block(height, create_test_block());
        }

        let calculator = WeightedRewardCalculator::new(&source, &params);
        let producer = test_producer(1);

        #[allow(deprecated)]
        let result = calculator
            .calculate_producer_reward(&producer, 0, 0)
            .unwrap();

        // Deprecated method returns 0 rewards
        assert_eq!(result.blocks_present, 0);
        assert_eq!(result.total_blocks, blocks_per_epoch);
        assert_eq!(result.reward_amount, 0);
        assert!(!result.has_reward());
    }

    #[test]
    fn test_calculation_is_deterministic() {
        // Same inputs always produce same outputs
        let mut source = MockBlockSource::new();
        let params = ConsensusParams::mainnet();

        let blocks_per_epoch = BLOCKS_PER_REWARD_EPOCH;
        for height in 0..blocks_per_epoch {
            source.add_block(height, create_test_block());
        }

        let calculator = WeightedRewardCalculator::new(&source, &params);

        // Calculate multiple times - should be deterministic
        #[allow(deprecated)]
        let result1 = calculator
            .calculate_producer_reward(&test_producer(1), 0, 0)
            .unwrap();
        #[allow(deprecated)]
        let result2 = calculator
            .calculate_producer_reward(&test_producer(1), 0, 0)
            .unwrap();
        #[allow(deprecated)]
        let result3 = calculator
            .calculate_producer_reward(&test_producer(1), 0, 0)
            .unwrap();

        assert_eq!(result1, result2);
        assert_eq!(result2, result3);
    }

    #[test]
    fn test_weighted_reward_calculation_methods() {
        let calc = WeightedRewardCalculation {
            epoch: 5,
            producer: test_producer(1),
            producer_index: 0,
            blocks_present: 180,
            total_blocks: 360,
            total_producer_weight: 180_000,
            total_all_weights: 360_000,
            block_reward: 100_000_000,
            reward_amount: 18_000_000_000,
        };

        assert!(calc.has_reward());
        assert_eq!(calc.presence_rate(), 50);
        assert_eq!(calc.average_weight(), 1000);
    }

    #[test]
    fn test_helper_functions() {
        // Test complete_epochs_at_height
        assert_eq!(complete_epochs_at_height(0), 0);
        assert_eq!(complete_epochs_at_height(359), 0);
        assert_eq!(complete_epochs_at_height(360), 1);
        assert_eq!(complete_epochs_at_height(720), 2);

        // Test epoch_boundaries
        assert_eq!(epoch_boundaries(0), (0, 360));
        assert_eq!(epoch_boundaries(5), (1800, 2160));

        // Test is_epoch_complete
        assert!(!is_epoch_complete(0, 359));
        assert!(is_epoch_complete(0, 360));

        // Test complete_epoch_range
        let range = complete_epoch_range(720);
        assert_eq!(range, 0..2);
    }

    #[test]
    fn test_reward_error_display() {
        let err = RewardError::BlockNotFound { height: 42 };
        assert_eq!(err.to_string(), "block not found at height 42");

        let err = RewardError::StorageError("disk full".to_string());
        assert_eq!(err.to_string(), "storage error: disk full");

        let err = RewardError::EpochNotComplete {
            epoch: 5,
            current_height: 1000,
        };
        assert_eq!(
            err.to_string(),
            "epoch 5 is not complete (current height: 1000)"
        );
    }
}
