// Activation heights are 0 (all features active from genesis). The height >= 0
// comparisons are structurally needed for the activation gate pattern but are
// always true on this chain. Suppress clippy warnings.
#![allow(clippy::absurd_extreme_comparisons)]

//! DOLI Node library — exposes internals for integration testing.
//!
//! The binary (main.rs) has its own module tree for the actual node.
//! This lib re-declares the modules needed by integration tests.

pub mod config;
pub mod node;
pub mod producer;
pub mod updater;
