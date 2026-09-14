//! INC-I-193 M1 / TEST-193-07 — the attestor-refill activation-height gate.
//!
//! OUTPUT CONTRACT: NetworkParams::defaults(Network) / env_loader::load_from_env(Network)
//!   O1 defaults(net).inc_i_193_attestor_refill_activation_height — the pinned value
//!   O2 load_from_env(net).<same field> — the env-resolved value
//!   O3 the pin vs the tip it was measured against and vs its neighbouring gates
//!   O4/O5/O6 none — both are pure constructors over env vars.
//! PATHS  mainnet arm (locked to defaults) | non-mainnet arm (env_parse)
//! INPUT PARTITIONS  IP-M mainnet | IP-T testnet | IP-D devnet; env unset | env set

use std::sync::Mutex;

use crate::Network;

use super::NetworkParams;

/// Pinned 2026-09-14 19:28Z (owner decision, HC-6). First boundary at/above it is h = 475_200
/// (epoch 1320 x 360).
const REFILL_MAINNET: u64 = 475_000;
/// Mainnet tip read by `getChainInfo` at pin time (slot 469_700) — a frozen floor, not a probe.
const REFILL_MAINNET_TIP_AT_PIN: u64 = 462_424;
/// Re-pinned 2026-09-14 (was 195_000). First boundary at/above it is h = 192_168 (epoch 5338 x 36).
const REFILL_TESTNET: u64 = 192_163;
/// Testnet tip read by `getChainInfo` on 2026-09-14, at the ORIGINAL pin time — a frozen floor,
/// not a probe. The re-pin measured 192_081 (slot 1_237_319), still below the gate.
const REFILL_TESTNET_TIP_AT_PIN: u64 = 191_020;
const REFILL_ENV: &str = "DOLI_INC_I_193_ATTESTOR_REFILL_ACTIVATION_HEIGHT";

/// Env vars are process-global; these tests mutate them. Serialize.
static ENV_MUTEX: Mutex<()> = Mutex::new(());

// TEST-193-07 — Decision: a failure means the gate shipped live on mainnet or dormant on
// testnet — the INC-I-054 shape.
#[test]
fn test_193_07_attestor_refill_gate_pinned_per_network() {
    let t = NetworkParams::defaults(Network::Testnet);
    let h = t.inc_i_193_attestor_refill_activation_height;

    assert_eq!(
        NetworkParams::defaults(Network::Mainnet).inc_i_193_attestor_refill_activation_height,
        REFILL_MAINNET,
        "O1/IP-M: mainnet is pinned at 475_000 (owner decision 2026-09-14)"
    );
    assert_eq!(h, REFILL_TESTNET, "O1/IP-T: testnet is pinned at 192_163");
    assert_eq!(
        NetworkParams::defaults(Network::Devnet).inc_i_193_attestor_refill_activation_height,
        u64::MAX,
        "O1/IP-D: devnet has <= 50 producers, so the gated branch is dormant — keep it frozen"
    );
}

// TEST-193-07 — Decision: a failure means the gate is retroactive or bundled onto a
// neighbouring height, which reinterprets sealed history (INC-I-054).
#[test]
fn test_193_07_attestor_refill_testnet_gate_is_above_the_tip_it_was_measured_against() {
    let t = NetworkParams::defaults(Network::Testnet);
    let h = t.inc_i_193_attestor_refill_activation_height;

    assert!(
        h > REFILL_TESTNET_TIP_AT_PIN,
        "O3/IP-T: the gate ({h}) must be strictly above the tip it was measured against \
         ({REFILL_TESTNET_TIP_AT_PIN}) — crossing it before all 18 nodes run the new binary is \
         a RETROACTIVE rule change (INC-I-054). Both sides are compile-time constants: GREEN \
         does NOT mean the live tip is still below it, so re-measure before deploy"
    );
    assert_ne!(h, 0, "O3/IP-T: a gate of 0 reinterprets sealed history");
    assert_ne!(h, u64::MAX, "O3/IP-T: u64::MAX means never on testnet");
    for (name, other) in [
        ("inc_i_190", t.inc_i_190_floor_bound_activation_height),
        ("epoch_prune", t.epoch_prune_activation_height),
        ("ghost_exclusion", t.ghost_exclusion_activation_height),
    ] {
        assert_ne!(h, other, "O3/IP-T: must not be bundled onto {name}");
    }
}

// TEST-193-07 — Decision: a failure means the mainnet gate is retroactive (crossed before the
// fleet runs the binary), bundled onto a neighbouring height, or moved after being crossed —
// every one of them the INC-I-054 shape on the chain where the gated path is LIVE (> 50).
#[test]
fn test_193_07_attestor_refill_mainnet_gate_is_above_the_tip_it_was_measured_against() {
    let m = NetworkParams::defaults(Network::Mainnet);
    let h = m.inc_i_193_attestor_refill_activation_height;

    assert!(
        h > REFILL_MAINNET_TIP_AT_PIN,
        "O3/IP-M: the gate ({h}) must be strictly above the tip it was measured against \
         ({REFILL_MAINNET_TIP_AT_PIN}) — crossing it before every mainnet producer runs \
         >= 6.40.0 is a RETROACTIVE rule change (INC-I-054). Both sides are compile-time \
         constants: GREEN does NOT mean the live tip is still below it, so re-measure before deploy"
    );
    assert_ne!(h, 0, "O3/IP-M: a gate of 0 reinterprets sealed history");
    assert_ne!(
        h,
        u64::MAX,
        "O3/IP-M: u64::MAX means never — the owner pinned it"
    );
    for (name, other) in [
        ("inc_i_190", m.inc_i_190_floor_bound_activation_height),
        ("inc_i_178", m.inc_i_178_attestation_bls_activation_height),
        ("epoch_prune", m.epoch_prune_activation_height),
        ("ghost_exclusion", m.ghost_exclusion_activation_height),
    ] {
        assert_ne!(h, other, "O3/IP-M: must not be bundled onto {name}");
    }
}

// TEST-193-07 — Decision: a failure means one operator can move a consensus gate from the
// environment on mainnet, forking the chain without a binary release.
#[test]
fn test_193_07_attestor_refill_env_override_off_mainnet_only() {
    let _lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let original = std::env::var(REFILL_ENV);

    std::env::set_var(REFILL_ENV, "7");
    let testnet = super::env_loader::load_from_env(Network::Testnet);
    let devnet = super::env_loader::load_from_env(Network::Devnet);
    let mainnet = super::env_loader::load_from_env(Network::Mainnet);

    match original {
        Ok(v) => std::env::set_var(REFILL_ENV, v),
        Err(_) => std::env::remove_var(REFILL_ENV),
    }

    assert_eq!(
        testnet.inc_i_193_attestor_refill_activation_height, 7,
        "O2/IP-T: testnet must honour it — the AH-crossing rehearsal needs to force it"
    );
    assert_eq!(
        devnet.inc_i_193_attestor_refill_activation_height, 7,
        "O2/IP-D: devnet must honour the override"
    );
    assert_eq!(
        mainnet.inc_i_193_attestor_refill_activation_height, REFILL_MAINNET,
        "O2/IP-M: mainnet must IGNORE the override — an env-settable consensus gate lets a \
         single operator fork mainnet"
    );
}
