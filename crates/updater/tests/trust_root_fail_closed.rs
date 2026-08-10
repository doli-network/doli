// INC-I-172 M1 — release-verification trust root must FAIL CLOSED.
// REQ-172-001 (Must), REQ-172-005 (Must), REQ-172-011 (Must), REQ-172-012 (Must)
//
// STATE: **RED**. This file does not COMPILE against the tree as it stands today:
//   `updater::TrustRoot`, `updater::TrustRootProvenance`,
//   `updater::verify_release_with_trust_root` and
//   `updater::UpdateError::TrustRootUnavailable` do not exist yet. That
//   compile failure IS the RED signal for the whole file — every assertion below
//   is unreachable until the developer lands the Layer-1 API pinned by
//   docs/.workflow/inc-i-172-M1-api-contract.md §1-§2.
//   Three tests in here are GREEN-lock (marked per test): they pin behaviour that
//   MUST NOT change (bootstrap root shape, 3-of-5 acceptance, unknown-key rejection).
//
// ============================================================================
// OUTPUT CONTRACT
// ============================================================================
// Function under test:
//   `updater::verify_release_with_trust_root(release: &Release, root: &TrustRoot)
//       -> Result<(), UpdateError>`
//   (api-contract §2; design specs/maintainer-trust-root-architecture.md F1 + F3)
//
// ENUMERATION OF OBSERVABLE OUTPUTS. The function takes `&Release` and `&TrustRoot`
// (both shared, no interior mutability), has no receiver, opens no file, touches no
// process/global state. Therefore:
//   - mutable params      : NONE
//   - receiver mutation   : NONE (free function)
//   - persistent store    : NONE (no disk, no DB, no network)
//   - return value        : the ONLY value-channel output.
//   - side channel        : `tracing` log records. api-contract §2.1 REQUIRES the
//     unusable-root rejection to log at `error!` with provenance + key count +
//     threshold. An integration test cannot observe that without installing a
//     capturing subscriber, which would serialise this whole file behind a global
//     dispatcher. DECLARED UNASSERTED, with justification: the same three facts
//     (provenance, keys, threshold) are carried in the RETURNED
//     `TrustRootUnavailable` payload and ARE asserted below (O3), so no
//     information is lost — only the emission channel is unverified. Recorded as
//     a residual cell in docs/.workflow/inc-i-172-M1-test-plan.md.
//
//   O1: Result discriminant                     — Ok / Err.
//   O2: Err variant identity                    — TrustRootUnavailable vs
//                                                 InsufficientSignatures. THE
//                                                 discriminator between a
//                                                 fail-CLOSED and a fail-OPEN
//                                                 implementation (see P1 note).
//   O3: TrustRootUnavailable payload            — { provenance, keys, threshold }.
//                                                 Must describe the root that was
//                                                 REFUSED (OnChain, 0 keys), never
//                                                 the compiled bootstrap root.
//   O4: InsufficientSignatures payload          — { found, required }. `found` is a
//                                                 DISTINCT-SIGNER count (F3), not an
//                                                 entry count. This is the cell the
//                                                 signature-stuffing attack moves.
//   O5: TrustRoot constructor outputs           — keys()/threshold()/provenance()/
//                                                 is_usable() (api-contract §1).
//
// CODE PATHS (of the function under test):
//   P1: root NOT usable — 0 keys           (the F1 attack case: set existed, emptied)
//   P2: root NOT usable — sub-threshold    (2 keys, threshold 3)
//   P3: root NOT usable — threshold 0      (FM-02: MaintainerState::default())
//   P4: root usable, distinct valid signers >= threshold      -> Ok
//   P5: root usable, distinct valid signers <  threshold      -> InsufficientSignatures
//   P6: TrustRoot constructors (no verification performed)
//
// INPUT PARTITIONS (what varies inside `release.signatures`):
//   I1: k distinct in-root keys, each signature cryptographically VALID.
//   I2: ONE in-root key, repeated as N separate signature ENTRIES, each VALID.
//       (F3 signature-stuffing: 1 stolen key must not satisfy a 3-of-n threshold.)
//   I3: signers whose public keys are NOT in the root.
//   I4: signature entries carrying the COMPILED BOOTSTRAP public keys.
//       Why this partition and why it is honest: the brief asks for signatures
//       "valid under bootstrap_maintainer_keys(network)". Those private keys are
//       not in this repo and must never be, so cryptographic validity under them
//       is unconstructible here. It is not needed: validity is NOT what
//       distinguishes fail-open from fail-closed. A fail-OPEN implementation
//       reaches the bootstrap key list and can only report
//       InsufficientSignatures{found:_, required:3} against 5 BOOTSTRAP keys; a
//       fail-CLOSED implementation returns TrustRootUnavailable{keys:0,
//       provenance:OnChain} and never looks at the entries at all. O2+O3 separate
//       them on every input, valid or not. Using I4 (real bootstrap identities)
//       rather than random keys additionally guarantees the test cannot pass
//       merely because the entries were unrecognisable.
//   I5: no signature entries at all (empty vec).
//
// MATRIX (every cell asserted by the test named in it):
//
//  path | partition | O1  | O2                    | O3                  | O4
//  -----|-----------|-----|-----------------------|---------------------|-------------------
//  P1   | I4        | Err | TrustRootUnavailable  | OnChain / 0 / 3     | n/a  [t01]
//  P1   | I1        | Err | TrustRootUnavailable  | OnChain / 0 / 3     | n/a  [t02]
//  P1   | I5        | Err | TrustRootUnavailable  | OnChain / 0 / 3     | n/a  [t02]
//  P2   | I1        | Err | TrustRootUnavailable  | OnChain / 2 / 3     | n/a  [t03]
//  P3   | I1        | Err | TrustRootUnavailable  | OnChain / 5 / 0     | n/a  [t04]
//  P4   | I1        | Ok  | n/a                   | n/a                 | n/a  [t05,t06]
//  P5   | I2        | Err | InsufficientSignatures| n/a                 | 1 / 3 [t07]
//  P5   | I2+I1     | Err | InsufficientSignatures| n/a                 | 2 / 3 [t08]
//  P5   | I3        | Err | InsufficientSignatures| n/a                 | 0 / 3 [t09]
//  P5   | I1+I3     | Err | InsufficientSignatures| n/a                 | 2 / 3 [t09]
//  P5   | I4        | Err | InsufficientSignatures| n/a                 | 0 / 3 [t10]
//  P6   | n/a       | O5 for bootstrap + on_chain constructors          [t11,t12]
//
// 12 tests, 12 matrix rows, every cell asserted. No cell is left to inference.
// ============================================================================

