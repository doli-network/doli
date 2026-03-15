//! Tests for network parameters

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
