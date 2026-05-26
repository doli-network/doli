# System Blueprint: observability-fork subsystem (INC-I-090)

**INC**: INC-I-090  
**RUN_ID**: 372  
**Scope**: observability-fork diagnostic pipeline  
**Author**: architect  
**Evidence basis**: Skill documents (.claude/skills/observability-fork/SKILL.md, LEDGER-SCHEMA.md) cross-referenced with analyst analysis. Code verification delegated to investigators (pipeline gate enforced sub-agent delegation for source reads).

---

## 1. Subsystem Boundary

### IN scope (observability-fork subsystem)

| Component | Location | Boundary justification |
|-----------|----------|----------------------|
| Diagnostic event emission sites | `bins/node/src/node/block_handling.rs:154-418` (per skill) | These are the points where consensus-adjacent code calls `DiagnosticEmitter::record`. The EMIT decision is observability. |
| Fork recovery emission sites | `bins/node/src/node/fork_recovery.rs` | Recovery path should emit `RecoveryClassifyCall`, `SnapSyncAttempted/Completed/Failed`. |
| AsyncChannelEmitter (ring buffer) | `crates/storage/src/diagnostic_ledger/emitter.rs` | The transport between emit site and writer. Drop-oldest overflow semantics are observability. |
| Writer task + RocksDB persistence | `crates/storage/src/diagnostic_ledger/mod.rs` | Drains ring buffer, serializes to `cf_events`. |
| Event types + serialization | `crates/storage/src/diagnostic_ledger/types.rs` | Schema version, EventKind enum, EventPayload variants, key layout. |
| Classifier (8 rules) | `crates/storage/src/diagnostic_ledger/classifier.rs` | Pure function. Assigns `ForkType` + `recommended_action` to event slices. |
| RPC surface (4 diagnostic methods) | `crates/rpc/src/methods/diagnostics.rs`, `diagnostics_fleet.rs` | `getForkDiagnostic`, `getFleetForkDiagnostic`. |
| RPC registration | `crates/rpc/src/methods/dispatch.rs:74-76` | Where methods are wired into the dispatch table. |
| State-root debug RPCs | `crates/rpc/src/methods/stats.rs` | `getStateRootDebug`, `getUtxoDiff` — cross-node comparison surface. |
| Operator scripts | `scripts/fork-monitor.sh`, `scripts/health-check.sh` | The only documented consumers of diagnostic data. |

### OUT of scope (adjacent but not observability)

| Component | Why excluded |
|-----------|-------------|
| Recovery state machine correctness (recovery.rs:312 fencepost) | Already diagnosed in `docs/.workflow/domain-diagnosis-report.md`. The fencepost is the FORK root cause, not the observability gap. |
| `apply_block()` logic | Consensus correctness, not observability. The observability system observes the OUTCOMES of apply_block, not its internals. |
| Sync state machine (manager.rs, sync_engine.rs) | Sync correctness is out of scope. Whether sync EMITS diagnostic events when it acts is in scope. |
| `crates/network/src/sync/manager/recovery.rs` | IN scope only for the narrow question: does the recovery coordinator path emit DiagnosticEvent when it iterates? The recovery logic itself is OUT. |
| Block production (`production.rs`) | Production scheduling correctness is out. Whether production of a minority block triggers observability events is in. |

---

## 2. Module Responsibility Map

### 2.1 Emission layer (L1)

**`bins/node/src/node/block_handling.rs` (lines 154-418 per skill)**

- **Responsibility**: Classify incoming gossip blocks and emit `ForkBlockReceived` events for non-tip arrivals (orphans, height-occupied, reorg candidates, rejected blocks).
- **Public surface**: `handle_new_block()` (called from event loop on gossip block arrival). Internally calls `classify_gossip_block()` to determine `BlockClass`, then emits `DiagnosticEvent` via `self.diagnostic_emitter.record(...)`.
- **Hot-path**: YES. Every gossip block arrival goes through this. The `let _ =` pattern on the emitter call is intentional — fire-and-forget to avoid blocking consensus.
- **Failure modes tolerated by design**:
  - Emitter `record()` returns `Err` — silently discarded (`let _ =`). By design.
  - Ring buffer full — oldest event evicted, `events_dropped_total` incremented. By design.
  - Emitter is `NoOpEmitter` — all events silently dropped. Graceful degradation when ledger unavailable.

