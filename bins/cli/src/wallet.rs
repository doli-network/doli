//! Wallet implementation

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use bip39::Mnemonic;
use crypto::{
    hash::hash_with_domain, signature, BlsKeyPair, KeyPair, PrivateKey, PublicKey, ADDRESS_DOMAIN,
};
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
    /// BLS private key (hex, 32 bytes) — for attestation signing
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bls_private_key: Option<String>,
    /// BLS public key (hex, 48 bytes) — for attestation verification
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bls_public_key: Option<String>,
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
    /// INC-I-167: the file this wallet was loaded from, if any. Runtime-only —
    /// never serialized, so the on-disk format is unchanged and old/new binaries
    /// read each other's wallets.
    ///
    /// `save()` uses it to tell a *save-back* (persisting a wallet to the file it
    /// came from — always allowed) from a *create* (writing a wallet somewhere new
    /// — must not clobber an existing file). Deserialization leaves this `None`,
    /// which is deliberately the conservative value: a wallet with no known origin
    /// gets create semantics, so forgetting to set it fails safe rather than
    /// destructive.
    #[serde(skip)]
    origin: Option<PathBuf>,
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
            origin: None,
        };

        (wallet, phrase)
    }

    /// Restore a wallet from a BIP-39 seed phrase.
    /// Uses the same derivation as `new()`: first 32 bytes of BIP-39 seed → Ed25519 key.
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
            origin: None,
        })
    }

    /// Load wallet from file
    pub fn load(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path).with_context(|| {
            if path.exists() {
                format!(
                    "cannot read wallet: {}\n  Check file permissions.",
                    path.display()
                )
            } else {
                #[cfg(target_os = "linux")]
                {
                    format!(
                        "wallet not found: {}\n  Create one: doli init",
                        path.display()
                    )
                }
                #[cfg(not(target_os = "linux"))]
                {
                    format!(
                        "wallet not found: {}\n  Use -w to specify the wallet path, e.g.: doli -w /path/to/wallet.json <command>",
                        path.display()
                    )
                }
            }
        })?;
        let mut wallet: Wallet = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse wallet file: {}", path.display()))?;
        // INC-I-167: remember where this wallet came from, so save() can allow a
        // save-back to this same file while refusing to clobber a different one.
        wallet.origin = Some(path.to_path_buf());
        Ok(wallet)
    }

    /// Save wallet to file.
    ///
    /// INC-I-167: refuses to overwrite an existing file that this wallet was not
    /// loaded from. Overwriting is opt-in via [`Wallet::save_forced`], not the
    /// default — `wallet.json` may be the only copy of a producer's registered BLS
    /// key, which a 24-word seed phrase does NOT restore (INC-I-162), so a silent
    /// clobber is unrecoverable except by exit + re-register at ~75% bond burn.
    ///
    /// # Errors
    /// Returns an error if `path` exists and is not this wallet's origin, or if the
    /// underlying atomic write fails.
    pub fn save(&self, path: &Path) -> Result<()> {
        if !self.is_origin(path) && path.exists() {
            anyhow::bail!(
                "Refusing to overwrite existing wallet at {}\n  \
                 That file may be the only copy of its BLS producer key — a 24-word \
                 seed phrase does NOT restore it.\n  \
                 Back it up first, then use a different path with -w, or --force if \
                 you really mean to replace it.",
                path.display()
            );
        }
        self.write_to(path)
    }

    /// Save wallet to file, bypassing the overwrite guard.
    ///
    /// Only for flows that have already obtained explicit destructive consent from
    /// the operator (`doli init --force`, which warns and requires the flag).
    ///
    /// # Errors
    /// Returns an error if the atomic write fails.
    pub fn save_forced(&self, path: &Path) -> Result<()> {
        self.write_to(path)
    }

    /// Is `path` the file this wallet was loaded from?
    ///
    /// Compares literally first, then by canonical path so that equivalent
    /// spellings (`./w.json` vs `w.json`) still count as the same file. A wallet
    /// with no origin is never a save-back, so the answer is `false` — the
    /// conservative direction.
    fn is_origin(&self, path: &Path) -> bool {
        let Some(origin) = self.origin.as_deref() else {
            return false;
        };
        if origin == path {
            return true;
        }
        match (origin.canonicalize(), path.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }

    /// Serialize and write atomically: a fully-written temp file in the same
    /// directory is `fsync`ed and then `rename`d over the destination.
    ///
    /// INC-I-167: the previous implementation used `std::fs::write`, so a crash or
    /// full disk mid-write left a truncated wallet — losing key material with no
    /// torn-write protection. `rename` within a directory is atomic on POSIX, so a
    /// reader sees either the old file or the new one, never a partial one.
    fn write_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let dir = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        let file_name = path
            .file_name()
            .ok_or_else(|| anyhow!("invalid wallet path: {}", path.display()))?;

        // Temp name is scoped to this process so it can never collide with, or be
        // mistaken for, another writer's in-progress file.
        let mut tmp_name = std::ffi::OsString::from(".");
        tmp_name.push(file_name);
        tmp_name.push(format!(".tmp{}", std::process::id()));
        let tmp = dir.join(tmp_name);
        let _ = std::fs::remove_file(&tmp); // clear a stale temp from a prior crash

        let contents = serde_json::to_string_pretty(self)?;

        let write_result = (|| -> Result<()> {
            use std::io::Write;
            // AUDIT-KEY-001: wallet files contain private keys. Create the temp file
            // with its final permissions BEFORE writing, so key material is never
            // briefly world-readable at the process umask.
            //
            // Mode: preserve the destination's existing mode if it has one, so a
            // save-back never WIDENS an operator's hardened permissions (a producer
            // wallet.json at 0600 stays 0600). New files get 0640 — owner rw, group
            // read for the doli service user, no world access.
            #[cfg(unix)]
            let mut file = {
                use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
                let mode = std::fs::metadata(path)
                    .map(|m| m.permissions().mode() & 0o777)
                    .unwrap_or(0o640);
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(mode)
                    .open(&tmp)
                    .with_context(|| format!("cannot create temp wallet file: {}", tmp.display()))?
            };
            #[cfg(not(unix))]
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)
                .with_context(|| format!("cannot create temp wallet file: {}", tmp.display()))?;

            file.write_all(contents.as_bytes())?;
            file.sync_all()?; // durable before the rename makes it visible
            Ok(())
        })();

        if let Err(e) = write_result {
            let _ = std::fs::remove_file(&tmp); // never leave key material behind
            return Err(e);
        }

        if let Err(e) = std::fs::rename(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(anyhow!("cannot write wallet to {}: {}", path.display(), e));
        }

        // Persist the directory entry so the rename survives a power loss.
        #[cfg(unix)]
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    }

    /// Export wallet (same as save, but explicit)
    pub fn export(&self, path: &Path) -> Result<()> {
        self.save(path)
    }

    /// Export wallet, bypassing the overwrite guard (`doli export --force`).
    ///
    /// # Errors
    /// Returns an error if the atomic write fails.
    pub fn export_forced(&self, path: &Path) -> Result<()> {
        self.save_forced(path)
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

    /// Generate a new address.
    ///
    /// WARNING: This generates a RANDOM keypair, NOT derived from the BIP-39 seed.
    /// If the wallet is restored from the seed phrase, this address will be LOST
    /// along with any funds sent to it. Users are warned at generation time.
    pub fn generate_address(&mut self, label: Option<&str>) -> Result<String> {
        eprintln!(
            "WARNING: This address is randomly generated, NOT derived from your seed phrase."
        );
        eprintln!("         If you restore from seed, funds at this address will be LOST.");
        eprintln!("         Use this only for temporary purposes.");

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

    /// Get pubkey_hashes for ALL addresses in the wallet.
    /// Returns Vec of (pubkey_hash_hex, address_index).
    pub fn all_pubkey_hashes(&self) -> Vec<(String, usize)> {
        self.addresses
            .iter()
            .enumerate()
            .filter_map(|(i, addr)| {
                let pubkey_bytes = hex::decode(&addr.public_key).ok()?;
                let hash = hash_with_domain(ADDRESS_DOMAIN, &pubkey_bytes);
                Some((hash.to_hex(), i))
            })
            .collect()
    }

    /// Get the keypair for a specific address by its pubkey_hash.
    /// Searches all addresses in the wallet.
    pub fn keypair_for_pubkey_hash(&self, pubkey_hash: &str) -> Result<KeyPair> {
        for addr in &self.addresses {
            let pubkey_bytes =
                hex::decode(&addr.public_key).map_err(|e| anyhow!("Invalid pubkey: {}", e))?;
            let hash = hash_with_domain(ADDRESS_DOMAIN, &pubkey_bytes);
            if hash.to_hex() == pubkey_hash {
                let private_key = PrivateKey::from_hex(&addr.private_key)
                    .map_err(|e| anyhow!("Invalid private key: {}", e))?;
                return Ok(KeyPair::from_private_key(private_key));
            }
        }
        Err(anyhow!(
            "No address in wallet matches pubkey_hash: {}",
            pubkey_hash
        ))
    }

    /// Check if the primary address has a BLS key
    pub fn has_bls_key(&self) -> bool {
        self.addresses
            .first()
            .and_then(|a| a.bls_private_key.as_ref())
            .is_some()
    }

    /// Get the primary BLS public key hex (if present)
    pub fn primary_bls_public_key(&self) -> Option<&str> {
        self.addresses
            .first()
            .and_then(|a| a.bls_public_key.as_deref())
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
    fn test_all_pubkey_hashes_returns_all_addresses() {
        let (mut wallet, _) = Wallet::new("test");
        wallet.generate_address(Some("secondary")).unwrap();

        let hashes = wallet.all_pubkey_hashes();
        assert_eq!(hashes.len(), 2, "should return both primary and secondary");
        assert_eq!(hashes[0].1, 0, "first entry should be index 0 (primary)");
        assert_eq!(hashes[1].1, 1, "second entry should be index 1 (secondary)");
        assert_ne!(
            hashes[0].0, hashes[1].0,
            "different addresses must have different hashes"
        );
    }

    #[test]
    fn test_keypair_for_pubkey_hash_finds_primary() {
        let (wallet, _) = Wallet::new("test");
        let primary_hash = wallet.primary_pubkey_hash();

        let kp = wallet.keypair_for_pubkey_hash(&primary_hash).unwrap();
        assert_eq!(kp.public_key().to_hex(), wallet.primary_public_key());
    }

    #[test]
    fn test_keypair_for_pubkey_hash_finds_secondary() {
        let (mut wallet, _) = Wallet::new("test");
        wallet.generate_address(Some("secondary")).unwrap();

        let hashes = wallet.all_pubkey_hashes();
        let secondary_hash = &hashes[1].0;

        // Must find the secondary key, not the primary
        let kp = wallet.keypair_for_pubkey_hash(secondary_hash).unwrap();
        assert_eq!(kp.public_key().to_hex(), wallet.addresses()[1].public_key);
        assert_ne!(kp.public_key().to_hex(), wallet.primary_public_key());
    }

    #[test]
    fn test_keypair_for_unknown_hash_fails() {
        let (wallet, _) = Wallet::new("test");
        let result = wallet.keypair_for_pubkey_hash(
            "0000000000000000000000000000000000000000000000000000000000000000",
        );
        assert!(result.is_err(), "should fail for unknown pubkey_hash");
    }

    #[test]
    fn test_all_pubkey_hashes_single_address() {
        let (wallet, _) = Wallet::new("test");
        let hashes = wallet.all_pubkey_hashes();
        assert_eq!(hashes.len(), 1);
        assert_eq!(hashes[0].0, wallet.primary_pubkey_hash());
    }

    #[test]
    fn test_secondary_address_not_derived_from_seed() {
        // This test documents the current behavior: secondary addresses are random,
        // NOT derived from the seed phrase. Restoring from seed will NOT recover them.
        let (mut wallet, phrase) = Wallet::new("test");
        wallet.generate_address(Some("secondary")).unwrap();
        let secondary_pubkey = wallet.addresses()[1].public_key.clone();

        // Restore from seed — only primary is recovered
        let restored = Wallet::from_seed_phrase("restored", &phrase).unwrap();
        assert_eq!(
            restored.addresses().len(),
            1,
            "restored wallet should only have primary"
        );
        assert_eq!(restored.primary_public_key(), wallet.primary_public_key());
        // Secondary is gone
        assert!(
            restored
                .addresses()
                .iter()
                .all(|a| a.public_key != secondary_pubkey),
            "secondary address must NOT be recoverable from seed"
        );
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

    #[test]
    fn test_restore_from_seed_phrase() {
        let (original, phrase) = Wallet::new("test");
        let restored = Wallet::from_seed_phrase("restored", &phrase).unwrap();
        assert_eq!(original.primary_public_key(), restored.primary_public_key());
    }

    #[test]
    fn test_restore_invalid_phrase() {
        let result = Wallet::from_seed_phrase("test", "not a valid seed phrase");
        assert!(result.is_err());
    }
}
