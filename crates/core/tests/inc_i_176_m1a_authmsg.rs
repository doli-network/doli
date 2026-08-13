//! INC-I-176 **M1a** — the signed-message ENCODING contract.
//!
//! The golden vector, the mutation set that makes it load-bearing, the legacy
//! bit-identity property, and the cross-purpose separation (REQ-176-040).
//! The BINDING properties (network / effect / gate) are in
//! `inc_i_176_m1a_binding.rs`; the WIRE-FREEZE properties in
//! `inc_i_176_m1a_wire_freeze.rs`; the single-owner properties in
//! `inc_i_176_m1a_ownership.rs`. Split for the 800-line test-file budget
//! (CLAUDE.md rule 19), not by accident.
//!
//! ---------------------------------------------------------------------------
//! WHY THIS FILE IS `m1a` AND NOT `m1`
//! ---------------------------------------------------------------------------
//! M1 attempt 1 swapped `MaintainerChangeData.reason: Option<String>` (1-byte
//! bincode `None` tail) for `valid_before: u64` (8-byte tail) with NO activation
//! gate. That is an ungated wire-format break on FROZEN CONSENSUS HISTORY:
//! testnet block **136690** carries a real `add_maintainer` transaction
//! (`62a3bfbd…bc81`, 385 bytes of `extra_data`, 3 signatures, `reason = None`),
//! and the height-ungated fatal decode at
//! `crates/core/src/validation/tx_types.rs:809` turns a `from_bytes` miss into a
//! hard block reject. Attempt 1's binary cannot sync past that block in either
//! deploy direction.
//!
//! **M1a therefore changes the wire format by ZERO bytes.** `reason` stays
//! exactly as at HEAD (`3f8bf185`). The payload swap moved to milestone M2.5
//! behind its own activation height. The wire freeze is locked by
//! `inc_i_176_m1a_wire_freeze.rs`; THIS file is unaffected by the deferral,
//! because `authmsg` never touched the payload — it only builds MESSAGE bytes.
//!
//! **`valid_before` stays a `signing_message*` PARAMETER.** That is CORRECT and
//! INTENDED for M1a even though no payload field feeds it: M2 wires the
//! constructor into production and M2.5 adds the field. `signing_message_at`
//! having zero production callers in M1a is the INTENDED state, not a defect —
//! do not re-flag it.
//!
//! TDD RED. This file does NOT compile against the tree at `3f8bf185`:
//! `doli_core::maintainer::{signing_message, signing_message_preimage,
//! signing_message_legacy}` and the `GOLDEN_AUTH_*` constants do not exist.
//! That failure IS the RED evidence.
//!
//! Contract + full matrix: `docs/.workflow/inc-i-176-M1a-output-contract.md`.
//! Spec: `specs/maintainer-authorization-architecture.md` §141, §312 (BINDING).
//! Analysis: `docs/redesigns/maintainer-authorization-redesign-analysis.md` §7.
//!
//! ---------------------------------------------------------------------------
//! REQUIRED API (house idiom = private `mod` + `pub use`, as `maintainer/mod.rs:65,75`
//! already does for `digest::maintainer_set_digest`)
//! ---------------------------------------------------------------------------
//! ```ignore
//! // crates/core/src/maintainer/mod.rs
//! mod authmsg;
//! pub use authmsg::{
//!     signing_message, signing_message_at, signing_message_legacy, signing_message_preimage,
//!     GOLDEN_AUTH_DIGEST_HEX, GOLDEN_AUTH_GENESIS_HASH, GOLDEN_AUTH_IS_ADD,
//!     GOLDEN_AUTH_PREIMAGE_HEX, GOLDEN_AUTH_TARGET_PUBKEY, GOLDEN_AUTH_VALID_BEFORE,
//! };
//!
//! // crates/core/src/maintainer/authmsg.rs  (LEAF: no dep on chainspec / network_params)
//! const MAINTAINER_AUTH_DOMAIN: &[u8] = b"DOLI-MAINTAINER-CHANGE-V1";
//! pub fn signing_message_preimage(genesis_hash: &[u8], is_add: bool, target: &PublicKey,
//!                                 valid_before: u64) -> Vec<u8>;
//! pub fn signing_message(genesis_hash: &[u8], is_add: bool, target: &PublicKey,
//!                        valid_before: u64) -> Vec<u8>;   // BLAKE3_256(preimage), 32 bytes
//! pub fn signing_message_legacy(is_add: bool, target: &PublicKey) -> Vec<u8>;
//! pub fn signing_message_at(genesis_hash: &[u8], is_add: bool, target: &PublicKey,
//!                           valid_before: u64, height: u64, activation_height: u64) -> Vec<u8>;
//!
//! // crates/core/src/maintainer/data.rs — UNCHANGED FROM HEAD except ONE delegation:
//! pub struct MaintainerChangeData { pub target, pub signatures, pub reason: Option<String> }
//! pub fn new(target, signatures) -> Self;                        // reason = None
//! pub fn with_reason(target, signatures, reason: String) -> Self;
//! pub fn signing_message(&self, is_add: bool) -> Vec<u8>;        // delegates to _legacy
//! ```
//!
//! The ONLY permitted edit to `data.rs` in M1a is the body of `signing_message`
//! becoming `super::signing_message_legacy(is_add, &self.target)`. That is a
//! pure de-duplication: it produces the same bytes and it is not serialized, so
//! it moves zero wire bytes. Every field, every constructor and every bincode
//! encoding stays byte-identical to HEAD — locked by
//! `inc_i_176_m1a_wire_freeze.rs`.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT (this file's share; full matrix in the .md)
//! ---------------------------------------------------------------------------
//! ENUMERATION OF OBSERVABLE OUTPUTS
//!   A `signing_message_preimage(genesis, is_add, target, valid_before)`
//!     A-O1 returned LENGTH.
//!     A-O2 returned CONTENT, field by field (tag | genesis | action | target | vb-LE).
//!     A-O3 field ORDER and per-field WIDTH — the encoder-parity property.
//!   B `signing_message(..)`
//!     B-O1 returned LENGTH (must be exactly 32).
//!     B-O2 returned DIGEST bytes.
//!     B-O3 composition: `B == BLAKE3_256(A)` for identical arguments.
//!   C `signing_message_legacy(is_add, target)`
//!     C-O1 returned BYTES (today's `format!("{}:{}", "add"|"remove", hex)`).
//!     C-O2 returned LENGTH (68 add / 71 remove).
//!     C-O3 genesis- and expiry-BLINDNESS (the defect, preserved below the gate).
//!   mutable params   : NONE (shared refs / `Copy`).
//!   receiver mutation: NONE (free functions).
//!   persistent store : NONE. No I/O on any path in this file.
//!   side channels    : NONE. DECLARED UNASSERTED — nothing is logged on these paths.
//!
//! CODE PATHS
//!   P-ENC   the single encoding path of A (no branches; the ONLY producer of these bytes).
//!   P-LEG   the single encoding path of C (frozen; must stay byte-identical to today).
//!
//! INPUT PARTITIONS
//!   IP-G0 genesis = GOLDEN_GENESIS (32 B, 0x00..0x1F)  -> the golden vector
//!   IP-T0 target  = GOLDEN_TARGET  (0x20..0x3F)        -> the golden vector
//!   IP-V2 valid_before = 17_280 (~2-day window)        -> the golden / operational value
//!   IP-A0 `is_add = true`                              -> the golden arm
//!   IP-A1 `is_add = false`                             -> the second golden arm
//!   IP-M  eight ENCODING MUTATIONS of IP-G0/T0/V2/A0 (order, width, endianness, tag,
//!         length-prefixing, hex-vs-raw) -> A-O3; each must MISS the pinned vector
//!   IP-K  three real keypairs (0x11 / 0x22 / 0xff)     -> C-O1, C-O2 across key material
//!   IP-V0 valid_before = 1 vs u64::MAX                 -> C-O3 expiry blindness
//!   IP-R  the release-signing family `format!("{}:{}", version, sha)` with
//!         version in {"add","remove","0.2.0"}          -> REQ-176-040 confusability
//!   MATRIX: (A-O1,A-O2) x IP-G0/T0/V2/A0;  A-O3 x IP-M;  (B-O1,B-O2) x IP-A0/IP-A1;
//!           B-O3 x IP-A0/IP-A1;  (C-O1,C-O2) x IP-K x IP-A0/IP-A1;  C-O3 x IP-V0;
//!           REQ-176-040 x IP-R + the sibling-domain probe.
//!
//! ---------------------------------------------------------------------------
//! WHAT THIS FILE DOES NOT DO (binding constraints, not revisited)
//! ---------------------------------------------------------------------------
//! 1. It adds NO reject condition to `crates/core/src/validation/tx_types.rs` (user gate 1:
//!    the fatal split was REJECTED). M1a makes NO change there at all.
//! 2. It does NOT test absolute single-use. REQ-176-010 is RELAXED to bounded-validity
//!    (user gate 2); the seen-set is DEFERRED, not built. The `valid_before` PAYLOAD
//!    field and any expiry ENFORCEMENT are M2.5's, not M1a's.
//! 3. It does NOT touch `crates/updater/`. REQ-176-040 is asserted CORE-SIDE only, by
//!    reproducing the release-signing message SHAPE locally. Flipping
//!    `the_collision_still_exists_and_only_m3_closes_it` is M4's job.
//! 4. It asserts NO consensus behavior change. Every production caller
//!    (`derivation.rs:190,203`, `apply_block/governance.rs:38,74`) still routes through
//!    `signing_message_legacy` in M1; the gate field does not exist until M2.

