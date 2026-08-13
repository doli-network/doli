//! Shared fixtures for the INC-I-176 M2 `valid_before` sentinel tripwire.
//!
//! Split out of `inc_i_176_m2_sentinel_tripwire.rs` purely to respect the
//! 800-line test-file budget (CLAUDE.md rule 19). NOTHING here changed in the
//! split: every constant, helper and pinned vector is byte-identical to the
//! single-file version it came from.
//!
//! The reasoning these fixtures encode lives in
//! `specs/maintainer-authorization-architecture.md`, section
//! "M2 RESOLUTION — the `valid_before` sentinel", which is the binding record.
//!
//! Requirements: REQ-176-021, REQ-176-022.

#![allow(dead_code, unused_imports)] // each test binary uses a subset of these fixtures

pub use std::fs;
pub use std::path::{Path, PathBuf};

pub use crypto::PublicKey;
pub use doli_core::maintainer::{
    signing_message, signing_message_at, signing_message_preimage, MaintainerChangeData,
    GOLDEN_AUTH_GENESIS_HASH, GOLDEN_AUTH_TARGET_PUBKEY, MAINTAINER_AUTH_VALID_BEFORE_UNSET,
};

// ===========================================================================
// THE M2.5 GUIDE
// ===========================================================================

/// What an M2.5 implementer must do when a tripwire in this file fires.
///
/// A GUIDE, not a complaint: a tripwire that only says "you broke me" gets
/// silenced; one that says what to write instead gets obeyed. Held as one constant
/// so every tripwire fails with the SAME instruction, one edit from the spec clause
/// it restates (`specs/maintainer-authorization-architecture.md:936`).
pub const M25_GUIDE: &str = "\
--------------------------------------------------------------------------\n\
YOU ARE (PROBABLY) IMPLEMENTING M2.5. READ THIS BEFORE CHANGING THE TEST.\n\
--------------------------------------------------------------------------\n\
This failure is EXPECTED at M2.5 and is not a defect. It exists to hand you\n\
the one rule M2 signed on your behalf when it declined to take a second\n\
activation height (spec: `M2 RESOLUTION - the valid_before sentinel`,\n\
`specs/maintainer-authorization-architecture.md:887`, BINDING OBLIGATIONS ON\n\
M2.5 at :936).\n\
\n\
WRITE EXACTLY THIS at every verifier that builds the authorization message:\n\
\n\
    let valid_before = payload\n\
        .valid_before()\n\
        .unwrap_or(MAINTAINER_AUTH_VALID_BEFORE_UNSET);\n\
\n\
NEVER `unwrap_or_default()`. NEVER `unwrap_or(0)`. NEVER a bare `0`.\n\
`u64::default()` is 0, and 0 reads as ALREADY EXPIRED under M3's\n\
`height >= valid_before` rule. That single character costs you BOTH of these\n\
at once:\n\
  (a) CONSENSUS HISTORY - every v1 payload already written above gate #22\n\
      would re-validate against a DIFFERENT message, so every node replaying\n\
      those blocks rejects an authorization it previously accepted;\n\
  (b) GOVERNANCE LIVENESS - every maintainer change above the gate becomes\n\
      unauthorizable. That is a lock-out, not a security improvement.\n\
\n\
The discriminator that selects v1 vs v2 MUST be EXPLICIT and MUST live in the\n\
transaction bytes - never length-based, never try-then-fallback\n\
(AUDIT-P1-001: `reason: Some(\"\")` decodes successfully as `valid_before = 1`).\n\
PREFER putting it INSIDE the signed preimage: that also closes the v1/v2\n\
sentinel aliasing structurally instead of by policy.\n\
\n\
THEN, and only then, retire the affected assertion here DELIBERATELY - and\n\
replace it with the M2.5 statement (a v1 payload and a v2 payload carrying\n\
`valid_before = u64::MAX` must NOT be interchangeable at the verifier).\n\
Editing the expectation to match the code is NOT the fix.\n\
--------------------------------------------------------------------------";

