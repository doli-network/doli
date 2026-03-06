//! Update application and rollback

use crate::{
    current_timestamp, download_binary, verify_hash, veto_deadline, veto_period_ended, Release,
    Result, UpdateError, VETO_THRESHOLD_PERCENT,
};
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{debug, error, info, warn};

/// Get the path to the current running binary
///
/// On Linux, if the binary was replaced via atomic rename while running,
/// `/proc/self/exe` returns the path with ` (deleted)` suffix.
/// We strip that suffix to get the actual install target path.
pub fn current_binary_path() -> Result<PathBuf> {
    let path = std::env::current_exe().map_err(|e| UpdateError::InstallFailed(e.to_string()))?;
    let path_str = path.to_string_lossy();
    if path_str.ends_with(" (deleted)") {
        Ok(PathBuf::from(path_str.trim_end_matches(" (deleted)")))
    } else {
        Ok(path)
    }
}

/// Get the backup path for the current binary
pub fn backup_path() -> Result<PathBuf> {
    let current = current_binary_path()?;
    let backup = current.with_extension("backup");
    Ok(backup)
}

/// Backup the current binary before update
pub async fn backup_current() -> Result<PathBuf> {
    let current = current_binary_path()?;
    let backup = backup_path()?;

    info!("Backing up current binary to {:?}", backup);

    // Remove old backup if exists
    if backup.exists() {
        fs::remove_file(&backup).await?;
    }

    // Copy current to backup
    fs::copy(&current, &backup).await?;

    debug!("Backup created successfully");
    Ok(backup)
}

/// Apply an update
///
/// This function:
/// 1. **SECURITY CHECK**: Verifies veto period has ended
/// 2. **SECURITY CHECK**: Verifies update was approved (not rejected)
/// 3. Downloads the new binary
/// 4. Verifies the hash
/// 5. Backs up the current binary
/// 6. Installs the new binary
/// 7. Sets executable permissions
///
/// # Arguments
/// * `release` - The release to apply
/// * `approved` - Whether the update was approved by the community
/// * `veto_percent` - The percentage of veto votes (if known)
///
/// # Security
/// Updates can ONLY be applied after:
/// - The 7-day veto period has ended
/// - The community has NOT rejected it (< 40% veto)
///
/// This prevents producers from applying potentially malicious updates
/// before the community has a chance to review and veto.
pub async fn apply_update(
    release: &Release,
    approved: bool,
    veto_percent: Option<u8>,
) -> Result<()> {
    info!("Attempting to apply update to version {}", release.version);

    // SECURITY CHECK 1: Veto period must be over
    if !veto_period_ended(release) {
        let deadline = veto_deadline(release);
        let remaining_secs = deadline.saturating_sub(current_timestamp());
        let remaining_hours = remaining_secs / 3600;

        warn!(
            "Cannot apply update v{}: veto period still active ({}h remaining)",
            release.version, remaining_hours
        );

        return Err(UpdateError::VetoPeriodActive {
            remaining_hours,
            message: format!(
                "Update v{} is still in veto period. The community must have the \
                 opportunity to review and veto. Time remaining: {} hours.",
                release.version, remaining_hours
            ),
        });
    }

    // SECURITY CHECK 2: Update must be approved
    if !approved {
        if let Some(pct) = veto_percent {
            if pct >= VETO_THRESHOLD_PERCENT {
                warn!(
                    "Cannot apply update v{}: rejected by community ({}% veto)",
                    release.version, pct
                );
                return Err(UpdateError::RejectedByVeto {
                    veto_percent: pct,
                    threshold: VETO_THRESHOLD_PERCENT,
                });
            }
        }
        warn!("Cannot apply update v{}: not yet approved", release.version);
        return Err(UpdateError::NotApproved);
    }

    info!(
        "Security checks passed. Applying update to version {}",
        release.version
    );

    // 1. Download
    let binary = download_binary(release).await?;

    // 2. Verify hash
    verify_hash(&binary, &release.binary_sha256)?;

    // 3. Backup current
    let _backup = backup_current().await?;

    // 4. Install new binary
    let current = current_binary_path()?;
    install_binary(&binary, &current).await?;

    info!("Update to {} applied successfully", release.version);
    info!("Node will restart to apply changes");

    Ok(())
}

/// Install binary to target path (temp write + atomic rename)
pub async fn install_binary(binary: &[u8], target: &Path) -> Result<()> {
    // On Unix, we need to handle the case where the binary is running
    // Strategy: write to temp, then atomic rename

    let temp_path = target.with_extension("new");

    // Write to temp file
    fs::write(&temp_path, binary).await?;

    // Set executable permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&temp_path).await?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&temp_path, perms).await?;
    }

    // Atomic rename
    fs::rename(&temp_path, target).await?;

    debug!("Binary installed to {:?}", target);
    Ok(())
}

