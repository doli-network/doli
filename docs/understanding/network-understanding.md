# DOLI Network Layer -- Deep Understanding

> Scoped analysis of the DOLI blockchain's P2P networking, synchronization, block propagation, peer management, and consensus networking subsystems.
> Generated from code as the single source of truth. 2026-03-14.

## 1. Network Architecture Overview

### Tech Stack

| Layer | Technology | Version | Purpose |
|-------|-----------|---------|---------|
| Language | Rust | 1.85+ | Primary language |
| P2P Framework | libp2p | 0.53 | Networking abstractions |
| Transport | TCP + DNS | via libp2p | Connection transport |
| Encryption | Noise | via libp2p | Authenticated encryption |
| Multiplexing | Yamux | via libp2p | Stream multiplexing (1MB rx window) |
| Pub/Sub | GossipSub | via libp2p | Block/tx propagation |
| Discovery | Kademlia DHT | via libp2p | Peer discovery |
| Request/Response | libp2p req-resp | via libp2p | Status exchange, sync protocol |
| NAT Traversal | Relay + DCUtR + AutoNAT | via libp2p | Hole punching, relay |
| Serialization | bincode | 1.3 | Wire format for messages |
| Async Runtime | tokio | 1.35 | Event loop |

### Network Crate Structure (13 source files, ~9,400 lines)

```
crates/network/src/
  lib.rs            (102 lines)   -- Module exports, constants (DEFAULT_PORT=30300, PROTOCOL_ID="/doli/1.0.0")
  config.rs         (105 lines)   -- NetworkConfig: listen addr, bootstrap nodes, mesh params, NAT
  messages.rs       (76 lines)    -- Wire message types (NewBlock, GetBlocks, Status, etc.)
  transport.rs      (53 lines)    -- DNS/TCP + Noise + Yamux transport stack
  behaviour.rs      (91 lines)    -- DoliBehaviour: composed libp2p behaviour
  discovery.rs      (28 lines)    -- Kademlia DHT setup (/doli/kad/1.0.0, replication=20)
  gossip.rs         (627 lines)   -- GossipSub topics, tiered mesh, TX batching
  peer.rs           (363 lines)   -- PeerInfo, PeerState machine, PeerDiversity (eclipse prevention)
  peer_cache.rs     (171 lines)   -- Persistent peer cache (max 100, bincode, atomic save)
  scoring.rs        (388 lines)   -- Reputation system (-1000 to +1000, ban at -500)
  rate_limit.rs     (388 lines)   -- Token bucket rate limiter (per-peer + global)
  nat.rs            (145 lines)   -- NAT config: relay servers, autonat, dcutr
  service.rs        (1572 lines)  -- Core NetworkService: swarm event loop, command handling
  protocols/
    status.rs       (199 lines)   -- /doli/status/1.0.0 request-response protocol
    sync.rs         (230 lines)   -- /doli/sync/1.0.0 request-response protocol (64MB max)
  sync/
    mod.rs          (27 lines)    -- Public exports
    manager.rs      (4026 lines)  -- SyncManager: state machine, production gate, snap sync
    headers.rs      (191 lines)   -- HeaderDownloader: chain linkage validation
    bodies.rs       (393 lines)   -- BodyDownloader: parallel multi-peer download
    fork_sync.rs    (657 lines)   -- Binary search for common ancestor (O(log N))
    fork_recovery.rs (322 lines)  -- Backward parent chain walk for orphan blocks
    reorg.rs        (771 lines)   -- Weight-based fork choice rule
    equivocation.rs (392 lines)   -- Double-signing detection
```

### Integration Layer (in `bins/node/src/`)

```
main.rs   (1236 lines) -- CLI, run_node() bootstraps NetworkService + RPC
node.rs   (7214 lines) -- Node struct: handle_network_event(), run_event_loop(),
                           handle_sync_request(), run_periodic_tasks(),
                           handle_new_block(), execute_reorg(), apply_snap_snapshot()
```

### Boundary Design

The network crate is intentionally isolated from storage. It knows about `Block`, `BlockHeader`, `Transaction`, `Hash`, and `PublicKey` (from `doli-core` and `crypto`), but has zero dependency on `storage`. All block store lookups happen in `node.rs`, which acts as the integration layer. This means:

- `SyncManager` tracks peer heights and sync state but never reads the block store directly
- Fork sync probes are dispatched by `SyncManager` but resolved by `node.rs` comparing hashes from `BlockStore`
- Snap sync snapshots are created and applied by `node.rs` -- the network crate only shuttles bytes

This separation is clean and intentional: it prevents the network layer from accidentally coupling to storage internals.

