//! Encrypted content primitives: AES-256-GCM + ECIES key wrapping.
//!
//! Content is encrypted with a random symmetric key (AES-256-GCM).
//! The key is wrapped with the owner's public key (ECIES: X25519 + BLAKE3 KDF).
//! Transfer re-wraps the key with the new owner's public key.
//! RevealContent publishes the symmetric key on-chain (irrevocable).

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::RngCore;
use sha2::Digest;
use x25519_dalek::{EphemeralSecret, PublicKey as X25519Public, StaticSecret};

use crate::hash::hash;
use crate::PrivateKey;

/// Encrypted content errors.
#[derive(Debug, thiserror::Error)]
pub enum EncryptedContentError {
    /// AES-256-GCM encryption failed.
    #[error("encryption failed")]
    EncryptionFailed,
    /// AES-256-GCM decryption failed (wrong key or corrupted ciphertext).
    #[error("decryption failed")]
    DecryptionFailed,
    /// Wrapped key is not 80 bytes.
    #[error("invalid wrapped key length")]
    InvalidWrappedKey,
    /// ECIES key unwrap failed (wrong private key).
    #[error("key unwrap failed")]
    KeyUnwrapFailed,
}

/// Generate a random 32-byte content key.
pub fn generate_content_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

/// Encrypt plaintext with AES-256-GCM.
/// Returns (ciphertext, nonce).
pub fn encrypt_content(
    key: &[u8; 32],
    plaintext: &[u8],
) -> Result<(Vec<u8>, [u8; 12]), EncryptedContentError> {
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|_| EncryptedContentError::EncryptionFailed)?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| EncryptedContentError::EncryptionFailed)?;
    Ok((ciphertext, nonce_bytes))
}

/// Decrypt ciphertext with AES-256-GCM.
pub fn decrypt_content(
    key: &[u8; 32],
    ciphertext: &[u8],
    nonce: &[u8; 12],
) -> Result<Vec<u8>, EncryptedContentError> {
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|_| EncryptedContentError::DecryptionFailed)?;
    let nonce = Nonce::from_slice(nonce);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| EncryptedContentError::DecryptionFailed)
}

/// Convert an Ed25519 private key to an X25519 static secret.
/// Ed25519 private key → SHA-512 → first 32 bytes (clamped) → X25519.
fn ed25519_to_x25519_secret(private_key: &PrivateKey) -> StaticSecret {
    let hash = sha2::Sha512::digest(private_key.as_bytes());
    let mut secret_bytes = [0u8; 32];
    secret_bytes.copy_from_slice(&hash[..32]);
    // Clamp (X25519 requirement)
    secret_bytes[0] &= 248;
    secret_bytes[31] &= 127;
    secret_bytes[31] |= 64;
    StaticSecret::from(secret_bytes)
}

/// Convert an Ed25519 public key to an X25519 public key.
/// Uses the birational map from Edwards to Montgomery form.
fn ed25519_to_x25519_public(pubkey: &crate::PublicKey) -> X25519Public {
    use curve25519_dalek::edwards::CompressedEdwardsY;
    let compressed =
        CompressedEdwardsY::from_slice(pubkey.as_bytes()).expect("valid 32-byte pubkey");
    let edwards = compressed.decompress().expect("valid Edwards point");
    let montgomery = edwards.to_montgomery();
    X25519Public::from(*montgomery.as_bytes())
}

/// Wrap a content key with the owner's public key (ECIES).
///
/// Returns 80 bytes: [ephemeral_public(32) | encrypted_key(32) | tag(16)]
///
/// Protocol:
/// 1. Generate ephemeral X25519 keypair
/// 2. Compute shared secret: ECDH(ephemeral_secret, owner_x25519_public)
/// 3. Derive symmetric key: BLAKE3(shared_secret)
/// 4. Encrypt content_key with AES-256-GCM using derived key
pub fn wrap_key(
    content_key: &[u8; 32],
    owner_pubkey: &crate::PublicKey,
) -> Result<[u8; 80], EncryptedContentError> {
    let owner_x25519 = ed25519_to_x25519_public(owner_pubkey);

    // Ephemeral keypair
    let ephemeral_secret = EphemeralSecret::random_from_rng(rand::thread_rng());
    let ephemeral_public = X25519Public::from(&ephemeral_secret);

    // Shared secret via ECDH
    let shared_secret = ephemeral_secret.diffie_hellman(&owner_x25519);

    // KDF: BLAKE3(shared_secret)
    let derived_key = hash(shared_secret.as_bytes());
    let aes_key: [u8; 32] = *derived_key.as_bytes();

    // Encrypt the content key with derived key
    // Use first 12 bytes of ephemeral_public as nonce (deterministic, unique per ephemeral)
    let cipher =
        Aes256Gcm::new_from_slice(&aes_key).map_err(|_| EncryptedContentError::EncryptionFailed)?;
    let nonce_bytes: [u8; 12] = ephemeral_public.as_bytes()[..12].try_into().unwrap();
    let nonce = Nonce::from_slice(&nonce_bytes);
    let encrypted_key = cipher
        .encrypt(nonce, content_key.as_slice())
        .map_err(|_| EncryptedContentError::EncryptionFailed)?;

    // Pack: ephemeral_public(32) + encrypted_key_with_tag(48) = 80 bytes
    let mut result = [0u8; 80];
    result[..32].copy_from_slice(ephemeral_public.as_bytes());
    result[32..80].copy_from_slice(&encrypted_key);
    Ok(result)
}

