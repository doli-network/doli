//! INC-I-215 M3 outcome-probe fixture: lay out a real staged handoff on disk.
//!
//! argv: `<scratch_dir>`. Builds, using the SHIPPED libraries only:
//!   `<scratch>/data/`  — a 3-of-5 on-chain maintainer trust root (threshold 3)
//!   `<scratch>/updates/` — `release.tar.gz` (one `doli-node` entry), `CHECKSUMS.txt` in
//!                        the real per-platform format, `SIGNATURES.json` signed by 3 of
//!                        the 5 keys for v99.0.0, and the `ready` marker
//!   `<scratch>/bin/doli-node` — the OLD binary the install must replace and back up
//!
//! Prints nothing but a final `ok`. `fn main` only — no `pub fn` (wiring-census rule).
//!
//! The tarball deliberately carries NO `doli` entry: `--from-staged` installs the CLI over
//! `std::env::current_exe()`, which under `cargo run` is a build artifact.

use std::path::Path;

const VERSION: &str = "99.0.0";
const NEW_NODE_BYTES: &[u8] = b"#!/bin/sh\n# INC-I-215 STAGED PAYLOAD v99.0.0\nexit 0\n";
const OLD_NODE_BYTES: &[u8] = b"#!/bin/sh\n# INC-I-215 PRE-EXISTING BINARY v6.29.0\nexit 0\n";
const ON_CHAIN_HEIGHT: u64 = 331_450;

fn main() {
    let scratch = std::env::args()
        .nth(1)
        .expect("usage: inc_i_215_from_staged_fixture <scratch_dir>");
    let scratch = Path::new(&scratch);

    let keys: Vec<crypto::KeyPair> = (41u8..46)
        .map(|seed| {
            let mut bytes = [0u8; 32];
            bytes[0] = seed;
            bytes[31] = seed.wrapping_add(7);
            crypto::KeyPair::from_private_key(crypto::PrivateKey::from_bytes(bytes))
        })
        .collect();

    let data_dir = scratch.join("data");
    std::fs::create_dir_all(&data_dir).expect("mkdir data");
    let mut set = doli_core::maintainer::MaintainerSet::new();
    set.members = keys.iter().map(|kp| *kp.public_key()).collect();
    set.threshold = doli_core::maintainer::MaintainerSet::calculate_threshold(keys.len());
    set.last_updated = ON_CHAIN_HEIGHT;
    storage::MaintainerState {
        set,
        last_derived_height: ON_CHAIN_HEIGHT,
        ..Default::default()
    }
    .save(&data_dir)
    .expect("save maintainer_state.bin");

    let tarball = tarball_with_node(NEW_NODE_BYTES);
    let checksums = format!(
        "{}  doli-node-v{}-{}.tar.gz\n",
        sha256_hex(&tarball),
        VERSION,
        platform_triple()
    )
    .into_bytes();
    let sha = sha256_hex(&checksums);
    let signatures = updater::SignaturesFile {
        version: VERSION.to_string(),
        checksums_sha256: sha.clone(),
        signatures: keys[..3]
            .iter()
            .map(|kp| updater::sign_release_hash(kp, VERSION, &sha))
            .collect(),
    };

    // Through the shipped stager, so the probe exercises the real handoff layout.
    let staging = scratch.join("updates");
    updater::stage_release(&staging, VERSION, &tarball, &checksums, &signatures)
        .expect("stage the release");

    let target = scratch.join("bin").join("doli-node");
    std::fs::create_dir_all(target.parent().expect("bin parent")).expect("mkdir bin");
    std::fs::write(&target, OLD_NODE_BYTES).expect("write the pre-existing binary");

    println!("ok");
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn platform_triple() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return "x86_64-unknown-linux-gnu";
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return "aarch64-unknown-linux-gnu";
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return "x86_64-apple-darwin";
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return "aarch64-apple-darwin";
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
    )))]
    return "unknown";
}

fn tarball_with_node(payload: &[u8]) -> Vec<u8> {
    use flate2::write::GzEncoder;
    use flate2::Compression;

    let mut header = tar::Header::new_gnu();
    header.set_path("doli-node").expect("tar path");
    header.set_size(payload.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();

    let encoder = GzEncoder::new(Vec::new(), Compression::fast());
    let mut builder = tar::Builder::new(encoder);
    builder.append(&header, payload).expect("append doli-node");
    builder
        .into_inner()
        .expect("finish tar")
        .finish()
        .expect("finish gzip")
}
