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
//! - `guardian`: seed guardian (production halt, checkpoints, status)
//! - `fork_choice_version`: `getForkChoiceVersion` readiness probe (INC-I-204 M5)
//! - `fork_escape`: the audited `forceReorgTo` operator wedge escape (INC-I-204 M4.1)

mod backfill;
mod balance;
mod block;
mod context;
mod defi_health;
mod dispatch;
mod fork_choice_version;
mod fork_escape;
mod governance;
mod guardian;
mod history;
mod network;
mod oracle;
mod oracle_status;
mod pool;
mod producer;
mod pruning;
mod schedule;
mod snapshot;
mod stats;
mod transaction;

// Re-export public API (unchanged)
pub use context::{BackfillState, RpcContext, SyncStatus};
