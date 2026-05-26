// TDD RED PHASE -- Workflow #346, Milestone M3
// All tests FAIL because production code (diagnostics.rs RPC handler) does not exist yet.
//
// These tests call the handler function directly via RpcContext::get_fork_diagnostic(&self, params).
// The handler is expected to be implemented in crates/rpc/src/methods/diagnostics.rs
// and wired into dispatch.rs as "getForkDiagnostic".
//
// OUTPUT CONTRACT: fn RpcContext::get_fork_diagnostic(params: Value) -> Result<Value, RpcError>
//   O3: return Ok(Value) -- DiagnosticBundle JSON with: schema_version=1, node_peer_id, query_timestamp_ms,
//       events[], fork_summary{fork_events_in_window, by_producer, by_event_kind, first_fork_height,
//       last_fork_height}, classification(Option), baseline, health
//   O3: return Err(RpcError) -- code=-32603 when diagnostic_ledger is None
//   O4: RocksDB -- NO WRITES (read-only method, REQ-FORKOBS-SEC-002)
//
// PATHS:
//   P1: success -- ledger available, returns valid bundle with schema_version=1
//   P2: success -- window_secs filters events to time window
//   P3: success -- fork_event_id returns causal chain
//   P4: success -- limit capped at 10,000 (REQ-FORKOBS-SEC-003)
//   P5: success -- fork_summary.by_producer counts correctly
//   P6: success -- fork_summary.by_event_kind counts correctly
//   P7: success -- classification populated when events match a rule
//   P8: success -- classification is Unknown for empty window
//   P9: error -- diagnostic_ledger is None -> code -32603
//   P10: success -- method is read-only (no state mutation)
//   P11: success -- node_peer_id is populated from context
//   P12: success -- first_fork_height and last_fork_height correct
//   P13: success -- min_height/max_height filters events by height range (D7 INC-I-090)
//
// INPUT PARTITIONS:
//   P1a: default params -> full bundle with schema_version=1
//   P2a: window_secs=3600, mixed-age events -> only last hour
//   P3a: fork_event_id=B.event_id with A->B chain -> events=[A,B]
//   P4a: limit=999_999 -> <=10_000 events returned
//   P5a: 3 from X, 2 from Y (fork events) -> by_producer={X:3,Y:2}
//   P6a: mixed kinds -> by_event_kind has correct counts
//   P7a: equivocation events -> classification=Some(ProducerEquivocation)
//   P8a: empty ledger -> classification=Some(Unknown) with empty evidence
//   P9a: RpcContext with diagnostic_ledger=None -> Err(-32603)
//   P10a: call method, re-read ledger -> same event count
//   P11a: peer_id set in context -> bundle.node_peer_id matches
//   P12a: fork events at h=10,25,50 -> first=10, last=50
//   P13a: events at h=100..190, min_height=120 max_height=160 -> only h=120,130,140,150,160
//
// MATRIX: 2 output paths (Ok/Err) x 13 partitions = 13 tests
//
// --- INC-I-087: getDiagnosticHealth live counter values ---
//
// OUTPUT CONTRACT: fn get_fork_diagnostic(&self) -> health.events_written_total, health.events_dropped_total, health.last_heartbeat_ms
//   OUTPUTS: O1=events_written_total, O2=events_dropped_total, O3=last_heartbeat_ms
//   PATHS: P1=stats-unset(default zero), P2=stats-with-nonzero-counters, P3=stats-with-zero-counters-but-heartbeat-set
//   MATRIX: O1xP1=0, O1xP2=N>0, O1xP3=0; O2xP1=0, O2xP2=M>0, O2xP3=0; O3xP1=None, O3xP2=Some(t>0), O3xP3=Some(t>0)
//
// INPUT PARTITIONS:
//   P1a: Default DiagnosticWriterStats (all zeros) -> written=0, dropped=0, heartbeat=None
//   P2a: Stats with events_written=42, events_dropped=3, last_heartbeat_ms=1716200000000 -> written=42, dropped=3, heartbeat=Some(1716200000000)
//   P3a: Stats with events_written=0, events_dropped=0, last_heartbeat_ms=1716200000000 -> written=0, dropped=0, heartbeat=Some(1716200000000)

