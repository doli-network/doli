//! INC-I-176 M1a — the maintainer-authorization signing message (AUDIT-P0-011,
//! AUDIT-P1-004, AUDIT-P1-016).
//!
//! This module is the SOLE producer of the bytes a maintainer signs to authorize
//! an `AddMaintainer` / `RemoveMaintainer` change. Every verifier, every builder,
//! every CLI and every out-of-repo signer derives them from here — REQ-176-030,
//! "exactly ONE implementation of the signed message".
//!
//! It is a LEAF module by construction. The genesis hash arrives as a plain byte
//! slice and the activation height as a plain `u64` — the idiom already used by
//! [`super::digest::maintainer_set_digest`] and by
//! [`super::MaintainerSet::verify_multisig_at`] (`set.rs:259-261`) — so
//! `crates::maintainer` gains NO dependency edge toward `chainspec` or
//! `network_params`.
//!
//! # What was wrong with the old message
//!
//! The pre-INC-I-176 message was two lines inside
//! [`super::MaintainerChangeData::signing_message`]:
//!
//! ```text
//! format!("{}:{}", "add"|"remove", target.to_hex()).into_bytes()
//! ```
//!
//! It is retained verbatim as [`signing_message_legacy`] because it is frozen
//! consensus history below the gate, and it carries three measured defects:
//!
//! 1. **No domain separation (AUDIT-P0-011).** The release-signing family is
//!    `format!("{}:{}", version, binary_sha256)`
//!    (`crates/updater/src/verification.rs:33`). With `version = "add"` and
//!    `binary_sha256 = target.to_hex()` the two are BYTE-IDENTICAL: a maintainer
//!    who signs what looks like a release approval can be made to sign a
//!    permanent seat for an attacker key.
//! 2. **No chain identity (AUDIT-P1-016).** The mainnet and testnet
//!    `BOOTSTRAP_MAINTAINER_KEYS_*` arrays have been byte-identical, so membership
//!    alone never distinguished the networks. A signature harvested on testnet
//!    authorized the same change on mainnet.
//! 3. **No expiry (AUDIT-P1-004).** The message is a pure function of
//!    `(is_add, target)`, so an authorization is a permanent bearer token: an
//!    archived `add:<hex>` blob re-authorizes the same seat forever, and
//!    transactions carry no nonce that could dedupe it.
//!
//! # The new message
//!
//! ```text
//! BLAKE3_256( b"DOLI-MAINTAINER-CHANGE-V1"   // domain tag       — defect 1
//!           || genesis_hash                  // network scope    — defect 2
//!           || [is_add as u8]                // effect: action
//!           || target.as_bytes()             // effect: target
//!           || valid_before.to_le_bytes() )  // 8 B, expiry      — defect 3
//! ```
//!
//! This is the house digest idiom verbatim
//! (`crates/core/src/maintainer/digest.rs:20,77-89`, which already binds
//! `b"DOLI-MAINTAINER-SET-V1" || genesis_hash || …`). It is the in-house pattern
//! and it outranks any industry pattern.
//!
//! The ACTION and the TARGET — the two terms that decide the effect — are inside
//! these bytes, so effect coverage (REQ-176-012) holds by construction rather
//! than by review.
//!
//! # What is NOT inside these bytes yet, stated plainly
//!
//! `valid_before` above is a FUNCTION PARAMETER, not a payload field.
//! [`super::MaintainerChangeData`] carries no such field in M1a, and its
//! `reason: Option<String>` is outside the signed bytes exactly as it always
//! was. Both facts are M2.5's to change, behind an activation height and a
//! format discriminator; see the type's own doc comment for why the payload may
//! not move a single byte before then (a real `add_maintainer` payload of the
//! frozen shape is already in testnet history at block 136_690, and its decoder
//! is consumed fatally and UNGATED).
//!
//! `signatures` is likewise unsigned and cannot sign itself. Canonical ordering
//! is the remedy and it, too, is M2.5's: ordering the vector changes the txid
//! for the same caller input, so it is a behaviour change and does not belong in
//! a zero-wire-change milestone. It is not an adversarial control in any case —
//! `sendTransaction` accepts any ordering off the wire (security audit F3).
//!
//! # M1a changes NO consensus behavior
//!
//! Every production caller still emits the LEGACY bytes in M1a. The gate field
//! (`inc_i_176_auth_binding_activation_height`) does not exist until M2;
//! [`signing_message_at`] is built and tested now so that M2 only has to supply a
//! number. Pinned by `crates/core/tests/inc_i_176_m1a_authmsg.rs` and
//! `crates/core/tests/inc_i_176_m1a_binding.rs`.

use crypto::{Hasher, PublicKey};

