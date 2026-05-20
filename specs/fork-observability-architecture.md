<!--
OUTPUT CONTRACT: N/A — architecture spec
INPUT PARTITIONS: N/A
-->

# Fork-Diagnostic Observability Architecture (Phase 1)

**Workflow**: #346
**Date**: 2026-05-20
**Author**: Architect agent (synthesis of 3 independent perspectives)
**Status**: COMMITTED
**Requirements**: `specs/fork-observability-requirements.md` (39 Must, 8 Should)

---

## Scope

### Phase 1 IN (workflow #346)

Emitter layer (read-only instrumentation), DiagnosticLedger (separate RocksDB),
`getForkDiagnostic` RPC, deterministic classifier with `Unknown` safety valve,
`doli forks` CLI, emit-latency benchmark, unit tests, docs sync.

### Phase 2 OUT (separate workflow)

Historical-log replay tool, retroactive fixture tests, JsonSchema export,
`getFleetForkDiagnostic` cross-fleet RPC, fork honeypot, push alerts, causality
DAG visualization, dashboard integration.

---

## Convergent Decisions (C1-C5)

**C1: Async mpsc as DEFAULT emit path.** `conf(0.75, inferred)`
The 50us sync target is fragile under WAL fsync, compaction storms, and unknown
disk profiles. Kernel tracing (ftrace) learned sync I/O in hot path always
eventually spikes. We start async, not sync-with-fallback. Bounded channel(1024),
drop-oldest policy, dropped-event counter exposed in `getForkDiagnostic`
health section. (Skeptic Attack 1 + Attack 3 + Analogist Rec 1)

**C2: Separate RocksDB instance at `<data_dir>/diagnostics/`.** `conf(0.80, observed)`
4 existing precedents (block_store, state_db, utxo_rocks, content_store). Avoids
snap-sync wipe coupling (`state_db/writes.rs:164` wipes 4 CFs). Follows
established `open()` pattern with `create_if_missing`, Lz4, WAL recovery.
(Skeptic + Analogist convergence; `writes.rs:164` evidence)

**C3: Classifier returns `Unknown` for novel incidents.** `conf(0.70, observed)`
The system's primary value is structured evidence capture, not automated
classification. `Unknown` carries `reason_unknown: String` + `evidence_event_ids:
Vec<String>` for agent escalation. Every historical incident that does not match
a named variant (INC-I-082, INC-I-083) gets correctly triaged via evidence, not
misclassified. (All 3 perspectives agree; Skeptic Attack 7 FM-7b)

**C4: Emit at `classify_gossip_block` dispatch, not just `apply_block`.** `conf(0.80, observed)`
Fork decisions happen upstream in `block_handling.rs:162-321`. Only
`ExtendsTip` reaches `apply_block`. `ForkBlock(HeightOccupied)`, `Orphan`, and
`ReorgCandidate` all return BEFORE apply. The `[FORK_GUARD]` info log at
line 184 is the actual fork signal. Emitter captures these as `fork_block_received`
events. (Skeptic Reframe 5, confirmed by Architecture Skeptic)

**C5: Full RecoveryContext capture (12 fields).** `conf(0.75, observed)`
REQ-FORKOBS-EMIT-007 mandates all 12+ fields from `RecoveryContext` struct
(`recovery.rs:130-152`). Without this, INC-I-083's root cause (classify coverage
hole) would be invisible in the bundle. (Analyst Skeptic Reframe 2)

---

## Resolved Contradictions (O1-O7)

**O1: Realization = Trait-injected emitter + async writer task (hybrid C+B).**
`conf(0.75, inferred)` -- Trait `DiagnosticEmitter: Send + Sync` with two impls:
`NoOpEmitter` (graceful degradation + tests) and `AsyncChannelEmitter` (production:
wraps `mpsc::Sender`). A dedicated `diagnostic_writer` tokio task owns the
`DiagnosticLedger` exclusively. Node holds `Arc<dyn DiagnosticEmitter>`. This
gives: (a) guaranteed <1us hot-path (mpsc send), (b) clean test story (MockEmitter
captures events), (c) structural graceful degradation (NoOp if DB fails).
Pure inline (Realization A) rejected because sync RocksDB write violates C1.
Pure event bus (Realization B) rejected because it lacks the trait boundary that
makes testing and graceful degradation structural. (Explorer C + Skeptic C1)

**O2: Bincode with format-marker byte prefix.** `conf(0.70, observed)` -- Bincode
is already a workspace dependency in 9 crates. No other persistent store uses it
for on-disk format, but diagnostic data is local-only and disposable. Mitigate
Analogist's ABI concern with a 1-byte format marker (`0x01`=bincode) before the
`schema_version: u16`, enabling future migration to CBOR/MessagePack without
re-reading the whole DB. (Analogist Rec 3, pragmatic override)

**O3: Cascade-origin pin.** `conf(0.70, inferred)` -- During cascade, 100k cap
fills in ~10 min, pruning the trigger events first. For each unique
`correlation_key`, retain the FIRST event even when pruning by age/count. The
pruner skips events marked as `is_cascade_origin=true`. Approximately 100 LoC
in the pruner module. (Analogist INC-I-009 analogue)

**O4: `Option<BlockProvenance>` parameter on `apply_block`.** `conf(0.75, observed)`
The side-channel (`last_block_source` on `&mut self`) is unsafe: 6 non-test call
sites for `apply_block` bypass `handle_new_block` (measured: `block_handling.rs`
lines 379/815, `production/mod.rs:589`, `fork_recovery.rs:223/691`,
`periodic.rs:312`). Each would read stale provenance. Adding
`Option<BlockProvenance>` to the signature is ~5 lines per call site, explicit
about what data is available at each call, and eliminates the stale-data class
of bugs. The parameter is `None` for self-produced blocks, snap sync, replay,
and fork recovery. (Skeptic FM-1c, confirmed by grep)