---

## 2. P2P Networking Core

### Transport Stack

**File**: `crates/network/src/transport.rs`

The transport is built from three composed layers:

1. **DNS/TCP** -- resolves DNS names and establishes TCP connections
2. **Noise** -- XX handshake pattern for authenticated encryption using the node's Ed25519 keypair
3. **Yamux** -- stream multiplexing with 1MB receive window

When relay is enabled (for NAT traversal), the base TCP transport is composed with `relay::Transport` to allow relayed connections through seed nodes.

### Network Identity

The node's Ed25519 keypair is persisted to disk at `{data_dir_parent}/node_key`. Storing it one directory above the data directory ensures chain resets (which wipe the data dir) do not regenerate the peer ID. This prevents Kademlia mismatch storms where the DHT has stale entries for the old peer ID.

### Composed Behaviour

**File**: `crates/network/src/behaviour.rs`

`DoliBehaviour` uses libp2p's `#[derive(NetworkBehaviour)]` to compose 9 sub-protocols:

| Sub-behaviour | Purpose |
|---|---|
| `connection_limits` | Max 2 connections per peer, cap incoming+outgoing at `max_peers` |
| `gossipsub` | Pub/sub for blocks, txs, attestations, producer announcements |
| `kademlia` | DHT peer discovery (protocol: `/doli/kad/1.0.0`) |
| `identify` | Exchange peer metadata (agent, protocols, listen addrs) |
| `status` | Request-response: `/doli/status/1.0.0` (timeout 30s, max 64KB) |
| `sync` | Request-response: `/doli/sync/1.0.0` (timeout 120s, max 64MB) |
| `relay_client` | Client-side relay for NAT'd nodes |
| `relay_server` | Server-side relay (seed nodes only, via `--relay-server`) |
| `dcutr` | Direct Connection Upgrade through Relay (hole punching) |
| `autonat` | Automatic NAT detection |

### GossipSub Configuration

**File**: `crates/network/src/gossip.rs`

**8 gossip topics:**

| Topic | String | Purpose |
|---|---|---|
| BLOCKS | `/doli/blocks/1` | Full block propagation |
| TRANSACTIONS | `/doli/txs/1` | Transaction gossip |
| PRODUCERS | `/doli/producers/1` | Producer announcements (legacy) |
| VOTES | `/doli/votes/1` | Update system vote propagation |
| HEARTBEATS | `/doli/heartbeats/1` | Deprecated (deterministic scheduler) |
| T1_BLOCKS | `/doli/t1/blocks/1` | Tier-1 priority block relay |
| HEADERS | `/doli/headers/1` | Header pre-announcements |
| ATTESTATIONS | `/doli/attestations/1` | BLS attestation finality gadget |

Plus a regional topic generator: `region_topic(region_id)` producing `/doli/region/{id}/1`.

**Universal mesh parameters** (same for all networks and all node roles):

| mesh_n | mesh_n_low | mesh_n_high | gossip_lazy |
|--------|------------|-------------|-------------|
| 12 | 8 | 24 | 12 |

Networks with ≤24 peers are effectively full-mesh (all peers fit within `mesh_n_high`). At 1000+ nodes, propagation takes ~3-4 hops (~400ms) — well within 10s slot margin.

**TX Batching**: Outbound transactions are buffered and flushed every 100ms or when 50 txs accumulate. Batches use a `0xBA` prefix byte followed by 2-byte count then concatenated bincode-encoded transactions. This reduces gossip overhead significantly.

### Peer Discovery

**Kademlia DHT** (`/doli/kad/1.0.0`):
- Replication factor: 20
- Query timeout: 60s
- Mode: server
- Periodic bootstrap: every 60 seconds via the main event loop's DHT refresh timer
- Can be disabled with `--no-dht` for isolated test networks

**Persistent peer cache** (`peers.cache`):
- Stores up to 100 recently-seen peers
- Bincode serialized, atomic save via tmp file + rename
- Rejects loopback addresses
- On startup, cached peers are dialed before bootstrap nodes for faster reconnection

### Eclipse Attack Prevention

**File**: `crates/network/src/peer.rs`

`PeerDiversity` tracks IP prefix distribution:
- Max 3 peers per /24 IPv4 prefix
- Max 5 peers per /48 IPv6 prefix
- Max 5 peers per ASN (placeholder -- ASN lookup not yet implemented)

Connections that would violate diversity constraints are rejected at the evaluation stage.

### Peer Scoring

**File**: `crates/network/src/scoring.rs`

Scores are clamped to [-1000, +1000]:

