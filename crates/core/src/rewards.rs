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
            blocks_per_epoch: reward_epoch::blocks_per_epoch(),
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
    pub fn calculate_producer_reward(
        &self,
        producer: &PublicKey,
        producer_index: usize,
        epoch: u64,
    ) -> Result<WeightedRewardCalculation, RewardError> {
        let (start_height, end_height) =
            reward_epoch::boundaries_with(epoch, self.blocks_per_epoch);

        let mut blocks_present: u64 = 0;
        let mut total_blocks: u64 = 0;
        let mut total_producer_weight: Amount = 0;
        let mut total_all_weights: Amount = 0;
        let mut reward_amount: Amount = 0;

        // Scan all blocks in the epoch
        for height in start_height..end_height {
            let block = match self.block_source.get_block_by_height(height)? {
                Some(b) => b,
                None => continue, // Block not yet produced (shouldn't happen for complete epochs)
            };

            total_blocks += 1;

            // Check if block has presence commitment
            let presence = match &block.presence {
                Some(p) => p,
                None => continue, // No presence data, skip
            };

            // Check if producer was present
            if let Some(weight) = presence.get_weight(producer_index) {
                blocks_present += 1;
                total_producer_weight += weight;
                total_all_weights += presence.total_weight;

                // Calculate reward for this block
                // reward = block_reward × weight / total_weight
                // Use u128 to prevent overflow
                let block_reward = self.params.block_reward(height);

                if presence.total_weight > 0 {
                    let numerator = (block_reward as u128) * (weight as u128);
                    let block_share = (numerator / (presence.total_weight as u128)) as Amount;
                    reward_amount += block_share;
                }
            }
        }

        Ok(WeightedRewardCalculation {
            epoch,
            producer: *producer,
            producer_index,
            blocks_present,
            total_blocks,
            total_producer_weight,
            total_all_weights,
            block_reward: self.params.block_reward(start_height),
            reward_amount,
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
#[inline]
pub fn complete_epochs_at_height(height: BlockHeight) -> u64 {
    reward_epoch::complete_epochs(height)
}

/// Get the epoch boundaries for a given epoch number.
#[inline]
pub fn epoch_boundaries(epoch: u64) -> (BlockHeight, BlockHeight) {
    reward_epoch::boundaries(epoch)
}

/// Check if an epoch is complete at the given height.
#[inline]
pub fn is_epoch_complete(epoch: u64, current_height: BlockHeight) -> bool {
    reward_epoch::is_complete(epoch, current_height)
}

/// Get all complete epochs up to (but not including) the current epoch.
pub fn complete_epoch_range(current_height: BlockHeight) -> std::ops::Range<u64> {
    let current_epoch = reward_epoch::from_height(current_height);
    0..current_epoch
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::BLOCKS_PER_REWARD_EPOCH;
    use crate::presence::PresenceCommitment;
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

    /// Create a test block with presence commitment.
    fn create_test_block(
        producer_count: usize,
        present_indices: &[usize],
        weights: Vec<Amount>,
    ) -> Block {
        let header = BlockHeader {
            version: 1,
            prev_hash: Hash::ZERO,
            merkle_root: Hash::ZERO,
            timestamp: 0,
            slot: 0,
            producer: PublicKey::from_bytes([0u8; 32]),
            vdf_output: VdfOutput {
                value: vec![0u8; 32],
            },
            vdf_proof: VdfProof { pi: vec![] },
        };

        let presence =
            PresenceCommitment::new(producer_count, present_indices, weights, Hash::ZERO);

        Block::new_with_presence(header, vec![], presence)
    }

    /// Create test producer public key.
    fn test_producer(id: u8) -> PublicKey {
        let mut bytes = [0u8; 32];
        bytes[0] = id;
        PublicKey::from_bytes(bytes)
    }

    #[test]
    fn test_single_producer_all_present() {
        // Single producer present in all blocks gets 100% of rewards
        let mut source = MockBlockSource::new();
        let params = ConsensusParams::mainnet();

        // Create blocks for epoch 0 using the global BLOCKS_PER_REWARD_EPOCH constant
        let blocks_per_epoch = BLOCKS_PER_REWARD_EPOCH;
        for height in 0..blocks_per_epoch {
            let block = create_test_block(1, &[0], vec![1000]);
            source.add_block(height, block);
        }

        let calculator = WeightedRewardCalculator::new(&source, &params);
        let producer = test_producer(1);
        let result = calculator
            .calculate_producer_reward(&producer, 0, 0)
            .unwrap();

        // Producer should get 100% of each block's reward
        let block_reward = params.block_reward(0);
        let expected_reward = block_reward * blocks_per_epoch;

        assert_eq!(result.blocks_present, blocks_per_epoch);
        assert_eq!(result.total_blocks, blocks_per_epoch);
        assert_eq!(result.reward_amount, expected_reward);
        assert_eq!(result.presence_rate(), 100);
    }

    #[test]
    fn test_two_producers_equal_weight() {
        // Two producers with equal weight, both present, get 50% each
        let mut source = MockBlockSource::new();
        let params = ConsensusParams::mainnet();

        let blocks_per_epoch = BLOCKS_PER_REWARD_EPOCH;
        for height in 0..blocks_per_epoch {
            // Both producers present with weight 1000 each
            let block = create_test_block(2, &[0, 1], vec![1000, 1000]);
            source.add_block(height, block);
        }

        let calculator = WeightedRewardCalculator::new(&source, &params);

        // Check producer 0
        let result0 = calculator
            .calculate_producer_reward(&test_producer(1), 0, 0)
            .unwrap();
        let block_reward = params.block_reward(0);
        let expected_each = (block_reward / 2) * blocks_per_epoch;

        assert_eq!(result0.blocks_present, blocks_per_epoch);
        assert_eq!(result0.reward_amount, expected_each);

        // Check producer 1
        let result1 = calculator
            .calculate_producer_reward(&test_producer(2), 1, 0)
            .unwrap();
        assert_eq!(result1.blocks_present, blocks_per_epoch);
        assert_eq!(result1.reward_amount, expected_each);

        // Combined should equal total block rewards
        let total = result0.reward_amount + result1.reward_amount;
        let expected_total = block_reward * blocks_per_epoch;
        assert_eq!(total, expected_total);
    }

    #[test]
    fn test_two_producers_different_weight() {
        // Producer with 2x weight gets 2x reward
        let mut source = MockBlockSource::new();
        let params = ConsensusParams::mainnet();

        let blocks_per_epoch = BLOCKS_PER_REWARD_EPOCH;
        for height in 0..blocks_per_epoch {
            // Producer 0 has weight 2000, producer 1 has weight 1000
            // Total weight = 3000, so producer 0 gets 2/3, producer 1 gets 1/3
            let block = create_test_block(2, &[0, 1], vec![2000, 1000]);
            source.add_block(height, block);
        }

        let calculator = WeightedRewardCalculator::new(&source, &params);

        let result0 = calculator
            .calculate_producer_reward(&test_producer(1), 0, 0)
            .unwrap();
        let result1 = calculator
            .calculate_producer_reward(&test_producer(2), 1, 0)
            .unwrap();

        // Producer 0 should get roughly 2x what producer 1 gets
        // Due to integer division, check ratio is close to 2:1
        assert!(result0.reward_amount > result1.reward_amount);

        // More precise check: result0 / result1 should be ~2
        let ratio = result0.reward_amount as f64 / result1.reward_amount as f64;
        assert!(
            (ratio - 2.0).abs() < 0.01,
            "Ratio should be ~2.0, got {}",
            ratio
        );
    }

    #[test]
    fn test_producer_absent_some_blocks() {
        // Producer present in half the blocks gets half the reward
        let mut source = MockBlockSource::new();
        let params = ConsensusParams::mainnet();

        let blocks_per_epoch = BLOCKS_PER_REWARD_EPOCH;
        for height in 0..blocks_per_epoch {
            // Producer 0 only present in even-numbered blocks
            if height % 2 == 0 {
                let block = create_test_block(1, &[0], vec![1000]);
                source.add_block(height, block);
            } else {
                // No one present (empty presence)
                let block = create_test_block(1, &[], vec![]);
                source.add_block(height, block);
            }
        }

        let calculator = WeightedRewardCalculator::new(&source, &params);
        let result = calculator
            .calculate_producer_reward(&test_producer(1), 0, 0)
            .unwrap();

        // Present in half the blocks
        assert_eq!(result.blocks_present, blocks_per_epoch / 2);
        assert_eq!(result.total_blocks, blocks_per_epoch);
        assert_eq!(result.presence_rate(), 50);

        // Reward should be for only the blocks where present
        let block_reward = params.block_reward(0);
        let expected = block_reward * (blocks_per_epoch / 2);
        assert_eq!(result.reward_amount, expected);
    }

    #[test]
    fn test_empty_epoch() {
        // No blocks with presence → zero reward
        let mut source = MockBlockSource::new();
        let params = ConsensusParams::mainnet();

        let blocks_per_epoch = BLOCKS_PER_REWARD_EPOCH;
        for height in 0..blocks_per_epoch {
            // Blocks exist but have no presence commitment
            let header = BlockHeader {
                version: 1,
                prev_hash: Hash::ZERO,
                merkle_root: Hash::ZERO,
                timestamp: 0,
                slot: 0,
                producer: PublicKey::from_bytes([0u8; 32]),
                vdf_output: VdfOutput {
                    value: vec![0u8; 32],
                },
                vdf_proof: VdfProof { pi: vec![] },
            };
            let block = Block::new(header, vec![]);
            source.add_block(height, block);
        }

        let calculator = WeightedRewardCalculator::new(&source, &params);
        let result = calculator
            .calculate_producer_reward(&test_producer(1), 0, 0)
            .unwrap();

        assert_eq!(result.blocks_present, 0);
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
            let block = create_test_block(3, &[0, 1, 2], vec![1000, 2000, 3000]);
            source.add_block(height, block);
        }

        let calculator = WeightedRewardCalculator::new(&source, &params);

        // Calculate multiple times
        let result1 = calculator
            .calculate_producer_reward(&test_producer(1), 0, 0)
            .unwrap();
        let result2 = calculator
            .calculate_producer_reward(&test_producer(1), 0, 0)
            .unwrap();
        let result3 = calculator
            .calculate_producer_reward(&test_producer(1), 0, 0)
            .unwrap();

        assert_eq!(result1, result2);
        assert_eq!(result2, result3);
    }

    #[test]
    fn test_u128_prevents_overflow() {
        // Test with large weights that would overflow u64
        let mut source = MockBlockSource::new();
        let params = ConsensusParams::mainnet();

        let blocks_per_epoch = BLOCKS_PER_REWARD_EPOCH;

        // Use maximum-ish weights to test overflow protection
        let large_weight: Amount = 1_000_000_000_000_000; // 1 quadrillion

        for height in 0..blocks_per_epoch {
            let block = create_test_block(2, &[0, 1], vec![large_weight, large_weight]);
            source.add_block(height, block);
        }

        let calculator = WeightedRewardCalculator::new(&source, &params);

        // This should not panic due to overflow
        let result = calculator.calculate_producer_reward(&test_producer(1), 0, 0);
        assert!(result.is_ok());

        let calc = result.unwrap();
        // Each producer should get 50% of rewards
        let block_reward = params.block_reward(0);
        let expected = (block_reward / 2) * blocks_per_epoch;
        assert_eq!(calc.reward_amount, expected);
    }

    #[test]
    fn test_multiple_epochs() {
        let mut source = MockBlockSource::new();
        let params = ConsensusParams::mainnet();

        let blocks_per_epoch = BLOCKS_PER_REWARD_EPOCH;

        // Create blocks for epochs 0, 1, 2
        for epoch in 0..3 {
            let (start, end) = reward_epoch::boundaries(epoch);
            for height in start..end {
                let block = create_test_block(1, &[0], vec![1000]);
                source.add_block(height, block);
            }
        }

        let calculator = WeightedRewardCalculator::new(&source, &params);
        let results = calculator
            .calculate_multiple_epochs(&test_producer(1), 0, 0..3)
            .unwrap();

        assert_eq!(results.len(), 3);

        // All epochs should have the same reward (mainnet era is long enough)
        let expected_per_epoch = params.block_reward(0) * blocks_per_epoch;
        for result in &results {
            assert_eq!(result.reward_amount, expected_per_epoch);
        }
    }

    #[test]
    fn test_total_claimable_reward() {
        let mut source = MockBlockSource::new();
        let params = ConsensusParams::mainnet();

        let blocks_per_epoch = BLOCKS_PER_REWARD_EPOCH;

        // Create blocks for epochs 0, 1, 2
        for epoch in 0..3 {
            let (start, end) = reward_epoch::boundaries(epoch);
            for height in start..end {
                let block = create_test_block(1, &[0], vec![1000]);
                source.add_block(height, block);
            }
        }

        let calculator = WeightedRewardCalculator::new(&source, &params);
        let total = calculator
            .total_claimable_reward(&test_producer(1), 0, 0..3)
            .unwrap();

        let expected_per_epoch = params.block_reward(0) * blocks_per_epoch;
        assert_eq!(total, expected_per_epoch * 3);
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

    #[test]
    fn test_devnet_epoch_boundaries() {
        // Test that with_blocks_per_epoch correctly uses devnet's 60-block epochs
        let mut source = MockBlockSource::new();
        let params = ConsensusParams::mainnet();

        const DEVNET_BLOCKS_PER_EPOCH: u64 = 60;

        // Create blocks for devnet epoch 0 (heights 0-59)
        for height in 0..DEVNET_BLOCKS_PER_EPOCH {
            let block = create_test_block(1, &[0], vec![1000]);
            source.add_block(height, block);
        }

        // Use the devnet-specific epoch size
        let calculator = WeightedRewardCalculator::with_blocks_per_epoch(
            &source,
            &params,
            DEVNET_BLOCKS_PER_EPOCH,
        );
        let producer = test_producer(1);
        let result = calculator
            .calculate_producer_reward(&producer, 0, 0)
            .unwrap();

        // Should see all 60 blocks for devnet epoch 0
        assert_eq!(result.total_blocks, DEVNET_BLOCKS_PER_EPOCH);
        assert_eq!(result.blocks_present, DEVNET_BLOCKS_PER_EPOCH);

        // Reward should be for 60 blocks, not 360
        let block_reward = params.block_reward(0);
        let expected_reward = block_reward * DEVNET_BLOCKS_PER_EPOCH;
        assert_eq!(result.reward_amount, expected_reward);
    }

    #[test]
    fn test_devnet_epoch_1_boundaries() {
        // Test that devnet epoch 1 uses blocks 60-119
        let mut source = MockBlockSource::new();
        let params = ConsensusParams::mainnet();

        const DEVNET_BLOCKS_PER_EPOCH: u64 = 60;

        // Create blocks for devnet epoch 1 (heights 60-119)
        for height in 60..120 {
            let block = create_test_block(1, &[0], vec![1000]);
            source.add_block(height, block);
        }

        let calculator = WeightedRewardCalculator::with_blocks_per_epoch(
            &source,
            &params,
            DEVNET_BLOCKS_PER_EPOCH,
        );
        let producer = test_producer(1);
        let result = calculator
            .calculate_producer_reward(&producer, 0, 1)
            .unwrap();

        // Should see all 60 blocks for devnet epoch 1
        assert_eq!(result.total_blocks, DEVNET_BLOCKS_PER_EPOCH);
        assert_eq!(result.blocks_present, DEVNET_BLOCKS_PER_EPOCH);

        // Epoch 1 boundary check
        assert_eq!(result.epoch, 1);
    }

    #[test]
    fn test_mainnet_vs_devnet_epoch_isolation() {
        // Verify that mainnet (360 blocks) and devnet (60 blocks) are properly isolated
        let mut source = MockBlockSource::new();
        let params = ConsensusParams::mainnet();

        // Create blocks 0-59 (full devnet epoch 0, partial mainnet epoch 0)
        for height in 0..60 {
            let block = create_test_block(1, &[0], vec![1000]);
            source.add_block(height, block);
        }

        let producer = test_producer(1);

        // Devnet calculator should see all 60 blocks in epoch 0
        let devnet_calc = WeightedRewardCalculator::with_blocks_per_epoch(&source, &params, 60);
        let devnet_result = devnet_calc
            .calculate_producer_reward(&producer, 0, 0)
            .unwrap();
        assert_eq!(devnet_result.total_blocks, 60);
        assert_eq!(devnet_result.blocks_present, 60);

        // Mainnet calculator should only see 60 blocks (out of 360 in epoch 0)
        let mainnet_calc = WeightedRewardCalculator::with_blocks_per_epoch(&source, &params, 360);
        let mainnet_result = mainnet_calc
            .calculate_producer_reward(&producer, 0, 0)
            .unwrap();
        assert_eq!(mainnet_result.total_blocks, 60); // Only 60 blocks exist in storage
        assert_eq!(mainnet_result.blocks_present, 60);

        // Devnet epoch 0 reward should be 1/6 of mainnet epoch 0 potential reward
        // (60 blocks vs 360 blocks at same block reward rate)
        assert_eq!(devnet_result.reward_amount, mainnet_result.reward_amount);
    }
}
