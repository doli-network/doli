// INC-I-172 M1 review pass 1, [F1] CRITICAL + [F9] — the install gate must bind the
// maintainer signatures to the ARTIFACT BEING INSTALLED.
// REQ-172-006 (Must), REQ-172-001 (Must).
//
// WHY THIS FILE EXISTS. The four `inc_i_172_*_verify_blocks_test.rs` files assert
// SOURCE TEXT: that a verification call appears before `install_binary` and that its
// failure aborts. Every one of them passed against a gate that verified
// `"{sf.version}:{sf.checksums_sha256}"` — both operands read out of the same
// attacker-supplied SIGNATURES.json — while installing a tarball neither operand
// described. Wiring order was correct; the thing being verified was not. No
// source-text assertion can see that, so this file evaluates the real function on real
// bytes and real Ed25519 signatures.
//
// ============================================================================
// OUTPUT CONTRACT
// ============================================================================
// Function under test:
//   `updater::verify_release_artifact(release_info: &GithubReleaseInfo, tarball: &[u8],
//        signatures: &SignaturesFile, root: &TrustRoot) -> Result<usize, UpdateError>`
//
// ENUMERATION OF OBSERVABLE OUTPUTS. All four parameters are shared references with no
// interior mutability; the function has no receiver, opens no file, performs no network
// I/O and touches no global state (verified by reading crates/updater/src/install_gate.rs
// end to end: its only calls are hashing, string comparison, `verify_release_with_trust_root`,
// `platform_tarball_hash` and `verify_hash`, none of which write anything). Therefore:
//   - mutable params    : NONE
//   - receiver mutation : NONE (free function)
//   - persistent store  : NONE (no disk, no DB, no network, no process state)
//   - return value      : the ONLY value channel.
//   - side channel      : `tracing` records at `error!`/`info!`. DECLARED UNASSERTED —
//     observing them needs a global capturing subscriber, which would serialise this
//     whole file behind one dispatcher. No information is lost: every fact logged
//     (which link broke, the signed value, the actual value) is carried in the returned
//     `ArtifactBindingMismatch` payload and IS asserted below (O3).
//
//   O1: Result discriminant                — Ok / Err.
//   O2: Err variant identity               — ArtifactBindingMismatch vs HashMismatch vs
//                                            InsufficientSignatures vs TrustRootUnavailable
//                                            vs DownloadFailed. This is the cell that
//                                            says WHICH link broke; a gate that returns
//                                            Ok here is the F1 defect.
//   O3: ArtifactBindingMismatch payload    — { field, signed, actual }. `field` names
//                                            the broken link; the two values must be the
//                                            real ones so an operator can diagnose.
//   O4: Ok payload                         — the DISTINCT-signer count, which operator
//                                            output prints.
//
// CODE PATHS (of the function under test):
//   P1: sf.version           != release tag                  -> ArtifactBindingMismatch{version}
//   P2: sf.checksums_sha256  != sha256(checksums_body)       -> ArtifactBindingMismatch{checksums_sha256}
//   P3: signatures do not satisfy the root                   -> Insufficient/TrustRootUnavailable
//   P4: tarball              != per-platform hash in CHECKSUMS.txt -> HashMismatch
//   P5: CHECKSUMS.txt has no line for this platform          -> DownloadFailed
//   P6: every link holds                                     -> Ok(distinct signers)
//
// INPUT PARTITIONS:
//   I1: honest release — SIGNATURES.json, CHECKSUMS.txt and tarball all self-consistent.
//   I2: REPLAY — a genuine, correctly-signed SIGNATURES.json copied verbatim from a real
//       PAST release, served alongside an attacker's tarball and an attacker's
//       CHECKSUMS.txt that matches that tarball. This is the F1 attack exactly, and the
//       partition the four wiring tests cannot express.
//   I3: PARTIAL REPLAY — the attacker also rewrites the version field to the victim
//       version, so only the checksums link is broken. Separates the two bindings; a fix
//       that only compares versions still fails here.
//   I4: cosmetic variation that must NOT be treated as an attack — `v`-prefixed version,
//       uppercase-hex checksums digest. A gate that rejects these would be refused by
//       every genuine release, so this partition is what stops the fix from being a
//       denial of service.
//
// NEGATIVE CONTROL (the reason any of this is evidence): every replay fixture is first
// passed through `verify_release_with_trust_root` on its OWN reconstructed release, and
// asserted to return `Ok`. That proves the replayed signatures are cryptographically
// VALID under the very same trust root — so a rejection below can only come from the
// artifact binding, never from bad signatures. Without this control an `Err` here would
// be indistinguishable from a broken fixture.
// ============================================================================

