//! Transaction types and operations

mod core;
mod data;
mod input;
mod output;
mod producer_auth;
mod rotate_bls;
mod slash_canonical;
mod types;

pub mod legacy;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_delegation_auth;

#[cfg(test)]
mod tests_price_attestation;

#[cfg(test)]
mod tests_oracle_output;

#[cfg(test)]
mod wire_golden_tests;

#[cfg(test)]
mod input_module_tests;

// Re-export everything for API compatibility
pub use self::core::*;
pub use data::*;
pub use input::*;
pub use output::*;
pub use producer_auth::*;
pub use rotate_bls::*;
pub use types::*;
