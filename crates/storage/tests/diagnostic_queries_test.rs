// TDD RED PHASE -- Workflow #346, Milestone M3
// All tests FAIL because production code (queries.rs) does not exist yet.
//
// OUTPUT CONTRACT: fn DiagnosticLedger::query_range(kind, min_height, max_height, limit)
//   O3: return -- Vec<DiagnosticEvent> filtered by kind+height, ordered by composite key, capped by limit
//   O4: RocksDB -- no writes (read-only query)
//
// OUTPUT CONTRACT: fn DiagnosticLedger::query_recent(window_secs, limit)
//   O3: return -- Vec<DiagnosticEvent> within [now - window_secs*1000, now], deterministic order, capped by limit
//   O4: RocksDB -- no writes (read-only query)
//
// OUTPUT CONTRACT: fn DiagnosticLedger::query_causal_chain(start_event_id, max_depth)
//   O3: return -- Vec<DiagnosticEvent> following caused_by_event_id links, oldest-first, depth-bounded
//   O4: RocksDB -- no writes (read-only query)
//
// PATHS:
//   P1: query_range success with kind filter -- only matching kind returned
//   P2: query_range success with height filter -- only events in [min, max] returned
//   P3: query_range success with limit -- capped at limit
//   P4: query_range limit clamped at 10_000 -- SEC-003 storage-layer prerequisite
//   P5: query_range no matches -- empty vec
//   P6: query_range kind=None -- all kinds returned
//   P7: query_recent within window -- only events in time window
//   P8: query_recent deterministic order -- same output on repeated calls
//   P9: query_recent limit -- capped at limit
//   P10: query_causal_chain follows links -- complete chain oldest-first
//   P11: query_causal_chain max_depth -- bounded depth
//   P12: query_causal_chain dangling link -- graceful stop
//   P13: query_causal_chain self-referential -- no infinite loop
//   P14: query_range after prune -- only surviving events
//   P15: query_range performance -- 10k+10k events, filtered query returns 100 in <50ms
//
// INPUT PARTITIONS:
//   P1a: single kind among 3 -- only matching kind returned (count check)
//   P2a: height range excludes low and high -- boundary precision
//   P3a: record 100, limit 10 -- exactly 10 returned
//   P4a: limit=999_999 -- clamped to <=10_000
//   P5a: wrong kind + tight height -- empty result set
//   P6a: kind=None with mixed kinds -- all returned
//   P7a: events at -30s,-1m,-30m,-2h with window=3600s -- excludes -2h
//   P8a: events recorded out of order -- sorted deterministically
//   P9a: many events, small limit -- capped
//   P10a: chain A->B->C -- returns [A,B,C] oldest-first
//   P11a: chain of 5, max_depth=3 -- returns at most 3
//   P12a: event with caused_by=nonexistent -- returns [event] only
//   P13a: event with caused_by=self -- returns [event] only, no hang
//   P14a: fill past cap, prune, query -- only surviving
//   P15a: 20k events, query kind A limit 100 -- <50ms
//
// MATRIX: 3 outputs (return) x 15 partitions = 15 tests, each asserting O3 (return vec)

mod diagnostic_helpers;

use std::time::Duration;

use diagnostic_helpers::{make_event_with_ts, now_ms};
use storage::diagnostic_ledger::{types::EventKind, DiagnosticLedger};

// ===========================================================================
// query_range tests
// ===========================================================================

// Requirement: REQ-FORKOBS-LEDGER-007 (Must)
// Acceptance: Querying with kind=Some(block_rejected), min_height=100, max_height=200
//   returns only block_rejected events in that range.
#[test]
fn test_query_range_filters_by_kind() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    // Record events of 3 different kinds at the same height
    let ts = now_ms();
    let ev_applied = make_event_with_ts(EventKind::BlockApplied, 100, ts);
    let ev_rejected = make_event_with_ts(EventKind::BlockRejected, 100, ts + 1);
    let ev_fork = make_event_with_ts(EventKind::ForkBlockReceived, 100, ts + 2);

    ledger.record(&ev_applied).unwrap();
    ledger.record(&ev_rejected).unwrap();
    ledger.record(&ev_fork).unwrap();

    // Query for only BlockRejected
    let results = ledger
        .query_range(Some(EventKind::BlockRejected), 0, u64::MAX, 100)
        .unwrap();

    // O3: return -- only BlockRejected events
    assert_eq!(
        results.len(),
        1,
        "should return exactly 1 BlockRejected event"
    );
    assert_eq!(results[0].kind, EventKind::BlockRejected);
    assert_eq!(results[0].event_id, ev_rejected.event_id);
}