mod inc_i_176_m1a_common;

use doli_core::maintainer::{
    signing_message, signing_message_legacy, signing_message_preimage, MaintainerChangeData,
    GOLDEN_AUTH_DIGEST_HEX, GOLDEN_AUTH_GENESIS_HASH, GOLDEN_AUTH_IS_ADD, GOLDEN_AUTH_PREIMAGE_HEX,
    GOLDEN_AUTH_TARGET_PUBKEY, GOLDEN_AUTH_VALID_BEFORE,
};
use inc_i_176_m1a_common::{
    blake3, expected_preimage, golden_message, golden_target_key, hex_of, pk, sig_entry,
    AUTH_DOMAIN, GOLDEN_DIGEST_HEX_LITERAL, GOLDEN_GENESIS, GOLDEN_IS_ADD,
    GOLDEN_PREIMAGE_HEX_LITERAL, GOLDEN_PREIMAGE_LEN, GOLDEN_REMOVE_DIGEST_HEX_LITERAL,
    GOLDEN_TARGET, GOLDEN_VALID_BEFORE, SET_DIGEST_DOMAIN,
};

// ===========================================================================
// GOLDEN VECTOR — the load-bearing tests.
//
// Encoder/decoder-parity discipline (CLAUDE.md, Full Bitfield Decode pillar):
// "Any encoder/decoder pair MUST be verified for index parity." Here the
// "decoder" is every out-of-repo signer that must reproduce these bytes.
// ===========================================================================