use doli_core::network::Network;
use updater::{
    bootstrap_maintainer_keys, sign_release_hash, verify_release_signatures,
    verify_release_with_trust_root, MaintainerSignature, Release, TrustRoot, TrustRootProvenance,
    UpdateError, REQUIRED_SIGNATURES,
};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const VERSION: &str = "9.9.9";
const SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Deterministic Ed25519 keypair from a single seed byte. Test-only material:
/// nothing here is, or resembles, a real maintainer key.
fn kp(seed: u8) -> crypto::KeyPair {
    crypto::KeyPair::from_private_key(crypto::PrivateKey::from_bytes([seed; 32]))
}

fn hex_of(k: &crypto::KeyPair) -> String {
    k.public_key().to_hex()
}

/// A release over ("{VERSION}:{SHA256}") carrying one VALID signature entry per
/// element of `signers` — repeats included, so the caller controls entry count
/// independently of distinct-signer count (partition I2).
fn release_signed_by(signers: &[&crypto::KeyPair]) -> Release {
    Release {
        version: VERSION.to_string(),
        binary_sha256: SHA256.to_string(),
        binary_url_template: String::new(),
        changelog: String::new(),
        published_at: 0,
        signatures: signers
            .iter()
            .map(|k| sign_release_hash(k, VERSION, SHA256))
            .collect(),
        target_networks: Vec::new(),
    }
}

/// A release whose signature entries carry the COMPILED BOOTSTRAP public keys
/// (partition I4). The signature bytes are structurally well-formed but cannot be
/// cryptographically valid — see the I4 note in the OUTPUT CONTRACT for why that
/// does not weaken the discriminator.
fn release_claiming_bootstrap_signers(network: Network) -> Release {
    let filler = sign_release_hash(&kp(0xF0), VERSION, SHA256).signature;
    Release {
        version: VERSION.to_string(),
        binary_sha256: SHA256.to_string(),
        binary_url_template: String::new(),
        changelog: String::new(),
        published_at: 0,
        signatures: bootstrap_maintainer_keys(network)
            .iter()
            .map(|pk| MaintainerSignature {
                public_key: (*pk).to_string(),
                signature: filler.clone(),
            })
            .collect(),
        target_networks: Vec::new(),
    }
}

