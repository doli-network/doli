//! Ed25519 key management with security hardening.
//!
//! This module provides key generation, derivation, and management for Ed25519
//! digital signatures. It implements several security best practices:
//!
//! - **Zeroization**: Private keys are automatically zeroized when dropped
//! - **Constant-time comparison**: Prevents timing attacks on key comparison
//! - **Type safety**: Distinct types for private keys, public keys, and addresses
//! - **Secure generation**: Uses OS-provided cryptographic randomness
//!
//! ## Security Considerations
//!
//! - Never serialize private keys without encryption
//! - Never log or display private keys (Debug impl shows "[redacted]")
//! - Use `KeyPair` when you need both keys to ensure they match
//! - Always verify signatures with the expected public key

use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use subtle::ConstantTimeEq;
use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::address_type::Address;
use crate::hash::hash_with_domain;
use crate::{ADDRESS_DOMAIN, ADDRESS_SIZE, PRIVATE_KEY_SIZE, PUBLIC_KEY_SIZE};

/// Errors related to key operations.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum KeyError {
    /// The provided key has an invalid length.
    #[error("invalid key length: expected {expected}, got {got}")]
    InvalidLength {
        /// Expected length in bytes.
        expected: usize,
        /// Actual length in bytes.
        got: usize,
    },

    /// The key bytes do not represent a valid key.
    #[error("invalid key bytes: not a valid Ed25519 key")]
    InvalidKey,

    /// The provided hex string is invalid.
    #[error("invalid hex string: {0}")]
    InvalidHex(String),
}

impl From<hex::FromHexError> for KeyError {
    fn from(e: hex::FromHexError) -> Self {
        KeyError::InvalidHex(e.to_string())
    }
}

/// A 32-byte Ed25519 public key.
///
/// Public keys can be freely shared and are used to verify signatures
/// and derive addresses.
#[derive(Clone, Copy, Eq)]
pub struct PublicKey([u8; PUBLIC_KEY_SIZE]);

impl PublicKey {
    /// Create a public key from raw bytes.
    ///
    /// Note: This does not validate the bytes represent a valid curve point.
    /// Use `try_from_slice` for validation.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; PUBLIC_KEY_SIZE]) -> Self {
        Self(bytes)
    }

    /// Try to create a public key from a byte slice with validation.
    ///
    /// # Errors
    ///
    /// Returns an error if the slice length is wrong or if the bytes
    /// don't represent a valid Ed25519 public key.
    pub fn try_from_slice(slice: &[u8]) -> Result<Self, KeyError> {
        if slice.len() != PUBLIC_KEY_SIZE {
            return Err(KeyError::InvalidLength {
                expected: PUBLIC_KEY_SIZE,
                got: slice.len(),
            });
        }
        let mut bytes = [0u8; PUBLIC_KEY_SIZE];
        bytes.copy_from_slice(slice);

        // Validate it's a valid curve point
        VerifyingKey::from_bytes(&bytes).map_err(|_| KeyError::InvalidKey)?;

        Ok(Self(bytes))
    }

    /// Get the underlying bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; PUBLIC_KEY_SIZE] {
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
    /// Returns an error if the hex is invalid or represents an invalid key.
    pub fn from_hex(s: &str) -> Result<Self, KeyError> {
        let bytes = hex::decode(s)?;
        Self::try_from_slice(&bytes)
    }

    /// Convert to a lowercase hex string.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Derive the address from this public key.
    ///
    /// The address is the first 20 bytes of the domain-separated hash
    /// of the public key.
    #[must_use]
    pub fn to_address(&self) -> Address {
        let h = hash_with_domain(ADDRESS_DOMAIN, &self.0);
        let mut addr = [0u8; ADDRESS_SIZE];
        addr.copy_from_slice(&h.as_bytes()[..ADDRESS_SIZE]);
        Address(addr)
    }

    /// Get the internal verifying key for signature verification.
    pub(crate) fn to_verifying_key(self) -> Result<VerifyingKey, KeyError> {
        VerifyingKey::from_bytes(&self.0).map_err(|_| KeyError::InvalidKey)
    }
}

impl ConstantTimeEq for PublicKey {
    fn ct_eq(&self, other: &Self) -> subtle::Choice {
        self.0.ct_eq(&other.0)
    }
}

