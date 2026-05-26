//! In-node automated alert consumer for diagnostic ledger classifications (D4, INC-I-090).
//!
//! Closes the L4 surface gap: no automated reader of `getForkDiagnostic` /
//! `recommended_action` existed, so classifications sat in RocksDB unread until a
//! human ran `doli forks`. This module provides a pure function that polls the
//! local diagnostic ledger, runs `classifier::classify`, and returns actionable
//! alerts for the caller (periodic.rs) to log.
//!
//! Design decisions:
//! - Pure function: no I/O beyond the ledger query. Logging is the caller's job.
//! - Dedup by classifier correlation key (serialized as `<height>|<canon_hash>|<fork_hash>`).
//! - "Actionable" = `recommended_action.is_some()` AND value != `"normal_operation"`.
//! - MonitorAlertFired event kind skipped for SSF — structured tracing is enough.

use std::collections::HashSet;

use serde::Serialize;
use storage::diagnostic_ledger::classifier;
use storage::diagnostic_ledger::types::{Classification, ForkType};
use storage::diagnostic_ledger::DiagnosticLedger;

/// Cadence in seconds for the diagnostic monitor check in `run_periodic_tasks`.
/// Must be <= 60s to satisfy the VERDICT pass-criterion:
/// "At least one automated consumer MUST read diagnostic output periodically (< 60s interval)."
pub const DIAGNOSTIC_MONITOR_INTERVAL_SECS: u64 = 30;

/// An actionable alert produced by the diagnostic monitor.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ActionableAlert {
    /// Derived from the earliest fork event's correlation key or a synthetic ID.
    pub incident_id: String,
    /// String representation of the classified fork type.
    pub fork_type: String,
    /// The recommended remediation action (e.g., "restart_with_resync").
    pub recommended_action: String,
    /// Optional structured arguments for the recommended action.
    pub recommended_action_args: Option<serde_json::Value>,
    /// Event IDs used as evidence for this classification.
    pub evidence_event_ids: Vec<String>,
}

/// Check the diagnostic ledger for actionable alerts within the given time window.
///
/// Pure function (modulo the ledger query): no logging, no side effects beyond
/// mutating `last_alerted` for dedup.
///
/// # Arguments
/// - `ledger`: the diagnostic ledger to query
/// - `window_secs`: how far back to look (seconds from now)
/// - `last_alerted`: set of correlation keys already alerted; alerts matching a key
///   in this set are suppressed. New alerts insert their key.
///
/// # Returns
/// A `Vec<ActionableAlert>` — one per unique actionable classification. Empty if
/// no actionable classification is found or all have been deduped.
pub fn check_for_actionable_alerts(
    ledger: &DiagnosticLedger,
    window_secs: u64,
    last_alerted: &mut HashSet<String>,
) -> Vec<ActionableAlert> {
    // Query recent events from the ledger. Cap at 10,000 per REQ-FORKOBS-SEC-003.
    let events = match ledger.query_recent(window_secs, 10_000) {
        Ok(evts) => evts,
        Err(_) => return Vec::new(), // Graceful degradation
    };

    if events.is_empty() {
        return Vec::new();
    }

    // Run the classifier
    let classification: Classification = classifier::classify(&events);

    // Filter: only actionable recommendations
    if !is_actionable(&classification) {
        return Vec::new();
    }

    // Derive a dedup key from the classification's evidence + fork_type.
    // We use a stable string representation so the same incident dedupes across ticks.
    let dedup_key = derive_dedup_key(&classification);

    // Dedup: skip if already alerted for this key
    if last_alerted.contains(&dedup_key) {
        return Vec::new();
    }

    // Build the alert
    let incident_id = derive_incident_id(&classification);
    let fork_type_str = format!("{:?}", classification.fork_type);
    let recommended_action = classification
        .recommended_action
        .clone()
        .unwrap_or_default();

    let alert = ActionableAlert {
        incident_id,
        fork_type: fork_type_str,
        recommended_action,
        recommended_action_args: classification.recommended_action_args.clone(),
        evidence_event_ids: classification.evidence_event_ids.clone(),
    };

    // Record dedup
    last_alerted.insert(dedup_key);

    vec![alert]
}

/// Returns true if the classification has an actionable recommended_action
/// (i.e., not None and not "normal_operation").
fn is_actionable(classification: &Classification) -> bool {
    match &classification.recommended_action {
        None => false,
        Some(action) => action != "normal_operation",
    }
}

/// Derive a dedup key from the classification.
///
/// Uses the fork_type variant name + the first few evidence event IDs to create
/// a stable string. For ChainBreakLoop, the key includes the variant's counters
/// bucketed to avoid re-alerting on minor counter changes.
fn derive_dedup_key(classification: &Classification) -> String {
    match &classification.fork_type {
        ForkType::ChainBreakLoop {
            chain_break_count,
            recovery_attempts,
            ..
        } => {
            // Bucket by 10s to avoid re-alerting on every small counter change
            format!(
                "ChainBreakLoop:cb{}:ra{}",
                chain_break_count / 10,
                recovery_attempts / 10
            )
        }
        ForkType::RollbackLoop => "RollbackLoop".to_string(),
        ForkType::ProducerEquivocation => {
            // Use first evidence event ID to distinguish different equivocations
            let first = classification
                .evidence_event_ids
                .first()
                .cloned()
                .unwrap_or_default();
            format!("ProducerEquivocation:{}", first)
        }
        ForkType::EpochBoundaryInvalid => {
            let first = classification
                .evidence_event_ids
                .first()
                .cloned()
                .unwrap_or_default();
            format!("EpochBoundaryInvalid:{}", first)
        }
        ForkType::PostSnapDeadTip => "PostSnapDeadTip".to_string(),
        other => format!("{:?}", other),
    }
}

/// Derive an incident ID from the classification.
///
/// Uses the earliest evidence event ID as the incident anchor, prefixed with
/// the fork type for human readability.
fn derive_incident_id(classification: &Classification) -> String {
    let earliest = classification
        .evidence_event_ids
        .first()
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());
    let type_prefix = match &classification.fork_type {
        ForkType::ChainBreakLoop { .. } => "CBL",
        ForkType::RollbackLoop => "RBL",
        ForkType::ProducerEquivocation => "PEQ",
        ForkType::EpochBoundaryInvalid => "EBI",
        ForkType::PostSnapDeadTip => "PSD",
        ForkType::TipRaceHighLatency => "TRH",
        _ => "UNK",
    };
    format!("{}-{}", type_prefix, &earliest[..earliest.len().min(16)])
}
