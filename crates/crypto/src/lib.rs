//! # doli-crypto
//!
//! Production-grade cryptographic primitives for the DOLI protocol.
//!
//! ## Security Guarantees
//!
//! This crate implements the following security properties:
//!
//! - **Constant-time operations**: All secret-dependent comparisons use constant-time
//!   algorithms to prevent timing side-channel attacks.
//! - **Zeroization**: All secret key material is automatically zeroized when dropped,
//!   preventing secrets from lingering in memory.
//! - **Type safety**: The type system enforces correct usage of cryptographic primitives,
//!   making misuse difficult at compile time.
//!
//! ## Cryptographic Algorithms
//!
//! | Primitive | Algorithm | Security Level |
//! |-----------|-----------|----------------|
//! | Hash | BLAKE3-256 | 128-bit |
//! | Signature | Ed25519 | ~128-bit |
//! | Key Derivation | BLAKE3 (keyed) | 128-bit |
//!
//! ## Example Usage
//!
//! ```rust
//! use crypto::{KeyPair, Hash, signature};
//!
//! // Generate a new keypair
//! let keypair = KeyPair::generate();
//!
//! // Hash a message
//! let message = b"Hello, DOLI!";
//! let hash = crypto::hash::hash(message);
//!
//! // Sign and verify
//! let sig = signature::sign(message, keypair.private_key());
//! assert!(signature::verify(message, &sig, keypair.public_key()).is_ok());
//! ```
//!
//! ## Security Considerations
//!
//! - Never log or display private keys
//! - Use secure random number generation (this crate uses `rand::rngs::OsRng`)
//! - Verify all signatures before trusting signed data
//! - Hash all messages before signing to ensure domain separation

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::unwrap_used)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod hash;
pub mod keys;
pub mod merkle;
pub mod signature;

pub use hash::{Hash, Hasher};
pub use keys::{Address, KeyPair, PrivateKey, PublicKey};
pub use signature::Signature;

/// Size of a BLAKE3-256 hash output in bytes.
pub const HASH_SIZE: usize = 32;

/// Size of an Ed25519 public key in bytes.
pub const PUBLIC_KEY_SIZE: usize = 32;

/// Size of an Ed25519 private key seed in bytes.
pub const PRIVATE_KEY_SIZE: usize = 32;

/// Size of an Ed25519 signature in bytes.
pub const SIGNATURE_SIZE: usize = 64;

/// Size of a DOLI address in bytes (truncated hash of public key).
pub const ADDRESS_SIZE: usize = 20;

/// Domain separation tag for message signing.
pub const SIGN_DOMAIN: &[u8] = b"DOLI_SIGN_V1";

/// Domain separation tag for address derivation.
pub const ADDRESS_DOMAIN: &[u8] = b"DOLI_ADDR_V1";

/// Domain separation tag for transaction hashing.
pub const TX_DOMAIN: &[u8] = b"DOLI_TX_V1";

/// Domain separation tag for block hashing.
pub const BLOCK_DOMAIN: &[u8] = b"DOLI_BLOCK_V1";

/// Domain separation tag for VDF input.
pub const VDF_DOMAIN: &[u8] = b"DOLI_VDF_V1";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants_are_correct() {
        assert_eq!(HASH_SIZE, 32);
        assert_eq!(PUBLIC_KEY_SIZE, 32);
        assert_eq!(PRIVATE_KEY_SIZE, 32);
        assert_eq!(SIGNATURE_SIZE, 64);
        assert_eq!(ADDRESS_SIZE, 20);
    }

    #[test]
    fn test_domain_tags_are_unique() {
        let domains = [
            SIGN_DOMAIN,
            ADDRESS_DOMAIN,
            TX_DOMAIN,
            BLOCK_DOMAIN,
            VDF_DOMAIN,
        ];
        for (i, a) in domains.iter().enumerate() {
            for (j, b) in domains.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "Domain tags must be unique");
                }
            }
        }
    }
}
