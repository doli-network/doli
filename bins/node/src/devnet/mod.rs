//! Local devnet management for development and testing.
//!
//! This module provides CLI commands to manage a local multi-node devnet:
//! - `devnet init --nodes N` - Initialize devnet with N producers
//! - `devnet start` - Start all nodes
//! - `devnet stop` - Stop all nodes
//! - `devnet status` - Show chain status
//! - `devnet clean [--keep-keys]` - Remove devnet data

use serde::{Deserialize, Serialize};

mod add_producer;
mod clean;
mod config;
mod init;
mod process;
mod start;
mod status;
mod stop;

pub use add_producer::add_producer;
pub use clean::clean;
pub use init::init;
pub use start::start;
pub use status::status;
pub use stop::stop;

/// Devnet configuration stored in devnet.toml
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DevnetConfig {
    /// Number of nodes in the devnet
    pub node_count: u32,
    /// Base P2P port (nodes use base + node_index)
    pub base_p2p_port: u16,
    /// Base RPC port (nodes use base + node_index)
    pub base_rpc_port: u16,
    /// Base metrics port (nodes use base + node_index)
    pub base_metrics_port: u16,
}

impl Default for DevnetConfig {
    fn default() -> Self {
        Self {
            node_count: 3,
            base_p2p_port: 50300,
            base_rpc_port: 28500,
            base_metrics_port: 29000,
        }
    }
}

impl DevnetConfig {
    /// Get P2P port for a specific node (0-indexed)
    pub fn p2p_port(&self, node_index: u32) -> u16 {
        self.base_p2p_port + node_index as u16
    }

    /// Get RPC port for a specific node (0-indexed)
    pub fn rpc_port(&self, node_index: u32) -> u16 {
        self.base_rpc_port + node_index as u16
    }

    /// Get metrics port for a specific node (0-indexed)
    pub fn metrics_port(&self, node_index: u32) -> u16 {
        self.base_metrics_port + node_index as u16
    }
}

/// Wallet file format (compatible with doli-cli)
#[derive(Clone, Debug, Serialize, Deserialize)]
struct WalletAddress {
    address: String,
    public_key: String,
    private_key: String,
    label: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Wallet {
    name: String,
    version: u32,
    addresses: Vec<WalletAddress>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_devnet_config_ports() {
        let config = DevnetConfig::default();
        assert_eq!(config.p2p_port(0), 50300);
        assert_eq!(config.p2p_port(1), 50301);
        assert_eq!(config.rpc_port(0), 28500);
        assert_eq!(config.rpc_port(1), 28501);
        assert_eq!(config.metrics_port(0), 29000);
        assert_eq!(config.metrics_port(1), 29001);
    }

    #[test]
    fn test_next_producer_index() {
        use std::fs;

        let tmp = std::env::temp_dir().join("doli_test_next_idx");
        let keys_dir = tmp.join("keys");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&keys_dir).unwrap();

        // Empty directory → index 0
        assert_eq!(add_producer::next_producer_index(&tmp), 0);

        // Add producer_0.json → next is 1
        fs::write(keys_dir.join("producer_0.json"), "{}").unwrap();
        assert_eq!(add_producer::next_producer_index(&tmp), 1);

        // Add producer_2.json (gap) → next is 3
        fs::write(keys_dir.join("producer_2.json"), "{}").unwrap();
        assert_eq!(add_producer::next_producer_index(&tmp), 3);

        let _ = fs::remove_dir_all(&tmp);
    }
}
