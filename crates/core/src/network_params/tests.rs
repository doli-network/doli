//! Tests for network parameters
//!
//! OUTPUT CONTRACT: this module covers `NetworkParams` defaults, env overrides,
//! chainspec loading, and (INC-I-096) the AMM-conservation activation-ordering
//! guard. The per-function contract + INPUT PARTITIONS for the INC-I-096 guard
//! are documented inline above its tests below.

use std::sync::Mutex;

use crate::Network;

use super::chainspec_loader::apply_chainspec_defaults;
use super::env_loader::{env_parse, env_parse_vec, get_default_data_dir, load_env_for_network};
use super::NetworkParams;

/// Global mutex to serialize tests that modify process environment variables.
/// Env vars are process-global, so parallel tests can interfere with each other.
static ENV_MUTEX: Mutex<()> = Mutex::new(());

#[test]
fn test_defaults_match_network_rs() {
    // Verify that defaults match the original hardcoded values
    let mainnet = NetworkParams::defaults(Network::Mainnet);
    assert_eq!(mainnet.default_p2p_port, 30300);
    assert_eq!(mainnet.default_rpc_port, 8500);
    assert_eq!(mainnet.slot_duration, 10);
    assert_eq!(mainnet.bond_unit, 1_000_000_000); // 10 DOLI
    assert_eq!(mainnet.blocks_per_year, 3_153_600);

    let devnet = NetworkParams::defaults(Network::Devnet);
    assert_eq!(devnet.default_p2p_port, 50300);
    assert_eq!(devnet.default_rpc_port, 28500);
    assert_eq!(devnet.bond_unit, 100_000_000);
    assert_eq!(devnet.blocks_per_year, 144);
}

#[test]
fn test_derived_parameters() {
    let mainnet = NetworkParams::defaults(Network::Mainnet);
    assert_eq!(mainnet.blocks_per_month(), mainnet.blocks_per_year / 12);
    assert_eq!(mainnet.blocks_per_era(), mainnet.blocks_per_year * 4);
    assert_eq!(mainnet.commitment_period(), mainnet.blocks_per_era());
    assert_eq!(
        mainnet.exit_history_retention(),
        mainnet.blocks_per_era() * 2
    );
}

#[test]
fn test_env_override() {
    let _lock = ENV_MUTEX.lock().unwrap();

    // Save original value to restore later
    let original_val = std::env::var("DOLI_SLOT_DURATION");

    // Set test value (override default of 10s/1s)
    std::env::set_var("DOLI_SLOT_DURATION", "42");

    // Load params for Devnet (which allows env overrides)
    let params = super::env_loader::load_from_env(Network::Devnet);

    // Restore environment
    if let Ok(val) = original_val {
        std::env::set_var("DOLI_SLOT_DURATION", val);
    } else {
        std::env::remove_var("DOLI_SLOT_DURATION");
    }

    // Verify override took effect
    assert_eq!(params.slot_duration, 42);

    // Verify Mainnet IGNORES the override (locked params)
    let mainnet_params = super::env_loader::load_from_env(Network::Mainnet);
    assert_eq!(mainnet_params.slot_duration, 10); // Should remain 10 despite env var
}

#[test]
fn test_env_parse() {
    // Test with non-existent env var (should use default)
    let result: u16 = env_parse("NONEXISTENT_VAR_12345", 42);
    assert_eq!(result, 42);
}

#[test]
fn test_env_parse_vec() {
    // Test with non-existent env var (should use default)
    let default = vec!["a".to_string(), "b".to_string()];
    let result = env_parse_vec("NONEXISTENT_VAR_12345", default.clone());
    assert_eq!(result, default);
}

#[test]
fn test_load_env_for_network_no_file() {
    // Should not panic when .env file doesn't exist
    let temp_dir = tempfile::TempDir::new().unwrap();
    load_env_for_network("testnet", temp_dir.path());
}

#[test]
fn test_load_env_for_network_with_file() {
    let _lock = ENV_MUTEX.lock().unwrap();
    let temp_dir = tempfile::TempDir::new().unwrap();
    let env_path = temp_dir.path().join(".env");

    // Write a test .env file
    std::fs::write(&env_path, "DOLI_TEST_VAR_NETWORK_PARAMS=test_value\n").unwrap();

    // Clear any existing value
    std::env::remove_var("DOLI_TEST_VAR_NETWORK_PARAMS");

    // Load the env file
    load_env_for_network("testnet", temp_dir.path());

    // Verify the value was loaded
    assert_eq!(
        std::env::var("DOLI_TEST_VAR_NETWORK_PARAMS").ok(),
        Some("test_value".to_string())
    );

    // Clean up
    std::env::remove_var("DOLI_TEST_VAR_NETWORK_PARAMS");
}

