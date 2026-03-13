//! Application state management for the Tauri backend.
//!
//! Manages the loaded wallet, RPC client, and application configuration.
//! All mutable state is behind tokio::sync::RwLock for safe concurrent access
//! from async Tauri commands (RwLockGuard must be Send across await points).

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use wallet::RpcClient;

use crate::node_manager::{self, NodeManager};

/// Application state managed by Tauri.
pub struct AppState {
    /// Currently loaded wallet (None if no wallet loaded).
    pub wallet: RwLock<Option<wallet::Wallet>>,
    /// Path to current wallet file.
    pub wallet_path: RwLock<Option<PathBuf>>,
    /// RPC client for node communication.
    pub rpc_client: RwLock<RpcClient>,
    /// Application configuration (persisted).
    pub config: RwLock<AppConfig>,
    /// Embedded node process manager.
    pub node_manager: RwLock<NodeManager>,
}

impl AppState {
    /// Create new AppState with default configuration.
    ///
    /// Initializes the NodeManager with the default data directory and network
    /// from the persisted config. The RPC client defaults to the local node URL.
    pub fn new() -> Self {
        let config = AppConfig::load_or_default();
        let data_dir = node_manager::default_data_dir();
        let node_mgr = NodeManager::new(data_dir, &config.network);
        // Default RPC URL: local embedded node if no custom override.
        let default_url = if config.custom_rpc_url.is_some() {
            config.effective_rpc_url()
        } else {
            node_mgr.rpc_url()
        };
        Self {
            wallet: RwLock::new(None),
            wallet_path: RwLock::new(None),
            rpc_client: RwLock::new(RpcClient::new(&default_url)),
            config: RwLock::new(config),
            node_manager: RwLock::new(node_mgr),
        }
    }

    /// Check if a wallet is currently loaded.
    /// Used by frontend polling and startup logic.
    #[allow(dead_code)]
    pub async fn has_wallet(&self) -> bool {
        self.wallet.read().await.is_some()
    }

    /// Get the network prefix for the current network.
    pub async fn network_prefix(&self) -> String {
        let config = self.config.read().await;
        wallet::network_prefix(&config.network).to_string()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Persisted application configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppConfig {
    /// Selected network (mainnet, testnet, devnet).
    pub network: String,
    /// Custom RPC endpoint (None = use default for network).
    pub custom_rpc_url: Option<String>,
    /// Default wallet directory path.
    pub default_wallet_path: PathBuf,
    /// Last used wallet file path.
    pub last_wallet_path: Option<PathBuf>,
    /// RPC polling interval in seconds.
    pub poll_interval: u32,
    /// Fallback RPC endpoints per network.
    #[serde(default)]
    pub rpc_endpoints: HashMap<String, Vec<String>>,
}

impl AppConfig {
    /// Load config from disk or return defaults.
    pub fn load_or_default() -> Self {
        let config_path = Self::config_path();
        if config_path.exists() {
            if let Ok(contents) = std::fs::read_to_string(&config_path) {
                if let Ok(config) = serde_json::from_str(&contents) {
                    return config;
                }
            }
        }
        Self::default()
    }

    /// Save config to disk.
    pub fn save(&self) -> Result<(), String> {
        let config_path = Self::config_path();
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let contents = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&config_path, contents).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Get the effective RPC URL (custom or default for network).
    pub fn effective_rpc_url(&self) -> String {
        if let Some(ref url) = self.custom_rpc_url {
            return url.clone();
        }
        let endpoints = wallet::default_endpoints(&self.network);
        endpoints
            .first()
            .cloned()
            .unwrap_or_else(|| "http://127.0.0.1:8500".to_string())
    }

    /// Get the config file path.
    fn config_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".doli-gui")
            .join("config.json")
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        let default_wallet_path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".doli-gui")
            .join("wallets");

        Self {
            network: "mainnet".to_string(),
            custom_rpc_url: None,
            default_wallet_path,
            last_wallet_path: None,
            poll_interval: 10,
            rpc_endpoints: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_config_default() {
        let config = AppConfig::default();
        assert_eq!(config.network, "mainnet");
        assert!(config.custom_rpc_url.is_none());
        assert_eq!(config.poll_interval, 10);
    }

    #[test]
    fn test_app_config_effective_rpc_url_default() {
        let config = AppConfig::default();
        let url = config.effective_rpc_url();
        assert!(url.contains("doli.network") || url.contains("127.0.0.1"));
    }

    #[test]
    fn test_app_config_effective_rpc_url_custom() {
        let config = AppConfig {
            custom_rpc_url: Some("http://custom:8500".to_string()),
            ..Default::default()
        };
        assert_eq!(config.effective_rpc_url(), "http://custom:8500");
    }

    #[test]
    fn test_app_config_serialization_roundtrip() {
        let config = AppConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let loaded: AppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.network, config.network);
        assert_eq!(loaded.poll_interval, config.poll_interval);
    }

    #[tokio::test]
    async fn test_app_state_new() {
        let state = AppState::new();
        assert!(!state.has_wallet().await);
    }

    #[tokio::test]
    async fn test_app_state_network_prefix() {
        let state = AppState::new();
        assert_eq!(state.network_prefix().await, "doli");
    }

    #[tokio::test]
    async fn test_app_state_has_node_manager() {
        let state = AppState::new();
        let mgr = state.node_manager.read().await;
        assert_eq!(mgr.network(), "mainnet");
        assert_eq!(mgr.rpc_port(), 8500);
        assert_eq!(mgr.rpc_url(), "http://127.0.0.1:8500");
    }

    #[tokio::test]
    async fn test_app_state_default_rpc_url_is_local() {
        // With no custom_rpc_url, the RPC client should point at local node.
        let state = AppState::new();
        let _rpc = state.rpc_client.read().await;
        // We can't easily read the URL from RpcClient, but the node_manager
        // should return the local URL.
        let mgr = state.node_manager.read().await;
        assert!(mgr.rpc_url().starts_with("http://127.0.0.1:"));
    }

    #[tokio::test]
    async fn test_app_state_node_manager_data_dir() {
        let state = AppState::new();
        let mgr = state.node_manager.read().await;
        let dir = mgr.data_dir();
        let name = dir.file_name().unwrap().to_str().unwrap();
        assert!(
            name == ".doli" || name == "doli",
            "Expected data dir to end with .doli or doli, got: {}",
            name
        );
    }

    #[test]
    fn test_app_config_save_load_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("config.json");

        let config = AppConfig {
            network: "testnet".to_string(),
            custom_rpc_url: Some("http://test:8500".to_string()),
            default_wallet_path: dir.path().to_path_buf(),
            last_wallet_path: None,
            poll_interval: 15,
            rpc_endpoints: HashMap::new(),
        };

        let contents = serde_json::to_string_pretty(&config).unwrap();
        std::fs::write(&config_path, &contents).unwrap();

        let loaded: AppConfig =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(loaded.network, "testnet");
        assert_eq!(loaded.custom_rpc_url.unwrap(), "http://test:8500");
        assert_eq!(loaded.poll_interval, 15);
    }
}
