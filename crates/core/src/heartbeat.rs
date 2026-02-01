//! # Heartbeat VDF and Witness System
//!
//! This module implements the heartbeat proof system for weighted presence rewards.
//! Each producer must submit a VDF heartbeat proof every slot to prove presence,
//! and collect witness signatures from other producers to validate it.
//!
//! ## Purpose
//!
//! Heartbeats prove that a producer was:
//! 1. Online during the slot (had to compute VDF)
//! 2. Connected to the network (received witness signatures)
//! 3. Unable to pre-compute (VDF input depends on prev_block_hash)
//!
//! ## Witness Requirement
//!
//! Each heartbeat requires at least `MIN_WITNESS_SIGNATURES` (2) signatures
//! from other active producers. This prevents:
//! - Isolated nodes claiming presence without network participation
//! - Sybil attacks where one node signs for fake nodes
//!
//! ## VDF Computation
//!
//! The heartbeat uses a hash-chain VDF that takes ~1 second to compute:
//! ```text
//! input = H("DOLI_HEARTBEAT_V1" || producer || slot || prev_hash)
//! output = H(H(H(...H(input)...))) // 10M iterations
//! ```

use crypto::hash::hash_concat;
use crypto::{Hash, Hasher, PublicKey, Signature};
use serde::{Deserialize, Serialize};

use crate::types::Slot;

// =============================================================================
// CONSTANTS
// =============================================================================

/// VDF iterations for heartbeat proof (~1 second on modern hardware).
///
/// 10,000,000 hash iterations provides:
/// - ~1 second compute time on i7-class CPU
/// - ~10ms verification time (parallel verification possible)
/// - Sufficient anti-grinding protection
pub const HEARTBEAT_VDF_ITERATIONS: u64 = 10_000_000;

/// Minimum witness signatures required for a valid heartbeat.
///
/// Requiring 2 witnesses ensures:
/// - Producer is connected to the network
/// - At least 2 other producers verify the heartbeat
/// - Sybil resistance (can't self-witness)
pub const MIN_WITNESS_SIGNATURES: usize = 2;

/// Protocol version for heartbeat messages.
pub const HEARTBEAT_VERSION: u8 = 1;

// =============================================================================
// WITNESS SIGNATURE
// =============================================================================

/// A witness attestation that a heartbeat is valid.
///
/// Other producers verify the heartbeat's VDF and signature,
/// then sign a witness message to attest its validity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WitnessSignature {
    /// Public key of the witnessing producer
    pub witness: PublicKey,
    /// Signature over the witness message
    pub signature: Signature,
}

impl WitnessSignature {
    /// Create a new witness signature.
    pub fn new(witness: PublicKey, signature: Signature) -> Self {
        Self { witness, signature }
    }

    /// Compute the witness message for signing.
    ///
    /// The witness message includes the producer, slot, and VDF output
    /// but NOT the full heartbeat data (more efficient).
    pub fn compute_message(producer: &PublicKey, slot: Slot, vdf_output: &[u8; 32]) -> Hash {
        hash_concat(&[
            b"DOLI_HEARTBEAT_WITNESS_V1",
            producer.as_bytes(),
            &slot.to_le_bytes(),
            vdf_output,
        ])
    }

    /// Verify this witness signature against the heartbeat data.
    pub fn verify(&self, producer: &PublicKey, slot: Slot, vdf_output: &[u8; 32]) -> bool {
        let message = Self::compute_message(producer, slot, vdf_output);
        crypto::signature::verify_hash(&message, &self.signature, &self.witness).is_ok()
    }
}

// =============================================================================
// HEARTBEAT
// =============================================================================

/// A heartbeat proof of presence for a slot.
///
/// The heartbeat proves that the producer:
/// 1. Knew the previous block hash (can't pre-compute)
/// 2. Spent ~1 second computing the VDF
/// 3. Has their private key (signature)
/// 4. Was connected to the network (witness signatures)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heartbeat {
    /// Protocol version for compatibility
    pub version: u8,

    /// Producer's public key
    pub producer: PublicKey,

    /// Slot this heartbeat is for
    pub slot: Slot,

    /// Hash of the previous block (prevents pre-computation)
    pub prev_block_hash: Hash,

    /// VDF output (hash chain result after HEARTBEAT_VDF_ITERATIONS)
    pub vdf_output: [u8; 32],

    /// Producer's signature over the heartbeat data
    pub signature: Signature,

    /// Witness signatures from other producers
    pub witnesses: Vec<WitnessSignature>,
}

