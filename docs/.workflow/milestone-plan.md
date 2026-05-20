# Milestone Plan — Fork-Diagnostic Observability (Phase 1)

**Workflow**: #346
**Architecture**: `specs/fork-observability-architecture.md`
**Date**: 2026-05-20

---

## Overview

4 milestones, each independently mergeable and testable. Execute in order
(M2 depends on M1; M3 depends on M1+M2; M4 depends on M3).

---

## M1: Types + Ledger + Emitter Trait

**Est. Size**: M (2 modules, moderate logic)

### Requirements Covered

REQ-FORKOBS-LEDGER-001, LEDGER-002, LEDGER-003, LEDGER-004, LEDGER-005,
LEDGER-006, LEDGER-009, REQ-FORKOBS-PERF-001, PERF-002, REQ-FORKOBS-SEC-004
(foundational — no decision logic modified).

### Files Created

| File | LoC | Purpose |
|------|-----|---------|
| `crates/storage/src/diagnostic_ledger/mod.rs` | ~120 | DiagnosticLedger: open(), record(), prune() |
| `crates/storage/src/diagnostic_ledger/types.rs` | ~200 | DiagnosticEvent, EventKind, EventPayload, ForkType, Classification, DiagnosticBundle, BlockProvenance, CorrelationKey |
| `crates/storage/src/diagnostic_ledger/emitter.rs` | ~80 | trait DiagnosticEmitter, NoOpEmitter, AsyncChannelEmitter, MockEmitter |

### Files Modified

| File | Change |
|------|--------|
| `crates/storage/src/lib.rs` | `pub mod diagnostic_ledger;` |
| `crates/storage/Cargo.toml` | Add `ulid` dependency |

### Estimated LoC: ~400 new + ~4 modified

### Acceptance Criteria

1. DiagnosticLedger opens separate RocksDB at `<data_dir>/diagnostics/`
2. `record()` writes events via async mpsc channel
3. NoOpEmitter fallback works when DB open fails (graceful degradation)
4. MockEmitter captures events into Vec for test assertions
5. Emit-latency benchmark exists: measures mpsc try_send cost (~1us expected)
6. Unit tests: ledger open/close, record/query round-trip, pruner age+count,
   NoOp behavior on degraded instance
7. No file exceeds 500 lines

---

## M2: Writer Task + Emit Sites

**Est. Size**: L (3 modules including large node modifications)

### Requirements Covered

REQ-FORKOBS-EMIT-001 to 011, REQ-FORKOBS-SEC-001, SEC-005, SEC-006.

### Files Created

| File | LoC | Purpose |
|------|-----|---------|
| `bins/node/src/node/diagnostic_writer.rs` | ~80 | Tokio task: drain channel, batch writes, track drops, heartbeat |
| `bins/node/src/node/diagnostics_pruner.rs` | ~80 | Pruner: age/count retention, cascade-origin pin |

### Files Modified

| File | Change | Delta |
|------|--------|-------|
| `bins/node/src/node/mod.rs` | Add `mod diagnostic_writer; mod diagnostics_pruner;` | +2 |
| `bins/node/src/node/init.rs` | Open DiagnosticLedger, spawn writer task, create Arc<dyn DiagnosticEmitter> | +25 |
| `bins/node/src/node/apply_block/mod.rs` | Add `provenance: Option<BlockProvenance>` parameter; 2 emit calls (block_applied, block_rejected) | +20 |
| `bins/node/src/node/block_handling.rs` | 4 emit calls in classify dispatch (Rejected, HeightOccupied, Orphan, ReorgCandidate); update apply_block calls with provenance | +30 |
| `bins/node/src/node/fork_recovery.rs` | 2 emit calls (reorg_executed); update apply_block calls with provenance=None | +15 |
| `bins/node/src/node/rollback.rs` | 2 emit calls (rollback_started, rollback_completed) | +15 |
| `bins/node/src/node/periodic.rs` | 1 emit call (recovery_classify_call); delegate to diagnostics_pruner | +10 |
| `bins/node/src/node/production/mod.rs` | Update apply_block call with provenance=None | +3 |

### Estimated LoC: ~160 new + ~120 modified

### Acceptance Criteria

1. Writer task spawns on node startup, drains channel, writes batches
2. `apply_block` signature includes `Option<BlockProvenance>` at all 6 non-test call sites
3. Running a testnet node produces diagnostic events in `data/diagnostics/`
4. Gossip blocks have from_peer_id populated; self-produced blocks have None
5. Fork events (ForkBlock, Orphan, Rejected) emit `fork_block_received` events
6. Recovery classify calls emit full 12-field RecoveryContext
7. Pruner runs every 60s; respects DOLI_DIAG_RETENTION_DAYS and DOLI_DIAG_MAX_EVENTS
8. Cascade-origin pin preserves first event per correlation_key during pruning
9. No PII (IP addresses) in any event — only PeerId
10. No file exceeds 500 lines

