//! INC-I-236 Run A — pre-height compatibility locks and gate configuration.
//!
// covers: block.rs signature.rs signature_strict.rs lib.rs defaults.rs env_loader.rs mod.rs types.rs error.rs data.rs transaction.rs tx_types.rs slash_evidence.rs validation
// covers: pool.rs equivocation.rs headers.rs block_lifecycle.rs response.rs sync_engine manager
// covers: tx_processing.rs block_handling.rs fork_recovery.rs header_auth.rs init.rs assembly.rs signing.rs production validation_checks event_loop.rs wedge_escape.rs snapshot_install.rs apply_block rollback.rs periodic.rs
// covers: tests_inc_i236_m5_headers.rs
//
// OUTPUT CONTRACT: fn validate_block_with_mode(&Block, &ValidationContext, ValidationMode) -> Result<(), ValidationError>
//   O1 return verdict; O2 none (pure: no mutable params, receiver or store)
// OUTPUT CONTRACT: fn BlockHeader::hash(&self) -> Hash, and bincode of BlockHeader / Block
//   O1 hash; O2 encoded bytes
// OUTPUT CONTRACT: fn NetworkParams::defaults(Network) and fn NetworkParams::load(Network)
//   O1 .inc_i_236_block_signature_activation_height; O5 per-process OnceLock cache (load only)
// PATHS: P-BELOW (ctx AH above block height) x {Full, Light, Replay} | P-DEF (defaults) | P-ENV (load, override set)
// INPUT PARTITIONS:
//   P-BELOW: pi empty | valid 64-byte producer signature | 64 garbage bytes, AH in {h+1, u64::MAX} -> one verdict (Ok) per mode
//   hash/bincode: signed copy vs unsigned copy -> same hash(); equal encodings once pi is cleared; signed is 64 bytes longer
//   P-DEF: mainnet -> 806_859; testnet -> 330_849; devnet -> 0
//   P-ENV: override set -> mainnet and testnet ignore it (keep their pin); devnet reads it
// MATRIX:
//   O1,O2(hash/bincode) x {signed, unsigned}          -> req_auth_014_signed_and_unsigned_copies_differ_only_in_pi
//   O1(validate) x P-BELOW x 3 pi x 2 AH x 3 modes    -> req_auth_014_below_ah_verdict_ignores_pi_all_modes
//   O1(defaults) x P-DEF                              -> req_auth_015_defaults_pinned_per_network
//   O1(load) x P-ENV (re-exec child, clean cache)     -> req_auth_015_env_override_honoured_on_devnet_only

use super::inc_i_236_forged_block_authn::{forge_block_as, sign_into_pi, HEIGHT, MODES};
use crypto::KeyPair;
use doli_core::block::{Block, BLOCK_SIGNATURE_DOMAIN};
use doli_core::validation::validate_block_with_mode;
use doli_core::{Network, NetworkParams};

const AH_ENV: &str = "DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_236_RUN_A_ENV_CHILD";
const OVERRIDE: u64 = 777;
const SELF_NAME: &str =
    "inc_i_236_run_a_pre_height::req_auth_015_env_override_honoured_on_devnet_only";

/// One block by a real key, as (unsigned, signed) copies.
fn copies() -> (Block, Block, KeyPair) {
    let kp = KeyPair::from_seed([0x3C; 32]);
    let (unsigned, _ctx) = forge_block_as(*kp.public_key());
    let mut signed = unsigned.clone();
    sign_into_pi(&mut signed, &kp);
    (unsigned, signed, kp)
}

fn encode<T: serde::Serialize>(v: &T) -> Vec<u8> {
    bincode::serialize(v).expect("bincode encode")
}

