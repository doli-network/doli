// INC-I-172 M1 — `doli upgrade` must ABORT on signature-verification failure.
// REQ-172-006 (Must), REQ-172-001 (Must)
//
// STATE: **RED**. This file compiles today; the RED is observable as test failures.
// `bins/cli/src/cmd_upgrade.rs:70-106` verifies signatures and then installs
// regardless: the comment at :70 says "informational — never blocks manual upgrade",
// every Err arm is a bare `println!`, and `install_binary` runs at :110 whatever the
// verdict was. The network is hardcoded to `doli_core::Network::Mainnet` at :82, so a
// testnet/devnet operator's release is checked against mainnet keys.
//
// ============================================================================
// OUTPUT CONTRACT
// ============================================================================
// Subject under test: the SOURCE of `bins/cli/src/cmd_upgrade.rs`, sliced to the
// body of `cmd_upgrade`. `doli-cli` is a BIN-ONLY crate (no `src/lib.rs`), so
// `cmd_upgrade` is `pub(crate)` and unreachable from an integration test; and the
// behavioural path requires a live GitHub release plus a writable install target.
// We therefore assert STRUCTURAL WIRING via `include_str!`, the convention already
// used by bins/cli/tests/logrotate_dropin_test.rs and delegation_bond_cap.rs.
//
// ENUMERATION OF OBSERVABLE OUTPUTS. The observable under a source-text subject is
// the text itself; there are no runtime params, receiver, or store writes. The
// outputs are the four properties the design (api-contract §5 / spec F6) requires
// the text to have:
//   O1: control flow on verification failure — an ABORT token (`return Err(` /
//       `bail!` / `?`) exists between the signature check and the install calls.
//       This is the whole finding: today there is none.
//   O2: ordering — that abort lies BEFORE `extract_named_binary_from_tarball` and
//       `install_binary` in the function body. Enforced structurally by slicing the
//       source between the verification landmark and the install landmark, so an
//       abort placed AFTER the install cannot satisfy O1 either. Ordering is real,
//       not whole-file substring luck.
//   O3: trust-root selection — no hardcoded `Network::Mainnet` in the body; the
//       network comes from the command.
//   O4: absence of the warn-and-continue markers — the specific strings that today
//       mark each fail-open arm ("informational — never blocks manual upgrade";
//       the "Warning: ..." arms; the "Note: no maintainer signatures" Ok(None) arm).
//
//   Declared limitation, stated rather than hidden: O4 is an ABSENCE assertion over
//   source text. A fix that keeps one of those exact wordings while also aborting
//   would fail this test spuriously. That is deliberate — each of those strings is
//   the user-visible promise that the upgrade proceeds anyway, so it must not
//   survive a fix that blocks. Every O4 assertion message says so explicitly.
//
// CODE PATHS (of `cmd_upgrade`, at the source level):
//   P1: whole `cmd_upgrade` body                        (O3, O4-a)
//   P2: verification block -> install landmark slice    (O1, O2, O4-b/c/d)
//   P3: install landmark onwards                        (control: install still exists,
//                                                        so P2's slice is a real prefix
//                                                        and not an empty accident)
//
// INPUT PARTITIONS: one — the committed source text. The subject is static text;
// there is no second input that could change which branch of the assertion is taken.
// A second partition would be provably blind.
//
// MATRIX (every cell asserted by the test named in it):
//
//  path | O1        | O2                | O3              | O4
//  -----|-----------|-------------------|-----------------|-------------------------
//  P1   | n/a       | n/a               | no Mainnet [t02]| no "informational" [t01]
//  P2   | abort[t03]| slice-is-pre-[t03]| n/a             | no warn/note arms  [t04]
//  P3   | n/a       | install exists    | n/a             | n/a                [t05]
// ============================================================================

const SRC: &str = include_str!("../src/cmd_upgrade.rs");

/// Landmark: where signature verification begins in `cmd_upgrade`.
const VERIFY_LANDMARK: &str = "download_signatures_json";
/// Landmark: the first irreversible act — pulling the binary out of the tarball.
const EXTRACT_LANDMARK: &str = "extract_named_binary_from_tarball";
/// Landmark: writing the binary over the running one.
const INSTALL_LANDMARK: &str = "install_binary";

