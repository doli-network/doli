//! Ed25519 adaptor signatures for cross-chain atomic swaps.
//!
//! Adaptor signatures enable trustless atomic swaps with chains that lack HTLC support
//! (like Monero). The protocol works by creating a "pre-signature" that becomes a valid
//! Ed25519 signature only when a secret scalar is applied.
//!
//! # Protocol Overview
//!
//! 1. Alice generates an ephemeral keypair `(t, T)` where `T = t * G`
//! 2. Alice creates a pre-signature (adaptor signature) using `T` as the adaptor point
//! 3. Bob verifies the adaptor signature is valid for `(message, pubkey, T)`
//! 4. Alice reveals `t` by completing the signature: `s' = s_hat + t`
//! 5. Bob extracts `t` from the completed signature: `t = s' - s_hat`
//!
//! # Security Properties
//!
//! - The adaptor signature does not reveal `t` (the secret scalar)
//! - Only the holder of `t` can complete the pre-signature into a valid signature
//! - Anyone can extract `t` given both the pre-signature and the completed signature
//! - The identity point is rejected as an adaptor point (would make extraction trivial)

use curve25519_dalek::constants::ED25519_BASEPOINT_TABLE;
use curve25519_dalek::edwards::{CompressedEdwardsY, EdwardsPoint};
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::traits::IsIdentity;
use rand::rngs::OsRng;
use sha2::{Digest, Sha512};
use thiserror::Error;
use zeroize::ZeroizeOnDrop;

use crate::hash::Hash;

/// Domain separator for adaptor signature nonce generation.
const ADAPTOR_NONCE_DOMAIN: &[u8] = b"DOLI_ADAPTOR_NONCE_V1";

/// Errors related to adaptor signature operations.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AdaptorError {
    /// The adaptor point is the identity element (would be insecure).
    #[error("adaptor point is the identity element")]
    IdentityPoint,

    /// The adaptor point bytes do not represent a valid curve point.
    #[error("invalid adaptor point")]
    InvalidPoint,

    /// The scalar bytes are not a valid scalar.
    #[error("invalid scalar")]
    InvalidScalar,
}

/// An adaptor (pre-)signature.
///
/// Contains `(R', s_hat)` where:
/// - `R' = R + T` (nonce commitment shifted by adaptor point)
/// - `s_hat = r + H(R', A, m) * a` (partial scalar, not yet complete)
///
/// This is NOT a valid Ed25519 signature. It becomes one when completed with `t`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdaptorSignature {
    /// The shifted nonce commitment R' = R + T (compressed, 32 bytes)
    r_prime: CompressedEdwardsY,
    /// The partial scalar `s_hat` (32 bytes)
    s_hat: Scalar,
}

impl AdaptorSignature {
    /// Serialize to 64 bytes: `[R' || s_hat]`
    #[must_use]
    pub fn to_bytes(&self) -> [u8; 64] {
        let mut bytes = [0u8; 64];
        bytes[..32].copy_from_slice(self.r_prime.as_bytes());
        bytes[32..].copy_from_slice(self.s_hat.as_bytes());
        bytes
    }

    /// Deserialize from 64 bytes.
    ///
    /// # Errors
    /// Returns error if the point is not on the curve.
    pub fn from_bytes(bytes: &[u8; 64]) -> Result<Self, AdaptorError> {
        let mut r_bytes = [0u8; 32];
        r_bytes.copy_from_slice(&bytes[..32]);
        let r_prime = CompressedEdwardsY(r_bytes);

        // Validate it decompresses to a valid point
        r_prime.decompress().ok_or(AdaptorError::InvalidPoint)?;

        let mut s_bytes = [0u8; 32];
        s_bytes.copy_from_slice(&bytes[32..]);
        let s_hat: Option<Scalar> = Scalar::from_canonical_bytes(s_bytes).into();
        let s_hat = s_hat.ok_or(AdaptorError::InvalidScalar)?;

        Ok(Self { r_prime, s_hat })
    }
}

/// Secret adaptor scalar (zeroized on drop).
#[derive(Clone, ZeroizeOnDrop)]
pub struct AdaptorSecret {
    /// The secret scalar `t` such that `T = t * G`.
    scalar_bytes: [u8; 32],
}

impl AdaptorSecret {
    /// Generate a random adaptor secret.
    #[must_use]
    pub fn generate() -> Self {
        let t = Scalar::random(&mut OsRng);
        Self {
            scalar_bytes: t.to_bytes(),
        }
    }

