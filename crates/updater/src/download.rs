//! Binary download and verification
//!
//! Downloads DOLI binaries from GitHub Releases (primary) or fallback mirror.
//! GitHub Releases provides:
//! - Global CDN for fast downloads
//! - High availability (99.9%+ uptime)
//! - Verifiable history
//! - Free hosting

use crate::{
    platform_identifier, MaintainerSignature, Release, ReleaseMetadata, Result, SignaturesFile,
    UpdateError, FALLBACK_MIRROR, GITHUB_RELEASES_URL,
};
use doli_core::network::Network;
use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};

/// Download the binary for the current platform
///
/// Tries sources in order:
/// 1. Primary URL from release.binary_url_template
/// 2. GitHub Releases (CDN)
/// 3. Fallback mirror
pub async fn download_binary(release: &Release) -> Result<Vec<u8>> {
    let platform = platform_identifier();
    let url = release.binary_url_template.replace("{platform}", platform);

    info!("Downloading {} for {}", release.version, platform);

    // Build list of URLs to try
    let mut urls_to_try = vec![url.clone()];

    // GitHub Releases URL
    urls_to_try.push(format!(
        "{}/v{}/doli-node-{}",
        GITHUB_RELEASES_URL, release.version, platform
    ));

    // Fallback mirror
    urls_to_try.push(format!(
        "{}/v{}/doli-node-{}",
        FALLBACK_MIRROR, release.version, platform
    ));

    let mut last_error = None;

    for (i, url) in urls_to_try.iter().enumerate() {
        let source = match i {
            0 => "primary",
            1 => "GitHub",
            _ => "fallback",
        };
        debug!("Trying download from {} ({})", source, url);

        match download_from_url(url).await {
            Ok(bytes) => {
                info!("Downloaded {} bytes from {}", bytes.len(), source);
                return Ok(bytes);
            }
            Err(e) => {
                warn!("Download failed from {}: {}", url, e);
                last_error = Some(e);
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| UpdateError::DownloadFailed("All download sources failed".into())))
}

/// Download from a specific URL
pub async fn download_from_url(url: &str) -> Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300)) // 5 min timeout
        .build()?;

    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(UpdateError::DownloadFailed(format!(
            "HTTP {}",
            response.status()
        )));
    }

    let bytes = response.bytes().await?;
    Ok(bytes.to_vec())
}

/// Verify the SHA-256 hash of downloaded binary
pub fn verify_hash(binary: &[u8], expected_hash: &str) -> Result<()> {
    let mut hasher = Sha256::new();
    hasher.update(binary);
    let actual_hash = hex::encode(hasher.finalize());

    if actual_hash.eq_ignore_ascii_case(expected_hash) {
        info!("Binary hash verified: {}...", &actual_hash[..16]);
        Ok(())
    } else {
        Err(UpdateError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        })
    }
}

/// Fetch the latest release metadata
///
/// Tries sources in order:
/// 1. Custom URL (if provided)
/// 2. GitHub API (gets latest release tag, then downloads release.json)
/// 3. Fallback mirror (legacy releases.doli.network/latest.json)
pub async fn fetch_latest_release(
    custom_url: Option<&str>,
    network: Option<Network>,
) -> Result<Option<Release>> {
    // If custom URL provided, use it directly
    if let Some(url) = custom_url {
        debug!("Checking custom URL: {}", url);
        match fetch_release_from_url(&format!("{}/latest.json", url)).await {
            Ok(release) => {
                if let Some(filtered) = filter_release_by_network(release, network) {
                    return Ok(Some(filtered));
                }
                return Ok(None);
            }
            Err(e) => {
                warn!("Failed to fetch from custom URL: {}", e);
                return Ok(None);
            }
        }
    }

    // Try GitHub API first (primary source)
    debug!("Checking GitHub for latest release...");
    match fetch_from_github().await {
        Ok(Some(release)) => {
            if let Some(filtered) = filter_release_by_network(release, network) {
                info!("Found latest release v{} from GitHub", filtered.version);
                return Ok(Some(filtered));
            }
            // Release exists but not for our network
            return Ok(None);
        }
        Ok(None) => {
            debug!("No releases found on GitHub");
        }
        Err(e) => {
            warn!("GitHub API check failed: {}", e);
        }
    }

    // Fallback to legacy mirror
    let fallback_url = format!("{}/latest.json", FALLBACK_MIRROR);
    debug!("Trying fallback mirror: {}", fallback_url);

    match fetch_release_from_url(&fallback_url).await {
        Ok(release) => {
            if let Some(filtered) = filter_release_by_network(release, network) {
                info!("Found release v{} from fallback mirror", filtered.version);
                return Ok(Some(filtered));
            }
            return Ok(None);
        }
        Err(e) => {
            warn!("Fallback mirror failed: {}", e);
        }
    }

    // All sources failed
    warn!("Could not fetch release info from any source");
    Ok(None)
}

