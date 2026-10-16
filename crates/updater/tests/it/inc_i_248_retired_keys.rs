// INC-I-248 M3. REQ-248-001 (Must) — Decision: a failure means a release trust root that holds a retired, publicly compromised genesis key can still authorise an install, or the fix broke a clean root.
// covers: crates/updater/src/trust_root.rs, crates/updater/src/constants.rs, crates/updater/src/verification.rs, crates/updater/src/lib.rs, crates/updater/src/types.rs
//
// OUTPUT CONTRACT:
// Functions under test:
//   F1 `TrustRoot::resolve(keys, threshold, last_derived_height, network) -> TrustRoot`
//   F2 `TrustRoot::is_usable(&self) -> bool`
//   F3 `verify_release_with_trust_root(&Release, &TrustRoot) -> Result<usize, UpdateError>`
// Outputs:
//   O1 mutable params  : NONE (F1 takes `keys` by value; F2/F3 take shared refs)
//   O2 receiver        : NONE (F2 is `&self`, no interior mutability)
//   O3 return value    : F1 -> keys() / threshold() / provenance() / is_usable();
//                        F2 -> bool; F3 -> Ok(n) | Err(TrustRootUnavailable) |
//                        Err(InsufficientSignatures)
//   O4 persistent store: NONE
//   O5 global state    : NONE
//   O6 events          : `tracing` records only; not asserted (no capturing subscriber)
// Paths:
//   P1 F1 non-empty keys            -> OnChain, keys verbatim
//   P2 F1 empty keys, height 0      -> Bootstrap (compiled keys of `network`)
//   P3 F3 root not usable           -> Err(TrustRootUnavailable)
//   P4 F3 usable, valid < threshold -> Err(InsufficientSignatures)
//   P5 F3 usable, valid >= threshold-> Ok(valid)
//
// INPUT PARTITIONS: key-set content of the root
//   K1 exactly the five retired genesis keys
//   K2 four clean test keys + one retired key, lowercase (each of the five)
//   K3 four clean test keys + one retired key, UPPERCASE (each of the five)
//   K4 four clean test keys only (non-vacuity control for K2/K3)
//   K5 compiled bootstrap keys of each network (via P2 and `bootstrap`)
//   K6 canonical rotated mainnet set, and the compiled testnet five, as on-chain roots
//
// MATRIX (input -> O3 asserted)
//   case | input             | F1 provenance / keys   | F2    | F3
//   1    | K1, every network | OnChain (not Bootstrap) | false | TrustRootUnavailable
//   2    | K2, K3, Mainnet   | OnChain (not Bootstrap) | false | TrustRootUnavailable
//   2c   | K4, Mainnet       | OnChain / verbatim      | true  | Ok(3)
//   3    | K5, every network | Bootstrap               | true  | n/a
//   4    | K6                | OnChain / verbatim      | true  | n/a

use doli_core::network::Network;
use updater::{
    bootstrap_maintainer_keys, sign_release_hash, verify_release_with_trust_root, Release,
    TrustRoot, TrustRootProvenance, UpdateError,
};

const RETIRED: [&str; 5] = [
    "202047256a8072a8b8f476691b9a5ae87710cc545e8707ca9fe0c803c3e6d3df",
    "effe88fefb6d992a1329277a1d49c7296d252bbc368319cb4bc061119926272b",
    "54323cefd0eabac89b2a2198c95a8f261598c341a8e579a05e26322325c48c2b",
    "2d27fdcc6a240b76ecaea64ad05c9b70d1adad90b6f9c43e8cbbbc0f1ab04116",
    "3047e96b13276dd92ef5eb2d6396e66c29909217f11f8c0544ea7d76a76c7602",
];

const NETWORKS: [Network; 3] = [Network::Mainnet, Network::Testnet, Network::Devnet];
const DERIVED_AT: u64 = 480_001;
const VERSION: &str = "9.9.9";
const SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn kp(seed: u8) -> crypto::KeyPair {
    crypto::KeyPair::from_private_key(crypto::PrivateKey::from_bytes([seed; 32]))
}

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

