//! VDF computation and verification
//!
//! This module implements the Wesolowski VDF (Verifiable Delay Function)
//! over imaginary quadratic class groups. The construction provides:
//!
//! - **Sequential computation**: Computing y = x^(2^t) requires t sequential squarings
//! - **Efficient verification**: Verifying the proof requires only O(log t) operations
//! - **Compact proofs**: The proof is a single group element
//!
//! ## The Wesolowski Protocol
//!
//! Given input x and time parameter t:
//!
//! 1. **Compute**: y = x^(2^t) by repeated squaring
//! 2. **Prove**: Generate proof π = x^(floor(2^t / l)) where l is a Fiat-Shamir challenge
//! 3. **Verify**: Check that y = π^l · x^r where r = 2^t mod l
//!
//! The verification equation holds because:
//! π^l · x^r = x^(l·floor(2^t/l)) · x^r = x^(l·floor(2^t/l) + r) = x^(2^t) = y

use rug::integer::Order;
use rug::Integer;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::class_group::{div_2pow_by_l, generate_discriminant, pow2_mod, ClassGroupElement};
use crate::proof::VdfProof;
use crate::DISCRIMINANT_BITS;
use crypto::Hash;

/// Minimum challenge prime bit size for security.
const CHALLENGE_BITS: usize = 128;

/// Errors in VDF operations
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum VdfError {
    /// The input hash is invalid
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// The proof format is invalid
    #[error("invalid proof: {0}")]
    InvalidProof(String),

    /// Verification failed (proof doesn't match output)
    #[error("verification failed: computed output doesn't match claimed output")]
    VerificationFailed,

    /// Internal computation error
    #[error("computation error: {0}")]
    ComputationError(String),

    /// The time parameter is invalid
    #[error("invalid time parameter: {0}")]
    InvalidTimeParameter(String),
}

/// VDF computation parameters
#[derive(Clone, Debug)]
pub struct VdfParams {
    /// The discriminant for the class group
    pub discriminant: Integer,
    /// Number of bits in the discriminant
    pub discriminant_bits: usize,
}

impl VdfParams {
    /// Create default parameters with the standard DOLI discriminant.
    ///
    /// The discriminant is generated deterministically from a fixed seed
    /// to ensure all nodes use the same class group.
    #[must_use]
    pub fn default_params() -> Self {
        let discriminant = generate_discriminant(DISCRIMINANT_BITS, b"DOLI_VDF_DISCRIMINANT_V1");

        Self {
            discriminant,
            discriminant_bits: DISCRIMINANT_BITS,
        }
    }

    /// Create parameters with a custom discriminant.
    ///
    /// # Arguments
    /// * `bits` - Number of bits for the discriminant
    /// * `seed` - Seed for deterministic generation
    #[must_use]
    pub fn with_seed(bits: usize, seed: &[u8]) -> Self {
        let discriminant = generate_discriminant(bits, seed);

        Self {
            discriminant,
            discriminant_bits: bits,
        }
    }
}

impl Default for VdfParams {
    fn default() -> Self {
        Self::default_params()
    }
}

/// VDF output (the result of the computation).
///
/// This is the group element y = x^(2^t).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VdfOutput {
    /// The serialized output group element
    pub value: Vec<u8>,
}

impl VdfOutput {
    /// Create a new VDF output from a class group element.
    #[must_use]
    pub fn new(element: &ClassGroupElement) -> Self {
        Self {
            value: element.to_bytes(),
        }
    }

    /// Get the size of the output in bytes.
    #[must_use]
    pub fn size(&self) -> usize {
        self.value.len()
    }

    /// Serialize to bytes (length-prefixed).
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(4 + self.value.len());
        bytes.extend_from_slice(&(self.value.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&self.value);
        bytes
    }

    /// Deserialize from bytes.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 4 {
            return None;
        }

        let len = u32::from_le_bytes(bytes[0..4].try_into().ok()?) as usize;
        if bytes.len() < 4 + len {
            return None;
        }

        Some(Self {
            value: bytes[4..4 + len].to_vec(),
        })
    }

    /// Convert to hex string.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(&self.value)
    }

    /// Create from hex string.
    pub fn from_hex(s: &str) -> Result<Self, hex::FromHexError> {
        Ok(Self {
            value: hex::decode(s)?,
        })
    }
}

