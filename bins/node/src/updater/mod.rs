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
//! - If the veto threshold is not reached before the CONFIGURED veto period
//!   (`UpdateConfig::veto_period_secs`, default `VETO_PERIOD`) expires, the update
//!   auto-applies. There is no 7-day period and no seniority weighting; the veto
//!   window is measured from the node-local moment this node first saw the release.

mod notifications;
mod service;
mod trust_root_wiring;

use std::path::Path;
use tracing::warn;

// Re-export items from the updater crate (preserves original public API)
pub use updater::{
    apply_update, backup_current, bootstrap_maintainer_keys, check_production_allowed,
    current_binary_path, current_version, download_from_url, download_signatures_json,
    extract_binary_from_tarball, fetch_github_release, fetch_latest_release, install_binary,
    is_newer_version, restart_node, rollback, sign_release_hash, validate_release_hash,
    validate_release_version, verify_release_artifact, verify_release_with_trust_root,
    veto_deadline, veto_period_ended, ProductionBlocked, Release, UpdateConfig, VersionEnforcement,
    Vote, VoteMessage, VoteTracker, GITHUB_RELEASES_URL,
};

// Re-export sub-module public items
pub use service::spawn_update_service;
pub use trust_root_wiring::{
    command_trust_root, load_maintainer_state, maintainer_trust_root_fn, resolve_trust_root,
};

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
    /// When the notification was first shown — the node-local moment this node first saw
    /// the release, and the SOLE clock the install gate runs on (INC-I-172 F7(b)).
    ///
    /// NO `#[serde(default)]` (AUDIT-P2-013). `pending_update.json` is unauthenticated
    /// node-local state, and defaulting an absent field to `0` puts the veto deadline at
    /// the Unix epoch: `current_timestamp() >= 0 + veto_period` is true on the next 60 s
    /// tick, so the update is APPROVED and auto-installed with no veto window at all.
    /// Deleting one line from a JSON file must not be an install trigger. Without the
    /// default, a file missing the field fails to parse, and `load` already degrades
    /// safely to `None` = "no pending update".
    pub first_notified_at: u64,
    /// Whether the update was approved (veto period passed)
    #[serde(default)]
    pub approved: bool,
    /// Version enforcement state (active after grace period)
    #[serde(default)]
    pub enforcement: Option<VersionEnforcement>,
}

impl PendingUpdate {
    /// Load pending update from disk.
    ///
    /// Returns `None` — "no pending update" — for anything that does not parse, and for a
    /// `first_notified_at` of ZERO (AUDIT-P2-013). Zero is not a timestamp: it is the
    /// Unix epoch, which places the veto deadline decades in the past and makes the next
    /// 60 s tick install the update unattended. "No pending update" is the fail-closed
    /// reading — the update service simply re-detects the release on its next poll and
    /// starts a fresh, honest veto window.
    pub fn load(data_dir: &Path) -> Option<Self> {
        let path = data_dir.join("pending_update.json");
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(content) => match serde_json::from_str::<Self>(&content) {
                    Ok(pending) if pending.first_notified_at == 0 => {
                        warn!(
                            "Ignoring pending_update.json for {}: first_notified_at is 0. That is \
                             the Unix epoch, not a notification time — it would place the veto \
                             deadline in the past and install the update on the next tick. \
                             Treating it as NO pending update; the release will be re-detected \
                             and given a fresh veto window.",
                            pending.release.version
                        );
                        None
                    }
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

    /// Calculate days remaining in veto period.
    ///
    /// Measured from `first_notified_at` — the node-local moment this node first saw
    /// the release — NOT from `release.published_at`, which is unsigned and
    /// attacker-supplied (INC-I-172 F7(b)). A forged `published_at` used to move this
    /// operator-facing countdown independently of the deadline the service enforces.
    pub fn days_remaining(&self) -> u64 {
        let deadline = veto_deadline(self.first_notified_at);
        let now = updater::current_timestamp();
        let remaining_secs = deadline.saturating_sub(now);
        remaining_secs / 86400
    }

    /// Calculate hours remaining (after days). Same node-local reference as
    /// `days_remaining`; the two must never diverge.
    pub fn hours_remaining(&self) -> u64 {
        let deadline = veto_deadline(self.first_notified_at);
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
