# Architecture Perspective: Analogist — Workflow #346

## Perspective
Analogist — match this design to known architectures and known failure patterns in similar systems.

## What I Don't Understand
1. Whether the 50us emit latency budget was benchmarked against actual `apply_block` hot path duration (is it 1ms? 100ms? Budget meaningless without denominator).
2. Whether DOLI nodes are typically I/O-bound or CPU-bound during block application.
3. How many fork events per hour actually occur during normal operation vs incident cascade. Retention policy (100k events) is meaningless without this cardinality baseline.
4. Whether the `content_store.rs` separate-DB pattern was added before or after snap-sync wipe coupling was understood.

## Analysis

### Analogy 1: Bitcoin Core debug.log + diagnostic RPCs

**What's the same**: Bitcoin emits structured log events from validation, tracks chain tips via `getchaintips` RPC (returns fork info: status, height, hash), maintains append-only debug.log. DOLI's `block_applied`, `block_rejected`, `fork_block_received` map directly to Bitcoin's `UpdateTip`, `InvalidChainFound`, fork tracking.

**What's different**: Bitcoin has NO indexed event store. `getchaintips` returns current state, not history. Cannot query "fork events between height X and Y" — you grep debug.log. DOLI's proposal is strictly better.

**Lessons (good)**: Bitcoin's `getchaintips` returns a typed `status` field (`active`, `valid-fork`, `valid-headers`, `headers-only`, `invalid`) — precedent for `ForkType` enum. Bitcoin learned that a SMALL set of typed states is far more useful than free-text. 9-variant enum is right size. conf(0.6, observed).

**Lessons (bad)**: Bitcoin's debug.log has no retention and grows unbounded (10-20GB on long-running nodes). The 30-day/100k cap correctly avoids this. Bitcoin has no causal linking — `caused_by_event_id` is a genuine improvement. conf(0.55, inferred).

### Analogy 2: Ethereum debug_trace* RPCs

**What's the same**: Geth's `debug_traceBlock` returns structured traces — analogous to `DiagnosticBundle` for a height range. Both are read-only.

**What's different**: Ethereum traces are computed on-demand (re-executing the block), not pre-recorded. DOLI pre-records into RocksDB. Fundamental tradeoff: pre-recording adds emit latency but data survives state corruption.

**Lessons (good)**: Geth caps trace output via `limit` and `timeout` (default 5s). The `limit` cap at 10,000 in REQ-FORKOBS-SEC-003 follows correctly. Expensive queries must not block the main execution thread → async tracing. The 50us gate + async fallback mirrors this. conf(0.5, inferred).

**Lessons (bad)**: Geth's tracing schema changed across versions with no backward compatibility, breaking tooling. The `schema_version` + additive-only policy is the correct response. However: **bincode is NOT a stable format across Rust compiler versions.** Known footgun. Consider adding a format marker byte (0x01=bincode, 0x02=future) before schema_version to allow format migration. conf(0.6, observed).

### Analogy 3: Linux kernel ftrace / eBPF

**What's the same**: Kernel has IDENTICAL constraint: instrumentation in hot path (syscall execution / block application) must have near-zero overhead, must not affect correctness, must use bounded storage. ftrace uses per-CPU ring buffers with fixed size — events overwritten when buffer full.

**What's different**: ftrace ring buffers are lock-free and in-memory. DOLI writes to RocksDB (disk I/O). **Most significant divergence and most concerning.**

**Lessons (good)**: Tracepoint model — compile-time instrumentation NOP when disabled, active when consumer attached — maps to `let _ = ledger.record(...)`. Key insight: **fire-and-forget semantics**, emitter NEVER checks return value in hot path. REQ-FORKOBS-PERF-002 is kernel-derived and correct. conf(0.65, observed).

**Lessons (bad)**: Kernel learned that **synchronous write in hot path is death.** ftrace moved to lock-free ring buffers specifically because any form of mutex or I/O wait in the trace path caused latency spikes. If `DiagnosticLedger::record()` does sync RocksDB put, 50us budget will be violated under write amplification or compaction storms. **Async mpsc should be DEFAULT, not fallback.** Start async, prove sync is safe — not the other way around. conf(0.6, inferred).

### Analogy 4: Distributed tracing (OpenTelemetry)

**What's the same**: OTel's span model — trace_id, span_id, parent_span_id, start/end timestamps, typed attributes — maps directly to DiagnosticEvent's event_id, caused_by_event_id, timestamp_ms, kind-specific fields. The `correlation_key` tuple is a domain-specific trace_id.

**What's different**: OTel is designed for cross-process tracing with sampling. DOLI records every event (no sampling) within single process. Simpler, avoids cardinality explosion at scale.

**Lessons (good)**: OTel's W3C TraceContext proved structured correlation ID propagated through causal chains is worth more than all other fields combined. Populate `correlation_key` on fork-related events, leave None on canonical events — right discipline. conf(0.55, inferred).

