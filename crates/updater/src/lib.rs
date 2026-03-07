//! DOLI Auto-Update System
//!
//! Simple, transparent auto-updates with community veto power.
//!
//! # Rules (no exceptions)
//! - ALL updates: 2-epoch veto period (configurable per network)
//! - 40% of producers can veto any update
//! - 3 of 5 maintainer signatures required (via SIGNATURES.json in GitHub Releases)
//!
//! # Flow
//! 1. Release published on GitHub (CI creates CHECKSUMS.txt)
//! 2. Maintainers sign with `doli release sign` → SIGNATURES.json uploaded
//! 3. Veto period begins (2 epochs mainnet, 1 min devnet)
//! 4. Producers can vote to veto
//! 5. If >= 40% veto: REJECTED
//! 6. If < 40% veto: APPROVED and applied
//!
//! # Network-Aware Parameters
//!
//! All timing parameters are configurable per network via `UpdateParams`:
//! - Mainnet/Testnet: Production timing (2 epochs veto, 1 epoch grace)
//! - Devnet: Accelerated timing (60s veto, 30s grace) for fast testing

mod apply;
mod download;
pub mod hardfork;
pub mod test_keys;
mod vote;
pub mod watchdog;

pub use apply::{
    apply_update, auto_apply_from_github, backup_current, current_binary_path,
    extract_binary_from_tarball, extract_named_binary_from_tarball, install_binary, restart_node,
    rollback,
};
pub use download::{
    download_binary, download_checksums_txt, download_from_url, download_signatures_json,
    fetch_github_release, fetch_latest_release, verify_hash, GithubReleaseInfo,
};
pub use test_keys::{
    create_test_release_signatures, should_use_test_keys, sign_with_test_key,
    test_maintainer_pubkeys, TestMaintainerKey, TEST_MAINTAINER_KEYS,
};
pub use vote::{Vote, VoteMessage, VoteTracker};

use crypto::{PublicKey, Signature as CryptoSignature};
use doli_core::network::Network;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, info, warn};

// ============================================================================
// Constants - Simple, fixed, no exceptions
// ============================================================================

/// Veto period: 5 minutes for ALL updates (early network; target: 7 days)
pub const VETO_PERIOD: Duration = Duration::from_secs(5 * 60);

/// Grace period after approval: 1 epoch (~1h) to update before enforcement
pub const GRACE_PERIOD: Duration = Duration::from_secs(3600);

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

/// Bootstrap maintainer public keys (Ed25519, hex-encoded)
///
/// These keys are used as a **fallback** for release signature verification
/// before on-chain state is available (e.g., during initial sync or CLI commands
/// without a running node). Once the node has synced and the on-chain maintainer
/// set is derived from the first 5 registered producers, those on-chain keys
/// take precedence.
///
/// See `verify_release_signatures_with_keys()` for the key selection logic.
pub const BOOTSTRAP_MAINTAINER_KEYS: [&str; 5] = [
    // Bootstrap keys (first 5 registered producers on mainnet)
    // These are overridden by on-chain state once the node is synced
    // N1 — omegacortex — producer_1.json
    "202047256a8072a8b8f476691b9a5ae87710cc545e8707ca9fe0c803c3e6d3df",
    // N2 — omegacortex — producer_2.json
    "effe88fefb6d992a1329277a1d49c7296d252bbc368319cb4bc061119926272b",
    // N3 — omegacortex — producer_3.json
    "54323cefd0eabac89b2a2198c95a8f261598c341a8e579a05e26322325c48c2b",
    // N4 — pro-KVM1 — producer.json
    "a1596a36fd3344bae323f8cdb7a0be7f4ca2a118de3cca184b465608e9beda1d",
    // N5 — fpx — producer.json
    "c5acb5b359c7a2093b8c788862cf57c5418e94de8b1fc6a254dc0862ee3c03a9",
];