// NOTE: This test file requires the developer to add to crates/rpc/Cargo.toml:
//   [dev-dependencies]
//   tempfile.workspace = true
//   storage = { workspace = true }
//   ulid = "1"
//
// And the RpcContext must gain a `diagnostic_ledger: Option<Arc<DiagnosticLedger>>` field
// and a `with_diagnostic_ledger` builder method.
//
// The test also needs to construct an RpcContext, which requires its complex dependencies.
// We use the existing `new_for_network` constructor with minimal stubs.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde_json::json;

use storage::diagnostic_ledger::types::{DiagnosticEvent, EventKind, EventPayload};
use storage::diagnostic_ledger::DiagnosticLedger;
use storage::diagnostic_ledger::DiagnosticWriterStats;

// The RPC method is expected to be callable as:
//   ctx.get_fork_diagnostic(params).await
// where ctx is an RpcContext with `diagnostic_ledger: Option<Arc<DiagnosticLedger>>`

/// Helper: current wall-clock in milliseconds.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Helper: build a minimal ForkBlockReceived event with a specific producer.
fn make_fork_event(height: u64, ts: u64, producer: &str) -> DiagnosticEvent {
    DiagnosticEvent {
        event_id: ulid::Ulid::new().to_string(),
        kind: EventKind::ForkBlockReceived,
        timestamp_ms: ts,
        height: Some(height),
        correlation_key: None,
        caused_by_event_id: None,
        is_cascade_origin: false,
        payload: EventPayload::ForkBlockReceived {
            block_hash: format!("fork_hash_{}", height),
            block_slot: height as u32,
            block_height_estimate: Some(height),
            producer_pubkey: producer.to_string(),
            from_peer_id: "12D3KooWFork".to_string(),
            classification: "ForkBlock".to_string(),
            fork_kind: Some("HeightOccupied".to_string()),
            local_tip_hash: format!("tip_hash_{}", height),
            local_tip_height: height,
        },
    }
}

/// Helper: build a BlockApplied event with a specific producer and block_hash.
fn make_block_applied(height: u64, ts: u64, producer: &str, block_hash: &str) -> DiagnosticEvent {
    DiagnosticEvent {
        event_id: ulid::Ulid::new().to_string(),
        kind: EventKind::BlockApplied,
        timestamp_ms: ts,
        height: Some(height),
        correlation_key: None,
        caused_by_event_id: None,
        is_cascade_origin: false,
        payload: EventPayload::BlockApplied {
            slot: height as u32,
            block_hash: block_hash.to_string(),
            producer_pubkey: producer.to_string(),
            from_peer_id: Some("12D3KooWTest".to_string()),
            received_at_ms: Some(ts.saturating_sub(5)),
            applied_at_ms: ts,
            validation_duration_ms: 10,
            mode: "Full".to_string(),
            tx_count: 1,
        },
    }
}

/// Helper: create a DiagnosticLedger in a tempdir and return (ledger, tempdir).
/// The tempdir is returned to keep it alive for the test duration.
fn make_test_ledger() -> (Arc<DiagnosticLedger>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open test ledger");
    (Arc::new(ledger), dir)
}

// NOTE: The test helper that creates an RpcContext with diagnostic_ledger is expected
// to use the builder pattern:
//   RpcContext::new_for_network(...).with_diagnostic_ledger(ledger).with_peer_id(...)
//
// Since RpcContext::new_for_network requires heavy dependencies (ChainState, BlockStore, etc.),
// these tests construct it via the same minimal pattern used in other RPC tests.
// If this is too complex, the developer may add a `new_for_test()` constructor.
//
// For now, these tests are written to call the handler method directly.
// They WILL fail at compile time because:
//   1. `diagnostic_ledger` field doesn't exist on RpcContext yet
//   2. `get_fork_diagnostic` method doesn't exist yet
//   3. `diagnostics.rs` module doesn't exist yet

