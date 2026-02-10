//! Integration tests for DOLI's VDF system.
//!
//! DOLI uses TWO VDF types:
//!   - **Hash-chain VDF** (iterated SHA3): Block production (~55ms at 800K iterations)
//!   - **Wesolowski VDF** (class groups): Producer registration (anti-Sybil, ~10min)
//!
//! The hash-chain VDF is the performance-critical path tested here.
//! Wesolowski correctness is covered by the crate's unit tests (256-bit, small t).
//!
//! Run with: cargo test -p vdf --test wesolowski_bench --release -- --nocapture

use std::time::Instant;

use crypto::hash::hash;
use vdf::vdf::{compute_with_params, verify_with_params, VdfParams};

/// 256-bit discriminant for fast Wesolowski tests.
fn test_params() -> VdfParams {
    VdfParams::with_seed(256, b"integration_test_v1")
}

// =========================================================================
// Hash-Chain VDF (Block Production) — the ~55ms target
// =========================================================================

#[test]
fn hc_01_timing_800k() {
    use doli_core::heartbeat::{hash_chain_vdf, verify_hash_chain_vdf};

    let iterations: u64 = 800_000;
    let input = crypto::Hash::from_bytes(*hash(b"block vdf benchmark").as_bytes());

    let start = Instant::now();
    let output = hash_chain_vdf(&input, iterations);
    let compute_ms = start.elapsed().as_millis();

    let start = Instant::now();
    let valid = verify_hash_chain_vdf(&input, &output, iterations);
    let verify_ms = start.elapsed().as_millis();

    println!("--- Hash-Chain VDF Timing (800K iterations) ---");
    println!("  Compute: {compute_ms}ms (target: ~55ms)");
    println!("  Verify:  {verify_ms}ms");

    assert!(valid, "Verification must pass");
    assert!(
        compute_ms < 500,
        "Hash-chain VDF too slow: {compute_ms}ms (target ~55ms, max 500ms)"
    );
}

#[test]
fn hc_02_deterministic() {
    use doli_core::heartbeat::hash_chain_vdf;

    let input = crypto::Hash::from_bytes(*hash(b"deterministic").as_bytes());
    let out1 = hash_chain_vdf(&input, 10_000);
    let out2 = hash_chain_vdf(&input, 10_000);
    assert_eq!(out1, out2);
}

#[test]
fn hc_03_rejects_wrong_iterations() {
    use doli_core::heartbeat::{hash_chain_vdf, verify_hash_chain_vdf};

    let input = crypto::Hash::from_bytes(*hash(b"wrong iterations").as_bytes());
    let output = hash_chain_vdf(&input, 1000);

    assert!(!verify_hash_chain_vdf(&input, &output, 999));
    assert!(!verify_hash_chain_vdf(&input, &output, 1001));
}

#[test]
fn hc_04_input_sensitivity() {
    use doli_core::heartbeat::hash_chain_vdf;

    let out_a = hash_chain_vdf(
        &crypto::Hash::from_bytes(*hash(b"input A").as_bytes()),
        1000,
    );
    let out_b = hash_chain_vdf(
        &crypto::Hash::from_bytes(*hash(b"input B").as_bytes()),
        1000,
    );
    assert_ne!(
        out_a, out_b,
        "Different inputs must produce different outputs"
    );
}

#[test]
fn hc_05_anti_grinding() {
    use doli_core::heartbeat::hash_chain_vdf;

    // Simulates grinding: changing one transaction changes VDF output
    let base_input = hash(b"prev_hash || merkle_A || slot || producer");
    let grind_input = hash(b"prev_hash || merkle_B || slot || producer");

    let base_out = hash_chain_vdf(&crypto::Hash::from_bytes(*base_input.as_bytes()), 800_000);
    let grind_out = hash_chain_vdf(&crypto::Hash::from_bytes(*grind_input.as_bytes()), 800_000);

    assert_ne!(
        base_out, grind_out,
        "Changing block contents must change VDF output (anti-grinding)"
    );
}

// =========================================================================
// Wesolowski VDF (Class Groups) — correctness at small parameters
// =========================================================================
//
// These use 256-bit discriminant and small t for speed.
// Full Wesolowski tests are in the crate's unit tests.

