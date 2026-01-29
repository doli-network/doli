//! JSON-RPC error types

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Standard JSON-RPC error codes
pub mod codes {
    /// Parse error - Invalid JSON was received
    pub const PARSE_ERROR: i32 = -32700;
    /// Invalid Request - The JSON sent is not a valid Request object
    pub const INVALID_REQUEST: i32 = -32600;
    /// Method not found
    pub const METHOD_NOT_FOUND: i32 = -32601;
    /// Invalid params - Invalid method parameters
    pub const INVALID_PARAMS: i32 = -32602;
    /// Internal error
    pub const INTERNAL_ERROR: i32 = -32603;

    // Custom error codes (application-specific)
    /// Block not found
    pub const BLOCK_NOT_FOUND: i32 = -32000;
    /// Transaction not found
    pub const TX_NOT_FOUND: i32 = -32001;
    /// Invalid transaction
    pub const INVALID_TX: i32 = -32002;
    /// Transaction already in mempool
    pub const TX_ALREADY_KNOWN: i32 = -32003;
    /// Mempool full
    pub const MEMPOOL_FULL: i32 = -32004;
    /// UTXO not found
    pub const UTXO_NOT_FOUND: i32 = -32005;
    /// Producer not found
    pub const PRODUCER_NOT_FOUND: i32 = -32006;
}

/// RPC error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    /// Error code
    pub code: i32,
    /// Error message
    pub message: String,
    /// Additional data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    /// Create a new RPC error
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// Create an error with additional data
    pub fn with_data(code: i32, message: impl Into<String>, data: Value) -> Self {
        Self {
            code,
            message: message.into(),
            data: Some(data),
        }
    }

    /// Parse error
    pub fn parse_error() -> Self {
        Self::new(codes::PARSE_ERROR, "Parse error")
    }

    /// Invalid request
    pub fn invalid_request() -> Self {
        Self::new(codes::INVALID_REQUEST, "Invalid request")
    }

    /// Method not found
    pub fn method_not_found(method: &str) -> Self {
        Self::new(
            codes::METHOD_NOT_FOUND,
            format!("Method not found: {}", method),
        )
    }

    /// Invalid params
    pub fn invalid_params(msg: impl Into<String>) -> Self {
        Self::new(codes::INVALID_PARAMS, msg)
    }

    /// Internal error
    pub fn internal_error(msg: impl Into<String>) -> Self {
        Self::new(codes::INTERNAL_ERROR, msg)
    }

    /// Block not found
    pub fn block_not_found() -> Self {
        Self::new(codes::BLOCK_NOT_FOUND, "Block not found")
    }

    /// Transaction not found
    pub fn tx_not_found() -> Self {
        Self::new(codes::TX_NOT_FOUND, "Transaction not found")
    }

    /// Invalid transaction
    pub fn invalid_tx(reason: impl Into<String>) -> Self {
        Self::new(codes::INVALID_TX, reason)
    }

    /// Transaction already in mempool
    pub fn tx_already_known() -> Self {
        Self::new(codes::TX_ALREADY_KNOWN, "Transaction already in mempool")
    }

    /// Mempool full
    pub fn mempool_full() -> Self {
        Self::new(codes::MEMPOOL_FULL, "Mempool full")
    }

    /// UTXO not found
    pub fn utxo_not_found() -> Self {
        Self::new(codes::UTXO_NOT_FOUND, "UTXO not found")
    }

    /// Producer not found
    pub fn producer_not_found() -> Self {
        Self::new(codes::PRODUCER_NOT_FOUND, "Producer not found")
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RPC error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for RpcError {}