/// Unwrap a content key with the owner's private key (ECIES).
///
/// Input: 80 bytes from wrap_key.
/// Returns the 32-byte content key.
pub fn unwrap_key(
    wrapped: &[u8; 80],
    owner_private_key: &PrivateKey,
) -> Result<[u8; 32], EncryptedContentError> {
    if wrapped.len() != 80 {
        return Err(EncryptedContentError::InvalidWrappedKey);
    }

    // Extract ephemeral public key
    let mut ephemeral_bytes = [0u8; 32];
    ephemeral_bytes.copy_from_slice(&wrapped[..32]);
    let ephemeral_public = X25519Public::from(ephemeral_bytes);

    // Convert owner's Ed25519 private key to X25519
    let owner_x25519_secret = ed25519_to_x25519_secret(owner_private_key);

    // Shared secret via ECDH
    let shared_secret = owner_x25519_secret.diffie_hellman(&ephemeral_public);

    // KDF: BLAKE3(shared_secret)
    let derived_key = hash(shared_secret.as_bytes());
    let aes_key: [u8; 32] = *derived_key.as_bytes();

    // Decrypt the content key
    let cipher =
        Aes256Gcm::new_from_slice(&aes_key).map_err(|_| EncryptedContentError::KeyUnwrapFailed)?;
    let nonce_bytes: [u8; 12] = ephemeral_bytes[..12].try_into().unwrap();
    let nonce = Nonce::from_slice(&nonce_bytes);
    let content_key = cipher
        .decrypt(nonce, &wrapped[32..80])
        .map_err(|_| EncryptedContentError::KeyUnwrapFailed)?;

    let mut key = [0u8; 32];
    if content_key.len() != 32 {
        return Err(EncryptedContentError::KeyUnwrapFailed);
    }
    key.copy_from_slice(&content_key);
    Ok(key)
}

/// Compute the content hash (BLAKE3 of plaintext).
pub fn content_hash(plaintext: &[u8]) -> [u8; 32] {
    *hash(plaintext).as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KeyPair;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = generate_content_key();
        let plaintext = b"Hello, DOLI encrypted content!";
        let (ciphertext, nonce) = encrypt_content(&key, plaintext).unwrap();
        let decrypted = decrypt_content(&key, &ciphertext, &nonce).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_wrap_unwrap_roundtrip() {
        let kp = KeyPair::generate();
        let content_key = generate_content_key();
        let wrapped = wrap_key(&content_key, kp.public_key()).unwrap();
        let unwrapped = unwrap_key(&wrapped, kp.private_key()).unwrap();
        assert_eq!(unwrapped, content_key);
    }

    #[test]
    fn test_wrong_key_fails() {
        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();
        let content_key = generate_content_key();
        let wrapped = wrap_key(&content_key, kp1.public_key()).unwrap();
        // Wrong private key should fail
        assert!(unwrap_key(&wrapped, kp2.private_key()).is_err());
    }

    #[test]
    fn test_content_hash() {
        let plaintext = b"test content";
        let h = content_hash(plaintext);
        assert_eq!(h, *hash(plaintext).as_bytes());
    }

    #[test]
    fn test_rewrap_transfer() {
        let sender = KeyPair::generate();
        let recipient = KeyPair::generate();
        let content_key = generate_content_key();

        // Sender wraps
        let wrapped = wrap_key(&content_key, sender.public_key()).unwrap();

        // Sender unwraps
        let unwrapped = unwrap_key(&wrapped, sender.private_key()).unwrap();
        assert_eq!(unwrapped, content_key);

        // Re-wrap for recipient
        let rewrapped = wrap_key(&unwrapped, recipient.public_key()).unwrap();

        // Recipient unwraps
        let final_key = unwrap_key(&rewrapped, recipient.private_key()).unwrap();
        assert_eq!(final_key, content_key);

        // Sender can no longer unwrap the new wrapped key
        assert!(unwrap_key(&rewrapped, sender.private_key()).is_err());
    }
}
