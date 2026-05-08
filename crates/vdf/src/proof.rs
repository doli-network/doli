//! VDF proof structure
//!
//! A lightweight proof container used in block headers. In production, the hash-chain
//! VDF does not produce a compact proof (verification requires recomputation), so the
//! proof field is empty. This struct exists for wire-format compatibility.

use serde::{Deserialize, Serialize};

/// A VDF proof.
///
/// In production (hash-chain VDF), this is always empty — verification
/// requires recomputation. The struct is retained for block header format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VdfProof {
    /// The serialized proof bytes (empty for hash-chain VDF)
    pub pi: Vec<u8>,
}

impl VdfProof {
    /// Create an empty/placeholder proof.
    ///
    /// This is the standard constructor for hash-chain VDF blocks.
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
        #[allow(clippy::cast_possible_truncation)]
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
        let short = VdfProof {
            pi: vec![0xde, 0xad, 0xbe, 0xef],
        };
        assert_eq!(format!("{short}"), "VdfProof(deadbeef)");

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
        assert!(VdfProof::from_bytes(&[]).is_none());
        assert!(VdfProof::from_bytes(&[1, 2, 3]).is_none());

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
}
