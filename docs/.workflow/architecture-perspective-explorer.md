# Explorer Perspective -- Workflow #346

## Perspective
Explorer -- generate alternatives ordered by simplicity, subtraction first, addition last.

## What I Don't Understand
1. Whether bincode is already a dependency of the storage crate or only serde_json -- this affects serialization choice for the ledger.
2. The exact latency profile of RocksDB `put()` on this codebase's target (macOS, NVMe?) -- the 50us gate in REQ-FORKOBS-PERF-001 is the pivotal design fork between sync and async.
3. Whether `periodic.rs` at 1,793 lines will be refactored independently or only as part of this work -- determines pruner placement strategy.
4. How `RpcContext` is threaded to the Node -- it is built in `startup.rs:337` and passed `Arc` references. Adding a new `Arc<DiagnosticLedger>` field is mechanically easy but must be wired through the builder.

## Analysis

### Realization A: Inline Emitter, Central Ledger

**Architecture.** Every emit site calls `self.diagnostic_ledger.record(event)` directly. The `DiagnosticLedger` is an `Arc<DiagnosticLedger>` field on `Node` (alongside `block_store`, `state_db`). It opens a separate RocksDB at `<data_dir>/diagnostics/`. Writes are synchronous (`db.put_cf()`). If the 50us benchmark fails, a `parking_lot::Mutex<VecDeque>` buffer is drained by a timer in periodic.rs.

**Module boundaries.**
- Types (`DiagnosticEvent`, `ForkType`, `Classification`, `DiagnosticBundle`): new module `crates/storage/src/diagnostic_ledger/types.rs`.
- Ledger (`record`, `query_range`, `query_recent`, `prune`): `crates/storage/src/diagnostic_ledger/mod.rs` + `open.rs` + `queries.rs`.
- Classifier (pure function): `crates/storage/src/diagnostic_ledger/classifier.rs` (or `crates/core/src/fork_observability/classifier.rs` if core-crate purity is preferred -- but core has no RocksDB dep, so types-only in core, impl in storage).
- RPC: `crates/rpc/src/methods/diagnostics.rs` + dispatch entry.
- CLI: `bins/cli/src/commands/forks.rs`.
- Emitter call sites: 5-7 `let _ = self.diagnostic_ledger.record(...)` calls in `block_handling.rs`, `apply_block/mod.rs`, `apply_block/post_commit.rs`, `fork_recovery.rs`, `rollback.rs`, `periodic.rs`.

**How from_peer_id is threaded.** The requirements propose `last_block_source: Option<(Hash, PeerId, u64)>` on Node. Alternative: pass a `BlockProvenance { from_peer: Option<PeerId>, received_at: Instant }` struct into `handle_new_block`, then thread it through to the emitter via a scoped local. Since `handle_new_block` already receives `source_peer: PeerId` (line 116 of `block_handling.rs`), the provenance is available WITHOUT a side-channel. The emitter call in `block_handling.rs` can capture `source_peer` directly; the only gap is inside `apply_block` which is called from `handle_new_block` without the peer info. The side-channel (`last_block_source` on `&mut self`) is the lowest-touch solution -- one field set, one field read, cleared after apply. conf(0.6, observed)

**Classifier home.** Storage crate. The classifier is a pure function over `&[DiagnosticEvent]`, which are storage types. Placing it in `crates/core` would require core to depend on storage types (wrong direction). Placing it in a new `crates/fork_observability` crate is an option but adds a crate for ~200 lines of match logic -- over-engineered for Phase 1. conf(0.55, inferred)

**Pruner integration.** `periodic.rs` is already 1,793 lines (observed). Adding a pruner inline would push it further past the 500-line module limit. Extract to `bins/node/src/node/periodic/pruner.rs` submodule. The pruner runs every 60s, calls `diagnostic_ledger.prune(retention_days, max_events)`. ~30 lines. conf(0.65, observed)

