// INC-I-172 M1 — `doli-node update verify` must FAIL when verification fails.
// REQ-172-006 (Must), REQ-172-001 (Must)
//
// STATE: **RED**. This file compiles today; the RED is observable as test failures.
// `bins/node/src/commands/update.rs:384-400`: the `Err(e)` arm of
// `verify_release_signatures` prints "❌ Verification failed" inside a decorative box
// and the command still returns `Ok(())`. An operator scripting
// `doli-node update verify vX && doli upgrade` gets exit 0 for a release whose
// signatures did not verify, which makes the verify step worse than useless — it
// launders an unverified release as a checked one.
//
// ============================================================================
// OUTPUT CONTRACT
// ============================================================================
// Subject under test: the SOURCE of `bins/node/src/commands/update.rs`, sliced to
// the maintainer-signature region of `handle_update_command`. The fn is
// `pub(crate)` in a bin-only module tree (`bins/node/src/lib.rs` exposes
// `config, metrics, node, producer, updater` — NOT `commands`), and the runtime
// path needs a live GitHub release, so this uses the `include_str!` structural
// convention (bins/cli/tests/logrotate_dropin_test.rs).
//
// ENUMERATION OF OBSERVABLE OUTPUTS. Source text is the subject: no runtime
// params, no receiver, no store writes. The required properties are:
//   O1: control flow on verification failure — an ABORT/propagate token exists in
//       the verification region, so the process exit status can differ from 0.
//       Today: none; every arm falls through to `Ok(())` at the fn tail.
//   O2: trust-root selection — the call passes the in-scope `network`, and no
//       hardcoded `Network::Mainnet` appears in the region.
//   O3: absence of the print-and-succeed marker — the "Verification failed" arm
//       must not be the terminal handling of a failure.
//
//   Declared limitation: O3 is an ABSENCE assertion over source text. A fix that
//   keeps the exact banner wording while also returning Err would fail it
//   spuriously. That is deliberate: the banner is currently the ENTIRE failure
//   handling. The assertion message says so.
//
//   NOT asserted, and why: the real end-to-end observable is the process exit code
//   of `doli-node update verify`. Reaching it requires the command to contact
//   GitHub for a release, which makes the test network-dependent and
//   non-hermetic — it would pass or fail on connectivity rather than on the
//   defect. Recorded as a residual cell in
//   docs/.workflow/inc-i-172-M1-test-plan.md for the QA agent to probe live.
//
// CODE PATHS (at the source level):
//   P1: the maintainer-signature region — from the "MAINTAINER SIGNATURES" banner
//       to the release-not-found `None =>` arm.   (O1, O2, O3)
//   P2: the whole `handle_update_command` body.   (O2 — no Mainnet anywhere)
//
// INPUT PARTITIONS: one — the committed source text (static; no second input can
// change which assertion branch is taken).
//
// MATRIX (every cell asserted by the test named in it):
//
//  path | O1          | O2                 | O3
//  -----|-------------|--------------------|--------------------------
//  P1   | abort [t01] | network arg  [t02] | no print-only fail  [t03]
//  P2   | n/a         | no Mainnet   [t02] | n/a
// ============================================================================

const SRC: &str = include_str!("../src/commands/update.rs");

/// Start of the maintainer-signature region inside the `verify` subcommand.
const REGION_START: &str = "MAINTAINER SIGNATURES";
/// End of that region: the arm handling "release not found".
const REGION_END: &str = "None => {";

const ABORT_TOKENS: [&str; 5] = ["return Err(", "bail!(", "?;", "?)", "anyhow!("];

/// The maintainer-signature region of `handle_update_command`. Slicing here is
/// what makes the assertions specific: `handle_update_command` is the only
/// top-level fn in this 422-line file, so a whole-file search would be satisfied
/// by an unrelated `?` elsewhere in the command dispatcher.
fn signature_region() -> &'static str {
    let s = SRC.find(REGION_START).unwrap_or_else(|| {
        panic!(
            "the `{REGION_START}` banner is gone from commands/update.rs. If the verify \
             subcommand was restructured, re-anchor this test — do not delete it: \
             REQ-172-006 still requires `update verify` to FAIL on a bad signature."
        )
    });
    let rest = &SRC[s..];
    match rest.find(REGION_END) {
        Some(i) => &rest[..i],
        None => rest,
    }
}

// ---------------------------------------------------------------------------

/// REQ-172-006 (Must). RED.
/// Acceptance: a failed signature verification produces an error result, not a
/// printed banner followed by success.
/// [P1 -> O1, O3]
#[test]
fn req_172_006_update_verify_failure_returns_an_error() {
    let region = signature_region();

    assert!(
        region.contains("verify_release"),
        "the verify subcommand must still perform signature verification; \
         no `verify_release*` call found in the region"
    );

    assert!(
        ABORT_TOKENS.iter().any(|t| region.contains(t)),
        "the maintainer-signature region of `update verify` contains no error-producing \
         token ({:?}). Today the Err arm only prints \
         \"❌ Verification failed\" and `handle_update_command` still returns Ok(()), so \
         the process exits 0 on an UNVERIFIED release. An operator who scripts \
         `doli-node update verify vX && doli upgrade` is told the release checked out \
         when it did not (api-contract §5: \"same treatment\").\n--- region ---\n{region}",
        ABORT_TOKENS
    );
}