fn assert_refused(case: &str, root: &TrustRoot, release: &Release) {
    assert_ne!(
        root.provenance(),
        TrustRootProvenance::Bootstrap,
        "{case}: a host with an on-chain set must never fall back to the compiled keys"
    );
    assert!(
        !root.is_usable(),
        "{case}: root holds a retired genesis key but is_usable() is true"
    );
    let got = verify_release_with_trust_root(release, root);
    assert!(
        !matches!(got, Err(UpdateError::InsufficientSignatures { .. })),
        "{case}: expected TrustRootUnavailable, got InsufficientSignatures: {got:?}"
    );
    assert!(
        got.is_err(),
        "{case}: root holds a retired genesis key but the release verified: {got:?}"
    );
    assert!(
        matches!(got, Err(UpdateError::TrustRootUnavailable { .. })),
        "{case}: expected TrustRootUnavailable, got {got:?}"
    );
}

fn owned(keys: &[&str]) -> Vec<String> {
    keys.iter().map(|k| (*k).to_string()).collect()
}

#[test]
fn inc_i_248_retired_keys_never_usable_root() {
    let clean = [kp(0x41), kp(0x42), kp(0x43), kp(0x44)];
    let clean_hex: Vec<String> = clean.iter().map(|k| k.public_key().to_hex()).collect();
    let release = release_signed_by(&[&clean[0], &clean[1], &clean[2]]);

    // case 3
    for network in NETWORKS {
        assert!(
            TrustRoot::bootstrap(network).is_usable(),
            "case 3 {network:?}: compiled bootstrap root must stay usable"
        );
        let fresh = TrustRoot::resolve(Vec::new(), 0, 0, network);
        assert_eq!(
            fresh.provenance(),
            TrustRootProvenance::Bootstrap,
            "case 3 {network:?}: never-derived host must resolve to Bootstrap"
        );
        assert!(
            fresh.is_usable(),
            "case 3 {network:?}: never-derived Bootstrap root must stay usable"
        );
    }

    // case 4
    for network in [Network::Mainnet, Network::Testnet] {
        let canonical = owned(bootstrap_maintainer_keys(network));
        let root = TrustRoot::resolve(canonical.clone(), 3, DERIVED_AT, network);
        assert_eq!(
            root.provenance(),
            TrustRootProvenance::OnChain,
            "case 4 {network:?}: canonical on-chain set must resolve OnChain"
        );
        assert_eq!(
            root.keys(),
            canonical.as_slice(),
            "case 4 {network:?}: canonical on-chain keys must be carried verbatim"
        );
        assert!(
            root.is_usable(),
            "case 4 {network:?}: canonical rotated on-chain set must stay usable"
        );
    }

    // case 2c
    let control = TrustRoot::resolve(clean_hex.clone(), 3, DERIVED_AT, Network::Mainnet);
    assert!(
        control.is_usable(),
        "case 2c: four clean keys must be usable"
    );
    assert_eq!(
        verify_release_with_trust_root(&release, &control).ok(),
        Some(3),
        "case 2c: the release must verify 3-of-4 against the clean root"
    );

    // case 1
    for network in NETWORKS {
        let root = TrustRoot::resolve(owned(&RETIRED), 3, DERIVED_AT, network);
        assert_refused(&format!("case 1 {network:?} retired five"), &root, &release);
    }

    // case 2
    for (i, retired) in RETIRED.iter().enumerate() {
        for (label, key) in [
            ("lowercase", retired.to_string()),
            ("UPPERCASE", retired.to_ascii_uppercase()),
        ] {
            let mut keys = clean_hex.clone();
            keys.insert(i.min(keys.len()), key);
            let root = TrustRoot::resolve(keys, 3, DERIVED_AT, Network::Mainnet);
            assert_refused(
                &format!("case 2 retired[{i}] {label} mixed into clean set"),
                &root,
                &release,
            );
        }
    }
}
