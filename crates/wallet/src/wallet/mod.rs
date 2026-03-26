//! Wallet management: creation, restoration, key derivation, address generation,
//! file persistence, and message signing.
//!
//! This module is extracted from `bins/cli/src/wallet.rs` to be shared between
//! the CLI and GUI. The wallet file format is identical.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use bip39::Mnemonic;
use crypto::{
    hash::hash_with_domain, signature, BlsKeyPair, KeyPair, PrivateKey, PublicKey, ADDRESS_DOMAIN,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// A wallet address with optional label.
/// Matches the CLI's WalletAddress struct exactly for format compatibility (GUI-NF-008).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WalletAddress {
    /// The address (hex, 20-byte truncated hash)
    pub address: String,
    /// Public key (hex, 32 bytes)
    pub public_key: String,
    /// Private key (hex, 32 bytes) -- NEVER exposed to frontend (GUI-NF-004)
    private_key: String,
    /// Optional label
    pub label: Option<String>,
    /// BLS private key (hex, 32 bytes)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bls_private_key: Option<String>,
    /// BLS public key (hex, 48 bytes)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bls_public_key: Option<String>,
}

/// Wallet file format.
/// Version 1 = legacy (random key), Version 2 = BIP-39 derived key.
/// Matches the CLI's Wallet struct exactly for format compatibility (GUI-NF-008).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Wallet {
    /// Wallet name
    name: String,
    /// Version (1 = legacy, 2 = BIP-39 derived key)
    version: u32,
    /// Addresses
    addresses: Vec<WalletAddress>,
}

impl Wallet {
    /// Create a new wallet with a BIP-39 seed phrase (24 words).
    /// Returns (wallet, seed_phrase). The seed phrase is NOT stored in the wallet file.
    pub fn new(name: &str) -> (Self, String) {
        let mnemonic = Mnemonic::generate(24).expect("mnemonic generation failed");
        let phrase = mnemonic.to_string();

        // Derive Ed25519 key from first 32 bytes of BIP-39 seed (empty passphrase)
        let bip39_seed = mnemonic.to_seed("");
        let mut ed25519_seed = [0u8; 32];
        ed25519_seed.copy_from_slice(&bip39_seed[..32]);

        let kp = KeyPair::from_seed(ed25519_seed);
        ed25519_seed.zeroize();

        // Generate BLS keypair for attestation
        let bls_kp = BlsKeyPair::generate();

        let primary = WalletAddress {
            address: kp.address().to_hex(),
            public_key: kp.public_key().to_hex(),
            private_key: kp.private_key().to_hex(),
            label: Some("primary".to_string()),
            bls_private_key: Some(bls_kp.secret_key().to_hex()),
            bls_public_key: Some(bls_kp.public_key().to_hex()),
        };

        let wallet = Self {
            name: name.to_string(),
            version: 2,
            addresses: vec![primary],
        };

        (wallet, phrase)
    }

    /// Restore a wallet from a BIP-39 seed phrase.
    /// Derives identical Ed25519 key as `new()`. Generates new BLS keypair.
    pub fn from_seed_phrase(name: &str, phrase: &str) -> Result<Self> {
        let mnemonic: Mnemonic = phrase
            .parse()
            .map_err(|e| anyhow!("Invalid seed phrase: {}", e))?;
        let bip39_seed = mnemonic.to_seed("");
        let mut ed25519_seed = [0u8; 32];
        ed25519_seed.copy_from_slice(&bip39_seed[..32]);

        let kp = KeyPair::from_seed(ed25519_seed);
        ed25519_seed.zeroize();

        let bls_kp = BlsKeyPair::generate();

        let primary = WalletAddress {
            address: kp.address().to_hex(),
            public_key: kp.public_key().to_hex(),
            private_key: kp.private_key().to_hex(),
            label: Some("primary".to_string()),
            bls_private_key: Some(bls_kp.secret_key().to_hex()),
            bls_public_key: Some(bls_kp.public_key().to_hex()),
        };

        Ok(Self {
            name: name.to_string(),
            version: 2,
            addresses: vec![primary],
        })
    }