**Skill claims to verify against code** (investigators must confirm):
- Skill says `ForkBlockReceived` is emitted for `BlockClass::ForkBlock(HeightOccupied)` at lines 195-258. Code must be checked.
- Skill says `canonical_hash=None` at emit time (only `fork_hash` set). Code must be checked.
- Skill says `BlockApplied` events are emitted from `apply_block.rs` with `BlockProvenance`. Code must be checked.

**`bins/node/src/node/fork_recovery.rs`**

- **Responsibility**: Orchestrate recovery when node detects it's on a fork (via FORK_GUARD or stuck detection). Should emit `RecoveryClassifyCall`, `SnapSyncAttempted`, `SnapSyncCompleted`, `SnapSyncFailed`.
- **Public surface**: Recovery entry points called from event loop or periodic tasks.
- **Hot-path**: NO (cold path — only fires during fork recovery).
- **Failure modes**: Same fire-and-forget pattern as block_handling.

**`crates/network/src/sync/manager/recovery.rs`**

- **Responsibility**: The sync recovery coordinator. Contains the FINALITY_GUARD fencepost (line 312) that is the root cause of INC-I-090's FORK. The observability question is narrow: does this module emit any `DiagnosticEvent` when it iterates recovery attempts?
- **Public surface**: `classify_recovery_action()`, `execute_recovery()` (or similar — names from skill, need code verification).
- **Hot-path**: NO (cold path — only during sync recovery).
- **CRITICAL QUESTION**: The `RecoveryClassifyCall` EventKind exists in the schema. The skill claims it's emitted. If the sync recovery coordinator does NOT call `DiagnosticEmitter::record(RecoveryClassifyCall)`, then the 253 recovery iterations during INC-I-090 were invisible to the diagnostic ledger. This is the single most important code verification for L1.

### 2.2 Transport layer (L2)

**`crates/storage/src/diagnostic_ledger/emitter.rs`**

- **Responsibility**: Non-blocking channel between hot-path emission sites and the cold-path writer task. `AsyncChannelEmitter` wraps a bounded `VecDeque` (ring buffer).
- **Public surface**:
  - Trait: `DiagnosticEmitter::record(&self, event: DiagnosticEvent) -> Result<(), StorageError>`
  - Factory: `AsyncChannelEmitter::new(capacity) -> (Arc<dyn DiagnosticEmitter>, Receiver)`
  - `NoOpEmitter` — implements trait, drops all events. Used when ledger is unavailable.
  - Health counters: `events_written_total`, `events_dropped_total`, `last_heartbeat_ms` exposed via `DiagnosticWriterStats`.
- **Hot-path**: YES (the `record()` call is on the gossip handling path).
- **Failure modes tolerated by design**:
  - Ring buffer full → oldest event evicted, `dropped_count` incremented (skill: lines 175-178). Health counter is the only signal.
  - Writer task dies/wedges → ring buffer fills, events accumulate then overflow. No watchdog documented.

### 2.3 Persistence layer (L2 continued)

**`crates/storage/src/diagnostic_ledger/mod.rs`**

- **Responsibility**: Writer task drains ring buffer and writes events to RocksDB `cf_events` column family. Also provides query methods for RPC consumers.
- **Public surface**:
  - `DiagnosticLedger::open(path)` — opens RocksDB at `<data_dir>/diagnostics/`.
  - `DiagnosticLedger::record(&self, event: &DiagnosticEvent) -> Result<(), StorageError>` — single-event write.
  - `DiagnosticLedger::query_recent(window_secs, limit) -> Vec<DiagnosticEvent>` — time-window scan.
  - `DiagnosticLedger::query_range(kind, min_height, max_height, limit) -> Vec<DiagnosticEvent>` — prefix scan.
  - `DiagnosticLedger::query_causal_chain(start_event_id, max_depth) -> Vec<DiagnosticEvent>` — follows `caused_by_event_id` links.
  - `DiagnosticLedger::prune(retention_secs, max_events)` — age + count pruning with pin protection.