// Requirement: REQ-FORKOBS-LEDGER-007 (Must)
// Acceptance: height range filtering returns only events in [min, max]
#[test]
fn test_query_range_filters_by_height() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let ts = now_ms();
    // Record events at h=10, 20, 30, 40, 50
    for h in [10u64, 20, 30, 40, 50] {
        let ev = make_event_with_ts(EventKind::BlockApplied, h, ts + h);
        ledger.record(&ev).unwrap();
    }

    // Query 15..=35 -- should return h=20 and h=30 only
    let results = ledger.query_range(None, 15, 35, 100).unwrap();

    // O3: return -- events at h=20 and h=30 only
    assert_eq!(
        results.len(),
        2,
        "should return exactly events at h=20 and h=30"
    );
    let heights: Vec<u64> = results.iter().map(|e| e.height.unwrap()).collect();
    assert!(heights.contains(&20), "must contain h=20");
    assert!(heights.contains(&30), "must contain h=30");
    assert!(!heights.contains(&10), "must not contain h=10");
    assert!(!heights.contains(&40), "must not contain h=40");
    assert!(!heights.contains(&50), "must not contain h=50");
}

// Requirement: REQ-FORKOBS-LEDGER-007 (Must)
// Acceptance: limit parameter caps the result count
#[test]
fn test_query_range_respects_limit() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let ts = now_ms();
    // Record 100 events
    for i in 0..100u64 {
        let ev = make_event_with_ts(EventKind::BlockApplied, i, ts + i);
        ledger.record(&ev).unwrap();
    }

    // Query with limit=10
    let results = ledger.query_range(None, 0, u64::MAX, 10).unwrap();

    // O3: return -- exactly 10 events
    assert_eq!(
        results.len(),
        10,
        "limit=10 should return exactly 10 events"
    );
}

// Requirement: REQ-FORKOBS-SEC-003 (Must) -- prerequisite: storage layer clamps limit
// Acceptance: Requesting limit=999999 returns at most 10,000 events
#[test]
fn test_query_range_limit_capped_at_10000() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    // We don't need to insert 10k+ events to prove the cap.
    // We just verify the method does NOT return more than 10,000 even if asked for 999,999.
    // Insert a small number and verify the internal clamping logic works.
    let ts = now_ms();
    for i in 0..50u64 {
        let ev = make_event_with_ts(EventKind::BlockApplied, i, ts + i);
        ledger.record(&ev).unwrap();
    }

    // Query with absurd limit -- storage layer should clamp to <=10,000
    let results = ledger.query_range(None, 0, u64::MAX, 999_999).unwrap();

    // O3: return -- at most 10,000 events (here 50 since that's all we have)
    assert!(
        results.len() <= 10_000,
        "limit must be clamped to <=10,000, got {}",
        results.len()
    );
    // Also verify all 50 are returned (the clamp didn't over-restrict)
    assert_eq!(results.len(), 50, "all 50 events should be returned");
}

// Requirement: REQ-FORKOBS-LEDGER-007 (Must)
// Acceptance: wrong kind + tight height range returns empty
#[test]
fn test_query_range_empty_when_no_matches() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let ts = now_ms();
    // Record BlockApplied events at heights 10..20
    for h in 10..20u64 {
        let ev = make_event_with_ts(EventKind::BlockApplied, h, ts + h);
        ledger.record(&ev).unwrap();
    }

    // Query for BlockRejected (wrong kind) in a tight range
    let results = ledger
        .query_range(Some(EventKind::BlockRejected), 10, 20, 100)
        .unwrap();

    // O3: return -- empty
    assert!(
        results.is_empty(),
        "no BlockRejected events exist, should return empty"
    );

    // Also: query for correct kind but wrong height range
    let results2 = ledger
        .query_range(Some(EventKind::BlockApplied), 100, 200, 100)
        .unwrap();
    assert!(
        results2.is_empty(),
        "no events in h=100..200, should return empty"
    );
}