**O5: Classifier rule precedence = first-match-wins, listed order.** `conf(0.70, inferred)`
Matches `recovery.rs:classify()` precedent (lines 252-363). "No other signals"
means "no other event with kind in {fork_block_received, block_rejected,
rollback_started, recovery_classify_call with action != None} sharing the same
correlation_key." "snap_sync followed by fork_block" means within 300 seconds.
(Skeptic FM-6a/6b/6c)

**O6: Extract `diagnostics_pruner.rs` from periodic.rs.** `conf(0.80, observed)`
`periodic.rs` is 1793 lines (measured), already 3.6x the 500-line module budget.
Pruner + cascade-origin pin logic goes into a new
`bins/node/src/node/diagnostics_pruner.rs` (~80 lines). Called from
`run_periodic_tasks()` via a one-line delegation. (All perspectives agree)

**O7: `ulid` crate as new dependency.** `conf(0.65, inferred)` -- Not currently
a transitive dependency (grep confirmed). `ulid` is small (1 file, no-std
compatible, well-maintained, ~2k downloads/day). Provides monotonic ordering
within a process, 128-bit sortable ID. Falls back to
`(timestamp_ms: u64, counter: u32)` tuple if the dependency is rejected during
review. Architect recommendation: accept `ulid`.

---

## System Diagram

```
                          DOLI Node Process
 ┌─────────────────────────────────────────────────────────────┐
 │                                                             │
 │  ┌──────────────┐   ┌───────────────┐   ┌──────────────┐   │
 │  │ block_       │   │ apply_block/  │   │ fork_        │   │
 │  │ handling.rs  │──>│ mod.rs        │   │ recovery.rs  │   │
 │  │              │   │               │   │              │   │
 │  │ EMIT(3,4)    │   │ EMIT(1,2)     │   │ EMIT(6)      │   │
 │  └──────┬───────┘   └──────┬────────┘   └──────┬───────┘   │
 │         │                  │                    │           │
 │  ┌──────┴───────┐   ┌─────┴────────┐   ┌──────┴───────┐   │
 │  │ rollback.rs  │   │ periodic.rs   │   │recovery.rs   │   │
 │  │ EMIT(4,5)    │   │ EMIT(7)       │   │ classify()   │   │
 │  └──────┬───────┘   └──────┬────────┘   └──────┬───────┘   │
 │         │                  │                    │           │
 │         ▼                  ▼                    ▼           │
 │  ┌──────────────────────────────────────────────────────┐   │
 │  │        Arc<dyn DiagnosticEmitter>                    │   │
 │  │  ┌──────────────────┐  ┌──────────────────────────┐  │   │
 │  │  │  NoOpEmitter     │  │  AsyncChannelEmitter     │  │   │
 │  │  │  (degraded/test) │  │  mpsc::Sender (bounded   │  │   │
 │  │  └──────────────────┘  │  1024, drop-oldest)      │  │   │
 │  │                        └───────────┬──────────────┘  │   │
 │  └────────────────────────────────────┼─────────────────┘   │
 │                                       │                     │
 │                                       ▼                     │
 │  ┌────────────────────────────────────────────────────┐     │
 │  │  diagnostic_writer task (tokio)                    │     │
 │  │  - drains mpsc::Receiver                           │     │
 │  │  - batches into WriteBatch (10 events or 100ms)    │     │
 │  │  - writes to DiagnosticLedger                      │     │
 │  │  - tracks dropped_event_count                      │     │
 │  └──────────────────────┬─────────────────────────────┘     │
 │                         │                                   │
 │                         ▼                                   │
 │  ┌──────────────────────────────────┐                       │
 │  │  DiagnosticLedger                │                       │
 │  │  Separate RocksDB instance       │                       │
 │  │  <data_dir>/diagnostics/         │                       │
 │  │  CF: cf_events                   │                       │
 │  │  Key: [kind_u8][height_be_u64]   │                       │
 │  │       [ulid_16_bytes]            │                       │
 │  │  Val: [format_u8][version_u16]   │                       │
 │  │       [bincode payload]          │                       │
 │  └─────────────┬────────────────────┘                       │
 │                │                                            │
 │  ┌─────────────┴──────────────────────────────────────┐     │
 │  │  diagnostics_pruner.rs (called from periodic.rs)   │     │
 │  │  - every 60s: prune by age (30d) and count (100k)  │     │
 │  │  - cascade-origin pin: keep first per corr_key     │     │
 │  └────────────────────────────────────────────────────┘     │
 │                                                             │
 │  ┌────────────────────────────────┐                         │
 │  │  RPC: getForkDiagnostic        │ ◄── HTTP JSON-RPC       │
 │  │  reads DiagnosticLedger        │                         │
 │  │  runs classifier (pure fn)     │                         │
 │  │  returns DiagnosticBundle      │                         │
 │  └────────────────────────────────┘                         │
 │                                                             │
 └─────────────────────────────────────────────────────────────┘

                          CLI
 ┌────────────────────────────────────┐
 │  doli forks [--last 1h] [--human]  │
 │  doli forks --explain              │
 │  doli forks --by-producer          │
 │  → calls getForkDiagnostic via RPC │
 └────────────────────────────────────┘
```

---

## Module Map

### New Files

