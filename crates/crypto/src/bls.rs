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

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop};

use blst::min_pk::{AggregateSignature, PublicKey as BlstPublicKey, SecretKey, Signature};
use blst::BLST_ERROR;

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
const POP_DST: &[u8] = b"BLS_POP_BLS12381G2_XMD:SHA-256_SSWU_RO_DOLI_POP_V1";

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

/// A BLS12-381 public key (48 bytes, compressed G1 point).
#[derive(Clone, Copy, Eq)]
pub struct BlsPublicKeyWrapped([u8; BLS_PUBLIC_KEY_SIZE]);

impl BlsPublicKeyWrapped {
    /// All-zero value (used as default / "no BLS key").
    pub const ZERO: Self = Self([0u8; BLS_PUBLIC_KEY_SIZE]);

    /// Create from raw compressed bytes **without** curve validation.
    ///
    /// Use [`try_from_slice`] for validated construction from untrusted input.
    /// This is intended for deserialization of already-validated data.
    #[must_use]
    pub const fn from_bytes_unchecked(bytes: [u8; BLS_PUBLIC_KEY_SIZE]) -> Self {
        Self(bytes)
    }

    /// Try to create from a byte slice with curve validation.
    ///
    /// # Errors
    ///
    /// Returns error if length is wrong or bytes are not a valid G1 point.
    pub fn try_from_slice(slice: &[u8]) -> Result<Self, BlsError> {
        if slice.len() != BLS_PUBLIC_KEY_SIZE {
            return Err(BlsError::InvalidPublicKey);
        }
        let mut bytes = [0u8; BLS_PUBLIC_KEY_SIZE];
        bytes.copy_from_slice(slice);

        // Validate it's a valid curve point
        BlstPublicKey::from_bytes(&bytes).map_err(|_| BlsError::InvalidPublicKey)?;

        Ok(Self(bytes))
    }

    /// Get the underlying bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; BLS_PUBLIC_KEY_SIZE] {
        &self.0
    }

    /// Check if this is the zero (unset) key.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; BLS_PUBLIC_KEY_SIZE]
    }

    /// Create from a hex string with curve validation.
    ///
    /// # Errors
    ///
    /// Returns error if hex is invalid or not a valid public key.
    pub fn from_hex(s: &str) -> Result<Self, BlsError> {
        let bytes = hex::decode(s)?;
        Self::try_from_slice(&bytes)
    }

    /// Convert to lowercase hex string.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Convert to the internal blst type.
    fn to_blst(self) -> Result<BlstPublicKey, BlsError> {
        BlstPublicKey::from_bytes(&self.0).map_err(|_| BlsError::InvalidPublicKey)
    }
}

impl PartialEq for BlsPublicKeyWrapped {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl std::hash::Hash for BlsPublicKeyWrapped {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl fmt::Debug for BlsPublicKeyWrapped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BlsPubKey({}...)", &self.to_hex()[..12])
    }
}

impl fmt::Display for BlsPublicKeyWrapped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl Default for BlsPublicKeyWrapped {
    fn default() -> Self {
        Self::ZERO
    }
}

impl Serialize for BlsPublicKeyWrapped {
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

impl<'de> Deserialize<'de> for BlsPublicKeyWrapped {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let s = String::deserialize(deserializer)?;
            if s.is_empty() || s.bytes().all(|b| b == b'0') {
                return Ok(Self::ZERO);
            }
            BlsPublicKeyWrapped::from_hex(&s).map_err(serde::de::Error::custom)
        } else {
            let bytes: Vec<u8> = Deserialize::deserialize(deserializer)?;
            if bytes.len() == BLS_PUBLIC_KEY_SIZE && bytes.iter().all(|&b| b == 0) {
                return Ok(Self::ZERO);
            }
            BlsPublicKeyWrapped::try_from_slice(&bytes).map_err(serde::de::Error::custom)
        }
    }
}

