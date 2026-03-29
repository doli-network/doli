//! # rpc
//!
//! JSON-RPC API server for DOLI nodes.
//!
//! This crate provides an HTTP-based JSON-RPC API for querying blockchain state,
//! submitting transactions, and monitoring network status.
//!
//! ## Endpoints
//!
//! All RPC methods are accessed via POST to `/` with a JSON-RPC 2.0 request body.
//!
//! ## Available Methods
//!
//! | Method | Description |
//! |--------|-------------|
//! | `getBlockByHash` | Get block by hash |
//! | `getBlockByHeight` | Get block by height |
//! | `getTransaction` | Get transaction by hash |
//! | `sendTransaction` | Submit signed transaction |
//! | `getBalance` | Get address balance |
//! | `getUtxos` | Get UTXOs for address |
//! | `getMempoolInfo` | Get mempool statistics |
//! | `getNetworkInfo` | Get network status |
//! | `getChainInfo` | Get chain tip info |
//! | `getEpochInfo` | Get current reward epoch info |
//! | `getProducer` | Get producer info |
//! | `getProducers` | Get all producers |
//! | `getHistory` | Get transaction history for address |
//! | `submitVote` | Submit governance vote |
//! | `getUpdateStatus` | Get auto-update status |
//! | `getMaintainerSet` | Get maintainer set |
//! | `submitMaintainerChange` | Submit maintainer change transaction |
//! | `getPeerInfo` | Get detailed peer list |
//! | `getNodeInfo` | Get node version info |
//! | `getNetworkParams` | Get network parameters |
//! | `getBondDetails` | Get per-bond vesting details |
//! | `getChainStats` | Get chain statistics |
//! | `getMempoolTransactions` | Get pending mempool transactions |
//! | `getSlotSchedule` | Get upcoming slot assignments |
//! | `getProducerSchedule` | Get producer schedule info |
//! | `getAttestationStats` | Get attestation statistics |
//! | `getBlockRaw` | Get raw block data |
//! | `backfillFromPeer` | Trigger block backfill |
//! | `backfillStatus` | Check backfill progress |
//! | `verifyChainIntegrity` | Verify chain completeness |
//! | `getStateRootDebug` | Debug state root hashes |
//! | `getUtxoDiff` | Diff UTXOs across nodes |
//!
//! | `getStateSnapshot` | Get full state snapshot |
//! | `getPoolInfo` | Get AMM pool details |
//! | `getPoolList` | List all AMM pools |
//! | `getPoolPrice` | Get pool spot price and TWAP |
//! | `getSwapQuote` | Simulate a swap |
//! | `getLoanInfo` | Get loan details |
//! | `getLoanList` | List active loans |
//! | `pauseProduction` | Pause block production (guardian) |
//! | `resumeProduction` | Resume block production (guardian) |
//! | `createCheckpoint` | Create RocksDB checkpoint (guardian) |
//! | `getGuardianStatus` | Get guardian system status |
//!
//! ## Example Request
//!
//! ```json
//! {
//!     "jsonrpc": "2.0",
//!     "method": "getBlockByHeight",
//!     "params": [0],
//!     "id": 1
//! }
//! ```

pub mod error;
pub mod methods;
pub mod server;
pub mod types;
pub mod ws;

pub use error::RpcError;
pub use methods::{RpcContext, SyncStatus};
pub use server::{RpcServer, RpcServerConfig};
pub use types::PeerInfoEntry;
pub use ws::WsEvent;

// Mempool is now a separate crate
// Re-export for backward compatibility
pub use mempool::{Mempool, MempoolEntry, MempoolError, MempoolPolicy};