// ===========================================================================
// THE PINNED VECTORS
//
// Computed OUTSIDE this repository with a BLAKE3 reference implementation. The tool
// was first validated against a vector this repo already publishes -
// `GOLDEN_AUTH_DIGEST_HEX` (`authmsg.rs:370`), the digest for
// `valid_before = 17_280` - and reproduced it exactly before minting anything below.
// A pinned constant from an unvalidated tool is just that tool's opinion.
//
// NEITHER may be regenerated from `authmsg.rs` output: that destroys the only
// instrument able to detect a field reorder, a width change or a re-encoding of the
// sentinel message - the encoder/decoder parity discipline the Full Bitfield Decode
// pillar mandates (CLAUDE.md).
//
// Inputs, in all cases:
//   genesis      = GOLDEN_AUTH_GENESIS_HASH   (0x00..=0x1F)
//   target       = GOLDEN_AUTH_TARGET_PUBKEY  (0x20..=0x3F)
//   valid_before = MAINTAINER_AUTH_VALID_BEFORE_UNSET (u64::MAX)
// ===========================================================================

/// Preimage for the ADD arm at the sentinel. 98 bytes:
/// domain 25 | genesis 32 | action 1 | target 32 | expiry 8.
///
/// The expiry group `ffffffffffffffff` is byte-symmetric, so it is the ONE value an
/// LE/BE swap cannot move — which is why the sibling `inc_i_176_m1a_*` suite drives
/// a non-palindromic window as well, and why these pins do not replace it.
pub const PINNED_SENTINEL_PREIMAGE_ADD_HEX: &str = concat!(
    "444f4c492d4d41494e5441494e45522d4348414e47452d5631",
    "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
    "01",
    "202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f",
    "ffffffffffffffff",
);

/// Preimage for the REMOVE arm at the sentinel: identical to ADD except the single
/// action byte, `00`.
///
/// That byte is INSIDE the signed bytes precisely so an `add` authorization can
/// never be replayed as a `remove` (REQ-176-012). The remove arm also runs a
/// DIFFERENT verifier (`verify_multisig_excluding_at`), so a wiring change that
/// fixed only the add arm would leave this one unbound.
pub const PINNED_SENTINEL_PREIMAGE_REMOVE_HEX: &str = concat!(
    "444f4c492d4d41494e5441494e45522d4348414e47452d5631",
    "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
    "00",
    "202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f",
    "ffffffffffffffff",
);

/// `BLAKE3_256(PINNED_SENTINEL_PREIMAGE_ADD_HEX)` — the exact 32 bytes a
/// maintainer signs to AUTHORIZE AN ADDITION at or above `#22` for a v1 payload.
pub const PINNED_SENTINEL_DIGEST_ADD_HEX: &str =
    "1db2c2f01d0489e77e471ca79353b088113a7898999a8390fb31f70a7d86d618";

/// `BLAKE3_256(PINNED_SENTINEL_PREIMAGE_REMOVE_HEX)` — the same, for a REMOVAL.
pub const PINNED_SENTINEL_DIGEST_REMOVE_HEX: &str =
    "af3f6179633093f7bdca32b74f825c81bcd81bfa98cb4e57b67349bc6a9183c9";

/// The exact field list `MaintainerChangeData` is frozen at. ORDER matters: it is
/// the bincode field order, and bincode writes fields positionally with no names on
/// the wire, so a REORDER is as fatal as an addition — the testnet block already
/// carrying this shape (136_690) would decode into different values rather than
/// failing loudly.
pub const V1_FIELDS: &[&str] = &["target", "signatures", "reason"];

/// The sibling struct used as the extractor's POSITIVE CONTROL, and its fields. It
/// lives in the SAME file, is read by the SAME extractor, and `activation_epoch` is
/// a bare `u64` — the exact shape a future `valid_before` field would take.
pub const CONTROL_STRUCT: &str = "ProtocolActivationData";
pub const CONTROL_FIELDS: &[&str] = &[
    "protocol_version",
    "activation_epoch",
    "description",
    "signatures",
];

/// Path, relative to the repo root, of the file that declares the payload.
pub const DATA_RS: &str = "crates/core/src/maintainer/data.rs";

