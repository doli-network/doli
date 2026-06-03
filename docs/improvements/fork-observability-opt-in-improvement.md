# Fork-Observability Opt-In Refactor

**Workflow**: omega-improve RUN_ID=395
**Type**: Refactor (behavior-preserving operational toggle)
**Scope**: `bins/node/src/` (5 files), CLI error-message polish in `bins/cli/src/`
**Date**: 2026-05-30

---

## Bottom Line (BLUF)

Add a single CLI flag **`--fork-diagnostics`** to `doli-node`. Default = OFF. When absent, the diagnostic ledger never opens, the writer/pruner/monitor tasks never spawn, emit-site helpers short-circuit before any allocation, and the diagnostic RPCs return the existing "Diagnostic ledger unavailable" error. When present, behavior is bit-identical to today's always-on path.

This matches DOLI's established operational-toggle convention (`--no-snap-sync`, `--no-dht`, `--relay-server`, `--recovery-mode`, `--auto-checkpoint`). It piggybacks on the existing graceful-degradation path (`NoOpEmitter` + `diagnostic_ledger: None`), so the OFF state is already-tested.

### Decisions (locked 2026-05-30)
- **Flag name**: `--fork-diagnostics` (scopes the toggle specifically to this subsystem, leaves room for future diagnostic surfaces).
- **Structural nodes default**: ON. The 15 structural nodes (N1-N12 + 3 seeds) get `--fork-diagnostics` added to their launchd plists / systemd units in this PR, preserving today's fleet-level fork coverage. External producers go dark unless they opt in.
- **Zero-cost emit gating**: INCLUDED in this PR (REQ-OBS-OPTIN-006 upgraded from Should → Must). When OFF, emit-site helpers short-circuit on `emitter.is_noop()` before any ULID / hex / String construction. Implementation: add `is_noop(&self) -> bool { false }` to the `DiagnosticEmitter` trait with `true` override on `NoOpEmitter`; guard the ~5 emit helpers in `apply_block/diagnostics.rs` and adjacent files. The 17 call sites are untouched.

---

## Architecture Context

### Module Boundaries

The fork-observability subsystem spans 4 crates and the CLI:

| Module | Responsibility | Depends on | Depended by |
|--------|---------------|------------|-------------|
| `crates/storage/src/diagnostic_ledger/` | Data types, emitter trait, RocksDB ledger, classifier, fleet aggregation, log replay | `rocksdb`, `bincode`, `ulid`, `serde` | Node, RPC, CLI |
| `bins/node/src/node/` (6 files) | Event emission at hot paths, writer task, pruner task, monitor alerts | `storage::diagnostic_ledger` | Nothing (fire-and-forget side channel) |
| `crates/rpc/src/methods/` (2 files) | `getForkDiagnostic`, `getFleetForkDiagnostic` handlers | `storage::diagnostic_ledger`, `RpcContext` | CLI |
| `bins/cli/src/` (3 files) | `doli forks`, `doli forks fleet`, `doli forks replay` | RPC client | External operator |

### Data Flow

```
hot path (block_handling, apply_block, rollback, fork_recovery, periodic)
  → DiagnosticEmitter::record(DiagnosticEvent)     [fire-and-forget]
    → AsyncChannelEmitter ring buffer (1024 slots)
      → diagnostic_writer task → DiagnosticLedger (RocksDB at <data_dir>/diagnostics/)
        → diagnostic_pruner task (every 60s, retention-based)
        → diagnostic_monitor (every 30s, reads ledger → classify → log WARN)
        → RPC getForkDiagnostic/getFleetForkDiagnostic (on-demand read)
          → CLI doli forks / doli forks fleet / doli forks replay
```

### Architectural Invariants (MUST hold ON or OFF)

1. **No consensus impact**: Diagnostics never write to UTXO, ChainState, ProducerSet, EpochState, or block fields. Verified via grep: zero matches for `diagnostic.*utxo_set|diagnostic.*producer_set|diagnostic.*chain_state.*write`. Gating diagnostics CANNOT change block content → no activation height needed, no synchronized deploy needed.
2. **Graceful degradation already exists**: If `DiagnosticLedger::open()` fails today, the node already runs with `NoOpEmitter` and `diagnostic_ledger: None`. RPC returns "Diagnostic ledger unavailable". This is REQ-FORKOBS-LEDGER-009 contract. Our OFF path REUSES this existing fallback.
3. **Fire-and-forget emission**: All emit sites use `let _ = self.diagnostic_emitter.record(...)`. The emitter is `Arc<dyn DiagnosticEmitter>` (virtual dispatch).

