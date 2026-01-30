//! Chain specification for network genesis configuration
//!
//! This module implements industry-standard chain specification (chainspec) support.
//! Instead of hardcoding genesis parameters in source code, networks are configured
//! via external JSON files that can be audited, versioned, and verified.
//!
//! # Industry Standard
//!
//! This follows the pattern used by:
//! - Ethereum: genesis.json
//! - Cosmos: genesis.json
//! - Polkadot/Substrate: chain_spec.json
//! - Solana: genesis config
//!
//! # Usage
//!
//! ```ignore
//! // Load chainspec from file
//! let spec = ChainSpec::load("mainnet.json")?;
//!
//! // Or use built-in specs
//! let spec = ChainSpec::mainnet();
//! let spec = ChainSpec::testnet();
//! let spec = ChainSpec::devnet();
//! ```

use crate::network::Network;
use crypto::PublicKey;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Chain specification defining all genesis parameters
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChainSpec {
    /// Human-readable chain name
    pub name: String,

    /// Chain identifier (mainnet, testnet, devnet)
    pub id: String,

    /// Network type
    pub network: Network,

    /// Genesis configuration
    pub genesis: GenesisSpec,

    /// Consensus parameters
    pub consensus: ConsensusSpec,

    /// Initial producers (optional - if empty, uses bootstrap mode)
    #[serde(default)]
    pub genesis_producers: Vec<GenesisProducer>,

    /// Maintainer public keys for auto-update system (3-of-5 threshold)
    #[serde(default)]
    pub maintainer_keys: Vec<String>,
}

/// Genesis block specification
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenesisSpec {
    /// Genesis timestamp (Unix seconds)
    /// Use 0 for dynamic (current time at first block)
    pub timestamp: u64,

    /// Genesis message embedded in coinbase
    pub message: String,

    /// Initial block reward in atomic units (1 DOLI = 100_000_000)
    pub initial_reward: u64,
}

/// Consensus parameters
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsensusSpec {
    /// Slot duration in seconds
    pub slot_duration: u64,

    /// Slots per epoch
    pub slots_per_epoch: u32,

    /// Bond amount required for producer registration (atomic units)
    pub bond_amount: u64,
}

/// Genesis producer specification
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenesisProducer {
    /// Producer name/identifier (for documentation)
    pub name: String,

    /// Ed25519 public key (hex-encoded, 64 characters)
    pub public_key: String,

    /// Number of bonds (typically 1)
    #[serde(default = "default_bond_count")]
    pub bond_count: u32,
}

fn default_bond_count() -> u32 {
    1
}

impl ChainSpec {
    /// Load chainspec from a JSON file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ChainSpecError> {
        let contents = std::fs::read_to_string(path.as_ref())
            .map_err(|e: std::io::Error| ChainSpecError::IoError(e.to_string()))?;

        let spec: ChainSpec = serde_json::from_str(&contents)
            .map_err(|e: serde_json::Error| ChainSpecError::ParseError(e.to_string()))?;

