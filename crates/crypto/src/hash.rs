//! BLAKE3-256 hashing with domain separation.
//!
//! This module provides the core hashing functionality for DOLI using BLAKE3,
//! a cryptographic hash function that is:
//!
//! - **Fast**: BLAKE3 is significantly faster than SHA-256 and SHA-3
//! - **Secure**: 256-bit output with 128-bit security level
//! - **Parallelizable**: Can leverage SIMD and multiple cores
//! - **Versatile**: Supports keyed hashing, key derivation, and XOF mode
//!
//! ## Domain Separation
//!
//! All hash operations use domain separation to prevent cross-protocol attacks.
//! Different contexts (transactions, blocks, addresses) use different domain tags.
//!
//! ## Test Vectors
//!
//! The implementation is verified against official BLAKE3 test vectors and
//! DOLI protocol specification vectors.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use subtle::ConstantTimeEq;

use crate::HASH_SIZE;

/// A 32-byte BLAKE3-256 hash.
///
/// This type provides constant-time equality comparison to prevent
/// timing side-channel attacks when comparing hashes.
#[derive(Clone, Copy, Eq)]
pub struct Hash([u8; HASH_SIZE]);

impl Hash {
    /// The zero hash (all bytes are 0x00).
    pub const ZERO: Self = Self([0u8; HASH_SIZE]);

    /// Create a hash from raw bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; HASH_SIZE]) -> Self {
        Self(bytes)
    }

    /// Legacy method - use `ZERO` constant instead.
    #[must_use]
    #[deprecated(since = "0.1.0", note = "Use Hash::ZERO instead")]
    pub const fn zero() -> Self {
        Self::ZERO
    }

    /// Try to create a hash from a byte slice.
    ///
    /// Returns `None` if the slice is not exactly 32 bytes.
    #[must_use]
    pub fn try_from_slice(slice: &[u8]) -> Option<Self> {
        if slice.len() != HASH_SIZE {
            return None;
        }
        let mut bytes = [0u8; HASH_SIZE];
        bytes.copy_from_slice(slice);
        Some(Self(bytes))
    }

    /// Get the underlying bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; HASH_SIZE] {
        &self.0
    }

    /// Convert to a byte vector.
    #[must_use]
    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }

    /// Create from a hex string.
    ///
    /// Returns `None` if the hex string is invalid or wrong length.
    #[must_use]
    pub fn from_hex(s: &str) -> Option<Self> {
        let bytes = hex::decode(s).ok()?;
        Self::try_from_slice(&bytes)
    }

    /// Convert to a lowercase hex string.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Check if this is the zero hash.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.ct_eq(&Self::ZERO).into()
    }

    /// Get the first N bytes of the hash.
    ///
    /// Useful for creating truncated identifiers.
    #[must_use]
    pub fn prefix<const N: usize>(&self) -> [u8; N] {
        let mut result = [0u8; N];
        result.copy_from_slice(&self.0[..N]);
        result
    }

    /// XOR this hash with another.
    ///
    /// Useful for hash-based accumulators and commitment schemes.
    #[must_use]
    pub fn xor(&self, other: &Self) -> Self {
        let mut result = [0u8; HASH_SIZE];
        for (i, (a, b)) in self.0.iter().zip(other.0.iter()).enumerate() {
            result[i] = a ^ b;
        }
        Self(result)
    }
}

// Constant-time equality comparison
impl ConstantTimeEq for Hash {
    fn ct_eq(&self, other: &Self) -> subtle::Choice {
        self.0.ct_eq(&other.0)
    }
}

impl PartialEq for Hash {
    fn eq(&self, other: &Self) -> bool {
        self.ct_eq(other).into()
    }
}

// Standard library Hash trait for use in HashMaps
impl std::hash::Hash for Hash {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl PartialOrd for Hash {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Hash {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl Default for Hash {
    fn default() -> Self {
        Self::ZERO
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({}...)", &self.to_hex()[..8])
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl AsRef<[u8]> for Hash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8; HASH_SIZE]> for Hash {
    fn as_ref(&self) -> &[u8; HASH_SIZE] {
        &self.0
    }
}

impl From<[u8; HASH_SIZE]> for Hash {
    fn from(bytes: [u8; HASH_SIZE]) -> Self {
        Self(bytes)
    }
}

impl From<Hash> for [u8; HASH_SIZE] {
    fn from(hash: Hash) -> Self {
        hash.0
    }
}

impl Serialize for Hash {
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

impl<'de> Deserialize<'de> for Hash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let s = String::deserialize(deserializer)?;
            Hash::from_hex(&s).ok_or_else(|| serde::de::Error::custom("invalid hash hex"))
        } else {
            let bytes: Vec<u8> = Deserialize::deserialize(deserializer)?;
            Hash::try_from_slice(&bytes)
                .ok_or_else(|| serde::de::Error::custom("invalid hash length"))
        }
    }
}

/// Incremental hasher for streaming data.
///
/// Use this when you need to hash data in chunks, or when you need
/// to hash large amounts of data without loading it all into memory.
///
/// # Example
///
/// ```rust
/// use crypto::Hasher;
///
/// let mut hasher = Hasher::new();
/// hasher.update(b"Hello, ");
/// hasher.update(b"World!");
/// let hash = hasher.finalize();
/// ```
pub struct Hasher {
    inner: blake3::Hasher,
}

impl Hasher {
    /// Create a new hasher.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: blake3::Hasher::new(),
        }
    }

