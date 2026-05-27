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

/// Multi-modal stuck-state detector. Fires when ANY of four aggregate signals trips,
/// indicating the node is in a chain-break / recovery-churn loop rather than a transient
/// natural tip race. See the rule precedence section of
/// `specs/fork-observability-architecture.md` for the design rationale.
///
/// Window: the most recent 1 hour ending at the slice's latest `timestamp_ms`. Keeping
/// the reference point inside the slice preserves `classify()` purity (no system clock).
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

    for e in events {
        if e.timestamp_ms < window_start {
            continue;
        }
        match e.kind {
            EventKind::ChainBreakDetected => chain_break_count += 1,
            EventKind::BlockApplied => {
                block_applied_count += 1;
                latest_applied_ms =
                    Some(latest_applied_ms.map_or(e.timestamp_ms, |t| t.max(e.timestamp_ms)));
            }
            EventKind::ForkBlockReceived => fork_block_received_count += 1,
            EventKind::RollbackStarted => rollback_count += 1,
            EventKind::RecoveryClassifyCall => recovery_attempts += 1,
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