/// Tokens that abort a `-> anyhow::Result<()>` function.
const ABORT_TOKENS: [&str; 4] = ["return Err(", "bail!(", "?;", "?)"];

/// Body of the top-level `cmd_upgrade` fn: from its signature to the next
/// top-level item. Brace counting is not usable here — the body is full of
/// `println!` format strings containing `{}` — so the boundary is the next
/// top-level `pub(crate)` item, which is a stable property of this file.
fn cmd_upgrade_body() -> &'static str {
    const SIG: &str = "pub(crate) async fn cmd_upgrade(";
    let start = SRC.find(SIG).unwrap_or_else(|| {
        panic!(
            "cmd_upgrade signature not found in cmd_upgrade.rs. If the fn was renamed \
             or its visibility changed, update SIG here — do not delete this test: \
             REQ-172-006 still requires the manual upgrade path to block on an \
             unverified release."
        )
    });
    let rest = &SRC[start + SIG.len()..];
    match rest.find("\npub(crate) ") {
        Some(i) => &rest[..i],
        None => rest,
    }
}

/// Source between the verification landmark and the first install landmark.
/// Slicing here is what makes the ordering assertion real: anything asserted on
/// this slice is, by construction, code that runs BEFORE the binary is extracted
/// or written.
fn pre_install_verification_slice() -> &'static str {
    let body = cmd_upgrade_body();
    let v = body.find(VERIFY_LANDMARK).unwrap_or_else(|| {
        panic!(
            "no `{VERIFY_LANDMARK}` call inside cmd_upgrade. The manual upgrade path \
             must still fetch and check maintainer signatures (REQ-172-006); removing \
             the check is not a valid way to make this test pass."
        )
    });
    let e = body.find(EXTRACT_LANDMARK).unwrap_or_else(|| {
        panic!("no `{EXTRACT_LANDMARK}` call inside cmd_upgrade — landmark lost")
    });
    assert!(
        v < e,
        "signature verification ({VERIFY_LANDMARK} at {v}) must appear BEFORE binary \
         extraction ({EXTRACT_LANDMARK} at {e}). Verifying after installing is not \
         verifying."
    );
    &body[v..e]
}

fn contains_abort(slice: &str) -> bool {
    ABORT_TOKENS.iter().any(|t| slice.contains(t))
}

// ---------------------------------------------------------------------------

/// REQ-172-006 (Must). RED.
/// Acceptance: the "advisory verification" contract is gone from the source.
/// [P1 -> O4-a]
///
/// The comment at cmd_upgrade.rs:70 is not decoration — it is the stated design of
/// the defect, and it is the reason the Err arms below it are `println!`s. It must
/// not survive a fix that blocks.
#[test]
fn req_172_006_advisory_verification_comment_is_gone() {
    let body = cmd_upgrade_body();
    assert!(
        !body.contains("informational — never blocks manual upgrade"),
        "cmd_upgrade still declares maintainer-signature checking \"informational — \
         never blocks manual upgrade\". `doli upgrade` is the documented INC-I-153 \
         remediation path and runs as root on producer hosts; an advisory verify that \
         installs anyway is not a control (spec F6). Remove the comment AND the \
         behaviour it describes."
    );
}

/// REQ-172-001 (Must) / REQ-172-006 (Must). RED.
/// Acceptance: the trust root is chosen from the network the command was invoked
/// with, not pinned to Mainnet in the source.
/// [P1 -> O3]
///
/// `verify_release_signatures(&sig_release, doli_core::Network::Mainnet)` at :82
/// means a testnet or devnet operator's release is checked against the MAINNET
/// bootstrap keys. Those two arrays happen to be byte-identical today
/// (constants.rs:37-48 vs :56-67), which is the FM-12 cross-network replay hazard —
/// so the bug is currently invisible at runtime and can only be caught here.
#[test]
fn req_172_001_upgrade_does_not_hardcode_the_mainnet_trust_root() {
    let body = cmd_upgrade_body();
    assert!(
        !body.contains("Network::Mainnet"),
        "cmd_upgrade still hardcodes `Network::Mainnet` for signature verification. \
         The network must come from the command argument (api-contract §5), otherwise \
         a testnet/devnet upgrade is authorised by the mainnet trust root."
    );
}

