//! BLS12-381 aggregate signatures for attestation.
//!
//! Provides key generation, signing, verification, and signature aggregation
//! using the BLS12-381 curve (same as Ethereum 2.0). Used for attestation
//! proofs that are committed on-chain as aggregate signatures.
//!
//! ## Properties
//!
//! - **Aggregation**: N individual signatures compress to 1 signature (96 bytes)
//! - **Unfakeable**: Aggregate verification fails if any constituent is invalid
//! - **Compact**: 48-byte public keys, 96-byte signatures regardless of signer count
//!
//! ## Security
//!
//! - Uses `blst` (Supranational), the same library used by Ethereum consensus clients
//! - Domain separation via DST (Domain Separation Tag) prevents cross-protocol attacks
//! - Proof-of-possession (`PoP`) prevents rogue public key attacks on aggregation
//! - Private keys are zeroized on drop
//!
//! ## Known Limitation
//!
//! Intermediate `blst::SecretKey` values (C struct) are not zeroized by `blst`.
//! Our `BlsSecretKey` wrapper zeroizes the Rust-side bytes on drop, but the
//! temporary `blst::SecretKey` used during `sign()` / `public_key()` may linger
//! until the allocator reuses the memory. This matches Ethereum client behavior.

mod operations;
mod types;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;

use thiserror::Error;

/// BLS12-381 public key size in bytes (compressed G1 point).
pub const BLS_PUBLIC_KEY_SIZE: usize = 48;

/// BLS12-381 signature size in bytes (compressed G2 point).
pub const BLS_SIGNATURE_SIZE: usize = 96;

/// Domain Separation Tag for DOLI attestation signing.
///
/// Follows the hash-to-curve convention (RFC 9380) used by Ethereum's BLS spec.
/// Format: `BLS_SIG_<curve>_<hash>_<map>_<scheme>_<app>`
pub const ATTESTATION_DST: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_DOLI_ATTEST_V1";

/// Domain Separation Tag for Proof-of-Possession.
///
/// Used at registration to prove ownership of a BLS secret key.
/// Different DST prevents cross-domain signature reuse.
pub(crate) const POP_DST: &[u8] = b"BLS_POP_BLS12381G2_XMD:SHA-256_SSWU_RO_DOLI_POP_V1";

/// Errors from BLS operations.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BlsError {
    /// The secret key bytes are invalid (not a valid BLS scalar).
    #[error("invalid BLS secret key")]
    InvalidSecretKey,

    /// The public key bytes are invalid or not on the curve.
    #[error("invalid BLS public key")]
    InvalidPublicKey,

    /// The signature bytes are invalid or not on the G2 curve.
    #[error("invalid BLS signature")]
    InvalidSignature,

    /// Signature verification failed.
    #[error("BLS signature verification failed")]
    VerificationFailed,

    /// Proof-of-possession verification failed.
    #[error("BLS proof-of-possession verification failed")]
    InvalidProofOfPossession,

    /// No signatures provided for aggregation.
    #[error("empty signature set for aggregation")]
    EmptyAggregation,

    /// Invalid hex encoding.
    #[error("invalid hex: {0}")]
    InvalidHex(String),
}

impl From<hex::FromHexError> for BlsError {
    fn from(e: hex::FromHexError) -> Self {
        BlsError::InvalidHex(e.to_string())
    }
}

// Re-export all public types and functions so external paths remain unchanged.
pub use operations::{
    attestation_message, bls_aggregate, bls_sign, bls_sign_pop, bls_verify, bls_verify_aggregate,
    bls_verify_pop,
};
pub use types::{BlsKeyPair, BlsPublicKeyWrapped, BlsSecretKey, BlsSignature};