// ---------------------------------------------------------------------------
// Scaffolding: We cannot easily construct a full RpcContext in integration tests
// without the full node dependencies. Instead, these tests validate the handler
// logic by constructing the minimum viable RpcContext.
//
// The developer is expected to either:
//   (a) Add a `RpcContext::new_for_diagnostic_test(ledger, peer_id)` constructor, or
//   (b) Make the handler function accept (ledger, peer_id, params) directly.
//
// For TDD red phase, we write the tests as they SHOULD work with option (a).
// The developer will adapt the test scaffolding as needed.
// ---------------------------------------------------------------------------

/// Create a minimal RpcContext for diagnostic tests.
///
/// This function will fail to compile until the developer adds:
///   - `diagnostic_ledger: Option<Arc<DiagnosticLedger>>` to RpcContext
///   - `with_diagnostic_ledger()` builder method
///
/// It uses the `new_for_network` constructor with minimal stubs.
async fn make_test_rpc_context(ledger: Option<Arc<DiagnosticLedger>>) -> rpc::RpcContext {
    use tokio::sync::RwLock;

    use doli_core::consensus::ConsensusParams;
    use doli_core::network::Network;

    // Minimal deps for RpcContext
    let chain_state = Arc::new(RwLock::new(storage::ChainState::new(crypto::Hash::ZERO)));
    let block_store = {
        let dir = tempfile::tempdir().unwrap();
        // Leak the TempDir so it lives long enough for the test.
        // This is acceptable in tests to avoid the directory being cleaned up.
        let dir = Box::leak(Box::new(dir));
        Arc::new(storage::BlockStore::open(dir.path()).unwrap())
    };
    let utxo_set = Arc::new(RwLock::new(storage::UtxoSet::new()));
    let params = ConsensusParams::default();
    let mempool = Arc::new(RwLock::new(mempool::Mempool::new(
        mempool::MempoolPolicy::default(),
        params.clone(),
        Network::Devnet,
    )));

    rpc::RpcContext::new_for_network(
        chain_state,
        block_store,
        utxo_set,
        mempool,
        params,
        Network::Devnet,
    )
    .with_peer_id("12D3KooWTestNode1234567890".to_string())
    .with_diagnostic_ledger(ledger)
}

/// Create a minimal RpcContext for diagnostic tests WITH shared writer stats.
async fn make_test_rpc_context_with_stats(
    ledger: Option<Arc<DiagnosticLedger>>,
    stats: Arc<DiagnosticWriterStats>,
) -> rpc::RpcContext {
    use tokio::sync::RwLock;

    use doli_core::consensus::ConsensusParams;
    use doli_core::network::Network;

    let chain_state = Arc::new(RwLock::new(storage::ChainState::new(crypto::Hash::ZERO)));
    let block_store = {
        let dir = tempfile::tempdir().unwrap();
        let dir = Box::leak(Box::new(dir));
        Arc::new(storage::BlockStore::open(dir.path()).unwrap())
    };
    let utxo_set = Arc::new(RwLock::new(storage::UtxoSet::new()));
    let params = ConsensusParams::default();
    let mempool = Arc::new(RwLock::new(mempool::Mempool::new(
        mempool::MempoolPolicy::default(),
        params.clone(),
        Network::Devnet,
    )));

    rpc::RpcContext::new_for_network(
        chain_state,
        block_store,
        utxo_set,
        mempool,
        params,
        Network::Devnet,
    )
    .with_peer_id("12D3KooWTestNode1234567890".to_string())
    .with_diagnostic_ledger(ledger)
    .with_diagnostic_writer_stats(stats)
}

// ===========================================================================
// Tests
// ===========================================================================

