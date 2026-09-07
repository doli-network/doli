//! `doli upgrade --from-staged <DIR>` — the privileged half of the INC-I-215 handoff.
//!
//! The node stages a release it verified but could not install, and this runs as root to
//! install it. The staging dir is written by the unprivileged `doli` process and read
//! here by root (TB-1), so nothing in it is trusted on sight: the one shared L1-L4 gate
//! is re-run against this host's on-chain trust root before any byte reaches a target.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use crate::cmd_upgrade::resolve_upgrade_trust_root;
use crate::upgrade_restart::{find_doli_node_path, restart_doli_service, restart_specific_service};

/// Marker name after a refusal: preserves the evidence, disarms the trigger.
const REJECTED_MARKER: &str = "ready.rejected";

/// Marker name after an install whose service restart did not succeed.
const INSTALLED_MARKER: &str = "ready.installed";

/// How a staged install ended. The variant decides what happens to the `ready` marker.
enum StagedFailure {
    /// Nothing was installed.
    Refused(anyhow::Error),
    /// The binary is in place; the service did not come back.
    RestartFailed(anyhow::Error),
}

impl From<anyhow::Error> for StagedFailure {
    fn from(err: anyhow::Error) -> Self {
        StagedFailure::Refused(err)
    }
}

/// Install a release the node staged into `dir`, after re-verifying it.
///
/// Every exit renames or removes the `ready` marker. M4 arms a systemd `.path` unit on
/// `PathExists=<dir>/ready`, so a marker left in place retriggers the oneshot in a tight
/// loop until TriggerLimit fails the unit. The three arms below are the only places that
/// touch it, and none of them can be skipped.
pub(crate) async fn cmd_upgrade_from_staged(
    dir: PathBuf,
    yes: bool,
    doli_node_path: Option<PathBuf>,
    service: Option<String>,
    data_dir: Option<PathBuf>,
    network: doli_core::Network,
) -> Result<()> {
    match staged_install(&dir, yes, doli_node_path, service, data_dir, network).await {
        Ok(version) => {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                println!(
                    "Note: could not remove the staging dir {}: {}",
                    dir.display(),
                    e
                );
                rename_marker(&dir, INSTALLED_MARKER);
            }
            println!();
            println!("Upgrade to v{} complete!", version);
            Ok(())
        }
        Err(StagedFailure::Refused(e)) => {
            rename_marker(&dir, REJECTED_MARKER);
            Err(e)
        }
        Err(StagedFailure::RestartFailed(e)) => {
            rename_marker(&dir, INSTALLED_MARKER);
            Err(e)
        }
    }
}

