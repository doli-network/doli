//! Binary download and verification

use crate::{platform_identifier, Release, Result, UpdateError, UPDATE_MIRRORS};
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{debug, info, warn};

/// Download the binary for the current platform
pub async fn download_binary(release: &Release) -> Result<Vec<u8>> {
    let platform = platform_identifier();
    let url = release.binary_url_template.replace("{platform}", platform);

    info!("Downloading {} for {}", release.version, platform);

    // Try primary URL first, then mirrors
    let mut urls_to_try = vec![url.clone()];
    for mirror in UPDATE_MIRRORS {
        urls_to_try.push(format!(
            "{}/v{}/doli-node-{}",
            mirror, release.version, platform
        ));
    }

    let mut last_error = None;

    for (i, url) in urls_to_try.iter().enumerate() {
        debug!("Trying download from: {}", url);

        match download_from_url(url).await {
            Ok(bytes) => {
                info!(
                    "Downloaded {} bytes from {}",
                    bytes.len(),
                    if i == 0 { "primary" } else { "mirror" }
                );
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

/// Fetch the latest release metadata from update server
pub async fn fetch_latest_release(custom_url: Option<&str>) -> Result<Option<Release>> {
    let base_urls: Vec<&str> = if let Some(url) = custom_url {
        vec![url]
    } else {
        UPDATE_MIRRORS.to_vec()
    };

    for base_url in base_urls {
        let url = format!("{}/latest.json", base_url);
        debug!("Checking for updates at: {}", url);

        match fetch_release_from_url(&url).await {
            Ok(release) => return Ok(Some(release)),
            Err(e) => {
                warn!("Failed to fetch from {}: {}", url, e);
                continue;
            }
        }
    }

    // All mirrors failed
    warn!("Could not fetch release info from any mirror");
    Ok(None)
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

/// Save binary to a temporary file for verification
pub async fn save_to_temp(binary: &[u8], version: &str) -> Result<std::path::PathBuf> {
    let temp_dir = std::env::temp_dir();
    let temp_path = temp_dir.join(format!("doli-node-{}.tmp", version));

    let mut file = fs::File::create(&temp_path).await?;
    file.write_all(binary).await?;
    file.sync_all().await?;

    debug!("Saved binary to: {:?}", temp_path);
    Ok(temp_path)
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
