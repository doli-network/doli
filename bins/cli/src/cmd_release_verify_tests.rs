// OUTPUT CONTRACT: cmd_release_verify::verify_manifest_dir (INC-I-202 M2)
//   O1 Ok(usize)        — distinct maintainer signer count returned on success
//   O2 Err(anyhow)      — refusal, and the message chain the operator reads
//   O3 on-disk fixture  — SIGNATURES.json + CHECKSUMS.txt bytes AFTER the call (a
//                         verifier is read-only; it must never rewrite what it judged)
//   PATHS: cmd_release_verify.rs::verify_manifest_dir
//            -> read <dir>/SIGNATURES.json (serde -> updater::SignaturesFile)
//            -> read <dir>/CHECKSUMS.txt (raw bytes)
//            -> updater::verify_release_manifest (install_gate.rs)
//                 -> L1 normalize_version(sf.version) == normalize_version(version)
//                 -> L2 sf.checksums_sha256 == sha256(checksums_body)
//                 -> L3 verification.rs::verify_release_with_trust_root (distinct k-of-n)
// INPUT PARTITIONS:
//   P1 0-entry manifest, OnChain root 5 keys / threshold 3          — O2 (0/3)
//   P2 3 distinct valid signers, root 5 keys / threshold 3          — O1=3, O3 unchanged
//   P3 real mainnet pubkeys + forged (zero) signature bytes         — O2 (0/3)
//   P4 manifest version 6.26.2, caller asks v6.26.3, 3 valid sigs   — O2 (version binding)
//   P5 manifest checksums_sha256 != sha256(CHECKSUMS.txt on disk)   — O2 (checksums binding)
//   P6 2 valid distinct signers, threshold 3                        — O2 (2/3)
//   P7 one signer, 3 duplicate entries, threshold 3                 — O2 (1/3)
//   P8 caller passes "v6.26.3", manifest says "6.26.3"              — O1=3
//   P9 SIGNATURES.json absent from the dir                          — O2 (names the file)
// MATRIX: 3 outputs x 9 partitions; only reachable cells are asserted.
//   P1 O2 | P2 O1 O3 | P3 O2 | P4 O2 | P5 O2 | P6 O2 | P7 O2 | P8 O1 | P9 O2
//   O3 is asserted once, on the only partition that reaches the end of the chain (P2).

use super::verify_manifest_dir;
use std::path::Path;
use tempfile::TempDir;
use updater::{MaintainerSignature, SignaturesFile, TrustRoot, BOOTSTRAP_MAINTAINER_KEYS_MAINNET};

const VERSION: &str = "6.26.3";
const THRESHOLD: usize = 3;

/// The digest published in the real v6.26.3 SIGNATURES.json. It is used ONLY as a
/// wrong value (P5): pairing it with a synthetic CHECKSUMS.txt would need a preimage.
const REAL_V6263_CHECKSUMS_SHA256: &str =
    "7e0dd5f2a89306f1cd8f0e2a31e45a60b9f3f605400455e737d0fe8c4e3ce6cd";

const CHECKSUMS_BODY: &str = "\
b6f0e7f3c1a2d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d  doli-v6.26.3-linux-x86_64.tar.gz
1c2d3e4f50617283940a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60  doli-v6.26.3-darwin-arm64.tar.gz
";

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// sha256 of the fixture CHECKSUMS.txt, computed at run time. The signed operand must be
/// the hash of bytes that actually exist on disk, so it cannot be a hard-coded constant.
fn fixture_checksums_hash() -> String {
    sha256_hex(CHECKSUMS_BODY.as_bytes())
}

fn keypairs(n: usize) -> Vec<crypto::KeyPair> {
    (0..n).map(|_| crypto::KeyPair::generate()).collect()
}

fn hex_keys(kps: &[crypto::KeyPair]) -> Vec<String> {
    kps.iter().map(|k| k.public_key().to_hex()).collect()
}

fn sign_all(kps: &[crypto::KeyPair], version: &str, hash: &str) -> Vec<MaintainerSignature> {
    kps.iter()
        .map(|kp| updater::sign_release_hash(kp, version, hash))
        .collect()
}

fn manifest(version: &str, hash: &str, sigs: Vec<MaintainerSignature>) -> SignaturesFile {
    SignaturesFile {
        version: version.to_string(),
        checksums_sha256: hash.to_string(),
        signatures: sigs,
    }
}

/// Writes CHECKSUMS.txt + SIGNATURES.json into a fresh temp dir.
fn fixture_dir(sf: &SignaturesFile, checksums: &str) -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    write_checksums(dir.path(), checksums);
    std::fs::write(
        dir.path().join("SIGNATURES.json"),
        serde_json::to_vec_pretty(sf).expect("serialize SIGNATURES.json"),
    )
    .expect("write SIGNATURES.json");
    dir
}

fn write_checksums(dir: &Path, checksums: &str) {
    std::fs::write(dir.join("CHECKSUMS.txt"), checksums).expect("write CHECKSUMS.txt");
}