/// A BLS12-381 secret key (32 bytes).
///
/// Automatically zeroized on drop. See module-level docs for known limitation
/// regarding intermediate `blst::SecretKey` temporaries.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct BlsSecretKey([u8; 32]);

impl BlsSecretKey {
    /// Generate a new random secret key.
    ///
    /// # Panics
    ///
    /// Panics if `blst::SecretKey::key_gen` fails (should never happen with 32-byte IKM).
    #[must_use]
    pub fn generate() -> Self {
        let mut ikm = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut ikm);
        let sk = SecretKey::key_gen(&ikm, &[]).expect("key_gen with 32 bytes always succeeds");
        ikm.zeroize();
        Self(sk.to_bytes())
    }

    /// Create from raw bytes with scalar validation.
    ///
    /// # Errors
    ///
    /// Returns error if the bytes are not a valid BLS scalar (e.g. zero or >= group order).
    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self, BlsError> {
        // Validate by attempting to construct a blst SecretKey
        SecretKey::from_bytes(&bytes).map_err(|_| BlsError::InvalidSecretKey)?;
        Ok(Self(bytes))
    }

    /// Get the raw bytes.
    ///
    /// # Security
    ///
    /// Handle with care — do not log or persist unencrypted.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Derive the public key.
    ///
    /// # Panics
    ///
    /// Panics if the stored bytes are not a valid BLS secret key. This cannot
    /// happen when constructed via `generate()`, `from_bytes()`, or `from_hex()`
    /// since all validate the scalar on construction.
    #[must_use]
    pub fn public_key(&self) -> BlsPublicKeyWrapped {
        let sk = SecretKey::from_bytes(&self.0).expect("validated on construction");
        let pk = sk.sk_to_pk();
        BlsPublicKeyWrapped(pk.to_bytes())
    }

    /// Create from hex string with scalar validation.
    ///
    /// # Errors
    ///
    /// Returns error if hex is invalid, wrong length, or not a valid scalar.
    pub fn from_hex(s: &str) -> Result<Self, BlsError> {
        let bytes = hex::decode(s)?;
        if bytes.len() != 32 {
            return Err(BlsError::InvalidSecretKey);
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Self::from_bytes(arr)
    }

    /// Convert to hex string.
    ///
    /// # Security
    ///
    /// Handle with care.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Get the internal blst `SecretKey`.
    fn to_blst(&self) -> SecretKey {
        SecretKey::from_bytes(&self.0).expect("validated on construction")
    }
}

impl Clone for BlsSecretKey {
    fn clone(&self) -> Self {
        Self(self.0)
    }
}

impl fmt::Debug for BlsSecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BlsSecretKey([REDACTED])")
    }
}

/// A BLS12-381 signature (96 bytes, compressed G2 point).
///
/// Can represent either an individual or aggregate signature.
#[derive(Clone, Eq)]
pub struct BlsSignature([u8; BLS_SIGNATURE_SIZE]);

impl BlsSignature {
    /// All-zero value (empty/unset signature).
    pub const ZERO: Self = Self([0u8; BLS_SIGNATURE_SIZE]);

    /// Create from raw bytes **without** G2 curve validation.
    ///
    /// Use [`try_from_slice`] for validated construction from untrusted input.
    #[must_use]
    pub const fn from_bytes_unchecked(bytes: [u8; BLS_SIGNATURE_SIZE]) -> Self {
        Self(bytes)
    }

    /// Try to create from a byte slice with G2 curve validation.
    ///
    /// # Errors
    ///
    /// Returns error if length is wrong or bytes are not a valid G2 point.
    pub fn try_from_slice(slice: &[u8]) -> Result<Self, BlsError> {
        if slice.len() != BLS_SIGNATURE_SIZE {
            return Err(BlsError::InvalidSignature);
        }
        let mut bytes = [0u8; BLS_SIGNATURE_SIZE];
        bytes.copy_from_slice(slice);

        // Validate it's a valid G2 curve point
        Signature::from_bytes(&bytes).map_err(|_| BlsError::InvalidSignature)?;

        Ok(Self(bytes))
    }

