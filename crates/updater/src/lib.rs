//! DOLI Auto-Update System
//!
//! Simple, transparent auto-updates with community veto power.
//!
//! # Rules (no exceptions)
//! - ALL updates: 2-epoch veto period (configurable per network)
//! - 40% of producers can veto any update
//! - 3 of 5 maintainer signatures required per network (via SIGNATURES.json in GitHub Releases)
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
mod params;
mod types;
mod util;
mod verification;

// Re-exports: apply
pub use apply::{
    apply_update, auto_apply_from_github, backup_current, current_binary_path,
    extract_binary_from_tarball, extract_named_binary_from_tarball, install_binary, restart_node,
    rollback,
};

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
    BOOTSTRAP_MAINTAINER_KEYS_TESTNET, CHECK_INTERVAL, FALLBACK_MIRROR, GITHUB_API_URL,
    GITHUB_RELEASES_URL, GITHUB_REPO, GRACE_PERIOD, REQUIRED_SIGNATURES, VETO_PERIOD,
    VETO_THRESHOLD_PERCENT,
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

// Re-exports: verification
pub use verification::{
    calculate_veto_result, sign_release_hash, verify_release_signatures,
    verify_release_signatures_with_keys,
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

        // Edge case: no producers
        let result = calculate_veto_result(0, 0);
        assert_eq!(result.veto_percent, 0);
        assert!(result.approved);
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
    fn test_vote_weight_bonds_times_seniority() {
        let params = UpdateParams::for_network(doli_core::network::Network::Devnet);
        // Devnet: seniority_step_blocks = 144 (1 "year")

        // 1 bond, 0 years → 1.0 × 1.0 = 1.0
        let w = params.calculate_vote_weight(1, 0);
        assert!((w - 1.0).abs() < 0.001, "1 bond, 0 years: got {}", w);

        // 10 bonds, 0 years → 10.0 × 1.0 = 10.0
        let w = params.calculate_vote_weight(10, 0);
        assert!((w - 10.0).abs() < 0.001, "10 bonds, 0 years: got {}", w);

        // 1 bond, 1 year (144 blocks on devnet) → 1.0 × 1.75 = 1.75
        let w = params.calculate_vote_weight(1, params.seniority_step_blocks);
        assert!((w - 1.75).abs() < 0.001, "1 bond, 1 year: got {}", w);

        // 2 bonds, 4 years (576 blocks on devnet) → 2.0 × 4.0 = 8.0
        let w = params.calculate_vote_weight(2, params.seniority_maturity_blocks);
        assert!((w - 8.0).abs() < 0.001, "2 bonds, 4 years: got {}", w);

        // 10 bonds, 4 years → 10.0 × 4.0 = 40.0
        let w = params.calculate_vote_weight(10, params.seniority_maturity_blocks);
        assert!((w - 40.0).abs() < 0.001, "10 bonds, 4 years: got {}", w);

        // Seniority caps at 4 years (4.0x) even after 10 years
        let w = params.calculate_vote_weight(1, params.seniority_step_blocks * 10);
        assert!(
            (w - 4.0).abs() < 0.001,
            "1 bond, 10 years (capped): got {}",
            w
        );
    }

    #[test]
    fn test_vote_weight_as_u64_for_tracker() {
        let params = UpdateParams::for_network(doli_core::network::Network::Devnet);

        // Weights stored as u64 with 100x multiplier for 2-decimal precision
        let w = params.calculate_vote_weight(1, 0);
        assert_eq!((w * 100.0) as u64, 100); // 1.0 × 100

        let w = params.calculate_vote_weight(10, 0);
        assert_eq!((w * 100.0) as u64, 1000); // 10.0 × 100

        let w = params.calculate_vote_weight(2, params.seniority_maturity_blocks);
        assert_eq!((w * 100.0) as u64, 800); // 8.0 × 100

        let w = params.calculate_vote_weight(10, params.seniority_maturity_blocks);
        assert_eq!((w * 100.0) as u64, 4000); // 40.0 × 100
    }

    #[test]
    fn test_vote_weight_veto_with_bonds() {
        use crate::vote::{Vote, VoteTracker};
        let params = UpdateParams::for_network(doli_core::network::Network::Devnet);

        // Scenario: 5 genesis producers (1 bond, ~24 blocks active) + 1 whale (10 bonds, new)
        let mut weights = std::collections::HashMap::new();
        for i in 0..5 {
            let w = params.calculate_vote_weight(1, 24); // ~0.125 years
            weights.insert(format!("genesis_{}", i), (w * 100.0) as u64);
        }
        let whale_w = params.calculate_vote_weight(10, 1);
        weights.insert("whale".to_string(), (whale_w * 100.0) as u64);

        let total_weight: u64 = weights.values().sum();
        let mut tracker = VoteTracker::with_weights("99.0.0".into(), weights);

        // Whale alone vetoes — should exceed 40%
        tracker.record_vote("whale".into(), Vote::Veto);
        assert!(
            tracker.should_reject_weighted(total_weight),
            "Whale (10 bonds) should be able to veto alone (weight {}/{})",
            tracker.veto_weight(),
            total_weight
        );
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

    #[test]
    fn test_seniority_multiplier() {
        let params = UpdateParams::for_network(doli_core::network::Network::Devnet);

        assert!((params.seniority_multiplier(0) - 1.0).abs() < 0.001);
        assert!((params.seniority_multiplier(params.seniority_step_blocks) - 1.75).abs() < 0.001);
        assert!(
            (params.seniority_multiplier(params.seniority_step_blocks * 2) - 2.5).abs() < 0.001
        );
        assert!(
            (params.seniority_multiplier(params.seniority_maturity_blocks) - 4.0).abs() < 0.001
        );
    }
}
