//! Update application and rollback

use crate::extract::{extract_binary_from_tarball, extract_named_binary_from_tarball};
use crate::fetch_verified::fetch_verified_release;
use crate::{
    current_timestamp, verify_release_with_trust_root, veto_deadline, veto_period_ended, Release,
    Result, TrustRoot, UpdateError, VETO_THRESHOLD_PERCENT,
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

/// Apply an update: veto period ended, community approved, and the release re-verified
/// against the CURRENT trust root, then download + install via [`auto_apply_from_github`].
///
/// # Arguments
/// * `first_notified_at` - NODE-LOCAL time this node first observed the release. The veto
///   window is measured from here, never from the unsigned `published_at` (INC-I-172 F7(b)).
/// * `root` - The CURRENT release-verification trust root. **Required, not optional**: a
///   pending update staged before a key rotation used to install under the revoked signers
///   (INC-I-172 F2), and an argument the compiler demands cannot be forgotten.
pub async fn apply_update(
    release: &Release,
    first_notified_at: u64,
    approved: bool,
    veto_percent: Option<u8>,
    root: &TrustRoot,
) -> Result<()> {
    info!("Attempting to apply update to version {}", release.version);

    // SECURITY CHECK 1: Veto period must be over
    if !veto_period_ended(first_notified_at) {
        let deadline = veto_deadline(first_notified_at);
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

    // SECURITY CHECK 3 (INC-I-172 F2): re-verify against the CURRENT trust root.
    // A pending update is restored from `pending_update.json` across any number of
    // restarts, so the root that authorised it may since have been rotated. This is
    // the same gate `UpdateService::auto_apply` performs; both must exist or the
    // manual command is a bypass of the automatic one.
    let distinct_signers = verify_release_with_trust_root(release, root).inspect_err(|e| {
        error!(
            "Refusing to apply v{}: the staged release does not verify against the current {} \
             trust root ({} keys, threshold {}): {}",
            release.version,
            root.provenance(),
            root.keys().len(),
            root.threshold(),
            e
        );
    })?;

    info!(
        "Security checks passed ({} distinct signer(s) under the {} trust root). Applying \
         update to version {}",
        distinct_signers,
        root.provenance(),
        release.version
    );

    // Download + install through the SAME artifact-bound path the auto-updater uses.
    //
    // `release.binary_sha256` is sha256(CHECKSUMS.txt) — the value the maintainer
    // signatures cover (`SignaturesFile::checksums_sha256`), NOT the hash of any
    // binary. The code this replaced called `verify_hash(&binary, &release.binary_sha256)`,
    // comparing a downloaded binary against the hash of a text file: two different
    // objects, so the check could never pass for a GitHub-sourced release. Routing
    // through `auto_apply_from_github` fixes the operand and closes the chain
    // signatures → CHECKSUMS.txt → per-platform hash → tarball.
    auto_apply_from_github(&release.version, &release.binary_sha256).await?;

    info!("Update to {} applied successfully", release.version);
    info!("Node will restart to apply changes");

    Ok(())
}

/// Install binary to target path (temp write + atomic rename)
///
/// Tries direct write first. If Permission denied (e.g., /usr/local/bin/ owned by root
/// but process runs as `doli` user), falls back to `sudo cp` for the install step.
/// This handles both cases:
/// - Self-owned binary path (e.g., /mainnet/bin/) → direct write
/// - Root-owned binary path (e.g., /usr/local/bin/, /usr/bin/) → sudo fallback
pub async fn install_binary(binary: &[u8], target: &Path) -> Result<()> {
    let temp_path = target.with_extension("new");

    // Try direct write first
    match fs::write(&temp_path, binary).await {
        Ok(()) => {
            // Set executable permissions. Same mode as the sudo fallback installs —
            // the two branches of this function MUST agree (INC-I-153).
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&temp_path).await?.permissions();
                perms.set_mode(INSTALLED_BINARY_MODE);
                fs::set_permissions(&temp_path, perms).await?;
            }
            // Atomic rename
            fs::rename(&temp_path, target).await?;
            debug!("Binary installed to {:?} (direct)", target);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            // Fallback: write to /tmp, then sudo cp to target
            info!("Direct write to {:?} denied, using sudo fallback", target);
            install_binary_sudo(binary, target).await
        }
        Err(e) => Err(e.into()),
    }
}