#[test]
fn test_get_default_data_dir() {
    let data_dir = get_default_data_dir("mainnet");
    assert!(data_dir.ends_with(".doli/mainnet"));
}

#[test]
fn test_load_env_fallback_to_network_root() {
    let _lock = ENV_MUTEX.lock().unwrap();
    // Create a "network root" dir with .env, and a "subdir" without .env
    let root_dir = tempfile::TempDir::new().unwrap();
    let sub_dir = root_dir.path().join("data").join("node5");
    std::fs::create_dir_all(&sub_dir).unwrap();

    // Write .env only in root
    let env_path = root_dir.path().join(".env");
    std::fs::write(&env_path, "DOLI_TEST_FALLBACK_VAR=from_root\n").unwrap();
    std::env::remove_var("DOLI_TEST_FALLBACK_VAR");

    // The sub_dir has no .env, so load_env_for_network won't find it there.
    // The fallback uses get_default_data_dir which goes to ~/.doli/{network},
    // so we can't fully test the fallback path here without mocking HOME.
    // Instead, verify the function doesn't panic on subdirs without .env.
    load_env_for_network("devnet", &sub_dir);

    // Clean up
    std::env::remove_var("DOLI_TEST_FALLBACK_VAR");
}

#[test]
fn test_apply_chainspec_defaults_sets_vars() {
    let _lock = ENV_MUTEX.lock().unwrap();
    let temp_dir = tempfile::TempDir::new().unwrap();
    let chainspec_path = temp_dir.path().join("chainspec.json");

    // Write a minimal devnet chainspec
    let chainspec_json = r#"{
        "name": "Test Devnet",
        "id": "devnet",
        "network": "Devnet",
        "genesis": {
            "timestamp": 1700000000,
            "message": "test",
            "initial_reward": 5000000000
        },
        "consensus": {
            "slot_duration": 7,
            "slots_per_epoch": 42,
            "bond_amount": 200000000
        },
        "genesis_producers": []
    }"#;
    std::fs::write(&chainspec_path, chainspec_json).unwrap();

    // Clear all related vars
    std::env::remove_var("DOLI_SLOT_DURATION");
    std::env::remove_var("DOLI_BOND_UNIT");
    std::env::remove_var("DOLI_SLOTS_PER_REWARD_EPOCH");
    std::env::remove_var("DOLI_INITIAL_REWARD");
    std::env::remove_var("DOLI_GENESIS_TIME");

    apply_chainspec_defaults(&chainspec_path);

    assert_eq!(std::env::var("DOLI_SLOT_DURATION").unwrap(), "7");
    assert_eq!(std::env::var("DOLI_BOND_UNIT").unwrap(), "200000000");
    assert_eq!(std::env::var("DOLI_SLOTS_PER_REWARD_EPOCH").unwrap(), "42");
    assert_eq!(std::env::var("DOLI_INITIAL_REWARD").unwrap(), "5000000000");
    assert_eq!(std::env::var("DOLI_GENESIS_TIME").unwrap(), "1700000000");

    // Clean up
    std::env::remove_var("DOLI_SLOT_DURATION");
    std::env::remove_var("DOLI_BOND_UNIT");
    std::env::remove_var("DOLI_SLOTS_PER_REWARD_EPOCH");
    std::env::remove_var("DOLI_INITIAL_REWARD");
    std::env::remove_var("DOLI_GENESIS_TIME");
}

