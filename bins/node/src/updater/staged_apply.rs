//! Staged handoff for hosts whose install target this process cannot write (INC-I-215).
//!
//! The node fetches and verifies the release exactly as the in-process install does,
//! then leaves it under `{data_dir}/updates` for a privileged installer. It owns no
//! pending-state lifecycle and no process lifecycle: both stay in `service.rs`.

use std::path::{Path, PathBuf};

use tracing::info;
use updater::MaintainerSignature;

/// Whether `{data_dir}/updates` already carries a ready marker naming `version`
/// (REQ-215-005).
pub(super) fn already_staged(data_dir: &Path, version: &str) -> bool {
    updater::staged_ready_version(&updater::staging_dir(data_dir)).as_deref() == Some(version)
}

/// The operator line for the already-staged short circuit (REQ-215-005).
pub(super) fn log_already_staged(data_dir: &Path, version: &str) {
    info!(
        "Update v{} is already staged in {} — waiting for the privileged helper to install it",
        version,
        updater::staging_dir(data_dir).display()
    );
}

/// Whether this process can install over its own binary (REQ-215-003).
///
/// An unresolvable binary path answers `false`: staging leaves the running node intact,
/// while an install attempt on an unknown target cannot succeed.
pub(super) fn install_target_is_writable() -> bool {
    updater::current_binary_path()
        .map(|target| updater::target_dir_is_writable(&target))
        .unwrap_or(false)
}

/// Fetch, verify and stage `version` under `{data_dir}/updates`, returning the published
/// ready-marker path (REQ-215-004, REQ-215-015).
pub(super) async fn stage_for_privileged_install(
    data_dir: &Path,
    version: &str,
    signed_checksums_sha256: &str,
    signatures: &[MaintainerSignature],
) -> updater::Result<PathBuf> {
    let verified = updater::fetch_verified_release(version, signed_checksums_sha256).await?;
    let manifest = updater::SignaturesFile {
        version: version.to_string(),
        checksums_sha256: signed_checksums_sha256.to_string(),
        signatures: signatures.to_vec(),
    };
    let staging = updater::staging_dir(data_dir);
    let marker = updater::stage_release(
        &staging,
        version,
        &verified.tarball,
        &verified.checksums_body,
        &manifest,
    )?;
    info!(
        "Staged update v{} in {} — a privileged helper must finish the install with \
         `doli upgrade --from-staged`",
        version,
        staging.display()
    );
    Ok(marker)
}
