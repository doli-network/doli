//! Shared fixtures for the INC-I-215 `doli upgrade --from-staged` tests.
//!
//! OUTPUT CONTRACT: N/A — fixture module, no test logic. The contract for the subject
//! under test lives in `inc_i_215_from_staged.rs`.
//! INPUT PARTITIONS: N/A — fixture module.
//!
//! ####################################################################################
//! # SAFETY HAZARD — THIS MODULE EXISTS TO KEEP TESTS AWAY FROM THE LIVE TESTNET      #
//! ####################################################################################
//! `bins/cli/src/upgrade_restart.rs::restart_doli_service` on macOS enumerates
//! `launchctl list`, filters EVERY label containing "doli", and `launchctl kickstart -k`s
//! all of them. This developer machine runs an 18-node local testnet under launchd, so a
//! test that reaches that path bounces the whole fleet.
//!
//! Therefore every `doli upgrade --from-staged` invocation in these tests MUST:
//!   1. pass `--service <fake>` so the CLI takes `restart_specific_service` (which shells
//!      out to `sudo systemctl …`) and NEVER the launchd / pgrep tiers, and
//!   2. run with `PATH` prefixed by [`shim_dir`], which supplies a `sudo` that just
//!      `exec "$@"` and a `systemctl` that exits 0 without touching anything.
//!
//! [`doli_from_staged`] does both. Do not spawn the binary any other way.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Staged version under test: strictly newer than the workspace version (6.29.0), so the
/// downgrade guard is not what refuses the happy path.
pub const STAGED_VERSION: &str = "99.0.0";

/// Bytes of the `doli-node` payload inside the staged tarball.
pub const NEW_NODE_BYTES: &[u8] = b"#!/bin/sh\n# INC-I-215 STAGED PAYLOAD v99.0.0\nexit 0\n";

/// Bytes the install target holds BEFORE the run — what `.backup` must end up holding.
pub const OLD_NODE_BYTES: &[u8] = b"#!/bin/sh\n# INC-I-215 PRE-EXISTING BINARY v6.29.0\nexit 0\n";

/// Fake unit name. It does not exist; the shimmed `systemctl` reports success anyway.
pub const FAKE_SERVICE: &str = "doli-inc-i-215-test.service";

/// `last_derived_height` of the on-chain root, so a stale snapshot is visible in the banner.
pub const ON_CHAIN_HEIGHT: u64 = 331_450;

/// Rust target triple `updater::platform_tarball_hash` looks for in CHECKSUMS.txt.
///
/// Mirrors `updater::platform_identifier` -> `platform_target_triple`, which are
/// `pub(crate)`. Same `cfg` shape, so it cannot drift by accident.
pub fn platform_triple() -> &'static str {
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

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Deterministic maintainer keypair for seed `seed`.
pub fn keypair(seed: u8) -> crypto::KeyPair {
    let mut bytes = [0u8; 32];
    bytes[0] = seed;
    bytes[31] = seed.wrapping_add(7);
    crypto::KeyPair::from_private_key(crypto::PrivateKey::from_bytes(bytes))
}

/// The five members of the on-chain root used by every test here (threshold 3).
pub fn members() -> Vec<crypto::KeyPair> {
    (41u8..46).map(keypair).collect()
}

