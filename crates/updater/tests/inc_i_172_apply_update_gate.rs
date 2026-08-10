// INC-I-172 M1 review pass 1, [F2] MAJOR — `apply_update` (the `doli-node update apply`
// path) must re-verify against the CURRENT trust root before it installs anything.
// REQ-172-006 (Must).
//
// WHY THIS FILE EXISTS. `apply_update` was the FIFTH install path and it performed
// exactly two checks — veto period and approval — then downloaded and installed. The
// F7(a) re-verification landed only in `UpdateService::auto_apply`, so a pending update
// staged before a maintainer-key rotation, then applied by hand, installed under the
// REVOKED signers. Revocation that cannot reach the manual apply path is not revocation.
//
// ============================================================================
// OUTPUT CONTRACT
// ============================================================================
// Function under test:
//   `updater::apply_update(release: &Release, first_notified_at: u64, approved: bool,
//        veto_percent: Option<u8>, root: &TrustRoot) -> Result<(), UpdateError>`
//
// ENUMERATION OF OBSERVABLE OUTPUTS.
//   - return value     : the Result. O1 (discriminant) + O2 (Err variant identity).
//   - persistent store : the INSTALLED BINARY. This is the output that matters and it
//     is the one a hermetic test must never produce. It is observed INDIRECTLY and
//     soundly through O2: every write in this function is downstream of
//     `auto_apply_from_github`, which begins with a network fetch. So a refusal
//     reported as `TrustRootUnavailable` / `InsufficientSignatures` proves the function
//     returned BEFORE the first network call, and therefore before backup, extract or
//     install. A missing gate cannot produce those variants — it would report
//     `DownloadFailed` / `Network` (or succeed). Asserting the exact variant is what
//     makes this test a discriminator rather than "it errored".
//   - mutable params   : NONE (all shared refs).
//   - side channel     : `tracing` records. DECLARED UNASSERTED — same rationale as
//     the sibling files; the facts logged are carried in the returned variant.
//
//   O1: Result discriminant     — Ok / Err.
//   O2: Err variant identity    — VetoPeriodActive / NotApproved / TrustRootUnavailable
//                                 / InsufficientSignatures / DownloadFailed. The last
//                                 one is the FAILURE signal for this file: reaching the
//                                 network means the trust-root gate did not run.
//
// CODE PATHS:
//   P1: veto period still running                       -> VetoPeriodActive
//   P2: veto ended, not approved                        -> NotApproved
//   P3: veto ended, approved, root does not trust the signers -> InsufficientSignatures
//   P4: veto ended, approved, root emptied (revoked)    -> TrustRootUnavailable
//   (the success path is NOT exercised: it downloads from GitHub and overwrites the
//    running binary. Non-hermetic and destructive; covered by
//    `inc_i_172_install_gate_binding.rs` for the artifact chain and by the wiring test
//    for the call site.)
//
// INPUT PARTITIONS:
//   I1: the staged release is genuinely signed by keys that WERE trusted — the rotation
//       scenario. The signatures are real Ed25519 signatures over the real message, so a
//       refusal cannot be blamed on a malformed fixture.
//   I2: `--force` set (approved = true through the force flag). `--force` waives
//       community APPROVAL; the test pins that it does not waive maintainer authority.
// ============================================================================

use updater::{apply_update, sign_release_hash, Release, TrustRoot, UpdateError};

const VERSION: &str = "8.8.8";
const CHECKSUMS_SHA256: &str = "aa11bb22cc33dd44ee55ff6600778899aabbccddeeff00112233445566778899";

fn kp(seed: u8) -> crypto::KeyPair {
    crypto::KeyPair::from_private_key(crypto::PrivateKey::from_bytes([seed; 32]))
}

/// A staged release, genuinely signed by `signers`, exactly as it would sit in
/// `pending_update.json`.
fn staged_release(signers: &[&crypto::KeyPair]) -> Release {
    Release {
        version: VERSION.to_string(),
        binary_sha256: CHECKSUMS_SHA256.to_string(),
        // Deliberately unroutable: if a regression removes the gate, the function
        // proceeds to I/O and fails fast and locally instead of reaching the internet.
        binary_url_template: "http://127.0.0.1:1/{platform}".to_string(),
        changelog: String::new(),
        published_at: 0,
        signatures: signers
            .iter()
            .map(|k| sign_release_hash(k, VERSION, CHECKSUMS_SHA256))
            .collect(),
        target_networks: Vec::new(),
    }
}

/// A `first_notified_at` far enough in the past that the veto window has certainly
/// closed on every network profile.
fn long_ago() -> u64 {
    1
}

