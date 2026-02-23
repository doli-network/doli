//! Attestation types for the DOLI finality gadget.
//!
//! Tier 1 and Tier 2 producers sign attestations for blocks they observe.
//! Attestations are aggregated per region into `RegionAggregate` messages,
//! then committed into the next block's `presence_root` field (version >= 2).

use crypto::{signature, Hash, PrivateKey, PublicKey, Signature, ATTESTATION_DOMAIN};
use serde::{Deserialize, Serialize};

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
    /// Create and sign a new attestation.
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
        }
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

/// Aggregated attestations from a single region.
///
/// Bundles individual Ed25519 signatures (no BLS) from all attestors
/// in one region for a single block. At ~1000 attestors per region,
/// this is ~64KB per region per slot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionAggregate {
    /// Hash of the block being attested.
    pub block_hash: Hash,
    /// Slot of the attested block.
    pub slot: u32,
    /// Region index (0..NUM_REGIONS).
    pub region: u32,
    /// Number of attestors who signed.
    pub attester_count: u32,
    /// Total effective weight of all attestors in this aggregate.
    pub total_weight: u64,
    /// Individual Ed25519 signatures (paired with attesters).
    pub signatures: Vec<Signature>,
    /// Public keys of attestors (same order as signatures).
    pub attesters: Vec<PublicKey>,
}

impl RegionAggregate {
    /// Build a region aggregate from individual attestations.
    ///
    /// All attestations must be for the same block_hash.
    pub fn from_attestations(
        attestations: Vec<Attestation>,
        region: u32,
    ) -> Result<Self, AttestationError> {
        if attestations.is_empty() {
            return Err(AttestationError::EmptyAttestations);
        }

        let block_hash = attestations[0].block_hash;
        let slot = attestations[0].slot;

        let mut signatures = Vec::with_capacity(attestations.len());
        let mut attesters = Vec::with_capacity(attestations.len());
        let mut total_weight: u64 = 0;

        for att in &attestations {
            if att.block_hash != block_hash {
                return Err(AttestationError::BlockMismatch {
                    expected: block_hash,
                    got: att.block_hash,
                });
            }
            signatures.push(att.signature);
            attesters.push(att.attester);
            total_weight = total_weight.saturating_add(att.attester_weight);
        }

        Ok(Self {
            block_hash,
            slot,
            region,
            attester_count: attestations.len() as u32,
            total_weight,
            signatures,
            attesters,
        })
    }

    /// Verify all signatures in this aggregate.
    pub fn verify(&self) -> Result<(), AttestationError> {
        if self.signatures.len() != self.attesters.len() {
            return Err(AttestationError::InvalidSignature);
        }

        let msg = Attestation::signing_bytes(self.block_hash, self.slot);

        for (sig, pk) in self.signatures.iter().zip(self.attesters.iter()) {
            signature::verify_with_domain(ATTESTATION_DOMAIN, &msg, sig, pk)
                .map_err(|_| AttestationError::InvalidSignature)?;
        }

        Ok(())
    }

    /// Total attestation weight in this aggregate.
    pub fn attestation_weight(&self) -> u64 {
        self.total_weight
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
        };

