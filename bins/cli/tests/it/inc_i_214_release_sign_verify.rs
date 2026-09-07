// OUTPUT CONTRACT: fn cmd_release_sign(wallet_path, version, key, <checksums>) — bins/cli/src/cmd_governance.rs
// OUTPUT CONTRACT: fn cmd_release_verify(version, dir, data_dir, network, <trust_root>) — bins/cli/src/cmd_upgrade.rs
//   Both are reached only across the process boundary: doli-cli is a bin-only crate, so
//   every observable below is read from the real binary (env!("CARGO_BIN_EXE_doli")).
//
//   O1 (sign)   = stdout signature JSON {public_key, signature} + process exit status.
//                 The signature MUST equal updater::sign_release_hash(key, version_bare,
//                 sha256_hex(CHECKSUMS.txt bytes)) — the same operand the fetch path builds.
//   O2 (verify) = stdout banner (trust-root provenance, key count, threshold, data dir,
//                 last_derived_height, bypass notice) + the "Verified: N distinct maintainer
//                 signature(s)" threshold line + process exit status.
//   O3 (both)   = stderr diagnostic text on refusal.
//   No mutable params, no receiver mutation. Persistent writes: none by either command.
//
// PATHS
//   PS1 sign, --checksums names a readable file        -> O1 signature, exit 0, no HTTP fetch
//   PS2 sign, --checksums names a missing path         -> O3 names the path, exit != 0, no O1
//   PS3 sign, --checksums names a directory            -> O3 refusal, exit != 0, no O1
//   PV1 verify, --trust-root bootstrap, on-chain root present -> O2 shows Bootstrap 5/3 + height
//                 + bypass, and the on-chain-signed manifest is REFUSED (root really switched)
//   PV2 verify, no --trust-root                        -> O2 shows OnChain 3/2 + "Verified: 3", exit 0
//   PV3 upgrade, --trust-root bootstrap                -> rejected by clap; the INSTALL path
//                 must never receive the override (resolve_upgrade_trust_root has 2 callers)
//
// INPUT PARTITIONS
//   version         : v99.99.99 — a version that does not exist on GitHub, so any surviving
//                     network fetch fails and PS1 cannot pass by accident.
//   checksums bytes : fixed non-empty bytes; the sha256 is computed in-test from those bytes.
//   signing key     : deterministic keypair minted into a temp wallet JSON (never ~/.ssh/doli).
//   on-chain root   : 3 distinct members (threshold 2) at last_derived_height 331450, so the
//                     banner discriminates it from the compiled bootstrap root (5 keys, threshold 3).
//
// MATRIX (partition x output)
//   PS1 x O1 : signature equals the fixture, exit 0            -> sign_local_checksums_matches_fetch_path_signature
//   PS2 x O3 : error names the path, no O1, exit != 0          -> sign_missing_checksums_file_refuses_and_names_the_path
//   PS3 x O3 : refusal, no O1, exit != 0                       -> sign_missing_checksums_file_refuses_and_names_the_path
//   PV1 x O2 : Bootstrap 5/3 + 331450 + bypass, refusal        -> verify_trust_root_bootstrap_overrides_the_stale_on_chain_root
//   PV2 x O2 : OnChain 3/2 + "Verified: 3", exit 0             -> verify_without_trust_root_flag_still_uses_the_on_chain_root
//   PV3 x O3 : clap rejects --trust-root on upgrade            -> upgrade_must_not_accept_the_trust_root_override
//
// AUTHORISES EDITS TO: bins/cli/src/commands.rs, bins/cli/src/cmd_governance.rs,
//                      bins/cli/src/cmd_upgrade.rs, bins/cli/src/main.rs

use std::path::Path;
use std::process::{Command, Output};

const VERSION_TAG: &str = "v99.99.99";
const VERSION_BARE: &str = "99.99.99";
const CHECKSUMS_BYTES: &[u8] = b"deadbeef  doli-x86_64-unknown-linux-gnu.tar.gz\n\
    cafebabe  doli-aarch64-apple-darwin.tar.gz\n";
const ON_CHAIN_HEIGHT: u64 = 331_450;

fn doli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_doli"))
        .arg("--network")
        .arg("devnet")
        .args(args)
        .output()
        .expect("failed to run the doli binary")
}