/// Filter a release by network. Returns None if the release is not for the given network.
/// Empty target_networks = targets all networks (backward compat for releases without metadata.json).
fn filter_release_by_network(release: Release, network: Option<Network>) -> Option<Release> {
    if let Some(net) = network {
        if !release.target_networks.is_empty()
            && !release
                .target_networks
                .iter()
                .any(|n| n.eq_ignore_ascii_case(net.name()))
        {
            info!(
                "Release v{} not for {} (targets: {:?}), skipping",
                release.version,
                net.name(),
                release.target_networks
            );
            return None;
        }
    }
    Some(release)
}

/// Fetch release info from GitHub API
///
/// Builds a `Release` from GitHub Release assets (no release.json needed):
/// 1. GET /releases/latest from GitHub API
/// 2. Parse tag_name, body (changelog), published_at
/// 3. Find CHECKSUMS.txt asset → download → compute SHA-256 of the file itself
/// 4. Find SIGNATURES.json asset → download → parse as SignaturesFile
/// 5. Construct Release with checksums_sha256 as the signed hash
async fn fetch_from_github() -> Result<Option<Release>> {
    use crate::GITHUB_API_URL;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("doli-node")
        .build()?;

    let response = client.get(GITHUB_API_URL).send().await?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }

    if !response.status().is_success() {
        return Err(UpdateError::DownloadFailed(format!(
            "GitHub API returned HTTP {}",
            response.status()
        )));
    }

    let github_release: serde_json::Value = response.json().await?;
    let tag_name = github_release["tag_name"]
        .as_str()
        .ok_or_else(|| UpdateError::DownloadFailed("No tag_name in GitHub response".into()))?;
    let version = tag_name.strip_prefix('v').unwrap_or(tag_name);
    let changelog = github_release["body"].as_str().unwrap_or("").to_string();
    let published_at = github_release["published_at"]
        .as_str()
        .and_then(parse_iso8601_timestamp)
        .unwrap_or(0);

    debug!(
        "Latest GitHub release: {} (published {})",
        tag_name, published_at
    );

    let assets = github_release["assets"].as_array();

    // Download CHECKSUMS.txt
    let checksums_url = assets
        .and_then(|a| {
            a.iter()
                .find(|a| a["name"].as_str() == Some("CHECKSUMS.txt"))
                .and_then(|a| a["browser_download_url"].as_str())
        })
        .ok_or_else(|| {
            UpdateError::DownloadFailed("CHECKSUMS.txt not found in release assets".into())
        })?
        .to_string();

    let checksums_body = download_from_url(&checksums_url).await?;
    let checksums_sha256 = {
        let mut hasher = Sha256::new();
        hasher.update(&checksums_body);
        hex::encode(hasher.finalize())
    };

    // Try to download SIGNATURES.json (optional — may not exist yet)
    let signatures: Vec<MaintainerSignature> = if let Some(sigs_url) = assets.and_then(|a| {
        a.iter()
            .find(|a| a["name"].as_str() == Some("SIGNATURES.json"))
            .and_then(|a| a["browser_download_url"].as_str())
    }) {
        match download_from_url(sigs_url).await {
            Ok(body) => match serde_json::from_slice::<SignaturesFile>(&body) {
                Ok(sf) => {
                    info!(
                        "SIGNATURES.json: {} signatures for v{}",
                        sf.signatures.len(),
                        sf.version
                    );
                    sf.signatures
                }
                Err(e) => {
                    warn!("Failed to parse SIGNATURES.json: {}", e);
                    vec![]
                }
            },
            Err(e) => {
                warn!("Failed to download SIGNATURES.json: {}", e);
                vec![]
            }
        }
    } else {
        debug!("No SIGNATURES.json in release assets");
        vec![]
    };

    // Try to download metadata.json (optional — may not exist for older releases)
    let target_networks: Vec<String> = if let Some(meta_url) = assets.and_then(|a| {
        a.iter()
            .find(|a| a["name"].as_str() == Some("metadata.json"))
            .and_then(|a| a["browser_download_url"].as_str())
    }) {
        match download_from_url(meta_url).await {
            Ok(body) => match serde_json::from_slice::<ReleaseMetadata>(&body) {
                Ok(meta) => {
                    info!("metadata.json: networks={:?}", meta.networks);
                    meta.networks
                }
                Err(e) => {
                    warn!("Failed to parse metadata.json: {}", e);
                    vec![]
                }
            },
            Err(e) => {
                warn!("Failed to download metadata.json: {}", e);
                vec![]
            }
        }
    } else {
        debug!("No metadata.json in release assets — targeting all networks");
        vec![]
    };

    Ok(Some(Release {
        version: version.to_string(),
        binary_sha256: checksums_sha256,
        binary_url_template: format!(
            "{}/{}/doli-node-{{platform}}",
            GITHUB_RELEASES_URL, tag_name
        ),
        changelog,
        published_at,
        signatures,
        target_networks,
    }))
}

