//! Fuzz target for signature verification
//!
//! Tests that signature::verify handles arbitrary inputs safely.

#![no_main]

use libfuzzer_sys::fuzz_target;
use arbitrary::Arbitrary;
use crypto::{signature, PublicKey, Signature};

#[derive(Debug, Arbitrary)]
struct FuzzInput {
    message: Vec<u8>,
    pubkey_bytes: [u8; 32],
    signature_bytes: [u8; 64],
}

fuzz_target!(|data: FuzzInput| {
    let pubkey = PublicKey::from_bytes(data.pubkey_bytes);
    let sig = Signature::from_bytes(data.signature_bytes);

    // Verify arbitrary signature - should not panic
    let _ = signature::verify(&data.message, &sig, &pubkey);

    // Also test with empty message
    let _ = signature::verify(&[], &sig, &pubkey);

    // Test with large message
    if data.message.len() > 0 {
        let large_message: Vec<u8> = data.message.iter().cycle().take(10000).copied().collect();
        let _ = signature::verify(&large_message, &sig, &pubkey);
    }
});
