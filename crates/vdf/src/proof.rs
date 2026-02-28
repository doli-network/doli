//! VDF proof structure and operations
//!
//! This module defines the Wesolowski proof format used to efficiently
//! verify VDF computations.
//!
//! ## The Wesolowski Proof
//!
//! A Wesolowski proof is a single group element π that allows verification
//! of y = x^(2^t) in O(log t) operations instead of O(t).
//!
//! Given:
//! - Input: x (group element)
//! - Output: y = x^(2^t) (group element)
//! - Proof: π = x^q where q = floor(2^t / l)
//! - Challenge: l (prime from Fiat-Shamir)
//!
//! Verification checks: y == π^l · x^r where r = 2^t mod l
//!
//! This works because:
//! - 2^t = l·q + r (by definition of q and r)
//! - So π^l · x^r = x^(l·q) · x^r = x^(l·q + r) = x^(2^t) = y

use serde::{Deserialize, Serialize};

use crate::class_group::ClassGroupElement;

/// A Wesolowski VDF proof.
///
/// The proof allows verification that y = x^(2^t) was computed correctly
/// using only O(log t) group operations for verification, compared to
/// O(t) operations for direct computation.
///
/// # Size
///
/// The proof size depends on the discriminant size:
/// - 2048-bit discriminant: ~512 bytes
/// - 1024-bit discriminant: ~256 bytes
///
/// # Security
///
/// The proof is sound under the low-order assumption in class groups:
/// it is computationally infeasible to find a non-trivial element of
/// low order in a class group with unknown group order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VdfProof {
    /// The serialized proof element π = x^(floor(2^t / l))
    pub pi: Vec<u8>,
}

impl VdfProof {
    /// Create a new proof from a class group element.
    #[must_use]
    pub fn new(element: &ClassGroupElement) -> Self {
        Self {
            pi: element.to_bytes(),
        }
    }

    /// Create an empty/placeholder proof.
    ///
    /// This is used for testing or when a proof is not yet computed.
    /// An empty proof will always fail verification.
    #[must_use]
    pub fn empty() -> Self {
        Self { pi: Vec::new() }
    }

    /// Check if this is an empty proof.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pi.is_empty()
    }

    /// Get the size of the proof in bytes.
    #[must_use]
    pub fn size(&self) -> usize {
        self.pi.len()
    }

    /// Serialize to bytes (length-prefixed format).
    ///
    /// Format: [length (4 bytes LE)][proof bytes]
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(4 + self.pi.len());
        bytes.extend_from_slice(&(self.pi.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&self.pi);
        bytes
    }

    /// Deserialize from bytes.
    ///
    /// Returns `None` if the bytes are malformed.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 4 {
            return None;
        }

        let len = u32::from_le_bytes(bytes[0..4].try_into().ok()?) as usize;
        if bytes.len() < 4 + len {
            return None;
        }

        Some(Self {
            pi: bytes[4..4 + len].to_vec(),
        })
    }

    /// Convert to hex string.
    ///
    /// Useful for debugging and logging.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(&self.pi)
    }

    /// Create from hex string.
    pub fn from_hex(s: &str) -> Result<Self, hex::FromHexError> {
        Ok(Self {
            pi: hex::decode(s)?,
        })
    }

    /// Get a truncated hex representation for display.
    ///
    /// Shows the first and last 8 characters with "..." in between.
    #[must_use]
    pub fn display_hex(&self) -> String {
        let hex = self.to_hex();
        if hex.len() <= 20 {
            hex
        } else {
            format!("{}...{}", &hex[..8], &hex[hex.len() - 8..])
        }
    }
}

impl Default for VdfProof {
    fn default() -> Self {
        Self::empty()
    }
}

impl std::fmt::Display for VdfProof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VdfProof({})", self.display_hex())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_proof_new() {
        let proof = VdfProof {
            pi: vec![1, 2, 3, 4, 5],
        };
        assert_eq!(proof.size(), 5);
        assert!(!proof.is_empty());
    }

    #[test]
    fn test_proof_empty() {
        let proof = VdfProof::empty();
        assert!(proof.is_empty());
        assert_eq!(proof.size(), 0);
    }

    #[test]
    fn test_proof_default() {
        let proof = VdfProof::default();
        assert!(proof.is_empty());
    }

    #[test]
    fn test_proof_serialization() {
        let proof = VdfProof {
            pi: vec![1, 2, 3, 4, 5],
        };

        let bytes = proof.to_bytes();
        assert_eq!(bytes.len(), 9); // 4 bytes length + 5 bytes data

        let recovered = VdfProof::from_bytes(&bytes).expect("should deserialize");
        assert_eq!(proof, recovered);
    }

    #[test]
    fn test_proof_serialization_empty() {
        let proof = VdfProof::empty();
        let bytes = proof.to_bytes();
        let recovered = VdfProof::from_bytes(&bytes).expect("should deserialize");
        assert_eq!(proof, recovered);
    }

    #[test]
    fn test_proof_hex() {
        let proof = VdfProof {
            pi: vec![0xde, 0xad, 0xbe, 0xef],
        };

        let hex = proof.to_hex();
        assert_eq!(hex, "deadbeef");

        let recovered = VdfProof::from_hex(&hex).expect("should parse hex");
        assert_eq!(proof, recovered);
    }

    #[test]
    fn test_proof_display() {
        // Short proof
        let short = VdfProof {
            pi: vec![0xde, 0xad, 0xbe, 0xef],
        };
        assert_eq!(format!("{short}"), "VdfProof(deadbeef)");

        // Long proof
        let long = VdfProof {
            pi: vec![
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x33, 0x44,
            ],
        };
        let display = format!("{long}");
        assert!(display.contains("..."));
    }

    #[test]
    fn test_proof_from_bytes_malformed() {
        // Too short
        assert!(VdfProof::from_bytes(&[]).is_none());
        assert!(VdfProof::from_bytes(&[1, 2, 3]).is_none());

        // Length mismatch
        let bad = [10, 0, 0, 0, 1, 2, 3]; // Claims 10 bytes but only has 3
        assert!(VdfProof::from_bytes(&bad).is_none());
    }

    #[test]
    fn test_proof_hex_invalid() {
        let result = VdfProof::from_hex("not valid hex!");
        assert!(result.is_err());
    }

    #[test]
    fn test_proof_serde_json() {
        let proof = VdfProof {
            pi: vec![1, 2, 3, 4, 5],
        };

        let json = serde_json::to_string(&proof).expect("serialize");
        let recovered: VdfProof = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(proof, recovered);
    }

    // Property-based tests
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn prop_serialization_roundtrip(data: Vec<u8>) {
            let proof = VdfProof { pi: data };
            let bytes = proof.to_bytes();
            let recovered = VdfProof::from_bytes(&bytes);
            prop_assert!(recovered.is_some());
            prop_assert_eq!(proof, recovered.unwrap());
        }

        #[test]
        fn prop_hex_roundtrip(data: Vec<u8>) {
            let proof = VdfProof { pi: data };
            let hex = proof.to_hex();
            let recovered = VdfProof::from_hex(&hex);
            prop_assert!(recovered.is_ok());
            prop_assert_eq!(proof, recovered.unwrap());
        }
    }
}
