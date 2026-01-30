//! DOLI Auto-Update System
//!
//! Simple, transparent auto-updates with community veto power.
//!
//! # Rules (no exceptions)
//! - ALL updates: 7 days veto period
//! - 33% of producers can veto any update
//! - 3 of 5 maintainer signatures required
//!
//! # Flow
//! 1. Release published (signed by 3/5 maintainers)
//! 2. 7-day veto period begins
//! 3. Producers can vote to veto
//! 4. If >= 33% veto: REJECTED
//! 5. If < 33% veto: APPROVED and applied

mod apply;
mod download;
pub mod test_keys;
mod vote;

pub use apply::{apply_update, backup_current, restart_node, rollback};
pub use download::{download_binary, fetch_latest_release, verify_hash};
pub use test_keys::{
    create_test_release_signatures, should_use_test_keys, sign_with_test_key,
    test_maintainer_pubkeys, TestMaintainerKey, TEST_MAINTAINER_KEYS,
};
pub use vote::{Vote, VoteMessage, VoteTracker};

use crypto::{PublicKey, Signature as CryptoSignature};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, error, info, warn};

// ============================================================================
// Constants - Simple, fixed, no exceptions
// ============================================================================

/// Veto period: 7 days for ALL updates
pub const VETO_PERIOD: Duration = Duration::from_secs(7 * 24 * 3600);

/// Grace period after approval: 48 hours to update before enforcement
pub const GRACE_PERIOD: Duration = Duration::from_secs(48 * 3600);

/// Veto threshold: 40% of active producers (weighted by seniority)
///
/// Why 40% instead of 33%:
/// - 33% allows a $44K early attacker to block governance for 4 years
/// - 40% raises the cost: requires 15 nodes instead of 10 for sustained blocking
/// - Combined with activity penalty, makes "register and wait" attacks expensive
///
/// Note: This must match VETO_THRESHOLD_PERCENT in doli-storage
pub const VETO_THRESHOLD_PERCENT: u8 = 40;

/// Required maintainer signatures: 3 of 5
pub const REQUIRED_SIGNATURES: usize = 3;

/// Maintainer public keys (Ed25519, hex-encoded)
/// These are hardcoded for security - no external configuration
///
/// # IMPORTANT: Production Keys Required
///
/// The keys below are **PLACEHOLDER VALUES** for development only.
/// Before mainnet launch, these must be replaced with real Ed25519 public keys
/// belonging to the 5 designated maintainers.
///
/// ## Key Generation Process
///
/// Each maintainer should:
/// 1. Generate an Ed25519 keypair offline
/// 2. Store the private key securely (hardware wallet, air-gapped machine)
/// 3. Provide only the public key (hex-encoded) for inclusion here
///
/// ## Security Requirements
///
/// - Keys must be generated on secure, air-gapped systems
/// - Private keys must never touch an internet-connected device
/// - Each maintainer must control exactly one key
/// - 3 of 5 signatures required for any update
///
/// ## Verification
///
/// Before mainnet: `is_using_placeholder_keys()` must return `false`
pub const MAINTAINER_KEYS: [&str; 5] = [
    // Testnet maintainer keys (also used for mainnet until replaced)
    // These are the 5 keys that control protocol updates (3-of-5 threshold)
    // Key 1
    "721d2bc74ced1842eb77754dac75dc78d8cf7a47e10c83a7dc588c82187b70b9",
    // Key 2
    "d0c62cb4e143d548271eb97c4651e77b6cf52909a016bda6fb500c3bc022298d",
    // Key 3
    "9fac605a1ebf2acfa54ef8406ab66d604df97d63da1f1ab6a45561c7e51be697",
    // Key 4
    "97bdb0a9a52d4ed178c2307e3eb17e316b57d098af095b9cefc0c69d73e8817f",
    // Key 5
    "82ed55afabfe38d826c1e2b870aefcc9ed0de45e5620adb4f858e6f47c8d4096",
];