    /// Get the underlying bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; BLS_SIGNATURE_SIZE] {
        &self.0
    }

    /// Check if this is the zero (unset) signature.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; BLS_SIGNATURE_SIZE]
    }

    /// Create from hex string with G2 curve validation.
    ///
    /// # Errors
    ///
    /// Returns error if hex is invalid, wrong length, or not a valid G2 point.
    pub fn from_hex(s: &str) -> Result<Self, BlsError> {
        let bytes = hex::decode(s)?;
        Self::try_from_slice(&bytes)
    }

    /// Convert to hex string.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Convert to the internal blst type.
    fn to_blst(&self) -> Result<Signature, BlsError> {
        Signature::from_bytes(&self.0).map_err(|_| BlsError::InvalidSignature)
    }
}

impl PartialEq for BlsSignature {
    fn eq(&self, other: &Self) -> bool {
        self.0[..] == other.0[..]
    }
}

impl fmt::Debug for BlsSignature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BlsSig({}...)", &self.to_hex()[..16])
    }
}

impl Default for BlsSignature {
    fn default() -> Self {
        Self::ZERO
    }
}

impl Serialize for BlsSignature {
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

impl<'de> Deserialize<'de> for BlsSignature {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let s = String::deserialize(deserializer)?;
            if s.is_empty() || s.bytes().all(|b| b == b'0') {
                return Ok(Self::ZERO);
            }
            BlsSignature::from_hex(&s).map_err(serde::de::Error::custom)
        } else {
            let bytes: Vec<u8> = Deserialize::deserialize(deserializer)?;
            if bytes.len() == BLS_SIGNATURE_SIZE && bytes.iter().all(|&b| b == 0) {
                return Ok(Self::ZERO);
            }
            BlsSignature::try_from_slice(&bytes).map_err(serde::de::Error::custom)
        }
    }
}

/// A BLS key pair (secret + public).
pub struct BlsKeyPair {
    secret: BlsSecretKey,
    public: BlsPublicKeyWrapped,
}

impl BlsKeyPair {
    /// Generate a new random key pair.
    #[must_use]
    pub fn generate() -> Self {
        let secret = BlsSecretKey::generate();
        let public = secret.public_key();
        Self { secret, public }
    }

    /// Create from a secret key.
    #[must_use]
    pub fn from_secret_key(secret: BlsSecretKey) -> Self {
        let public = secret.public_key();
        Self { secret, public }
    }

    /// Get a reference to the secret key.
    #[must_use]
    pub fn secret_key(&self) -> &BlsSecretKey {
        &self.secret
    }

    /// Get a reference to the public key.
    #[must_use]
    pub fn public_key(&self) -> &BlsPublicKeyWrapped {
        &self.public
    }

    /// Generate a proof-of-possession for this key pair.
    ///
    /// The `PoP` is a BLS signature over the serialized public key using the
    /// PoP-specific DST. Must be submitted at registration time to prevent
    /// rogue public key attacks on aggregate verification.
    ///
    /// # Errors
    ///
    /// Returns error if signing fails (should not happen with a valid key pair).
    pub fn proof_of_possession(&self) -> Result<BlsSignature, BlsError> {
        bls_sign_pop(&self.secret, &self.public)
    }
}

impl Clone for BlsKeyPair {
    fn clone(&self) -> Self {
        Self {
            secret: self.secret.clone(),
            public: self.public,
        }
    }
}

impl fmt::Debug for BlsKeyPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlsKeyPair")
            .field("public", &self.public)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

// ==================== Operations ====================

/// Sign a message with a BLS secret key.
///
/// Uses the DOLI attestation DST for domain separation.
///
/// # Errors
///
/// Returns error if the secret key bytes are invalid.
pub fn bls_sign(message: &[u8], secret_key: &BlsSecretKey) -> Result<BlsSignature, BlsError> {
    let sk = secret_key.to_blst();
    let sig = sk.sign(message, ATTESTATION_DST, &[]);
    Ok(BlsSignature(sig.to_bytes()))
}

