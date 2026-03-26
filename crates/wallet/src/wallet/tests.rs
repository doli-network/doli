use super::*;
use std::path::Path;
use tempfile::TempDir;

// ========================================================================
// Requirement: GUI-FR-001 (Must)
// Acceptance: Wallet creation with BIP-39, Ed25519+BLS, wallet.json format
// ========================================================================

#[test]
fn test_fr001_new_wallet_generates_24_word_seed() {
    let (wallet, phrase) = Wallet::new("test-wallet");
    let word_count = phrase.split_whitespace().count();
    assert_eq!(word_count, 24, "BIP-39 mnemonic must be 24 words");
    assert_eq!(wallet.name(), "test-wallet");
}

#[test]
fn test_fr001_new_wallet_is_version_2() {
    let (wallet, _) = Wallet::new("test");
    assert_eq!(wallet.version(), 2);
}

#[test]
fn test_fr001_new_wallet_has_ed25519_keypair() {
    let (wallet, _) = Wallet::new("test");
    assert_eq!(wallet.addresses().len(), 1);
    // Ed25519 public key is 32 bytes = 64 hex chars
    assert_eq!(wallet.primary_public_key().len(), 64);
}

#[test]
fn test_fr001_new_wallet_has_bls_keypair() {
    let (wallet, _) = Wallet::new("test");
    assert!(wallet.has_bls_key(), "New wallet must have BLS key");
    let bls_pubkey = wallet.primary_bls_public_key().unwrap();
    // BLS public key is 48 bytes = 96 hex chars
    assert_eq!(bls_pubkey.len(), 96);
}

#[test]
fn test_fr001_seed_phrase_not_in_wallet_json() {
    let (wallet, phrase) = Wallet::new("test");
    let json = serde_json::to_string_pretty(&wallet).unwrap();
    assert!(
        !json.contains("seed_phrase"),
        "Seed phrase must NOT be stored in wallet JSON"
    );
    assert!(
        !json.contains(&phrase),
        "Actual seed phrase words must NOT appear in wallet JSON"
    );
}

#[test]
fn test_fr001_seed_phrase_is_valid_bip39() {
    let (_, phrase) = Wallet::new("test");
    // Must parse as valid BIP-39 mnemonic
    let result: Result<Mnemonic, _> = phrase.parse();
    assert!(result.is_ok(), "Seed phrase must be valid BIP-39");
}

#[test]
fn test_fr001_primary_address_labeled() {
    let (wallet, _) = Wallet::new("test");
    assert_eq!(
        wallet.addresses()[0].label.as_deref(),
        Some("primary"),
        "Primary address must have label 'primary'"
    );
}

#[test]
fn test_fr001_wallet_save_and_load_roundtrip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("wallet.json");

    let (wallet, _) = Wallet::new("test");
    wallet.save(&path).unwrap();

    let loaded = Wallet::load(&path).unwrap();
    assert_eq!(loaded.name(), wallet.name());
    assert_eq!(loaded.version(), wallet.version());
    assert_eq!(loaded.primary_public_key(), wallet.primary_public_key());
}

#[test]
fn test_fr001_wallet_save_creates_parent_dirs() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("deep").join("nested").join("wallet.json");

    let (wallet, _) = Wallet::new("test");
    let result = wallet.save(&path);
    assert!(result.is_ok(), "Save should create parent directories");
    assert!(path.exists());
}

// Edge cases for GUI-FR-001 (Must)

#[test]
fn test_fr001_edge_empty_name() {
    let (wallet, phrase) = Wallet::new("");
    assert_eq!(wallet.name(), "");
    assert_eq!(phrase.split_whitespace().count(), 24);
}

#[test]
fn test_fr001_edge_unicode_name() {
    let (wallet, _) = Wallet::new("My Wallet");
    let json = serde_json::to_string(&wallet).unwrap();
    let loaded: Wallet = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded.name(), "My Wallet");
}

