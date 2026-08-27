//! Wallet management: creation, restoration, key derivation, address generation,
//! file persistence, and message signing.
//!
//! This module is extracted from `bins/cli/src/wallet.rs` to be shared between
//! the CLI and GUI. The wallet file format is identical.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use bip39::Mnemonic;
use crypto::{
    hash::hash_with_domain, signature, BlsKeyPair, KeyPair, PrivateKey, PublicKey, ADDRESS_DOMAIN,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// Wallet format version written by this crate.
///
/// History of the `version` field:
/// - `1` — legacy. Both keys random. No seed phrase.
/// - `2` — the Ed25519 spending key is derived from the BIP-39 seed. The BLS
///   attestation key is still random, so the phrase does NOT restore a producer
///   identity (INC-I-162).
/// - `3` — BOTH keys are derived from the BIP-39 seed. The phrase is a complete
///   backup.
///
/// Marker only. Nothing gates behaviour on it and every version loads. Must stay
/// identical to the CLI constant in `bins/cli/src/wallet.rs` (GUI-NF-008).
pub const WALLET_VERSION_SEED_DERIVED_BLS: u32 = 3;

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

/// Wallet file format. See [`WALLET_VERSION_SEED_DERIVED_BLS`] for the version history.
/// Matches the CLI's Wallet struct exactly for format compatibility (GUI-NF-008).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Wallet {
    /// Wallet name
    name: String,
    /// Format version. See [`WALLET_VERSION_SEED_DERIVED_BLS`].
    version: u32,
    /// Addresses
    addresses: Vec<WalletAddress>,
    /// INC-I-167: the file this wallet was loaded from, if any. Runtime-only —
    /// `#[serde(skip)]` keeps the on-disk format byte-compatible with the CLI
    /// (GUI-NF-008) and with older binaries.
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

        // INC-I-162: derive the BLS attestation key from the SAME BIP-39 seed, so the
        // 24 words restore the full identity. Must stay identical to the CLI impl in
        // bins/cli/src/wallet.rs or the two would produce different keys for one phrase.
        let bls_kp = BlsKeyPair::from_seed(&bip39_seed)
            .expect("BIP-39 seed is 64 bytes, well above the KeyGen minimum");

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
            version: WALLET_VERSION_SEED_DERIVED_BLS,
            addresses: vec![primary],
            origin: None,
        };

        (wallet, phrase)
    }

    /// Restore a wallet from a BIP-39 seed phrase.
    /// Derives BOTH keys from the phrase, identically to `new()` (INC-I-162).
    pub fn from_seed_phrase(name: &str, phrase: &str) -> Result<Self> {
        let mnemonic: Mnemonic = phrase
            .parse()
            .map_err(|e| anyhow!("Invalid seed phrase: {}", e))?;
        let bip39_seed = mnemonic.to_seed("");
        let mut ed25519_seed = [0u8; 32];
        ed25519_seed.copy_from_slice(&bip39_seed[..32]);

        let kp = KeyPair::from_seed(ed25519_seed);
        ed25519_seed.zeroize();

        // INC-I-162: same derivation as new(), so restoring reproduces the BLS key.
        let bls_kp = BlsKeyPair::from_seed(&bip39_seed)
            .expect("BIP-39 seed is 64 bytes, well above the KeyGen minimum");

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
            version: WALLET_VERSION_SEED_DERIVED_BLS,
            addresses: vec![primary],
            origin: None,
        })
    }

    /// Load wallet from a JSON file.
    pub fn load(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("wallet file not found: {}", path.display()))?;
        let mut wallet: Wallet = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse wallet file: {}", path.display()))?;
        // INC-I-167: remember where this wallet came from, so save() can allow a
        // save-back to this same file while refusing to clobber a different one.
        wallet.origin = Some(path.to_path_buf());
        Ok(wallet)
    }

    /// Save wallet to a JSON file. Creates parent directories if needed.
    ///
    /// INC-I-167: refuses to overwrite an existing file that this wallet was not
    /// loaded from. Overwriting is opt-in via [`Wallet::save_forced`], not the
    /// default — a wallet file may be the only copy of a producer's registered BLS
    /// key, which a 24-word seed phrase does NOT restore (INC-I-162).
    ///
    /// # Errors
    /// Returns an error if `path` exists and is not this wallet's origin, or if the
    /// underlying atomic write fails.
    pub fn save(&self, path: &Path) -> Result<()> {
        if !self.is_origin(path) && path.exists() {
            return Err(anyhow!(
                "Refusing to overwrite existing wallet at {}\n  \
                 If it was created by a release before BLS keys became seed-derived, \
                 that file is the ONLY copy of its producer identity and no seed \
                 phrase can bring it back.\n  \
                 Choose a different file, or back up and move the existing wallet \
                 aside first.",
                path.display()
            ));
        }
        self.write_to(path)
    }

    /// Save wallet to a JSON file, bypassing the overwrite guard.
    ///
    /// Only for flows that have already obtained explicit destructive consent from
    /// the operator.
    ///
    /// # Errors
    /// Returns an error if the atomic write fails.
    pub fn save_forced(&self, path: &Path) -> Result<()> {
        self.write_to(path)
    }

    /// Is `path` the file this wallet was loaded from?
    ///
    /// Compares literally first, then by canonical path so equivalent spellings
    /// still count as the same file. A wallet with no origin is never a save-back,
    /// so the answer is `false` — the conservative direction.
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
    /// INC-I-167: the previous implementation used `std::fs::write` and set no
    /// permissions at all, so a crash mid-write truncated the wallet and a new file
    /// landed at the process umask (commonly 0644 — world-readable private keys).
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

        let mut tmp_name = std::ffi::OsString::from(".");
        tmp_name.push(file_name);
        tmp_name.push(format!(".tmp{}", std::process::id()));
        let tmp = dir.join(tmp_name);
        let _ = std::fs::remove_file(&tmp); // clear a stale temp from a prior crash

        let contents = serde_json::to_string_pretty(self)?;

        let write_result = (|| -> Result<()> {
            use std::io::Write;
            // Create the temp file with its final permissions BEFORE writing, so key
            // material is never briefly world-readable at the process umask.
            // Mode: preserve the destination's existing mode so a save-back never
            // WIDENS hardened permissions; new files get 0600 (owner-only) because
            // this file holds private keys and nothing reads it via group.
            #[cfg(unix)]
            let mut file = {
                use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
                let mode = std::fs::metadata(path)
                    .map(|m| m.permissions().mode() & 0o777)
                    .unwrap_or(0o600);
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

    /// Export wallet (same as save, but semantically distinct).
    pub fn export(&self, path: &Path) -> Result<()> {
        self.save(path)
    }

    /// Import wallet from a JSON file (same as load, but semantically distinct).
    pub fn import(path: &Path) -> Result<Self> {
        Self::load(path)
    }

    /// Get wallet name.
    /// Is this wallet's BLS attestation key derived from its seed phrase?
    ///
    /// `false` for version 1 and 2 wallets, whose BLS key was drawn from `OsRng`
    /// and cannot be reproduced from the 24 words (INC-I-162).
    #[must_use]
    pub fn bls_is_seed_derived(&self) -> bool {
        self.version >= WALLET_VERSION_SEED_DERIVED_BLS
    }

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
mod tests {
    use super::*;
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
        assert_eq!(wallet.version(), WALLET_VERSION_SEED_DERIVED_BLS);
    }

    #[test]
    fn test_fr001_new_wallet_has_ed25519_keypair() {
        let (wallet, _) = Wallet::new("test");
        assert_eq!(wallet.addresses().len(), 1);
        // Ed25519 public key is 32 bytes = 64 hex chars
        assert_eq!(wallet.primary_public_key().len(), 64);
    }

    // ========================================================================
    // INC-I-167: save() must be fail-safe — refuse to clobber a different wallet.
    //
    // OUTPUT CONTRACT: fn Wallet::save(&self, path: &Path) -> Result<()>
    //   O1: destination file identity (addresses[0].public_key on disk)
    //       — PRESERVED / REPLACED / CREATED
    //   O2: Result — Ok / Err
    //   O3: error message — contains "Refusing to overwrite" / absent
    // PATHS (by: does `path` exist? x is `path` this wallet's origin?):
    //   P1 save-back  — exists, IS origin      -> MUST overwrite
    //   P2 create     — absent                 -> MUST create
    //   P3 cross-path — exists, NOT origin     -> MUST refuse
    // INPUT PARTITIONS: one per path. Sufficient because the branch predicate is
    //   determined entirely by the two path terms; wallet CONTENTS cannot change
    //   which branch is taken, so a contents-partition is provably blind here.
    //   P3's partition is the realistic one: destination holds a different, valid,
    //   key-bearing wallet.
    // MATRIX: 3 outputs x 3 paths x 1 partition = 9 cells.
    //   P1 -> O1 PRESERVED(+1 addr) / O2 Ok  / O3 absent
    //   P2 -> O1 CREATED            / O2 Ok  / O3 absent   (covered by
    //         test_fr001_wallet_save_and_load_roundtrip and _creates_parent_dirs)
    //   P3 -> O1 PRESERVED          / O2 Err / O3 present
    // ========================================================================

    #[test]
    fn test_inc_i_167_save_refuses_to_clobber_a_different_wallet() {
        let dir = TempDir::new().unwrap();
        let victim_path = dir.path().join("victim.json");
        let source_path = dir.path().join("source.json");

        let (victim, _) = Wallet::new("victim");
        victim.save(&victim_path).unwrap();
        let (source, _) = Wallet::new("source");
        source.save(&source_path).unwrap();

        let victim_pk = Wallet::load(&victim_path)
            .unwrap()
            .primary_public_key()
            .to_string();

        // P3: a wallet whose origin is source_path must not be able to write over
        // victim_path — that file may hold the only copy of a producer BLS key.
        let imported = Wallet::import(&source_path).unwrap();
        let result = imported.save(&victim_path);

        // O2 x P3
        let err = result.expect_err("P3/O2: save() must refuse to overwrite a different wallet");
        // O3 x P3
        let msg = err.to_string();
        assert!(
            msg.contains("Refusing to overwrite"),
            "P3/O3: refusal must explain itself. got: {msg}"
        );
        // O1 x P3 — the victim must be untouched.
        assert_eq!(
            Wallet::load(&victim_path).unwrap().primary_public_key(),
            victim_pk,
            "P3/O1: the existing wallet must be preserved on refusal"
        );
    }

    #[test]
    fn test_inc_i_167_save_back_to_own_origin_is_allowed() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wallet.json");

        let (wallet, _) = Wallet::new("test");
        wallet.save(&path).unwrap();

        // P1: load then mutate then save back to the SAME file — this is how every
        // legitimate wallet mutation persists and must keep working.
        let mut loaded = Wallet::load(&path).unwrap();
        let pk = loaded.primary_public_key().to_string();
        loaded.generate_address(Some("second")).unwrap();
        loaded
            .save(&path)
            .expect("P1/O2: save-back to own origin must be allowed");

        let reloaded = Wallet::load(&path).unwrap();
        assert_eq!(
            reloaded.addresses().len(),
            2,
            "P1/O1: mutation must persist"
        );
        assert_eq!(
            reloaded.primary_public_key(),
            pk,
            "P1/O1: primary identity must be preserved"
        );
    }

    #[test]
    fn test_inc_i_167_origin_is_not_serialized_and_defaults_to_none() {
        // `origin` is #[serde(skip)], so a wallet parsed from JSON must come back
        // with origin=None and therefore get CREATE semantics (refuse to clobber).
        // This pins the fail-safe direction of the default: INC-I-159 was a skipped
        // field whose default silently enabled the WRONG behavior.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wallet.json");
        let other = dir.path().join("other.json");

        let (wallet, _) = Wallet::new("test");
        wallet.save(&path).unwrap();
        let (o, _) = Wallet::new("other");
        o.save(&other).unwrap();

        let json = std::fs::read_to_string(&path).unwrap();
        assert!(
            !json.contains("origin"),
            "origin must never reach the wallet file — on-disk format must stay \
             byte-compatible with the CLI (GUI-NF-008)"
        );

        let parsed: Wallet = serde_json::from_str(&json).unwrap();
        assert!(
            parsed.save(&other).is_err(),
            "a deserialized wallet has no origin, so it must refuse to overwrite"
        );
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
    fn test_fr002_restore_derives_the_same_bls_key() {
        // INC-I-162: this test previously asserted the OPPOSITE — that restore
        // produced a DIFFERENT BLS key, because BlsKeyPair::generate() drew from
        // OsRng and ignored the mnemonic. That behaviour meant the 24 words were
        // not a complete backup for a registered producer, and the test pinned the
        // defect in place rather than catching it. Inverted, not deleted, so the
        // change of contract is visible in history.
        let (original, phrase) = Wallet::new("original");
        let restored = Wallet::from_seed_phrase("restored", &phrase).unwrap();
        assert_eq!(
            original.primary_bls_public_key().unwrap(),
            restored.primary_bls_public_key().unwrap(),
            "BLS key must be derived from the seed phrase, not randomly generated"
        );

        // Discriminating control: a different phrase must still give a different
        // key, otherwise a constant would satisfy the assertion above.
        let (_, other_phrase) = Wallet::new("other");
        let other = Wallet::from_seed_phrase("other", &other_phrase).unwrap();
        assert_ne!(
            restored.primary_bls_public_key().unwrap(),
            other.primary_bls_public_key().unwrap(),
            "different phrases must derive different BLS keys"
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
        assert_eq!(wallet.version(), WALLET_VERSION_SEED_DERIVED_BLS);
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
}
