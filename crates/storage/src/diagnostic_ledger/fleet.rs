//! Fleet-level diagnostic types for cross-node fork observability.
//!
//! These types aggregate per-node `DiagnosticBundle` data across a fleet of
//! peers. The `FleetBundle` is the top-level response from the
//! `getFleetForkDiagnostic` RPC method.
//!
//! PII constraint: `PeerStatus.rpc_url` is redacted before serialization.
//! The `redact_rpc_url` helper replaces raw URLs with positional `"peer-N"`
//! labels so no IP addresses appear in the serialized output.

use serde::{Deserialize, Serialize};

use super::types::{Classification, CorrelationKey, DiagnosticBundle, ForkType};

// ---------------------------------------------------------------------------
// FleetBundle — top-level fleet diagnostic response
// ---------------------------------------------------------------------------

/// Aggregated diagnostic bundle across a fleet of peers.
///
/// Returned by `getFleetForkDiagnostic`. Contains per-peer results,
/// fleet-wide summary statistics, fork group partitioning, and a
/// divergence table highlighting heights where peers disagree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FleetBundle {
    /// Schema version for this fleet bundle format (currently 1).
    pub schema_version: u16,
    /// Wall-clock timestamp (ms since UNIX epoch) when the fleet query started.
    pub query_timestamp_ms: u64,
    /// Per-peer query results (success or failure).
    pub queried_peers: Vec<PeerStatus>,
    /// Fleet-wide aggregate statistics.
    pub fleet_summary: FleetSummary,
    /// Fork events grouped by correlation key across the fleet.
    pub fork_groups: Vec<ForkGroup>,
    /// Heights where peers disagree on the block hash.
    pub divergence_table: Vec<DivergencePoint>,
}

// ---------------------------------------------------------------------------
// PeerStatus — per-peer query result
// ---------------------------------------------------------------------------

/// Result of querying a single peer's `getForkDiagnostic` RPC.
///
/// `rpc_url` is redacted (replaced with `"peer-N"` label) in the serialized
/// `FleetBundle` to satisfy the PII constraint (no IP addresses in output).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeerStatus {
    /// Redacted RPC URL label (e.g., `"peer-0"`). Raw URLs are never serialized.
    pub rpc_url: String,
    /// Peer ID from the remote node's `DiagnosticBundle::node_peer_id`, if reachable.
    pub node_peer_id: Option<String>,
    /// The diagnostic bundle returned by the peer (None if RPC failed).
    pub bundle: Option<DiagnosticBundle>,
    /// Error string if the peer query failed (e.g., `"timeout"`, `"connection-refused"`).
    pub error: Option<String>,
    /// Round-trip latency of the RPC query in milliseconds.
    pub query_latency_ms: u64,
}

// ---------------------------------------------------------------------------
// FleetSummary — aggregate statistics
// ---------------------------------------------------------------------------

/// Aggregate statistics across all queried peers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FleetSummary {
    /// Total number of peers in the query list.
    pub total_peers_queried: u32,
    /// Number of peers that responded (HTTP success, regardless of diagnostics).
    pub peers_reachable: u32,
    /// Number of peers that returned a valid `DiagnosticBundle`.
    pub peers_with_diagnostics: u32,
    /// Sum of `fork_summary.fork_events_in_window` across all peer bundles.
    pub total_fork_events_across_fleet: u64,
    /// Number of distinct `ForkGroup`s (unique correlation keys).
    pub unique_fork_groups: u32,
    /// The most common `ForkType` across all peer classifications.
    pub majority_classification: Option<ForkType>,
    /// Fork types that are NOT the majority, with their occurrence counts.
    pub minority_classifications: Vec<(ForkType, u32)>,
}

// ---------------------------------------------------------------------------
// ForkGroup — correlation-key-grouped fork partition
// ---------------------------------------------------------------------------

