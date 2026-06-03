//! Deterministic fork-type classifier — pure function, no I/O.
//!
//! `classify(events)` applies 8 rules in first-match-wins priority order:
//!
//! (a) ProducerEquivocation — two BlockApplied at same height, same producer, different hash
//! (b) EpochBoundaryInvalid — BlockRejected at epoch boundary with "EpochReward" reason
//! (c) RollbackLoop — >3 RollbackStarted within any 60s window
//! (d) PostSnapDeadTip — SnapSyncCompleted followed by ForkBlockReceived within 300s
//! (h) ChainBreakLoop — node stuck in post-snap chain-break / recovery-churn loop. ANY of:
//!     (sig_a) chain_break_count > 3; (sig_b) >100 ForkBlockReceived AND ratio fork/applied > 10;
//!     (sig_c) >10 RollbackStarted in 1h window; (sig_d) >20 RecoveryClassifyCall in window.
//!     Workflow #349 — fires BEFORE (e)/(f) because the n6 2026-05-20 incident proved
//!     rule (f) confidently mis-labels stuck nodes as TipRaceNatural / normal_operation.
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
//!
//! Correlation grouping (INC-I-090 D3): events are grouped by `divergence_height` when
//! present, not by exact `CorrelationKey` equality. Different emitters may populate
//! different subsets of the key (e.g., block_handling knows `fork_hash`, recovery
//! coordinator only knows `divergence_height`). Two events with the same
//! `divergence_height` belong to the same fork episode regardless of hash fields.

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
    if let Some(c) = rule_h_chain_break_loop(events) {
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

        // Rule (f) per spec: validation_duration < 500ms (tight fast-race window).
        // Middle range 500-2000ms intentionally falls through to Unknown for human
        // escalation (rule e covers > 2000ms, this rule covers fast races only).
        if latency >= 500 {
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
            recommended_action: Some("normal_operation".to_string()),
            recommended_action_args: None,
        });
    }
    None
}

