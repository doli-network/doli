//! Constants and bootstrap maintainer key management

use doli_core::network::Network;
use std::time::Duration;

use crate::test_keys::{should_use_test_keys, test_maintainer_pubkeys};

// ============================================================================
// Constants - Simple, fixed, no exceptions
// ============================================================================

/// Veto period: 5 minutes for ALL updates (early network; target: 7 days)
pub const VETO_PERIOD: Duration = Duration::from_secs(5 * 60);

/// Grace period after approval: 1 epoch (~1h) to update before enforcement
pub const GRACE_PERIOD: Duration = Duration::from_secs(3600);

/// Veto threshold: 40% of active producers (weighted by seniority)
///
/// Why 40% instead of 33%:
/// - 33% allows a $44K early attacker to block governance for 4 years
/// - 40% raises the cost: requires 15 nodes instead of 10 for sustained blocking
/// - Combined with activity penalty, makes "register and wait" attacks expensive
///
/// Note: This must match VETO_THRESHOLD_PERCENT in doli-storage
pub const VETO_THRESHOLD_PERCENT: u8 = 40;

/// Required maintainer signatures: 3 of 5
pub const REQUIRED_SIGNATURES: usize = 3;

/// Bootstrap maintainer public keys for mainnet (Ed25519, hex-encoded)
///
/// N1-N5 are both **producers AND maintainers** on mainnet (dual role).
/// N6-N12 are producers only — they produce blocks but cannot sign releases.
/// These keys are used as fallback for release signature verification (3-of-5)
/// before on-chain state is available. Once synced, on-chain keys take precedence.
pub const BOOTSTRAP_MAINTAINER_KEYS_MAINNET: [&str; 5] = [
    // N1 — producer + maintainer
    "202047256a8072a8b8f476691b9a5ae87710cc545e8707ca9fe0c803c3e6d3df",
    // N2 — producer + maintainer
    "effe88fefb6d992a1329277a1d49c7296d252bbc368319cb4bc061119926272b",
    // N3 — producer + maintainer
    "54323cefd0eabac89b2a2198c95a8f261598c341a8e579a05e26322325c48c2b",
    // N4 — producer + maintainer
    "2d27fdcc6a240b76ecaea64ad05c9b70d1adad90b6f9c43e8cbbbc0f1ab04116",
    // N5 — producer + maintainer
    "3047e96b13276dd92ef5eb2d6396e66c29909217f11f8c0544ea7d76a76c7602",
];

/// Bootstrap maintainer public keys for testnet (Ed25519, hex-encoded)
///
/// NT1-NT5 are both **producers AND maintainers** on testnet (dual role).
/// NT6-NT12 are producers only — they produce blocks but cannot sign releases.
/// These keys are used as fallback for release signature verification (3-of-5)
/// before on-chain state is available. Once synced, on-chain keys take precedence.
pub const BOOTSTRAP_MAINTAINER_KEYS_TESTNET: [&str; 5] = [
    // NT1 — producer + maintainer
    "273a257357a0fefeba0d97f4e61ea069e2cb2758239b315824ea73410d06a199",
    // NT2 — producer + maintainer
    "d70259cb4fc7acaeddb5028014a62b8d359a8e9fbd98b6cc7b8ca6e9bb1270df",
    // NT3 — producer + maintainer
    "f23fb0840f985b781cdce2a8f9996e58dc154909e6fc36eb419b2b31a88fcc7f",
    // NT4 — producer + maintainer
    "7e5f6f49f934099c78edfbc7967143d8e32c88feb36a10864e8f5575b4f0028b",
    // NT5 — producer + maintainer
    "952f3d72abd9708ea7f3760b0113a522143895a0948e76220e8c5b320c3ca91d",
];

/// Get the bootstrap maintainer keys for a specific network
pub fn bootstrap_maintainer_keys(network: Network) -> &'static [&'static str; 5] {
    match network {
        Network::Mainnet => &BOOTSTRAP_MAINTAINER_KEYS_MAINNET,
        Network::Testnet | Network::Devnet => &BOOTSTRAP_MAINTAINER_KEYS_TESTNET,
    }
}

/// Check if the bootstrap maintainer keys are still placeholders
///
/// Returns `true` if any key starts with "00000000" (placeholder pattern).
/// This MUST return `false` before mainnet launch.
pub fn is_using_placeholder_keys(network: Network) -> bool {
    bootstrap_maintainer_keys(network)
        .iter()
        .any(|k| k.starts_with("00000000"))
}

/// Verify that bootstrap maintainer keys are production-ready
///
/// Panics if placeholder keys are detected.
/// Call this during node initialization.
pub fn assert_production_keys(network: Network) {
    if is_using_placeholder_keys(network) {
        panic!(
            "FATAL: Placeholder bootstrap maintainer keys detected for {:?}!\n\
             This build cannot be used for {}.\n\
             Replace bootstrap maintainer keys in doli-updater/src/lib.rs with real keys.",
            network,
            network.name()
        );
    }
}

/// Get the bootstrap maintainer public keys for a network
///
/// Returns test keys if DOLI_TEST_KEYS=1 is set, otherwise returns
/// the network-specific bootstrap keys.
pub fn get_maintainer_keys(network: Network) -> Vec<&'static str> {
    if should_use_test_keys() && network == Network::Devnet {
        test_maintainer_pubkeys()
    } else {
        bootstrap_maintainer_keys(network).to_vec()
    }
}

/// Default update check interval: 6 hours
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);

/// GitHub repository for releases (primary source)
/// Using author's personal repo for authenticity (like torvalds/linux, antirez/redis)
pub const GITHUB_REPO: &str = "e-weil/doli";

/// GitHub API URL for latest release
pub const GITHUB_API_URL: &str = "https://api.github.com/repos/e-weil/doli/releases/latest";

/// GitHub releases download base URL
pub const GITHUB_RELEASES_URL: &str = "https://github.com/e-weil/doli/releases/download";

/// Fallback update server (if GitHub is unreachable)
pub const FALLBACK_MIRROR: &str = "https://releases.doli.network";