// REQ-AUTH-014 — Decision: a failure means a signed block has a different identity or wire form for an old node, so old and new builds split before the height.
#[test]
fn req_auth_014_signed_and_unsigned_copies_differ_only_in_pi() {
    let (unsigned, signed, kp) = copies();
    assert!(unsigned.header.vdf_proof.pi.is_empty());
    assert_eq!(signed.header.vdf_proof.pi.len(), 64);
    let mut preimage = BLOCK_SIGNATURE_DOMAIN.to_vec();
    preimage.extend_from_slice(unsigned.header.hash().as_bytes());
    let msg = crypto::hash::hash(&preimage);
    let sig = crypto::Signature::try_from_slice(&signed.header.vdf_proof.pi).expect("64-byte sig");
    assert!(
        crypto::signature::verify(msg.as_bytes(), &sig, kp.public_key()).is_ok(),
        "NON-VACUITY: signed copy must carry a real producer signature"
    );

    assert_eq!(
        signed.header.hash(),
        unsigned.header.hash(),
        "hash() must ignore pi"
    );
    assert_eq!(signed.hash(), unsigned.hash(), "block hash must ignore pi");

    let (eu, es) = (encode(&unsigned.header), encode(&signed.header));
    assert_ne!(eu, es, "signed header bytes must carry the signature");
    assert_eq!(es.len(), eu.len() + 64, "only the 64 pi bytes are added");
    let mut stripped = signed.clone();
    stripped.header.vdf_proof.pi.clear();
    assert_eq!(
        encode(&stripped.header),
        eu,
        "header bytes differ only in pi"
    );
    assert_eq!(
        encode(&stripped),
        encode(&unsigned),
        "block bytes differ only in pi"
    );
    assert_eq!(encode(&signed).len(), encode(&unsigned).len() + 64);
}

// REQ-AUTH-014 — Decision: a failure means pi is read below the height, so an old build and a new build give different verdicts on the same block.
#[test]
fn req_auth_014_below_ah_verdict_ignores_pi_all_modes() {
    let (unsigned, signed, _kp) = copies();
    let mut garbage = unsigned.clone();
    garbage.header.vdf_proof.pi = vec![0xC3; 64];
    let (_b, base_ctx) = forge_block_as(unsigned.header.producer);

    for ah in [HEIGHT + 1, u64::MAX] {
        let mut ctx = base_ctx.clone();
        ctx.inc_i_236_block_signature_activation_height = ah;
        assert!(ctx.current_height < ah, "PRECONDITION: block below the AH");
        for mode in MODES {
            let base = validate_block_with_mode(&unsigned, &ctx, mode);
            assert_eq!(base, Ok(()), "unsigned block below AH {ah}, {mode:?}");
            for (what, b) in [("valid signature", &signed), ("garbage pi", &garbage)] {
                assert_eq!(
                    validate_block_with_mode(b, &ctx, mode),
                    base,
                    "{what} below AH {ah}, {mode:?}: verdict must equal the unsigned copy"
                );
            }
        }
    }
}

// REQ-AUTH-015 — Decision: a failure means the rule engages on mainnet or testnet at a height other than the pinned one, or devnet loses its armed default.
#[test]
fn req_auth_015_defaults_pinned_per_network() {
    for (network, expected) in [
        // Pins of the four defense gates (user decisions); see
        // `network_params::tests_defense_pin`.
        (Network::Mainnet, 806_859),
        (Network::Testnet, 330_849),
        (Network::Devnet, 0),
    ] {
        assert_eq!(
            NetworkParams::defaults(network).inc_i_236_block_signature_activation_height,
            expected,
            "{network:?}"
        );
    }
}

// REQ-AUTH-015 — Decision: a failure means an operator env var can arm or move the rule on a mainnet or testnet build (a local consensus split).
#[test]
fn req_auth_015_env_override_honoured_on_devnet_only() {
    if std::env::var_os(CHILD_ENV).is_none() {
        let out = std::process::Command::new(std::env::current_exe().expect("current exe"))
            .args([SELF_NAME, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD_ENV, "1")
            .env(AH_ENV, OVERRIDE.to_string())
            .output()
            .expect("spawn child test process");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "env-override child failed or did not run:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }

    assert_eq!(std::env::var(AH_ENV).as_deref(), Ok("777"), "PRECONDITION");
    assert_eq!(
        NetworkParams::load(Network::Mainnet).inc_i_236_block_signature_activation_height,
        806_859,
        "mainnet must ignore the override and keep its pin"
    );
    assert_eq!(
        NetworkParams::load(Network::Testnet).inc_i_236_block_signature_activation_height,
        330_849,
        "testnet must ignore the override and keep the 2026-10-25 pin"
    );
    assert_eq!(
        NetworkParams::load(Network::Devnet).inc_i_236_block_signature_activation_height,
        OVERRIDE,
        "devnet must honour the override"
    );
}