#[test]
fn test_apply_chainspec_defaults_no_override() {
    let _lock = ENV_MUTEX.lock().unwrap();
    let temp_dir = tempfile::TempDir::new().unwrap();
    let chainspec_path = temp_dir.path().join("chainspec.json");

    let chainspec_json = r#"{
        "name": "Test Devnet",
        "id": "devnet",
        "network": "Devnet",
        "genesis": {
            "timestamp": 0,
            "message": "test",
            "initial_reward": 5000000000
        },
        "consensus": {
            "slot_duration": 7,
            "slots_per_epoch": 42,
            "bond_amount": 200000000
        },
        "genesis_producers": []
    }"#;
    std::fs::write(&chainspec_path, chainspec_json).unwrap();

    // Pre-set a var — chainspec should NOT override it
    std::env::set_var("DOLI_SLOT_DURATION", "99");

    apply_chainspec_defaults(&chainspec_path);

    // Should remain 99, not 7 from chainspec
    assert_eq!(std::env::var("DOLI_SLOT_DURATION").unwrap(), "99");

    // Clean up
    std::env::remove_var("DOLI_SLOT_DURATION");
    std::env::remove_var("DOLI_BOND_UNIT");
    std::env::remove_var("DOLI_SLOTS_PER_REWARD_EPOCH");
    std::env::remove_var("DOLI_INITIAL_REWARD");
}

#[test]
fn test_apply_chainspec_defaults_mainnet_skipped() {
    let _lock = ENV_MUTEX.lock().unwrap();
    let temp_dir = tempfile::TempDir::new().unwrap();
    let chainspec_path = temp_dir.path().join("chainspec.json");

    let chainspec_json = r#"{
        "name": "Test Mainnet",
        "id": "mainnet",
        "network": "Mainnet",
        "genesis": {
            "timestamp": 1700000000,
            "message": "test",
            "initial_reward": 999
        },
        "consensus": {
            "slot_duration": 999,
            "slots_per_epoch": 999,
            "bond_amount": 999
        },
        "genesis_producers": []
    }"#;
    std::fs::write(&chainspec_path, chainspec_json).unwrap();

    // Clear vars
    std::env::remove_var("DOLI_SLOT_DURATION_MAINNET_TEST");

    apply_chainspec_defaults(&chainspec_path);

    // Mainnet chainspec should be skipped entirely — vars should NOT be set
    assert!(
        std::env::var("DOLI_SLOT_DURATION").is_err()
            || std::env::var("DOLI_SLOT_DURATION").unwrap() != "999"
    );
}

#[test]
fn test_apply_chainspec_defaults_malformed_file() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let chainspec_path = temp_dir.path().join("chainspec.json");

    // Write invalid JSON
    std::fs::write(&chainspec_path, "{ not valid json }").unwrap();

    // Should not panic, just log a warning
    apply_chainspec_defaults(&chainspec_path);
}

// === INC-I-096 INV-DEPLOY-002: AMM-conservation activation ordering ===
//
// OUTPUT CONTRACT: fn NetworkParams::validate_amm_conservation_ordering(&self, network) -> Result<(), String>
//   Outputs:
//     O1: Ok(())   — ordering safe (or AMM disabled, or testnet grandfather)
//     O2: Err(msg) — AMM enabled before conservation on a non-testnet network
//   PATHS:
//     P1: amm_activation_height == u64::MAX (AMM disabled)        -> O1
//     P2: network == Testnet (grandfathered)                      -> O1
//     P3: inc_i_096_activation_height >  amm_activation_height     -> O2
//     P4: inc_i_096_activation_height <= amm_activation_height     -> O1
//   INPUT PARTITIONS:
//     IP1: mainnet defaults (amm=MAX,inc=MAX)        -> P1 -> O1
//     IP2: devnet  defaults (amm=0,inc=0)            -> P4 -> O1
//     IP3: testnet defaults (amm=20_099,inc=MAX)     -> P2 -> O1 (grandfather; precondition: violated ordering)
//     IP4: synthetic mainnet (amm=1000,inc=2000)     -> P3 -> O2
//     IP5: synthetic mainnet (amm=2000,inc=1000)     -> P4 -> O1
//   MATRIX:
//     T1: O1 x P1 x IP1 -> inv_deploy_002_mainnet_defaults_ok
//     T2: O1 x P4 x IP2 -> inv_deploy_002_devnet_defaults_ok
//     T3: O1 x P2 x IP3 -> inv_deploy_002_testnet_grandfathered_ok
//     T4: O2 x P3 x IP4 -> inv_deploy_002_rejects_amm_before_conservation_on_mainnet
//     T5: O1 x P4 x IP5 -> inv_deploy_002_accepts_conservation_before_amm_on_mainnet

#[test]
fn inv_deploy_002_mainnet_defaults_ok() {
    // mainnet: amm=u64::MAX, inc_i_096=u64::MAX → AMM disabled → guard passes.
    let p = NetworkParams::defaults(Network::Mainnet);
    assert!(p
        .validate_amm_conservation_ordering(Network::Mainnet)
        .is_ok());
}

