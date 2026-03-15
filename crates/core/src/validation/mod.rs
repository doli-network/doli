//! Validation rules for transactions and blocks.
//!
//! This module provides comprehensive validation for DOLI transactions and blocks.
//! Validation is split into two levels:
//!
//! ## Structural Validation (Context-Free)
//!
//! These checks can be performed without access to the UTXO set:
//! - Version and format validity
//! - Non-empty inputs/outputs
//! - Positive output amounts
//! - Amount overflow protection
//! - Registration data validity
//!
//! ## Contextual Validation (Requires UTXO Set)
//!
//! These checks require access to the UTXO set and are performed via the
//! `UtxoValidator` trait:
//! - Signature verification
//! - Input existence and spendability
//! - Input/output balance
//! - Lock time enforcement

mod block;
mod error;
mod producer;
mod registration;
mod rewards_legacy;
#[cfg(test)]
mod tests;
mod transaction;
mod tx_types;
mod types;
mod utxo;

pub use block::{validate_block, validate_block_with_mode, validate_header};
pub use error::ValidationError;
pub use producer::{
    bootstrap_fallback_order, bootstrap_schedule_with_liveness, validate_producer_eligibility,
};
#[allow(deprecated)]
pub use rewards_legacy::{
    calculate_expected_epoch_rewards, epoch_needing_rewards, validate_block_rewards_exact,
};
#[cfg(test)]
use rewards_legacy::{validate_block_rewards, validate_coinbase};
pub use transaction::validate_transaction;
pub use types::{
    EpochBlockSource, RegistrationChainState, UtxoInfo, UtxoProvider, ValidationContext,
    ValidationMode,
};
pub use utxo::validate_transaction_with_utxos;