| Event | Score Change |
|---|---|
| Valid block received | +10 |
| Valid transaction | +1 |
| Invalid block | -100 |
| Invalid transaction | -20 |
| Timeout (per occurrence) | -5 * count |
| Spam | -50 |
| Malformed message | -30 |
| Duplicate (per occurrence) | -5 |

**Thresholds**: Disconnect at -200, ban at -500. Ban duration: 1 hour. Score decay: 1 point per minute toward zero.

### Rate Limiting

**File**: `crates/network/src/rate_limit.rs`

Token bucket rate limiter with per-peer and global limits:

| Category | Per-Peer Limit | Global Limit |
|---|---|---|
| Blocks | 10/min | 100/min |
| Transactions | 50/sec | 200/sec |
| Requests | 20/sec | -- |
| Bandwidth | 1 MB/sec | -- |

LRU eviction at 1000 tracked peers. Cleanup runs every 5 minutes.

### NAT Traversal

**File**: `crates/network/src/nat.rs`

Three-layer NAT traversal:

1. **AutoNAT** -- detects whether the node is publicly reachable
2. **Relay** -- seed nodes (`seed1.doli.network`, `seed2.doli.network`) act as relay servers. NAT'd nodes connect through them.
3. **DCUtR** -- Direct Connection Upgrade through Relay. Once two peers have a relayed connection, they attempt a direct connection via hole punching.

Seed nodes run with `--relay-server` to enable `NatConfig::relay_server()` mode.

---

## 3. Message Flow and Gossip Architecture

### How a Block Propagates

1. **Producer** builds block, computes VDF, calls `network.broadcast_block(block)` which sends `NetworkCommand::BroadcastBlock`
2. **NetworkService** serializes the block via bincode and publishes to `/doli/blocks/1` GossipSub topic
3. **Receiving node's** `run_swarm()` matches `GossipsubEvent::Message` on BLOCKS_TOPIC
4. Rate limiter checks per-peer block rate (10/min) and global (100/min)
5. If allowed, emits `NetworkEvent::NewBlock(block, source_peer)` to the event channel
6. **Node's** `run_event_loop()` receives the event (biased select -- network events have priority)
7. `handle_network_event()` dispatches to `NetworkEvent::NewBlock` handler:
   - Ignores during snap sync
   - Slot sanity check: rejects blocks with slot too far in future (>MAX_FUTURE_SLOTS) or past (>MAX_PAST_SLOTS)
   - Updates network tip slot for production gating
   - Calls `handle_new_block(block, source_peer)`
8. `handle_new_block()`:
   - Deduplicates against block store
   - Checks for equivocation (double-signing)
   - If `prev_hash != best_hash`: caches in fork_block_cache, evaluates reorg via weight-based fork choice, or starts fork recovery
   - If builds on tip: validates producer eligibility, calls `apply_block(block, ValidationMode::Full)`

### The Biased Select -- Why It Matters

The main event loop uses `tokio::select! { biased; }` with network events as the first branch. Without bias, `tokio::select!` can randomly choose the production timer branch even when a `NewBlock` event is ready. This caused propagation race forks where nodes produced blocks on stale tips. The biased select ensures all pending blocks are processed before any production attempt.

Additionally, right before VDF computation (which takes ~550-700ms and blocks the event loop), the node drains ALL pending network events via `try_next_event()`. If a new block arrived during block construction, VDF is aborted.

### Transaction Flow

1. TX enters via RPC (`sendTransaction`) or gossip (`NetworkEvent::NewTransaction`)
2. Validated against UTXO set and mempool policy
3. If valid, added to mempool and broadcast via `NetworkCommand::BroadcastTransaction`
4. TX batching: transactions buffer for 100ms or until 50 accumulate, then flush as a batch with `0xBA` prefix

### Producer Discovery (Bootstrap Phase)

Three concurrent mechanisms ensure all producers discover each other:

1. **Status protocol**: On peer connection, nodes exchange `StatusRequest` which optionally includes `producer_pubkey`. Discovered pubkeys are added to `known_producers`.

2. **ProducerGSet CRDT**: Each producer creates a cryptographically signed `ProducerAnnouncement` containing their pubkey, network_id, genesis_hash, and a monotonically increasing sequence number. These merge using CRDT union semantics -- guaranteed convergence even with packet loss. Ghost producers are purged after 4 hours of inactivity.

3. **Adaptive gossip**: Full-sync (broadcast all announcements) for networks under 50 producers. Delta-sync (bloom filter) for larger networks -- peers send a bloom filter of known producers, recipients respond with delta of missing announcements.

### Attestation/Finality Flow