/// REQ-176-040 / A-O1, A-O2 — the exact preimage, byte for byte. IP-G0/T0/V2/A0.
#[test]
fn req_176_040_golden_vector_preimage_is_byte_exact() {
    let actual = signing_message_preimage(
        &GOLDEN_GENESIS,
        GOLDEN_IS_ADD,
        &golden_target_key(),
        GOLDEN_VALID_BEFORE,
    );
    let expected = expected_preimage(
        &GOLDEN_GENESIS,
        GOLDEN_IS_ADD,
        &GOLDEN_TARGET,
        GOLDEN_VALID_BEFORE,
    );

    assert_eq!(
        expected.len(),
        GOLDEN_PREIMAGE_LEN,
        "fixture: the suite's own encoder must produce 25+32+1+32+8 = {} bytes",
        GOLDEN_PREIMAGE_LEN
    );
    assert_eq!(
        hex_of(&expected),
        GOLDEN_PREIMAGE_HEX_LITERAL,
        "fixture: the field-by-field expectation must equal the published hex literal. If \
         these disagree, the suite contradicts itself and NOTHING below is trustworthy."
    );
    assert_eq!(
        actual.len(),
        GOLDEN_PREIMAGE_LEN,
        "A-O1: the preimage must be exactly {} bytes (domain 25 | genesis 32 | is_add 1 | \
         target 32 | valid_before 8); got {}",
        GOLDEN_PREIMAGE_LEN,
        actual.len()
    );
    assert_eq!(
        hex_of(&actual),
        GOLDEN_PREIMAGE_HEX_LITERAL,
        "A-O2 / REQ-176-040: the signed preimage is NOT the pinned golden vector. Any signer \
         built against the published vector now produces signatures this node will reject, \
         and vice versa. Do NOT regenerate the literal from this output — find which field \
         moved, resized or changed encoding."
    );
}