/// Fetch release metadata from a specific URL
async fn fetch_release_from_url(url: &str) -> Result<Release> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(UpdateError::DownloadFailed(format!(
            "HTTP {}",
            response.status()
        )));
    }

    let release: Release = response.json().await?;
    Ok(release)
}

/// Release info fetched directly from GitHub API (no release.json needed)
pub struct GithubReleaseInfo {
    /// Semantic version (without 'v' prefix)
    pub version: String,
    /// Direct download URL for the platform tarball
    pub tarball_url: String,
    /// Expected SHA-256 hash from CHECKSUMS.txt
    pub expected_hash: String,
    /// Release changelog (body from GitHub)
    pub changelog: String,
}

/// Map platform_identifier() to Rust target triple for asset matching
fn platform_target_triple() -> &'static str {
    match platform_identifier() {
        "linux-x64" => "x86_64-unknown-linux-gnu",
        "linux-arm64" => "aarch64-unknown-linux-gnu",
        "macos-x64" => "x86_64-apple-darwin",
        "macos-arm64" => "aarch64-apple-darwin",
        _ => "unknown",
    }
}

/// Fetch release info directly from GitHub API
///
/// Works without release.json — parses the GitHub API response directly:
/// 1. GET /releases/latest (or /releases/tags/v{version})
/// 2. Parse tag_name, body (changelog), assets
/// 3. Find CHECKSUMS.txt asset → download → parse hash for current platform
/// 4. Find tarball asset for current platform
pub async fn fetch_github_release(version: Option<&str>) -> Result<GithubReleaseInfo> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("doli-node")
        .build()?;

    let api_url = match version {
        Some(v) => {
            let tag = if v.starts_with('v') {
                v.to_string()
            } else {
                format!("v{}", v)
            };
            format!(
                "https://api.github.com/repos/{}/releases/tags/{}",
                crate::GITHUB_REPO,
                tag
            )
        }
        None => crate::GITHUB_API_URL.to_string(),
    };

    debug!("Fetching release from: {}", api_url);

    let response = client.get(&api_url).send().await?;
    if !response.status().is_success() {
        return Err(UpdateError::DownloadFailed(format!(
            "GitHub API returned HTTP {}",
            response.status()
        )));
    }

    let release: serde_json::Value = response.json().await?;

    let tag = release["tag_name"]
        .as_str()
        .ok_or_else(|| UpdateError::DownloadFailed("No tag_name in response".into()))?;
    let version_str = tag.strip_prefix('v').unwrap_or(tag);
    let changelog = release["body"].as_str().unwrap_or("").to_string();

    let assets = release["assets"]
        .as_array()
        .ok_or_else(|| UpdateError::DownloadFailed("No assets in release".into()))?;

    // Find CHECKSUMS.txt asset
    let checksums_url = assets
        .iter()
        .find(|a| a["name"].as_str() == Some("CHECKSUMS.txt"))
        .and_then(|a| a["browser_download_url"].as_str())
        .ok_or_else(|| {
            UpdateError::DownloadFailed("CHECKSUMS.txt not found in release assets".into())
        })?;

    // Download and parse CHECKSUMS.txt
    let checksums_body = download_from_url(checksums_url).await?;
    let checksums_text = String::from_utf8_lossy(&checksums_body);

    let triple = platform_target_triple();
    let expected_hash = checksums_text
        .lines()
        .find(|line| line.contains(triple) && line.contains(".tar.gz"))
        .and_then(|line| line.split_whitespace().next())
        .ok_or_else(|| {
            UpdateError::DownloadFailed(format!(
                "No checksum for platform {} in CHECKSUMS.txt",
                triple
            ))
        })?
        .to_string();

    // Find tarball asset for current platform
    let tarball_url = assets
        .iter()
        .find(|a| {
            a["name"]
                .as_str()
                .map(|n| n.contains(triple) && n.ends_with(".tar.gz"))
                .unwrap_or(false)
        })
        .and_then(|a| a["browser_download_url"].as_str())
        .ok_or_else(|| {
            UpdateError::DownloadFailed(format!(
                "No tarball for platform {} in release assets",
                triple
            ))
        })?
        .to_string();

    info!("Found release v{} for {}", version_str, triple);

    Ok(GithubReleaseInfo {
        version: version_str.to_string(),
        tarball_url,
        expected_hash,
        changelog,
    })
}

