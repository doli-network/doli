use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use tracing::info;

use super::DevnetConfig;

/// Get the devnet root directory
pub(super) fn devnet_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".doli")
        .join("devnet")
}

/// Read .env from the repo root directory at runtime.
pub(super) fn load_devnet_env() -> Result<String> {
    // Read from .env in current working directory (repo root)
    let env_file = std::env::current_dir()?.join(".env");

    if !env_file.exists() {
        return Err(anyhow!(
            "Missing .env in current directory: {:?}\n\
             Run this command from the repository root.\n\
             You can copy .env.example to .env to get started.",
            std::env::current_dir().unwrap_or_default()
        ));
    }

    let contents =
        fs::read_to_string(&env_file).with_context(|| format!("Failed to read {:?}", env_file))?;

    info!("Loaded .env from {:?}", env_file);
    Ok(contents)
}

/// Load devnet configuration
pub(super) fn load_config() -> Result<DevnetConfig> {
    let config_path = devnet_dir().join("devnet.toml");
    if !config_path.exists() {
        return Err(anyhow!(
            "Devnet not initialized. Run 'doli-node devnet init' first."
        ));
    }
    let contents = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read {:?}", config_path))?;
    toml::from_str(&contents).with_context(|| "Failed to parse devnet.toml")
}

/// Save devnet configuration
pub(super) fn save_config(config: &DevnetConfig) -> Result<()> {
    let config_path = devnet_dir().join("devnet.toml");
    let contents = toml::to_string_pretty(config)?;
    fs::write(&config_path, contents)?;
    Ok(())
}