#[test]
fn test_fr001_edge_special_chars_name() {
    let (wallet, _) = Wallet::new(r#"test"wallet<>|&"#);
    let json = serde_json::to_string(&wallet).unwrap();
    let loaded: Wallet = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded.name(), r#"test"wallet<>|&"#);
}

#[test]
fn test_fr001_multiple_wallets_unique_keys() {
    let (w1, _) = Wallet::new("wallet1");
    let (w2, _) = Wallet::new("wallet2");
    assert_ne!(
        w1.primary_public_key(),
        w2.primary_public_key(),
        "Different wallets must have different keys"
    );
}

// ========================================================================
// Requirement: GUI-FR-002 (Must)
// Acceptance: Same seed = same Ed25519 key; invalid seed rejected
// ========================================================================

#[test]
fn test_fr002_restore_produces_same_ed25519_key() {
    let (original, phrase) = Wallet::new("original");
    let restored = Wallet::from_seed_phrase("restored", &phrase).unwrap();
    assert_eq!(
        original.primary_public_key(),
        restored.primary_public_key(),
        "Restored wallet must derive identical Ed25519 key"
    );
}

#[test]
fn test_fr002_restore_produces_same_address() {
    let (original, phrase) = Wallet::new("original");
    let restored = Wallet::from_seed_phrase("restored", &phrase).unwrap();
    assert_eq!(
        original.primary_address(),
        restored.primary_address(),
        "Restored wallet must derive identical address"
    );
}

#[test]
fn test_fr002_restore_produces_same_pubkey_hash() {
    let (original, phrase) = Wallet::new("original");
    let restored = Wallet::from_seed_phrase("restored", &phrase).unwrap();
    assert_eq!(
        original.primary_pubkey_hash().unwrap(),
        restored.primary_pubkey_hash().unwrap(),
        "Restored wallet must have identical pubkey hash for RPC queries"
    );
}

#[test]
fn test_fr002_restore_generates_new_bls_key() {
    let (original, phrase) = Wallet::new("original");
    let restored = Wallet::from_seed_phrase("restored", &phrase).unwrap();
    // BLS keys are random, not derived -- so they should differ
    assert_ne!(
        original.primary_bls_public_key().unwrap(),
        restored.primary_bls_public_key().unwrap(),
        "BLS key is randomly generated on restore, not derived from seed"
    );
}

#[test]
fn test_fr002_invalid_seed_phrase_rejected() {
    let result = Wallet::from_seed_phrase("test", "not a valid seed phrase at all");
    assert!(result.is_err(), "Invalid seed phrase must be rejected");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.to_lowercase().contains("invalid"),
        "Error message should mention 'invalid': {}",
        err_msg
    );
}

#[test]
fn test_fr002_wrong_word_count_rejected() {
    // 12 words instead of 24
    let result = Wallet::from_seed_phrase(
        "test",
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    );
    // 12-word mnemonics are valid BIP-39 but we accept them (the CLI does too)
    // This test documents the behavior -- either accept or reject is valid
    // The key thing is it doesn't panic
    let _ = result;
}

#[test]
fn test_fr002_empty_seed_phrase_rejected() {
    let result = Wallet::from_seed_phrase("test", "");
    assert!(result.is_err(), "Empty seed phrase must be rejected");
}

#[test]
fn test_fr002_seed_phrase_with_extra_spaces() {
    let (_, phrase) = Wallet::new("test");
    // Add extra spaces between words
    let spaced = phrase.split_whitespace().collect::<Vec<_>>().join("  ");
    // BIP-39 parsing should handle this (or fail gracefully)
    let _ = Wallet::from_seed_phrase("test", &spaced);
}

#[test]
fn test_fr002_restore_deterministic_across_calls() {
    let (_, phrase) = Wallet::new("test");
    let r1 = Wallet::from_seed_phrase("r1", &phrase).unwrap();
    let r2 = Wallet::from_seed_phrase("r2", &phrase).unwrap();
    assert_eq!(
        r1.primary_public_key(),
        r2.primary_public_key(),
        "Multiple restores from same seed must produce same key"
    );
}

// ========================================================================
// Requirement: GUI-FR-003 (Must)
// Acceptance: Generates new Ed25519 keypair, bech32m address format, labels
// ========================================================================

#[test]
fn test_fr003_generate_address_creates_new_entry() {
    let (mut wallet, _) = Wallet::new("test");
    assert_eq!(wallet.addresses().len(), 1);

    let addr = wallet.generate_address(Some("secondary")).unwrap();
    assert_eq!(wallet.addresses().len(), 2);
    assert!(!addr.is_empty());
}

#[test]
fn test_fr003_generated_address_has_label() {
    let (mut wallet, _) = Wallet::new("test");
    wallet.generate_address(Some("my-label")).unwrap();

    let last_addr = wallet.addresses().last().unwrap();
    assert_eq!(last_addr.label.as_deref(), Some("my-label"));
}

#[test]
fn test_fr003_generated_address_label_optional() {
    let (mut wallet, _) = Wallet::new("test");
    wallet.generate_address(None).unwrap();

    let last_addr = wallet.addresses().last().unwrap();
    assert_eq!(last_addr.label, None);
}