/// A `.tar.gz` holding exactly one entry, `doli-node`, with `payload`.
///
/// Deliberately NO `doli` entry: `--from-staged` installs the CLI over
/// `std::env::current_exe()`, which under cargo is the test/probe binary itself.
pub fn tarball_with_node(payload: &[u8]) -> Vec<u8> {
    use flate2::write::GzEncoder;
    use flate2::Compression;

    let mut header = tar::Header::new_gnu();
    header.set_path("doli-node").expect("tar path");
    header.set_size(payload.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();

    let encoder = GzEncoder::new(Vec::new(), Compression::fast());
    let mut builder = tar::Builder::new(encoder);
    builder
        .append(&header, payload)
        .expect("append doli-node entry");
    builder
        .into_inner()
        .expect("finish tar")
        .finish()
        .expect("finish gzip")
}

/// A `CHECKSUMS.txt` body naming `tarball`'s sha256 for THIS platform.
///
/// Line shape is what `platform_tarball_hash` parses: the hash is the first
/// whitespace-separated token, and the line carries the target triple and `.tar.gz`.
pub fn checksums_body(version: &str, tarball: &[u8]) -> Vec<u8> {
    format!(
        "{}  doli-node-v{}-{}.tar.gz\n",
        sha256_hex(tarball),
        version,
        platform_triple()
    )
    .into_bytes()
}

/// A `SignaturesFile` for `version` over `checksums`, signed by `signers`.
pub fn manifest(
    version: &str,
    checksums: &[u8],
    signers: &[crypto::KeyPair],
) -> updater::SignaturesFile {
    let sha = sha256_hex(checksums);
    updater::SignaturesFile {
        version: version.to_string(),
        checksums_sha256: sha.clone(),
        signatures: signers
            .iter()
            .map(|kp| updater::sign_release_hash(kp, version, &sha))
            .collect(),
    }
}

/// Write a 5-member on-chain maintainer root (threshold 3) into `data_dir`.
pub fn write_on_chain_root(data_dir: &Path, members: &[crypto::KeyPair], height: u64) {
    std::fs::create_dir_all(data_dir).expect("mkdir data dir");
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

/// Lay a staging directory out by hand, using the names `updater::read_staged` reads.
///
/// Written directly rather than through `stage_release` so a test can omit or corrupt one
/// file: the point of most of these tests is what the installer does with a BAD staging.
pub fn write_staging(
    dir: &Path,
    ready_version: Option<&str>,
    tarball: Option<&[u8]>,
    checksums: Option<&[u8]>,
    signatures: Option<&updater::SignaturesFile>,
) {
    std::fs::create_dir_all(dir).expect("mkdir staging dir");
    if let Some(t) = tarball {
        std::fs::write(dir.join(updater::STAGED_TARBALL), t).expect("write tarball");
    }
    if let Some(c) = checksums {
        std::fs::write(dir.join(updater::STAGED_CHECKSUMS), c).expect("write CHECKSUMS.txt");
    }
    if let Some(s) = signatures {
        std::fs::write(
            dir.join(updater::STAGED_SIGNATURES),
            serde_json::to_vec_pretty(s).expect("serialise manifest"),
        )
        .expect("write SIGNATURES.json");
    }
    if let Some(v) = ready_version {
        std::fs::write(dir.join(updater::READY_MARKER), format!("{v}\n")).expect("write ready");
    }
}

/// A complete, VALID staging for `STAGED_VERSION` signed by 3 of the 5 root members.
pub fn write_good_staging(dir: &Path) {
    let all = members();
    let tarball = tarball_with_node(NEW_NODE_BYTES);
    let checksums = checksums_body(STAGED_VERSION, &tarball);
    let sf = manifest(STAGED_VERSION, &checksums, &all[..3]);
    write_staging(
        dir,
        Some(STAGED_VERSION),
        Some(&tarball),
        Some(&checksums),
        Some(&sf),
    );
}

/// Pre-populate the node install target with [`OLD_NODE_BYTES`].
pub fn write_install_target(target: &Path) {
    std::fs::create_dir_all(target.parent().expect("target has a parent")).expect("mkdir bin");
    std::fs::write(target, OLD_NODE_BYTES).expect("write install target");
}

/// Create `<tmp>/shim` holding a permissive `sudo` and an inert `systemctl`.
///
/// SAFETY: this is what keeps `sudo systemctl restart <fake unit>` from ever reaching the
/// real `systemctl` (or, on a host that has one, a real unit).
pub fn shim_dir(tmp: &Path) -> PathBuf {
    let dir = tmp.join("shim");
    std::fs::create_dir_all(&dir).expect("mkdir shim");
    write_shim(&dir, "sudo", "#!/bin/sh\nexec \"$@\"\n");
    write_shim(&dir, "systemctl", "#!/bin/sh\nexit 0\n");
    dir
}

fn write_shim(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write shim");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod shim");
    }
}

/// Run `doli --network devnet upgrade --from-staged …` with the restart shims in place.
///
/// SAFETY: `--service FAKE_SERVICE` is passed unconditionally. Never call the CLI for
/// this feature without going through here.
pub fn doli_from_staged(
    tmp: &Path,
    staging: &Path,
    data_dir: &Path,
    node_target: &Path,
    extra: &[&str],
) -> Output {
    let shim = shim_dir(tmp);
    let path = match std::env::var("PATH") {
        Ok(p) => format!("{}:{}", shim.display(), p),
        Err(_) => shim.display().to_string(),
    };
    Command::new(env!("CARGO_BIN_EXE_doli"))
        .env("PATH", path)
        .arg("--network")
        .arg("devnet")
        .arg("upgrade")
        .arg("--from-staged")
        .arg(staging)
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--doli-node-path")
        .arg(node_target)
        .arg("--service")
        .arg(FAKE_SERVICE)
        .arg("--yes")
        .args(extra)
        .output()
        .expect("failed to run the doli binary")
}

/// stdout + stderr of a run, joined — refusals may land on either stream.
pub fn combined(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Absolute path of a `bins/cli` source file, resolved at RUNTIME.
///
/// Runtime, not `include_str!`: a module that does not exist yet must make the test FAIL,
/// not fail to compile.
pub fn cli_src(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name)
}

/// Source text of a `bins/cli` module, or `None` when the file does not exist.
pub fn read_cli_src(name: &str) -> Option<String> {
    std::fs::read_to_string(cli_src(name)).ok()
}

/// Does `text` NAME the file `token`, rather than merely containing its letters?
///
/// `contains("ready")` is true of "already", which would let a clap usage error pass for
/// "the refusal named the ready marker". The token must start at a non-alphanumeric
/// boundary.
pub fn names_file(text: &str, token: &str) -> bool {
    let bytes = text.as_bytes();
    text.match_indices(token).any(|(i, _)| {
        i == 0 || !(bytes[i - 1] as char).is_ascii_alphanumeric() && bytes[i - 1] != b'_'
    })
}
