//! Artifact-bound install gate (INC-I-172 F1).
//!
//! Verifying that a SIGNATURES.json carries enough valid maintainer signatures proves
//! only that the maintainers signed *something*. Before this module, both operator
//! install paths built the signed message out of `sf.version` and `sf.checksums_sha256`
//! — two fields read out of the SAME attacker-supplied file that carries the signatures
//! — and then installed a tarball that was never compared to either. That check is
//! circular: a verbatim copy of ANY past genuine SIGNATURES.json satisfies it while an
//! arbitrary binary is installed, so the gate added zero integrity over the checksum it
//! was supposed to backstop.
//!
//! [`verify_release_artifact`] closes the chain end to end. Every link is checked here,
//! in one place, so the two operator paths (`doli upgrade`, `doli-node upgrade`)
//! cannot drift apart:
//!
//! ```text
//!   L1  sf.version            == the release TAG being installed
//!   L2  sf.checksums_sha256   == sha256(the CHECKSUMS.txt actually fetched)
//!   L3  threshold distinct maintainer signatures over "{sf.version}:{sf.checksums_sha256}"
//!   L4  sha256(tarball)       == the per-platform hash parsed from THAT CHECKSUMS.txt
//! ```
//!
//! L2 and L4 are recomputed here from `GithubReleaseInfo::checksums_body`, not read
//! from the caller's derived fields: a "verified" hash that no one ever compared to
//! real bytes is the unbound operand this module exists to remove. Any broken link is
//! an `Err` — there is no advisory outcome.

use tracing::{error, info};

use crate::download::{platform_tarball_hash, verify_hash, GithubReleaseInfo};
use crate::trust_root::TrustRoot;
use crate::types::{Release, Result, SignaturesFile, UpdateError};
use crate::verification::verify_release_with_trust_root;

/// Compare release versions ignoring a leading `v` and surrounding whitespace.
///
/// The tag is `v6.24.1` while `doli release sign` strips the prefix before signing, so
/// a literal comparison would refuse every genuine release. Nothing else is normalised:
/// `6.24.1` and `6.24.10` stay different.
fn normalize_version(version: &str) -> &str {
    version.trim().trim_start_matches('v')
}

/// L1+L2+L3 over a release manifest with no tarball in hand (INC-I-202 M2, REQ-202-005).
///
/// `version` is the authoritative release TAG and `checksums_body` the raw CHECKSUMS.txt
/// bytes fetched under it. Returns the number of DISTINCT maintainer signers.
///
/// - **L1 (version).** Without it, a genuine SIGNATURES.json from release *A* is
///   accepted while release *B* is installed — a downgrade or a cross-release replay.
/// - **L2 (checksums hash).** This is the operand the maintainers actually signed. It
///   must equal the hash of the CHECKSUMS.txt bytes in hand, otherwise the signature
///   covers a file nobody is going to read.
/// - **L3 (signatures).** Distinct-signer k-of-n against a fail-closed [`TrustRoot`].
///
/// Every branch returns `Err`; none warns and continues.
pub fn verify_release_manifest(
    version: &str,
    checksums_body: &[u8],
    signatures: &SignaturesFile,
    root: &TrustRoot,
) -> Result<usize> {
    // ---- L1: the signed version must be the version being installed ----------
    if normalize_version(&signatures.version) != normalize_version(version) {
        error!(
            "Refusing to install v{}: SIGNATURES.json is for v{}. A genuine signature over a \
             different release is a replay, not an authorisation (INC-I-172 F1).",
            version, signatures.version
        );
        return Err(UpdateError::ArtifactBindingMismatch {
            field: "version",
            signed: signatures.version.clone(),
            actual: version.to_string(),
        });
    }

    // ---- L2: the signed CHECKSUMS.txt hash must be the hash of the bytes fetched ----
    let actual_checksums_sha256 = sha256_hex(checksums_body);
    if !signatures
        .checksums_sha256
        .eq_ignore_ascii_case(&actual_checksums_sha256)
    {
        error!(
            "Refusing to install v{}: the maintainer signatures cover CHECKSUMS.txt {} but the \
             CHECKSUMS.txt fetched for this release hashes to {}. The signed file is not the file \
             the artifact hash comes from (INC-I-172 F1).",
            version, signatures.checksums_sha256, actual_checksums_sha256
        );
        return Err(UpdateError::ArtifactBindingMismatch {
            field: "checksums_sha256",
            signed: signatures.checksums_sha256.clone(),
            actual: actual_checksums_sha256,
        });
    }

    // ---- L3: enough DISTINCT maintainer signatures over that bound pair ------
    // `sf`'s own strings reproduce the publisher's message byte-for-byte: L1 only
    // pinned them modulo the `v` prefix.
    let sig_release = Release {
        version: signatures.version.clone(),
        binary_sha256: signatures.checksums_sha256.clone(),
        signatures: signatures.signatures.clone(),
        binary_url_template: String::new(),
        changelog: String::new(),
        published_at: 0,
        target_networks: Vec::new(),
    };
    verify_release_with_trust_root(&sig_release, root)
}