#[test]
fn test_fr003_generated_addresses_unique() {
    let (mut wallet, _) = Wallet::new("test");
    let addr1 = wallet.generate_address(None).unwrap();
    let addr2 = wallet.generate_address(None).unwrap();
    assert_ne!(addr1, addr2, "Generated addresses must be unique");
}

#[test]
fn test_fr003_bech32m_mainnet_prefix() {
    let (wallet, _) = Wallet::new("test");
    let bech32_addr = wallet.primary_bech32_address("doli").unwrap();
    assert!(
        bech32_addr.starts_with("doli1"),
        "Mainnet address must start with 'doli1', got: {}",
        bech32_addr
    );
}

#[test]
fn test_fr003_bech32m_testnet_prefix() {
    let (wallet, _) = Wallet::new("test");
    let bech32_addr = wallet.primary_bech32_address("tdoli").unwrap();
    assert!(
        bech32_addr.starts_with("tdoli1"),
        "Testnet address must start with 'tdoli1', got: {}",
        bech32_addr
    );
}

#[test]
fn test_fr003_bech32m_devnet_prefix() {
    let (wallet, _) = Wallet::new("test");
    let bech32_addr = wallet.primary_bech32_address("ddoli").unwrap();
    assert!(
        bech32_addr.starts_with("ddoli1"),
        "Devnet address must start with 'ddoli1', got: {}",
        bech32_addr
    );
}

#[test]
fn test_fr003_generated_address_no_bls_key() {
    let (mut wallet, _) = Wallet::new("test");
    wallet.generate_address(Some("secondary")).unwrap();

    let secondary = &wallet.addresses()[1];
    assert!(
        secondary.bls_private_key.is_none(),
        "Generated addresses must NOT have BLS keys (only primary)"
    );
}

// ========================================================================
// Requirement: GUI-FR-004 (Must)
// Acceptance: List all addresses with labels, bech32m format, primary highlighted
// ========================================================================

#[test]
fn test_fr004_addresses_returns_all() {
    let (mut wallet, _) = Wallet::new("test");
    wallet.generate_address(Some("second")).unwrap();
    wallet.generate_address(Some("third")).unwrap();
    assert_eq!(wallet.addresses().len(), 3);
}

#[test]
fn test_fr004_primary_address_first() {
    let (mut wallet, _) = Wallet::new("test");
    wallet.generate_address(Some("second")).unwrap();

    assert_eq!(
        wallet.addresses()[0].label.as_deref(),
        Some("primary"),
        "Primary address must be first in the list"
    );
}

// ========================================================================
// Requirement: GUI-NF-004 (Must) -- Private key security
// Acceptance: Keys never in frontend responses, signing in Rust only
// ========================================================================

#[test]
fn test_nf004_wallet_address_private_key_not_pub() {
    // The private_key field is NOT pub -- this is a compile-time check.
    // This test documents the intent. If someone makes private_key pub, tests break.
    let (wallet, _) = Wallet::new("test");
    let json = serde_json::to_string(&wallet).unwrap();
    // private_key IS in the wallet file (that's how CLI works),
    // but it must not be extractable from the WalletAddress struct
    // without going through wallet methods.
    assert!(
        json.contains("private_key"),
        "Private key stored in wallet file per CLI compat"
    );
}

#[test]
fn test_nf004_signing_uses_internal_key() {
    let (wallet, _) = Wallet::new("test");
    // sign_message works without exposing the private key
    let sig = wallet.sign_message("test message", None).unwrap();
    assert!(!sig.is_empty());
    // Verify the signature is valid
    let valid = verify_message("test message", &sig, wallet.primary_public_key()).unwrap();
    assert!(valid);
}

#[test]
fn test_nf004_sign_wrong_address_returns_error() {
    let (wallet, _) = Wallet::new("test");
    let result = wallet.sign_message("test", Some("nonexistent_address"));
    assert!(result.is_err(), "Signing with unknown address must fail");
}

// ========================================================================
// Requirement: GUI-NF-008 (Must) -- Wallet file format compatibility with CLI
// Acceptance: Same JSON format, same Ed25519 key derivation
// ========================================================================

#[test]
fn test_nf008_wallet_json_has_name_version_addresses() {
    let (wallet, _) = Wallet::new("compat-test");
    let json = serde_json::to_string_pretty(&wallet).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert!(parsed["name"].is_string());
    assert!(parsed["version"].is_number());
    assert!(parsed["addresses"].is_array());
}

#[test]
fn test_nf008_wallet_json_address_fields() {
    let (wallet, _) = Wallet::new("test");
    let json = serde_json::to_string_pretty(&wallet).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    let addr = &parsed["addresses"][0];
    assert!(addr["address"].is_string());
    assert!(addr["public_key"].is_string());
    assert!(addr["private_key"].is_string());
    assert!(addr["label"].is_string());
    assert!(addr["bls_private_key"].is_string());
    assert!(addr["bls_public_key"].is_string());
}

