//! Fleet-level fork diagnostics CLI module.
//!
//! Extends `doli forks` with `--fleet <comma-list>` flag for querying multiple
//! peers via the `getFleetForkDiagnostic` RPC and rendering a fleet topology
//! summary.
//!
//! Split from `cmd_forks.rs` to stay under the 500-line modular budget.

use anyhow::Result;
use storage::diagnostic_ledger::fleet::{
    DivergencePoint, FleetBundle, FleetSummary, ForkGroup, PeerStatus,
};
use storage::diagnostic_ledger::types::ForkType;

use crate::rpc_client::RpcClient;

// ---------------------------------------------------------------------------
// Fleet peer cap (configurable via DOLI_FLEET_MAX_PEERS env)
// ---------------------------------------------------------------------------

/// Maximum number of peers accepted in the `--fleet` argument.
/// Configurable via `DOLI_FLEET_MAX_PEERS` environment variable.
fn fleet_max_peers() -> usize {
    std::env::var("DOLI_FLEET_MAX_PEERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50)
}

// ---------------------------------------------------------------------------
// parse_fleet_arg — parse and validate the --fleet comma-separated string
// ---------------------------------------------------------------------------

/// Parse a comma-separated list of RPC URLs from the `--fleet` flag.
///
/// Validation rules:
/// - Split on commas, trim whitespace from each entry.
/// - Reject empty entries (e.g. trailing commas).
/// - Each entry must start with `http://` or `https://`.
/// - Total entries must not exceed `fleet_max_peers()`.
pub(crate) fn parse_fleet_arg(s: &str) -> Result<Vec<String>, String> {
    let entries: Vec<String> = s
        .split(',')
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
        .collect();

    if entries.is_empty() {
        return Err("--fleet expects at least one RPC URL".to_string());
    }

    let max = fleet_max_peers();
    if entries.len() > max {
        return Err(format!(
            "--fleet expects at most {} comma-separated RPC URLs, got {}",
            max,
            entries.len()
        ));
    }

    for entry in &entries {
        if !entry.starts_with("http://") && !entry.starts_with("https://") {
            return Err(format!(
                "invalid RPC URL '{}': must start with http:// or https://",
                entry
            ));
        }
    }

    Ok(entries)
}

// ---------------------------------------------------------------------------
// render_fleet_human — human-readable fleet topology output
// ---------------------------------------------------------------------------

/// Render a `FleetBundle` as a human-readable fleet topology summary.
///
/// Sections: Fleet Health, Fleet Classifications, Fork Groups, Divergence
/// Points, Peer Status Detail.
pub(crate) fn render_fleet_human(bundle: &FleetBundle) -> String {
    let mut out = String::new();

    // === Fleet Health ===
    render_health_section(&mut out, &bundle.fleet_summary);

    // === Fleet Classifications ===
    render_classifications_section(&mut out, &bundle.fleet_summary);

    // === Fork Groups ===
    render_fork_groups_section(&mut out, &bundle.fork_groups);

    // === Divergence Points ===
    render_divergence_section(&mut out, &bundle.divergence_table);

    // === Peer Status Detail ===
    render_peer_status_section(&mut out, &bundle.queried_peers);

    out
}

fn render_health_section(out: &mut String, summary: &FleetSummary) {
    out.push_str("=== Fleet Health ===\n");
    out.push_str(&format!(
        "  total_peers_queried: {}\n",
        summary.total_peers_queried
    ));
    out.push_str(&format!("  peers_reachable: {}\n", summary.peers_reachable));
    out.push_str(&format!(
        "  peers_with_diagnostics: {}\n",
        summary.peers_with_diagnostics
    ));
    out.push_str(&format!(
        "  total_fork_events_across_fleet: {}\n",
        summary.total_fork_events_across_fleet
    ));
    out.push_str(&format!(
        "  unique_fork_groups: {}\n",
        summary.unique_fork_groups
    ));
}

fn render_classifications_section(out: &mut String, summary: &FleetSummary) {
    out.push_str("\n=== Fleet Classifications ===\n");
    match &summary.majority_classification {
        Some(ft) => {
            out.push_str(&format!("  majority: {}\n", format_fork_type(ft)));
        }
        None => {
            out.push_str("  majority: (none)\n");
        }
    }
    for (ft, count) in &summary.minority_classifications {
        out.push_str(&format!(
            "  minority: {} (count={})\n",
            format_fork_type(ft),
            count
        ));
    }
}

fn render_fork_groups_section(out: &mut String, groups: &[ForkGroup]) {
    out.push_str("\n=== Fork Groups ===\n");
    if groups.is_empty() {
        out.push_str("  (no fork groups)\n");
        return;
    }
    for (i, group) in groups.iter().enumerate() {
        let ck = &group.correlation_key;
        out.push_str(&format!(
            "  [{}] correlation: div_height={} canonical={} fork={}\n",
            i + 1,
            ck.divergence_height
                .map(|h| h.to_string())
                .unwrap_or_else(|| "?".to_string()),
            ck.canonical_hash
                .as_deref()
                .map(truncate_hash)
                .unwrap_or("?"),
            ck.fork_hash.as_deref().map(truncate_hash).unwrap_or("?"),
        ));

        out.push_str(&format!(
            "      on_canonical ({}): {}\n",
            group.peers_on_canonical.len(),
            join_or_none(&group.peers_on_canonical),
        ));
        out.push_str(&format!(
            "      on_fork ({}):      {}\n",
            group.peers_on_fork.len(),
            join_or_none(&group.peers_on_fork),
        ));
        out.push_str(&format!(
            "      undecided ({}):    {}\n",
            group.peers_undecided.len(),
            join_or_none(&group.peers_undecided),
        ));

        let clf = &group.representative_classification;
        out.push_str(&format!(
            "      classification: {} (conf={:.2})\n",
            format_fork_type(&clf.fork_type),
            clf.confidence
        ));
    }
}

fn render_divergence_section(out: &mut String, table: &[DivergencePoint]) {
    out.push_str("\n=== Divergence Points ===\n");
    if table.is_empty() {
        out.push_str("  (no divergence points)\n");
        return;
    }
    for dp in table {
        out.push_str(&format!(
            "  height={}: {} hashes\n",
            dp.height,
            dp.competing_hashes.len()
        ));
        for (hash, peers) in &dp.competing_hashes {
            out.push_str(&format!(
                "    {} endorsed by [{}]\n",
                truncate_hash(hash),
                peers.join(", ")
            ));
        }
        out.push_str(&format!("    action: {}\n", dp.recommended_action));
    }
}

fn render_peer_status_section(out: &mut String, peers: &[PeerStatus]) {
    out.push_str("\n=== Peer Status Detail ===\n");
    if peers.is_empty() {
        out.push_str("  (no peers)\n");
        return;
    }
    for ps in peers {
        let status = if let Some(ref err) = ps.error {
            format!("error: {}", err)
        } else if ps.bundle.is_some() {
            "bundle: yes".to_string()
        } else {
            "bundle: no".to_string()
        };
        out.push_str(&format!(
            "  {}  (latency {}ms)   {}\n",
            ps.rpc_url, ps.query_latency_ms, status
        ));
    }
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

/// Format a `ForkType` for human display (variant name only).
fn format_fork_type(ft: &ForkType) -> String {
    match ft {
        ForkType::TipRaceNatural => "TipRaceNatural".to_string(),
        ForkType::TipRaceHighLatency => "TipRaceHighLatency".to_string(),
        ForkType::ProducerEquivocation => "ProducerEquivocation".to_string(),
        ForkType::EpochBoundaryInvalid => "EpochBoundaryInvalid".to_string(),
        ForkType::PostSnapDeadTip => "PostSnapDeadTip".to_string(),
        ForkType::ValidationDisagreement => "ValidationDisagreement".to_string(),
        ForkType::RollbackLoop => "RollbackLoop".to_string(),
        ForkType::SnapSyncToMinorityFork => "SnapSyncToMinorityFork".to_string(),
        ForkType::Unknown { .. } => "Unknown".to_string(),
    }
}

/// Truncate a hex hash to `prefix...` form for display (first 8 chars).
fn truncate_hash(h: &str) -> &str {
    if h.len() > 8 {
        &h[..8]
    } else {
        h
    }
}

/// Join a list of strings with ", " or return "(none)".
fn join_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "(none)".to_string()
    } else {
        items.join(", ")
    }
}