**Retroactive validation.** Straightforward. Schema captures all fields needed for INC-I-081 (`block_rejected` with `rejection_reason` containing "missing EpochReward" at epoch boundary) and INC-I-083 (`recovery_classify_call` with full RecoveryContext showing snap_attempts exhausted + `fork_block_received` events showing dead tip). The inline model means every call site is visible in the code -- easy to audit completeness. conf(0.6, inferred)

**Pros**: Simplest mental model -- grep for `diagnostic_ledger.record` and you see all emit sites. No new abstractions. Follows existing `ws_sender` pattern on Node. Minimal new dependencies (ulid crate only).

**Cons**: Emit calls are scattered across 6+ files -- blast radius of schema changes. Sync writes couple hot-path latency to disk I/O. If RocksDB `put()` exceeds 50us, the fallback buffer adds complexity equivalent to Realization B anyway.

**Risks**: Lock contention if `DiagnosticLedger` uses internal `Mutex` for buffered mode. The 50us gate is the critical unknown -- if it fails, this realization degrades into a worse version of B. conf(0.55, inferred)

---

### Realization B: Event Bus + Actor

**Architecture.** Emit sites push a `DiagnosticEvent` into a `tokio::mpsc::Sender<DiagnosticEvent>` (bounded, capacity 1024). A dedicated `diagnostic_writer` tokio task drains the channel and writes to the DiagnosticLedger. The hot path pays only the cost of `mpsc::send()` (~50-100ns), not RocksDB `put()`. The writer task batches writes (e.g., 10 events or 100ms, whichever first) into a single `WriteBatch` for throughput.

**Module boundaries.** Same as A for types, ledger, classifier, RPC, CLI. The difference is the writer task lives in `bins/node/src/node/diagnostic_writer.rs` (~80 lines) and is spawned in `startup.rs` alongside the RPC server. The `mpsc::Sender` replaces the `Arc<DiagnosticLedger>` on `Node`; the `DiagnosticLedger` is private to the writer task.

**How from_peer_id is threaded.** Same as A -- the `last_block_source` side-channel on Node is the pragmatic choice. The event is constructed at the call site with peer info already available, then sent into the channel. No change vs. A. conf(0.6, observed)

**Classifier home.** Same as A -- storage crate. The RPC handler calls the classifier with events queried from the ledger. The writer task does not classify -- it only persists. conf(0.55, inferred)

**Pruner integration.** The writer task itself can run pruning every N writes or on a timer. This is cleaner than periodic.rs because the writer owns the `DiagnosticLedger` exclusively -- no shared access, no lock. Alternatively, the periodic.rs pruner sends a `PruneCommand` message on a second channel. conf(0.6, inferred)

**Retroactive validation.** Same schema adequacy as A. The async gap means events could be lost if the node crashes before the writer flushes. For diagnostic (non-consensus) data, this is acceptable -- the requirement says "graceful degradation." conf(0.6, inferred)

**Pros**: Guaranteed <1us hot-path overhead (mpsc send). No latency coupling between consensus and diagnostics. Writer task owns the DB exclusively -- no lock contention possible. Batched writes are more efficient for RocksDB.

**Cons**: Event ordering has a small async gap (events queued but not yet written are invisible to RPC queries). Adds a tokio task + channel -- more moving parts. Shutdown ordering: must drain channel before closing DB. Channel overflow policy needed (drop oldest? block? -- REQ-FORKOBS-SEC-004 says emit failures must not propagate, so drop).

**Risks**: Channel backpressure under sustained fork storms (1000+ events/sec). Unlikely in practice -- DOLI produces 1 block per 10s slot, so even a severe fork generates ~10 events/min. Overflow is theoretical. conf(0.6, inferred)

---

### Realization C: Trait-Injected Emitter

