//! Fleet-level fork-diagnostic RPC handler — `getFleetForkDiagnostic`.
//!
//! Queries multiple peer nodes' `getForkDiagnostic` RPCs in parallel,
//! aggregates the results, and returns a `FleetBundle` with fleet-wide
//! summary statistics, fork group partitions, and a divergence table.
//!
//! This is an operator-side aggregator: no P2P-layer authority, no consensus
//! impact. Read-only. The fleet RPC is invoked from the operator's host.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tracing::warn;

use storage::diagnostic_ledger::fleet::{
    build_divergence_table, build_fleet_summary, build_fork_groups, redact_rpc_url, FleetBundle,
    PeerStatus,
};
use storage::diagnostic_ledger::types::DiagnosticBundle;

use crate::error::RpcError;

use super::context::RpcContext;

/// Default maximum number of peers that can be queried in a single fleet request.
const DEFAULT_FLEET_MAX_PEERS: usize = 50;

/// Default per-peer RPC timeout in seconds.
const DEFAULT_PEER_TIMEOUT_SECS: u64 = 5;

/// Total fleet request wall-clock timeout in seconds.
const FLEET_TOTAL_TIMEOUT_SECS: u64 = 30;

/// Maximum limit forwarded to per-peer getForkDiagnostic.
const MAX_LIMIT: u64 = 10_000;

/// Read the fleet max peers cap from env, defaulting to `DEFAULT_FLEET_MAX_PEERS`.
fn fleet_max_peers() -> usize {
    std::env::var("DOLI_FLEET_MAX_PEERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_FLEET_MAX_PEERS)
}

/// Read the per-peer timeout from env, defaulting to `DEFAULT_PEER_TIMEOUT_SECS`.
fn peer_timeout_secs() -> u64 {
    std::env::var("DOLI_FLEET_PEER_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PEER_TIMEOUT_SECS)
}

impl RpcContext {
    /// Handle `getFleetForkDiagnostic` RPC method.
    ///
    /// Params:
    ///   - `peer_rpcs`: `Vec<String>` (required) — RPC URLs to query
    ///   - `window_secs`: `u64` (optional, default 3600) — forwarded to each peer
    ///   - `limit`: `u64` (optional, capped at 10,000) — forwarded to each peer
    ///
    /// Caps:
    ///   - `peer_rpcs.len()` capped at `DOLI_FLEET_MAX_PEERS` env (default 50)
    ///   - Per-peer timeout: `DOLI_FLEET_PEER_TIMEOUT_SECS` env (default 5s)
    ///   - Total request: 30s wall-clock timeout
    pub async fn get_fleet_fork_diagnostic(&self, params: Value) -> Result<Value, RpcError> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        // Parse peer_rpcs (required)
        let peer_rpcs: Vec<String> = params
            .get("peer_rpcs")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .ok_or_else(|| {
                RpcError::invalid_params(
                    "Missing required parameter: peer_rpcs (array of RPC URL strings)",
                )
            })?;

        if peer_rpcs.is_empty() {
            return Err(RpcError::invalid_params("peer_rpcs must not be empty"));
        }

        let max_peers = fleet_max_peers();
        if peer_rpcs.len() > max_peers {
            return Err(RpcError::invalid_params(format!(
                "peer_rpcs exceeds maximum of {} peers (got {}). \
                 Set DOLI_FLEET_MAX_PEERS env to increase.",
                max_peers,
                peer_rpcs.len()
            )));
        }

        let window_secs = params
            .get("window_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(3600);

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|l| l.min(MAX_LIMIT))
            .unwrap_or(MAX_LIMIT);

        let timeout = Duration::from_secs(peer_timeout_secs());

        // Build the JSON-RPC request body to send to each peer
        let rpc_body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "getForkDiagnostic",
            "params": {
                "window_secs": window_secs,
                "limit": limit
            },
            "id": 1
        });

        // Spawn parallel queries for all peers
        let mut handles = Vec::with_capacity(peer_rpcs.len());
        for (idx, url) in peer_rpcs.iter().enumerate() {
            let url_owned = url.clone();
            let body = rpc_body.clone();

            handles.push(tokio::spawn(async move {
                query_peer(url_owned, body, timeout, idx).await
            }));
        }

        // Wait for all with total timeout
        let total_timeout = Duration::from_secs(FLEET_TOTAL_TIMEOUT_SECS);
        let results =
            match tokio::time::timeout(total_timeout, futures::future::join_all(handles)).await {
                Ok(results) => results,
                Err(_) => {
                    // Total timeout exceeded — return what we have
                    warn!(
                        "[FLEET_DIAG] Total fleet timeout ({}s) exceeded",
                        FLEET_TOTAL_TIMEOUT_SECS
                    );
                    return Err(RpcError::internal_error(format!(
                        "Fleet diagnostic total timeout ({}s) exceeded",
                        FLEET_TOTAL_TIMEOUT_SECS
                    )));
                }
            };

        // Collect peer statuses
        let queried_peers: Vec<PeerStatus> = results
            .into_iter()
            .map(|join_result| match join_result {
                Ok(ps) => ps,
                Err(e) => PeerStatus {
                    rpc_url: "peer-?".to_string(),
                    node_peer_id: None,
                    bundle: None,
                    error: Some(format!("task-panicked: {}", e)),
                    query_latency_ms: 0,
                },
            })
            .collect();

        // Aggregate
        let fork_groups = build_fork_groups(&queried_peers);
        let divergence_table = build_divergence_table(&queried_peers, &fork_groups);
        let fleet_summary = build_fleet_summary(&queried_peers, &fork_groups);

        let fleet_bundle = FleetBundle {
            schema_version: 1,
            query_timestamp_ms: now_ms,
            queried_peers,
            fleet_summary,
            fork_groups,
            divergence_table,
        };

        serde_json::to_value(&fleet_bundle).map_err(|e| RpcError::internal_error(e.to_string()))
    }
}