| Crate | File | Responsibility | Est. LoC |
|-------|------|---------------|----------|
| `crates/storage` | `src/diagnostic_ledger/mod.rs` | DiagnosticLedger struct, open(), record(), prune(), write canary | ~120 |
| `crates/storage` | `src/diagnostic_ledger/types.rs` | DiagnosticEvent enum, ForkType, Classification, DiagnosticBundle, BlockProvenance, CorrelationKey | ~200 |
| `crates/storage` | `src/diagnostic_ledger/queries.rs` | query_range(), query_recent(), query_by_correlation_key(), query_causal_chain() | ~120 |
| `crates/storage` | `src/diagnostic_ledger/emitter.rs` | `trait DiagnosticEmitter`, NoOpEmitter, AsyncChannelEmitter, MockEmitter (test) | ~80 |
| `crates/storage` | `src/diagnostic_ledger/classifier.rs` | classify() pure function, rule matching, ForkType determination | ~180 |
| `crates/rpc` | `src/methods/diagnostics.rs` | get_fork_diagnostic() RPC handler, bundle assembly | ~150 |
| `bins/node` | `src/node/diagnostic_writer.rs` | Writer task: drain channel, batch, write, track drops | ~80 |
| `bins/node` | `src/node/diagnostics_pruner.rs` | Pruner: age/count retention, cascade-origin pin | ~80 |
| `bins/cli` | `src/cmd_forks.rs` | `doli forks` subcommand, JSON/human output | ~120 |
| **TOTAL** | | | **~1130** |

### Modified Files

| File | Change | Est. Delta |
|------|--------|-----------|
| `crates/storage/src/lib.rs` | `pub mod diagnostic_ledger;` re-export | +3 |
| `crates/storage/Cargo.toml` | Add `ulid` dependency | +1 |
| `bins/node/src/node/mod.rs` | `mod diagnostic_writer; mod diagnostics_pruner;` | +2 |
| `bins/node/src/node/init.rs` | Open DiagnosticLedger, spawn writer, wire emitter | +25 |
| `bins/node/src/node/startup.rs` | Pass `Arc<DiagnosticLedger>` to RpcContext | +5 |
| `bins/node/src/node/block_handling.rs` | 4 emit calls after classify dispatch branches | +30 |
| `bins/node/src/node/apply_block/mod.rs` | Add `provenance: Option<BlockProvenance>` param, 2 emit calls | +20 |
| `bins/node/src/node/fork_recovery.rs` | 2 emit calls (reorg_executed) | +15 |
| `bins/node/src/node/rollback.rs` | 2 emit calls (rollback_started, rollback_completed) | +15 |
| `bins/node/src/node/periodic.rs` | 1 emit call (recovery_classify_call), delegate to pruner | +10 |
| `bins/node/src/node/production/mod.rs` | Update apply_block call with provenance=None | +3 |
| `crates/rpc/src/methods/mod.rs` | `mod diagnostics;` | +1 |
| `crates/rpc/src/methods/dispatch.rs` | `"getForkDiagnostic" =>` entry | +1 |
| `crates/rpc/src/methods/context.rs` | `diagnostic_ledger: Option<Arc<DiagnosticLedger>>` field | +5 |
| `bins/cli/src/commands.rs` | `Forks { ... }` variant in Commands enum | +15 |
| `bins/cli/src/main.rs` | `mod cmd_forks;` + dispatch | +5 |
| **TOTAL delta** | | **~156** |

**Grand total: ~1286 LoC new + modified.** No file exceeds 500 lines.

---

## Data Flow

```
Gossip block arrives
  → handle_new_block(block, source_peer)
    → classify_gossip_block() [pure function, no emit inside]
    → match class {
        Rejected       → EMIT fork_block_received(classification=Rejected)
        ForkBlock(HO)  → EMIT fork_block_received(ForkBlock, is_better, ...)
        Orphan         → EMIT fork_block_received(Orphan, need_height)
        ReorgCandidate → EMIT fork_block_received(ReorgCandidate)
                       → execute_reorg() → EMIT reorg_executed(...)
                           → apply_block(b, Light, None)  // no provenance
        ExtendsTip     → apply_block(b, mode, Some(provenance))
                           → on success: EMIT block_applied(...)
                           → on failure: EMIT block_rejected(...)
      }

Rollback
  → rollback_one_block()
    → EMIT rollback_started(from, to, trigger)
    → ... rollback logic ...
    → EMIT rollback_completed(duration_ms, success)

Recovery tick (periodic.rs)
  → classify() called
    → EMIT recovery_classify_call(full RecoveryContext + action + rule)

Async channel → diagnostic_writer task → WriteBatch → DiagnosticLedger (RocksDB)

RPC query
  → getForkDiagnostic(params)
    → DiagnosticLedger.query_recent() or query_range()
    → classifier::classify(&events) → Classification
    → assemble DiagnosticBundle { events, classification, fork_summary, baseline }
    → return JSON

CLI
  → doli forks --last 1h
    → HTTP POST getForkDiagnostic {"window_secs": 3600}
    → print JSON (or --human: formatted text)
```

---

## Core Type Sketches (Rust Pseudocode)

