# Event Subscriptions Redesign Analysis -- DOLI L1 Phase 2.2

**Date:** 2026-05-25
**Mode:** Proposal-only (Pass 2 of 4 sequential design passes)
**Author:** Antonio Lozada <antonio@omegacortex.ai>
**Position:** Additive to Phase 1 (specs/defi-foundations-economics.md S0 — LOCKED), AMM-First base (specs/defi-subsystem-architecture.md — LOCKED), and Pass 1 Oracle SSF (specs/oracle-structural-anchored-economics.md — LOCKED)

---

## 1. Affected Subsystems (RPC, Transport, Network Layer)

### 1.1 Existing WebSocket Infrastructure (ALREADY OPERATIONAL)

DOLI already has a functioning WebSocket push surface:

- **Transport**: Axum HTTP server at `/ws` endpoint (`crates/rpc/src/server.rs:110-112`) performs WebSocket upgrade via `axum::extract::ws::WebSocketUpgrade`.
- **Event bus**: `tokio::sync::broadcast::channel(256)` (`crates/rpc/src/ws.rs:47`). Single in-process broadcast channel. All WS clients receive all events (no filtering).
- **Event types**: `WsEvent::NewBlock` and `WsEvent::NewTx` (`crates/rpc/src/ws.rs:26-43`). Two variants only.
- **Connection cap**: `MAX_WS_CONNECTIONS = 100` (`crates/rpc/src/ws.rs:18`). Enforced at upgrade time; returns 503 when full.
- **Backpressure**: When a receiver lags, `tokio::sync::broadcast` silently drops oldest events and reports `RecvError::Lagged(n)` (`crates/rpc/src/ws.rs:84`). Client receives no indication.
- **Emission points**:
  - `NewBlock` emitted in `post_commit_actions()` at `bins/node/src/node/apply_block/post_commit.rs:391-400` — AFTER apply_block completes, AFTER archive buffering.
  - `NewTx` emitted in `handle_new_transaction()` at `bins/node/src/node/validation_checks.rs:883-891` — AFTER mempool admission succeeds.

### 1.2 Where Event-Emit Hooks Need to Attach

| Source | Location | Current state |
|--------|----------|---------------|
| Block applied | `apply_block/post_commit.rs:391` | Already emits `NewBlock` (height, hash, slot, producer, tx_count) |
| TX mempool admission | `validation_checks.rs:883` | Already emits `NewTx` (hash, type, size, fee=0) |
| UTXO mutations (create/spend) | `state_db/batch.rs:41,62` (batch ops) OR `utxo/set.rs:77,94` (wrapper) | NO event emission |
| Epoch boundary | `apply_block/post_commit.rs:~296-345` (epoch state transition) | NO event emission |
| Pool UTXO contention | `crates/mempool/src/contention.rs` (diagnostic returned to submitter only) | Per-call return, NOT broadcast |
| Reorg/rollback | `bins/node/src/node/rollback.rs` + `fork_recovery.rs` | Diagnostic ledger only (storage) |
| Producer slash | `bins/node/src/node/rewards.rs` (equivocation) | Log only |
| Oracle price update | `apply_block` at epoch boundary (Pass 1, future) | Does not exist yet |
| Oracle sunset trigger | Epoch boundary (Pass 1, future) | Does not exist yet |

### 1.3 Existing In-Process Event Bus

**Yes, there is one**: `tokio::sync::broadcast::Sender<WsEvent>` stored as `Node.ws_sender: Arc<RwLock<Option<...>>>` (`bins/node/src/node/mod.rs:201`). Created in `startup.rs:485-486` when the RPC server starts.

**Can it back the subscription surface?** Partially. The broadcast channel is write-once-read-many. The problem: it broadcasts ALL events to ALL receivers with no per-subscriber filtering. Filtering must happen either:
- (a) Server-side before `send()` — requires multiple channels or a filter layer, OR
- (b) Client-side after receive — wastes bandwidth, violates bounded-execution (HC-8).