    /// Load wallet from a JSON file.
    pub fn load(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("wallet file not found: {}", path.display()))?;
        let wallet: Wallet = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse wallet file: {}", path.display()))?;
        Ok(wallet)
    }

    /// Save wallet to a JSON file. Creates parent directories if needed.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string_pretty(self)?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// Export wallet (same as save, but semantically distinct).
    pub fn export(&self, path: &Path) -> Result<()> {
        self.save(path)
    }

    /// Import wallet from a JSON file (same as load, but semantically distinct).
    pub fn import(path: &Path) -> Result<Self> {
        Self::load(path)
    }

    /// Get wallet name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get wallet version.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Get all addresses.
    pub fn addresses(&self) -> &[WalletAddress] {
        &self.addresses
    }

    /// Get the primary address (20-byte truncated hash, hex).
    pub fn primary_address(&self) -> &str {
        &self.addresses[0].address
    }

    /// Get the primary public key hex string.
    pub fn primary_public_key(&self) -> &str {
        &self.addresses[0].public_key
    }

    /// Get the pubkey_hash for the primary address.
    /// 32-byte domain-separated BLAKE3 hash of public key using ADDRESS_DOMAIN.
    ///
    /// Returns an error if the wallet's public key hex is invalid.
    pub fn primary_pubkey_hash(&self) -> Result<String> {
        let pubkey_bytes = hex::decode(&self.addresses[0].public_key)
            .map_err(|e| anyhow!("invalid public key hex in wallet: {}", e))?;
        let hash = hash_with_domain(ADDRESS_DOMAIN, &pubkey_bytes);
        Ok(hash.to_hex())
    }

    /// Get a bech32m-encoded address for the primary key.
    /// `network_prefix` should be `"doli"`, `"tdoli"`, or `"ddoli"`.
    ///
    /// Returns an error if the public key hex is invalid or bech32 encoding fails.
    pub fn primary_bech32_address(&self, network_prefix: &str) -> Result<String> {
        let pubkey_bytes = hex::decode(&self.addresses[0].public_key)
            .map_err(|e| anyhow!("invalid public key hex in wallet: {}", e))?;
        let addr = crypto::address::from_pubkey(&pubkey_bytes, network_prefix)
            .map_err(|e| anyhow!("bech32 encoding failed: {}", e))?;
        Ok(addr)
    }

    /// Get the keypair for the primary address.
    pub fn primary_keypair(&self) -> Result<KeyPair> {
        let private_key = PrivateKey::from_hex(&self.addresses[0].private_key)
            .map_err(|e| anyhow!("Invalid private key: {}", e))?;
        Ok(KeyPair::from_private_key(private_key))
    }

    /// Check if the primary address has a BLS key.
    pub fn has_bls_key(&self) -> bool {
        self.addresses
            .first()
            .and_then(|a| a.bls_private_key.as_ref())
            .is_some()
    }

    /// Get the primary BLS public key hex (if present).
    pub fn primary_bls_public_key(&self) -> Option<&str> {
        self.addresses
            .first()
            .and_then(|a| a.bls_public_key.as_deref())
    }

    /// Generate a new address (random Ed25519 keypair).
    pub fn generate_address(&mut self, label: Option<&str>) -> Result<String> {
        let kp = KeyPair::generate();
        let addr = WalletAddress {
            address: kp.address().to_hex(),
            public_key: kp.public_key().to_hex(),
            private_key: kp.private_key().to_hex(),
            label: label.map(String::from),
            bls_private_key: None,
            bls_public_key: None,
        };

        let address = addr.address.clone();
        self.addresses.push(addr);
        Ok(address)
    }

    /// Add a BLS keypair to the primary address.
    /// Returns the BLS public key hex. Errors if BLS key already exists.
    pub fn add_bls_key(&mut self) -> Result<String> {
        if self.has_bls_key() {
            return Err(anyhow!("BLS key already exists in this wallet"));
        }
        let bls_kp = BlsKeyPair::generate();
        let bls_pub_hex = bls_kp.public_key().to_hex();
        let addr = self
            .addresses
            .first_mut()
            .ok_or_else(|| anyhow!("Wallet has no addresses"))?;
        addr.bls_private_key = Some(bls_kp.secret_key().to_hex());
        addr.bls_public_key = Some(bls_pub_hex.clone());
        Ok(bls_pub_hex)
    }

    /// Sign a message with a specific address (or primary if None).
    pub fn sign_message(&self, message: &str, address: Option<&str>) -> Result<String> {
        let addr = match address {
            Some(a) => self
                .find_address(a)
                .ok_or_else(|| anyhow!("Address not found: {}", a))?,
            None => &self.addresses[0],
        };

        let private_key = PrivateKey::from_hex(&addr.private_key)?;
        let message_hash = crypto::hash::hash(message.as_bytes());
        let sig = signature::sign(message_hash.as_bytes(), &private_key);

        Ok(sig.to_hex())
    }

    /// Find address entry by address string.
    fn find_address(&self, address: &str) -> Option<&WalletAddress> {
        self.addresses.iter().find(|a| a.address == address)
    }
}

/// Verify a message signature.
pub fn verify_message(message: &str, sig_hex: &str, pubkey_hex: &str) -> Result<bool> {
    let public_key = PublicKey::from_hex(pubkey_hex)?;
    let sig = crypto::Signature::from_hex(sig_hex)?;
    let message_hash = crypto::hash::hash(message.as_bytes());

    match signature::verify(message_hash.as_bytes(), &sig, &public_key) {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests;
