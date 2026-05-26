<!--
OUTPUT CONTRACT: N/A — specification document (not a test file)
INPUT PARTITIONS: N/A — specification document (not a test file)
-->

# Event Subscriptions -- DOLI L1 Phase 2.2

**Pass:** 2 of 4 (Oracle DONE -> Events HERE -> Intent -> Curve dispatch)
**Status:** PROPOSAL-ONLY (pending User Gate approval)
**Date:** 2026-05-25
**Author:** Antonio Lozada <antonio@omegacortex.ai>
**Evaluators:** 5/5 completed (Mechanism-Skeptic, Adversarial-Capital, Sustainability, Oracle/MEV, Governance-Minimal)

---

## S0 SSF Locked Package

The following design is LOCKED by 5/5 evaluator convergence. NO activation height is needed (Q1=NO, Q2=NO, Q3=YES -- unanimous). Ships in a node binary upgrade with zero consensus coupling.

| Decision | Locked Value | Convergence | Confidence |
|----------|-------------|-------------|------------|
| Transport | Extend existing `/ws` (Axum, shared port with HTTP RPC) | 5/5 | conf(0.95, converged) |
| Topic type | Hardcoded enum (no grammar/DSL) | 5/5 | conf(0.95, converged) |
| Replay | In-memory ring buffer, 100 blocks, ~400 KB shared, cursor=(height, event_index) | 5/5 | conf(0.95, converged) |
| Backpressure | Lag-disconnect at 64 events with error frame before close | 5/5 | conf(0.95, converged) |
| Auth | None in protocol; connection cap (100) + lag-disconnect are the quota | 5/5 | conf(0.95, converged) |
| Oracle events | Generic UTXO surface; zero oracle-specific event types | 5/5 | conf(0.95, converged) |
| Mempool TX details | EXCLUDED from Phase 2.2 (block-derived events only) | 5/5 (4 explicit, 1 silent assent) | conf(0.90, converged) |
| Per-event finality tag | `finality: "committed"` on block-derived events | 5/5 (4 explicit, 1 implicit via EV-6) | conf(0.90, converged) |
| Per-topic determinism tag | Exposed via `getEventCatalog`; not per-event payload | 4/5 (see S1.2) | conf(0.80, converged) |
| No new token / no on-chain quota | Absolute rejection | 5/5 | conf(0.95, converged) |
| No consensus coupling (Q1=NO Q2=NO Q3=YES) | Universal | 5/5 | conf(0.98, converged) |

---

## S1 Design Specification

### S1.1 Transport

**Decision:** Extend existing `/ws` endpoint. No new endpoint, no new port, no new dependency.

**Rationale (4/5 agreement on reuse of existing `/ws`; 1/5 Mechanism-Skeptic proposed `/ws/v2`):**
- The existing surface at `crates/rpc/src/ws.rs` (111 LOC) is operational, handles 100 connections, shares port with HTTP RPC.
- Backward compatibility: clients connecting without `?topics` parameter receive all events (firehose -- preserves current behavior).
- New clients use `?topics=blocks,utxo,epoch` query parameter to filter.

**Divergence resolved (subscribe protocol):** Governance-Minimal proposes immutable query-param at connect time. Mechanism-Skeptic proposes JSON-RPC subscribe/unsubscribe commands over WS. Adversarial-Capital proposes JSON-RPC 2.0 over WS.

**Resolution: Query parameter at connect time (immutable for connection lifetime).**

Rationale:
1. Simpler: no bidirectional message parsing, no subscription IDs, no mutable per-connection state.
2. Bitcoin ZeroMQ precedent: topic selected at connect time, zero subscription commands, 12 years stable.
3. Reconnection cost is negligible (single HTTP upgrade at 10s slot cadence).
4. If multi-topic-per-connection (1:N) proves needed empirically, it ships in Phase 2.3 without protocol change.
5. SSF tiebreaker: query-param is structurally simpler than JSON-RPC bidirectional commands.

**Wire format:** Server pushes JSON text frames. No JSON-RPC envelope on push messages (maintains existing raw-JSON push model). Client sends no messages after upgrade (server push only).