/// Path used for staging the new binary before the privileged `sudo cp`.
///
/// ISSUE-174 #7: previously this was `/tmp/doli-update-binary`. The world-writable
/// `/tmp` plus a predictable filename allowed any local user to win a TOCTOU race
/// between our `fs::write` and the `sudo cp` that follows, gaining root code execution
/// when the auto-updater fired. The new path lives inside `/var/lib/doli/` (created
/// mode 2770 doli:doli by the installer), and the file is opened with `O_NOFOLLOW`
/// to defeat symlink swaps from inside the trusted `doli` group.
const STAGED_BINARY_PATH: &str = "/var/lib/doli/update.bin";

/// Mode every installed DOLI binary must carry: `rwxr-xr-x`.
///
/// INC-I-153: the systemd unit runs the node as `User=doli`, while the privileged
/// install leaves the file `root:root`. The service account is therefore neither the
/// owner nor in the group, so `execve` is decided by the OTHER-execute bit alone.
/// Both branches of [`install_binary`] install this same mode; nothing may install a
/// binary whose mode is not executable by others.
#[cfg(unix)]
const INSTALLED_BINARY_MODE: u32 = 0o755;

/// Install binary via sudo (fallback for root-owned paths like /usr/local/bin/)
///
/// Stages the binary at `/var/lib/doli/update.bin` (doli:doli, 2770, opened with
/// `O_NOFOLLOW`), then installs it using the only two privileged verbs the sudoers
/// whitelist grants: `sudo rm -f <target>` followed by `sudo cp <staged> <target>`.
/// There is deliberately NO privileged mode change — install.sh / postinst.sh whitelist
/// exactly two `rm -f` and two `cp` invocations, so any other privileged verb is denied
/// on every already-deployed host.
///
/// Because the `rm -f` unlinks the target first, the `cp` always takes its CREATE path,
/// where the new inode's mode is `staged_mode & ~umask` — and sudo's effective umask
/// (`caller | sudoers Defaults umask`) is not under this process's control. Staging with
/// the other-execute bit set is therefore NECESSARY BUT NOT SUFFICIENT, so the function
/// ends by reading the installed mode back off disk and returning
/// `UpdateError::InstallFailed` unless the target is executable by a user who is neither
/// its owner nor in its group (see [`INSTALLED_BINARY_MODE`]).
///
/// INC-I-153: no postcondition on the installed target has ever existed here — the read-back
/// above is the first. What `5a9414cf` deleted was a best-effort `sudo chmod 755 <target>`
/// run after the copy, a corrective action whose failure only produced
/// `warn!("sudo chmod failed, binary may not be executable")`; it was replaced by a mode set
/// on the staged file BEFORE the copy. From that point the installed mode was an inherited
/// coincidence with nothing verifying it, and `857746b6` tightening the staged mode to
/// `0o750` silently installed a binary the service account could not exec —
/// `status=203/EXEC` on a mainnet producer.
///
/// Works for any target path as long as the user has passwordless sudo (standard for the
/// `doli` group). The sudoers rule MUST list this same staging path; see install.sh /
/// postinst.sh.
async fn install_binary_sudo(binary: &[u8], target: &Path) -> Result<()> {
    use std::process::Command;

    let staged = PathBuf::from(STAGED_BINARY_PATH);

    // Make sure the parent dir exists. On a properly installed Linux system this
    // was created by install.sh / postinst.sh with mode 2770 doli:doli. If we
    // create it here as a fallback (single-user dev box), the inherited perms
    // come from the parent. The directory ownership/mode is the operator's lever;
    // this code does not try to fix a broken layout.
    if let Some(parent) = staged.parent() {
        fs::create_dir_all(parent).await.map_err(|e| {
            UpdateError::InstallFailed(format!("Failed to create staging dir {:?}: {}", parent, e))
        })?;
    }

    // Remove any stale staging file from a previous run before we open with
    // O_NOFOLLOW. This narrows the symlink-swap window to a single syscall.
    let _ = fs::remove_file(&staged).await;

    // Open with O_NOFOLLOW: refuses to follow a symlink at the final path
    // component, blocking the classic /tmp-style symlink attack.
    //
    // INC-I-153: the staged mode is NOT free to choose. `sudo cp` onto the unlinked
    // target propagates `staged_mode & ~umask`, and masking can only clear bits, never
    // add them — so the staged file must already carry the other-execute bit that the
    // installed binary needs (0o755, INSTALLED_BINARY_MODE). This does not weaken the
    // ISSUE-174 closures: those are the staging DIRECTORY (/var/lib/doli, 2770 doli:doli,
    // which "other" cannot even traverse) and O_NOFOLLOW on the open, both independent of
    // the file's own permission bits. No write bit is granted to group or other here.
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let mut opts = std::fs::OpenOptions::new();
        opts.write(true)
            .create(true)
            .truncate(true)
            .mode(0o755)
            .custom_flags(libc::O_NOFOLLOW);
        let mut f = opts.open(&staged).map_err(|e| {
            UpdateError::InstallFailed(format!("Failed to stage binary at {:?}: {}", staged, e))
        })?;
        f.write_all(binary).map_err(|e| {
            UpdateError::InstallFailed(format!("Failed to write staged binary: {}", e))
        })?;
        f.sync_all().ok();

        // chmod(2), not umask-filtered: the staged mode is exact in every environment.
        let mut perms = std::fs::metadata(&staged)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&staged, perms)?;
    }
    #[cfg(not(unix))]
    {
        fs::write(&staged, binary).await?;
    }

    // On Linux, overwriting a running binary with `cp` fails with "Text file busy".
    // Fix: delete the old binary first (the running process keeps its inode open),
    // then copy the new one to the now-free path.
    let _ = Command::new("sudo")
        .args(["rm", "-f", &target.to_string_lossy()])
        .status();

    let cp_status = Command::new("sudo")
        .args(["cp", &staged.to_string_lossy(), &target.to_string_lossy()])
        .status()
        .map_err(|e| UpdateError::InstallFailed(format!("sudo cp failed: {}", e)))?;

    if !cp_status.success() {
        let _ = fs::remove_file(&staged).await;
        return Err(UpdateError::InstallFailed(format!(
            "sudo cp to {:?} failed with exit code {:?}",
            target,
            cp_status.code()
        )));
    }

    // Cleanup staged file. From here on the target already holds the new bytes, so the
    // staging copy is dead weight on every remaining exit, success or failure.
    let _ = fs::remove_file(&staged).await;

    // POSTCONDITION (INC-I-153). A zero-exit `cp` proves the BYTES landed; it proves
    // nothing about the MODE they landed with. The installed mode is `staged_mode & ~umask`
    // where umask is `caller | sudoers Defaults umask` — a value this process cannot read
    // or set through sudo. Staging at 0o755 is necessary but not sufficient: a site with
    // `Defaults umask=0027` still yields 0o750 and reproduces the identical brick. The only
    // trustworthy evidence is the mode of the file on disk, so read it back and fail loudly
    // rather than logging "Binary installed" over a target the service account cannot exec.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        // Belt: when this process owns the installed file it can correct the mode itself
        // with chmod(2) — exact, umask-independent and needing no privilege at all. On the
        // normal path `sudo cp` left the file root:root and this is refused with EPERM,
        // which is exactly why it is not the guarantee. Its outcome is kept for diagnosis.
        let self_chmod = std::fs::set_permissions(
            target,
            std::fs::Permissions::from_mode(INSTALLED_BINARY_MODE),
        );

        let installed_mode = std::fs::metadata(target)
            .map_err(|e| {
                UpdateError::InstallFailed(format!(
                    "installed {:?} but could not stat it to verify its mode: {}",
                    target, e
                ))
            })?
            .permissions()
            .mode()
            & 0o7777;

        // The condition is exactly S_IXOTH, nothing more. `sudo cp` leaves the file
        // root:root while systemd runs the node as `User=doli`, so the service account
        // falls in the OTHER class and `execve` consults the other-execute bit alone.
        // Requiring u+x and g+x as well would reject modes the account can in fact run
        // (a umask clearing only g+x installs 0o745, which `doli` executes fine) and
        // would abort the upgrade after the target has already been replaced.
        if installed_mode & 0o001 == 0 {
            let chmod_note = match self_chmod {
                Ok(()) => "in-process chmod reported success".to_string(),
                Err(e) => format!("in-process chmod was refused: {}", e),
            };
            return Err(UpdateError::InstallFailed(format!(
                "installed {:?} at mode {:o}: the other-execute bit (0o001) is clear, so the \
                 file cannot be execve'd by a user who is neither its owner nor in its group \
                 ({}). The node runs as `User=doli` under systemd while the privileged copy \
                 leaves the file root:root, so this binary would fail to execve with \
                 status=203/EXEC. Recover with: sudo chmod {:o} {}",
                target,
                installed_mode,
                chmod_note,
                INSTALLED_BINARY_MODE,
                target.display()
            )));
        }

        info!(
            "Binary installed to {:?} (via sudo), mode {:o} verified",
            target, installed_mode
        );
    }

    #[cfg(not(unix))]
    {
        info!("Binary installed to {:?} (via sudo)", target);
    }

    Ok(())
}