The broadcast capacity (256) is adequate for 10s slots with ~1 block + ~20 TXs per slot. At 100 subscribers, each subscriber gets its own `broadcast::Receiver` (zero-copy clone of Arc'd message).

### 1.4 Network Layer: NOT the Right Place

Gossipsub topics (`/doli/blocks/1`, `/doli/txs/1`, `/doli/producers/1`, `/doli/votes/1`, `/doli/heartbeats/1`, `/doli/headers/1`, `/doli/attestations/1`, `/doli/t1/blocks/1`) are P2P consensus infrastructure. They:
- Serve inter-node propagation, not client subscriptions
- Require libp2p peer identity (not available to RPC clients)
- Are NOT filtered per-subscriber
- Would create consensus coupling if external clients could inject messages

**Conclusion**: Events ship via the existing RPC/WebSocket surface. Gossipsub is irrelevant.

### 1.5 RPC Dispatch Surface

43 methods registered in `crates/rpc/src/methods/dispatch.rs`. All use request-response JSON-RPC 2.0 over HTTP POST. The WebSocket endpoint (`/ws`) is a SEPARATE axum route (`/ws` vs `/`) — it does NOT use JSON-RPC framing. This is a design choice the evaluators must address: should subscriptions use JSON-RPC over WS (standard: `eth_subscribe`) or the existing raw-JSON push model?

---

## 2. Operational Invariants That MUST Be Preserved

| ID | Invariant | Derivation |
|----|-----------|------------|
| EV-1 | Events are READ-ONLY projection — no event emission can mutate UTXO set, mempool, ProducerSet, or ChainState. | HC-3: events are projection of existing state. |
| EV-2 | Three-question checklist: Q1=NO (no user-submittable TX triggers event delivery logic), Q2=NO (no producer/attester action depends on event delivery), Q3=YES (consensus behavior is bit-identical with or without event subscribers). | HC-6, INC-I-075. Events are post-hoc projections of already-committed state. |
| EV-3 | Event ordering for a subscription derived from block stream is deterministic: same block sequence produces same event sequence on any node. | HC-2: deterministic protocol. Required for agent-readiness (reproducibility). |
| EV-4 | Event delivery MUST NOT block `apply_block()` or any consensus-critical path. The emission point uses fire-and-forget (`let _ = ws_tx.send(...)` — already the pattern at `post_commit.rs:392`). | INC-I-081 precedent: sync cascade from blocking paths. |
| EV-5 | Subscription state (filter, cursor) is local to each node instance. No cross-node subscription registry exists. | Creates consensus dependency if shared. |
| EV-6 | Events derived from mempool (pre-finality) MUST carry a `finality: "pending"` marker. Events derived from apply_block carry `finality: "committed"`. | Anti-pattern: agents acting on unfinalized data without awareness. |
| EV-7 | A slow subscriber MUST be disconnected, not buffered indefinitely. Node liveness is non-negotiable. | INC-I-009 (Yamux buffer explosion at 86GB from 136 connections). Backpressure = disconnect. |
| EV-8 | Block-derived events are reproducible: a snap-synced node processing the same blocks will emit the same event stream (for block-derived events). Mempool events are inherently per-node and non-reproducible. | Determinism requirement — agents must know which events are canonical. |
| EV-9 | Event emission adds O(subscribers) work per block, NOT O(events × subscribers). The broadcast channel achieves this (single Arc-clone per send, N receivers wake). | DoS surface: 100 subscribers × 1 block/10s = 10 sends/10s — negligible. |
| EV-10 | The existing `MAX_WS_CONNECTIONS = 100` cap (or its successor) MUST remain enforced. No unbounded connection growth. | INC-I-014 (pending connections bypass limits — must not repeat). |

---

## 3. Quantified Acceptance Criteria

| ID | Criterion | Recommended Value | Derivation |
|----|-----------|-------------------|------------|
| AC-EV-1 | Memory per active subscription | < 4 KB | Filter struct (~256 bytes) + tokio broadcast Receiver overhead (~256 bytes) + per-sub queue depth (16 events × ~200 bytes JSON = 3.2 KB). Total < 4 KB. |
| AC-EV-2 | Maximum concurrent subscriptions per node | ≥ 100 (inherited from existing `MAX_WS_CONNECTIONS`) | 100 × 4 KB = 400 KB total. Negligible relative to node memory (UTXO set alone is ~50-200 MB). |
| AC-EV-3 | Event delivery latency: apply_block completion to subscriber socket write | ≤ 5 ms p50, ≤ 50 ms p99 | `tokio::sync::broadcast::send()` is O(1). Socket write depends on kernel buffer. 50ms p99 accounts for GC/scheduler jitter. |
| AC-EV-4 | Replay-on-reconnect window | Cursor-based: subscriber resumes from `(block_height, event_index)` up to FINALITY_DEPTH (100 blocks) back | 100 blocks × ~20 events/block × 200 bytes = 400 KB ring buffer per node (not per subscriber). |
| AC-EV-5 | DoS resistance: subscriber spawning K subscriptions per connection | Max 1 subscription per WS connection (simplest). Connection cap = 100. Malicious client consuming 100 connections = 400 KB + 100 TCP sockets. | Bounded by existing connection cap. |
| AC-EV-6 | Filter evaluation time | < 1 ms per filter per event | Filter is a struct comparison (OutputType enum match, address byte equality, AND/OR of 2-3 predicates). No regex, no computation. |
| AC-EV-7 | Agent-readiness 4/4 | (A) subscribe/unsubscribe + getEventCatalog discoverable via JSON-RPC; (B) block-derived events ordered by (height, tx_index, output_index); (C) cost ceiling known: 1 WS connection, ≤ 4 KB memory, disconnect on lag; (D) event JSON parsable with standard JSON decoder, no bespoke binary. | HC-8 primary acceptance criterion. |
| AC-EV-8 | Slow-consumer disconnect threshold | > 64 lagged events (broadcast channel lag count). Client receives a `{"type":"error","code":"SUBSCRIPTION_LAGGED","missed":N}` before disconnect. | tokio broadcast already tracks lag. |

---

## 4. Architecture Context

### Module Boundaries

| Module | Responsibility | Depends on | Depended by |
|--------|---------------|------------|-------------|
| `crates/rpc/` | HTTP + WS server, dispatch, auth | `crates/mempool/`, core types | External clients, CLI |
| `crates/rpc/src/ws.rs` | WS event types, broadcast, socket handler | tokio broadcast | `bins/node/` (emits events) |
| `bins/node/src/node/apply_block/` | Block application, UTXO mutation, epoch boundary | `crates/storage/`, `crates/core/` | Event emission hook (post_commit) |
| `bins/node/src/node/validation_checks.rs` | TX admission, mempool write | `crates/mempool/` | Event emission hook (NewTx) |
| `crates/mempool/src/contention.rs` | P5 pre-simulation diagnostic | `crates/storage/` (UTXO lookup) | Returned to submitter via `sendTransaction` response |
| `crates/storage/` | UTXO set, block store, state DB | RocksDB | Node, RPC |

### Data Flow: Event Emission

```
[apply_block completes]
  → post_commit_actions()
    → ws_sender.send(WsEvent::NewBlock{...})
      → tokio::sync::broadcast fan-out (zero-copy Arc)
        → per-subscriber Receiver::recv()
          → [FILTER EVAL: new layer needed]
            → JSON serialize
              → WebSocket frame → TCP → client
```

For UTXO-level events (new capability):
```
[apply_block: tx_processing loop]
  → utxo_set.add_transaction(tx, height, ...)
  → [NEW HOOK: emit per-output event with OutputType, address, amount]
    → broadcast channel → filter → serialize → socket
```

### Dependency Direction

- Events are a LEAF — nothing depends on them. They depend on:
  - `WsEvent` enum (crates/rpc)
  - `tokio::sync::broadcast` (runtime)
  - Data from `Block`, `Transaction`, `Output` structs (crates/core)
- Events do NOT feed back into any consensus path.
- Events do NOT require storage persistence (ephemeral ring buffer for replay only).

### Snap-Sync Irrelevance

Events are runtime-only. They are NOT persisted to the state root. A snap-synced node emits events only for blocks it processes AFTER snap sync completes. Historical events for blocks before the snap point are unavailable (this is acceptable — events are real-time, not archival).

### Three-Question Checklist (Design-Space Level)

1. **Q1: Can any user-submittable transaction trigger this code path?** NO. Event delivery is triggered by `apply_block` completing (which processes transactions, but the event emission is a post-hoc notification, not a transaction-triggered path). A subscriber cannot cause any write.
2. **Q2: Can any producer-action or attestation pattern trigger it?** NO. Producers emit blocks which trigger events, but the event delivery itself is invisible to producers and attesters. No producer behavior changes based on whether subscribers exist.
3. **Q3: Is the new behavior bit-identical to the old behavior for ALL reachable inputs?** YES. The node processes blocks identically whether 0 or 100 subscribers are connected. Event emission is fire-and-forget (`let _ = send(...)`) — the `_` discard means send failure does not propagate.

---

## 5. Capability Inventory (Existing Primitives)

### 5.1 RPC Dispatch Pattern

- camelCase verb-first naming: `getBlockByHeight`, `sendTransaction`, `getPoolPrice`
- JSON-RPC 2.0 over HTTP POST to `/`
- Error codes: standard (-32700 to -32603) + custom (-32000 to -32008)
- Params: positional or named (object)
- Auth: bearer token for admin methods; no auth for read methods

### 5.2 Transport Layer (WebSocket Already Exists)

- Axum 0.x with `axum::extract::ws` module
- `/ws` route registered alongside `/` (RPC POST)
- WebSocket upgrade via HTTP GET + Upgrade headers (standard RFC 6455)
- Runs on the SAME `tokio::net::TcpListener` as the JSON-RPC HTTP server (port 8500/18500/28500)
- NO separate WebSocket port needed

### 5.3 Rate-Limit / Quota Infrastructure

- **RPC layer**: NONE. No rate limiting exists in `crates/rpc/src/`. The only protection is `MAX_BODY_SIZE = 2MB` and admin-token auth.
- **Network layer**: Token bucket rate limiter exists (`crates/network/src/rate_limit.rs`) for P2P peers — not applicable to RPC clients.
- **WebSocket layer**: Connection count cap (`MAX_WS_CONNECTIONS = 100`) but no per-connection message rate limit.

### 5.4 Existing In-Process Event Surfaces

| Surface | What it provides | Usable for subscriptions? |
|---------|-----------------|--------------------------|
| `tokio::sync::broadcast<WsEvent>` | NewBlock + NewTx broadcast to all WS clients | YES — extend WsEvent enum |
| Diagnostic Ledger (`storage/diagnostic_ledger/`) | BlockApplied, BlockRejected, ForkBlockReceived, RollbackStarted, etc. | INDIRECTLY — these write to RocksDB for forensics; could emit events in parallel |
| P5 Contention Diagnostic | `MempoolDiagnostic { contention: Option<ContentionInfo> }` returned per-TX | NO — returned synchronously to submitter, not broadcast |
| Gossip mesh metrics (swarm_loop.rs:340-346) | Periodic mesh peer counts per topic | NO — internal instrumentation |

### 5.5 Gossipsub Topics (Not Usable for Client Events)

8 topics exist: blocks, transactions, producers, votes, heartbeats, headers, tier1-blocks, attestations. All P2P inter-node — require libp2p peer identity. NOT usable for RPC client subscriptions.

### 5.6 Logging/Metrics Infrastructure

- `tracing` crate for structured logging
- Log lines like `[APPLY_START]`, `[APPLY_END]`, `[GOSSIP_RECV]` exist for correlation
- No Prometheus/metrics endpoint found in RPC
- Could serve as parallel event sink if needed (low priority)

---

## 6. Open Design Questions (For Evaluators)

### Q1: Transport Choice

| Option | Pros | Cons | Fit for DOLI |
|--------|------|------|--------------|
| **WebSocket (extend existing `/ws`)** | Already operational; bidirectional (client can send subscribe/unsubscribe commands); standard; all agent SDKs support it; single port | Stateful connection; needs ping/pong keepalive; existing implementation has no filtering or subscription commands | HIGH — already deployed, proven at 100 connections |
| SSE (Server-Sent Events) | Unidirectional (simpler); auto-reconnect built into browsers; HTTP/2 multiplexing | No client→server channel (subscribe params must go via separate HTTP call); less agent-SDK support than WS; no binary frames | MEDIUM — simpler but requires a second mechanism for subscribe commands |
| Long-poll | Stateless; works through any proxy | Latency = poll interval; high overhead per event; not real-time | LOW — contradicts "without polling" requirement |
| gRPC server-streaming | Strong typing via protobuf; flow control built in | Requires separate port + proto tooling; DOLI has no protobuf anywhere; heavyweight for a 2-event stream | LOW — overengineered for current scale |
| libp2p gossipsub | Already exists for P2P | Requires libp2p peer identity; mixes consensus and client traffic; no filtering; massive complexity for external clients | REJECTED — consensus coupling |

**Evaluator question:** Is extending the existing WebSocket at `/ws` sufficient, or should a new `/ws/subscribe` endpoint be created to preserve backward compatibility with current unfiltered clients?

### Q2: Filter Language

| Option | Expressiveness | Complexity | Example |
|--------|---------------|------------|---------|
| **JSON predicate object** | High (AND/OR, nested) | Medium | `{"output_type": "Pool", "address": "abc...", "op": "AND"}` |
| Hardcoded enum of subscription types | Low | Trivial | `subscribe("new_blocks")`, `subscribe("utxo_pool")` |
| GraphQL-style | Very high | Heavy (parser, resolver) | `subscription { utxoCreated(outputType: Pool) { ... } }` |
| Custom DSL string | Configurable | Custom parser needed | `"output_type=Pool AND address=abc..."` |

**Evaluator question:** The hardcoded enum approach (e.g., `subscribe({topic: "blocks"})`, `subscribe({topic: "utxo", filter: {output_type: 9}})`) satisfies agent-readiness (bounded, discoverable) without requiring a grammar. Is this sufficient or do agents need compound predicates (AND/OR)?

### Q3: Ordering Guarantees

| Mode | Guarantee | Cost |
|------|-----------|------|
| **Block-ordered** (default) | Events within a block delivered in deterministic order: (tx_index ASC, output_index ASC). Cross-block: block_height ASC. | Zero cost — natural apply_block processing order |
| Best-effort | Events may arrive out of order during reorgs or heavy load | Simpler implementation but breaks agent-readiness (determinism) |
| Ordered-on-finality | Only deliver events after finality depth (100 blocks) | 100-block latency (~16 minutes) — too slow for reactivity |

**Evaluator question:** Should the default be block-ordered with an optional `finality_depth` parameter that delays delivery until N confirmations?

### Q4: Replay-on-Reconnect

| Approach | Persistent state? | Complexity |
|----------|-------------------|------------|
| **Cursor-based (height + event_index)** | Node keeps ring buffer of last N blocks' events (ephemeral, in-memory) | Low — ring buffer ~400 KB |
| Durable subscription state | Persisted to RocksDB | Medium — new CF, compaction, cleanup |
| No replay (reconnect = fresh start) | None | Trivial but breaks agent-readiness (agents must handle gaps) |

**Evaluator question:** Is an in-memory ring buffer of the last 100 blocks' events (erasable on restart) sufficient, or do agents need replay across node restarts (requiring persistent state)?

### Q5: Backpressure / Rate-Limit

| Policy | Behavior | Risk |
|--------|----------|------|
| **Slow-consumer disconnect** | After N lagged events, send error frame + close socket | Agent must reconnect (cursor-based replay covers gap) |
| Per-sub queue depth limit + drop-oldest | Server drops old events, delivers most recent | Agent sees gaps without explanation |
| Unlimited buffering | Never disconnect | INC-I-009 repeat (memory explosion) |

**Evaluator question:** Is disconnect-after-64-lag the right threshold? Should a warning frame be sent at 32 lag before disconnect at 64?

### Q6: Auth / Quota

| Model | For whom | Complexity |
|-------|----------|------------|
| **No auth (same as read-only RPC)** | Events are read-only projections; same trust model as `getBlockByHeight` | Zero |
| Optional API key + per-key concurrent-subscription cap | Multi-tenant hosting providers | Medium (key store, per-key accounting) |
| DOLI-staked quota | Subscription budget proportional to on-chain stake | High (on-chain dependency — likely violates HC-3) |

**Evaluator question:** Given that events are read-only and the connection cap (100) already limits resource consumption, is no-auth the correct default? If a future multi-tenant scenario arises, can API-key auth be added later without protocol change?

### Q7: Oracle Event Projection

| Approach | Events needed | Complexity |
|----------|---------------|------------|
| **Generic UTXO event surface** | `utxo_created{output_type: OraclePrice, ...}` covers price update; `tx_included{tx_type: PriceAttestation}` covers attestation inclusion; slash events via existing slash hook | ZERO oracle-specific event types |
| Oracle-specific event types | `oracle_price_update{pair_id, price, epoch}`, `oracle_sunset{pair_id, threshold}`, `oracle_equivocation{attester, epoch}` | 3 new event type variants + dedicated emission hooks |

**Evaluator question:** The generic surface covers all 4 oracle event requirements (price UTXO mutation, sunset trigger, equivocation slash, attestation inclusion) without oracle-specific code. Does this satisfy agent ergonomics, or do agents need pre-composed oracle event types for developer experience?

### Q8: Multi-Node Event Consistency

- **Block-derived events**: Deterministic. Same blocks = same events on any node (EV-3, EV-8).
- **Mempool events**: Per-node. Different nodes see different mempool admission order, different contention states.

**Evaluator question:** Should the spec explicitly mark each event type as `deterministic: true/false`? Should mempool events carry a `node_id` or be opt-in?

### Q9: Mempool-Event Safety (MEV Vectors)

Exposing mempool state creates MEV vectors:
- A subscriber sees pending swaps → front-runs by submitting a higher-fee swap before the next block
- Contention information (P5 diagnostic) reveals how many competing TXs exist for a pool

DOLI mitigates: P5 already returns contention to the SUBMITTER only, not broadcast. If mempool events are broadcast, they MUST NOT include identifying information about competing transactions (only aggregate counts).

**Evaluator question:** Should mempool events be:
- (a) Opt-in via a separate subscription topic (default OFF),
- (b) Always available but stripped of TX hashes (only counts/types),
- (c) Gated behind admin auth (same as `getStateSnapshot`), or
- (d) Excluded entirely from Phase 2.2 (defer to Phase 3)?

---

## 7. Out of Scope (Won't)

| Item | Reason |
|------|--------|
| Cross-node subscription registry | Creates consensus dependency (violates EV-5, HC-2) |
| Pub/sub middleware (Pusher, Ably, Kafka) | HC-1: L1-native only |
| New TX types for event emission | HC-3: events are read projection |
| Activation height for events | HC-6: purely additive RPC, not consensus-visible |
| Events that mutate state | HC-3: read-only projection |
| WebSub / W3C ActivityPub standards | Overengineering for a 34-producer PoS chain |
| Long-term persistent subscription state beyond cursor | Overengineering — in-memory ring buffer suffices |
| Server-side aggregation/computation | Events are deliveries, not queries; RPC handles queries |
| Frontend SDK design | Downstream consumer problem (Pass 4 or external) |
| Metrics/Prometheus endpoint | Orthogonal infra concern, not event subscription |
| Historical event replay from genesis | Events are real-time; archival access uses existing RPC queries |
| Binary event encoding (protobuf/MessagePack) | JSON is sufficient for 1 block/10s cadence; satisfies agent-readiness composability |

---

## Appendix: What I DON'T Understand (Intellectual Honesty)

1. **Broadcast channel vs. per-subscriber channel performance at 100 subscribers with filtering**: The existing `tokio::sync::broadcast` sends ALL events to ALL receivers. If filtering is server-side, we need either (a) multiple broadcast channels per topic, or (b) a filter layer between broadcast and socket write. I have not measured the overhead of option (b) for 100 concurrent subscribers each evaluating a 3-predicate filter.

2. **Ring buffer memory under reorg**: If a 10-block reorg occurs, the ring buffer must invalidate events for those 10 blocks. The mechanism for invalidation (mark-and-skip vs. rebuild) is not designed.

3. **Interaction between WS connection cap and subscription cap**: Currently 1 WS connection = 1 unfiltered firehose. If we add subscription commands, does 1 connection = 1 subscription, or can 1 connection host multiple subscriptions? The latter is more agent-friendly but increases per-connection memory.

4. **Event volume at full DeFi load**: With AMM pools active, a single block could contain 50+ transactions, each creating/spending 2-5 UTXOs. At 100-250 UTXO events per block per 10 seconds, the broadcast channel capacity (256) is tight. May need increase or separate high-volume channel.

---

## Appendix: Anchor Detection

**FIRST READ:** "Events need WebSocket because it already exists and works." Setting aside.

**SECOND READ (contradicting):** "The existing WS implementation is a dumb firehose with no filtering, no subscription commands, no replay, no cursor. It is a prototype that broadcasts 2 event types to all connected clients. What Phase 2.2 needs is fundamentally different: per-subscriber filtering, JSON-RPC subscription protocol, replay cursors, backpressure signaling. The existing `/ws` is a starting point for the transport layer only — every other aspect (protocol, filtering, lifecycle) must be designed from scratch."

**Resolution:** Both are true. The TRANSPORT (WebSocket over Axum, TCP port sharing, TLS, connection handling) is reusable. The PROTOCOL (subscription commands, filter language, cursor semantics, lifecycle) does not exist and must be designed. The evaluators should not conflate "WebSocket exists" with "subscription system exists."