impl PartialEq for PublicKey {
    fn eq(&self, other: &Self) -> bool {
        self.ct_eq(other).into()
    }
}

impl std::hash::Hash for PublicKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({}...)", &self.to_hex()[..8])
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl AsRef<[u8]> for PublicKey {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8; PUBLIC_KEY_SIZE]> for PublicKey {
    fn as_ref(&self) -> &[u8; PUBLIC_KEY_SIZE] {
        &self.0
    }
}

impl From<[u8; PUBLIC_KEY_SIZE]> for PublicKey {
    fn from(bytes: [u8; PUBLIC_KEY_SIZE]) -> Self {
        Self(bytes)
    }
}

impl Serialize for PublicKey {
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

impl<'de> Deserialize<'de> for PublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let s = String::deserialize(deserializer)?;
            PublicKey::from_hex(&s).map_err(serde::de::Error::custom)
        } else {
            let bytes: Vec<u8> = Deserialize::deserialize(deserializer)?;
            PublicKey::try_from_slice(&bytes).map_err(serde::de::Error::custom)
        }
    }
}

/// A 32-byte Ed25519 private key (seed).
///
/// This type automatically zeroizes its contents when dropped to prevent
/// secrets from lingering in memory.
///
/// # Security
///
/// - Never log or display private keys
/// - Never serialize without encryption
/// - Consider using `KeyPair` instead for safer key management
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct PrivateKey([u8; PRIVATE_KEY_SIZE]);

impl PrivateKey {
    /// Create a private key from raw bytes.
    ///
    /// # Security
    ///
    /// The input bytes should come from a cryptographically secure source.
    #[must_use]
    pub fn from_bytes(bytes: [u8; PRIVATE_KEY_SIZE]) -> Self {
        Self(bytes)
    }

    /// Generate a new random private key using OS randomness.
    ///
    /// This uses `OsRng` which provides cryptographically secure random bytes.
    #[must_use]
    pub fn generate() -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        Self(signing_key.to_bytes())
    }

    /// Get the underlying bytes.
    ///
    /// # Security
    ///
    /// Be careful with the returned reference - don't log or persist it.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; PRIVATE_KEY_SIZE] {
        &self.0
    }

    /// Derive the public key from this private key.
    #[must_use]
    pub fn public_key(&self) -> PublicKey {
        let signing_key = SigningKey::from_bytes(&self.0);
        let verifying_key = signing_key.verifying_key();
        PublicKey(verifying_key.to_bytes())
    }

    /// Get the internal signing key.
    pub(crate) fn to_signing_key(&self) -> SigningKey {
        SigningKey::from_bytes(&self.0)
    }

    /// Create from a hex string.
    ///
    /// # Errors
    ///
    /// Returns an error if the hex is invalid or wrong length.
    ///
    /// # Security
    ///
    /// Be careful when loading keys from strings - ensure the source is secure.
    pub fn from_hex(s: &str) -> Result<Self, KeyError> {
        let bytes = hex::decode(s)?;
        if bytes.len() != PRIVATE_KEY_SIZE {
            return Err(KeyError::InvalidLength {
                expected: PRIVATE_KEY_SIZE,
                got: bytes.len(),
            });
        }
        let mut arr = [0u8; PRIVATE_KEY_SIZE];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }

    /// Convert to a hex string.
    ///
    /// # Security
    ///
    /// Be careful with the returned string - don't log or display it.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl Clone for PrivateKey {
    fn clone(&self) -> Self {
        Self(self.0)
    }
}

// Debug intentionally hides the key material
impl fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PrivateKey([REDACTED])")
    }
}

/// A key pair containing both private and public keys.
///
/// Using `KeyPair` ensures that the public key always matches the private key.
/// The private key is automatically zeroized when the keypair is dropped.
pub struct KeyPair {
    private: PrivateKey,
    public: PublicKey,
}

impl KeyPair {
    /// Generate a new random key pair.
    #[must_use]
    pub fn generate() -> Self {
        let private = PrivateKey::generate();
        let public = private.public_key();
        Self { private, public }
    }