use doli_core::network::Network;
use updater::{
    platform_identifier, sign_release_hash, verify_release_artifact,
    verify_release_with_trust_root, GithubReleaseInfo, Release, SignaturesFile, TrustRoot,
    UpdateError,
};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Deterministic Ed25519 keypair from one seed byte. Test-only material: nothing here
/// is, or resembles, a real maintainer key.
fn kp(seed: u8) -> crypto::KeyPair {
    crypto::KeyPair::from_private_key(crypto::PrivateKey::from_bytes([seed; 32]))
}

/// A usable 3-of-3 on-chain root over the three test keys.
fn root_of(signers: &[&crypto::KeyPair]) -> TrustRoot {
    TrustRoot::on_chain(
        signers.iter().map(|k| k.public_key().to_hex()).collect(),
        signers.len(),
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// The Rust target triple `fetch_github_release` (and therefore the gate) selects for
/// this host. Mirrors `download::platform_target_triple`, which is crate-private.
///
/// If the mapping ever drifts, the fixtures stop matching and the tests FAIL loudly —
/// they cannot silently pass, because `platform_tarball_hash` would find no line and
/// return `DownloadFailed` where `Ok` is asserted.
fn current_triple() -> &'static str {
    match platform_identifier() {
        "linux-x64" => "x86_64-unknown-linux-gnu",
        "linux-arm64" => "aarch64-unknown-linux-gnu",
        "macos-x64" => "x86_64-apple-darwin",
        "macos-arm64" => "aarch64-apple-darwin",
        other => panic!("unsupported test platform: {other}"),
    }
}

/// A CHECKSUMS.txt naming `tarball`'s real hash for THIS platform, plus decoy lines for
/// the other platforms so the per-platform parse has something to get wrong.
fn checksums_for(tarball: &[u8], version: &str) -> Vec<u8> {
    let mine = current_triple();
    let mut text = String::new();
    for triple in [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
    ] {
        let hash = if triple == mine {
            sha256_hex(tarball)
        } else {
            sha256_hex(format!("decoy-{triple}-{version}").as_bytes())
        };
        text.push_str(&format!("{hash}  doli-v{version}-{triple}.tar.gz\n"));
    }
    text.into_bytes()
}

/// One complete, self-consistent release: tarball bytes, its CHECKSUMS.txt, the
/// `GithubReleaseInfo` a real fetch would produce, and a SIGNATURES.json genuinely
/// signed by `signers`.
struct Fixture {
    tarball: Vec<u8>,
    info: GithubReleaseInfo,
    signatures: SignaturesFile,
}

fn fixture(version: &str, payload: &[u8], signers: &[&crypto::KeyPair]) -> Fixture {
    let tarball = payload.to_vec();
    let checksums_body = checksums_for(&tarball, version);
    let checksums_sha256 = sha256_hex(&checksums_body);
    let info = GithubReleaseInfo {
        version: version.to_string(),
        tarball_url: format!("https://example.invalid/v{version}/doli.tar.gz"),
        expected_hash: sha256_hex(&tarball),
        checksums_sha256: checksums_sha256.clone(),
        checksums_body,
        changelog: String::new(),
    };
    let signatures = SignaturesFile {
        version: version.to_string(),
        checksums_sha256: checksums_sha256.clone(),
        signatures: signers
            .iter()
            .map(|k| sign_release_hash(k, version, &checksums_sha256))
            .collect(),
    };
    Fixture {
        tarball,
        info,
        signatures,
    }
}

/// NEGATIVE CONTROL. Assert the signatures in `sf` really do satisfy `root` when checked
/// the way the OLD gate checked them — against `sf`'s own self-reported pair. Any test
/// that then asserts refusal is therefore testing the BINDING, not signature validity.
fn assert_signatures_are_genuinely_valid(sf: &SignaturesFile, root: &TrustRoot) {
    let self_reported = Release {
        version: sf.version.clone(),
        binary_sha256: sf.checksums_sha256.clone(),
        binary_url_template: String::new(),
        changelog: String::new(),
        published_at: 0,
        signatures: sf.signatures.clone(),
        target_networks: Vec::new(),
    };
    let found = verify_release_with_trust_root(&self_reported, root).expect(
        "NEGATIVE CONTROL BROKEN: this SIGNATURES.json is supposed to carry genuine, \
         threshold-satisfying signatures under this root. If it does not, a refusal below \
         proves nothing about artifact binding.",
    );
    assert_eq!(
        found,
        root.threshold(),
        "control expects exactly threshold distinct signers"
    );
}

fn expect_binding_mismatch(err: UpdateError, field: &str) {
    match err {
        UpdateError::ArtifactBindingMismatch {
            field: got,
            signed,
            actual,
        } => {
            assert_eq!(
                got, field,
                "the gate blamed the wrong link: expected `{field}`, got `{got}` \
                 (signed={signed}, actual={actual})"
            );
            assert_ne!(
                signed, actual,
                "the payload must report the two values that differ, not the same string twice"
            );
        }
        other => panic!(
            "expected ArtifactBindingMismatch{{field: \"{field}\"}} — the install must be \
             REFUSED because the signatures do not describe this artifact. Got: {other:?}"
        ),
    }
}

// ---------------------------------------------------------------------------
// P6 / I1 — the honest case still installs (GREEN-lock)
// ---------------------------------------------------------------------------

/// REQ-172-006 (Must). GREEN-lock.
/// Acceptance: a self-consistent release verifies and reports the distinct-signer count.
/// Without this, every assertion below could be satisfied by a gate that refuses
/// everything — which would be a fleet-wide denial of upgrades, not a fix.
/// [P6, I1 -> O1, O4]
#[test]
fn req_172_006_a_self_consistent_release_is_authorised() {
    let (a, b, c) = (kp(11), kp(12), kp(13));
    let root = root_of(&[&a, &b, &c]);
    let f = fixture("9.9.9", b"the-real-tarball-bytes", &[&a, &b, &c]);

    let signers = verify_release_artifact(&f.info, &f.tarball, &f.signatures, &root)
        .expect("an honest, self-consistent release must install");
    assert_eq!(
        signers, 3,
        "must report the DISTINCT signers actually found"
    );
}

/// REQ-172-006 (Must). GREEN-lock, partition I4.
/// Acceptance: the two bindings tolerate the cosmetic variation real publishers emit —
/// a `v`-prefixed tag in SIGNATURES.json and an uppercase-hex digest. A gate that
/// treats these as attacks refuses every genuine release.
/// [P6, I4 -> O1]
#[test]
fn req_172_006_v_prefix_and_hex_case_are_not_treated_as_tampering() {
    let (a, b) = (kp(21), kp(22));
    let root = root_of(&[&a, &b]);
    let mut f = fixture("9.9.9", b"tarball-with-cosmetic-variation", &[&a, &b]);

    // Re-sign over the exact strings a publisher would have used.
    let v_version = format!("v{}", f.signatures.version);
    let upper = f.signatures.checksums_sha256.to_uppercase();
    f.signatures.signatures = [&a, &b]
        .iter()
        .map(|k| sign_release_hash(k, &v_version, &upper))
        .collect();
    f.signatures.version = v_version;
    f.signatures.checksums_sha256 = upper;

    let signers = verify_release_artifact(&f.info, &f.tarball, &f.signatures, &root).expect(
        "`v6.24.1` vs `6.24.1` and uppercase hex are formatting, not tampering; refusing \
         them would break every published release",
    );
    assert_eq!(signers, 2);
}

// ---------------------------------------------------------------------------
// P1 / P2 / I2 — THE F1 ATTACK: a replayed genuine SIGNATURES.json
// ---------------------------------------------------------------------------

/// REQ-172-006 (Must). RED before this fix.
/// Acceptance: a verbatim copy of a genuine SIGNATURES.json from a DIFFERENT release
/// does not authorise the artifact in hand.
///
/// This is the whole milestone claim. The adversary is the one INC-I-157 and INC-I-172
/// exist for — control of the release origin — so it serves (a) a malicious tarball,
/// (b) a CHECKSUMS.txt matching it, and (c) a real, correctly-signed SIGNATURES.json
/// lifted from a past release. Under the old gate the tarball checksum passed (a and b
/// agree) and the signature check passed (c is genuine), and the install proceeded.
/// [P1, I2 -> O1, O2, O3]
#[test]
fn req_172_006_a_replayed_genuine_signatures_json_does_not_authorise_another_tarball() {
    let (a, b, c) = (kp(31), kp(32), kp(33));
    let root = root_of(&[&a, &b, &c]);

    // (c) The genuine, correctly-signed SIGNATURES.json of a real past release.
    let past = fixture("1.0.0", b"the-genuine-1.0.0-tarball", &[&a, &b, &c]);
    assert_signatures_are_genuinely_valid(&past.signatures, &root);

    // (a) + (b) The attacker's tarball and a CHECKSUMS.txt that matches it, published
    // under a NEWER version the operator is upgrading to.
    let malicious = fixture("2.0.0", b"#!/bin/sh\nexfiltrate-producer-key", &[]);

    let err = verify_release_artifact(
        &malicious.info,
        &malicious.tarball,
        &past.signatures, // replayed verbatim
        &root,
    )
    .expect_err(
        "a genuine SIGNATURES.json for v1.0.0 must NOT authorise the v2.0.0 artifact. \
         The maintainers signed 1.0.0's CHECKSUMS.txt; they never saw these bytes.",
    );
    expect_binding_mismatch(err, "version");

    // Control that the attack is otherwise complete: the malicious tarball DOES match
    // the malicious CHECKSUMS.txt, so the checksum gate the signature was meant to
    // backstop is satisfied. Only the binding stops this install.
    assert_eq!(
        sha256_hex(&malicious.tarball),
        malicious.info.expected_hash,
        "the attacker's own checksum must be self-consistent, otherwise this test would \
         pass for the wrong reason"
    );
}

/// REQ-172-006 (Must). RED before this fix.
/// Acceptance: rewriting the version field is not enough — the signed CHECKSUMS.txt
/// digest must also be the digest of the file actually fetched.
///
/// Partition I3 exists because a fix that compares only versions looks correct against
/// the test above and is still fully bypassable: the attacker controls SIGNATURES.json,
/// so it costs nothing to copy the victim's version string into it.
/// [P2, I3 -> O1, O2, O3]
#[test]
fn req_172_006_version_alone_is_not_the_binding_the_checksums_digest_must_match_too() {
    let (a, b, c) = (kp(41), kp(42), kp(43));
    let root = root_of(&[&a, &b, &c]);

    let past = fixture("2.0.0", b"the-genuine-2.0.0-tarball", &[&a, &b, &c]);
    assert_signatures_are_genuinely_valid(&past.signatures, &root);

    // Same version string, different bytes: the signatures still cover the GENUINE
    // release's CHECKSUMS.txt, which is not the one this tarball's hash came from.
    let malicious = fixture("2.0.0", b"#!/bin/sh\nrm -rf --no-preserve-root /", &[]);
    assert_ne!(
        past.info.checksums_sha256, malicious.info.checksums_sha256,
        "fixture sanity: the two CHECKSUMS.txt files must differ"
    );

    let err = verify_release_artifact(&malicious.info, &malicious.tarball, &past.signatures, &root)
        .expect_err(
            "the maintainers signed a CHECKSUMS.txt whose digest is not the digest of the \
         CHECKSUMS.txt this tarball's hash was read from — the chain is broken and the \
         install must be refused",
        );
    expect_binding_mismatch(err, "checksums_sha256");
}

/// REQ-172-006 (Must). RED before this fix.
/// Acceptance: the digest the gate compares is recomputed from the CHECKSUMS.txt BYTES,
/// not taken from the caller's `checksums_sha256` field.
///
/// The field is derived data. If the gate trusted it, an attacker who can shape the
/// fetch result (or a future refactor that fills the two fields from different fetches
/// — the TOCTOU shape AUDIT-UPDATE-002 closed) re-opens the hole while every other
/// assertion in this file still passes.
/// [P2 -> O1, O2, O3]
#[test]
fn req_172_006_the_checksums_digest_is_recomputed_from_bytes_not_trusted_from_the_field() {
    let (a, b) = (kp(51), kp(52));
    let root = root_of(&[&a, &b]);
    let mut f = fixture("3.1.4", b"tarball-for-3.1.4", &[&a, &b]);
    assert_signatures_are_genuinely_valid(&f.signatures, &root);

    // The struct now LIES: its `checksums_sha256` still equals what the signatures
    // cover, but `checksums_body` — the bytes the per-platform hash is parsed from —
    // has been swapped for an attacker's file.
    let other = fixture("3.1.4", b"attacker-tarball", &[]);
    f.info.checksums_body = other.info.checksums_body.clone();

    let err = verify_release_artifact(&f.info, &other.tarball, &f.signatures, &root).expect_err(
        "the gate must hash `checksums_body` itself. Believing the `checksums_sha256` \
         field is exactly the unverified-operand defect (F1).",
    );
    expect_binding_mismatch(err, "checksums_sha256");
}

// ---------------------------------------------------------------------------
// P3 / P4 / P5 — the remaining links
// ---------------------------------------------------------------------------

/// REQ-172-001 (Must). RED-adjacent (the link existed but was unreachable behind F1).
/// Acceptance: signatures that do not satisfy the root still refuse, even when both
/// bindings hold. Pins that the binding checks were ADDED to the signature check, not
/// substituted for it.
/// [P3 -> O1, O2]
#[test]
fn req_172_001_binding_does_not_replace_the_signature_threshold() {
    let (a, b, c) = (kp(61), kp(62), kp(63));
    let root = root_of(&[&a, &b, &c]); // threshold 3
    let f = fixture("4.0.0", b"tarball-for-4.0.0", &[&a, &b]); // only 2 signers

    let err = verify_release_artifact(&f.info, &f.tarball, &f.signatures, &root)
        .expect_err("2 of 3 distinct signers must not authorise an install");
    match err {
        UpdateError::InsufficientSignatures { found, required } => {
            assert_eq!((found, required), (2, 3));
        }
        other => panic!("expected InsufficientSignatures, got {other:?}"),
    }
}

/// REQ-172-001 (Must). GREEN-lock.
/// Acceptance: an unusable root refuses before any signature is examined, and the
/// refusal is `TrustRootUnavailable` — never a silent fall back to the compiled keys.
/// [P3 -> O1, O2]
#[test]
fn req_172_001_an_emptied_on_chain_root_refuses_a_perfectly_signed_release() {
    let (a, b, c) = (kp(71), kp(72), kp(73));
    let usable = root_of(&[&a, &b, &c]);
    let f = fixture("5.0.0", b"tarball-for-5.0.0", &[&a, &b, &c]);
    assert_signatures_are_genuinely_valid(&f.signatures, &usable);

    let emptied = TrustRoot::on_chain(Vec::new(), 3);
    let err = verify_release_artifact(&f.info, &f.tarball, &f.signatures, &emptied)
        .expect_err("an emptied on-chain root authorises nothing");
    assert!(
        matches!(err, UpdateError::TrustRootUnavailable { .. }),
        "expected TrustRootUnavailable, got {err:?} — falling back to the compiled \
         bootstrap keys is the F1 defect"
    );
}

/// REQ-172-006 (Must). RED before this fix.
/// Acceptance: the tarball must hash to the per-platform line of the VERIFIED
/// CHECKSUMS.txt. This is the last link: signatures and bindings can all be perfect and
/// a substituted tarball still has to be refused.
/// [P4 -> O1, O2]
#[test]
fn req_172_006_a_substituted_tarball_is_refused_even_with_perfect_signatures() {
    let (a, b, c) = (kp(81), kp(82), kp(83));
    let root = root_of(&[&a, &b, &c]);
    let f = fixture(
        "6.0.0",
        b"the-tarball-the-maintainers-signed",
        &[&a, &b, &c],
    );
    assert_signatures_are_genuinely_valid(&f.signatures, &root);

    let swapped = b"a-different-tarball-with-the-same-name".to_vec();
    let err = verify_release_artifact(&f.info, &swapped, &f.signatures, &root)
        .expect_err("the artifact must be the one CHECKSUMS.txt names for this platform");
    match err {
        UpdateError::HashMismatch { expected, actual } => {
            assert_eq!(expected, sha256_hex(&f.tarball));
            assert_eq!(actual, sha256_hex(&swapped));
        }
        other => panic!("expected HashMismatch, got {other:?}"),
    }
}

/// REQ-172-006 (Must).
/// Acceptance: a CHECKSUMS.txt with no line for THIS platform refuses rather than
/// falling through. Also the positive control for `current_triple()`: if the fixture's
/// platform selection were wrong, the honest tests above would land here instead.
/// [P5 -> O1, O2]
#[test]
fn req_172_006_checksums_without_a_line_for_this_platform_refuses() {
    let (a, b) = (kp(91), kp(92));
    let root = root_of(&[&a, &b]);
    let mut f = fixture("7.0.0", b"tarball-for-7.0.0", &[&a, &b]);

    // Drop the line for this host, keep the others, and re-sign over the new digest so
    // the ONLY thing wrong is the missing platform entry.
    let text = String::from_utf8(f.info.checksums_body.clone()).unwrap();
    let filtered: String = text
        .lines()
        .filter(|l| !l.contains(current_triple()))
        .map(|l| format!("{l}\n"))
        .collect();
    f.info.checksums_body = filtered.into_bytes();
    let digest = sha256_hex(&f.info.checksums_body);
    f.info.checksums_sha256 = digest.clone();
    f.signatures.checksums_sha256 = digest.clone();
    f.signatures.signatures = [&a, &b]
        .iter()
        .map(|k| sign_release_hash(k, "7.0.0", &digest))
        .collect();

    let err = verify_release_artifact(&f.info, &f.tarball, &f.signatures, &root)
        .expect_err("no checksum line for this platform means nothing binds the tarball");
    assert!(
        matches!(err, UpdateError::DownloadFailed(ref m) if m.contains("No checksum for platform")),
        "expected the per-platform parse to refuse, got {err:?}"
    );
}

/// REQ-172-001 (Must). GREEN-lock.
/// Acceptance: the CLI's Bootstrap root is a real, usable root on mainnet — so the
/// `doli upgrade` path is gated by the binding above rather than dying at
/// `TrustRootUnavailable` for unrelated reasons.
#[test]
fn req_172_001_the_bootstrap_root_the_cli_uses_is_usable() {
    let root = TrustRoot::bootstrap(Network::Mainnet);
    assert!(
        root.is_usable(),
        "the CLI's only available root must be usable, otherwise `doli upgrade` cannot \
         install anything at all"
    );
}
