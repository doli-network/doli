//! Environment variable loader for network configuration
//!
//! Loads configuration from `.env` files in the data directory.
//! This should be called early in the application startup, before
//! any Network methods that depend on configurable parameters.

use std::path::Path;

use tracing::{debug, info, warn};

/// Load environment variables for a specific network
///
/// Loads from `{data_dir}/.env` if it exists.
/// This populates `std::env` with the values from the file,
/// which are then read by `NetworkParams::load()`.
///
/// # Arguments
///
/// * `network_name` - The network name (mainnet, testnet, devnet)
/// * `data_dir` - The data directory path (e.g., `~/.doli/mainnet`)
///
/// # Example
///
/// ```ignore
/// use doli_core::env_loader::load_env_for_network;
/// use std::path::PathBuf;
///
/// let data_dir = PathBuf::from("/home/user/.doli/devnet");
/// load_env_for_network("devnet", &data_dir);
/// ```
pub fn load_env_for_network(network_name: &str, data_dir: &Path) {
    let env_path = data_dir.join(".env");

    if env_path.exists() {
        match dotenvy::from_path(&env_path) {
            Ok(()) => {
                info!(
                    "Loaded environment from {:?} for {} network",
                    env_path, network_name
                );
            }
            Err(e) => {
                warn!("Failed to load environment from {:?}: {}", env_path, e);
            }
        }
    } else {
        debug!(
            "No .env file found at {:?}, using defaults for {} network",
            env_path, network_name
        );
    }
}

/// Get the default data directory for a network
///
/// Returns `~/.doli/{network_name}` or falls back to `./.doli/{network_name}`
/// if the home directory cannot be determined.
pub fn get_default_data_dir(network_name: &str) -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".doli")
        .join(network_name)
}

/// Initialize environment for a network using the default data directory
///
/// Convenience function that combines `get_default_data_dir` and `load_env_for_network`.
pub fn init_env_for_network(network_name: &str) {
    let data_dir = get_default_data_dir(network_name);
    load_env_for_network(network_name, &data_dir);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn test_load_env_for_network_no_file() {
        // Should not panic when .env file doesn't exist
        let temp_dir = TempDir::new().unwrap();
        load_env_for_network("testnet", temp_dir.path());
    }

    #[test]
    fn test_load_env_for_network_with_file() {
        let temp_dir = TempDir::new().unwrap();
        let env_path = temp_dir.path().join(".env");

        // Write a test .env file
        std::fs::write(&env_path, "DOLI_TEST_VAR=test_value\n").unwrap();

        // Clear any existing value
        std::env::remove_var("DOLI_TEST_VAR");

        // Load the env file
        load_env_for_network("testnet", temp_dir.path());

        // Verify the value was loaded
        assert_eq!(
            std::env::var("DOLI_TEST_VAR").ok(),
            Some("test_value".to_string())
        );

        // Clean up
        std::env::remove_var("DOLI_TEST_VAR");
    }

    #[test]
    fn test_get_default_data_dir() {
        let data_dir = get_default_data_dir("mainnet");
        assert!(data_dir.ends_with(".doli/mainnet"));
    }
}
