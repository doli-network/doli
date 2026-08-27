// INC-I-172 M1 — `doli-node upgrade` must VERIFY maintainer signatures and ABORT.
// REQ-172-006 (Must), REQ-172-001 (Must). Source: QA ISSUE-001, api-contract §10.
//
// THE DEFECT this locks shut: `bins/node/src/commands/misc.rs::handle_upgrade_command`
// is the FOURTH install path. It downloaded a tarball, checked it against
// `release_info.expected_hash`, and called `updater::install_binary` — with no
// signature verification anywhere on the path. The checksum is not an independent
// control: `expected_hash` is parsed from CHECKSUMS.txt fetched from the SAME GitHub
// release as the tarball, so an origin that can serve a malicious binary serves its
// hash too. With the other three paths now fail-closed, an operator whose upgrade
// correctly refuses reaches for the one command that still installs anything, so this
// path's share of upgrade traffic goes UP as a direct result of the M1 diff.
//
// ============================================================================
// OUTPUT CONTRACT
// ============================================================================
// Subject under test: the SOURCE of `bins/node/src/commands/misc.rs`, sliced to the
// body of `handle_upgrade_command`. `doli-node`'s lib (`bins/node/src/lib.rs`) exposes
// `config, metrics, node, producer, updater` — NOT `commands` — so the fn is
// unreachable from an integration test; and the behavioural path needs a live GitHub
// release plus a writable install target. We therefore assert STRUCTURAL WIRING via
// `include_str!`, the convention already used by
// bins/node/tests/inc_i_172_update_cmd_verify_blocks_test.rs and
// bins/cli/tests/inc_i_172_upgrade_verify_blocks_test.rs.
//
// ENUMERATION OF OBSERVABLE OUTPUTS. The observable under a source-text subject is the
// text itself: no runtime params, no receiver, no store writes. The required
// properties are:
//   O1: a maintainer-signature verification call exists inside the fn body at all.
//       This is the whole finding — today there is none.
//   O2: control flow on verification failure — an ABORT token (`return Err(` /
//       `bail!` / `?`) exists in the region between the verification call and the
//       first irreversible act.
//   O3: ordering — verify < extract < backup < install, enforced by slicing the body
//       between the verification landmark and the extract landmark. Anything asserted
//       on that slice is BY CONSTRUCTION code that runs before the binary is pulled
//       out of the tarball or written over the running one. This is what stops the
//       assertions from being whole-file substring luck: `misc.rs` also contains
//       `handle_release_command`, which is full of unrelated `?;` tokens, so an
//       unsliced search would pass on a file with no gate at all.
//   O4: trust-root selection — the call passes the in-scope `network` and the body
//       hardcodes no `Network::Mainnet` (FM-12 cross-network replay: a devnet
//       operator's release checked against the mainnet root).
//   O5: an ABSENT `SIGNATURES.json` blocks too — the `Option` returned by
//       `download_signatures_json` is converted into an error inside the pre-install
//       slice, not unwrapped-or-skipped. An unsigned release is not a verified one.
//
//   Declared limitation, stated rather than hidden: these are assertions over source
//   text. A restructuring that keeps the property but moves the landmarks will fail
//   them; every assertion message therefore names the property and says to re-anchor
//   rather than delete. O5 in particular cannot distinguish `ok_or_else(...)?` from
//   any other Option→Result conversion; it asserts that the Option is not silently
//   discarded, which is the defect class.
//
//   NOT asserted, and why: the real end-to-end observable is the process exit code of
//   `doli-node upgrade` against an unsigned release. Reaching it needs the command to
//   contact GitHub, which makes the test pass or fail on connectivity rather than on
//   the defect, and needs a writable copy of the running binary. Recorded here as a
//   residual for live QA probing, exactly as the sibling test does.
//
// CODE PATHS (of `handle_upgrade_command`, at the source level):
//   P1: whole `handle_upgrade_command` body                     (O1, O4)
//   P2: verification landmark -> extract landmark slice         (O2, O4, O5)
//   P3: extract landmark onwards                                (control: the install
//                                                                path still exists, so
//                                                                P2 is a real prefix
//                                                                and not an empty
//                                                                accident)
//   P4: the sibling fn `handle_release_command`                 (negative control: it
//                                                                proves the slice is
//                                                                doing the work — its
//                                                                `?;` tokens must NOT
//                                                                be able to satisfy P2)
//
// INPUT PARTITIONS: one — the committed source text. The subject is static; there is
// no second input that could change which assertion branch is taken. A second
// partition would be provably blind.
//
// MATRIX (every cell asserted by the test named in it):
//
//  path | O1         | O2          | O3               | O4               | O5
//  -----|------------|-------------|------------------|------------------|-----------
//  P1   | verify[t01]| n/a         | n/a              | no Mainnet [t03] | n/a
//  P2   | n/a        | abort [t02] | slice-is-pre[t02]| network arg [t03]| block [t04]
//  P3   | n/a        | n/a         | order      [t05] | n/a              | n/a
//  P4   | n/a        | n/a         | disjoint   [t05] | n/a              | n/a
// ============================================================================