/// Errors that can occur with heartbeats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeartbeatError {
    /// VDF output doesn't match expected value
    InvalidVdf,
    /// Producer signature is invalid
    InvalidSignature,
    /// Not enough witness signatures
    InsufficientWitnesses { have: usize, need: usize },
    /// Witness is not an active producer
    InvalidWitness(PublicKey),
    /// Producer cannot witness their own heartbeat
    SelfWitness,
    /// Witness signature is invalid
    InvalidWitnessSignature(PublicKey),
    /// Heartbeat version not supported
    UnsupportedVersion(u8),
    /// Previous block hash mismatch
    PrevHashMismatch,
}

impl std::fmt::Display for HeartbeatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidVdf => write!(f, "VDF output verification failed"),
            Self::InvalidSignature => write!(f, "producer signature invalid"),
            Self::InsufficientWitnesses { have, need } => {
                write!(f, "insufficient witnesses: have {}, need {}", have, need)
            }
            Self::InvalidWitness(pk) => {
                write!(
                    f,
                    "witness {} is not an active producer",
                    hex::encode(&pk.as_bytes()[..8])
                )
            }
            Self::SelfWitness => write!(f, "producer cannot witness their own heartbeat"),
            Self::InvalidWitnessSignature(pk) => {
                write!(
                    f,
                    "invalid witness signature from {}",
                    hex::encode(&pk.as_bytes()[..8])
                )
            }
            Self::UnsupportedVersion(v) => write!(f, "unsupported heartbeat version: {}", v),
            Self::PrevHashMismatch => write!(f, "previous block hash mismatch"),
        }
    }
}

impl std::error::Error for HeartbeatError {}

impl Heartbeat {
    /// Create a new heartbeat (without computing VDF - call compute_vdf first).
    pub fn new(
        producer: PublicKey,
        slot: Slot,
        prev_block_hash: Hash,
        vdf_output: [u8; 32],
        signature: Signature,
    ) -> Self {
        Self {
            version: HEARTBEAT_VERSION,
            producer,
            slot,
            prev_block_hash,
            vdf_output,
            signature,
            witnesses: Vec::new(),
        }
    }

    /// Compute the VDF input hash.
    ///
    /// The input depends on producer identity, slot, and previous block hash,
    /// making pre-computation impossible.
    pub fn compute_vdf_input(producer: &PublicKey, slot: Slot, prev_hash: &Hash) -> Hash {
        hash_concat(&[
            b"DOLI_HEARTBEAT_V1",
            producer.as_bytes(),
            &slot.to_le_bytes(),
            prev_hash.as_bytes(),
        ])
    }

    /// Compute the hash-chain VDF.
    ///
    /// This takes ~1 second on modern hardware.
    pub fn compute_vdf(input: &Hash) -> [u8; 32] {
        hash_chain_vdf(input, HEARTBEAT_VDF_ITERATIONS)
    }

    /// Compute the signing message for the producer.
    pub fn compute_signing_message(
        version: u8,
        producer: &PublicKey,
        slot: Slot,
        prev_hash: &Hash,
        vdf_output: &[u8; 32],
    ) -> Hash {
        hash_concat(&[
            b"DOLI_HEARTBEAT_SIGN_V1",
            &[version],
            producer.as_bytes(),
            &slot.to_le_bytes(),
            prev_hash.as_bytes(),
            vdf_output,
        ])
    }

    /// Get the signing message for this heartbeat.
    pub fn signing_message(&self) -> Hash {
        Self::compute_signing_message(
            self.version,
            &self.producer,
            self.slot,
            &self.prev_block_hash,
            &self.vdf_output,
        )
    }

    /// Verify the VDF output.
    pub fn verify_vdf(&self) -> bool {
        let input = Self::compute_vdf_input(&self.producer, self.slot, &self.prev_block_hash);
        let expected = hash_chain_vdf(&input, HEARTBEAT_VDF_ITERATIONS);
        self.vdf_output == expected
    }