/// Parse an ISO 8601 timestamp (e.g. "2026-03-01T12:00:00Z") to Unix timestamp.
/// Only handles the `YYYY-MM-DDThh:mm:ssZ` format returned by GitHub API.
fn parse_iso8601_timestamp(s: &str) -> Option<u64> {
    // Minimal parser for GitHub's "2026-03-01T12:00:00Z" format
    let s = s.trim_end_matches('Z');
    let (date_part, time_part) = s.split_once('T')?;
    let mut date_iter = date_part.split('-');
    let year: i64 = date_iter.next()?.parse().ok()?;
    let month: i64 = date_iter.next()?.parse().ok()?;
    let day: i64 = date_iter.next()?.parse().ok()?;

    let mut time_iter = time_part.split(':');
    let hour: i64 = time_iter.next()?.parse().ok()?;
    let min: i64 = time_iter.next()?.parse().ok()?;
    let sec: i64 = time_iter.next()?.parse().ok()?;

    // Days from year 1970 to the given year (simplified, no leap second)
    let mut days: i64 = 0;
    for y in 1970..year {
        days += if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            366
        } else {
            365
        };
    }
    let is_leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [
        31,
        if is_leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    for &d in month_days.iter().take((month - 1) as usize) {
        days += d as i64;
    }
    days += day - 1;

    let timestamp = days * 86400 + hour * 3600 + min * 60 + sec;
    Some(timestamp as u64)
}

/// Download SIGNATURES.json for a specific release version
///
/// Returns `None` if the asset doesn't exist. Used by `doli upgrade` to
/// show signature verification status.
pub async fn download_signatures_json(version: &str) -> Result<Option<SignaturesFile>> {
    let tag = if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{}", version)
    };

    let url = format!("{}/{}/SIGNATURES.json", GITHUB_RELEASES_URL, tag);
    debug!("Fetching SIGNATURES.json: {}", url);

    match download_from_url(&url).await {
        Ok(body) => {
            let sf: SignaturesFile = serde_json::from_slice(&body)?;
            Ok(Some(sf))
        }
        Err(UpdateError::DownloadFailed(msg)) if msg.contains("404") || msg.contains("HTTP") => {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Download CHECKSUMS.txt for a specific release version and return its content + SHA-256
pub async fn download_checksums_txt(version: &str) -> Result<(String, String)> {
    let tag = if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{}", version)
    };

    let url = format!("{}/{}/CHECKSUMS.txt", GITHUB_RELEASES_URL, tag);
    debug!("Fetching CHECKSUMS.txt: {}", url);

    let body = download_from_url(&url).await?;
    let content = String::from_utf8_lossy(&body).to_string();
    let sha256 = {
        let mut hasher = Sha256::new();
        hasher.update(&body);
        hex::encode(hasher.finalize())
    };

    Ok((content, sha256))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_verification() {
        let data = b"test binary content";
        let hash = "8b5b9db0c13db24256c829aa364aa90c6d2eba318b9232a4ab9313b954d3555f";

        // This should fail with wrong hash
        let result = verify_hash(data, hash);
        assert!(result.is_err());

        // Calculate correct hash
        let mut hasher = Sha256::new();
        hasher.update(data);
        let correct_hash = hex::encode(hasher.finalize());

        // This should succeed
        let result = verify_hash(data, &correct_hash);
        assert!(result.is_ok());
    }
}
