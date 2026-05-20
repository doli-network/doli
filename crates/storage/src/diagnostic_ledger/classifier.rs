//! Deterministic fork-type classifier — pure function, no I/O.
//!
//! `classify(events)` applies 7 rules in first-match-wins priority order:
//!
//! (a) ProducerEquivocation — two BlockApplied at same height, same producer, different hash
//! (b) EpochBoundaryInvalid — BlockRejected at epoch boundary with "EpochReward" reason
//! (c) RollbackLoop — >3 RollbackStarted within any 60s window
//! (d) PostSnapDeadTip — SnapSyncCompleted followed by ForkBlockReceived within 300s
//! (e) TipRaceHighLatency — ForkBlockReceived where cross-referenced BlockApplied has
//!     validation_duration_ms > 2000   (Decision A2: cross-reference, not inline field)
//! (f) TipRaceNatural — ForkBlockReceived with low latency AND no other fork signals
//!     in the same correlation_key group (Decision D: no other ForkBlockReceived/BlockRejected/
//!     RollbackStarted/RecoveryClassifyCall sharing the same correlation_key)
//! (g) Unknown — fallback when no rule matches
//!
//! Cross-reference rule (Decision A2):
//! Rules (e) and (f) need `validation_duration_ms` which lives in `BlockApplied` payloads.
//! When classifying a `ForkBlockReceived`, the classifier looks for a `BlockApplied` at the
//! same height. If no corresponding `BlockApplied` exists, validation_duration defaults to 0
//! (meaning "no latency signal"), so rule (e) does NOT match.

use super::types::{Classification, DiagnosticEvent, EventKind, EventPayload, ForkType};

/// Default blocks-per-epoch for epoch-boundary detection (mainnet/testnet).
const BLOCKS_PER_EPOCH: u64 = 360;

/// Classify a slice of diagnostic events into a fork type.
///
/// This is a **pure function**: no I/O, no `Instant::now()`, no PRNG, no logging.
/// The same input always produces the same output.
pub fn classify(events: &[DiagnosticEvent]) -> Classification {
    // Try rules in priority order (first-match-wins)
    if let Some(c) = rule_a_producer_equivocation(events) {
        return c;
    }
    if let Some(c) = rule_b_epoch_boundary_invalid(events) {
        return c;
    }
    if let Some(c) = rule_c_rollback_loop(events) {
        return c;
    }
    if let Some(c) = rule_d_post_snap_dead_tip(events) {
        return c;
    }
    if let Some(c) = rule_e_tip_race_high_latency(events) {
        return c;
    }
    if let Some(c) = rule_f_tip_race_natural(events) {
        return c;
    }
    rule_g_unknown(events)
}

// ---------------------------------------------------------------------------
// Rule (a): ProducerEquivocation
// ---------------------------------------------------------------------------

/// Two BlockApplied at the same height by the same producer with different block_hash.
fn rule_a_producer_equivocation(events: &[DiagnosticEvent]) -> Option<Classification> {
    let applied: Vec<&DiagnosticEvent> = events
        .iter()
        .filter(|e| e.kind == EventKind::BlockApplied)
        .collect();

    for (i, a) in applied.iter().enumerate() {
        for b in applied.iter().skip(i + 1) {
            let (h_a, hash_a, prod_a) = extract_applied_info(a);
            let (h_b, hash_b, prod_b) = extract_applied_info(b);
            if h_a == h_b && prod_a == prod_b && hash_a != hash_b {
                return Some(Classification {
                    fork_type: ForkType::ProducerEquivocation,
                    confidence: 0.95,
                    evidence_event_ids: vec![a.event_id.clone(), b.event_id.clone()],
                    recommended_action: Some("investigate_producer".to_string()),
                    recommended_action_args: None,
                });
            }
        }
    }
    None
}

fn extract_applied_info(e: &DiagnosticEvent) -> (Option<u64>, &str, &str) {
    if let EventPayload::BlockApplied {
        ref block_hash,
        ref producer_pubkey,
        ..
    } = e.payload
    {
        (e.height, block_hash.as_str(), producer_pubkey.as_str())
    } else {
        (None, "", "")
    }
}

