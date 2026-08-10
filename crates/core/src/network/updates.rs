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

    /// Crash-detection window, in seconds, for the UNWIRED update watchdog.
    ///
    /// NOT A LIVE CONTROL (INC-I-172 M1, AUDIT-P1-014). This value is only read by
    /// `updater::watchdog::UpdateWatchdog`, which has zero production callers, so nothing
    /// on a running node counts crashes and nothing rolls back automatically. Changing it
    /// changes no behaviour. See that module's header before citing it anywhere.
    ///
    /// Configurable via `DOLI_CRASH_WINDOW_SECS` environment variable.
    pub fn crash_window_secs(&self) -> u64 {
        self.params().crash_window_secs
    }

    /// Crashes within `crash_window_secs` that WOULD trigger a rollback.
    ///
    /// NOT A LIVE CONTROL — same reason as [`Network::crash_window_secs`].
    pub fn crash_threshold(&self) -> u32 {
        // Same for all networks - 3 crashes would trigger rollback, if the watchdog
        // were wired.
        3
    }

    /// Veto period for software updates in blocks
    pub fn veto_period_blocks(&self) -> u64 {
        self.params().veto_period_blocks()
    }
}
