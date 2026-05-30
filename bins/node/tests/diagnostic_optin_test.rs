//! Fork-diagnostics opt-in toggle tests (REQ-OBS-OPTIN-001 through 007).
//!
//! Verifies that the `--fork-diagnostics` flag correctly gates the diagnostic
//! subsystem: OFF by default (no ledger, no tasks, NoOpEmitter), ON when set
//! (ledger opens, writer+pruner spawn, events recorded).
//!
//! OUTPUT CONTRACT:
//!   Node with fork_diagnostics=false:
//!     - diagnostic_ledger is None
//!     - diagnostic_shutdown_tx is None
//!     - diagnostic_emitter.is_noop() == true
//!     - No diagnostics/ directory created
//!   Node with fork_diagnostics=true:
//!     - diagnostic_ledger is Some
//!     - diagnostic_shutdown_tx is Some
//!     - diagnostic_emitter.is_noop() == false
//!   Emitter is_noop() trait method:
//!     - NoOpEmitter.is_noop() == true
//!     - AsyncChannelEmitter.is_noop() == false
//!     - MockEmitter.is_noop() == false (default)
//!   Emit helpers with NoOpEmitter:
//!     - Short-circuit before any allocation (no panic on PanicEmitter)
//!
//! INPUT PARTITIONS:
//!   P1: fork_diagnostics=false (default) -> OFF state (None, NoOp, no dir)
//!   P2: fork_diagnostics=true -> ON state (Some, AsyncChannel, tasks spawned)
//!   P3: NoOpEmitter -> is_noop()=true, record()=Ok always
//!   P4: AsyncChannelEmitter -> is_noop()=false
//!   P5: MockEmitter -> is_noop()=false (default trait impl)
//!   P6: Arc<dyn DiagnosticEmitter> -> is_noop() dispatches correctly
//!   P7: new_for_test() without explicit flag -> works (regression lock)

use std::sync::Arc;

use crypto::KeyPair;
use doli_node::node::Node;
use storage::diagnostic_ledger::emitter::{
    AsyncChannelEmitter, DiagnosticEmitter, MockEmitter, NoOpEmitter,
};
use tempfile::TempDir;

// ===========================================================================
// TEST-OBS-OPTIN-001: Default OFF — no ledger, no tasks, no diagnostics dir
// Partition: P1 (fork_diagnostics=false)
// ===========================================================================

/// REQ-OBS-OPTIN-001 (Must): Node started without fork_diagnostics flag
/// has no diagnostic ledger, no shutdown channel, and NoOpEmitter.
#[tokio::test]
async fn test_optin_default_off_no_ledger_no_tasks() {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..2).map(|_| KeyPair::generate()).collect();

    // new_for_test sets fork_diagnostics=false by default
    let node = Node::new_for_test(temp.path().to_path_buf(), producers)
        .await
        .expect("Node::new_for_test failed");

    // No diagnostic ledger
    assert!(
        node.diagnostic_ledger.is_none(),
        "diagnostic_ledger must be None when fork_diagnostics=false"
    );

    // No shutdown channel (proxy for no background tasks)
    assert!(
        node.diagnostic_shutdown_tx.is_none(),
        "diagnostic_shutdown_tx must be None when fork_diagnostics=false"
    );

    // Emitter is NoOp
    assert!(
        node.diagnostic_emitter.is_noop(),
        "emitter must be NoOp when fork_diagnostics=false"
    );

    // No diagnostics/ subdirectory created
    let diag_dir = temp.path().join("diagnostics");
    assert!(
        !diag_dir.exists(),
        "diagnostics/ directory must not exist when fork_diagnostics=false"
    );
}

// ===========================================================================
// TEST-OBS-OPTIN-003: No background tasks spawned (shutdown_tx proxy)
// Partition: P1 (fork_diagnostics=false)
// ===========================================================================

/// REQ-OBS-OPTIN-003 (Must): With fork_diagnostics=false, no writer or
/// pruner tasks are spawned. We use diagnostic_shutdown_tx=None as proxy.
#[tokio::test]
async fn test_optin_off_no_background_tasks() {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..2).map(|_| KeyPair::generate()).collect();

    let node = Node::new_for_test(temp.path().to_path_buf(), producers)
        .await
        .expect("Node::new_for_test failed");

    // Proxy: if no shutdown_tx, no tasks were spawned to listen on it
    assert!(
        node.diagnostic_shutdown_tx.is_none(),
        "No tasks should be spawned when fork_diagnostics=false"
    );
}

