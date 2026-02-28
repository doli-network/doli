//! Human-readable address encoding using Bech32m (BIP-350).
//!
//! Provides `doli1...` / `tdoli1...` / `ddoli1...` addresses that encode
//! the 32-byte `pubkey_hash` used internally by the UTXO set.
//!
//! No consensus, storage, or wire-protocol changes. Purely presentation layer.

use bech32::{Bech32m, Hrp};
use thiserror::Error;

use crate::hash::hash_with_domain;
use crate::{Hash, ADDRESS_DOMAIN, HASH_SIZE};

/// Errors from address encoding/decoding.
#[derive(Debug, Error)]
pub enum AddressError {
    /// Bech32 encoding/decoding failed.
    #[error("bech32: {0}")]
    Bech32(String),

    /// Decoded data has wrong length.
    #[error("invalid data length: expected {expected}, got {got}")]
    InvalidLength {
        /// Expected byte count.
        expected: usize,
        /// Actual byte count.
        got: usize,
    },

    /// The HRP does not match any known network.
    #[error("unknown network prefix: {0}")]
    UnknownPrefix(String),

    /// Address is for a different network than expected.
    #[error("network mismatch: address is for '{address_network}', expected '{expected_network}'")]
    NetworkMismatch {
        /// Network the address belongs to.
        address_network: String,
        /// Network the caller expected.
        expected_network: String,
    },

    /// Generic parse failure.
    #[error("{0}")]
    InvalidFormat(String),
}

// ---------------------------------------------------------------------------
// HRP constants (must match Network::address_prefix)
// ---------------------------------------------------------------------------

const HRP_MAINNET: &str = "doli";
const HRP_TESTNET: &str = "tdoli";
const HRP_DEVNET: &str = "ddoli";

/// All known HRPs, ordered by decreasing length so the longest prefix matches first.
const ALL_HRPS: [&str; 3] = [HRP_TESTNET, HRP_DEVNET, HRP_MAINNET];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Encode a 32-byte `pubkey_hash` into a bech32m address for the given network prefix.
///
/// `network_prefix` should be `"doli"`, `"tdoli"`, or `"ddoli"`.
///
/// # Errors
///
/// Returns [`AddressError::Bech32`] if the prefix is invalid or encoding fails.
pub fn encode(pubkey_hash: &Hash, network_prefix: &str) -> Result<String, AddressError> {
    let hrp = Hrp::parse(network_prefix).map_err(|e| AddressError::Bech32(e.to_string()))?;
    bech32::encode::<Bech32m>(hrp, pubkey_hash.as_bytes())
        .map_err(|e| AddressError::Bech32(e.to_string()))
}

/// Decode a bech32m address into (pubkey hash, network prefix).
///
/// # Errors
///
/// Returns an error if decoding fails, the HRP is unknown, or data length is wrong.
pub fn decode(s: &str) -> Result<(Hash, String), AddressError> {
    let (hrp, data) = bech32::decode(s).map_err(|e| AddressError::Bech32(e.to_string()))?;

    let hrp_str = hrp.to_string().to_lowercase();
    // Validate HRP is a known network prefix
    if !ALL_HRPS.contains(&hrp_str.as_str()) {
        return Err(AddressError::UnknownPrefix(hrp_str));
    }

    if data.len() != HASH_SIZE {
        return Err(AddressError::InvalidLength {
            expected: HASH_SIZE,
            got: data.len(),
        });
    }

    let hash = Hash::try_from_slice(&data).ok_or(AddressError::InvalidLength {
        expected: HASH_SIZE,
        got: data.len(),
    })?;

    Ok((hash, hrp_str))
}

/// Derive a bech32m address directly from a raw public key.
///
/// Computes `pubkey_hash = BLAKE3(ADDRESS_DOMAIN || pubkey_bytes)` then encodes.
///
/// # Errors
///
/// Returns [`AddressError::Bech32`] if encoding fails.
pub fn from_pubkey(pubkey_bytes: &[u8], network_prefix: &str) -> Result<String, AddressError> {
    let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, pubkey_bytes);
    encode(&pubkey_hash, network_prefix)
}