```rust
// -- crates/storage/src/diagnostic_ledger/types.rs --

/// Provenance data set by the caller of apply_block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockProvenance {
    pub from_peer_id: Option<String>,  // PeerId as string
    pub received_at_ms: u64,           // wall-clock millis
}

/// Correlation key for cross-node JOIN of the same fork event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct CorrelationKey {
    pub divergence_height: Option<u64>,
    pub canonical_hash: Option<String>,   // hex
    pub fork_hash: Option<String>,        // hex
}

/// Every event kind has a u8 discriminant for the composite key prefix.
#[repr(u8)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EventKind {
    BlockApplied = 1,
    BlockRejected = 2,
    ForkBlockReceived = 3,
    RollbackStarted = 4,
    RollbackCompleted = 5,
    ReorgExecuted = 6,
    RecoveryClassifyCall = 7,
    SnapSyncAttempted = 8,
    SnapSyncCompleted = 9,
    SnapSyncFailed = 10,
    ChainBreakDetected = 11,
    WriterHeartbeat = 12,   // write canary for health detection
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticEvent {
    pub event_id: String,              // ULID
    pub kind: EventKind,
    pub timestamp_ms: u64,
    pub height: Option<u64>,
    pub correlation_key: Option<CorrelationKey>,
    pub caused_by_event_id: Option<String>,
    pub is_cascade_origin: bool,       // set by pruner, not emitter
    pub payload: EventPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventPayload {
    BlockApplied {
        slot: u32,
        block_hash: String,
        producer_pubkey: String,
        from_peer_id: Option<String>,
        received_at_ms: Option<u64>,
        applied_at_ms: u64,
        validation_duration_ms: u64,
        mode: String,           // "Full" | "Light" | "Replay"
        tx_count: u32,
    },
    BlockRejected {
        slot: u32,
        block_hash: String,
        producer_pubkey: String,
        from_peer_id: Option<String>,
        rejection_reason: String,
        mode: String,
    },
    ForkBlockReceived {
        block_hash: String,
        block_slot: u32,
        block_height_estimate: Option<u64>,
        producer_pubkey: String,
        from_peer_id: String,
        classification: String,   // "ExtendsTip"|"ForkBlock"|"Orphan"|"Rejected"
        fork_kind: Option<String>,// "HeightOccupied"|"ReorgCandidate"
        local_tip_hash: String,
        local_tip_height: u64,
    },
    RollbackStarted {
        from_height: u64,
        to_height: u64,
        trigger: String,          // "shallow_recovery"|"reorg"|"manual"
        cumulative_depth: u32,
    },
    RollbackCompleted {
        from_height: u64,
        to_height: u64,
        duration_ms: u64,
        success: bool,
    },
    ReorgExecuted {
        old_tip_hash: String,
        new_tip_hash: String,
        rollback_depth: u32,
        applied_count: u32,
        weight_delta: u64,
        trigger_block_hash: String,
        trigger_from_peer_id: Option<String>,
    },
    RecoveryClassifyCall {
        // Full RecoveryContext (12 fields from recovery.rs:130-152)
        local_height: u64,
        network_tip_height: u64,
        peer_count: usize,
        last_applied_secs: u64,
        shallow_rollback_count: u32,
        snap_attempts: u8,
        last_rollback_local_height: Option<u64>,
        in_grace_period: bool,
        last_finality_height: Option<u64>,
        // Classifier output
        action_returned: Option<String>,
        rule_matched: Option<String>,
    },
    SnapSyncAttempted {
        local_height: u64,
        target_height: u64,
        source_peer_id: String,
    },
    SnapSyncCompleted {
        result: String,
        duration_ms: u64,
    },
    SnapSyncFailed {
        error: String,
        duration_ms: u64,
    },
    ChainBreakDetected {
        expected_prev_hash: String,
        actual_prev_hash: String,
        header_slot: u32,
        valid_so_far_count: u32,
        from_peer_id: String,
    },
    WriterHeartbeat {
        events_written_total: u64,
        events_dropped_total: u64,
    },
}

// -- Classification --

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ForkType {
    TipRaceNatural,
    TipRaceHighLatency,
    ProducerEquivocation,
    EpochBoundaryInvalid,
    PostSnapDeadTip,
    ValidationDisagreement,
    RollbackLoop,
    SnapSyncToMinorityFork,
    Unknown {
        reason_unknown: String,
        evidence_event_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Classification {
    pub fork_type: ForkType,
    pub confidence: f64,
    pub evidence_event_ids: Vec<String>,
    pub recommended_action: Option<String>,
    pub recommended_action_args: Option<serde_json::Value>,
}

// -- DiagnosticBundle (the RPC return type) --

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticBundle {
    pub schema_version: u16,       // 1
    pub node_peer_id: String,
    pub query_timestamp_ms: u64,
    pub events: Vec<DiagnosticEvent>,
    pub fork_summary: ForkSummary,
    pub classification: Option<Classification>,
    pub baseline: BaselineComparison,
    pub health: DiagnosticHealth,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkSummary {
    pub fork_events_in_window: u64,
    pub by_producer: HashMap<String, u64>,
    pub by_event_kind: HashMap<String, u64>,
    pub first_fork_height: Option<u64>,
    pub last_fork_height: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineComparison {
    pub fork_events_per_hour_current: f64,
    pub fork_events_per_hour_24h_avg: f64,
    pub delta_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticHealth {
    pub ledger_available: bool,
    pub events_written_total: u64,
    pub events_dropped_total: u64,
    pub last_heartbeat_ms: Option<u64>,
}
```

---

## Async Writer Task Design

The `diagnostic_writer` task is spawned in `init.rs` alongside the RPC server.

**Channel**: `tokio::sync::mpsc::channel::<DiagnosticEvent>(1024)`.

**Drop policy**: When the channel is full, the `AsyncChannelEmitter::record()` method
calls `try_send()`. On `TrySendError::Full`, it increments an `AtomicU64`
dropped counter and returns. The event is lost. This is acceptable for diagnostic
data -- consensus is never affected.

**Write batching**: The writer task accumulates events in a `Vec<DiagnosticEvent>`
until either (a) 10 events accumulated or (b) 100ms elapsed since first event
in batch (whichever comes first). Then issues a single `WriteBatch` to RocksDB.
This amortizes fsync cost across multiple events.

**Fsync policy**: RocksDB WAL is enabled for crash recovery, but individual
`WriteBatch` calls use default (non-sync) writes. Diagnostic data tolerates
~100ms of loss on crash.

**Shutdown**: On node shutdown, the `mpsc::Sender` is dropped (closing the
channel). The writer task drains remaining events, writes a final batch, then
exits. The DiagnosticLedger is dropped cleanly.

**Write canary**: Every 60 seconds, the writer task emits a `WriterHeartbeat`
event with cumulative counts. The RPC handler checks recency of the last
heartbeat. If missing for >120 seconds, `DiagnosticHealth.ledger_available`
returns false and the bundle carries a warning.

---

## Cascade-Origin Pin Algorithm

During an incident cascade, the pruner would normally delete the oldest events
first -- which are the trigger events (the most diagnostic).

**Algorithm** (in `diagnostics_pruner.rs`):

1. Pruner runs every 60 seconds.
2. Scan all events. For each unique `correlation_key` (non-None), find the
   event with the lowest ULID (earliest). Mark it `is_cascade_origin = true`
   via a `put` (idempotent).
