//! Wallet implementation

use std::path::Path;

use anyhow::{anyhow, Result};
use crypto::{hash::hash_with_domain, signature, KeyPair, PrivateKey, PublicKey, ADDRESS_DOMAIN};
use serde::{Deserialize, Serialize};

/// A wallet address with optional label
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WalletAddress {
    /// The address (hex)
    pub address: String,
    /// Public key (hex)
    pub public_key: String,
    /// Private key (hex, encrypted in real implementation)
    private_key: String,
    /// Optional label
    pub label: Option<String>,
}

/// Wallet file format
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Wallet {
    /// Wallet name
    name: String,
    /// Version
    version: u32,
    /// Addresses
    addresses: Vec<WalletAddress>,
}

impl Wallet {
    /// Create a new wallet with a primary address
    pub fn new(name: &str) -> Self {
        let kp = KeyPair::generate();
        let primary = WalletAddress {
            address: kp.address().to_hex(),
            public_key: kp.public_key().to_hex(),
            private_key: kp.private_key().to_hex(),
            label: Some("primary".to_string()),
        };

        Self {
            name: name.to_string(),
            version: 1,
            addresses: vec![primary],
        }
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

    /// Get primary address (20-byte truncated hash)
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
    fn test_new_wallet() {
        let wallet = Wallet::new("test");
        assert_eq!(wallet.name(), "test");
        assert_eq!(wallet.addresses().len(), 1);
    }

    #[test]
    fn test_generate_address() {
        let mut wallet = Wallet::new("test");
        let addr = wallet.generate_address(Some("secondary")).unwrap();

        assert_eq!(wallet.addresses().len(), 2);
        assert!(!addr.is_empty());
    }

    #[test]
    fn test_sign_verify() {
        let wallet = Wallet::new("test");
        let message = "Hello, DOLI!";

        let sig = wallet.sign_message(message, None).unwrap();
        let pubkey = &wallet.addresses()[0].public_key;

        let valid = verify_message(message, &sig, pubkey).unwrap();
        assert!(valid);
    }

    #[test]
    fn test_primary_pubkey_hash() {
        let wallet = Wallet::new("test");

        // pubkey_hash should be 32 bytes (64 hex chars)
        let pubkey_hash = wallet.primary_pubkey_hash();
        assert_eq!(pubkey_hash.len(), 64);

        // Verify it's the domain-separated BLAKE3 hash of the public key
        let pubkey_bytes = hex::decode(wallet.primary_public_key()).unwrap();
        let expected_hash = hash_with_domain(ADDRESS_DOMAIN, &pubkey_bytes);
        assert_eq!(pubkey_hash, expected_hash.to_hex());
    }

    #[test]
    fn test_primary_keypair() {
        let wallet = Wallet::new("test");

        // Should be able to get keypair
        let keypair = wallet.primary_keypair().unwrap();

        // Public key from keypair should match wallet's public key
        assert_eq!(keypair.public_key().to_hex(), wallet.primary_public_key());
    }
}
