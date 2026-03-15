//! Transaction builder for constructing DOLI transactions without depending on `doli-core`.
//!
//! This module provides a standalone transaction builder that constructs raw transaction
//! bytes matching the canonical encoding used by `doli-core`. It duplicates ~200 lines
//! of serialization logic to avoid pulling in the entire `doli-core` crate (and thus
//! the VDF/GMP dependency chain).
//!
//! # Architecture Decision
//!
//! The wallet crate does NOT depend on `doli-core`. See `gui-architecture.md` Module 1
//! "Design Decision: No doli-core Dependency" for rationale.

mod builder;
mod fees;
mod types;

#[cfg(test)]
mod tests;

pub use builder::TxBuilder;
pub use fees::{calculate_registration_cost, calculate_withdrawal_net, vesting_penalty_pct};
pub use types::{TxInput, TxOutput, TxType};
