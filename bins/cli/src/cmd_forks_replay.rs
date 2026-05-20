//! `doli forks replay` subcommand — offline replay of historical log files.
//!
//! Parses a raw node log file through M3's `log_replay` parser, classifies
//! the resulting events, and outputs a `DiagnosticBundle` (JSON or human).
//! This is the CLI half of Phase 2a Deliverable 2 (GAP 2 closure).

use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

use storage::diagnostic_ledger::classifier;
use storage::diagnostic_ledger::log_replay::replay_log_file;
use storage::diagnostic_ledger::types::{
    BaselineComparison, Classification, DiagnosticBundle, DiagnosticHealth, EventKind,
    EventPayload, ForkSummary, ForkType,
};

// ---------------------------------------------------------------------------
// Fork-relevant event kinds (mirrors diagnostics.rs)
// ---------------------------------------------------------------------------

const FORK_RELEVANT_KINDS: &[EventKind] = &[
    EventKind::ForkBlockReceived,
    EventKind::BlockRejected,
    EventKind::RollbackStarted,
    EventKind::ReorgExecuted,
    EventKind::RecoveryClassifyCall,
    EventKind::SnapSyncFailed,
];

fn is_fork_relevant(kind: EventKind) -> bool {
    FORK_RELEVANT_KINDS.contains(&kind)
}

// ---------------------------------------------------------------------------
// build_fork_summary — mirrors crates/rpc/src/methods/diagnostics.rs
// ---------------------------------------------------------------------------