/// Auto-apply an approved update from GitHub
///
/// Called by the UpdateService once an update is approved, so the veto/approval checks of
/// `apply_update()` are already done. Fetch + verification live in
/// [`fetch_verified_release`]; this function extracts, backs up and installs.
///
/// `signed_checksums_sha256` is the SHA-256 of CHECKSUMS.txt the maintainer signatures
/// cover. It anchors the chain signatures -> CHECKSUMS.txt -> per-platform hash -> tarball.
///
/// Does NOT call `restart_node()` — the caller cleans up state before exec().
pub async fn auto_apply_from_github(version: &str, signed_checksums_sha256: &str) -> Result<()> {
    info!("Auto-applying approved update v{}...", version);

    // 1-3. Fetch the release, bind CHECKSUMS.txt to the signed hash, download and
    //      hash-check the tarball.
    let tarball = fetch_verified_release(version, signed_checksums_sha256)
        .await?
        .tarball;

    // 4. Extract doli-node binary
    let binary = extract_binary_from_tarball(&tarball)?;

    // 5. Backup current
    let _backup = backup_current().await?;

    // 6. Install doli-node
    let target = current_binary_path()?;
    install_binary(&binary, &target).await?;
    info!("doli-node installed to {:?}", target);

    // 7. Also update the CLI binary (doli) — it's in the same tarball
    //    Best-effort: CLI failure must never block the node update.
    if let Some(dir) = target.parent() {
        let cli_path = dir.join("doli");
        if cli_path.exists() && cli_path != target {
            match extract_named_binary_from_tarball(&tarball, "doli") {
                Ok(cli_binary) => {
                    // Backup CLI
                    let cli_backup = cli_path.with_extension("backup");
                    if cli_backup.exists() {
                        let _ = fs::remove_file(&cli_backup).await;
                    }
                    let _ = fs::copy(&cli_path, &cli_backup).await;

                    match install_binary(&cli_binary, &cli_path).await {
                        Ok(()) => info!("doli CLI also updated to v{} at {:?}", version, cli_path),
                        Err(e) => warn!(
                            "Failed to update doli CLI at {:?}: {} (non-fatal)",
                            cli_path, e
                        ),
                    }
                }
                Err(e) => warn!("doli CLI not found in tarball: {} (non-fatal)", e),
            }
        } else if !cli_path.exists() {
            debug!("No doli CLI found at {:?}, skipping CLI update", cli_path);
        }
    }

    // 8. Update agent skills (best-effort — skill failure never blocks node update)
    match crate::skills::install_skills_from_tarball(&tarball) {
        Ok(count) if count > 0 => info!("Updated {} agent skills", count),
        Ok(_) => debug!("No skills found in tarball"),
        Err(e) => warn!("Failed to update agent skills: {} (non-fatal)", e),
    }

    info!(
        "Auto-apply complete: v{} installed to {:?}",
        version, target
    );
    Ok(())
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
