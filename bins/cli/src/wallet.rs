//! Wallet implementation

use std::path::Path;

use anyhow::{anyhow, Result};
use bip39::Mnemonic;
use crypto::{hash::hash_with_domain, signature, KeyPair, PrivateKey, PublicKey, ADDRESS_DOMAIN};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// A wallet address with optional label
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WalletAddress {
    /// The address (hex)
    pub address: String,
    /// Public key (hex)
    pub public_key: String,
    /// Private key (hex)
    private_key: String,
    /// Optional label
    pub label: Option<String>,
}

/// Wallet file format
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
    /// Create a new wallet with a BIP-39 seed phrase.
    /// Returns (wallet, seed_phrase) — the phrase is returned for external storage.
    /// The seed phrase is NOT stored in the wallet file.
    pub fn new(name: &str) -> (Self, String) {
        let mnemonic = Mnemonic::generate(24).expect("mnemonic generation failed");
        let phrase = mnemonic.to_string();

        // Derive Ed25519 key from first 32 bytes of BIP-39 seed (empty passphrase)
        let bip39_seed = mnemonic.to_seed("");
        let mut ed25519_seed = [0u8; 32];
        ed25519_seed.copy_from_slice(&bip39_seed[..32]);

        let kp = KeyPair::from_seed(ed25519_seed);
        ed25519_seed.zeroize();

        let primary = WalletAddress {
            address: kp.address().to_hex(),
            public_key: kp.public_key().to_hex(),
            private_key: kp.private_key().to_hex(),
            label: Some("primary".to_string()),
        };

        let wallet = Self {
            name: name.to_string(),
            version: 2,
            addresses: vec![primary],
        };

        (wallet, phrase)
    }

    /// Load wallet from file
    pub fn load(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let wallet: Wallet = serde_json::from_str(&contents)?;
        Ok(wallet)
    }

    /// Save wallet to file
    pub fn save(&self, path: &Path) -> Result<()> {
        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let contents = serde_json::to_string_pretty(self)?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// Export wallet (same as save, but explicit)
    pub fn export(&self, path: &Path) -> Result<()> {
        self.save(path)
    }

    /// Import wallet from file
    pub fn import(path: &Path) -> Result<Self> {
        Self::load(path)
    }

    /// Get wallet name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get all addresses
    pub fn addresses(&self) -> &[WalletAddress] {
        &self.addresses
    }

    /// Get primary address (20-byte truncated hash, hex)
    #[allow(dead_code)]
    pub fn primary_address(&self) -> &str {
        &self.addresses[0].address
    }

    /// Get the pubkey_hash for the primary address (32-byte domain-separated BLAKE3 hash of public key)
    /// This is what the RPC endpoints expect for balance/UTXO queries
    /// Uses ADDRESS_DOMAIN for domain separation to match the rest of the system
    pub fn primary_pubkey_hash(&self) -> String {
        let pubkey_bytes =
            hex::decode(&self.addresses[0].public_key).expect("invalid public key in wallet");
        let hash = hash_with_domain(ADDRESS_DOMAIN, &pubkey_bytes);
        hash.to_hex()
    }

    /// Get the primary public key hex string
    pub fn primary_public_key(&self) -> &str {
        &self.addresses[0].public_key
    }

    /// Get a bech32m-encoded address for the primary key.
    ///
    /// `network_prefix` should be `"doli"`, `"tdoli"`, or `"ddoli"`.
    pub fn primary_bech32_address(&self, network_prefix: &str) -> String {
        let pubkey_bytes =
            hex::decode(&self.addresses[0].public_key).expect("invalid public key in wallet");
        crypto::address::from_pubkey(&pubkey_bytes, network_prefix).expect("bech32 encoding failed")
    }

    /// Get the keypair for the primary address
    pub fn primary_keypair(&self) -> Result<KeyPair> {
        let private_key = PrivateKey::from_hex(&self.addresses[0].private_key)
            .map_err(|e| anyhow!("Invalid private key: {}", e))?;
        Ok(KeyPair::from_private_key(private_key))
    }

    /// Generate a new address
    pub fn generate_address(&mut self, label: Option<&str>) -> Result<String> {
        let kp = KeyPair::generate();
        let addr = WalletAddress {
            address: kp.address().to_hex(),
            public_key: kp.public_key().to_hex(),
            private_key: kp.private_key().to_hex(),
            label: label.map(String::from),
        };

        let address = addr.address.clone();
        self.addresses.push(addr);

        Ok(address)
    }

    /// Find address entry by address string
    fn find_address(&self, address: &str) -> Option<&WalletAddress> {
        self.addresses.iter().find(|a| a.address == address)
    }

    /// Sign a message with a specific address (or primary)
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

    /// Get private key for an address
    #[allow(dead_code)]
    pub fn get_private_key(&self, address: &str) -> Result<PrivateKey> {
        let addr = self
            .find_address(address)
            .ok_or_else(|| anyhow!("Address not found: {}", address))?;

        PrivateKey::from_hex(&addr.private_key).map_err(|e| anyhow!("Invalid key: {}", e))
    }
}