3. When pruning by age or count, skip events where `is_cascade_origin == true`.
4. If the number of pinned origins exceeds 1000 (hard cap), prune the oldest
   origins to stay under the cap. This bounds memory even under adversarial
   correlation key generation.

**Cost**: One full scan per prune cycle (every 60s). At 100k events with ~500
byte keys, this is ~50MB of sequential reads -- well within the 60s budget.
The scan runs in the writer task's thread, not the consensus path.

---

## Classifier Rule Precedence

Rules are evaluated in listed order. First match wins. This mirrors
`recovery.rs:classify()` at lines 252-363.

| Priority | Rule | ForkType | Condition |
|----------|------|----------|-----------|
| 1 | Two `block_applied` for same height, same producer | `ProducerEquivocation` | Same height AND same producer_pubkey |
| 2 | `block_rejected` at epoch boundary with "missing EpochReward" | `EpochBoundaryInvalid` | rejection_reason contains "EpochReward" AND height % blocks_per_epoch == 0 |
| 3 | >3 `rollback_started` in 60s | `RollbackLoop` | Count of rollback_started events with timestamp_ms within 60_000ms of newest |
| 4 | `snap_sync_completed` followed by `fork_block_received` within 300s | `PostSnapDeadTip` | Events in same correlation_key group OR within 300s window |
| 5 | `fork_block_received` with validation_duration > 2000ms | `TipRaceHighLatency` | validation_duration_ms field > 2000 |
| 6 | `fork_block_received` with validation_duration < 500ms AND no other signals | `TipRaceNatural` | "No other signals" = no other event with kind in {ForkBlockReceived, BlockRejected, RollbackStarted, RecoveryClassifyCall(action != None)} in same correlation_key group |
| 7 | Otherwise | `Unknown` | reason_unknown = description of available evidence; evidence_event_ids = all event IDs in the window |

### Workflow #349 Phase 1.5 — Rule (h) `ChainBreakLoop` Insertion

Inserted between rules 4 and 5. Re-numbered as priority 5; rules 5–7 shift to 6–8.

| Priority | Rule | ForkType | Condition |
|----------|------|----------|-----------|
| 5 (new)  | Multi-modal chain-break loop | `ChainBreakLoop { chain_break_count, recovery_attempts, seconds_stuck, rollback_count }` | ANY of: (a) `chain_break_count > 3`; (b) `fork_block_received_count > 100` AND `fork_block_received_count / max(block_applied_count, 1) > 10`; (c) `rollback_count > 10`; (d) `recovery_attempts > 20` |

**Why this precedence position.** Rule (h) MUST fire BEFORE (e) `TipRaceHighLatency` and (f) `TipRaceNatural` because those rules iterate `ForkBlockReceived` events and match per-event timing. A node stuck in a chain-break loop emits hundreds of `ForkBlockReceived` — many with low validation latency on the local-tip-recompute path — and rule (f)'s correlation-group-locality check finds no peer signals in any single key, returning `TipRaceNatural` with confidence 0.70 + `recommended_action = normal_operation`. This is **catastrophically wrong** for a frozen node. Rule (h) inspects the **aggregate shape** of the window (cardinality, ratio, recovery-machine churn) and overrides the per-event rules. Rule (h) MUST fire AFTER (a)–(d) because those detect specific known patterns where the targeted remediation differs (e.g., PostSnapDeadTip → `investigate_snap_sync`, RollbackLoop → `investigate_recovery_params`).

**signal_d proxy decision (recovery_attempts vs empty_header_count).** The Phase 1.5 brief proposed using `RecoveryClassifyCall.empty_count` as the "peers returning empty headers" signal. That field does not exist on the current payload (`crates/storage/src/diagnostic_ledger/types.rs:134-146`). Adding it would require touching the emitter, the log replayer, and the bundle schema — out of scope for a single-rule hotfix and would expand the blast radius. Instead, rule (h) counts `RecoveryClassifyCall` events themselves and surfaces them as `recovery_attempts: u32`. This is honestly named (the field measures what its name says: how many times the recovery state machine was invoked) and on the n6 fixture it gives 241 events in 1 hour — strong stuck-state signal independent of header content. A future workflow can add `empty_count` to the payload and extend rule (h) without breaking the wire-format. The variant field is intentionally **not** renamed to `empty_header_count` to avoid lying about the underlying measurement.

**`seconds_stuck` calculation.** Time since the most recent `BlockApplied` in the window (or window start if zero `BlockApplied`). Computed against the latest event timestamp in the slice — keeps `classify()` pure (no `SystemTime::now()`). Agents reading the bundle can sanity-check this against `query_timestamp_ms` to detect very-old bundles.

**Confidence: 0.85.** Lower than `ProducerEquivocation` (0.95) and `EpochBoundaryInvalid` (0.90) because the multi-modal trigger admits more false-positive surface than equivocation (which is bit-identical proof) or epoch-boundary rejection (which is a single matched string). Higher than `TipRaceNatural` (0.70) because the four-signal OR is a stronger combined posterior than any single-event timing rule.

**recommended_action: "restart_with_resync"** with structured args naming the safe wipe scope. The CLAUDE.md "data directory wipe" rule applies: preserve `wallet.json` and `producer.seed.txt`. Args structure:
```json
{
  "approach": "stop_node + rm -rf <data_dir>/{blocks,state_db,utxo,diagnostics} + restart with --no-snap=false",
  "preserve": ["wallet.json", "producer.seed.txt"],
  "verify_after": "doli forks --explain --human after 10 minutes of sync"
}
```

