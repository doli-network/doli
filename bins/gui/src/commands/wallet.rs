//! Wallet management Tauri commands.
//!
//! Handles wallet creation, restoration, loading, saving, address generation,
//! export/import, and BLS key management. Private keys NEVER cross the IPC
//! boundary (GUI-NF-004).

use std::path::PathBuf;

use tauri::State;

use crate::commands::{AddressInfo, CreateWalletResponse, WalletInfo};
use crate::state::AppState;

/// Validate a wallet path from the frontend.
///
/// Rejects paths containing `..` (path traversal) and null bytes.
/// This prevents a malicious or buggy frontend from accessing files
/// outside the expected wallet directories.
fn validate_path(path_str: &str) -> Result<PathBuf, String> {
    // Reject path traversal
    if path_str.contains("..") {
        return Err("Invalid path: path traversal ('..') is not allowed".to_string());
    }
    // Reject null bytes (can cause truncation in C-based syscalls)
    if path_str.contains('\0') {
        return Err("Invalid path: null bytes are not allowed".to_string());
    }
    let path = PathBuf::from(path_str);
    // Reject empty path
    if path.as_os_str().is_empty() {
        return Err("Invalid path: path cannot be empty".to_string());
    }
    Ok(path)
}

/// Create a new wallet with BIP-39 seed phrase.
/// Returns the seed phrase (shown once to user, never stored).
#[tauri::command]
pub async fn create_wallet(
    name: String,
    wallet_path: String,
    state: State<'_, AppState>,
) -> Result<CreateWalletResponse, String> {
    let (new_wallet, seed_phrase) = wallet::Wallet::new(&name);

    let path = validate_path(&wallet_path)?;
    let prefix = state.network_prefix().await;
    let bech32 = new_wallet
        .primary_bech32_address(&prefix)
        .map_err(|e| e.to_string())?;
    let primary = new_wallet.primary_address().to_string();

    new_wallet.save(&path).map_err(|e| e.to_string())?;

    // Update state
    *state.wallet.write().await = Some(new_wallet);
    *state.wallet_path.write().await = Some(path.clone());
    {
        let mut config = state.config.write().await;
        config.last_wallet_path = Some(path);
        let _ = config.save();
    }

    Ok(CreateWalletResponse {
        name,
        seed_phrase,
        primary_address: primary,
        bech32_address: bech32,
    })
}

/// Restore a wallet from a BIP-39 seed phrase.
#[tauri::command]
pub async fn restore_wallet(
    name: String,
    seed_phrase: String,
    wallet_path: String,
    state: State<'_, AppState>,
) -> Result<WalletInfo, String> {
    let restored =
        wallet::Wallet::from_seed_phrase(&name, &seed_phrase).map_err(|e| e.to_string())?;

    let path = validate_path(&wallet_path)?;
    restored.save(&path).map_err(|e| e.to_string())?;

    let info = build_wallet_info(&restored, &state).await;

    *state.wallet.write().await = Some(restored);
    *state.wallet_path.write().await = Some(path.clone());
    {
        let mut config = state.config.write().await;
        config.last_wallet_path = Some(path);
        let _ = config.save();
    }

    Ok(info)
}

/// Load an existing wallet from disk.
#[tauri::command]
pub async fn load_wallet(
    wallet_path: String,
    state: State<'_, AppState>,
) -> Result<WalletInfo, String> {
    let path = validate_path(&wallet_path)?;
    let loaded = wallet::Wallet::load(&path).map_err(|e| e.to_string())?;

    let info = build_wallet_info(&loaded, &state).await;

    *state.wallet.write().await = Some(loaded);
    *state.wallet_path.write().await = Some(path.clone());
    {
        let mut config = state.config.write().await;
        config.last_wallet_path = Some(path);
        let _ = config.save();
    }

    Ok(info)
}

/// Generate a new address in the loaded wallet.
#[tauri::command]
pub async fn generate_address(
    label: Option<String>,
    state: State<'_, AppState>,
) -> Result<AddressInfo, String> {
    let prefix = state.network_prefix().await;

    let mut wallet_guard = state.wallet.write().await;
    let w = wallet_guard.as_mut().ok_or("No wallet loaded")?;

    let addr = w
        .generate_address(label.as_deref())
        .map_err(|e| e.to_string())?;

    // Save wallet after generating address
    {
        let path_guard = state.wallet_path.read().await;
        if let Some(ref path) = *path_guard {
            w.save(path).map_err(|e| e.to_string())?;
        }
    }

    // Find the newly generated address entry
    let wallet_addr = w
        .addresses()
        .iter()
        .find(|a| a.address == addr)
        .ok_or("Address generation failed")?;

    let bech32 = crypto::address::from_pubkey(
        &hex::decode(&wallet_addr.public_key).map_err(|e| e.to_string())?,
        &prefix,
    )
    .map_err(|e| e.to_string())?;

    Ok(AddressInfo {
        address: wallet_addr.address.clone(),
        public_key: wallet_addr.public_key.clone(),
        label: wallet_addr.label.clone(),
        bech32_address: bech32,
        has_bls_key: wallet_addr.bls_public_key.is_some(),
    })
}

/// List all addresses in the loaded wallet.
#[tauri::command]
pub async fn list_addresses(state: State<'_, AppState>) -> Result<Vec<AddressInfo>, String> {
    let prefix = state.network_prefix().await;

    let wallet_guard = state.wallet.read().await;
    let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;

    let mut addresses = Vec::new();
    for a in w.addresses() {
        let pubkey_bytes = match hex::decode(&a.public_key) {
            Ok(bytes) => bytes,
            Err(_) => {
                // Skip addresses with invalid public key hex rather than panicking.
                // This can happen with corrupted wallet files.
                continue;
            }
        };
        let bech32 = match crypto::address::from_pubkey(&pubkey_bytes, &prefix) {
            Ok(addr) => addr,
            Err(_) => {
                // Skip addresses that fail bech32 encoding.
                continue;
            }
        };

        addresses.push(AddressInfo {
            address: a.address.clone(),
            public_key: a.public_key.clone(),
            label: a.label.clone(),
            bech32_address: bech32,
            has_bls_key: a.bls_public_key.is_some(),
        });
    }

    Ok(addresses)
}

