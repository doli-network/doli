<!--
OUTPUT CONTRACT: N/A — requirements spec (not a test file)
INPUT PARTITIONS: N/A — requirements spec (not a test file)
-->

# Requirements: Agent-Consumable Fork-Diagnostic Observability (Phase 1)

**Workflow**: #346
**Date**: 2026-05-20
**Author**: Analyst agent
**Phase**: 1 (locked by user)

---

## Scope

### Phase 1 IN-SCOPE (workflow #346)

1. **Emitter layer** — read-only instrumentation calls in: `apply_block/mod.rs`, `block_handling.rs` (incl. `classify_gossip_block` — see Skeptic Reframe 5), `fork_recovery.rs`, `rollback.rs`, `periodic.rs` (recovery classify trace), network sync events
2. **DiagnosticLedger** — new SEPARATE RocksDB instance at `data/diagnostics/` (NOT a CF in state_db — see Skeptic Reframe 3 & "snap-sync wipe" risk) with single CF `cf_events`, composite key for range scans by (kind, height), bounded retention via periodic-task pruner
3. **One RPC method** — `getForkDiagnostic` returning a self-contained DiagnosticBundle (event chain + classification + baseline comparison)
4. **Deterministic classifier** — typed enum with `Unknown` variant carrying `reason_unknown` + `evidence_event_ids` (see Skeptic Reframe 4 — Unknown is the safety valve, not a defect)
5. **CLI subcommand** — `doli forks` with JSON default output, `--human` flag opt-in (see Skeptic Reframe 1 — human audit is preserved)
6. **Emit-latency benchmark** — gate: <50us per emit or async channel
7. **Unit tests** — emitter, ledger I/O, RPC, classifier, one tip-race reproduction test
8. **Docs** — `docs/rpc_reference.md`, `docs/troubleshooting.md`, new `docs/fork_observability.md`

### Phase 2 OUT-OF-SCOPE (deferred — DO NOT implement in #346)

- Historical-log replay tool (`doli forks replay --log <file>`)
- Retroactive INC-I-083 / INC-I-081 fixture replay tests
- `schemars` / JsonSchema export to `docs/fork_observability_schema.json`
- `getFleetForkDiagnostic` cross-fleet correlation RPC
- Fork honeypot debug mode
- Pre-fork warning stream / push alerts (see Skeptic Reframe 7 — acknowledged as a real gap but separate observability domain)
- Causality DAG / fork tree visualization
- Dashboard / explorer integration

---

## Skeptic Reconciliation (per Rule 4)

The skeptic raised 7 reframes. Position on each:

| # | Reframe | Position | Where addressed |
|---|---------|----------|-----------------|
| 1 | Human audit gap if JSON-only | **INCORPORATED** | REQ-FORKOBS-CLI-001: `--human` flag mandatory in Phase 1 |
| 2 | Bundle not self-contained without full RecoveryContext | **INCORPORATED** | REQ-FORKOBS-EMIT-007: full 12-field RecoveryContext in every recovery_classify_call event |
| 3 | JSONL vs RocksDB CF tradeoff | **PARTIALLY INCORPORATED** | Separate RocksDB instance (avoids snap-sync wipe coupling) BUT keep RocksDB for ordered range queries by (kind, height). JSONL export is Phase 2. |
| 4 | Classifier `Unknown` will dominate | **INCORPORATED** | REQ-FORKOBS-CLF-002 / REQ-FORKOBS-RETRO-003: Unknown variant MUST carry reason_unknown + evidence_event_ids; classifier admits ignorance instead of mis-classifying |
| 5 | Instrument classify_gossip_block, not just apply_block | **INCORPORATED** | REQ-FORKOBS-EMIT-003: fork_block_received event emitted at classify_gossip_block decision points (ForkBlock, Orphan) BEFORE apply_block |
| 6 | "Sky's the limit" is rhetorical | **DEFENDED** | User explicitly accepted Phase 1 scope via the AskUserQuestion flow. We are building Phase 1, not maximalist scope. Scope statement above is authoritative. |
| 7 | Push alerts beat pull RPC for detection latency | **DEFERRED to Phase 2** | Acknowledged as a real gap (2h detection lag vs 5s query lag) but is a separate observability domain (prediction/alerting) requiring different infrastructure. Out-of-scope explicitly flagged. |

