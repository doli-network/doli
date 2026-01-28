//! Fuzz target for hashing functions
//!
//! Tests that hash functions handle arbitrary inputs safely.

#![no_main]

use libfuzzer_sys::fuzz_target;
use crypto::{hash, Hash, Hasher};

fuzz_target!(|data: &[u8]| {
    // Test basic hashing
    let h1 = hash::hash(data);
    let h2 = hash::hash(data);

    // Hash should be deterministic
    assert_eq!(h1, h2);

    // Hash should be non-zero for non-empty data (with very high probability)
    // Note: theoretically a collision with zero is possible but astronomically unlikely

    // Test Hasher API
    let mut hasher = Hasher::new();
    hasher.update(data);
    let h3 = hasher.finalize();

    // Single update should match direct hash
    assert_eq!(h1, h3);

    // Test incremental hashing
    if data.len() > 1 {
        let mid = data.len() / 2;
        let mut hasher_inc = Hasher::new();
        hasher_inc.update(&data[..mid]);
        hasher_inc.update(&data[mid..]);
        let h4 = hasher_inc.finalize();

        // Incremental hashing should match single hash
        assert_eq!(h1, h4);
    }

    // Test Hash::from_bytes and to_hex
    let hash_bytes = h1.as_bytes();
    let h5 = Hash::from_bytes(*hash_bytes);
    assert_eq!(h1, h5);

    // Test hex encoding
    let hex = h1.to_hex();
    assert_eq!(hex.len(), 64); // 32 bytes = 64 hex chars

    // Test zero detection
    let _ = h1.is_zero();

    // Test constant-time comparison (through PartialEq)
    let _ = h1 == h2;
    let _ = h1 == Hash::ZERO;
});
