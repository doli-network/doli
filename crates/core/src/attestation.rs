//! Attestation types for the DOLI finality gadget.
//!
//! Tier 1 and Tier 2 producers sign attestations for blocks they observe.
//! Attestations are aggregated per region into `RegionAggregate` messages,
//! then committed into the next block's `presence_root` field (version >= 2).

use std::collections::{HashMap, HashSet};

use crypto::{bls_sign, BlsKeyPair};
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
    /// BLS12-381 signature over the attestation message (96 bytes).
    ///
    /// Present when the attester has a BLS key. Used for aggregate signature
    /// verification in block validation. Empty for pre-BLS attestations.
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
    pub fn new_with_bls(
        block_hash: Hash,
        slot: u32,
        height: u64,
        weight: u64,
        private_key: &PrivateKey,
        public_key: PublicKey,
        bls_key: &BlsKeyPair,
    ) -> Self {
        let msg = Self::signing_bytes(block_hash, slot);
        let sig = signature::sign_with_domain(ATTESTATION_DOMAIN, &msg, private_key);

        // BLS signs the attestation message: block_hash || slot
        let bls_msg = crypto::attestation_message(&block_hash, slot);
        let bls_bytes = match bls_sign(&bls_msg, bls_key.secret_key()) {
            Ok(sig) => sig.as_bytes().to_vec(),
            Err(_) => Vec::new(), // Graceful fallback — Ed25519 still works
        };

        Self {
            block_hash,
            slot,
            height,
            attester: public_key,
            attester_weight: weight,
            signature: sig,
            bls_signature: bls_bytes,
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
    /// Region index for gossip sharding.
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

// ==================== On-Chain Attestation Bitfield ====================
//
// Each block producer commits which producers attested the current minute
// by packing a bitfield into the `presence_root` header field (32 bytes = 256 bits).
//
// Bit N = 1 means producer at index N (sorted by pubkey, same as DeterministicScheduler)
// sent an attestation for this minute. At epoch boundary, scan all blocks to count
// unique minutes per producer. ≥90% of 60 minutes = qualified for epoch rewards.
//
// Supports up to 256 producers. Beyond that, requires a body-stored bitfield (future).

/// Slots per attestation minute (6 slots × 10s = 60s).
pub const SLOTS_PER_ATTESTATION_MINUTE: u32 = 6;

/// Attestation minutes per epoch (360 slots / 6 = 60 minutes) — mainnet default.
pub const ATTESTATION_MINUTES_PER_EPOCH: u32 = 60;

/// Attestation qualification threshold: 90% of 60 = 54 minutes — mainnet default.
pub const ATTESTATION_QUALIFICATION_THRESHOLD: u32 = 54;

/// Compute attestation minutes per epoch from blocks_per_epoch.
/// For mainnet (360): 360/6 = 60. For testnet (36): 36/6 = 6.
#[inline]
pub fn attestation_minutes_per_epoch(blocks_per_epoch: u64) -> u32 {
    (blocks_per_epoch as u32) / SLOTS_PER_ATTESTATION_MINUTE
}

/// Compute attestation qualification threshold (90%) from blocks_per_epoch.
/// For mainnet (360): 90% of 60 = 54. For testnet (36): 90% of 6 = 5.
#[inline]
pub fn attestation_qualification_threshold(blocks_per_epoch: u64) -> u32 {
    let minutes = attestation_minutes_per_epoch(blocks_per_epoch);
    (minutes * 90) / 100
}

/// Compute the attestation minute from a slot number.
///
/// Each minute covers 6 slots (60 seconds at 10s/slot).
/// Deterministic: all nodes compute the same minute from the same slot.
#[inline]
pub fn attestation_minute(slot: u32) -> u32 {
    slot / SLOTS_PER_ATTESTATION_MINUTE
}

/// Encode an attestation bitfield into a Hash for `presence_root`.
///
/// `attested_indices` are indices into the sorted producer list.
/// Bit N = 1 means producer at index N attested the current minute.
/// Supports up to 256 producers (32 bytes × 8 bits).
pub fn encode_attestation_bitfield(attested_indices: &[usize]) -> Hash {
    let mut bytes = [0u8; 32];
    for &idx in attested_indices {
        if idx < 256 {
            bytes[idx / 8] |= 1 << (idx % 8);
        }
    }
    Hash::from_bytes(bytes)
}

/// Encode an attestation bitfield into a Vec<u8> (no 256-producer cap).
///
/// Used for post-BITFIELD_BODY_ACTIVATION_HEIGHT blocks where the bitfield
/// is stored in the block body instead of packed into the 32-byte presence_root.
///
/// `attested_indices` are indices into the sorted producer list.
/// `producer_count` determines the length of the output (ceil(producer_count / 8) bytes).
pub fn encode_attestation_bitfield_vec(
    attested_indices: &[usize],
    producer_count: usize,
) -> Vec<u8> {
    let byte_count = producer_count.div_ceil(8);
    let mut bytes = vec![0u8; byte_count];
    for &idx in attested_indices {
        if idx < producer_count {
            bytes[idx / 8] |= 1 << (idx % 8);
        }
    }
    bytes
}

/// Decode attestation bitfield from a Vec<u8> (no 256-producer cap).
///
/// Used for post-BITFIELD_BODY_ACTIVATION_HEIGHT blocks where the bitfield
/// is stored in the block body.
///
/// Returns indices of producers that attested.
/// `producer_count` limits the scan range.
pub fn decode_attestation_bitfield_vec(bitfield: &[u8], producer_count: usize) -> Vec<usize> {
    let mut indices = Vec::new();
    for idx in 0..producer_count {
        let byte_idx = idx / 8;
        if byte_idx >= bitfield.len() {
            break;
        }
        if bitfield[byte_idx] & (1 << (idx % 8)) != 0 {
            indices.push(idx);
        }
    }
    indices
}

/// Validate a body attestation bitfield: no bits set beyond `producer_count`.
pub fn validate_attestation_bitfield_vec(bitfield: &[u8], producer_count: usize) -> bool {
    let expected_bytes = producer_count.div_ceil(8);
    // Extra bytes beyond expected must be zero
    for b in bitfield.iter().skip(expected_bytes) {
        if *b != 0 {
            return false;
        }
    }
    // Stray bits in the last expected byte
    let remainder = producer_count % 8;
    if remainder > 0 && expected_bytes <= bitfield.len() {
        let mask = !((1u8 << remainder) - 1); // bits above remainder
        if bitfield[expected_bytes - 1] & mask != 0 {
            return false;
        }
    }
    true
}

/// Decode attestation bitfield from `presence_root`.
///
/// Returns indices of producers that attested this minute.
/// `producer_count` limits the scan range.
pub fn decode_attestation_bitfield(presence_root: &Hash, producer_count: usize) -> Vec<usize> {
    let bytes = presence_root.as_bytes();
    let max = producer_count.min(256);
    let mut indices = Vec::new();
    for idx in 0..max {
        if bytes[idx / 8] & (1 << (idx % 8)) != 0 {
            indices.push(idx);
        }
    }
    indices
}

/// Validate that a presence_root bitfield has no bits set beyond `producer_count`.
///
/// Returns false if any stray bits are set (potential manipulation).
pub fn validate_attestation_bitfield(presence_root: &Hash, producer_count: usize) -> bool {
    if producer_count >= 256 {
        return true; // All bits valid
    }
    let bytes = presence_root.as_bytes();
    for idx in producer_count..256 {
        if bytes[idx / 8] & (1 << (idx % 8)) != 0 {
            return false;
        }
    }
    true
}

/// In-memory tracker for minute attestations received via gossip.
///
/// Used by the block producer to build the bitfield for `presence_root`
/// and aggregate BLS signatures for the block body.
/// NOT used for epoch reward qualification — that comes from scanning on-chain bitfields.
pub struct MinuteAttestationTracker {
    /// pubkey → set of minutes they attested in
    attested: HashMap<PublicKey, HashSet<u32>>,
    /// (pubkey, minute) → BLS signature bytes (latest per minute per producer)
    bls_sigs: HashMap<(PublicKey, u32), Vec<u8>>,
}

impl MinuteAttestationTracker {
    /// Create an empty tracker.
    pub fn new() -> Self {
        Self {
            attested: HashMap::new(),
            bls_sigs: HashMap::new(),
        }
    }

    /// Record that a producer attested in a given minute.
    pub fn record(&mut self, pubkey: PublicKey, minute: u32) {
        self.attested.entry(pubkey).or_default().insert(minute);
    }

    /// Record that a producer attested in a given minute with a BLS signature.
    pub fn record_with_bls(&mut self, pubkey: PublicKey, minute: u32, bls_sig: Vec<u8>) {
        self.attested.entry(pubkey).or_default().insert(minute);
        if !bls_sig.is_empty() {
            self.bls_sigs.insert((pubkey, minute), bls_sig);
        }
    }

    /// Get all producers that attested in a specific minute.
    pub fn attested_in_minute(&self, minute: u32) -> Vec<&PublicKey> {
        self.attested
            .iter()
            .filter(|(_, minutes)| minutes.contains(&minute))
            .map(|(pk, _)| pk)
            .collect()
    }

    /// Get BLS signatures for all producers that attested in a specific minute.
    ///
    /// Returns (pubkey, bls_sig_bytes) pairs. Only includes producers with BLS sigs.
    pub fn bls_sigs_for_minute(&self, minute: u32) -> Vec<(&PublicKey, &[u8])> {
        self.attested
            .iter()
            .filter(|(_, minutes)| minutes.contains(&minute))
            .filter_map(|(pk, _)| {
                self.bls_sigs
                    .get(&(*pk, minute))
                    .map(|sig| (pk, sig.as_slice()))
            })
            .collect()
    }

    /// Total BLS signatures stored (diagnostic).
    pub fn bls_sig_count(&self) -> usize {
        self.bls_sigs.len()
    }

    /// Reset the tracker (at epoch boundary).
    pub fn reset(&mut self) {
        self.attested.clear();
        self.bls_sigs.clear();
    }
}

impl Default for MinuteAttestationTracker {
    fn default() -> Self {
        Self::new()
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
            bls_signature: Vec::new(),
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
}

#[cfg(test)]
mod bitfield_tests {
    use super::*;

    #[test]
    fn test_attestation_minute() {
        assert_eq!(attestation_minute(0), 0);
        assert_eq!(attestation_minute(5), 0);
        assert_eq!(attestation_minute(6), 1);
        assert_eq!(attestation_minute(11), 1);
        assert_eq!(attestation_minute(12), 2);
        assert_eq!(attestation_minute(359), 59);
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let indices = vec![0, 3, 7, 11];
        let hash = encode_attestation_bitfield(&indices);
        let decoded = decode_attestation_bitfield(&hash, 12);
        assert_eq!(decoded, indices);
    }

    #[test]
    fn test_encode_empty() {
        let hash = encode_attestation_bitfield(&[]);
        assert!(hash.is_zero());
    }

    #[test]
    fn test_decode_zero_is_empty() {
        let decoded = decode_attestation_bitfield(&Hash::ZERO, 12);
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_encode_all_producers() {
        let indices: Vec<usize> = (0..12).collect();
        let hash = encode_attestation_bitfield(&indices);
        let decoded = decode_attestation_bitfield(&hash, 12);
        assert_eq!(decoded, indices);
    }

    #[test]
    fn test_validate_bitfield_clean() {
        let indices = vec![0, 3, 7];
        let hash = encode_attestation_bitfield(&indices);
        assert!(validate_attestation_bitfield(&hash, 12));
    }

    #[test]
    fn test_validate_bitfield_stray_bit() {
        // Set bit 15 but claim only 12 producers
        let hash = encode_attestation_bitfield(&[0, 15]);
        assert!(!validate_attestation_bitfield(&hash, 12));
    }

    #[test]
    fn test_validate_bitfield_256_producers() {
        let hash = encode_attestation_bitfield(&[255]);
        assert!(validate_attestation_bitfield(&hash, 256));
    }

    #[test]
    fn test_tracker_basic() {
        let mut tracker = MinuteAttestationTracker::new();
        let pk = PublicKey::from_bytes([1u8; 32]);

        tracker.record(pk, 5);
        tracker.record(pk, 6);

        let minute_5 = tracker.attested_in_minute(5);
        assert_eq!(minute_5.len(), 1);
        assert_eq!(*minute_5[0], pk);

        let minute_7 = tracker.attested_in_minute(7);
        assert!(minute_7.is_empty());
    }

    #[test]
    fn test_tracker_multiple_producers() {
        let mut tracker = MinuteAttestationTracker::new();
        let pk1 = PublicKey::from_bytes([1u8; 32]);
        let pk2 = PublicKey::from_bytes([2u8; 32]);

        tracker.record(pk1, 5);
        tracker.record(pk2, 5);
        tracker.record(pk1, 6);

        assert_eq!(tracker.attested_in_minute(5).len(), 2);
        assert_eq!(tracker.attested_in_minute(6).len(), 1);
    }

    #[test]
    fn test_tracker_reset() {
        let mut tracker = MinuteAttestationTracker::new();
        let pk = PublicKey::from_bytes([1u8; 32]);

        tracker.record(pk, 5);
        assert_eq!(tracker.attested_in_minute(5).len(), 1);

        tracker.reset();
        assert!(tracker.attested_in_minute(5).is_empty());
    }

    #[test]
    fn test_constants() {
        assert_eq!(SLOTS_PER_ATTESTATION_MINUTE, 6);
        assert_eq!(ATTESTATION_MINUTES_PER_EPOCH, 60);
        assert_eq!(ATTESTATION_QUALIFICATION_THRESHOLD, 54);
    }
}
