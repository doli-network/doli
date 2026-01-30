//! Binary download and verification
//!
//! Downloads DOLI binaries from GitHub Releases (primary) or fallback mirror.
//! GitHub Releases provides:
//! - Global CDN for fast downloads
//! - High availability (99.9%+ uptime)
//! - Verifiable history
//! - Free hosting

use crate::{
    platform_identifier, Release, Result, UpdateError, FALLBACK_MIRROR, GITHUB_RELEASES_URL,
};
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
async fn download_from_url(url: &str) -> Result<Vec<u8>> {
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
pub async fn fetch_latest_release(custom_url: Option<&str>) -> Result<Option<Release>> {
    // If custom URL provided, use it directly
    if let Some(url) = custom_url {
        debug!("Checking custom URL: {}", url);
        match fetch_release_from_url(&format!("{}/latest.json", url)).await {
            Ok(release) => return Ok(Some(release)),
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
            info!("Found latest release v{} from GitHub", release.version);
            return Ok(Some(release));
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
            info!("Found release v{} from fallback mirror", release.version);
            return Ok(Some(release));
        }
        Err(e) => {
            warn!("Fallback mirror failed: {}", e);
        }
    }

    // All sources failed
    warn!("Could not fetch release info from any source");
    Ok(None)
}

/// Fetch release info from GitHub API
///
/// 1. Get latest release tag from GitHub API
/// 2. Download release.json from that release
async fn fetch_from_github() -> Result<Option<Release>> {
    use crate::GITHUB_API_URL;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("doli-node") // GitHub requires User-Agent
        .build()?;

    // Get latest release info from GitHub API
    let response = client.get(GITHUB_API_URL).send().await?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        // No releases yet
        return Ok(None);
    }

    if !response.status().is_success() {
        return Err(UpdateError::DownloadFailed(format!(
            "GitHub API returned HTTP {}",
            response.status()
        )));
    }

    // Parse GitHub API response to get tag name
    let github_release: serde_json::Value = response.json().await?;
    let tag_name = github_release["tag_name"]
        .as_str()
        .ok_or_else(|| UpdateError::DownloadFailed("No tag_name in GitHub response".into()))?;

    debug!("Latest GitHub release: {}", tag_name);

    // Download release.json from this release
    let release_json_url = format!("{}/{}/release.json", GITHUB_RELEASES_URL, tag_name);
    debug!("Fetching release metadata: {}", release_json_url);

    fetch_release_from_url(&release_json_url).await.map(Some)
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
