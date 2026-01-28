//! Ed25519 digital signatures with domain separation.
//!
//! This module provides Ed25519 signature generation and verification.
//! Ed25519 is a high-performance, high-security signature scheme that:
//!
//! - Produces 64-byte signatures
//! - Is deterministic (same key + message = same signature)
//! - Provides ~128-bit security level
//! - Is resistant to timing attacks
//!
//! ## Domain Separation
//!
//! For production use, messages should be hashed with domain separation
//! before signing to prevent cross-protocol attacks.
//!
//! ## Example
//!
//! ```rust
//! use crypto::{KeyPair, signature};
//!
//! let kp = KeyPair::generate();
//! let message = b"Hello, DOLI!";
//!
//! let sig = signature::sign(message, kp.private_key());
//! assert!(signature::verify(message, &sig, kp.public_key()).is_ok());
//! ```

use ed25519_dalek::Signer;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use subtle::ConstantTimeEq;
use thiserror::Error;

use crate::hash::{hash_with_domain, Hash};
use crate::keys::{KeyError, PrivateKey, PublicKey};
use crate::{SIGNATURE_SIZE, SIGN_DOMAIN};

/// Errors related to signature operations.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SignatureError {
    /// The signature has an invalid length.
    #[error("invalid signature length: expected {expected}, got {got}")]
    InvalidLength {
        /// Expected length in bytes.
        expected: usize,
        /// Actual length in bytes.
        got: usize,
    },

    /// The signature verification failed.
    #[error("signature verification failed")]
    VerificationFailed,

    /// An error occurred with the key.
    #[error("key error: {0}")]
    KeyError(#[from] KeyError),

    /// The hex string is invalid.
    #[error("invalid hex string: {0}")]
    InvalidHex(String),
}

impl From<hex::FromHexError> for SignatureError {
    fn from(e: hex::FromHexError) -> Self {
        SignatureError::InvalidHex(e.to_string())
    }
}

/// A 64-byte Ed25519 signature.
///
/// Signatures are produced by signing a message with a private key and
/// can be verified with the corresponding public key.
#[derive(Clone, Copy, Eq)]
pub struct Signature([u8; SIGNATURE_SIZE]);

impl Signature {
    /// Create a signature from raw bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; SIGNATURE_SIZE]) -> Self {
        Self(bytes)
    }

    /// Try to create a signature from a byte slice.
    ///
    /// # Errors
    ///
    /// Returns an error if the slice is not exactly 64 bytes.
    pub fn try_from_slice(slice: &[u8]) -> Result<Self, SignatureError> {
        if slice.len() != SIGNATURE_SIZE {
            return Err(SignatureError::InvalidLength {
                expected: SIGNATURE_SIZE,
                got: slice.len(),
            });
        }
        let mut bytes = [0u8; SIGNATURE_SIZE];
        bytes.copy_from_slice(slice);
        Ok(Self(bytes))
    }

    /// Get the underlying bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; SIGNATURE_SIZE] {
        &self.0
    }

    /// Convert to a byte vector.
    #[must_use]
    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }

    /// Create from a hex string.
    ///
    /// # Errors
    ///
    /// Returns an error if the hex is invalid or wrong length.
    pub fn from_hex(s: &str) -> Result<Self, SignatureError> {
        let bytes = hex::decode(s)?;
        Self::try_from_slice(&bytes)
    }

    /// Convert to a lowercase hex string.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Get the R component of the signature (first 32 bytes).
    #[must_use]
    pub fn r_bytes(&self) -> [u8; 32] {
        let mut r = [0u8; 32];
        r.copy_from_slice(&self.0[..32]);
        r
    }

    /// Get the S component of the signature (last 32 bytes).
    #[must_use]
    pub fn s_bytes(&self) -> [u8; 32] {
        let mut s = [0u8; 32];
        s.copy_from_slice(&self.0[32..]);
        s
    }
}

impl ConstantTimeEq for Signature {
    fn ct_eq(&self, other: &Self) -> subtle::Choice {
        self.0.ct_eq(&other.0)
    }
}

impl PartialEq for Signature {
    fn eq(&self, other: &Self) -> bool {
        self.ct_eq(other).into()
    }
}

impl std::hash::Hash for Signature {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signature({}...)", &self.to_hex()[..16])
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl Default for Signature {
    fn default() -> Self {
        Self([0u8; SIGNATURE_SIZE])
    }
}

impl AsRef<[u8]> for Signature {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8; SIGNATURE_SIZE]> for Signature {
    fn as_ref(&self) -> &[u8; SIGNATURE_SIZE] {
        &self.0
    }
}

impl From<[u8; SIGNATURE_SIZE]> for Signature {
    fn from(bytes: [u8; SIGNATURE_SIZE]) -> Self {
        Self(bytes)
    }
}

impl Serialize for Signature {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            serializer.serialize_str(&self.to_hex())
        } else {
            serializer.serialize_bytes(&self.0)
        }
    }
}

