# Tier 2 Scaling Improvement: ~5,000 Nodes

## Status: IMPLEMENTED

## Context

Tier 1 (bb96f3d) delivered 3 structural fixes:
1. Production escape hatch — guarantees production under event flooding
2. Sync serving rate limit — caps aggregate sync responses (8/interval)
3. RecoveryPhase enum — replaces 9 boolean flags with 5-variant enum

These get DOLI from "breaks at 112" to "works at ~500." Tier 2 targets ~5,000 nodes
by addressing the next tier of bottlenecks: single-task event processing, O(N^2)
gossip amplification, and insufficient connection management.

## Current Behavior (to preserve)

- Biased select! ensures blocks are processed before production (fork prevention)
- Production escape hatch fires when production_interval elapses under event load
- GSet CRDT with cryptographic announcements for producer discovery
- Delta gossip (bloom filter) available for networks > 50 producers
- Connection limits via libp2p: max_established_incoming + max_established_outgoing
- Per-peer rate limiting in swarm loop
- Event channel: mpsc::channel(1024) for both events and commands

## Bottleneck Analysis at 5,000 Nodes

### B1: Single-task event processing
All 15 NetworkEvent types are handled sequentially in `handle_network_event()` on the
main event loop task. At 5,000 nodes:
- ~5,000 attestation messages/minute (each with signature verification)
- ~5,000 gossip merge operations/round (each with N signature verifications)
- Block processing (apply_block ~2-5ms) blocks ALL event processing
- Result: event queue fills, backpressure blocks swarm loop, gossip stalls

### B2: Full-state gossip below 50 producers
Event loop line 147: `if use_delta && producer_count > 50` — uses full-state broadcast
for < 50 producers. At 5,000 nodes with 13 producers:
- Each node broadcasts ALL 13 announcements to ALL mesh peers every gossip round
- 5,000 nodes × mesh_n × 13 = ~650,000+ messages per round
- Each message triggers merge_one() with 13 signature verifications
- DIAG-ARCH-003: already measured at 100+ DUPLICATE events/node/round at 112 nodes

### B3: Undersized channel buffers
mpsc::channel(1024) fills instantly at 5,000 nodes. When full:
- Swarm loop blocks on event_tx.send() — stops processing libp2p events
- Commands from Node to swarm queue up — blocks status requests, sync, gossip
- Cascade: blocked swarm → peers think we're dead → disconnect → reconnect storm

### B4: No inbound connection prioritization
max_peers limits total connections but doesn't distinguish:
- Producers (critical — need them for block propagation)
- Sync peers (temporary — should be evictable)
- Random peers (lowest priority)
At 5,000 nodes, connection slots fill with random peers, crowding out producers.

---

## Requirements

### SCALE-T2-001: Parallel Event Handlers (Must)

**Current**: All 15 event types processed sequentially on one task.
**Target**: Categorize events into pipelines, each on its own tokio::spawn task.

**Design — Event Router Pattern**:
```
NetworkService → event_rx (main channel, 1024→8192)
    ↓
Event Router (main task — lightweight dispatch only)
    ├── [inline] Block Pipeline: NewBlock, SyncRequest, SyncResponse
    ├── gossip_tx → Gossip Task: ProducerAnnouncementsReceived, ProducerDigestReceived, ProducersAnnounced
    ├── attestation_tx → Attestation Task: NewAttestation
    └── [inline] Light events: PeerConnected/Disconnected, PeerStatus, StatusRequest, NewTransaction, NewVote, NewHeader, mismatches
```

**Event categorization rationale**:
- **Block Pipeline (inline, sequential)**: NewBlock, SyncRequest, SyncResponse
  - These mutate chain_state/utxo_set/producer_set atomically via apply_block()
  - MUST remain sequential — reordering causes state divergence
  - SyncRequest stays inline because it reads block_store (fast) and the rate limit keeps volume low
- **Gossip Task (spawned)**: ProducerAnnouncementsReceived, ProducerDigestReceived, ProducersAnnounced
  - CPU-intensive: each announcement requires signature verification (ed25519)
  - Writes only to producer_gset (Arc<RwLock>) and known_producers (Arc<RwLock>)
  - No ordering dependency with block processing
  - At 5,000 nodes: highest event volume, most CPU per event
- **Attestation Task (spawned)**: NewAttestation
  - Signature verification per attestation
  - Writes to sync_manager attestation_weight + minute_tracker
  - Independent of block processing order
- **Light events (inline)**: PeerConnected, PeerDisconnected, PeerStatus, StatusRequest, NewTransaction, NewVote, NewHeader, mismatches
  - Low CPU, low volume, fast to process
  - Some update sync_manager peer state (needed for production decisions)

**Shared state (already Arc<RwLock<>>)**:
- producer_gset, adaptive_gossip, known_producers, sync_manager, chain_state,
  mempool, equivocation_detector, fork_block_cache, cached_state_root — all ready