1. When a producer applies a block at the tip, it creates an `Attestation` signed with Ed25519 (and optionally BLS for aggregation)
2. Broadcast via `/doli/attestations/1` topic
3. Receiving nodes verify the signature, accumulate weight per block hash in `SyncManager`
4. The `MinuteAttestationTracker` records attester liveness per minute for on-chain bitfield embedding
5. Once a block reaches finality weight, it is flushed to the archiver

---

## 4. Sync State Machine

### States

**File**: `crates/network/src/sync/manager.rs` (4,026 lines)

```
SyncState:
  Idle                -- No active sync, waiting for peer height updates
  DownloadingHeaders  -- Fetching headers from best peer (header-first sync)
  DownloadingBodies   -- Fetching block bodies from multiple peers in parallel
  Processing          -- Applying downloaded blocks to chain state
  Synchronized        -- Caught up with network, normal operation
  SnapCollectingRoots -- Collecting state root votes from 5+ peers
  SnapDownloading     -- Downloading full state snapshot from one peer
  SnapReady           -- Snapshot received and verified, ready to apply
```

### Sync Decision Logic

`start_sync()` chooses between snap sync and header-first based on:

- **Snap sync** if: 3+ peers available, fewer than 3 snap attempts, AND (height gap > 1000 OR local height = 0)
- **Header-first** otherwise

Fresh nodes (height 0) wait up to 60 seconds for 5 peers before starting snap sync, ensuring a robust quorum for state root voting.

### Header-First Sync Flow

