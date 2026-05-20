//! `doli forks` subcommand — fork-diagnostic observability CLI.
//!
//! Calls `getForkDiagnostic` RPC and renders the `DiagnosticBundle` as JSON
//! (default) or human-readable text (`--human`).

use anyhow::Result;
use serde::Serialize;
use storage::diagnostic_ledger::types::DiagnosticBundle;

use crate::rpc_client::RpcClient;

// ---------------------------------------------------------------------------
// ProducerAggregate — public struct for by-producer output
// ---------------------------------------------------------------------------

/// Per-producer fork-event count, sorted by count descending.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProducerAggregate {
    pub producer_pubkey: String,
    pub count: u64,
}

// ---------------------------------------------------------------------------
// parse_duration — convert human duration strings to seconds
// ---------------------------------------------------------------------------

/// Parse a duration string like "1h", "30m", "5s" into seconds.
///
/// Supported suffixes: `h` (hours, * 3600), `m` (minutes, * 60), `s` (seconds).
/// Rejects unsupported suffixes like "1y", non-numeric prefixes, and empty input.
pub(crate) fn parse_duration(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("duration string is empty".to_string());
    }
    if s.len() < 2 {
        return Err(format!(
            "invalid duration: '{}' (expected e.g. 1h, 30m, 5s)",
            s
        ));
    }
    let (num_str, suffix) = s.split_at(s.len() - 1);
    let number: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid duration: '{}' (non-numeric prefix)", s))?;

    match suffix.chars().next().unwrap() {
        'h' => Ok(number * 3600),
        'm' => Ok(number * 60),
        's' => Ok(number),
        other => Err(format!(
            "unsupported duration suffix '{}' in '{}' (use h, m, or s)",
            other, s
        )),
    }
}

// ---------------------------------------------------------------------------
// build_rpc_params — construct JSON-RPC params for getForkDiagnostic
// ---------------------------------------------------------------------------

/// Build the JSON params object for the `getForkDiagnostic` RPC call.
pub(crate) fn build_rpc_params(
    window_secs: u64,
    fork_event_id: Option<&str>,
    limit: Option<u64>,
) -> serde_json::Value {
    let mut params = serde_json::json!({
        "window_secs": window_secs,
    });
    if let Some(id) = fork_event_id {
        params["fork_event_id"] = serde_json::Value::String(id.to_string());
    }
    if let Some(l) = limit {
        params["limit"] = serde_json::Value::Number(serde_json::Number::from(l));
    }
    params
}

// ---------------------------------------------------------------------------
// render_human — human-readable output for DiagnosticBundle
// ---------------------------------------------------------------------------

/// Render a `DiagnosticBundle` as human-readable text with section headers.
///
/// Sections: Events, Classification, Baseline, Health.
pub(crate) fn render_human(bundle: &DiagnosticBundle) -> String {
    let mut out = String::new();

    // === Health ===
    out.push_str("=== Health ===\n");
    out.push_str(&format!(
        "  ledger_available: {}\n",
        bundle.health.ledger_available
    ));
    out.push_str(&format!(
        "  events_written: {}  dropped: {}\n",
        bundle.health.events_written_total, bundle.health.events_dropped_total
    ));
    if let Some(hb) = bundle.health.last_heartbeat_ms {
        out.push_str(&format!("  last_heartbeat_ms: {}\n", hb));
    }

    // === Events ===
    out.push_str("\n=== Events ===\n");
    if bundle.events.is_empty() {
        out.push_str("  (no events in window)\n");
    } else {
        out.push_str(&format!("  total: {}\n", bundle.events.len()));
        for evt in &bundle.events {
            out.push_str(&format!(
                "  [{}] {:?} h={:?} id={}\n",
                evt.timestamp_ms,
                evt.kind,
                evt.height.unwrap_or(0),
                evt.event_id
            ));
        }
    }
    out.push_str(&format!(
        "  fork_events_in_window: {}\n",
        bundle.fork_summary.fork_events_in_window
    ));

    // === Classification ===
    out.push_str("\n=== Classification ===\n");
    match &bundle.classification {
        None => {
            out.push_str(
                "  No classification available (not enough evidence or no fork events).\n",
            );
        }
        Some(clf) => {
            match &clf.fork_type {
                storage::diagnostic_ledger::types::ForkType::Unknown {
                    reason_unknown,
                    evidence_event_ids,
                } => {
                    out.push_str("  type: Unknown\n");
                    out.push_str(&format!("  reason_unknown: {}\n", reason_unknown));
                    out.push_str(&format!("  evidence_event_ids: {:?}\n", evidence_event_ids));
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
                "  evidence_event_ids: {:?}\n",
                clf.evidence_event_ids
            ));
        }
    }

    // === Baseline ===
    out.push_str("\n=== Baseline ===\n");
    out.push_str(&format!(
        "  fork_events_per_hour (current): {:.2}\n",
        bundle.baseline.fork_events_per_hour_current
    ));
    out.push_str(&format!(
        "  fork_events_per_hour (24h avg): {:.2}\n",
        bundle.baseline.fork_events_per_hour_24h_avg
    ));
    out.push_str(&format!("  delta: {:.1}%\n", bundle.baseline.delta_pct));

    out
}

// ---------------------------------------------------------------------------
// aggregate_by_producer — group fork events by producer, sorted desc
// ---------------------------------------------------------------------------

