//! Tests for the diagnostic monitor — in-node automated alert consumer (D4, INC-I-090).
//!
//! OUTPUT CONTRACT: fn check_for_actionable_alerts(
//!     ledger: &DiagnosticLedger, window_secs: u64, last_alerted: &mut HashSet<String>
//! ) -> Vec<ActionableAlert>
//!
//!   O1: return  — Vec<ActionableAlert>: non-empty when classifier finds actionable
//!               recommended_action (anything except None / "normal_operation").
//!   O2: mutation — last_alerted HashSet<String>: correlation keys of returned alerts
//!               are inserted, so subsequent calls with the same ledger state return empty.
//!
//! PATHS:
//!   P1: ChainBreakLoop via signal_d (>20 RecoveryClassifyCall in 1h window)
//!       -> returns Vec with recommended_action = "restart_with_resync", fork_type contains "ChainBreakLoop"
//!   P2: Dedup — same ledger + same last_alerted -> returns empty Vec
//!   P3: No actionable events (only BlockApplied) -> returns empty Vec
//!   P4: TipRaceNatural (recommended_action = "normal_operation") -> returns empty Vec
//!       (filtered out — "normal_operation" is not actionable)
//!
//! MATRIX: 2 outputs (O1 Vec, O2 mutation) x 4 paths = 8 cells.
//!   P1-O1: Vec.len() >= 1, alert fields correct    ASSERT
//!   P1-O2: last_alerted.len() >= 1                  ASSERT
//!   P2-O1: Vec.is_empty()                           ASSERT
//!   P2-O2: last_alerted unchanged                   ASSERT (len stays same)
//!   P3-O1: Vec.is_empty()                           ASSERT
//!   P3-O2: last_alerted.is_empty()                  ASSERT
//!   P4-O1: Vec.is_empty()                           ASSERT
//!   P4-O2: last_alerted.is_empty()                  ASSERT
//!
//! INPUT PARTITIONS:
//!   IP1 (event count, signal_d threshold): 25 RecoveryClassifyCall events — well
//!        above the >20 boundary that triggers rule (h) signal_d in classifier.rs.
//!        Events placed 10s inside the window edge to avoid timing races.
//!   IP2 (dedup state): non-empty last_alerted HashSet containing the same correlation
//!        key as the events in the ledger — exercises the dedup guard.
//!   IP3 (event kind, non-fork): only BlockApplied events — classifier falls through
//!        all rules to Unknown (recommended_action = None).
//!   IP4 (actionable filter, normal_operation): ForkBlockReceived + BlockApplied with
//!        low validation_duration (< 500ms) and no other signals — classifier returns
//!        TipRaceNatural with recommended_action = "normal_operation", which the monitor
//!        filters out as non-actionable.

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use storage::diagnostic_ledger::types::{
        CorrelationKey, DiagnosticEvent, EventKind, EventPayload,
    };
    use storage::diagnostic_ledger::DiagnosticLedger;
    use tempfile::TempDir;

    use crate::node::diagnostic_monitor::check_for_actionable_alerts;

    /// Helper: create a DiagnosticLedger in a temp directory.
    fn make_ledger() -> (TempDir, DiagnosticLedger) {
        let dir = TempDir::new().expect("failed to create temp dir");
        let ledger = DiagnosticLedger::open(dir.path()).expect("failed to open ledger");
        (dir, ledger)
    }

    /// Helper: create a RecoveryClassifyCall event at a given timestamp.
    fn make_recovery_event(
        ts_ms: u64,
        height: u64,
        corr_key: Option<CorrelationKey>,
    ) -> DiagnosticEvent {
        DiagnosticEvent {
            event_id: ulid::Ulid::new().to_string(),
            kind: EventKind::RecoveryClassifyCall,
            timestamp_ms: ts_ms,
            height: Some(height),
            correlation_key: corr_key,
            caused_by_event_id: None,
            is_cascade_origin: false,
            payload: EventPayload::RecoveryClassifyCall {
                local_height: height,
                network_tip_height: height + 100,
                peer_count: 5,
                last_applied_secs: 600,
                shallow_rollback_count: 0,
                snap_attempts: 0,
                last_rollback_local_height: None,
                in_grace_period: false,
                last_finality_height: Some(height.saturating_sub(10)),
                action_returned: Some("None".to_string()),
                rule_matched: None,
            },
        }
    }

    /// Helper: create a BlockApplied event.
    fn make_block_applied(ts_ms: u64, height: u64) -> DiagnosticEvent {
        DiagnosticEvent {
            event_id: ulid::Ulid::new().to_string(),
            kind: EventKind::BlockApplied,
            timestamp_ms: ts_ms,
            height: Some(height),
            correlation_key: None,
            caused_by_event_id: None,
            is_cascade_origin: false,
            payload: EventPayload::BlockApplied {
                slot: height as u32,
                block_hash: format!("hash_{}", height),
                producer_pubkey: "producer_abc".to_string(),
                from_peer_id: None,
                received_at_ms: Some(ts_ms),
                applied_at_ms: ts_ms,
                validation_duration_ms: 50,
                mode: "Full".to_string(),
                tx_count: 1,
            },
        }
    }

    /// Helper: create a ForkBlockReceived event with no other signals in group.
    fn make_fork_natural(ts_ms: u64, height: u64) -> DiagnosticEvent {
        DiagnosticEvent {
            event_id: ulid::Ulid::new().to_string(),
            kind: EventKind::ForkBlockReceived,
            timestamp_ms: ts_ms,
            height: Some(height),
            correlation_key: Some(CorrelationKey {
                divergence_height: Some(height),
                canonical_hash: Some("canonical_hash".to_string()),
                fork_hash: Some("fork_hash".to_string()),
            }),
            caused_by_event_id: None,
            is_cascade_origin: false,
            payload: EventPayload::ForkBlockReceived {
                block_hash: format!("fork_hash_{}", height),
                block_slot: height as u32,
                block_height_estimate: Some(height),
                producer_pubkey: "producer_xyz".to_string(),
                from_peer_id: "peer_123".to_string(),
                classification: "fork".to_string(),
                fork_kind: None,
                local_tip_hash: "local_tip".to_string(),
                local_tip_height: height,
            },
        }
    }

    // -----------------------------------------------------------------------
    // P1: ChainBreakLoop via signal_d (>20 RecoveryClassifyCall in window)
    //     IP1: 25 events, well above threshold, 10s margin from window edge
    // -----------------------------------------------------------------------
    #[test]
    fn p1_chain_break_loop_returns_actionable_alert() {
        let (_dir, ledger) = make_ledger();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // Shared correlation key for the incident
        let corr = Some(CorrelationKey {
            divergence_height: Some(100_000),
            canonical_hash: Some("canon".to_string()),
            fork_hash: Some("fork".to_string()),
        });

        // Emit 25 RecoveryClassifyCall events, all within the last 4 minutes
        // (window_secs=300 = 5 min, we place events starting at now-240s with 10s gaps)
        // This leaves a 60s buffer from the window edge to avoid timing races.
        for i in 0..25u64 {
            let event = make_recovery_event(
                now_ms - 240_000 + i * 5_000, // -240s to -120s, 5s apart
                100_000 + i,
                corr.clone(),
            );
            ledger.record(&event).expect("failed to record event");
        }

        let mut last_alerted = HashSet::new();
        let alerts = check_for_actionable_alerts(&ledger, 300, &mut last_alerted);

        // P1-O1: Vec has at least 1 alert with correct fields
        assert!(
            !alerts.is_empty(),
            "Expected at least one actionable alert from 25 RecoveryClassifyCall events"
        );
        let alert = &alerts[0];
        assert_eq!(
            alert.recommended_action, "restart_with_resync",
            "ChainBreakLoop rule (h) signal_d should recommend restart_with_resync"
        );
        assert!(
            alert.fork_type.contains("ChainBreakLoop"),
            "fork_type should contain 'ChainBreakLoop', got: {}",
            alert.fork_type
        );
        assert!(
            !alert.incident_id.is_empty(),
            "incident_id should be non-empty"
        );
        assert!(
            !alert.evidence_event_ids.is_empty(),
            "evidence_event_ids should be non-empty"
        );

        // P1-O2: last_alerted was mutated
        assert!(
            !last_alerted.is_empty(),
            "last_alerted should contain the correlation key after alert"
        );
    }

    // -----------------------------------------------------------------------
    // P2: Dedup — same ledger + same last_alerted -> empty
    //     IP2: pre-populated last_alerted
    // -----------------------------------------------------------------------
    #[test]
    fn p2_dedup_suppresses_repeat_alerts() {
        let (_dir, ledger) = make_ledger();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let corr = Some(CorrelationKey {
            divergence_height: Some(200_000),
            canonical_hash: Some("canon2".to_string()),
            fork_hash: Some("fork2".to_string()),
        });

        // 25 events, same timing margins as P1
        for i in 0..25u64 {
            let event =
                make_recovery_event(now_ms - 240_000 + i * 5_000, 200_000 + i, corr.clone());
            ledger.record(&event).expect("failed to record event");
        }

        let mut last_alerted = HashSet::new();

        // First call: should return alerts
        let alerts_1 = check_for_actionable_alerts(&ledger, 300, &mut last_alerted);
        assert!(!alerts_1.is_empty(), "First call should return alerts");

        let alerted_len = last_alerted.len();

        // P2-O1: Second call with same last_alerted -> empty
        let alerts_2 = check_for_actionable_alerts(&ledger, 300, &mut last_alerted);
        assert!(
            alerts_2.is_empty(),
            "Second call should return empty (deduped)"
        );

        // P2-O2: last_alerted unchanged
        assert_eq!(
            last_alerted.len(),
            alerted_len,
            "last_alerted should not grow on dedup"
        );
    }

    // -----------------------------------------------------------------------
    // P3: No actionable events (only BlockApplied) -> empty
    //     IP3: only BlockApplied events
    // -----------------------------------------------------------------------
    #[test]
    fn p3_no_actionable_events_returns_empty() {
        let (_dir, ledger) = make_ledger();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // Only canonical BlockApplied events — classifier returns Unknown/normal
        for i in 0..10u64 {
            let event = make_block_applied(now_ms - 240_000 + i * 20_000, 50_000 + i);
            ledger.record(&event).expect("failed to record event");
        }

        let mut last_alerted = HashSet::new();
        let alerts = check_for_actionable_alerts(&ledger, 300, &mut last_alerted);

        // P3-O1: empty
        assert!(
            alerts.is_empty(),
            "BlockApplied-only events should not trigger alerts"
        );

        // P3-O2: last_alerted empty
        assert!(
            last_alerted.is_empty(),
            "last_alerted should be empty when no alerts fire"
        );
    }

    // -----------------------------------------------------------------------
    // P4: TipRaceNatural (normal_operation) filtered out
    //     IP4: ForkBlockReceived + low-latency BlockApplied
    // -----------------------------------------------------------------------
    #[test]
    fn p4_normal_operation_not_actionable() {
        let (_dir, ledger) = make_ledger();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // A single ForkBlockReceived + matching BlockApplied with low latency
        // -> classifier should match TipRaceNatural -> recommended_action = "normal_operation"
        let applied = make_block_applied(now_ms - 60_000, 300_000);
        let fork = make_fork_natural(now_ms - 59_000, 300_000);

        ledger.record(&applied).expect("failed to record applied");
        ledger.record(&fork).expect("failed to record fork");

        let mut last_alerted = HashSet::new();
        let alerts = check_for_actionable_alerts(&ledger, 300, &mut last_alerted);

        // P4-O1: empty — normal_operation is not actionable
        assert!(
            alerts.is_empty(),
            "normal_operation should be filtered out as non-actionable"
        );

        // P4-O2: last_alerted empty
        assert!(
            last_alerted.is_empty(),
            "last_alerted should be empty when only normal_operation fires"
        );
    }
}