---

## M3: Queries + Classifier + RPC

**Est. Size**: M (3 modules, moderate logic)

### Requirements Covered

REQ-FORKOBS-LEDGER-007, LEDGER-008, REQ-FORKOBS-CLF-001 to 005,
REQ-FORKOBS-RPC-001 to 006, REQ-FORKOBS-SEC-002, SEC-003,
REQ-FORKOBS-RETRO-003.

### Files Created

| File | LoC | Purpose |
|------|-----|---------|
| `crates/storage/src/diagnostic_ledger/queries.rs` | ~120 | query_range(), query_recent(), query_causal_chain() |
| `crates/storage/src/diagnostic_ledger/classifier.rs` | ~180 | classify() pure function, 7 rules in precedence order |
| `crates/rpc/src/methods/diagnostics.rs` | ~150 | get_fork_diagnostic() handler, bundle assembly |

### Files Modified

| File | Change | Delta |
|------|--------|-------|
| `crates/rpc/src/methods/mod.rs` | `mod diagnostics;` | +1 |
| `crates/rpc/src/methods/dispatch.rs` | `"getForkDiagnostic" => self.get_fork_diagnostic(request.params).await,` | +1 |
| `crates/rpc/src/methods/context.rs` | `pub diagnostic_ledger: Option<Arc<DiagnosticLedger>>` field + builder | +5 |
| `bins/node/src/node/startup.rs` | Pass DiagnosticLedger to RpcContext | +5 |

### Estimated LoC: ~450 new + ~12 modified

### Acceptance Criteria

1. `getForkDiagnostic` RPC callable via JSON-RPC
2. Returns valid DiagnosticBundle with schema_version=1
3. `window_secs` parameter filters by time; `fork_event_id` returns causal chain
4. `limit` capped at 10,000 (REQ-FORKOBS-SEC-003)
5. Classifier returns correct ForkType for fixture inputs:
   - Two block_applied same height same producer -> ProducerEquivocation
   - block_rejected with "EpochReward" at epoch boundary -> EpochBoundaryInvalid
   - 4 rollback_started in 60s -> RollbackLoop
   - snap_sync_completed + fork_block_received within 300s -> PostSnapDeadTip
   - fork_block_received with low latency, no other signals -> TipRaceNatural
   - Unclassifiable sequence -> Unknown with reason + evidence
6. RPC returns error -32603 when diagnostics unavailable
7. No file exceeds 500 lines

---

## M4: CLI + Docs

**Est. Size**: S (1 module + documentation)

### Requirements Covered

REQ-FORKOBS-CLI-001 to 004, REQ-FORKOBS-DOC-001 to 004,
REQ-FORKOBS-RETRO-001, RETRO-002.

### Files Created

| File | LoC | Purpose |
|------|-----|---------|
| `bins/cli/src/cmd_forks.rs` | ~120 | doli forks subcommand: JSON default, --human, --last, --explain, --by-producer |
| `docs/fork_observability.md` | ~200 | Agent-facing schema doc: event kinds, fields, bundle schema, classification types, retention, env vars |

### Files Modified

| File | Change | Delta |
|------|--------|-------|
| `bins/cli/src/commands.rs` | `Forks { ... }` variant in Commands enum | +15 |
| `bins/cli/src/main.rs` | `mod cmd_forks;` + match arm | +5 |
| `docs/rpc_reference.md` | `getForkDiagnostic` entry with params, return type, example | +30 |
| `docs/troubleshooting.md` | Already updated (stub section 6b) | +0 |

### Estimated LoC: ~320 new + ~50 modified

### Acceptance Criteria

1. `doli forks` outputs valid JSON parseable by `jq`
2. `doli forks --human` outputs formatted text summary
3. `doli forks --last 1h` queries last 3600 seconds
4. `doli forks --explain` returns most recent fork with causal chain
5. `doli forks --by-producer` returns producer attribution
6. `docs/rpc_reference.md` has complete getForkDiagnostic entry
7. `docs/fork_observability.md` covers all event kinds, fields, and env vars
8. Commit message includes three-question consensus-shape checklist (NO/NO/YES)

---

## Suggested Order

```
M1 (Types + Ledger)
  |
  v
M2 (Writer + Emit Sites)
  |
  v
M3 (Queries + Classifier + RPC)
  |
  v
M4 (CLI + Docs)
```

Each milestone is independently mergeable. M1 can be deployed alone (it just
opens an empty diagnostic DB). M2 starts recording events. M3 makes them
queryable. M4 exposes them to operators.

**Total estimated LoC**: ~1286 new + modified across all milestones.