// ---------------------------------------------------------------------------
// Rule (b): EpochBoundaryInvalid
// ---------------------------------------------------------------------------

/// BlockRejected at an epoch boundary height with "EpochReward" in rejection reason.
fn rule_b_epoch_boundary_invalid(events: &[DiagnosticEvent]) -> Option<Classification> {
    for e in events {
        if e.kind != EventKind::BlockRejected {
            continue;
        }
        let h = e.height.unwrap_or(0);
        if h == 0 || h % BLOCKS_PER_EPOCH != 0 {
            continue;
        }
        if let EventPayload::BlockRejected {
            ref rejection_reason,
            ..
        } = e.payload
        {
            if rejection_reason.contains("EpochReward") {
                return Some(Classification {
                    fork_type: ForkType::EpochBoundaryInvalid,
                    confidence: 0.90,
                    evidence_event_ids: vec![e.event_id.clone()],
                    recommended_action: Some("investigate_producer".to_string()),
                    recommended_action_args: None,
                });
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Rule (c): RollbackLoop
// ---------------------------------------------------------------------------

/// More than 3 RollbackStarted events within any 60-second window.
fn rule_c_rollback_loop(events: &[DiagnosticEvent]) -> Option<Classification> {
    let mut rollbacks: Vec<&DiagnosticEvent> = events
        .iter()
        .filter(|e| e.kind == EventKind::RollbackStarted)
        .collect();

    if rollbacks.len() <= 3 {
        return None;
    }

    rollbacks.sort_by_key(|e| e.timestamp_ms);

    // Sliding window: check if any window of 60s contains >3 rollbacks
    for window_start_idx in 0..rollbacks.len() {
        let start_ts = rollbacks[window_start_idx].timestamp_ms;
        let end_ts = start_ts + 60_000;
        let count = rollbacks
            .iter()
            .skip(window_start_idx)
            .take_while(|e| e.timestamp_ms <= end_ts)
            .count();
        if count > 3 {
            let evidence: Vec<String> = rollbacks.iter().map(|e| e.event_id.clone()).collect();
            return Some(Classification {
                fork_type: ForkType::RollbackLoop,
                confidence: 0.85,
                evidence_event_ids: evidence,
                recommended_action: Some("investigate_recovery_params".to_string()),
                recommended_action_args: None,
            });
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Rule (d): PostSnapDeadTip
// ---------------------------------------------------------------------------

/// SnapSyncCompleted followed by ForkBlockReceived within 300 seconds.
fn rule_d_post_snap_dead_tip(events: &[DiagnosticEvent]) -> Option<Classification> {
    let snap_events: Vec<&DiagnosticEvent> = events
        .iter()
        .filter(|e| e.kind == EventKind::SnapSyncCompleted)
        .collect();

    let fork_events: Vec<&DiagnosticEvent> = events
        .iter()
        .filter(|e| e.kind == EventKind::ForkBlockReceived)
        .collect();

    for snap in &snap_events {
        for fork in &fork_events {
            let delta_ms = fork.timestamp_ms.saturating_sub(snap.timestamp_ms);
            if delta_ms <= 300_000 {
                return Some(Classification {
                    fork_type: ForkType::PostSnapDeadTip,
                    confidence: 0.80,
                    evidence_event_ids: vec![snap.event_id.clone(), fork.event_id.clone()],
                    recommended_action: Some("investigate_snap_sync".to_string()),
                    recommended_action_args: None,
                });
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Rule (e): TipRaceHighLatency
// ---------------------------------------------------------------------------

/// ForkBlockReceived where the cross-referenced BlockApplied at the same height
/// has `validation_duration_ms > 2000`.
fn rule_e_tip_race_high_latency(events: &[DiagnosticEvent]) -> Option<Classification> {
    let fork_events: Vec<&DiagnosticEvent> = events
        .iter()
        .filter(|e| e.kind == EventKind::ForkBlockReceived)
        .collect();

    if fork_events.is_empty() {
        return None;
    }

    // Cross-reference: find a BlockApplied at the same height with high latency
    for fork_ev in &fork_events {
        let fork_height = fork_ev.height;
        let latency = find_validation_duration(events, fork_height);
        if latency > 2000 {
            return Some(Classification {
                fork_type: ForkType::TipRaceHighLatency,
                confidence: 0.75,
                evidence_event_ids: vec![fork_ev.event_id.clone()],
                recommended_action: Some("investigate_latency".to_string()),
                recommended_action_args: None,
            });
        }
    }
    None
}

/// Cross-reference helper: find the `validation_duration_ms` from a
/// `BlockApplied` event at the same height as the fork event.
/// Returns 0 if no matching BlockApplied found (Decision A2 fallback).
fn find_validation_duration(events: &[DiagnosticEvent], height: Option<u64>) -> u64 {
    let h = match height {
        Some(h) => h,
        None => return 0,
    };
    for e in events {
        if e.kind == EventKind::BlockApplied && e.height == Some(h) {
            if let EventPayload::BlockApplied {
                validation_duration_ms,
                ..
            } = &e.payload
            {
                return *validation_duration_ms;
            }
        }
    }
    0
}

// ---------------------------------------------------------------------------
// Rule (f): TipRaceNatural
// ---------------------------------------------------------------------------

/// ForkBlockReceived with low latency (<= 2000ms) and no other fork signals
/// in the same correlation_key group.
///
/// "No other signals" (Decision D): no other event with kind in
/// {ForkBlockReceived (other than the one being classified), BlockRejected,
/// RollbackStarted, RecoveryClassifyCall} sharing the same correlation_key.
/// Events with all-None correlation_key are in their own singleton group.
fn rule_f_tip_race_natural(events: &[DiagnosticEvent]) -> Option<Classification> {
    let fork_events: Vec<&DiagnosticEvent> = events
        .iter()
        .filter(|e| e.kind == EventKind::ForkBlockReceived)
        .collect();

    if fork_events.is_empty() {
        return None;
    }

    for fork_ev in &fork_events {
        let fork_height = fork_ev.height;
        let latency = find_validation_duration(events, fork_height);

        // Rule (e) already checked > 2000, so if we get here latency <= 2000
        if latency > 2000 {
            continue;
        }

        // Check "no other signals" in the same correlation group
        if has_other_signals(events, fork_ev) {
            continue;
        }

        return Some(Classification {
            fork_type: ForkType::TipRaceNatural,
            confidence: 0.70,
            evidence_event_ids: vec![fork_ev.event_id.clone()],
            recommended_action: Some("none_natural_fork".to_string()),
            recommended_action_args: None,
        });
    }
    None
}

/// Check whether the given fork event has "other signals" in its correlation group.
fn has_other_signals(events: &[DiagnosticEvent], fork_ev: &DiagnosticEvent) -> bool {
    let signal_kinds = [
        EventKind::ForkBlockReceived,
        EventKind::BlockRejected,
        EventKind::RollbackStarted,
        EventKind::RecoveryClassifyCall,
    ];

    match &fork_ev.correlation_key {
        None => {
            // All-None correlation_key: singleton group. No other events share it.
            // BUT we still check for other fork/recovery events with None correlation_key
            // that are NOT the fork event itself.
            // Per Decision D: treat as singleton, so no other signals.
            false
        }
        Some(corr_key) => {
            for e in events {
                if e.event_id == fork_ev.event_id {
                    continue;
                }
                if !signal_kinds.contains(&e.kind) {
                    continue;
                }
                // Check if this event shares the same correlation_key
                if let Some(ref other_ck) = e.correlation_key {
                    if other_ck == corr_key {
                        return true;
                    }
                }
            }
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Rule (g): Unknown
// ---------------------------------------------------------------------------

/// Fallback when no other rule matches. Returns all event IDs as evidence
/// per REQ-FORKOBS-RETRO-003.
fn rule_g_unknown(events: &[DiagnosticEvent]) -> Classification {
    let evidence: Vec<String> = events.iter().map(|e| e.event_id.clone()).collect();
    let reason = if events.is_empty() {
        "no events to classify".to_string()
    } else {
        "no classification rule matched the provided events".to_string()
    };

    Classification {
        fork_type: ForkType::Unknown {
            reason_unknown: reason,
            evidence_event_ids: evidence.clone(),
        },
        confidence: 0.0,
        evidence_event_ids: evidence,
        recommended_action: None,
        recommended_action_args: None,
    }
}