    /// Create from raw bytes.
    ///
    /// # Errors
    /// Returns error if bytes are not a valid canonical scalar.
    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self, AdaptorError> {
        // Verify it's a valid canonical scalar
        let check: Option<Scalar> = Scalar::from_canonical_bytes(bytes).into();
        check.ok_or(AdaptorError::InvalidScalar)?;
        Ok(Self {
            scalar_bytes: bytes,
        })
    }

    /// Get the scalar value.
    fn scalar(&self) -> Scalar {
        // Safe: we validated in from_bytes/generate
        let opt: Option<Scalar> = Scalar::from_canonical_bytes(self.scalar_bytes).into();
        opt.expect("validated at construction")
    }

    /// Get the raw bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.scalar_bytes
    }

    /// Compute the public adaptor point `T = t * G`.
    #[must_use]
    pub fn public_point(&self) -> EdwardsPoint {
        &self.scalar() * ED25519_BASEPOINT_TABLE
    }

    /// Compute the compressed public adaptor point.
    #[must_use]
    pub fn public_point_compressed(&self) -> CompressedEdwardsY {
        self.public_point().compress()
    }
}

impl PartialEq for AdaptorSecret {
    fn eq(&self, other: &Self) -> bool {
        self.scalar_bytes == other.scalar_bytes
    }
}

impl Eq for AdaptorSecret {}

impl std::fmt::Debug for AdaptorSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AdaptorSecret([redacted])")
    }
}

/// Create an adaptor (pre-)signature.
///
/// Signs `message` with `private_key`, shifted by adaptor point `adaptor_point`.
/// The result is NOT a valid Ed25519 signature — it must be completed with the
/// adaptor secret scalar `t` to become valid.
///
/// # Errors
/// Returns error if `adaptor_point` is the identity element.
pub fn adaptor_sign(
    message: &[u8],
    private_key: &crate::PrivateKey,
    adaptor_point: &EdwardsPoint,
) -> Result<AdaptorSignature, AdaptorError> {
    // Reject identity point — would make the protocol trivially breakable
    if adaptor_point.is_identity() {
        return Err(AdaptorError::IdentityPoint);
    }

    // Derive the private scalar and nonce prefix from the Ed25519 seed using SHA-512.
    // This is the standard Ed25519 key expansion: SHA-512(seed) -> (scalar, prefix).
    let mut h = Sha512::new();
    h.update(private_key.as_bytes());
    let expanded = h.finalize();

    // Lower 32 bytes -> scalar (with clamping per Ed25519 spec)
    let mut scalar_bytes = [0u8; 32];
    scalar_bytes.copy_from_slice(&expanded[..32]);
    scalar_bytes[0] &= 0b1111_1000;
    scalar_bytes[31] &= 0b0111_1111;
    scalar_bytes[31] |= 0b0100_0000;
    let a = Scalar::from_bytes_mod_order(scalar_bytes);

    // Upper 32 bytes -> nonce prefix (for deterministic nonce generation)
    let nonce_prefix = &expanded[32..64];

    // Public key as point
    let a_point = &a * ED25519_BASEPOINT_TABLE;
    let a_compressed = a_point.compress();

    // Generate deterministic nonce: r = H(DOMAIN || nonce_prefix || message)
    let mut hasher = crate::Hasher::new_with_domain(ADAPTOR_NONCE_DOMAIN);
    hasher.update(nonce_prefix);
    hasher.update(message);
    let nonce_hash = hasher.finalize();
    let r = Scalar::from_bytes_mod_order(*nonce_hash.as_bytes());

    // R = r * G
    let r_point = &r * ED25519_BASEPOINT_TABLE;

    // R' = R + T (shifted nonce commitment)
    let r_prime = r_point + adaptor_point;
    let r_prime_compressed = r_prime.compress();

    // Challenge: e = H(R' || A || message) using SHA-512 (Ed25519 standard)
    let e = compute_challenge(&r_prime_compressed, &a_compressed, message);

    // s_hat = r + e * a (partial scalar — does NOT include t)
    let s_hat = r + e * a;

    Ok(AdaptorSignature {
        r_prime: r_prime_compressed,
        s_hat,
    })
}