const SRC: &str = include_str!("../src/commands/misc.rs");

/// Landmark: where signature verification begins in `handle_upgrade_command`.
const VERIFY_LANDMARK: &str = "download_signatures_json";
/// Landmark: the verification call itself.
const VERIFY_CALL: &str = "verify_release";
/// Landmark: the first irreversible act — pulling the binary out of the tarball.
const EXTRACT_LANDMARK: &str = "extract_binary_from_tarball";
/// Landmark: moving the currently running binary aside.
const BACKUP_LANDMARK: &str = "backup_current";
/// Landmark: writing the new binary over the running one.
const INSTALL_LANDMARK: &str = "install_binary";

/// Tokens that abort a `-> anyhow::Result<()>` function.
const ABORT_TOKENS: [&str; 4] = ["return Err(", "bail!(", "?;", "?)"];

/// Body of the top-level `handle_upgrade_command` fn: from its signature to the next
/// top-level item. Brace counting is not usable here — the body is full of `println!`
/// format strings containing `{}` — so the boundary is the next top-level `pub(crate)`
/// item, which is a stable property of this file. `handle_upgrade_command` is the last
/// item today, so the slice runs to EOF; the search keeps it correct if one is added.
fn upgrade_command_body() -> &'static str {
    const SIG: &str = "pub(crate) async fn handle_upgrade_command(";
    let start = SRC.find(SIG).unwrap_or_else(|| {
        panic!(
            "`handle_upgrade_command` signature not found in commands/misc.rs. If the fn \
             was renamed, moved, or its parameter list reformatted, re-anchor SIG here — \
             do NOT delete this test: REQ-172-006 still requires `doli-node upgrade` to \
             refuse an unverified release (api-contract §10)."
        )
    });
    let rest = &SRC[start + SIG.len()..];
    match rest.find("\npub(crate) ") {
        Some(i) => &rest[..i],
        None => rest,
    }
}

/// Body of the sibling `handle_release_command`, used as a NEGATIVE CONTROL. It is full
/// of `?;` and `return Err(` tokens that have nothing to do with the install gate; if a
/// future edit widened the slice to the whole file, these would silently satisfy the
/// abort assertion and the test would pass on an ungated upgrade path.
fn release_command_body() -> &'static str {
    const SIG: &str = "pub(crate) async fn handle_release_command(";
    let start = SRC
        .find(SIG)
        .expect("`handle_release_command` not found in commands/misc.rs — negative control lost");
    let rest = &SRC[start + SIG.len()..];
    match rest.find("\npub(crate) ") {
        Some(i) => &rest[..i],
        None => rest,
    }
}

