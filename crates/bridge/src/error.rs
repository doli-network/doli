//! Bridge watcher error types.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("DOLI RPC error: {0}")]
    DoliRpc(String),

    #[error("Bitcoin RPC error: {0}")]
    BitcoinRpc(String),

    #[error("Ethereum RPC error: {0}")]
    EthereumRpc(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Swap not found: {0}")]
    SwapNotFound(String),

    #[error("Invalid preimage: {0}")]
    InvalidPreimage(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("Swap expired: {0}")]
    SwapExpired(String),

    #[error("Chain mismatch: expected {expected}, got {got}")]
    ChainMismatch { expected: String, got: String },
}

pub type Result<T> = std::result::Result<T, BridgeError>;