/// Authorize the install of `tarball` for `release_info` under `root`.
///
/// L1-L3 are [`verify_release_manifest`] — ONE implementation, shared with the publish
/// gate so the two cannot drift. L4 lives here: the artifact must be what the
/// just-verified CHECKSUMS.txt names for this platform, parsed from the verified bytes
/// rather than from `release_info.expected_hash`, so no caller can substitute a
/// different operand.
///
/// Returns the number of DISTINCT maintainer signers, so an operator-facing caller can
/// report what was actually verified. The order is deliberate: cheap bindings first, then
/// the signature check, then the hash of a possibly large tarball.
pub fn verify_release_artifact(
    release_info: &GithubReleaseInfo,
    tarball: &[u8],
    signatures: &SignaturesFile,
    root: &TrustRoot,
) -> Result<usize> {
    let distinct_signers = verify_release_manifest(
        &release_info.version,
        &release_info.checksums_body,
        signatures,
        root,
    )?;

    // ---- L4: the artifact must be what the verified CHECKSUMS.txt names -------
    let checksums_text = String::from_utf8_lossy(&release_info.checksums_body);
    let expected_tarball_hash = platform_tarball_hash(&checksums_text)?;
    verify_hash(tarball, &expected_tarball_hash)?;

    let actual_checksums_sha256 = sha256_hex(&release_info.checksums_body);
    info!(
        "Install authorised for v{}: {} distinct signer(s) under the {} trust root, \
         CHECKSUMS.txt {} bound to the signatures, tarball bound to CHECKSUMS.txt",
        release_info.version,
        distinct_signers,
        root.provenance(),
        &actual_checksums_sha256[..actual_checksums_sha256.len().min(16)]
    );
    Ok(distinct_signers)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::BOOTSTRAP_MAINTAINER_KEYS_MAINNET;

    // REQ-202-004 — Decision: an Ok here means the publish gate authorises the exact zero-signature manifest shape CI actually published.
    #[test]
    fn verify_release_manifest_refuses_a_zero_entry_manifest() {
        let body = b"deadbeef  doli-node-v6.26.3-linux-x86_64.tar.gz\n";
        let sf = SignaturesFile {
            version: "6.26.3".to_string(),
            checksums_sha256: sha256_hex(body),
            signatures: Vec::new(),
        };
        let root = TrustRoot::on_chain(
            BOOTSTRAP_MAINTAINER_KEYS_MAINNET
                .iter()
                .map(|k| (*k).to_string())
                .collect(),
            3,
        );

        let err = verify_release_manifest("v6.26.3", body, &sf, &root)
            .expect_err("a 0-entry manifest must never verify");

        assert!(
            matches!(
                err,
                UpdateError::InsufficientSignatures {
                    found: 0,
                    required: 3
                }
            ),
            "expected InsufficientSignatures 0/3, got {err}"
        );
    }

    #[test]
    fn version_normalisation_strips_only_a_leading_v() {
        assert_eq!(normalize_version("v6.24.1"), "6.24.1");
        assert_eq!(normalize_version(" 6.24.1 "), "6.24.1");
        assert_ne!(normalize_version("6.24.1"), normalize_version("6.24.10"));
    }

    #[test]
    fn sha256_hex_matches_a_known_vector() {
        // sha256("") — pins the helper against a published vector, so a future
        // refactor cannot silently change which digest the L2 binding compares.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