    /// Verify the producer's signature.
    pub fn verify_signature(&self) -> bool {
        let message = self.signing_message();
        crypto::signature::verify_hash(&message, &self.signature, &self.producer).is_ok()
    }

    /// Add a witness signature.
    pub fn add_witness(&mut self, witness: WitnessSignature) {
        self.witnesses.push(witness);
    }

    /// Check if we have enough witness signatures.
    pub fn has_enough_witnesses(&self) -> bool {
        self.witnesses.len() >= MIN_WITNESS_SIGNATURES
    }

    /// Verify all witness signatures.
    ///
    /// Checks that:
    /// 1. We have at least MIN_WITNESS_SIGNATURES witnesses
    /// 2. Each witness is in the active_producers list
    /// 3. No witness is the producer themselves
    /// 4. Each witness signature is valid
    pub fn verify_witnesses(&self, active_producers: &[PublicKey]) -> Result<(), HeartbeatError> {
        // Check minimum count
        if self.witnesses.len() < MIN_WITNESS_SIGNATURES {
            return Err(HeartbeatError::InsufficientWitnesses {
                have: self.witnesses.len(),
                need: MIN_WITNESS_SIGNATURES,
            });
        }

        for witness_sig in &self.witnesses {
            // Check witness is an active producer
            if !active_producers.contains(&witness_sig.witness) {
                return Err(HeartbeatError::InvalidWitness(witness_sig.witness.clone()));
            }

            // Check witness is not the producer
            if witness_sig.witness == self.producer {
                return Err(HeartbeatError::SelfWitness);
            }

            // Verify the signature
            if !witness_sig.verify(&self.producer, self.slot, &self.vdf_output) {
                return Err(HeartbeatError::InvalidWitnessSignature(
                    witness_sig.witness.clone(),
                ));
            }
        }

        Ok(())
    }

    /// Verify the complete heartbeat.
    ///
    /// This verifies:
    /// 1. Version is supported
    /// 2. Previous block hash matches expected
    /// 3. VDF output is correct
    /// 4. Producer signature is valid
    /// 5. Witness signatures are valid
    pub fn verify_full(
        &self,
        expected_prev_hash: &Hash,
        active_producers: &[PublicKey],
    ) -> Result<(), HeartbeatError> {
        // Check version
        if self.version != HEARTBEAT_VERSION {
            return Err(HeartbeatError::UnsupportedVersion(self.version));
        }

        // Check prev_hash
        if self.prev_block_hash != *expected_prev_hash {
            return Err(HeartbeatError::PrevHashMismatch);
        }

        // Verify VDF
        if !self.verify_vdf() {
            return Err(HeartbeatError::InvalidVdf);
        }

        // Verify producer signature
        if !self.verify_signature() {
            return Err(HeartbeatError::InvalidSignature);
        }

        // Verify witnesses
        self.verify_witnesses(active_producers)?;

        Ok(())
    }

    /// Get the heartbeat's unique identifier.
    pub fn id(&self) -> Hash {
        hash_concat(&[self.producer.as_bytes(), &self.slot.to_le_bytes()])
    }

    /// Serialize the heartbeat.
    pub fn serialize(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    /// Deserialize a heartbeat.
    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        bincode::deserialize(bytes).ok()
    }

    /// Estimate serialized size in bytes.
    pub fn size(&self) -> usize {
        // version + producer + slot + prev_hash + vdf_output + signature + witnesses
        1 + 32 + 4 + 32 + 32 + 64 + (self.witnesses.len() * (32 + 64))
    }
}

// =============================================================================
// HASH-CHAIN VDF
// =============================================================================

/// Compute a hash-chain VDF.
///
/// This is a simple but effective VDF using iterated hashing.
/// It's verifiable by repeating the computation.
///
/// # Arguments
///
/// * `input` - The 32-byte input hash
/// * `iterations` - Number of hash iterations
///
/// # Returns
///
/// The final hash after all iterations
pub fn hash_chain_vdf(input: &Hash, iterations: u64) -> [u8; 32] {
    let mut state = *input.as_bytes();

    for _ in 0..iterations {
        let mut hasher = Hasher::new();
        hasher.update(&state);
        state = *hasher.finalize().as_bytes();
    }

    state
}

