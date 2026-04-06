//! Bridge library for cross-chain atomic swaps.
//!
//! Provides chain clients (Bitcoin, Ethereum, DOLI), swap types,
//! and a watcher daemon for automatic claim/refund of bridge HTLCs.

pub mod bitcoin;
pub mod doli;
pub mod error;
pub mod ethereum;
pub mod swap;
pub mod watcher;

pub use error::{BridgeError, Result};
pub use swap::{SwapRecord, SwapRole, SwapState};
pub use watcher::{Watcher, WatcherConfig};
