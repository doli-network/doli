//! Shared fixtures for the INC-I-215 M1 staging tests (crates/updater/tests/inc_i_215_staging.rs).
//! Split out of that file to keep it inside the 800-line test-file budget.

#![allow(dead_code)]

pub use std::fs;
pub use std::os::unix::fs::PermissionsExt;
pub use std::path::{Path, PathBuf};
pub use updater::{
    platform_identifier, read_staged, sign_release_hash, stage_release, staged_ready_version,
    staging_dir, target_dir_is_writable, verify_release_artifact, verify_release_artifact_bytes,
    GithubReleaseInfo, SignaturesFile, TrustRoot, UpdateError, READY_MARKER, STAGED_CHECKSUMS,
    STAGED_SIGNATURES, STAGED_TARBALL, STAGING_SUBDIR,
};

// ---------------------------------------------------------------------------
// Fixtures — same construction as inc_i_172_install_gate_binding.rs
// ---------------------------------------------------------------------------

/// Deterministic Ed25519 keypair from one seed byte. Test-only material.
pub fn kp(seed: u8) -> crypto::KeyPair {
    crypto::KeyPair::from_private_key(crypto::PrivateKey::from_bytes([seed; 32]))
}

pub fn root_of(signers: &[&crypto::KeyPair], threshold: usize) -> TrustRoot {
    TrustRoot::on_chain(
        signers.iter().map(|k| k.public_key().to_hex()).collect(),
        threshold,
    )
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// The Rust target triple the gate selects for this host (mirrors the crate-private
/// `download::platform_target_triple`). A drift here makes the fixtures FAIL loudly.
pub fn current_triple() -> &'static str {
    match platform_identifier() {
        "linux-x64" => "x86_64-unknown-linux-gnu",
        "linux-arm64" => "aarch64-unknown-linux-gnu",
        "macos-x64" => "x86_64-apple-darwin",
        "macos-arm64" => "aarch64-apple-darwin",
        other => panic!("unsupported test platform: {other}"),
    }
}

/// CHECKSUMS.txt naming `tarball`'s real hash for THIS platform, with decoys for the rest.
pub fn checksums_for(tarball: &[u8], version: &str) -> Vec<u8> {
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

pub fn platform_hash_in(checksums_body: &[u8]) -> String {
    let text = String::from_utf8(checksums_body.to_vec()).expect("CHECKSUMS.txt is utf-8");
    text.lines()
        .find(|l| l.contains(current_triple()) && l.contains(".tar.gz"))
        .and_then(|l| l.split_whitespace().next())
        .expect("fixture must name this platform")
        .to_string()
}

/// A real gzip tarball carrying one entry — the shape `extract_binary_from_tarball` reads.
pub fn tarball_bytes(entry: &str, content: &[u8]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut builder = tar::Builder::new(gz);
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o755);
    builder
        .append_data(&mut header, entry, content)
        .expect("tar append");
    builder
        .into_inner()
        .expect("tar finish")
        .finish()
        .expect("gzip finish")
}

/// Bytes that look like the extracted `doli-node` ELF — what must NEVER be staged.
pub fn elf_payload() -> Vec<u8> {
    let mut elf = b"\x7fELF\x02\x01\x01\x00".to_vec();
    elf.extend_from_slice(&[0x42u8; 512]);
    elf
}

pub struct Fixture {
    pub version: String,
    pub tarball: Vec<u8>,
    pub checksums_body: Vec<u8>,
    pub info: GithubReleaseInfo,
    pub signatures: SignaturesFile,
}

pub fn fixture(version: &str, payload: &[u8], signers: &[&crypto::KeyPair]) -> Fixture {
    let tarball = payload.to_vec();
    let checksums_body = checksums_for(&tarball, version);
    let checksums_sha256 = sha256_hex(&checksums_body);
    let info = GithubReleaseInfo {
        version: version.to_string(),
        tarball_url: format!("https://example.invalid/v{version}/doli.tar.gz"),
        expected_hash: sha256_hex(&tarball),
        checksums_sha256: checksums_sha256.clone(),
        checksums_body: checksums_body.clone(),
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
        version: version.to_string(),
        tarball,
        checksums_body,
        info,
        signatures,
    }
}

pub fn stage(dir: &Path, f: &Fixture) -> PathBuf {
    stage_release(
        dir,
        &f.version,
        &f.tarball,
        &f.checksums_body,
        &f.signatures,
    )
    .expect("staging a verified release into a writable dir must succeed")
}

pub fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read_dir {dir:?}: {e}"))
        .map(|e| {
            e.expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

pub fn staged_names() -> Vec<String> {
    let mut names = vec![
        STAGED_CHECKSUMS.to_string(),
        STAGED_SIGNATURES.to_string(),
        READY_MARKER.to_string(),
        STAGED_TARBALL.to_string(),
    ];
    names.sort();
    names
}

/// True when this process can still create files inside a 0o555 dir — i.e. it is root and
/// the EACCES partition is unobservable here.
pub fn running_as_root(dir: &Path) -> bool {
    let probe = dir.join(".inc-i-215-root-check");
    match fs::File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}