/// Compute VDF: y = x^(2^t) in the class group.
///
/// This is the prover's computation. It requires t sequential squarings
/// and cannot be parallelized (this is the VDF's core security property).
///
/// # Arguments
/// * `input` - The 32-byte input hash
/// * `t` - The difficulty parameter (number of squarings)
///
/// # Returns
/// The VDF output and Wesolowski proof, or an error.
///
/// # Performance
/// - Computation time is O(t) group operations
/// - Proof generation adds O(t) additional work
/// - Memory usage is O(1) for the main computation
pub fn compute(input: &Hash, t: u64) -> Result<(VdfOutput, VdfProof), VdfError> {
    if t == 0 {
        return Err(VdfError::InvalidTimeParameter(
            "t must be positive".to_string(),
        ));
    }

    let params = VdfParams::default_params();

    // Convert input hash to initial group element
    let x = ClassGroupElement::from_hash(input.as_bytes(), &params.discriminant);

    // Compute y = x^(2^t) by repeated squaring
    // This is inherently sequential - the core VDF property
    let y = compute_repeated_squaring(&x, t);

    // Compute Wesolowski proof
    let proof = compute_wesolowski_proof(&x, &y, t, &params)?;

    Ok((VdfOutput::new(&y), proof))
}

/// Compute y = x^(2^t) by repeated squaring.
fn compute_repeated_squaring(x: &ClassGroupElement, t: u64) -> ClassGroupElement {
    let mut y = x.clone();
    for _ in 0..t {
        y = y.square();
    }
    y
}

/// Compute the Wesolowski proof.
///
/// The proof is π = x^q where q = floor(2^t / l) and l is a prime challenge.
///
/// This allows the verifier to check y = π^l · x^r where r = 2^t mod l,
/// which holds because l*q + r = 2^t, so π^l · x^r = x^(l*q) · x^r = x^(2^t) = y.
fn compute_wesolowski_proof(
    x: &ClassGroupElement,
    y: &ClassGroupElement,
    t: u64,
    params: &VdfParams,
) -> Result<VdfProof, VdfError> {
    // Generate challenge prime l using Fiat-Shamir
    let l = generate_challenge_prime(x, y, t, params);

    // Compute quotient q = floor(2^t / l)
    let q = div_2pow_by_l(t, &l);

    // Compute π = x^q
    // This is the expensive part of proof generation
    let pi = x.pow(&q);

    Ok(VdfProof::new(&pi))
}

/// Generate a prime challenge using Fiat-Shamir heuristic.
///
/// The challenge is derived deterministically from the input x, output y,
/// and time parameter t. This makes the proof non-interactive.
///
/// We use hash-and-increment to find a prime:
/// 1. Hash the inputs to get a seed
/// 2. Increment until we find a probable prime
fn generate_challenge_prime(
    x: &ClassGroupElement,
    y: &ClassGroupElement,
    t: u64,
    _params: &VdfParams,
) -> Integer {
    use crypto::Hasher;

    // Create deterministic seed from inputs
    let mut hasher = Hasher::new();
    hasher.update(b"DOLI_VDF_CHALLENGE_V1");
    hasher.update(&x.to_bytes());
    hasher.update(&y.to_bytes());
    hasher.update(&t.to_le_bytes());
    let seed = hasher.finalize();

    // Expand to CHALLENGE_BITS
    let mut expanded = Vec::with_capacity((CHALLENGE_BITS + 7) / 8);
    let mut counter = 0u64;

    while expanded.len() * 8 < CHALLENGE_BITS {
        let mut h = Hasher::new();
        h.update(b"DOLI_CHALLENGE_EXPAND_V1");
        h.update(seed.as_bytes());
        h.update(&counter.to_le_bytes());
        let hash = h.finalize();
        expanded.extend_from_slice(hash.as_bytes());
        counter += 1;
    }

    expanded.truncate((CHALLENGE_BITS + 7) / 8);

    // Set high bit and low bit (ensure odd and sufficient size)
    if let Some(first) = expanded.first_mut() {
        *first |= 0x80;
    }
    if let Some(last) = expanded.last_mut() {
        *last |= 0x01;
    }

    let mut candidate = Integer::from_digits(&expanded, Order::MsfBe);

    // Find next probable prime using rug's built-in primality test (much faster)
    candidate.next_prime_mut();

    candidate
}

// Note: With rug, we use the built-in next_prime_mut() which is much faster
// than our own Miller-Rabin implementation because GMP uses optimized algorithms.