1. `start_sync()` sets state to `DownloadingHeaders`, picks target peer (highest height, majority hash preference via `consensus_target_hash()`)
2. `next_request()` generates `SyncRequest::GetHeaders { start_hash: local_tip_hash, max_count: 500 }`
3. `handle_response()` receives headers, validates chain linkage (each header's `prev_hash` matches previous header's hash), timestamps (not >1hr future)
4. When all headers collected (empty response), transitions to `DownloadingBodies`
5. `BodyDownloader` distributes hash requests across peers round-robin, max 128 per request, 8 concurrent in-flight
6. Bodies arrive out of order. `get_blocks_to_apply()` (called in `run_periodic_tasks()`) walks `pending_headers` in order and extracts matching bodies in correct chain sequence
7. Node applies blocks via `apply_block(block, ValidationMode::Light)` (light = skip VDF for sync'd blocks)
8. When all blocks applied, transitions to `Synchronized`

### Snap Sync Flow

1. Enter `SnapCollectingRoots` -- batch-send `GetStateRoot` to ALL peers simultaneously
2. Responses cluster in 1-2 seconds so peers report the same height/root
3. Wait for 5+ peers to respond; find quorum agreement on (block_hash, state_root) pair
4. Send `GetStateSnapshot` to one peer in the quorum
5. Receive snapshot: `chain_state + utxo_set + producer_set + state_root` (serialized bytes)
6. **Verification in node.rs**: recompute state root from the three byte blobs, verify it matches the quorum-agreed root
7. **Envelope check**: deserialized chain_state's (height, hash) must match the envelope
8. Replace all in-memory state atomically (chain_state, utxo_set, producer_set)
9. Persist via `StateDb::atomic_replace()` in a single RocksDB WriteBatch
10. Seed canonical index so post-snap blocks can chain properly
11. Set `store_floor` in SyncManager so fork sync won't binary-search below snap height

### Fork Sync (Binary Search for Common Ancestor)

**File**: `crates/network/src/sync/fork_sync.rs`

Triggers when the node is on a dead fork -- peers consistently return empty headers for our tip hash.

**Algorithm**: O(log N) binary search between local height and 0:
1. Probe midpoint: send `GetBlockByHeight { height: mid }` to peer
2. Compare returned hash with our block store's hash at that height
3. `Match` -> search upper half (ancestor is at or above mid)
4. `Mismatch` -> search lower half (ancestor is below mid)
5. `NotInStore` -> mark as store-limited (snap sync gap, not a real fork)
6. Converge when `low + 1 >= high`

**Safety features**:
- Max depth: 1000 blocks
- Timeout: 60 seconds
- Store floor awareness: if search hits the snap sync boundary, it is NOT interpreted as a deep fork
- Floor probe verification: explicitly probes the store floor to distinguish "no data below here" from "genuine chain divergence"

### Fork Recovery (Parent Chain Walk)

**File**: `crates/network/src/sync/fork_recovery.rs`

For orphan blocks that don't match any planned reorg path, walks backward through the parent chain:

1. Start from orphan block, send `GetBlockByHash { hash: block.prev_hash }` to the source peer
2. If parent is in our block store, recovery is complete -- we have a chain from our store to the orphan
3. If parent is unknown, cache it, request ITS parent
4. Continue until connection or max depth (1000) or timeout (120s)
5. Returns `CompletedRecovery` with blocks in forward order for reorg evaluation

30-second cooldown between recovery attempts to prevent hammering.

### Weight-Based Fork Choice

**File**: `crates/network/src/sync/reorg.rs` (771 lines)

The fork choice rule is weight-based (not longest-chain):

- Each block has a weight equal to its producer's `effective_weight(height)` (derived from bond amount)
- Chain weight = sum of block weights along the chain
- **Only reorg if the new chain is STRICTLY heavier** -- incumbent wins ties
- Tie-breaking by hash is deterministic but ties are resolved in favor of the existing chain
- Tracks recent 10,000 blocks with accumulated weight
- **Finality-aware**: rejects reorgs that would roll back past finalized blocks
- `record_fork_block()` vs `record_block_with_weight()` -- fork blocks do NOT update `current_chain_weight` to prevent weight inflation from orphan chains

### Equivocation Detection

**File**: `crates/network/src/sync/equivocation.rs`

Tracks `(producer_pubkey, slot)` -> first block header seen. If a second DIFFERENT block appears for the same (producer, slot) pair, it constitutes double-signing.

Creates `EquivocationProof` containing both full headers, enabling:
- VDF verification of both blocks (proves both are legitimately produced, not fabricated)
- Slash transaction submission by any node

Sliding window: 360 slots, max 10,000 tracked entries.

---

## 5. Production Gate (Defense-in-Depth)

`can_produce()` in `SyncManager` implements 11+ layers of production authorization checks, returning a `ProductionAuthorization` enum with 13 variants:

| Check | Blocks Production If | Authorization Variant |
|---|---|---|
| Sync state | Not `Synchronized` or `Idle` | `BlockedSyncing` |
| Post-rollback/recovery | `post_rollback` or `post_recovery_grace` flag set | `BlockedResync` |
| Behind peers (height) | Local height < best peer height - 2 | `BlockedBehindPeers` |
| Behind peers (slot) | Local slot < network tip slot - 2 | `BlockedBehindPeers` |
| Ahead of peers | Local height > best peer height + 5 | `BlockedAheadOfPeers` |
| Chain mismatch | `fork_mismatch_detected` flag | `BlockedChainMismatch` |
| Insufficient peers | peer_count < 2 AND solo production timer expired (50s) | `BlockedInsufficientPeers` |
| Sync failures | 3+ consecutive sync failures | `BlockedSyncFailures` |
| No gossip activity | No block received in 180 seconds | `BlockedNoGossipActivity` |
| Explicit block | `production_blocked` flag | `BlockedExplicit` |
| Bootstrap gate | peer_connected but no status response yet | `BlockedBootstrap` |
| Finality conflict | Block conflicts with finalized chain | `BlockedConflictsFinality` |
| Post-snap gate | Awaiting first canonical block after snap sync | `BlockedAwaitingCanonicalBlock` |

The node additionally checks:
- Version enforcement (approved update past grace period blocks outdated producers)
- Signed slots DB (prevents double-signing after restart)
- VDF pre-drain (processes all pending network events before VDF computation)

---

## 6. Node Integration

### Bootstrapping (main.rs -> node.rs)

1. `main.rs` parses CLI via clap, loads chainspec (embedded for mainnet/testnet, disk for devnet)
2. Creates `NodeConfig`, loads producer key + BLS key if `--producer`
3. Producer safety checks: lock file, signed slots DB, force-start confirmation
4. Loads ProducerSet (from disk, chainspec genesis producers, or hardcoded)
5. Creates `Node::new()`: opens BlockStore + StateDb (RocksDB), loads chain state + UTXO + producers
6. `node.run()`:
   - Calls `start_network()` which creates `NetworkConfig`, `NetworkService::new()`, dials cached peers + bootstrap
   - Self-registers as producer in GSet CRDT
   - Computes genesis VDF proof if in genesis phase
   - Starts RPC server
   - Enters `run_event_loop()`

### Main Event Loop (biased select!)

Three branches, priority-ordered:

1. **Network events** (highest priority): `network.next_event()` -> `handle_network_event()`
2. **Production timer**: 1s interval (200ms for devnet) -> `try_produce_block()` + `run_periodic_tasks()`
3. **Gossip timer**: Adaptive interval -> broadcast producer announcements, check divergence

### How node.rs Handles Sync Responses

When `NetworkEvent::SyncResponse` arrives:

1. Note the peer as active data sender
2. `SyncManager::handle_response()` processes the response and returns blocks ready for application
3. Blocks are routed through `handle_new_block()` (not directly applied) so fork/orphan detection works
4. If snap sync produced a ready snapshot, `apply_snap_snapshot()` is called
5. In `run_periodic_tasks()`: `get_blocks_to_apply()` extracts body-downloaded blocks in chain order, applies them via `apply_block(ValidationMode::Light)`

### Reorg Execution (node.rs:2260-2500)

Atomic chain reorganization:

1. Validate chain integrity: each new block must build on the previous
2. If rollback depth > 0:
   - Check for undo data availability (kept for UNDO_KEEP_DEPTH = 2000 blocks)
   - **Undo-based** (preferred): Apply undo data in reverse -- restore spent UTXOs, remove created UTXOs, deserialize producer snapshot
   - **Legacy fallback**: Rebuild from genesis if undo data missing
3. Persist common ancestor state via `StateDb::atomic_replace()`
4. Apply new fork blocks via `apply_block(block, ValidationMode::Light)`
5. State is consistent at every point -- if reorg apply fails partway, state reflects common ancestor + successfully applied blocks

### Periodic Tasks (node.rs:7211+)

`run_periodic_tasks()` runs every production timer tick:

1. Apply pending sync blocks in correct order (body downloader delivers out of order)
2. `SyncManager::cleanup()` -- handles timeouts, stale peers, stuck sync recovery
3. Archive catch-up after sync completion
4. Mempool expiration
5. Fork recovery polling -- check if parent chain reached block store
6. Fork exceeded max depth -> force recovery
7. Peer maintenance -- redial bootstrap when isolated (every slot_duration ~10s)
8. Stale chain detection (Ethereum-style) -- no block for 3 slots triggers diagnosis
9. Fork sync binary search driving -- compare probes, handle transitions
10. Genesis resync and deep fork escalation
11. Drive sync requests (snap batch + normal one-per-tick)
12. Periodic status refresh from peers (every 2s during bootstrap, 5s normal)
13. Port reachability warning for mainnet producers

---

## 7. RPC and External Interfaces

Network-related RPC methods (from `crates/rpc/src/methods.rs`):

| Method | Purpose |
|---|---|
| `getNetworkInfo` | Peer count, peer ID, sync status |
| `getPeerInfo` | List connected peers with heights, latency, uptime |
| `getNodeInfo` | Node version, network, chain info |
| `backfillFromPeer` | Start live backfill of missing blocks from another node's RPC endpoint |
| `backfillStatus` | Check backfill progress (running, imported, total, error) |
| `getStateRootDebug` | Get current state root for sync debugging |
| `getUtxoDiff` | Compare UTXO state between nodes for divergence diagnosis |
| `getChainStats` | Chain statistics including sync state |
| `verifyChainIntegrity` | Verify chain linkage and block store consistency |

**CRITICAL**: `backfillFromPeer` takes an **RPC URL** (e.g., `http://127.0.0.1:8500`), NOT a peer ID.

---

## 8. Sync Recovery Mechanisms (Escalation Ladder)

The system has a carefully ordered escalation ladder for sync problems:

| Level | Mechanism | Trigger | Cost |
|---|---|---|---|
| 1 | Header-first sync | Peer reports higher height | Low -- incremental |
| 2 | Fork recovery (parent walk) | Orphan block received | Low -- walks back from orphan |
| 3 | Stale chain detection | No block for 3 slots | Low -- re-requests status |
| 4 | Fork sync (binary search) | 3+ empty header responses | Medium -- O(log N) probes |
| 5 | Snap sync | Height gap > 1000, 3+ peers | Medium -- full state download |
| 6 | State reset + re-snap | Fork sync bottoms out or store-limited | High -- wipes UTXO/chain state |
| 7 | Genesis resync | SyncManager detects persistent rejection | High -- full state rebuild |

**Escape valves at every level:**
- Fork sync has a 60s timeout
- Fork recovery has 120s timeout and 30s cooldown
- Force recovery has exponential backoff: 60s -> 120s -> 300s -> 600s -> 960s
- Stuck-on-fork detection uses 120s time-based threshold (not counter-based, per recent commit `89530cf`)
- Body download stalls get 3 soft retries before hard reset
- Header blacklisting expires after 30 seconds

---

## 9. Security Model (Networking)

### Trust Boundaries

| Boundary | What crosses it | Verification |
|---|---|---|
| Gossip -> Node | Blocks, transactions | Full validation (signatures, UTXO, VDF) |
| Sync -> Node | Headers, bodies, state snapshots | Chain linkage, state root quorum, weight comparison |
| Status protocol | Height, hash, genesis hash | Network ID + genesis hash match |
| Peer connection | Peer identity | Noise handshake (Ed25519) |

### Attack Mitigations

| Attack | Mitigation | Location |
|---|---|---|
| Eclipse attack | IP prefix diversity (/24 IPv4, /48 IPv6) | `peer.rs` |
| Sybil flood | Connection limits (2 per peer, max_peers cap) | `behaviour.rs` |
| Block spam | Rate limiting (10/min per peer, 100/min global) | `rate_limit.rs` |
| TX spam | Rate limiting (50/sec per peer, 200/sec global) | `rate_limit.rs` |
| Invalid data | Peer scoring (ban at -500, 1hr) | `scoring.rs` |
| Double-signing | Equivocation detection -> slash transaction | `equivocation.rs` |
| Genesis time hijack | Slot sanity check (MAX_FUTURE_SLOTS, MAX_PAST_SLOTS) | `node.rs` |
| Solo production | 50s max solo timer, then blocks production | `manager.rs` |
| Stale fork production | 11+ layer production gate | `manager.rs:can_produce()` |
| Snap sync poisoning | State root quorum (5+ peers), recompute root from bytes | `node.rs` |
| Chainspec tampering | Mainnet/testnet use embedded (compiled-in) chainspec only | `main.rs` |
| Malformed messages | Bincode deserialization errors -> ignore + score penalty | `service.rs` |
| Network partition | Stale chain detection, bootstrap redial, DHT re-bootstrap | `node.rs` |

### Critical Invariants

1. **Gossip blocks get full validation** including VDF. Sync blocks get light validation (VDF skipped for already-trusted blocks).
2. **Snap sync state root is verified independently**: the node recomputes the root from the raw bytes, never trusting the peer's claimed root.
3. **The production gate prevents unsafe block production** even if the scheduler says it is our slot.
4. **Reorgs respect finality**: blocks past the finalization threshold cannot be rolled back.
5. **Weight-based fork choice prevents longer-but-lighter attacks**: a chain of low-bond producers cannot outweight a shorter chain of high-bond producers.

---

## 10. Complexity and Risk Map

### High-Complexity Areas

| Area | File | Lines | Why Complex | Risk Level |
|---|---|---|---|---|
| SyncManager | `sync/manager.rs` | 4,026 | 50+ fields, 8 states, 13 production authorization variants, snap sync + fork sync + fork recovery + cleanup all interleaved | **Critical** |
| Node integration | `node.rs` | 7,214 | God module -- handles ALL network events, sync, production, reorg, archiving, producer discovery | **Critical** |
| NetworkService | `service.rs` | 1,572 | Main swarm event loop with 20+ event handlers, command dispatch, genesis mismatch cooldowns | **High** |
| ReorgHandler | `sync/reorg.rs` | 771 | Weight calculations, finality awareness, chain tracking for 10K blocks | **High** |
| Fork sync | `sync/fork_sync.rs` | 657 | Binary search state machine with store-floor awareness, bottomed-out detection | **Medium** |
| Gossip config | `gossip.rs` | 627 | Tiered mesh reconfiguration, TX batching, topic management | **Medium** |

### Known Complexity Hotspots

1. **SyncManager has ~50 fields** including multiple boolean flags (`post_rollback`, `post_recovery_grace`, `fork_mismatch_detected`, `production_blocked`, etc.) that interact in subtle ways. The state machine is effectively a state + flags hybrid, making it hard to reason about which flag combinations are valid.

2. **Node.rs is a 7,214-line god module** acknowledged in MEMORY.md as an open item. It handles network event processing, block production, sync request serving, reorg execution, periodic maintenance, producer discovery, attestation tracking, archiving, and more. This is the single highest-risk file for introducing bugs.

3. **The sync recovery escalation ladder** has 7 levels with intricate interactions. For example, fork sync can trigger state reset, which triggers snap sync, which can fail and trigger another fork sync. The exponential backoff on forced recoveries (60s -> 960s) and post-recovery grace periods are the primary circuit breakers preventing cascade loops.

4. **The production gate's 11+ layers** are individually simple but their composition creates emergent behavior. For example: a node with 2 peers that receives no gossip for 180 seconds will be blocked by `BlockedNoGossipActivity` even if it is correctly synchronized, because the gossip timeout check doesn't distinguish "no blocks produced" from "not receiving blocks."

### Recent Bug Fixes (from git history)

| Commit | Fix | What it addressed |
|---|---|---|
| `89530cf` | Time-based stuck-on-fork detection (120s) replaces counter-based | Counter-based detection could fire or never fire depending on cleanup() call frequency |
| `3452436` | Remove aggressive small-gap empty-headers escalation | Was triggering unnecessary genesis resyncs for minor temporary header gaps |
| `5aaab34` | 3 escape valves for fork recovery loop | Fork recovery guards could prevent sync from ever recovering |
| `0c94cc4` | Activate fork_sync on repeated apply failures with small gap | Nodes with repeated block application failures were never entering fork_sync |
| `6a5dc73` | Raise empty-headers gap threshold from 2 to 50 | Too-sensitive threshold caused false fork detection |

### Technical Debt

1. **Heartbeat topic still subscribed** -- the deterministic scheduler model no longer uses heartbeats, but the topic exists and the handler is a no-op. Minor waste.
2. **Legacy `known_producers` list** maintained alongside `ProducerGSet` CRDT for backward compatibility. Divergence detection is implemented but the dual system adds complexity.
3. **ASN-based diversity** in `PeerDiversity` is a placeholder -- the actual ASN lookup is not implemented.
4. **`getBlockByHash` RPC returns height=0** because there is no reverse index from hash to height.

---

## 11. Key Files Map

| File | Lines | Why It Matters |
|---|---|---|
| `crates/network/src/sync/manager.rs` | 4,026 | The brain of synchronization -- state machine, production gate, snap sync orchestration, all recovery logic |
| `bins/node/src/node.rs` | 7,214 | Integration point -- consumes all NetworkEvents, drives sync, executes reorgs, produces blocks |
| `crates/network/src/service.rs` | 1,572 | libp2p swarm event loop -- translates raw network events into clean NetworkEvent channel |
| `crates/network/src/gossip.rs` | 627 | GossipSub configuration -- topics, tiered mesh, TX batching |
| `crates/network/src/sync/reorg.rs` | 771 | Fork choice rule -- weight-based chain selection with finality awareness |
| `crates/network/src/sync/fork_sync.rs` | 657 | O(log N) common ancestor search -- critical for fork recovery |
| `crates/network/src/sync/equivocation.rs` | 392 | Double-signing detection -- creates slash evidence |
| `crates/network/src/scoring.rs` | 388 | Peer reputation -- bans misbehaving peers |
| `crates/network/src/rate_limit.rs` | 388 | DoS protection -- token bucket per-peer and global |
| `crates/network/src/peer.rs` | 363 | Peer state machine + eclipse attack prevention |
| `crates/network/src/sync/fork_recovery.rs` | 322 | Parent chain walk for orphan blocks |
| `crates/network/src/sync/bodies.rs` | 393 | Parallel body download with round-robin peer assignment |
| `crates/network/src/protocols/sync.rs` | 230 | Sync protocol definition -- all request/response types |
| `crates/network/src/behaviour.rs` | 91 | Composed libp2p behaviour -- 9 sub-protocols in one struct |
| `bins/node/src/main.rs` | 1,236 | CLI + bootstrapping -- chainspec security, network config, node creation |

---

## 12. Onboarding Guide for the Network Layer

If a new engineer needed to understand DOLI's network layer, read in this order:

1. **`crates/network/src/lib.rs`** -- Constants, module structure, public exports. Gives you the vocabulary.
2. **`crates/network/src/behaviour.rs`** -- See all 9 sub-protocols composed together.
3. **`crates/network/src/gossip.rs`** -- The 8 topics, tiered mesh, TX batching. How data moves.
4. **`crates/network/src/protocols/status.rs` and `sync.rs`** -- The two request-response protocols.
5. **`crates/network/src/service.rs`** -- The swarm event loop. Trace `handle_swarm_event()` and `handle_behaviour_event()`.
6. **`crates/network/src/sync/manager.rs`** -- Start with `SyncState` enum, then `SyncManager` fields, then `can_produce()`, then `start_sync()`, then `handle_response()`, then `cleanup()`.
7. **`bins/node/src/node.rs` lines 1420-2260** -- Main event loop and `handle_network_event()`. Then `handle_new_block()` and `execute_reorg()`.
8. **`crates/network/src/sync/reorg.rs`** -- Weight-based fork choice.
9. **`crates/network/src/sync/fork_sync.rs`** and **`fork_recovery.rs`** -- The two fork recovery strategies.
10. **`crates/network/src/scoring.rs`**, **`rate_limit.rs`**, **`peer.rs`** -- Defense mechanisms.

---

## Analysis Metadata

- **Layers completed**: 7/7
- **Files read in full**: All 22 source files in `crates/network/src/` plus `main.rs`
- **Files read in sections**: `node.rs` (key network integration sections), `rpc/methods.rs` (network methods)
- **Confidence level**: High -- every source file in the network crate was read
- **Date**: 2026-03-14