---

## Summary

Every time a DOLI node forks or falls behind, diagnosing the cause takes 2-4 hours of grep across 18 log files. This feature adds structured event recording at every fork-relevant decision point in the node, stores those events in a bounded local database, and exposes them through one RPC call that returns a complete diagnostic verdict. A future Claude diagnostic sub-agent calls that one RPC and gets everything it needs to issue a verdict — no log grep, no timestamp correlation, no sub-investigators. A human operator can audit the agent's verdict via `--human` rendering.

---

## User Stories

- As a **diagnostic sub-agent** (future Claude session), I want to call ONE RPC on a node and receive a self-contained DiagnosticBundle so that I can issue a fork verdict without spawning sub-investigators or grepping logs.
- As a **diagnostic sub-agent**, I want the bundle to contain a typed classification enum (not free text) so that I can branch on the classification type programmatically without re-implementing fork-type inference logic.
- As a **human operator**, I want to run `doli forks` and see the most recent fork events in machine-readable JSON so that I can pipe them to other tools or read them directly.
- As a **human operator auditing an agent's verdict**, I want `doli forks --human` to render a readable summary so that I can verify the classifier's conclusion before acting.
- As a **human operator**, I want fork events to be bounded and auto-pruned so that they do not consume unbounded disk space.
- As a **node developer**, I want every fork-relevant decision point to emit a structured event with full causal context so that future incidents produce their own diagnosis automatically.

---

## Requirements