/// A group of peers that observed the same fork episode (same correlation key).
///
/// Peers are partitioned into canonical/fork/undecided based on which block
/// hash they applied at the divergence height.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForkGroup {
    /// The correlation key linking these events (divergence height + hashes).
    pub correlation_key: CorrelationKey,
    /// Peer IDs that applied the canonical block at divergence height.
    pub peers_on_canonical: Vec<String>,
    /// Peer IDs that applied the fork block at divergence height.
    pub peers_on_fork: Vec<String>,
    /// Peer IDs that have the correlation key but no `BlockApplied` at that height.
    pub peers_undecided: Vec<String>,
    /// Representative classification from the majority of peers in this group.
    pub representative_classification: Classification,
}

// ---------------------------------------------------------------------------
// DivergencePoint — height with competing block hashes
// ---------------------------------------------------------------------------

/// A blockchain height where multiple distinct block hashes were observed
/// across the fleet, indicating an active or historical fork.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DivergencePoint {
    /// The block height where divergence was observed.
    pub height: u64,
    /// Each distinct block hash and the peer IDs that endorsed it.
    pub competing_hashes: Vec<(String, Vec<String>)>,
    /// Timestamp (ms since epoch) of the earliest observation of this divergence.
    pub first_seen_ms: u64,
    /// Suggested operator action: `"manual_intervention"`, `"auto_recover"`, or `"watch"`.
    pub recommended_action: String,
}

// ---------------------------------------------------------------------------
// Redaction helper
// ---------------------------------------------------------------------------

/// Redact an RPC URL by replacing it with a positional `"peer-N"` label.
///
/// This ensures no IP addresses (dotted-quad patterns like `127.0.0.1`) appear
/// in the serialized `FleetBundle`. The caller provides the zero-based index.
///
/// # Examples
/// ```
/// use storage::diagnostic_ledger::fleet::redact_rpc_url;
/// assert_eq!(redact_rpc_url("http://127.0.0.1:8500", 0), "peer-0");
/// assert_eq!(redact_rpc_url("http://10.0.0.1:8501/rpc", 3), "peer-3");
/// ```
pub fn redact_rpc_url(_url: &str, index: usize) -> String {
    format!("peer-{}", index)
}

// ---------------------------------------------------------------------------
// Aggregation helpers
// ---------------------------------------------------------------------------

/// Build fork groups from a set of peer statuses by grouping events that share
/// the same `CorrelationKey`.
///
/// For each unique correlation key found across all peer bundles:
/// 1. Collect all peers that have events with this key.
/// 2. Check each peer's `BlockApplied` events at the divergence height.
/// 3. Partition peers into canonical/fork/undecided based on hash match.
pub fn build_fork_groups(peers: &[PeerStatus]) -> Vec<ForkGroup> {
    use super::types::{EventKind, EventPayload};
    use std::collections::HashMap;

    // Collect all unique correlation keys and the peers that have them
    let mut key_to_peers: HashMap<String, Vec<(String, &DiagnosticBundle)>> = HashMap::new();

    for ps in peers {
        let peer_id = ps
            .node_peer_id
            .clone()
            .unwrap_or_else(|| ps.rpc_url.clone());
        if let Some(ref bundle) = ps.bundle {
            for event in &bundle.events {
                if let Some(ref ck) = event.correlation_key {
                    let key_str = correlation_key_string(ck);
                    key_to_peers
                        .entry(key_str)
                        .or_default()
                        .push((peer_id.clone(), bundle));
                }
            }
        }
    }

    // Deduplicate peers per key
    let mut groups = Vec::new();
    for (key_str, peer_bundles) in &key_to_peers {
        // Parse correlation key back
        let parts: Vec<&str> = key_str.splitn(3, '|').collect();
        if parts.len() < 3 {
            continue;
        }
        let div_height = parts[0].parse::<u64>().ok();
        let canonical_hash = if parts[1].is_empty() {
            None
        } else {
            Some(parts[1].to_string())
        };
        let fork_hash = if parts[2].is_empty() {
            None
        } else {
            Some(parts[2].to_string())
        };

        let ck = CorrelationKey {
            divergence_height: div_height,
            canonical_hash: canonical_hash.clone(),
            fork_hash: fork_hash.clone(),
        };

        let mut peers_on_canonical = Vec::new();
        let mut peers_on_fork = Vec::new();
        let mut peers_undecided = Vec::new();
        let mut seen_peers: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut classifications = Vec::new();

        for (peer_id, bundle) in peer_bundles {
            if !seen_peers.insert(peer_id.clone()) {
                continue;
            }

            // Check if this peer has a BlockApplied at divergence_height
            if let Some(h) = div_height {
                let mut found_block = false;
                for ev in &bundle.events {
                    if ev.kind == EventKind::BlockApplied && ev.height == Some(h) {
                        if let EventPayload::BlockApplied { ref block_hash, .. } = ev.payload {
                            found_block = true;
                            if Some(block_hash.clone()) == canonical_hash {
                                peers_on_canonical.push(peer_id.clone());
                            } else if Some(block_hash.clone()) == fork_hash {
                                peers_on_fork.push(peer_id.clone());
                            } else {
                                peers_undecided.push(peer_id.clone());
                            }
                            break;
                        }
                    }
                }
                if !found_block {
                    peers_undecided.push(peer_id.clone());
                }
            } else {
                peers_undecided.push(peer_id.clone());
            }

            if let Some(ref c) = bundle.classification {
                classifications.push(c.clone());
            }
        }

        // Pick representative classification (first available or unknown)
        let representative = classifications
            .into_iter()
            .next()
            .unwrap_or(Classification {
                fork_type: ForkType::Unknown {
                    reason_unknown: "no peer classification available".to_string(),
                    evidence_event_ids: vec![],
                },
                confidence: 0.0,
                evidence_event_ids: vec![],
                recommended_action: None,
                recommended_action_args: None,
            });

        groups.push(ForkGroup {
            correlation_key: ck,
            peers_on_canonical,
            peers_on_fork,
            peers_undecided,
            representative_classification: representative,
        });
    }

    groups
}