- **Hot-path**: Writer task is async/background — NOT on gossip handling path.
- **Failure modes**:
  - RocksDB write failure → event lost. No retry.
  - Pruning misconfigured → unbounded growth or premature eviction. Production defaults NOT set in codebase (per LEDGER-SCHEMA: "caller-determined").

### 2.4 Classification layer (L3)

**`crates/storage/src/diagnostic_ledger/classifier.rs`**

- **Responsibility**: Pure function. 8 rules, first-match-wins. Given a slice of `DiagnosticEvent`, returns `Classification { fork_type, confidence, evidence_event_ids, recommended_action }`.
- **Public surface**: `classify(events: &[DiagnosticEvent]) -> Classification`
- **Hot-path**: NO. Called on-demand when `getForkDiagnostic` RPC is invoked. There is NO background/proactive classification.
- **Failure modes**: None (pure function, no I/O). Cannot fail, only produce wrong/low-confidence results.

**Rule priority order** (from LEDGER-SCHEMA, verified against skill):

| Priority | Rule | ForkType | Trigger | recommended_action |
|----------|------|----------|---------|-------------------|
| 1 | (a) | ProducerEquivocation | 2x BlockApplied same height+producer, diff hash | manual_intervention |
| 2 | (b) | EpochBoundaryInvalid | BlockRejected at epoch boundary with "EpochReward" | manual_intervention |
| 3 | (c) | RollbackLoop | >3 RollbackStarted in 60s | auto_recover |
| 4 | (d) | PostSnapDeadTip | SnapSyncCompleted then ForkBlockReceived within 300s | auto_recover |
| 5 | (h) | ChainBreakLoop | 4 signals in 1h window (chain_break>3 OR fork_recv>100+ratio>10 OR rollback>10 OR recovery_attempts>20) | restart_with_resync |
| 6 | (e) | TipRaceHighLatency | ForkBlockReceived + BlockApplied same height with validation_duration>2000ms | normal_operation |
| 7 | (f) | TipRaceNatural | ForkBlockReceived + low latency, no other signals | normal_operation |
| 8 | (g) | Unknown | Fallback | (none) |

**INC-I-090 classification analysis**: The incident shape is:
- N3 produced a block (BlockApplied for its own block)
- N3 received canonical block at same height (ForkBlockReceived with HeightOccupied)
- N3 got stuck for 9 minutes (recovery coordinator iterated 253 times)
- N3 recovered via snap-sync (SnapSyncCompleted)

If ALL events were emitted, rule (d) PostSnapDeadTip or (h) ChainBreakLoop should fire:
- (h) fires if `recovery_attempts > 20` in 1h window — 253 RecoveryClassifyCall events would satisfy signal_d. But ONLY if `RecoveryClassifyCall` events were actually emitted.
- (d) fires if SnapSyncCompleted followed by ForkBlockReceived within 300s.
- If neither (h) nor (d) fires (because recovery events were not emitted), the classifier would see only 1 ForkBlockReceived + 1 BlockApplied at the same height → rule (f) TipRaceNatural → `recommended_action: normal_operation`. This is the H5 pathway.

### 2.5 Surface layer (L4)

**`crates/rpc/src/methods/diagnostics.rs`**

- **Responsibility**: `getForkDiagnostic` RPC handler. Reads events from ledger, runs classifier, builds `DiagnosticBundle` (events + fork_summary + classification + baseline + health).
- **Public surface**: Registered at `dispatch.rs:74`.
- **Hot-path**: NO (on-demand RPC call).
- **CRITICAL**: Classification is LAZY — computed per RPC call, not proactively. No background task runs the classifier.

**`crates/rpc/src/methods/diagnostics_fleet.rs`**

- **Responsibility**: `getFleetForkDiagnostic` RPC handler. Fans out `getForkDiagnostic` to peer RPCs in parallel (max 50 peers, 5s per-peer timeout, 30s total).
- **Public surface**: Registered at `dispatch.rs:76`.

**`crates/rpc/src/methods/stats.rs`**

- **Responsibility**: `getStateRootDebug` and `getUtxoDiff` — cross-node state comparison tools.
- **Public surface**: Registered at `dispatch.rs:48-49`.

**`scripts/fork-monitor.sh`**