### EMITTER Layer (REQ-FORKOBS-EMIT-*)

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|-------------------|
| REQ-FORKOBS-EMIT-001 | When apply_block succeeds, emit a `block_applied` event containing: event_id (ULID), height, slot, block_hash, producer_pubkey (hex, first 8 chars), from_peer_id (if known), received_at_ms, applied_at_ms, validation_duration_ms, mode (Full/Light/Replay), tx_count | Must | Given a testnet node, when block h=100 is applied via gossip, the ledger contains exactly one `block_applied` event for h=100 with all specified fields non-null. from_peer_id is populated for gossip blocks, null for self-produced. validation_duration_ms >= 0. |
| REQ-FORKOBS-EMIT-002 | When apply_block fails validation, emit a `block_rejected` event containing: event_id, height, slot, block_hash, producer_pubkey, from_peer_id, rejection_reason (string), mode | Must | Given a node receiving an invalid block, the ledger contains a `block_rejected` event with rejection_reason matching the validation error message. |
| REQ-FORKOBS-EMIT-003 | When `classify_gossip_block` returns `ForkBlock` or `Orphan`, emit a `fork_block_received` event containing: event_id, block_hash, block_slot, block_height_estimate, producer_pubkey, from_peer_id, classification (ExtendsTip/ForkBlock/Orphan/Rejected), fork_kind (if ForkBlock: HeightOccupied/ReorgCandidate), local_tip_hash, local_tip_height. **Per Skeptic Reframe 5, this is a FIRST-CLASS emit point — fork decisions happen here BEFORE apply_block.** | Must | Given two competing blocks for the same slot, the node emits a `fork_block_received` event for the second block with classification=ForkBlock. Given an orphan block, the node emits the event with classification=Orphan. |
| REQ-FORKOBS-EMIT-004 | When a rollback begins, emit `rollback_started` with: event_id, from_height, to_height, trigger (shallow_recovery/reorg/manual), cumulative_depth | Must | Given a shallow rollback from h=50 to h=49, a `rollback_started` event exists with from_height=50, to_height=49. |
| REQ-FORKOBS-EMIT-005 | When a rollback completes, emit `rollback_completed` with: event_id, caused_by_event_id (the rollback_started event), from_height, to_height, duration_ms, success (bool) | Must | The `rollback_completed` event's caused_by_event_id matches the preceding `rollback_started` event_id. |
| REQ-FORKOBS-EMIT-006 | When fork recovery completes a reorg evaluation, emit `reorg_executed` with: event_id, old_tip_hash, new_tip_hash, rollback_depth, applied_count, weight_delta, trigger_block_hash, trigger_from_peer_id | Must | After a successful reorg from tip A to tip B, a `reorg_executed` event records both hashes and the weight_delta. |
| REQ-FORKOBS-EMIT-007 | When RecoveryCoordinator.classify() is invoked and returns a non-None action, emit `recovery_classify_call` with: event_id, full RecoveryContext (local_height, network_tip_height, peer_count, last_applied_secs, shallow_rollback_count, snap_attempts, in_grace_period, last_finality_height, empty_count, deep_fork, rollback_exhausted, large_gap), action_returned, rule_matched. **Per Skeptic Reframe 2, ALL 12+ fields MUST be captured — not just the action output.** | Must | When classify() returns ShallowRollback, the ledger contains a `recovery_classify_call` event with action=ShallowRollback and ALL 12+ RecoveryContext fields populated. |
| REQ-FORKOBS-EMIT-008 | When snap sync is attempted, emit `snap_sync_attempted` with: event_id, local_height, target_height, source_peer_id. When snap sync completes/fails, emit `snap_sync_completed` / `snap_sync_failed` with: event_id, caused_by_event_id, result, duration_ms, error (if failed) | Should | A snap sync attempt produces exactly one `snap_sync_attempted` event followed by exactly one completed/failed event with matching caused_by. |
| REQ-FORKOBS-EMIT-009 | When chain_break is detected in header validation (sync/headers.rs), emit `chain_break_detected` with: event_id, expected_prev_hash, actual_prev_hash, header_slot, valid_so_far_count, from_peer_id | Should | A chain break in header sync produces a `chain_break_detected` event with the mismatched hashes. |
| REQ-FORKOBS-EMIT-010 | Every emitted event MUST carry a `correlation_key` field: `(divergence_height: Option<u64>, canonical_hash: Option<hex_string>, fork_hash: Option<hex_string>)`. Canonical block_applied events have all three None. Fork-related events populate at least divergence_height. | Should | Fork events from two nodes experiencing the same fork share identical (divergence_height, canonical_hash) values, enabling cross-node JOIN. |
| REQ-FORKOBS-EMIT-011 | Peer provenance threading: the Node struct gains a `last_block_source: Option<(Hash, PeerId, u64)>` field set by `handle_new_block` before calling `apply_block`. The emitter reads this inside apply_block to populate `from_peer_id` and `received_at_ms`. Avoids changing apply_block's signature. | Must | Self-produced blocks have from_peer_id=null. Gossip blocks have from_peer_id set to the source peer's PeerId string. |