impl<'de> Deserialize<'de> for Signature {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let s = String::deserialize(deserializer)?;
            Signature::from_hex(&s).map_err(serde::de::Error::custom)
        } else {
            let bytes: Vec<u8> = Deserialize::deserialize(deserializer)?;
            Signature::try_from_slice(&bytes).map_err(serde::de::Error::custom)
        }
    }
}

/// Sign a message with a private key.
///
/// This function signs the raw message bytes. For domain-separated signing,
/// use `sign_with_domain` instead.
///
/// # Example
///
/// ```rust
/// use crypto::{KeyPair, signature};
///
/// let kp = KeyPair::generate();
/// let sig = signature::sign(b"message", kp.private_key());
/// ```
#[must_use]
pub fn sign(message: &[u8], private_key: &PrivateKey) -> Signature {
    let signing_key = private_key.to_signing_key();
    let sig = signing_key.sign(message);
    Signature(sig.to_bytes())
}

/// Sign a hash value with a private key.
///
/// Use this when you've already computed the hash of the message.
#[must_use]
pub fn sign_hash(hash: &Hash, private_key: &PrivateKey) -> Signature {
    sign(hash.as_bytes(), private_key)
}

/// Sign a message with domain separation.
///
/// This hashes the message with the given domain before signing,
/// preventing cross-protocol signature attacks.
///
/// # Example
///
/// ```rust
/// use crypto::{KeyPair, signature};
///
/// let kp = KeyPair::generate();
/// let sig = signature::sign_with_domain(b"TX", b"transaction data", kp.private_key());
/// ```
#[must_use]
pub fn sign_with_domain(domain: &[u8], message: &[u8], private_key: &PrivateKey) -> Signature {
    let hash = hash_with_domain(domain, message);
    sign_hash(&hash, private_key)
}

/// Sign a message using the default DOLI signing domain.
///
/// This is the recommended signing function for general-purpose message signing.
#[must_use]
pub fn sign_message(message: &[u8], private_key: &PrivateKey) -> Signature {
    sign_with_domain(SIGN_DOMAIN, message, private_key)
}

/// Verify a signature against a message and public key.
///
/// # Errors
///
/// Returns an error if the signature is invalid.
///
/// # Example
///
/// ```rust
/// use crypto::{KeyPair, signature};
///
/// let kp = KeyPair::generate();
/// let sig = signature::sign(b"message", kp.private_key());
/// assert!(signature::verify(b"message", &sig, kp.public_key()).is_ok());
/// ```
pub fn verify(
    message: &[u8],
    signature: &Signature,
    public_key: &PublicKey,
) -> Result<(), SignatureError> {
    use ed25519_dalek::Verifier;

    let verifying_key = public_key.to_verifying_key()?;
    let sig = ed25519_dalek::Signature::from_bytes(&signature.0);

    verifying_key
        .verify(message, &sig)
        .map_err(|_| SignatureError::VerificationFailed)
}

/// Verify a signature against a hash value and public key.
///
/// Use this when you've already computed the hash of the message.
///
/// # Errors
///
/// Returns an error if the signature is invalid.
pub fn verify_hash(
    hash: &Hash,
    signature: &Signature,
    public_key: &PublicKey,
) -> Result<(), SignatureError> {
    verify(hash.as_bytes(), signature, public_key)
}

/// Verify a signature with domain separation.
///
/// This hashes the message with the given domain before verifying.
///
/// # Errors
///
/// Returns an error if the signature is invalid.
pub fn verify_with_domain(
    domain: &[u8],
    message: &[u8],
    signature: &Signature,
    public_key: &PublicKey,
) -> Result<(), SignatureError> {
    let hash = hash_with_domain(domain, message);
    verify_hash(&hash, signature, public_key)
}

/// Verify a message signed with the default DOLI signing domain.
///
/// This is the counterpart to `sign_message`.
///
/// # Errors
///
/// Returns an error if the signature is invalid.
pub fn verify_message(
    message: &[u8],
    signature: &Signature,
    public_key: &PublicKey,
) -> Result<(), SignatureError> {
    verify_with_domain(SIGN_DOMAIN, message, signature, public_key)
}

/// A signed message containing the message, signature, and signer's public key.
///
/// This is useful for self-contained signed data that can be verified
/// without additional context.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedMessage {
    /// The message that was signed.
    pub message: Vec<u8>,
    /// The signature.
    pub signature: Signature,
    /// The signer's public key.
    pub public_key: PublicKey,
}

impl SignedMessage {
    /// Create a new signed message.
    #[must_use]
    pub fn new(message: Vec<u8>, private_key: &PrivateKey) -> Self {
        let public_key = private_key.public_key();
        let signature = sign_message(&message, private_key);
        Self {
            message,
            signature,
            public_key,
        }
    }

