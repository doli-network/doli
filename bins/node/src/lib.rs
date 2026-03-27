//! DOLI Node library — exposes internals for integration testing.
//!
//! The binary (main.rs) has its own module tree for the actual node.
//! This lib re-declares the modules needed by integration tests.

pub mod config;
pub mod node;
pub mod producer;
pub mod updater;
