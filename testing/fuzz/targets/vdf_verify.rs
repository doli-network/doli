//! Fuzz target for VDF verification
//!
//! Tests that VDF verify handles arbitrary inputs safely.

#![no_main]

use libfuzzer_sys::fuzz_target;
use arbitrary::Arbitrary;
use crypto::Hash;
use vdf::{VdfOutput, VdfProof, vdf};

#[derive(Debug, Arbitrary)]
struct FuzzInput {
    hash_bytes: [u8; 32],
    output_bytes: Vec<u8>,
    proof_bytes: Vec<u8>,
    t: u16, // Use smaller t values for fuzzing
}

fuzz_target!(|data: FuzzInput| {
    let input = Hash::from_bytes(data.hash_bytes);
    let output = VdfOutput { value: data.output_bytes };
    let proof = VdfProof { pi: data.proof_bytes };

    // Use small discriminant for faster fuzzing
    let params = vdf::VdfParams::with_seed(256, b"fuzz_test");

    // Try verification with arbitrary data - should not panic
    let _ = vdf::verify_with_params(
        &input,
        &output,
        &proof,
        data.t as u64,
        &params,
    );
});