        spec.validate()?;
        Ok(spec)
    }

    /// Save chainspec to a JSON file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), ChainSpecError> {
        let contents = serde_json::to_string_pretty(self)
            .map_err(|e: serde_json::Error| ChainSpecError::SerializeError(e.to_string()))?;

        std::fs::write(path, contents)
            .map_err(|e: std::io::Error| ChainSpecError::IoError(e.to_string()))?;

        Ok(())
    }

    /// Validate the chainspec
    pub fn validate(&self) -> Result<(), ChainSpecError> {
        // Validate genesis producers have valid pubkeys
        for producer in &self.genesis_producers {
            if producer.public_key.len() != 64 {
                return Err(ChainSpecError::InvalidPubkey(format!(
                    "Producer '{}' has invalid pubkey length: {} (expected 64 hex chars)",
                    producer.name,
                    producer.public_key.len()
                )));
            }

            // Verify it's valid hex
            if hex::decode(&producer.public_key).is_err() {
                return Err(ChainSpecError::InvalidPubkey(format!(
                    "Producer '{}' has invalid hex pubkey",
                    producer.name
                )));
            }

            // Check for placeholder keys
            if producer.public_key.starts_with("00000000") {
                return Err(ChainSpecError::PlaceholderKey(format!(
                    "Producer '{}' has placeholder pubkey - replace with real key!",
                    producer.name
                )));
            }
        }

        // Validate maintainer keys
        for (i, key) in self.maintainer_keys.iter().enumerate() {
            if key.len() != 64 {
                return Err(ChainSpecError::InvalidPubkey(format!(
                    "Maintainer key {} has invalid length",
                    i + 1
                )));
            }
            if key.starts_with("00000000") {
                return Err(ChainSpecError::PlaceholderKey(format!(
                    "Maintainer key {} is a placeholder - replace with real key!",
                    i + 1
                )));
            }
        }

        // Validate consensus params
        if self.consensus.slot_duration == 0 {
            return Err(ChainSpecError::InvalidParam("slot_duration cannot be 0".into()));
        }
        if self.consensus.slots_per_epoch == 0 {
            return Err(ChainSpecError::InvalidParam("slots_per_epoch cannot be 0".into()));
        }

        Ok(())
    }

    /// Get genesis producers as (PublicKey, bond_count) pairs
    pub fn get_genesis_producers(&self) -> Vec<(PublicKey, u32)> {
        self.genesis_producers
            .iter()
            .filter_map(|p| {
                let bytes = hex::decode(&p.public_key).ok()?;
                if bytes.len() != 32 {
                    return None;
                }
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                Some((PublicKey::from_bytes(arr), p.bond_count))
            })
            .collect()
    }

    /// Check if this spec has genesis producers configured
    pub fn has_genesis_producers(&self) -> bool {
        !self.genesis_producers.is_empty()
    }

    /// Built-in mainnet specification
    ///
    /// Note: For production, load from external file instead!
    pub fn mainnet() -> Self {
        Self {
            name: "DOLI Mainnet".into(),
            id: "mainnet".into(),
            network: Network::Mainnet,
            genesis: GenesisSpec {
                timestamp: 1769904000, // 2026-02-01T00:00:00Z
                message: "Time is the only fair currency. 01/Feb/2026".into(),
                initial_reward: 100_000_000, // 1 DOLI
            },
            consensus: ConsensusSpec {
                slot_duration: 10,
                slots_per_epoch: 360,
                bond_amount: 100_000_000_000, // 1000 DOLI
            },
            genesis_producers: vec![], // Load from file for production!
            maintainer_keys: vec![],   // Load from file for production!
        }
    }

    /// Built-in testnet specification
    pub fn testnet() -> Self {
        Self {
            name: "DOLI Testnet".into(),
            id: "testnet".into(),
            network: Network::Testnet,
            genesis: GenesisSpec {
                timestamp: 1769738400, // 2026-01-29T22:00:00Z
                message: "DOLI Testnet v2 Genesis - Time is the only fair currency".into(),
                initial_reward: 100_000_000, // 1 DOLI
            },
            consensus: ConsensusSpec {
                slot_duration: 10,
                slots_per_epoch: 360,
                bond_amount: 100_000_000_000, // 1000 DOLI
            },
            genesis_producers: vec![], // Will be loaded from chainspec file
            maintainer_keys: vec![],
        }
    }

    /// Built-in devnet specification (for local development)
    pub fn devnet() -> Self {
        Self {
            name: "DOLI Devnet".into(),
            id: "devnet".into(),
            network: Network::Devnet,
            genesis: GenesisSpec {
                timestamp: 0, // Dynamic - uses current time
                message: "DOLI Devnet - Development and Testing".into(),
                initial_reward: 100_000_000, // 1 DOLI
            },
            consensus: ConsensusSpec {
                slot_duration: 5, // Faster for testing
                slots_per_epoch: 60,
                bond_amount: 100_000_000, // 1 DOLI (easier testing)
            },
            genesis_producers: vec![], // Uses bootstrap mode
            maintainer_keys: vec![],
        }
    }
}

/// Errors that can occur with chainspec operations
#[derive(Debug, Clone)]
pub enum ChainSpecError {
    IoError(String),
    ParseError(String),
    SerializeError(String),
    InvalidPubkey(String),
    PlaceholderKey(String),
    InvalidParam(String),
}

impl std::fmt::Display for ChainSpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IoError(e) => write!(f, "IO error: {}", e),
            Self::ParseError(e) => write!(f, "Parse error: {}", e),
            Self::SerializeError(e) => write!(f, "Serialize error: {}", e),
            Self::InvalidPubkey(e) => write!(f, "Invalid public key: {}", e),
            Self::PlaceholderKey(e) => write!(f, "Placeholder key detected: {}", e),
            Self::InvalidParam(e) => write!(f, "Invalid parameter: {}", e),
        }
    }
}

impl std::error::Error for ChainSpecError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mainnet_spec() {
        let spec = ChainSpec::mainnet();
        assert_eq!(spec.id, "mainnet");
        assert_eq!(spec.genesis.initial_reward, 100_000_000);
        assert_eq!(spec.consensus.slot_duration, 10);
    }

    #[test]
    fn test_testnet_spec() {
        let spec = ChainSpec::testnet();
        assert_eq!(spec.id, "testnet");
        assert_eq!(spec.genesis.timestamp, 1769738400);
    }

    #[test]
    fn test_devnet_spec() {
        let spec = ChainSpec::devnet();
        assert_eq!(spec.id, "devnet");
        assert_eq!(spec.genesis.timestamp, 0); // Dynamic
        assert_eq!(spec.consensus.slot_duration, 5); // Faster
    }

    #[test]
    fn test_serialize_deserialize() {
        let spec = ChainSpec::testnet();
        let json = serde_json::to_string_pretty(&spec).unwrap();
        let parsed: ChainSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec.id, parsed.id);
    }

    #[test]
    fn test_validate_placeholder_key() {
        let mut spec = ChainSpec::testnet();
        spec.genesis_producers.push(GenesisProducer {
            name: "test".into(),
            public_key: "0000000000000000000000000000000000000000000000000000000000000001".into(),
            bond_count: 1,
        });
        assert!(matches!(
            spec.validate(),
            Err(ChainSpecError::PlaceholderKey(_))
        ));
    }

    #[test]
    fn test_validate_invalid_pubkey_length() {
        let mut spec = ChainSpec::testnet();
        spec.genesis_producers.push(GenesisProducer {
            name: "test".into(),
            public_key: "abc123".into(), // Too short
            bond_count: 1,
        });
        assert!(matches!(
            spec.validate(),
            Err(ChainSpecError::InvalidPubkey(_))
        ));
    }
}
