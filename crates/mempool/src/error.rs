//! Mempool admission errors.

use crypto::Hash;
use doli_core::validation::ValidationError;
use serde_json::Value;
use thiserror::Error;

/// Mempool errors
#[derive(Debug, Error)]
pub enum MempoolError {
    #[error("transaction already exists in mempool")]
    AlreadyExists,

    #[error("mempool is full")]
    Full,

    /// Transient: the live producer set could not be read; retry the same tx.
    #[error("producer set busy; retry later")]
    Busy,

    #[error("invalid transaction: {0}")]
    InvalidTransaction(String),

    #[error("validation failed: {0}")]
    Validation(#[from] ValidationError),

    #[error("fee too low: {0} < {1}")]
    FeeTooLow(u64, u64),

    #[error("transaction too large: {0} > {1}")]
    TooLarge(usize, usize),

    #[error("too many ancestors: {0} > {1}")]
    TooManyAncestors(usize, usize),

    #[error("too many descendants: {0} > {1}")]
    TooManyDescendants(usize, usize),

    #[error("missing input: {0}:{1}")]
    MissingInput(Hash, u32),

    #[error(
        "double spend: outpoint {tx_hash}:{output_index} already spent by mempool tx {spending_tx}"
    )]
    DoubleSpend {
        /// Transaction hash of the output being double-spent.
        tx_hash: crypto::Hash,
        /// Output index within that transaction.
        output_index: u32,
        /// Hash of the mempool transaction that already spends this output.
        spending_tx: crypto::Hash,
    },
}

impl MempoolError {
    /// Returns a stable, machine-readable error code.
    pub fn error_code(&self) -> &str {
        match self {
            Self::AlreadyExists => "TX_ALREADY_EXISTS",
            Self::Full => "MEMPOOL_FULL",
            Self::Busy => "MEMPOOL_BUSY",
            Self::InvalidTransaction(msg) => {
                // Extract [MPTX0XX] code if present
                if msg.starts_with("[MPTX") {
                    if let Some(end) = msg.find(']') {
                        return &msg[1..end];
                    }
                }
                "INVALID_TRANSACTION"
            }
            Self::Validation(e) => e.error_code(),
            Self::FeeTooLow(..) => "FEE_TOO_LOW",
            Self::TooLarge(..) => "TX_TOO_LARGE",
            Self::TooManyAncestors(..) => "TOO_MANY_ANCESTORS",
            Self::TooManyDescendants(..) => "TOO_MANY_DESCENDANTS",
            Self::MissingInput(..) => "MISSING_INPUT",
            Self::DoubleSpend { .. } => "DOUBLE_SPEND",
        }
    }

    /// Serializes this error to structured JSON for agentic consumption.
    ///
    /// Returns a JSON object with `error_code`, `message`, `stage`, and
    /// variant-specific fields. Agents can match on `error_code` programmatically.
    pub fn to_structured_json(&self) -> Value {
        let mut obj = serde_json::json!({
            "error_code": self.error_code(),
            "message": self.to_string(),
            "stage": "mempool",
        });

        let map = obj.as_object_mut().unwrap();

        match self {
            Self::AlreadyExists | Self::Full | Self::Busy => {}
            Self::InvalidTransaction(msg) => {
                map.insert("detail".into(), Value::String(msg.clone()));
            }
            Self::Validation(e) => {
                // Merge the full ValidationError JSON (which has all fields)
                let val_json = e.to_structured_json();
                if let Value::Object(val_map) = val_json {
                    for (k, v) in val_map {
                        if k != "message" {
                            map.insert(k, v);
                        }
                    }
                }
                map.insert("stage".into(), Value::String("mempool_validation".into()));
            }
            Self::FeeTooLow(actual, minimum) => {
                map.insert("actual_fee".into(), (*actual).into());
                map.insert("minimum_fee".into(), (*minimum).into());
            }
            Self::TooLarge(size, max) => {
                map.insert("size".into(), (*size).into());
                map.insert("max".into(), (*max).into());
            }
            Self::TooManyAncestors(count, max) => {
                map.insert("count".into(), (*count).into());
                map.insert("max".into(), (*max).into());
            }
            Self::TooManyDescendants(count, max) => {
                map.insert("count".into(), (*count).into());
                map.insert("max".into(), (*max).into());
            }
            Self::MissingInput(hash, index) => {
                map.insert("tx_hash".into(), Value::String(hash.to_hex()));
                map.insert("output_index".into(), (*index).into());
            }
            Self::DoubleSpend {
                tx_hash,
                output_index,
                spending_tx,
            } => {
                map.insert("tx_hash".into(), Value::String(tx_hash.to_hex()));
                map.insert("output_index".into(), (*output_index).into());
                map.insert("spending_tx".into(), Value::String(spending_tx.to_hex()));
            }
        }

        obj
    }
}