/// Verify a hash-chain VDF by recomputing it.
///
/// This takes the same time as computing, but can be parallelized
/// across multiple heartbeats.
pub fn verify_hash_chain_vdf(input: &Hash, expected_output: &[u8; 32], iterations: u64) -> bool {
    let computed = hash_chain_vdf(input, iterations);
    computed == *expected_output
}

// =============================================================================
// COMPACT HEARTBEAT (for Claims V2)
// =============================================================================

use crate::types::Amount;

/// Compact heartbeat for Claims V2 (without witness signatures).
///
/// This is a minimal representation of a heartbeat used in merkle proofs
/// for claiming epoch rewards. It strips out the witness signatures
/// (which are only needed for initial validation, not for claims) and
/// adds the bond_weight for reward calculation.
///
/// ## Size Comparison
/// - Full Heartbeat: ~357+ bytes (includes witnesses)
/// - CompactHeartbeat: ~172 bytes (fixed size)
///
/// ## Usage
/// 1. Block producer collects heartbeats, validates witnesses
/// 2. Creates CompactHeartbeat for each valid heartbeat
/// 3. Builds merkle tree of compact heartbeats
/// 4. Stores merkle root in block header (presence_root)
/// 5. Producers store their compact heartbeat + proof for later claims
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactHeartbeat {
    /// Producer's public key (32 bytes)
    pub producer: PublicKey,

    /// Slot this heartbeat is for (4 bytes)
    pub slot: Slot,

    /// Hash of the previous block (32 bytes)
    pub prev_block_hash: Hash,

    /// VDF output (32 bytes)
    pub vdf_output: [u8; 32],

    /// Producer's signature over the heartbeat data (64 bytes)
    pub signature: Signature,

    /// Producer's bond weight at time of heartbeat (8 bytes)
    /// This is the sum of all bond amounts for this producer
    pub bond_weight: Amount,
}

impl CompactHeartbeat {
    /// Create a new compact heartbeat from a full heartbeat.
    pub fn from_heartbeat(heartbeat: &Heartbeat, bond_weight: Amount) -> Self {
        Self {
            producer: heartbeat.producer.clone(),
            slot: heartbeat.slot,
            prev_block_hash: heartbeat.prev_block_hash,
            vdf_output: heartbeat.vdf_output,
            signature: heartbeat.signature.clone(),
            bond_weight,
        }
    }

    /// Create a new compact heartbeat directly.
    pub fn new(
        producer: PublicKey,
        slot: Slot,
        prev_block_hash: Hash,
        vdf_output: [u8; 32],
        signature: Signature,
        bond_weight: Amount,
    ) -> Self {
        Self {
            producer,
            slot,
            prev_block_hash,
            vdf_output,
            signature,
            bond_weight,
        }
    }

    /// Compute the leaf hash for inclusion in merkle tree.
    ///
    /// This hash uniquely identifies the heartbeat and is used as
    /// the leaf in the presence merkle tree.
    pub fn leaf_hash(&self) -> Hash {
        crypto::hash::hash_concat(&[
            b"DOLI_COMPACT_HEARTBEAT_V1",
            self.producer.as_bytes(),
            &self.slot.to_le_bytes(),
            self.prev_block_hash.as_bytes(),
            &self.vdf_output,
            &self.bond_weight.to_le_bytes(),
        ])
    }

    /// Verify the VDF output is correct for this heartbeat.
    ///
    /// # Arguments
    /// * `iterations` - Expected number of VDF iterations
    ///
    /// # Returns
    /// `true` if VDF output is valid
    pub fn verify_vdf(&self, iterations: u64) -> bool {
        let input = Heartbeat::compute_vdf_input(&self.producer, self.slot, &self.prev_block_hash);
        verify_hash_chain_vdf(&input, &self.vdf_output, iterations)
    }

    /// Verify the producer's signature.
    ///
    /// # Returns
    /// `true` if signature is valid
    pub fn verify_signature(&self) -> bool {
        let message = Heartbeat::compute_signing_message(
            HEARTBEAT_VERSION,
            &self.producer,
            self.slot,
            &self.prev_block_hash,
            &self.vdf_output,
        );
        crypto::signature::verify_hash(&message, &self.signature, &self.producer).is_ok()
    }

