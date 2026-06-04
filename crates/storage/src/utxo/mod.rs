//! UTXO set management
//!
//! Provides two backends:
//! - `InMemoryUtxoStore`: HashMap-based (used for testing and snap sync deserialization)
//! - `RocksDb(Arc<StateDb>)`: state_db-backed (production, sole UTXO store since Phase 4)
//!
//! The `UtxoSet` enum dispatches to the active backend. Consumers don't need
//! to know which backend is active — all methods work identically.

mod in_memory;
mod set;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_oracle_snapsync;
mod types;

// Re-export everything for identical public API
pub use in_memory::InMemoryUtxoStore;
pub use set::UtxoSet;
pub use types::reward_maturity_for_network;
pub use types::{uid_key, UID_PREFIX_ASSET, UID_PREFIX_CHANNEL, UID_PREFIX_NFT, UID_PREFIX_POOL};
#[allow(deprecated)]
pub use types::{Outpoint, UtxoEntry, DEFAULT_REWARD_MATURITY};