/// Verify a VDF proof.
///
/// This verifies that the prover correctly computed y = x^(2^t).
///
/// # Arguments
/// * `input` - The original input hash
/// * `output` - The claimed VDF output y
/// * `proof` - The Wesolowski proof π
/// * `t` - The difficulty parameter
///
/// # Returns
/// `Ok(())` if the proof is valid, `Err` otherwise.
///
/// # Verification Equation
/// We check: y == π^l · x^r where:
/// - l is the challenge prime (derived via Fiat-Shamir)
/// - r = 2^t mod l
///
/// This works because 2^t = l·floor(2^t/l) + r, so:
/// π^l · x^r = x^(l·floor(2^t/l)) · x^r = x^(2^t) = y
pub fn verify(input: &Hash, output: &VdfOutput, proof: &VdfProof, t: u64) -> Result<(), VdfError> {
    if t == 0 {
        return Err(VdfError::InvalidTimeParameter(
            "t must be positive".to_string(),
        ));
    }

    let params = VdfParams::default_params();

    // Reconstruct group elements
    let x = ClassGroupElement::from_hash(input.as_bytes(), &params.discriminant);

    let y = ClassGroupElement::from_bytes(&output.value, &params.discriminant)
        .map_err(|e| VdfError::InvalidProof(format!("invalid output: {e}")))?;

    let pi = ClassGroupElement::from_bytes(&proof.pi, &params.discriminant)
        .map_err(|e| VdfError::InvalidProof(format!("invalid proof: {e}")))?;

    // Verify discriminants match
    if x.discriminant() != y.discriminant() || x.discriminant() != pi.discriminant() {
        return Err(VdfError::InvalidProof("discriminant mismatch".to_string()));
    }

    // Generate challenge prime (same as prover)
    let l = generate_challenge_prime(&x, &y, t, &params);

    // Compute r = 2^t mod l
    let r = pow2_mod(t, &l);

    // Verify: y == π^l · x^r
    // Compute left side: π^l
    let pi_l = pi.pow(&l);

    // Compute right side: x^r
    let x_r = x.pow(&r);

    // Combine: π^l · x^r
    let computed_y = pi_l.compose(&x_r);

    // Compare with claimed y
    if computed_y != y {
        return Err(VdfError::VerificationFailed);
    }

    Ok(())
}

/// Compute VDF with custom parameters.
///
/// Use this for testing or when non-standard parameters are needed.
pub fn compute_with_params(
    input: &Hash,
    t: u64,
    params: &VdfParams,
) -> Result<(VdfOutput, VdfProof), VdfError> {
    if t == 0 {
        return Err(VdfError::InvalidTimeParameter(
            "t must be positive".to_string(),
        ));
    }

    let x = ClassGroupElement::from_hash(input.as_bytes(), &params.discriminant);
    let y = compute_repeated_squaring(&x, t);
    let proof = compute_wesolowski_proof(&x, &y, t, params)?;

    Ok((VdfOutput::new(&y), proof))
}