/// Build a divergence table from peer statuses by finding heights where
/// multiple distinct block hashes were applied across peers.
pub fn build_divergence_table(
    peers: &[PeerStatus],
    fork_groups: &[ForkGroup],
) -> Vec<DivergencePoint> {
    use super::types::{EventKind, EventPayload};
    use std::collections::HashMap;

    // Collect (height, hash) -> (peer_ids, first_seen_ms)
    let mut height_hashes: HashMap<u64, HashMap<String, (Vec<String>, u64)>> = HashMap::new();

    for ps in peers {
        let peer_id = ps
            .node_peer_id
            .clone()
            .unwrap_or_else(|| ps.rpc_url.clone());
        if let Some(ref bundle) = ps.bundle {
            for ev in &bundle.events {
                if ev.kind == EventKind::BlockApplied {
                    if let (Some(h), EventPayload::BlockApplied { ref block_hash, .. }) =
                        (ev.height, &ev.payload)
                    {
                        let entry = height_hashes
                            .entry(h)
                            .or_default()
                            .entry(block_hash.clone())
                            .or_insert_with(|| (Vec::new(), ev.timestamp_ms));
                        entry.0.push(peer_id.clone());
                        if ev.timestamp_ms < entry.1 {
                            entry.1 = ev.timestamp_ms;
                        }
                    }
                }
            }
        }
    }

    // Only heights with >1 distinct hash are divergence points
    let mut table = Vec::new();
    for (height, hashes) in &height_hashes {
        if hashes.len() <= 1 {
            continue;
        }

        let first_seen_ms = hashes.values().map(|(_, ts)| *ts).min().unwrap_or(0);

        let competing_hashes: Vec<(String, Vec<String>)> = hashes
            .iter()
            .map(|(hash, (peers, _))| (hash.clone(), peers.clone()))
            .collect();

        let action = determine_recommended_action(*height, fork_groups);

        table.push(DivergencePoint {
            height: *height,
            competing_hashes,
            first_seen_ms,
            recommended_action: action,
        });
    }

    table.sort_by_key(|dp| dp.height);
    table
}