/// REQ-172-006 (Must). RED.
/// Acceptance: a verification failure ABORTS, and the abort is positioned before
/// the binary is extracted or installed.
/// [P2 -> O1, O2]
#[test]
fn req_172_006_verification_failure_aborts_before_install() {
    let slice = pre_install_verification_slice();
    assert!(
        contains_abort(slice),
        "the signature-verification block in cmd_upgrade contains no abort ({:?}) \
         between `{VERIFY_LANDMARK}` and `{EXTRACT_LANDMARK}`. Today every failure arm \
         is a bare println! and the install proceeds. A failed verification MUST return \
         an error and abort (spec F6, api-contract §5).\n--- slice ---\n{slice}",
        ABORT_TOKENS
    );
}

/// REQ-172-006 (Must). RED.
/// Acceptance: none of the three warn-and-continue arms survive — an unsigned
/// release, an under-signed release, and an unreachable SIGNATURES.json each block.
/// [P2 -> O4-b, O4-c, O4-d]
///
/// Each string below is the user-visible promise that the upgrade continues anyway.
/// If a fix keeps one of these wordings while aborting, update the string here — the
/// requirement is that no path through this block reaches the install.
#[test]
fn req_172_006_no_warn_and_continue_arms_remain() {
    let slice = pre_install_verification_slice();

    // O4-b: `Ok(None)` — no SIGNATURES.json at all. An unsigned release is not a
    // verified release; api-contract §5 says this arm blocks too.
    assert!(
        !slice.contains("Note: no maintainer signatures"),
        "the `Ok(None)` arm still notes the absence of SIGNATURES.json and proceeds. \
         An unsigned release must ABORT (api-contract §5: \"`Ok(None)` (no \
         SIGNATURES.json) also blocks — an unsigned release is not a verified \
         release\"). If an operator override is wanted it must be an explicit, \
         documented flag, not the default."
    );

    // O4-c: `InsufficientSignatures` and the catch-all `Err(e)` verification arms.
    assert!(
        !slice.contains("Warning: only "),
        "the InsufficientSignatures arm still prints \"Warning: only N/M required \
         maintainer signatures found\" and installs. Below-threshold means REFUSE."
    );
    assert!(
        !slice.contains("Warning: signature verification failed"),
        "the catch-all verification Err arm still warns and installs. A failed \
         verification must abort."
    );

    // O4-d: the outer fetch error — could not reach SIGNATURES.json at all.
    assert!(
        !slice.contains("Note: could not check signatures"),
        "the signature-FETCH error arm still proceeds to install. \"I could not check\" \
         is not \"it is fine\": a network-level failure to retrieve SIGNATURES.json is \
         indistinguishable from an attacker withholding it, so it must abort."
    );
}

/// REQ-172-006 (Must). GREEN-lock (control).
/// Acceptance: the install landmarks still exist and still follow verification — so
/// the slice asserted above is a genuine prefix of the install path and not an
/// accidental empty region.
/// [P3 -> O2]
#[test]
fn req_172_006_install_path_still_exists_after_verification() {
    let body = cmd_upgrade_body();
    let v = body.find(VERIFY_LANDMARK).expect("verification landmark");
    let e = body.find(EXTRACT_LANDMARK).expect("extract landmark");
    let i = body.find(INSTALL_LANDMARK).expect("install landmark");

    assert!(
        v < e && e < i,
        "expected order verify({v}) < extract({e}) < install({i}) inside cmd_upgrade; \
         the upgrade command must still be able to install once verification passes"
    );
    assert!(
        !pre_install_verification_slice().is_empty(),
        "the verification slice must be non-empty, otherwise every assertion over it \
         is vacuous"
    );
}