/// REQ-176-040 / B-O1, B-O2 — the resulting 32-byte digest, byte for byte.
/// IP-A0 and IP-A1.
#[test]
fn req_176_040_golden_vector_digest_is_byte_exact() {
    let msg = golden_message(GOLDEN_IS_ADD);
    assert_eq!(
        msg.len(),
        32,
        "B-O1: the maintainer-authorization message is a BLAKE3-256 digest and must be \
         exactly 32 bytes; got {}",
        msg.len()
    );
    assert_eq!(
        hex_of(&msg),
        GOLDEN_DIGEST_HEX_LITERAL,
        "B-O2 / REQ-176-040: the golden digest changed. This is a signer/verifier contract \
         break, not a cosmetic one."
    );
    assert_eq!(
        hex_of(&golden_message(false)),
        GOLDEN_REMOVE_DIGEST_HEX_LITERAL,
        "B-O2: the `remove` arm of the golden vector changed"
    );
}

/// REQ-176-040 / B-O3 — the message IS the BLAKE3 of the published preimage.
///
/// Without this, the preimage function and the message function could drift
/// apart and both golden assertions above would still pass individually.
#[test]
fn req_176_040_signing_message_is_blake3_of_the_published_preimage() {
    for is_add in [true, false] {
        let pre = signing_message_preimage(
            &GOLDEN_GENESIS,
            is_add,
            &golden_target_key(),
            GOLDEN_VALID_BEFORE,
        );
        assert_eq!(
            golden_message(is_add),
            blake3(&pre),
            "B-O3: signing_message(is_add={}) must be BLAKE3_256 of \
             signing_message_preimage(..) with the SAME arguments. If these diverge, an \
             offline signer shown the preimage signs something the node never verifies.",
            is_add
        );
    }
}

