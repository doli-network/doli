//! INC-I-215 M1 outcome probe: how many artifacts does the updater stage when its
//! install target is not writable?
//!
//! Usage: `inc_i_215_stage_probe <target-binary-path> <data-dir>`.
//! Writable target -> stage nothing. Not writable -> stage a release into
//! `{data_dir}/updates/` and print the ready-marker path.

use std::path::PathBuf;

use updater::{stage_release, staging_dir, target_dir_is_writable, SignaturesFile};

const PROBE_VERSION: &str = "6.29.0";

fn main() {
    let mut args = std::env::args().skip(1);
    let target = PathBuf::from(args.next().expect("argv[1]: target binary path"));
    let data_dir = PathBuf::from(args.next().expect("argv[2]: data dir"));

    if target_dir_is_writable(&target) {
        println!("writable: nothing staged");
        return;
    }

    let tarball = b"inc-i-215-probe-tarball".to_vec();
    let checksums_body =
        format!("{:064x}  doli-v{}-probe.tar.gz\n", 0u128, PROBE_VERSION).into_bytes();
    let signatures = SignaturesFile {
        version: PROBE_VERSION.to_string(),
        checksums_sha256: format!("{:064x}", 0u128),
        signatures: Vec::new(),
    };

    let ready = stage_release(
        &staging_dir(&data_dir),
        PROBE_VERSION,
        &tarball,
        &checksums_body,
        &signatures,
    )
    .expect("staging into a writable data dir must succeed");

    println!("{}", ready.display());
}
