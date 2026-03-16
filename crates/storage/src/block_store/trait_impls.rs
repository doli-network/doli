//! Trait implementations for BlockStore

use doli_core::Block;

use super::types::BlockStore;

/// Implement BlockSource trait for BlockStore to enable weighted reward calculation.
///
/// This allows the WeightedRewardCalculator to access blocks by height for
/// calculating epoch rewards based on presence commitments.
impl doli_core::rewards::BlockSource for BlockStore {
    fn get_block_by_height(
        &self,
        height: doli_core::BlockHeight,
    ) -> Result<Option<Block>, doli_core::rewards::RewardError> {
        self.get_block_by_height(height)
            .map_err(|e| doli_core::rewards::RewardError::StorageError(e.to_string()))
    }
}
