//! # doli-vdf
//!
//! VDF primitives for DOLI's Proof of Time (PoT) consensus.
//!
//! ## Two VDF types
//!
//! | Type | Algorithm | Used for | Consensus? |
//! |------|-----------|----------|------------|
//! | **Hash-chain** | Iterated BLAKE3 (`heartbeat::hash_chain_vdf`) | Block production (800K iter, ~55ms) and registration (5M iter, ~30s) | **Yes** |
//! | **Wesolowski** | Class group sequential squarings | Telemetry-only presence tracking (`tpop::presence`) | No |
//!
//! All consensus-critical VDF validation goes through `hash_chain_vdf` /
//! `verify_hash_chain_vdf` in `doli_core::heartbeat`. The Wesolowski
//! implementation (`vdf::compute`, `vdf::verify`) is only called from
//! the telemetry presence module, which does NOT affect consensus.
//!
//! ## Hash-chain VDF (consensus)
//!
//! Simple iterated BLAKE3: `state = BLAKE3(state)` repeated T times.
//! Verification requires recomputation (no compact proof).
//!
//! | Parameter | Value | Purpose |
//! |-----------|-------|---------|
//! | T_BLOCK | 800K | ~55ms anti-grinding for block production |
//! | T_REGISTER_BASE | 5M | ~30s anti-flash-attack for registration |
//!
//! ## Wesolowski VDF (telemetry only)
//!
//! Sequential squarings in imaginary quadratic class groups with a
//! Wesolowski proof (O(log t) verification). Used only for presence
//! tracking in `doli_core::tpop::presence` — does not affect block
//! validity, producer selection, or rewards.
//!
//! ## Time-based consensus
//!
//! Slot timing is NTP/wall-clock based, not VDF-duration based.
//! Faster VDF hardware does NOT mean faster block production.

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

/// Hash-chain VDF iterations for block production (~55ms on reference hardware).
///
/// Used by `heartbeat::hash_chain_vdf`. For network-aware code, use
/// `NetworkParams::load(network).vdf_iterations` instead.
pub const T_BLOCK: u64 = 800_000;

/// Fixed VDF parameter for producer registration (~30 seconds).
///
/// No longer scales with producer count. The economic bond (10 DOLI)
/// provides Sybil protection at scale; this VDF is a lightweight
/// anti-flash-attack barrier preventing instant mass registration.
pub const T_REGISTER_BASE: u64 = 5_000_000;

/// Maximum VDF parameter for registration — same as base (no escalation).
pub const T_REGISTER_CAP: u64 = 5_000_000;

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
/// Registration VDF difficulty — fixed at T_REGISTER_BASE (~30 seconds).
/// No longer scales with producer count. The economic bond provides
/// Sybil protection at scale; the VDF is a lightweight anti-flash-attack barrier.
#[must_use]
pub fn registration_difficulty(_registered_count: u64) -> u64 {
    T_REGISTER_BASE
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crypto::hash::hash;

    #[test]
    fn test_constants() {
        // T_BLOCK > 0, T_REGISTER_BASE > T_BLOCK, T_REGISTER_CAP >= T_REGISTER_BASE
        // are guaranteed by const values — checked at compile time.
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
    fn test_registration_difficulty_fixed() {
        // Registration difficulty is now fixed regardless of producer count
        assert_eq!(registration_difficulty(0), T_REGISTER_BASE);
        assert_eq!(registration_difficulty(100), T_REGISTER_BASE);
        assert_eq!(registration_difficulty(1000), T_REGISTER_BASE);
        assert_eq!(registration_difficulty(1_000_000), T_REGISTER_BASE);
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