// Requirement: REQ-FORKOBS-RPC-002 (Must)
// Acceptance: schema_version is 1
#[tokio::test]
async fn test_rpc_returns_valid_bundle_schema_version_1() {
    let (ledger, _dir) = make_test_ledger();
    let ctx = make_test_rpc_context(Some(ledger)).await;

    let result = ctx
        .get_fork_diagnostic(json!({"window_secs": 3600}))
        .await
        .expect("RPC should succeed");

    // O3: return -- schema_version=1
    assert_eq!(result["schema_version"], 1, "schema_version must be 1");
    // Also verify top-level fields exist
    assert!(
        result.get("node_peer_id").is_some(),
        "must have node_peer_id"
    );
    assert!(
        result.get("query_timestamp_ms").is_some(),
        "must have query_timestamp_ms"
    );
    assert!(result.get("events").is_some(), "must have events");
    assert!(
        result.get("fork_summary").is_some(),
        "must have fork_summary"
    );
    assert!(result.get("baseline").is_some(), "must have baseline");
    assert!(result.get("health").is_some(), "must have health");
}

// Requirement: REQ-FORKOBS-RPC-001 (Must)
// Acceptance: window_secs filters events by time
#[tokio::test]
async fn test_rpc_window_secs_filters_events() {
    let (ledger, _dir) = make_test_ledger();
    let now = now_ms();

    // Record events: one recent (5 min ago), one old (2 hours ago)
    let ev_recent = make_fork_event(100, now - 300_000, "aabbccdd");
    let ev_old = make_fork_event(50, now - 7_200_000, "aabbccdd");

    ledger.record(&ev_recent).unwrap();
    ledger.record(&ev_old).unwrap();

    let ctx = make_test_rpc_context(Some(ledger)).await;

    let result = ctx
        .get_fork_diagnostic(json!({"window_secs": 3600}))
        .await
        .expect("RPC should succeed");

    let events = result["events"].as_array().expect("events is array");

    // O3: return -- only events within the last hour
    // The recent event (5 min ago) should be present; the old (2h ago) should not
    let event_ids: Vec<String> = events
        .iter()
        .filter_map(|e| e["event_id"].as_str().map(String::from))
        .collect();
    assert!(
        event_ids.contains(&ev_recent.event_id),
        "recent event should be in window"
    );
    assert!(
        !event_ids.contains(&ev_old.event_id),
        "old event should be outside window"
    );
}

// Requirement: REQ-FORKOBS-RPC-004 (Should)
// Acceptance: fork_event_id returns causal chain
#[tokio::test]
async fn test_rpc_fork_event_id_returns_causal_chain() {
    let (ledger, _dir) = make_test_ledger();
    let ts = now_ms();

    // Build causal chain: A -> B
    let mut ev_a = make_fork_event(100, ts, "aabbccdd");
    ev_a.caused_by_event_id = None;

    let mut ev_b = make_fork_event(101, ts + 1000, "aabbccdd");
    ev_b.caused_by_event_id = Some(ev_a.event_id.clone());

    ledger.record(&ev_a).unwrap();
    ledger.record(&ev_b).unwrap();

    let ctx = make_test_rpc_context(Some(ledger)).await;

    let result = ctx
        .get_fork_diagnostic(json!({"fork_event_id": ev_b.event_id}))
        .await
        .expect("RPC should succeed");

    let events = result["events"].as_array().expect("events is array");

    // O3: return -- events contains both A and B in order
    let event_ids: Vec<String> = events
        .iter()
        .filter_map(|e| e["event_id"].as_str().map(String::from))
        .collect();
    assert!(
        event_ids.contains(&ev_a.event_id),
        "causal chain should include ancestor A"
    );
    assert!(
        event_ids.contains(&ev_b.event_id),
        "causal chain should include start event B"
    );
}