- **Responsibility**: Polls `getChainInfo` (NOT `getForkDiagnostic`) across nodes, groups by `bestHash`, reports divergence.
- **Hot-path**: Depends on deployment — could be cron, systemd timer, or manual.
- **CRITICAL**: This script polls `getChainInfo` which returns `{bestHeight, bestHash}`. It detects ACTIVE tip divergence only — a 1-block fork where the forked node is STUCK (same bestHash for 9 minutes while fleet advances) would show as a HEIGHT divergence, not a HASH divergence... unless polled at exactly the right moment.
- **Deployment status**: NOT documented as a systemd unit. The skill shows it as a manual command with `--loop` mode. Investigators must verify whether it runs as a service on mainnet.

**`scripts/health-check.sh`**

- **Responsibility**: 7-check operator health suite (RPC responding, peer count, genesis hash, height delta, peer-ID errors, service file flags).
- **Does NOT** consume diagnostic ledger data or run the classifier.

---

## 3. Data Flow Trace

### Scenario A: N3 applies its own block 8ede1526 at h=284677, then receives canonical block 150b4a7b at h=284677

**Step 1: N3 produces and applies block 8ede1526**

- Location: `bins/node/src/node/production.rs` → `apply_block()` in `bins/node/src/node/apply_block.rs`
- State mutation: UTXO set, chain state updated. `best_hash = 8ede1526`, `best_height = 284677`.
- Event SHOULD be emitted: `BlockApplied { slot: 291216, block_hash: "8ede1526", producer_pubkey: "54323cef" (N3), mode: "SelfProduced" }`
- Emission mechanism: `self.diagnostic_emitter.record(...)` — fire-and-forget (`let _ =`).
- **VERIFICATION NEEDED**: Does `apply_block.rs` emit `BlockApplied` for self-produced blocks? The skill says yes ("BlockProvenance passed as Option" per DEPENDENCIES line 86). Investigators must confirm.

**Step 2: N3 receives canonical block 150b4a7b via gossip**

- Location: `bins/node/src/node/event_loop.rs` → `handle_new_block()` in `block_handling.rs`
- Classification: `classify_gossip_block()` returns `BlockClass::ForkBlock(HeightOccupied)` because h=284677 is already occupied by N3's own block.
- State mutation: Block is NOT applied (N3 already has a block at this height). Stored as potential fork candidate.
- Event SHOULD be emitted: `ForkBlockReceived { block_hash: "150b4a7b", block_slot: 291215, block_height_estimate: 284677, producer_pubkey: "50fd1758", classification: "ForkBlock", fork_kind: "HeightOccupied", local_tip_hash: "8ede1526", local_tip_height: 284677 }`
- Emission mechanism: `self.diagnostic_emitter.record(...)` — fire-and-forget.
- CorrelationKey: `{ divergence_height: 284677, canonical_hash: None, fork_hash: "150b4a7b" }`
- **NOTE**: From N3's perspective, its OWN block is the local tip, and the canonical block is the "fork" block. The naming is node-local.

**Step 3: N3 detects it's on the minority fork**

- Location: `block_handling.rs` or `fork_recovery.rs` — FORK_GUARD detection.
- The canonical block 150b4a7b has slot 291215, N3's block has slot 291216. The canonical block has LOWER slot (better weight). N3 should detect it needs to reorg.
- Event SHOULD be emitted: Depends on whether FORK_GUARD emits. Skill does not explicitly document a FORK_GUARD emit site.
- **CRITICAL PATH**: This is where the fencepost at `recovery.rs:312` blocks the ShallowRollback. N3 WANTS to roll back from h=284677 to h=284676 and apply the canonical block, but `target_height <= finality` (should be `<`) prevents it.

**Step 4: N3 enters recovery loop (253 iterations over 9 minutes)**

- Location: `crates/network/src/sync/manager/recovery.rs`
- Each iteration: recovery coordinator classifies the situation and decides on action. The fencepost makes ShallowRollback unavailable, so it keeps trying and failing.
- Event SHOULD be emitted (per schema): `RecoveryClassifyCall { local_height: 284677, network_tip_height: 284678+, peer_count: N, last_applied_secs: increasing, shallow_rollback_count: 0, snap_attempts: 0→increasing, action_returned: "...", rule_matched: "..." }`
- **CRITICAL QUESTION**: Does recovery.rs actually call `DiagnosticEmitter::record(RecoveryClassifyCall)` on each iteration? If not, the 253 iterations are invisible to L2+L3+L4.
- **Investigators must verify**: The `RecoveryClassifyCall` EventKind exists (u8=7 in schema). The skill lists it. But the skill does NOT cite a specific file:line for the emit call in recovery.rs. The DEPENDENCIES section says `block_handling.rs` emits ForkBlockReceived and `apply_block.rs` emits BlockApplied — it does NOT mention recovery.rs emitting RecoveryClassifyCall.