/// REQ-176-040 — the published constants and the test literals are ONE vector.
///
/// This binds `authmsg.rs`'s `pub const` surface — which M4's CLI and the
/// replacement for the out-of-repo `sign_maintainer.py` consume — to the
/// independently computed literals in `inc_i_176_m1a_common`.
#[test]
fn req_176_040_published_golden_constants_equal_the_test_literals() {
    assert_eq!(
        GOLDEN_AUTH_GENESIS_HASH, GOLDEN_GENESIS,
        "the published golden genesis must equal the suite's literal"
    );
    assert_eq!(
        GOLDEN_AUTH_TARGET_PUBKEY, GOLDEN_TARGET,
        "the published golden target must equal the suite's literal"
    );
    assert_eq!(GOLDEN_AUTH_IS_ADD, GOLDEN_IS_ADD);
    assert_eq!(GOLDEN_AUTH_VALID_BEFORE, GOLDEN_VALID_BEFORE);
    assert_eq!(
        GOLDEN_AUTH_PREIMAGE_HEX, GOLDEN_PREIMAGE_HEX_LITERAL,
        "the published preimage hex must equal the suite's literal — this constant is what \
         an air-gapped signer self-checks against"
    );
    assert_eq!(
        GOLDEN_AUTH_DIGEST_HEX, GOLDEN_DIGEST_HEX_LITERAL,
        "the published digest hex must equal the suite's literal"
    );
}

/// REQ-176-040 / A-O3 — the golden vector FAILS if any field is reordered,
/// resized or re-encoded. IP-M.
///
/// Eight mutations. Each builds an alternative encoding and asserts it differs
/// from the pinned preimage AND from the pinned digest. If any mutation matched,
/// the golden vector would be blind to that class of change and the parity
/// discipline would be decorative.
#[test]
fn req_176_040_golden_vector_detects_reorder_resize_and_reencoding() {
    let pinned = signing_message_preimage(
        &GOLDEN_GENESIS,
        GOLDEN_IS_ADD,
        &golden_target_key(),
        GOLDEN_VALID_BEFORE,
    );
    let pinned_digest = golden_message(GOLDEN_IS_ADD);
    let vb = GOLDEN_VALID_BEFORE.to_le_bytes();
    let mut mutations: Vec<(&str, Vec<u8>)> = Vec::new();

    // M1 — genesis and target swapped.
    let mut m = Vec::from(AUTH_DOMAIN);
    m.extend_from_slice(&GOLDEN_TARGET);
    m.push(1);
    m.extend_from_slice(&GOLDEN_GENESIS);
    m.extend_from_slice(&vb);
    mutations.push(("M1 genesis/target swapped", m));

    // M2 — the action byte moved after the target.
    let mut m = Vec::from(AUTH_DOMAIN);
    m.extend_from_slice(&GOLDEN_GENESIS);
    m.extend_from_slice(&GOLDEN_TARGET);
    m.push(1);
    m.extend_from_slice(&vb);
    mutations.push(("M2 action byte relocated", m));

    // M3 — valid_before big-endian.
    let mut m = Vec::from(AUTH_DOMAIN);
    m.extend_from_slice(&GOLDEN_GENESIS);
    m.push(1);
    m.extend_from_slice(&GOLDEN_TARGET);
    m.extend_from_slice(&GOLDEN_VALID_BEFORE.to_be_bytes());
    mutations.push(("M3 valid_before big-endian", m));

    // M4 — the action as ASCII '1' instead of 1u8.
    let mut m = Vec::from(AUTH_DOMAIN);
    m.extend_from_slice(&GOLDEN_GENESIS);
    m.push(b'1');
    m.extend_from_slice(&GOLDEN_TARGET);
    m.extend_from_slice(&vb);
    mutations.push(("M4 action as ASCII", m));

    // M5 — no domain tag at all.
    let mut m = Vec::new();
    m.extend_from_slice(&GOLDEN_GENESIS);
    m.push(1);
    m.extend_from_slice(&GOLDEN_TARGET);
    m.extend_from_slice(&vb);
    mutations.push(("M5 domain tag dropped", m));

    // M6 — the SIBLING domain tag.
    let mut m = Vec::from(SET_DIGEST_DOMAIN);
    m.extend_from_slice(&GOLDEN_GENESIS);
    m.push(1);
    m.extend_from_slice(&GOLDEN_TARGET);
    m.extend_from_slice(&vb);
    mutations.push(("M6 sibling DOLI-MAINTAINER-SET-V1 tag", m));

    // M7 — genesis length-prefixed (the `update_with_length` idiom).
    let mut m = Vec::from(AUTH_DOMAIN);
    m.extend_from_slice(&(GOLDEN_GENESIS.len() as u64).to_le_bytes());
    m.extend_from_slice(&GOLDEN_GENESIS);
    m.push(1);
    m.extend_from_slice(&GOLDEN_TARGET);
    m.extend_from_slice(&vb);
    mutations.push(("M7 length-prefixed genesis", m));

    // M8 — target hex-encoded instead of raw (the legacy habit).
    let mut m = Vec::from(AUTH_DOMAIN);
    m.extend_from_slice(&GOLDEN_GENESIS);
    m.push(1);
    m.extend_from_slice(golden_target_key().to_hex().as_bytes());
    m.extend_from_slice(&vb);
    mutations.push(("M8 hex-encoded target", m));

    for (name, mutated) in mutations {
        assert_ne!(
            mutated, pinned,
            "A-O3 / {}: this mutation produced the PINNED preimage. The golden vector is \
             blind to that change class, so a signer using it would not be detectably \
             incompatible.",
            name
        );
        assert_ne!(
            blake3(&mutated),
            pinned_digest,
            "A-O3 / {}: this mutation produced the PINNED digest",
            name
        );
    }
}