// Requirement: REQ-FORKOBS-SEC-003 (Must)
// Acceptance: limit capped at 10,000
#[tokio::test]
async fn test_rpc_limit_capped_at_10000() {
    let (ledger, _dir) = make_test_ledger();
    let ctx = make_test_rpc_context(Some(ledger)).await;

    // Request with absurd limit
    let result = ctx
        .get_fork_diagnostic(json!({"window_secs": 86400, "limit": 999_999}))
        .await
        .expect("RPC should succeed");

    let events = result["events"].as_array().expect("events is array");

    // O3: return -- at most 10,000 events
    assert!(
        events.len() <= 10_000,
        "limit must be capped at 10,000, got {}",
        events.len()
    );
}

// Requirement: REQ-FORKOBS-RPC-003 (Must)
// Acceptance: by_producer counts correctly
#[tokio::test]
async fn test_rpc_fork_summary_counts_by_producer() {
    let (ledger, _dir) = make_test_ledger();
    let now = now_ms();

    // 3 fork events from producer X, 2 from producer Y
    for i in 0..3u64 {
        let ev = make_fork_event(100 + i, now - (5 - i) * 1000, "producerX");
        ledger.record(&ev).unwrap();
    }
    for i in 0..2u64 {
        let ev = make_fork_event(200 + i, now - (5 - i) * 1000, "producerY");
        ledger.record(&ev).unwrap();
    }

    let ctx = make_test_rpc_context(Some(ledger)).await;

    let result = ctx
        .get_fork_diagnostic(json!({"window_secs": 3600}))
        .await
        .expect("RPC should succeed");

    let by_producer = &result["fork_summary"]["by_producer"];

    // O3: return -- by_producer = {producerX: 3, producerY: 2}
    assert_eq!(
        by_producer["producerX"], 3,
        "producerX should have 3 fork events"
    );
    assert_eq!(
        by_producer["producerY"], 2,
        "producerY should have 2 fork events"
    );
}

// Requirement: REQ-FORKOBS-RPC-003 (Must)
// Acceptance: by_event_kind counts correctly
#[tokio::test]
async fn test_rpc_fork_summary_counts_by_event_kind() {
    let (ledger, _dir) = make_test_ledger();
    let now = now_ms();

    // Record events of different kinds
    let ev1 = make_fork_event(100, now - 5000, "aabbccdd");
    let ev2 = make_fork_event(101, now - 4000, "aabbccdd");
    let ev3 = make_block_applied(102, now - 3000, "aabbccdd", "hash_102");

    ledger.record(&ev1).unwrap();
    ledger.record(&ev2).unwrap();
    ledger.record(&ev3).unwrap();

    let ctx = make_test_rpc_context(Some(ledger)).await;

    let result = ctx
        .get_fork_diagnostic(json!({"window_secs": 3600}))
        .await
        .expect("RPC should succeed");

    let by_kind = &result["fork_summary"]["by_event_kind"];

    // O3: return -- correct counts per kind
    // ForkBlockReceived: 2, BlockApplied: 1
    assert_eq!(
        by_kind["ForkBlockReceived"], 2,
        "ForkBlockReceived should be counted as 2"
    );
    assert_eq!(
        by_kind["BlockApplied"], 1,
        "BlockApplied should be counted as 1"
    );
}

// Requirement: REQ-FORKOBS-CLF-005 (Must) + REQ-FORKOBS-RPC-002 (Must)
// Acceptance: classification populated when events match a rule
#[tokio::test]
async fn test_rpc_classification_populated_when_events_match_rule() {
    let (ledger, _dir) = make_test_ledger();
    let now = now_ms();

    // Producer equivocation: same height, same producer, different hash
    let ev1 = make_block_applied(100, now - 2000, "producerAABB", "hash_A");
    let ev2 = make_block_applied(100, now - 1000, "producerAABB", "hash_B");

    ledger.record(&ev1).unwrap();
    ledger.record(&ev2).unwrap();

    let ctx = make_test_rpc_context(Some(ledger)).await;

    let result = ctx
        .get_fork_diagnostic(json!({"window_secs": 3600}))
        .await
        .expect("RPC should succeed");

    // O3: return -- classification is not null and is ProducerEquivocation
    let classification = &result["classification"];
    assert!(
        !classification.is_null(),
        "classification should be populated when events match a rule"
    );
    assert_eq!(
        classification["fork_type"], "ProducerEquivocation",
        "expected ProducerEquivocation classification"
    );
}