/// Assert `err` is `TrustRootUnavailable` with exactly this payload, and report
/// loudly (with the actual variant) when it is not — a bare `matches!` here would
/// hide the fail-open signal, which is precisely the thing under test.
fn assert_trust_root_unavailable(err: UpdateError, keys: usize, threshold: usize) {
    match err {
        UpdateError::TrustRootUnavailable {
            provenance,
            keys: k,
            threshold: t,
        } => {
            // O3: the payload must describe the root that was REFUSED.
            assert_eq!(
                k, keys,
                "TrustRootUnavailable must report the REFUSED root's key count ({keys}), got {k}. \
                 Reporting {} here would mean the compiled bootstrap root was consulted — the \
                 exact fail-open defect (F1).",
                bootstrap_maintainer_keys(Network::Mainnet).len()
            );
            assert_eq!(
                t, threshold,
                "TrustRootUnavailable must report the REFUSED root's threshold ({threshold}), got {t}"
            );
            assert!(
                provenance.contains("OnChain"),
                "TrustRootUnavailable provenance must name the OnChain root that failed, got {provenance:?}. \
                 'Bootstrap' here means the compiled leaked keys were reached (F1 fail-open)."
            );
        }
        other => panic!(
            "expected UpdateError::TrustRootUnavailable{{provenance:OnChain, keys:{keys}, threshold:{threshold}}}, \
             got {other:?}. An `InsufficientSignatures` here is the fail-open signature: it can only be \
             produced by counting signatures against SOME key list, i.e. the compiled bootstrap keys."
        ),
    }
}