**Step 5: N3 snap-syncs at ~23:04:30**

- Location: snap sync code path (invoked after recovery escalation).
- Events SHOULD be emitted:
  - `SnapSyncAttempted` at start
  - `SnapSyncCompleted` on success (or `SnapSyncFailed` on failure)
- After snap-sync, N3's chain state catches up to the fleet's canonical tip.

**Step 6: Events reach RocksDB (L2)**

- Each emitted event enters the `AsyncChannelEmitter` ring buffer.
- Writer task drains buffer, serializes (bincode), writes to `cf_events` with key `[kind_u8][height_u64_BE][ulid_16B]`.
- Under normal load (1 block/10s + occasional fork events), ring buffer should NOT overflow.
- Health counters updated: `events_written_total++` per event written, `events_dropped_total` incremented on overflow.

**Step 7: RPC visibility (L4)**

- An operator calls `getForkDiagnostic` with `min_height=284670, max_height=284685`.
- Handler reads events from RocksDB for that range.
- Classifier runs on the event slice — first-match-wins.
- Result packaged into `DiagnosticBundle` with health counters.
- **BUT**: Nobody calls this automatically. It's strictly on-demand.

### Scenario B: N3 produces 8ede1526 AFTER hearing 150b4a7b as parent candidate

This is the inverse scenario. Per the analyst's evidence (finding 822/827): N3 missed the canonical block due to a 26-second gossip delay. N3 was eligible for slot 291216 and produced at 22:54:37.082. The canonical block for slot 291215 arrived 55ms LATER.

So the actual sequence was:
1. N3 produces 8ede1526 at h=284677 (BlockApplied event — self-produced)
2. 55ms later, N3 receives 150b4a7b at h=284677 (ForkBlockReceived with HeightOccupied)

N3 was NEVER in HeightOccupied from canon's perspective — from N3's perspective, IT occupied the height first, and the canonical block arrived as the "fork." The canonical block is structurally the fork block from N3's local view.