// ---------------------------------------------------------------------------
// cmd_forks_fleet — main entry point
// ---------------------------------------------------------------------------

/// Execute the `doli forks --fleet <peers>` command.
///
/// Calls `getFleetForkDiagnostic` on the entry-point RPC (the `--rpc` flag
/// target, typically the seed) with the parsed peer list, then renders the
/// result as JSON or human-readable text.
pub(crate) async fn cmd_forks_fleet(
    rpc_endpoint: &str,
    peers: Vec<String>,
    last: Option<String>,
    human: bool,
) -> Result<()> {
    let duration_str = last.as_deref().unwrap_or("1h");
    let window_secs = crate::cmd_forks::parse_duration(duration_str)
        .map_err(|e| anyhow::anyhow!("invalid --last value: {}", e))?;

    let client = RpcClient::new(rpc_endpoint);

    let params = serde_json::json!({
        "peer_rpcs": peers,
        "window_secs": window_secs,
        "limit": 10_000u64,
    });

    let bundle: FleetBundle = client
        .call_raw("getFleetForkDiagnostic", params)
        .await
        .map_err(|e| anyhow::anyhow!("RPC unavailable: {}", e))
        .and_then(|v| {
            serde_json::from_value(v)
                .map_err(|e| anyhow::anyhow!("failed to parse fleet bundle: {}", e))
        })?;

    if human {
        print!("{}", render_fleet_human(&bundle));
    } else {
        println!("{}", serde_json::to_string_pretty(&bundle)?);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use storage::diagnostic_ledger::types::*;

    // -----------------------------------------------------------------------
    // Test fixture builder
    // -----------------------------------------------------------------------

    /// Build a minimal 3-peer FleetBundle with one fork group for testing.
    fn make_test_fleet_bundle() -> FleetBundle {
        let classification = Classification {
            fork_type: ForkType::PostSnapDeadTip,
            confidence: 0.80,
            evidence_event_ids: vec!["ev-1".to_string()],
            recommended_action: Some("auto_recover".to_string()),
            recommended_action_args: None,
        };

        let fork_group = ForkGroup {
            correlation_key: CorrelationKey {
                divergence_height: Some(110596),
                canonical_hash: Some("1f3aabcd12345678".to_string()),
                fork_hash: Some("8a7cef0987654321".to_string()),
            },
            peers_on_canonical: vec!["peer-0".to_string(), "peer-1".to_string()],
            peers_on_fork: vec!["peer-2".to_string()],
            peers_undecided: vec![],
            representative_classification: classification.clone(),
        };

        let divergence = DivergencePoint {
            height: 110596,
            competing_hashes: vec![
                (
                    "1f3aabcd12345678".to_string(),
                    vec!["peer-0".to_string(), "peer-1".to_string()],
                ),
                ("8a7cef0987654321".to_string(), vec!["peer-2".to_string()]),
            ],
            first_seen_ms: 1716200000000,
            recommended_action: "auto_recover".to_string(),
        };

        let empty_bundle = DiagnosticBundle {
            schema_version: 1,
            node_peer_id: "test".to_string(),
            query_timestamp_ms: 1716200000000,
            events: vec![],
            fork_summary: ForkSummary {
                fork_events_in_window: 5,
                by_producer: HashMap::new(),
                by_event_kind: HashMap::new(),
                first_fork_height: None,
                last_fork_height: None,
            },
            classification: Some(classification),
            baseline: BaselineComparison {
                fork_events_per_hour_current: 1.0,
                fork_events_per_hour_24h_avg: 0.5,
                delta_pct: 100.0,
            },
            health: DiagnosticHealth {
                ledger_available: true,
                events_written_total: 100,
                events_dropped_total: 0,
                last_heartbeat_ms: Some(1716200000000),
            },
        };

        FleetBundle {
            schema_version: 1,
            query_timestamp_ms: 1716200000000,
            queried_peers: vec![
                PeerStatus {
                    rpc_url: "peer-0".to_string(),
                    node_peer_id: Some("12D3KooWPeer0".to_string()),
                    bundle: Some(empty_bundle.clone()),
                    error: None,
                    query_latency_ms: 89,
                },
                PeerStatus {
                    rpc_url: "peer-1".to_string(),
                    node_peer_id: Some("12D3KooWPeer1".to_string()),
                    bundle: Some(empty_bundle.clone()),
                    error: None,
                    query_latency_ms: 124,
                },
                PeerStatus {
                    rpc_url: "peer-2".to_string(),
                    node_peer_id: Some("12D3KooWPeer2".to_string()),
                    bundle: Some(empty_bundle),
                    error: None,
                    query_latency_ms: 5000,
                },
            ],
            fleet_summary: FleetSummary {
                total_peers_queried: 3,
                peers_reachable: 3,
                peers_with_diagnostics: 3,
                total_fork_events_across_fleet: 15,
                unique_fork_groups: 1,
                majority_classification: Some(ForkType::PostSnapDeadTip),
                minority_classifications: vec![],
            },
            fork_groups: vec![fork_group],
            divergence_table: vec![divergence],
        }
    }

    // -----------------------------------------------------------------------
    // parse_fleet_arg tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_fleet_arg_splits_commas() {
        let result =
            parse_fleet_arg("http://127.0.0.1:8501,http://127.0.0.1:8502,http://127.0.0.1:8503")
                .unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "http://127.0.0.1:8501");
        assert_eq!(result[1], "http://127.0.0.1:8502");
        assert_eq!(result[2], "http://127.0.0.1:8503");
    }

    #[test]
    fn test_parse_fleet_arg_trims_whitespace() {
        let result = parse_fleet_arg(
            " http://127.0.0.1:8501 , http://127.0.0.1:8502 , http://127.0.0.1:8503 ",
        )
        .unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "http://127.0.0.1:8501");
        assert_eq!(result[1], "http://127.0.0.1:8502");
        assert_eq!(result[2], "http://127.0.0.1:8503");
    }

    #[test]
    fn test_parse_fleet_arg_rejects_empty_entries() {
        // All entries are empty after trimming
        let result = parse_fleet_arg(",,,");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("at least one"));
    }

    #[test]
    fn test_parse_fleet_arg_rejects_over_max_peers() {
        // Set max to 3 for this test
        std::env::set_var("DOLI_FLEET_MAX_PEERS", "3");
        let result = parse_fleet_arg("http://a:1,http://b:2,http://c:3,http://d:4");
        std::env::remove_var("DOLI_FLEET_MAX_PEERS");

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("at most 3"), "error was: {}", err);
        assert!(err.contains("got 4"), "error was: {}", err);
    }

    #[test]
    fn test_parse_fleet_arg_rejects_non_url_garbage() {
        let result = parse_fleet_arg("not-a-url,ftp://also-wrong");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("must start with http://"),
            "error was: {}",
            err
        );
    }

    // -----------------------------------------------------------------------
    // render_fleet_human tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_render_fleet_human_handles_empty_bundle() {
        let bundle = FleetBundle {
            schema_version: 1,
            query_timestamp_ms: 0,
            queried_peers: vec![],
            fleet_summary: FleetSummary {
                total_peers_queried: 0,
                peers_reachable: 0,
                peers_with_diagnostics: 0,
                total_fork_events_across_fleet: 0,
                unique_fork_groups: 0,
                majority_classification: None,
                minority_classifications: vec![],
            },
            fork_groups: vec![],
            divergence_table: vec![],
        };

        let text = render_fleet_human(&bundle);
        assert!(text.contains("Fleet Health"));
        assert!(text.contains("total_peers_queried: 0"));
        assert!(text.contains("Fleet Classifications"));
        assert!(text.contains("majority: (none)"));
        assert!(text.contains("no fork groups"));
        assert!(text.contains("no divergence points"));
        assert!(text.contains("no peers"));
    }

    #[test]
    fn test_render_fleet_human_shows_majority_classification() {
        let bundle = make_test_fleet_bundle();
        let text = render_fleet_human(&bundle);

        assert!(
            text.contains("majority: PostSnapDeadTip"),
            "text was:\n{}",
            text
        );
    }

    #[test]
    fn test_render_fleet_human_shows_divergence_points() {
        let bundle = make_test_fleet_bundle();
        let text = render_fleet_human(&bundle);

        assert!(text.contains("height=110596"), "text was:\n{}", text);
        assert!(text.contains("2 hashes"), "text was:\n{}", text);
        assert!(text.contains("1f3aabcd"), "text was:\n{}", text);
        assert!(text.contains("8a7cef09"), "text was:\n{}", text);
        assert!(text.contains("auto_recover"), "text was:\n{}", text);
    }

    #[test]
    fn test_render_fleet_human_lists_peer_statuses_with_latency() {
        let bundle = make_test_fleet_bundle();
        let text = render_fleet_human(&bundle);

        assert!(
            text.contains("peer-0  (latency 89ms)"),
            "text was:\n{}",
            text
        );
        assert!(
            text.contains("peer-1  (latency 124ms)"),
            "text was:\n{}",
            text
        );
        assert!(
            text.contains("peer-2  (latency 5000ms)"),
            "text was:\n{}",
            text
        );
        assert!(text.contains("bundle: yes"), "text was:\n{}", text);
    }

    #[test]
    fn test_render_fleet_human_shows_fork_group_detail() {
        let bundle = make_test_fleet_bundle();
        let text = render_fleet_human(&bundle);

        // Fork group header
        assert!(text.contains("[1] correlation:"), "text was:\n{}", text);
        assert!(text.contains("div_height=110596"), "text was:\n{}", text);
        // Peer partition
        assert!(text.contains("on_canonical (2):"), "text was:\n{}", text);
        assert!(text.contains("on_fork (1):"), "text was:\n{}", text);
        // Classification
        assert!(
            text.contains("PostSnapDeadTip (conf=0.80)"),
            "text was:\n{}",
            text
        );
    }

    #[test]
    fn test_render_fleet_human_shows_error_peers() {
        let bundle = FleetBundle {
            schema_version: 1,
            query_timestamp_ms: 0,
            queried_peers: vec![PeerStatus {
                rpc_url: "peer-7".to_string(),
                node_peer_id: None,
                bundle: None,
                error: Some("timeout".to_string()),
                query_latency_ms: 5000,
            }],
            fleet_summary: FleetSummary {
                total_peers_queried: 1,
                peers_reachable: 0,
                peers_with_diagnostics: 0,
                total_fork_events_across_fleet: 0,
                unique_fork_groups: 0,
                majority_classification: None,
                minority_classifications: vec![],
            },
            fork_groups: vec![],
            divergence_table: vec![],
        };

        let text = render_fleet_human(&bundle);
        assert!(
            text.contains("peer-7  (latency 5000ms)   error: timeout"),
            "text was:\n{}",
            text
        );
    }
}
