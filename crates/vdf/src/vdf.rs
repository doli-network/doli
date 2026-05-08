//! VDF data types
//!
//! Contains `VdfOutput` and `VdfError` used by block headers and the hash-chain VDF.
//! The Wesolowski class group implementation has been removed — all consensus-critical
//! VDF operations use the BLAKE3 hash-chain in `doli_core::tpop::heartbeat`.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors in VDF operations
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum VdfError {
    /// The input hash is invalid
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// The proof format is invalid
    #[error("invalid proof: {0}")]
    InvalidProof(String),

    /// Verification failed (proof doesn't match output)
    #[error("verification failed: computed output doesn't match claimed output")]
    VerificationFailed,

    /// Internal computation error
    #[error("computation error: {0}")]
    ComputationError(String),

    /// The time parameter is invalid
    #[error("invalid time parameter: {0}")]
    InvalidTimeParameter(String),
}

/// VDF output (the result of the computation).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VdfOutput {
    /// The serialized output bytes
    pub value: Vec<u8>,
}

impl VdfOutput {
    /// Get the size of the output in bytes.
    #[must_use]
    pub fn size(&self) -> usize {
        self.value.len()
    }

    /// Serialize to bytes (length-prefixed).
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(4 + self.value.len());
        #[allow(clippy::cast_possible_truncation)]
        bytes.extend_from_slice(&(self.value.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&self.value);
        bytes
    }

    /// Deserialize from bytes.
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
            value: bytes[4..4 + len].to_vec(),
        })
    }

    /// Convert to hex string.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(&self.value)
    }

    /// Create from hex string.
    pub fn from_hex(s: &str) -> Result<Self, hex::FromHexError> {
        Ok(Self {
            value: hex::decode(s)?,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_vdf_output_serialization() {
        let output = VdfOutput {
            value: vec![1, 2, 3, 4],
        };

        let bytes = output.to_bytes();
        let recovered = VdfOutput::from_bytes(&bytes).expect("should deserialize");

        assert_eq!(output, recovered);
    }

    #[test]
    fn test_vdf_output_hex() {
        let output = VdfOutput {
            value: vec![0xde, 0xad, 0xbe, 0xef],
        };

        let hex = output.to_hex();
        assert_eq!(hex, "deadbeef");

        let recovered = VdfOutput::from_hex(&hex).expect("should parse hex");
        assert_eq!(output, recovered);
    }
}