### LEDGER Layer (REQ-FORKOBS-LEDGER-*)

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|-------------------|
| REQ-FORKOBS-LEDGER-001 | Diagnostic events are persisted to a **SEPARATE** RocksDB instance at `<data_dir>/diagnostics/` with a single column family `cf_events`. The DB uses `create_if_missing(true)`, `create_missing_column_families(true)`, Lz4 compression. **Per Skeptic Reframe 3: separate DB (NOT a CF in state_db) to avoid snap-sync wipe coupling.** | Must | After node startup, `<data_dir>/diagnostics/` exists. DB is separate from block_store and state_db. Snap sync of state_db does NOT touch `data/diagnostics/`. |
| REQ-FORKOBS-LEDGER-002 | Event keys are composite: `[event_kind_u8][height_u64_be][ulid_16_bytes]`. Enables prefix scans by event kind and range scans by height within a kind. | Must | Querying all `block_applied` events between h=100 and h=200 returns only events in that range, ordered by height then ULID. |
| REQ-FORKOBS-LEDGER-003 | Event values are bincode-serialized Rust structs (one enum variant per event kind) with a `schema_version: u16` header byte prefix. Additive-only field changes; removal requires deprecation cycle. | Must | Events written at schema_version=1 can be deserialized by schema_version=1 code. New fields added in schema_version=2 use `#[serde(default)]` for backward compat. |
| REQ-FORKOBS-LEDGER-004 | Bounded retention: a periodic-task pruner runs every 60 seconds and deletes events older than the configured retention period. Default: 30 days. Configurable via `DOLI_DIAG_RETENTION_DAYS`. | Must | After 30 days, events from day 1 are no longer returned by range queries. Setting DOLI_DIAG_RETENTION_DAYS=1 prunes events older than 24h. |
| REQ-FORKOBS-LEDGER-005 | Event count cap: if total events exceed 100,000, the pruner deletes oldest events first regardless of age. Configurable via `DOLI_DIAG_MAX_EVENTS`. | Must | With max_events=100, inserting event 101 causes the oldest event to be pruned within 60 seconds. |
| REQ-FORKOBS-LEDGER-006 | The DiagnosticLedger exposes `record(event: DiagnosticEvent) -> Result<()>` that writes synchronously. If emit latency exceeds 50us (per REQ-FORKOBS-PERF-001), switches to async mpsc channel with a background writer. | Must | The `record()` method does not panic and returns Ok on success. |
| REQ-FORKOBS-LEDGER-007 | `query_range(kind, min_height, max_height, limit) -> Vec<DiagnosticEvent>` for the RPC layer. | Must | Querying with kind=Some(block_rejected), min_height=100, max_height=200 returns only block_rejected events in that range. |
| REQ-FORKOBS-LEDGER-008 | `query_recent(duration_secs, limit) -> Vec<DiagnosticEvent>` for time-window queries. | Must | Querying with duration_secs=3600 returns events from the last hour. |
| REQ-FORKOBS-LEDGER-009 | If the diagnostics DB fails to open on startup, the node MUST continue operating normally. `diagnostic_ledger` becomes a no-op stub. A WARN log is emitted once. | Must | Deleting `data/diagnostics/` and restarting the node does not crash it. A warning is logged about diagnostics being unavailable. |

### RPC Layer (REQ-FORKOBS-RPC-*)

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|-------------------|
| REQ-FORKOBS-RPC-001 | New RPC `getForkDiagnostic` accepts optional params: `{"window_secs": u64, "limit": u64, "fork_event_id": string}`. Returns a `DiagnosticBundle` JSON object. | Must | Calling `getForkDiagnostic` with `{"window_secs": 3600}` returns a valid JSON bundle. Calling with `{"fork_event_id": "..."}` returns the bundle for that specific fork event. |
| REQ-FORKOBS-RPC-002 | The DiagnosticBundle struct contains: `schema_version` (u16), `node_peer_id` (string), `query_timestamp_ms` (u64), `events` (array), `fork_summary` (object), `classification` (object or null), `baseline` (object). | Must | The returned JSON contains all specified top-level fields. schema_version is 1. |
| REQ-FORKOBS-RPC-003 | The `fork_summary` aggregates: `fork_events_in_window`, `by_producer` (map), `by_event_kind` (map), `first_fork_height`, `last_fork_height`. | Must | After 5 fork events from 2 producers, `by_producer` has 2 entries summing to 5. |
| REQ-FORKOBS-RPC-004 | When `fork_event_id` is provided, the bundle returns the causal chain: the specified event plus all events linked via `caused_by_event_id`, ordered oldest-first. | Should | A rollback_completed event references rollback_started; querying rollback_completed's ID returns both events. |
| REQ-FORKOBS-RPC-005 | If `diagnostic_ledger` is unavailable, the RPC returns: `{"code": -32603, "message": "Diagnostic ledger unavailable"}`. | Must | When diagnostics are disabled, calling getForkDiagnostic returns the specified error. |
| REQ-FORKOBS-RPC-006 | The RPC is added to `dispatch.rs` and implemented in a new `diagnostics.rs` module under `crates/rpc/src/methods/`. | Must | The method is callable via JSON-RPC with method name "getForkDiagnostic". |

