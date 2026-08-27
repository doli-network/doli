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

    /// NOT WIRED — read by nothing (INC-I-172 M1, AUDIT-P1-014).
    ///
    /// Set from `--no-auto-rollback` and defaulted to `true`, but no code path reads it:
    /// [`crate::watchdog::UpdateWatchdog`] has zero production callers, so there is no
    /// automatic post-update rollback to enable or disable. The field is retained so the
    /// long-standing CLI flag keeps parsing — removing the flag would fail startup on
    /// every systemd unit that carries it — and it is named here so nobody reports it as
    /// a live control. Rollback today is manual only (`doli-node update rollback`).
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

    /// The resolved trust root cannot authorise anything (INC-I-172 F1).
    ///
    /// Returned when the root has fewer keys than its own threshold, or a
    /// threshold of zero. This is a FAIL-CLOSED outcome: verification refuses
    /// rather than falling back to the compile-time bootstrap keys.
    #[error(
        "Trust root unavailable: {provenance} root has {keys} key(s) for a threshold of {threshold} \
         — refusing to verify (no fallback to compiled bootstrap keys)"
    )]
    TrustRootUnavailable {
        provenance: String,
        keys: usize,
        threshold: usize,
    },

    /// The SIGNATURES.json presented for an install does not describe the artifact
    /// that is about to be installed (INC-I-172 F1).
    ///
    /// A maintainer signature covers `"{version}:{sha256(CHECKSUMS.txt)}"`. If those
    /// two operands are read back out of the same SIGNATURES.json that carries the
    /// signatures, the check is circular: a verbatim copy of ANY past genuine
    /// SIGNATURES.json verifies, while an unrelated tarball is installed. This error
    /// is what a broken link in that chain returns; it must always BLOCK.
    #[error(
        "Signature/artifact binding FAILED on `{field}`: SIGNATURES.json says {signed}, but the \
         release being installed has {actual}. A genuine maintainer signature over a DIFFERENT \
         release authorises nothing here — refusing to install."
    )]
    ArtifactBindingMismatch {
        field: &'static str,
        signed: String,
        actual: String,
    },

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

    /// An operator-supplied release-signing argument does not have the shape the
    /// signing message assumes (INC-I-172 M2, AUDIT-P0-011).
    ///
    /// A release signature is raw Ed25519 over `"{version}:{hash}"`, and the maintainer
    /// governance families sign `"add:{pubkey_hex}"`, `"remove:{pubkey_hex}"` and
    /// `"activate:{version}:{epoch}"` — the same interpolation, no domain tag on either
    /// side. Free-form arguments therefore let ONE release-signing invocation mint a
    /// governance authorization for an entirely different intent. This error is what a
    /// mis-shaped argument returns; it must always BLOCK, before any signing.
    #[error(
        "Refusing to sign: `--{field}` must be {expected}, but got `{value}`. A release \
         signature is raw bytes over \"{{version}}:{{hash}}\", and maintainer governance \
         authorizations use the SAME shape (\"add:<64-hex pubkey>\", \"remove:<64-hex \
         pubkey>\", \"activate:<version>:<epoch>\"). An argument outside the expected \
         shape can make one signing command produce a valid authorization for a \
         different action (INC-I-172 AUDIT-P0-011) — check where these arguments came \
         from before retrying."
    )]
    InvalidSigningArgument {
        field: &'static str,
        expected: &'static str,
        value: String,
    },
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
