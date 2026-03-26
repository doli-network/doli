//! BLS12-381 type definitions: public keys, secret keys, signatures, and key pairs.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop};

use blst::min_pk::{PublicKey as BlstPublicKey, SecretKey, Signature};

use super::{BlsError, BLS_PUBLIC_KEY_SIZE, BLS_SIGNATURE_SIZE};

// ---------------------------------------------------------------------------
// BlsPublicKeyWrapped
// ---------------------------------------------------------------------------

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
    pub(super) fn to_blst(self) -> Result<BlstPublicKey, BlsError> {
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

// ---------------------------------------------------------------------------
// BlsSecretKey
// ---------------------------------------------------------------------------

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
    pub(super) fn to_blst(&self) -> SecretKey {
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

// ---------------------------------------------------------------------------
// BlsSignature
// ---------------------------------------------------------------------------

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
    pub(super) fn to_blst(&self) -> Result<Signature, BlsError> {
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

// ---------------------------------------------------------------------------
// BlsKeyPair
// ---------------------------------------------------------------------------

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
        super::bls_sign_pop(&self.secret, &self.public)
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
