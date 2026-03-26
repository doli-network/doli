//! The 20-byte `Address` type derived from an Ed25519 public key.
//!
//! Addresses are the primary way to identify accounts in DOLI.
//! They are derived by hashing the public key with domain separation
//! and taking the first 20 bytes.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use subtle::ConstantTimeEq;

use crate::hash::hash;
use crate::keys::KeyError;
use crate::ADDRESS_SIZE;

/// A 20-byte address derived from a public key.
///
/// Addresses are the primary way to identify accounts in DOLI.
/// They are derived by hashing the public key with domain separation
/// and taking the first 20 bytes.
#[derive(Clone, Copy, Eq)]
pub struct Address(pub(crate) [u8; ADDRESS_SIZE]);

impl Address {
    /// The zero address (all bytes are 0x00).
    pub const ZERO: Self = Self([0u8; ADDRESS_SIZE]);

    /// Create an address from raw bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; ADDRESS_SIZE]) -> Self {
        Self(bytes)
    }

    /// Try to create an address from a byte slice.
    ///
    /// # Errors
    ///
    /// Returns an error if the slice is not exactly 20 bytes.
    pub fn try_from_slice(slice: &[u8]) -> Result<Self, KeyError> {
        if slice.len() != ADDRESS_SIZE {
            return Err(KeyError::InvalidLength {
                expected: ADDRESS_SIZE,
                got: slice.len(),
            });
        }
        let mut bytes = [0u8; ADDRESS_SIZE];
        bytes.copy_from_slice(slice);
        Ok(Self(bytes))
    }

    /// Get the underlying bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; ADDRESS_SIZE] {
        &self.0
    }

    /// Convert to a byte vector.
    #[must_use]
    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }

    /// Create the canonical burn address.
    ///
    /// Coins sent to this address are permanently unspendable.
    #[must_use]
    pub fn burn() -> Self {
        let h = hash(b"DOLI_BURN_ADDRESS_V1");
        let mut addr = [0u8; ADDRESS_SIZE];
        addr.copy_from_slice(&h.as_bytes()[..ADDRESS_SIZE]);
        Self(addr)
    }

    /// Check if this is the zero address.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.ct_eq(&Self::ZERO).into()
    }

    /// Check if this is the burn address.
    #[must_use]
    pub fn is_burn(&self) -> bool {
        self.ct_eq(&Self::burn()).into()
    }

    /// Create from a hex string.
    ///
    /// # Errors
    ///
    /// Returns an error if the hex is invalid or wrong length.
    pub fn from_hex(s: &str) -> Result<Self, KeyError> {
        let bytes = hex::decode(s)?;
        Self::try_from_slice(&bytes)
    }

    /// Convert to a lowercase hex string.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Convert to a checksummed hex string (EIP-55 style).
    ///
    /// This provides error detection for manually entered addresses.
    #[must_use]
    pub fn to_checksum_hex(&self) -> String {
        let hex_addr = hex::encode(self.0);
        let hash = hash(hex_addr.as_bytes());
        let hash_hex = hex::encode(hash.as_bytes());

        let mut result = String::with_capacity(40);
        for (i, c) in hex_addr.chars().enumerate() {
            if c.is_ascii_alphabetic() {
                // Use the hash nibble to determine case
                let hash_nibble = u8::from_str_radix(&hash_hex[i..=i], 16).unwrap_or(0);
                if hash_nibble >= 8 {
                    result.push(c.to_ascii_uppercase());
                } else {
                    result.push(c);
                }
            } else {
                result.push(c);
            }
        }
        result
    }
}

impl ConstantTimeEq for Address {
    fn ct_eq(&self, other: &Self) -> subtle::Choice {
        self.0.ct_eq(&other.0)
    }
}

impl PartialEq for Address {
    fn eq(&self, other: &Self) -> bool {
        self.ct_eq(other).into()
    }
}

impl std::hash::Hash for Address {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl PartialOrd for Address {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Address {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl Default for Address {
    fn default() -> Self {
        Self::ZERO
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Address({}...)", &self.to_hex()[..8])
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl AsRef<[u8]> for Address {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8; ADDRESS_SIZE]> for Address {
    fn as_ref(&self) -> &[u8; ADDRESS_SIZE] {
        &self.0
    }
}

impl From<[u8; ADDRESS_SIZE]> for Address {
    fn from(bytes: [u8; ADDRESS_SIZE]) -> Self {
        Self(bytes)
    }
}

impl Serialize for Address {
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

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let s = String::deserialize(deserializer)?;
            // Accept both checksummed and non-checksummed addresses
            let s = s.to_lowercase();
            Address::from_hex(&s).map_err(serde::de::Error::custom)
        } else {
            let bytes: Vec<u8> = Deserialize::deserialize(deserializer)?;
            Address::try_from_slice(&bytes).map_err(serde::de::Error::custom)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::keys::KeyPair;
    use crate::ADDRESS_SIZE;

    #[test]
    fn test_address_derivation() {
        let kp = KeyPair::generate();
        let addr = kp.address();
        assert_eq!(addr.as_bytes().len(), ADDRESS_SIZE);
    }

    #[test]
    fn test_address_deterministic() {
        let kp = KeyPair::generate();
        let addr1 = kp.public_key().to_address();
        let addr2 = kp.public_key().to_address();
        assert_eq!(addr1, addr2);
    }

    #[test]
    fn test_address_hex_roundtrip() {
        let kp = KeyPair::generate();
        let addr_hex = kp.address().to_hex();
        let addr_recovered = Address::from_hex(&addr_hex).expect("valid hex");
        assert_eq!(kp.address(), addr_recovered);
    }

    #[test]
    fn test_burn_address() {
        let burn = Address::burn();
        assert!(burn.is_burn());
        assert!(!burn.is_zero());

        // Burn address is deterministic
        assert_eq!(burn, Address::burn());
    }

    #[test]
    fn test_zero_address() {
        let zero = Address::ZERO;
        assert!(zero.is_zero());
        assert!(!zero.is_burn());
    }

    #[test]
    fn test_checksum_hex() {
        let kp = KeyPair::generate();
        let checksum = kp.address().to_checksum_hex();
        assert_eq!(checksum.len(), 40);
        // Checksum should contain some uppercase letters
        assert!(checksum.chars().any(|c| c.is_ascii_uppercase()));
    }

    #[test]
    fn test_serde_json_address() {
        let kp = KeyPair::generate();

        let addr_json = serde_json::to_string(&kp.address()).expect("serialize");
        let addr_recovered: Address = serde_json::from_str(&addr_json).expect("deserialize");
        assert_eq!(kp.address(), addr_recovered);
    }

    // Property-based tests
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn prop_address_hex_roundtrip(seed: [u8; 32]) {
            let kp = KeyPair::from_seed(seed);
            let hex = kp.address().to_hex();
            let recovered = Address::from_hex(&hex).unwrap();
            prop_assert_eq!(kp.address(), recovered);
        }
    }
}
