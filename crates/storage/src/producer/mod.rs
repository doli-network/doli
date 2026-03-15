//! Producer state management
//!
//! Tracks the state of block producers including their bond status,
//! registration height, exit/cooldown state, and seniority-based weight.
//!
//! ## Anti-Sybil: Weight by Seniority (Discrete Steps)
//!
//! A producer's power increases in discrete yearly steps based on their
//! time active in the network. This prevents whale attacks where an
//! attacker could instantly acquire significant influence.
//!
//! Weight by years active:
//! - 0-1 years: weight 1
//! - 1-2 years: weight 2
//! - 2-3 years: weight 3
//! - 3+ years: weight 4 (cap)
//!
//! This uses SLOTS_PER_YEAR (3,153,600 slots at 10s per slot) for
//! time calculation.
//!
//! Weight affects:
//! - Reward distribution (proportional to weight)
//! - Veto power (40% of total weight, not count)
//! - Producer selection probability (proportional to weight)

mod constants;
mod info;
mod seniority;
mod set_core;
mod set_delegation;
mod set_governance;
mod set_lifecycle;
mod set_persistence;
mod set_registration;
#[cfg(test)]
#[allow(deprecated)]
mod tests;
mod types;

// Re-export everything for identical public API
#[allow(deprecated)]
pub use constants::{
    blocks_per_year_for_network, bond_unit_for_network, inactivity_threshold_for_network,
    ACTIVATION_DELAY, BLOCKS_PER_MONTH, BLOCKS_PER_YEAR, BOND_UNIT, EXIT_HISTORY_RETENTION,
    INACTIVITY_THRESHOLD, MAX_WEIGHT, MIN_WEIGHT, REACTIVATION_THRESHOLD, SLOTS_PER_YEAR,
    VETO_BOND_AMOUNT, VETO_THRESHOLD_PERCENT,
};
pub use info::calculate_withdrawal_from_bonds;
#[allow(deprecated)]
pub use seniority::{
    producer_weight, producer_weight_for_network, producer_weight_precise,
    producer_weight_precise_for_network, total_weight, total_weight_for_network,
    weighted_veto_threshold, weighted_veto_threshold_for_network,
};
pub use types::{
    ActivityStatus, PendingProducerUpdate, ProducerInfo, ProducerSet, ProducerStatus,
    StoredBondEntry,
};