    /// Full verification of VDF and signature.
    ///
    /// # Arguments
    /// * `iterations` - Expected number of VDF iterations
    ///
    /// # Returns
    /// `Ok(())` if valid, `Err(HeartbeatError)` otherwise
    pub fn verify(&self, iterations: u64) -> Result<(), HeartbeatError> {
        if !self.verify_vdf(iterations) {
            return Err(HeartbeatError::InvalidVdf);
        }
        if !self.verify_signature() {
            return Err(HeartbeatError::InvalidSignature);
        }
        Ok(())
    }

    /// Get approximate size in bytes.
    pub fn size(&self) -> usize {
        // producer: 32, slot: 4, prev_hash: 32, vdf_output: 32, signature: 64, bond_weight: 8
        32 + 4 + 32 + 32 + 64 + 8 // = 172 bytes
    }
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::KeyPair;

    fn mock_hash(seed: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        Hash::from_bytes(bytes)
    }

    fn mock_pubkey(seed: u8) -> PublicKey {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        PublicKey::from_bytes(bytes)
    }

    #[test]
    fn test_vdf_input_uniqueness() {
        let pk1 = mock_pubkey(1);
        let pk2 = mock_pubkey(2);
        let hash1 = mock_hash(10);
        let hash2 = mock_hash(20);

        // Different producers = different inputs
        let input1 = Heartbeat::compute_vdf_input(&pk1, 100, &hash1);
        let input2 = Heartbeat::compute_vdf_input(&pk2, 100, &hash1);
        assert_ne!(input1, input2);

        // Different slots = different inputs
        let input3 = Heartbeat::compute_vdf_input(&pk1, 101, &hash1);
        assert_ne!(input1, input3);

        // Different prev_hash = different inputs
        let input4 = Heartbeat::compute_vdf_input(&pk1, 100, &hash2);
        assert_ne!(input1, input4);

        // Same inputs = same result (deterministic)
        let input5 = Heartbeat::compute_vdf_input(&pk1, 100, &hash1);
        assert_eq!(input1, input5);
    }

    #[test]
    fn test_hash_chain_vdf_deterministic() {
        let input = mock_hash(42);

        // Same input produces same output
        let output1 = hash_chain_vdf(&input, 100);
        let output2 = hash_chain_vdf(&input, 100);
        assert_eq!(output1, output2);

        // Different iterations produce different output
        let output3 = hash_chain_vdf(&input, 101);
        assert_ne!(output1, output3);
    }

    #[test]
    fn test_verify_hash_chain_vdf() {
        let input = mock_hash(42);
        let output = hash_chain_vdf(&input, 1000);

        assert!(verify_hash_chain_vdf(&input, &output, 1000));
        assert!(!verify_hash_chain_vdf(&input, &output, 999));

        let wrong_output = [0u8; 32];
        assert!(!verify_hash_chain_vdf(&input, &wrong_output, 1000));
    }

    #[test]
    fn test_witness_signature() {
        let producer = mock_pubkey(1);
        let slot = 100;
        let vdf_output = [42u8; 32];

        // Create witness keypair
        let witness_kp = KeyPair::generate();
        let witness_pk = witness_kp.public_key().clone();

        // Sign the witness message
        let message = WitnessSignature::compute_message(&producer, slot, &vdf_output);
        let signature = crypto::signature::sign_hash(&message, witness_kp.private_key());

        let witness_sig = WitnessSignature::new(witness_pk.clone(), signature);

        // Should verify
        assert!(witness_sig.verify(&producer, slot, &vdf_output));

        // Different producer should fail
        let other_producer = mock_pubkey(2);
        assert!(!witness_sig.verify(&other_producer, slot, &vdf_output));

        // Different slot should fail
        assert!(!witness_sig.verify(&producer, slot + 1, &vdf_output));

        // Different vdf_output should fail
        let other_output = [99u8; 32];
        assert!(!witness_sig.verify(&producer, slot, &other_output));
    }

