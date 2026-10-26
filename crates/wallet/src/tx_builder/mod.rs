//! Transaction builder for DOLI transactions.
//!
//! Transfer-style transactions are encoded by `TxBuilder` itself. Producer
//! transactions (AddBond, RequestWithdrawal) are built and signed by the shared
//! `doli-producer-tx` builders, the same code the CLI uses.

mod builder;
mod fees;
mod types;

#[cfg(test)]
mod tests;

pub use builder::TxBuilder;
pub use fees::{calculate_registration_cost, calculate_withdrawal_net, vesting_penalty_pct};
pub use types::{TxInput, TxOutput, TxType};

use crate::types::Utxo;
use crypto::KeyPair;

/// The key that signs a producer tx and the chain it is bound to.
pub struct ProducerSigner<'a> {
    pub keypair: &'a KeyPair,
    /// `getChainInfo.genesis_hash` (64 hex chars).
    pub genesis_hash: &'a str,
}

/// A RequestWithdrawal as the caller resolved it from RPC.
pub struct WithdrawalRequest<'a> {
    /// Network name from the node (`mainnet`, `testnet`, `devnet`).
    pub network: &'a str,
    pub best_height: u64,
    pub bond_count: u32,
    /// Bond UTXOs to spend, all of them, in this order.
    pub bond_utxos: &'a [Utxo],
    pub total_net: u64,
    pub fee_utxos: &'a [Utxo],
    pub fee: u64,
    /// `None` pays the producer's own address.
    pub destination: Option<[u8; 32]>,
}