// Requirement: REQ-FORKOBS-CLF-005 (Must) + REQ-FORKOBS-RETRO-003 (Must)
// Acceptance: empty window -> Unknown classification
#[tokio::test]
async fn test_rpc_classification_unknown_for_empty_window() {
    let (ledger, _dir) = make_test_ledger();
    // Empty ledger -- no events
    let ctx = make_test_rpc_context(Some(ledger)).await;

    let result = ctx
        .get_fork_diagnostic(json!({"window_secs": 3600}))
        .await
        .expect("RPC should succeed");

    // O3: return -- classification should be Unknown (or null if no events to classify)
    let classification = &result["classification"];
    if !classification.is_null() {
        // If the classifier runs even on empty input, it should return Unknown
        let fork_type = &classification["fork_type"];
        // Unknown is serialized as an object: {"Unknown": {"reason_unknown": ..., "evidence_event_ids": ...}}
        // OR as a string "Unknown" depending on serde config.
        // We accept either representation.
        let is_unknown = fork_type == "Unknown"
            || fork_type.get("Unknown").is_some()
            || fork_type
                .as_object()
                .is_some_and(|m| m.contains_key("Unknown"));
        assert!(
            is_unknown,
            "empty window should classify as Unknown, got {:?}",
            fork_type
        );
    }
    // classification=null is also acceptable for empty window
}

// Requirement: REQ-FORKOBS-RPC-005 (Must)
// Acceptance: returns error -32603 when diagnostic_ledger is None
#[tokio::test]
async fn test_rpc_returns_error_when_ledger_unavailable() {
    // Create context WITHOUT a diagnostic_ledger
    let ctx = make_test_rpc_context(None).await;

    let result = ctx.get_fork_diagnostic(json!({"window_secs": 3600})).await;

    // O3: return -- Err with code -32603
    match result {
        Err(err) => {
            assert_eq!(
                err.code, -32603,
                "error code should be -32603 (internal error), got {}",
                err.code
            );
            assert!(
                err.message.contains("Diagnostic ledger unavailable")
                    || err.message.contains("diagnostic")
                    || err.message.contains("ledger"),
                "error message should mention diagnostic ledger, got: {}",
                err.message
            );
        }
        Ok(val) => panic!(
            "expected RPC error when ledger is None, got success: {:?}",
            val
        ),
    }
}

// Requirement: REQ-FORKOBS-SEC-002 (Must)
// Acceptance: method is read-only -- no state mutation in the ledger
#[tokio::test]
async fn test_rpc_method_is_readonly() {
    let (ledger, _dir) = make_test_ledger();
    let now = now_ms();

    // Record a known number of events
    let ev1 = make_fork_event(100, now - 1000, "aabbccdd");
    let ev2 = make_fork_event(101, now - 500, "aabbccdd");
    ledger.record(&ev1).unwrap();
    ledger.record(&ev2).unwrap();

    // Count events before RPC call
    let before = ledger.query_recent(86400, 100_000).unwrap().len();

    let ctx = make_test_rpc_context(Some(ledger.clone())).await;

    // Call the RPC method
    let _result = ctx
        .get_fork_diagnostic(json!({"window_secs": 3600}))
        .await
        .expect("RPC should succeed");

    // Count events after RPC call
    let after = ledger.query_recent(86400, 100_000).unwrap().len();

    // O4: RocksDB -- no writes (event count unchanged)
    assert_eq!(
        before, after,
        "RPC method must be read-only: event count before={} after={}",
        before, after
    );
}

