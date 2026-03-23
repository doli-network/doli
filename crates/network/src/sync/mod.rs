//! Blockchain synchronization
//!
//! This module handles chain synchronization with peers, including:
//! - Header-first download for efficient initial sync
//! - Parallel body download from multiple peers
//! - Chain reorganization handling
//! - Equivocation detection for slashing

mod bodies;
mod equivocation;
mod fork_recovery;
mod fork_sync;
mod headers;
mod manager;
mod reorg;

pub use bodies::BodyDownloader;
pub use equivocation::{EquivocationDetector, EquivocationProof};
pub use fork_recovery::CompletedRecovery;
pub use fork_sync::{ForkSync, ForkSyncResult, ProbeResult};
pub use headers::HeaderDownloader;
pub use manager::{
    ForkAction, ProductionAuthorization, RecoveryPhase, RecoveryReason, SyncConfig, SyncManager,
    SyncState, VerifiedSnapshot, MAX_CONSECUTIVE_RESYNCS,
};
pub use reorg::{ReorgHandler, ReorgResult};