/// The two variants that mean "the gate did not run".
fn assert_not_an_io_error(err: &UpdateError, ctx: &str) {
    assert!(
        !matches!(
            err,
            UpdateError::DownloadFailed(_) | UpdateError::Network(_) | UpdateError::Io(_)
        ),
        "{ctx}: apply_update reached I/O ({err}). The trust-root check must happen \
         BEFORE the download, otherwise a revoked release is fetched and installed \
         before anyone asks whether it is still trusted."
    );
}

/// REQ-172-006 (Must). GREEN-lock.
/// Acceptance: the veto-period check still runs first. The new gate must be ADDED to
/// the existing checks, not substituted for them.
/// [P1 -> O1, O2]
#[tokio::test]
async fn f2_veto_period_is_still_checked_first() {
    let (a, b, c) = (kp(1), kp(2), kp(3));
    let root = TrustRoot::on_chain(
        vec![
            a.public_key().to_hex(),
            b.public_key().to_hex(),
            c.public_key().to_hex(),
        ],
        3,
    );
    let release = staged_release(&[&a, &b, &c]);

    let err = apply_update(&release, updater::current_timestamp(), true, None, &root)
        .await
        .expect_err("a release still inside its veto window must not be applied");
    assert!(
        matches!(err, UpdateError::VetoPeriodActive { .. }),
        "expected VetoPeriodActive, got {err:?}"
    );
}

/// REQ-172-006 (Must). GREEN-lock.
/// Acceptance: the approval check still runs.
/// [P2 -> O1, O2]
#[tokio::test]
async fn f2_unapproved_update_is_still_refused() {
    let a = kp(1);
    let root = TrustRoot::on_chain(vec![a.public_key().to_hex()], 1);
    let release = staged_release(&[&a]);

    let err = apply_update(&release, long_ago(), false, None, &root)
        .await
        .expect_err("an unapproved update must not be applied");
    assert!(
        matches!(err, UpdateError::NotApproved),
        "expected NotApproved, got {err:?}"
    );
}

/// REQ-172-006 (Must). RED before this fix.
/// Acceptance: a staged release whose signers are no longer in the trust root is
/// REFUSED, and refused before any download.
///
/// This is the rotation scenario: the update was staged and approved while keys A/B/C
/// were the maintainer set; by the time the operator runs `doli-node update apply`, the
/// on-chain set is D/E/F. The staged signatures are still cryptographically valid — they
/// are simply no longer authoritative.
/// [P3, I1 -> O1, O2]
#[tokio::test]
async fn f2_a_staged_release_signed_by_rotated_out_keys_is_refused_before_download() {
    let (a, b, c) = (kp(1), kp(2), kp(3));
    let release = staged_release(&[&a, &b, &c]);

    // The CURRENT on-chain set, after the rotation: none of the staged signers remain.
    let rotated = TrustRoot::on_chain(
        vec![
            kp(4).public_key().to_hex(),
            kp(5).public_key().to_hex(),
            kp(6).public_key().to_hex(),
        ],
        3,
    );

    let err = apply_update(&release, long_ago(), true, None, &rotated)
        .await
        .expect_err(
            "the staged signers were rotated out; applying under revoked keys is exactly \
             what F7(a) exists to prevent, and the manual path must obey it too",
        );
    assert_not_an_io_error(&err, "rotated-out signers");
    match err {
        UpdateError::InsufficientSignatures { found, required } => {
            assert_eq!(
                (found, required),
                (0, 3),
                "none of the staged signers is in the current root"
            );
        }
        other => panic!("expected InsufficientSignatures, got {other:?}"),
    }
}

/// REQ-172-006 (Must). RED before this fix.
/// Acceptance: `--force` waives community approval, never maintainer authority.
///
/// The `Apply` arm passes `approved || force`, so `--force` walks straight past the
/// approval check. If the trust-root gate were the thing `--force` skipped, the flag
/// would be a one-word bypass of the entire milestone.
/// [P4, I2 -> O1, O2]
#[tokio::test]
async fn f2_force_does_not_bypass_the_trust_root() {
    let a = kp(1);
    let release = staged_release(&[&a]);

    // The set existed and was emptied — the fail-closed attack case.
    let emptied = TrustRoot::on_chain(Vec::new(), 3);

    let err = apply_update(
        &release,
        long_ago(),
        /* approved (forced) */ true,
        None,
        &emptied,
    )
    .await
    .expect_err("an emptied on-chain root authorises nothing, forced or not");
    assert_not_an_io_error(&err, "forced apply against an emptied root");
    match err {
        UpdateError::TrustRootUnavailable {
            provenance,
            keys,
            threshold,
        } => {
            assert_eq!(
                provenance, "OnChain",
                "must NOT degrade to the compiled keys"
            );
            assert_eq!((keys, threshold), (0, 3));
        }
        other => panic!("expected TrustRootUnavailable, got {other:?}"),
    }
}