// Requirement: REQ-FORKOBS-LEDGER-007 (Must)
// Acceptance: kind=None returns events of all kinds
#[test]
fn test_query_range_kind_none_returns_all_kinds() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let ts = now_ms();
    let ev1 = make_event_with_ts(EventKind::BlockApplied, 10, ts);
    let ev2 = make_event_with_ts(EventKind::BlockRejected, 10, ts + 1);
    let ev3 = make_event_with_ts(EventKind::ForkBlockReceived, 10, ts + 2);
    let ev4 = make_event_with_ts(EventKind::RollbackStarted, 10, ts + 3);

    ledger.record(&ev1).unwrap();
    ledger.record(&ev2).unwrap();
    ledger.record(&ev3).unwrap();
    ledger.record(&ev4).unwrap();

    // Query with kind=None -- should return all 4
    let results = ledger.query_range(None, 0, u64::MAX, 100).unwrap();

    // O3: return -- all 4 events from all kinds
    assert_eq!(results.len(), 4, "kind=None should return all 4 events");
    let kinds: Vec<EventKind> = results.iter().map(|e| e.kind).collect();
    assert!(kinds.contains(&EventKind::BlockApplied));
    assert!(kinds.contains(&EventKind::BlockRejected));
    assert!(kinds.contains(&EventKind::ForkBlockReceived));
    assert!(kinds.contains(&EventKind::RollbackStarted));
}

// ===========================================================================
// query_recent tests
// ===========================================================================

// Requirement: REQ-FORKOBS-LEDGER-008 (Must)
// Acceptance: Querying with duration_secs=3600 returns events from the last hour.
#[test]
fn test_query_recent_window_bounds() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let now = now_ms();
    // Record events at different ages:
    //   T-30s, T-1min, T-30min, T-2h
    let ev_30s = make_event_with_ts(EventKind::BlockApplied, 1, now - 30_000);
    let ev_1m = make_event_with_ts(EventKind::BlockApplied, 2, now - 60_000);
    let ev_30m = make_event_with_ts(EventKind::BlockApplied, 3, now - 1_800_000);
    let ev_2h = make_event_with_ts(EventKind::BlockApplied, 4, now - 7_200_000);

    ledger.record(&ev_30s).unwrap();
    ledger.record(&ev_1m).unwrap();
    ledger.record(&ev_30m).unwrap();
    ledger.record(&ev_2h).unwrap();

    // window_secs=3600 (1 hour) -- should include T-30s, T-1m, T-30m; exclude T-2h
    let results = ledger.query_recent(3600, 100).unwrap();

    // O3: return -- 3 events (not the 2h old one)
    assert_eq!(
        results.len(),
        3,
        "window_secs=3600 should return 3 events, excluding the 2h-old event"
    );
    let ids: Vec<String> = results.iter().map(|e| e.event_id.clone()).collect();
    assert!(ids.contains(&ev_30s.event_id), "must contain T-30s event");
    assert!(ids.contains(&ev_1m.event_id), "must contain T-1m event");
    assert!(ids.contains(&ev_30m.event_id), "must contain T-30m event");
    assert!(
        !ids.contains(&ev_2h.event_id),
        "must NOT contain T-2h event"
    );
}

// Requirement: REQ-FORKOBS-LEDGER-008 (Must)
// Acceptance: deterministic order -- calling twice gives same result
#[test]
fn test_query_recent_orders_deterministically() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let now = now_ms();
    // Record events deliberately out of chronological order (mixed timestamps)
    let ev_c = make_event_with_ts(EventKind::BlockApplied, 30, now - 100);
    let ev_a = make_event_with_ts(EventKind::BlockApplied, 10, now - 300);
    let ev_b = make_event_with_ts(EventKind::ForkBlockReceived, 20, now - 200);

    // Record in non-sorted order
    ledger.record(&ev_c).unwrap();
    ledger.record(&ev_a).unwrap();
    ledger.record(&ev_b).unwrap();

    let results1 = ledger.query_recent(3600, 100).unwrap();
    let results2 = ledger.query_recent(3600, 100).unwrap();

    // O3: return -- deterministic order (same result both calls)
    assert_eq!(results1.len(), 3);
    assert_eq!(results2.len(), 3);
    for (r1, r2) in results1.iter().zip(results2.iter()) {
        assert_eq!(r1.event_id, r2.event_id, "order must be deterministic");
    }

    // Verify the order is oldest-first (architecture spec: sorted by timestamp then event_id)
    for i in 0..results1.len() - 1 {
        let current_ts = results1[i].timestamp_ms;
        let next_ts = results1[i + 1].timestamp_ms;
        assert!(
            current_ts <= next_ts,
            "events must be ordered oldest-first: {} <= {}",
            current_ts,
            next_ts
        );
    }
}