// ===========================================================================
// TEST-OBS-OPTIN-006: is_noop() trait method correctness
// Partitions: P3 (NoOp), P4 (Async), P5 (Mock), P6 (Arc<dyn>)
// ===========================================================================

/// REQ-OBS-OPTIN-006, Partition P3: NoOpEmitter.is_noop() returns true.
#[test]
fn test_noop_emitter_is_noop_true() {
    let emitter = NoOpEmitter;
    assert!(emitter.is_noop(), "NoOpEmitter.is_noop() must return true");
}

/// REQ-OBS-OPTIN-006, Partition P4: AsyncChannelEmitter.is_noop() returns false.
#[test]
fn test_async_channel_emitter_is_noop_false() {
    let (emitter, _rx) = AsyncChannelEmitter::new(16);
    assert!(
        !emitter.is_noop(),
        "AsyncChannelEmitter.is_noop() must return false"
    );
}

/// REQ-OBS-OPTIN-006, Partition P5: MockEmitter.is_noop() returns false (default).
#[test]
fn test_mock_emitter_is_noop_false() {
    let emitter = MockEmitter::new();
    assert!(
        !emitter.is_noop(),
        "MockEmitter.is_noop() must return false (default)"
    );
}

/// REQ-OBS-OPTIN-006, Partition P6: is_noop() through Arc<dyn DiagnosticEmitter>.
#[test]
fn test_is_noop_through_arc_dyn() {
    let noop: Arc<dyn DiagnosticEmitter> = Arc::new(NoOpEmitter);
    assert!(noop.is_noop(), "Arc<dyn> NoOp should be noop");

    let (async_em, _rx) = AsyncChannelEmitter::new(16);
    let active: Arc<dyn DiagnosticEmitter> = Arc::new(async_em);
    assert!(
        !active.is_noop(),
        "Arc<dyn> AsyncChannel should not be noop"
    );
}

// ===========================================================================
// TEST-OBS-OPTIN-006: Emit helpers short-circuit on NoOpEmitter
// Partition: P3 (NoOp emitter => record has no side effects)
// ===========================================================================

/// REQ-OBS-OPTIN-006: NoOpEmitter.record() has no side effects.
/// The emit helpers gate on is_noop() before constructing events.
/// We verify the contract: is_noop()=true and record() is a no-op.
#[test]
fn test_noop_emitter_record_has_no_side_effects() {
    use storage::diagnostic_ledger::types::{DiagnosticEvent, EventKind, EventPayload};

    let emitter = NoOpEmitter;

    // If is_noop() is true, the emit helpers return early.
    assert!(emitter.is_noop());

    let event = DiagnosticEvent {
        event_id: "test".to_string(),
        kind: EventKind::BlockApplied,
        timestamp_ms: 0,
        height: Some(1),
        correlation_key: None,
        caused_by_event_id: None,
        is_cascade_origin: false,
        payload: EventPayload::BlockApplied {
            slot: 1,
            block_hash: "aa".to_string(),
            producer_pubkey: "bb".to_string(),
            from_peer_id: None,
            received_at_ms: None,
            applied_at_ms: 0,
            validation_duration_ms: 0,
            mode: "Full".to_string(),
            tx_count: 0,
        },
    };
    let result = emitter.record(event);
    assert!(result.is_ok(), "NoOpEmitter.record() must always succeed");
}

// ===========================================================================
// TEST-OBS-OPTIN-007: new_for_test works unchanged (regression lock)
// Partition: P7 (no explicit flag => default false => works)
// ===========================================================================

/// REQ-OBS-OPTIN-007 (Must): Node::new_for_test() continues to work without
/// the new flag. All existing diagnostic-related tests pass unchanged.
#[tokio::test]
async fn test_new_for_test_works_without_explicit_flag() {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..2).map(|_| KeyPair::generate()).collect();

    // new_for_test does NOT set fork_diagnostics explicitly — uses default false
    let node = Node::new_for_test(temp.path().to_path_buf(), producers)
        .await
        .expect("Node::new_for_test must succeed without fork_diagnostics flag");

    // Node is functional: can read chain state
    let height = node.chain_state.read().await.best_height;
    assert_eq!(height, 0, "fresh test node starts at height 0");
}