#[test]
fn inv_deploy_002_devnet_defaults_ok() {
    // devnet: amm=0, inc_i_096=0 → 0 <= 0 → guard passes.
    let p = NetworkParams::defaults(Network::Devnet);
    assert!(p
        .validate_amm_conservation_ordering(Network::Devnet)
        .is_ok());
}

#[test]
fn inv_deploy_002_testnet_grandfathered_ok() {
    // testnet: amm=20_099 enabled, inc_i_096=u64::MAX (not yet pinned).
    // Ordering is historically violated (AMM predates the fix) but explicitly
    // grandfathered — below-gate conservation rejects (never drains) AMM
    // DOLI-outflow txs, so this is safe. Guard must return Ok for Testnet.
    // Testnet fresh-genesis redeploy (2026-07-07): amm and inc_i_096 both
    // activate at genesis (0), so the historical ordering violation no longer
    // exists — conservation engages the same block AMM does. Guard passes.
    let p = NetworkParams::defaults(Network::Testnet);
    assert_eq!(p.amm_activation_height, 0);
    assert_eq!(p.inc_i_096_activation_height, 0);
    assert!(p
        .validate_amm_conservation_ordering(Network::Testnet)
        .is_ok());
}

#[test]
fn inv_deploy_002_rejects_amm_before_conservation_on_mainnet() {
    // Synthetic dangerous config: AMM enabled at 1000 but conservation at 2000.
    // On a non-testnet network this MUST be rejected (would run AMM on the
    // pre-INC-I-096 drainable conservation between h=1000 and h=2000).
    let mut p = NetworkParams::defaults(Network::Mainnet);
    p.amm_activation_height = 1000;
    p.inc_i_096_activation_height = 2000;
    assert!(p
        .validate_amm_conservation_ordering(Network::Mainnet)
        .is_err());
}

#[test]
fn inv_deploy_002_accepts_conservation_before_amm_on_mainnet() {
    // Conservation activates at/before AMM → safe → Ok.
    let mut p = NetworkParams::defaults(Network::Mainnet);
    p.amm_activation_height = 2000;
    p.inc_i_096_activation_height = 1000;
    assert!(p
        .validate_amm_conservation_ordering(Network::Mainnet)
        .is_ok());
}

// ============================================================================
// INC-I-172 M1 security audit, AUDIT-P3-012 — the update-timing params must be
// LOCKED on mainnet, like every one of their neighbours.
//
// THE DEFECT this locks shut: `veto_period_secs` and `grace_period_secs` were the only
// update-timing parameters not wrapped in the `is_mainnet` lock. `.env` is read from the
// DATA DIRECTORY, so anything that can write there — the same reach as
// `maintainer_state.bin` — could set `DOLI_VETO_PERIOD_SECS=0` on mainnet and collapse
// the veto window to nothing, making the next approved release install on the following
// 60 s tick with no opportunity for producers to object.
//
// OUTPUT CONTRACT: `env_loader::load_from_env(network)`
//   O1: return.veto_period_secs
//   O2: return.grace_period_secs
//   O3: process env DOLI_VETO_PERIOD_SECS / DOLI_GRACE_PERIOD_SECS — restored
// PATHS:
//   P1: is_mainnet == true   -> env IGNORED, defaults returned
//   P2: is_mainnet == false  -> env HONOURED (devnet must stay fast for testing)
// INPUT PARTITIONS: the attack value `0` (the veto window an attacker wants) on P1, and
//   a distinctive non-default on P2. P2 is the control: without it, a `load_from_env`
//   that returned defaults unconditionally would satisfy P1 and prove nothing.
// ============================================================================