**Architecture.** Define `trait DiagnosticEmitter: Send + Sync { fn record(&self, event: DiagnosticEvent); }`. Two implementations: `NoOpEmitter` (zero-cost, used in tests and when diagnostics DB fails to open per REQ-FORKOBS-LEDGER-009) and `RocksDbEmitter` (wraps DiagnosticLedger, used in production). Node holds `Arc<dyn DiagnosticEmitter>`. Emit calls: `self.emitter.record(event)`.

**Module boundaries.** The trait lives in `crates/storage/src/diagnostic_ledger/emitter.rs`. `NoOpEmitter` is in the same file (~5 lines). `RocksDbEmitter` wraps the ledger. The trait boundary means `bins/node` depends on the trait, not the concrete implementation -- testable with mock emitters that capture events into a `Vec<DiagnosticEvent>` for assertion.

**How from_peer_id is threaded.** Same as A/B -- the `last_block_source` side-channel. The trait interface accepts `DiagnosticEvent` which already has `from_peer_id: Option<String>`. conf(0.6, observed)

**Classifier home.** Same as A/B. The trait does not affect classifier placement. conf(0.55, inferred)

**Pruner integration.** The trait does not prune -- pruning is a concern of the concrete `RocksDbEmitter` or a separate periodic call. Same extraction to periodic submodule as A. conf(0.55, inferred)

**Retroactive validation.** Same schema. The trait boundary adds a test advantage: unit tests can inject a `MockEmitter` that captures events, then pass those events to the classifier for assertion -- no RocksDB needed in tests. This directly supports REQ-FORKOBS-CLF-003 (classifier is pure function, no I/O). conf(0.65, inferred)

**Pros**: Clean test story -- `NoOpEmitter` for existing tests (zero regression risk), `MockEmitter` for new tests (capture + assert), `RocksDbEmitter` for production. Graceful degradation is structural: if DB fails, swap to `NoOpEmitter` at startup. Compile-time guarantee that emit calls cannot affect consensus (trait has no return value that could be branched on).

**Cons**: Trait object dispatch (`dyn DiagnosticEmitter`) adds vtable indirection (~1ns per call, negligible). More abstractions than A -- a trait, two impls, and injection wiring in `init.rs`. Risk of premature abstraction if the trait interface changes frequently in Phase 1 iteration.

**Risks**: Over-engineering concern: the `NoOpEmitter` is structurally identical to `let _ = ledger.record(...)` with `record()` returning `Ok(())` on a degraded instance. The trait buys testability but the codebase does not currently use trait injection for any other subsystem (observed: `block_store`, `state_db`, `utxo_set` are all concrete types on Node). Adding a new pattern for one subsystem may confuse future contributors. conf(0.5, inferred)

---

### Unconventional Option: In-Memory Ring Buffer + Lazy Spill to RocksDB on Query

**What it is.** Instead of writing to RocksDB on every emit, maintain a lock-free ring buffer (e.g., `crossbeam::queue::ArrayQueue<DiagnosticEvent>` with capacity 10,000) in memory. Events are pushed in O(1) with zero I/O. The RocksDB write happens lazily: (a) the periodic pruner task flushes the buffer to disk every 5 seconds, and (b) the RPC handler flushes before querying. The ring buffer IS the primary store for recent events (<5s old); RocksDB is the cold store.

**Why it could be better.** Hot-path overhead is literally a single atomic CAS (~10ns). No async channel, no background task, no RocksDB write on the consensus path. The 50us benchmark gate (REQ-FORKOBS-PERF-001) becomes trivially satisfied. For the primary use case -- "show me what happened in the last hour" -- most events are either still in the ring buffer or were recently flushed. The ring buffer naturally caps memory: 10,000 events x ~500 bytes = ~5MB. Crash loss is bounded to ~5s of events, acceptable for diagnostics.

