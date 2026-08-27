//! Shape validation for the operator-supplied arguments of `release sign`.
//!
//! INC-I-172 M2, AUDIT-P0-011. A release signature is raw Ed25519 over
//! `format!("{version}:{binary_sha256}")` ([`crate::sign_release_hash`]). The maintainer
//! governance families build their authorizations by the SAME interpolation, also raw,
//! also with no domain tag:
//!
//! | family                    | signed bytes                      |
//! |---------------------------|-----------------------------------|
//! | release                   | `"{version}:{sha256}"`            |
//! | `AddMaintainer`           | `"add:{target_pubkey_hex}"`       |
//! | `RemoveMaintainer`        | `"remove:{target_pubkey_hex}"`    |
//! | `ProtocolActivation`      | `"activate:{version}:{epoch}"`    |
//!
//! An Ed25519 public key in hex and a SHA-256 digest in hex are both 64 characters, so
//! with free-form arguments those are not four templates — they are one. `release sign
//! --version add --hash <64-hex-of-a-key>` produces bytes that the governance verifier
//! recomputes exactly, and the resulting signature seats that key as a maintainer. Three
//! maintainers persuaded to run one signing command with supplied arguments — plausible
//! during an incident — is a permanent maintainer seat with NO key theft, against a
//! freshly rotated and fully honest key set.
//!
//! These two functions are the M2 containment: the shipped CLIs cannot CONSTRUCT the
//! colliding message. They are node-local and touch no consensus rule and no signed
//! format, so they need no activation height and no coordinated deploy.
//!
//! They are containment, not a cure. The cure is domain separation on every family
//! (`DOLI_RELEASE_V1`, `DOLI_MAINTAINER_ADD_V1`, ...), which changes the SIGNED BYTES
//! and is therefore M3 with its own activation height. Until then, anything else that
//! raw-signs operator-supplied bytes with a maintainer key re-opens the oracle — so new
//! signing entry points route through here.
//!
//! One implementation, called by BOTH signing entry points
//! (`bins/node/src/commands/misc.rs` and `bins/cli/src/cmd_governance.rs`). A second,
//! separately-maintained copy is how the compiled keys stayed authoritative on the
//! `doli upgrade` path after the node path was fixed (AUDIT-P1-012) — the same mistake
//! is available here.

use crate::types::{Result, UpdateError};

/// Number of hex characters in a SHA-256 digest — and, not by coincidence, in an
/// Ed25519 public key. The collision this module exists to break lives in that equality.
const SHA256_HEX_LEN: usize = 64;

/// Validate a release version and return the BARE form that gets signed.
///
/// Accepts `MAJOR.MINOR.PATCH`, with an optional leading `v`. Each component must be a
/// non-empty run of ASCII digits that fits a `u32` — the same shape
/// [`crate::is_newer_version`] parses when it decides whether a release is newer, so a
/// version this function admits is a version the update path can actually order.
///
/// The `v` is stripped, then validated: both CLIs strip it BEFORE interpolating, so
/// validating the raw argument instead of the stripped one would leave the oracle open
/// behind one character (`--version vadd` signs `"add:{hash}"`).
///
/// The case is deliberately narrow. Anything the shape admits is a string an operator
/// can be talked into signing, and every real DOLI release tag is exactly this shape;
/// pre-release and build-metadata suffixes are refused because nothing in the update
/// path can compare them either.
///
/// # Errors
/// [`UpdateError::InvalidSigningArgument`] naming `--version` and the expected shape.
pub fn validate_release_version(version: &str) -> Result<String> {
    let bare = version.strip_prefix('v').unwrap_or(version);

    let invalid = || UpdateError::InvalidSigningArgument {
        field: "version",
        expected: "a release version of the form MAJOR.MINOR.PATCH (an optional leading \
                   `v` is accepted), for example `6.24.1` or `v6.24.1`",
        value: version.to_string(),
    };

    let mut parts = bare.split('.');
    let (major, minor, patch) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(a), Some(b), Some(c), None) => (a, b, c),
        _ => return Err(invalid()),
    };

    for component in [major, minor, patch] {
        // `u32::from_str` accepts a leading `+`, and a bare `is_ascii_digit` sweep is
        // what keeps `+1`, `-1`, whitespace and empty components out.
        if component.is_empty() || !component.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid());
        }
        if component.parse::<u32>().is_err() {
            return Err(invalid());
        }
    }

    Ok(bare.to_string())
}

/// Validate a binary/checksums digest and return it unchanged.
///
/// Accepts exactly [`SHA256_HEX_LEN`] hex characters, in either case.
///
/// The case is NOT folded. `verify_release_artifact` reconstructs the signed message
/// from the strings carried in SIGNATURES.json, and
/// `crates/updater/tests/inc_i_172_install_gate_binding.rs`
/// `req_172_006_v_prefix_and_hex_case_are_not_treated_as_tampering` locks it to
/// tolerating the uppercase digests real publishers emit. Lowercasing here would sign
/// bytes the verifier never rebuilds — a "canonicalization" that breaks the releases it
/// was meant to protect.
///
/// This check alone CANNOT close the AddMaintainer leg: a 64-hex Ed25519 public key is
/// byte-indistinguishable from a 64-hex digest, and any check able to tell them apart
/// would have to reject genuine digests. [`validate_release_version`] is what closes
/// that leg. This one closes the `ProtocolActivation` leg, where the second operand is
/// an epoch number (`--hash 1000`), and it refuses the malformed CHECKSUMS.txt line as
/// a side effect.
///
/// # Errors
/// [`UpdateError::InvalidSigningArgument`] naming `--hash` and the expected shape.
pub fn validate_release_hash(hash: &str) -> Result<String> {
    if hash.len() != SHA256_HEX_LEN || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(UpdateError::InvalidSigningArgument {
            field: "hash",
            expected: "a SHA-256 digest of exactly 64 hexadecimal characters",
            value: hash.to_string(),
        });
    }
    Ok(hash.to_string())
}
