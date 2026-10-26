//! Shared wallet library for DOLI GUI and CLI.
//!
//! This crate provides wallet management, JSON-RPC client, and transaction building
//! for the DOLI blockchain. It is shared between the CLI (`bins/cli`) and GUI (`bins/gui`)
//! to guarantee wallet file format compatibility (GUI-NF-008).
//!
//! # Architecture
//!
//! `TxBuilder` encodes transfer-style transactions itself, in `doli-core`'s byte
//! format. Producer transactions go through the shared `doli-producer-tx`
//! builders, so the wallet, the GUI and the CLI emit the same signed bytes.
//!
//! # Modules
//!
//! - `wallet` -- Wallet creation, restoration, key derivation, address management
//! - `rpc_client` -- Async JSON-RPC client for DOLI node communication
//! - `tx_builder` -- Transaction construction and signing
//! - `types` -- Shared response types (Balance, UTXO, ChainInfo, etc.)

pub mod rpc_client;
pub mod tx_builder;
pub mod types;
pub mod wallet;

// Re-exports for convenience
pub use rpc_client::{default_endpoints, network_prefix, RpcClient};
pub use tx_builder::{
    calculate_registration_cost, calculate_withdrawal_net, vesting_penalty_pct, ProducerSigner,
    TxBuilder, TxType, WithdrawalRequest,
};
pub use types::*;
pub use wallet::{verify_message, Wallet, WalletAddress};