**What needs wrapping**: The spawned tasks need clones of:
- `network.command_sender()` (for sending responses) — already Clone via mpsc::Sender
- `config`, `params` — read-only, can be Arc'd
- `producer_key` — read-only after init, can be Arc'd

**Acceptance Criteria**:
- [ ] Main event loop dispatches events to typed channels without blocking
- [ ] Gossip events processed on dedicated task with no impact on block pipeline
- [ ] Attestation events processed on dedicated task
- [ ] Production timer still fires within interval under 5,000-node event load
- [ ] All 1,557 existing tests pass
- [ ] No behavior change — same events, same processing, different task scheduling

---

### SCALE-T2-002: Always-On Delta Gossip (Must)

**Current**: Full-state gossip for < 50 producers (event_loop.rs:147).
**Target**: Always use delta gossip (bloom filter), eliminate full-state broadcast.

**Changes**:
1. Remove the `producer_count > 50` guard in event_loop.rs
2. Lower DELTA_SYNC_THRESHOLD from 20 to 0 (or remove the flag entirely)
3. Keep full-state as a rare fallback (e.g., first 30 seconds after startup when
   bloom filter would be empty)

**Message reduction at 5,000 nodes with 13 producers**:
- Full-state: 5,000 × mesh_n × 13 announcements = ~650K messages/round
- Delta: 5,000 × mesh_n × 1 bloom filter (64 bytes) = ~65K messages/round
  - Only missing announcements sent back (converges to 0 quickly)
- **~10x reduction in gossip message volume**

**Acceptance Criteria**:
- [ ] Delta gossip used regardless of producer count (except first 30s startup)
- [ ] Bloom filter broadcasts replace full-state broadcasts
- [ ] New producer discovery still converges within 3 gossip rounds
- [ ] Existing adaptive backoff still works (interval scaling)
- [ ] All GSet tests pass

---

### SCALE-T2-003: Scaled Channel Buffers (Must)

**Current**: mpsc::channel(1024) for events and commands.
**Target**: Scale based on max_peers configuration.

**Design**:
- Event channel: `max(4096, max_peers * 16)` — at max_peers=100, that's 4096
- Command channel: `max(2048, max_peers * 4)` — commands are lower volume
- Gossip handler channel: `max(2048, max_peers * 8)` — high volume
- Attestation handler channel: `max(1024, max_peers * 4)`

**Acceptance Criteria**:
- [ ] Channel buffers scale with max_peers
- [ ] No backpressure-induced swarm stalls at 5,000 connected peers
- [ ] Memory overhead is bounded (max 32K entries per channel)

---

### SCALE-T2-004: Connection Prioritization (Should)

**Current**: libp2p connection_limits with flat max_peers for incoming + outgoing.
**Target**: Reserve connection slots for producers.

**Design**:
- Reserve 20% of max_peers slots for known producer peers
- When connection limit is hit, evict lowest-scoring non-producer peer
- Scoring: producers > peers with recent blocks > peers with recent status > new peers

**Changes**: In swarm_events.rs connection handling:
- Before accepting new connection, check if slots are available
- If full, check if new peer is a producer (from producer_gset)
- If yes, evict lowest-priority non-producer connection

**Acceptance Criteria**:
- [ ] Producer peers always maintain connections when slots are full
- [ ] Non-producer peers can still connect when slots are available
- [ ] Connection churn doesn't cause reconnection storms

---

### SCALE-T2-005: Gossip Signature Verification Cache (Should)

**Current**: Every gossip announcement triggers full ed25519 signature verification
in merge_one(), even duplicates that will be rejected by sequence check.
**Target**: Cache recently verified (pubkey, sequence, signature) tuples.

**Design**:
- LRU cache of 1,000 recently verified announcement hashes
- Before sig verify in merge_one(), check cache
- If cached and sequence <= known: skip verify, return Duplicate immediately
- Evict entries older than gossip interval

**Message savings at 5,000 nodes**:
- 99%+ of gossip messages are duplicates (DIAG-ARCH-003)
- Each dup currently costs: deserialize + sig verify (~50μs ed25519)
- With cache: deserialize + hash lookup (~1μs)
- **~50x CPU reduction for duplicate gossip processing**

**Acceptance Criteria**:
- [ ] Duplicate announcements skip signature verification
- [ ] Cache is bounded (LRU, max 1,000 entries)
- [ ] Cache doesn't affect security (new sequences always verified)
- [ ] All GSet merge tests pass

---

### SCALE-T2-006: Activate Tiered GossipSub at Runtime (Must)

**Current**: Tiered gossipsub infrastructure exists in `gossip/config.rs` (~340 lines)
but is never activated. All nodes run Tier 0 (legacy) regardless of role. The code
includes `new_gossipsub_for_tier()`, `subscribe_to_topics_for_tier()`,
`reconfigure_topics_for_tier()`, `topics_for_tier()`, `region_topic()`, and
`compute_dynamic_mesh()` — all tested but dormant.