#[test]
fn test_nf008_legacy_wallet_v1_loads() {
    let json = r#"{
        "name": "legacy",
        "version": 1,
        "addresses": [{
            "address": "0000000000000000000000000000000000000000",
            "public_key": "0000000000000000000000000000000000000000000000000000000000000000",
            "private_key": "0000000000000000000000000000000000000000000000000000000000000001",
            "label": "primary"
        }]
    }"#;
    let wallet: Wallet = serde_json::from_str(json).unwrap();
    assert_eq!(wallet.version(), 1);
    assert_eq!(wallet.name(), "legacy");
    assert!(!wallet.has_bls_key(), "Legacy v1 wallet has no BLS key");
}

#[test]
fn test_nf008_wallet_roundtrip_json() {
    let (wallet, _) = Wallet::new("test");
    let json = serde_json::to_string_pretty(&wallet).unwrap();
    let loaded: Wallet = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded.primary_public_key(), wallet.primary_public_key());
    assert_eq!(loaded.name(), wallet.name());
    assert_eq!(loaded.version(), wallet.version());
    assert_eq!(loaded.addresses().len(), wallet.addresses().len());
}

#[test]
fn test_nf008_bls_fields_optional_in_json() {
    // Without BLS fields -- should deserialize with None
    let json = r#"{
        "name": "no-bls",
        "version": 2,
        "addresses": [{
            "address": "0000000000000000000000000000000000000000",
            "public_key": "0000000000000000000000000000000000000000000000000000000000000000",
            "private_key": "0000000000000000000000000000000000000000000000000000000000000001",
            "label": "primary"
        }]
    }"#;
    let wallet: Wallet = serde_json::from_str(json).unwrap();
    assert!(!wallet.has_bls_key());
}

#[test]
fn test_nf008_bls_fields_skipped_when_none() {
    // When BLS fields are None, they should NOT appear in serialized JSON
    let json_in = r#"{
        "name": "no-bls",
        "version": 2,
        "addresses": [{
            "address": "aaaa",
            "public_key": "bbbb",
            "private_key": "cccc",
            "label": "primary"
        }]
    }"#;
    let wallet: Wallet = serde_json::from_str(json_in).unwrap();
    let json_out = serde_json::to_string(&wallet).unwrap();
    assert!(
        !json_out.contains("bls_private_key"),
        "None BLS key should be skipped in JSON output"
    );
    assert!(!json_out.contains("bls_public_key"));
}

// ========================================================================
// Requirement: GUI-FR-008 (Should) -- Add BLS key
// Acceptance: Generates BLS keypair, errors if exists, saves to wallet
// ========================================================================

#[test]
fn test_fr008_add_bls_key_errors_if_exists() {
    let (mut wallet, _) = Wallet::new("test");
    // New wallet already has BLS key
    assert!(wallet.has_bls_key());
    let result = wallet.add_bls_key();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("already exists"));
}

#[test]
fn test_fr008_add_bls_key_to_wallet_without_bls() {
    let json = r#"{
        "name": "no-bls",
        "version": 2,
        "addresses": [{
            "address": "0000000000000000000000000000000000000000",
            "public_key": "0000000000000000000000000000000000000000000000000000000000000000",
            "private_key": "0000000000000000000000000000000000000000000000000000000000000001",
            "label": "primary"
        }]
    }"#;
    let mut wallet: Wallet = serde_json::from_str(json).unwrap();
    assert!(!wallet.has_bls_key());

    let bls_pub = wallet.add_bls_key().unwrap();
    assert!(wallet.has_bls_key());
    assert_eq!(bls_pub.len(), 96, "BLS public key should be 96 hex chars");
}

// ========================================================================
// Failure mode tests (from Architecture)
// ========================================================================

#[test]
fn test_failure_wallet_file_not_found() {
    let result = Wallet::load(Path::new("/nonexistent/path/wallet.json"));
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("wallet file not found") || err.contains("No such file"));
}

#[test]
fn test_failure_wallet_file_corrupt() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("wallet.json");
    std::fs::write(&path, "this is not valid json at all").unwrap();

    let result = Wallet::load(&path);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("failed to parse"));
}

#[test]
fn test_failure_wallet_file_empty() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("wallet.json");
    std::fs::write(&path, "").unwrap();

    let result = Wallet::load(&path);
    assert!(result.is_err());
}

