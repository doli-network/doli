//! Existential Risk Simulations for DOLI
//!
//! These tests validate the core premise of DOLI:
//! "Verifiable sequential time can be a scarce resource
//!  without killing liveness or creating aristocracy"
//!
//! Three critical simulations:
//! 1. Onboarding stress test (liveness under viral growth)
//! 2. Elite simulation (power concentration over time)
//! 3. Slow infiltration attack (economic security)

#[path = "../../common/mod.rs"]
mod common;

mod aristocracy;
mod early_attacker;
mod economics;
mod infrastructure;
mod infiltration;
mod onboarding;
mod producer_lifecycle;

use doli_core::network::Network;
use doli_core::consensus::{
    RegistrationQueue, PendingRegistration, BASE_REGISTRATION_FEE,
    fee_multiplier_x100, MAX_FEE_MULTIPLIER_X100,
};
use storage::{
    ProducerInfo, ProducerSet,
    producer_weight_for_network,
    MAX_WEIGHT,
};
use crypto::{KeyPair, Hash};

// ============================================================================
// Simulation Parameters
// ============================================================================

/// Devnet: 1 block = 5 seconds, 12 blocks = 1 year
/// Note: For production code, use NetworkParams::load(network).blocks_per_year
const DEVNET_BLOCKS_PER_YEAR: u64 = 12;
const DEVNET_BLOCKS_PER_MONTH: u64 = 1;

/// Registration limits (simulation value)
/// Note: For production code, use NetworkParams::load(network).max_registrations_per_block
const MAX_REGISTRATIONS_PER_BLOCK: usize = 5;

/// Alert thresholds
mod thresholds {
    // Liveness
    pub const QUEUE_WAIT_ALERT: u64 = 10;  // blocks
    pub const QUEUE_WAIT_CRITICAL: u64 = 60;  // 1 hour in blocks at 1/min
    pub const FEE_MULTIPLIER_ALERT: f64 = 5.0;
    pub const FEE_MULTIPLIER_CRITICAL: f64 = 100.0;
    pub const ABANDONMENT_RATE_CRITICAL: f64 = 0.50;

    // Aristocracy
    pub const GINI_HEALTHY: f64 = 0.3;
    pub const GINI_CONCERNING: f64 = 0.5;
    pub const TOP5_HEALTHY: f64 = 0.20;
    pub const TOP5_CONCERNING: f64 = 0.35;
    pub const FOUNDERS_HEALTHY: f64 = 0.15;
    pub const FOUNDERS_CONCERNING: f64 = 0.25;

    // Security
    pub const ATTACK_COST_CRITICAL: u64 = 500_000;  // $500K
    pub const ATTACK_COST_RISKY: u64 = 2_000_000;   // $2M
    pub const ATTACK_COST_ACCEPTABLE: u64 = 10_000_000;  // $10M
}

// ============================================================================
// Shared Types
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
enum AlertLevel {
    Green,
    Yellow,
    Red,
}

// ============================================================================
// Shared Helpers
// ============================================================================

fn calculate_gini(values: &[u64]) -> f64 {
    let n = values.len();
    if n == 0 {
        return 0.0;
    }

    let mut sorted: Vec<u64> = values.to_vec();
    sorted.sort();

    let sum: u64 = sorted.iter().sum();
    if sum == 0 {
        return 0.0;
    }

    let mut cumulative = 0u64;
    let mut gini_sum = 0.0;

    for (i, &value) in sorted.iter().enumerate() {
        cumulative += value;
        let expected = (i + 1) as f64 / n as f64;
        let actual = cumulative as f64 / sum as f64;
        gini_sum += expected - actual;
    }

    2.0 * gini_sum / n as f64
}

fn calculate_fee_multiplier(queue_length: usize) -> f64 {
    // Use the deterministic table-based multiplier from consensus (capped at 10x)
    fee_multiplier_x100(queue_length as u32) as f64 / 100.0
}