    /// Create a key pair from a private key.
    ///
    /// The public key is derived from the private key.
    #[must_use]
    pub fn from_private_key(private: PrivateKey) -> Self {
        let public = private.public_key();
        Self { private, public }
    }

    /// Create a key pair from a private key seed (raw bytes).
    #[must_use]
    pub fn from_seed(seed: [u8; PRIVATE_KEY_SIZE]) -> Self {
        Self::from_private_key(PrivateKey::from_bytes(seed))
    }

    /// Get a reference to the private key.
    #[must_use]
    pub fn private_key(&self) -> &PrivateKey {
        &self.private
    }

    /// Get a reference to the public key.
    #[must_use]
    pub fn public_key(&self) -> &PublicKey {
        &self.public
    }

    /// Get the address derived from the public key.
    #[must_use]
    pub fn address(&self) -> Address {
        self.public.to_address()
    }
}

impl Clone for KeyPair {
    fn clone(&self) -> Self {
        Self {
            private: self.private.clone(),
            public: self.public,
        }
    }
}

// Debug intentionally hides the private key
impl fmt::Debug for KeyPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyPair")
            .field("public", &self.public)
            .field("private", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_key_generation() {
        let kp = KeyPair::generate();
        assert_eq!(kp.public_key().as_bytes().len(), PUBLIC_KEY_SIZE);
        assert_eq!(kp.private_key().as_bytes().len(), PRIVATE_KEY_SIZE);
    }

    #[test]
    fn test_key_generation_unique() {
        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();
        assert_ne!(kp1.public_key(), kp2.public_key());
    }

    #[test]
    fn test_public_key_from_private() {
        let private = PrivateKey::generate();
        let public1 = private.public_key();
        let public2 = private.public_key();
        assert_eq!(public1, public2);
    }

    #[test]
    fn test_hex_roundtrip() {
        let kp = KeyPair::generate();

        let pub_hex = kp.public_key().to_hex();
        let pub_recovered = PublicKey::from_hex(&pub_hex).expect("valid hex");
        assert_eq!(kp.public_key(), &pub_recovered);

        let priv_hex = kp.private_key().to_hex();
        let priv_recovered = PrivateKey::from_hex(&priv_hex).expect("valid hex");
        assert_eq!(kp.private_key().as_bytes(), priv_recovered.as_bytes());
    }

    #[test]
    fn test_private_key_debug_redacted() {
        let private = PrivateKey::generate();
        let debug = format!("{private:?}");
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains(&private.to_hex()));
    }

    #[test]
    fn test_keypair_from_seed() {
        let seed = [42u8; 32];
        let kp1 = KeyPair::from_seed(seed);
        let kp2 = KeyPair::from_seed(seed);
        assert_eq!(kp1.public_key(), kp2.public_key());
    }

    #[test]
    fn test_constant_time_eq() {
        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();

        assert!(bool::from(kp1.public_key().ct_eq(kp1.public_key())));
        assert!(!bool::from(kp1.public_key().ct_eq(kp2.public_key())));
    }

    #[test]
    fn test_invalid_public_key_length() {
        // Wrong length should fail
        let result = PublicKey::try_from_slice(&[0u8; 16]);
        assert!(result.is_err());

        let result = PublicKey::try_from_slice(&[0u8; 64]);
        assert!(result.is_err());

        let result = PublicKey::try_from_slice(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_valid_public_key_from_keypair() {
        // A public key derived from a keypair should always be valid
        let kp = KeyPair::generate();
        let pub_bytes = kp.public_key().as_bytes();
        let result = PublicKey::try_from_slice(pub_bytes);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), *kp.public_key());
    }

    #[test]
    fn test_serde_json_public_key() {
        let kp = KeyPair::generate();

        let pub_json = serde_json::to_string(kp.public_key()).expect("serialize");
        let pub_recovered: PublicKey = serde_json::from_str(&pub_json).expect("deserialize");
        assert_eq!(kp.public_key(), &pub_recovered);
    }

    // Property-based tests
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn prop_keypair_roundtrip(seed: [u8; 32]) {
            let kp1 = KeyPair::from_seed(seed);
            let kp2 = KeyPair::from_seed(seed);
            prop_assert_eq!(kp1.public_key(), kp2.public_key());
            prop_assert_eq!(kp1.address(), kp2.address());
        }
    }
}