// ===========================================================================
// LEGACY BIT-IDENTITY — the below-gate safety property.
//
// M1 changes NO consensus behavior. `derivation.rs:190,203` and
// `apply_block/governance.rs:38,74` keep emitting today's bytes.
// ===========================================================================

/// REQ-176-030 / C-O1, C-O2 — `signing_message_legacy` reproduces TODAY's bytes.
/// IP-K x IP-A0/IP-A1.
///
/// The expectation is built independently, from the current implementation
/// semantics at `crates/core/src/maintainer/data.rs:46-49`:
/// `format!("{}:{}", "add"|"remove", target.to_hex()).into_bytes()`. It never
/// calls the thing under test to build its own expectation.
#[test]
fn req_176_030_legacy_message_is_byte_identical_to_todays_format() {
    for seed in [0x11u8, 0x22, 0xff] {
        let target = pk(seed);
        let hex = target.to_hex();

        assert_eq!(
            signing_message_legacy(true, &target),
            format!("add:{}", hex).into_bytes(),
            "C-O1: the legacy ADD message must stay byte-identical to data.rs:46-49. Any \
             drift here silently invalidates every signature the current fleet would accept."
        );
        assert_eq!(
            signing_message_legacy(false, &target),
            format!("remove:{}", hex).into_bytes(),
            "C-O1: the legacy REMOVE message must stay byte-identical to data.rs:46-49"
        );
        assert_eq!(
            signing_message_legacy(true, &target).len(),
            68,
            "C-O2: \"add:\" (4) + 64 hex chars = 68 bytes"
        );
        assert_eq!(
            signing_message_legacy(false, &target).len(),
            71,
            "C-O2: \"remove:\" (7) + 64 hex chars = 71 bytes"
        );
    }
}