fn on_chain(keys: Vec<String>, threshold: usize) -> TrustRoot {
    TrustRoot::on_chain(keys, threshold)
}

/// Full anyhow chain on one line, so a `.context()` wrapper cannot hide the cause the
/// operator needs to act on.
fn chain(err: &anyhow::Error) -> String {
    format!("{err:#}")
}

fn refusal_message(dir: &TempDir, version: &str, root: &TrustRoot) -> String {
    let err = verify_manifest_dir(dir.path(), version, root).expect_err("verification must refuse");
    chain(&err)
}

// =============================================================================
// P1 — the actual INC-I-202 defect: the CI scaffold manifest
// =============================================================================

// REQ-202-004 — Decision: an Ok here means the publish gate promotes the exact zero-signature manifest CI wrote for v6.26.2/v6.26.3, which is the bug.
#[test]
fn zero_entry_manifest_is_refused_naming_the_zero_count() {
    let hash = fixture_checksums_hash();
    let sf = manifest(VERSION, &hash, Vec::new());
    let dir = fixture_dir(&sf, CHECKSUMS_BODY);
    let root = on_chain(
        BOOTSTRAP_MAINTAINER_KEYS_MAINNET
            .iter()
            .map(|k| (*k).to_string())
            .collect(),
        THRESHOLD,
    );

    let msg = refusal_message(&dir, VERSION, &root);

    assert!(
        msg.to_lowercase().contains("insufficient"),
        "error must say the signatures are insufficient, got: {msg}"
    );
    assert!(
        msg.contains("0/3"),
        "error must name the 0-of-3 count an operator can act on, got: {msg}"
    );
}

// =============================================================================
// P2 — happy path, and the read-only property of a verifier
// =============================================================================

// REQ-202-005 — Decision: a failure here means the locally callable verifier cannot pass a genuine 3-of-5 release, so the gate would block every real publish.
#[test]
fn three_distinct_valid_signers_verify_and_return_the_signer_count() {
    let hash = fixture_checksums_hash();
    let signers = keypairs(3);
    let bystanders = keypairs(2);
    let sf = manifest(VERSION, &hash, sign_all(&signers, VERSION, &hash));
    let dir = fixture_dir(&sf, CHECKSUMS_BODY);

    let mut keys = hex_keys(&signers);
    keys.extend(hex_keys(&bystanders));
    let root = on_chain(keys, THRESHOLD);

    let sig_before = std::fs::read(dir.path().join("SIGNATURES.json")).expect("read manifest");
    let sums_before = std::fs::read(dir.path().join("CHECKSUMS.txt")).expect("read checksums");

    let count = verify_manifest_dir(dir.path(), VERSION, &root).expect("3-of-5 must verify");

    assert_eq!(count, 3, "must report the distinct signer count");
    assert_eq!(
        std::fs::read(dir.path().join("SIGNATURES.json")).expect("re-read manifest"),
        sig_before,
        "a verifier must not rewrite SIGNATURES.json"
    );
    assert_eq!(
        std::fs::read(dir.path().join("CHECKSUMS.txt")).expect("re-read checksums"),
        sums_before,
        "a verifier must not rewrite CHECKSUMS.txt"
    );
}

// =============================================================================
// P3 — real mainnet keys, forged signature bytes
// =============================================================================

// REQ-202-004 — Decision: an Ok here means the gate counts manifest entries instead of checking Ed25519 bytes, so anyone can name the real maintainer keys and publish.
#[test]
fn real_mainnet_pubkeys_with_forged_signature_bytes_are_refused() {
    let hash = fixture_checksums_hash();
    // 64 zero bytes: a well-formed Ed25519 signature length that verifies against nothing.
    let forged = "00".repeat(64);
    let sigs = BOOTSTRAP_MAINTAINER_KEYS_MAINNET
        .iter()
        .take(3)
        .map(|k| MaintainerSignature {
            public_key: (*k).to_string(),
            signature: forged.clone(),
        })
        .collect();
    let sf = manifest(VERSION, &hash, sigs);
    let dir = fixture_dir(&sf, CHECKSUMS_BODY);
    let root = on_chain(
        BOOTSTRAP_MAINTAINER_KEYS_MAINNET
            .iter()
            .map(|k| (*k).to_string())
            .collect(),
        THRESHOLD,
    );

    let msg = refusal_message(&dir, VERSION, &root);

    assert!(
        msg.contains("0/3"),
        "three forged entries must count as zero valid signers, got: {msg}"
    );
}

// =============================================================================
// P4 — L1 version binding
// =============================================================================