**The catch.** Two data sources must be merged for queries (ring buffer + RocksDB). Ordering across the boundary needs care -- events in the ring buffer may overlap with recently-flushed events in RocksDB. Deduplication by ULID handles this but adds query complexity. The `crossbeam` dependency may be unwanted. The pattern has no precedent in this codebase. If the periodic flush stalls (e.g., RocksDB compaction), the ring buffer silently overwrites unflushed events.

**Should we keep it on the table?** As a refinement of Realization B, yes. The ring buffer replaces the mpsc channel as the hot-path primitive, and the "lazy spill" replaces the dedicated writer task. But as a standalone realization, the merged-query complexity is not worth the ~40ns improvement over mpsc. conf(0.4, inferred)

---

## Key Evidence

- `periodic.rs` is 1,793 lines (measured) -- any pruner MUST be extracted to a submodule.
- `block_handling.rs:116` -- `handle_new_block` already receives `source_peer: PeerId`, so peer provenance is available at the classify and apply call sites without a new side-channel for classify events.
- `block_handling.rs:42-94` -- `classify_gossip_block` is a pure function with all needed inputs. Emitting a diagnostic event here requires access to the emitter, which the pure function does not have. The emit must happen in the `match class` dispatch at lines 162-321 (in Node's method), not inside the pure function. This is true for all three realizations.
- `state_db/open.rs` -- existing RocksDB open pattern with 6 CFs, Lz4 compression, WAL recovery. DiagnosticLedger can follow the same pattern in a separate DB.
- `RpcContext` (context.rs:41-98) -- already holds `Arc<BlockStore>`, `Arc<StateDb>`, etc. Adding `Arc<DiagnosticLedger>` (or `Option<Arc<DiagnosticLedger>>`) follows the established pattern.
- Node struct (`mod.rs:80-218`) has ~40 fields. All three realizations add 1-2 fields.
- `ws_sender` pattern (`mod.rs:197`, `post_commit.rs:391`) -- an existing `Arc<RwLock<Option<broadcast::Sender>>>` on Node, read in hot paths. Precedent for "fire-and-forget side-channel from apply_block."

## Cross-Perspective Signals

- The Skeptic should examine whether the `classify_gossip_block` pure function creates an awkward emit boundary. The function has no access to the emitter -- the emit must be post-hoc in the match dispatch. This means the event is constructed AFTER the decision, not AT the decision point, which could lead to subtle field mismatches if the dispatch modifies state between classify and emit.
- The Analogist should check whether the `ws_sender` broadcast pattern (already used in post_commit for WebSocket events) is a viable fourth realization -- "piggyback on ws_sender for diagnostic events."
- The 500-line module limit (CLAUDE.md) means `periodic.rs` at 1,793 lines is already in violation. The architect should flag this as pre-existing tech debt, not caused by this feature.

## Gaps

- No benchmark data for RocksDB `put()` latency on the target hardware. The sync-vs-async decision (which distinguishes A from B) cannot be made without this measurement.
- No measurement of how many diagnostic events a severe fork generates per minute. The channel/buffer sizing in B and the unconventional option depend on this.
- The CLI layer (`bins/cli/src/commands/`) was not examined -- the `forks.rs` subcommand integration may have additional constraints.

## Recommended Exploration for the Architect

**Deepest evaluation: Realization A (inline) and Realization C (trait-injected).** They represent the two ends of the simplicity-testability tradeoff. A is the SSF; C is the "proper engineering" option that buys testability. B (event bus) is only needed if the 50us benchmark fails for A.

**Distinguishing experiments (cheap):**
1. Benchmark `rocksdb::DB::put_cf()` with Lz4 compression on a ~500-byte value, 1000 iterations, on the target hardware. If median <50us, Realization A is sufficient. If >50us, B or C-with-async-writer is needed. This is a 20-line Rust benchmark.
2. Count the emit call sites in the final design. If <10, trait injection (C) adds abstraction cost for minimal testability gain -- A suffices. If >15 (e.g., Phase 2 expansion), the trait boundary in C pays for itself.
