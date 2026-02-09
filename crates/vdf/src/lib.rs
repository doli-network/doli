//! # doli-vdf
//!
//! Production-grade Wesolowski Verifiable Delay Function over class groups.
//!
//! This crate provides the VDF computation and verification used in DOLI's
//! Proof of Time (PoT) consensus mechanism.
//!
//! ## What is a VDF?
//!
//! A Verifiable Delay Function (VDF) is a function that:
//!
//! 1. **Requires sequential time**: Computing y = f(x) takes T sequential steps
//!    that cannot be significantly parallelized
//! 2. **Is efficiently verifiable**: Given x, y, and a proof, anyone can verify
//!    the computation in time much less than T
//! 3. **Is deterministic**: The same input always produces the same output
//!
//! ## The Wesolowski Construction
//!
//! We implement the Wesolowski VDF which works as follows:
//!
//! **Setup**: Choose a class group with unknown group order (derived from a
//! large discriminant Δ)
//!
//! **Compute** (prover, time T):
//! 1. Map input hash to group element x
//! 2. Compute y = x^(2^t) by t sequential squarings
//! 3. Generate proof π = x^q where q = floor(2^t / l) and l is a prime challenge
//!
//! **Verify** (verifier, time O(log t)):
//! 1. Compute challenge l from (x, y, t) using Fiat-Shamir
//! 2. Compute r = 2^t mod l
//! 3. Check: y == π^l · x^r
//!
//! The verification equation holds because:
//! - 2^t = l·q + r (by definition)
//! - π^l · x^r = x^(l·q) · x^r = x^(l·q + r) = x^(2^t) = y
//!
//! ## Security
//!
//! The VDF security relies on:
//!
//! - **Unknown group order**: The discriminant is generated from a public seed,
//!   so no one knows the group order (computing it requires factoring)
//! - **Sequential squaring**: There's no known way to compute x^(2^t) faster
//!   than ~t squarings in a group of unknown order
//! - **Low-order assumption**: It's computationally infeasible to find non-trivial
//!   elements of low order in the class group
//!
//! ## Parameters
//!
//! | Parameter | Value | Purpose |
//! |-----------|-------|---------|
//! | Discriminant bits | 2048 | Security (infeasible to factor) |
//! | Challenge bits | 128 | Proof soundness |
//! | T_BLOCK | 10M | ~700ms anti-grinding VDF |
//! | T_REGISTER_BASE | 600M | ~10 minutes for registration (anti-Sybil) |
//!
//! ## Time-Based Consensus
//!
//! DOLI uses time-based slot selection (like Ethereum) rather than VDF-based timing.
//! Slot timing is determined by NTP/wall-clock time, not VDF duration.
//! This eliminates hardware advantage: faster VDF ≠ faster block production.
//!
//! The small block VDF prevents grinding attacks where producers try many
//! block variations to manipulate randomness. Registration VDF remains long
//! to provide anti-Sybil protection.
//!
//! ## Example Usage
//!
//! ```rust
//! use vdf::{compute, verify};
//! use crypto::hash::hash;
//!
//! // Create input from a hash
//! let input = hash(b"block data");
//!
//! // Compute VDF (t=10 for example; real usage would be much larger)
//! let t = 10;
//! let (output, proof) = compute(&input, t).expect("VDF computation failed");
//!
//! // Verify the proof
//! verify(&input, &output, &proof, t).expect("VDF verification failed");
//! ```
//!
//! ## Usage in DOLI
//!
//! DOLI uses VDFs for:
//!
//! 1. **Block production**: Producers must compute a VDF on the previous block hash
//!    to create a new block, ensuring minimum time between blocks
//!
//! 2. **Producer registration**: New producers must compute a VDF to register,
//!    preventing Sybil attacks
//!
//! 3. **Leader selection**: VDF outputs provide unpredictable randomness for
//!    selecting the next block producer

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::unwrap_used)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::similar_names)]
// Math-heavy crate: allow short variable names and some stylistic choices
#![allow(clippy::many_single_char_names)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::redundant_closure_for_method_calls)]
#![allow(clippy::bool_to_int_with_if)]
#![allow(clippy::assigning_clones)]
#![allow(clippy::manual_div_ceil)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::unnecessary_wraps)]

