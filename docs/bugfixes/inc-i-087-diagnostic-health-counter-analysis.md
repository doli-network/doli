# INC-I-087 — Diagnostic Health Counter Analyst Report

- **Incident**: INC-I-087
- **Run**: 354
- **Branch**: feature/fork-observability-346
- **Severity**: Low
- **Triage verdict**: **FAST** — single file:line, deterministic substitution bug, single component, no consensus impact.

## 1. Bug confirmation

`crates/rpc/src/methods/diagnostics.rs:91-96` returns a `DiagnosticHealth` whose three observable fields are hardcoded literals:

```rust
let health = DiagnosticHealth {
    ledger_available: true,          // OK — diagnostic_ledger.is_some() implied above
    events_written_total: 0,         // BUG — always 0
    events_dropped_total: 0,         // BUG — always 0
    last_heartbeat_ms: None,         // BUG — always None
};
```

The live counter source is the `WriterHeartbeat` event payload (`crates/storage/src/diagnostic_ledger/types.rs:166-169`):

```rust
WriterHeartbeat {
    events_written_total: u64,
    events_dropped_total: u64,
},
```

The corresponding `last_heartbeat_ms` is the heartbeat event's own `timestamp_ms` (`DiagnosticEvent.timestamp_ms`, types.rs:207).

## 2. Wiring map

- **Producer of the counters**: `bins/node/src/node/diagnostic_writer.rs`.
  - `run_writer_task` (line 31) owns two `AtomicU64` instances, `events_written` and `events_dropped`, declared **locally** at lines 38-39.
  - Every 60s `write_heartbeat()` (lines 107-132) snapshots both counters into a `WriterHeartbeat` event and persists it via `ledger.record(...)`.
- **Spawn site**: `bins/node/src/node/init.rs:1042-1074`. The ledger `Arc<DiagnosticLedger>` is created, then the writer task is spawned with `(receiver, writer_ledger, shutdown_rx)`. The atomics are NOT exposed to the spawning scope today.
- **RPC plumbing**: `bins/node/src/node/startup.rs:361-362` calls `context.with_diagnostic_ledger(Some(ledger.clone()))`. The `RpcContext` (crates/rpc/src/methods/context.rs:100) holds `diagnostic_ledger: Option<Arc<DiagnosticLedger>>` — the **ledger** is wired, but the writer's atomic counters are not.
- **Gap**: the RPC handler has read access to the ledger contents but no path to the live atomics. Today it just hardcodes `0/0/None` instead of either (a) consulting the ledger for the latest WriterHeartbeat or (b) reading from a shared atomic.

M2 commit history confirms the writer was introduced in `251f5d73` (M2 follow-up — implement writer + pruner tasks + init wiring), with apply_block provenance (`1ffc5df8`) and EMIT-006/007 wiring (`259f6380`).

## 3. Fix approach recommendation

**Recommendation: shared atomics.** Lift `events_written` and `events_dropped` out of `run_writer_task` into a small `Arc<DiagnosticWriterStats>` (struct holding two `AtomicU64` plus an `AtomicU64` for `last_heartbeat_ms`). Construct it in `init.rs` alongside the ledger, hand a clone to `run_writer_task`, store another clone on `Node` (or directly on the RPC context via a new `with_diagnostic_writer_stats(...)` setter), and read it in the RPC handler with `Ordering::Relaxed`. The handler can then populate all three fields synchronously and consistently without a RocksDB scan on every request.

Why not the latest-heartbeat lookback: it would (1) require an extra ledger query per RPC call, (2) only refresh once per 60s (the heartbeat interval), (3) report a stale `last_heartbeat_ms` between ticks, and (4) need a new `query_latest_by_kind(WriterHeartbeat)` helper (today only `query_recent` / `query_range` / `query_causal_chain` exist). Shared atomics are cheaper, fresher, and reuse machinery that already exists in `diagnostic_writer.rs` — the atomics are already there; we only need to share them. Lowest-churn fit for current wiring.

## 4. Files that will need to change (estimate)

1. **`bins/node/src/node/diagnostic_writer.rs`** — extract `events_written`, `events_dropped`, and a new `last_heartbeat_ms` `AtomicU64` into a `pub struct DiagnosticWriterStats` (or similar); accept `Arc<DiagnosticWriterStats>` as a new parameter on `run_writer_task` instead of allocating them inside. Update `write_heartbeat` to also record `last_heartbeat_ms` into the shared atomic.
2. **`bins/node/src/node/init.rs`** — construct `Arc<DiagnosticWriterStats>` (around line 1045), pass a clone to `run_writer_task`, store another clone on `node.diagnostic_writer_stats` (new field).
3. **`bins/node/src/node/mod.rs`** — add `pub diagnostic_writer_stats: Option<Arc<DiagnosticWriterStats>>` field on `Node`.
4. **`bins/node/src/node/startup.rs`** — call new `context.with_diagnostic_writer_stats(...)` setter alongside the existing `with_diagnostic_ledger(...)`.
5. **`crates/rpc/src/methods/context.rs`** — add `pub diagnostic_writer_stats: Option<Arc<DiagnosticWriterStats>>` field + `with_diagnostic_writer_stats(...)` builder method; update both `new_for_network` and `new` constructors to initialize it to `None`.
6. **`crates/rpc/src/methods/diagnostics.rs`** — replace the hardcoded literals at 91-96 with reads from `self.diagnostic_writer_stats` (each atomic via `Ordering::Relaxed`; `None` when the stats handle is absent so test fixtures still build).
7. **Cargo deps** — `DiagnosticWriterStats` lives in `bins/node/src/node/diagnostic_writer.rs`, but `crates/rpc` needs to see the type. Cleanest move: define the struct in `crates/storage/src/diagnostic_ledger/` (e.g., a new `writer_stats.rs` re-exported from `diagnostic_ledger::mod`) so both `bins/node` and `crates/rpc` can depend on it without a new crate edge. (No new edge — `rpc` already depends on `storage`.)

## 5. Test plan

- **Location**: a new unit test in `crates/rpc/src/methods/diagnostics.rs` (or a sibling `diagnostics_tests.rs` module under `#[cfg(test)]`). Test name: `getDiagnosticHealth_reports_live_counter_values`.
- **OUTPUT CONTRACT**: outputs of `get_fork_diagnostic` = the four `DiagnosticHealth` fields (`ledger_available`, `events_written_total`, `events_dropped_total`, `last_heartbeat_ms`).
- **INPUT PARTITIONS**:
  1. stats unset (`None`) — handler must still build; `events_written_total=0, events_dropped_total=0, last_heartbeat_ms=None` (matches today's behavior when no writer is running).
  2. stats present, counters non-zero — assert exact pass-through of arbitrary fixture values (e.g., 7 written, 3 dropped, heartbeat at 1_700_000_000_000).
  3. stats present, counters zero but heartbeat seen — `last_heartbeat_ms = Some(ts)`, both totals = 0.
- **FAIL→PASS sequence**: pin the bug FIRST with partition (2) — build an `RpcContext` with `diagnostic_writer_stats` set to a fresh `Arc<DiagnosticWriterStats>` whose atomics are pre-loaded to non-zero. Today the handler ignores the stats handle (because it doesn't exist), so the test cannot even compile against the stub — first commit adds the field with stub propagation so the test compiles and FAILS by asserting `7 == 0`. Then the fix wires the read and the test goes PASS.
- **Acceptance**: `cargo test -p rpc --lib diagnostics`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`.

## 6. Out of scope (per refinement)

- Fleet test fixture under `crates/network/tests/` etc.
- Replay path — already correctly zeroed; do not modify.