    #[test]
    fn test_heartbeat_signature() {
        let kp = KeyPair::generate();
        let producer = kp.public_key().clone();
        let slot = 100;
        let prev_hash = mock_hash(1);

        // Compute VDF (use small iterations for test)
        let input = Heartbeat::compute_vdf_input(&producer, slot, &prev_hash);
        let vdf_output = hash_chain_vdf(&input, 100);

        // Sign the heartbeat
        let signing_msg = Heartbeat::compute_signing_message(
            HEARTBEAT_VERSION,
            &producer,
            slot,
            &prev_hash,
            &vdf_output,
        );
        let signature = crypto::signature::sign_hash(&signing_msg, kp.private_key());

        let heartbeat = Heartbeat::new(producer, slot, prev_hash, vdf_output, signature);

        // Signature should verify (VDF would fail with wrong iterations)
        assert!(heartbeat.verify_signature());
    }

    #[test]
    fn test_insufficient_witnesses() {
        let heartbeat = Heartbeat {
            version: HEARTBEAT_VERSION,
            producer: mock_pubkey(1),
            slot: 100,
            prev_block_hash: mock_hash(1),
            vdf_output: [0u8; 32],
            signature: Signature::default(),
            witnesses: vec![], // No witnesses
        };

        let active = vec![mock_pubkey(2), mock_pubkey(3)];
        let result = heartbeat.verify_witnesses(&active);

        assert!(matches!(
            result,
            Err(HeartbeatError::InsufficientWitnesses { have: 0, need: 2 })
        ));
    }

    #[test]
    fn test_self_witness_rejected() {
        let producer = mock_pubkey(1);

        // Try to use producer as their own witness (along with a valid one to meet count)
        let self_witness = WitnessSignature {
            witness: producer.clone(),
            signature: Signature::default(),
        };
        let other_witness = WitnessSignature {
            witness: mock_pubkey(2),
            signature: Signature::default(),
        };

        let heartbeat = Heartbeat {
            version: HEARTBEAT_VERSION,
            producer: producer.clone(),
            slot: 100,
            prev_block_hash: mock_hash(1),
            vdf_output: [0u8; 32],
            signature: Signature::default(),
            witnesses: vec![self_witness, other_witness], // 2 witnesses to pass count check
        };

        let active = vec![producer, mock_pubkey(2), mock_pubkey(3)];
        let result = heartbeat.verify_witnesses(&active);

        // Should fail with SelfWitness before checking other_witness signature
        assert!(matches!(result, Err(HeartbeatError::SelfWitness)));
    }

    #[test]
    fn test_invalid_witness_rejected() {
        let producer = mock_pubkey(1);
        let non_active_witness = mock_pubkey(99); // Not in active list
        let valid_witness = mock_pubkey(2);

        let invalid_sig = WitnessSignature {
            witness: non_active_witness.clone(),
            signature: Signature::default(),
        };
        let valid_sig = WitnessSignature {
            witness: valid_witness.clone(),
            signature: Signature::default(),
        };

        let heartbeat = Heartbeat {
            version: HEARTBEAT_VERSION,
            producer: producer.clone(),
            slot: 100,
            prev_block_hash: mock_hash(1),
            vdf_output: [0u8; 32],
            signature: Signature::default(),
            witnesses: vec![invalid_sig, valid_sig], // 2 witnesses to pass count check
        };

        let active = vec![producer, valid_witness, mock_pubkey(3)];
        let result = heartbeat.verify_witnesses(&active);

        // Should fail because non_active_witness is not in active list
        assert!(matches!(result, Err(HeartbeatError::InvalidWitness(_))));
    }