/// Determine the recommended action for a divergence point based on the
/// classification of any fork group that covers this height.
fn determine_recommended_action(height: u64, fork_groups: &[ForkGroup]) -> String {
    for group in fork_groups {
        if group.correlation_key.divergence_height == Some(height) {
            return match &group.representative_classification.fork_type {
                ForkType::ProducerEquivocation | ForkType::EpochBoundaryInvalid => {
                    "manual_intervention".to_string()
                }
                ForkType::PostSnapDeadTip => "auto_recover".to_string(),
                _ => "watch".to_string(),
            };
        }
    }
    "watch".to_string()
}

/// Compute the fleet summary from peer statuses and fork groups.
pub fn build_fleet_summary(peers: &[PeerStatus], fork_groups: &[ForkGroup]) -> FleetSummary {
    use std::collections::HashMap;

    let total_peers_queried = peers.len() as u32;
    let peers_reachable = peers.iter().filter(|p| p.error.is_none()).count() as u32;
    let peers_with_diagnostics = peers.iter().filter(|p| p.bundle.is_some()).count() as u32;

    let total_fork_events_across_fleet: u64 = peers
        .iter()
        .filter_map(|p| p.bundle.as_ref())
        .map(|b| b.fork_summary.fork_events_in_window)
        .sum();

    let unique_fork_groups = fork_groups.len() as u32;

    // Tally fork types from peer classifications
    let mut type_counts: HashMap<String, (ForkType, u32)> = HashMap::new();
    for ps in peers {
        if let Some(ref bundle) = ps.bundle {
            if let Some(ref classification) = bundle.classification {
                let key = fork_type_discriminant(&classification.fork_type);
                let entry = type_counts
                    .entry(key)
                    .or_insert_with(|| (classification.fork_type.clone(), 0));
                entry.1 += 1;
            }
        }
    }

    // Find majority
    let majority = type_counts
        .values()
        .max_by_key(|(_, count)| *count)
        .map(|(ft, _)| ft.clone());

    let majority_key = majority.as_ref().map(fork_type_discriminant);

    let minority_classifications: Vec<(ForkType, u32)> = type_counts
        .into_iter()
        .filter(|(k, _)| Some(k.clone()) != majority_key)
        .map(|(_, v)| v)
        .collect();

    FleetSummary {
        total_peers_queried,
        peers_reachable,
        peers_with_diagnostics,
        total_fork_events_across_fleet,
        unique_fork_groups,
        majority_classification: majority,
        minority_classifications,
    }
}

/// Produce a stable string discriminant for a `ForkType` variant (ignoring inner data).
fn fork_type_discriminant(ft: &ForkType) -> String {
    match ft {
        ForkType::TipRaceNatural => "TipRaceNatural".to_string(),
        ForkType::TipRaceHighLatency => "TipRaceHighLatency".to_string(),
        ForkType::ProducerEquivocation => "ProducerEquivocation".to_string(),
        ForkType::EpochBoundaryInvalid => "EpochBoundaryInvalid".to_string(),
        ForkType::PostSnapDeadTip => "PostSnapDeadTip".to_string(),
        ForkType::ValidationDisagreement => "ValidationDisagreement".to_string(),
        ForkType::RollbackLoop => "RollbackLoop".to_string(),
        ForkType::SnapSyncToMinorityFork => "SnapSyncToMinorityFork".to_string(),
        ForkType::ChainBreakLoop { .. } => "ChainBreakLoop".to_string(),
        ForkType::Unknown { .. } => "Unknown".to_string(),
    }
}

