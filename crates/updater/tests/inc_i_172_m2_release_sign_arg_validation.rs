// INC-I-172 M2 security-audit fix — AUDIT-P0-011 (the M2 half).
//
// `doli-node release sign --version add --hash <64-hex-of-attacker-pubkey>` mints a
// signature over the bytes `"add:<64hex>"`. `MaintainerChangeData::signing_message(true)`
// builds `format!("add:{}", target.to_hex())` — the SAME bytes. Both sides sign and
// verify RAW, with no domain tag, so ONE release-signing invocation with attacker-chosen
// arguments is a valid governance authorization to seat that key as a maintainer. An
// Ed25519 pubkey hex and a SHA-256 hex are both 64 characters: the two templates are one
// template. The same oracle covers `ProtocolActivation` via
// `--version "activate:7" --hash "1000"`.
//
// Finding: `docs/.workflow/security-audit-report-M2.md` § AUDIT-P0-011.
//
// SCOPE SPLIT, stated so nobody "fixes" the wrong half here:
//   * M2 (this file): the CLI cannot CONSTRUCT the colliding message. Reject a
//     `--version` that is not a semver and a `--hash` that is not 64 hex characters,
//     BEFORE the message is built. Node-local, no activation height, no coordination.
//   * M3: domain-separate every signing family (`DOLI_RELEASE_V1`,
//     `DOLI_MAINTAINER_ADD_V1`, ...). That changes the SIGNED BYTES, which is
//     consensus-visible for the governance families and needs its own activation height.
//     `the_collision_still_exists_and_only_m3_closes_it` below is the standing proof
//     that the M2 half is containment, not a cure.
//
// ============================================================================
// OUTPUT CONTRACT
// ============================================================================
// Functions under test:
//   `updater::validate_release_version(version: &str) -> Result<String, UpdateError>`
//   `updater::validate_release_hash(hash: &str)       -> Result<String, UpdateError>`
//   Both are called by `bins/node/src/commands/misc.rs` (`release sign`) and
//   `bins/cli/src/cmd_governance.rs` (`doli release sign`) before any signing message is
//   interpolated, and before the CHECKSUMS.txt fetch.
//
// ENUMERATION OF OBSERVABLE OUTPUTS.
//   - mutable params    : NONE (`&str`).
//   - receiver mutation : NONE (free functions).
//   - persistent store  : NONE. Pure.
//   - return value      : the value channel (O1, O2).
//   - process state     : none directly; the CLI turns an Err into a non-zero exit,
//                         which is the operator-visible behavior the audit asked for.
//
//   O1: Result discriminant  — Ok / Err. The security-load-bearing cell.
//   O2: On Ok, the returned canonical string. For a version this is the BARE form (the
//       `v` prefix stripped), because that is exactly the string the CLIs interpolate
//       today; the fix must not change what a VALID invocation signs. For a hash it is
//       the input unchanged — see t12 for why the case is not folded.
//   O3: On Err, the Display text — must name the offending FIELD (`--version` /
//       `--hash`) and the EXPECTED SHAPE, so a maintainer who mistyped a tag is not
//       left guessing, and one who was handed attacker arguments sees why.
//
// CODE PATHS:
//   P1: version, `v`-prefixed and well formed   -> Ok(bare)
//   P2: version, bare and well formed           -> Ok(same)
//   P3: version, not MAJOR.MINOR.PATCH          -> Err
//   P4: hash, exactly 64 hex characters         -> Ok
//   P5: hash, anything else                     -> Err
//
// INPUT PARTITIONS:
//   I1: real published tags — "6.24.1", "v6.24.1", "0.2.0", "9.9.9"      [accept]
//   I2: the governance action words — "add", "remove", "vadd"            [REJECT: the oracle]
//   I3: the ProtocolActivation prefix — "activate:7"                     [REJECT: the oracle]
//   I4: shapes that are neither — "", "v", "latest", "1.0", "1.0.0.0",
//       "1.0.x", "1. 0.0", "-1.0.0"                                      [reject]
//   I5: 64 hex, lower and UPPER case                                     [accept]
//   I6: a 64-hex Ed25519 PUBKEY                                          [accept by the
//       hash check alone — it is byte-indistinguishable from a digest. Asserted, not
//       hidden: the version check is the one that closes the oracle, and this partition
//       is what proves each check's real reach.]
//   I7: not 64 hex — "1000", 63 chars, 65 chars, non-hex chars, ""       [reject]
//
// TRUTH TABLE
//  case | path | input                  | O1  | O2          | O3
//  -----|------|------------------------|-----|-------------|------------------
//  t01  | P2   | "6.24.1"               | Ok  | "6.24.1"    | n/a
//  t02  | P1   | "v6.24.1"              | Ok  | "6.24.1"    | n/a
//  t03  | P3   | "add"                  | Err | n/a         | --version + shape
//  t04  | P3   | "remove"               | Err | n/a         | --version + shape
//  t05  | P3   | "vadd"                 | Err | n/a         | --version + shape
//  t06  | P3   | "activate:7"           | Err | n/a         | --version + shape
//  t07  | P3   | I4 members             | Err | n/a         | --version + shape
//  t08  | P4   | 64 lowercase hex       | Ok  | unchanged   | n/a
//  t09  | P4   | 64 UPPERCASE hex       | Ok  | unchanged   | n/a
//  t10  | P5   | "1000" and I7 members  | Err | n/a         | --hash + shape
//  t11  | P4   | 64-hex pubkey          | Ok  | unchanged   | n/a  [honest limit]
//  t12  | P4   | case is not folded     | Ok  | unchanged   | n/a
//  t13  | both | the full attack line   | Err | n/a         | refused
//  t14  | n/a  | the collision itself   | still present — M3 obligation, pinned