/// AUDIT-P3-012. RED before the fix.
/// [P1 -> O1, O2, O3]
#[test]
fn audit_p3_012_mainnet_ignores_veto_and_grace_env_overrides() {
    let _lock = ENV_MUTEX.lock().unwrap();
    let original_veto = std::env::var("DOLI_VETO_PERIOD_SECS");
    let original_grace = std::env::var("DOLI_GRACE_PERIOD_SECS");

    // The attack value: a zero-length veto window.
    std::env::set_var("DOLI_VETO_PERIOD_SECS", "0");
    std::env::set_var("DOLI_GRACE_PERIOD_SECS", "0");

    let observed = super::env_loader::load_from_env(Network::Mainnet);
    let defaults = NetworkParams::defaults(Network::Mainnet);

    // O3: restore BEFORE asserting so a failure does not leak process state.
    match original_veto {
        Ok(v) => std::env::set_var("DOLI_VETO_PERIOD_SECS", v),
        Err(_) => std::env::remove_var("DOLI_VETO_PERIOD_SECS"),
    }
    match original_grace {
        Ok(v) => std::env::set_var("DOLI_GRACE_PERIOD_SECS", v),
        Err(_) => std::env::remove_var("DOLI_GRACE_PERIOD_SECS"),
    }

    assert_eq!(
        observed.veto_period_secs, defaults.veto_period_secs,
        "mainnet MUST ignore DOLI_VETO_PERIOD_SECS. A data-dir .env setting it to 0 \
         collapses the mainnet veto window and auto-installs the next approved release \
         on the following tick (AUDIT-P3-012)"
    );
    assert_eq!(
        observed.grace_period_secs, defaults.grace_period_secs,
        "mainnet MUST ignore DOLI_GRACE_PERIOD_SECS, for the same reason"
    );
    assert!(
        defaults.veto_period_secs > 0,
        "the mainnet default veto period must be non-zero, or the lock protects nothing"
    );
}

/// AUDIT-P3-012. GREEN-lock (control).
/// Acceptance: non-mainnet networks still honour the override — devnet's accelerated
/// timing is what makes the update path testable at all.
/// [P2 -> O1, O2, O3]
#[test]
fn audit_p3_012_non_mainnet_still_honours_veto_and_grace_env_overrides() {
    let _lock = ENV_MUTEX.lock().unwrap();
    let original_veto = std::env::var("DOLI_VETO_PERIOD_SECS");
    let original_grace = std::env::var("DOLI_GRACE_PERIOD_SECS");

    std::env::set_var("DOLI_VETO_PERIOD_SECS", "77");
    std::env::set_var("DOLI_GRACE_PERIOD_SECS", "88");

    let observed = super::env_loader::load_from_env(Network::Devnet);

    match original_veto {
        Ok(v) => std::env::set_var("DOLI_VETO_PERIOD_SECS", v),
        Err(_) => std::env::remove_var("DOLI_VETO_PERIOD_SECS"),
    }
    match original_grace {
        Ok(v) => std::env::set_var("DOLI_GRACE_PERIOD_SECS", v),
        Err(_) => std::env::remove_var("DOLI_GRACE_PERIOD_SECS"),
    }

    assert_eq!(observed.veto_period_secs, 77);
    assert_eq!(observed.grace_period_secs, 88);
}

// ===========================================================================
// INC-I-190 F3 / REQ-AUTH-012.1 + .2 — the floor-bound activation height
//
// OUTPUT CONTRACT
//   O1 NetworkParams::defaults(net).inc_i_190_floor_bound_activation_height
//   O2 load_from_env(net).inc_i_190_floor_bound_activation_height
//   O3 staleness/no-op properties of the testnet pin
// PATHS   mainnet arm (locked to defaults) | non-mainnet arm (env_parse)
// INPUT PARTITIONS  IP-M mainnet | IP-T testnet | IP-D devnet; env unset | env set
// ===========================================================================

/// Mainnet stays frozen: pinning a real height is a separate decision-session.
const FLOOR_BOUND_MAINNET: u64 = u64::MAX;
/// Re-pinned 2026-08-28 (AUDIT-P1-502) after the 52_000 pin was overtaken.
const FLOOR_BOUND_TESTNET: u64 = 58_000;
/// Devnet is always above the gate, matching every other INC gate's devnet arm.
const FLOOR_BOUND_DEVNET: u64 = 0;
/// Testnet tip measured by read-only `getChainInfo` on 2026-08-28 23:38 WEST, when
/// the gate was re-pinned. Frozen on purpose: it records WHAT WAS MEASURED, so the
/// assertion below is a floor the pin can never sink under, not a live chain probe.
const FLOOR_BOUND_TESTNET_TIP_AT_PIN: u64 = 51_861;

