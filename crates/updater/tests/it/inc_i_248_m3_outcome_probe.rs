//! INC-I-248 M3 outcome-metric probe (not an assertion test).
//!
//! Resolves a release trust root from the five retired mainnet genesis PUBLIC keys
//! (the set a snap-joined node seeds today) and prints what release verification
//! does with it. It asserts nothing, so the same probe runs unchanged before and
//! after the fix. Driven only by `.claude/scripts/inc-i-248-m3-probe.sh`; skips when
//! its env is absent.
//!
//! OUTPUT CONTRACT: N/A — probe, prints one line, no assertion.
//! INPUT PARTITIONS: N/A — one fixed input (retired five, threshold 3, h=480_001).

use doli_core::network::Network;
use updater::{verify_release_with_trust_root, Release, TrustRoot, UpdateError};

const RETIRED_GENESIS_PUBKEYS: [&str; 5] = [
    "202047256a8072a8b8f476691b9a5ae87710cc545e8707ca9fe0c803c3e6d3df",
    "effe88fefb6d992a1329277a1d49c7296d252bbc368319cb4bc061119926272b",
    "54323cefd0eabac89b2a2198c95a8f261598c341a8e579a05e26322325c48c2b",
    "2d27fdcc6a240b76ecaea64ad05c9b70d1adad90b6f9c43e8cbbbc0f1ab04116",
    "3047e96b13276dd92ef5eb2d6396e66c29909217f11f8c0544ea7d76a76c7602",
];

#[test]
fn inc_i_248_m3_probe_retired_root_verdict() {
    if std::env::var("INC_I_248_M3_PROBE").ok().as_deref() != Some("1") {
        println!("INC-I-248-M3-PROBE=skipped (run .claude/scripts/inc-i-248-m3-probe.sh)");
        return;
    }
    let keys = RETIRED_GENESIS_PUBKEYS
        .iter()
        .map(|k| k.to_string())
        .collect();
    let root = TrustRoot::resolve(keys, 3, 480_001, Network::Mainnet);
    let release = Release {
        version: "6.44.0".to_string(),
        binary_sha256: "00".repeat(32),
        binary_url_template: String::new(),
        changelog: String::new(),
        published_at: 0,
        signatures: Vec::new(),
        target_networks: Vec::new(),
    };
    let verdict = match verify_release_with_trust_root(&release, &root) {
        Ok(n) => format!("Ok({n})"),
        Err(UpdateError::TrustRootUnavailable { .. }) => "TrustRootUnavailable".to_string(),
        Err(UpdateError::InsufficientSignatures { found, required }) => {
            format!("InsufficientSignatures({found}/{required})")
        }
        Err(e) => format!("Err({e})"),
    };
    println!(
        "INC-I-248-M3 retired-root usable={} verify={}",
        root.is_usable(),
        verdict
    );
}
