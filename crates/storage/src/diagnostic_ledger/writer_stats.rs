//! Shared atomic counters for the diagnostic writer task.
//!
//! `DiagnosticWriterStats` is constructed once and shared (via `Arc`) between
//! the writer task and the RPC context, so `getDiagnosticHealth` can report
//! live values instead of hardcoded zeros.

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

/// Live counters exposed by the diagnostic writer task.
///
/// All fields use `AtomicU64` with `Ordering::Relaxed` — exact snapshot
/// consistency is not required for health reporting.
///
/// `last_heartbeat_ms` uses `0` to represent "no heartbeat yet" because
/// `Option<u64>` is not atomic. The RPC handler maps `0 -> None`.
pub struct DiagnosticWriterStats {
    /// Total events successfully written since node start.
    pub events_written: AtomicU64,
    /// Total events dropped (channel overflow) since node start.
    pub events_dropped: AtomicU64,
    /// Epoch-millis timestamp of the last writer heartbeat (0 = none yet).
    pub last_heartbeat_ms: AtomicU64,
}

impl DiagnosticWriterStats {
    /// Create a new stats handle with all counters at zero.
    pub fn new_shared() -> Arc<Self> {
        Arc::new(Self {
            events_written: AtomicU64::new(0),
            events_dropped: AtomicU64::new(0),
            last_heartbeat_ms: AtomicU64::new(0),
        })
    }
}