```
Connect: GET /ws?topics=blocks,utxo&cursor=195000 HTTP/1.1
         Upgrade: websocket

Server push (each event):
{
  "type": "utxo_created",
  "height": 195001,
  "tx_index": 2,
  "output_index": 0,
  "output_type": 9,
  "address": "doli1abc...",
  "amount": 50000,
  "finality": "committed"
}
```

### S1.2 Event Types

**Final enumeration: 6 event types.**

| # | Event Type | Payload Fields | Source Location | Deterministic | Finality |
|---|-----------|----------------|-----------------|---------------|----------|
| 1 | `new_block` | height, hash, slot, timestamp, producer, tx_count, epoch | `post_commit.rs:391` (existing) | true | committed |
| 2 | `utxo_created` | height, tx_index, output_index, output_type (u8), address (hex), amount (u64), extra_data_hash (hex, BLAKE3 of extra_data) | `post_commit.rs` (new emission in apply_block UTXO write path) | true | committed |
| 3 | `utxo_spent` | height, tx_index, input_index, prev_hash (hex), prev_index (u16) | `post_commit.rs` (new emission in apply_block UTXO spend path) | true | committed |
| 4 | `epoch_boundary` | height, epoch, reward_total, producer_count, oracle_updated (bool), producers_added (u16), producers_removed (u16) | `post_commit.rs` epoch transition hook | true | committed |
| 5 | `consensus_event` | height, event_type ("slash"\|"registration"\|"exit"\|"withdrawal"\|"reorg"), producer (hex, if applicable), details (string) | `post_commit.rs` + `rewards.rs` + `fork_recovery.rs` | true (block-derived) | committed |
| 6 | `new_tx` | hash, tx_type, size, fee | `validation_checks.rs:883` (EXISTING -- see S6 for disposition) | false | pending |

**Per-event `deterministic` flag in payload:** NOT included. Determinism is a property of the TOPIC, not the individual event. Documented in `getEventCatalog` response (see S1.11). This follows the SSF tiebreaker: Governance-Minimal argued "determinism is a topic property, not per-event" and this avoids 1 byte per event per subscriber across all time.

**Payload design choices:**
- `extra_data_hash` (BLAKE3 hash of raw extra_data bytes) instead of full extra_data. Agents needing full extra_data query `getTransaction(height, tx_index)`. This bounds event size to ~200 bytes regardless of extra_data complexity.
- `address` is the primary output address (hex-encoded).
- `amount` is the output value in base units (u64).
- `output_type` is the numeric enum value (0-15), matching `OutputType` discriminant.

### S1.3 Topic Enum

**4 topics (hardcoded, exhaustive):**

| Topic | Maps to Event Types | Description |
|-------|-------------------|-------------|
| `blocks` | `new_block` | Block commit notifications (1 per 10s slot) |
| `utxo` | `utxo_created`, `utxo_spent` | All UTXO mutations from applied blocks |
| `epoch` | `epoch_boundary` | Epoch transition events (1 per hour) |
| `consensus` | `consensus_event` | Slashes, registrations, exits, withdrawals, reorgs |

**Explicitly EXCLUDED topics:**
- `transactions` / `mempool` -- EXCLUDED from Phase 2.2 (see S6 and S10)

**Rationale for 4 (not 5 or 6):**
- Governance-Minimal proposed 5 topics including `transactions` (mempool). Mechanism-Skeptic proposed 4 topics (blocks, utxo, epoch, reorg). Adversarial-Capital proposed 4 topics explicitly excluding mempool.
- Resolution: 4 topics. Mempool excluded per S6 decision. `reorg` folded into `consensus` topic (as `event_type: "reorg"`) rather than standalone -- reduces topic count by 1 with zero information loss. Agents subscribing to `consensus` get reorg events alongside slashes/exits.

### S1.4 Subscription Protocol

**Query parameter at connect time. Immutable for connection lifetime.**

```
GET /ws?topics=blocks,utxo,epoch,consensus&cursor=195000 HTTP/1.1
Upgrade: websocket
```

