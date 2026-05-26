//! Fork-diagnostic RPC handler — `getForkDiagnostic`.
//!
//! Returns a `DiagnosticBundle` JSON object containing events, summary,
//! classification, baseline, and health data. Read-only (REQ-FORKOBS-SEC-002).

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use storage::diagnostic_ledger::classifier::classify;
use storage::diagnostic_ledger::types::{
    BaselineComparison, DiagnosticBundle, DiagnosticHealth, EventKind, EventPayload, ForkSummary,
};

use crate::error::RpcError;

use super::context::RpcContext;

/// Maximum events the RPC layer will return (REQ-FORKOBS-SEC-003).
const RPC_MAX_LIMIT: usize = 10_000;

/// Fork-relevant event kinds for summary statistics.
const FORK_RELEVANT_KINDS: &[EventKind] = &[
    EventKind::ForkBlockReceived,
    EventKind::BlockRejected,
    EventKind::RollbackStarted,
    EventKind::ReorgExecuted,
    EventKind::RecoveryClassifyCall,
    EventKind::SnapSyncFailed,
];

impl RpcContext {
    /// Handle `getForkDiagnostic` RPC method.
    ///
    /// Params (all optional):
    ///   - `window_secs`: u64 (default 3600) — time window for recent events
    ///   - `fork_event_id`: String — if provided, return causal chain instead of recent
    ///   - `limit`: u64 (capped at 10,000)
    ///   - `kind`: String — filter by event kind name
    ///   - `min_height`, `max_height`: u64 — height range filter
    ///
    /// When `min_height` or `max_height` is provided, events are filtered by
    /// height range via `query_range` instead of the default time-window scan.
    /// This restores forensic reach for historical incidents beyond the 10k-event
    /// cap (INC-I-090 D7).
    pub async fn get_fork_diagnostic(&self, params: Value) -> Result<Value, RpcError> {
        let ledger = self
            .diagnostic_ledger
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("Diagnostic ledger unavailable".to_string()))?;

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        // Parse params
        let window_secs = params
            .get("window_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(3600);

        let fork_event_id = params
            .get("fork_event_id")
            .and_then(|v| v.as_str())
            .map(String::from);

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|l| (l as usize).min(RPC_MAX_LIMIT))
            .unwrap_or(RPC_MAX_LIMIT);

        // INC-I-090 D7: parse height-range filter params
        let min_height = params.get("min_height").and_then(|v| v.as_u64());
        let max_height = params.get("max_height").and_then(|v| v.as_u64());

        // Fetch events
        let events = if let Some(ref event_id) = fork_event_id {
            ledger
                .query_causal_chain(event_id, limit)
                .map_err(|e| RpcError::internal_error(e.to_string()))?
        } else if min_height.is_some() || max_height.is_some() {
            // Height-range query: use query_range for efficient prefix scan.
            // Default min=0 and max=u64::MAX when only one bound is provided.
            let lo = min_height.unwrap_or(0);
            let hi = max_height.unwrap_or(u64::MAX);
            ledger
                .query_range(None, lo, hi, limit)
                .map_err(|e| RpcError::internal_error(e.to_string()))?
        } else {
            ledger
                .query_recent(window_secs, limit)
                .map_err(|e| RpcError::internal_error(e.to_string()))?
        };

        // Build fork summary
        let fork_summary = build_fork_summary(&events);

        // Classify
        let classification = Some(classify(&events));

        // Build baseline (simplified: compare current window vs 24h)
        let baseline = build_baseline(ledger.as_ref(), window_secs, &events);

        // Build health from live writer stats (INC-I-087)
        let stats = &self.diagnostic_writer_stats;
        let written = stats.events_written.load(Ordering::Relaxed);
        let dropped = stats.events_dropped.load(Ordering::Relaxed);
        let heartbeat_raw = stats.last_heartbeat_ms.load(Ordering::Relaxed);
        let last_heartbeat_ms = if heartbeat_raw == 0 {
            None
        } else {
            Some(heartbeat_raw)
        };

