//! The `Attestation` message: construction, signing and verification.

use crypto::{bls_sign, BlsKeyPair};
use crypto::{signature, Hash, PrivateKey, PublicKey, Signature, ATTESTATION_DOMAIN};
use serde::{Deserialize, Serialize};

/// The BLS attestation preimage: the attested block hash and nothing else.
///
/// Frozen by INC-I-178 R1 (REQ-BLS-001). No slot: every attester on one block must
/// sign the SAME message or their signatures never aggregate, and any verifier must
/// be able to rebuild the message from the block alone. Domain separation is
/// `crypto::ATTESTATION_DST`, which is distinct from the proof-of-possession DST.
#[must_use]
pub fn bls_attest_msg(block_hash: &Hash) -> [u8; 32] {
    *block_hash.as_bytes()
}

/// A single attestation from one producer for a specific block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attestation {
    /// Hash of the block being attested.
    pub block_hash: Hash,
    /// Slot of the attested block.
    pub slot: u32,
    /// Height of the attested block.
    pub height: u64,
    /// Public key of the attester.
    pub attester: PublicKey,
    /// Effective weight of the attester at this epoch.
    pub attester_weight: u64,
    /// Ed25519 signature over (ATTESTATION_DOMAIN || block_hash || slot).
    pub signature: Signature,
    /// BLS12-381 signature over the attestation message (96 bytes).
    ///
    /// Present when the attester has a BLS key. Empty for pre-BLS attestations.
    #[serde(default)]
    pub bls_signature: Vec<u8>,
}

/// Errors from attestation operations.
#[derive(Debug, thiserror::Error)]
pub enum AttestationError {
    #[error("signature verification failed")]
    InvalidSignature,

    #[error("attestation for wrong block: expected {expected}, got {got}")]
    BlockMismatch { expected: Hash, got: Hash },

    #[error("empty attestation set")]
    EmptyAttestations,
}

impl Attestation {
    /// Create and sign a new attestation (Ed25519 only, no BLS).
    pub fn new(
        block_hash: Hash,
        slot: u32,
        height: u64,
        weight: u64,
        private_key: &PrivateKey,
        public_key: PublicKey,
    ) -> Self {
        let msg = Self::signing_bytes(block_hash, slot);
        let sig = signature::sign_with_domain(ATTESTATION_DOMAIN, &msg, private_key);

        Self {
            block_hash,
            slot,
            height,
            attester: public_key,
            attester_weight: weight,
            signature: sig,
            bls_signature: Vec::new(),
        }
    }

    /// Create and sign a new attestation with both Ed25519 and BLS signatures.
    ///
    /// The Ed25519 half is byte-identical to [`Attestation::new`] so an un-upgraded
    /// peer still verifies it; the BLS half signs [`bls_attest_msg`]. A BLS signing
    /// error is returned, never swallowed: only the egress may decide to ship the
    /// Ed25519-only attestation anyway.
    pub fn new_with_bls(
        block_hash: Hash,
        slot: u32,
        height: u64,
        weight: u64,
        private_key: &PrivateKey,
        public_key: PublicKey,
        bls_key: &BlsKeyPair,
    ) -> Result<Self, crypto::BlsError> {
        let bls_signature = bls_sign(&bls_attest_msg(&block_hash), bls_key.secret_key())?
            .as_bytes()
            .to_vec();

        Ok(Self {
            bls_signature,
            ..Self::new(block_hash, slot, height, weight, private_key, public_key)
        })
    }

    /// Verify the attestation signature.
    pub fn verify(&self) -> Result<(), AttestationError> {
        let msg = Self::signing_bytes(self.block_hash, self.slot);
        signature::verify_with_domain(ATTESTATION_DOMAIN, &msg, &self.signature, &self.attester)
            .map_err(|_| AttestationError::InvalidSignature)
    }

    /// Compute the bytes that are signed: block_hash || slot (big-endian).
    fn signing_bytes(block_hash: Hash, slot: u32) -> Vec<u8> {
        let mut msg = Vec::with_capacity(36);
        msg.extend_from_slice(block_hash.as_bytes());
        msg.extend_from_slice(&slot.to_be_bytes());
        msg
    }

    /// Serialize for gossip transmission.
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    /// Deserialize from gossip bytes.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        bincode::deserialize(bytes).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::KeyPair;

    #[test]
    fn test_attestation_sign_verify() {
        let kp = KeyPair::generate();
        let block_hash = crypto::hash::hash(b"test block");

        let att = Attestation::new(block_hash, 42, 100, 5, kp.private_key(), *kp.public_key());

        assert_eq!(att.block_hash, block_hash);
        assert_eq!(att.slot, 42);
        assert_eq!(att.height, 100);
        assert_eq!(att.attester_weight, 5);
        assert!(att.verify().is_ok());
    }

    #[test]
    fn test_attestation_wrong_key_fails() {
        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();
        let block_hash = crypto::hash::hash(b"test");

        // Sign with kp1 but claim kp2 as attester
        let msg = Attestation::signing_bytes(block_hash, 1);
        let sig = signature::sign_with_domain(ATTESTATION_DOMAIN, &msg, kp1.private_key());

        let att = Attestation {
            block_hash,
            slot: 1,
            height: 10,
            attester: *kp2.public_key(),
            attester_weight: 1,
            signature: sig,
            bls_signature: Vec::new(),
        };

        assert!(att.verify().is_err());
    }
}
