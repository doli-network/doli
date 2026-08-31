//! Constants and bootstrap maintainer key management

use doli_core::network::Network;
use std::time::Duration;

use crate::test_keys::{should_use_test_keys, test_maintainer_pubkeys};

// ============================================================================
// Constants - Simple, fixed, no exceptions
// ============================================================================

/// Veto period: 5 minutes for ALL updates.
///
/// This is the CONFIGURED value and the one the code enforces. Nothing in this
/// repository implements a "7-day veto" — do not describe one in docs, log lines or
/// operator-facing text (INC-I-172 F8/G4). Network-specific overrides live in
/// `UpdateParams::veto_period_secs`; report that value, never a literal.
pub const VETO_PERIOD: Duration = Duration::from_secs(5 * 60);

/// Grace period after approval: 1 epoch (~1h) to update before enforcement
pub const GRACE_PERIOD: Duration = Duration::from_secs(3600);

/// Veto threshold: 40% of active producers, by HEAD COUNT.
///
/// There is no seniority weighting: the weighted-veto machinery was never reachable
/// from production code and was deleted in INC-I-172 M1 (F8).
///
/// Why 40% instead of 33%:
/// - 33% allows a $44K early attacker to block governance for 4 years
/// - 40% raises the cost: requires 15 nodes instead of 10 for sustained blocking
/// - Combined with activity penalty, makes "register and wait" attacks expensive
///
/// Note: This must match VETO_THRESHOLD_PERCENT in doli-storage
pub const VETO_THRESHOLD_PERCENT: u8 = 40;

/// Required maintainer signatures for the BOOTSTRAP trust root: 3 DISTINCT signers of 5.
///
/// This is the threshold of `TrustRoot::bootstrap` only. An on-chain root carries and
/// uses its own `MaintainerSet::threshold`.
pub const REQUIRED_SIGNATURES: usize = 3;

/// Bootstrap maintainer public keys for mainnet (Ed25519, hex-encoded)
///
/// M1-M5 are **signing-only**: never registered as producers, never bonded. Do NOT
/// re-couple the roles — that separation is what keeps double-production slashing
/// (`force_remove_maintainer`) unable to drive the set below `MIN_MAINTAINERS`.
///
/// Seated on-chain by the INC-I-175 rotation at h=331_457; this array is the matching
/// compiled cutover. It replaces five keys whose private halves are public in this
/// repository's history and cannot be recalled.
///
/// These keys are the trust root ONLY for a node that has never established an
/// on-chain maintainer set, and for the CLI, which has no chain state to read.
/// They are NOT a fallback (INC-I-172 F1): once a node has an on-chain set, that set
/// is authoritative, and an on-chain set that exists and is empty or sub-threshold
/// FAILS CLOSED — verification refuses rather than returning here. Only
/// `TrustRoot::bootstrap` may read this array.
pub const BOOTSTRAP_MAINTAINER_KEYS_MAINNET: [&str; 5] = [
    "d07ec4ec146245e0ce31800ba2cf98b9fc649aa7a4021a09e8534a7764033f8d",
    "25c24110d98f2a34c37bab8fede0791d3de1281ca499a30fb7ff5223cdb0e23c",
    "2559a47ee898f8bb9a38d90573dcca2195a97a7c787b5712ba62102e225b9e0d",
    "e477c1f245612f7351f66ce7936e4ffa1e0afef26a12f90f3a86ed3544ca5b8c",
    "3fd5be3de8285140a461b12dbd7d14ce0d026b5e369e38daebf89f6f7cbc0245",
];

/// Bootstrap maintainer public keys for testnet (Ed25519, hex-encoded)
///
/// **Signing-only wallets.** They are not producers and are not testnet genesis
/// producers. Do NOT re-couple the roles: the previous array WAS NT1-NT5, whose private
/// halves are committed at `testnet/keys/producer_{1..5}.json`, so any reader of this
/// repository could sign a release that a host resolving this array would install.
///
/// Same rule as the mainnet array: this is the trust root ONLY for a node that has
/// never established an on-chain maintainer set, and for the CLI. It is not a
/// fallback, and an empty or sub-threshold on-chain root never reaches it
/// (INC-I-172 F1).
pub const BOOTSTRAP_MAINTAINER_KEYS_TESTNET: [&str; 5] = [
    "f53aa197f35c4a9be03b38b3d9b3b265d0b5e73ee6de2deb9459bd57097c4f9b",
    "b655a415e2fbada433537340f489dc150c485f68efccc018382ec60bccd4ad92",
    "b49d860d7b1f2a6b0d7d01ba710ed7d3bc75b698803dcc5bec0252bfcdf67229",
    "35ecc3e1c2467be8c4f888b4bf559e02f3ad8021ed667a34af258276ad685dd7",
    "3158868a93c8c96601703e2f9b75ef04a6ce5894ee5d69e25cc67b677ee6ecd9",
];

/// Get the bootstrap maintainer keys for a specific network.
///
/// # This selection is NOT a cross-network security boundary (AUDIT-P2-012)
///
/// The signed release message is `"{version}:{sha256(CHECKSUMS.txt)}"`
/// (`verification.rs`). It carries **no network term**.
///
/// The two arrays diverged with the INC-I-196 mainnet cutover, so an identical-key
/// signature is no longer automatic — but divergence is NOT the boundary. Any holder of a
/// testnet key still authorizes a MAINNET release on any host resolving the testnet array,
/// and a signer present in both arrays crosses freely. Threading `network` through
/// `doli upgrade` / `doli-node upgrade` selects a key ARRAY; it binds nothing. Do not
/// describe `--network` as preventing cross-network replay in code comments, operator
/// output or docs — see `docs/cli.md` §18.2, which states the gap explicitly.
///
/// Closing it requires putting the network into the signed bytes, which invalidates every
/// already-published `SIGNATURES.json` and so needs its own coordinated rollout. Deferred
/// out of INC-I-172 M1 (no activation height, no flag-day).
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

/// GitHub repository for releases (primary source).
///
/// INVARIANT (INC-I-157): the release origin MUST name a namespace the project
/// actually controls. These constants are the root of trust for every
/// auto-update and every `doli upgrade` on every host, so whoever owns the
/// namespace owns the binaries every operator installs.
///
/// This previously pointed at an abandoned personal namespace (unregistered,
/// HTTP 404) and only kept working because of a GitHub rename-redirect. A
/// rename-redirect is NOT a security boundary: it lapses the instant the
/// abandoned namespace is re-registered by anyone else, silently handing the
/// update channel to whoever claims it. Never re-point these at a namespace
/// outside the project's control, and never rely on a redirect to reach one.
/// See docs/bugfixes/inc-i-157-installer-integrity-analysis.md.
pub const GITHUB_REPO: &str = "doli-network/doli";

/// GitHub API URL for latest release
pub const GITHUB_API_URL: &str = "https://api.github.com/repos/doli-network/doli/releases/latest";

/// GitHub releases download base URL
pub const GITHUB_RELEASES_URL: &str = "https://github.com/doli-network/doli/releases/download";
