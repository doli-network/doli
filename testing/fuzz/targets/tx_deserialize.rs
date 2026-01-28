//! Fuzz target for Transaction deserialization
//!
//! Tests that Transaction::deserialize handles arbitrary byte sequences safely.

#![no_main]

use libfuzzer_sys::fuzz_target;
use doli_core::Transaction;

fuzz_target!(|data: &[u8]| {
    // Try to deserialize arbitrary data as a Transaction
    // This should never panic, only return None for invalid data
    if let Some(tx) = Transaction::deserialize(data) {
        // If deserialization succeeds, verify it can be serialized back
        let reserialized = tx.serialize();

        // Deserialize again to check consistency
        if let Some(tx2) = Transaction::deserialize(&reserialized) {
            // Hashes should match
            assert_eq!(tx.hash(), tx2.hash());
        }

        // Test transaction methods don't panic
        let _ = tx.hash();
        let _ = tx.total_output();
        let _ = tx.is_coinbase();
        let _ = tx.is_exit();
        let _ = tx.is_registration();
        let _ = tx.signing_message();
        let _ = tx.size();
        let _ = tx.exit_data();
        let _ = tx.registration_data();
    }
});
