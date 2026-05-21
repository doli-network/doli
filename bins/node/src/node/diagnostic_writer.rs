//! Diagnostic writer task — drains events from AsyncChannelEmitter receiver
//! and writes them to DiagnosticLedger (RocksDB).
//!
//! Shutdown semantics: when the shutdown watch fires, the task drains all
//! remaining queued events before exiting. Emits periodic WriterHeartbeat
//! events so the RPC (M3) can detect silent write failures (FM-4b).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use storage::diagnostic_ledger::emitter::DiagnosticReceiver;
use storage::diagnostic_ledger::types::{DiagnosticEvent, EventKind, EventPayload};
use storage::diagnostic_ledger::{DiagnosticLedger, DiagnosticWriterStats};
use tracing::{debug, info, warn};

/// Maximum events to drain per batch before yielding.
const BATCH_SIZE: usize = 16;

/// Interval between heartbeat writes.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

/// Interval between drain attempts when the channel is empty.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Run the diagnostic writer task.
///
/// Drains events from `receiver` and writes them to `ledger`. On shutdown
/// signal, drains all remaining events before returning. Emits a
/// `WriterHeartbeat` directly to the ledger every 60 seconds.
///
/// Counters are tracked on the shared `stats` handle so the RPC layer
/// can report live values via `getDiagnosticHealth` (INC-I-087).
pub async fn run_writer_task(
    mut receiver: DiagnosticReceiver,
    ledger: Arc<DiagnosticLedger>,
    stats: Arc<DiagnosticWriterStats>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    info!("[DiagnosticWriter] started");

    let mut heartbeat_timer = tokio::time::interval(HEARTBEAT_INTERVAL);
    // Consume the first immediate tick so the first heartbeat fires after 60s.
    heartbeat_timer.tick().await;

    loop {
        tokio::select! {
            biased;
            // Shutdown signal takes priority.
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    debug!("[DiagnosticWriter] shutdown signal received, draining remaining events");
                    drain_remaining(&mut receiver, &ledger, &stats);
                    info!(
                        "[DiagnosticWriter] shutdown complete (written={}, dropped={})",
                        stats.events_written.load(Ordering::Relaxed),
                        stats.events_dropped.load(Ordering::Relaxed),
                    );
                    return;
                }
            }
            // Heartbeat timer fires every 60 seconds.
            _ = heartbeat_timer.tick() => {
                write_heartbeat(&ledger, &stats);
            }
            // Poll interval — drain a batch of events.
            _ = tokio::time::sleep(POLL_INTERVAL) => {
                drain_batch(&mut receiver, &ledger, &stats);
            }
        }
    }
}

/// Drain up to `BATCH_SIZE` events from the receiver and write them to the ledger.
fn drain_batch(
    receiver: &mut DiagnosticReceiver,
    ledger: &DiagnosticLedger,
    stats: &DiagnosticWriterStats,
) {
    for _ in 0..BATCH_SIZE {
        match receiver.try_recv() {
            Ok(event) => {
                if let Err(e) = ledger.record(&event) {
                    warn!("[DiagnosticWriter] failed to write event: {:?}", e);
                } else {
                    stats.events_written.fetch_add(1, Ordering::Relaxed);
                }
            }
            Err(_) => break, // channel empty
        }
    }
}

/// Drain ALL remaining events from the receiver (used during shutdown).
fn drain_remaining(
    receiver: &mut DiagnosticReceiver,
    ledger: &DiagnosticLedger,
    stats: &DiagnosticWriterStats,
) {
    while let Ok(event) = receiver.try_recv() {
        if let Err(e) = ledger.record(&event) {
            warn!(
                "[DiagnosticWriter] failed to write event during shutdown drain: {:?}",
                e
            );
        } else {
            stats.events_written.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Write a WriterHeartbeat event directly to the ledger (not via channel).
fn write_heartbeat(ledger: &DiagnosticLedger, stats: &DiagnosticWriterStats) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    // Update the shared heartbeat timestamp so the RPC can report it.
    stats.last_heartbeat_ms.store(now_ms, Ordering::Relaxed);

    let event = DiagnosticEvent {
        event_id: ulid::Ulid::new().to_string(),
        kind: EventKind::WriterHeartbeat,
        timestamp_ms: now_ms,
        height: None,
        correlation_key: None,
        caused_by_event_id: None,
        is_cascade_origin: false,
        payload: EventPayload::WriterHeartbeat {
            events_written_total: stats.events_written.load(Ordering::Relaxed),
            events_dropped_total: stats.events_dropped.load(Ordering::Relaxed),
        },
    };

    if let Err(e) = ledger.record(&event) {
        warn!("[DiagnosticWriter] failed to write heartbeat: {:?}", e);
    } else {
        debug!("[DiagnosticWriter] heartbeat written");
    }
}
