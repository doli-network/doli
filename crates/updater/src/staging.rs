//! Staged release handoff for a non-writable install target (INC-I-215).
//!
//! Lays a VERIFIED release into `{data_dir}/updates/` for a privileged installer. The
//! staged operand is the signed `.tar.gz`, never the extracted binary: the signature chain
//! is SIGNATURES.json -> sha256(CHECKSUMS.txt) -> per-platform hash -> tarball, so an
//! extracted ELF leaves the installer nothing to re-verify (INV-REL-001).

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tracing::{debug, info};

use crate::types::{Result, SignaturesFile, UpdateError};

/// Directory under `data_dir` that holds the staged release.
pub const STAGING_SUBDIR: &str = "updates";

/// Marker file published last; its content is the staged version.
pub const READY_MARKER: &str = "ready";

/// Staged copy of the maintainer signature manifest.
pub const STAGED_SIGNATURES: &str = "SIGNATURES.json";

/// Staged copy of the CHECKSUMS.txt the signatures cover.
pub const STAGED_CHECKSUMS: &str = "CHECKSUMS.txt";

/// Staged release tarball.
pub const STAGED_TARBALL: &str = "release.tar.gz";

/// Counter that keeps two probe names in the same process distinct.
static PROBE_SEQ: AtomicU64 = AtomicU64::new(0);

/// A staged release read back off disk.
#[derive(Clone, Debug)]
pub struct StagedRelease {
    /// Version named by the ready marker.
    pub version: String,
    /// The staged `.tar.gz` bytes.
    pub tarball: Vec<u8>,
    /// The staged CHECKSUMS.txt bytes.
    pub checksums_body: Vec<u8>,
    /// The staged maintainer manifest.
    pub signatures: SignaturesFile,
}

/// Staging directory for `data_dir`.
pub fn staging_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(STAGING_SUBDIR)
}

/// Whether `target` can be installed in place.
///
/// Creates and removes a uniquely named file in `target.parent()`, mirroring the
/// temp-write + rename `install_binary` performs. ANY error answers `false` — EROFS on a
/// read-only mount and EACCES on a sandboxed unit arrive through the same channel, so
/// matching on one error kind misses the other. Never panics, never returns `Err`, and
/// leaves no artifact on any path.
pub fn target_dir_is_writable(target: &Path) -> bool {
    let Some(parent) = target.parent() else {
        return false;
    };
    let seq = PROBE_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let probe = parent.join(format!(
        ".doli-writable-probe-{}-{}-{}",
        std::process::id(),
        nanos,
        seq
    ));
    match File::create(&probe) {
        Ok(_) => fs::remove_file(&probe).is_ok(),
        Err(_) => false,
    }
}

/// Version named by the ready marker, or `None` when nothing is staged.
pub fn staged_ready_version(staging_dir: &Path) -> Option<String> {
    let text = fs::read_to_string(staging_dir.join(READY_MARKER)).ok()?;
    let version = text.trim();
    if version.is_empty() {
        None
    } else {
        Some(version.to_string())
    }
}

/// Stage a verified release and return the path of the published ready marker.
///
/// Each file is written to `<name>.tmp`, fsynced and renamed into place; the marker is
/// renamed in LAST, so a watcher that fires on it never sees a partial handoff. A staging
/// for another version is replaced: the marker is unpublished before the first write.
pub fn stage_release(
    staging_dir: &Path,
    version: &str,
    tarball: &[u8],
    checksums_body: &[u8],
    signatures: &SignaturesFile,
) -> Result<PathBuf> {
    fs::create_dir_all(staging_dir)?;

    let stale = staging_dir.join(READY_MARKER);
    let _ = fs::remove_file(&stale);

    publish(staging_dir, STAGED_TARBALL, tarball)?;
    publish(staging_dir, STAGED_CHECKSUMS, checksums_body)?;
    let manifest = serde_json::to_vec_pretty(signatures)?;
    publish(staging_dir, STAGED_SIGNATURES, &manifest)?;
    let marker = publish(staging_dir, READY_MARKER, format!("{version}\n").as_bytes())?;

    info!(
        "Staged release v{} for a privileged installer in {:?} ({} tarball bytes)",
        version,
        staging_dir,
        tarball.len()
    );
    Ok(marker)
}

/// Read a complete staging back.
///
/// Every refusal names the file that is missing, and a ready marker that disagrees with
/// the manifest is refused: the installer must not install a build the marker never named.
pub fn read_staged(staging_dir: &Path) -> Result<StagedRelease> {
    let version = staged_ready_version(staging_dir)
        .ok_or_else(|| missing(staging_dir, READY_MARKER, "no staged version marker"))?;
    let tarball = read_staged_file(staging_dir, STAGED_TARBALL)?;
    let checksums_body = read_staged_file(staging_dir, STAGED_CHECKSUMS)?;
    let manifest_bytes = read_staged_file(staging_dir, STAGED_SIGNATURES)?;

    let signatures: SignaturesFile = serde_json::from_slice(&manifest_bytes).map_err(|e| {
        UpdateError::InstallFailed(format!(
            "staged {} in {:?} is not a readable manifest: {}",
            STAGED_SIGNATURES, staging_dir, e
        ))
    })?;

    if signatures.version.trim() != version {
        return Err(UpdateError::InstallFailed(format!(
            "staged {} names v{} but {} is for v{} — refusing an unapproved handoff",
            READY_MARKER, version, STAGED_SIGNATURES, signatures.version
        )));
    }

    debug!("Read staged release v{} from {:?}", version, staging_dir);
    Ok(StagedRelease {
        version,
        tarball,
        checksums_body,
        signatures,
    })
}

/// Write `bytes` to `<dir>/<name>.tmp`, fsync it and rename it onto `<dir>/<name>`.
fn publish(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf> {
    let final_path = dir.join(name);
    let tmp_path = dir.join(format!("{name}.tmp"));

    let mut file = File::create(&tmp_path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);

    fs::rename(&tmp_path, &final_path)?;
    Ok(final_path)
}

fn read_staged_file(dir: &Path, name: &str) -> Result<Vec<u8>> {
    fs::read(dir.join(name)).map_err(|e| missing(dir, name, &e.to_string()))
}

fn missing(dir: &Path, name: &str, reason: &str) -> UpdateError {
    UpdateError::InstallFailed(format!(
        "incomplete staging in {:?}: cannot read {} ({})",
        dir, name, reason
    ))
}