/// Verify a single BLS signature.
///
/// # Errors
///
/// Returns error if signature or public key is invalid, or verification fails.
pub fn bls_verify(
    message: &[u8],
    signature: &BlsSignature,
    public_key: &BlsPublicKeyWrapped,
) -> Result<(), BlsError> {
    let sig = signature.to_blst()?;
    let pk = public_key.to_blst()?;

    let result = sig.verify(true, message, ATTESTATION_DST, &[], &pk, true);
    if result != BLST_ERROR::BLST_SUCCESS {
        return Err(BlsError::VerificationFailed);
    }
    Ok(())
}

/// Generate a proof-of-possession: sign the public key with the `PoP` DST.
///
/// This proves the caller possesses the secret key corresponding to the
/// public key, preventing rogue public key attacks on aggregate signatures.
///
/// # Errors
///
/// Returns error if signing fails.
pub fn bls_sign_pop(
    secret_key: &BlsSecretKey,
    public_key: &BlsPublicKeyWrapped,
) -> Result<BlsSignature, BlsError> {
    let sk = secret_key.to_blst();
    let sig = sk.sign(public_key.as_bytes(), POP_DST, &[]);
    Ok(BlsSignature(sig.to_bytes()))
}

/// Verify a proof-of-possession for a BLS public key.
///
/// Checks that the `PoP` signature is a valid signature over the public key
/// bytes using the `PoP` DST. Must be verified at registration time before
/// the public key is accepted for aggregate verification.
///
/// # Errors
///
/// Returns error if the `PoP` is invalid.
pub fn bls_verify_pop(
    public_key: &BlsPublicKeyWrapped,
    pop: &BlsSignature,
) -> Result<(), BlsError> {
    let sig = pop.to_blst()?;
    let pk = public_key.to_blst()?;

    let result = sig.verify(true, public_key.as_bytes(), POP_DST, &[], &pk, true);
    if result != BLST_ERROR::BLST_SUCCESS {
        return Err(BlsError::InvalidProofOfPossession);
    }
    Ok(())
}

/// Aggregate multiple BLS signatures into one.
///
/// The resulting signature can be verified against all corresponding public keys.
///
/// # Errors
///
/// Returns error if any signature is invalid or the set is empty.
pub fn bls_aggregate(signatures: &[BlsSignature]) -> Result<BlsSignature, BlsError> {
    if signatures.is_empty() {
        return Err(BlsError::EmptyAggregation);
    }

    let first = signatures[0].to_blst()?;
    let mut agg = AggregateSignature::from_signature(&first);

    for sig in &signatures[1..] {
        let s = sig.to_blst()?;
        agg.add_signature(&s, true)
            .map_err(|_| BlsError::InvalidSignature)?;
    }

    Ok(BlsSignature(agg.to_signature().to_bytes()))
}

/// Verify an aggregate BLS signature against multiple public keys.
///
/// All public keys must have been PoP-validated at registration time.
/// Each public key signed the same `message`. The aggregate signature
/// is valid iff ALL individual signatures were valid.
///
/// # Errors
///
/// Returns error if verification fails, or any key/signature is invalid.
pub fn bls_verify_aggregate(
    message: &[u8],
    aggregate_signature: &BlsSignature,
    public_keys: &[BlsPublicKeyWrapped],
) -> Result<(), BlsError> {
    if public_keys.is_empty() {
        return Err(BlsError::EmptyAggregation);
    }

    let sig = aggregate_signature.to_blst()?;

    let blst_pks: Vec<BlstPublicKey> = public_keys
        .iter()
        .copied()
        .map(BlsPublicKeyWrapped::to_blst)
        .collect::<Result<Vec<_>, _>>()?;

    let pk_refs: Vec<&BlstPublicKey> = blst_pks.iter().collect();

    let result = sig.fast_aggregate_verify(true, message, ATTESTATION_DST, &pk_refs);
    if result != BLST_ERROR::BLST_SUCCESS {
        return Err(BlsError::VerificationFailed);
    }

    Ok(())
}