    /// Create a new hasher with a domain separation context.
    ///
    /// This should be used when hashing for a specific purpose to prevent
    /// cross-protocol attacks.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn new_with_domain(domain: &[u8]) -> Self {
        let mut hasher = Self::new();
        // Length-prefix the domain to prevent ambiguity (domain length is always small)
        hasher.update(&(domain.len() as u32).to_le_bytes());
        hasher.update(domain);
        hasher
    }

    /// Create a keyed hasher for MAC operations.
    ///
    /// The key must be exactly 32 bytes.
    #[must_use]
    pub fn new_keyed(key: &[u8; 32]) -> Self {
        Self {
            inner: blake3::Hasher::new_keyed(key),
        }
    }

    /// Update the hasher with more data.
    pub fn update(&mut self, data: &[u8]) -> &mut Self {
        self.inner.update(data);
        self
    }

    /// Update with length-prefixed data.
    ///
    /// Useful for unambiguous encoding of variable-length fields.
    pub fn update_with_length(&mut self, data: &[u8]) -> &mut Self {
        self.update(&(data.len() as u64).to_le_bytes());
        self.update(data);
        self
    }

    /// Finalize the hash and return the result.
    #[must_use]
    pub fn finalize(&self) -> Hash {
        Hash(*self.inner.finalize().as_bytes())
    }

    /// Reset the hasher to its initial state.
    pub fn reset(&mut self) {
        self.inner.reset();
    }
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for Hasher {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Hash arbitrary data with BLAKE3-256.
///
/// This is the simplest hashing function for one-shot hashing.
#[must_use]
pub fn hash(data: &[u8]) -> Hash {
    Hash(*blake3::hash(data).as_bytes())
}

/// Hash data with domain separation.
///
/// Use this when hashing for a specific purpose (transactions, blocks, etc.)
/// to prevent cross-protocol attacks.
#[must_use]
pub fn hash_with_domain(domain: &[u8], data: &[u8]) -> Hash {
    let mut hasher = Hasher::new_with_domain(domain);
    hasher.update(data);
    hasher.finalize()
}

/// Hash multiple pieces of data together.
///
/// More efficient than concatenating data and hashing.
#[must_use]
pub fn hash_many(data: &[&[u8]]) -> Hash {
    let mut hasher = Hasher::new();
    for chunk in data {
        hasher.update(chunk);
    }
    hasher.finalize()
}

/// Legacy alias for `hash_many`.
#[must_use]
pub fn hash_concat(parts: &[&[u8]]) -> Hash {
    hash_many(parts)
}

/// Derive a key from a master key and context string.
///
/// This uses BLAKE3's key derivation mode which is suitable for
/// deriving multiple keys from a single master key.
#[must_use]
pub fn derive_key(context: &str, key_material: &[u8]) -> [u8; 32] {
    blake3::derive_key(context, key_material)
}

/// Double-hash for commitment schemes.
///
/// H(H(x)) provides additional security margin in some protocols.
#[must_use]
pub fn hash_twice(data: &[u8]) -> Hash {
    hash(hash(data).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Official BLAKE3 test vectors from https://github.com/BLAKE3-team/BLAKE3
    #[test]
    fn test_empty_input() {
        let h = hash(b"");
        assert_eq!(
            h.to_hex(),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn test_single_zero_byte() {
        let h = hash(&[0x00]);
        assert_eq!(
            h.to_hex(),
            "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213"
        );
    }

    #[test]
    fn test_hash_deterministic() {
        let h1 = hash(b"hello world");
        let h2 = hash(b"hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_different_inputs() {
        let h1 = hash(b"hello");
        let h2 = hash(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_incremental_hash() {
        let direct = hash(b"hello world");

        let mut hasher = Hasher::new();
        hasher.update(b"hello ");
        hasher.update(b"world");
        let incremental = hasher.finalize();

        assert_eq!(direct, incremental);
    }

    #[test]
    fn test_hash_hex_roundtrip() {
        let h = hash(b"test");
        let hex_str = h.to_hex();
        let recovered = Hash::from_hex(&hex_str).expect("valid hex");
        assert_eq!(h, recovered);
    }

    #[test]
    fn test_invalid_hex() {
        assert!(Hash::from_hex("not valid hex").is_none());
        assert!(Hash::from_hex("ab").is_none()); // Too short
        assert!(Hash::from_hex(&"ab".repeat(33)).is_none()); // Too long
    }

    #[test]
    fn test_zero_hash() {
        assert!(Hash::ZERO.is_zero());
        assert!(!hash(b"not zero").is_zero());
    }

    #[test]
    fn test_constant_time_eq() {
        let a = hash(b"test");
        let b = hash(b"test");
        let c = hash(b"other");

        assert!(bool::from(a.ct_eq(&b)));
        assert!(!bool::from(a.ct_eq(&c)));
    }

    #[test]
    fn test_domain_separation() {
        let data = b"same data";
        let h1 = hash_with_domain(b"DOMAIN_A", data);
        let h2 = hash_with_domain(b"DOMAIN_B", data);
        let h3 = hash(data);

        assert_ne!(h1, h2);
        assert_ne!(h1, h3);
        assert_ne!(h2, h3);
    }

    #[test]
    fn test_keyed_hash() {
        let key = [0x42u8; 32];
        let mut hasher = Hasher::new_keyed(&key);
        hasher.update(b"message");
        let mac = hasher.finalize();

        // Same key, same message = same MAC
        let mut hasher2 = Hasher::new_keyed(&key);
        hasher2.update(b"message");
        assert_eq!(mac, hasher2.finalize());

        // Different key = different MAC
        let key2 = [0x43u8; 32];
        let mut hasher3 = Hasher::new_keyed(&key2);
        hasher3.update(b"message");
        assert_ne!(mac, hasher3.finalize());
    }

    #[test]
    fn test_key_derivation() {
        let master = b"master key material";

        let key1 = derive_key("DOLI_KEY_1", master);
        let key2 = derive_key("DOLI_KEY_2", master);

        assert_ne!(key1, key2);
        assert_eq!(key1, derive_key("DOLI_KEY_1", master));
    }

    #[test]
    fn test_hash_xor() {
        let a = hash(b"a");
        let b = hash(b"b");

        let xored = a.xor(&b);
        // XOR is self-inverse
        assert_eq!(xored.xor(&b), a);
        assert_eq!(xored.xor(&a), b);
        // XOR with self is zero
        assert!(a.xor(&a).is_zero());
    }

    #[test]
    fn test_prefix() {
        let h = hash(b"test");
        let prefix: [u8; 4] = h.prefix();
        assert_eq!(&prefix, &h.as_bytes()[..4]);
    }

    #[test]
    fn test_serde_json() {
        let original = hash(b"test");
        let json = serde_json::to_string(&original).expect("serialize");
        let recovered: Hash = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    // DOLI Protocol Specification Test Vectors
    #[test]
    fn test_seed_vector_slot_0() {
        // Test vector from protocol spec
        // SEED || prev_hash (32 zeros) || slot (0)
        let mut input = Vec::new();
        input.extend_from_slice(b"SEED");
        input.extend_from_slice(&[0u8; 32]); // prev_hash
        input.extend_from_slice(&0u32.to_le_bytes()); // slot

        let result = hash(&input);
        assert_eq!(
            result.to_hex(),
            "f3b4b63bfa289f7b4b2f11f08cfc26bd38ccdbdd9dae33ef9b77c1fc3b96ebb2"
        );
    }

    #[test]
    fn test_seed_vector_slot_1() {
        // Test vector from protocol spec
        let mut input = Vec::new();
        input.extend_from_slice(b"SEED");
        input.extend_from_slice(&[0u8; 32]); // prev_hash
        input.extend_from_slice(&1u32.to_le_bytes()); // slot = 1

        let result = hash(&input);
        assert_eq!(
            result.to_hex(),
            "ac1d2a15e55cc413c69036ba29cd08066a560a5bf152ac89a35089eae1fd6bbe"
        );
    }

    // Property-based tests
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn prop_hash_deterministic(data: Vec<u8>) {
            let h1 = hash(&data);
            let h2 = hash(&data);
            prop_assert_eq!(h1, h2);
        }

        #[test]
        fn prop_different_data_different_hash(a: Vec<u8>, b: Vec<u8>) {
            prop_assume!(a != b);
            let h1 = hash(&a);
            let h2 = hash(&b);
            prop_assert_ne!(h1, h2);
        }

        #[test]
        fn prop_hex_roundtrip(data: Vec<u8>) {
            let h = hash(&data);
            let hex = h.to_hex();
            let recovered = Hash::from_hex(&hex).unwrap();
            prop_assert_eq!(h, recovered);
        }

        #[test]
        fn prop_xor_self_inverse(data: Vec<u8>) {
            let h = hash(&data);
            let other = hash(b"other");
            prop_assert_eq!(h.xor(&other).xor(&other), h);
        }

        #[test]
        fn prop_xor_self_is_zero(data: Vec<u8>) {
            let h = hash(&data);
            prop_assert!(h.xor(&h).is_zero());
        }
    }
}