/// Check if the maintainer keys are still placeholders
///
/// Returns `true` if any key starts with "00000000" (placeholder pattern).
/// This MUST return `false` before mainnet launch.
///
/// # Example
///
/// ```rust
/// use updater::is_using_placeholder_keys;
///
/// // In production startup code:
/// if is_using_placeholder_keys() {
///     eprintln!("WARNING: Using placeholder maintainer keys!");
///     eprintln!("This build is NOT suitable for mainnet.");
/// }
/// ```
pub fn is_using_placeholder_keys() -> bool {
    MAINTAINER_KEYS.iter().any(|k| k.starts_with("00000000"))
}

/// Verify that maintainer keys are production-ready
///
/// Panics if placeholder keys are detected.
/// Call this during mainnet node initialization.
///
/// # Panics
///
/// Panics with a descriptive message if placeholder keys are in use.
pub fn assert_production_keys() {
    if is_using_placeholder_keys() {
        panic!(
            "FATAL: Placeholder maintainer keys detected!\n\
             This build cannot be used for mainnet.\n\
             Replace MAINTAINER_KEYS in doli-updater/src/lib.rs with real keys."
        );
    }
}

/// Get the active maintainer public keys
///
/// Returns test keys if DOLI_TEST_KEYS=1 is set, otherwise returns production keys.
/// For devnet testing, set the environment variable before starting nodes.
pub fn get_maintainer_keys() -> Vec<&'static str> {
    if should_use_test_keys() {
        test_maintainer_pubkeys()
    } else {
        MAINTAINER_KEYS.to_vec()
    }
}

/// Default update check interval: 6 hours
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);

/// GitHub repository for releases (primary source)
/// Using author's personal repo for authenticity (like torvalds/linux, antirez/redis)
pub const GITHUB_REPO: &str = "e-weil/doli";

/// GitHub API URL for latest release
pub const GITHUB_API_URL: &str = "https://api.github.com/repos/e-weil/doli/releases/latest";

/// GitHub releases download base URL
pub const GITHUB_RELEASES_URL: &str = "https://github.com/e-weil/doli/releases/download";

/// Fallback update server (if GitHub is unreachable)
pub const FALLBACK_MIRROR: &str = "https://releases.doli.network";

// ============================================================================
// Types
// ============================================================================

/// A signed release from maintainers
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Release {
    /// Semantic version (e.g., "1.0.1")
    pub version: String,

    /// SHA-256 hash of the binary (hex-encoded)
    pub binary_sha256: String,

    /// URL template for binary download
    /// Use {platform} for: linux-x64, linux-arm64, macos-x64, macos-arm64
    pub binary_url_template: String,

    /// Human-readable changelog
    pub changelog: String,

    /// Unix timestamp when release was published
    pub published_at: u64,

    /// Maintainer signatures
    pub signatures: Vec<MaintainerSignature>,
}

/// A maintainer's signature on a release
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaintainerSignature {
    /// Maintainer's public key (hex-encoded)
    pub public_key: String,

    /// Signature over "version:binary_sha256" (hex-encoded)
    pub signature: String,
}

/// Update configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateConfig {
    /// Enable auto-updates (default: true)
    pub enabled: bool,

    /// Only notify, don't apply (default: false)
    pub notify_only: bool,

    /// Check interval in seconds (default: 6 hours)
    pub check_interval_secs: u64,

    /// Custom update URL (optional, uses mirrors by default)
    pub custom_url: Option<String>,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            notify_only: false,
            check_interval_secs: CHECK_INTERVAL.as_secs(),
            custom_url: None,
        }
    }
}

/// Errors that can occur during updates
#[derive(Error, Debug)]
pub enum UpdateError {
    #[error("Insufficient signatures: {found}/{required}")]
    InsufficientSignatures { found: usize, required: usize },

    #[error("Invalid signature from maintainer {0}")]
    InvalidSignature(String),

