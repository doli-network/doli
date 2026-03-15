use crate::types::Amount;

use super::constants::*;
use super::exit::RewardMode;
use super::params::ConsensusParams;

// ==================== Stress Test Parameters ====================

/// Parameters for extreme stress testing scenarios
#[derive(Clone, Debug)]
pub struct StressTestParams {
    /// Number of simulated producers
    pub producer_count: u32,
    /// Slot duration in seconds (can be very short for stress tests)
    pub slot_duration_secs: u64,
    /// VDF iterations (reduced for stress tests)
    pub vdf_iterations: u64,
    /// Bond amount per producer
    pub bond_per_producer: Amount,
    /// Block reward
    pub block_reward: Amount,
}

impl StressTestParams {
    /// Create stress test parameters for 600 producers (extreme scenario)
    ///
    /// Based on whitepaper calculations:
    /// - Each producer has 1/600 probability per slot
    /// - Expected blocks per producer per hour: 1.2
    /// - VDF reduced to 100ms for rapid testing
    pub fn extreme_600() -> Self {
        Self {
            producer_count: 600,
            slot_duration_secs: 1,          // 1 second slots for extreme stress
            vdf_iterations: 100_000,        // ~100ms VDF
            bond_per_producer: 100_000_000, // 1 DOLI
            block_reward: 50_000_000_000,   // 500 DOLI
        }
    }

    /// Create stress test parameters for N producers
    pub fn with_producers(count: u32) -> Self {
        Self {
            producer_count: count,
            slot_duration_secs: 2,          // 2 second slots
            vdf_iterations: 500_000,        // ~500ms VDF
            bond_per_producer: 100_000_000, // 1 DOLI
            block_reward: 50_000_000_000,   // 500 DOLI
        }
    }

    /// Calculate expected blocks per producer per hour
    pub fn expected_blocks_per_producer_per_hour(&self) -> f64 {
        let slots_per_hour = 3600.0 / self.slot_duration_secs as f64;
        slots_per_hour / self.producer_count as f64
    }

    /// Calculate expected reward per producer per hour
    pub fn expected_reward_per_producer_per_hour(&self) -> Amount {
        let blocks = self.expected_blocks_per_producer_per_hour();
        (blocks * self.block_reward as f64) as Amount
    }

    /// Calculate total bond locked with all producers
    pub fn total_bond_locked(&self) -> Amount {
        self.bond_per_producer * self.producer_count as u64
    }

    /// Calculate time between expected block production for one producer
    pub fn expected_time_between_blocks_secs(&self) -> f64 {
        self.slot_duration_secs as f64 * self.producer_count as f64
    }

    /// Calculate network efficiency (useful VDF vs total VDF)
    pub fn network_efficiency(&self) -> f64 {
        1.0 / self.producer_count as f64
    }

    /// Calculate slots needed for 51% attack
    pub fn slots_for_majority_attack(&self) -> u32 {
        (self.producer_count / 2) + 1
    }

    /// Generate a summary report
    pub fn summary(&self) -> String {
        format!(
            r#"
=== DOLI Stress Test: {} Producers ===

Timing:
  Slot duration: {}s
  VDF iterations: {} (~{}ms estimated)
  Expected time between own blocks: {:.1}s ({:.1} min)

Economics:
  Bond per producer: {} DOLI
  Total bond locked: {} DOLI
  Block reward: {} DOLI
  Expected reward/producer/hour: {:.2} DOLI

Probabilities:
  Probability of producing per slot: {:.4}%
  Expected blocks/producer/hour: {:.2}
  Network efficiency (useful VDF): {:.4}%

Security:
  Producers needed for 51% attack: {}
  Bond needed for attack: {} DOLI
"#,
            self.producer_count,
            self.slot_duration_secs,
            self.vdf_iterations,
            self.vdf_iterations / 1_000_000,
            self.expected_time_between_blocks_secs(),
            self.expected_time_between_blocks_secs() / 60.0,
            self.bond_per_producer / 100_000_000,
            self.total_bond_locked() / 100_000_000,
            self.block_reward / 100_000_000,
            self.expected_reward_per_producer_per_hour() as f64 / 100_000_000.0,
            100.0 / self.producer_count as f64,
            self.expected_blocks_per_producer_per_hour(),
            self.network_efficiency() * 100.0,
            self.slots_for_majority_attack(),
            (self.slots_for_majority_attack() as u64 * self.bond_per_producer) / 100_000_000,
        )
    }
}

impl ConsensusParams {
    /// Create consensus params optimized for stress testing with many producers
    ///
    /// Scales fallback windows proportionally to slot duration
    pub fn for_stress_test(stress: &StressTestParams) -> Self {
        Self {
            genesis_time: 0, // Dynamic, set at runtime
            slot_duration: stress.slot_duration_secs,
            slots_per_epoch: 60,
            slots_per_reward_epoch: 60, // Short reward epoch for stress tests
            attestation_interval: 5,    // Frequent attestations for stress tests
            min_attestation_rate: MIN_ATTESTATION_RATE,
            blocks_per_era: 10_000, // Faster era transitions for testing
            bootstrap_blocks: 10,   // Very short bootstrap for stress tests
            bootstrap_grace_period_secs: 2, // Minimal grace for stress tests
            initial_reward: stress.block_reward,
            initial_bond: stress.bond_per_producer,
            base_block_size: BASE_BLOCK_SIZE,
            max_block_size_cap: MAX_BLOCK_SIZE_CAP,
            reward_mode: RewardMode::EpochPool,
            genesis_hash: crypto::Hash::ZERO, // Stress tests don't validate genesis_hash
        }
    }
}