### CLASSIFIER (REQ-FORKOBS-CLF-*)

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|-------------------|
| REQ-FORKOBS-CLF-001 | A deterministic classifier function takes `&[DiagnosticEvent]` and returns a `Classification` with: `fork_type` (typed enum), `confidence` (f64 0.0-1.0), `evidence_event_ids` (Vec). | Must | Given two block_applied events for same height with different hashes, the classifier returns `TipRaceNatural` or `TipRaceHighLatency`. confidence ∈ [0.0, 1.0]. |
| REQ-FORKOBS-CLF-002 | ForkType enum has exactly: `TipRaceNatural`, `TipRaceHighLatency`, `ProducerEquivocation`, `EpochBoundaryInvalid`, `PostSnapDeadTip`, `ValidationDisagreement`, `RollbackLoop`, `SnapSyncToMinorityFork`, `Unknown`. **Per Skeptic Reframe 4 & REQ-FORKOBS-RETRO-003: `Unknown` carries `reason_unknown: String` and `evidence_event_ids: Vec<String>` so the bundle escalates to human review instead of misclassifying.** | Must | The enum is exhaustive. `Unknown` variant carries `reason_unknown` (non-empty) and `evidence_event_ids`. |
| REQ-FORKOBS-CLF-003 | The classifier is a pure function: no I/O, no network, no state. All input from the event slice. | Must | The classifier function signature takes `&[DiagnosticEvent]` and returns `Classification`. No I/O in the function body. |
| REQ-FORKOBS-CLF-004 | The classifier populates `recommended_action` (string) and `recommended_action_args` (optional JSON). | Should | TipRaceNatural returns recommended_action="normal_operation". RollbackLoop returns recommended_action="investigate_recovery_params". |
| REQ-FORKOBS-CLF-005 | Classification rules (deterministic, match-based): (a) two block_applied for same height same producer → ProducerEquivocation; (b) block_rejected at epoch boundary with "missing EpochReward" → EpochBoundaryInvalid; (c) >3 rollback_started in 60s → RollbackLoop; (d) snap_sync_completed followed by fork_block_received → PostSnapDeadTip; (e) fork_block_received with validation_duration > 2000ms → TipRaceHighLatency; (f) fork_block_received with validation_duration < 500ms and no other signals → TipRaceNatural; (g) otherwise → Unknown with reason_unknown describing what was inconclusive. | Must | Each rule is exercised by a fixture that triggers exactly that classification. |

### CLI Layer (REQ-FORKOBS-CLI-*)

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|-------------------|
| REQ-FORKOBS-CLI-001 | New `doli forks` subcommand. Default: machine-readable JSON (the full DiagnosticBundle). `--human` flag renders a human-readable summary. **Per Skeptic Reframe 1: --human is mandatory in Phase 1, not optional polish.** | Must | `doli forks` outputs valid JSON parseable by `jq`. `doli forks --human` outputs a formatted text summary. |
| REQ-FORKOBS-CLI-002 | `doli forks` accepts `--last <duration>` (e.g., `1h`, `30m`, `24h`). Default: 1h. | Must | `doli forks --last 1h` queries the last 3600 seconds. |
| REQ-FORKOBS-CLI-003 | `doli forks --explain` returns the DiagnosticBundle for the most recent fork event with full causal chain and classification. | Should | `doli forks --explain` returns a bundle with exactly one fork event and its causal chain. |
| REQ-FORKOBS-CLI-004 | `doli forks --by-producer` returns aggregation grouped by producer_pubkey, sorted by count desc. | Should | After 5 fork events from 2 producers, output shows 2 entries sorted by count. |

### PERFORMANCE (REQ-FORKOBS-PERF-*)

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|-------------------|
| REQ-FORKOBS-PERF-001 | Benchmark `DiagnosticLedger::record()` in the apply_block hot path. If median latency >50us over 1000 iterations, switch to async mpsc channel with a background writer. | Must | Benchmark exists. Either median <50us (sync) OR async channel is used (documented switch). |
| REQ-FORKOBS-PERF-002 | The DiagnosticLedger does not hold any lock also held during apply_block state mutations. Ledger write is lock-free vs state_db or behind an independent lock. | Must | No deadlock possible between diagnostic writes and state_db/block_store writes. |

