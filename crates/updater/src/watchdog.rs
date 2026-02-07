//! Update watchdog — detects post-update crashes and auto-rolls back.
//!
//! After an update is applied, the watchdog monitors for crashes within
//! a configurable window. If crash_threshold crashes occur within
//! crash_window_secs, the watchdog triggers an automatic rollback.
//!
//! At 150K nodes, this works because each node independently monitors
//! its own crash history — no coordination needed.

use doli_core::network::Network;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Persisted watchdog state (JSON file at {data_dir}/watchdog_state.json)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WatchdogState {
    /// Version that was last applied via update
    pub last_update_version: Option<String>,
    /// Unix timestamp when the update was applied
    pub last_update_time: Option<u64>,
    /// Timestamps of recent crashes (after an update)
    pub crash_timestamps: Vec<u64>,
    /// Whether clean shutdown was performed (reset on each startup)
    pub clean_shutdown: bool,
}

impl WatchdogState {
    fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("watchdog_state.json")
    }

    /// Load state from disk, or return default if not found.
    pub fn load(data_dir: &Path) -> Self {
        let path = Self::path(data_dir);
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save state to disk.
    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        let path = Self::path(data_dir);
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)
    }
}

/// Default crash threshold — 3 crashes within the window triggers rollback.
const DEFAULT_CRASH_THRESHOLD: u32 = 3;

/// The watchdog monitors for post-update crashes and triggers rollback.
pub struct UpdateWatchdog {
    data_dir: PathBuf,
    crash_window_secs: u64,
    crash_threshold: u32,
}

impl UpdateWatchdog {
    /// Create a new watchdog for the given network.
    pub fn new(data_dir: PathBuf, network: Network) -> Self {
        Self {
            data_dir,
            crash_window_secs: network.crash_window_secs(),
            crash_threshold: DEFAULT_CRASH_THRESHOLD,
        }
    }

    /// Record that an update was applied (call before restart).
    pub fn record_update(&self, version: &str) {
        let mut state = WatchdogState::load(&self.data_dir);
        state.last_update_version = Some(version.to_string());
        state.last_update_time = Some(crate::current_timestamp());
        state.crash_timestamps.clear();
        state.clean_shutdown = false;
        let _ = state.save(&self.data_dir);
        info!("Watchdog: recorded update to v{}", version);
    }

    /// Record a clean shutdown (call on graceful exit).
    pub fn record_clean_shutdown(&self) {
        let mut state = WatchdogState::load(&self.data_dir);
        state.clean_shutdown = true;
        let _ = state.save(&self.data_dir);
    }

    /// Check on startup whether we need to rollback.
    ///
    /// Returns `Some(version)` if rollback should be triggered,
    /// where `version` is the bad update version.
    pub fn check_and_maybe_rollback(&self) -> Option<String> {
        let mut state = WatchdogState::load(&self.data_dir);

        // No update recorded — nothing to watch
        let update_version = match &state.last_update_version {
            Some(v) => v.clone(),
            None => return None,
        };

        // Last shutdown was clean — update is stable, clear crash history
        if state.clean_shutdown {
            state.crash_timestamps.clear();
            state.clean_shutdown = false;
            let _ = state.save(&self.data_dir);
            return None;
        }

        // Unclean shutdown after update — record crash
        let now = crate::current_timestamp();
        state.crash_timestamps.push(now);

        // Prune crashes outside the window
        let window_start = now.saturating_sub(self.crash_window_secs);
        state.crash_timestamps.retain(|&ts| ts >= window_start);

        let crash_count = state.crash_timestamps.len() as u32;
        let _ = state.save(&self.data_dir);

        warn!(
            "Watchdog: unclean shutdown detected after update v{} ({}/{} crashes in {}s window)",
            update_version, crash_count, self.crash_threshold, self.crash_window_secs
        );

        if crash_count >= self.crash_threshold {
            warn!(
                "Watchdog: {} crashes in {}s window — triggering ROLLBACK of v{}",
                crash_count, self.crash_window_secs, update_version
            );
            // Clear state so rollback isn't re-triggered
            state.last_update_version = None;
            state.crash_timestamps.clear();
            let _ = state.save(&self.data_dir);
            Some(update_version)
        } else {
            info!(
                "Watchdog: crash {}/{} — not yet at threshold",
                crash_count, self.crash_threshold
            );
            None
        }
    }

    /// Clear watchdog state entirely (e.g., after successful manual rollback).
    pub fn clear(&self) {
        let state = WatchdogState::default();
        let _ = state.save(&self.data_dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn test_watchdog_no_update_no_rollback() {
        let dir = test_dir();
        let wd = UpdateWatchdog::new(dir.path().to_path_buf(), Network::Devnet);
        assert!(wd.check_and_maybe_rollback().is_none());
    }

    #[test]
    fn test_watchdog_clean_shutdown_no_rollback() {
        let dir = test_dir();
        let wd = UpdateWatchdog::new(dir.path().to_path_buf(), Network::Devnet);

        wd.record_update("1.0.1");
        wd.record_clean_shutdown();

        assert!(wd.check_and_maybe_rollback().is_none());
    }

    #[test]
    fn test_watchdog_single_crash_no_rollback() {
        let dir = test_dir();
        let wd = UpdateWatchdog::new(dir.path().to_path_buf(), Network::Devnet);

        wd.record_update("1.0.1");
        // Simulate 1 crash (no clean shutdown before check)
        assert!(wd.check_and_maybe_rollback().is_none());
    }

    #[test]
    fn test_watchdog_three_crashes_triggers_rollback() {
        let dir = test_dir();
        let wd = UpdateWatchdog::new(dir.path().to_path_buf(), Network::Devnet);

        wd.record_update("1.0.1");

        // Simulate 3 crashes (devnet threshold = 3)
        assert!(wd.check_and_maybe_rollback().is_none()); // crash 1
        assert!(wd.check_and_maybe_rollback().is_none()); // crash 2
        let result = wd.check_and_maybe_rollback(); // crash 3
        assert_eq!(result, Some("1.0.1".to_string()));
    }

    #[test]
    fn test_watchdog_rollback_clears_state() {
        let dir = test_dir();
        let wd = UpdateWatchdog::new(dir.path().to_path_buf(), Network::Devnet);

        wd.record_update("1.0.1");
        wd.check_and_maybe_rollback(); // crash 1
        wd.check_and_maybe_rollback(); // crash 2
        wd.check_and_maybe_rollback(); // crash 3 → rollback

        // After rollback, state is cleared — no more rollback
        assert!(wd.check_and_maybe_rollback().is_none());
    }

    #[test]
    fn test_watchdog_state_persistence() {
        let dir = test_dir();
        let wd = UpdateWatchdog::new(dir.path().to_path_buf(), Network::Devnet);

        wd.record_update("2.0.0");
        wd.check_and_maybe_rollback(); // crash 1

        // Create a new watchdog (simulates restart) — state persists
        let wd2 = UpdateWatchdog::new(dir.path().to_path_buf(), Network::Devnet);
        wd2.check_and_maybe_rollback(); // crash 2 (should still accumulate)
        let result = wd2.check_and_maybe_rollback(); // crash 3
        assert_eq!(result, Some("2.0.0".to_string()));
    }
}
