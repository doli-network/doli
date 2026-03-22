# Requirements: Sync Manager Architecture Redesign

## Scope
- **Primary**: `crates/network/src/sync/` (15 files, ~9,059 lines)
- **Secondary consumers**: `bins/node/src/node/` (13 files, 81 call sites into SyncManager)
- **Branch**: `fix/sync-state-explosion-root-causes`
- **Incident context**: INC-I-004 (DownloadingHeaders sync loop), INC-I-003, INC-001, production gate deadlock (2026-03-15)

## Summary

The sync manager keeps each DOLI node's blockchain in sync with the network. Over time, bug fixes have accumulated into a state machine with 55+ fields, 8 sync states, 6 recovery phases, 11 production gate layers, and 3 parallel fork recovery subsystems -- creating a system where interactions between recovery mechanisms cause new bugs.

## User Stories

- As a node operator, I want the sync manager to reliably recover from any fork scenario so that my node does not get permanently stuck.
- As a node operator, I want the sync manager to have predictable behavior so that I can diagnose sync issues from logs alone.
- As a developer, I want the sync manager's state transitions to be explicit and testable so that new fork scenarios don't require ad-hoc patches.
- As a developer, I want the sync manager's recovery mechanisms to be unified so that they don't interfere with each other.
- As a network operator, I want redeployed nodes to automatically rejoin the network without manual intervention.

---

## Architecture Context

### Current Module Boundaries

```
crates/network/src/sync/
├── mod.rs                         (24 lines)    — public re-exports
├── manager/
│   ├── mod.rs                     (942 lines)   — SyncManager struct (55 fields), peer mgmt, state defs
│   ├── sync_engine.rs             (878 lines)   — start_sync, next_request, handle_response, handle_headers
│   ├── cleanup.rs                 (472 lines)   — stuck detection, stall recovery, blacklist mgmt
│   ├── production_gate.rs         (1,029 lines) — 11-layer production authorization + signal_stuck_fork
│   ├── block_lifecycle.rs         (552 lines)   — block_applied, block_apply_failed, reset, fork sync API
│   ├── snap_sync.rs               (288 lines)   — state root voting, quorum, snapshot handling
│   └── tests.rs                   (1,353 lines) — unit tests
├── fork_sync.rs                   (657 lines)   — binary search for common ancestor
├── fork_recovery.rs               (322 lines)   — parent chain walk for orphan blocks
├── headers.rs                     (191 lines)   — header download + validation
├── bodies.rs                      (393 lines)   — parallel body download
├── reorg.rs                       (771 lines)   — weight-based fork choice + reorg planning
├── equivocation.rs                (392 lines)   — double-signing detection
└── chain_follower.rs              (795 lines)   — DEAD CODE, unused prior redesign attempt
```

**Total**: ~9,059 lines (including ~795 lines of dead code).

### Data Flows

```
                    ┌─────────────────────────────────────────────────┐
                    │  Node (event_loop.rs, periodic.rs, etc.)        │
                    │  81 call sites across 13 files                  │
                    └───┬──────────┬──────────┬──────────┬───────────┘
                        │          │          │          │
              add_peer/ │  block_  │  handle_ │  can_    │ resolve_shallow
              update_   │  applied │  response│  produce │ fork / fork_sync
              peer      │          │          │          │ _pending_probe
                        ▼          ▼          ▼          ▼
                 ┌──────────────────────────────────────────┐
                 │            SyncManager (55 fields)        │
                 │                                           │
                 │  SyncState (8 variants)                   │
                 │  RecoveryPhase (6 variants)               │
                 │  ProductionAuthorization (13 variants)    │
                 │                                           │
                 │  Sub-state-machines:                      │
                 │  ├── HeaderDownloader                     │
                 │  ├── BodyDownloader                       │
                 │  ├── ReorgHandler                         │
                 │  ├── ForkRecoveryTracker                  │
                 │  ├── ForkSync (Optional)                  │
                 │  └── FinalityTracker                      │
                 └──────────────────────────────────────────┘
```

### Architectural Constraints & Invariants

1. **Determinism**: All nodes MUST reach the same state given the same blocks. Sync must NOT make consensus decisions.
2. **Single-threaded**: Node runs a single event loop. SyncManager is called synchronously.
3. **Request/response model**: SyncManager generates requests (`next_request()`), Node sends them via libp2p.
4. **Production gate is safety-critical**: Wrong authorization = fork or halt.

---

## Problems Identified

### P1: State Explosion — 55 Fields, Combinatorial Interactions

The SyncManager struct has **55 fields** on a single struct. No type system enforcement prevents invalid combinations:
- `state` (8 variants) x `recovery_phase` (6 variants) = 48 combinations, most undefined
- `fork_sync` (Option, parallel to SyncState) creates a shadow state machine
- `consecutive_empty_headers` read by 4 methods with different threshold semantics
- `needs_genesis_resync` set from 5 code paths, consumed by Node, never reset within SyncManager

### P2: Three Parallel Fork Recovery Systems That Interfere