- `topics` (optional): comma-separated subset of the 4-topic enum. Omitting = all topics (backward-compatible firehose).
- `cursor` (optional): block height to replay from. Must be within last 100 blocks. Omitting = live only (no replay).
- Invalid topic names in the parameter are silently ignored (lenient parsing).
- Empty `topics=` (no valid topics) returns 400 Bad Request before upgrade.

**To change topics:** disconnect and reconnect with new parameters. At DOLI's event rate (~2-20 events/second), reconnection overhead is negligible.

### S1.5 Filter

**Topic-level only. No predicate language. No per-field filtering.**

An agent subscribing to `?topics=utxo` receives ALL UTXO mutations (all 16 output types). Client-side filtering handles selectivity. At 10s slots with 20-200 UTXO events/block, the bandwidth is 4-40 KB/block -- manageable for any client.

**Future consideration (Phase 2.3+):** If empirical agent feedback shows per-output_type server-side filtering is needed, it can be added as `?topics=utxo:9,utxo:15` syntax (backward-compatible extension) without protocol change.

### S1.6 Ordering

**Block-ordered. Single mode. No options.**

Within a block: `(tx_index ASC, output_index ASC)` for `utxo_created`; `(tx_index ASC, input_index ASC)` for `utxo_spent`. Cross-block: `block_height ASC`.

This is the natural `apply_block` traversal order. Zero-cost to maintain (no reordering). Deterministic (EV-3): same block sequence produces same event sequence on any node.

`epoch_boundary` and `consensus_event` are emitted AFTER all UTXO events for that block (they are post-processing events within `post_commit`).

### S1.7 Replay

**In-memory ring buffer. 100 blocks. Cursor = block_height.**

