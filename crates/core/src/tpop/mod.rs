//! # Temporal Proof of Presence (TPoP) - Telemetry Module
//!
//! **IMPORTANT: This module is TELEMETRY ONLY and does NOT affect consensus.**
//!
//! Producer selection is determined by `consensus::select_producer_for_slot()` using
//! bond-based round-robin. Block VDF (T_BLOCK = 800K iterations, ~55ms) provides anti-grinding.
//!
//! This module provides network health monitoring via heartbeat proofs.
//!
//! ## Modules
//!
//! - [`heartbeat`] - Micro-VDF presence proofs (1 second)
//! - [`calibration`] - Dynamic VDF calibration for consistent timing

pub mod calibration;
pub mod heartbeat;

// Re-export calibration types (VDF tuning)
#[allow(deprecated)]
pub use calibration::{
    CalibrationStats, VdfCalibrator, DEFAULT_VDF_ITERATIONS, MAX_VDF_ITERATIONS,
    MIN_VDF_ITERATIONS, TARGET_VDF_TIME_MS,
};

// Re-export heartbeat types (primary API)
#[allow(deprecated)]
pub use heartbeat::{
    calculate_heartbeat_score, validate_heartbeat_timing, HeartbeatCollector, HeartbeatError,
    PresenceHeartbeat, HEARTBEAT_DEADLINE_SECS, HEARTBEAT_DISCRIMINANT_BITS,
    HEARTBEAT_GRACE_PERIOD_SECS, HEARTBEAT_VDF_ITERATIONS,
};
