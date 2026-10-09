//! Strict Ed25519 verification: rejects small-order keys/R and non-canonical S.

use crate::keys::PublicKey;
use crate::signature::{Signature, SignatureError};

/// Verify `signature` over `message` with `ed25519_dalek::VerifyingKey::verify_strict`.
///
/// # Errors
///
/// Returns an error if the key is invalid or the signature fails strict verification.
pub fn verify_strict(
    message: &[u8],
    signature: &Signature,
    public_key: &PublicKey,
) -> Result<(), SignatureError> {
    let verifying_key = public_key.to_verifying_key()?;
    let sig = ed25519_dalek::Signature::from_bytes(signature.as_bytes());
    verifying_key
        .verify_strict(message, &sig)
        .map_err(|_| SignatureError::VerificationFailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{signature::sign, KeyPair};

    #[test]
    fn strict_accepts_valid_rejects_other_message() {
        let kp = KeyPair::from_seed([7; 32]);
        let sig = sign(b"msg", kp.private_key());
        assert!(verify_strict(b"msg", &sig, kp.public_key()).is_ok());
        assert!(verify_strict(b"other", &sig, kp.public_key()).is_err());
    }
}