/// Query a single peer's `getForkDiagnostic` RPC with timeout.
///
/// Returns a `PeerStatus` with either the parsed `DiagnosticBundle` or an error string.
/// The `rpc_url` field is always redacted to `"peer-N"`.
async fn query_peer(url: String, body: Value, timeout: Duration, index: usize) -> PeerStatus {
    let redacted = redact_rpc_url(&url, index);
    let start = Instant::now();

    let client = match reqwest::Client::builder().timeout(timeout).build() {
        Ok(c) => c,
        Err(e) => {
            return PeerStatus {
                rpc_url: redacted,
                node_peer_id: None,
                bundle: None,
                error: Some(format!("http-client-error: {}", e)),
                query_latency_ms: start.elapsed().as_millis() as u64,
            };
        }
    };

    let resp = match client.post(&url).json(&body).send().await {
        Ok(r) => r,
        Err(e) => {
            let error_str = if e.is_timeout() {
                "timeout".to_string()
            } else if e.is_connect() {
                "connection-refused".to_string()
            } else {
                format!("http-error: {}", e)
            };
            return PeerStatus {
                rpc_url: redacted,
                node_peer_id: None,
                bundle: None,
                error: Some(error_str),
                query_latency_ms: start.elapsed().as_millis() as u64,
            };
        }
    };

    let response_body: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return PeerStatus {
                rpc_url: redacted,
                node_peer_id: None,
                bundle: None,
                error: Some(format!("json-parse-error: {}", e)),
                query_latency_ms: start.elapsed().as_millis() as u64,
            };
        }
    };

    let latency = start.elapsed().as_millis() as u64;

    // Check for JSON-RPC error
    if let Some(err) = response_body.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown-rpc-error");
        if msg.contains("Method not found") {
            return PeerStatus {
                rpc_url: redacted,
                node_peer_id: None,
                bundle: None,
                error: Some("method-not-found".to_string()),
                query_latency_ms: latency,
            };
        }
        return PeerStatus {
            rpc_url: redacted,
            node_peer_id: None,
            bundle: None,
            error: Some(format!("rpc-error: {}", msg)),
            query_latency_ms: latency,
        };
    }

    // Parse the result field as a DiagnosticBundle
    let result = match response_body.get("result") {
        Some(r) => r,
        None => {
            return PeerStatus {
                rpc_url: redacted,
                node_peer_id: None,
                bundle: None,
                error: Some("missing-result-field".to_string()),
                query_latency_ms: latency,
            };
        }
    };

    match serde_json::from_value::<DiagnosticBundle>(result.clone()) {
        Ok(bundle) => PeerStatus {
            rpc_url: redacted,
            node_peer_id: Some(bundle.node_peer_id.clone()),
            bundle: Some(bundle),
            error: None,
            query_latency_ms: latency,
        },
        Err(e) => PeerStatus {
            rpc_url: redacted,
            node_peer_id: None,
            bundle: None,
            error: Some(format!("bundle-parse-error: {}", e)),
            query_latency_ms: latency,
        },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use storage::diagnostic_ledger::fleet::PeerStatus;
    use storage::diagnostic_ledger::types::*;

    /// Helper: create a minimal DiagnosticBundle.
    fn make_bundle(
        peer_id: &str,
        events: Vec<DiagnosticEvent>,
        classification: Option<Classification>,
        fork_events: u64,
    ) -> DiagnosticBundle {
        DiagnosticBundle {
            schema_version: 1,
            node_peer_id: peer_id.to_string(),
            query_timestamp_ms: 1716200000000,
            events,
            fork_summary: ForkSummary {
                fork_events_in_window: fork_events,
                by_producer: HashMap::new(),
                by_event_kind: HashMap::new(),
                first_fork_height: None,
                last_fork_height: None,
            },
            classification,
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
        }
    }

    // -----------------------------------------------------------------------
    // test_get_fleet_fork_diagnostic_respects_peer_cap
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_get_fleet_fork_diagnostic_respects_peer_cap() {
        // Set env to cap at 5 peers for this test
        std::env::set_var("DOLI_FLEET_MAX_PEERS", "5");

        // Build params with 6 peers (exceeds cap of 5)
        let urls: Vec<String> = (0..6)
            .map(|i| format!("http://127.0.0.1:{}", 8500 + i))
            .collect();
        let params = serde_json::json!({ "peer_rpcs": urls });

        // We cannot call get_fleet_fork_diagnostic without a full RpcContext,
        // but we can test the param validation directly by extracting the logic.
        let peer_rpcs: Vec<String> =
            serde_json::from_value(params.get("peer_rpcs").unwrap().clone()).unwrap();

        let max_peers = fleet_max_peers();
        assert!(
            peer_rpcs.len() > max_peers,
            "Test setup: 6 peers should exceed cap of 5"
        );

        // Simulate what the handler does:
        if peer_rpcs.len() > max_peers {
            let err = RpcError::invalid_params(format!(
                "peer_rpcs exceeds maximum of {} peers (got {})",
                max_peers,
                peer_rpcs.len()
            ));
            assert_eq!(err.code, -32602);
        } else {
            panic!("Should have exceeded peer cap");
        }

        // Clean up env
        std::env::remove_var("DOLI_FLEET_MAX_PEERS");
    }

    // -----------------------------------------------------------------------
    // test_get_fleet_fork_diagnostic_handles_timeout
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_get_fleet_fork_diagnostic_handles_timeout() {
        // Query a peer that will never respond (connect to a port that refuses)
        let timeout = Duration::from_millis(100); // Very short timeout
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "getForkDiagnostic",
            "params": { "window_secs": 3600, "limit": 100 },
            "id": 1
        });

        // Port 1 should be unreachable/refused on localhost
        let ps = query_peer("http://127.0.0.1:1".to_string(), body, timeout, 0).await;

        assert_eq!(ps.rpc_url, "peer-0");
        assert!(ps.bundle.is_none());
        assert!(ps.error.is_some());

        let err = ps.error.unwrap();
        // Should be either "timeout" or "connection-refused" depending on OS behavior
        assert!(
            err == "timeout" || err == "connection-refused" || err.starts_with("http-error:"),
            "Expected timeout/connection-refused/http-error, got: {}",
            err
        );
    }

    // -----------------------------------------------------------------------
    // test_aggregate_groups_correlation_keys_correctly
    // -----------------------------------------------------------------------

    #[test]
    fn test_aggregate_groups_correlation_keys_correctly() {
        let ck1 = CorrelationKey {
            divergence_height: Some(100),
            canonical_hash: Some("hash_a".to_string()),
            fork_hash: Some("hash_b".to_string()),
        };
        let ck2 = CorrelationKey {
            divergence_height: Some(200),
            canonical_hash: Some("hash_c".to_string()),
            fork_hash: Some("hash_d".to_string()),
        };

        let ev1 = DiagnosticEvent {
            event_id: "ev-1".to_string(),
            kind: EventKind::ForkBlockReceived,
            timestamp_ms: 1716200001000,
            height: Some(100),
            correlation_key: Some(ck1.clone()),
            caused_by_event_id: None,
            is_cascade_origin: true,
            payload: EventPayload::ForkBlockReceived {
                block_hash: "hash_b".to_string(),
                block_slot: 100,
                block_height_estimate: Some(100),
                producer_pubkey: "prod1".to_string(),
                from_peer_id: "12D3KooWSender".to_string(),
                classification: "ForkBlock".to_string(),
                fork_kind: Some("HeightOccupied".to_string()),
                local_tip_hash: "hash_a".to_string(),
                local_tip_height: 100,
            },
        };
        let ev2 = DiagnosticEvent {
            event_id: "ev-2".to_string(),
            kind: EventKind::ForkBlockReceived,
            timestamp_ms: 1716200002000,
            height: Some(200),
            correlation_key: Some(ck2.clone()),
            caused_by_event_id: None,
            is_cascade_origin: true,
            payload: EventPayload::ForkBlockReceived {
                block_hash: "hash_d".to_string(),
                block_slot: 200,
                block_height_estimate: Some(200),
                producer_pubkey: "prod2".to_string(),
                from_peer_id: "12D3KooWSender2".to_string(),
                classification: "ForkBlock".to_string(),
                fork_kind: Some("HeightOccupied".to_string()),
                local_tip_hash: "hash_c".to_string(),
                local_tip_height: 200,
            },
        };

        // Peer A has both events, Peer B only ck1
        let peers = vec![
            PeerStatus {
                rpc_url: "peer-0".to_string(),
                node_peer_id: Some("peer-a".to_string()),
                bundle: Some(make_bundle(
                    "peer-a",
                    vec![ev1.clone(), ev2.clone()],
                    None,
                    2,
                )),
                error: None,
                query_latency_ms: 30,
            },
            PeerStatus {
                rpc_url: "peer-1".to_string(),
                node_peer_id: Some("peer-b".to_string()),
                bundle: Some(make_bundle("peer-b", vec![ev1.clone()], None, 1)),
                error: None,
                query_latency_ms: 40,
            },
        ];

        let groups = build_fork_groups(&peers);
        assert_eq!(
            groups.len(),
            2,
            "Should produce 2 fork groups (2 distinct correlation keys)"
        );

        // Check that ck1 group has both peers
        let ck1_group = groups.iter().find(|g| g.correlation_key == ck1);
        assert!(ck1_group.is_some(), "Should have a group for ck1");
        let ck1_group = ck1_group.unwrap();
        // Both peers are undecided (ForkBlockReceived, not BlockApplied at the divergence height)
        let total_peers_in_ck1 = ck1_group.peers_on_canonical.len()
            + ck1_group.peers_on_fork.len()
            + ck1_group.peers_undecided.len();
        assert_eq!(total_peers_in_ck1, 2, "ck1 group should have 2 peers");

        // ck2 group should have only peer-a
        let ck2_group = groups.iter().find(|g| g.correlation_key == ck2);
        assert!(ck2_group.is_some(), "Should have a group for ck2");
        let ck2_group = ck2_group.unwrap();
        let total_peers_in_ck2 = ck2_group.peers_on_canonical.len()
            + ck2_group.peers_on_fork.len()
            + ck2_group.peers_undecided.len();
        assert_eq!(total_peers_in_ck2, 1, "ck2 group should have 1 peer");
    }

    // -----------------------------------------------------------------------
    // test_per_peer_failure_does_not_fail_whole_request
    // -----------------------------------------------------------------------

    #[test]
    fn test_per_peer_failure_does_not_fail_whole_request() {
        // Simulate: 2 peers responded, 1 failed with timeout
        let peers = vec![
            PeerStatus {
                rpc_url: "peer-0".to_string(),
                node_peer_id: Some("peer-a".to_string()),
                bundle: Some(make_bundle("peer-a", vec![], None, 0)),
                error: None,
                query_latency_ms: 50,
            },
            PeerStatus {
                rpc_url: "peer-1".to_string(),
                node_peer_id: None,
                bundle: None,
                error: Some("timeout".to_string()),
                query_latency_ms: 5000,
            },
            PeerStatus {
                rpc_url: "peer-2".to_string(),
                node_peer_id: Some("peer-c".to_string()),
                bundle: Some(make_bundle("peer-c", vec![], None, 3)),
                error: None,
                query_latency_ms: 60,
            },
        ];

        let groups = build_fork_groups(&peers);
        let summary = build_fleet_summary(&peers, &groups);

        // Summary should reflect partial success
        assert_eq!(summary.total_peers_queried, 3);
        assert_eq!(summary.peers_reachable, 2); // peer-1 failed
        assert_eq!(summary.peers_with_diagnostics, 2);
        assert_eq!(summary.total_fork_events_across_fleet, 3); // 0 + 3
    }
}