/// REQ-176-030 — `MaintainerChangeData::signing_message` delegates to
/// `signing_message_legacy`, byte for byte.
///
/// The method MUST survive M1a: `derivation.rs:190,203` call it, and so does the
/// read-only updater test at `inc_i_172_m2_release_sign_arg_validation.rs:301`.
/// It becomes a thin delegate — which is what satisfies REQ-176-030 ("exactly
/// ONE implementation of the signed message"), not deletion.
///
/// The `reason` partition matters here even though the legacy message ignores it:
/// it is the one payload field a caller can vary, and this test is what proves the
/// delegation did NOT start folding it in. M1a's message must stay
/// reason-blind exactly as HEAD's is.
#[test]
fn req_176_030_struct_method_delegates_to_the_legacy_constructor() {
    let target = pk(0x33);
    let reasons: [Option<String>; 4] = [
        None,
        Some(String::new()),
        Some("rotation".to_string()),
        Some("x".repeat(256)),
    ];
    for reason in reasons {
        let data = match reason.clone() {
            None => MaintainerChangeData::new(target, vec![sig_entry(0x44)]),
            Some(r) => MaintainerChangeData::with_reason(target, vec![sig_entry(0x44)], r),
        };
        for is_add in [true, false] {
            assert_eq!(
                data.signing_message(is_add),
                signing_message_legacy(is_add, &target),
                "REQ-176-030: MaintainerChangeData::signing_message must be the SAME bytes as \
                 authmsg::signing_message_legacy — one implementation, several callers. \
                 (reason={:?}, is_add={})",
                reason.as_ref().map(|r| r.len()),
                is_add
            );
        }
    }
}

/// REQ-176-011 / C-O3 — the LEGACY message is genesis-blind and expiry-blind.
/// IP-V0.
///
/// The DEFECT, pinned as a fact rather than described in a comment. It is what
/// below-the-gate history keeps, and it is the exact reason the new message
/// exists. It is also the positive control for the cross-network tests in
/// `inc_i_176_m1a_binding.rs`: without it, "the digests differ" could not be
/// attributed to the genesis term.
///
/// Expiry blindness is asserted on the FUNCTION (`signing_message_legacy` has no
/// `valid_before` parameter at all, and the bound form's two expiries are shown
/// to differ), NOT on a payload field — the payload has no expiry field in M1a
/// and must not grow one.
#[test]
fn req_176_011_legacy_message_is_genesis_and_expiry_blind() {
    let target = pk(0x55);
    let data_a = MaintainerChangeData::new(target, vec![]);
    let data_b = MaintainerChangeData::with_reason(target, vec![], "rotation".to_string());

    assert_eq!(
        data_a.signing_message(true),
        data_b.signing_message(true),
        "C-O3: the legacy message carries no payload content beyond the target — not even the \
         `reason` that changes the txid. This is the pre-INC-I-176 malleability defect."
    );
    assert_eq!(
        signing_message_legacy(true, &target),
        data_b.signing_message(true),
        "C-O3: the legacy message carries no chain identity — the same bytes authorize the \
         same change on EVERY DOLI network"
    );
    // POSITIVE CONTROL. Without this, "legacy is expiry-blind" could be true
    // merely because nothing anywhere reacts to `valid_before`. The BOUND form
    // must react.
    assert_ne!(
        signing_message(&GOLDEN_GENESIS, true, &target, 1),
        signing_message(&GOLDEN_GENESIS, true, &target, u64::MAX),
        "POSITIVE CONTROL: the BOUND message must react to valid_before. If it does not, the \
         expiry-blindness assertions above are vacuous."
    );
}

// ===========================================================================
// REQ-176-040 — CROSS-PURPOSE SEPARATION (core-side only; updater untouched)
// ===========================================================================