**Lessons (bad)**: OTel suffered "attribute explosion" — projects added unbounded string attributes, choking storage. DiagnosticEvent uses typed enum variants with fixed fields — avoids this. **Maintain discipline — do NOT add a `HashMap<String, Value>` "extra" field.** `Unknown.reason_unknown: String` is the ONLY free-text field; keep it that way. conf(0.5, inferred).

### Analogy 5: Datadog/Honeycomb event observability

**What's the same**: Both store every event (wide events) and query later. DOLI stores every fork-relevant event, queries via RPC.

**What's different**: Honeycomb supports arbitrary queries over high-cardinality fields. DOLI returns pre-structured bundle with fixed schema. Intentionally less flexible but vastly simpler.

**Lessons**: Honeycomb's main lesson: "you don't know what you'll query until you need to." Rigid struct approach works IF field set is comprehensive. Risk: incident N+1 needs a field not captured. `Unknown` variant with `evidence_event_ids` is safety valve — but only works if events capture enough raw data. 12-field RecoveryContext capture (REQ-FORKOBS-EMIT-007) is learning from this. conf(0.45, assumed).

## Past DOLI Mirrors

### [HEALTH] and [FORK_GUARD] — DOLI's existing structured logs

`[HEALTH]` at `periodic.rs:898`: height, slot, hash, peer_count, best_peer_height, sync_fails, sync_state, epoch info — 13 fields. `[FORK_GUARD]` at `block_handling.rs:184,191`: slot comparison, height, hash prefix. **DOLI's first-generation observability.**

**Why they failed**: (1) Free-text format requires regex parsing — `[HEALTH]` crams 13 fields into one `warn!()` with no structured separator. (2) No causal links. (3) No persistence beyond log file. (4) No query interface — must grep 1GB log files.

**What they got right**: (1) Instrument the CORRECT decision points — `classify_gossip_block` for fork detection, `periodic.rs` for health. (2) Fire AFTER the decision. (3) Include comparison context (local slot vs fork slot). **New emitter should emit at exactly these same points plus the additional ones in the spec.** conf(0.65, observed).

### Guardian system — fleet RPC precedent

`guardian.rs` already implements: production pause/resume via RPC, RocksDB checkpoint creation, health status query. `getGuardianStatus` returns structured JSON. **Exact pattern for `getForkDiagnostic`** — read-only RPC assembling structured response from node state. Phase 2 `getFleetForkDiagnostic` would be architecturally identical to `fork-monitor.sh` polling, but structured. conf(0.6, observed) — verified guardian.rs lines 161-199.

### INC-I-082 rebuild safety — instrumentation lessons

INC-I-082: bit-level rebuild defect. `rebuild_epoch_state_from_blocks` produced different results than `post_commit`. Fix: explicit `target_height` for bit-identity. **Lesson: any new code path reading consensus state (even read-only) can expose subtle differences between code paths.** Diagnostic emitter reads `chain_state`, `epoch_state`, `sync_manager` — all consensus state. If read not properly synchronized (reading at different points in apply cycle), diagnostic event could contain internally inconsistent data. Not a correctness risk (diagnostics are local-only) but could mislead the classifier. conf(0.5, inferred).

## DOLI Mistakes This Design Could Repeat

### INC-I-054 (schema version) analogue
INC-I-054: `CURRENT_PROTOCOL_VERSION` bump triggered `delete_epoch_state()` on restart → chain splits. Analogue: if `schema_version` in DiagnosticEvent were ever checked with `!=` (reject events from different versions), version bump silently discards all historical diagnostic data. **Risk: LOW.** Diagnostic schema_version is local-only, no consensus impact. Additive-only policy mitigates. **But: header byte prefix MUST use `>=` (accept current and older), never `==`.** conf(0.6, inferred).

### INC-I-062 (mixed-fleet classifier) analogue
INC-I-062: block content change without synchronized deploy → competing valid blocks. Analogue: if two nodes run different classifier versions, they produce different `Classification` results for the same event sequence. **Risk: NONE for Phase 1.** Classifier runs locally; classification is not part of consensus or block content. Two nodes disagreeing on classification is expected and harmless. Explicitly safe for rolling deploy per REQ-FORKOBS-SEC-006. conf(0.65, observed).

