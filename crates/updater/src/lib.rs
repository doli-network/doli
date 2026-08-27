//! DOLI Auto-Update System
//!
//! Simple, transparent auto-updates with community veto power.
//!
//! # Rules (no exceptions)
//! - ALL updates: 2-epoch veto period (configurable per network), counted from the
//!   NODE-LOCAL moment this node first observed the release — never from the
//!   unsigned `Release::published_at` (INC-I-172 F7(b))
//! - 40% of producers can veto any update (head count; there is no weighted veto)
//! - 3 of 5 DISTINCT maintainer signatures required per network (via SIGNATURES.json
//!   in GitHub Releases), checked against a resolved [`TrustRoot`] that fails closed
//!
//! # Flow
//! 1. Release published on GitHub (CI creates CHECKSUMS.txt)
//! 2. Maintainers sign with `doli release sign` → SIGNATURES.json uploaded
//! 3. Veto period begins (2 epochs mainnet, 1 min devnet)
//! 4. Producers can vote to veto
//! 5. If >= 40% veto: REJECTED
//! 6. If < 40% veto: APPROVED and applied
//!
//! # Network-Aware Parameters
//!
//! All timing parameters are configurable per network via `UpdateParams`:
//! - Mainnet/Testnet: Production timing (2 epochs veto, 1 epoch grace)
//! - Devnet: Accelerated timing (60s veto, 30s grace) for fast testing

// Existing sub-modules
mod apply;
mod download;
pub mod hardfork;
pub mod test_keys;
mod vote;
pub mod watchdog;

// Domain modules
mod constants;
mod enforcement;
mod install_gate;
mod params;
mod release_args;
mod skills;
mod trust_root;
mod types;
mod util;
mod verification;

// Re-exports: apply
pub use apply::{
    apply_update, auto_apply_from_github, backup_current, current_binary_path,
    extract_binary_from_tarball, extract_named_binary_from_tarball, install_binary, restart_node,
    rollback,
};

// Re-exports: skills
pub use skills::{install_skills_from_tarball, install_skills_into, skill_entry_path_is_safe};

// Re-exports: download
pub use download::{
    download_binary, download_checksums_txt, download_from_url, download_signatures_json,
    fetch_github_release, fetch_latest_release, verify_hash, GithubReleaseInfo,
};

// Re-exports: test_keys
pub use test_keys::{
    create_test_release_signatures, should_use_test_keys, sign_with_test_key,
    test_maintainer_pubkeys, TestMaintainerKey, TEST_MAINTAINER_KEYS,
};

// Re-exports: vote
pub use vote::{Vote, VoteMessage, VoteTracker};

// Re-exports: constants
pub use constants::{
    assert_production_keys, bootstrap_maintainer_keys, get_maintainer_keys,
    is_using_placeholder_keys, BOOTSTRAP_MAINTAINER_KEYS_MAINNET,
    BOOTSTRAP_MAINTAINER_KEYS_TESTNET, CHECK_INTERVAL, GITHUB_API_URL, GITHUB_RELEASES_URL,
    GITHUB_REPO, GRACE_PERIOD, REQUIRED_SIGNATURES, VETO_PERIOD, VETO_THRESHOLD_PERCENT,
};

// Re-exports: types
pub use types::{
    MaintainerSignature, Release, ReleaseMetadata, Result, SignaturesFile, UpdateConfig,
    UpdateError, VoteResult,
};

// Re-exports: params
pub use params::UpdateParams;

// Re-exports: enforcement
pub use enforcement::{
    check_production_allowed, grace_period_deadline, grace_period_deadline_for_network,
    in_grace_period, in_grace_period_for_network, veto_deadline, veto_period_ended,
    ProductionBlocked, VersionEnforcement,
};

// Re-exports: trust_root
pub use trust_root::{TrustRoot, TrustRootProvenance};

// Re-exports: install_gate
pub use install_gate::verify_release_artifact;

// Re-exports: release_args (INC-I-172 M2, AUDIT-P0-011 — every release-signing entry
// point must validate through THESE, before any signing message is interpolated)
pub use release_args::{validate_release_hash, validate_release_version};

// Re-exports: verification
pub use verification::{
    calculate_veto_result, sign_release_hash, verify_release_signatures,
    verify_release_with_trust_root,
};

// Re-exports: hardfork
pub use hardfork::{HardForkInfo, HardForkSchedule};

// Re-exports: util
pub use util::{current_timestamp, current_version, is_newer_version, platform_identifier};

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_comparison() {
        assert!(is_newer_version("1.0.1", "1.0.0"));
        assert!(is_newer_version("1.1.0", "1.0.9"));
        assert!(is_newer_version("2.0.0", "1.9.9"));
        assert!(!is_newer_version("1.0.0", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "1.0.1"));
        assert!(is_newer_version("v1.0.1", "v1.0.0"));
    }

    #[test]
    fn test_veto_calculation() {
        // 30% veto - should pass (< 40% threshold)
        let result = calculate_veto_result(30, 100);
        assert_eq!(result.veto_percent, 30);
        assert!(result.approved);

        // 39% veto - should pass (< 40% threshold)
        let result = calculate_veto_result(39, 100);
        assert_eq!(result.veto_percent, 39);
        assert!(result.approved);

        // 40% veto - should fail (>= 40% threshold)
        let result = calculate_veto_result(40, 100);
        assert_eq!(result.veto_percent, 40);
        assert!(!result.approved);

        // 50% veto - should fail
        let result = calculate_veto_result(50, 100);
        assert_eq!(result.veto_percent, 50);
        assert!(!result.approved);

        // Edge case: producer count UNKNOWN (AUDIT-P1-015). A zero denominator is not
        // "nobody vetoed" — it is "the electorate is unknown", which is what
        // `try_read().unwrap_or(0)` produced on ordinary lock contention. It must NOT
        // approve.
        let result = calculate_veto_result(0, 0);
        assert_eq!(result.veto_percent, 0);
        assert!(!result.approved);

        // ...and it must not approve when votes WERE cast either: this is the shape that
        // turned any number of vetoes into "0% — APPROVED".
        let result = calculate_veto_result(99, 0);
        assert!(!result.approved);
    }

    #[test]
    fn test_platform_identifier() {
        let platform = platform_identifier();
        assert!([
            "linux-x64",
            "linux-arm64",
            "macos-x64",
            "macos-arm64",
            "unknown"
        ]
        .contains(&platform));
    }

    #[test]
    fn test_sign_release_hash() {
        // Generate a test keypair
        let private_key = crypto::PrivateKey::from_bytes([42u8; 32]);
        let keypair = crypto::KeyPair::from_private_key(private_key);

        let sig = sign_release_hash(&keypair, "1.0.0", "abcdef1234567890");

        // Verify the signature matches what verify_release_signatures expects
        assert_eq!(sig.public_key, keypair.public_key().to_hex());
        assert!(!sig.signature.is_empty());

        // Manually verify the signature
        let message = b"1.0.0:abcdef1234567890";
        let pubkey = crypto::PublicKey::from_hex(&sig.public_key).unwrap();
        let signature = crypto::Signature::from_hex(&sig.signature).unwrap();
        assert!(crypto::signature::verify(message, &signature, &pubkey).is_ok());
    }
}