        let health = DiagnosticHealth {
            ledger_available: true,
            events_written_total: written,
            events_dropped_total: dropped,
            last_heartbeat_ms,
        };

        let bundle = DiagnosticBundle {
            schema_version: 1,
            node_peer_id: self.peer_id.clone(),
            query_timestamp_ms: now_ms,
            events: events.clone(),
            fork_summary,
            classification,
            baseline,
            health,
        };

        serde_json::to_value(&bundle).map_err(|e| RpcError::internal_error(e.to_string()))
    }
}

/// Build a `ForkSummary` from the event list.
fn build_fork_summary(
    events: &[storage::diagnostic_ledger::types::DiagnosticEvent],
) -> ForkSummary {
    let mut by_producer: HashMap<String, u64> = HashMap::new();
    let mut by_event_kind: HashMap<String, u64> = HashMap::new();
    let mut fork_heights: Vec<u64> = Vec::new();

    for ev in events {
        // Count by event kind (all events)
        let kind_name = format!("{:?}", ev.kind);
        *by_event_kind.entry(kind_name).or_default() += 1;

        // Count by producer (only for events that have a producer)
        if let Some(producer) = extract_producer_pubkey(ev) {
            if is_fork_relevant(ev.kind) {
                *by_producer.entry(producer.to_string()).or_default() += 1;
            }
        }

        // Track fork heights
        if is_fork_relevant(ev.kind) {
            if let Some(h) = ev.height {
                fork_heights.push(h);
            }
        }
    }

    let fork_events_in_window = events.iter().filter(|e| is_fork_relevant(e.kind)).count() as u64;

    let first_fork_height = fork_heights.iter().copied().min();
    let last_fork_height = fork_heights.iter().copied().max();

    ForkSummary {
        fork_events_in_window,
        by_producer,
        by_event_kind,
        first_fork_height,
        last_fork_height,
    }
}

/// Extract producer pubkey from event payload (if present).
fn extract_producer_pubkey(
    ev: &storage::diagnostic_ledger::types::DiagnosticEvent,
) -> Option<&str> {
    match &ev.payload {
        EventPayload::BlockApplied {
            producer_pubkey, ..
        } => Some(producer_pubkey.as_str()),
        EventPayload::BlockRejected {
            producer_pubkey, ..
        } => Some(producer_pubkey.as_str()),
        EventPayload::ForkBlockReceived {
            producer_pubkey, ..
        } => Some(producer_pubkey.as_str()),
        _ => None,
    }
}

/// Whether an event kind is "fork-relevant" for summary counting.
fn is_fork_relevant(kind: EventKind) -> bool {
    FORK_RELEVANT_KINDS.contains(&kind)
}

/// Build a simple `BaselineComparison`.
fn build_baseline(
    ledger: &storage::diagnostic_ledger::DiagnosticLedger,
    window_secs: u64,
    events: &[storage::diagnostic_ledger::types::DiagnosticEvent],
) -> BaselineComparison {
    let fork_count_current = events.iter().filter(|e| is_fork_relevant(e.kind)).count() as f64;

    let hours = (window_secs as f64) / 3600.0;
    let current_rate = if hours > 0.0 {
        fork_count_current / hours
    } else {
        0.0
    };

    // 24h baseline: query with a generous limit
    let avg_rate = match ledger.query_recent(86400, 100_000) {
        Ok(all_24h) => {
            let fork_24h = all_24h.iter().filter(|e| is_fork_relevant(e.kind)).count() as f64;
            fork_24h / 24.0
        }
        Err(_) => 0.0,
    };

    let delta_pct = if avg_rate > 0.0 {
        (current_rate - avg_rate) / avg_rate * 100.0
    } else {
        0.0
    };

    BaselineComparison {
        fork_events_per_hour_current: current_rate,
        fork_events_per_hour_24h_avg: avg_rate,
        delta_pct,
    }
}
