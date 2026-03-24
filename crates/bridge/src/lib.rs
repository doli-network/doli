//! Bridge library for cross-chain atomic swaps.
//!
//! Provides chain clients (Bitcoin, Ethereum, DOLI) and swap types
//! for use by CLI bridge commands. No daemon, no state — stateless library.

pub mod bitcoin;
pub mod doli;
pub mod error;
pub mod ethereum;
pub mod swap;

pub use error::{BridgeError, Result};
pub use swap::{SwapRecord, SwapRole, SwapState};
