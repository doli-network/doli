//! Version enforcement and production gating

use doli_core::network::Network;
use serde::{Deserialize, Serialize};

use crate::constants::{GRACE_PERIOD, VETO_PERIOD};
use crate::params::UpdateParams;
use crate::types::Release;
use crate::util::{current_timestamp, current_version, is_newer_version};

// ============================================================================
// Veto & Grace Period Free Functions
// ============================================================================

/// Get the deadline timestamp for when veto period ends
pub fn veto_deadline(release: &Release) -> u64 {
    release.published_at + VETO_PERIOD.as_secs()
}

/// Check if veto period has ended
pub fn veto_period_ended(release: &Release) -> bool {
    current_timestamp() >= veto_deadline(release)
}

/// Grace period deadline (when enforcement begins)
///
/// Uses mainnet defaults. For network-aware timing, use `UpdateParams::grace_period_deadline`.
pub fn grace_period_deadline(release: &Release) -> u64 {
    veto_deadline(release) + GRACE_PERIOD.as_secs()
}

/// Grace period deadline with network-specific timing
pub fn grace_period_deadline_for_network(release: &Release, network: Network) -> u64 {
    let params = UpdateParams::for_network(network);
    params.grace_period_deadline(release)
}

/// Check if we're in the grace period (after approval, before enforcement)
///
/// Uses mainnet defaults. For network-aware timing, use `UpdateParams::in_grace_period`.
pub fn in_grace_period(release: &Release) -> bool {
    let now = current_timestamp();
    let veto_end = veto_deadline(release);
    let grace_end = grace_period_deadline(release);
    now >= veto_end && now < grace_end
}

/// Check if we're in the grace period with network-specific timing
pub fn in_grace_period_for_network(release: &Release, network: Network) -> bool {
    let params = UpdateParams::for_network(network);
    params.in_grace_period(release)
}

// ============================================================================
// Version Enforcement
// ============================================================================

/// Maximum enforcement duration (30 minutes).
/// If enforcement has been active for this long without the update being applied,
/// it auto-expires. Prevents indefinite production halt when auto-apply fails
/// (e.g., download error, wrong tarball name, network issues).
pub const ENFORCEMENT_TIMEOUT_SECS: u64 = 30 * 60;

/// Version enforcement state - tracks when production should be blocked
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionEnforcement {
    /// Minimum required version to produce blocks
    pub min_version: String,
    /// Timestamp when enforcement begins (after grace period)
    pub enforcement_time: u64,
    /// Whether this enforcement is active
    pub active: bool,
    /// Whether the update binary was successfully downloaded and is ready to apply
    #[serde(default)]
    pub binary_ready: bool,
}

impl VersionEnforcement {
    /// Create enforcement for an approved release (using mainnet defaults)
    ///
    /// For network-aware timing, use `from_approved_release_with_params` instead.
    pub fn from_approved_release(release: &Release) -> Self {
        let enforcement_time = veto_deadline(release) + GRACE_PERIOD.as_secs();
        Self {
            min_version: release.version.clone(),
            enforcement_time,
            active: false,
            binary_ready: false,
        }
    }

    /// Create enforcement for an approved release with network-specific timing
    ///
    /// This is the preferred method when you have access to network context.
    pub fn from_approved_release_with_params(release: &Release, params: &UpdateParams) -> Self {
        let enforcement_time = params.grace_period_deadline(release);
        Self {
            min_version: release.version.clone(),
            enforcement_time,
            active: false,
            binary_ready: false,
        }
    }

    /// Check if enforcement should now be active
    pub fn should_enforce(&self) -> bool {
        current_timestamp() >= self.enforcement_time
    }

    /// Calculate seconds until enforcement
    pub fn seconds_until_enforcement(&self) -> u64 {
        self.enforcement_time.saturating_sub(current_timestamp())
    }

    /// Calculate hours until enforcement
    pub fn hours_until_enforcement(&self) -> u64 {
        self.seconds_until_enforcement() / 3600
    }

    /// Check if current version meets requirement
    pub fn version_meets_requirement(&self, current: &str) -> bool {
        !is_newer_version(&self.min_version, current)
    }
}

// ============================================================================
// Production Gating
// ============================================================================

/// Production blocked error with helpful message
#[derive(Debug, Clone)]
pub struct ProductionBlocked {
    pub current_version: String,
    pub required_version: String,
}

impl std::fmt::Display for ProductionBlocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            r#"
╔══════════════════════════════════════════════════════════════════╗
║                    ⚠️  PRODUCTION PAUSED                          ║
╠══════════════════════════════════════════════════════════════════╣
║                                                                  ║
║  Your node ({}) is outdated.                                     ║
║  Required version: {}                                            ║
║                                                                  ║
║  Network security requires all producers to be updated.          ║
║  Block production is paused until update is complete.            ║
║                                                                  ║
║  To update, run:                                                 ║
║    doli-node update apply                                        ║
║                                                                  ║
║  After updating, production will resume automatically.           ║
║                                                                  ║
╚══════════════════════════════════════════════════════════════════╝
"#,
            self.current_version, self.required_version
        )
    }
}

impl std::error::Error for ProductionBlocked {}

/// Check if production is allowed based on version enforcement
///
/// Production is blocked ONLY when ALL of these are true:
/// 1. Enforcement time has passed (veto + grace period expired)
/// 2. Current version doesn't meet the requirement
/// 3. The update binary was successfully downloaded (binary_ready = true)
/// 4. Enforcement hasn't timed out (< ENFORCEMENT_TIMEOUT_SECS since enforcement_time)
///
/// If the binary was never downloaded (auto-apply failed), production continues
/// with a warning — the network shouldn't halt because of a download failure.
pub fn check_production_allowed(
    enforcement: Option<&VersionEnforcement>,
) -> std::result::Result<(), ProductionBlocked> {
    if let Some(enf) = enforcement {
        if enf.should_enforce() && !enf.version_meets_requirement(current_version()) {
            // Don't block if binary was never downloaded
            if !enf.binary_ready {
                tracing::warn!(
                    "Update v{} approved but binary not downloaded — production continues",
                    enf.min_version
                );
                return Ok(());
            }
            // Don't block if enforcement has timed out
            let elapsed = current_timestamp().saturating_sub(enf.enforcement_time);
            if elapsed > ENFORCEMENT_TIMEOUT_SECS {
                tracing::warn!(
                    "Enforcement for v{} timed out after {}m — production resumed",
                    enf.min_version,
                    elapsed / 60
                );
                return Ok(());
            }
            return Err(ProductionBlocked {
                current_version: current_version().to_string(),
                required_version: enf.min_version.clone(),
            });
        }
    }
    Ok(())
}
