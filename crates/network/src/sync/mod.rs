//! Blockchain synchronization
//!
//! This module handles chain synchronization with peers, including:
//! - Header-first download for efficient initial sync
//! - Parallel body download from multiple peers
//! - Chain reorganization handling
//! - Equivocation detection for slashing

mod bodies;
pub mod chunk_sink;
mod equivocation;
mod fork_recovery;
mod headers;
mod manager;
mod reorg;

#[cfg(test)]
mod adversarial_tests;
#[cfg(test)]
mod tests_inc_i236_m2_intake;
#[cfg(test)]
mod tests_inc_i236_m5_headers;
#[cfg(test)]
mod tests_inc_i236_run_a_resume;

pub use bodies::BodyDownloader;
pub use chunk_sink::{StagedUtxoMarker, UtxoChunkSink};
pub use equivocation::{EquivocationDetector, EquivocationProof, MAX_EVIDENCE_CHECKS_AFTER_PROOF};
pub use fork_recovery::CompletedRecovery;
pub use headers::HeaderDownloader;
pub use manager::{
    ChainBreakInfo, ForceReorgPoll, NetworkEvidence, ProductionAuthorization, RecoveryAction,
    RecoveryContext, RecoveryEvidence, RecoveryPhase, RecoveryReason, StuckForkAlert, SyncConfig,
    SyncManager, SyncPhase, SyncPipelineData, SyncState, VerifiedSnapshot, WedgeReason,
    FORCE_REORG_MAX_HEIGHT_SPAN, FORCE_REORG_TTL_SECS, MAX_CONSECUTIVE_RESYNCS,
};
pub use reorg::{ForkChoiceFinality, ReorgHandler, ReorgObservations, ReorgResult, WeightVerdict};