/// Verify an adaptor (pre-)signature.
///
/// Checks that the pre-signature is consistent: given `message`, `public_key`, and
/// `adaptor_point`, verifies that `s_hat * G == R' - T + e * A` where `e = H(R', A, m)`.
///
/// This does NOT verify a valid Ed25519 signature — it verifies the pre-signature
/// is correctly formed so that completing with the correct `t` will yield a valid signature.
#[must_use]
pub fn adaptor_verify(
    message: &[u8],
    public_key: &crate::PublicKey,
    adaptor_sig: &AdaptorSignature,
    adaptor_point: &EdwardsPoint,
) -> bool {
    // Decompress R'
    let Some(r_prime) = adaptor_sig.r_prime.decompress() else {
        return false;
    };

    let a_compressed = CompressedEdwardsY(*public_key.as_bytes());
    let Some(a_point) = a_compressed.decompress() else {
        return false;
    };

    // e = H(R' || A || message)
    let e = compute_challenge(&adaptor_sig.r_prime, &a_compressed, message);

    // Verify: s_hat * G == R' - T + e * A
    // Rearranged: s_hat * G == (R' - T) + e * A
    let lhs = &adaptor_sig.s_hat * ED25519_BASEPOINT_TABLE;
    let rhs = (r_prime - adaptor_point) + e * a_point;

    lhs == rhs
}

/// Complete an adaptor signature into a valid Ed25519 signature.
///
/// Given the adaptor pre-signature and the secret scalar `t`, produces
/// a standard Ed25519 signature `(R', s' = s_hat + t)`.
#[must_use]
pub fn adaptor_complete(
    adaptor_sig: &AdaptorSignature,
    adaptor_secret: &AdaptorSecret,
) -> crate::Signature {
    let t = adaptor_secret.scalar();
    let s_prime = adaptor_sig.s_hat + t;

    let mut sig_bytes = [0u8; 64];
    sig_bytes[..32].copy_from_slice(adaptor_sig.r_prime.as_bytes());
    sig_bytes[32..].copy_from_slice(s_prime.as_bytes());

    crate::Signature::from_bytes(sig_bytes)
}

/// Extract the adaptor secret from a completed signature and the original pre-signature.
///
/// Given the completed Ed25519 signature `(R', s')` and the adaptor pre-signature
/// `(R', s_hat)`, computes `t = s' - s_hat`.
///
/// # Errors
/// Returns error if the s-component bytes are not valid scalars.
pub fn adaptor_extract(
    completed_sig: &crate::Signature,
    adaptor_sig: &AdaptorSignature,
) -> Result<AdaptorSecret, AdaptorError> {
    let s_prime_bytes = completed_sig.s_bytes();
    let s_prime: Option<Scalar> = Scalar::from_canonical_bytes(s_prime_bytes).into();
    let s_prime = s_prime.ok_or(AdaptorError::InvalidScalar)?;

    let t: Scalar = s_prime - adaptor_sig.s_hat;

    AdaptorSecret::from_bytes(t.to_bytes())
}

/// Convert an Ed25519 point to a `Hash` for use as `counter_hash` in `BridgeHTLC`.
///
/// For Monero swaps, the `counter_hash` field stores the compressed adaptor point `T`.
#[must_use]
pub fn adaptor_point_to_hash(point: &EdwardsPoint) -> Hash {
    Hash::from_bytes(*point.compress().as_bytes())
}

/// Convert a `Hash` (from `counter_hash`) back to an Ed25519 point.
///
/// # Errors
/// Returns error if the bytes don't represent a valid curve point.
pub fn hash_to_adaptor_point(hash: &Hash) -> Result<EdwardsPoint, AdaptorError> {
    let compressed = CompressedEdwardsY(*hash.as_bytes());
    compressed.decompress().ok_or(AdaptorError::InvalidPoint)
}