/// Source between the verification landmark and the first irreversible act.
/// Slicing here is what makes the ordering assertion real: anything asserted on this
/// slice is, by construction, code that runs BEFORE the binary is extracted, backed up
/// or written.
fn pre_install_verification_slice() -> &'static str {
    let body = upgrade_command_body();
    let v = body.find(VERIFY_LANDMARK).unwrap_or_else(|| {
        panic!(
            "no `{VERIFY_LANDMARK}` call inside `handle_upgrade_command`. This path must \
             fetch and check maintainer signatures before installing (api-contract §10); \
             the tarball checksum is NOT an independent control — `expected_hash` comes \
             from CHECKSUMS.txt in the same GitHub release, so a compromised origin owns \
             both the binary and its hash. Removing the check is not a valid way to make \
             this test pass."
        )
    });
    let e = body.find(EXTRACT_LANDMARK).unwrap_or_else(|| {
        panic!("no `{EXTRACT_LANDMARK}` call inside `handle_upgrade_command` — landmark lost")
    });
    assert!(
        v < e,
        "signature verification (`{VERIFY_LANDMARK}` at {v}) must appear BEFORE binary \
         extraction (`{EXTRACT_LANDMARK}` at {e}). Verifying after installing is not \
         verifying."
    );
    &body[v..e]
}

fn contains_abort(slice: &str) -> bool {
    ABORT_TOKENS.iter().any(|t| slice.contains(t))
}

// ---------------------------------------------------------------------------

/// REQ-172-006 (Must). RED before the fix.
/// Acceptance: the fourth install path performs maintainer-signature verification at
/// all. Before this change `handle_upgrade_command` contained no call to any verifier,
/// which is precisely why the design's "three verification call sites" survey never saw
/// it — a path that never calls a verifier is invisible to a survey of verifier callers.
/// [P1 -> O1]
#[test]
fn req_172_006_node_upgrade_verifies_maintainer_signatures() {
    let body = upgrade_command_body();

    assert!(
        body.contains(VERIFY_CALL),
        "`doli-node upgrade` contains no `{VERIFY_CALL}*` call. It downloads a tarball, \
         checks only `release_info.expected_hash`, and calls `{INSTALL_LANDMARK}`. \
         Shipping the other three paths fail-closed while this one installs anything is \
         a control with a documented one-command bypass (api-contract §10).\n--- body \
         ---\n{body}"
    );
    assert!(
        body.contains(VERIFY_LANDMARK),
        "`doli-node upgrade` must fetch SIGNATURES.json (`{VERIFY_LANDMARK}`) — there is \
         nothing to verify against otherwise"
    );
}

/// REQ-172-006 (Must). RED before the fix.
/// Acceptance: a verification failure ABORTS, and the abort is positioned before the
/// binary is extracted, backed up or installed.
/// [P2 -> O2, O3]
#[test]
fn req_172_006_node_upgrade_verification_failure_aborts_before_install() {
    let slice = pre_install_verification_slice();

    assert!(
        contains_abort(slice),
        "the signature-verification block in `handle_upgrade_command` contains no abort \
         ({:?}) between `{VERIFY_LANDMARK}` and `{EXTRACT_LANDMARK}`. A failed \
         verification MUST return an error and abort before anything is written \
         (api-contract §10).\n--- slice ---\n{slice}",
        ABORT_TOKENS
    );

    // Stronger than "an abort exists somewhere in the slice": the abort must follow the
    // verification CALL. An abort that only guards the SIGNATURES.json fetch would leave
    // a failed verification falling through to the install.
    let after_call = slice
        .find(VERIFY_CALL)
        .map(|i| &slice[i..])
        .unwrap_or_else(|| {
            panic!(
                "`{VERIFY_CALL}*` is not inside the pre-install slice, so its result cannot \
             be gating the install. slice:\n{slice}"
            )
        });
    assert!(
        contains_abort(after_call),
        "the `{VERIFY_CALL}*` call in `handle_upgrade_command` is not followed by an \
         abort ({:?}) before `{EXTRACT_LANDMARK}`. Computing a verdict and then \
         installing regardless is the INC-I-153 shape this milestone exists to \
         remove.\n--- after call ---\n{after_call}",
        ABORT_TOKENS
    );
}