#[test]
fn wes_01_compute_and_verify() {
    let params = test_params();
    let input = hash(b"wesolowski correctness");
    let t = 10;

    let (output, proof) = compute_with_params(&input, t, &params).expect("compute");
    assert!(!output.value.is_empty());
    assert!(!proof.pi.is_empty());

    verify_with_params(&input, &output, &proof, t, &params).expect("verify");
}

#[test]
fn wes_02_deterministic() {
    let params = test_params();
    let input = hash(b"deterministic");
    let t = 5;

    let (out1, prf1) = compute_with_params(&input, t, &params).expect("1");
    let (out2, prf2) = compute_with_params(&input, t, &params).expect("2");

    assert_eq!(out1, out2, "Output must be deterministic");
    assert_eq!(prf1, prf2, "Proof must be deterministic");
}

#[test]
fn wes_03_input_sensitivity() {
    let params = test_params();
    let t = 5;

    let (out_a, _) = compute_with_params(&hash(b"A"), t, &params).expect("A");
    let (out_b, _) = compute_with_params(&hash(b"B"), t, &params).expect("B");

    assert_ne!(out_a, out_b);
}

#[test]
fn wes_04_rejects_wrong_input() {
    let params = test_params();
    let t = 10;

    let (output, proof) = compute_with_params(&hash(b"real"), t, &params).expect("compute");
    assert!(verify_with_params(&hash(b"fake"), &output, &proof, t, &params).is_err());
}

#[test]
fn wes_05_rejects_wrong_output() {
    let params = test_params();
    let t = 10;

    let (_, proof) = compute_with_params(&hash(b"input"), t, &params).expect("compute");
    let (wrong_out, _) = compute_with_params(&hash(b"other"), t, &params).expect("other");

    assert!(verify_with_params(&hash(b"input"), &wrong_out, &proof, t, &params).is_err());
}

#[test]
fn wes_06_rejects_wrong_t() {
    let params = test_params();
    let t = 10;

    let (output, proof) = compute_with_params(&hash(b"t test"), t, &params).expect("compute");

    assert!(verify_with_params(&hash(b"t test"), &output, &proof, t + 1, &params).is_err());
}

#[test]
fn wes_07_rejects_zero_t() {
    let params = test_params();
    assert!(compute_with_params(&hash(b"zero"), 0, &params).is_err());
}

// ===== Serialization =====

#[test]
fn wes_08_output_serialization() {
    let params = test_params();
    let (output, _) = compute_with_params(&hash(b"serial"), 5, &params).expect("compute");

    let bytes = output.to_bytes();
    let recovered = vdf::VdfOutput::from_bytes(&bytes).expect("deserialize");
    assert_eq!(output, recovered);

    let hex_str = output.to_hex();
    let from_hex = vdf::VdfOutput::from_hex(&hex_str).expect("hex");
    assert_eq!(output, from_hex);
}

#[test]
fn wes_09_proof_serialization() {
    let params = test_params();
    let (_, proof) = compute_with_params(&hash(b"proof serial"), 5, &params).expect("compute");

    let bytes = proof.to_bytes();
    let recovered = vdf::VdfProof::from_bytes(&bytes).expect("deserialize");
    assert_eq!(proof.pi, recovered.pi);

    let hex_str = proof.to_hex();
    let from_hex = vdf::VdfProof::from_hex(&hex_str).expect("hex");
    assert_eq!(proof.pi, from_hex.pi);
}

// ===== Discriminant & Domain Separation =====

#[test]
fn wes_10_discriminant_properties() {
    let p1 = VdfParams::default_params();
    let p2 = VdfParams::default_params();
    assert_eq!(p1.discriminant, p2.discriminant, "Must be deterministic");
    assert_eq!(p1.discriminant_bits, 2048);
    assert!(p1.discriminant < rug::Integer::ZERO, "Must be negative");

    let a = VdfParams::with_seed(256, b"seed_A");
    let b = VdfParams::with_seed(256, b"seed_B");
    assert_ne!(
        a.discriminant, b.discriminant,
        "Different seeds must differ"
    );
}

#[test]
fn wes_11_domain_separation() {
    let prev_hash = hash(b"prev");
    let merkle_root = hash(b"merkle");
    let slot = 42u32;
    let producer = crypto::PublicKey::from_bytes([1u8; 32]);

    let block_in = vdf::block_input(&prev_hash, &merkle_root, slot, &producer);
    let reg_in = vdf::registration_input(&producer, slot);

    assert_ne!(block_in, reg_in, "Block and registration must differ");
}