### DOCUMENTATION (REQ-FORKOBS-DOC-*)

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|-------------------|
| REQ-FORKOBS-DOC-001 | `docs/rpc_reference.md` updated with `getForkDiagnostic`: params, return type, example request/response. | Must | Method documented with a complete example. |
| REQ-FORKOBS-DOC-002 | `docs/troubleshooting.md` updated with "How to diagnose a fork" section showing the `doli forks` workflow. | Must | A new section exists with step-by-step instructions. |
| REQ-FORKOBS-DOC-003 | New `docs/fork_observability.md`: event kinds, event fields, DiagnosticBundle schema, classification types, retention policy, configuration env vars. Schema-first, agent-readable. | Must | Document exists and covers all event kinds and their fields. |
| REQ-FORKOBS-DOC-004 | Commit message includes the three-question consensus-shape checklist: Q1=NO, Q2=NO, Q3=YES, with brief justification for each. | Must | Checklist present in the commit message. |

### SECURITY (REQ-FORKOBS-SEC-*)

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|-------------------|
| REQ-FORKOBS-SEC-001 | No PII in diagnostic events: PeerId (libp2p multihash) allowed. Raw IP addresses MUST NOT appear in any event field. Producer pubkeys truncated to first 8 hex chars in display but may be stored in full. | Must | Grep across all emitter call sites confirms no IP address fields. PeerId is the only peer identifier used. |
| REQ-FORKOBS-SEC-002 | `getForkDiagnostic` is read-only — no state mutation. | Must | Method implementation contains no write operations on any state. |
| REQ-FORKOBS-SEC-003 | `limit` parameter capped at 10,000. Requests exceeding this are silently clamped. Prevents DoS via unbounded result sets. | Must | Requesting limit=999999 returns at most 10,000 events. |
| REQ-FORKOBS-SEC-004 | DO NOT MODIFY decision logic in: `crates/core/src/consensus.rs`, `crates/core/src/network_params` activation heights, `bins/node/src/node/apply_block/*` (emit calls allowed, logic changes not), `crates/storage/src/snapshot.rs`, `crates/core/src/validation/*`. Merge blocker — reviewer MUST verify no decision logic changed. | Must | Diff of apply_block files shows only new emit calls, no changed conditionals or return values. No file in constraint list has modified decision logic. |
| REQ-FORKOBS-SEC-005 | Observability is default-ON. No env var/flag/config required to enable emitter/ledger/RPC. Debug/honeypot features (Phase 2) are default-OFF. | Must | A fresh node with no special configuration emits diagnostic events and responds to getForkDiagnostic. |
| REQ-FORKOBS-SEC-006 | Safe for rolling deploy: no block content change (INC-I-062 check: NO), no consensus rules change (INC-I-075 check: NO), no activation height or HardForkSchedule entry. | Must | No activation height added in network_params. No HardForkSchedule entry added. |

### RETROACTIVE VALIDATION (REQ-FORKOBS-RETRO-*)

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|-------------------|
| REQ-FORKOBS-RETRO-001 | Architect MUST defend the DiagnosticBundle schema against INC-I-083 ON PAPER. Demonstrate that captured events on n10/n14 during the incident would carry sufficient data for the classifier to output a correct verdict (likely `PostSnapDeadTip` or `Unknown` with the dead-fork evidence). | Must | Architecture doc contains "INC-I-083 Schema Adequacy" section with event trace and expected classification. |
| REQ-FORKOBS-RETRO-002 | Architect MUST defend the DiagnosticBundle schema against INC-I-081 ON PAPER: events from the broken producer's node (missing EpochReward block) and rejecting nodes would produce `EpochBoundaryInvalid`. | Must | Architecture doc contains "INC-I-081 Schema Adequacy" section with event trace and expected classification. |
| REQ-FORKOBS-RETRO-003 | The `Unknown` classification variant MUST carry `reason_unknown` and `evidence_event_ids` to enable human escalation when the classifier cannot determine the fork type. The classifier prefers `Unknown` with high evidence over a wrong specific variant. | Must | The Unknown variant struct has both fields. A fixture constructs an unclassifiable event sequence and verifies both fields are populated. |

