//! Auto-update integration for doli-node
//!
//! This module integrates the doli-updater crate with the node,
//! handling the update check loop and veto voting process.
//!
//! # Mandatory Notification System
//!
//! When an update is detected:
//! - ALL nodes display a prominent alert
//! - The alert shows version, changelog summary, and veto period
//! - Producers can vote to veto if they have objections
//! - If no veto threshold reached after 7 days, update auto-applies

mod notifications;
mod service;

use std::path::Path;
use tracing::warn;

// Re-export items from the updater crate (preserves original public API)
pub use updater::{
    apply_update, backup_current, bootstrap_maintainer_keys, check_production_allowed,
    current_binary_path, current_version, download_from_url, extract_binary_from_tarball,
    fetch_github_release, fetch_latest_release, install_binary, is_newer_version, restart_node,
    rollback, sign_release_hash, verify_hash, verify_release_signatures, veto_deadline,
    veto_period_ended, ProductionBlocked, Release, UpdateConfig, VersionEnforcement, Vote,
    VoteMessage, VoteTracker, GITHUB_RELEASES_URL,
};

// Re-export sub-module public items
pub use service::spawn_update_service;

/// Re-export CLI as a sub-module (preserves `updater::cli::show_status_from_disk` path)
pub mod cli {
    pub use super::cli_mod::show_status_from_disk;
}

#[path = "cli.rs"]
mod cli_mod;

/// ANSI color codes for terminal output
pub(crate) mod colors {
    pub const YELLOW: &str = "\x1b[1;33m";
    pub const CYAN: &str = "\x1b[0;36m";
    pub const GREEN: &str = "\x1b[0;32m";
    pub const RED: &str = "\x1b[0;31m";
    pub const BOLD: &str = "\x1b[1m";
    pub const RESET: &str = "\x1b[0m";
}

/// State of a pending update (persisted to disk)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PendingUpdate {
    pub release: Release,
    #[serde(default)]
    pub vote_tracker: VoteTracker,
    /// When the notification was first shown
    #[serde(default)]
    pub first_notified_at: u64,
    /// Whether the update was approved (veto period passed)
    #[serde(default)]
    pub approved: bool,
    /// Version enforcement state (active after grace period)
    #[serde(default)]
    pub enforcement: Option<VersionEnforcement>,
}

impl PendingUpdate {
    /// Load pending update from disk
    pub fn load(data_dir: &Path) -> Option<Self> {
        let path = data_dir.join("pending_update.json");
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(content) => match serde_json::from_str(&content) {
                    Ok(pending) => Some(pending),
                    Err(e) => {
                        warn!("Failed to parse pending_update.json: {}", e);
                        None
                    }
                },
                Err(e) => {
                    warn!("Failed to read pending_update.json: {}", e);
                    None
                }
            }
        } else {
            None
        }
    }

    /// Save pending update to disk
    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        let path = data_dir.join("pending_update.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)
    }

    /// Remove pending update from disk
    pub fn remove(data_dir: &Path) -> std::io::Result<()> {
        let path = data_dir.join("pending_update.json");
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// Calculate days remaining in veto period
    pub fn days_remaining(&self) -> u64 {
        let deadline = veto_deadline(&self.release);
        let now = updater::current_timestamp();
        let remaining_secs = deadline.saturating_sub(now);
        remaining_secs / 86400
    }

    /// Calculate hours remaining (after days)
    pub fn hours_remaining(&self) -> u64 {
        let deadline = veto_deadline(&self.release);
        let now = updater::current_timestamp();
        let remaining_secs = deadline.saturating_sub(now);
        (remaining_secs % 86400) / 3600
    }
}

/// Get the pending update version from disk (for CLI use)
pub fn get_pending_version(data_dir: &Path) -> Option<String> {
    PendingUpdate::load(data_dir).map(|p| p.release.version)
}

/// Get the current version enforcement state (for production check)
pub fn get_version_enforcement(data_dir: &Path) -> Option<VersionEnforcement> {
    PendingUpdate::load(data_dir).and_then(|p| p.enforcement)
}

/// Check if production is currently allowed based on version enforcement
pub fn is_production_allowed(data_dir: &Path) -> Result<(), ProductionBlocked> {
    let enforcement = get_version_enforcement(data_dir);
    check_production_allowed(enforcement.as_ref())
}

/// Get the pending update for display (for CLI use)
pub fn get_pending_update(data_dir: &Path) -> Option<PendingUpdate> {
    PendingUpdate::load(data_dir)
}