fn assert_insufficient(err: UpdateError, found: usize, required: usize) {
    match err {
        UpdateError::InsufficientSignatures {
            found: f,
            required: r,
        } => {
            assert_eq!(
                f, found,
                "distinct-signer count must be {found}, got {f} (F3: signature ENTRIES are not signers)"
            );
            assert_eq!(
                r, required,
                "required threshold must be {required}, got {r}"
            );
        }
        other => panic!("expected InsufficientSignatures{{{found}/{required}}}, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// P1 — empty on-chain root FAILS CLOSED  (REQ-172-001, REQ-172-011)
// ---------------------------------------------------------------------------

/// REQ-172-001 (Must) / REQ-172-011 (Must). RED.
/// Acceptance: an EMPTY on-chain trust root never falls back to the compiled
/// bootstrap keys; it refuses to authorise anything.
/// [P1 x I4 -> O1,O2,O3]
///
/// This is the incident in one test. `run.rs:461` hands the updater an empty key
/// list; `verification.rs:66-79` treats "empty" as "use the leaked compiled keys"
/// (constants.rs:37-48). The release here claims the five real bootstrap
/// identities, so a fail-open implementation has every reason to accept the
/// bootstrap path and report against it. Fail-closed must never look.
#[test]
fn req_172_001_empty_on_chain_root_never_falls_back_to_bootstrap_keys() {
    let root = TrustRoot::on_chain(Vec::new(), REQUIRED_SIGNATURES);
    let release = release_claiming_bootstrap_signers(Network::Mainnet);

    let err = verify_release_with_trust_root(&release, &root)
        .expect_err("an empty on-chain trust root must NOT authorise a release (F1 fail-closed)");

    assert_trust_root_unavailable(err, 0, REQUIRED_SIGNATURES);
}

/// REQ-172-001 (Must). RED.
/// Acceptance: the empty-root refusal is independent of what the release carries —
/// genuinely valid signatures from unrelated keys, or no signatures at all, are
/// both refused with the same TrustRootUnavailable verdict.
/// [P1 x I1 -> O1,O2,O3] and [P1 x I5 -> O1,O2,O3]
#[test]
fn req_172_001_empty_on_chain_root_refuses_regardless_of_release_contents() {
    let root = TrustRoot::on_chain(Vec::new(), REQUIRED_SIGNATURES);

    // I1: three cryptographically valid signatures from three distinct keys.
    let (a, b, c) = (kp(1), kp(2), kp(3));
    let signed = release_signed_by(&[&a, &b, &c]);
    let err = verify_release_with_trust_root(&signed, &root)
        .expect_err("empty root must refuse even a validly signed release");
    assert_trust_root_unavailable(err, 0, REQUIRED_SIGNATURES);

    // I5: no signature entries at all.
    let unsigned = release_signed_by(&[]);
    let err = verify_release_with_trust_root(&unsigned, &root)
        .expect_err("empty root must refuse an unsigned release");
    assert_trust_root_unavailable(err, 0, REQUIRED_SIGNATURES);
}

// ---------------------------------------------------------------------------
// P2/P3 — sub-threshold and zero-threshold roots FAIL CLOSED  (REQ-172-011)
// ---------------------------------------------------------------------------

/// REQ-172-011 (Must). RED.
/// Acceptance: a root with fewer keys than its own threshold is unusable — it is
/// NOT "as many signatures as it can manage", and it is NOT a bootstrap trigger.
/// [P2 x I1 -> O1,O2,O3]
#[test]
fn req_172_011_sub_threshold_on_chain_root_is_trust_root_unavailable() {
    let (a, b, c) = (kp(11), kp(12), kp(13));
    // Two keys, threshold three: the root cannot possibly satisfy itself.
    let root = TrustRoot::on_chain(vec![hex_of(&a), hex_of(&b)], 3);
    assert!(
        !root.is_usable(),
        "a 2-key root with threshold 3 must not be usable (api-contract §1 is_usable)"
    );

    // Sign with all three, including one key outside the root — a sub-threshold
    // root must refuse BEFORE counting, so the extra signature changes nothing.
    let release = release_signed_by(&[&a, &b, &c]);
    let err = verify_release_with_trust_root(&release, &root)
        .expect_err("a sub-threshold root must refuse, not count");

    assert_trust_root_unavailable(err, 2, 3);
}

/// REQ-172-011 (Must) / REQ-172-001 (Must). RED.
/// Acceptance: threshold 0 is refused, not satisfied vacuously.
/// [P3 x I1 -> O1,O2,O3]
///
/// FM-02: `MaintainerState::default()` yields `MaintainerSet::new()` whose
/// `threshold` is 0 (crates/core/src/maintainer.rs calculate_threshold(0) == 0).
/// Under an entry-count `valid >= threshold` test, 0 >= 0 accepts a release with
/// ZERO valid signatures. `is_usable()` must require `threshold >= 1`.
#[test]
fn req_172_011_zero_threshold_root_is_refused_not_vacuously_satisfied() {
    let keys: Vec<String> = (21..26u8).map(|s| hex_of(&kp(s))).collect();
    let root = TrustRoot::on_chain(keys, 0);
    assert!(
        !root.is_usable(),
        "threshold 0 must make a root unusable, however many keys it has \
         (api-contract §1: `self.threshold >= 1 && ...`)"
    );

    let unsigned = release_signed_by(&[]);
    let err = verify_release_with_trust_root(&unsigned, &root)
        .expect_err("threshold 0 must NOT accept an unsigned release (FM-02)");

    assert_trust_root_unavailable(err, 5, 0);
}

// ---------------------------------------------------------------------------
// P4 — a usable root still accepts a genuine quorum  (REQ-172-005)
// ---------------------------------------------------------------------------

/// REQ-172-005 (Must). GREEN-lock.
/// Acceptance: 3 distinct signers against a 5-key / threshold-3 root is accepted —
/// the shipping 3-of-5 semantics are unchanged by the fail-closed rework.
/// [P4 x I1 -> O1]
#[test]
fn req_172_005_three_of_five_distinct_signers_is_accepted() {
    let ks: Vec<crypto::KeyPair> = (31..36u8).map(kp).collect();
    let root = TrustRoot::on_chain(ks.iter().map(hex_of).collect(), REQUIRED_SIGNATURES);
    assert!(root.is_usable(), "a 5-key threshold-3 root must be usable");

    let release = release_signed_by(&[&ks[0], &ks[2], &ks[4]]);
    verify_release_with_trust_root(&release, &root)
        .expect("3 distinct in-root signers must satisfy a threshold of 3");
}

/// REQ-172-005 (Must). GREEN-lock.
/// Acceptance: an un-upgraded / unbootstrapped node keeps verifying exactly as it
/// does today — the BOOTSTRAP root is usable and accepts a genuine 3-of-5 quorum.
/// [P4 x I1 -> O1] + [P6 -> O5]
///
/// Honest limit, stated rather than hidden: the private halves of the compiled
/// bootstrap keys are not in this repo and must never be, so a release cannot be
/// signed under them here. This test therefore locks the two facts that ARE
/// checkable and that together constitute "the bootstrap path still works":
///   (a) `TrustRoot::bootstrap(net)` is usable and has exactly the compiled
///       key list at threshold REQUIRED_SIGNATURES (so nothing was dropped), and
///   (b) a root of the SAME SHAPE (5 keys, threshold 3) accepts 3 distinct
///       signers through the SAME `verify_release_with_trust_root` code path.
/// There is no branch in the function keyed on provenance (api-contract §2 lists
/// none), so (a)+(b) cover the bootstrap acceptance cell without private keys.
#[test]
fn req_172_005_bootstrap_root_preserves_three_of_five_acceptance() {
    for network in [Network::Mainnet, Network::Testnet, Network::Devnet] {
        let root = TrustRoot::bootstrap(network);

        // (a) shape preserved.
        assert_eq!(
            root.provenance(),
            TrustRootProvenance::Bootstrap,
            "{network:?}: TrustRoot::bootstrap must report Bootstrap provenance"
        );
        assert_eq!(
            root.threshold(),
            REQUIRED_SIGNATURES,
            "{network:?}: bootstrap threshold must stay at REQUIRED_SIGNATURES"
        );
        let expected: Vec<String> = bootstrap_maintainer_keys(network)
            .iter()
            .map(|k| (*k).to_string())
            .collect();
        assert_eq!(
            root.keys(),
            expected.as_slice(),
            "{network:?}: bootstrap root must carry exactly the compiled keys, in order"
        );
        assert!(
            root.is_usable(),
            "{network:?}: the bootstrap root must remain usable — an un-upgraded node \
             that has never established an on-chain set still needs to verify updates \
             (REQ-172-005)"
        );

        // (b) same shape, same code path, genuine quorum -> Ok.
        let ks: Vec<crypto::KeyPair> = (41..46u8).map(kp).collect();
        let same_shape = TrustRoot::on_chain(ks.iter().map(hex_of).collect(), root.threshold());
        let release = release_signed_by(&[&ks[1], &ks[3], &ks[4]]);
        verify_release_with_trust_root(&release, &same_shape).unwrap_or_else(|e| {
            panic!("{network:?}: a bootstrap-shaped root must accept 3 distinct signers, got {e:?}")
        });
    }
}

// ---------------------------------------------------------------------------
// P5 — DISTINCT-SIGNER counter (F3)  (REQ-172-012)
// ---------------------------------------------------------------------------

/// REQ-172-012 (Must). RED.
/// Acceptance: three valid signature ENTRIES produced by ONE key satisfy a
/// threshold of 3? NO — they count as ONE signer.
/// [P5 x I2 -> O1,O2,O4]
///
/// `verification.rs:83-121` is a flat `for sig in &release.signatures` with
/// `valid_count += 1` and no dedup, so an attacker holding ONE of five maintainer
/// keys clears a "3-of-5" gate by pasting the same signature three times. The fix
/// is the covenant k-of-n shape already mainnet-live at
/// crates/core/src/conditions/eval.rs:51-68 (outer loop over root keys, inner over
/// witnesses, `break` on first hit).
#[test]
fn req_172_012_three_entries_from_one_key_count_as_one_signer() {
    let ks: Vec<crypto::KeyPair> = (51..56u8).map(kp).collect();
    let root = TrustRoot::on_chain(ks.iter().map(hex_of).collect(), 3);

    // One stolen key, three identical-signer entries. All three are individually
    // cryptographically VALID — entry counting cannot tell them apart.
    let release = release_signed_by(&[&ks[0], &ks[0], &ks[0]]);
    assert_eq!(
        release.signatures.len(),
        3,
        "fixture must carry three signature ENTRIES for the test to mean anything"
    );

    let err = verify_release_with_trust_root(&release, &root).expect_err(
        "3 entries from 1 key must NOT satisfy a threshold of 3 (F3 signature stuffing)",
    );
    assert_insufficient(err, 1, 3);
}

/// REQ-172-012 (Must). RED.
/// Acceptance: a duplicate entry from an already-counted key does not inflate the
/// count past its distinct-signer value.
/// [P5 x I2+I1 -> O1,O2,O4]
#[test]
fn req_172_012_duplicate_entry_from_counted_key_does_not_inflate() {
    let ks: Vec<crypto::KeyPair> = (61..66u8).map(kp).collect();
    let root = TrustRoot::on_chain(ks.iter().map(hex_of).collect(), 3);

    // Two distinct signers; the first one duplicated. Entries = 3, signers = 2.
    let release = release_signed_by(&[&ks[0], &ks[0], &ks[1]]);
    let err = verify_release_with_trust_root(&release, &root)
        .expect_err("2 distinct signers must not clear a threshold of 3 by duplication");
    assert_insufficient(err, 2, 3);

    // Adding the third DISTINCT signer is what clears it — proving the failure
    // above was the duplicate, not an off-by-one in the counter.
    let release = release_signed_by(&[&ks[0], &ks[0], &ks[1], &ks[2]]);
    verify_release_with_trust_root(&release, &root).expect(
        "3 distinct in-root signers must clear a threshold of 3 even with a duplicate present",
    );
}

/// REQ-172-012 (Must) / REQ-172-001 (Must). GREEN-lock.
/// Acceptance: signatures from keys outside the root are ignored and never count.
/// [P5 x I3 -> O1,O2,O4] and [P5 x I1+I3 -> O1,O2,O4]
#[test]
fn req_172_012_signatures_from_keys_outside_the_root_do_not_count() {
    let ks: Vec<crypto::KeyPair> = (71..76u8).map(kp).collect();
    let root = TrustRoot::on_chain(ks.iter().map(hex_of).collect(), 3);
    let (x, y, z) = (kp(200), kp(201), kp(202));

    // I3: three valid signatures, none of them from a root member.
    let release = release_signed_by(&[&x, &y, &z]);
    let err = verify_release_with_trust_root(&release, &root)
        .expect_err("out-of-root signers must not authorise anything");
    assert_insufficient(err, 0, 3);

    // I1+I3: two in-root signers padded with three outsiders — still 2.
    let release = release_signed_by(&[&ks[0], &x, &ks[1], &y, &z]);
    let err = verify_release_with_trust_root(&release, &root)
        .expect_err("padding with out-of-root signatures must not reach the threshold");
    assert_insufficient(err, 2, 3);
}

/// REQ-172-012 (Must). GREEN-lock.
/// Acceptance: a usable on-chain root does not honour the compiled bootstrap
/// identities — revocation actually revokes.
/// [P5 x I4 -> O1,O2,O4]
#[test]
fn req_172_012_usable_on_chain_root_does_not_honour_bootstrap_identities() {
    let ks: Vec<crypto::KeyPair> = (81..86u8).map(kp).collect();
    let root = TrustRoot::on_chain(ks.iter().map(hex_of).collect(), REQUIRED_SIGNATURES);

    let release = release_claiming_bootstrap_signers(Network::Mainnet);
    let err = verify_release_with_trust_root(&release, &root).expect_err(
        "once an on-chain root exists, the compiled bootstrap keys must carry no authority",
    );
    assert_insufficient(err, 0, REQUIRED_SIGNATURES);
}

// ---------------------------------------------------------------------------
// P6 — TrustRoot constructors  (REQ-172-011)
// ---------------------------------------------------------------------------

/// REQ-172-011 (Must). RED.
/// Acceptance: `on_chain` is a faithful, non-lossy record of what the chain said —
/// including the empty case, which is representable and NOT a bootstrap trigger.
/// [P6 -> O5]
#[test]
fn req_172_011_on_chain_constructor_represents_the_empty_root_faithfully() {
    let empty = TrustRoot::on_chain(Vec::new(), REQUIRED_SIGNATURES);
    assert_eq!(
        empty.provenance(),
        TrustRootProvenance::OnChain,
        "an empty on-chain root must stay OnChain — silently becoming Bootstrap is the F1 defect"
    );
    assert!(empty.keys().is_empty(), "empty root must report zero keys");
    assert_eq!(
        empty.threshold(),
        REQUIRED_SIGNATURES,
        "the threshold survives an empty key list"
    );
    assert!(!empty.is_usable(), "an empty root is never usable");

    let ks: Vec<crypto::KeyPair> = (91..96u8).map(kp).collect();
    let hexes: Vec<String> = ks.iter().map(hex_of).collect();
    let full = TrustRoot::on_chain(hexes.clone(), 3);
    assert_eq!(full.provenance(), TrustRootProvenance::OnChain);
    assert_eq!(
        full.keys(),
        hexes.as_slice(),
        "keys must round-trip verbatim"
    );
    assert_eq!(full.threshold(), 3);
    assert!(full.is_usable());

    // Exactly-at-threshold is usable; one short is not.
    assert!(
        TrustRoot::on_chain(hexes[..3].to_vec(), 3).is_usable(),
        "keys.len() == threshold must be usable"
    );
    assert!(
        !TrustRoot::on_chain(hexes[..2].to_vec(), 3).is_usable(),
        "keys.len() < threshold must not be usable"
    );
}

/// REQ-172-005 (Must). GREEN-lock.
/// Acceptance: the compatibility shim `verify_release_signatures(release, network)`
/// keeps resolving to the BOOTSTRAP root (api-contract §2), so the CLI and any
/// caller without on-chain state behaves exactly as it does today.
/// [P6 -> O5] + [P5 x I1 -> O2,O4]
#[test]
fn req_172_005_legacy_shim_still_resolves_to_the_bootstrap_root() {
    // Keys that are NOT bootstrap keys must be rejected by the shim — which is
    // only observable if the shim really is using the bootstrap root.
    let ks: Vec<crypto::KeyPair> = (101..106u8).map(kp).collect();
    let release = release_signed_by(&[&ks[0], &ks[1], &ks[2]]);

    let err = verify_release_signatures(&release, Network::Mainnet)
        .expect_err("the bootstrap shim must reject signers that are not bootstrap maintainers");
    assert_insufficient(err, 0, REQUIRED_SIGNATURES);

    // And it must NOT report TrustRootUnavailable: the bootstrap root is usable,
    // so this path is a genuine signature shortfall, not an unavailable root.
    assert!(
        TrustRoot::bootstrap(Network::Mainnet).is_usable(),
        "the bootstrap root behind the shim must be usable"
    );
}

// ---------------------------------------------------------------------------
// F10 (review pass 1) — public-key comparison is CASE-INSENSITIVE
// ---------------------------------------------------------------------------

/// F10. RED before the fix.
/// Acceptance: a SIGNATURES.json entry whose `public_key` is uppercase hex still
/// matches a lowercase root key.
///
/// The root's keys are always lowercase (`PublicKey::to_hex`, and the compiled arrays),
/// while `sig.public_key` is free-form JSON text. An exact `String` comparison drops
/// the entry silently and reports `InsufficientSignatures`, so the operator is told
/// "not enough maintainers signed" when the truth is "your file used capital letters".
/// Fail-closed, so not exploitable — but a control that refuses valid input for a
/// reason it will not name is an availability defect and a diagnosis dead end.
/// [P4, new partition I5: hex case -> O1, O4]
#[test]
fn f10_uppercase_hex_public_keys_still_match_the_root() {
    let (a, b, c) = (kp(0xA1), kp(0xA2), kp(0xA3));
    let root = TrustRoot::on_chain(vec![hex_of(&a), hex_of(&b), hex_of(&c)], 3);

    let mut release = release_signed_by(&[&a, &b, &c]);
    // Control: as published (lowercase) this release verifies, so any failure after
    // the case change is caused by the case change alone.
    assert_eq!(
        verify_release_with_trust_root(&release, &root).expect("lowercase control must verify"),
        3
    );

    for sig in &mut release.signatures {
        sig.public_key = sig.public_key.to_uppercase();
    }
    let found = verify_release_with_trust_root(&release, &root).unwrap_or_else(|e| {
        panic!(
            "uppercase-hex public keys were rejected with {e}. Hex has no case semantics: \
             the same 32 bytes are the same key. Compare with `eq_ignore_ascii_case`."
        )
    });
    assert_eq!(
        found, 3,
        "all three distinct signers must still be counted after the case change"
    );
}