### Blast Radius

- **Direct**: `cli.rs`, `main.rs`, `run.rs`, `config.rs`, `node/init.rs`
- **Indirect**: None. No correctness path depends on diagnostics. CLI commands already handle the "unavailable" error response.

---

## Component Inventory

| # | Component | File:Line | Current Activation | Hot-Path Cost When OFF |
|---|-----------|-----------|-------------------|------------------------|
| E1 | `NoOpEmitter` init | `bins/node/src/node/init.rs:1085-1086` | Always (placeholder before ledger opens) | Zero |
| E2 | `DiagnosticLedger::open()` + `AsyncChannelEmitter::new(1024)` | `init.rs:1097-1101` | Always attempted in `Node::new()` | RocksDB open + 1024-slot VecDeque allocation |
| E3 | Writer task spawn | `init.rs:1107-1112` | When ledger opens | Tokio task polling every 100ms + heartbeat 60s |
| E4 | Pruner task spawn | `init.rs:1116-1119` | When ledger opens | Tokio task polling every 60s |
| E5-E7 | `emit_block_rejected` (2) + `emit_block_applied` (1) | `apply_block/diagnostics.rs:9,41`; `apply_block/mod.rs:94,106,491` | Every block apply | ULID gen + hex encoding + String allocs + vtable dispatch (event constructed THEN dropped by NoOp) |
| E8-E12 | `ForkBlockReceived` (5) | `block_handling.rs:168,201,262,311,434` | Every classified gossip block | Same |
| E13 | `ReorgExecuted` (1) | `block_handling.rs:1018` | Every reorg | Same |
| E14-E15 | `RollbackStarted` + `RollbackCompleted` | `rollback.rs:97,346` | Every rollback | Same |
| E16-E20 | `SnapSyncAttempted/Completed/Failed` + `RecoveryClassifyCall` (5) | `fork_recovery.rs:287,328,359,405,772` | Every snap sync + recovery | Same |
| E21 | `ChainBreakDetected` | `periodic.rs:398` | When chain break queued | Same |
| E22 | `RecoveryClassifyCall` | `periodic.rs:687` | When recovery action fires | Same |
| E23 | Diagnostic monitor | `periodic.rs:1044-1065` | Every 30s when `diagnostic_ledger.is_some()` | Already gated by Option |
| R1 | `getForkDiagnostic` RPC | `crates/rpc/src/methods/diagnostics.rs:48` | Always registered | Returns "unavailable" error if ledger is None |
| R2 | `getFleetForkDiagnostic` RPC | `diagnostics_fleet.rs` | Always registered | Same |
| R3 | RPC context wiring | `bins/node/src/node/startup.rs:360-369` | Always | Arc clones (cheap) |
| C1-C3 | CLI `doli forks` / `fleet` / `replay` | `bins/cli/src/cmd_forks*.rs` | Always | Client-side; no node cost |

### Key Finding: NoOpEmitter is not truly zero-cost today

Existing `NoOpEmitter` path constructs the full event (ULID + hex + Strings) BEFORE calling `record()`, which then drops it. True zero-cost requires gating event CONSTRUCTION at each emit site. On a 10s-slot chain this is nanoseconds vs. milliseconds of block validation — negligible. Captured as REQ-OBS-OPTIN-006 (Should, optional follow-up).

---

## Recommended Mechanism (SSF)

**One CLI flag: `--enable-diagnostics` (default: absent = OFF).**

### Design

1. **Flag location**: `bins/node/src/cli.rs` — add `#[arg(long)] enable_diagnostics: bool` to the `Start` variant.
2. **Propagation**: `cli.rs` → `main.rs` → `run.rs` → `NodeConfig` (new field `enable_diagnostics: bool`, default `false`).
3. **Init gate**: In `init.rs:1094-1137`, wrap the `DiagnosticLedger::open()` + emitter + writer + pruner block in `if config.enable_diagnostics { ... }`. When false, leave `diagnostic_ledger: None`, `diagnostic_emitter: Arc::new(NoOpEmitter)`, `diagnostic_shutdown_tx: None` — the same state today's graceful-degradation path produces.
4. **RPC wiring**: `startup.rs` already checks `if let Some(ref ledger) = self.diagnostic_ledger`. **No change needed**.
5. **Periodic monitor gate**: `periodic.rs:1044` already checks `if let Some(ref ledger)`. **No change needed**.
6. **Default**: `false`. Operators add `--enable-diagnostics` to their service file/launchd plist.
7. **Override precedence**: Single source: CLI flag. If env-var fallback is added later, it's a trivial `config.enable_diagnostics = cli_flag || env_var_set` in `run.rs`.