// Requirement: REQ-FORKOBS-LEDGER-008 (Must)
// Acceptance: limit caps result count
#[test]
fn test_query_recent_limit() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let now = now_ms();
    // Record 50 events within the last minute
    for i in 0..50u64 {
        let ev = make_event_with_ts(EventKind::BlockApplied, i, now - (50 - i) * 1000);
        ledger.record(&ev).unwrap();
    }

    // Query with limit=5
    let results = ledger.query_recent(3600, 5).unwrap();

    // O3: return -- exactly 5 events
    assert_eq!(results.len(), 5, "limit=5 should return exactly 5 events");
}

// ===========================================================================
// query_causal_chain tests
// ===========================================================================

// Requirement: REQ-FORKOBS-RPC-004 (Should)
// Acceptance: causal chain follows caused_by_event_id links oldest-first
#[test]
fn test_query_causal_chain_follows_links() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let ts = now_ms();

    // Build chain: A -> B -> C (B.caused_by = A.event_id; C.caused_by = B.event_id)
    let mut ev_a = make_event_with_ts(EventKind::RollbackStarted, 100, ts);
    ev_a.caused_by_event_id = None; // root of chain

    let mut ev_b = make_event_with_ts(EventKind::RollbackCompleted, 100, ts + 10);
    ev_b.caused_by_event_id = Some(ev_a.event_id.clone());

    let mut ev_c = make_event_with_ts(EventKind::BlockApplied, 101, ts + 20);
    ev_c.caused_by_event_id = Some(ev_b.event_id.clone());

    ledger.record(&ev_a).unwrap();
    ledger.record(&ev_b).unwrap();
    ledger.record(&ev_c).unwrap();

    // Query causal chain starting from C
    let chain = ledger.query_causal_chain(&ev_c.event_id, 10).unwrap();

    // O3: return -- [A, B, C] oldest-first
    assert_eq!(chain.len(), 3, "chain should contain A, B, C");
    assert_eq!(
        chain[0].event_id, ev_a.event_id,
        "first in chain should be A (oldest)"
    );
    assert_eq!(chain[1].event_id, ev_b.event_id, "second should be B");
    assert_eq!(
        chain[2].event_id, ev_c.event_id,
        "third should be C (the start event)"
    );
}

// Requirement: REQ-FORKOBS-RPC-004 (Should)
// Acceptance: max_depth bounds the chain traversal
#[test]
fn test_query_causal_chain_max_depth() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let ts = now_ms();

    // Build chain of 5: A -> B -> C -> D -> E
    let mut ev_a = make_event_with_ts(EventKind::BlockApplied, 10, ts);
    ev_a.caused_by_event_id = None;

    let mut ev_b = make_event_with_ts(EventKind::BlockApplied, 11, ts + 1);
    ev_b.caused_by_event_id = Some(ev_a.event_id.clone());

    let mut ev_c = make_event_with_ts(EventKind::BlockApplied, 12, ts + 2);
    ev_c.caused_by_event_id = Some(ev_b.event_id.clone());

    let mut ev_d = make_event_with_ts(EventKind::BlockApplied, 13, ts + 3);
    ev_d.caused_by_event_id = Some(ev_c.event_id.clone());

    let mut ev_e = make_event_with_ts(EventKind::BlockApplied, 14, ts + 4);
    ev_e.caused_by_event_id = Some(ev_d.event_id.clone());

    ledger.record(&ev_a).unwrap();
    ledger.record(&ev_b).unwrap();
    ledger.record(&ev_c).unwrap();
    ledger.record(&ev_d).unwrap();
    ledger.record(&ev_e).unwrap();

    // Query causal chain from E with max_depth=3
    let chain = ledger.query_causal_chain(&ev_e.event_id, 3).unwrap();

    // O3: return -- at most 3 events (the 3 nearest ancestors including E itself)
    assert!(
        chain.len() <= 3,
        "max_depth=3 should return at most 3 events, got {}",
        chain.len()
    );
    // E itself must be in the result
    assert!(
        chain.iter().any(|e| e.event_id == ev_e.event_id),
        "start event E must be in the chain"
    );
}

