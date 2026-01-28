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

pub use error::RpcError;
pub use methods::{RpcContext, SyncStatus};
pub use server::{RpcServer, RpcServerConfig};

// Mempool is now a separate crate
// Re-export for backward compatibility
pub use mempool::{Mempool, MempoolEntry, MempoolError, MempoolPolicy};
