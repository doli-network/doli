//! Network definitions for DOLI
//!
//! Defines the different networks (mainnet, testnet, devnet) with their
//! unique identifiers, parameters, and security boundaries.
//!
//! ## Environment Configuration
//!
//! Network parameters can be configured via environment variables or `.env` files
//! in the data directory (`~/.doli/{network}/.env`). See [`crate::network_params`]
//! for the full list of configurable parameters.
//!
//! Security-critical parameters (VDF, emission, timing) are locked for mainnet.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::network_params::NetworkParams;

mod economics;
#[cfg(test)]
mod tests;
mod timing;
mod updates;
mod vdf;

/// Network identifier
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
pub enum Network {
    /// Production network - real money
    #[default]
    Mainnet = 1,
    /// Public test network - valueless coins for testing
    Testnet = 2,
    /// Local development network - fast blocks for development
    Devnet = 99,
}

impl Network {
    /// Get the numeric network ID
    pub fn id(&self) -> u32 {
        *self as u32
    }

    /// Get the network parameters (loaded from environment)
    ///
    /// Parameters are loaded once from environment variables and cached.
    /// For mainnet, security-critical parameters are locked to hardcoded values.
    pub fn params(&self) -> &'static NetworkParams {
        NetworkParams::load(*self)
    }

    /// Get the network name
    pub fn name(&self) -> &'static str {
        match self {
            Network::Mainnet => "mainnet",
            Network::Testnet => "testnet",
            Network::Devnet => "devnet",
        }
    }

    /// Get the address prefix for this network
    /// Prevents sending coins to wrong network
    pub fn address_prefix(&self) -> &'static str {
        match self {
            Network::Mainnet => "doli",
            Network::Testnet => "tdoli",
            Network::Devnet => "ddoli",
        }
    }

    /// Get magic bytes for P2P protocol
    /// Used to identify network at protocol level
    pub fn magic_bytes(&self) -> [u8; 4] {
        match self {
            Network::Mainnet => [0xD0, 0x11, 0x00, 0x01], // "DOLI" mainnet
            Network::Testnet => [0xD0, 0x11, 0x00, 0x02], // "DOLI" testnet
            Network::Devnet => [0xD0, 0x11, 0x00, 0x63],  // "DOLI" devnet (0x63 = 99)
        }
    }

    /// Get default P2P port for this network
    ///
    /// Configurable via `DOLI_P2P_PORT` environment variable.
    pub fn default_p2p_port(&self) -> u16 {
        self.params().default_p2p_port
    }

    /// Get default RPC port for this network
    ///
    /// Configurable via `DOLI_RPC_PORT` environment variable.
    pub fn default_rpc_port(&self) -> u16 {
        self.params().default_rpc_port
    }

    /// Get default metrics port for this network
    ///
    /// Configurable via `DOLI_METRICS_PORT` environment variable.
    pub fn default_metrics_port(&self) -> u16 {
        self.params().default_metrics_port
    }

    /// Get default data directory suffix
    pub fn data_dir_name(&self) -> &'static str {
        match self {
            Network::Mainnet => "mainnet",
            Network::Testnet => "testnet",
            Network::Devnet => "devnet",
        }
    }

    /// Check if this is a test network (testnet or devnet)
    pub fn is_test(&self) -> bool {
        matches!(self, Network::Testnet | Network::Devnet)
    }

    /// Get all available networks
    pub fn all() -> &'static [Network] {
        &[Network::Mainnet, Network::Testnet, Network::Devnet]
    }

    /// Parse from network ID
    pub fn from_id(id: u32) -> Option<Network> {
        match id {
            1 => Some(Network::Mainnet),
            2 => Some(Network::Testnet),
            99 => Some(Network::Devnet),
            _ => None,
        }
    }
}

impl fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl FromStr for Network {
    type Err = NetworkParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "mainnet" | "main" => Ok(Network::Mainnet),
            "testnet" | "test" => Ok(Network::Testnet),
            "devnet" | "dev" | "local" => Ok(Network::Devnet),
            _ => Err(NetworkParseError(s.to_string())),
        }
    }
}

/// Error when parsing network name
#[derive(Debug, Clone)]
pub struct NetworkParseError(String);

impl fmt::Display for NetworkParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown network '{}'. Valid options: mainnet, testnet, devnet",
            self.0
        )
    }
}

impl std::error::Error for NetworkParseError {}
