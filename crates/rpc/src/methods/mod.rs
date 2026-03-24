//! RPC method handlers
//!
//! Split into domain modules:
//! - `context`: RpcContext struct, constructors, builder methods
//! - `dispatch`: handle_request routing
//! - `block`: block queries (getBlockByHash, getBlockByHeight, getBlockRaw)
//! - `transaction`: transaction queries and submission
//! - `balance`: balance and UTXO queries
//! - `network`: network/chain/node/epoch/params info
//! - `producer`: producer queries and bond details
//! - `history`: transaction history
//! - `governance`: voting, maintainer set, maintainer changes
//! - `backfill`: backfill from peer, integrity verification
//! - `stats`: chain stats, debug endpoints, mempool transactions
//! - `schedule`: slot and producer scheduling, attestation stats

mod backfill;
mod balance;
mod block;
mod context;
mod dispatch;
mod governance;
mod history;
mod network;
mod pool;
mod producer;
mod schedule;
mod snapshot;
mod stats;
mod transaction;

// Re-export public API (unchanged)
pub use context::{BackfillState, RpcContext, SyncStatus};