/// Domain-separation tag for the maintainer-authorization preimage.
///
/// Private on purpose. Nothing outside this module may build these bytes; a
/// second producer of the tag is a second producer of the message, which is what
/// REQ-176-030 forbids. Consumers that must SEE the tag read it out of
/// [`GOLDEN_AUTH_PREIMAGE_HEX`], which starts with it.
///
/// The `-V1` suffix is a format version, not a protocol version: if the field
/// list ever changes, the tag changes with it and old and new messages become
/// mutually unverifiable by construction instead of silently aliasing.
const MAINTAINER_AUTH_DOMAIN: &[u8] = b"DOLI-MAINTAINER-CHANGE-V1";

/// Byte width of every fixed-width term after `genesis_hash`:
/// `is_add` (1) + `target` (32) + `valid_before` (8).
const FIXED_TAIL_LEN: usize = 1 + 32 + 8;

/// The exact bytes that are hashed, before hashing.
///
/// Published so an air-gapped signer, the M4 `doli-node maintainer sign` command
/// and any out-of-repo tool can DISPLAY what is about to be signed and re-derive
/// it without linking this crate. A signer that can only see a 32-byte digest
/// cannot tell an authorization from any other digest, which is how AUDIT-P0-011
/// became reachable in the first place.
///
/// # Field order and width are load-bearing
///
/// The order below is a wire contract, not a style choice. `genesis_hash` is the
/// ONE variable-length term and every term after it is fixed-width, so two
/// preimages built from different-width genesis hashes always differ in length —
/// there is no concatenation ambiguity to exploit. Nothing is length-prefixed
/// (plain `update`, never `update_with_length`), exactly as `digest.rs` does.
///
/// The parity discipline the Full Bitfield Decode pillar mandates (CLAUDE.md)
/// applies here with the "decoder" being every out-of-repo signer:
/// `req_176_040_golden_vector_detects_reorder_resize_and_reencoding` drives eight
/// mutations (field swap, action relocation, big-endian expiry, ASCII action,
/// dropped tag, sibling tag, length-prefixed genesis, hex-encoded target) and
/// requires each to MISS the pinned vector.
///
/// `is_add` is written as `1u8` / `0u8` — never as ASCII `'1'` / `'0'`, and never
/// omitted on the strength of `tx.tx_type` also carrying the action. The tx type
/// is outside the signature; the action must be inside it, or an `add`
/// authorization is re-usable as a `remove` (REQ-176-012).
pub fn signing_message_preimage(
    genesis_hash: &[u8],
    is_add: bool,
    target: &PublicKey,
    valid_before: u64,
) -> Vec<u8> {
    let mut preimage =
        Vec::with_capacity(MAINTAINER_AUTH_DOMAIN.len() + genesis_hash.len() + FIXED_TAIL_LEN);
    preimage.extend_from_slice(MAINTAINER_AUTH_DOMAIN);
    preimage.extend_from_slice(genesis_hash);
    preimage.push(u8::from(is_add));
    preimage.extend_from_slice(target.as_bytes());
    preimage.extend_from_slice(&valid_before.to_le_bytes());
    preimage
}

/// The 32-byte message a maintainer signs AT AND ABOVE the INC-I-176 gate.
///
/// `BLAKE3_256([`signing_message_preimage`])`. It hashes the vector the preimage
/// function returns rather than streaming the same fields into a hasher a second
/// time: a second field-by-field copy is a second encoder, and two encoders drift.
/// An offline signer shown the preimage and a node computing the digest must be
/// provably looking at the same bytes —
/// `req_176_040_signing_message_is_blake3_of_the_published_preimage` is what binds
/// them.
///
/// Returns `Vec<u8>` rather than `[u8; 32]` because
/// [`super::MaintainerSet::verify_multisig_at`] takes `&[u8]` and the caller must
/// be able to hold either arm of [`signing_message_at`] in one binding.
pub fn signing_message(
    genesis_hash: &[u8],
    is_add: bool,
    target: &PublicKey,
    valid_before: u64,
) -> Vec<u8> {
    let mut hasher = Hasher::new();
    hasher.update(&signing_message_preimage(
        genesis_hash,
        is_add,
        target,
        valid_before,
    ));
    hasher.finalize().as_bytes().to_vec()
}

/// TODAY's message, verbatim: `format!("{}:{}", "add"|"remove", target.to_hex())`.
///
/// FROZEN. This is what every node on every DOLI network verifies below
/// `inc_i_176_auth_binding_activation_height`, so any drift here silently
/// invalidates signatures the running fleet accepts. It is defective by design
/// (no domain tag, no chain, no expiry — see the module header); it exists so the
/// below-gate branch of [`signing_message_at`] is well-defined, and so that the
/// defect lives in ONE place that can be pointed at instead of being re-typed at
/// each call site.
///
/// 68 bytes for `add` (`"add:"` + 64 hex chars), 71 for `remove`. Pinned by
/// `req_176_030_legacy_message_is_byte_identical_to_todays_format`.
pub fn signing_message_legacy(is_add: bool, target: &PublicKey) -> Vec<u8> {
    let action = if is_add { "add" } else { "remove" };
    format!("{}:{}", action, target.to_hex()).into_bytes()
}

