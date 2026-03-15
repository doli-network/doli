//! Consensus parameters and rules
//!
//! # Parameter Architecture
//!
//! This module contains **mainnet default constants** (the "DNA" of the protocol).
//! These values serve as the base defaults used by [`crate::network_params::NetworkParams`].
//!
//! **For network-aware code, use `NetworkParams` instead of importing constants directly:**
//!
//! ```ignore
//! // PREFERRED: Network-aware
//! let params = NetworkParams::load(network);
//! let bond_unit = params.bond_unit;  // 10 DOLI mainnet, 1 DOLI devnet
//!
//! // AVOID: Direct import (hardcoded mainnet values)
//! use doli_core::consensus::BOND_UNIT;  // Always 10 DOLI
//! ```
//!
//! Constants here are locked for mainnet but can be overridden via `.env` for devnet.
//! See [`crate::network_params`] for the configuration manager.
//!
//! # Proof of Time (PoT)
//!
//! DOLI uses Proof of Time consensus with VDF-based anti-grinding. Unlike
//! Proof of Work (parallelizable computation) or Proof of Stake (capital-based
//! selection), PoT uses **time as the scarce resource**.
//!
//! ## Core Principle
//!
//! Time is proven via VDF (Verifiable Delay Function):
//! - One producer per slot (10 seconds)
//! - Selection uses consecutive tickets based on bond count
//! - VDF computation (~7s) proves sequential work
//! - Producer receives 100% of block reward via coinbase
//!
//! ## Selection System
//!
//! Producer selection is deterministic and independent of previous block hash:
//! - Total tickets = sum of all producer bond counts
//! - Primary producer: slot % total_tickets
//! - Fallback producers at +33% and +50% ticket offsets
//!
//! This prevents grinding attacks - changing the block hash cannot influence
//! who gets selected for future slots.
//!
//! ## Why VDF for Blocks?
//!
//! VDF provides anti-grinding protection:
//! - VDF input: HASH(prev_hash || tx_root || slot || producer_key)
//! - ~700ms of sequential computation
//! - Mandatory for mainnet blocks (vdf_output must be present)
//! - Proof is self-verifying (hash-chain VDF)
//!
//! ## Slot Timing (10 seconds)
//!
//! ```text
//! 0-3s:    Primary producer window (rank 0 only)
//! 3-6s:    Secondary fallback window (rank 0-1)
//! 6-10s:   Tertiary fallback window (rank 0-2)
//! ```

mod bonds;
mod constants;
mod exit;
mod params;
mod producer_state;
mod registration;
pub mod reward_epoch;
mod selection;
mod stress;
#[cfg(test)]
mod tests;
mod tiers;
mod vdf;

// Re-export everything
pub use bonds::{BondEntry, BondError, BondsMaturitySummary, ProducerBonds, WithdrawalResult};
pub use constants::*;
pub use exit::{
    calculate_exit, calculate_exit_with_quarter, calculate_slash, ExitTerms, PenaltyDestination,
    RewardMode, SlashResult,
};
pub use params::ConsensusParams;
pub use producer_state::ProducerState;
pub use registration::{
    fee_multiplier_x100, registration_fee, registration_fee_for_network, PendingRegistration,
    RegistrationQueue, BASE_REGISTRATION_FEE, MAX_FEE_MULTIPLIER_X100, MAX_REGISTRATIONS_PER_BLOCK,
    MAX_REGISTRATION_FEE,
};
#[allow(deprecated)]
pub use selection::{
    allowed_producer_rank, allowed_producer_rank_ms, allowed_producer_rank_scaled,
    eligible_rank_at_ms, get_producer_rank, is_producer_eligible, is_producer_eligible_ms,
    is_rank_eligible_at_ms, is_rank_eligible_at_offset, scaled_fallback_windows,
    select_producer_for_slot,
};
pub use stress::StressTestParams;
pub use tiers::{compute_tier1_set, producer_region, producer_tier};
pub use vdf::*;