        assert!(att.verify().is_err());
    }

    #[test]
    fn test_region_aggregate_from_attestations() {
        let keys: Vec<KeyPair> = (0..5).map(|_| KeyPair::generate()).collect();
        let block_hash = crypto::hash::hash(b"block");

        let attestations: Vec<Attestation> = keys
            .iter()
            .enumerate()
            .map(|(i, kp)| {
                Attestation::new(
                    block_hash,
                    10,
                    50,
                    (i + 1) as u64,
                    kp.private_key(),
                    *kp.public_key(),
                )
            })
            .collect();

        let agg = RegionAggregate::from_attestations(attestations, 3).unwrap();
        assert_eq!(agg.attester_count, 5);
        assert_eq!(agg.total_weight, 1 + 2 + 3 + 4 + 5);
        assert_eq!(agg.region, 3);
        assert_eq!(agg.slot, 10);
        assert!(agg.verify().is_ok());
    }

    #[test]
    fn test_aggregate_verify_detects_bad_sig() {
        let kp = KeyPair::generate();
        let kp_bad = KeyPair::generate();
        let block_hash = crypto::hash::hash(b"block");

        let good = Attestation::new(block_hash, 1, 10, 1, kp.private_key(), *kp.public_key());
        // Create a bad attestation: signed by kp_bad but claiming kp as attester
        let bad_msg = Attestation::signing_bytes(block_hash, 1);
        let bad_sig =
            signature::sign_with_domain(ATTESTATION_DOMAIN, &bad_msg, kp_bad.private_key());
        let bad = Attestation {
            block_hash,
            slot: 1,
            height: 10,
            attester: *kp.public_key(), // wrong!
            attester_weight: 1,
            signature: bad_sig,
        };

        let agg = RegionAggregate {
            block_hash,
            slot: 1,
            region: 0,
            attester_count: 2,
            total_weight: 2,
            signatures: vec![good.signature, bad.signature],
            attesters: vec![good.attester, bad.attester],
        };

        assert!(agg.verify().is_err());
    }

    #[test]
    fn test_aggregate_empty_rejected() {
        let result = RegionAggregate::from_attestations(vec![], 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_aggregate_mismatched_blocks_rejected() {
        let kp = KeyPair::generate();
        let hash1 = crypto::hash::hash(b"block1");
        let hash2 = crypto::hash::hash(b"block2");

        let att1 = Attestation::new(hash1, 1, 10, 1, kp.private_key(), *kp.public_key());
        let att2 = Attestation::new(hash2, 1, 10, 1, kp.private_key(), *kp.public_key());

        let result = RegionAggregate::from_attestations(vec![att1, att2], 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_attestation_serialization_roundtrip() {
        let kp = KeyPair::generate();
        let block_hash = crypto::hash::hash(b"roundtrip");

        let att = Attestation::new(block_hash, 99, 500, 10, kp.private_key(), *kp.public_key());
        let bytes = att.to_bytes();
        assert!(!bytes.is_empty());

        let recovered = Attestation::from_bytes(&bytes).expect("deserialization failed");
        assert_eq!(recovered.block_hash, att.block_hash);
        assert_eq!(recovered.slot, att.slot);
        assert_eq!(recovered.height, att.height);
        assert_eq!(recovered.attester_weight, att.attester_weight);
        assert!(recovered.verify().is_ok());
    }

    #[test]
    fn test_attestation_from_invalid_bytes() {
        assert!(Attestation::from_bytes(&[0, 1, 2]).is_none());
        assert!(Attestation::from_bytes(&[]).is_none());
    }

    #[test]
    fn test_aggregate_weight_accumulation() {
        let keys: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
        let block_hash = crypto::hash::hash(b"weights");

        let attestations: Vec<Attestation> = keys
            .iter()
            .enumerate()
            .map(|(i, kp)| {
                Attestation::new(
                    block_hash,
                    1,
                    10,
                    (i as u64 + 1) * 100,
                    kp.private_key(),
                    *kp.public_key(),
                )
            })
            .collect();

        let agg = RegionAggregate::from_attestations(attestations, 5).unwrap();
        assert_eq!(agg.attestation_weight(), 600);
        assert_eq!(agg.attester_count, 3);
        assert_eq!(agg.region, 5);
    }

    #[test]
    fn test_region_assignment_deterministic() {
        use crate::consensus::{producer_region, NUM_REGIONS};
        let kp = KeyPair::generate();
        let r1 = producer_region(kp.public_key());
        let r2 = producer_region(kp.public_key());
        assert_eq!(r1, r2);
        assert!(r1 < NUM_REGIONS);
    }

    #[test]
    fn test_region_assignment_distribution() {
        use crate::consensus::{producer_region, NUM_REGIONS};
        let mut seen = std::collections::HashSet::new();
        for _ in 0..500 {
            let kp = KeyPair::generate();
            seen.insert(producer_region(kp.public_key()));
        }
        // With 15 regions and 500 keys, all regions should appear
        assert_eq!(seen.len(), NUM_REGIONS as usize);
    }
}