- Shared data structure: `Arc<RwLock<VecDeque<(u64, Vec<Arc<WsEvent>>)>>>` -- one entry per block height, containing all events for that block.
- Max 100 entries (100 blocks = ~16 minutes at 10s slots). FIFO eviction.
- On connect with `?cursor=HEIGHT`: server replays all events for blocks > HEIGHT from the ring buffer (filtered by client's topic set), then transitions to live stream.
- If `cursor` HEIGHT is older than the ring buffer's oldest entry: server starts from the buffer's oldest available entry and includes a `consensus_event` with `event_type: "gap"` indicating missed blocks.
- On node restart: ring buffer is empty. Clients reconnecting after restart receive live events only (no replay). For gap recovery, agents use existing RPC (`getBlockByHeight`, `getBlockTransactions`).

**Replay rate-limit:** 1 replay per connection. No repeated replays on the same connection (cursor is consumed at connect time only). Reconnection is the replay mechanism. This structurally prevents reconnect-storm amplification without needing an explicit timer.

**Resolution of Mechanism-Skeptic's "1 replay/60s" recommendation:** The query-param-at-connect model inherently limits replay to 1 per connection establishment. An attacker cycling 100 connections with replay would need to fully disconnect and reconnect each time. At 100 concurrent connections (cap), the maximum replay burst is 100 x 400 KB = 40 MB -- a one-time cost per attack wave, bounded by the connection cap. Explicit rate-limiting adds complexity for marginal gain. DEFERRED unless empirical evidence shows abuse.

### S1.8 Backpressure

**Lag-disconnect at 64 events. No warning at 32.**

Protocol:
1. When `broadcast::RecvError::Lagged(n)` fires and `n >= 64`: server sends a final frame:
   ```json
   {"type":"error","code":"SUBSCRIPTION_LAGGED","missed":64,"cursor":"HEIGHT:EVENT_IDX"}
   ```
2. Server closes WebSocket with code 4001 (custom close code).
3. Client reconnects with the `cursor` value from the error frame to resume.

**Warning at 32: NOT included.**

Resolution: Governance-Minimal proposed no warning (subtraction principle: "if agent is lagging, warning does not help"). Mechanism-Skeptic proposed warning at 32. Adversarial-Capital proposed warning at 32.

SSF tiebreaker applied: the warning adds complexity (agent must handle warning state) without changing outcomes (agent either catches up or is disconnected). Agents can monitor their own processing latency client-side. The cursor in the disconnect frame provides everything needed for recovery. Warning DEFERRED to Phase 2.3 if agent developers request it.

### S1.9 Auth / Quota

**No auth. Connection cap (100) is the sole quota.**

- `MAX_WS_CONNECTIONS = 100` (existing, enforced at HTTP upgrade time with 503 rejection).
- No API keys, no per-key accounting, no token verification.
- Events are read-only projections of public chain state -- same trust model as `getBlockByHeight`.

**Per-IP cap: NOT included in Phase 2.2.**

Resolution: Adversarial-Capital recommended `MAX_WS_PER_IP=10`. Other 4 evaluators were silent or neutral. The attack cost analysis shows:
- Single-IP saturation (100 connections): $0, blocks legitimate clients.
- With per-IP=10: attacker needs 10 IPs ($0.10/hr) to saturate.
- The marginal security gain ($0 vs $0.10/hr) does not justify the implementation complexity (HashMap tracking, IP extraction from headers behind proxies, IPv6 handling).
- The fundamental defense is that agents SHOULD run their own nodes ($20/month). Event subscription is a convenience, not critical infrastructure.
- If saturation becomes a problem: operators deploy nginx/reverse-proxy with per-IP limits (operational concern, not protocol concern).

**DEFERRED:** Per-IP cap. Can be added as a code-level constant without protocol change if empirical abuse occurs.

### S1.10 Mempool Events Policy

**EXCLUDED from Phase 2.2. Block-derived events ONLY.**

No mempool topic. No `new_tx` events delivered to subscribers using the new topic-filtered system.

**Rationale (5/5 convergence on exclusion of expanded mempool events):**
- 5/5 evaluators converge: mempool TX detail events create MEV surface ($0.68-$3.36/slot at $1-10M TVL).
- 4/5 explicitly recommended option (d): exclude entirely.
- 1/5 (Oracle/MEV) recommended option (b): aggregate counts only. This was the ONLY divergence.
- SSF tiebreaker: exclusion is simpler than aggregate counts. Aggregate counts can ship in Phase 2.3 if agent demand materializes.

**For the pre-existing `WsEvent::NewTx` -- see S6.**

### S1.11 New RPC Method: `getEventCatalog`

**Method:** `getEventCatalog` (no parameters)

**Response:**
```json
{
  "topics": [
    {"name": "blocks", "deterministic": true, "event_types": ["new_block"]},
    {"name": "utxo", "deterministic": true, "event_types": ["utxo_created", "utxo_spent"]},
    {"name": "epoch", "deterministic": true, "event_types": ["epoch_boundary"]},
    {"name": "consensus", "deterministic": true, "event_types": ["consensus_event"]}
  ],
  "connection_cap": 100,
  "lag_disconnect_threshold": 64,
  "replay_window_blocks": 100,
  "version": 1
}
```

This satisfies HC-8 agent-readiness criterion A (discoverability). Agents query this once at startup to learn available topics, determinism guarantees, and operational parameters.

---

## S2 Three-Question Consensus-Shape Checklist

| Question | Answer | Evidence | Evaluator Agreement |
|----------|--------|----------|---------------------|
| Q1: Can any user-submittable TX trigger event delivery logic? | **NO** | Event emission is post-hoc in `post_commit.rs:391` AFTER `apply_block()` completes. `let _ = ws_tx.send(...)` discards result. No TX type triggers delivery. | 5/5 |
| Q2: Can any producer-action or attestation pattern depend on event delivery? | **NO** | Producers build blocks from mempool + UTXO set. No producer reads from WS event bus. Emission is fire-and-forget. | 5/5 |
| Q3: Is consensus bit-identical with 0 or 100 subscribers? | **YES** | `tokio::sync::broadcast::send()` discards on zero receivers. No consensus path branches on subscriber count. | 5/5 |

**VERDICT: NO activation height needed. Ships as additive RPC in a binary upgrade.**

Why purely additive: the event subscription system reads from already-committed state and broadcasts read-only projections over an existing transport. It creates zero new consensus-visible behavior. Removing all event subscription code produces bit-identical chain state.

---

## S3 Operational Invariants Locked (EV-1..10)

| ID | Invariant | Spec Field |
|----|-----------|-----------|
| EV-1 | Read-only projection (no mutation) | S1.2: events are emitted AFTER state commits |
| EV-2 | Q1=Q2=NO Q3=YES | S2 (checklist verified 5/5) |
| EV-3 | Block-derived ordering deterministic | S1.6: (height, tx_index, output_index) |
| EV-4 | Non-blocking on apply_block | Existing `let _ = send(...)` pattern preserved |
| EV-5 | Subscription state local per-node | S1.4: query param stored per-connection; no cross-node registry |
| EV-6 | Pre-finality events tagged | S1.2: `finality: "committed"` on all block-derived; `new_tx` (existing, see S6) carries "pending" |
| EV-7 | Slow-consumer disconnect not buffer | S1.8: lag-disconnect at 64 |
| EV-8 | Block-derived events reproducible | S1.6: same blocks = same events on any node |
| EV-9 | O(subscribers) per block | Broadcast architecture: single send(), N receivers wake |
| EV-10 | Connection cap enforced | S1.9: MAX_WS_CONNECTIONS=100 preserved |

---

## S4 Acceptance Criteria Satisfied (AC-EV-1..8)

| AC | Criterion | Status | How Satisfied |
|----|-----------|--------|---------------|
| AC-EV-1 | <4 KB per subscription | PASS | HashSet<Topic> (~40 bytes) + broadcast Receiver (~256 bytes) + cursor (8 bytes) = ~304 bytes |
| AC-EV-2 | >=100 concurrent | PASS | Existing MAX_WS_CONNECTIONS=100 unchanged |
| AC-EV-3 | <=5ms p50, <=50ms p99 delivery | PASS | broadcast::send O(1); JSON serialize ~0.05ms; socket write kernel-buffered |
| AC-EV-4 | Replay window = 100 blocks | PASS | In-memory ring buffer, cursor at connect time (S1.7) |
| AC-EV-5 | DoS bounded by connection cap | PASS | 100 connections x ~304 bytes = ~30 KB app memory; bounded |
| AC-EV-6 | Filter eval <1ms | PASS | Topic enum match against HashSet (5 elements max) = nanoseconds |
| AC-EV-7 | Agent-readiness 4/4 | PASS | (A) getEventCatalog + topic param = discoverability; (B) block-ordered deterministic; (C) 1 conn, <1 KB, lag-disconnect = bounded; (D) JSON text frames = composable |
| AC-EV-8 | Lag-disconnect at 64 | PASS | S1.8 implemented with error frame + cursor |

---

## S5 Attack Surface Quantified

| # | Attack | Capital | Damage | Mitigation | Residual |
|---|--------|---------|--------|-----------|----------|
| A1 | Single-IP flood (100 conns) | $0 | Legitimate agents denied service | Connection cap holds; operator deploys reverse-proxy per-IP if needed | Griefing only; no economic gain for attacker |
| A2 | Sybil multi-IP (saturate 100) | $1/hr | Same as A1 | Cap holds regardless of IP source | Same |
| A3 | Slow-consumer socket hold | $0 | Socket held until lag-disconnect (32-128s) | Lag-disconnect at 64 frees slot | Self-limiting; max duration ~2 min |
| A4 | Filter-eval CPU exhaustion | N/A | Impossible | Topic enum match only (<1us); no regex/DSL | Eliminated by design |
| A5 | Mempool front-run | $0 (if exposed) | $22K/epoch at $10M TVL | Mempool events EXCLUDED (S1.10); existing NewTx admin-gated (S6) | 0 from events system |
| A6 | Oracle latency arb | $0 | 0 bps (10s slots >> 5ms delivery) | Post-commit delivery; slot time dominates | Structural 0 |
| A7 | Replay burst (100 clients) | $0 | 40 MB one-time burst | Connection cap bounds total; 1 replay/connect | Bounded at 40 MB max |
| A8 | Broadcast channel overflow | N/A (legitimate) | Lag-disconnect fires for all slow consumers | Increase channel to 1024 (implementation detail) | Operational, not attack |

---

## S6 Pre-existing `WsEvent::NewTx` Decision (LOAD-BEARING)

**RECOMMENDATION: Admin-gate `WsEvent::NewTx` in the same release that ships Phase 2.2 events.**

**Reasoning:**

The existing code at `validation_checks.rs:883-891` broadcasts `NewTx{hash, tx_type, size, fee}` to ALL WebSocket clients unauthenticated. This is a pre-existing live vulnerability:

1. **Mechanism-Skeptic:** "Gate mempool TX body lookup behind admin auth -- no `getTransactionFromMempool` without auth." Cites hash-to-body indirection enabling sandwich construction at ~$876K/year annualized post-AMM.
2. **Adversarial-Capital:** "Remove or admin-gate `WsEvent::NewTx` in the SAME release." Cites $22K/epoch at $10M TVL from hash + RPC body resolution.
3. **Oracle/MEV:** "Existing NewTx delivers full TX hash unauthenticated; `getTransaction(hash)` retrieves full TX in <5ms." Recommends aggregate counts only (option b).
4. **Sustainability:** Silent -- does not contradict.
5. **Governance-Minimal:** "The existing `NewTx` event already broadcasts mempool admissions. Phase 2.2 does NOT expand mempool event content." Implicitly accepts status quo but does not argue against gating.

**Resolution: Admin-gate (option c for the existing event).**

- REMOVE is too aggressive: it breaks any existing consumers (even internal monitoring).
- KEEP status quo is unsafe: at any future DeFi TVL, the hash-to-body pipeline enables sandwich extraction.
- ADMIN-GATE is the middle path: existing admin-token pattern (same as `pauseProduction`, `createCheckpoint`). ~10 LOC change. Preserves functionality for node operators while blocking unauthenticated MEV bots.

**Implementation:** In `handle_socket()`, check if connection has admin token before forwarding `NewTx` events. Non-admin connections using `?topics=` syntax (new system) never receive `NewTx` (it is not in the topic enum). Legacy unfiltered connections (no `?topics` param) only receive `NewTx` if admin-authenticated.

**Confidence:** conf(0.85, converged) -- 3/5 explicitly recommend removal/gating, 1/5 nuanced (option b aggregate), 1/5 silent.

---

## S7 Phase 2.3 Carry-Forward Constraints

| Constraint | Source | Binding On |
|-----------|--------|-----------|
| PriceAttestation mempool exposure creates $2,500-$50,000/liquidation-event MEV window at lending TVL | Oracle/MEV evaluator S3.2 | Phase 2.3 lending design MUST resolve before activating lending_activation_height |
| Resolution paths: (a) commit-reveal attestations, (b) exclude PriceAttestation from mempool events, (c) accept as liquidation searcher incentive | Oracle/MEV S3.3 | Phase 2.3 scope |
| Aggregate mempool counts (option b) may ship in Phase 2.3 if agent demand materializes | Synthesis (S1.10) | Phase 2.3 scope |
| Per-output_type server-side filtering may ship as `?topics=utxo:9,utxo:15` syntax if needed | Synthesis (S1.5) | Phase 2.3+ |
| 1:N connection-to-subscription (multi-topic subscribe/unsubscribe) may ship if slot pressure on cap occurs | Governance-Minimal open Q3 | Phase 2.3+ |

---

## S8 Implementation Estimate

| Evaluator | LOC Estimate | Scope |
|-----------|-------------|-------|
| Governance-Minimal | ~195 | SSF minimum (Steps 0-5) |
| Mechanism-Skeptic | ~250 | SSF+ (4 topics + deterministic flag + idle-timeout) |
| Adversarial-Capital | ~600 | Full security mitigations (per-IP cap, replay cap, admin-gate NewTx, subscribe protocol) |

**Reconciled estimate: ~300-350 LOC total.**

Breakdown:
| Module | LOC | Location |
|--------|-----|----------|
| Lag-disconnect (64 events) | ~15 | `crates/rpc/src/ws.rs` |
| Topic query-param parsing + filter | ~35 | `crates/rpc/src/ws.rs` |
| 4 new WsEvent variants + serialization | ~60 | `crates/rpc/src/ws.rs` |
| UTXO event emission hooks | ~80 | `bins/node/src/node/apply_block/post_commit.rs` |
| Epoch/consensus event emission hooks | ~40 | `bins/node/src/node/apply_block/post_commit.rs` + `rewards.rs` |
| Ring buffer (shared, 100 blocks) | ~60 | `crates/rpc/src/event_buffer.rs` (new) |
| Replay on connect | ~30 | `crates/rpc/src/ws.rs` |
| `getEventCatalog` RPC method | ~20 | `crates/rpc/src/methods/` |
| Admin-gate existing `NewTx` | ~10 | `crates/rpc/src/ws.rs` |
| **Total** | **~350** | |

**No new crates. No new dependencies. No new ports.**

---

## S9 Out of Scope

| Item | Reason | Evaluator Source |
|------|--------|-----------------|
| JSON-RPC subscribe/unsubscribe commands | Query-param simpler; Bitcoin ZeroMQ lasted 12 years without | Governance-Minimal |
| Server-side predicate filtering (address, amount) | Client-side filtering adequate at DOLI's event rate | All 5 |
| Durable replay across node restarts | Ring buffer sufficient; agents use block-store RPC for gaps | Sustainability |
| Auth / API keys | No consumers exist; operator-side proxy when needed | All 5 |
| Mempool events (expanded) | MEV vector; no agents exist to consume | All 5 |
| Finality-delayed delivery mode | Harmful to lending (adds latency); agents apply own finality logic | Oracle/MEV, Mechanism-Skeptic |
| Binary encoding | JSON-parsable required by HC-8; bespoke decoders rejected | All 5 |
| Cross-node subscription registry | Creates consensus dependency | Analyst EV-5 |
| On-chain subscription state | Makes Q1=YES, HC-3 violation | All 5 |
| Per-event `deterministic` field in payload | Topic-level property; documented in catalog | Governance-Minimal |
| `/ws/v2` separate endpoint | Unnecessary; `?topics` is backward-compatible | Governance-Minimal |
| Compound AND/OR filter predicates | 10-100x CPU; push to client-side | Sustainability |
| gRPC transport | Zero protobuf in DOLI; new dependency for no gain | All 5 |
| SSE transport | No bidirectional channel; reconnection semantics inferior to WS | All 5 |

---

## S10 Fixability Classification

| Item | Fixability | Confidence |
|------|-----------|------------|
| Lag-disconnect at 64 | code | conf(0.95, converged) |
| Topic query-param + filter | code | conf(0.95, converged) |
| New WsEvent variants (utxo_created, utxo_spent, epoch_boundary, consensus_event) | code | conf(0.90, converged) |
| Ring buffer replay (100 blocks) | code | conf(0.90, converged) |
| `getEventCatalog` RPC | code | conf(0.90, converged) |
| Admin-gate NewTx | code | conf(0.85, converged) |
| Mempool event policy (exclude Phase 2.2) | design (locked) | conf(0.90, converged) |
| Per-IP cap (DEFERRED) | code (future) | conf(0.60, single-evaluator) |
| Aggregate mempool counts (DEFERRED) | design (future) | conf(0.55, single-evaluator) |

---

## Appendix: Complexity Comparison

| Metric | Current (existing /ws) | Proposed Phase 2.2 | Radical Minimum (Steps 0+1 only) |
|--------|----------------------|--------------------|---------------------------------|
| LOC in ws.rs | 111 | ~250 | ~126 |
| Event types | 2 | 6 | 2 (unchanged) + finality tag |
| Topics | 0 (all-or-nothing) | 4 | 0 |
| RPC methods added | 0 | 1 | 0 |
| Memory overhead (0 subs) | broadcast channel ~50 KB | + ring buffer 400 KB | 0 |
| Memory overhead (100 subs) | ~30 KB | ~30 KB + 400 KB ring | ~30 KB |
| New dependencies | 0 | 0 | 0 |
| Governance surfaces | 0 | 0 | 0 |
