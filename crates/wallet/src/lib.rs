//! Shared wallet library for DOLI GUI and CLI.
//!
//! This crate provides wallet management, JSON-RPC client, and transaction building
//! for the DOLI blockchain. It is shared between the CLI (`bins/cli`) and GUI (`bins/gui`)
//! to guarantee wallet file format compatibility (GUI-NF-008).
//!
//! # Architecture
//!
//! This crate does NOT depend on `doli-core` to avoid the VDF/GMP dependency chain.
//! Transaction construction happens via a standalone `TxBuilder` that produces canonical
//! byte encodings matching `doli-core`'s serialization format.
//!
//! # Modules
//!
//! - `wallet` -- Wallet creation, restoration, key derivation, address management
//! - `rpc_client` -- Async JSON-RPC client for DOLI node communication
//! - `tx_builder` -- Transaction construction without `doli-core` dependency
//! - `types` -- Shared response types (Balance, UTXO, ChainInfo, etc.)

pub mod rpc_client;
pub mod tx_builder;
pub mod types;
pub mod wallet;

// Re-exports for convenience
pub use rpc_client::{default_endpoints, network_prefix, RpcClient};
pub use tx_builder::{
    calculate_registration_cost, calculate_withdrawal_net, vesting_penalty_pct, TxBuilder, TxType,
};
pub use types::*;
pub use wallet::{verify_message, Wallet, WalletAddress};
