//! Diagnostic emitter trait and implementations.
//!
//! The emitter is the hot-path interface for recording diagnostic events.
//! Production code calls `emitter.record(event)` which is non-blocking.
//!
//! ## Implementations
//!
//! - `NoOpEmitter` — drops events silently (graceful degradation / tests).
//! - `MockEmitter` — captures events in a `Mutex<Vec>` (unit tests).
//! - `AsyncChannelEmitter` — bounded ring-buffer backed by `Mutex<VecDeque>`.
//!   On buffer full: oldest event evicted, `dropped_count` incremented.
//!   The `new()` method returns `(emitter, receiver)` where the receiver
//!   drains from the same shared buffer. This avoids the tokio mpsc
//!   single-consumer ownership constraint while preserving drop-oldest
//!   semantics. The `DiagnosticReceiver` wrapper provides a `try_recv()`
//!   interface compatible with test expectations.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::types::DiagnosticEvent;

/// Error type for emitter operations.
#[derive(Debug)]
pub struct EmitError(pub String);

impl std::fmt::Display for EmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "emit error: {}", self.0)
    }
}

impl std::error::Error for EmitError {}

/// Trait for recording diagnostic events from the hot path.
///
/// Implementations must be `Send + Sync` so they can be shared via
/// `Arc<dyn DiagnosticEmitter>` across the node's async tasks.
pub trait DiagnosticEmitter: Send + Sync {
    /// Record a diagnostic event. Must be non-blocking in production.
    fn record(&self, event: DiagnosticEvent) -> Result<(), EmitError>;
}

// ---------------------------------------------------------------------------
// NoOpEmitter
// ---------------------------------------------------------------------------

/// Emitter that silently drops all events. Used for graceful degradation
/// when the diagnostic ledger is unavailable, and in tests that don't
/// need event capture.
pub struct NoOpEmitter;

impl DiagnosticEmitter for NoOpEmitter {
    /// Always returns `Ok(())` without side effects.
    fn record(&self, _event: DiagnosticEvent) -> Result<(), EmitError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MockEmitter
// ---------------------------------------------------------------------------

/// Emitter that captures events in memory for test assertions.
pub struct MockEmitter {
    events: Mutex<Vec<DiagnosticEvent>>,
}

impl Default for MockEmitter {
    fn default() -> Self {
        Self::new()
    }
}

impl MockEmitter {
    /// Create a new empty mock emitter.
    pub fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }

    /// Return a snapshot of all captured events.
    pub fn events(&self) -> Vec<DiagnosticEvent> {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl DiagnosticEmitter for MockEmitter {
    /// Appends the event to the internal Vec.
    fn record(&self, event: DiagnosticEvent) -> Result<(), EmitError> {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(event);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// DiagnosticReceiver — shared-buffer consumer for AsyncChannelEmitter
// ---------------------------------------------------------------------------

/// Error returned when the receiver buffer is empty.
#[derive(Debug)]
pub struct TryRecvError;

/// Consumer side of the `AsyncChannelEmitter` ring buffer.
///
/// Provides a `try_recv()` method that pops events from the shared buffer
/// in FIFO order (oldest first).
pub struct DiagnosticReceiver {
    buf: Arc<Mutex<VecDeque<DiagnosticEvent>>>,
}

impl DiagnosticReceiver {
    /// Try to receive the next (oldest) event from the buffer.
    ///
    /// Returns `Ok(event)` if an event is available, or `Err(TryRecvError)`
    /// if the buffer is currently empty.
    pub fn try_recv(&mut self) -> Result<DiagnosticEvent, TryRecvError> {
        let mut buf = self.buf.lock().unwrap_or_else(|e| e.into_inner());
        buf.pop_front().ok_or(TryRecvError)
    }
}

// ---------------------------------------------------------------------------
// AsyncChannelEmitter
// ---------------------------------------------------------------------------

/// Non-blocking emitter backed by a bounded ring buffer (`Mutex<VecDeque>`).
///
/// When the buffer is full, the oldest event is evicted to make room for the
/// new one (drop-oldest policy). The `dropped_count` `AtomicU64` tracks the
/// total number of evicted events.
///
/// The returned `DiagnosticReceiver` shares the same underlying buffer and
/// can drain events for forwarding to the `DiagnosticLedger`.
pub struct AsyncChannelEmitter {
    buf: Arc<Mutex<VecDeque<DiagnosticEvent>>>,
    cap: usize,
    dropped: AtomicU64,
}

impl AsyncChannelEmitter {
    /// Create a new emitter with the given logical capacity.
    ///
    /// Returns `(emitter, receiver)`. The receiver shares the same buffer
    /// and can drain events via `try_recv()`.
    pub fn new(capacity: usize) -> (Self, DiagnosticReceiver) {
        let buf = Arc::new(Mutex::new(VecDeque::with_capacity(capacity)));
        let emitter = Self {
            buf: buf.clone(),
            cap: capacity,
            dropped: AtomicU64::new(0),
        };
        let receiver = DiagnosticReceiver { buf };
        (emitter, receiver)
    }

    /// Return the total number of events dropped due to buffer overflow.
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl DiagnosticEmitter for AsyncChannelEmitter {
    /// Push an event into the ring buffer. If full, evict the oldest event.
    fn record(&self, event: DiagnosticEvent) -> Result<(), EmitError> {
        let mut buf = self.buf.lock().unwrap_or_else(|e| e.into_inner());
        if buf.len() >= self.cap {
            buf.pop_front(); // evict oldest
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        buf.push_back(event);
        Ok(())
    }
}