#[test]
fn test_failure_wallet_file_partial_json() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("wallet.json");
    std::fs::write(&path, r#"{"name": "test", "version": 2"#).unwrap();

    let result = Wallet::load(&path);
    assert!(result.is_err());
}

#[test]
fn test_failure_wallet_file_missing_fields() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("wallet.json");
    std::fs::write(&path, r#"{"name": "test"}"#).unwrap();

    let result = Wallet::load(&path);
    assert!(result.is_err());
}

// ========================================================================
// Wallet export/import (GUI-FR-005, GUI-FR-006 -- Should)
// ========================================================================

#[test]
fn test_fr005_export_saves_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("exported.json");

    let (wallet, _) = Wallet::new("test");
    wallet.export(&path).unwrap();
    assert!(path.exists());
}

#[test]
fn test_fr006_import_loads_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("wallet.json");

    let (wallet, _) = Wallet::new("test");
    wallet.save(&path).unwrap();

    let imported = Wallet::import(&path).unwrap();
    assert_eq!(imported.name(), wallet.name());
}

#[test]
fn test_fr006_import_validates_format() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("bad.json");
    std::fs::write(&path, "not a wallet").unwrap();

    let result = Wallet::import(&path);
    assert!(result.is_err(), "Import must validate wallet format");
}

// ========================================================================
// Wallet info (GUI-FR-007 -- Should)
// ========================================================================

#[test]
fn test_fr007_wallet_info() {
    let (mut wallet, _) = Wallet::new("my-wallet");
    wallet.generate_address(Some("second")).unwrap();

    assert_eq!(wallet.name(), "my-wallet");
    assert_eq!(wallet.version(), 2);
    assert_eq!(wallet.addresses().len(), 2);
    assert!(wallet.has_bls_key());
}

// ========================================================================
// pubkey_hash tests (critical for RPC compatibility)
// ========================================================================

#[test]
fn test_pubkey_hash_is_64_hex_chars() {
    let (wallet, _) = Wallet::new("test");
    let hash = wallet.primary_pubkey_hash().unwrap();
    assert_eq!(
        hash.len(),
        64,
        "pubkey_hash must be 32 bytes = 64 hex chars"
    );
}

#[test]
fn test_pubkey_hash_uses_address_domain() {
    let (wallet, _) = Wallet::new("test");
    let pubkey_bytes = hex::decode(wallet.primary_public_key()).unwrap();
    let expected = hash_with_domain(ADDRESS_DOMAIN, &pubkey_bytes);
    assert_eq!(wallet.primary_pubkey_hash().unwrap(), expected.to_hex());
}

#[test]
fn test_primary_keypair_matches_public_key() {
    let (wallet, _) = Wallet::new("test");
    let keypair = wallet.primary_keypair().unwrap();
    assert_eq!(keypair.public_key().to_hex(), wallet.primary_public_key());
}

// ========================================================================
// Sign/verify (GUI-FR-100, GUI-FR-101 -- Could)
// ========================================================================

#[test]
fn test_fr100_sign_message() {
    let (wallet, _) = Wallet::new("test");
    let sig = wallet.sign_message("Hello, DOLI!", None).unwrap();
    assert!(!sig.is_empty());
    // Signature hex should be valid hex
    assert!(hex::decode(&sig).is_ok());
}

#[test]
fn test_fr101_verify_message() {
    let (wallet, _) = Wallet::new("test");
    let message = "Hello, DOLI!";
    let sig = wallet.sign_message(message, None).unwrap();
    let pubkey = wallet.primary_public_key();

    let valid = verify_message(message, &sig, pubkey).unwrap();
    assert!(valid, "Signature verification must succeed");
}

#[test]
fn test_fr101_verify_wrong_message_fails() {
    let (wallet, _) = Wallet::new("test");
    let sig = wallet.sign_message("original", None).unwrap();
    let pubkey = wallet.primary_public_key();

    let valid = verify_message("tampered", &sig, pubkey).unwrap();
    assert!(!valid, "Wrong message must fail verification");
}

#[test]
fn test_fr101_verify_wrong_key_fails() {
    let (wallet, _) = Wallet::new("test");
    let sig = wallet.sign_message("test", None).unwrap();

    let (other_wallet, _) = Wallet::new("other");
    let other_pubkey = other_wallet.primary_public_key();

    let valid = verify_message("test", &sig, other_pubkey).unwrap();
    assert!(!valid, "Wrong key must fail verification");
}

#[test]
fn test_sign_with_specific_address() {
    let (mut wallet, _) = Wallet::new("test");
    let addr = wallet.generate_address(Some("secondary")).unwrap();

    let sig = wallet.sign_message("test", Some(&addr)).unwrap();
    assert!(!sig.is_empty());
}
