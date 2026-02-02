//! Blockchain synchronization
//!
//! This module handles chain synchronization with peers, including:
//! - Header-first download for efficient initial sync
//! - Parallel body download from multiple peers
//! - Chain reorganization handling
//! - Equivocation detection for slashing

mod bodies;
mod equivocation;
mod headers;
mod manager;
mod reorg;

pub use bodies::BodyDownloader;
pub use equivocation::{EquivocationDetector, EquivocationProof};
pub use headers::HeaderDownloader;
pub use manager::{ProductionAuthorization, SyncConfig, SyncManager, SyncState};
pub use reorg::{ReorgHandler, ReorgResult};