/// Verify a message signature
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
mod tests {
    use super::*;

    #[test]
    fn test_new_wallet_v2_returns_seed_phrase() {
        let (wallet, phrase) = Wallet::new("test");
        assert_eq!(wallet.name(), "test");
        assert_eq!(wallet.version, 2);
        assert_eq!(phrase.split_whitespace().count(), 24);
        assert_eq!(wallet.addresses().len(), 1);
    }

    #[test]
    fn test_v2_wallet_json_has_no_seed() {
        let (wallet, _phrase) = Wallet::new("test");
        let json = serde_json::to_string_pretty(&wallet).unwrap();
        assert!(!json.contains("seed_phrase"));
    }

    #[test]
    fn test_seed_phrase_deterministic_key() {
        let (wallet, phrase) = Wallet::new("test");

        // Re-derive key from same phrase
        let mnemonic: Mnemonic = phrase.parse().unwrap();
        let bip39_seed = mnemonic.to_seed("");
        let kp = KeyPair::from_seed(bip39_seed[..32].try_into().unwrap());

        assert_eq!(kp.public_key().to_hex(), wallet.primary_public_key());
    }

    #[test]
    fn test_legacy_wallet_loads() {
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
        assert_eq!(wallet.version, 1);
    }

    #[test]
    fn test_v2_wallet_roundtrip() {
        let (wallet, _phrase) = Wallet::new("test");
        let json = serde_json::to_string_pretty(&wallet).unwrap();
        let loaded: Wallet = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.primary_public_key(), wallet.primary_public_key());
    }

    #[test]
    fn test_generate_address() {
        let (mut wallet, _) = Wallet::new("test");
        let addr = wallet.generate_address(Some("secondary")).unwrap();

        assert_eq!(wallet.addresses().len(), 2);
        assert!(!addr.is_empty());
    }

    #[test]
    fn test_sign_verify() {
        let (wallet, _) = Wallet::new("test");
        let message = "Hello, DOLI!";

        let sig = wallet.sign_message(message, None).unwrap();
        let pubkey = &wallet.addresses()[0].public_key;

        let valid = verify_message(message, &sig, pubkey).unwrap();
        assert!(valid);
    }

    #[test]
    fn test_primary_pubkey_hash() {
        let (wallet, _) = Wallet::new("test");

        let pubkey_hash = wallet.primary_pubkey_hash();
        assert_eq!(pubkey_hash.len(), 64);

        let pubkey_bytes = hex::decode(wallet.primary_public_key()).unwrap();
        let expected_hash = hash_with_domain(ADDRESS_DOMAIN, &pubkey_bytes);
        assert_eq!(pubkey_hash, expected_hash.to_hex());
    }

    #[test]
    fn test_primary_keypair() {
        let (wallet, _) = Wallet::new("test");

        let keypair = wallet.primary_keypair().unwrap();
        assert_eq!(keypair.public_key().to_hex(), wallet.primary_public_key());
    }
}
