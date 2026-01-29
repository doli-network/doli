//! Test maintainer keys for development and testing
//!
//! These keys are used for testing the governance/veto system on devnet.
//! NEVER use these keys on mainnet or testnet.
//!
//! # Warning
//!
//! The private keys are included here for testing purposes only.
//! In production, maintainer private keys must be kept secure and offline.

use crypto::{PrivateKey, PublicKey, Signature};
use std::sync::LazyLock;

/// Test maintainer key pair (public and private keys as hex strings)
#[derive(Clone)]
pub struct TestMaintainerKey {
    pub public_key: String,
    pub private_key: String,
}

/// Generate deterministic test keys from a seed
fn generate_test_key(seed: u8) -> TestMaintainerKey {
    // Create a deterministic 32-byte seed for the private key
    let mut seed_bytes = [0u8; 32];
    seed_bytes[0] = seed;
    seed_bytes[31] = seed;
    // Fill with deterministic pattern
    for i in 1..31 {
        seed_bytes[i] = ((seed as u16 * (i as u16 + 1)) % 256) as u8;
    }

    let private_key = PrivateKey::from_bytes(seed_bytes);
    let keypair = crypto::KeyPair::from_private_key(private_key);

    TestMaintainerKey {
        public_key: keypair.public_key().to_hex(),
        private_key: keypair.private_key().to_hex(),
    }
}

/// 5 test maintainer keypairs for devnet testing (lazily generated)
pub static TEST_MAINTAINER_KEYS: LazyLock<[TestMaintainerKey; 5]> = LazyLock::new(|| {
    [
        generate_test_key(1),
        generate_test_key(2),
        generate_test_key(3),
        generate_test_key(4),
        generate_test_key(5),
    ]
});

/// Get test maintainer public keys (hex-encoded)
pub fn test_maintainer_pubkeys() -> Vec<&'static str> {
    TEST_MAINTAINER_KEYS
        .iter()
        .map(|k| k.public_key.as_str())
        .collect()
}

/// Sign a message with a test maintainer key
///
/// Returns the signature as a hex string, or None if the maintainer index is invalid.
pub fn sign_with_test_key(maintainer_index: usize, message: &[u8]) -> Option<String> {
    let key = TEST_MAINTAINER_KEYS.get(maintainer_index)?;

    let private_key = PrivateKey::from_hex(&key.private_key).ok()?;
    let signature = crypto::signature::sign(message, &private_key);
    Some(signature.to_hex())
}

/// Create a signed release using test maintainer keys
///
/// Signs with the first 3 test maintainers (minimum required).
pub fn create_test_release_signatures(version: &str, binary_sha256: &str) -> Vec<(String, String)> {
    let message = format!("{}:{}", version, binary_sha256);
    let message_bytes = message.as_bytes();

    (0..3)
        .filter_map(|i| {
            let pubkey = &TEST_MAINTAINER_KEYS[i].public_key;
            let sig = sign_with_test_key(i, message_bytes)?;
            Some((pubkey.clone(), sig))
        })
        .collect()
}

/// Check if we should use test keys
///
/// Returns true if DOLI_TEST_KEYS environment variable is set to "1"
/// or if running on devnet (network ID 99).
pub fn should_use_test_keys() -> bool {
    std::env::var("DOLI_TEST_KEYS")
        .map(|v| v == "1")
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_and_verify() {
        let message = b"test message";

        // Sign with first test key
        let signature_hex = sign_with_test_key(0, message).unwrap();

        // Verify
        let pubkey = PublicKey::from_hex(&TEST_MAINTAINER_KEYS[0].public_key).unwrap();
        let signature = Signature::from_hex(&signature_hex).unwrap();

        assert!(crypto::signature::verify(message, &signature, &pubkey).is_ok());
    }

    #[test]
    fn test_create_release_signatures() {
        let sigs = create_test_release_signatures("1.0.0", "abc123");
        assert_eq!(sigs.len(), 3);

        // Verify each signature
        let message = b"1.0.0:abc123";
        for (pubkey_hex, sig_hex) in &sigs {
            let pubkey = PublicKey::from_hex(pubkey_hex).unwrap();
            let sig = Signature::from_hex(sig_hex).unwrap();
            assert!(crypto::signature::verify(message, &sig, &pubkey).is_ok());
        }
    }

    #[test]
    fn test_deterministic_keys() {
        // Keys should be deterministic - same seed produces same key
        let key1 = generate_test_key(1);
        let key1_again = generate_test_key(1);
        assert_eq!(key1.public_key, key1_again.public_key);
        assert_eq!(key1.private_key, key1_again.private_key);

        // Different seeds produce different keys
        let key2 = generate_test_key(2);
        assert_ne!(key1.public_key, key2.public_key);
    }
}