/// REQ-176-040 — the domain-tagged digest is not confusable with the
/// release-signing family, nor with the legacy `"add:<hex>"` shape. IP-R.
///
/// The release family is `format!("{}:{}", version, binary_sha256)`
/// (`crates/updater/src/verification.rs:33`). AUDIT-P0-011: with
/// `version = "add"` and `binary_sha256 = target.to_hex()` it is BYTE-IDENTICAL
/// to the legacy governance message — one signing command away from a permanent
/// maintainer seat. That shape is reproduced HERE, locally; the updater crate is
/// not touched and its own collision test is M4's to flip.
///
/// The separation is proven STRUCTURALLY, by length, not probabilistically: the
/// new message is exactly 32 bytes, while every operationally reachable member of
/// either ASCII family carries a 64-char hex digest and is therefore at least 65
/// bytes. No member of those families can equal it.
#[test]
fn req_176_040_digest_is_not_confusable_with_the_release_or_legacy_families() {
    let target = pk(0x9a);
    let hex = target.to_hex();

    let release_shaped_add = format!("{}:{}", "add", hex).into_bytes();
    let release_shaped_remove = format!("{}:{}", "remove", hex).into_bytes();
    let release_shaped_version = format!("{}:{}", "0.2.0", hex).into_bytes();
    let legacy_add = signing_message_legacy(true, &target);
    let legacy_remove = signing_message_legacy(false, &target);

    assert_eq!(
        release_shaped_add, legacy_add,
        "positive control / AUDIT-P0-011: the release-signing shape and the LEGACY governance \
         message are byte-identical today. If this ever fails, the collision was closed \
         somewhere else and this test is measuring the wrong thing."
    );

    let new_msg = signing_message(&GOLDEN_GENESIS, true, &target, GOLDEN_VALID_BEFORE);
    assert_eq!(new_msg.len(), 32, "B-O1");

    for (name, other) in [
        ("release-shaped add", release_shaped_add),
        ("release-shaped remove", release_shaped_remove),
        ("release-shaped version", release_shaped_version),
        ("legacy add", legacy_add),
        ("legacy remove", legacy_remove),
    ] {
        assert!(
            other.len() >= 65,
            "structural premise: every operationally reachable member of these ASCII families \
             carries a 64-char hex digest, so it cannot be 32 bytes long; {} was {}",
            name,
            other.len()
        );
        assert_ne!(
            new_msg, other,
            "REQ-176-040: the new authorization message must not be confusable with {}",
            name
        );
    }

    // And the preimage NAMES its protocol — the property the length argument
    // rests on. A 32-byte attacker-chosen ASCII string cannot be produced by this
    // constructor, because the constructor hashes a domain-tagged preimage.
    let pre = signing_message_preimage(&GOLDEN_GENESIS, true, &target, GOLDEN_VALID_BEFORE);
    assert!(
        pre.starts_with(AUTH_DOMAIN),
        "REQ-176-040: the preimage must begin with {:?} — the domain tag is what makes the \
         signing families non-confusable at the SOURCE, independently of the length argument",
        std::str::from_utf8(AUTH_DOMAIN).unwrap()
    );
    assert!(
        !pre.starts_with(SET_DIGEST_DOMAIN),
        "REQ-176-040: the preimage must not carry the sibling maintainer-SET tag"
    );
}

/// REQ-176-040 — separation from the SIBLING domain `b"DOLI-MAINTAINER-SET-V1"`
/// (`crates/core/src/maintainer/digest.rs:20`).
///
/// Both digests are BLAKE3 over `tag | genesis | ...` with the SAME genesis. The
/// tag is the only thing keeping them apart, so it is asserted against a worst
/// case: a set-shaped preimage whose tail is byte-identical to the authorization
/// preimage's tail.
#[test]
fn req_176_040_digest_is_separated_from_the_sibling_set_digest_domain() {
    let target = pk(0x9b);
    let auth_pre = signing_message_preimage(&GOLDEN_GENESIS, true, &target, GOLDEN_VALID_BEFORE);
    let tail = &auth_pre[AUTH_DOMAIN.len()..];

    let mut sibling_pre = Vec::from(SET_DIGEST_DOMAIN);
    sibling_pre.extend_from_slice(tail);

    assert_ne!(
        AUTH_DOMAIN, SET_DIGEST_DOMAIN,
        "fixture: the two tags must actually differ"
    );
    assert_ne!(
        auth_pre, sibling_pre,
        "REQ-176-040: with an identical tail, only the tag separates the two maintainer \
         digest families"
    );
    assert_ne!(
        signing_message(&GOLDEN_GENESIS, true, &target, GOLDEN_VALID_BEFORE),
        blake3(&sibling_pre),
        "REQ-176-040: the authorization digest and a set-shaped digest over the same tail \
         must differ"
    );
}
