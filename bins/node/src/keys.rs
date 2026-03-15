use std::path::PathBuf;

use anyhow::{anyhow, Result};
use crypto::{KeyPair, PrivateKey};
use tracing::info;

/// Load a producer key from a JSON wallet file
pub(crate) fn load_producer_key(path: &PathBuf) -> Result<KeyPair> {
    let contents =
        std::fs::read_to_string(path).map_err(|e| anyhow!("Failed to read key file: {}", e))?;

    // Try to parse as a wallet JSON (same format as doli-cli)
    #[derive(serde::Deserialize)]
    struct WalletAddress {
        private_key: String,
    }

    #[derive(serde::Deserialize)]
    struct Wallet {
        addresses: Vec<WalletAddress>,
    }

    let wallet: Wallet =
        serde_json::from_str(&contents).map_err(|e| anyhow!("Failed to parse key file: {}", e))?;

    if wallet.addresses.is_empty() {
        return Err(anyhow!("Wallet file contains no addresses"));
    }

    let private_key = PrivateKey::from_hex(&wallet.addresses[0].private_key)
        .map_err(|e| anyhow!("Invalid private key: {}", e))?;

    Ok(KeyPair::from_private_key(private_key))
}

/// Load a BLS key from a JSON wallet file (optional — returns None if not present).
///
/// BLS keys are stored in the wallet alongside Ed25519 keys. Pre-BLS wallets
/// simply won't have the field, so this gracefully returns None.
pub(crate) fn load_bls_key(path: &PathBuf) -> Option<crypto::BlsKeyPair> {
    let contents = std::fs::read_to_string(path).ok()?;

    #[derive(serde::Deserialize)]
    struct WalletAddress {
        #[serde(default)]
        bls_private_key: Option<String>,
    }

    #[derive(serde::Deserialize)]
    struct Wallet {
        addresses: Vec<WalletAddress>,
    }

    let wallet: Wallet = serde_json::from_str(&contents).ok()?;
    let bls_hex = wallet.addresses.first()?.bls_private_key.as_ref()?;
    let bls_bytes = hex::decode(bls_hex).ok()?;
    let bls_arr: [u8; 32] = bls_bytes.try_into().ok()?;
    let secret = crypto::BlsSecretKey::from_bytes(bls_arr).ok()?;
    let kp = crypto::BlsKeyPair::from_secret_key(secret);
    info!(
        "BLS key loaded: {}",
        &hex::encode(kp.public_key().as_bytes())[..16]
    );
    Some(kp)
}