/// Compute the Ed25519 challenge scalar: `e = SHA-512(R || A || message)` reduced mod l.
///
/// This matches the standard Ed25519 challenge computation to ensure the completed
/// adaptor signature is a valid Ed25519 signature.
fn compute_challenge(
    r_compressed: &CompressedEdwardsY,
    a_compressed: &CompressedEdwardsY,
    message: &[u8],
) -> Scalar {
    let mut h = Sha512::new();
    h.update(r_compressed.as_bytes());
    h.update(a_compressed.as_bytes());
    h.update(message);
    let hash_output = h.finalize();
    let mut wide_bytes = [0u8; 64];
    wide_bytes.copy_from_slice(&hash_output);
    Scalar::from_bytes_mod_order_wide(&wide_bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::KeyPair;

    // ========== GRUPO 1 — Protocol mathematics ==========

    /// Test 1: Full protocol roundtrip — the most important test
    #[test]
    fn test_adaptor_full_protocol() {
        // Alice generates ephemeral keypair (t, T)
        let secret = AdaptorSecret::generate();
        let adaptor_point = secret.public_point();

        // Alice creates adaptor signature
        let msg = b"transfer 1.0 XMR to Alice";
        let keypair = KeyPair::generate();
        let adaptor_sig = adaptor_sign(msg, keypair.private_key(), &adaptor_point).unwrap();

        // Bob verifies the adaptor sig is valid for (msg, pubkey, T)
        assert!(adaptor_verify(
            msg,
            keypair.public_key(),
            &adaptor_sig,
            &adaptor_point
        ));

        // Alice reveals t — completes the signature
        let real_sig = adaptor_complete(&adaptor_sig, &secret);

        // The completed signature verifies as a standard Ed25519 signature
        assert!(crate::signature::verify(msg, &real_sig, keypair.public_key()).is_ok());

        // Bob extracts t from the completed sig + adaptor sig
        let extracted = adaptor_extract(&real_sig, &adaptor_sig).unwrap();

        // Extracted t == original t
        assert_eq!(secret, extracted);
    }

    /// Test 2: `adaptor_verify` fails with wrong T
    #[test]
    fn test_adaptor_verify_wrong_t() {
        let secret = AdaptorSecret::generate();
        let adaptor_point = secret.public_point();
        let wrong_point = AdaptorSecret::generate().public_point();

        let msg = b"test message";
        let keypair = KeyPair::generate();
        let adaptor_sig = adaptor_sign(msg, keypair.private_key(), &adaptor_point).unwrap();

        assert!(!adaptor_verify(
            msg,
            keypair.public_key(),
            &adaptor_sig,
            &wrong_point
        ));
    }

    /// Test 3: `adaptor_verify` fails with wrong message
    #[test]
    fn test_adaptor_verify_wrong_message() {
        let secret = AdaptorSecret::generate();
        let adaptor_point = secret.public_point();

        let keypair = KeyPair::generate();
        let adaptor_sig =
            adaptor_sign(b"correct message", keypair.private_key(), &adaptor_point).unwrap();

        assert!(!adaptor_verify(
            b"wrong message",
            keypair.public_key(),
            &adaptor_sig,
            &adaptor_point
        ));
    }

    /// Test 4: `adaptor_complete` with wrong t produces invalid signature
    #[test]
    fn test_adaptor_complete_wrong_t() {
        let secret = AdaptorSecret::generate();
        let adaptor_point = secret.public_point();
        let wrong_secret = AdaptorSecret::generate();

        let msg = b"test";
        let keypair = KeyPair::generate();
        let adaptor_sig = adaptor_sign(msg, keypair.private_key(), &adaptor_point).unwrap();
        let bad_sig = adaptor_complete(&adaptor_sig, &wrong_secret);

        assert!(crate::signature::verify(msg, &bad_sig, keypair.public_key()).is_err());
    }

    /// Test 5: `adaptor_extract` with wrong completed signature returns wrong t
    #[test]
    fn test_adaptor_extract_wrong_signature() {
        let secret = AdaptorSecret::generate();
        let adaptor_point = secret.public_point();

        let keypair = KeyPair::generate();
        let adaptor_sig = adaptor_sign(b"msg", keypair.private_key(), &adaptor_point).unwrap();

        // Sign a different message to get a different valid signature
        let other_sig = crate::signature::sign(b"other msg", keypair.private_key());

        // Extract from mismatched signatures should NOT return original t
        if let Ok(extracted) = adaptor_extract(&other_sig, &adaptor_sig) {
            assert_ne!(extracted, secret);
        }
    }

    // ========== GRUPO 2 — Security properties ==========

    /// Test 6: Different keys produce different adaptor sigs for same message and T
    #[test]
    fn test_adaptor_sig_not_deterministic_across_keys() {
        let secret = AdaptorSecret::generate();
        let adaptor_point = secret.public_point();
        let msg = b"same message";

        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();

        let sig1 = adaptor_sign(msg, kp1.private_key(), &adaptor_point).unwrap();
        let sig2 = adaptor_sign(msg, kp2.private_key(), &adaptor_point).unwrap();

        assert_ne!(sig1, sig2);
    }

    /// Test 7: The adaptor sig bytes do not contain t directly
    #[test]
    fn test_adaptor_sig_does_not_reveal_t() {
        let secret = AdaptorSecret::generate();
        let adaptor_point = secret.public_point();

        let keypair = KeyPair::generate();
        let adaptor_sig = adaptor_sign(b"msg", keypair.private_key(), &adaptor_point).unwrap();

        let sig_bytes = adaptor_sig.to_bytes();
        let t_bytes = secret.as_bytes();
        assert!(!sig_bytes.windows(32).any(|w| w == t_bytes));
    }

    /// Test 8: Identity point is rejected
    #[test]
    fn test_adaptor_sign_rejects_identity_point() {
        let identity = EdwardsPoint::default(); // identity
        let keypair = KeyPair::generate();
        let result = adaptor_sign(b"msg", keypair.private_key(), &identity);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), AdaptorError::IdentityPoint);
    }

    // ========== GRUPO 3 — BridgeHTLC integration ==========

    /// Test 9: `counter_hash` for Monero is the compressed adaptor point (32 bytes)
    #[test]
    fn test_monero_counter_hash_is_compressed_point() {
        let secret = AdaptorSecret::generate();
        let adaptor_point = secret.public_point();

        let counter_hash = adaptor_point_to_hash(&adaptor_point);

        // 32 bytes
        assert_eq!(counter_hash.as_bytes().len(), 32);

        // Roundtrip: counter_hash -> T
        let recovered = hash_to_adaptor_point(&counter_hash).unwrap();
        assert_eq!(adaptor_point.compress(), recovered.compress());
    }

    // Note: Test 10 (BridgeHTLC v2 Monero roundtrip) lives in doli-core's transaction/tests.rs
    // because it depends on doli_core::Output which can't be imported from the crypto crate.

    // ========== GRUPO 4 — Additional robustness tests ==========

    /// Test 11: `AdaptorSignature` serialization roundtrip
    #[test]
    fn test_adaptor_signature_serialization() {
        let secret = AdaptorSecret::generate();
        let adaptor_point = secret.public_point();
        let keypair = KeyPair::generate();

        let adaptor_sig = adaptor_sign(b"test", keypair.private_key(), &adaptor_point).unwrap();

        let bytes = adaptor_sig.to_bytes();
        let recovered = AdaptorSignature::from_bytes(&bytes).unwrap();
        assert_eq!(adaptor_sig, recovered);
    }

    /// Test 12: Multiple consecutive swaps with different secrets
    #[test]
    fn test_multiple_swaps_independent() {
        for _ in 0..5 {
            let secret = AdaptorSecret::generate();
            let adaptor_point = secret.public_point();

            let keypair = KeyPair::generate();
            let msg = b"swap iteration";

            let adaptor_sig = adaptor_sign(msg, keypair.private_key(), &adaptor_point).unwrap();
            assert!(adaptor_verify(
                msg,
                keypair.public_key(),
                &adaptor_sig,
                &adaptor_point
            ));

            let completed = adaptor_complete(&adaptor_sig, &secret);
            assert!(crate::signature::verify(msg, &completed, keypair.public_key()).is_ok());

            let extracted = adaptor_extract(&completed, &adaptor_sig).unwrap();
            assert_eq!(secret, extracted);
        }
    }

    /// Test 13: Same keypair, same message, same T produces same adaptor sig (deterministic nonce)
    #[test]
    fn test_adaptor_sign_deterministic() {
        let secret = AdaptorSecret::generate();
        let adaptor_point = secret.public_point();
        let keypair = KeyPair::generate();
        let msg = b"deterministic test";

        let sig1 = adaptor_sign(msg, keypair.private_key(), &adaptor_point).unwrap();
        let sig2 = adaptor_sign(msg, keypair.private_key(), &adaptor_point).unwrap();

        assert_eq!(sig1, sig2);
    }

    /// Test 14: `adaptor_verify` fails with wrong public key
    #[test]
    fn test_adaptor_verify_wrong_pubkey() {
        let secret = AdaptorSecret::generate();
        let adaptor_point = secret.public_point();

        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();

        let adaptor_sig = adaptor_sign(b"msg", kp1.private_key(), &adaptor_point).unwrap();

        // Verify with wrong pubkey should fail
        assert!(!adaptor_verify(
            b"msg",
            kp2.public_key(),
            &adaptor_sig,
            &adaptor_point
        ));
    }
}