1. **ForkRecoveryTracker** (`fork_recovery.rs`, 322 lines) — parent chain walk for orphan blocks
2. **ForkSync** (`fork_sync.rs`, 657 lines) — binary search for common ancestor
3. **resolve_shallow_fork** (in Node's `periodic.rs`) — sequential rollback via `consecutive_empty_headers >= 3`

These interfere: cleanup can reset fork_sync state, update_peer can restart sync mid-recovery, signal_stuck_fork called from 4 places with no coordination.

### P3: start_sync() Is a Destructive Reset Disguised as a Start

Called from 5+ entry points. Each call: increments sync_epoch (invalidates all in-flight requests), clears all downloaders and pending state, resets all peers. On a 44-node network, update_peer fires on every StatusResponse — potentially 43 calls/second, each nuking in-flight state.

### P4: Cleanup Is a God Function

472 lines, runs every tick, handles 13 responsibilities, each of which can trigger state transitions affecting the others. Has 10+ branches calling start_sync(), signal_stuck_fork(), or setting needs_genesis_resync.

### P5: Production Gate Layer Sprawl

11+ layers of checks (1,029 lines). Caused 2026-03-15 network-wide halt. `update_production_state()` was extracted from `can_produce()` because the "read" operation was mutating state.

### P6: Magic Numbers and Threshold Dead Zones

20+ hardcoded thresholds. Gap=51-1000 dead zone where neither small-fork rollback nor snap sync applies effectively.

### P7: Dead Code

`chain_follower.rs` (795 lines) — prior redesign attempt, not declared in mod.rs, not used anywhere.

### P8: Flag-Based Communication

Multiple boolean flags (needs_genesis_resync, fork_mismatch_detected, production_blocked) polled by Node with no priority system.

---

## Requirements

| ID | Requirement | Priority |
|----|------------|----------|
| REQ-SYNC-001 | Reduce SyncManager fields from 55 to ≤20 via typed sub-structs | Must |
| REQ-SYNC-002 | Unify 3 fork recovery mechanisms into single state machine | Must |
| REQ-SYNC-003 | Make start_sync() idempotent — no nuke of in-flight state on redundant calls | Must |
| REQ-SYNC-004 | Decompose cleanup() into focused functions with explicit triggers | Should |
| REQ-SYNC-005 | Extract magic thresholds into SyncConfig with documented rationale | Must |
| REQ-SYNC-006 | Remove dead code: chain_follower.rs and any other unused modules | Should |
| REQ-SYNC-007 | Explicit, logged state transitions (SyncState + RecoveryPhase) | Must |
| REQ-SYNC-008 | Reduce production gate from 11+ layers to ≤5 composable checks | Should |
| REQ-SYNC-009 | Replace flag-based communication with event/action queue | Could |
| REQ-SYNC-010 | Preserve all existing sync behaviors (behavior parity) | Must |
| REQ-SYNC-011 | Incremental redesign — no big-bang rewrite, ≤10 PRs | Must |
| REQ-SYNC-012 | Document state machine formally (states, transitions, triggers) | Should |
| REQ-SYNC-013 | Eliminate gap=51-1000 dead zone | Must |

## Acceptance Criteria

### REQ-SYNC-001: SyncManager struct has ≤20 direct fields. Related fields grouped into named sub-structs. All existing tests pass.

### REQ-SYNC-002: Single fork recovery entry point. No interference between recovery attempts. Recovery succeeds for forks at depth 1, 10, 50, 100, 500, 1000.

### REQ-SYNC-003: Consecutive start_sync() calls with no state change do not reset pending_requests. update_peer() calling 43 times/sec does not prevent sync progress.

### REQ-SYNC-005: All thresholds in SyncConfig with doc comments. Overridable for testing.

### REQ-SYNC-007: Every state transition logged with old state, new state, trigger source. Invalid transitions prevented.

### REQ-SYNC-010: All existing 1,353-line tests pass. INC-I-004, INC-001, and production gate deadlock scenarios regression-tested.

### REQ-SYNC-013: Nodes at gap=51, 500, 999 recover within 120 seconds via fork_sync binary search.

---

## Impact Analysis

| File/Module | How Affected | Risk |
|-------------|-------------|------|
| `manager/mod.rs` | Fields reorganized into sub-structs | High |
| `manager/sync_engine.rs` | start_sync() rewritten for idempotency | High |
| `manager/cleanup.rs` | Decomposed into focused functions | High |
| `manager/production_gate.rs` | Layers consolidated | High |
| `manager/block_lifecycle.rs` | Fork recovery API unified | Medium |
| `manager/snap_sync.rs` | Fields moved to sub-struct | Medium |
| `fork_sync.rs` | Subsumed into unified fork recovery | High |
| `fork_recovery.rs` | Subsumed into unified fork recovery | High |
| `bins/node/src/node/periodic.rs` | resolve_shallow_fork removed | Medium |
| `bins/node/src/node/fork_recovery.rs` | Adapter updated | Medium |
| `bins/node/src/node/event_loop.rs` | Response handling may change | Medium |

---

## Out of Scope (Won't)

1. Network protocol changes (SyncRequest/SyncResponse wire format)
2. Equivocation detection redesign (self-contained, works correctly)
3. ReorgHandler redesign (well-structured, works correctly)
4. Node event loop architecture
5. Performance optimization
6. HeaderDownloader / BodyDownloader internal logic

---

## Assumptions

| # | Assumption | Confirmed |
|---|-----------|-----------|
| 1 | Single-threaded within Node's event loop | Yes |
| 2 | 81 call sites = complete external API surface | Yes |
| 3 | chain_follower.rs is dead code, safe to delete | Yes |
| 4 | All existing tests represent intended behavior | Needs verification |
| 5 | Incremental deployment without network-wide coordinated upgrade | Needs verification |
| 6 | Wire protocol unchanged (internal restructuring only) | Yes |

---

*Analysis completed 2026-03-22. Analyst: OMEGA workflow RUN_ID=44, scope: syncmanager.*