/// Aggregate fork events by producer public key, sorted by count descending.
///
/// Uses `fork_summary.by_producer` from the bundle. Returns a Vec sorted
/// by count in descending order for display.
pub(crate) fn aggregate_by_producer(bundle: &DiagnosticBundle) -> Vec<ProducerAggregate> {
    let mut entries: Vec<ProducerAggregate> = bundle
        .fork_summary
        .by_producer
        .iter()
        .map(|(k, v)| ProducerAggregate {
            producer_pubkey: k.clone(),
            count: *v,
        })
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.count));
    entries
}

// ---------------------------------------------------------------------------
// cmd_forks — main entry point for the `doli forks` subcommand
// ---------------------------------------------------------------------------

/// Execute the `doli forks` command.
///
/// Calls `getForkDiagnostic` RPC and renders the result as JSON or human text.
pub(crate) async fn cmd_forks(
    rpc_endpoint: &str,
    last: Option<String>,
    human: bool,
    explain: bool,
    by_producer: bool,
) -> Result<()> {
    let duration_str = last.as_deref().unwrap_or("1h");
    let window_secs =
        parse_duration(duration_str).map_err(|e| anyhow::anyhow!("invalid --last value: {}", e))?;

    let client = RpcClient::new(rpc_endpoint);

    // If --explain, first fetch a short window to find the most recent fork event
    let fork_event_id = if explain {
        let initial_params = build_rpc_params(window_secs, None, Some(100));
        let initial_bundle: DiagnosticBundle = client
            .call_raw("getForkDiagnostic", initial_params)
            .await
            .map_err(|e| anyhow::anyhow!("RPC unavailable: {}", e))
            .and_then(|v| {
                serde_json::from_value(v)
                    .map_err(|e| anyhow::anyhow!("failed to parse bundle: {}", e))
            })?;

        // Find the most recent fork-relevant event
        let fork_id = initial_bundle
            .events
            .iter()
            .rev()
            .find(|e| {
                matches!(
                    e.kind,
                    storage::diagnostic_ledger::types::EventKind::ForkBlockReceived
                        | storage::diagnostic_ledger::types::EventKind::BlockRejected
                )
            })
            .map(|e| e.event_id.clone());

        if fork_id.is_none() {
            println!("No fork events found in the last {}.", duration_str);
            return Ok(());
        }
        fork_id
    } else {
        None
    };

    let params = build_rpc_params(window_secs, fork_event_id.as_deref(), None);
    let bundle: DiagnosticBundle = client
        .call_raw("getForkDiagnostic", params)
        .await
        .map_err(|e| anyhow::anyhow!("RPC unavailable: {}", e))
        .and_then(|v| {
            serde_json::from_value(v).map_err(|e| anyhow::anyhow!("failed to parse bundle: {}", e))
        })?;

    if by_producer {
        let aggregated = aggregate_by_producer(&bundle);
        if human {
            println!("=== Fork Events by Producer ===");
            for entry in &aggregated {
                println!("  {}  count={}", entry.producer_pubkey, entry.count);
            }
        } else {
            println!("{}", serde_json::to_string_pretty(&aggregated)?);
        }
        return Ok(());
    }

    if human {
        print!("{}", render_human(&bundle));
    } else {
        println!("{}", serde_json::to_string_pretty(&bundle)?);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration_hours() {
        assert_eq!(parse_duration("1h").unwrap(), 3600);
        assert_eq!(parse_duration("24h").unwrap(), 86400);
    }

    #[test]
    fn test_parse_duration_minutes() {
        assert_eq!(parse_duration("30m").unwrap(), 1800);
        assert_eq!(parse_duration("1m").unwrap(), 60);
    }

    #[test]
    fn test_parse_duration_seconds() {
        assert_eq!(parse_duration("5s").unwrap(), 5);
        assert_eq!(parse_duration("60s").unwrap(), 60);
    }

    #[test]
    fn test_parse_duration_rejects_invalid() {
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("1y").is_err());
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn test_build_rpc_params_default() {
        let p = build_rpc_params(3600, None, None);
        assert_eq!(p["window_secs"], 3600);
        assert!(p.get("fork_event_id").is_none());
    }

    #[test]
    fn test_build_rpc_params_with_event_id() {
        let p = build_rpc_params(1800, Some("01HY1234ABCD0001"), None);
        assert_eq!(p["window_secs"], 1800);
        assert_eq!(p["fork_event_id"], "01HY1234ABCD0001");
    }

    #[test]
    fn test_render_human_empty_bundle() {
        use std::collections::HashMap;
        use storage::diagnostic_ledger::types::*;

        let bundle = DiagnosticBundle {
            schema_version: 1,
            node_peer_id: "test".to_string(),
            query_timestamp_ms: 0,
            events: vec![],
            fork_summary: ForkSummary {
                fork_events_in_window: 0,
                by_producer: HashMap::new(),
                by_event_kind: HashMap::new(),
                first_fork_height: None,
                last_fork_height: None,
            },
            classification: None,
            baseline: BaselineComparison {
                fork_events_per_hour_current: 0.0,
                fork_events_per_hour_24h_avg: 0.0,
                delta_pct: 0.0,
            },
            health: DiagnosticHealth {
                ledger_available: true,
                events_written_total: 0,
                events_dropped_total: 0,
                last_heartbeat_ms: None,
            },
        };

        let text = render_human(&bundle);
        assert!(text.contains("Events"));
        assert!(text.contains("Classification"));
        assert!(text.contains("Baseline"));
        assert!(text.contains("no events"));
    }
}
