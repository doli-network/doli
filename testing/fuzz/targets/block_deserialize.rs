//! Fuzz target for Block deserialization
//!
//! Tests that Block::deserialize handles arbitrary byte sequences safely.

#![no_main]

use libfuzzer_sys::fuzz_target;
use doli_core::Block;

fuzz_target!(|data: &[u8]| {
    // Try to deserialize arbitrary data as a Block
    // This should never panic, only return None for invalid data
    let _ = Block::deserialize(data);
});