**Why this rule does not steal from rules (a)–(d).** Rule (a) requires a producer-equivocation pattern (two distinct hashes at same height same producer); rule (h)'s aggregate signals do not encode that pattern. Rule (b) requires a specific `BlockRejected` payload with a string match; rule (h) does not consume `BlockRejected` events. Rule (c) requires >3 rollbacks within a **60-second sliding window**; rule (h)'s `rollback_count > 10` triggers on >10 rollbacks anywhere in a 1-hour window — the n6 fixture has 42 rollbacks **spread across the hour** which does not trigger (c). Rule (d) requires a `SnapSyncCompleted` followed by `ForkBlockReceived` within 300s; rule (h) does not consume `SnapSyncCompleted`. Therefore (h) only fires on the gap between (d)'s short-window detection and (e)/(f)'s per-event timing — exactly the diagnostic gap n6 and INC-I-083 demonstrated.

---

## Failure Modes Table

| # | Claim | Failure Mode | Mitigation | Residual Risk |
|---|-------|-------------|------------|---------------|
| 1 | "No consensus impact" | FM-1a: Sync RocksDB write blocks apply_block past slot deadline | Async mpsc DEFAULT (C1); hot path pays <1us for try_send, not disk I/O | Writer task stall could cause channel backpressure, but drop-oldest prevents blocking |
| 1 | | FM-1b: OOM from ULID/bincode allocation under memory pressure | Allocations are small (~500 bytes/event). Under INC-I-009-level OOM, the node has larger problems. `let _ =` catches Err but not allocator panic | Very low probability; allocator panics are whole-process failures anyway |
| 1 | | FM-1c: Stale provenance from side-channel | Eliminated by O4: explicit `Option<BlockProvenance>` parameter on `apply_block`. Each call site provides the correct value or None | None -- the failure class is structurally eliminated |
| 2 | "Safe for rolling deploy" | FM-2a: New RocksDB instance fails to open on restart | REQ-FORKOBS-LEDGER-009: graceful degradation to NoOpEmitter. Node operates normally | RPC returns "ledger unavailable" during this window |
| 2 | | FM-2b: RPC inconsistency during mixed-version fleet | Documented: old nodes return method-not-found. CLI/agent handles gracefully | Cosmetic; no consensus impact |
| 3 | "<50us emit latency" | FM-3a: RocksDB put latency varies by disk | Bypassed: async mpsc default means hot path never touches disk | Writer task may batch-delay up to 100ms, acceptable for diagnostics |
| 3 | | FM-3b: Undefined backpressure semantics | Specified: bounded(1024), drop-oldest, dropped counter in health | Lost events during extreme cascade; cascade-origin pin preserves trigger events |
| 4 | "Graceful degradation" | FM-4a: DB fails mid-operation after successful open | All emit sites go through trait; trait impl catches errors. Writer task logs and continues | Silent write failure produces empty bundles |
| 4 | | FM-4b: False-negative assurance from silent write failure | Write canary heartbeat every 60s. RPC checks heartbeat recency. Missing heartbeat = warning in bundle | Agent must check `health.ledger_available` field |
| 5 | "ULID ordering" | FM-5a: Counter reset on restart | Vanishingly unlikely (requires restart within same millisecond). ULID timestamp component ensures ordering | No practical risk |
| 6 | "Classifier is deterministic" | FM-6a: Overlapping rule match | First-match-wins order (O5). Rule precedence table above | None -- precedence is explicit |
| 6 | | FM-6b/6c: "No other signals" and temporal windows undefined | Defined precisely in O5 | None -- definitions are in the spec |
| 7 | "Unknown escalates safely" | FM-7a: Unknown dominates for novel incidents | By design. Primary value is structured evidence, not classification. Agent reads evidence fields | Agent must handle Unknown as "evidence needs human review" |
| 8 | "Retroactive validation" | FM-8a: INC-I-083 would produce Unknown | Correct triage: evidence points to recovery.rs. See INC-I-083 adequacy section | Unknown with rich evidence is the designed outcome |
| 8 | | FM-8b: INC-I-081 maps to EpochBoundaryInvalid | Genuine win. Rule 2 fires correctly | None |
| 9 | Workflow #349 rule (h) | FM-9a: False positive on healthy node with high block-throughput | Threshold `fork_block_received_count > 100` requires sustained mis-routed gossip; healthy nodes see <5 fork events/hr. signal_c requires >10 rollbacks/hr which a healthy node never reaches. signal_d requires >20 RecoveryClassifyCall which only triggers on chronic sync churn | Operator can confirm with raw bundle: if BlockApplied is advancing AND signals fire, escalate to design |
| 9 | | FM-9b: Stealing events from rule (d) `PostSnapDeadTip` | Rule (h) runs AFTER rule (d) by precedence. PostSnapDeadTip's 300s window detects the early-phase signal; rule (h) catches the chronic stuck state if recovery never converges | None — explicit precedence |
| 9 | | FM-9c: Variant field name `recovery_attempts` masks the design intent ("empty header signal") | Architecture doc documents the proxy decision (this section). Future workflow can add `empty_count` to RecoveryClassifyCall payload and extend rule (h) without breaking the variant shape (add a new field, keep existing) | Field name accurately describes what is measured |
| 9 | | FM-9d: Confidence 0.85 too high for multi-modal trigger | Validated against n6 fixture (1582 fork events, 241 recovery calls — overwhelming signal) AND INC-I-083 fixture (5 chain breaks in 178 log lines — unambiguous). Bounds-checked: lower than equivocation (proof-grade) and epoch-invalid (string-match-grade), higher than TipRaceNatural (single-event-timing) | Operators see 0.85 → not certain, recommend verification |

---

## INC-I-083 Schema Adequacy (REQ-FORKOBS-RETRO-001)

**Incident**: Post-snap fork-recovery deadlock. 5-8 nodes frozen after deploy.
Natural tip race at h=110360 amplified by sparse height-index across snap-synced
fleet. classify() had no escalation path for dead-fork nodes.

**Event trace on n10 (frozen node, snap-synced to canonical in 90s, then stuck)**:

| Time | Event | Key Fields |
|------|-------|------------|
| T+0s | `block_applied` | h=110359, normal advance |
| T+10s | `fork_block_received` | classification=ForkBlock, fork_kind=HeightOccupied, block_slot < canonical_slot (is_better=true), local_tip_height=110359 |
| T+10s | (internal) signal_stuck_fork() fires | (no direct event, but recovery tick will fire) |
| T+60s | `recovery_classify_call` | local_height=110360, network_tip_height=110380+, peer_count=17, last_applied_secs=50, shallow_rollback_count=0, snap_attempts=0, in_grace_period=false, action_returned=Some("HeaderFirstSync"), rule_matched="Rule3_catchall" |
| T+120s | `recovery_classify_call` | Same pattern. action=HeaderFirstSync. last_applied_secs=110 |
| T+180s | `recovery_classify_call` | Same pattern. last_applied_secs=170. No ShallowRollback (recently_synced=false). No SnapSync (gap<500). Stuck in HeaderFirstSync loop |
| ... every 60s ... | `recovery_classify_call` | action=HeaderFirstSync repeating. last_applied_secs climbing |

**Event trace on n14 (frozen due to missing data directory)**:

| Time | Event |
|------|-------|
| T+0 | No events -- node had data wiped, starts from fresh |
| T+1s | `snap_sync_attempted` (if snap enabled) or no sync events |
| T+60s+ | `recovery_classify_call` | local_height=0 or very low, gap=110000+, action=SnapSync or HeaderFirstSync |

**Expected classification**: `Unknown`
- `reason_unknown`: "recovery_classify_call returning HeaderFirstSync repeatedly with last_applied_secs > 120 and gap > 0. No ShallowRollback or SnapSync dispatched. Possible classify() coverage hole."
- `evidence_event_ids`: [all recovery_classify_call event IDs showing the repeating pattern + the initial fork_block_received showing the better-block trigger]

**Diagnostic value**: The agent reads the `RecoveryClassifyCall` events, sees:
(a) `last_applied_secs` climbing past 60s, (b) `action = HeaderFirstSync` every
time, (c) `shallow_rollback_count = 0`, (d) `snap_attempts = 0`. This directly
points to recovery.rs classify() as the code area to investigate. No log grep
needed. Estimated diagnosis time: <60 seconds for an agent, <5 minutes for a human
reading `--human` output.

---

## INC-I-081 Schema Adequacy (REQ-FORKOBS-RETRO-002)

**Incident**: Broken producer emitted invalid epoch-boundary block missing
EpochReward transaction. Fleet rejected the block. Sync state machine amplified
via ShallowRollback past finality.

**Event trace on the broken producer's node**:

| Time | Event | Key Fields |
|------|-------|------------|
| T+0s | `block_applied` | h=epoch_boundary-1, normal |
| T+10s | (production) block produced at epoch boundary | No event for production itself (Phase 2) |
| T+10s | `block_applied` | h=epoch_boundary, block_hash=X (the invalid block, applied locally because producer trusts own block) |

**Event trace on rejecting nodes (e.g., seed, n1-n12)**:

| Time | Event | Key Fields |
|------|-------|------------|
| T+10s | `block_rejected` | h=epoch_boundary, block_hash=X, rejection_reason="missing EpochReward at epoch boundary", producer_pubkey=broken_producer |
| T+20s | `fork_block_received` | classification=ForkBlock, fork_kind=HeightOccupied if they already have a valid block at that height |
| T+30s+ | `rollback_started` | If ShallowRollback triggered, from_height=epoch_boundary, trigger="shallow_recovery" |
| T+30s+ | `rollback_completed` | duration_ms=..., success=true |
| T+60s+ | `recovery_classify_call` | Shows cascading recovery actions |

**Expected classification**: `EpochBoundaryInvalid` `conf(0.85)`
- Rule 2 fires: `block_rejected` with rejection_reason containing "EpochReward"
  at an epoch boundary height.
- `evidence_event_ids`: [the block_rejected event ID]
- `recommended_action`: "investigate_producer" (the broken producer needs fixing)

**Diagnostic value**: The classification is CORRECT and SPECIFIC. The agent
immediately knows the fork type and which producer caused it. No grep needed.

---

## Milestones

| ID | Name | Scope (Modules) | Scope (Requirements) | Est. Size | Dependencies |
|----|------|-----------------|---------------------|-----------|-------------|
| M1 | Types + Ledger + Emitter Trait | diagnostic_ledger/{types,mod,emitter}.rs | REQ-FORKOBS-LEDGER-001 to 009, REQ-FORKOBS-PERF-001 to 002, REQ-FORKOBS-SEC-004 | M | None |
| M2 | Writer Task + Emit Sites | diagnostic_writer.rs, block_handling.rs, apply_block/mod.rs, rollback.rs, fork_recovery.rs, periodic.rs, diagnostics_pruner.rs | REQ-FORKOBS-EMIT-001 to 011, REQ-FORKOBS-SEC-001, SEC-005, SEC-006 | L | M1 |
| M3 | Queries + Classifier + RPC | diagnostic_ledger/queries.rs, classifier.rs, rpc/methods/diagnostics.rs | REQ-FORKOBS-LEDGER-007 to 008, REQ-FORKOBS-CLF-001 to 005, REQ-FORKOBS-RPC-001 to 006, REQ-FORKOBS-SEC-002 to 003 | M | M1, M2 |
| M4 | CLI + Docs | cmd_forks.rs, docs updates | REQ-FORKOBS-CLI-001 to 004, REQ-FORKOBS-DOC-001 to 004, REQ-FORKOBS-RETRO-001 to 003 | S | M3 |

### M1: Types + Ledger + Emitter Trait

Create the storage foundation: `DiagnosticEvent` enum with all variants,
`DiagnosticLedger` struct with `open()` / `record()` / `prune()`,
`trait DiagnosticEmitter` with `NoOpEmitter` and `AsyncChannelEmitter`.
Benchmark `record()` to confirm async channel is needed (expected: yes).