/// Verify VDF with custom parameters.
pub fn verify_with_params(
    input: &Hash,
    output: &VdfOutput,
    proof: &VdfProof,
    t: u64,
    params: &VdfParams,
) -> Result<(), VdfError> {
    if t == 0 {
        return Err(VdfError::InvalidTimeParameter(
            "t must be positive".to_string(),
        ));
    }

    let x = ClassGroupElement::from_hash(input.as_bytes(), &params.discriminant);

    let y = ClassGroupElement::from_bytes(&output.value, &params.discriminant)
        .map_err(|e| VdfError::InvalidProof(format!("invalid output: {e}")))?;

    let pi = ClassGroupElement::from_bytes(&proof.pi, &params.discriminant)
        .map_err(|e| VdfError::InvalidProof(format!("invalid proof: {e}")))?;

    let l = generate_challenge_prime(&x, &y, t, params);
    let r = pow2_mod(t, &l);

    let pi_l = pi.pow(&l);
    let x_r = x.pow(&r);
    let computed_y = pi_l.compose(&x_r);

    if computed_y != y {
        return Err(VdfError::VerificationFailed);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::hash::hash;

    /// Test params with smaller discriminant for faster tests
    fn test_params() -> VdfParams {
        VdfParams::with_seed(256, b"test_discriminant")
    }

    #[test]
    fn test_vdf_params() {
        let params = VdfParams::default_params();
        assert_eq!(params.discriminant_bits, DISCRIMINANT_BITS);

        // Discriminant should be negative
        assert!(params.discriminant < 0);
    }

    #[test]
    fn test_vdf_output_serialization() {
        let output = VdfOutput {
            value: vec![1, 2, 3, 4],
        };

        let bytes = output.to_bytes();
        let recovered = VdfOutput::from_bytes(&bytes).expect("should deserialize");

        assert_eq!(output, recovered);
    }

    #[test]
    fn test_vdf_output_hex() {
        let output = VdfOutput {
            value: vec![0xde, 0xad, 0xbe, 0xef],
        };

        let hex = output.to_hex();
        assert_eq!(hex, "deadbeef");

        let recovered = VdfOutput::from_hex(&hex).expect("should parse hex");
        assert_eq!(output, recovered);
    }

    #[test]
    fn test_vdf_small() {
        // Test with very small t and small discriminant for speed
        let params = test_params();
        let input = hash(b"test input");
        let t = 10;

        let (output, proof) = compute_with_params(&input, t, &params).expect("should compute");
        assert!(!output.value.is_empty());
        assert!(!proof.pi.is_empty());

        // Verify the proof
        verify_with_params(&input, &output, &proof, t, &params).expect("should verify");
    }

    #[test]
    fn test_vdf_deterministic() {
        let params = test_params();
        let input = hash(b"deterministic test");
        let t = 5;

        let (output1, proof1) = compute_with_params(&input, t, &params).expect("first compute");
        let (output2, proof2) = compute_with_params(&input, t, &params).expect("second compute");

        assert_eq!(output1, output2);
        assert_eq!(proof1, proof2);
    }

    #[test]
    fn test_vdf_different_inputs() {
        let params = test_params();
        let input1 = hash(b"input 1");
        let input2 = hash(b"input 2");
        let t = 5;

        let (output1, _) = compute_with_params(&input1, t, &params).expect("compute 1");
        let (output2, _) = compute_with_params(&input2, t, &params).expect("compute 2");

        assert_ne!(output1, output2);
    }

    #[test]
    fn test_vdf_verification_fails_wrong_output() {
        let params = test_params();
        let input = hash(b"test");
        let t = 5;

        let (output, proof) = compute_with_params(&input, t, &params).expect("compute");

        // Create wrong output by computing VDF on different input
        let different_input = hash(b"different");
        let (wrong_output, _) =
            compute_with_params(&different_input, t, &params).expect("compute different");

        // Make sure outputs are actually different
        assert_ne!(output.value, wrong_output.value);

        // Verification should fail
        let result = verify_with_params(&input, &wrong_output, &proof, t, &params);
        assert!(result.is_err());
    }

    #[test]
    fn test_vdf_verification_fails_wrong_input() {
        let params = test_params();
        let input = hash(b"test");
        let wrong_input = hash(b"wrong");
        let t = 5;

        let (output, proof) = compute_with_params(&input, t, &params).expect("compute");

        // Verification with wrong input should fail
        let result = verify_with_params(&wrong_input, &output, &proof, t, &params);
        assert!(result.is_err());
    }

    #[test]
    fn test_vdf_zero_t_rejected() {
        let params = test_params();
        let input = hash(b"test");
        let result = compute_with_params(&input, 0, &params);
        assert!(matches!(result, Err(VdfError::InvalidTimeParameter(_))));
    }

    #[test]
    fn test_rug_primality() {
        // Test that rug's next_prime works correctly
        let mut n = Integer::from(2);
        assert!(n.is_probably_prime(25) != rug::integer::IsPrime::No);

        n = Integer::from(97);
        assert!(n.is_probably_prime(25) != rug::integer::IsPrime::No);

        n = Integer::from(100);
        assert!(n.is_probably_prime(25) == rug::integer::IsPrime::No);
    }

    #[test]
    fn test_vdf_with_custom_params() {
        let params = VdfParams::with_seed(512, b"test_params");
        let input = hash(b"custom params test");
        let t = 5;

        let (output, proof) = compute_with_params(&input, t, &params).expect("compute");
        verify_with_params(&input, &output, &proof, t, &params).expect("verify");
    }

    // Property-based tests with smaller discriminant for speed
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(10))]

        #[test]
        fn prop_vdf_always_verifies(seed: [u8; 32], t in 1u64..10u64) {
            let params = VdfParams::with_seed(256, b"prop_test");
            let input = hash(&seed);
            let (output, proof) = compute_with_params(&input, t, &params).expect("compute should succeed");
            verify_with_params(&input, &output, &proof, t, &params).expect("verify should succeed");
        }

        #[test]
        fn prop_vdf_output_changes_with_input(seed1: [u8; 32], seed2: [u8; 32]) {
            prop_assume!(seed1 != seed2);

            let params = VdfParams::with_seed(256, b"prop_test");
            let input1 = hash(&seed1);
            let input2 = hash(&seed2);
            let t = 5;

            let (output1, _) = compute_with_params(&input1, t, &params).expect("compute 1");
            let (output2, _) = compute_with_params(&input2, t, &params).expect("compute 2");

            prop_assert_ne!(output1, output2);
        }

        #[test]
        fn prop_vdf_output_changes_with_t(t_offset in 1u64..5u64) {
            // Use fixed input to avoid identity-like elements
            let params = VdfParams::with_seed(256, b"prop_test");
            let input = hash(b"fixed_input_for_t_test");

            // Use t1=1 and t2=1+offset to ensure different values
            let t1 = 1u64;
            let t2 = t1 + t_offset;

            let (output1, _) = compute_with_params(&input, t1, &params).expect("compute 1");
            let (output2, _) = compute_with_params(&input, t2, &params).expect("compute 2");

            prop_assert_ne!(output1, output2);
        }
    }
}