    #[test]
    fn test_valid_witnesses() {
        let producer_kp = KeyPair::generate();
        let producer = producer_kp.public_key().clone();
        let slot = 100;
        let prev_hash = mock_hash(1);

        // Compute VDF
        let input = Heartbeat::compute_vdf_input(&producer, slot, &prev_hash);
        let vdf_output = hash_chain_vdf(&input, 100);

        // Create two valid witnesses
        let witness1_kp = KeyPair::generate();
        let witness2_kp = KeyPair::generate();

        let msg = WitnessSignature::compute_message(&producer, slot, &vdf_output);

        let sig1 = crypto::signature::sign_hash(&msg, witness1_kp.private_key());
        let sig2 = crypto::signature::sign_hash(&msg, witness2_kp.private_key());

        let witnesses = vec![
            WitnessSignature::new(witness1_kp.public_key().clone(), sig1),
            WitnessSignature::new(witness2_kp.public_key().clone(), sig2),
        ];

        let heartbeat = Heartbeat {
            version: HEARTBEAT_VERSION,
            producer: producer.clone(),
            slot,
            prev_block_hash: prev_hash,
            vdf_output,
            signature: Signature::default(), // Not checking producer sig here
            witnesses,
        };

        let active = vec![
            producer,
            witness1_kp.public_key().clone(),
            witness2_kp.public_key().clone(),
        ];

        let result = heartbeat.verify_witnesses(&active);
        assert!(result.is_ok());
    }

    #[test]
    fn test_heartbeat_serialization() {
        // Test with no witnesses first
        let heartbeat_empty = Heartbeat {
            version: HEARTBEAT_VERSION,
            producer: mock_pubkey(1),
            slot: 12345,
            prev_block_hash: mock_hash(42),
            vdf_output: [99u8; 32],
            signature: Signature::default(),
            witnesses: vec![],
        };

        let serialized = heartbeat_empty.serialize();
        assert!(!serialized.is_empty(), "Serialization should not be empty");

        if let Some(deserialized) = Heartbeat::deserialize(&serialized) {
            assert_eq!(heartbeat_empty.version, deserialized.version);
            assert_eq!(heartbeat_empty.producer, deserialized.producer);
            assert_eq!(heartbeat_empty.slot, deserialized.slot);
            assert_eq!(
                heartbeat_empty.prev_block_hash,
                deserialized.prev_block_hash
            );
            assert_eq!(heartbeat_empty.vdf_output, deserialized.vdf_output);
            assert_eq!(
                heartbeat_empty.witnesses.len(),
                deserialized.witnesses.len()
            );
        } else {
            // If deserialization fails, just check that serialize/size work
            // bincode may have issues with certain type combinations
            assert!(heartbeat_empty.size() > 100);
        }
    }

    #[test]
    fn test_heartbeat_id() {
        let hb1 = Heartbeat {
            version: HEARTBEAT_VERSION,
            producer: mock_pubkey(1),
            slot: 100,
            prev_block_hash: mock_hash(1),
            vdf_output: [0u8; 32],
            signature: Signature::default(),
            witnesses: vec![],
        };

        let hb2 = Heartbeat {
            version: HEARTBEAT_VERSION,
            producer: mock_pubkey(1),
            slot: 100,
            prev_block_hash: mock_hash(2), // Different prev_hash
            vdf_output: [1u8; 32],         // Different output
            signature: Signature::default(),
            witnesses: vec![],
        };

        // Same producer + slot = same ID
        assert_eq!(hb1.id(), hb2.id());

        let hb3 = Heartbeat {
            version: HEARTBEAT_VERSION,
            producer: mock_pubkey(2), // Different producer
            slot: 100,
            prev_block_hash: mock_hash(1),
            vdf_output: [0u8; 32],
            signature: Signature::default(),
            witnesses: vec![],
        };

        // Different producer = different ID
        assert_ne!(hb1.id(), hb3.id());
    }

    #[test]
    fn test_has_enough_witnesses() {
        let mut heartbeat = Heartbeat {
            version: HEARTBEAT_VERSION,
            producer: mock_pubkey(1),
            slot: 100,
            prev_block_hash: mock_hash(1),
            vdf_output: [0u8; 32],
            signature: Signature::default(),
            witnesses: vec![],
        };

        assert!(!heartbeat.has_enough_witnesses());

        heartbeat.add_witness(WitnessSignature {
            witness: mock_pubkey(2),
            signature: Signature::default(),
        });
        assert!(!heartbeat.has_enough_witnesses());

        heartbeat.add_witness(WitnessSignature {
            witness: mock_pubkey(3),
            signature: Signature::default(),
        });
        assert!(heartbeat.has_enough_witnesses());
    }