This means:
- N3 did NOT produce "on a stale view" — it produced legitimately on its own slot, not having seen the canonical block yet.
- The `ForkBlockReceived` event (if emitted) would have `local_tip_hash: "8ede1526"` (N3's own block) and the incoming block 150b4a7b as the fork.
- The canonical block has a LOWER slot (291215 < 291216), which means better weight. N3 should detect this and attempt to reorg.

### Scenario C: Recovery path (9 minutes of iteration)

Per evidence (finding 821): `recently_synced()` threshold (60s) suppressed shallow rollback initially. Then the FINALITY_GUARD fencepost (recovery.rs:312) blocked all subsequent ShallowRollback attempts.

The recovery coordinator iterated 253 times over 9 minutes. Each iteration:
1. Checks if ShallowRollback is appropriate → blocked by fencepost
2. Checks if SnapSync is appropriate → eventually escalates
3. Logs sync_fails counter (visible in node logs as FORK_GUARD entries)

**DiagnosticEmitter calls in the recovery loop**: UNKNOWN. This is the single most critical verification point. If `RecoveryClassifyCall` events are NOT emitted in the recovery coordinator loop, then:
- The classifier never sees `recovery_attempts > 20` → rule (h) ChainBreakLoop signal_d never fires
- The diagnostic ledger has a 9-minute blind spot containing only the initial ForkBlockReceived
- The classifier would see a single ForkBlockReceived → rule (f) TipRaceNatural → `normal_operation`

---

## 4. Trust Boundaries and Seams

### Seam 1: Gossip network → handle_new_block()
- **Boundary**: External network data enters the node.
- **Silent failure**: Block arrives but is processed by a code path that doesn't emit. E.g., if the block is rejected before reaching the classification/emit logic.

### Seam 2: handle_new_block() → DiagnosticEmitter::record()
- **Boundary**: Consensus-adjacent hot-path code calls observability infrastructure.
- **Silent failure**: `let _ =` discards the Result. If `record()` returns Err, no signal reaches any downstream component. **This is by design but creates a structural blind spot.**

### Seam 3: Recovery coordinator → DiagnosticEmitter::record()
- **Boundary**: Sync recovery cold-path calls observability infrastructure.
- **Silent failure**: If recovery.rs does not call `record()` at all, no events are generated for recovery iterations. The `RecoveryClassifyCall` EventKind would exist in the schema but never be populated. **Investigators must verify this seam exists in code.**

### Seam 4: DiagnosticEmitter ring buffer → Writer task
- **Boundary**: In-process async channel.
- **Silent failure**: Writer task wedged/slow → ring buffer fills → oldest events evicted. The ONLY signal is `events_dropped_total` counter, which is only visible via the `health` field in `getForkDiagnostic` response. If nobody calls that RPC, the overflow is invisible.

### Seam 5: Writer task → RocksDB
- **Boundary**: In-process → on-disk persistence.
- **Silent failure**: RocksDB write error → event lost. No retry, no dead-letter queue.

### Seam 6: RocksDB → RPC handler (query path)
- **Boundary**: On-disk → RPC response.
- **Silent failure**: Query window doesn't cover the incident (wrong `window_secs` or height range). Pruning removed events before query. No failure per se, just user error.

### Seam 7: RPC response → External consumer
- **Boundary**: JSON response → operator/script/dashboard.
- **Silent failure**: **No automated consumer documented.** The `recommended_action` field is computed, serialized into JSON, and... returned to whoever called the RPC. If nobody calls, the field is never read. `fork-monitor.sh` calls `getChainInfo`, NOT `getForkDiagnostic` — it never sees the classifier output.

### Seam 8: fork-monitor.sh → operator
- **Boundary**: Script output → human (or systemd journal, or syslog).
- **Silent failure**: Script not deployed as a service → never runs. Script runs but at too-long interval → misses transient divergence. Script runs but polls `getChainInfo` (tip only) → a stuck node shows as height-lagging, not hash-diverging, which may not trigger FORK exit code.

---

## 5. Implicit Assumptions

1. **"Writer task drains faster than emitter pushes."** — Under steady-state (1 block/10s), this holds. Under stress (fork cascade with rapid block arrivals), it may not. No backpressure signal exists beyond the dropped counter.

2. **"Operator runs fork-monitor.sh periodically."** — No systemd unit or cron job documented. The script has `--loop` mode but no deployment automation. The skill documents it as a manual command.

3. **"`recommended_action` is consumed by SOMEONE."** — The skill's OPERATIONS table row 50 maps recommended_action to guardian procedures. But no code path automatically reads the recommended_action field and triggers recovery. It's aspirational documentation. The RPC returns it; nobody acts on it automatically.

4. **"RecoveryClassifyCall events are emitted during recovery iterations."** — The EventKind exists. The classifier's rule (h) ChainBreakLoop has signal_d (`recovery_attempts > 20`). But the actual emit call in recovery.rs is UNVERIFIED. If the emit call doesn't exist, the entire ChainBreakLoop detection for recovery-stuck scenarios is structurally broken.

5. **"Ledger pruning is configured by the caller."** — Per LEDGER-SCHEMA: "Production default retention and max_events: NOT set in this codebase (caller-determined)." If the writer task doesn't call `prune()`, events accumulate unboundedly. If it prunes too aggressively, incident evidence is lost. The pruning policy is an operational gap.

6. **"A 1-block fork is distinguishable from a natural tip race."** — The classifier's rules (e)/(f) both produce `normal_operation`. A 1-block fork where N3 gets STUCK is distinguishable ONLY if `RecoveryClassifyCall` events push the event count high enough to trigger rule (h). Without those events, the incident is indistinguishable from benign slot contention.

7. **"The fleet aggregator compensates for per-node blindness."** — `getFleetForkDiagnostic` polls each node's individual ledger. If node N3 didn't emit events, the fleet view has no N3 data. The aggregator cannot fabricate evidence.

8. **"Height divergence in getChainInfo implies fork detection."** — A stuck node falls behind in height. `fork-monitor.sh` groups by `bestHash`. If only one node has a different bestHash (N3's fork block), it's detected. But if the rest of the fleet has moved to h=284690+ while N3 is stuck at h=284677, the hash comparison may show N3 as a laggard (different height group) rather than a fork. The script's logic needs verification.

9. **"The diagnostic subsystem is compiled into the mainnet binary."** — The feature could be behind a cargo feature flag, or the mainnet binary could predate the feature merge. H7 hypothesis. Investigators must verify binary artifact.

10. **"NoOpEmitter is only used when ledger is unavailable."** — If a code path constructs the node with `NoOpEmitter` (e.g., due to a RocksDB open failure at startup), ALL diagnostic events for the entire session are silently dropped. No log warning documented for this fallback.

---

## 6. Load-Bearing Invariants

These invariants MUST be preserved by any fix to the observability subsystem:

| Invariant | Description | Location (per skill) | Consequence of violation |
|-----------|-------------|---------------------|------------------------|
| INV-1 | Emission is fire-and-forget on hot path | `block_handling.rs:168-190` (`let _ =`) | Blocking the emitter would slow gossip processing, potentially causing missed slots |
| INV-2 | Classifier is a pure function (no I/O, no system clock) | `classifier.rs:1-3` per skill; time anchored to `max(event.timestamp_ms)` | Making classifier impure would break testability and introduce non-determinism |
| INV-3 | Ledger is per-node (no gossip of diagnostic events) | `diagnostic_ledger/mod.rs` header | Gossiping diagnostic events would add consensus-visible network traffic and potential amplification |
| INV-4 | RPC methods are strictly read-only | `diagnostics.rs` (per INV-OBS-001) | Write side-effects from RPC would violate trust boundary |
| INV-5 | Ring buffer drops oldest on overflow (not newest) | `emitter.rs:175-178` per skill | Dropping newest would lose the most recent (most relevant) events during an incident |
| INV-6 | Prune preserves cascade-origin pins (first event per CorrelationKey) | `mod.rs:136-238` per skill | Losing cascade origins breaks causal chain traversal |
| INV-7 | Schema version check uses `>` not `!=` (forward-compatible) | `types.rs:449-452` per skill | Breaking forward compatibility would require coordinated fleet-wide upgrades for schema changes |
| INV-8 | Fleet peer RPCs are redacted (no raw IPs in serialized output) | `fleet.rs:140-142` per skill | IP leakage in RPC responses is a security violation |
| INV-9 | `getChainInfo` is the heartbeat poll target (lightweight, always available) | `dispatch.rs:28` | Replacing it with `getForkDiagnostic` for polling would add RocksDB reads per poll cycle |

---

## 7. Architectural Smells

These are structural properties of the design that are SUSPECT for the symptom "user noticed visually, no alert fired." They are observations, not diagnoses — investigators must determine which (if any) are causal.

### Smell 1: No automated consumer of recommended_action
The classifier produces `recommended_action` (a string like `"manual_intervention"`, `"auto_recover"`, `"restart_with_resync"`, `"normal_operation"`). The skill's OPERATIONS table (row 50) maps these to guardian procedures. But **no code path in the codebase automatically reads the recommended_action field and triggers any action**. It sits in the `Classification` struct, gets serialized into the `getForkDiagnostic` JSON response, and is returned to... whoever called the RPC. If nobody called the RPC during the 9-minute incident, the action was never read. The classification system is a consulting service with no clients.

### Smell 2: fork-monitor.sh polls getChainInfo, not getForkDiagnostic
The script that operators run for fork detection (`fork-monitor.sh`) polls `getChainInfo` which returns `{bestHeight, bestHash}`. It groups by `bestHash` and reports divergence. But:
- It never calls `getForkDiagnostic` (the method that runs the classifier and returns `recommended_action`).
- It detects only ACTIVE tip divergence visible in the chain tip at poll time.
- A 1-block fork on one node that results in that node being STUCK manifests as a height-lagging node, not necessarily a hash-diverging one (if the fleet has moved past h=284677 by the time the script polls).
- The script's deployment status on mainnet is unknown.

### Smell 3: Classification is lazy (on-demand only, no proactive detection)
The classifier runs ONLY when `getForkDiagnostic` is called via RPC. There is no background task that periodically runs the classifier against recent events and fires an alert. This means:
- Events can be written to the ledger and classified as `ChainBreakLoop` with `recommended_action: "restart_with_resync"` — but nobody will know unless they call the RPC.
- The observability system is a passive data store, not an active monitoring system.

### Smell 4: Recovery coordinator emission gap (unverified but structurally critical)
The `RecoveryClassifyCall` EventKind exists in the schema. The classifier's rule (h) `ChainBreakLoop` has signal_d (`recovery_attempts > 20`). But the skill does NOT document a specific emit call in `recovery.rs`. If the recovery coordinator doesn't emit, then:
- 253 recovery iterations are invisible
- The classifier sees 1 ForkBlockReceived → TipRaceNatural → normal_operation
- The event that would DISTINGUISH "benign tip race" from "stuck for 9 minutes" is structurally absent

### Smell 5: Fire-and-forget emission means missing events are invisible
The `let _ =` pattern means emission failure (emitter error, NoOpEmitter, ring overflow) produces no signal visible to the node operator. The only canary is `events_dropped_total` in the health block — which is only visible via the same RPC that nobody calls automatically (Smell 3). A missing event is indistinguishable from "nothing happened."

### Smell 6: No pruning configuration documented for production
The LEDGER-SCHEMA states that retention and max_events are "caller-determined" with no production defaults. If pruning runs too aggressively, incident evidence evaporates before investigation. If it doesn't run, disk grows unboundedly. Either failure mode degrades the system silently.

### Smell 7: health-check.sh doesn't consume diagnostic ledger
The operator-facing health check script runs 7 checks (RPC, peers, genesis, height, etc.) but does NOT query the diagnostic ledger or run the classifier. A node could have a ledger full of `ChainBreakLoop` events with `restart_with_resync` recommendations, and `health-check.sh` would report "OK."

---

## 8. Architecture Feasibility Verdict

```
━━━ ARCHITECTURE FEASIBILITY VERDICT ━━━
Verdict:          CODE-FIXABLE
Confidence:       conf(0.80, inferred)
Reasoning:        The observability-fork subsystem has the right structural components — emit sites, ring buffer, persistence, classifier with 8 rules, RPC surface, operator scripts. The architectural gap is NOT a missing abstraction but a missing WIRING: (1) likely missing emit calls in recovery.rs that would provide the signal to distinguish stuck-for-9-minutes from benign-tip-race, (2) no automated consumer of the classifier's recommended_action output, and (3) fork-monitor.sh polling getChainInfo instead of getForkDiagnostic. All three are localized fixes — add emit calls to existing code paths, add a periodic classifier invocation (a function call in an existing periodic task, not a new subsystem), and wire fork-monitor.sh to call getForkDiagnostic when height divergence is detected. The overall 4-layer architecture (emit → persist → classify → surface) is sound; the problem is that the pipeline has gaps at L1 (missing emit in recovery path) and L4 (no automated surface consumer). These are code-level fixes, not architectural redesign.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## Verification Delegation

The following claims in this blueprint are marked `basis=observed-via-skill` and MUST be verified by investigators against the actual source code:

1. `block_handling.rs:154-418` emits `ForkBlockReceived` for `HeightOccupied` — verify emit call exists at cited lines
2. `apply_block.rs` emits `BlockApplied` with `BlockProvenance` — verify emit call
3. `recovery.rs` emits `RecoveryClassifyCall` — **HIGHEST PRIORITY verification**: if this emit does NOT exist, it is the primary L1 gap
4. `fork_recovery.rs` emits `SnapSyncAttempted/Completed/Failed` — verify emit calls
5. `emitter.rs:175-178` implements drop-oldest on overflow — verify ring buffer semantics
6. `classifier.rs:36-60` implements the 8 rules in the documented priority order — verify
7. `fork-monitor.sh` polls only `getChainInfo` and never `getForkDiagnostic` — verify
8. No code path in the codebase automatically acts on `recommended_action` — verify via grep
9. Mainnet binary on ai1 contains the diagnostic ledger subsystem — verify (H7)
10. `fork-monitor.sh` is/is not deployed as a systemd service on mainnet — verify (H4)
