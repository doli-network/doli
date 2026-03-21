use std::path::PathBuf;

/// Resolve the base data directory for a given network.
/// Priority: flag > env > platform default > legacy fallback
pub fn resolve_base_dir(network: &str, flag_override: Option<&str>) -> PathBuf {
    // 1. Explicit flag
    if let Some(dir) = flag_override {
        return PathBuf::from(dir);
    }

    // 2. DOLI_DATA_DIR env var
    if let Ok(dir) = std::env::var("DOLI_DATA_DIR") {
        return PathBuf::from(dir);
    }

    // 3. Platform default
    let platform_default = platform_data_dir(network);
    if platform_default.exists() {
        return platform_default;
    }

    // 4. Legacy fallback (~/.doli/{network}/)
    if let Some(home) = dirs::home_dir() {
        let legacy = home.join(".doli").join(network);
        if legacy.exists() {
            eprintln!(
                "Note: Using legacy data directory {}. Run 'doli migrate' to move to {}.",
                legacy.display(),
                platform_default.display()
            );
            return legacy;
        }
    }

    // 5. Return platform default (even if it doesn't exist yet)
    platform_default
}

/// Resolve wallet path.
/// Priority: -w flag > DOLI_WALLET_FILE env > {base_dir}/wallet.json
pub fn resolve_wallet_path(
    network: &str,
    wallet_flag: Option<&str>,
    data_dir_flag: Option<&str>,
) -> PathBuf {
    if let Some(w) = wallet_flag {
        return PathBuf::from(w);
    }
    if let Ok(w) = std::env::var("DOLI_WALLET_FILE") {
        return PathBuf::from(w);
    }
    resolve_base_dir(network, data_dir_flag).join("wallet.json")
}

/// Platform-specific data directory
fn platform_data_dir(network: &str) -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        PathBuf::from("/var/lib/doli").join(network)
    }
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("Library/Application Support/doli")
            .join(network)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".doli")
            .join(network)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_base_dir_with_flag_override() {
        let result = resolve_base_dir("mainnet", Some("/custom/path"));
        assert_eq!(result, PathBuf::from("/custom/path"));
    }

    #[test]
    fn test_resolve_wallet_path_with_flag() {
        let result = resolve_wallet_path("mainnet", Some("/custom/wallet.json"), None);
        assert_eq!(result, PathBuf::from("/custom/wallet.json"));
    }

    #[test]
    fn test_resolve_wallet_path_default_ends_with_wallet_json() {
        // Clear env to avoid interference
        std::env::remove_var("DOLI_WALLET_FILE");
        std::env::remove_var("DOLI_DATA_DIR");
        let result = resolve_wallet_path("mainnet", None, Some("/tmp/test-doli"));
        assert_eq!(result, PathBuf::from("/tmp/test-doli/wallet.json"));
    }

    #[test]
    fn test_platform_data_dir_not_empty() {
        let dir = platform_data_dir("mainnet");
        assert!(dir.to_str().unwrap().contains("mainnet"));
    }
}
