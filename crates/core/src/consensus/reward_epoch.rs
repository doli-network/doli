//! Block-height based reward epoch utilities.
//!
//! Unlike slot-based epochs, block-height epochs are sequential with no gaps,
//! making reward calculation simpler and deterministic.
//!
//! # Examples
//!
//! ```
//! use doli_core::consensus::reward_epoch;
//!
//! // Epoch 0: blocks 0-359
//! assert_eq!(reward_epoch::from_height(0), 0);
//! assert_eq!(reward_epoch::from_height(359), 0);
//!
//! // Epoch 1: blocks 360-719
//! assert_eq!(reward_epoch::from_height(360), 1);
//!
//! // Get epoch boundaries
//! let (start, end) = reward_epoch::boundaries(5);
//! assert_eq!(start, 1800);
//! assert_eq!(end, 2160);
//! ```

use super::constants::BLOCKS_PER_REWARD_EPOCH;
use crate::types::BlockHeight;

/// Get reward epoch number from block height.
///
/// Simple division: `height / BLOCKS_PER_REWARD_EPOCH`
///
/// # Examples
///
/// With `BLOCKS_PER_REWARD_EPOCH = 360`:
/// - Height 0 → Epoch 0
/// - Height 359 → Epoch 0
/// - Height 360 → Epoch 1
/// - Height 1000 → Epoch 2
#[inline]
pub fn from_height(height: BlockHeight) -> u64 {
    height / BLOCKS_PER_REWARD_EPOCH
}

/// Get (start_height, end_height) for a reward epoch.
///
/// Note: `end_height` is exclusive (the range is `start..end`).
///
/// # Examples
///
/// ```
/// use doli_core::consensus::reward_epoch;
///
/// let (start, end) = reward_epoch::boundaries(0);
/// assert_eq!(start, 0);
/// assert_eq!(end, 360);
///
/// let (start, end) = reward_epoch::boundaries(5);
/// assert_eq!(start, 1800);
/// assert_eq!(end, 2160);
/// ```
#[inline]
pub fn boundaries(epoch: u64) -> (BlockHeight, BlockHeight) {
    let start = epoch * BLOCKS_PER_REWARD_EPOCH;
    let end = start + BLOCKS_PER_REWARD_EPOCH;
    (start, end)
}

/// Check if a reward epoch is complete given the current block height.
///
/// An epoch is complete when the current height is at or beyond the
/// epoch's end boundary.
///
/// # Examples
///
/// ```
/// use doli_core::consensus::reward_epoch;
///
/// // Epoch 0 ends at block 360
/// assert!(!reward_epoch::is_complete(0, 359));
/// assert!(reward_epoch::is_complete(0, 360));
/// assert!(reward_epoch::is_complete(0, 1000));
/// ```
#[inline]
pub fn is_complete(epoch: u64, current_height: BlockHeight) -> bool {
    let (_, end) = boundaries(epoch);
    current_height >= end
}

/// Get the current reward epoch from block height.
///
/// This is an alias for `from_height` for clarity in contexts
/// where we're interested in the "current" epoch.
#[inline]
pub fn current(height: BlockHeight) -> u64 {
    from_height(height)
}

/// Get the last complete reward epoch from block height.
///
/// Returns `None` if no epoch has been completed yet (height < BLOCKS_PER_REWARD_EPOCH).
///
/// # Examples
///
/// ```
/// use doli_core::consensus::reward_epoch;
///
/// // No complete epochs yet
/// assert_eq!(reward_epoch::last_complete(0), None);
/// assert_eq!(reward_epoch::last_complete(359), None);
///
/// // First epoch just completed
/// assert_eq!(reward_epoch::last_complete(360), Some(0));
/// assert_eq!(reward_epoch::last_complete(719), Some(0));
///
/// // Two epochs completed
/// assert_eq!(reward_epoch::last_complete(720), Some(1));
/// ```
#[inline]
pub fn last_complete(height: BlockHeight) -> Option<u64> {
    let current_epoch = from_height(height);
    if current_epoch > 0 {
        Some(current_epoch - 1)
    } else {
        None
    }
}

/// Check if height is the first block of a reward epoch.
///
/// Useful for detecting epoch boundaries in block processing.
#[inline]
pub fn is_epoch_start(height: BlockHeight) -> bool {
    height.is_multiple_of(BLOCKS_PER_REWARD_EPOCH)
}

/// Get the number of blocks per reward epoch.
///
/// Returns the constant `BLOCKS_PER_REWARD_EPOCH`.
#[inline]
pub fn blocks_per_epoch() -> BlockHeight {
    BLOCKS_PER_REWARD_EPOCH
}

/// Calculate how many complete epochs exist up to a given height.
///
/// This is the same as `from_height` for heights >= BLOCKS_PER_REWARD_EPOCH.
/// For heights < BLOCKS_PER_REWARD_EPOCH, returns 0.
#[inline]
pub fn complete_epochs(height: BlockHeight) -> u64 {
    if height < BLOCKS_PER_REWARD_EPOCH {
        0
    } else {
        from_height(height)
    }
}

// ========================================================================
// Network-aware versions (_with suffix)
// These functions accept blocks_per_epoch as a parameter to support
// different networks (mainnet=360, testnet=360, devnet=60).
// ========================================================================

/// Get reward epoch number from block height (network-aware version).
///
/// Use this when you have access to `Network::blocks_per_reward_epoch()`.
#[inline]
pub fn from_height_with(height: BlockHeight, blocks_per_epoch: u64) -> u64 {
    height / blocks_per_epoch
}

/// Get (start_height, end_height) for a reward epoch (network-aware version).
///
/// Note: `end_height` is exclusive (the range is `start..end`).
#[inline]
pub fn boundaries_with(epoch: u64, blocks_per_epoch: u64) -> (BlockHeight, BlockHeight) {
    let start = epoch * blocks_per_epoch;
    let end = start + blocks_per_epoch;
    (start, end)
}

/// Check if a reward epoch is complete (network-aware version).
#[inline]
pub fn is_complete_with(epoch: u64, current_height: BlockHeight, blocks_per_epoch: u64) -> bool {
    let (_, end) = boundaries_with(epoch, blocks_per_epoch);
    current_height >= end
}

/// Get last complete reward epoch (network-aware version).
#[inline]
pub fn last_complete_with(height: BlockHeight, blocks_per_epoch: u64) -> Option<u64> {
    let current_epoch = from_height_with(height, blocks_per_epoch);
    if current_epoch > 0 {
        Some(current_epoch - 1)
    } else {
        None
    }
}

/// Check if height is first block of a reward epoch (network-aware version).
#[inline]
pub fn is_epoch_start_with(height: BlockHeight, blocks_per_epoch: u64) -> bool {
    height.is_multiple_of(blocks_per_epoch)
}

/// Calculate complete epochs up to height (network-aware version).
#[inline]
pub fn complete_epochs_with(height: BlockHeight, blocks_per_epoch: u64) -> u64 {
    if height < blocks_per_epoch {
        0
    } else {
        from_height_with(height, blocks_per_epoch)
    }
}