fn keypair(seed: u8) -> crypto::KeyPair {
    let mut bytes = [0u8; 32];
    bytes[0] = seed;
    bytes[31] = seed.wrapping_add(7);
    crypto::KeyPair::from_private_key(crypto::PrivateKey::from_bytes(bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn write_key_file(path: &Path, kp: &crypto::KeyPair) {
    let json = serde_json::json!({
        "name": "inc-i-214",
        "version": 3,
        "addresses": [{
            "address": kp.address().to_hex(),
            "public_key": kp.public_key().to_hex(),
            "private_key": kp.private_key().to_hex(),
            "label": "primary",
        }],
    });
    std::fs::write(path, serde_json::to_string_pretty(&json).unwrap()).expect("write key file");
}

fn write_manifest_dir(dir: &Path, signers: &[crypto::KeyPair]) -> String {
    std::fs::write(dir.join("CHECKSUMS.txt"), CHECKSUMS_BYTES).expect("write CHECKSUMS.txt");
    let sha = sha256_hex(CHECKSUMS_BYTES);
    let signatures = signers
        .iter()
        .map(|kp| updater::sign_release_hash(kp, VERSION_BARE, &sha))
        .collect();
    let manifest = updater::SignaturesFile {
        version: VERSION_BARE.to_string(),
        checksums_sha256: sha.clone(),
        signatures,
    };
    std::fs::write(
        dir.join("SIGNATURES.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .expect("write SIGNATURES.json");
    sha
}

fn write_on_chain_root(data_dir: &Path, members: &[crypto::KeyPair], height: u64) {
    let mut set = doli_core::maintainer::MaintainerSet::new();
    set.members = members.iter().map(|kp| *kp.public_key()).collect();
    set.threshold = doli_core::maintainer::MaintainerSet::calculate_threshold(members.len());
    set.last_updated = height;
    let state = storage::MaintainerState {
        set,
        last_derived_height: height,
        ..Default::default()
    };
    state.save(data_dir).expect("write maintainer_state.bin");
}

fn signature_block(stdout: &str) -> Option<serde_json::Value> {
    let start = stdout.find('{')?;
    let end = stdout.rfind('}')?;
    serde_json::from_str(&stdout[start..=end]).ok()
}

// REQ-214-001 — Decision: whether a local-file operand produces the SAME signed bytes as the
// fetch path; a mismatch means every signature made off a draft is unusable at install time.
#[test]
fn sign_local_checksums_matches_fetch_path_signature() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let key_path = tmp.path().join("maintainer.json");
    let kp = keypair(11);
    write_key_file(&key_path, &kp);

    let checksums_path = tmp.path().join("CHECKSUMS.txt");
    std::fs::write(&checksums_path, CHECKSUMS_BYTES).expect("write CHECKSUMS.txt");
    let expected = updater::sign_release_hash(&kp, VERSION_BARE, &sha256_hex(CHECKSUMS_BYTES));

    let out = doli(&[
        "release",
        "sign",
        "--version",
        VERSION_TAG,
        "--key",
        key_path.to_str().unwrap(),
        "--checksums",
        checksums_path.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(
        out.status.success(),
        "PS1/O1: `release sign --checksums <file>` must be accepted and must sign without any \
         network fetch ({VERSION_TAG} does not exist on GitHub, so a surviving fetch cannot \
         succeed). exit={:?} stderr: {stderr}",
        out.status.code()
    );

    let block = signature_block(&stdout).unwrap_or_else(|| {
        panic!("PS1/O1: no signature JSON block on stdout. stdout: {stdout} stderr: {stderr}")
    });
    assert_eq!(
        block["public_key"].as_str().unwrap_or_default(),
        expected.public_key,
        "PS1/O1: the printed public key must be the key that signed"
    );
    assert_eq!(
        block["signature"].as_str().unwrap_or_default(),
        expected.signature,
        "PS1/O1: the local-file signature must be byte-identical to \
         sign_release_hash(key, \"{VERSION_BARE}\", sha256(CHECKSUMS.txt bytes))"
    );
}

// REQ-214-001 — Decision: whether a bad --checksums path refuses loudly; a signature emitted
// over absent or unreadable bytes would be an authorisation over an operand nobody chose.
#[test]
fn sign_missing_checksums_file_refuses_and_names_the_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let key_path = tmp.path().join("maintainer.json");
    write_key_file(&key_path, &keypair(12));

    let missing = tmp.path().join("no-such-CHECKSUMS.txt");
    let out = doli(&[
        "release",
        "sign",
        "--version",
        VERSION_TAG,
        "--key",
        key_path.to_str().unwrap(),
        "--checksums",
        missing.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(
        !out.status.success(),
        "PS2/O3: a missing --checksums file must exit non-zero. stdout: {stdout}"
    );
    assert!(
        stderr.contains(missing.to_str().unwrap()),
        "PS2/O3: the refusal must name the path that could not be read, so the operator can \
         fix the argument. stderr: {stderr}"
    );
    assert!(
        signature_block(&stdout).is_none(),
        "PS2/O1: no signature block may be printed for an unreadable operand. stdout: {stdout}"
    );

    let dir_out = doli(&[
        "release",
        "sign",
        "--version",
        VERSION_TAG,
        "--key",
        key_path.to_str().unwrap(),
        "--checksums",
        tmp.path().to_str().unwrap(),
    ]);
    let dir_stdout = String::from_utf8_lossy(&dir_out.stdout).to_string();
    assert!(
        !dir_out.status.success(),
        "PS3/O3: a directory passed where a file is expected must exit non-zero. \
         stdout: {dir_stdout}"
    );
    assert!(
        signature_block(&dir_stdout).is_none(),
        "PS3/O1: no signature block may be printed for a directory operand. stdout: {dir_stdout}"
    );
}

// REQ-214-002 — Decision: whether --trust-root bootstrap actually replaces the root used by
// the VERIFICATION, not merely the banner text; a banner-only change would report a bypass
// while still judging against the stale on-chain set.
#[test]
fn verify_trust_root_bootstrap_overrides_the_stale_on_chain_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let release_dir = tmp.path().join("release");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&release_dir).expect("mkdir release");
    std::fs::create_dir_all(&data_dir).expect("mkdir data");

    let members = [keypair(21), keypair(22), keypair(23)];
    write_manifest_dir(&release_dir, &members);
    write_on_chain_root(&data_dir, &members, ON_CHAIN_HEIGHT);

    let out = doli(&[
        "release",
        "verify",
        "--version",
        VERSION_TAG,
        "--dir",
        release_dir.to_str().unwrap(),
        "--data-dir",
        data_dir.to_str().unwrap(),
        "--trust-root",
        "bootstrap",
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let banner = format!("{stdout}{stderr}");

    assert!(
        banner.contains("Bootstrap"),
        "PV1/O2: with --trust-root bootstrap the banner must report the Bootstrap provenance, \
         not the on-chain one. exit={:?} output: {banner}",
        out.status.code()
    );
    assert!(
        banner.contains("5 key(s), threshold 3"),
        "PV1/O2: the bootstrap root is the compiled 5-key array at threshold 3; the on-chain \
         fixture is 3 keys at threshold 2, so this string is what distinguishes them. \
         output: {banner}"
    );
    assert!(
        banner.contains(&ON_CHAIN_HEIGHT.to_string()),
        "PV1/O2: the banner must print the on-chain last_derived_height ({ON_CHAIN_HEIGHT}) so a \
         stale snapshot is visible to the operator. output: {banner}"
    );
    assert!(
        banner.to_lowercase().contains("bypass"),
        "PV1/O2: the banner must state that the on-chain root was bypassed. output: {banner}"
    );
    assert!(
        !out.status.success() && !stdout.contains("Verified: 3 distinct maintainer signature(s)"),
        "PV1/O2: the override must reach the VERIFICATION. This manifest is signed by the \
         on-chain members only, so under the compiled bootstrap root it must be refused; \
         accepting it proves the banner changed but the judged root did not. output: {banner}"
    );
}

// REQ-214-002 — Decision: whether the default (no-flag) resolution is untouched; a regression
// here would silently re-point every unattended `release verify` at the compiled keys.
#[test]
fn verify_without_trust_root_flag_still_uses_the_on_chain_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let release_dir = tmp.path().join("release");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&release_dir).expect("mkdir release");
    std::fs::create_dir_all(&data_dir).expect("mkdir data");

    let members = [keypair(21), keypair(22), keypair(23)];
    write_manifest_dir(&release_dir, &members);
    write_on_chain_root(&data_dir, &members, ON_CHAIN_HEIGHT);

    let out = doli(&[
        "release",
        "verify",
        "--version",
        VERSION_TAG,
        "--dir",
        release_dir.to_str().unwrap(),
        "--data-dir",
        data_dir.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(
        out.status.success(),
        "PV2/O2: the unflagged path must still verify a manifest signed by the on-chain \
         members. exit={:?} stdout: {stdout} stderr: {stderr}",
        out.status.code()
    );
    assert!(
        stdout.contains("Trust root: OnChain (3 key(s), threshold 2,"),
        "PV2/O2: without the flag the resolver must still return the on-chain root verbatim. \
         stdout: {stdout}"
    );
    assert!(
        stdout.contains(data_dir.to_str().unwrap()),
        "PV2/O2: the banner must keep naming the data dir the root came from. stdout: {stdout}"
    );
    assert!(
        stdout.contains("Verified: 3 distinct maintainer signature(s)"),
        "PV2/O2: the threshold line must report the DISTINCT signers found. stdout: {stdout}"
    );
    assert!(
        !stdout.to_lowercase().contains("bypass"),
        "PV2/O2: the unflagged path bypasses nothing and must not say it does. stdout: {stdout}"
    );
}

// REQ-214-002 — Decision: whether the override leaked to the INSTALL path; resolve_upgrade_trust_root
// has two callers, and giving `doli upgrade` this flag turns a revocation into a one-word bypass.
#[test]
fn upgrade_must_not_accept_the_trust_root_override() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = doli(&[
        "upgrade",
        "--version",
        VERSION_BARE,
        "--data-dir",
        tmp.path().to_str().unwrap(),
        "--trust-root",
        "bootstrap",
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();

    assert!(
        !out.status.success(),
        "PV3/O3: `doli upgrade --trust-root bootstrap` must be refused; the install path may \
         never take a trust-root override."
    );
    assert!(
        stderr.contains("unexpected argument") && stderr.contains("--trust-root"),
        "PV3/O3: the refusal must come from argument parsing, before any install work. \
         stderr: {stderr}"
    );
}