// Requirement: REQ-FORKOBS-RPC-002 (Must)
// Acceptance: node_peer_id is populated from context
#[tokio::test]
async fn test_rpc_node_peer_id_populated() {
    let (ledger, _dir) = make_test_ledger();
    let ctx = make_test_rpc_context(Some(ledger)).await;

    let result = ctx
        .get_fork_diagnostic(json!({"window_secs": 3600}))
        .await
        .expect("RPC should succeed");

    // O3: return -- node_peer_id is a non-empty string matching the context peer_id
    let peer_id = result["node_peer_id"]
        .as_str()
        .expect("node_peer_id must be a string");
    assert!(!peer_id.is_empty(), "node_peer_id must be non-empty");
    assert_eq!(
        peer_id, "12D3KooWTestNode1234567890",
        "node_peer_id must match the context peer_id"
    );
}

// Requirement: REQ-FORKOBS-RPC-003 (Must)
// Acceptance: first_fork_height and last_fork_height correct
#[tokio::test]
async fn test_rpc_first_and_last_fork_height_correct() {
    let (ledger, _dir) = make_test_ledger();
    let now = now_ms();

    // Record fork events at heights 10, 25, 50
    let ev1 = make_fork_event(10, now - 3000, "aabbccdd");
    let ev2 = make_fork_event(25, now - 2000, "aabbccdd");
    let ev3 = make_fork_event(50, now - 1000, "aabbccdd");

    ledger.record(&ev1).unwrap();
    ledger.record(&ev2).unwrap();
    ledger.record(&ev3).unwrap();

    let ctx = make_test_rpc_context(Some(ledger)).await;

    let result = ctx
        .get_fork_diagnostic(json!({"window_secs": 3600}))
        .await
        .expect("RPC should succeed");

    let summary = &result["fork_summary"];

    // O3: return -- first_fork_height=10, last_fork_height=50
    assert_eq!(
        summary["first_fork_height"], 10,
        "first_fork_height should be 10"
    );
    assert_eq!(
        summary["last_fork_height"], 50,
        "last_fork_height should be 50"
    );
}

// ===========================================================================
// INC-I-087: getDiagnosticHealth reports live counter values
// ===========================================================================

// Requirement: INC-I-087 (Must)
// Acceptance: health.events_written_total, events_dropped_total, last_heartbeat_ms
//   reflect live DiagnosticWriterStats values, not hardcoded zeros.

/// P1: stats-unset (default zero) -- all counters at zero
///   O1xP1: events_written_total = 0
///   O2xP1: events_dropped_total = 0
///   O3xP1: last_heartbeat_ms = None (zero maps to None)
#[tokio::test]
async fn get_diagnostic_health_reports_live_counter_values_p1_default_zero() {
    let (ledger, _dir) = make_test_ledger();
    let stats = DiagnosticWriterStats::new_shared();
    // All counters default to 0 — no mutation.
    let ctx = make_test_rpc_context_with_stats(Some(ledger), stats).await;

    let result = ctx
        .get_fork_diagnostic(json!({"window_secs": 3600}))
        .await
        .expect("RPC should succeed");

    let health = &result["health"];
    assert_eq!(
        health["events_written_total"], 0,
        "P1: events_written_total should be 0 for default stats"
    );
    assert_eq!(
        health["events_dropped_total"], 0,
        "P1: events_dropped_total should be 0 for default stats"
    );
    assert!(
        health["last_heartbeat_ms"].is_null(),
        "P1: last_heartbeat_ms should be null (None) for default stats, got {:?}",
        health["last_heartbeat_ms"]
    );
}

/// P2: stats-with-nonzero-counters
///   O1xP2: events_written_total = 42
///   O2xP2: events_dropped_total = 3
///   O3xP2: last_heartbeat_ms = Some(1716200000000)
#[tokio::test]
async fn get_diagnostic_health_reports_live_counter_values_p2_nonzero() {
    let (ledger, _dir) = make_test_ledger();
    let stats = DiagnosticWriterStats::new_shared();
    stats.events_written.store(42, Ordering::Relaxed);
    stats.events_dropped.store(3, Ordering::Relaxed);
    stats
        .last_heartbeat_ms
        .store(1_716_200_000_000, Ordering::Relaxed);

    let ctx = make_test_rpc_context_with_stats(Some(ledger), stats).await;

    let result = ctx
        .get_fork_diagnostic(json!({"window_secs": 3600}))
        .await
        .expect("RPC should succeed");

    let health = &result["health"];
    assert_eq!(
        health["events_written_total"], 42,
        "P2: events_written_total should be 42"
    );
    assert_eq!(
        health["events_dropped_total"], 3,
        "P2: events_dropped_total should be 3"
    );
    assert_eq!(
        health["last_heartbeat_ms"], 1_716_200_000_000u64,
        "P2: last_heartbeat_ms should be Some(1716200000000)"
    );
}