pub mod class_group;
pub mod proof;
pub mod vdf;

pub use proof::VdfProof;
pub use vdf::{VdfError, VdfOutput, VdfParams};

// Re-export class group types for advanced usage
pub use class_group::{ClassGroupElement, ClassGroupError};

use crypto::Hash;

/// VDF parameter for block production - MAINNET DEFAULT (~700ms on reference hardware).
///
/// **Note**: This is the mainnet default value. For network-aware code, use
/// `doli_core::network_params::NetworkParams::load(network).vdf_iterations` instead.
/// Devnet uses 1 iteration for fast testing.
///
/// # Time-Based Consensus Model
///
/// DOLI uses time-based consensus where slot timing is determined by real
/// time (NTP), not VDF duration. This eliminates hardware advantage:
/// faster VDF computation does NOT mean faster block production.
///
/// The block VDF serves as an anti-grinding mechanism to prevent
/// producers from:
/// 1. Trying many block variations to find favorable randomness
/// 2. Instantly producing blocks when their slot arrives
///
/// With 10-second slots and 2s sequential fallback windows, the VDF
/// takes ~55ms, leaving ~1945ms margin within each fallback window
/// (55ms VDF + ~600ms propagation = 655ms, margin = 1345ms).
///
/// This is calibrated so that:
/// - A single-threaded CPU takes ~55ms
/// - Even with ASIC speedup, still provides anti-grinding protection
/// - Does not create hardware-based competitive advantage
///
/// Registration VDF (T_REGISTER_BASE) remains long for anti-Sybil protection.
pub const T_BLOCK: u64 = 800_000;

/// Base VDF parameter for producer registration - MAINNET DEFAULT (~10 minutes).
///
/// **Note**: This is the mainnet default value. For network-aware code, use
/// `doli_core::network_params::NetworkParams::load(network).vdf_register_iterations` instead.
/// Devnet uses 5M iterations (~5 seconds) for fast testing.
///
/// Registration VDF difficulty scales with the number of registered producers
/// to maintain Sybil resistance as the network grows.
pub const T_REGISTER_BASE: u64 = 600_000_000;

/// Maximum VDF parameter for registration (~24 hours).
///
/// This caps the registration difficulty to prevent the network from
/// becoming completely closed to new participants.
pub const T_REGISTER_CAP: u64 = 86_400_000_000;

/// Size of the discriminant in bits.
///
/// 2048 bits provides approximately 112-bit security against the best known
/// factoring algorithms (similar to RSA-2048).
pub const DISCRIMINANT_BITS: usize = 2048;

/// Compute a VDF.
///
/// This is the main entry point for VDF computation.
///
/// # Arguments
/// * `input` - The 32-byte input hash
/// * `t` - The difficulty parameter (number of squarings)
///
/// # Returns
/// The VDF output and proof, or an error
///
/// # Example
/// ```rust
/// use vdf::compute;
/// use crypto::hash::hash;
///
/// let input = hash(b"test");
/// let (output, proof) = compute(&input, 10).expect("compute failed");
/// ```
#[inline]
pub fn compute(input: &Hash, t: u64) -> Result<(VdfOutput, VdfProof), VdfError> {
    vdf::compute(input, t)
}

