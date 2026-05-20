// TDD RED PHASE — Workflow #346, Milestone M1
// All tests FAIL because production code does not exist yet.
//
// DEV-DEPENDENCIES REQUIRED (add to crates/storage/Cargo.toml):
//   tokio = { workspace = true, features = ["rt", "macros", "sync", "time"] }
//
// OUTPUT CONTRACT:
//   For NoOpEmitter::record(event):
//     O3: return — Ok(())
//     Side effect: NONE
//   For MockEmitter::record(event):
//     O3: return — Ok(())
//     O2: self.events — appended; readable via .events()
//   For AsyncChannelEmitter::record(event):
//     O3: return — Ok(()) always (non-blocking try_send)
//     O6: channel — event sent to mpsc; on full: oldest evicted, counter incremented
//     O2: self.dropped_count — AtomicU64 incremented on channel-full
//   For AsyncChannelEmitter::dropped_count():
//     O3: return — u64 total dropped events
//   For DiagnosticEmitter trait:
//     Compile-time: must be Send + Sync
//     Arc<dyn DiagnosticEmitter> must be Send + Sync
//
// INPUT PARTITIONS:
//   NoOp: any event kind — always Ok, no side effects
//   Mock: two events in order — captured in Vec, order preserved
//   Async record (normal): channel has capacity — Ok, <100us
//   Async record (full): channel at capacity, +1 more — oldest dropped, counter=1
//   Async dropped_count: N overflow events — counter == N
//
// MATRIX: 2 NoOp cells + 3 Mock cells + 6 Async cells + 2 trait cells = 13

mod diagnostic_helpers;

use std::time::Duration;

use diagnostic_helpers::{make_event, make_event_with_ts, now_ms};
use storage::diagnostic_ledger::{
    emitter::{AsyncChannelEmitter, DiagnosticEmitter, MockEmitter, NoOpEmitter},
    types::EventKind,
};

// ===========================================================================
// NoOpEmitter
// ===========================================================================

// Requirement: REQ-FORKOBS-LEDGER-006, REQ-FORKOBS-LEDGER-009 (Must)
// Acceptance: NoOpEmitter::record() always Ok for any event kind
#[test]
fn test_noop_emitter_record_returns_ok() {
    let emitter = NoOpEmitter;
    for kind in [
        EventKind::BlockApplied,
        EventKind::RollbackStarted,
        EventKind::WriterHeartbeat,
    ] {
        assert!(
            emitter.record(make_event(kind, 1)).is_ok(),
            "NoOpEmitter({:?}) must return Ok",
            kind
        );
    }
}

// ===========================================================================
// MockEmitter
// ===========================================================================

// Requirement: REQ-FORKOBS-LEDGER-006 (Must)
// Acceptance: MockEmitter captures events in order, readable via .events()
#[test]
fn test_mock_emitter_captures_events() {
    let emitter = MockEmitter::new();

    let ev_a = make_event(EventKind::BlockApplied, 10);
    let ev_b = make_event(EventKind::ForkBlockReceived, 11);
    let (id_a, id_b) = (ev_a.event_id.clone(), ev_b.event_id.clone());

    emitter.record(ev_a).expect("record a");
    emitter.record(ev_b).expect("record b");

    let cap = emitter.events();
    assert_eq!(cap.len(), 2, "both events captured");
    assert_eq!(cap[0].event_id, id_a);
    assert_eq!(cap[1].event_id, id_b);
}

// ===========================================================================
// AsyncChannelEmitter
// ===========================================================================

// Requirement: REQ-FORKOBS-PERF-001 (Must)
// Acceptance: record() returns in <100us via try_send (non-blocking)
#[tokio::test]
async fn test_async_channel_emitter_record_is_nonblocking() {
    let (emitter, _rx) = AsyncChannelEmitter::new(1024);
    let ev = make_event(EventKind::BlockApplied, 100);

    let start = std::time::Instant::now();
    let result = emitter.record(ev);
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "record must return Ok");
    assert!(elapsed < Duration::from_micros(100), "took {:?}", elapsed);
}

// Requirement: REQ-FORKOBS-PERF-001 (Must)
// Acceptance: channel full => drop oldest, increment counter, keep newest
#[tokio::test]
async fn test_async_channel_emitter_drop_oldest_on_full() {
    let cap = 4;
    let (emitter, mut rx) = AsyncChannelEmitter::new(cap);

    // Fill channel
    for i in 0..cap {
        emitter
            .record(make_event_with_ts(
                EventKind::BlockApplied,
                i as u64,
                now_ms() + i as u64,
            ))
            .unwrap();
    }

    // Overflow — should drop oldest
    let overflow = make_event_with_ts(EventKind::BlockApplied, 999, now_ms() + 999);
    let overflow_id = overflow.event_id.clone();
    emitter
        .record(overflow)
        .expect("must succeed even when full");

    assert_eq!(emitter.dropped_count(), 1, "one event dropped");

    // Drain and verify newest is present
    let mut ids = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        ids.push(ev.event_id.clone());
    }
    assert!(
        ids.contains(&overflow_id),
        "overflow event must be in channel"
    );
    assert_eq!(ids.len(), cap, "channel has exactly cap events");
}

// Requirement: REQ-FORKOBS-PERF-001 (Must)
// Acceptance: dropped_count accumulates correctly over N overflows
#[tokio::test]
async fn test_async_channel_emitter_dropped_counter_exposed() {
    let cap = 2;
    let (emitter, _rx) = AsyncChannelEmitter::new(cap);

    // Fill
    for i in 0..cap {
        emitter
            .record(make_event_with_ts(
                EventKind::BlockApplied,
                i as u64,
                now_ms() + i as u64,
            ))
            .unwrap();
    }

    // 5 overflows
    for i in 0..5u64 {
        emitter
            .record(make_event_with_ts(
                EventKind::BlockApplied,
                100 + i,
                now_ms() + 100 + i,
            ))
            .unwrap();
    }

    assert_eq!(
        emitter.dropped_count(),
        5,
        "counter must equal overflow count"
    );
}

// ===========================================================================
// Trait bound checks (compile-time, REQ-FORKOBS-PERF-002)
// ===========================================================================

// Requirement: REQ-FORKOBS-PERF-002 (Must)
// Acceptance: DiagnosticEmitter impls are Send + Sync
#[test]
fn test_diagnostic_emitter_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<NoOpEmitter>();
    assert_send_sync::<MockEmitter>();
}

// Requirement: REQ-FORKOBS-PERF-002 (Must)
// Acceptance: Arc<dyn DiagnosticEmitter> is Send + Sync for Node struct
#[test]
fn test_arc_dyn_emitter_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<std::sync::Arc<dyn DiagnosticEmitter>>();
}
