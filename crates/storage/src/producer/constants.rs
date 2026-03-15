//! Constants for producer state management

use doli_core::network::Network;
use doli_core::network_params::NetworkParams;

// ==================== Seniority Weight System ====================

/// Slots per year (mainnet default: 3,153,600 at 10s slots)
///
/// **Deprecated**: Use `NetworkParams::load(network).blocks_per_year` instead.
/// Devnet uses 144 blocks/year for accelerated testing.
#[deprecated(
    note = "Use NetworkParams::load(network).blocks_per_year for network-aware calculations"
)]
pub const SLOTS_PER_YEAR: u64 = 3_153_600;

/// Blocks per month - legacy alias (mainnet default)
///
/// **Deprecated**: Use `NetworkParams::load(network).blocks_per_year / 12` instead.
#[deprecated(
    note = "Use NetworkParams::load(network).blocks_per_year / 12 for network-aware calculations"
)]
pub const BLOCKS_PER_MONTH: u64 = 3_153_600 / 12;

/// Blocks per year - legacy alias (mainnet default)
///
/// **Deprecated**: Use `NetworkParams::load(network).blocks_per_year` instead.
/// Devnet uses 144 blocks/year for accelerated testing.
#[deprecated(
    note = "Use NetworkParams::load(network).blocks_per_year for network-aware calculations"
)]
pub const BLOCKS_PER_YEAR: u64 = 3_153_600;

/// Get blocks per year for a specific network
pub fn blocks_per_year_for_network(network: Network) -> u64 {
    NetworkParams::load(network).blocks_per_year
}

/// Get inactivity threshold for a specific network
pub fn inactivity_threshold_for_network(network: Network) -> u64 {
    NetworkParams::load(network).inactivity_threshold
}

/// Maximum weight a producer can achieve
pub const MAX_WEIGHT: u64 = 4;

/// Minimum weight for any producer
pub const MIN_WEIGHT: u64 = 1;

/// Veto threshold percentage (40% of effective weight required to block)
///
/// Why 40% instead of 33%:
/// - 33% allows a $44K early attacker to block governance for 4 years
/// - 40% raises the bar: requires more nodes or longer sustained attack
/// - Combined with activity penalty, makes "register and wait" attacks expensive
pub const VETO_THRESHOLD_PERCENT: u64 = 40;

/// Veto bond amount (in smallest units) required to vote BLOCK
///
/// This is a temporary bond locked when voting to block a proposal.
/// The bond is returned after the vote concludes (regardless of outcome).
/// This adds friction to frivolous or attack-driven vetoes.
///
/// 10 DOLI = 1_000_000_000 units (same as registration fee)
pub const VETO_BOND_AMOUNT: u64 = 1_000_000_000;

/// Activation delay in blocks before a new producer can participate in scheduling.
///
/// When a producer registers, they must wait this many blocks before they become
/// eligible for block production. This ensures all nodes have time to see the
/// registration transaction and update their producer sets, preventing scheduling
/// conflicts where some nodes see the new producer and others don't.
///
/// With 10-second slots:
/// - 10 blocks = ~100 seconds of propagation time
/// - All nodes will have received and confirmed the registration block
/// - ProducerSet becomes consistent across the network
pub const ACTIVATION_DELAY: u64 = 10;

/// Inactivity threshold (mainnet default): ~1 week without producing triggers inactive status
/// At 10s slots (6 blocks/minute): 7 days × 24 hours × 360 blocks/hour = 60,480 blocks
///
/// **Deprecated**: Use `inactivity_threshold_for_network(network)` for network-aware calculations.
/// Devnet uses 30 blocks for faster testing.
#[deprecated(note = "Use inactivity_threshold_for_network(network) for network-aware calculations")]
pub const INACTIVITY_THRESHOLD: u64 = 60_480;

/// Reactivation threshold (mainnet default): ~1 day of continuous activity to regain Active status
/// At 10s slots (6 blocks/minute): 1 day × 24 hours × 360 blocks/hour = 8,640 blocks
pub const REACTIVATION_THRESHOLD: u64 = 8_640;

/// Bond unit constant for mainnet/testnet: 1 bond = 100 DOLI = 10,000,000,000 base units
/// Note: For devnet, use Network::initial_bond() which returns 100_000_000 (1 DOLI per bond)
/// IMPORTANT: For production code, prefer using `NetworkParams::bond_unit()` instead.
pub const BOND_UNIT: u64 = 10_000_000_000;

/// Get the bond unit for a specific network
/// - Mainnet/Testnet: 100_000_000_000 (1000 DOLI per bond)
/// - Devnet: 100_000_000 (1 DOLI per bond)
pub fn bond_unit_for_network(network: Network) -> u64 {
    network.initial_bond()
}

/// Exit history retention period: 8 years (2 eras)
/// After this period, a producer can re-register without the prior_exit penalty.
pub const EXIT_HISTORY_RETENTION: u64 = 2 * 2_102_400; // 2 eras = ~8 years