**Acceptance criteria**: DiagnosticLedger opens a separate RocksDB at
`data/diagnostics/`, writes events via the async channel, survives DB-open
failure with NoOpEmitter fallback. Benchmark exists showing hot-path cost.
Unit tests for ledger CRUD and emitter trait behavior.

**Requirements**: REQ-FORKOBS-LEDGER-001 to 009, REQ-FORKOBS-PERF-001 to 002,
REQ-FORKOBS-SEC-004.

### M2: Writer Task + Emit Sites

Spawn the `diagnostic_writer` tokio task. Add `Option<BlockProvenance>`
parameter to `apply_block`. Insert emit calls at all 7+ decision points in
`block_handling.rs`, `apply_block/mod.rs`, `rollback.rs`, `fork_recovery.rs`,
`periodic.rs`. Create `diagnostics_pruner.rs` with cascade-origin pin.

**Acceptance criteria**: After running a testnet node, `data/diagnostics/` DB
contains events for blocks applied, forks detected, rollbacks, and recovery
classify calls. `apply_block` signature updated at all 6 non-test call sites.
Events have correct provenance (None for non-gossip paths, populated for gossip).
Pruner runs every 60s, respects age/count caps, preserves cascade origins.

**Requirements**: REQ-FORKOBS-EMIT-001 to 011, REQ-FORKOBS-SEC-001, SEC-005,
SEC-006.

### M3: Queries + Classifier + RPC

Implement `query_range()`, `query_recent()`, and `query_causal_chain()` on
DiagnosticLedger. Implement the classifier pure function with 7 rules in
precedence order. Create `diagnostics.rs` RPC handler assembling
`DiagnosticBundle`.

**Acceptance criteria**: `getForkDiagnostic` RPC returns a valid JSON bundle
with events, classification, fork_summary, baseline, and health. Classifier
correctly identifies TipRaceNatural, ProducerEquivocation, EpochBoundaryInvalid,
RollbackLoop in unit test fixtures. Unknown carries reason and evidence for
unclassifiable sequences.

**Requirements**: REQ-FORKOBS-LEDGER-007 to 008, REQ-FORKOBS-CLF-001 to 005,
REQ-FORKOBS-RPC-001 to 006, REQ-FORKOBS-SEC-002 to 003.

### M4: CLI + Docs

Create `cmd_forks.rs` with `doli forks`, `--human`, `--last`, `--explain`,
`--by-producer`. Update `docs/rpc_reference.md`, `docs/troubleshooting.md`,
create `docs/fork_observability.md`.

**Acceptance criteria**: `doli forks` outputs valid JSON. `doli forks --human`
outputs readable text. `docs/rpc_reference.md` has `getForkDiagnostic` entry.
`docs/troubleshooting.md` has "How to diagnose a fork" section.

**Requirements**: REQ-FORKOBS-CLI-001 to 004, REQ-FORKOBS-DOC-001 to 004,
REQ-FORKOBS-RETRO-001 to 003.

---

## Deploy Safety (INC-I-062/075 Checklist)

1. **Can any user-submittable transaction trigger this code path?** NO.
   Diagnostic events are emitted by the node's internal block processing, not
   by any transaction type.

2. **Can any producer-action or attestation pattern trigger it?** NO.
   The emitter observes decisions; it does not change what decisions are made.

3. **Is the new behavior bit-identical to the old behavior for ALL reachable
   inputs?** YES for all consensus-visible outputs (block content, state root,
   producer set, UTXO set). The only new output is the diagnostics RocksDB
   (non-consensus, local-only).

**Conclusion**: Safe for rolling deploy. No activation height. No HardForkSchedule
entry. No block content change.

---

## DO NOT MODIFY List (REQ-FORKOBS-SEC-004)

The following files/areas MUST NOT have decision logic changed. Emit calls
(read-only instrumentation) ARE allowed INSIDE these areas.

- `crates/core/src/consensus.rs` — no changes of any kind
- `crates/core/src/network_params/` activation heights — no changes
- `bins/node/src/node/apply_block/*` — emit calls only; no changed conditionals,
  no changed return values, no changed error handling
- `crates/storage/src/snapshot.rs` — no changes of any kind
- `crates/core/src/validation/*` — no changes of any kind

**Reviewer merge-blocker**: diff of these files must show ONLY new `let _ = emitter.record(...)` lines.

---

## Traceability Matrix

| Requirement | Architecture Section | Milestone |
|------------|---------------------|-----------|
| REQ-FORKOBS-EMIT-001 to 011 | Data Flow, Module Map (emit sites) | M2 |
| REQ-FORKOBS-LEDGER-001 to 009 | System Diagram, Module Map (diagnostic_ledger/) | M1 |
| REQ-FORKOBS-RPC-001 to 006 | Module Map (diagnostics.rs), Type Sketches (DiagnosticBundle) | M3 |
| REQ-FORKOBS-CLF-001 to 005 | Classifier Rule Precedence, Type Sketches (ForkType) | M3 |
| REQ-FORKOBS-CLI-001 to 004 | Module Map (cmd_forks.rs), System Diagram (CLI) | M4 |
| REQ-FORKOBS-PERF-001 to 002 | C1, Async Writer Task Design | M1 |
| REQ-FORKOBS-DOC-001 to 004 | Milestone M4 | M4 |
| REQ-FORKOBS-SEC-001 to 006 | DO NOT MODIFY List, Deploy Safety, Failure Modes | M1-M4 |
| REQ-FORKOBS-RETRO-001 | INC-I-083 Schema Adequacy | M4 (paper defense; replay tool is Phase 2) |
| REQ-FORKOBS-RETRO-002 | INC-I-081 Schema Adequacy | M4 (paper defense; replay tool is Phase 2) |
| REQ-FORKOBS-RETRO-003 | Type Sketches (ForkType::Unknown), C3 | M3 |
