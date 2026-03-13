//! Bridge watcher for cross-chain atomic swaps.
//!
//! Monitors DOLI and Bitcoin chains for HTLC activity,
//! tracking swap state and auto-claiming when preimages are revealed.

pub mod bitcoin;
pub mod config;
pub mod doli;
pub mod error;
pub mod store;
pub mod swap;
pub mod watcher;

pub use config::WatcherConfig;
pub use error::{BridgeError, Result};
pub use swap::{SwapRecord, SwapRole, SwapState};
pub use watcher::BridgeWatcher;
