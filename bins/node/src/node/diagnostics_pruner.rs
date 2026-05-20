//! Diagnostic pruner task — periodically prunes old/excess events from
//! DiagnosticLedger. Reads DOLI_DIAG_RETENTION_DAYS and DOLI_DIAG_MAX_EVENTS
//! env vars. Respects cascade-origin pin (already implemented in M1's
//! DiagnosticLedger::prune()).

use std::sync::Arc;
use std::time::Duration;

use storage::diagnostic_ledger::DiagnosticLedger;
use tracing::{debug, info, warn};

/// Default retention period in days.
const DEFAULT_RETENTION_DAYS: u64 = 30;

/// Default maximum event count.
const DEFAULT_MAX_EVENTS: usize = 100_000;

/// Interval between pruning runs.
const PRUNE_INTERVAL: Duration = Duration::from_secs(60);

/// Run the diagnostic pruner task.
///
/// Every 60 seconds, reads env-configurable retention parameters and calls
/// `ledger.prune()`. On shutdown signal, exits cleanly without a final prune.
pub async fn run_pruner_task(
    ledger: Arc<DiagnosticLedger>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    info!("[DiagnosticPruner] started");

    let mut timer = tokio::time::interval(PRUNE_INTERVAL);
    // Consume the first immediate tick so the first prune fires after 60s.
    timer.tick().await;

    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    info!("[DiagnosticPruner] shutdown signal received, exiting");
                    return;
                }
            }
            _ = timer.tick() => {
                run_prune_cycle(&ledger);
            }
        }
    }
}

/// Execute a single prune cycle: read env config, call ledger.prune(), log result.
fn run_prune_cycle(ledger: &DiagnosticLedger) {
    let retention_days = std::env::var("DOLI_DIAG_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_RETENTION_DAYS);

    let max_events = std::env::var("DOLI_DIAG_MAX_EVENTS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_MAX_EVENTS);

    let retention_secs = retention_days * 86_400;

    match ledger.prune(retention_secs, max_events) {
        Ok(pruned) => {
            if pruned > 0 {
                info!(
                    "[DiagnosticPruner] pruned {} events (retention={}d, cap={})",
                    pruned, retention_days, max_events
                );
            } else {
                debug!(
                    "[DiagnosticPruner] no events pruned (retention={}d, cap={})",
                    retention_days, max_events
                );
            }
        }
        Err(e) => {
            warn!("[DiagnosticPruner] prune failed: {:?}", e);
        }
    }
}
