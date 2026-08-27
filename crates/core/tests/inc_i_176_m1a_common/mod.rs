//! INC-I-176 **M1a** — shared test fixtures and THE GOLDEN VECTOR.
//!
//! OUTPUT CONTRACT: N/A — fixture file. This module asserts nothing. It holds the
//! golden-vector literals and the deterministic key/payload builders that the
//! three INC-I-176 M1 test files drive.
//! INPUT PARTITIONS: N/A — fixture file.
//!
//! Contract: `docs/.workflow/inc-i-176-M1a-output-contract.md`.
//! Spec: `specs/maintainer-authorization-architecture.md` §"Exact bytes signed" (:141).
//!
//! WHY A SHARED FIXTURE. All four `inc_i_176_m1a_*` files make statements about
//! the SAME signing message. If each built its own golden literals, a drift
//! between them would read as a disagreement between tests rather than as a
//! contract break. One vector, one place.
//!
//! `GOLDEN_VALID_BEFORE` is a `signing_message*` ARGUMENT, not a payload field.
//! `MaintainerChangeData` has NO expiry field in M1a and must not grow one — that
//! is M2.5's wire change, behind its own activation height. See
//! `inc_i_176_m1a_wire_freeze.rs`.

#![allow(dead_code)]

use crypto::{Hasher, KeyPair, PublicKey, Signature};
use doli_core::maintainer::{signing_message, MaintainerSignature};

// ===========================================================================
// THE GOLDEN VECTOR — hard-coded here, INDEPENDENTLY of the implementation.
//
// These literals were computed OUTSIDE this repository (BLAKE3 reference
// implementation, verified against the official empty-input vector
// af1349b9...3262 before use). They are NOT to be regenerated from whatever
// `authmsg.rs` happens to output: doing so destroys the only instrument that can
// detect a field reorder, a width change or a re-encoding — the encoder/decoder
// parity discipline the Full Bitfield Decode pillar mandates (CLAUDE.md).
//
// The same six values MUST also be published as `pub const` from
// `crates/core/src/maintainer/authmsg.rs` (re-exported at `doli_core::maintainer`),
// because M4's `doli-node maintainer sign` and every out-of-repo signer must be
// able to self-check against them without linking this test crate.
// `req_176_040_published_golden_constants_equal_the_test_literals`
// (`inc_i_176_m1a_authmsg.rs`) is what binds the two copies together.
// ===========================================================================

/// Fixed 32-byte genesis hash `0x00..=0x1F`. Deliberately NOT a uniform fill: a
/// repeated-byte genesis would survive several of the mutation probes.
pub const GOLDEN_GENESIS: [u8; 32] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
];

/// Fixed 32-byte target public key `0x20..=0x3F`.
///
/// A raw literal rather than a derived key: the golden vector must be
/// re-derivable by an air-gapped signer that has no Rust and no keypair, and only
/// the RAW BYTES of the target enter the preimage. Curve validity is irrelevant to
/// message construction — `PublicKey::from_bytes` does not validate the point
/// either (`crates/crypto/src/keys.rs:68`). REAL keypairs are exercised by every
/// other test in the suite.
pub const GOLDEN_TARGET: [u8; 32] = [
    0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f,
    0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f,
];

/// 17_280 blocks ~= 2 days at `SLOT_DURATION = 10s` — the INC-I-175 rotation
/// window the spec names ("GATE-ADDED GUIDANCE"). Chosen over a round number
/// because its little-endian encoding (`80 43 00 ..`) is not a palindrome, so an
/// LE-vs-BE swap is detectable.
pub const GOLDEN_VALID_BEFORE: u64 = 17_280;

pub const GOLDEN_IS_ADD: bool = true;

/// The domain-separation tag. Restated here rather than imported from the module
/// under test, so the suite carries an expectation the implementation cannot
/// supply.
pub const AUTH_DOMAIN: &[u8] = b"DOLI-MAINTAINER-CHANGE-V1";

/// The SIBLING tag (`crates/core/src/maintainer/digest.rs:20`). Two BLAKE3
/// digests over maintainer data must not be confusable with each other.
pub const SET_DIGEST_DOMAIN: &[u8] = b"DOLI-MAINTAINER-SET-V1";

/// `AUTH_DOMAIN (25) | genesis (32) | is_add (1) | target (32) | valid_before LE (8)`.
pub const GOLDEN_PREIMAGE_LEN: usize = 25 + 32 + 1 + 32 + 8;

pub const GOLDEN_PREIMAGE_HEX_LITERAL: &str = concat!(
    "444f4c492d4d41494e5441494e45522d4348414e47452d5631",
    "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
    "01",
    "202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f",
    "8043000000000000",
);

pub const GOLDEN_DIGEST_HEX_LITERAL: &str =
    "bbd393a6fd1b1ed229e8f47978186be1697aa24e49352f4ce3688116d262dcd2";

/// The same vector with `is_add = false`. Pinned so the effect bit is proven to
/// be a real input to the hash, not merely a different-looking output.
pub const GOLDEN_REMOVE_DIGEST_HEX_LITERAL: &str =
    "611730797666c2471c332f702e6efc7f5044c6975711e958f20d7840def1838a";

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

pub fn golden_target_key() -> PublicKey {
    PublicKey::from_bytes(GOLDEN_TARGET)
}

/// Deterministic real keypair. Fixed seed so every payload is byte-stable.
pub fn kp(seed: u8) -> KeyPair {
    KeyPair::from_seed([seed; 32])
}

pub fn pk(seed: u8) -> PublicKey {
    *kp(seed).public_key()
}

/// A signature entry from a deterministic key. The signature bytes are never
/// verified by anything this suite drives — `authmsg` builds messages and
/// `MaintainerChangeData` stores them; quorum verification lives at the node
/// layer — so a default signature is exactly as representative as a real one and
/// keeps every payload size exact.
pub fn sig_entry(seed: u8) -> MaintainerSignature {
    MaintainerSignature::new(pk(seed), Signature::default())
}

/// The suite's OWN, field-by-field expectation of the preimage. This is the
/// independent encoder the golden vector is checked against; it never calls the
/// thing under test.
pub fn expected_preimage(
    genesis: &[u8],
    is_add: bool,
    target: &[u8; 32],
    valid_before: u64,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(AUTH_DOMAIN.len() + genesis.len() + 41);
    out.extend_from_slice(AUTH_DOMAIN);
    out.extend_from_slice(genesis);
    out.push(u8::from(is_add));
    out.extend_from_slice(target);
    out.extend_from_slice(&valid_before.to_le_bytes());
    out
}

/// BLAKE3-256 through the house hasher (`crypto::Hasher`, plain `new()` — NOT
/// `new_with_domain`, which length-prefixes and would not match `digest.rs`).
pub fn blake3(bytes: &[u8]) -> Vec<u8> {
    let mut h = Hasher::new();
    h.update(bytes);
    h.finalize().as_bytes().to_vec()
}

pub fn hex_of(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

/// The golden `signing_message` call, spelled once.
pub fn golden_message(is_add: bool) -> Vec<u8> {
    signing_message(
        &GOLDEN_GENESIS,
        is_add,
        &golden_target_key(),
        GOLDEN_VALID_BEFORE,
    )
}
