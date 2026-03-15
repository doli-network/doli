use std::path::PathBuf;
use std::sync::OnceLock;

/// Address prefix resolved from --network flag at startup.
pub(crate) static ADDRESS_PREFIX: OnceLock<String> = OnceLock::new();

pub(crate) fn address_prefix() -> &'static str {
    ADDRESS_PREFIX.get().map(|s| s.as_str()).unwrap_or("doli")
}

pub(crate) fn prefix_for_network(network: &str) -> &'static str {
    match network {
        "testnet" => "tdoli",
        "devnet" => "ddoli",
        _ => "doli",
    }
}

pub(crate) fn default_rpc_for_network(network: &str) -> &'static str {
    match network {
        "testnet" => "http://127.0.0.1:18500",
        "devnet" => "http://127.0.0.1:28500",
        _ => "http://127.0.0.1:8500",
    }
}

/// Expand `~` or `~/...` to the user's home directory.
/// Shell tilde expansion doesn't happen inside Rust — clap default values
/// like `~/.doli/wallet.json` arrive as literal strings.
pub(crate) fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(rest)
    } else if path == "~" {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
    } else {
        PathBuf::from(path)
    }
}