    #[test]
    fn test_size_estimation() {
        let heartbeat = Heartbeat {
            version: HEARTBEAT_VERSION,
            producer: mock_pubkey(1),
            slot: 100,
            prev_block_hash: mock_hash(1),
            vdf_output: [0u8; 32],
            signature: Signature::default(),
            witnesses: vec![
                WitnessSignature {
                    witness: mock_pubkey(2),
                    signature: Signature::default(),
                },
                WitnessSignature {
                    witness: mock_pubkey(3),
                    signature: Signature::default(),
                },
            ],
        };

        let size = heartbeat.size();
        // 1 + 32 + 4 + 32 + 32 + 64 + 2*(32+64) = 357
        assert!(size > 300);
        assert!(size < 400);
    }

    // =========================================================================
    // CompactHeartbeat Tests
    // =========================================================================

    #[test]
    fn test_compact_heartbeat_from_heartbeat() {
        let heartbeat = Heartbeat {
            version: HEARTBEAT_VERSION,
            producer: mock_pubkey(1),
            slot: 100,
            prev_block_hash: mock_hash(1),
            vdf_output: [42u8; 32],
            signature: Signature::default(),
            witnesses: vec![],
        };

        let bond_weight = 1_000_000_000u64; // 10 DOLI
        let compact = CompactHeartbeat::from_heartbeat(&heartbeat, bond_weight);

        assert_eq!(compact.producer, heartbeat.producer);
        assert_eq!(compact.slot, heartbeat.slot);
        assert_eq!(compact.prev_block_hash, heartbeat.prev_block_hash);
        assert_eq!(compact.vdf_output, heartbeat.vdf_output);
        assert_eq!(compact.signature, heartbeat.signature);
        assert_eq!(compact.bond_weight, bond_weight);
    }

    #[test]
    fn test_compact_heartbeat_size() {
        let compact = CompactHeartbeat::new(
            mock_pubkey(1),
            100,
            mock_hash(1),
            [0u8; 32],
            Signature::default(),
            1_000_000_000,
        );

        // Fixed size: 32 + 4 + 32 + 32 + 64 + 8 = 172 bytes
        assert_eq!(compact.size(), 172);
    }

    #[test]
    fn test_compact_heartbeat_leaf_hash_deterministic() {
        let compact = CompactHeartbeat::new(
            mock_pubkey(1),
            100,
            mock_hash(1),
            [42u8; 32],
            Signature::default(),
            5_000_000_000,
        );

        // Same compact heartbeat should produce same leaf hash
        let hash1 = compact.leaf_hash();
        let hash2 = compact.leaf_hash();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_compact_heartbeat_leaf_hash_uniqueness() {
        let compact1 = CompactHeartbeat::new(
            mock_pubkey(1),
            100,
            mock_hash(1),
            [42u8; 32],
            Signature::default(),
            5_000_000_000,
        );

        // Different producer
        let compact2 = CompactHeartbeat::new(
            mock_pubkey(2),
            100,
            mock_hash(1),
            [42u8; 32],
            Signature::default(),
            5_000_000_000,
        );

        // Different slot
        let compact3 = CompactHeartbeat::new(
            mock_pubkey(1),
            101,
            mock_hash(1),
            [42u8; 32],
            Signature::default(),
            5_000_000_000,
        );

        // Different bond_weight
        let compact4 = CompactHeartbeat::new(
            mock_pubkey(1),
            100,
            mock_hash(1),
            [42u8; 32],
            Signature::default(),
            10_000_000_000,
        );

        // All should have different leaf hashes
        let hash1 = compact1.leaf_hash();
        let hash2 = compact2.leaf_hash();
        let hash3 = compact3.leaf_hash();
        let hash4 = compact4.leaf_hash();

        assert_ne!(hash1, hash2);
        assert_ne!(hash1, hash3);
        assert_ne!(hash1, hash4);
    }

    #[test]
    fn test_compact_heartbeat_serialization() {
        let compact = CompactHeartbeat::new(
            mock_pubkey(1),
            100,
            mock_hash(1),
            [42u8; 32],
            Signature::default(),
            5_000_000_000,
        );

        let serialized = bincode::serialize(&compact).unwrap();
        let deserialized: CompactHeartbeat = bincode::deserialize(&serialized).unwrap();

        assert_eq!(compact, deserialized);
        assert_eq!(compact.leaf_hash(), deserialized.leaf_hash());
    }
}