/// Build a `ForkSummary` from the replayed event list.
///
/// Mirrors `build_fork_summary` in `crates/rpc/src/methods/diagnostics.rs`.
/// Copied here because that function is private to the RPC crate (M5 can
/// factor out a shared helper if warranted).
fn build_fork_summary(
    events: &[storage::diagnostic_ledger::types::DiagnosticEvent],
) -> ForkSummary {
    let mut by_producer: HashMap<String, u64> = HashMap::new();
    let mut by_event_kind: HashMap<String, u64> = HashMap::new();
    let mut fork_heights: Vec<u64> = Vec::new();

    for ev in events {
        let kind_name = format!("{:?}", ev.kind);
        *by_event_kind.entry(kind_name).or_default() += 1;

        if let Some(producer) = extract_producer_pubkey(ev) {
            if is_fork_relevant(ev.kind) {
                *by_producer.entry(producer.to_string()).or_default() += 1;
            }
        }

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

// ---------------------------------------------------------------------------
// cmd_forks_replay — main entry point
// ---------------------------------------------------------------------------

/// Execute the `doli forks replay` command.
///
/// Replays a log file through the M3 parser, classifies the events, builds
/// a `DiagnosticBundle`, and outputs it as JSON (default) or human text.
pub(crate) async fn cmd_forks_replay(
    log_path: &Path,
    out_path: Option<&Path>,
    human: bool,
) -> Result<()> {
    let replayed = replay_log_file(log_path)
        .map_err(|e| anyhow::anyhow!("failed to replay log file: {}", e))?;

    if replayed.is_empty() {
        eprintln!("No diagnostic events found in {}", log_path.display());
        return Ok(());
    }

    let events: Vec<_> = replayed.iter().map(|r| r.event.clone()).collect();

    let fork_summary = build_fork_summary(&events);
    let classification = Some(classifier::classify(&events));

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let bundle = DiagnosticBundle {
        schema_version: 1,
        node_peer_id: "(log-replay)".to_string(),
        query_timestamp_ms: now_ms,
        events,
        fork_summary,
        classification,
        baseline: BaselineComparison {
            fork_events_per_hour_current: 0.0,
            fork_events_per_hour_24h_avg: 0.0,
            delta_pct: 0.0,
        },
        health: DiagnosticHealth {
            ledger_available: false,
            events_written_total: 0,
            events_dropped_total: 0,
            last_heartbeat_ms: None,
        },
    };

    // Write to file if --out specified
    if let Some(out) = out_path {
        let json = serde_json::to_string_pretty(&bundle)?;
        std::fs::write(out, &json)
            .map_err(|e| anyhow::anyhow!("failed to write {}: {}", out.display(), e))?;
        eprintln!("Bundle written to {}", out.display());
    }

    // Output to stdout
    if human {
        print!("{}", render_replay_bundle_human(&bundle));
    } else {
        println!("{}", serde_json::to_string_pretty(&bundle)?);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// render_replay_bundle_human — human-readable output for replay bundles
// ---------------------------------------------------------------------------

/// Render a replay-produced `DiagnosticBundle` as human-readable text.
///
/// Sections: Source, Summary, Events, Classification.
/// Omits Baseline and Health (always zero for replay).
pub(crate) fn render_replay_bundle_human(bundle: &DiagnosticBundle) -> String {
    let mut out = String::new();

    // === Source ===
    out.push_str("=== Replay Source ===\n");
    out.push_str(&format!("  node: {}\n", bundle.node_peer_id));
    out.push_str(&format!("  events parsed: {}\n", bundle.events.len()));

    // === Summary ===
    out.push_str("\n=== Summary ===\n");
    out.push_str(&format!(
        "  fork-relevant events: {}\n",
        bundle.fork_summary.fork_events_in_window
    ));
    if let Some(h) = bundle.fork_summary.first_fork_height {
        out.push_str(&format!("  first fork height: {}\n", h));
    }
    if let Some(h) = bundle.fork_summary.last_fork_height {
        out.push_str(&format!("  last fork height: {}\n", h));
    }
    if !bundle.fork_summary.by_event_kind.is_empty() {
        out.push_str("  by kind:\n");
        let mut kinds: Vec<_> = bundle.fork_summary.by_event_kind.iter().collect();
        kinds.sort_by_key(|(_, c)| std::cmp::Reverse(**c));
        for (kind, count) in kinds {
            out.push_str(&format!("    {}: {}\n", kind, count));
        }
    }

    // === Events (limited) ===
    out.push_str("\n=== Events ===\n");
    if bundle.events.is_empty() {
        out.push_str("  (no events)\n");
    } else {
        let show_count = bundle.events.len().min(50);
        for evt in &bundle.events[..show_count] {
            out.push_str(&format!(
                "  [{}] {:?} h={}\n",
                evt.timestamp_ms,
                evt.kind,
                evt.height.unwrap_or(0),
            ));
        }
        if bundle.events.len() > 50 {
            out.push_str(&format!(
                "  ... and {} more events\n",
                bundle.events.len() - 50
            ));
        }
    }

    // === Classification ===
    out.push_str("\n=== Classification ===\n");
    match &bundle.classification {
        None => {
            out.push_str("  No classification available.\n");
        }
        Some(clf) => {
            render_classification(&mut out, clf);
        }
    }

    out
}

fn render_classification(out: &mut String, clf: &Classification) {
    match &clf.fork_type {
        ForkType::Unknown {
            reason_unknown,
            evidence_event_ids,
        } => {
            out.push_str("  type: Unknown\n");
            out.push_str(&format!("  reason: {}\n", reason_unknown));
            out.push_str(&format!("  evidence_count: {}\n", evidence_event_ids.len()));
        }
        other => {
            out.push_str(&format!("  type: {:?}\n", other));
        }
    }
    out.push_str(&format!("  confidence: {:.2}\n", clf.confidence));
    if let Some(action) = &clf.recommended_action {
        out.push_str(&format!("  recommended_action: {}\n", action));
    }
    out.push_str(&format!(
        "  evidence_event_ids: {} total\n",
        clf.evidence_event_ids.len()
    ));
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // OUTPUT CONTRACT:
    // Functions under test:
    //   1. render_replay_bundle_human(bundle) -> String
    //   2. cmd_forks_replay(log_path, out_path, human) -> Result<()>
    //
    // Observable outputs:
    //   - render_replay_bundle_human: String containing section headers and data
    //   - cmd_forks_replay: side effects (file write, stdout), Result status
    //
    // Code paths x partitions:
    //   P1: render with classification present -> all sections rendered
    //   P2: render with Unknown classification -> reason + evidence_count shown
    //   P3: cmd_forks_replay writes JSON to file when out_path provided
    //
    // INPUT PARTITIONS:
    //   P1: DiagnosticBundle with named ForkType -> type shown as Debug format
    //   P2: DiagnosticBundle with Unknown ForkType -> reason_unknown shown
    //   P3: Log file with parseable events + out_path -> JSON file written

    fn make_test_bundle() -> DiagnosticBundle {
        DiagnosticBundle {
            schema_version: 1,
            node_peer_id: "(log-replay)".to_string(),
            query_timestamp_ms: 1716200000000,
            events: vec![storage::diagnostic_ledger::types::DiagnosticEvent {
                event_id: "01HY0000000000000001".to_string(),
                kind: EventKind::ChainBreakDetected,
                timestamp_ms: 1716200000000,
                height: Some(110367),
                correlation_key: None,
                caused_by_event_id: None,
                is_cascade_origin: false,
                payload: EventPayload::ChainBreakDetected {
                    expected_prev_hash: "aabb".to_string(),
                    actual_prev_hash: "ccdd".to_string(),
                    header_slot: 218898,
                    valid_so_far_count: 0,
                    from_peer_id: "(unknown)".to_string(),
                },
            }],
            fork_summary: ForkSummary {
                fork_events_in_window: 0,
                by_producer: HashMap::new(),
                by_event_kind: {
                    let mut m = HashMap::new();
                    m.insert("ChainBreakDetected".to_string(), 1);
                    m
                },
                first_fork_height: None,
                last_fork_height: None,
            },
            classification: Some(Classification {
                fork_type: ForkType::Unknown {
                    reason_unknown: "no rule matched".to_string(),
                    evidence_event_ids: vec!["01HY0000000000000001".to_string()],
                },
                confidence: 0.0,
                evidence_event_ids: vec!["01HY0000000000000001".to_string()],
                recommended_action: None,
                recommended_action_args: None,
            }),
            baseline: BaselineComparison {
                fork_events_per_hour_current: 0.0,
                fork_events_per_hour_24h_avg: 0.0,
                delta_pct: 0.0,
            },
            health: DiagnosticHealth {
                ledger_available: false,
                events_written_total: 0,
                events_dropped_total: 0,
                last_heartbeat_ms: None,
            },
        }
    }

    #[test]
    fn test_render_replay_bundle_human_includes_summary_and_classification() {
        let bundle = make_test_bundle();
        let text = render_replay_bundle_human(&bundle);

        assert!(
            text.contains("Replay Source"),
            "missing Replay Source section"
        );
        assert!(text.contains("(log-replay)"), "missing node peer id");
        assert!(text.contains("Summary"), "missing Summary section");
        assert!(text.contains("Events"), "missing Events section");
        assert!(
            text.contains("Classification"),
            "missing Classification section"
        );
        assert!(
            text.contains("Unknown"),
            "missing Unknown classification type"
        );
        assert!(text.contains("no rule matched"), "missing reason_unknown");
        assert!(text.contains("ChainBreakDetected"), "missing event kind");
    }

    #[tokio::test]
    async fn test_cmd_forks_replay_writes_json_output() {
        let dir = tempfile::tempdir().unwrap();

        // Create a small log file with parseable content
        let log_path = dir.path().join("test.log");
        let log_content = "\x1b[2m2026-05-01T00:00:01.000000Z\x1b[0m \x1b[32m INFO\x1b[0m \x1b[2mdoli_node::node::apply_block\x1b[0m\x1b[2m:\x1b[0m [BLOCK] Applied h=1 hash=aabb producer=0011 slot=10 txs=1 epoch=0\n\x1b[2m2026-05-01T00:00:02.000000Z\x1b[0m \x1b[33m WARN\x1b[0m \x1b[2mdoli_node::node::periodic\x1b[0m\x1b[2m:\x1b[0m [HEALTH] h=1 s=10 hash=aabb0000 | peers=5 best_peer_h=100 best_peer_s=200 net_tip_h=100 net_tip_s=200 | sync_fails=0 state=\"Idle\" | snap_epoch=0 snap_bonds=3 snap_producers=3\n";
        std::fs::write(&log_path, log_content).unwrap();

        // Write JSON output to file
        let out_path = dir.path().join("bundle.json");
        cmd_forks_replay(&log_path, Some(&out_path), false)
            .await
            .expect("replay should succeed");

        // Verify the JSON file was written
        assert!(out_path.exists(), "JSON output file should exist");
        let json_content = std::fs::read_to_string(&out_path).unwrap();
        let bundle: DiagnosticBundle =
            serde_json::from_str(&json_content).expect("output should be valid JSON");

        assert_eq!(bundle.schema_version, 1);
        assert_eq!(bundle.node_peer_id, "(log-replay)");
        assert_eq!(bundle.events.len(), 2);
        assert!(!bundle.health.ledger_available);
    }
}