/// Auto-apply an approved update from GitHub
///
/// This is called by the UpdateService after an update is approved (veto period
/// passed without rejection). It bypasses the veto/approval checks in `apply_update()`
/// because those were already verified by the UpdateService.
///
/// Steps:
/// 1. Fetch release info from GitHub (to get tarball URL)
/// 2. Download the tarball
/// 3. Verify SHA-256 hash
/// 4. Extract doli-node binary
/// 5. Backup current binary
/// 6. Install new binary via atomic rename
///
/// Does NOT call `restart_node()` — the caller is responsible for that
/// (because it needs to clean up state before exec()).
pub async fn auto_apply_from_github(version: &str) -> Result<()> {
    info!("Auto-applying approved update v{}...", version);

    // 1. Fetch release info (gets tarball URL + expected hash)
    let release_info = crate::fetch_github_release(Some(version)).await?;

    // 2. Download tarball
    info!("Downloading v{} tarball...", version);
    let tarball = crate::download_from_url(&release_info.tarball_url).await?;

    // 3. Verify hash
    crate::verify_hash(&tarball, &release_info.expected_hash)?;
    info!("Checksum verified for v{}", version);

    // 4. Extract doli-node binary
    let binary = extract_binary_from_tarball(&tarball)?;

    // 5. Backup current
    let _backup = backup_current().await?;

    // 6. Install
    let target = current_binary_path()?;
    install_binary(&binary, &target).await?;

    info!(
        "Auto-apply complete: v{} installed to {:?}",
        version, target
    );
    Ok(())
}

/// Extract a named binary from a .tar.gz tarball
///
/// CI produces tarballs like `doli-node-v0.1.0-x86_64-unknown-linux-gnu.tar.gz`
/// containing entries like `doli-node-v0.1.0-x86_64-unknown-linux-gnu/doli-node`
/// and `doli-node-v0.1.0-x86_64-unknown-linux-gnu/doli`.
/// This function decompresses and finds the entry matching `name`.
pub fn extract_named_binary_from_tarball(tarball: &[u8], name: &str) -> Result<Vec<u8>> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    use tar::Archive;

    let decoder = GzDecoder::new(tarball);
    let mut archive = Archive::new(decoder);

    for entry in archive
        .entries()
        .map_err(|e| UpdateError::InstallFailed(e.to_string()))?
    {
        let mut entry = entry.map_err(|e| UpdateError::InstallFailed(e.to_string()))?;
        let path = entry
            .path()
            .map_err(|e| UpdateError::InstallFailed(e.to_string()))?;

        if path.file_name().map(|n| n == name).unwrap_or(false) {
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .map_err(|e| UpdateError::InstallFailed(e.to_string()))?;
            info!("Extracted {} binary ({} bytes)", name, bytes.len());
            return Ok(bytes);
        }
    }

    Err(UpdateError::InstallFailed(format!(
        "{} binary not found in tarball",
        name
    )))
}

/// Extract the doli-node binary from a .tar.gz tarball
///
/// Convenience wrapper around `extract_named_binary_from_tarball` for "doli-node".
pub fn extract_binary_from_tarball(tarball: &[u8]) -> Result<Vec<u8>> {
    extract_named_binary_from_tarball(tarball, "doli-node")
}

/// Rollback to the backup binary
pub async fn rollback() -> Result<()> {
    let current = current_binary_path()?;
    let backup = backup_path()?;

    if !backup.exists() {
        error!("No backup found at {:?}", backup);
        return Err(UpdateError::InstallFailed("No backup available".into()));
    }

    warn!("Rolling back to previous version");

    // Restore from backup
    fs::copy(&backup, &current).await?;

    info!("Rollback completed");
    Ok(())
}

/// Restart the node process
///
/// This function does not return - it replaces the current process
pub fn restart_node() -> ! {
    info!("Restarting node...");

    let current = match current_binary_path() {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to get binary path for restart: {}", e);
            std::process::exit(1);
        }
    };

    // Get current args (skip the program name)
    let args: Vec<String> = std::env::args().skip(1).collect();

    // On Unix, use exec to replace the process
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(&current).args(&args).exec();
        // exec only returns on error
        error!("Failed to restart: {}", err);
        std::process::exit(1);
    }

    // On Windows, spawn new process and exit
    #[cfg(windows)]
    {
        match std::process::Command::new(&current).args(&args).spawn() {
            Ok(_) => std::process::exit(0),
            Err(e) => {
                error!("Failed to restart: {}", e);
                std::process::exit(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backup_path() {
        // Just verify the function doesn't panic
        let result = current_binary_path();
        assert!(result.is_ok());

        let result = backup_path();
        assert!(result.is_ok());
    }
}