/// P3: stats-with-zero-counters-but-heartbeat-set
///   O1xP3: events_written_total = 0
///   O2xP3: events_dropped_total = 0
///   O3xP3: last_heartbeat_ms = Some(1716200000000)
#[tokio::test]
async fn get_diagnostic_health_reports_live_counter_values_p3_heartbeat_only() {
    let (ledger, _dir) = make_test_ledger();
    let stats = DiagnosticWriterStats::new_shared();
    // Leave events_written and events_dropped at 0, but set heartbeat
    stats
        .last_heartbeat_ms
        .store(1_716_200_000_000, Ordering::Relaxed);

    let ctx = make_test_rpc_context_with_stats(Some(ledger), stats).await;

    let result = ctx
        .get_fork_diagnostic(json!({"window_secs": 3600}))
        .await
        .expect("RPC should succeed");

    let health = &result["health"];
    assert_eq!(
        health["events_written_total"], 0,
        "P3: events_written_total should be 0"
    );
    assert_eq!(
        health["events_dropped_total"], 0,
        "P3: events_dropped_total should be 0"
    );
    assert_eq!(
        health["last_heartbeat_ms"], 1_716_200_000_000u64,
        "P3: last_heartbeat_ms should be Some(1716200000000) even when counters are zero"
    );
}

// ===========================================================================
// INC-I-090 D7: min_height/max_height filter for getForkDiagnostic
// ===========================================================================

// Requirement: INC-I-090 D7 (P0 -- forensic API height filter)
// Acceptance: When min_height=120 and max_height=160 are provided in params,
//   only events at heights 120, 130, 140, 150, 160 are returned.
//   Today this test FAILS because the handler ignores height params entirely
//   and always calls query_recent(window_secs, limit).
// Evidence: [E13] Probe 2 -- requested {min_height:284670, max_height:284685}
//   but got heights_seen {min:274251, max:282815}.
#[tokio::test]
async fn test_rpc_min_max_height_filters_events_d7() {
    let (ledger, _dir) = make_test_ledger();
    let now = now_ms();

    // Record 10 BlockApplied events at heights 100, 110, 120, ..., 190
    // All timestamps are recent (within the last minute) so query_recent would
    // return ALL of them -- the only way to get 5 is via height filtering.
    for i in 0..10u64 {
        let height = 100 + i * 10;
        let ev = make_block_applied(
            height,
            now - (10 - i) * 1000,
            "aabbccdd",
            &format!("hash_{}", height),
        );
        ledger.record(&ev).unwrap();
    }

    let ctx = make_test_rpc_context(Some(ledger)).await;

    let result = ctx
        .get_fork_diagnostic(json!({
            "min_height": 120,
            "max_height": 160,
            "window_secs": 3600,
            "limit": 1000
        }))
        .await
        .expect("RPC should succeed");

    let events = result["events"].as_array().expect("events is array");

    // O3: return -- only events in [120, 160]
    // Expected: exactly 5 events (heights 120, 130, 140, 150, 160)
    assert_eq!(
        events.len(),
        5,
        "min_height=120, max_height=160 should return exactly 5 events, got {}",
        events.len()
    );

    // Every returned event must have height in [120, 160]
    for ev in events {
        let h = ev["height"].as_u64().expect("event must have height");
        assert!(
            h >= 120 && h <= 160,
            "event height {} outside requested range [120, 160]",
            h
        );
    }
}
