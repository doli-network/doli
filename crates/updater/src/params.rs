//! Network-aware update parameters

use doli_core::network::Network;
use serde::{Deserialize, Serialize};
use std::time::Duration;

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

    /// Get veto deadline, measured from the node-local `first_notified_at` (F7(b)).
    /// `saturating_add` (AUDIT-P2-013): `first_notified_at` comes from unauthenticated
    /// node-local JSON, and a wrapping add would place the deadline in the PAST.
    pub fn veto_deadline(&self, first_notified_at: u64) -> u64 {
        first_notified_at.saturating_add(self.veto_period_secs)
    }

    /// Get grace period deadline (when enforcement begins)
    pub fn grace_period_deadline(&self, first_notified_at: u64) -> u64 {
        self.veto_deadline(first_notified_at)
            .saturating_add(self.grace_period_secs)
    }

    /// Check if the veto period has ended, relative to when this node first saw the release.
    pub fn veto_period_ended(&self, first_notified_at: u64) -> bool {
        current_timestamp() >= self.veto_deadline(first_notified_at)
    }

    /// Check if we're in the grace period (after approval, before enforcement)
    pub fn in_grace_period(&self, first_notified_at: u64) -> bool {
        let now = current_timestamp();
        let veto_end = self.veto_deadline(first_notified_at);
        let grace_end = self.grace_period_deadline(first_notified_at);
        now >= veto_end && now < grace_end
    }
}

impl Default for UpdateParams {
    /// Default parameters use mainnet timing
    fn default() -> Self {
        Self::for_network(Network::Mainnet)
    }
}