// REQ-202-004 — Decision: an Ok here means a genuine manifest from a previous tag promotes a different release, which is a cross-release replay.
#[test]
fn manifest_for_a_different_version_is_refused() {
    let hash = fixture_checksums_hash();
    let stale = "6.26.2";
    let signers = keypairs(3);
    let sf = manifest(stale, &hash, sign_all(&signers, stale, &hash));
    let dir = fixture_dir(&sf, CHECKSUMS_BODY);
    let root = on_chain(hex_keys(&signers), THRESHOLD);

    let msg = refusal_message(&dir, "v6.26.3", &root);

    assert!(
        msg.contains("version"),
        "error must name the version binding, got: {msg}"
    );
    assert!(
        msg.contains(stale),
        "error must show the version the manifest actually covers, got: {msg}"
    );
}

// =============================================================================
// P5 — L2 checksums binding
// =============================================================================

// REQ-202-004 — Decision: an Ok here means the signatures cover a CHECKSUMS.txt nobody read, so the promoted artifact hashes are unbound.
#[test]
fn manifest_bound_to_a_different_checksums_file_is_refused() {
    let claimed = REAL_V6263_CHECKSUMS_SHA256;
    let signers = keypairs(3);
    let sf = manifest(VERSION, claimed, sign_all(&signers, VERSION, claimed));
    let dir = fixture_dir(&sf, CHECKSUMS_BODY);
    let root = on_chain(hex_keys(&signers), THRESHOLD);

    let msg = refusal_message(&dir, VERSION, &root);

    assert!(
        msg.contains("checksums_sha256"),
        "error must name the checksums binding, got: {msg}"
    );
    assert!(
        msg.contains(claimed),
        "error must show the digest the signatures actually cover, got: {msg}"
    );
}

// =============================================================================
// P6 — sub-threshold
// =============================================================================

// REQ-202-004 — Decision: an Ok here means two maintainers can publish alone, dropping the k-of-n floor the trust root exists to enforce.
#[test]
fn two_valid_signers_against_a_threshold_of_three_are_refused() {
    let hash = fixture_checksums_hash();
    let signers = keypairs(2);
    let bystanders = keypairs(3);
    let sf = manifest(VERSION, &hash, sign_all(&signers, VERSION, &hash));
    let dir = fixture_dir(&sf, CHECKSUMS_BODY);

    let mut keys = hex_keys(&signers);
    keys.extend(hex_keys(&bystanders));
    let root = on_chain(keys, THRESHOLD);

    let msg = refusal_message(&dir, VERSION, &root);

    assert!(
        msg.contains("2/3"),
        "error must report 2 of 3 distinct signers, got: {msg}"
    );
}

// =============================================================================
// P7 — distinct-signer counting
// =============================================================================

// REQ-202-004 — Decision: an Ok here means one maintainer reaches the threshold by repeating their own entry, collapsing k-of-n to 1-of-n.
#[test]
fn one_signer_repeated_three_times_counts_as_one_signer() {
    let hash = fixture_checksums_hash();
    let signer = keypairs(1);
    let bystanders = keypairs(2);
    let one = updater::sign_release_hash(&signer[0], VERSION, &hash);
    let sf = manifest(VERSION, &hash, vec![one.clone(), one.clone(), one]);
    let dir = fixture_dir(&sf, CHECKSUMS_BODY);

    let mut keys = hex_keys(&signer);
    keys.extend(hex_keys(&bystanders));
    let root = on_chain(keys, THRESHOLD);

    let msg = refusal_message(&dir, VERSION, &root);

    assert!(
        msg.contains("1/3"),
        "three copies of one signature must count as one signer, got: {msg}"
    );
}

// =============================================================================
// P8 — `v` prefix tolerance
// =============================================================================

// REQ-202-005 — Decision: a failure here means the gate refuses every real release, because the git tag carries a `v` the signed version string does not.
#[test]
fn caller_tag_with_v_prefix_matches_a_manifest_without_it() {
    let hash = fixture_checksums_hash();
    let signers = keypairs(3);
    let sf = manifest(VERSION, &hash, sign_all(&signers, VERSION, &hash));
    let dir = fixture_dir(&sf, CHECKSUMS_BODY);
    let root = on_chain(hex_keys(&signers), THRESHOLD);

    let count =
        verify_manifest_dir(dir.path(), "v6.26.3", &root).expect("a v-prefixed tag must match");

    assert_eq!(count, 3, "must report the distinct signer count");
}

// =============================================================================
// P9 — absent manifest
// =============================================================================

// REQ-202-005 — Decision: an Ok here means a release with no SIGNATURES.json at all is treated as verified, which is exactly the unsigned release the gate must stop.
#[test]
fn absent_signatures_file_is_refused_naming_the_missing_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_checksums(dir.path(), CHECKSUMS_BODY);
    let root = on_chain(
        BOOTSTRAP_MAINTAINER_KEYS_MAINNET
            .iter()
            .map(|k| (*k).to_string())
            .collect(),
        THRESHOLD,
    );

    let err = verify_manifest_dir(dir.path(), VERSION, &root)
        .expect_err("an absent manifest must never verify");
    let msg = chain(&err);

    assert!(
        msg.contains("SIGNATURES.json"),
        "error must name the missing file, got: {msg}"
    );
}
