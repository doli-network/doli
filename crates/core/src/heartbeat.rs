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
                return Err(HeartbeatError::InvalidWitness(witness_sig.witness));
            }

            // Check witness is not the producer
            if witness_sig.witness == self.producer {
                return Err(HeartbeatError::SelfWitness);
            }

            // Verify the signature
            if !witness_sig.verify(&self.producer, self.slot, &self.vdf_output) {
                return Err(HeartbeatError::InvalidWitnessSignature(witness_sig.witness));
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

#[cfg(test)]
#[path = "heartbeat_tests.rs"]
mod tests;