/// REQ-172-001 (Must). RED before the fix.
/// Acceptance: the trust root follows the network the command was invoked with. The
/// mainnet and testnet bootstrap arrays are byte-identical today
/// (`crates/updater/src/constants.rs`), so a hardcoded root is invisible at runtime and
/// can only be caught structurally — that is the FM-12 cross-network replay hazard.
/// [P1 -> O4] and [P2 -> O4]
#[test]
fn req_172_001_node_upgrade_uses_the_invoked_network_not_a_hardcoded_root() {
    let body = upgrade_command_body();
    assert!(
        !body.contains("Network::Mainnet"),
        "`handle_upgrade_command` must not pin the trust root to Mainnet; the network \
         comes from the invoking `--network` flag (api-contract §8 G2, §10)"
    );

    let slice = pre_install_verification_slice();
    assert!(
        slice.contains("network)") || slice.contains("network,"),
        "the verification call must pass the in-scope `network`; slice:\n{slice}"
    );
}

/// REQ-172-006 (Must). RED before the fix.
/// Acceptance: an ABSENT or empty `SIGNATURES.json` blocks. `download_signatures_json`
/// returns `Ok(None)` for a release with no signatures file at all; treating that as
/// "nothing to check, carry on" is how an attacker publishes an unsigned release and
/// has it installed. The `Option` must become an error inside the pre-install slice.
/// [P2 -> O5]
#[test]
fn req_172_006_node_upgrade_blocks_when_signatures_json_is_absent() {
    let slice = pre_install_verification_slice();

    assert!(
        slice.contains("ok_or_else(") || slice.contains("ok_or("),
        "the `Option` returned by `{VERIFY_LANDMARK}` is not converted into an error \
         inside the pre-install region. `Ok(None)` means the release has NO \
         SIGNATURES.json; an unsigned release is not a verified release and must block \
         (api-contract §5, §10).\n--- slice ---\n{slice}"
    );

    // And the conversion must itself abort, not merely produce a Result that is dropped.
    let idx = slice
        .find("ok_or")
        .expect("checked above that an ok_or* conversion exists");
    assert!(
        contains_abort(&slice[idx..]),
        "the absent-SIGNATURES.json arm produces an error but never propagates it \
         before `{EXTRACT_LANDMARK}`.\n--- after ---\n{}",
        &slice[idx..]
    );
}

/// REQ-172-006 (Must). GREEN-lock (control).
/// Acceptance: (a) the install landmarks still exist and still follow verification, so
/// the slice asserted above is a genuine prefix of the install path and not an empty
/// accident; and (b) the slice is disjoint from `handle_release_command`, so that fn's
/// unrelated `?;` tokens cannot be what satisfies the abort assertions.
/// [P3 -> O3] and [P4 -> O3]
#[test]
fn req_172_006_install_path_still_exists_after_verification() {
    let body = upgrade_command_body();
    let v = body.find(VERIFY_LANDMARK).expect("verification landmark");
    let e = body.find(EXTRACT_LANDMARK).expect("extract landmark");
    let b = body.find(BACKUP_LANDMARK).expect("backup landmark");
    let i = body.find(INSTALL_LANDMARK).expect("install landmark");

    assert!(
        v < e && e < b && b < i,
        "expected order verify({v}) < extract({e}) < backup({b}) < install({i}) inside \
         `handle_upgrade_command`; the command must still be able to install once \
         verification passes, and must not touch the running binary before it does"
    );

    let slice = pre_install_verification_slice();
    assert!(
        !slice.is_empty(),
        "the verification slice must be non-empty, otherwise every assertion over it is \
         vacuous"
    );

    // Negative control: the sibling fn is full of abort tokens and is NOT part of the
    // slice. If this ever fails, the slice has widened and the abort assertions above
    // have stopped proving anything about the install gate.
    let sibling = release_command_body();
    assert!(
        contains_abort(sibling),
        "negative control is broken: `handle_release_command` was expected to contain \
         abort tokens of its own"
    );
    assert!(
        !sibling.contains(INSTALL_LANDMARK),
        "the negative control must not overlap the install path"
    );
    assert!(
        !slice.contains("Fetching checksums from"),
        "the pre-install slice has widened into `handle_release_command`; re-anchor the \
         landmarks — the abort assertions are no longer specific to the install gate"
    );
}