use doli_core::maintainer::MaintainerChangeData;
use updater::{sign_release_hash, validate_release_hash, validate_release_version};

fn kp(seed: u8) -> crypto::KeyPair {
    crypto::KeyPair::from_private_key(crypto::PrivateKey::from_bytes([seed; 32]))
}

/// O3 — the refusal must be usable by the human who typed the command.
fn assert_names_field_and_shape(err: &updater::UpdateError, field: &str, ctx: &str) {
    let text = err.to_string();
    assert!(
        text.contains(field),
        "{ctx}: O3 — the error must name the offending field `{field}`; got: {text}"
    );
}

// ---------------------------------------------------------------------------
// THE ORACLE — RED before this fix.
// ---------------------------------------------------------------------------

/// AUDIT-P0-011, the attack invocation. [t13]
/// Acceptance: the exact command line from the report — a governance action word as the
/// version and an Ed25519 public key as the hash — cannot reach the signer.
#[test]
fn p0_011_the_attack_invocation_is_refused_before_anything_is_signed() {
    let attacker = *kp(0x51).public_key();

    let err = validate_release_version("add").expect_err(
        "`--version add` is the whole oracle: it makes the release message \
         \"add:<hash>\", which is byte-identical to the AddMaintainer authorization \
         MaintainerChangeData::signing_message(true) produces",
    );
    assert_names_field_and_shape(&err, "--version", "t13");

    // The hash half is a SECOND, independent barrier for the ProtocolActivation shape.
    assert!(
        validate_release_hash(&attacker.to_hex()).is_ok(),
        "t13/t11: honest limit — a 64-hex pubkey is byte-indistinguishable from a \
         SHA-256 digest, so the hash check CANNOT reject it. The version check is what \
         closes the AddMaintainer leg; asserting otherwise would be a false comfort"
    );
}

/// AUDIT-P0-011, the `remove` leg. [t04]
#[test]
fn p0_011_the_remove_action_word_is_refused() {
    assert_names_field_and_shape(
        &validate_release_version("remove")
            .expect_err("`remove:<64hex>` is a RemoveMaintainer authorization"),
        "--version",
        "t04",
    );
}

/// AUDIT-P0-011, the `v`-strip leg. [t05]
/// Acceptance: both CLIs strip a leading `v` BEFORE interpolating, so `--version vadd`
/// signs `"add:<hash>"` exactly as `--version add` does. Validating the raw argument
/// instead of the stripped one would leave the oracle wide open behind one character.
#[test]
fn p0_011_a_v_prefixed_action_word_is_refused_too() {
    assert_names_field_and_shape(
        &validate_release_version("vadd").expect_err(
            "misc.rs and cmd_governance.rs both strip the `v` before signing, so the \
             validation must apply to the STRIPPED string",
        ),
        "--version",
        "t05",
    );
}

/// AUDIT-P0-011, the ProtocolActivation leg. [t06/t10]
/// Acceptance: `--version "activate:7" --hash "1000"` yields `"activate:7:1000"`, which
/// is `ProtocolActivationData::signing_message()` for version 7 at epoch 1000. Both
/// halves refuse it independently.
#[test]
fn p0_011_the_protocol_activation_shape_is_refused_by_both_halves() {
    assert_names_field_and_shape(
        &validate_release_version("activate:7").expect_err("this is the activation prefix"),
        "--version",
        "t06",
    );
    assert_names_field_and_shape(
        &validate_release_hash("1000").expect_err("an epoch number is not a SHA-256 digest"),
        "--hash",
        "t10",
    );
}

