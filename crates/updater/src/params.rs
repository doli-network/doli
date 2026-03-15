//! Network-aware update parameters

use doli_core::network::Network;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::types::Release;
use crate::util::current_timestamp;

// ============================================================================
// Network-Aware Parameters
// ============================================================================

/// Update system parameters derived from network configuration
///
/// Use this struct to get network-specific timing parameters instead of
/// the global constants. This enables accelerated testing on devnet.
///
/// # Example
///
/// ```rust
/// use updater::UpdateParams;
/// use doli_core::network::Network;
///
/// let params = UpdateParams::for_network(Network::Devnet);
/// assert_eq!(params.veto_period_secs, 60); // 1 minute on devnet
///
/// let params = UpdateParams::for_network(Network::Mainnet);
/// assert_eq!(params.veto_period_secs, 5 * 60); // 5 minutes (early network)
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateParams {
    /// Veto period in seconds
    pub veto_period_secs: u64,
    /// Grace period after approval in seconds
    pub grace_period_secs: u64,
    /// Minimum producer age before voting is allowed (seconds)
    pub min_voting_age_secs: u64,
    /// Minimum producer age before voting is allowed (blocks)
    pub min_voting_age_blocks: u64,
    /// Update check interval in seconds
    pub check_interval_secs: u64,
    /// Crash detection window in seconds
    pub crash_window_secs: u64,
    /// Number of crashes within window that triggers rollback
    pub crash_threshold: u32,
    /// Blocks needed to reach full seniority (4x vote weight)
    pub seniority_maturity_blocks: u64,
    /// Blocks per seniority step (1 year equivalent)
    pub seniority_step_blocks: u64,
    /// Network identifier
    pub network: Network,
}

impl UpdateParams {
    /// Create update parameters for a specific network
    pub fn for_network(network: Network) -> Self {
        Self {
            veto_period_secs: network.veto_period_secs(),
            grace_period_secs: network.grace_period_secs(),
            min_voting_age_secs: network.min_voting_age_secs(),
            min_voting_age_blocks: network.min_voting_age_blocks(),
            check_interval_secs: network.update_check_interval_secs(),
            crash_window_secs: network.crash_window_secs(),
            crash_threshold: network.crash_threshold(),
            seniority_maturity_blocks: network.seniority_maturity_blocks(),
            seniority_step_blocks: network.seniority_step_blocks(),
            network,
        }
    }

    /// Get veto period as Duration
    pub fn veto_period(&self) -> Duration {
        Duration::from_secs(self.veto_period_secs)
    }

    /// Get grace period as Duration
    pub fn grace_period(&self) -> Duration {
        Duration::from_secs(self.grace_period_secs)
    }

    /// Get check interval as Duration
    pub fn check_interval(&self) -> Duration {
        Duration::from_secs(self.check_interval_secs)
    }

    /// Get crash window as Duration
    pub fn crash_window(&self) -> Duration {
        Duration::from_secs(self.crash_window_secs)
    }

    /// Calculate vote weight based on bonds staked and producer seniority.
    ///
    /// weight = bond_count × seniority_multiplier
    /// seniority_multiplier = 1.0 + min(years, 4) × 0.75
    ///
    /// This balances economic stake (bonds) with time commitment (seniority).
    /// A whale with 100 bonds registered yesterday gets 100 × 1.0 = 100,
    /// while 25 veterans at 4 years get 25 × 4.0 = 100 — equal power.
    pub fn calculate_vote_weight(&self, bond_count: u32, blocks_active: u64) -> f64 {
        let years = blocks_active as f64 / self.seniority_step_blocks as f64;
        let capped_years = years.min(4.0);
        let seniority_multiplier = 1.0 + capped_years * 0.75;
        bond_count as f64 * seniority_multiplier
    }

    /// Calculate seniority multiplier only (for display purposes).
    pub fn seniority_multiplier(&self, blocks_active: u64) -> f64 {
        let years = blocks_active as f64 / self.seniority_step_blocks as f64;
        let capped_years = years.min(4.0);
        1.0 + capped_years * 0.75
    }

    /// Check if a producer is old enough to vote
    pub fn is_eligible_to_vote(&self, blocks_since_registration: u64) -> bool {
        blocks_since_registration >= self.min_voting_age_blocks
    }

    /// Get veto deadline for a release
    pub fn veto_deadline(&self, release: &Release) -> u64 {
        release.published_at + self.veto_period_secs
    }

    /// Get grace period deadline (when enforcement begins)
    pub fn grace_period_deadline(&self, release: &Release) -> u64 {
        self.veto_deadline(release) + self.grace_period_secs
    }

    /// Check if veto period has ended for a release
    pub fn veto_period_ended(&self, release: &Release) -> bool {
        current_timestamp() >= self.veto_deadline(release)
    }

    /// Check if we're in the grace period (after approval, before enforcement)
    pub fn in_grace_period(&self, release: &Release) -> bool {
        let now = current_timestamp();
        let veto_end = self.veto_deadline(release);
        let grace_end = self.grace_period_deadline(release);
        now >= veto_end && now < grace_end
    }
}

impl Default for UpdateParams {
    /// Default parameters use mainnet timing
    fn default() -> Self {
        Self::for_network(Network::Mainnet)
    }
}
