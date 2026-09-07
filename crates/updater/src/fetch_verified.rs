//! Fetch + verify prefix of the GitHub auto-apply path (INC-I-215 REQ-215-016).
//!
//! Lifted verbatim out of `apply::auto_apply_from_github` so the staged path and the
//! in-process install share ONE fetch/verify implementation instead of two that drift.

use tracing::{error, info};

use crate::types::{Result, UpdateError};

/// A release whose tarball is bound to the signed CHECKSUMS.txt.
pub struct VerifiedRelease {
    /// Release tarball bytes, hash-checked against `platform_hash`.
    pub tarball: Vec<u8>,
    /// Raw CHECKSUMS.txt bytes the maintainer signatures cover.
    pub checksums_body: Vec<u8>,
    /// Per-platform tarball hash parsed from CHECKSUMS.txt.
    pub platform_hash: String,
    /// Version taken from the release TAG.
    pub version: String,
}

/// Fetch a GitHub release and verify its tarball against the signed CHECKSUMS.txt.
///
/// `signed_checksums_sha256` is the value maintainer signatures cover. Comparing it to
/// the freshly-fetched CHECKSUMS.txt closes the TOCTOU window (AUDIT-UPDATE-002): without
/// it a release modified after signing would be installed. The tarball is then checked
/// against the per-platform hash from that same file, not against sha256(CHECKSUMS.txt)
/// (AUDIT-UPDATE-005).
pub async fn fetch_verified_release(
    version: &str,
    signed_checksums_sha256: &str,
) -> Result<VerifiedRelease> {
    let release_info = crate::fetch_github_release(Some(version)).await?;

    if !release_info
        .checksums_sha256
        .eq_ignore_ascii_case(signed_checksums_sha256)
    {
        error!(
            "CHECKSUMS.txt integrity failure: signed={}, fetched={}. \
             Possible TOCTOU attack — GitHub release may have been modified after signing.",
            signed_checksums_sha256, release_info.checksums_sha256
        );
        return Err(UpdateError::HashMismatch {
            expected: signed_checksums_sha256.to_string(),
            actual: release_info.checksums_sha256.clone(),
        });
    }
    info!("CHECKSUMS.txt integrity verified against signed hash");

    info!("Downloading v{} tarball...", version);
    let tarball = crate::download_from_url(&release_info.tarball_url).await?;

    crate::verify_hash(&tarball, &release_info.expected_hash)?;
    info!("Tarball checksum verified for v{}", version);

    Ok(VerifiedRelease {
        tarball,
        checksums_body: release_info.checksums_body,
        platform_hash: release_info.expected_hash,
        version: release_info.version,
    })
}
