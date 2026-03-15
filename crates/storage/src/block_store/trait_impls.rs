//! Trait implementations for BlockStore

use doli_core::Block;

use super::types::BlockStore;

/// Implement EpochBlockSource trait for BlockStore to enable deterministic
/// epoch reward validation in the core crate.
impl doli_core::validation::EpochBlockSource for BlockStore {
    fn last_rewarded_epoch(&self) -> Result<u64, String> {
        self.get_last_rewarded_epoch()
            .map_err(|e| format!("storage error: {}", e))
    }

    fn blocks_in_slot_range(&self, start: u32, end: u32) -> Result<Vec<Block>, String> {
        self.get_blocks_in_slot_range(start, end)
            .map_err(|e| format!("storage error: {}", e))
    }

    fn has_any_block_in_slot_range(&self, start: u32, end: u32) -> Result<bool, String> {
        self.has_any_block_in_slot_range(start, end)
            .map_err(|e| format!("storage error: {}", e))
    }
}

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
