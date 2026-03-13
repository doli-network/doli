//! Channel error types.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChannelError {
    #[error("invalid state transition: {from} -> {to}")]
    InvalidTransition { from: String, to: String },

    #[error("channel not found: {0}")]
    NotFound(String),

    #[error("insufficient balance: need {need}, have {have}")]
    InsufficientBalance { need: u64, have: u64 },

    #[error("invalid revocation preimage")]
    InvalidRevocation,

    #[error("condition encoding error: {0}")]
    Condition(#[from] doli_core::ConditionError),

    #[error("invalid signature")]
    InvalidSignature,

    #[error("funding output not confirmed")]
    FundingUnconfirmed,

    #[error("channel already closed")]
    AlreadyClosed,

    #[error("HTLC not found: {0}")]
    HtlcNotFound(String),

    #[error("HTLC expired")]
    HtlcExpired,

    #[error("capacity mismatch: expected {expected}, got {actual}")]
    CapacityMismatch { expected: u64, actual: u64 },

    #[error("reserve violation: balance {balance} below reserve {reserve}")]
    ReserveViolation { balance: u64, reserve: u64 },

    #[error("dispute window active: {blocks_remaining} blocks remaining")]
    DisputeWindowActive { blocks_remaining: u64 },

    #[error("RPC error: {0}")]
    Rpc(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("configuration error: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, ChannelError>;
