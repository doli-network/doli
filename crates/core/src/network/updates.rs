//! Auto-update system parameters for DOLI networks
//!
//! Veto periods, grace periods, voting age, crash detection, and update intervals.

use super::Network;

impl Network {
    /// Get veto period for software updates (in seconds)
    ///
    /// Configurable via `DOLI_VETO_PERIOD_SECS` environment variable.
    pub fn veto_period_secs(&self) -> u64 {
        self.params().veto_period_secs
    }

    /// Get grace period after update approval before enforcement (in seconds)
    ///
    /// After the veto period ends and an update is approved, producers have
    /// this grace period to apply the update before version enforcement begins.
    ///
    /// Configurable via `DOLI_GRACE_PERIOD_SECS` environment variable.
    pub fn grace_period_secs(&self) -> u64 {
        self.params().grace_period_secs
    }

    /// Get minimum producer age before voting is allowed (in seconds)
    ///
    /// Producers must be registered for at least this long before they can
    /// vote on updates. This prevents flash Sybil attacks where an attacker
    /// registers many producers just before a vote.
    ///
    /// Configurable via `DOLI_MIN_VOTING_AGE_SECS` environment variable.
    pub fn min_voting_age_secs(&self) -> u64 {
        self.params().min_voting_age_secs
    }

    /// Get minimum producer age for voting in blocks
    ///
    /// Converts min_voting_age_secs to blocks using slot_duration.
    pub fn min_voting_age_blocks(&self) -> u64 {
        self.params().min_voting_age_blocks()
    }

    /// Get interval between automatic update checks (in seconds)
    ///
    /// Configurable via `DOLI_UPDATE_CHECK_INTERVAL_SECS` environment variable.
    pub fn update_check_interval_secs(&self) -> u64 {
        self.params().update_check_interval_secs
    }

    /// Get crash detection window for automatic rollback (in seconds)
    ///
    /// If the node crashes multiple times within this window after an update,
    /// the watchdog will automatically rollback to the previous version.
    ///
    /// Configurable via `DOLI_CRASH_WINDOW_SECS` environment variable.
    pub fn crash_window_secs(&self) -> u64 {
        self.params().crash_window_secs
    }

    /// Get number of crashes within crash_window that trigger automatic rollback
    pub fn crash_threshold(&self) -> u32 {
        // Same for all networks - 3 crashes triggers rollback
        3
    }

    /// Veto period for software updates in blocks
    pub fn veto_period_blocks(&self) -> u64 {
        self.params().veto_period_blocks()
    }
}
