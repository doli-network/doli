//! Seniority-based weight calculation functions

use doli_core::network::Network;

use super::constants::*;
use super::types::ProducerInfo;

/// Calculate producer weight based on seniority (mainnet defaults)
///
/// **Deprecated**: Use `producer_weight_for_network()` for network-aware calculations.
///
/// Weight is determined by years active:
/// - 0-1 years: weight 1
/// - 1-2 years: weight 2
/// - 2-3 years: weight 3
/// - 3+ years: weight 4 (cap)
#[deprecated(
    note = "Use producer_weight_for_network(registered_at, current_height, network) for network-aware calculations"
)]
pub fn producer_weight(registered_at: u64, current_height: u64) -> u64 {
    // Use mainnet default for backwards compatibility
    #[allow(deprecated)]
    let slots_per_year = SLOTS_PER_YEAR;
    let slots_active = current_height.saturating_sub(registered_at);
    let years_active = slots_active / slots_per_year;

    // Discrete yearly steps: 0-1=1, 1-2=2, 2-3=3, 3+=4
    match years_active {
        0 => 1,
        1 => 2,
        2 => 3,
        _ => 4, // 3+ years
    }
}

/// Calculate producer weight with decimal precision (for proportional calculations)
///
/// Same as `producer_weight` but returns f64 for API compatibility.
/// Uses discrete yearly steps.
#[allow(deprecated)]
pub fn producer_weight_precise(registered_at: u64, current_height: u64) -> f64 {
    producer_weight(registered_at, current_height) as f64
}

// ==================== Network-Aware Weight Functions ====================

/// Calculate producer weight for a specific network
///
/// Uses network-specific blocks_per_year for time acceleration on devnet/testnet.
pub fn producer_weight_for_network(
    registered_at: u64,
    current_height: u64,
    network: Network,
) -> u64 {
    let blocks_active = current_height.saturating_sub(registered_at);
    let blocks_per_year = network.blocks_per_year();
    let years_active = blocks_active / blocks_per_year;

    // Discrete yearly steps: 0-1=1, 1-2=2, 2-3=3, 3+=4
    match years_active {
        0 => 1,
        1 => 2,
        2 => 3,
        _ => 4,
    }
}

/// Calculate producer weight with decimal precision for a specific network
pub fn producer_weight_precise_for_network(
    registered_at: u64,
    current_height: u64,
    network: Network,
) -> f64 {
    producer_weight_for_network(registered_at, current_height, network) as f64
}

/// Calculate total weight of all active producers for a specific network
pub fn total_weight_for_network(
    producers: &[ProducerInfo],
    current_height: u64,
    network: Network,
) -> u64 {
    producers
        .iter()
        .filter(|p| p.is_active())
        .map(|p| producer_weight_for_network(p.registered_at, current_height, network))
        .sum()
}

/// Calculate the weighted veto threshold for a specific network
///
/// Returns the minimum total weight required for a veto to pass (40%).
pub fn weighted_veto_threshold_for_network(
    producers: &[ProducerInfo],
    current_height: u64,
    network: Network,
) -> u64 {
    let total = total_weight_for_network(producers, current_height, network);
    // 40% threshold, rounded up to be conservative
    (total * VETO_THRESHOLD_PERCENT).div_ceil(100)
}

/// Calculate total weight of all active producers
///
/// # Arguments
/// - `producers`: Slice of producer information
/// - `current_height`: Current block height
///
/// # Returns
/// Sum of all active producers' weights
#[allow(deprecated)]
pub fn total_weight(producers: &[ProducerInfo], current_height: u64) -> u64 {
    producers
        .iter()
        .filter(|p| p.is_active())
        .map(|p| producer_weight(p.registered_at, current_height))
        .sum()
}

/// Calculate the weighted veto threshold
///
/// Returns the minimum total weight required for a veto to pass.
/// This is 40% of the total weight of all active producers.
///
/// # Arguments
/// - `producers`: Slice of producer information
/// - `current_height`: Current block height
///
/// # Returns
/// The weight threshold needed for veto (40% of total weight)
#[allow(deprecated)]
pub fn weighted_veto_threshold(producers: &[ProducerInfo], current_height: u64) -> u64 {
    let total = total_weight(producers, current_height);
    // 40% threshold, rounded up to be conservative
    (total * VETO_THRESHOLD_PERCENT).div_ceil(100)
}