// ===========================================================================
// THE MAPPING, WRITTEN AS M2.5 MUST WRITE IT
// ===========================================================================

/// The v1 payload's `valid_before`: **`None`**, because the type has no such field.
/// This is the premise `tripwire_maintainer_change_data_is_still_v1` guards.
///
/// M2.5 replaces this body with `payload.valid_before()`. Nothing else about the
/// call site below may change — in particular the `unwrap_or` argument.
pub fn v1_payload_valid_before(_payload: &MaintainerChangeData) -> Option<u64> {
    None
}

/// The message the PRODUCTION path builds, with the M2.5 verifier rule spelled out.
///
/// Byte-for-byte the call `bins/node/src/node/apply_block/governance.rs:97-104` (add
/// arm) and `:157-164` (remove arm) make today, with
/// `MAINTAINER_AUTH_VALID_BEFORE_UNSET` replaced by the `unwrap_or` expression it
/// will become. For a v1 payload the two are the same value BY THE MAPPING — and
/// that identity is the whole point.
pub fn production_message_for_v1_payload(
    genesis_hash: &[u8],
    is_add: bool,
    payload: &MaintainerChangeData,
    height: u64,
    activation_height: u64,
) -> Vec<u8> {
    // ===== THE MAPPING UNDER TEST =====
    // `unwrap_or(MAINTAINER_AUTH_VALID_BEFORE_UNSET)`, never `unwrap_or_default()`.
    let valid_before =
        v1_payload_valid_before(payload).unwrap_or(MAINTAINER_AUTH_VALID_BEFORE_UNSET);

    signing_message_at(
        genesis_hash,
        is_add,
        &payload.target,
        valid_before,
        height,
        activation_height,
    )
}

// ===========================================================================
// FIXTURES
// ===========================================================================

/// The golden target as a `PublicKey`. Raw literal bytes, not a derived key: only
/// the RAW BYTES of the target enter the preimage, and `PublicKey::from_bytes`
/// does not validate the curve point either (`crates/crypto/src/keys.rs:68`).
pub fn golden_target() -> PublicKey {
    PublicKey::from_bytes(GOLDEN_AUTH_TARGET_PUBKEY)
}

/// A v1 `MaintainerChangeData` over the golden target.
///
/// `signatures` is empty on purpose: nothing in this file verifies a signature,
/// and the signature vector is OUTSIDE the signed bytes in any case.
pub fn v1_payload() -> MaintainerChangeData {
    MaintainerChangeData::new(golden_target(), Vec::new())
}

pub fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is <repo>/crates/core.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root must resolve")
}

/// Strip `//` line comments so a doc comment naming a field is not counted as a
/// field. Block comments are not used for this in tree.
pub fn strip_line_comments(src: &str) -> String {
    src.lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Brace-matched declaration body of `pub struct <name>`, braces included.
///
/// Same extraction idiom as the derivation tripwire uses for a function body.
pub fn extract_struct_body(src: &str, name: &str) -> Option<String> {
    let decl = format!("pub struct {name}");
    let start = src.find(&decl)?;
    let open = src[start..].find('{')? + start;

    let mut depth = 0usize;
    for (i, c) in src[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(src[open..=open + i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// Ordered field names declared in a struct body.
///
/// Comments are stripped first, attribute lines (`#[..]`) and the braces are
/// dropped, and a leading visibility token of ANY form (`pub`, `pub(crate)`, …)
/// is removed by taking the last whitespace-separated token before the `:`. That
/// last detail matters: a `pub(crate) valid_before: u64` must be SEEN, not
/// silently discarded as unparseable.
pub fn struct_field_names(body: &str) -> Vec<String> {
    strip_line_comments(body)
        .lines()
        .map(str::trim)
        .filter(|l| {
            !l.is_empty() && !l.starts_with('#') && !l.starts_with('{') && !l.starts_with('}')
        })
        .filter_map(|l| {
            let (lhs, _) = l.split_once(':')?;
            let name = lhs.split_whitespace().last()?;
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return None;
            }
            Some(name.to_string())
        })
        .collect()
}