**Target**: Activate tier computation at epoch boundaries and call `reconfigure_tier()`.

**Design**:
1. At each epoch boundary (in `periodic.rs` or `apply_block.rs`), compute node tier:
   - Tier 1: node is a registered producer with active bond
   - Tier 2: node is a full node (not producing but validating all blocks)
   - Tier 3: future — light clients (header-only, not yet supported)
2. If tier changed from last epoch, call `network.reconfigure_tier(new_tier, region)`
3. This triggers `reconfigure_topics_for_tier()` which:
   - Unsubscribes from topics not needed for new tier (except protected: blocks, txs)
   - Subscribes to tier-specific topics (TIER1_BLOCKS_TOPIC for producers)
4. Tier 1 producers get mesh_n=20 (dense mesh for instant block propagation)
5. Tier 2 full nodes get mesh_n=8 (moderate, within regional shards)

**What's already coded** (verified in `gossip/config.rs`):
- `new_gossipsub_for_tier(tier)` — creates gossipsub with tier-specific mesh params
- `subscribe_to_topics_for_tier(gs, tier, region)` — subscribes to tier-appropriate topics
- `reconfigure_topics_for_tier(gs, tier, region)` — live migration with safety guards
- `topics_for_tier(tier, region)` — single source of truth for tier→topic mapping
- Peer scoring: producers get `first_message_deliveries_weight=10.0` on BLOCKS_TOPIC
- Protected topics: BLOCKS and TRANSACTIONS never unsubscribed regardless of tier

**ALREADY ACTIVE AT RUNTIME** (verified during implementation):
- `recompute_tier()` runs in `apply_block/post_commit.rs` after every block
- Calls `network.reconfigure_tier()` on epoch boundary when tier changes
- `our_tier` and `last_tier_epoch` already tracked on Node struct
- No code changes needed — this was already fully wired up

**Acceptance Criteria**:
- [ ] Node computes tier at each epoch boundary
- [ ] Producers get Tier 1 subscriptions (TIER1_BLOCKS_TOPIC, mesh_n=20)
- [ ] Non-producers get Tier 2 subscriptions (mesh_n=8)
- [ ] Tier changes are applied via reconfigure_topics_for_tier()
- [ ] Protected topics (blocks, txs) never unsubscribed
- [ ] All existing gossip tests pass

---

## Explicit Boundaries (what will NOT change)

1. **Block processing order** — blocks are still applied sequentially on one task
2. **Consensus rules** — no changes to validation, scheduling, or rewards
3. **On-disk format** — no storage changes
4. **RPC API** — no changes to RPC methods
5. **Network protocol** — gossipsub topics, request-response protocols unchanged
6. **P2P wire format** — no serialization changes
7. **Biased select! for blocks** — blocks still have priority over production
8. **Production escape hatch** — still fires when interval elapses

## Impact Analysis

| Module | Impact | Risk |
|--------|--------|------|
| `bins/node/src/node/event_loop.rs` | Major refactor (event routing) | HIGH — core event dispatch |
| `bins/node/src/node/mod.rs` | Minor (new channel fields) | LOW |
| `crates/network/src/service/mod.rs` | Minor (channel buffer sizes) | LOW |
| `crates/core/src/discovery/gossip.rs` | Minor (threshold change) | LOW |
| `crates/core/src/discovery/gset/merge.rs` | Moderate (sig cache) | MEDIUM |
| `crates/network/src/service/swarm_events.rs` | Moderate (connection priority) | MEDIUM |
| `bins/node/src/node/periodic.rs` | Minor (tier computation) | LOW |
| `crates/network/src/gossip/config.rs` | None (already coded) | NONE |

## Priority Order

1. SCALE-T2-001 (Parallel handlers) — highest impact, removes main bottleneck
2. SCALE-T2-002 (Always delta gossip) — 10x message reduction, simple change
3. SCALE-T2-003 (Channel buffers) — prevents backpressure cascade
4. SCALE-T2-005 (Sig cache) — 50x CPU savings on duplicates
5. SCALE-T2-006 (Tiered gossipsub) — activates existing infrastructure
6. SCALE-T2-004 (Connection priority) — resilience, deferred to Tier 2.5

## Risk Assessment

- **Deadlock risk**: Spawned tasks acquiring RwLock on shared state. Mitigated by:
  existing code already uses Arc<RwLock<>> everywhere, and spawned tasks hold locks
  briefly (no cross-lock dependencies between gossip and block pipelines).
- **Race condition**: Gossip task updates known_producers while block pipeline reads it.
  Safe because both use RwLock and known_producers changes don't affect block validation.
- **Event ordering**: Attestation processed on different task from blocks. Safe because
  attestation_weight is additive and order-independent.
- **Tier reconfiguration**: Unsubscribing from topics mid-operation. Mitigated by
  PROTECTED_TOPICS guard — blocks and transactions never unsubscribed.