### Files Touched

| File | Change | Risk |
|------|--------|------|
| `bins/node/src/cli.rs` | Add `enable_diagnostics: bool` arg | Low |
| `bins/node/src/main.rs` | Pass new arg to `run()` | Low |
| `bins/node/src/run.rs` | Pass to `NodeConfig` | Low |
| `bins/node/src/config.rs` | Add `enable_diagnostics: bool` field (default false) | Low |
| `bins/node/src/node/init.rs` | Wrap diagnostic setup in `if config.enable_diagnostics` | Low |

5 files, all in `bins/node/src/`. Optional REQ-OBS-OPTIN-005 (friendlier CLI error) adds ~1 file in `bins/cli/src/`.

---

## Considered Alternatives (Appendix)

1. **Environment variable (`DOLI_ENABLE_DIAGNOSTICS=1`)** — Rejected as primary. DOLI's convention is CLI flags for subsystem on/off, env vars for secondary tuning (`DOLI_DIAG_RETENTION_DAYS`, `DOLI_FLEET_MAX_PEERS`). Could be a future fallback.
2. **Config file field** — Rejected. `NodeConfig` is constructed programmatically; no config-file loader exists. Adding one for a single boolean is overengineering.
3. **Compile-time feature flag (`--features diagnostics`)** — Rejected. Requires recompilation to toggle; conflicts with ~30 external producers sharing the same binary (per MEMORY.md `feedback_structural_vs_external_fleet.md`).

---

## Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|---------------------|
| REQ-OBS-OPTIN-001 | Default OFF: subsystem activates only when `--enable-diagnostics` is passed | Must | No `diagnostics/` RocksDB dir created; no writer/pruner spawned; `getForkDiagnostic` returns "unavailable" error |
| REQ-OBS-OPTIN-002 | ON path is bit-identical to today | Must | With flag: `diagnostics/` opens, writer+pruner spawn, all events recorded; existing diagnostic tests pass unchanged |
| REQ-OBS-OPTIN-003 | OFF state spawns no background tasks | Must | `diagnostic_ledger=None`, `diagnostic_emitter=NoOpEmitter`, `diagnostic_shutdown_tx=None`, zero tokio tasks for diagnostics |
| REQ-OBS-OPTIN-004 | RPCs remain registered but return error when OFF | Must | Dispatch table unchanged; error `{"code": -32603, "message": "Diagnostic ledger unavailable"}`; no panic |
| REQ-OBS-OPTIN-005 | CLI `doli forks` shows friendly message on unavailable | Should | "Diagnostics not enabled on this node (use --enable-diagnostics)"; non-zero exit |
| REQ-OBS-OPTIN-006 | Emit-site event construction gated to avoid allocations when OFF | **Must** | Hot-path emit helpers short-circuit on `emitter.is_noop()` before any ULID/hex/String construction; verified by inspection of 5 helpers |
| REQ-OBS-OPTIN-007 | `Node::new_for_test()` unaffected | Must | All `#[test]` and integration tests pass without changes |
| REQ-OBS-OPTIN-008 | Startup log line indicates diagnostic state | Could | ON: existing log; OFF: `[Diagnostics] Disabled (use --enable-diagnostics to activate)` |

---

## Impact Analysis

### Downstream consumers

- **`scripts/fork-monitor.sh`**: Uses `getChainInfo`, not diagnostic RPCs. **Not affected.**
- **`scripts/health-check.sh`**: Same. **Not affected.**
- **CLI `doli forks`**: Will get RPC error when targeting a node without `--enable-diagnostics`. Already-tested path. REQ-OBS-OPTIN-005 adds friendlier message.
- **External monitoring/dashboards**: None known to consume diagnostic RPCs.
- **In-node diagnostic_monitor**: Already gated by `if let Some(ref ledger)`. **Not affected.**

### Default operator experience tradeoff

A fresh node operator has NO in-ledger fork detection until they flip the flag. Mitigations: (a) `fork-monitor.sh` (tip-divergence detection via `getChainInfo`) continues to work regardless; (b) REQ-OBS-OPTIN-008 startup log tells the operator how to enable.

### Regression risk