---

## Architecture Context

### Module Boundaries

| Module | Responsibility | Depends on | Depended by |
|--------|---------------|------------|-------------|
| `crates/storage` (DiagnosticLedger) | Persist/query diagnostic events via separate RocksDB | `rocksdb`, `serde`, `bincode`, `ulid` | `bins/node` (emitter), `crates/rpc` (query) |
| `bins/node` (emitter sites) | Emit events at fork-relevant decision points | `crates/storage::DiagnosticLedger` | None (fire-and-forget) |
| `crates/rpc` (diagnostics) | Expose DiagnosticBundle via JSON-RPC | `crates/storage::DiagnosticLedger` | `bins/cli` (via HTTP) |
| `bins/cli` (cmd_forks) | CLI subcommand calling RPC | `crates/rpc` (via HTTP client) | None |
| `crates/core` or `crates/storage` (types) | DiagnosticEvent, Classification, DiagnosticBundle | `serde` | All of the above |

### Architectural Invariants

1. **apply_block is consensus-critical** — emit calls MUST NOT alter control flow. Emit failures MUST NOT propagate as errors. Use `let _ = ledger.record(...)` or equivalent.
2. **DiagnosticLedger is separate from state_db** — separate RocksDB instance. Prevents diagnostic corruption from affecting consensus and vice versa. Survives snap-sync wipe of state_db.
3. **No lock contention** — DiagnosticLedger uses its own DB with no shared locks.
4. **Graceful degradation** — diagnostic DB failure does not affect node operation.
5. **Event ordering** — ULID gives monotonic ordering per node. Cross-node ordering uses wall-clock timestamps + correlation_key tuple.
6. **Events fire AFTER decisions, never before** — emit cannot affect the decision.

### Blast Radius

- **Direct impact**: Node struct (new fields), block_handling.rs (~5 emit sites incl. classify_gossip_block), apply_block/mod.rs (2), fork_recovery.rs (2), rollback.rs (2), periodic.rs (pruner + classify emit), RpcContext, dispatch.rs, commands.rs.
- **Indirect impact**: None — all changes additive. No existing behavior modified.
- **Risk assessment**: LOW. Read-only instrumentation. Only non-trivial risk is emit latency, mitigated by REQ-FORKOBS-PERF-001.

---

## Assumptions

| # | Assumption | Confirmed |
|---|------------|-----------|
| 1 | `ulid` crate acceptable as new dependency | No — architect decides |
| 2 | Separate RocksDB instance adds <50ms to startup | No — to be measured |
| 3 | periodic.rs needs a submodule for the pruner to stay <500 lines | No — architect decides |
| 4 | `bincode` acceptable for event values | No — architect decides |
| 5 | `last_block_source` side-channel preferred over changing apply_block signature | No — architect decides |
| 6 | Events fire AFTER decisions, never before — invariant | Yes — by requirement |

---

## Identified Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Emit latency >50us, slowing apply_block | Medium | Medium | REQ-FORKOBS-PERF-001 benchmark gate, async channel fallback |
| periodic.rs exceeds 500-line module limit | High | Low | Extract pruner to submodule |
| ULID dependency rejected | Low | Low | Fall back to custom (timestamp_ms, counter) |
| DiagnosticLedger corruption corrupts consensus | None (separate DB) | Critical if not separated | REQ-FORKOBS-LEDGER-001 + graceful degradation |
| Schema needs breaking changes before Phase 2 | Low | Medium | schema_version + additive-only policy |
| Classifier dominated by Unknown variant (Skeptic Reframe 4) | Medium | Low (Unknown is safe failure mode) | Unknown carries reason_unknown + evidence_event_ids for escalation |

---

## Priority Summary

| Priority | Count |
|---|---|
| Must | 39 |
| Should | 8 |
| Could | 0 |
| Won't (Phase 2) | 8 |
