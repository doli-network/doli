//! Test-only block signing (INC-I-236): mirrors `production/signing.rs`.
//! Shared by integration tests and in-src `#[cfg(test)]` modules via `#[path]`.
//!
//! OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Sign the final header; call after every header field is set.
pub fn sign_header(header: &mut doli_core::BlockHeader, kp: &crypto::KeyPair) {
    let msg = header.signing_message();
    header.vdf_proof.pi = crypto::signature::sign(msg.as_bytes(), kp.private_key()).to_vec();
}

/// Re-sign a built block in place.
pub fn sign_block(block: &mut doli_core::Block, kp: &crypto::KeyPair) {
    sign_header(&mut block.header, kp);
}

fn registry() -> &'static Mutex<HashMap<[u8; 32], crypto::KeyPair>> {
    static KEYS: OnceLock<Mutex<HashMap<[u8; 32], crypto::KeyPair>>> = OnceLock::new();
    KEYS.get_or_init(Default::default)
}

/// Make `kps` available to [`resign`] for fixtures that only carry the producer pubkey.
pub fn register_keys(kps: &[crypto::KeyPair]) {
    let mut keys = registry().lock().unwrap();
    for kp in kps {
        keys.insert(*kp.public_key().as_bytes(), kp.clone());
    }
}

/// Sign with the registered key of `header.producer`; panics if it was never registered.
pub fn resign(header: &mut doli_core::BlockHeader) {
    let kp = registry()
        .lock()
        .unwrap()
        .get(header.producer.as_bytes())
        .cloned();
    let kp = kp.expect("block producer key not registered with block_sig::register_keys");
    sign_header(header, &kp);
}