- `init.rs` conditional must not accidentally gate non-diagnostic code.
- Test constructors must remain unaffected (`Node::new_for_test()` already passes `NoOpEmitter`).

---

## What WILL NOT change

- Consensus rules, block content, scheduler inputs, fork_id, NetworkParams.
- Diagnostic event types, schema, RocksDB layout (when ON).
- `getForkDiagnostic` / `getFleetForkDiagnostic` response schema (when ON).
- `getStateRootDebug` / `getUtxoDiff` — those are general debug RPCs in `stats.rs`, not part of fork-observability. They remain always-available.
- CLI commands (`doli forks ...`). Only the error-message wording changes (REQ-OBS-OPTIN-005).
- `Node::new_for_test()` test constructors.

No activation height needed. No synchronized deploy needed. No protocol version bump. No genesis reset.

---

## Open Questions for User

1. **Structural-node default**: Should the 15 structural nodes (N1-N12 + 3 seeds) have `--enable-diagnostics` added to their launchd/systemd service files as part of this work, preserving current behavior? Or do you want ALL nodes (structural and external) to go dark on diagnostics until explicitly enabled?

2. **Flag naming**: `--enable-diagnostics` (positive, clear) vs `--diagnostics` (shorter) vs `--fork-diagnostics` (more specific). Recommendation: `--enable-diagnostics` — matches `--enable-foo` mental model and avoids ambiguity with future debug RPCs.

3. **REQ-OBS-OPTIN-006 (zero-cost emit gating)**: Adds ~17 guard checks to emit sites. Practical perf impact on a 10s-slot chain is nanoseconds. Keep as **Should** (do if straightforward) or downgrade to **Won't** (defer)?

---

## Milestones

**Single milestone** — all changes in one PR, well within one agent's 60% budget.

- **M1 — Opt-in toggle (full)**:
  - `bins/node/src/cli.rs` — add `#[arg(long)] fork_diagnostics: bool`
  - `bins/node/src/main.rs` — propagate
  - `bins/node/src/run.rs` — propagate to `NodeConfig`
  - `bins/node/src/config.rs` — `fork_diagnostics: bool` field, default `false`
  - `bins/node/src/node/init.rs` — wrap `DiagnosticLedger::open()` + writer + pruner spawn block (lines 1094-1137) in `if config.fork_diagnostics { ... }`; add startup log line for OFF case
  - `bins/cli/src/cmd_forks.rs` + `cmd_forks_fleet.rs` + `cmd_forks_replay.rs` — friendlier message when RPC returns "unavailable" (REQ-OBS-OPTIN-005)
  - `crates/storage/src/diagnostic_ledger/emitter.rs` — add `fn is_noop(&self) -> bool { false }` to trait; override to `true` on `NoOpEmitter`
  - `bins/node/src/node/apply_block/diagnostics.rs` + any sibling emit helpers — add `if emitter.is_noop() { return; }` at the top of each helper (REQ-OBS-OPTIN-006)
  - Structural-node service files — add `--fork-diagnostics` to launchd plists / systemd units for N1-N12 + seeds (preserves current behavior on the structural fleet)
  - Requirements: REQ-OBS-OPTIN-001 through 008
  - Dependencies: none

---

## Traceability Matrix

| Requirement | Priority | Test IDs | Impl Module |
|-------------|----------|----------|-------------|
| REQ-OBS-OPTIN-001 | Must | (TBD) | bins/node/src/node/init.rs |
| REQ-OBS-OPTIN-002 | Must | (TBD) | bins/node/src/node/init.rs |
| REQ-OBS-OPTIN-003 | Must | (TBD) | bins/node/src/node/init.rs |
| REQ-OBS-OPTIN-004 | Must | (TBD) | crates/rpc/src/methods/diagnostics.rs (no change) |
| REQ-OBS-OPTIN-005 | Should | (TBD) | bins/cli/src/cmd_forks*.rs |
| REQ-OBS-OPTIN-006 | **Must** | (TBD) | crates/storage/src/diagnostic_ledger/emitter.rs (trait + NoOp) + bins/node/src/node/apply_block/diagnostics.rs (helpers) |
| REQ-OBS-OPTIN-007 | Must | (TBD) | bins/node/src/node/init.rs |
| REQ-OBS-OPTIN-008 | Could | (TBD) | bins/node/src/node/init.rs |

---

## Specs Drift Detected

None. `specs/fork-observability-architecture.md` and `specs/fork-observability-requirements.md` describe the subsystem accurately. This improvement adds an operational toggle on top of the existing design.