/// Check if the bootstrap maintainer keys are still placeholders
///
/// Returns `true` if any key starts with "00000000" (placeholder pattern).
/// This MUST return `false` before mainnet launch.
pub fn is_using_placeholder_keys() -> bool {
    BOOTSTRAP_MAINTAINER_KEYS
        .iter()
        .any(|k| k.starts_with("00000000"))
}

/// Verify that bootstrap maintainer keys are production-ready
///
/// Panics if placeholder keys are detected.
/// Call this during mainnet node initialization.
pub fn assert_production_keys() {
    if is_using_placeholder_keys() {
        panic!(
            "FATAL: Placeholder bootstrap maintainer keys detected!\n\
             This build cannot be used for mainnet.\n\
             Replace BOOTSTRAP_MAINTAINER_KEYS in doli-updater/src/lib.rs with real keys."
        );
    }
}

/// Get the bootstrap maintainer public keys
///
/// Returns test keys if DOLI_TEST_KEYS=1 is set, otherwise returns bootstrap keys.
/// For on-chain maintainer keys, use `verify_release_signatures_with_keys()`.
pub fn get_maintainer_keys() -> Vec<&'static str> {
    if should_use_test_keys() {
        test_maintainer_pubkeys()
    } else {
        BOOTSTRAP_MAINTAINER_KEYS.to_vec()
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

// ============================================================================
// Types
// ============================================================================

/// Release metadata for network targeting (metadata.json in GitHub Releases)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReleaseMetadata {
    pub version: String,
    pub networks: Vec<String>,
    #[serde(default)]
    pub min_protocol_version: Option<u32>,
}

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

    /// Target networks from metadata.json. Empty = all networks (backward compat).
    #[serde(default)]
    pub target_networks: Vec<String>,
}

/// A maintainer's signature on a release
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaintainerSignature {
    /// Maintainer's public key (hex-encoded)
    pub public_key: String,

    /// Signature over "version:checksums_sha256" (hex-encoded)
    pub signature: String,
}

/// SIGNATURES.json file format (uploaded to GitHub Releases)
///
/// Each maintainer signs `"{version}:{sha256(CHECKSUMS.txt)}"` — one signature
/// covers all platforms since CHECKSUMS.txt contains per-platform hashes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignaturesFile {
    /// Semantic version (e.g., "1.0.27")
    pub version: String,

    /// SHA-256 hash of CHECKSUMS.txt (hex-encoded)
    pub checksums_sha256: String,

    /// Maintainer signatures
    pub signatures: Vec<MaintainerSignature>,
}

