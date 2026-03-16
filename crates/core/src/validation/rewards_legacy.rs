// rewards_legacy.rs — REMOVED
//
// This module previously contained the deprecated automatic epoch reward
// distribution system (validate_coinbase, calculate_expected_epoch_rewards,
// epoch_needing_rewards, validate_block_rewards, validate_block_rewards_exact).
//
// All functions were dead code. The active reward validation lives in the node
// layer at bins/node/src/node/validation_checks.rs (validate_block_economics)
// using the weighted presence reward model (crate::rewards::WeightedRewardCalculator).
//
// Removed: 2026-03-16