    /// Verify the signed message.
    ///
    /// # Errors
    ///
    /// Returns an error if the signature is invalid.
    pub fn verify(&self) -> Result<(), SignatureError> {
        verify_message(&self.message, &self.signature, &self.public_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::KeyPair;

    #[test]
    fn test_sign_verify() {
        let kp = KeyPair::generate();
        let message = b"hello world";

        let sig = sign(message, kp.private_key());
        assert!(verify(message, &sig, kp.public_key()).is_ok());
    }

    #[test]
    fn test_verify_wrong_message() {
        let kp = KeyPair::generate();
        let message = b"hello world";

        let sig = sign(message, kp.private_key());
        assert!(verify(b"wrong message", &sig, kp.public_key()).is_err());
    }

    #[test]
    fn test_verify_wrong_key() {
        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();
        let message = b"hello world";

        let sig = sign(message, kp1.private_key());
        assert!(verify(message, &sig, kp2.public_key()).is_err());
    }

    #[test]
    fn test_signature_deterministic() {
        let kp = KeyPair::generate();
        let message = b"test message";

        // Ed25519 signatures are deterministic
        let sig1 = sign(message, kp.private_key());
        let sig2 = sign(message, kp.private_key());
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn test_signature_hex_roundtrip() {
        let kp = KeyPair::generate();
        let sig = sign(b"test", kp.private_key());

        let hex = sig.to_hex();
        let recovered = Signature::from_hex(&hex).expect("valid hex");
        assert_eq!(sig, recovered);
    }

    #[test]
    fn test_domain_separated_signing() {
        let kp = KeyPair::generate();
        let message = b"test message";

        // Different domains produce different signatures
        let sig1 = sign_with_domain(b"DOMAIN_A", message, kp.private_key());
        let sig2 = sign_with_domain(b"DOMAIN_B", message, kp.private_key());
        assert_ne!(sig1, sig2);

        // Can only verify with the correct domain
        assert!(verify_with_domain(b"DOMAIN_A", message, &sig1, kp.public_key()).is_ok());
        assert!(verify_with_domain(b"DOMAIN_B", message, &sig1, kp.public_key()).is_err());
    }

    #[test]
    fn test_sign_message_and_verify_message() {
        let kp = KeyPair::generate();
        let message = b"Hello, DOLI!";

        let sig = sign_message(message, kp.private_key());
        assert!(verify_message(message, &sig, kp.public_key()).is_ok());
    }

    #[test]
    fn test_signed_message() {
        let kp = KeyPair::generate();
        let message = b"test data".to_vec();

        let signed = SignedMessage::new(message.clone(), kp.private_key());
        assert!(signed.verify().is_ok());
        assert_eq!(signed.message, message);
        assert_eq!(signed.public_key, *kp.public_key());
    }

    #[test]
    fn test_signature_components() {
        let kp = KeyPair::generate();
        let sig = sign(b"test", kp.private_key());

        let r = sig.r_bytes();
        let s = sig.s_bytes();

        // Components should be 32 bytes each
        assert_eq!(r.len(), 32);
        assert_eq!(s.len(), 32);

        // Components should match the signature bytes
        assert_eq!(&r, &sig.as_bytes()[..32]);
        assert_eq!(&s, &sig.as_bytes()[32..]);
    }

    #[test]
    fn test_serde_json() {
        let kp = KeyPair::generate();
        let sig = sign(b"test", kp.private_key());

        let json = serde_json::to_string(&sig).expect("serialize");
        let recovered: Signature = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(sig, recovered);
    }

    #[test]
    fn test_constant_time_eq() {
        let kp = KeyPair::generate();
        let sig1 = sign(b"test", kp.private_key());
        let sig2 = sign(b"test", kp.private_key());
        let sig3 = sign(b"other", kp.private_key());

        assert!(bool::from(sig1.ct_eq(&sig2)));
        assert!(!bool::from(sig1.ct_eq(&sig3)));
    }

    // Property-based tests
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn prop_sign_verify(message: Vec<u8>, seed: [u8; 32]) {
            let kp = KeyPair::from_seed(seed);
            let sig = sign(&message, kp.private_key());
            prop_assert!(verify(&message, &sig, kp.public_key()).is_ok());
        }

        #[test]
        fn prop_signature_hex_roundtrip(message: Vec<u8>, seed: [u8; 32]) {
            let kp = KeyPair::from_seed(seed);
            let sig = sign(&message, kp.private_key());
            let hex = sig.to_hex();
            let recovered = Signature::from_hex(&hex).unwrap();
            prop_assert_eq!(sig, recovered);
        }

        #[test]
        fn prop_wrong_key_fails(message: Vec<u8>, seed1: [u8; 32], seed2: [u8; 32]) {
            prop_assume!(seed1 != seed2);
            let kp1 = KeyPair::from_seed(seed1);
            let kp2 = KeyPair::from_seed(seed2);
            let sig = sign(&message, kp1.private_key());
            prop_assert!(verify(&message, &sig, kp2.public_key()).is_err());
        }
    }
}