/// Height-gated message selection — the ONLY form consensus paths may use once
/// M2 lands.
///
/// * `height <  activation_height` → [`signing_message_legacy`]
/// * `height >= activation_height` → [`signing_message`]
///
/// The comparison is `>=`, matching [`super::MaintainerSet::verify_multisig_at`]
/// (`set.rs:269`) exactly. A `>` here would shift this activation one block
/// relative to every other maintainer gate, and the two gates are read by the same
/// code path.
///
/// `height` MUST be a chain-derived block height, never a per-process counter
/// (INV-SYNC-012). `activation_height` is
/// `NetworkParams::inc_i_176_auth_binding_activation_height`, passed in as a plain
/// `u64` so this module stays a leaf.
///
/// **M1a has no production caller, and that is the INTENDED state — not a
/// defect.** The gate field does not exist until M2; this function is built and
/// pinned now (`inc_i_176_m1a_binding.rs`, including the `0/0` devnet origin and
/// the `u64::MAX/u64::MAX` frozen-mainnet extremes) so that M2 is a one-line
/// wiring change rather than a new mechanism. `valid_before` is a parameter here
/// for the same reason: M2 supplies it, M2.5 gives it a payload field.
pub fn signing_message_at(
    genesis_hash: &[u8],
    is_add: bool,
    target: &PublicKey,
    valid_before: u64,
    height: u64,
    activation_height: u64,
) -> Vec<u8> {
    if height >= activation_height {
        signing_message(genesis_hash, is_add, target, valid_before)
    } else {
        signing_message_legacy(is_add, target)
    }
}

// ============================================================================
// THE PUBLISHED GOLDEN VECTOR
//
// M4's `doli-node maintainer sign` and every out-of-repo signer (the replacement
// for `sign_maintainer.py`) MUST self-check against these constants before
// signing anything. They are published from the crate — not only from the test
// suite — precisely so a tool that cannot link the test crate can still prove it
// reproduces this node's encoding.
//
// The values were computed INDEPENDENTLY of this implementation (BLAKE3
// reference implementation, verified against the official empty-input vector
// af1349b9…3262 first) and are duplicated in
// `crates/core/tests/inc_i_176_m1a_common/mod.rs`.
// `req_176_040_published_golden_constants_equal_the_test_literals` binds the two
// copies. NEITHER copy may be regenerated from the other, or from this module's
// output: that would destroy the only instrument able to detect a field reorder,
// a width change or a re-encoding.
// ============================================================================

/// Golden-vector genesis hash: `0x00..=0x1F`.
///
/// Deliberately NOT a uniform fill — a repeated-byte genesis would survive
/// several of the mutation probes (a byte-swap inside it would be invisible).
pub const GOLDEN_AUTH_GENESIS_HASH: [u8; 32] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
];

/// Golden-vector target public key: `0x20..=0x3F`.
///
/// A raw literal, not a derived key: the vector must be re-derivable by an
/// air-gapped signer that has no Rust and no keypair, and only the RAW BYTES of
/// the target enter the preimage. Curve validity is irrelevant to message
/// construction — [`PublicKey::from_bytes`] does not validate the point either.
pub const GOLDEN_AUTH_TARGET_PUBKEY: [u8; 32] = [
    0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f,
    0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f,
];

/// Golden-vector action: an ADDITION.
pub const GOLDEN_AUTH_IS_ADD: bool = true;

/// Golden-vector expiry: 17_280 blocks ≈ 2 days at `SLOT_DURATION = 10s` — the
/// INC-I-175 rotation window the spec names.
///
/// Chosen over a round number because its little-endian encoding
/// (`80 43 00 00 00 00 00 00`) is not a palindrome, so an LE↔BE swap is
/// detectable in the vector itself.
pub const GOLDEN_AUTH_VALID_BEFORE: u64 = 17_280;

/// Hex of `signing_message_preimage(GOLDEN_AUTH_GENESIS_HASH, GOLDEN_AUTH_IS_ADD,
/// GOLDEN_AUTH_TARGET_PUBKEY, GOLDEN_AUTH_VALID_BEFORE)`.
///
/// 98 bytes: domain 25 | genesis 32 | action 1 | target 32 | expiry 8. Read it in
/// those five groups; that grouping IS the contract.
pub const GOLDEN_AUTH_PREIMAGE_HEX: &str = concat!(
    "444f4c492d4d41494e5441494e45522d4348414e47452d5631",
    "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
    "01",
    "202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f",
    "8043000000000000",
);

/// Hex of `signing_message(..)` over the same four golden inputs — i.e. of
/// `BLAKE3_256` applied to [`GOLDEN_AUTH_PREIMAGE_HEX`].
pub const GOLDEN_AUTH_DIGEST_HEX: &str =
    "bbd393a6fd1b1ed229e8f47978186be1697aa24e49352f4ce3688116d262dcd2";