    #[error("Binary hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },

    #[error("Download failed: {0}")]
    DownloadFailed(String),

    #[error("Installation failed: {0}")]
    InstallFailed(String),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Veto period still active: {remaining_hours}h remaining")]
    VetoPeriodActive {
        remaining_hours: u64,
        message: String,
    },

    #[error("Update rejected by community: {veto_percent}% veto (threshold: {threshold}%)")]
    RejectedByVeto {
        veto_percent: u8,
        threshold: u8,
    },

    #[error("Update not yet approved")]
    NotApproved,
}

pub type Result<T> = std::result::Result<T, UpdateError>;

/// Result of the veto period
#[derive(Debug, Clone)]
pub struct VoteResult {
    pub total_producers: usize,
    pub veto_count: usize,
    pub veto_percent: u8,
    pub approved: bool,
}

/// Version enforcement state - tracks when production should be blocked
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionEnforcement {
    /// Minimum required version to produce blocks
    pub min_version: String,
    /// Timestamp when enforcement begins (after grace period)
    pub enforcement_time: u64,
    /// Whether this enforcement is active
    pub active: bool,
}

impl VersionEnforcement {
    /// Create enforcement for an approved release
    pub fn from_approved_release(release: &Release) -> Self {
        let enforcement_time = veto_deadline(release) + GRACE_PERIOD.as_secs();
        Self {
            min_version: release.version.clone(),
            enforcement_time,
            active: false,
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
pub fn check_production_allowed(enforcement: Option<&VersionEnforcement>) -> std::result::Result<(), ProductionBlocked> {
    if let Some(enf) = enforcement {
        if enf.should_enforce() && !enf.version_meets_requirement(current_version()) {
            return Err(ProductionBlocked {
                current_version: current_version().to_string(),
                required_version: enf.min_version.clone(),
            });
        }
    }
    Ok(())
}

/// Grace period deadline (when enforcement begins)
pub fn grace_period_deadline(release: &Release) -> u64 {
    veto_deadline(release) + GRACE_PERIOD.as_secs()
}

/// Check if we're in the grace period (after approval, before enforcement)
pub fn in_grace_period(release: &Release) -> bool {
    let now = current_timestamp();
    let veto_end = veto_deadline(release);
    let grace_end = grace_period_deadline(release);
    now >= veto_end && now < grace_end
}

// ============================================================================
// Core Logic
// ============================================================================

/// Verify that a release has sufficient valid maintainer signatures
pub fn verify_release_signatures(release: &Release) -> Result<()> {
    let message = format!("{}:{}", release.version, release.binary_sha256);
    let message_bytes = message.as_bytes();

    let mut valid_count = 0;

    for sig in &release.signatures {
        // Check if this is a known maintainer
        if !MAINTAINER_KEYS.contains(&sig.public_key.as_str()) {
            debug!("Ignoring signature from unknown key: {}", sig.public_key);
            continue;
        }

        // Decode public key
        let pubkey_bytes = match hex::decode(&sig.public_key) {
            Ok(bytes) => bytes,
            Err(_) => {
                warn!("Invalid hex in public key: {}", sig.public_key);
                continue;
            }
        };

        // Decode signature
        let sig_bytes = match hex::decode(&sig.signature) {
            Ok(bytes) => bytes,
            Err(_) => {
                warn!("Invalid hex in signature");
                continue;
            }
        };

        // Verify signature using doli-crypto
        if verify_ed25519(&pubkey_bytes, message_bytes, &sig_bytes) {
            valid_count += 1;
            debug!(
                "Valid signature from maintainer: {}...",
                &sig.public_key[..16]
            );
        } else {
            warn!(
                "Invalid signature from maintainer: {}...",
                &sig.public_key[..16]
            );
        }
    }

    if valid_count >= REQUIRED_SIGNATURES {
        info!(
            "Release {} verified: {}/{} valid signatures",
            release.version, valid_count, REQUIRED_SIGNATURES
        );
        Ok(())
    } else {
        Err(UpdateError::InsufficientSignatures {
            found: valid_count,
            required: REQUIRED_SIGNATURES,
        })
    }
}

/// Calculate veto result
pub fn calculate_veto_result(veto_count: usize, total_producers: usize) -> VoteResult {
    let veto_percent = if total_producers > 0 {
        ((veto_count * 100) / total_producers) as u8
    } else {
        0
    };

    let approved = veto_percent < VETO_THRESHOLD_PERCENT;

    VoteResult {
        total_producers,
        veto_count,
        veto_percent,
        approved,
    }
}

/// Get the deadline timestamp for when veto period ends
pub fn veto_deadline(release: &Release) -> u64 {
    release.published_at + VETO_PERIOD.as_secs()
}

/// Check if veto period has ended
pub fn veto_period_ended(release: &Release) -> bool {
    current_timestamp() >= veto_deadline(release)
}

/// Get current version from the running binary
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Compare versions (simple semver comparison)
pub fn is_newer_version(new: &str, current: &str) -> bool {
    // Parse as semver and compare
    let parse = |v: &str| -> (u32, u32, u32) {
        let parts: Vec<&str> = v.trim_start_matches('v').split('.').collect();
        (
            parts.first().and_then(|s| s.parse().ok()).unwrap_or(0),
            parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0),
            parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0),
        )
    };

    parse(new) > parse(current)
}

/// Get platform identifier for binary download
pub fn platform_identifier() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return "linux-x64";

    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return "linux-arm64";

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return "macos-x64";

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return "macos-arm64";

    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
    )))]
    return "unknown";
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Get current Unix timestamp
pub fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Verify Ed25519 signature using doli-crypto
fn verify_ed25519(pubkey_bytes: &[u8], message: &[u8], sig_bytes: &[u8]) -> bool {
    use crypto::signature::verify;

    // Parse public key
    let pubkey = match PublicKey::try_from_slice(pubkey_bytes) {
        Ok(pk) => pk,
        Err(_) => return false,
    };

    // Parse signature
    let signature = match CryptoSignature::try_from_slice(sig_bytes) {
        Ok(sig) => sig,
        Err(_) => return false,
    };

    // Verify
    verify(message, &signature, &pubkey).is_ok()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_comparison() {
        assert!(is_newer_version("1.0.1", "1.0.0"));
        assert!(is_newer_version("1.1.0", "1.0.9"));
        assert!(is_newer_version("2.0.0", "1.9.9"));
        assert!(!is_newer_version("1.0.0", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "1.0.1"));
        assert!(is_newer_version("v1.0.1", "v1.0.0"));
    }

    #[test]
    fn test_veto_calculation() {
        // 30% veto - should pass (< 40% threshold)
        let result = calculate_veto_result(30, 100);
        assert_eq!(result.veto_percent, 30);
        assert!(result.approved);

        // 39% veto - should pass (< 40% threshold)
        let result = calculate_veto_result(39, 100);
        assert_eq!(result.veto_percent, 39);
        assert!(result.approved);

        // 40% veto - should fail (>= 40% threshold)
        let result = calculate_veto_result(40, 100);
        assert_eq!(result.veto_percent, 40);
        assert!(!result.approved);

        // 50% veto - should fail
        let result = calculate_veto_result(50, 100);
        assert_eq!(result.veto_percent, 50);
        assert!(!result.approved);

        // Edge case: no producers
        let result = calculate_veto_result(0, 0);
        assert_eq!(result.veto_percent, 0);
        assert!(result.approved);
    }

    #[test]
    fn test_platform_identifier() {
        let platform = platform_identifier();
        assert!([
            "linux-x64",
            "linux-arm64",
            "macos-x64",
            "macos-arm64",
            "unknown"
        ]
        .contains(&platform));
    }
}
