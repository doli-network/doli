//! # Temporal Proof of Presence (TPoP) - Telemetry Module
//!
//! **IMPORTANT: This module is TELEMETRY ONLY and does NOT affect consensus.**
//!
//! Producer selection is determined by `consensus::select_producer_for_slot()` using
//! consecutive tickets based on bond count. Presence score does NOT affect:
//! - Block validity
//! - Producer selection
//! - Fallback eligibility
//!
//! This module provides metrics and tracking for network health monitoring.
//!
//! ## Overview
//!
//! The presence tracking system monitors producer activity for telemetry:
//! - Personal VDF chains prove temporal presence (informational)
//! - Presence score tracks commitment (metrics only)
//! - Records used for network health dashboards
//!
//! ## What This Module Does NOT Do
//!
//! - Does NOT affect which producer is selected for a slot
//! - Does NOT validate blocks based on presence
//! - Does NOT gate block production on presence threshold
//! - Does NOT influence reward distribution (bond-weighted)

use crate::network::Network;
use crate::network_params::NetworkParams;

mod rewards;
mod scoring;
mod selection;
#[cfg(test)]
mod tests;
mod types;
mod vdf_helpers;

pub use rewards::*;
pub use scoring::*;
pub use selection::*;
pub use types::*;
pub use vdf_helpers::*;

// =============================================================================
// CONSTANTS
// =============================================================================

/// VDF iterations for presence proof (~55 seconds on reference hardware)
/// This is calibrated to fit within a 60-second slot with margin for propagation
///
/// Note: This is TELEMETRY ONLY and does not affect consensus.
/// Not yet configurable via NetworkParams since it's informational.
pub const PRESENCE_VDF_ITERATIONS: u64 = 55_000_000;

/// Checkpoint interval in slots (1 hour)
/// Presence state is anchored on-chain at these intervals
pub const CHECKPOINT_INTERVAL: u64 = 60;

/// Maximum VDF chain length before requiring checkpoint
/// Prevents unbounded growth of presence proofs
pub const MAX_CHAIN_LENGTH: usize = 120;

/// Slots of inactivity before presence score starts decaying (mainnet default: 5)
///
/// **Deprecated**: Use `presence_grace_period_for_network(network)` for network-aware calculations.
#[deprecated(
    note = "Use presence_grace_period_for_network(network) for network-aware calculations"
)]
pub const PRESENCE_GRACE_PERIOD: u64 = 5;

/// Get presence grace period for a specific network
pub fn presence_grace_period_for_network(network: Network) -> u64 {
    // Use the general grace_period_secs from NetworkParams
    // For presence-specific behavior, this can be customized
    NetworkParams::load(network).grace_period_secs
}

/// Decay rate for presence score during inactivity (per slot, in basis points)
/// 10 = 0.1% decay per missed slot
pub const PRESENCE_DECAY_RATE_BPS: u64 = 10;

/// Maximum presence score (prevents overflow, ~4 years of perfect presence)
pub const MAX_PRESENCE_SCORE: u64 = 2_000_000;

/// Eligibility windows: (slot_offset_secs, max_rank)
/// Producers with rank < max_rank can produce after slot_offset_secs
pub const PRESENCE_WINDOWS: [(u64, usize); 4] = [
    (0, 1),    // 0-15s: Only #1 can produce
    (15, 3),   // 15-30s: Top 3 can produce
    (30, 10),  // 30-45s: Top 10 can produce
    (45, 100), // 45-60s: Top 100 can produce
];