/// Check whether the given fork event has "other signals" in its correlation group.
///
/// Correlation grouping uses `divergence_height` as the primary key (INC-I-090 D3).
/// Two events belong to the same fork episode if they share the same
/// `divergence_height`, regardless of whether `canonical_hash` or `fork_hash`
/// differ (different emitters populate different subsets of the CorrelationKey).
/// Events with `correlation_key = None` remain singletons per Decision D.
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
            // Per Decision D: treat as singleton, so no other signals.
            false
        }
        Some(corr_key) => {
            // Extract the divergence_height for grouping (primary correlation
            // dimension). If the key has no divergence_height, fall back to
            // exact equality (legacy behavior).
            let div_height = corr_key.divergence_height;

            for e in events {
                if e.event_id == fork_ev.event_id {
                    continue;
                }
                if !signal_kinds.contains(&e.kind) {
                    continue;
                }
                if let Some(ref other_ck) = e.correlation_key {
                    // INC-I-090 D3: match on divergence_height when both sides
                    // have it set, rather than requiring full CorrelationKey
                    // equality. This allows RecoveryClassifyCall events (which
                    // only know divergence_height) to group with
                    // ForkBlockReceived events (which also carry fork_hash).
                    let same_group = match (div_height, other_ck.divergence_height) {
                        (Some(h1), Some(h2)) => h1 == h2,
                        _ => other_ck == corr_key,
                    };
                    if same_group {
                        return true;
                    }
                }
            }
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Rule (h): ChainBreakLoop — Workflow #349 (Phase 1.5)
// ---------------------------------------------------------------------------

/// Analysis window for rule (h): 1 hour, in milliseconds.
const CHAIN_BREAK_LOOP_WINDOW_MS: u64 = 3_600_000;

/// Evidence TTL for the health gate: if the most recent BlockApplied is within
/// this many seconds of `now_ms`, the node is considered currently healthy.
const EVIDENCE_TTL_SECS: u64 = 120;

/// Multi-modal stuck-state detector. Fires when ANY of four aggregate signals trips,
/// indicating the node is in a chain-break / recovery-churn loop rather than a transient
/// natural tip race. See the rule precedence section of
/// `specs/fork-observability-architecture.md` for the design rationale.
///
/// Window: the most recent 1 hour ending at the slice's latest `timestamp_ms`. Keeping
/// the reference point inside the slice preserves `classify()` purity (no system clock).
///
/// INC-I-091 D1: `signal_d` only counts `RecoveryClassifyCall` events where
/// `action_returned` is a real recovery action (not `"None"`). This prevents
/// healthy nodes from tripping the threshold via ~1/sec no-op classifier calls.
///
/// INC-I-091 D2: Before returning a ChainBreakLoop classification, a health gate
/// checks whether the node has recently recovered. If the most recent BlockApplied
/// is within `EVIDENCE_TTL_SECS` (120s) of `now_ms` AND no ChainBreakDetected
/// occurred after that BlockApplied, the classification is suppressed (returns None).
fn rule_h_chain_break_loop(events: &[DiagnosticEvent]) -> Option<Classification> {
    if events.is_empty() {
        return None;
    }

    // Anchor "now" to the latest event timestamp in the slice (pure-function discipline).
    let now_ms = events.iter().map(|e| e.timestamp_ms).max()?;
    let window_start = now_ms.saturating_sub(CHAIN_BREAK_LOOP_WINDOW_MS);

    let mut chain_break_count: u32 = 0;
    let mut block_applied_count: u32 = 0;
    let mut fork_block_received_count: u32 = 0;
    let mut rollback_count: u32 = 0;
    let mut recovery_attempts: u32 = 0;
    let mut latest_applied_ms: Option<u64> = None;
    let mut latest_chain_break_ms: Option<u64> = None;

    for e in events {
        if e.timestamp_ms < window_start {
            continue;
        }
        match e.kind {
            EventKind::ChainBreakDetected => {
                chain_break_count += 1;
                latest_chain_break_ms =
                    Some(latest_chain_break_ms.map_or(e.timestamp_ms, |t| t.max(e.timestamp_ms)));
            }
            EventKind::BlockApplied => {
                block_applied_count += 1;
                latest_applied_ms =
                    Some(latest_applied_ms.map_or(e.timestamp_ms, |t| t.max(e.timestamp_ms)));
            }
            EventKind::ForkBlockReceived => fork_block_received_count += 1,
            EventKind::RollbackStarted => rollback_count += 1,
            EventKind::RecoveryClassifyCall => {
                // INC-I-091 D1: only count recovery calls where a real action
                // was taken (not "None"). The action_returned field is
                // populated by periodic.rs EMIT-007 with format!("{:?}", action).
                if let EventPayload::RecoveryClassifyCall {
                    ref action_returned,
                    ..
                } = e.payload
                {
                    let is_real_action = match action_returned {
                        Some(a) => a != "None",
                        None => false,
                    };
                    if is_real_action {
                        recovery_attempts += 1;
                    }
                }
            }
            _ => {}
        }
    }

    // Signals (any-of). Each threshold is justified in
    // specs/fork-observability-architecture.md § Workflow #349 Phase 1.5.
    let signal_a = chain_break_count > 3;
    let signal_b = fork_block_received_count > 100
        && fork_block_received_count / block_applied_count.max(1) > 10;
    let signal_c = rollback_count > 10;
    let signal_d = recovery_attempts > 20;

    if !(signal_a || signal_b || signal_c || signal_d) {
        return None;
    }

    // seconds_stuck: time since the most recent BlockApplied in the window, or the
    // window span if no BlockApplied is present.
    let seconds_stuck = match latest_applied_ms {
        Some(t) => now_ms.saturating_sub(t) / 1000,
        None => now_ms.saturating_sub(window_start) / 1000,
    };

    // INC-I-091 D2: Health gate — suppress stale classification when the node
    // has recovered. If the most recent BlockApplied is within EVIDENCE_TTL of
    // now AND no ChainBreakDetected occurred after that BlockApplied, the node
    // is currently healthy and we return None.
    if let Some(last_applied) = latest_applied_ms {
        let age_secs = now_ms.saturating_sub(last_applied) / 1000;
        if age_secs <= EVIDENCE_TTL_SECS {
            // Node has a recent BlockApplied — check if any ChainBreakDetected
            // occurred AFTER that BlockApplied.
            let chain_break_after_applied =
                latest_chain_break_ms.is_some_and(|cb_ms| cb_ms > last_applied);
            if !chain_break_after_applied {
                return None;
            }
        }
    }

    // Evidence: up to 20 representative events from the three churn-indicator kinds.
    // BlockApplied is excluded because its presence/absence is summarized in seconds_stuck.
    let evidence_event_ids: Vec<String> = events
        .iter()
        .filter(|e| e.timestamp_ms >= window_start)
        .filter(|e| {
            matches!(
                e.kind,
                EventKind::ChainBreakDetected
                    | EventKind::RecoveryClassifyCall
                    | EventKind::RollbackStarted
            )
        })
        .take(20)
        .map(|e| e.event_id.clone())
        .collect();

    let recommended_action_args = serde_json::json!({
        "approach": "stop_node + rm -rf <data_dir>/{blocks,state_db,utxo,diagnostics} + restart with --no-snap=false",
        "preserve": ["wallet.json", "producer.seed.txt"],
        "verify_after": "doli forks --explain --human after 10 minutes of sync",
    });

    Some(Classification {
        fork_type: ForkType::ChainBreakLoop {
            chain_break_count,
            recovery_attempts,
            seconds_stuck,
            rollback_count,
        },
        confidence: 0.85,
        evidence_event_ids,
        recommended_action: Some("restart_with_resync".to_string()),
        recommended_action_args: Some(recommended_action_args),
    })
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

// ---------------------------------------------------------------------------
// Tests — INC-I-091
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic_ledger::types::{CorrelationKey, DiagnosticEvent, EventPayload};

    // OUTPUT CONTRACT: fn rule_h_chain_break_loop(events: &[DiagnosticEvent])
    //                  -> Option<Classification>
    //   O1: Option<Classification> — Some(ChainBreakLoop{..}) or None
    // PATHS:
    //   P1: No signals trip → None
    //   P2: signal_d trips (recovery_attempts > 20) → Some(ChainBreakLoop)
    //   P3: signal_c trips (rollback_count > 10) → Some(ChainBreakLoop)
    //   P4: Health gate suppresses (recent BlockApplied, no ChainBreakDetected
    //       after) → None even though signals trip
    //   P5: Health gate does NOT suppress (no recent BlockApplied OR
    //       ChainBreakDetected after last applied) → Some(ChainBreakLoop)
    // INPUT PARTITIONS:
    //   IP1: 3600 RecoveryClassifyCall action=None + 360 BlockApplied recent
    //        → exercises D1 fix (P1: no real recovery attempts counted)
    //   IP2: 25 RecoveryClassifyCall action=ShallowRollback, 0 BlockApplied
    //        → exercises real-alarm path (P2: signal_d trips, no health gate)
    //   IP3: 30 RollbackStarted ~10min ago + 50 BlockApplied in last 60s
    //        → exercises D2 fix (P4: signal_c trips but health gate suppresses)
    //   IP4: 30 RollbackStarted last 5min + 0 BlockApplied in 120s +
    //        5 ChainBreakDetected after last applied
    //        → exercises ongoing-brokenness path (P5: signal_c, no suppression)
    // MATRIX: 1 output (O1) x 4 partitions = 4 cells
    //   IP1: O1=None (D1 fix — FAILS pre-fix: was Some)
    //   IP2: O1=Some(ChainBreakLoop) (regression guard, signal_d real alarm)
    //   IP3: O1=None (D2 fix — FAILS pre-fix: was Some)
    //   IP4: O1=Some(ChainBreakLoop) (regression guard, ongoing brokenness)

    /// Helper: create a RecoveryClassifyCall event with the given action string.
    fn make_recovery_event(id: &str, ts_ms: u64, action: Option<&str>) -> DiagnosticEvent {
        DiagnosticEvent {
            event_id: id.to_string(),
            kind: EventKind::RecoveryClassifyCall,
            timestamp_ms: ts_ms,
            height: Some(1000),
            correlation_key: Some(CorrelationKey {
                divergence_height: Some(1000),
                canonical_hash: None,
                fork_hash: None,
            }),
            caused_by_event_id: None,
            is_cascade_origin: false,
            payload: EventPayload::RecoveryClassifyCall {
                local_height: 1000,
                network_tip_height: 1000,
                peer_count: 5,
                last_applied_secs: 10,
                shallow_rollback_count: 0,
                snap_attempts: 0,
                last_rollback_local_height: None,
                in_grace_period: false,
                last_finality_height: None,
                action_returned: action.map(|s| s.to_string()),
                rule_matched: None,
            },
        }
    }

    /// Helper: create a BlockApplied event.
    fn make_block_applied(id: &str, ts_ms: u64, height: u64) -> DiagnosticEvent {
        DiagnosticEvent {
            event_id: id.to_string(),
            kind: EventKind::BlockApplied,
            timestamp_ms: ts_ms,
            height: Some(height),
            correlation_key: None,
            caused_by_event_id: None,
            is_cascade_origin: false,
            payload: EventPayload::BlockApplied {
                slot: height as u32,
                block_hash: format!("hash_{}", height),
                producer_pubkey: "prod1".to_string(),
                from_peer_id: None,
                received_at_ms: None,
                applied_at_ms: ts_ms,
                validation_duration_ms: 50,
                mode: "normal".to_string(),
                tx_count: 1,
            },
        }
    }

    /// Helper: create a RollbackStarted event.
    fn make_rollback_started(id: &str, ts_ms: u64) -> DiagnosticEvent {
        DiagnosticEvent {
            event_id: id.to_string(),
            kind: EventKind::RollbackStarted,
            timestamp_ms: ts_ms,
            height: Some(1000),
            correlation_key: None,
            caused_by_event_id: None,
            is_cascade_origin: false,
            payload: EventPayload::RollbackStarted {
                from_height: 1000,
                to_height: 999,
                trigger: "fork_detected".to_string(),
                cumulative_depth: 1,
            },
        }
    }

    /// Helper: create a ChainBreakDetected event.
    fn make_chain_break(id: &str, ts_ms: u64) -> DiagnosticEvent {
        DiagnosticEvent {
            event_id: id.to_string(),
            kind: EventKind::ChainBreakDetected,
            timestamp_ms: ts_ms,
            height: Some(1000),
            correlation_key: None,
            caused_by_event_id: None,
            is_cascade_origin: false,
            payload: EventPayload::ChainBreakDetected {
                expected_prev_hash: "expected".to_string(),
                actual_prev_hash: "actual".to_string(),
                header_slot: 1000,
                valid_so_far_count: 50,
                from_peer_id: "peer1".to_string(),
            },
        }
    }

    /// INC-I-091 D1: 3600 RecoveryClassifyCall events with action=None over 1h
    /// plus 360 BlockApplied events (one every 10s) should NOT trip signal_d.
    /// Pre-fix: FAILS (returns Some(ChainBreakLoop) because all 3600 events
    /// were counted as recovery_attempts).
    /// Post-fix: PASSES (only real recovery actions counted).
    #[test]
    fn healthy_node_does_not_trip_signal_d() {
        let base_ms: u64 = 1_700_000_000_000; // arbitrary epoch base
        let mut events = Vec::new();

        // 3600 RecoveryClassifyCall events with action=None, 1/sec over 1 hour
        for i in 0..3600u64 {
            events.push(make_recovery_event(
                &format!("rc_{}", i),
                base_ms + i * 1_000,
                Some("None"),
            ));
        }

        // 360 BlockApplied events, one every 10s over the same hour
        // Most recent within last 10s of the window
        for i in 0..360u64 {
            events.push(make_block_applied(
                &format!("ba_{}", i),
                base_ms + i * 10_000,
                1000 + i,
            ));
        }

        let result = rule_h_chain_break_loop(&events);
        assert!(
            result.is_none(),
            "INC-I-091 D1: healthy node with 3600 action=None RecoveryClassifyCall \
             events should NOT trip ChainBreakLoop. Got: {:?}",
            result
        );
    }

    /// INC-I-091 D1 regression guard: 25 RecoveryClassifyCall events with
    /// action=ShallowRollback over 10 minutes, zero BlockApplied.
    /// Must return Some(ChainBreakLoop) with signal_d fired.
    /// This must PASS both pre-fix AND post-fix.
    #[test]
    fn real_recovery_loop_still_trips_signal_d() {
        let base_ms: u64 = 1_700_000_000_000;
        let mut events = Vec::new();

        // 25 recovery calls with real action over 10 minutes
        for i in 0..25u64 {
            events.push(make_recovery_event(
                &format!("rc_{}", i),
                base_ms + i * 24_000, // ~24s apart over 10 minutes
                Some("ShallowRollback { depth: 1 }"),
            ));
        }

        let result = rule_h_chain_break_loop(&events);
        assert!(
            result.is_some(),
            "INC-I-091 D1 regression: 25 real recovery attempts must trip ChainBreakLoop"
        );

        let classification = result.unwrap();
        match &classification.fork_type {
            ForkType::ChainBreakLoop {
                recovery_attempts, ..
            } => {
                assert!(
                    *recovery_attempts >= 20,
                    "Expected recovery_attempts >= 20, got {}",
                    recovery_attempts
                );
            }
            other => panic!("Expected ChainBreakLoop, got {:?}", other),
        }
    }

    /// INC-I-091 D2: Node that recovered (30 RollbackStarted ~10min ago,
    /// 50 BlockApplied in last 60s, most recent within 5s, zero
    /// ChainBreakDetected after last applied) should NOT show ChainBreakLoop.
    /// Pre-fix: FAILS (returns Some(ChainBreakLoop) via rollback_count > 10).
    /// Post-fix: PASSES (health gate suppresses stale classification).
    #[test]
    fn recovered_node_clears_chain_break_loop() {
        let now_ms: u64 = 1_700_000_000_000;
        let mut events = Vec::new();

        // 30 RollbackStarted events ~10 minutes ago (within the 1h window)
        let rollback_base = now_ms - 600_000; // 10 minutes ago
        for i in 0..30u64 {
            events.push(make_rollback_started(
                &format!("rb_{}", i),
                rollback_base + i * 1_000, // spread over 30s
            ));
        }

        // 50 BlockApplied events in the last 60s, most recent within 5s of now
        for i in 0..50u64 {
            let ts = now_ms - 60_000 + i * 1_200; // spread over 60s
            events.push(make_block_applied(&format!("ba_{}", i), ts, 2000 + i));
        }

        // No ChainBreakDetected after the most recent BlockApplied

        let result = rule_h_chain_break_loop(&events);
        assert!(
            result.is_none(),
            "INC-I-091 D2: recovered node with recent BlockApplied and no \
             ChainBreakDetected should NOT show ChainBreakLoop. Got: {:?}",
            result
        );
    }

    /// INC-I-091 D2 regression guard: Node that is still broken (30
    /// RollbackStarted in last 5 minutes, zero BlockApplied in last 120s,
    /// 5 ChainBreakDetected after the most recent BlockApplied) must still
    /// alarm with ChainBreakLoop.
    #[test]
    fn still_broken_node_still_alarms() {
        let now_ms: u64 = 1_700_000_000_000;
        let mut events = Vec::new();

        // 30 RollbackStarted in last 5 minutes
        let rollback_base = now_ms - 300_000; // 5 minutes ago
        for i in 0..30u64 {
            events.push(make_rollback_started(
                &format!("rb_{}", i),
                rollback_base + i * 10_000,
            ));
        }

        // Some BlockApplied events, but ALL more than 120s ago
        let old_applied_base = now_ms - 500_000; // ~8 minutes ago
        for i in 0..5u64 {
            events.push(make_block_applied(
                &format!("ba_{}", i),
                old_applied_base + i * 10_000,
                1000 + i,
            ));
        }

        // 5 ChainBreakDetected events AFTER the most recent BlockApplied
        let last_applied_ms = old_applied_base + 4 * 10_000; // ~460s ago
        for i in 0..5u64 {
            events.push(make_chain_break(
                &format!("cb_{}", i),
                last_applied_ms + 10_000 + i * 5_000,
            ));
        }

        let result = rule_h_chain_break_loop(&events);
        assert!(
            result.is_some(),
            "INC-I-091 D2 regression: still-broken node must show ChainBreakLoop"
        );

        let classification = result.unwrap();
        match &classification.fork_type {
            ForkType::ChainBreakLoop { rollback_count, .. } => {
                assert!(
                    *rollback_count > 10,
                    "Expected rollback_count > 10, got {}",
                    rollback_count
                );
            }
            other => panic!("Expected ChainBreakLoop, got {:?}", other),
        }
    }
}
