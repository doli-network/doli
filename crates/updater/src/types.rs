//! Core types for the update system

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::constants::{CHECK_INTERVAL, GRACE_PERIOD, VETO_PERIOD};

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