/// Verify a VDF proof.
///
/// This is the main entry point for VDF verification.
///
/// # Arguments
/// * `input` - The original input hash
/// * `output` - The claimed VDF output
/// * `proof` - The Wesolowski proof
/// * `t` - The difficulty parameter
///
/// # Returns
/// `Ok(())` if the proof is valid, `Err` otherwise
///
/// # Example
/// ```rust
/// use vdf::{compute, verify};
/// use crypto::hash::hash;
///
/// let input = hash(b"test");
/// let (output, proof) = compute(&input, 10).expect("compute failed");
/// verify(&input, &output, &proof, 10).expect("verify failed");
/// ```
#[inline]
pub fn verify(input: &Hash, output: &VdfOutput, proof: &VdfProof, t: u64) -> Result<(), VdfError> {
    vdf::verify(input, output, proof, t)
}

/// Build a block VDF input from components.
///
/// This creates a deterministic input hash for block VDF computation,
/// combining:
/// - Previous block hash (chain continuity)
/// - Merkle root of transactions (block content commitment)
/// - Slot number (time ordering)
/// - Producer public key (identity binding)
///
/// # Domain Separation
///
/// The "BLK" prefix provides domain separation to prevent cross-protocol attacks.
#[must_use]
pub fn block_input(
    prev_hash: &Hash,
    merkle_root: &Hash,
    slot: u32,
    producer: &crypto::PublicKey,
) -> Hash {
    use crypto::Hasher;

    let mut hasher = Hasher::new();
    hasher.update(b"DOLI_VDF_BLOCK_V1");
    hasher.update(prev_hash.as_bytes());
    hasher.update(merkle_root.as_bytes());
    hasher.update(&slot.to_le_bytes());
    hasher.update(producer.as_bytes());
    hasher.finalize()
}

/// Build a registration VDF input from components.
///
/// This creates a deterministic input for producer registration VDF,
/// combining:
/// - Producer public key (identity)
/// - Epoch number (temporal binding)
///
/// # Domain Separation
///
/// The "REG" prefix provides domain separation.
#[must_use]
pub fn registration_input(public_key: &crypto::PublicKey, epoch: u32) -> Hash {
    use crypto::Hasher;

    let mut hasher = Hasher::new();
    hasher.update(b"DOLI_VDF_REGISTER_V1");
    hasher.update(public_key.as_bytes());
    hasher.update(&epoch.to_le_bytes());
    hasher.finalize()
}

/// Build a selection seed from components.
///
/// This creates a deterministic seed for leader selection,
/// derived from the previous block hash and slot number.
///
/// # Security
///
/// The selection seed is unpredictable because:
/// - It depends on the VDF output in the previous block
/// - VDF outputs cannot be predicted without computing the VDF
#[must_use]
pub fn selection_seed(prev_hash: &Hash, slot: u32) -> Hash {
    use crypto::Hasher;

    let mut hasher = Hasher::new();
    hasher.update(b"SEED");
    hasher.update(prev_hash.as_bytes());
    hasher.update(&slot.to_le_bytes());
    hasher.finalize()
}

