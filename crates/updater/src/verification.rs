//! Release signature verification and veto calculation

use crypto::{PublicKey, Signature as CryptoSignature};
use doli_core::network::Network;
use tracing::{debug, info, warn};

use crate::constants::{bootstrap_maintainer_keys, REQUIRED_SIGNATURES, VETO_THRESHOLD_PERCENT};
use crate::types::{MaintainerSignature, Release, Result, UpdateError, VoteResult};

// ============================================================================
// Release Signing
// ============================================================================

/// Sign a release hash with a maintainer's private key
///
/// Signs the message `"version:sha256"` which matches the format verified by
/// `verify_release_signatures_with_keys()`. Returns a `MaintainerSignature`
/// containing the public key and hex-encoded signature.
///
/// # Usage
///
/// ```ignore
/// let keypair = crypto::KeyPair::from_private_key(private_key);
/// let sig = sign_release_hash(&keypair, "0.2.0", "abcdef1234...");
/// println!("{}", serde_json::to_string(&sig).unwrap());
/// ```
pub fn sign_release_hash(
    keypair: &crypto::KeyPair,
    version: &str,
    binary_sha256: &str,
) -> MaintainerSignature {
    let message = format!("{}:{}", version, binary_sha256);
    let signature = crypto::signature::sign(message.as_bytes(), keypair.private_key());
    MaintainerSignature {
        public_key: keypair.public_key().to_hex(),
        signature: signature.to_hex(),
    }
}

// ============================================================================
// Signature Verification
// ============================================================================

/// Verify release signatures using bootstrap keys only (convenience wrapper)
///
/// Use this for CLI commands and contexts where on-chain state is not available.
/// For the running node, prefer `verify_release_signatures_with_keys()`.
pub fn verify_release_signatures(release: &Release, network: Network) -> Result<()> {
    verify_release_signatures_with_keys(release, &[], network)
}

/// Verify that a release has sufficient valid maintainer signatures
///
/// If `on_chain_keys` is non-empty, those keys are used for verification
/// (derived from first 5 registered producers on-chain). If empty, falls
/// back to the network-specific bootstrap maintainer keys.
pub fn verify_release_signatures_with_keys(
    release: &Release,
    on_chain_keys: &[String],
    network: Network,
) -> Result<()> {
    let message = format!("{}:{}", release.version, release.binary_sha256);
    let message_bytes = message.as_bytes();

    // Determine which keys to use
    let using_on_chain = !on_chain_keys.is_empty();
    let allowed_keys: Vec<&str> = if using_on_chain {
        info!(
            "Verifying release signatures with {} on-chain maintainer keys",
            on_chain_keys.len()
        );
        on_chain_keys.iter().map(|s| s.as_str()).collect()
    } else {
        debug!(
            "Verifying release signatures with {:?} bootstrap maintainer keys",
            network
        );
        bootstrap_maintainer_keys(network).to_vec()
    };

    let mut valid_count = 0;

    for sig in &release.signatures {
        // Check if this is a known maintainer
        if !allowed_keys.contains(&sig.public_key.as_str()) {
            debug!("Ignoring signature from unknown key: {}", sig.public_key);
            continue;
        }

        // Decode public key
        let pubkey_bytes = match hex::decode(&sig.public_key) {
            Ok(bytes) => bytes,
            Err(_) => {
                warn!("Invalid hex in public key: {}", sig.public_key);
                continue;
            }
        };

        // Decode signature
        let sig_bytes = match hex::decode(&sig.signature) {
            Ok(bytes) => bytes,
            Err(_) => {
                warn!("Invalid hex in signature");
                continue;
            }
        };

        // Verify signature using doli-crypto
        if verify_ed25519(&pubkey_bytes, message_bytes, &sig_bytes) {
            valid_count += 1;
            debug!(
                "Valid signature from maintainer: {}...",
                &sig.public_key[..16]
            );
        } else {
            warn!(
                "Invalid signature from maintainer: {}...",
                &sig.public_key[..16]
            );
        }
    }

    if valid_count >= REQUIRED_SIGNATURES {
        info!(
            "Release {} verified: {}/{} valid signatures (source: {})",
            release.version,
            valid_count,
            REQUIRED_SIGNATURES,
            if using_on_chain {
                "on-chain"
            } else {
                "bootstrap"
            }
        );
        Ok(())
    } else {
        Err(UpdateError::InsufficientSignatures {
            found: valid_count,
            required: REQUIRED_SIGNATURES,
        })
    }
}

// ============================================================================
// Veto Calculation
// ============================================================================

/// Calculate veto result
pub fn calculate_veto_result(veto_count: usize, total_producers: usize) -> VoteResult {
    let veto_percent = if total_producers > 0 {
        ((veto_count * 100) / total_producers) as u8
    } else {
        0
    };

    let approved = veto_percent < VETO_THRESHOLD_PERCENT;

    VoteResult {
        total_producers,
        veto_count,
        veto_percent,
        approved,
    }
}

// ============================================================================
// Helpers (private)
// ============================================================================

/// Verify Ed25519 signature using doli-crypto
fn verify_ed25519(pubkey_bytes: &[u8], message: &[u8], sig_bytes: &[u8]) -> bool {
    use crypto::signature::verify;

    // Parse public key
    let pubkey = match PublicKey::try_from_slice(pubkey_bytes) {
        Ok(pk) => pk,
        Err(_) => return false,
    };

    // Parse signature
    let signature = match CryptoSignature::try_from_slice(sig_bytes) {
        Ok(sig) => sig,
        Err(_) => return false,
    };

    // Verify
    verify(message, &signature, &pubkey).is_ok()
}