/// REQ-172-001 (Must). GREEN-lock.
/// Acceptance: the trust root follows the invoked network — `update.rs` already has
/// `network` in scope and passes it. This must not regress into the hardcoded
/// mainnet root that `cmd_upgrade.rs` has.
/// [P1 -> O2] and [P2 -> O2]
#[test]
fn req_172_001_update_verify_uses_the_invoked_network_not_a_hardcoded_root() {
    let region = signature_region();
    assert!(
        !region.contains("Network::Mainnet"),
        "the verify region must not pin the trust root to Mainnet; it already has the \
         invoked `network` in scope (api-contract §5)"
    );
    assert!(
        !SRC.contains("Network::Mainnet"),
        "commands/update.rs must not hardcode `Network::Mainnet` anywhere — a \
         testnet/devnet operator would be verifying against the mainnet trust root \
         (FM-12 cross-network replay)"
    );
    assert!(
        region.contains(", network)") || region.contains("network)"),
        "the verification call must pass the in-scope `network`; region:\n{region}"
    );
}

/// REQ-172-006 (Must). RED.
/// Acceptance: the success banner is not reachable from a failed verification, and
/// the failure banner is not the terminal handling.
/// [P1 -> O3]
#[test]
fn req_172_006_failure_is_not_handled_by_a_banner_alone() {
    let region = signature_region();

    // Locate the failure banner. If it is still present, an abort token must
    // appear after it inside the same region.
    if let Some(idx) = region.find("Verification failed") {
        let after = &region[idx..];
        assert!(
            ABORT_TOKENS.iter().any(|t| after.contains(t)),
            "\"Verification failed\" is printed and then execution continues to the \
             end of `handle_update_command`, which returns Ok(()). Printing a red cross \
             inside a box is not a failure: the command must return an error so the \
             shell sees a non-zero exit.\n--- after banner ---\n{after}"
        );
    }
}

// ---------------------------------------------------------------------------
// F2 / F3 (review pass 1) — the `Apply` arm is the FIFTH install path
// ---------------------------------------------------------------------------

/// The `UpdateCommands::Apply` arm, from its match label to the next one.
///
/// Wiring guard only: the behaviour it guards — that `apply_update` refuses a release
/// whose signers are no longer in the root, before any download — is asserted for real
/// in `crates/updater/tests/inc_i_172_apply_update_gate.rs`. What source text CAN prove,
/// and what that behavioural test cannot, is that this call site actually supplies a
/// root resolved from THIS HOST's on-chain set rather than a bootstrap one.
fn apply_arm() -> &'static str {
    const LABEL: &str = "UpdateCommands::Apply {";
    let start = SRC.find(LABEL).unwrap_or_else(|| {
        panic!(
            "`{LABEL}` not found in commands/update.rs. If the arm was renamed or moved, \
             re-anchor this test — do NOT delete it: `doli-node update apply` installs \
             binaries and must not do so under revoked maintainer keys (F2)."
        )
    });
    let rest = &SRC[start + LABEL.len()..];
    match rest.find("\n        UpdateCommands::") {
        Some(i) => &rest[..i],
        None => rest,
    }
}

/// REQ-172-006 (Must). RED before this fix.
/// Acceptance: the apply arm resolves a trust root from the host's data dir and hands it
/// to `apply_update`. Before this change the arm called `apply_update` with four
/// arguments and no root at all — a fifth install path with zero signature verification.
#[test]
fn f2_apply_arm_resolves_a_trust_root_and_passes_it_to_apply_update() {
    let arm = apply_arm();

    assert!(
        arm.contains("command_trust_root(data_dir, network)"),
        "the apply arm must resolve the trust root from THIS host's maintainer_state.bin \
         (`command_trust_root`), not from the compiled bootstrap keys (F3). The data dir \
         and the network are both already in scope.\n--- arm ---\n{arm}"
    );

    let apply_call = arm.find("apply_update(").unwrap_or_else(|| {
        panic!("no `apply_update(` call in the apply arm — landmark lost\n{arm}")
    });
    let resolve = arm
        .find("command_trust_root(")
        .expect("checked above that the resolution exists");
    assert!(
        resolve < apply_call,
        "the trust root must be resolved BEFORE `apply_update` is called ({resolve} vs \
         {apply_call}); resolving it afterwards cannot gate anything"
    );
    assert!(
        arm[apply_call..].contains("&root"),
        "the resolved root must be PASSED to `apply_update`. A root that is computed and \
         then dropped is the shape this milestone exists to remove.\n--- after call ---\n{}",
        &arm[apply_call..]
    );
}

/// REQ-172-006 (Must). RED before this fix.
/// Acceptance: `--force` does not skip the trust-root resolution. The arm computes
/// `approved || force` for the APPROVAL check; the root must be resolved outside any
/// branch that flag can steer.
#[test]
fn f2_force_does_not_route_around_the_trust_root() {
    let arm = apply_arm();
    let force = arm
        .find("pending.approved || force")
        .expect("the `--force` compound is the landmark for the approval waiver");
    let resolve = arm
        .find("command_trust_root(")
        .expect("the apply arm must resolve a trust root");
    assert!(
        force < resolve,
        "the trust root is resolved before `--force` is even computed, which would put it \
         inside whatever branch the flag steers. It must be resolved on the unconditional \
         path so `--force` can only waive community APPROVAL, never maintainer authority."
    );
}