/// Calculate registration VDF difficulty based on network size.
///
/// The difficulty scales with the number of registered producers to
/// maintain Sybil resistance. More producers = harder to register.
///
/// # Formula
///
/// t = min(T_REGISTER_CAP, T_REGISTER_BASE * (1 + registered_count / 100))
#[must_use]
pub fn registration_difficulty(registered_count: u64) -> u64 {
    let multiplier = 1 + registered_count / 100;
    let difficulty = T_REGISTER_BASE.saturating_mul(multiplier);
    std::cmp::min(difficulty, T_REGISTER_CAP)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::hash::hash;

    #[test]
    fn test_constants() {
        assert!(T_BLOCK > 0);
        assert!(T_REGISTER_BASE > T_BLOCK);
        assert!(T_REGISTER_CAP > T_REGISTER_BASE);
        assert_eq!(DISCRIMINANT_BITS, 2048);
    }

    #[test]
    fn test_block_input_format() {
        let prev_hash = Hash::ZERO;
        let merkle_root = Hash::ZERO;
        let slot = 0u32;
        let producer = crypto::PublicKey::from_bytes([0u8; 32]);

        let input = block_input(&prev_hash, &merkle_root, slot, &producer);

        // Verify it produces a non-zero hash
        assert!(!input.is_zero());
    }

    #[test]
    fn test_block_input_deterministic() {
        let prev_hash = hash(b"prev");
        let merkle_root = hash(b"merkle");
        let slot = 42u32;
        let producer = crypto::PublicKey::from_bytes([1u8; 32]);

        let input1 = block_input(&prev_hash, &merkle_root, slot, &producer);
        let input2 = block_input(&prev_hash, &merkle_root, slot, &producer);

        assert_eq!(input1, input2);
    }

    #[test]
    fn test_block_input_changes() {
        let prev_hash = hash(b"prev");
        let merkle_root = hash(b"merkle");
        let producer = crypto::PublicKey::from_bytes([1u8; 32]);

        let input1 = block_input(&prev_hash, &merkle_root, 0, &producer);
        let input2 = block_input(&prev_hash, &merkle_root, 1, &producer);

        assert_ne!(input1, input2);
    }

    #[test]
    fn test_selection_seed_deterministic() {
        let prev_hash = Hash::ZERO;
        let slot = 0u32;

        let seed1 = selection_seed(&prev_hash, slot);
        let seed2 = selection_seed(&prev_hash, slot);

        assert_eq!(seed1, seed2);
    }

    #[test]
    fn test_selection_seed_vector() {
        // Test vector for slot 0
        let prev_hash = Hash::ZERO;
        let slot = 0u32;

        let seed = selection_seed(&prev_hash, slot);

        assert_eq!(
            seed.to_hex(),
            "f3b4b63bfa289f7b4b2f11f08cfc26bd38ccdbdd9dae33ef9b77c1fc3b96ebb2"
        );
    }

    #[test]
    fn test_selection_seed_slot_1() {
        // Test vector for slot 1 (endianness check)
        let prev_hash = Hash::ZERO;
        let slot = 1u32;

        let seed = selection_seed(&prev_hash, slot);

        assert_eq!(
            seed.to_hex(),
            "ac1d2a15e55cc413c69036ba29cd08066a560a5bf152ac89a35089eae1fd6bbe"
        );
    }

    #[test]
    fn test_registration_input_deterministic() {
        let public_key = crypto::PublicKey::from_bytes([42u8; 32]);
        let epoch = 1u32;

        let input1 = registration_input(&public_key, epoch);
        let input2 = registration_input(&public_key, epoch);

        assert_eq!(input1, input2);
    }

    #[test]
    fn test_registration_difficulty_scaling() {
        // Base case
        assert_eq!(registration_difficulty(0), T_REGISTER_BASE);

        // With 100 producers
        assert_eq!(registration_difficulty(100), T_REGISTER_BASE * 2);

        // With 1000 producers
        assert_eq!(registration_difficulty(1000), T_REGISTER_BASE * 11);

        // Should cap at T_REGISTER_CAP
        let large_count = 1_000_000_000;
        assert_eq!(registration_difficulty(large_count), T_REGISTER_CAP);
    }

    #[test]
    fn test_compute_and_verify() {
        // Use smaller discriminant for faster test
        let params = vdf::VdfParams::with_seed(256, b"integration_test");
        let input = hash(b"integration test");
        let t = 5;

        let (output, proof) =
            vdf::compute_with_params(&input, t, &params).expect("compute should succeed");
        vdf::verify_with_params(&input, &output, &proof, t, &params)
            .expect("verify should succeed");
    }

    #[test]
    fn test_verification_fails_with_wrong_t() {
        // Use smaller discriminant for faster test
        let params = vdf::VdfParams::with_seed(256, b"wrong_t_test");
        let input = hash(b"test");
        let t = 5;

        let (output, proof) = vdf::compute_with_params(&input, t, &params).expect("compute");

        // Different t should fail
        let result = vdf::verify_with_params(&input, &output, &proof, t + 1, &params);
        assert!(result.is_err());
    }
}