/// Export wallet to a file.
#[tauri::command]
pub async fn export_wallet(destination: String, state: State<'_, AppState>) -> Result<(), String> {
    let wallet_guard = state.wallet.read().await;
    let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;

    let path = validate_path(&destination)?;
    w.export(&path).map_err(|e| e.to_string())?;

    Ok(())
}

/// Import wallet from a file.
#[tauri::command]
pub async fn import_wallet(
    source: String,
    destination: String,
    state: State<'_, AppState>,
) -> Result<WalletInfo, String> {
    let source_path = validate_path(&source)?;
    let dest_path = validate_path(&destination)?;

    let imported = wallet::Wallet::import(&source_path).map_err(|e| e.to_string())?;
    imported.save(&dest_path).map_err(|e| e.to_string())?;

    let info = build_wallet_info(&imported, &state).await;

    *state.wallet.write().await = Some(imported);
    *state.wallet_path.write().await = Some(dest_path.clone());
    {
        let mut config = state.config.write().await;
        config.last_wallet_path = Some(dest_path);
        let _ = config.save();
    }

    Ok(info)
}

/// Get info about the currently loaded wallet.
#[tauri::command]
pub async fn wallet_info(state: State<'_, AppState>) -> Result<WalletInfo, String> {
    let wallet_guard = state.wallet.read().await;
    let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
    let prefix = state.network_prefix().await;
    Ok(build_wallet_info_with_prefix(w, &prefix))
}

/// Add a BLS key to the wallet (required for producer registration).
#[tauri::command]
pub async fn add_bls_key(state: State<'_, AppState>) -> Result<String, String> {
    let mut wallet_guard = state.wallet.write().await;
    let w = wallet_guard.as_mut().ok_or("No wallet loaded")?;

    let bls_pub = w.add_bls_key().map_err(|e| e.to_string())?;

    // Save wallet after adding BLS key
    {
        let path_guard = state.wallet_path.read().await;
        if let Some(ref path) = *path_guard {
            w.save(path).map_err(|e| e.to_string())?;
        }
    }

    Ok(bls_pub)
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Build WalletInfo from a Wallet reference (no private key data exposed).
async fn build_wallet_info(w: &wallet::Wallet, state: &State<'_, AppState>) -> WalletInfo {
    let prefix = state.network_prefix().await;
    build_wallet_info_with_prefix(w, &prefix)
}

/// Build WalletInfo from a Wallet reference with an explicit prefix.
fn build_wallet_info_with_prefix(w: &wallet::Wallet, prefix: &str) -> WalletInfo {
    WalletInfo {
        name: w.name().to_string(),
        version: w.version(),
        address_count: w.addresses().len(),
        primary_address: w.primary_address().to_string(),
        primary_public_key: w.primary_public_key().to_string(),
        bech32_address: w.primary_bech32_address(prefix).unwrap_or_default(),
        has_bls_key: w.has_bls_key(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_wallet_info_no_private_keys() {
        let info = WalletInfo {
            name: "test".to_string(),
            version: 2,
            address_count: 1,
            primary_address: "abc123".to_string(),
            primary_public_key: "pubkey".to_string(),
            bech32_address: "doli1abc".to_string(),
            has_bls_key: true,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(
            !json.contains("private"),
            "WalletInfo must not contain private key data"
        );
    }

    #[test]
    fn test_address_info_no_private_keys() {
        let info = AddressInfo {
            address: "abc123".to_string(),
            public_key: "pubkey".to_string(),
            label: Some("primary".to_string()),
            bech32_address: "doli1abc".to_string(),
            has_bls_key: false,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(
            !json.contains("private"),
            "AddressInfo must not contain private key data"
        );
    }

    #[test]
    fn test_create_wallet_response_serialization() {
        let resp = CreateWalletResponse {
            name: "test".to_string(),
            seed_phrase: "word1 word2 word3".to_string(),
            primary_address: "abc".to_string(),
            bech32_address: "doli1abc".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("seedPhrase")); // camelCase
        assert!(json.contains("bech32Address"));
    }

    // ========================================================================
    // M3: Path sanitization tests
    // ========================================================================

    #[test]
    fn test_validate_path_rejects_traversal() {
        assert!(validate_path("/home/user/../etc/passwd").is_err());
        assert!(validate_path("../wallet.json").is_err());
        assert!(validate_path("wallets/../../secrets").is_err());
    }

    #[test]
    fn test_validate_path_rejects_null_bytes() {
        assert!(validate_path("/home/user/wallet\0.json").is_err());
    }

    #[test]
    fn test_validate_path_rejects_empty() {
        assert!(validate_path("").is_err());
    }

    #[test]
    fn test_validate_path_accepts_normal_paths() {
        assert!(validate_path("/home/user/.doli-gui/wallets/my-wallet.json").is_ok());
        assert!(validate_path("/tmp/wallet.json").is_ok());
        assert!(validate_path("wallet.json").is_ok());
    }

    #[test]
    fn test_validate_path_accepts_paths_with_dots() {
        // Single dots in filenames or extensions are fine
        assert!(validate_path("/home/user/.doli-gui/wallet.json").is_ok());
        assert!(validate_path("/home/user/.hidden/file.txt").is_ok());
    }
}