/// Update configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateConfig {
    /// Enable auto-updates (default: true)
    pub enabled: bool,

    /// Only notify, don't apply (default: false)
    pub notify_only: bool,

    /// Enable automatic rollback on update failures (default: true)
    pub auto_rollback: bool,

    /// Check interval in seconds (default: 6 hours)
    pub check_interval_secs: u64,

    /// Veto period in seconds (default: 2 hours)
    pub veto_period_secs: u64,

    /// Grace period after approval in seconds (default: 1 hour)
    pub grace_period_secs: u64,

    /// Custom update URL (optional, uses mirrors by default)
    pub custom_url: Option<String>,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            notify_only: false,
            auto_rollback: true,
            check_interval_secs: CHECK_INTERVAL.as_secs(),
            veto_period_secs: VETO_PERIOD.as_secs(),
            grace_period_secs: GRACE_PERIOD.as_secs(),
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
    RejectedByVeto { veto_percent: u8, threshold: u8 },

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
    /// Create enforcement for an approved release (using mainnet defaults)
    ///
    /// For network-aware timing, use `from_approved_release_with_params` instead.
    pub fn from_approved_release(release: &Release) -> Self {
        let enforcement_time = veto_deadline(release) + GRACE_PERIOD.as_secs();
        Self {
            min_version: release.version.clone(),
            enforcement_time,
            active: false,
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
pub fn check_production_allowed(
    enforcement: Option<&VersionEnforcement>,
) -> std::result::Result<(), ProductionBlocked> {
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
// Release Signing
// ============================================================================

/// Sign a release hash with a maintainer's private key
///
/// Signs the message `"version:sha256"` which matches the format verified by
/// `verify_release_signatures_with_keys()`. Returns a `MaintainerSignature`
/// containing the public key and hex-encoded signature.
///
/// # Usage
///
/// ```ignore
/// let keypair = crypto::KeyPair::from_private_key(private_key);
/// let sig = sign_release_hash(&keypair, "0.2.0", "abcdef1234...");
/// println!("{}", serde_json::to_string(&sig).unwrap());
/// ```
pub fn sign_release_hash(
    keypair: &crypto::KeyPair,
    version: &str,
    binary_sha256: &str,
) -> MaintainerSignature {
    let message = format!("{}:{}", version, binary_sha256);
    let signature = crypto::signature::sign(message.as_bytes(), keypair.private_key());
    MaintainerSignature {
        public_key: keypair.public_key().to_hex(),
        signature: signature.to_hex(),
    }
}

// ============================================================================
// Core Logic
// ============================================================================

/// Verify release signatures using bootstrap keys only (convenience wrapper)
///
/// Use this for CLI commands and contexts where on-chain state is not available.
/// For the running node, prefer `verify_release_signatures_with_keys()`.
pub fn verify_release_signatures(release: &Release) -> Result<()> {
    verify_release_signatures_with_keys(release, &[])
}

/// Verify that a release has sufficient valid maintainer signatures
///
/// If `on_chain_keys` is non-empty, those keys are used for verification
/// (derived from first 5 registered producers on-chain). If empty, falls
/// back to `BOOTSTRAP_MAINTAINER_KEYS`.
pub fn verify_release_signatures_with_keys(
    release: &Release,
    on_chain_keys: &[String],
) -> Result<()> {
    let message = format!("{}:{}", release.version, release.binary_sha256);
    let message_bytes = message.as_bytes();

    // Determine which keys to use
    let using_on_chain = !on_chain_keys.is_empty();
    let allowed_keys: Vec<&str> = if using_on_chain {
        info!(
            "Verifying release signatures with {} on-chain maintainer keys",
            on_chain_keys.len()
        );
        on_chain_keys.iter().map(|s| s.as_str()).collect()
    } else {
        debug!("Verifying release signatures with bootstrap maintainer keys");
        BOOTSTRAP_MAINTAINER_KEYS.to_vec()
    };

    let mut valid_count = 0;

    for sig in &release.signatures {
        // Check if this is a known maintainer
        if !allowed_keys.contains(&sig.public_key.as_str()) {
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
            "Release {} verified: {}/{} valid signatures (source: {})",
            release.version,
            valid_count,
            REQUIRED_SIGNATURES,
            if using_on_chain {
                "on-chain"
            } else {
                "bootstrap"
            }
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

    #[test]
    fn test_vote_weight_bonds_times_seniority() {
        let params = UpdateParams::for_network(Network::Devnet);
        // Devnet: seniority_step_blocks = 144 (1 "year")

        // 1 bond, 0 years → 1.0 × 1.0 = 1.0
        let w = params.calculate_vote_weight(1, 0);
        assert!((w - 1.0).abs() < 0.001, "1 bond, 0 years: got {}", w);

        // 10 bonds, 0 years → 10.0 × 1.0 = 10.0
        let w = params.calculate_vote_weight(10, 0);
        assert!((w - 10.0).abs() < 0.001, "10 bonds, 0 years: got {}", w);

        // 1 bond, 1 year (144 blocks on devnet) → 1.0 × 1.75 = 1.75
        let w = params.calculate_vote_weight(1, params.seniority_step_blocks);
        assert!((w - 1.75).abs() < 0.001, "1 bond, 1 year: got {}", w);

        // 2 bonds, 4 years (576 blocks on devnet) → 2.0 × 4.0 = 8.0
        let w = params.calculate_vote_weight(2, params.seniority_maturity_blocks);
        assert!((w - 8.0).abs() < 0.001, "2 bonds, 4 years: got {}", w);

        // 10 bonds, 4 years → 10.0 × 4.0 = 40.0
        let w = params.calculate_vote_weight(10, params.seniority_maturity_blocks);
        assert!((w - 40.0).abs() < 0.001, "10 bonds, 4 years: got {}", w);

        // Seniority caps at 4 years (4.0x) even after 10 years
        let w = params.calculate_vote_weight(1, params.seniority_step_blocks * 10);
        assert!(
            (w - 4.0).abs() < 0.001,
            "1 bond, 10 years (capped): got {}",
            w
        );
    }

    #[test]
    fn test_vote_weight_as_u64_for_tracker() {
        let params = UpdateParams::for_network(Network::Devnet);

        // Weights stored as u64 with 100x multiplier for 2-decimal precision
        let w = params.calculate_vote_weight(1, 0);
        assert_eq!((w * 100.0) as u64, 100); // 1.0 × 100

        let w = params.calculate_vote_weight(10, 0);
        assert_eq!((w * 100.0) as u64, 1000); // 10.0 × 100

        let w = params.calculate_vote_weight(2, params.seniority_maturity_blocks);
        assert_eq!((w * 100.0) as u64, 800); // 8.0 × 100

        let w = params.calculate_vote_weight(10, params.seniority_maturity_blocks);
        assert_eq!((w * 100.0) as u64, 4000); // 40.0 × 100
    }

    #[test]
    fn test_vote_weight_veto_with_bonds() {
        use crate::vote::{Vote, VoteTracker};
        let params = UpdateParams::for_network(Network::Devnet);

        // Scenario: 5 genesis producers (1 bond, ~24 blocks active) + 1 whale (10 bonds, new)
        let mut weights = std::collections::HashMap::new();
        for i in 0..5 {
            let w = params.calculate_vote_weight(1, 24); // ~0.125 years
            weights.insert(format!("genesis_{}", i), (w * 100.0) as u64);
        }
        let whale_w = params.calculate_vote_weight(10, 1);
        weights.insert("whale".to_string(), (whale_w * 100.0) as u64);

        let total_weight: u64 = weights.values().sum();
        let mut tracker = VoteTracker::with_weights("99.0.0".into(), weights);

        // Whale alone vetoes — should exceed 40%
        tracker.record_vote("whale".into(), Vote::Veto);
        assert!(
            tracker.should_reject_weighted(total_weight),
            "Whale (10 bonds) should be able to veto alone (weight {}/{})",
            tracker.veto_weight(),
            total_weight
        );
    }

    #[test]
    fn test_sign_release_hash() {
        // Generate a test keypair
        let private_key = crypto::PrivateKey::from_bytes([42u8; 32]);
        let keypair = crypto::KeyPair::from_private_key(private_key);

        let sig = sign_release_hash(&keypair, "1.0.0", "abcdef1234567890");

        // Verify the signature matches what verify_release_signatures expects
        assert_eq!(sig.public_key, keypair.public_key().to_hex());
        assert!(!sig.signature.is_empty());

        // Manually verify the signature
        let message = b"1.0.0:abcdef1234567890";
        let pubkey = crypto::PublicKey::from_hex(&sig.public_key).unwrap();
        let signature = crypto::Signature::from_hex(&sig.signature).unwrap();
        assert!(crypto::signature::verify(message, &signature, &pubkey).is_ok());
    }

    #[test]
    fn test_seniority_multiplier() {
        let params = UpdateParams::for_network(Network::Devnet);

        assert!((params.seniority_multiplier(0) - 1.0).abs() < 0.001);
        assert!((params.seniority_multiplier(params.seniority_step_blocks) - 1.75).abs() < 0.001);
        assert!(
            (params.seniority_multiplier(params.seniority_step_blocks * 2) - 2.5).abs() < 0.001
        );
        assert!(
            (params.seniority_multiplier(params.seniority_maturity_blocks) - 4.0).abs() < 0.001
        );
    }
}