/// [t07] Everything that is not `MAJOR.MINOR.PATCH` is refused.
#[test]
fn p0_011_non_semver_versions_are_refused() {
    for bad in [
        "",
        "v",
        "latest",
        "1.0",
        "1",
        "1.0.0.0",
        "1.0.x",
        "1. 0.0",
        "-1.0.0",
        "1.0.0 ",
        " 1.0.0",
        "1.0.-1",
        "a.b.c",
        "add:",
        ":",
        "1.0.0:extra",
    ] {
        assert!(
            validate_release_version(bad).is_err(),
            "t07: `{bad}` is not a release version. Every string the check admits is a \
             string an operator can be talked into signing"
        );
    }
}

/// [t10] Everything that is not exactly 64 hex characters is refused.
#[test]
fn p0_011_non_hex_hashes_are_refused() {
    let sixty_four = "a".repeat(64);
    for bad in [
        "".to_string(),
        "1000".to_string(),
        "a".repeat(63),
        "a".repeat(65),
        "g".repeat(64),
        format!("{} ", &sixty_four[..63]),
        format!("0x{}", &sixty_four[..62]),
    ] {
        assert!(
            validate_release_hash(&bad).is_err(),
            "t10: `{bad}` is not a 64-character hex digest and must be refused"
        );
    }
}

// ---------------------------------------------------------------------------
// GREEN-LOCKS — a validation that refuses genuine releases is an outage, not a
// control. Every one of these is a string a real maintainer types or a real
// CHECKSUMS.txt carries.
// ---------------------------------------------------------------------------

/// [t01/t02] Real published tags pass, and the returned value is the BARE form the CLIs
/// already interpolate — a valid invocation must sign exactly what it signs today.
#[test]
fn real_release_tags_still_validate_and_canonicalize_to_the_bare_form() {
    for (input, expected) in [
        ("6.24.1", "6.24.1"),
        ("v6.24.1", "6.24.1"),
        ("0.2.0", "0.2.0"),
        ("v0.2.0", "0.2.0"),
        ("9.9.9", "9.9.9"),
        ("10.0.123", "10.0.123"),
    ] {
        let got = validate_release_version(input)
            .unwrap_or_else(|e| panic!("`{input}` is a real tag shape and must pass: {e}"));
        assert_eq!(got, expected, "O2: the bare form is what gets signed");
    }
}

/// [t08/t09/t12] A real SHA-256 digest passes in either case, and the case is NOT folded.
///
/// `crates/updater/tests/inc_i_172_install_gate_binding.rs`
/// `req_172_006_v_prefix_and_hex_case_are_not_treated_as_tampering` locks the VERIFIER to
/// tolerating an uppercase digest, because real publishers emit one. The verifier
/// recomputes the message from the string in SIGNATURES.json, so lowercasing at signing
/// time would produce a signature over bytes the verifier never reconstructs — the
/// validation would break the releases it exists to protect.
#[test]
fn a_real_digest_passes_in_either_case_and_is_returned_unchanged() {
    let digest = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
    assert_eq!(
        validate_release_hash(digest).expect("a genuine sha256 digest must pass"),
        digest,
        "t08: O2 — returned unchanged"
    );

    let upper = digest.to_uppercase();
    assert_eq!(
        validate_release_hash(&upper).expect(
            "publishers emit uppercase digests and the install gate tolerates them; \
             refusing here would break a genuine release"
        ),
        upper,
        "t12: O2 — the case must NOT be folded, or the signed bytes stop matching what \
         the verifier reconstructs from SIGNATURES.json"
    );
}

// ---------------------------------------------------------------------------
// THE STANDING M3 OBLIGATION.
// ---------------------------------------------------------------------------

/// [t14] AUDIT-P0-011's other half, pinned as an executable fact.
///
/// This test PASSES today and is EXPECTED to pass: it asserts the collision is still
/// there. The M2 fix stops the shipped CLIs from constructing the colliding message; it
/// does not make the two message families distinguishable. Anything else that ever
/// raw-signs operator-supplied bytes with a maintainer key re-opens this.
///
/// When M3 lands domain separation, this test MUST flip to asserting NO collision. Its
/// failure at that point is the signal, not a regression.
#[test]
fn the_collision_still_exists_and_only_m3_closes_it() {
    let signer = kp(0x60);
    let target = *kp(0x61).public_key();

    // What a release-signing invocation with attacker arguments would have produced.
    let release_sig = sign_release_hash(&signer, "add", &target.to_hex());

    // What the governance path recomputes and verifies.
    let governance_message = MaintainerChangeData::new(target, vec![]).signing_message(true);

    let signature = crypto::Signature::from_hex(&release_sig.signature)
        .expect("sign_release_hash emits hex-encoded Ed25519");
    assert!(
        crypto::signature::verify(&governance_message, &signature, signer.public_key()).is_ok(),
        "t14: if this ever fails, the signing families have been domain-separated (M3) \
         and this assertion must be INVERTED. Until then the M2 argument validation is \
         the only thing standing between one signing command and a permanent maintainer \
         seat, and it must not be weakened."
    );
}