/// Universal address resolver: accept **any** user-supplied string and return
/// the 32-byte `pubkey_hash`.
///
/// Accepted formats (tried in order):
/// 1. `doli1...` / `tdoli1...` / `ddoli1...` — bech32m address.
/// 2. 64-char hex — raw 32-byte `pubkey_hash`.
/// 3. Anything else — error with helpful guidance.
///
/// When `expected_prefix` is `Some`, bech32m addresses must match that prefix.
///
/// # Errors
///
/// Returns an error if the input cannot be parsed as any recognized format,
/// or if the network prefix does not match `expected_prefix`.
pub fn resolve(input: &str, expected_prefix: Option<&str>) -> Result<Hash, AddressError> {
    let trimmed = input.trim();

    // 1. Try bech32m if it looks like one (contains '1' separator)
    if looks_like_bech32(trimmed) {
        let (hash, hrp) = decode(trimmed)?;
        if let Some(expected) = expected_prefix {
            if hrp != expected {
                return Err(AddressError::NetworkMismatch {
                    address_network: hrp,
                    expected_network: expected.to_string(),
                });
            }
        }
        return Ok(hash);
    }

    // 2. 64-char hex (32-byte pubkey_hash)
    if trimmed.len() == 64 {
        if let Some(hash) = Hash::from_hex(trimmed) {
            return Ok(hash);
        }
    }

    // 3. Nothing matched
    Err(AddressError::InvalidFormat(format!(
        "unrecognized address format: expected 'doli1...' or 64-char hex pubkey_hash, got '{}'",
        if trimmed.len() > 20 {
            format!("{}...", &trimmed[..20])
        } else {
            trimmed.to_string()
        }
    )))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Quick heuristic: does this string look like a bech32 address?
fn looks_like_bech32(s: &str) -> bool {
    // Bech32 addresses have an HRP, then '1', then data.
    // Our HRPs are "doli", "tdoli", "ddoli", so minimum prefix length is 4.
    for hrp in &ALL_HRPS {
        let prefix = format!("{hrp}1");
        if s.to_lowercase().starts_with(&prefix) {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::hash::hash_with_domain;

    #[test]
    fn test_encode_decode_roundtrip() {
        let data = [0x42u8; 32];
        let hash = Hash::from_bytes(data);

        for prefix in &[HRP_MAINNET, HRP_TESTNET, HRP_DEVNET] {
            let encoded = encode(&hash, prefix).unwrap();
            assert!(encoded.starts_with(&format!("{prefix}1")));

            let (decoded_hash, decoded_prefix) = decode(&encoded).unwrap();
            assert_eq!(decoded_hash, hash);
            assert_eq!(decoded_prefix, *prefix);
        }
    }

    #[test]
    fn test_from_pubkey() {
        let pubkey = [0xAB; 32];
        let addr = from_pubkey(&pubkey, HRP_MAINNET).unwrap();
        assert!(addr.starts_with("doli1"));

        // Verify it decodes to the correct pubkey_hash
        let (decoded, _) = decode(&addr).unwrap();
        let expected = hash_with_domain(ADDRESS_DOMAIN, &pubkey);
        assert_eq!(decoded, expected);
    }

    #[test]
    fn test_resolve_bech32() {
        let hash = Hash::from_bytes([0x01; 32]);
        let addr = encode(&hash, HRP_MAINNET).unwrap();

        let resolved = resolve(&addr, Some(HRP_MAINNET)).unwrap();
        assert_eq!(resolved, hash);
    }

    #[test]
    fn test_resolve_hex() {
        let hash = Hash::from_bytes([0xDE; 32]);
        let hex_str = hash.to_hex();

        let resolved = resolve(&hex_str, None).unwrap();
        assert_eq!(resolved, hash);
    }

    #[test]
    fn test_resolve_wrong_network() {
        let hash = Hash::from_bytes([0x01; 32]);
        let addr = encode(&hash, HRP_TESTNET).unwrap();

        let err = resolve(&addr, Some(HRP_MAINNET)).unwrap_err();
        assert!(err.to_string().contains("network mismatch"));
    }

    #[test]
    fn test_resolve_invalid() {
        let err = resolve("not_an_address", None).unwrap_err();
        assert!(err.to_string().contains("unrecognized address format"));
    }

    #[test]
    fn test_decode_unknown_prefix() {
        // Encode with a valid HRP that isn't ours
        let hrp = Hrp::parse("xyz").unwrap();
        let data = [0x01u8; 32];
        let encoded = bech32::encode::<Bech32m>(hrp, &data).unwrap();

        let err = decode(&encoded).unwrap_err();
        assert!(err.to_string().contains("unknown network prefix"));
    }

    #[test]
    fn test_checksum_corruption() {
        let hash = Hash::from_bytes([0x01; 32]);
        let mut addr = encode(&hash, HRP_MAINNET).unwrap();

        // Corrupt the last character
        let last = addr.pop().unwrap();
        let replacement = if last == 'q' { 'p' } else { 'q' };
        addr.push(replacement);

        assert!(decode(&addr).is_err());
    }

    #[test]
    fn test_case_insensitive() {
        let hash = Hash::from_bytes([0x01; 32]);
        let addr = encode(&hash, HRP_MAINNET).unwrap();

        // bech32 is case-insensitive (but must not mix cases)
        let upper = addr.to_uppercase();
        let (decoded, _) = decode(&upper).unwrap();
        assert_eq!(decoded, hash);
    }
}