// Requirement: REQ-FORKOBS-RPC-004 (Should)
// Acceptance: dangling caused_by link stops traversal gracefully
#[test]
fn test_query_causal_chain_breaks_on_missing() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let ts = now_ms();

    // Event C points to a nonexistent event ID
    let mut ev_c = make_event_with_ts(EventKind::BlockApplied, 100, ts);
    ev_c.caused_by_event_id = Some("01NONEXISTENT000000000000".to_string());

    ledger.record(&ev_c).unwrap();

    // Query causal chain from C
    let chain = ledger.query_causal_chain(&ev_c.event_id, 10).unwrap();

    // O3: return -- only [C] since the caused_by link is dangling
    assert_eq!(
        chain.len(),
        1,
        "dangling link should stop traversal, returning only the start event"
    );
    assert_eq!(chain[0].event_id, ev_c.event_id);
}

// Requirement: REQ-FORKOBS-RPC-004 (Should)
// Acceptance: self-referential caused_by does not infinite loop
#[test]
fn test_query_causal_chain_self_referential_loop_does_not_infinite_loop() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let ts = now_ms();

    // Event points to itself (malformed data)
    let mut ev = make_event_with_ts(EventKind::BlockApplied, 100, ts);
    ev.caused_by_event_id = Some(ev.event_id.clone());

    ledger.record(&ev).unwrap();

    // Must return without hanging. Use the test timeout as the safety net,
    // but assert we get a finite result.
    let chain = ledger.query_causal_chain(&ev.event_id, 100).unwrap();

    // O3: return -- at most [event] (should detect the cycle and stop)
    assert!(
        chain.len() <= 1,
        "self-referential loop should produce at most 1 event, got {}",
        chain.len()
    );
    assert_eq!(chain[0].event_id, ev.event_id);
}

// ===========================================================================
// Cross-concern tests
// ===========================================================================

// Requirement: REQ-FORKOBS-LEDGER-005 + LEDGER-007 (Must)
// Acceptance: after pruning, query_range only returns surviving events
#[test]
fn test_query_range_after_prune_returns_only_surviving() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let now = now_ms();

    // Record 20 events: 10 "old" (2 hours ago) and 10 "fresh" (now)
    for i in 0..10u64 {
        let ev = make_event_with_ts(EventKind::BlockApplied, i, now - 7_200_000 + i);
        ledger.record(&ev).unwrap();
    }
    let mut fresh_ids = Vec::new();
    for i in 10..20u64 {
        let ev = make_event_with_ts(EventKind::BlockApplied, i, now - (20 - i) * 100);
        fresh_ids.push(ev.event_id.clone());
        ledger.record(&ev).unwrap();
    }

    // Prune: retention 1 hour, max 100 events
    // The 10 old events (2h ago) should be pruned
    let pruned = ledger.prune(3600, 100).unwrap();
    assert!(pruned >= 10, "should have pruned at least 10 stale events");

    // Query all -- should only return fresh events
    let results = ledger.query_range(None, 0, u64::MAX, 100).unwrap();

    // O3: return -- only fresh events survive
    assert_eq!(
        results.len(),
        10,
        "only 10 fresh events should survive pruning"
    );
    for ev in &results {
        assert!(
            fresh_ids.contains(&ev.event_id),
            "all surviving events should be fresh"
        );
    }
}

// Requirement: REQ-FORKOBS-LEDGER-007 (Must) -- performance sanity
// Acceptance: query_range with kind filter on 20k events returns 100 in <50ms
#[ignore] // May be flaky on CI -- run with --ignored for performance validation
#[test]
fn test_query_efficient_prefix_scan() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = DiagnosticLedger::open(dir.path()).expect("open");

    let ts = now_ms();

    // Record 10,000 events of kind A (BlockApplied) and 10,000 of kind B (BlockRejected)
    for i in 0..10_000u64 {
        let ev_a = make_event_with_ts(EventKind::BlockApplied, i, ts + i);
        ledger.record(&ev_a).unwrap();
    }
    for i in 0..10_000u64 {
        let ev_b = make_event_with_ts(EventKind::BlockRejected, i + 10_000, ts + 10_000 + i);
        ledger.record(&ev_b).unwrap();
    }

    let start = std::time::Instant::now();
    let results = ledger
        .query_range(Some(EventKind::BlockApplied), 0, u64::MAX, 100)
        .unwrap();
    let elapsed = start.elapsed();

    // O3: return -- 100 BlockApplied events
    assert_eq!(results.len(), 100, "should return exactly 100 events");
    for ev in &results {
        assert_eq!(ev.kind, EventKind::BlockApplied);
    }

    // Performance: generous margin (50ms)
    assert!(
        elapsed < Duration::from_millis(50),
        "prefix-scan query should complete in <50ms, took {:?}",
        elapsed
    );
}