/// Serialize a `CorrelationKey` to a stable string for use as a hash map key.
fn correlation_key_string(ck: &CorrelationKey) -> String {
    format!(
        "{}|{}|{}",
        ck.divergence_height.unwrap_or(0),
        ck.canonical_hash.as_deref().unwrap_or(""),
        ck.fork_hash.as_deref().unwrap_or("")
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic_ledger::types::*;
    use std::collections::HashMap;

    /// Helper: create a minimal DiagnosticBundle for testing.
    fn make_bundle(
        peer_id: &str,
        events: Vec<DiagnosticEvent>,
        classification: Option<Classification>,
    ) -> DiagnosticBundle {
        DiagnosticBundle {
            schema_version: 1,
            node_peer_id: peer_id.to_string(),
            query_timestamp_ms: 1716200000000,
            events,
            fork_summary: ForkSummary {
                fork_events_in_window: 0,
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

    /// Helper: create a BlockApplied event at a given height with a given hash.
    fn make_block_applied(
        event_id: &str,
        height: u64,
        hash: &str,
        producer: &str,
        correlation_key: Option<CorrelationKey>,
    ) -> DiagnosticEvent {
        DiagnosticEvent {
            event_id: event_id.to_string(),
            kind: EventKind::BlockApplied,
            timestamp_ms: 1716200000000 + height * 10000,
            height: Some(height),
            correlation_key,
            caused_by_event_id: None,
            is_cascade_origin: false,
            payload: EventPayload::BlockApplied {
                slot: height as u32,
                block_hash: hash.to_string(),
                producer_pubkey: producer.to_string(),
                from_peer_id: None,
                received_at_ms: None,
                applied_at_ms: 1716200000000 + height * 10000,
                validation_duration_ms: 50,
                mode: "Full".to_string(),
                tx_count: 1,
            },
        }
    }

    // -----------------------------------------------------------------------
    // test_fleet_bundle_serde_roundtrip
    // -----------------------------------------------------------------------

    #[test]
    fn test_fleet_bundle_serde_roundtrip() {
        let bundle = FleetBundle {
            schema_version: 1,
            query_timestamp_ms: 1716200000000,
            queried_peers: vec![PeerStatus {
                rpc_url: "peer-0".to_string(),
                node_peer_id: Some("12D3KooWTest".to_string()),
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

        let json = serde_json::to_string(&bundle).expect("serialize");
        let roundtrip: FleetBundle = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(bundle, roundtrip);
    }

    // -----------------------------------------------------------------------
    // test_peer_status_redact_rpc_url_strips_ip
    // -----------------------------------------------------------------------

    #[test]
    fn test_peer_status_redact_rpc_url_strips_ip() {
        assert_eq!(redact_rpc_url("http://127.0.0.1:8500", 0), "peer-0");
        assert_eq!(redact_rpc_url("http://10.0.0.1:8501/rpc", 1), "peer-1");
        assert_eq!(redact_rpc_url("http://192.168.1.100:28500", 5), "peer-5");
        assert_eq!(redact_rpc_url("http://localhost:8500", 2), "peer-2");

        // Verify no IP pattern in the output
        let re = regex::Regex::new(r"\d+\.\d+\.\d+\.\d+").unwrap();
        for i in 0..10 {
            let redacted = redact_rpc_url(&format!("http://10.0.0.{}:8500", i), i);
            assert!(
                !re.is_match(&redacted),
                "Redacted URL should not contain IPv4 pattern: {}",
                redacted
            );
        }
    }

    // -----------------------------------------------------------------------
    // test_fleet_summary_majority_classification_picks_mode
    // -----------------------------------------------------------------------

    #[test]
    fn test_fleet_summary_majority_classification_picks_mode() {
        // 3 peers: 2 with TipRaceNatural, 1 with RollbackLoop
        let peers = vec![
            PeerStatus {
                rpc_url: "peer-0".to_string(),
                node_peer_id: Some("peer-a".to_string()),
                bundle: Some(make_bundle(
                    "peer-a",
                    vec![],
                    Some(Classification {
                        fork_type: ForkType::TipRaceNatural,
                        confidence: 0.7,
                        evidence_event_ids: vec![],
                        recommended_action: None,
                        recommended_action_args: None,
                    }),
                )),
                error: None,
                query_latency_ms: 50,
            },
            PeerStatus {
                rpc_url: "peer-1".to_string(),
                node_peer_id: Some("peer-b".to_string()),
                bundle: Some(make_bundle(
                    "peer-b",
                    vec![],
                    Some(Classification {
                        fork_type: ForkType::TipRaceNatural,
                        confidence: 0.7,
                        evidence_event_ids: vec![],
                        recommended_action: None,
                        recommended_action_args: None,
                    }),
                )),
                error: None,
                query_latency_ms: 60,
            },
            PeerStatus {
                rpc_url: "peer-2".to_string(),
                node_peer_id: Some("peer-c".to_string()),
                bundle: Some(make_bundle(
                    "peer-c",
                    vec![],
                    Some(Classification {
                        fork_type: ForkType::RollbackLoop,
                        confidence: 0.85,
                        evidence_event_ids: vec![],
                        recommended_action: None,
                        recommended_action_args: None,
                    }),
                )),
                error: None,
                query_latency_ms: 70,
            },
        ];

        let groups = build_fork_groups(&peers);
        let summary = build_fleet_summary(&peers, &groups);

        assert_eq!(summary.total_peers_queried, 3);
        assert_eq!(summary.peers_reachable, 3);
        assert_eq!(summary.peers_with_diagnostics, 3);

        // Majority should be TipRaceNatural (2 vs 1)
        assert_eq!(
            summary.majority_classification,
            Some(ForkType::TipRaceNatural)
        );

        // Minority should contain RollbackLoop with count=1
        assert_eq!(summary.minority_classifications.len(), 1);
        assert_eq!(
            summary.minority_classifications[0].0,
            ForkType::RollbackLoop
        );
        assert_eq!(summary.minority_classifications[0].1, 1);
    }

    // -----------------------------------------------------------------------
    // test_fork_group_partitions_canonical_vs_fork_correctly
    // -----------------------------------------------------------------------

    #[test]
    fn test_fork_group_partitions_canonical_vs_fork_correctly() {
        let ck = CorrelationKey {
            divergence_height: Some(100),
            canonical_hash: Some("canonical_abc".to_string()),
            fork_hash: Some("fork_xyz".to_string()),
        };

        // peer-a applied canonical hash
        let ev_a = make_block_applied("ev-1", 100, "canonical_abc", "prod1", Some(ck.clone()));
        // peer-b applied fork hash
        let ev_b = make_block_applied("ev-2", 100, "fork_xyz", "prod1", Some(ck.clone()));
        // peer-c has the correlation key but no BlockApplied at height 100
        let ev_c = DiagnosticEvent {
            event_id: "ev-3".to_string(),
            kind: EventKind::ForkBlockReceived,
            timestamp_ms: 1716200001000,
            height: Some(100),
            correlation_key: Some(ck.clone()),
            caused_by_event_id: None,
            is_cascade_origin: false,
            payload: EventPayload::ForkBlockReceived {
                block_hash: "fork_xyz".to_string(),
                block_slot: 100,
                block_height_estimate: Some(100),
                producer_pubkey: "prod1".to_string(),
                from_peer_id: "12D3KooWSender".to_string(),
                classification: "ForkBlock".to_string(),
                fork_kind: Some("HeightOccupied".to_string()),
                local_tip_hash: "canonical_abc".to_string(),
                local_tip_height: 100,
            },
        };

        let peers = vec![
            PeerStatus {
                rpc_url: "peer-0".to_string(),
                node_peer_id: Some("peer-a".to_string()),
                bundle: Some(make_bundle("peer-a", vec![ev_a], None)),
                error: None,
                query_latency_ms: 30,
            },
            PeerStatus {
                rpc_url: "peer-1".to_string(),
                node_peer_id: Some("peer-b".to_string()),
                bundle: Some(make_bundle("peer-b", vec![ev_b], None)),
                error: None,
                query_latency_ms: 40,
            },
            PeerStatus {
                rpc_url: "peer-2".to_string(),
                node_peer_id: Some("peer-c".to_string()),
                bundle: Some(make_bundle("peer-c", vec![ev_c], None)),
                error: None,
                query_latency_ms: 50,
            },
        ];

        let groups = build_fork_groups(&peers);
        assert_eq!(groups.len(), 1, "Should produce exactly one fork group");

        let group = &groups[0];
        assert_eq!(group.correlation_key, ck);
        assert!(
            group.peers_on_canonical.contains(&"peer-a".to_string()),
            "peer-a should be on canonical"
        );
        assert!(
            group.peers_on_fork.contains(&"peer-b".to_string()),
            "peer-b should be on fork"
        );
        assert!(
            group.peers_undecided.contains(&"peer-c".to_string()),
            "peer-c should be undecided (ForkBlockReceived, not BlockApplied)"
        );
    }

    // -----------------------------------------------------------------------
    // test_divergence_point_recommended_action_mapping
    // -----------------------------------------------------------------------

    #[test]
    fn test_divergence_point_recommended_action_mapping() {
        // Create fork groups with different classifications to test action mapping
        let equivocation_group = ForkGroup {
            correlation_key: CorrelationKey {
                divergence_height: Some(100),
                canonical_hash: Some("hash_a".to_string()),
                fork_hash: Some("hash_b".to_string()),
            },
            peers_on_canonical: vec!["p1".to_string()],
            peers_on_fork: vec!["p2".to_string()],
            peers_undecided: vec![],
            representative_classification: Classification {
                fork_type: ForkType::ProducerEquivocation,
                confidence: 0.95,
                evidence_event_ids: vec![],
                recommended_action: None,
                recommended_action_args: None,
            },
        };

        let snap_group = ForkGroup {
            correlation_key: CorrelationKey {
                divergence_height: Some(200),
                canonical_hash: Some("hash_c".to_string()),
                fork_hash: Some("hash_d".to_string()),
            },
            peers_on_canonical: vec!["p1".to_string()],
            peers_on_fork: vec!["p3".to_string()],
            peers_undecided: vec![],
            representative_classification: Classification {
                fork_type: ForkType::PostSnapDeadTip,
                confidence: 0.80,
                evidence_event_ids: vec![],
                recommended_action: None,
                recommended_action_args: None,
            },
        };

        let tip_race_group = ForkGroup {
            correlation_key: CorrelationKey {
                divergence_height: Some(300),
                canonical_hash: Some("hash_e".to_string()),
                fork_hash: Some("hash_f".to_string()),
            },
            peers_on_canonical: vec!["p1".to_string()],
            peers_on_fork: vec!["p4".to_string()],
            peers_undecided: vec![],
            representative_classification: Classification {
                fork_type: ForkType::TipRaceNatural,
                confidence: 0.70,
                evidence_event_ids: vec![],
                recommended_action: None,
                recommended_action_args: None,
            },
        };

        let groups = vec![equivocation_group, snap_group, tip_race_group];

        assert_eq!(
            determine_recommended_action(100, &groups),
            "manual_intervention"
        );
        assert_eq!(determine_recommended_action(200, &groups), "auto_recover");
        assert_eq!(determine_recommended_action(300, &groups), "watch");
        // Unknown height defaults to "watch"
        assert_eq!(determine_recommended_action(999, &groups), "watch");
    }

    // -----------------------------------------------------------------------
    // test_no_ipv4_in_serialized_bundle
    // -----------------------------------------------------------------------

    #[test]
    fn test_no_ipv4_in_serialized_bundle() {
        // Build a FleetBundle with various IP-bearing input URLs,
        // all redacted via redact_rpc_url
        let raw_urls = [
            "http://127.0.0.1:8500",
            "http://10.0.0.1:8501",
            "http://192.168.1.100:28500",
            "http://172.16.0.5:8502/rpc?key=val",
        ];

        let queried_peers: Vec<PeerStatus> = raw_urls
            .iter()
            .enumerate()
            .map(|(i, url)| PeerStatus {
                rpc_url: redact_rpc_url(url, i),
                node_peer_id: Some(format!("12D3KooW{}", i)),
                bundle: None,
                error: Some("timeout".to_string()),
                query_latency_ms: 5000,
            })
            .collect();

        let bundle = FleetBundle {
            schema_version: 1,
            query_timestamp_ms: 1716200000000,
            queried_peers,
            fleet_summary: FleetSummary {
                total_peers_queried: 4,
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

        let serialized = serde_json::to_string(&bundle).expect("serialize");

        // Assert no IPv4 dotted-quad pattern
        let re = regex::Regex::new(r"\d+\.\d+\.\d+\.\d+").unwrap();
        assert!(
            !re.is_match(&serialized),
            "Serialized FleetBundle must not contain IPv4 dotted-quad pattern. Found in: {}",
            serialized
        );
    }
}
