//! UTXO set management
//!
//! Provides two backends:
//! - `InMemoryUtxoStore`: HashMap-based (original, used for migration and testing)
//! - `RocksDbUtxoStore`: Disk-backed via RocksDB (production, scales to millions of UTXOs)
//!
//! The `UtxoSet` enum dispatches to the active backend. Consumers don't need
//! to know which backend is active — all methods work identically.

mod in_memory;
mod set;
#[cfg(test)]
mod tests;
mod types;

// Re-export everything for identical public API
pub use in_memory::InMemoryUtxoStore;
pub use set::UtxoSet;
pub use types::reward_maturity_for_network;
#[allow(deprecated)]
pub use types::{Outpoint, UtxoEntry, DEFAULT_REWARD_MATURITY};
