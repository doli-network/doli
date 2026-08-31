//! `doli release verify` — offline check of a published release manifest (INC-I-202 M2).

use std::path::Path;

use anyhow::Context;

/// Verify `<dir>/SIGNATURES.json` against `<dir>/CHECKSUMS.txt` for `version`, returning
/// the distinct maintainer signer count. Every check is delegated to
/// [`updater::verify_release_manifest`]; none is reimplemented here (REQ-202-005).
pub fn verify_manifest_dir(
    dir: &Path,
    version: &str,
    root: &updater::TrustRoot,
) -> anyhow::Result<usize> {
    let manifest_path = dir.join("SIGNATURES.json");
    let manifest_bytes = std::fs::read(&manifest_path)
        .with_context(|| format!("cannot read {}", manifest_path.display()))?;
    let signatures: updater::SignaturesFile = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("cannot parse {}", manifest_path.display()))?;

    let checksums_path = dir.join("CHECKSUMS.txt");
    let checksums_body = std::fs::read(&checksums_path)
        .with_context(|| format!("cannot read {}", checksums_path.display()))?;

    Ok(updater::verify_release_manifest(
        version,
        &checksums_body,
        &signatures,
        root,
    )?)
}

#[cfg(test)]
#[path = "cmd_release_verify_tests.rs"]
mod tests;