/// Build the attestation message that producers sign.
///
/// Format: `block_hash || slot (4 bytes BE)`
///
/// Same structure as Ed25519 attestations for consistency.
#[must_use]
pub fn attestation_message(block_hash: &crate::Hash, slot: u32) -> Vec<u8> {
    let mut msg = Vec::with_capacity(36);
    msg.extend_from_slice(block_hash.as_bytes());
    msg.extend_from_slice(&slot.to_be_bytes());
    msg
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_keygen() {
        let kp = BlsKeyPair::generate();
        assert!(!kp.public_key().is_zero());
    }

    #[test]
    fn test_keygen_deterministic_from_bytes() {
        let sk = BlsSecretKey::generate();
        let pk1 = sk.public_key();
        let pk2 = sk.public_key();
        assert_eq!(pk1, pk2);
    }

    #[test]
    fn test_sign_verify() {
        let kp = BlsKeyPair::generate();
        let msg = b"test message";

        let sig = bls_sign(msg, kp.secret_key()).unwrap();
        assert!(!sig.is_zero());

        bls_verify(msg, &sig, kp.public_key()).unwrap();
    }

    #[test]
    fn test_wrong_key_fails() {
        let kp1 = BlsKeyPair::generate();
        let kp2 = BlsKeyPair::generate();
        let msg = b"test message";

        let sig = bls_sign(msg, kp1.secret_key()).unwrap();
        assert!(bls_verify(msg, &sig, kp2.public_key()).is_err());
    }

    #[test]
    fn test_wrong_message_fails() {
        let kp = BlsKeyPair::generate();
        let sig = bls_sign(b"message A", kp.secret_key()).unwrap();
        assert!(bls_verify(b"message B", &sig, kp.public_key()).is_err());
    }

    #[test]
    fn test_aggregate_verify() {
        let keys: Vec<BlsKeyPair> = (0..5).map(|_| BlsKeyPair::generate()).collect();
        let msg = b"shared message";

        let sigs: Vec<BlsSignature> = keys
            .iter()
            .map(|kp| bls_sign(msg, kp.secret_key()).unwrap())
            .collect();

        let agg = bls_aggregate(&sigs).unwrap();

        let pks: Vec<BlsPublicKeyWrapped> = keys.iter().map(|kp| *kp.public_key()).collect();
        bls_verify_aggregate(msg, &agg, &pks).unwrap();
    }

    #[test]
    fn test_aggregate_missing_signer_fails() {
        let keys: Vec<BlsKeyPair> = (0..5).map(|_| BlsKeyPair::generate()).collect();
        let msg = b"shared message";

        // Only sign with first 4
        let sigs: Vec<BlsSignature> = keys[..4]
            .iter()
            .map(|kp| bls_sign(msg, kp.secret_key()).unwrap())
            .collect();

        let agg = bls_aggregate(&sigs).unwrap();

        // Verify claims all 5 signed — should fail
        let all_pks: Vec<BlsPublicKeyWrapped> = keys.iter().map(|kp| *kp.public_key()).collect();
        assert!(bls_verify_aggregate(msg, &agg, &all_pks).is_err());
    }

    #[test]
    fn test_aggregate_wrong_message_fails() {
        let keys: Vec<BlsKeyPair> = (0..3).map(|_| BlsKeyPair::generate()).collect();

        let sigs: Vec<BlsSignature> = keys
            .iter()
            .map(|kp| bls_sign(b"message A", kp.secret_key()).unwrap())
            .collect();

        let agg = bls_aggregate(&sigs).unwrap();
        let pks: Vec<BlsPublicKeyWrapped> = keys.iter().map(|kp| *kp.public_key()).collect();

        assert!(bls_verify_aggregate(b"message B", &agg, &pks).is_err());
    }

    #[test]
    fn test_aggregate_single_signer() {
        let kp = BlsKeyPair::generate();
        let msg = b"solo";

        let sig = bls_sign(msg, kp.secret_key()).unwrap();
        let agg = bls_aggregate(&[sig]).unwrap();

        bls_verify_aggregate(msg, &agg, &[*kp.public_key()]).unwrap();
    }

    #[test]
    fn test_empty_aggregate_fails() {
        assert!(bls_aggregate(&[]).is_err());
    }

    #[test]
    fn test_hex_roundtrip() {
        let kp = BlsKeyPair::generate();

        let pk_hex = kp.public_key().to_hex();
        let pk_recovered = BlsPublicKeyWrapped::from_hex(&pk_hex).unwrap();
        assert_eq!(kp.public_key(), &pk_recovered);

        let sk_hex = kp.secret_key().to_hex();
        let sk_recovered = BlsSecretKey::from_hex(&sk_hex).unwrap();
        assert_eq!(kp.secret_key().as_bytes(), sk_recovered.as_bytes());

        let msg = b"test";
        let sig = bls_sign(msg, kp.secret_key()).unwrap();
        let sig_hex = sig.to_hex();
        let sig_recovered = BlsSignature::from_hex(&sig_hex).unwrap();
        assert_eq!(sig, sig_recovered);
    }

    #[test]
    fn test_serde_json_pubkey() {
        let kp = BlsKeyPair::generate();
        let json = serde_json::to_string(kp.public_key()).unwrap();
        let recovered: BlsPublicKeyWrapped = serde_json::from_str(&json).unwrap();
        assert_eq!(kp.public_key(), &recovered);
    }

    #[test]
    fn test_serde_json_signature() {
        let kp = BlsKeyPair::generate();
        let sig = bls_sign(b"test", kp.secret_key()).unwrap();
        let json = serde_json::to_string(&sig).unwrap();
        let recovered: BlsSignature = serde_json::from_str(&json).unwrap();
        assert_eq!(sig, recovered);
    }

    #[test]
    fn test_attestation_message_format() {
        let hash = crate::hash::hash(b"block data");
        let msg = attestation_message(&hash, 42);
        assert_eq!(msg.len(), 36); // 32 (hash) + 4 (slot)
        assert_eq!(&msg[32..], &42u32.to_be_bytes());
    }

    #[test]
    fn test_attestation_sign_verify_flow() {
        let kp = BlsKeyPair::generate();
        let block_hash = crate::hash::hash(b"block 123");
        let slot = 42u32;

        let msg = attestation_message(&block_hash, slot);
        let sig = bls_sign(&msg, kp.secret_key()).unwrap();
        bls_verify(&msg, &sig, kp.public_key()).unwrap();
    }

    #[test]
    fn test_aggregate_attestation_flow() {
        // Simulate 12 producers attesting the same block
        let producers: Vec<BlsKeyPair> = (0..12).map(|_| BlsKeyPair::generate()).collect();
        let block_hash = crate::hash::hash(b"block at slot 100");
        let msg = attestation_message(&block_hash, 100);

        // Each producer signs
        let sigs: Vec<BlsSignature> = producers
            .iter()
            .map(|kp| bls_sign(&msg, kp.secret_key()).unwrap())
            .collect();

        // Block producer aggregates
        let agg = bls_aggregate(&sigs).unwrap();

        // Validators verify (only need the aggregate + public keys)
        let pks: Vec<BlsPublicKeyWrapped> = producers.iter().map(|kp| *kp.public_key()).collect();
        bls_verify_aggregate(&msg, &agg, &pks).unwrap();

        // Verify the aggregate signature is exactly 96 bytes regardless of signer count
        assert_eq!(agg.as_bytes().len(), BLS_SIGNATURE_SIZE);
    }

    #[test]
    fn test_debug_redacts_secret() {
        let sk = BlsSecretKey::generate();
        let debug = format!("{sk:?}");
        assert!(debug.contains("REDACTED"));
    }

    #[test]
    fn test_zero_pubkey() {
        let zero = BlsPublicKeyWrapped::ZERO;
        assert!(zero.is_zero());
    }

    // ==================== Proof of Possession Tests ====================

    #[test]
    fn test_pop_sign_verify() {
        let kp = BlsKeyPair::generate();
        let pop = kp.proof_of_possession().unwrap();
        bls_verify_pop(kp.public_key(), &pop).unwrap();
    }

    #[test]
    fn test_pop_wrong_key_fails() {
        let kp1 = BlsKeyPair::generate();
        let kp2 = BlsKeyPair::generate();
        let pop = kp1.proof_of_possession().unwrap();
        // PoP from kp1 must not verify against kp2's pubkey
        assert!(bls_verify_pop(kp2.public_key(), &pop).is_err());
    }

    #[test]
    fn test_pop_prevents_rogue_key_attack() {
        // Scenario: attacker tries to forge aggregate attestation for victim
        let victim = BlsKeyPair::generate();
        let attacker = BlsKeyPair::generate();

        // Attacker cannot produce a valid PoP for a rogue key
        // (they'd need the secret key corresponding to the rogue pubkey)
        // This test verifies the `PoP` mechanism works — the actual rogue key
        // construction is prevented by requiring PoP at registration
        let attacker_pop = attacker.proof_of_possession().unwrap();
        bls_verify_pop(attacker.public_key(), &attacker_pop).unwrap();

        // Attacker's PoP doesn't verify against victim's key
        assert!(bls_verify_pop(victim.public_key(), &attacker_pop).is_err());
    }

    #[test]
    fn test_pop_domain_separated_from_attestation() {
        // PoP signature must not be valid as an attestation signature and vice versa
        let kp = BlsKeyPair::generate();

        let pop = kp.proof_of_possession().unwrap();
        // PoP should NOT verify as an attestation over the same bytes
        assert!(bls_verify(kp.public_key().as_bytes(), &pop, kp.public_key()).is_err());

        // Attestation over pubkey bytes should NOT verify as PoP
        let attest_sig = bls_sign(kp.public_key().as_bytes(), kp.secret_key()).unwrap();
        assert!(bls_verify_pop(kp.public_key(), &attest_sig).is_err());
    }

    // ==================== Invalid Input Tests ====================

    #[test]
    fn test_invalid_scalar_from_bytes() {
        // All zeros is not a valid BLS scalar
        assert!(BlsSecretKey::from_bytes([0u8; 32]).is_err());
    }

    #[test]
    fn test_invalid_pubkey_bytes() {
        // Random bytes are almost certainly not a valid G1 point
        let bad = [0xFFu8; BLS_PUBLIC_KEY_SIZE];
        assert!(BlsPublicKeyWrapped::try_from_slice(&bad).is_err());
    }

    #[test]
    fn test_invalid_signature_bytes() {
        // Random bytes are almost certainly not a valid G2 point
        let bad = [0xFFu8; BLS_SIGNATURE_SIZE];
        assert!(BlsSignature::try_from_slice(&bad).is_err());
    }

    #[test]
    fn test_serde_zero_pubkey_roundtrip() {
        let zero = BlsPublicKeyWrapped::ZERO;
        let json = serde_json::to_string(&zero).unwrap();
        let recovered: BlsPublicKeyWrapped = serde_json::from_str(&json).unwrap();
        assert_eq!(zero, recovered);
        assert!(recovered.is_zero());
    }

    #[test]
    fn test_serde_zero_signature_roundtrip() {
        let zero = BlsSignature::ZERO;
        let json = serde_json::to_string(&zero).unwrap();
        let recovered: BlsSignature = serde_json::from_str(&json).unwrap();
        assert_eq!(zero, recovered);
        assert!(recovered.is_zero());
    }
}