/// REQ-AUTH-012.1 — O1 x {IP-M, IP-T, IP-D}.
#[test]
fn req_auth_012_1_floor_bound_gate_pinned_per_network() {
    assert_eq!(
        NetworkParams::defaults(Network::Mainnet).inc_i_190_floor_bound_activation_height,
        FLOOR_BOUND_MAINNET,
        "O1/IP-M: mainnet must ship fail-closed — a pinned mainnet height becomes \
         IMMUTABLE the moment the chain crosses it (INC-I-054)"
    );
    assert_eq!(
        NetworkParams::defaults(Network::Testnet).inc_i_190_floor_bound_activation_height,
        FLOOR_BOUND_TESTNET,
        "O1/IP-T: testnet is pinned at 58_000"
    );
    assert_eq!(
        NetworkParams::defaults(Network::Devnet).inc_i_190_floor_bound_activation_height,
        FLOOR_BOUND_DEVNET,
        "O1/IP-D: devnet activates from genesis"
    );
}

/// REQ-AUTH-012.1 — O3 x IP-T: the pin is neither a no-op nor retroactive, and is
/// its own field rather than a bundle onto a neighbouring gate (INV-PARAMS-001).
///
/// SCOPE (REV-I190-M4-F11 / SEC-FIXVERIFY-003): this is a FLOOR, not a liveness
/// check. A unit test cannot read the live chain, so it can only prove the pin was
/// above the tip AT PIN TIME. It stays green after the chain overtakes the gate.
/// The live control is the pre-deploy tip re-measurement in the M4 deploy checklist.
#[test]
fn req_auth_012_1_floor_bound_testnet_gate_is_above_the_tip_it_was_measured_against() {
    let t = NetworkParams::defaults(Network::Testnet);
    let h = t.inc_i_190_floor_bound_activation_height;

    assert!(
        h > FLOOR_BOUND_TESTNET_TIP_AT_PIN,
        "O3: the gate ({}) must be strictly above the tip it was measured against ({}) — \
         a gate crossed before the binary is deployed is a RETROACTIVE consensus rule \
         change and forks a rolling deploy (AUDIT-P1-502 / INC-I-054). NOTE: both sides \
         are compile-time constants, so a GREEN result does NOT mean the live tip is \
         still below the gate — re-measure with getChainInfo immediately before deploy",
        h,
        FLOOR_BOUND_TESTNET_TIP_AT_PIN
    );
    assert_ne!(h, 0, "O3: a testnet gate of 0 reinterprets sealed history");
    assert_ne!(
        h,
        u64::MAX,
        "O3: u64::MAX makes the fix unreachable on testnet"
    );
    assert_ne!(
        h, t.epoch_prune_activation_height,
        "O3: the floor-bound gate must not be bundled onto epoch_prune"
    );
    assert_ne!(
        h, t.ghost_exclusion_activation_height,
        "O3: the floor-bound gate must not be bundled onto ghost_exclusion"
    );
    assert_ne!(
        h, t.maintainer_derivation_activation_height,
        "O3: the floor-bound gate must not be bundled onto maintainer_derivation"
    );
}

/// REQ-AUTH-012.2 — O2 x {IP-M, IP-T, IP-D}: non-mainnet honours the env override
/// (the AH-crossing test forces the crossing locally); mainnet ignores it.
#[test]
fn req_auth_012_2_floor_bound_env_override_off_mainnet_only() {
    let _lock = ENV_MUTEX.lock().unwrap();
    let original = std::env::var("DOLI_INC_I_190_FLOOR_BOUND_ACTIVATION_HEIGHT");

    std::env::set_var("DOLI_INC_I_190_FLOOR_BOUND_ACTIVATION_HEIGHT", "7");
    let testnet = super::env_loader::load_from_env(Network::Testnet);
    let devnet = super::env_loader::load_from_env(Network::Devnet);
    let mainnet = super::env_loader::load_from_env(Network::Mainnet);

    match original {
        Ok(v) => std::env::set_var("DOLI_INC_I_190_FLOOR_BOUND_ACTIVATION_HEIGHT", v),
        Err(_) => std::env::remove_var("DOLI_INC_I_190_FLOOR_BOUND_ACTIVATION_HEIGHT"),
    }

    assert_eq!(
        testnet.inc_i_190_floor_bound_activation_height, 7,
        "O2/IP-T: testnet must honour the override — the AH-crossing validation \
         forces the crossing on the local nodes"
    );
    assert_eq!(
        devnet.inc_i_190_floor_bound_activation_height, 7,
        "O2/IP-D: devnet must honour the override"
    );
    assert_eq!(
        mainnet.inc_i_190_floor_bound_activation_height, FLOOR_BOUND_MAINNET,
        "O2/IP-M: mainnet must IGNORE the override — an env-settable consensus gate \
         lets a single operator fork mainnet"
    );
}