/// Read, re-verify and install the staged release; return the version installed.
///
/// Nothing here removes or renames anything in `dir`, and no refusal path touches the
/// install target: a refused install leaves the existing binary byte-identical
/// (INC-I-153).
async fn staged_install(
    dir: &Path,
    yes: bool,
    doli_node_path: Option<PathBuf>,
    service: Option<String>,
    data_dir: Option<PathBuf>,
    network: doli_core::Network,
) -> std::result::Result<String, StagedFailure> {
    let data_dir = match data_dir {
        Some(d) => d,
        None => crate::paths::resolve_base_dir(network.name(), None),
    };

    let staged = updater::read_staged(dir)
        .map_err(|e| anyhow!("Refusing the staged handoff in {}: {}", dir.display(), e))?;
    println!("Staged release: v{} in {}", staged.version, dir.display());

    // A load failure is fatal inside `resolve_upgrade_trust_root`; the height read here is
    // best-effort and only feeds the banner, so it cannot mask one (INC-I-206).
    let root = resolve_upgrade_trust_root(&data_dir, network)?;
    println!(
        "Trust root: {} ({} key(s), threshold {}, {}) from {} (on-chain last_derived_height {})",
        root.provenance(),
        root.keys().len(),
        root.threshold(),
        network,
        data_dir.display(),
        on_chain_height(&data_dir)
    );

    let distinct_signers = updater::verify_release_artifact_bytes(
        &staged.version,
        &staged.checksums_body,
        &staged.tarball,
        &staged.signatures,
        &root,
    )
    .map_err(|e| {
        anyhow!(
            "Maintainer verification FAILED for the staged v{} on {}: {}. Refusing to install.",
            staged.version,
            network,
            e
        )
    })?;
    println!(
        "Verified: {} distinct maintainer signature(s) bound to this v{} tarball \
         (threshold {}, {} trust root)",
        distinct_signers,
        staged.version,
        root.threshold(),
        root.provenance()
    );

    let current = updater::current_version();
    if !updater::is_newer_version(&staged.version, current) {
        return Err(anyhow!(
            "Staged v{} is not newer than the installed v{}. Refusing: a stale staging dir \
             left by a crashed helper must never reinstall an older build.",
            staged.version,
            current
        )
        .into());
    }

    let node_bytes = updater::extract_binary_from_tarball(&staged.tarball).map_err(|e| {
        anyhow!(
            "The staged v{} tarball carries no doli-node binary: {}",
            staged.version,
            e
        )
    })?;
    let node_target = doli_node_path.or_else(find_doli_node_path).ok_or_else(|| {
        anyhow!("No doli-node binary found on this host. Name it with --doli-node-path <PATH>.")
    })?;
    if !updater::target_dir_is_writable(&node_target) {
        return Err(anyhow!(
            "Cannot write {}: its directory is not writable by this process.\n  \
             Try: sudo doli upgrade --from-staged {}{}",
            node_target.display(),
            dir.display(),
            if yes { " --yes" } else { "" }
        )
        .into());
    }

    back_up(&node_target)?;
    install(&node_bytes, &node_target).await?;
    println!("Installed doli-node to {}", node_target.display());

    // The CLI ships in the same tarball on a real release, but not in every one, and the
    // running `doli` is not always replaceable. Best-effort, exactly as `doli upgrade`.
    if let Ok(cli_bytes) = updater::extract_named_binary_from_tarball(&staged.tarball, "doli") {
        match std::env::current_exe() {
            Ok(cli_target) if updater::target_dir_is_writable(&cli_target) => {
                install(&cli_bytes, &cli_target).await?;
                println!("Installed doli to {}", cli_target.display());
            }
            _ => println!("Skipping the doli CLI: its install target is not writable."),
        }
    }

    let restarted = match service {
        Some(ref svc) => restart_specific_service(svc),
        None => restart_doli_service(Some(node_target.as_path())),
    };
    if !restarted {
        return Err(StagedFailure::RestartFailed(anyhow!(
            "Installed v{} from {} but the service did not restart. The staged files are \
             kept for inspection; restart the unit by hand.",
            staged.version,
            dir.display()
        )));
    }

    Ok(staged.version)
}

/// Rename `<dir>/ready` to `<dir>/<to>` when it is there.
fn rename_marker(dir: &Path, to: &str) {
    let marker = dir.join(updater::READY_MARKER);
    if !marker.exists() {
        return;
    }
    if let Err(e) = std::fs::rename(&marker, dir.join(to)) {
        eprintln!(
            "Warning: could not rename {} to {}: {}",
            marker.display(),
            to,
            e
        );
    }
}

/// `last_derived_height` of this host's maintainer state, for the banner only.
fn on_chain_height(data_dir: &Path) -> String {
    match storage::MaintainerState::load(data_dir) {
        Ok(state) => state.last_derived_height.to_string(),
        Err(_) => "unavailable".to_string(),
    }
}

/// Copy `target` aside as `<target>.backup` before it is replaced.
///
/// Not `updater::backup_current()`: that backs up the RUNNING binary, which on this path
/// is the CLI, not the node, and would leave the node with no rollback copy.
fn back_up(target: &Path) -> Result<()> {
    if !target.exists() {
        return Ok(());
    }
    let backup = PathBuf::from(format!("{}.backup", target.display()));
    std::fs::copy(target, &backup).map(|_| ()).map_err(|e| {
        anyhow!(
            "Failed to back up {} to {}: {}",
            target.display(),
            backup.display(),
            e
        )
    })
}

/// Temp-write + atomic rename onto `target`, through the shared installer.
async fn install(bytes: &[u8], target: &Path) -> Result<()> {
    updater::install_binary(bytes, target)
        .await
        .map_err(|e| anyhow!("Failed to install {}: {}", target.display(), e))
}