### INC-I-009 (cardinality explosion) analogue
INC-I-009: max_peers=200 → 86GB RAM. Analogue: unbounded event recording. Worst case during cascade (INC-I-081-style): every node emits hundreds of events/second (block_rejected, rollback_started, rollback_completed, recovery_classify_call, snap_sync_attempted, snap_sync_failed). With 18 nodes × 10 events/sec/node = 180 events/sec, 648k events/hour. **100k cap hit in under 10 minutes, meaning the OLDEST events (the cascade's origin — the most diagnostic) are pruned FIRST.**

**This is the most dangerous analogy.** Retention prunes oldest first, but during cascade, the oldest events are the MOST diagnostic (they show the trigger). **Consider per-kind cap or a "pin" mechanism for first N events in a correlation group.** conf(0.6, inferred).

## Borrow Recommendations

- **From Kafka**: Append-only, retention-by-time-or-size. 30-day + 100k dual cap is correct Kafka thinking. Also borrow log compaction concept: when pruning, keep LAST event per correlation_key so every known fork retains at least final state. conf(0.55, inferred).
- **From Prometheus**: Cardinality discipline. Bounded label cardinality. Apply to event fields: `producer_pubkey` bounded (max ~1000 producers), `event_kind` bounded (enum), `fork_type` bounded (enum). Only unbounded is `reason_unknown`. Keep that way. conf(0.6, observed).
- **From journald**: Binary structured records with backward-compatible field addition. `_FIELD=value` format allows adding new fields without breaking old readers. `#[serde(default)]` achieves the same in bincode. Borrow journald's `__CURSOR` concept — monotonic position identifier (ULID serves this). conf(0.5, inferred).
- **From systemd-journal-remote**: Cross-node correlation by `_MACHINE_ID` + `__SEQNUM`. Map to `node_peer_id` + `event_id(ULID)`. The `correlation_key` tuple adds domain-specific correlation on top. Right layering. conf(0.5, inferred).

## Architecture Recommendations Distilled From the Analogies

1. **Start with async channel, not sync RocksDB write.** Kernel tracing analogy is strongest evidence: sync I/O in hot path always eventually causes latency spikes. Use mpsc as DEFAULT emit path, background writer thread. 50us gate validates channel send, not DB write. conf(0.6, inferred).

2. **Protect cascade-origin events from pruning.** INC-I-009 cardinality analogy: 100k cap hit during exactly the incidents where diagnostics matter most. When pruning, retain FIRST event per unique `correlation_key` to preserve cascade origins. Adds ~100 LoC but prevents "evidence self-destructs" failure mode. conf(0.55, inferred).

3. **Bincode is a ticking migration bomb.** Ethereum schema instability + journald field-addition model both point same direction: use a format with explicit field names (CBOR, MessagePack, or even JSON) for long-term stored events, not bincode (no self-describing schema). If bincode chosen for performance, add format-marker byte before `schema_version` for future migration. conf(0.5, assumed).

4. **Do NOT add a `HashMap<String, Value>` extra field.** OTel attribute-explosion anti-pattern is strongest signal. Typed enum variants with fixed fields are correct. Every future incident needing a new field gets a new enum variant or new field with `#[serde(default)]` — never a schemaless bag. conf(0.65, observed).

5. **Separate-RocksDB-instance decision is correct and well-precedented.** DOLI already opens 4 separate RocksDB instances (block_store, state_db, utxo_rocks, content_store). A 5th for diagnostics follows established pattern and avoids snap-sync wipe coupling. conf(0.65, measured).

## Key Evidence
- `state_db/open.rs:36` — 6 CFs, `create_missing_column_families(true)`, Lz4. Established pattern.
- `state_db/writes.rs:164-169` — `deletable_cfs` list that snap-sync wipes: UTXO, UTXO_BY_PUBKEY, PRODUCERS, EXIT_HISTORY. A diagnostic CF here WOULD be wiped.
- `block_store/open.rs:40` — 9 CFs, separate DB. Precedent.
- `content_store.rs:36` — Another separate DB. 5th DB is established pattern.
- `periodic.rs:898` — `[HEALTH]` log: 13 fields in one `warn!()` line. The failure mode this design replaces.
- `block_handling.rs:184,191` — `[FORK_GUARD]` log: free-text fork detection at correct decision point.
- Zero hits for `compaction_filter`, `TTL`, or `CompactionFilter` — retention pruner is novel infrastructure.

## Cross-Perspective Signals
- **For Explorer**: Separate-RocksDB pattern is well-established (4 existing instances). "Do nothing" alternative is weak — `[HEALTH]` and `[FORK_GUARD]` are already the "do nothing" answer, demonstrably failed across 30+ incidents.
- **For Skeptic**: Cascade-origin pruning problem is a potential fatal flaw in retention policy. During INC-I-081-style cascades, 100k cap could delete trigger events within minutes. Async-vs-sync emit decision is highest-risk performance question.
- **Bincode choice deserves scrutiny.** No other persistent store in this codebase uses bincode for on-disk format — block bodies use custom `deserialize_body`, state_db uses raw bytes with known layouts. Bincode would be novel; version stability across serde/bincode crate updates not guaranteed.

## Gaps
1. Could not verify actual `apply_block` execution duration to assess whether 50us is meaningful (1% of path? 50%?).
2. Did not examine `content_store.rs` open pattern in detail to confirm graceful degradation behavior — closest precedent for REQ-FORKOBS-LEDGER-009.
3. Did not verify whether `ulid` is already a transitive dependency.
4. Real-world fork event rate during cascades unknown. Estimate of 10 events/sec/node during cascade extrapolated from incident descriptions, not measured.
