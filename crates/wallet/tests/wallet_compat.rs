//! Integration tests: Wallet format compatibility between CLI and shared wallet crate.
//!
//! These tests verify that wallets created by the shared crate are identical in format
//! to wallets created by the CLI, ensuring cross-tool interoperability (GUI-NF-008).

use tempfile::TempDir;
use wallet::Wallet;

// ============================================================================
// Requirement: GUI-NF-008 (Must) -- Wallet file format compatibility with CLI
// Acceptance: Same JSON format as CLI wallet.json
// ============================================================================

#[test]
fn test_nf008_wallet_json_structure_matches_cli() {
    let (wallet, _) = Wallet::new("compat-test");
    let json = serde_json::to_string_pretty(&wallet).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    // CLI wallet.json has exactly these top-level fields: name, version, addresses
    let obj = parsed.as_object().unwrap();
    assert!(obj.contains_key("name"), "Missing 'name' field");
    assert!(obj.contains_key("version"), "Missing 'version' field");
    assert!(obj.contains_key("addresses"), "Missing 'addresses' field");

    // No extra fields that CLI wouldn't understand
    assert_eq!(
        obj.len(),
        3,
        "Wallet JSON should have exactly 3 top-level fields (name, version, addresses), got: {:?}",
        obj.keys().collect::<Vec<_>>()
    );
}

#[test]
fn test_nf008_address_json_fields_match_cli() {
    let (wallet, _) = Wallet::new("test");
    let json = serde_json::to_string_pretty(&wallet).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    let addr = &parsed["addresses"][0];
    let addr_obj = addr.as_object().unwrap();

    // Required fields in CLI's WalletAddress
    assert!(addr_obj.contains_key("address"), "Missing 'address'");
    assert!(addr_obj.contains_key("public_key"), "Missing 'public_key'");
    assert!(
        addr_obj.contains_key("private_key"),
        "Missing 'private_key'"
    );
    assert!(addr_obj.contains_key("label"), "Missing 'label'");

    // BLS fields present for v2 wallet with BLS
    assert!(
        addr_obj.contains_key("bls_private_key"),
        "Missing 'bls_private_key'"
    );
    assert!(
        addr_obj.contains_key("bls_public_key"),
        "Missing 'bls_public_key'"
    );
}

/// Simulate a CLI-created wallet JSON and verify the shared crate can load it.
#[test]
fn test_nf008_load_cli_wallet_json() {
    // This JSON matches the exact format produced by bins/cli/src/wallet.rs
    let cli_json = r#"{
        "name": "cli-wallet",
        "version": 2,
        "addresses": [
            {
                "address": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
                "public_key": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "private_key": "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210",
                "label": "primary",
                "bls_private_key": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
                "bls_public_key": "aabbccdd00112233445566778899aabb00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"
            }
        ]
    }"#;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cli-wallet.json");
    std::fs::write(&path, cli_json).unwrap();

    let wallet = Wallet::load(&path).unwrap();
    assert_eq!(wallet.name(), "cli-wallet");
    assert_eq!(wallet.version(), 2);
    assert_eq!(wallet.addresses().len(), 1);
    assert!(wallet.has_bls_key());
}

/// Simulate a legacy v1 CLI wallet (no BLS) and verify loading.
#[test]
fn test_nf008_load_legacy_v1_cli_wallet() {
    let v1_json = r#"{
        "name": "legacy-wallet",
        "version": 1,
        "addresses": [
            {
                "address": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
                "public_key": "0000000000000000000000000000000000000000000000000000000000000001",
                "private_key": "0000000000000000000000000000000000000000000000000000000000000002",
                "label": "primary"
            }
        ]
    }"#;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("v1.json");
    std::fs::write(&path, v1_json).unwrap();

    let wallet = Wallet::load(&path).unwrap();
    assert_eq!(wallet.version(), 1);
    assert!(!wallet.has_bls_key());
}

/// Verify roundtrip: create with shared crate -> save -> reload -> same data.
#[test]
fn test_nf008_save_load_roundtrip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("roundtrip.json");

    let (original, _) = Wallet::new("roundtrip-test");
    original.save(&path).unwrap();

    let loaded = Wallet::load(&path).unwrap();

    assert_eq!(original.name(), loaded.name());
    assert_eq!(original.version(), loaded.version());
    assert_eq!(original.addresses().len(), loaded.addresses().len());
    assert_eq!(original.primary_public_key(), loaded.primary_public_key());
    assert_eq!(original.has_bls_key(), loaded.has_bls_key());
    assert_eq!(
        original.primary_bls_public_key(),
        loaded.primary_bls_public_key()
    );
}

/// Verify that wallet created in GUI -> CLI seed restore produces same key.
#[test]
fn test_nf008_same_seed_produces_same_key() {
    let (gui_wallet, seed_phrase) = Wallet::new("gui-wallet");
    let cli_restored = Wallet::from_seed_phrase("cli-restored", &seed_phrase).unwrap();

    assert_eq!(
        gui_wallet.primary_public_key(),
        cli_restored.primary_public_key(),
        "GUI and CLI must derive identical Ed25519 keys from same seed"
    );
    assert_eq!(
        gui_wallet.primary_pubkey_hash().unwrap(),
        cli_restored.primary_pubkey_hash().unwrap(),
        "pubkey_hash must match for RPC queries"
    );
}

/// Verify export/import cycle preserves wallet identity.
#[test]
fn test_nf008_export_import_preserves_wallet() {
    let dir = TempDir::new().unwrap();
    let original_path = dir.path().join("original.json");
    let export_path = dir.path().join("exported.json");

    let (wallet, _) = Wallet::new("test");
    wallet.save(&original_path).unwrap();
    wallet.export(&export_path).unwrap();

    let imported = Wallet::import(&export_path).unwrap();
    assert_eq!(wallet.primary_public_key(), imported.primary_public_key());
}

/// Verify wallet with multiple addresses roundtrips correctly.
#[test]
fn test_nf008_multi_address_roundtrip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("multi.json");

    let (mut wallet, _) = Wallet::new("multi-addr");
    wallet.generate_address(Some("savings")).unwrap();
    wallet.generate_address(Some("spending")).unwrap();
    wallet.generate_address(None).unwrap();
    wallet.save(&path).unwrap();

    let loaded = Wallet::load(&path).unwrap();
    assert_eq!(loaded.addresses().len(), 4);
    assert_eq!(loaded.addresses()[0].label.as_deref(), Some("primary"));
    assert_eq!(loaded.addresses()[1].label.as_deref(), Some("savings"));
    assert_eq!(loaded.addresses()[2].label.as_deref(), Some("spending"));
    assert_eq!(loaded.addresses()[3].label, None);
}
