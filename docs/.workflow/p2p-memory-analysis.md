# P2P Network Advisory: Non-Linear Memory Scaling at 100+ Nodes (INC-I-014)

## Task Type
Performance Analysis + Architecture Assessment

## Protocol Stack Map
| Layer | Implementation | Library/Custom | Status |
|-------|---------------|----------------|--------|
| Transport | TCP via tokio + DNS resolution | libp2p | OK |
| Security | Noise protocol (ed25519) | libp2p | OK |
| Multiplexing | Yamux (512KB receive window) | libp2p | CONCERN |
| Discovery | Kademlia DHT (replication=20, MemoryStore) | libp2p | CONCERN |
| Routing | GossipSub v1.1 (mesh_n=12, 7 topics) | libp2p | CONCERN |
| NAT Traversal | AutoNAT + DCUtR + Relay client/server | libp2p | OK |
| Peer Scoring | GossipSub peer scoring + custom PeerScorer | hybrid | OK |
| Application | Status/Sync/TxFetch req-resp + gossip topics | custom | CONCERN |
| Connection Limits | max_established_per_peer=2, in/out=conn_limit | libp2p | CONCERN |

---

## 1. Memory Model: Total Memory as f(N, max_peers, yamux_window, ...)

### Definitions

- **N** = total nodes on the machine
- **MP** = max_peers per node (testnet=25, devnet=150)
- **BS** = bootstrap_slots (default=10)
- **YW** = yamux receive window (512KB default)
- **CL** = conn_limit = MP * 2 + BS (testnet: 25*2+10 = 60)

### Per-Node Fixed Memory (baseline, no peers)

| Component | Estimated Size | Source |
|-----------|---------------|--------|
| Node struct (all Arc<RwLock<...>> fields) | ~2 KB | `bins/node/src/node/mod.rs` |
| BlockStore (RocksDB) | ~30-50 MB | Mapped files |
| StateDb (RocksDB) | ~20-40 MB | Mapped files |
| UtxoSet (in-memory) | ~1 MB per 10K UTXOs | At genesis: ~100KB |
| ProducerSet (in-memory) | ~50 KB | Per producer: ~500 bytes |
| Mempool | ~1 MB (empty) | Bounded by policy |
| SyncManager | ~10 KB | `crates/network/src/sync/manager/types.rs` |
| Tokio runtime overhead | ~2-5 MB | Per-node runtime |
| libp2p Swarm (no connections) | ~5 MB | Behaviour tree, Kademlia store, gossipsub state |
| **Subtotal (no connections)** | **~60-100 MB** | |

### Per-Connection Memory

Each established TCP connection carries:

| Component | Memory Per Connection | Source |
|-----------|----------------------|--------|
| Yamux session state | ~1 KB | Session metadata, stream tracking |
| Yamux receive window buffer | **512 KB** (configured) | `crates/network/src/transport.rs:17` |
| Yamux send buffer | ~256 KB (kernel) | macOS TCP socket buffer |
| TCP socket kernel buffers | ~128 KB (recv) + 128 KB (send) | macOS default: `net.inet.tcp.recvspace=131072` |
| Noise session state | ~1 KB | Post-handshake |
| libp2p connection handler | ~2 KB | Per-connection handler stack |
| Per-stream overhead (gossipsub, identify, kad, status, sync, txfetch) | ~6 streams x 8 KB = **48 KB** | Active yamux streams |
| File descriptor kernel overhead | ~10 KB | macOS kernel state per fd |
| **Subtotal per connection** | **~1.1 MB** | |

### Per-Peer Application Memory

For each peer in the peer table (separate from raw connections):

| Component | Memory Per Peer | Source |
|-----------|----------------|--------|
| PeerInfo struct | ~300 bytes | `crates/network/src/peer.rs:15-40` |
| PeerSyncStatus in SyncManager | ~200 bytes | `crates/network/src/sync/manager/types.rs:255-268` |
| RateLimiter PeerLimits | ~200 bytes (4 token buckets) | `crates/network/src/rate_limit.rs:71-113` |
| Gossipsub per-peer scoring state | ~500 bytes (3 scored topics) | `crates/network/src/gossip/config.rs:100-152` |
| Gossipsub mesh/fanout/gossip tracking | ~200 bytes per topic x 7 topics = **1.4 KB** | 7 subscribed topics |
| Kademlia routing table entry | ~200 bytes | Per-bucket entry |
| **Subtotal per peer** | **~3 KB** | |

### Connection Topology on Localhost (N nodes, max_peers=MP)

**Critical insight**: On localhost, all N nodes share the same IP (127.0.0.1). There is no IP diversity filtering in effect (peer diversity is disabled for loopback). Every node can connect to every other node.

**Connection count formula (localhost)**:

Each node attempts to maintain MP connections. With N nodes available, each node connects to min(N-1, MP) peers. But connections are bidirectional and counted at both ends.

- When **N <= MP+1**: near-full mesh. Total connections = N*(N-1)/2. Each node has ~(N-1) connections.
- When **N > MP+1**: Each node has ~MP connections. Total connections ≈ N*MP/2 (each connection counted once).

However, the **conn_limit** (not max_peers) governs the actual connection count:
- `conn_limit = MP * 2 + BS` (per-direction: incoming AND outgoing)
- With `max_established_per_peer = 2`, each peer-pair can have up to 2 connections

**Testnet configuration (max_peers=25, bootstrap_slots=10):**

| N nodes | Connections per node | Total connections across system | Total connection memory |
|---------|---------------------|-------------------------------|------------------------|
| 50 | ~25 peers x ~1.5 conns = 37 | ~925 | 925 x 2 sides x 1.1MB = ~2 GB |
| 103 | ~25 peers x ~1.5 conns = 37 | ~1,905 | 1,905 x 2 x 1.1MB = ~4.2 GB |
| 150 | ~25 peers x ~1.5 conns = 37 | ~2,775 | 2,775 x 2 x 1.1MB = ~6.1 GB |

But this is only connection memory. The total per-node is:

```
Per_Node_Total = Baseline(~80MB) + Connections(~37 x 1.1MB) + Peers(25 x 3KB)
               = 80 + 41 + 0.075 = ~121 MB per node
```

```
System_Total(N) = N * Per_Node_Total = N * 121 MB
```

At N=103: 103 * 121 MB = **~12.5 GB** (linear component only).

This does NOT explain non-linear explosion. The non-linearity must come from:

---

## 2. Non-Linearity Sources: What Causes the Cliff

### Source A: Gossipsub Message Amplification (O(N * mesh_n * topics) per message)

**This is the primary suspect for non-linear scaling.**

Configuration: mesh_n=12, mesh_n_high=24, gossip_lazy=12, 7 subscribed topics.

When a block is produced:
1. The producer publishes to BLOCKS_TOPIC (1 gossipsub publish)
2. Gossipsub forwards to mesh_n=12 mesh peers
3. Each mesh peer forwards to their mesh_n=12 peers
4. Gossipsub also sends IHAVE to gossip_lazy=12 non-mesh peers
5. Non-mesh peers request via IWANT if not seen

At 7 topics with heartbeat_interval=1s, each node processes:
- Per heartbeat: 7 topics * 12 mesh peers = 84 GRAFT/PRUNE/IHAVE messages
- Per block (every 10s): the full block (~1-5 KB) delivered to mesh_n peers, plus IHAVE/IWANT to gossip_lazy peers

**The non-linear component**: gossipsub's message cache.

```
gossip config (config.rs:84-89):
  history_length = 5
  history_gossip = 3
  duplicate_cache_time = 60s
  max_transmit_size = 1MB
```

The **duplicate message cache** stores message IDs for 60 seconds. At 7 topics:
- Every 10s: 1 block, N-1 tx-announce batches, producer digests, attestations, headers
- Messages per 60s window: 6 blocks + ~100 txs + ~N producer gossip rounds + 6 attestations + 6 headers + votes
- Per-node seen-message cache: **O(messages_per_minute * N_sources)**

At 103 nodes producing gossip on 7 topics, the deduplication cache per node grows as O(N * topics * msgs_per_slot). The cache stores 20-byte message IDs, so:
- ~103 sources * 7 topics * 6 slots/minute * 20 bytes = ~87 KB per node (manageable)

That alone is not the explosion. The real issue is...

### Source B: Gossipsub IHAVE/IWANT Storm with gossip_lazy=12 and 103+ Peers

`gossip_factor = 0.25` means each heartbeat (1s), gossipsub sends IHAVE messages to 25% of non-mesh peers.

At N=103 and max_peers=25:
- Mesh peers per topic: 12
- Non-mesh but subscribed peers per topic: 25 - 12 = 13
- IHAVE targets: 13 * 0.25 = ~3 peers per topic per heartbeat
- Total IHAVE outbound: 7 topics * 3 = 21 IHAVE messages per second per node

At the **system level**: 103 nodes * 21 = 2,163 IHAVE messages/second across the system.

Each IHAVE message is small, but **each triggers a yamux stream negotiation**. With 103 nodes, this creates a sustained 2K+ stream-open-close per second across the system. Each stream open allocates yamux state and a receive window.

### Source C: Connection Churn and Eviction Thrashing (THE CLIFF MECHANISM)

**This is the most likely mechanism for the cliff at N=103.**

File: `crates/network/src/service/swarm_events.rs:77-148`

When N > max_peers (N=103 > MP=25), the peer table is always full. Every new incoming connection triggers:
1. Peer table full check (line 95)
2. Find lowest-scored evictable peer (lines 102-109)
3. Evict: remove from peers, disconnect, send PeerDisconnected event (lines 117-121)
4. Add evicted peer to cooldown map (line 124)
5. Accept new peer (lines 127-131)

The **eviction cooldown is only 30 seconds** (line 83). After 30 seconds, the evicted peer reconnects via Kademlia DHT discovery (60-second refresh cycle) or bootstrap redial. This creates a **steady-state churn cycle**:

```
Excess peers = N - MP = 103 - 25 = 78 peers in rotation
Churn rate = 78 peers / 30s cooldown = ~2.6 evictions per second per node
System churn = 103 * 2.6 = ~268 connection-open-close cycles per second
```

Each churn cycle:
- Opens a TCP connection (kernel buffers: ~256KB)
- Completes Noise handshake (CPU + memory for session state)
- Opens yamux session (512KB receive window allocated)
- Runs Identify protocol exchange
- Triggers gossipsub subscription messages
- Then gets evicted and all of this is torn down

**At 268 cycles/second**, with each cycle briefly allocating ~1.1MB, the **transient memory peak** is:

```
Transient = 268 cycles/s * ~0.5s lifecycle * 1.1MB = ~147 MB transient churn memory
```

This is significant but not catastrophic alone. However, it interacts with...

### Source D: Kademlia MemoryStore Growth (O(N) entries, refreshed every 60s)

File: `crates/network/src/discovery.rs:13-19`

The Kademlia MemoryStore has **no configured size limit**. Default libp2p MemoryStore stores up to 1024 records per peer with a max_records default. But the **replication_factor is set to 20** (line 19).

With 103 nodes, each node's Kademlia routing table accumulates all 102 other peer IDs and their addresses. Every 60s DHT bootstrap (swarm_loop.rs:149-159) queries the DHT and refreshes. With replication=20, each node tries to replicate 20 copies of each record.

The Kademlia store itself is modest (~100 bytes per entry * 102 entries = ~10KB), but the **DHT query traffic** during bootstrap generates a burst of streams.

### Source E: Channel Buffer Sizing Formula Creates Large Allocations

File: `crates/network/src/service/mod.rs:76-78`

```rust
let event_buf = (config.max_peers * 16).clamp(4096, 32768);
let command_buf = (config.max_peers * 4).clamp(2048, 32768);
```

With max_peers=25: event_buf=4096, command_buf=2048.

Each `NetworkEvent` contains large enum variants:
- `NewBlock(Block, PeerId)`: a full Block can be ~5-50KB
- `SyncResponse { response: SyncResponse }`: headers can be ~100KB per batch
- `ProducerAnnouncementsReceived(Vec<ProducerAnnouncement>)`: scales with N producers

With event_buf=4096 slots, if the channel fills during a connection storm:
- Worst case: 4096 * 50KB (blocks) = **200MB per channel**
- Typical case: 4096 * 1KB (mixed events) = **4MB per channel**

At 103 nodes: 103 * 2 channels * 4MB = ~824 MB in channel buffers (typical).

### Source F: RocksDB Memory-Mapped File Accumulation

Each node opens 2 RocksDB instances (block_store + state_db). RocksDB uses mmap for SST files. macOS accounts for mapped memory in the process's address space, and the kernel must manage page tables for each mapping.

With 103 nodes * 2 instances * ~40MB = ~8.2 GB of memory-mapped regions. macOS's unified buffer cache handles this, but **page table overhead** (8 bytes per 4KB page) adds:

```
Page table overhead = 8.2 GB / 4KB * 8 bytes = ~16 MB (negligible)
```

However, RocksDB's block cache defaults to 8MB per instance. With 103 nodes:
```
RocksDB block cache = 103 * 2 * 8MB = ~1.6 GB
```

### Source G: The Real Non-Linearity — O(N^2) GossipSub in Localhost Topology

**Key finding**: In a localhost deployment, all nodes share the same IP. libp2p's gossipsub sees all peers as being from the same "network." The `ip_colocation_factor_threshold` is set to 500 (config.rs:147), so scoring penalties don't apply until 500+ nodes on the same IP.

But the gossipsub **heartbeat processing** scans ALL connected peers for EACH of 7 topics, performing score lookups, mesh maintenance, and IHAVE generation. The heartbeat runs every 1 second.

Per heartbeat per node:
```
Work = topics(7) * peers(25) * (score_lookup + mesh_check + ihave_decision)
     = 7 * 25 * ~1us = ~175us
```

At the system level per heartbeat:
```
System work = 103 nodes * 175us = ~18ms (manageable)
```

The real O(N^2) comes from **message delivery events**. When a block is published:
1. It propagates through the mesh: O(N) deliveries
2. Each delivery generates a NetworkEvent that the node must process: O(N) events
3. Each event includes a full deserialized Block

But each node only receives the block ONCE (deduplication works), so this is O(N), not O(N^2).

**The actual O(N^2) is in the connection CHURN combined with gossipsub resync.**

When a peer is evicted (Source C), gossipsub must:
1. PRUNE the peer from all 7 topic meshes (7 messages)
2. When a new peer joins, GRAFT into up to 7 topic meshes (7 messages)
3. Exchange message caches (history_gossip=3 heartbeats of messages)

With churn rate of ~268/s system-wide, gossipsub processes ~268 * 14 = 3,752 GRAFT/PRUNE events per second across all 103 nodes. Each GRAFT triggers a mesh rebalance.

---

## 3. Top 5 Memory Hotspots (Ranked by Estimated Memory Consumption)

### Hotspot 1: Yamux Receive Window Buffers (CRITICAL)
- **Estimated memory**: N * max_peers * 1.5_connections * 2_directions * 512KB_yamux = **103 * 25 * 1.5 * 2 * 0.5MB = 3,862 MB (~3.8 GB)**
- **Location**: `crates/network/src/transport.rs:12-20`
- **Why it's first**: Yamux allocates the receive window per-stream, and there are ~6 active streams per connection (gossipsub + identify + kademlia + status + sync + txfetch). The 512KB is per-connection, but with 6 streams potentially active: worst case is 512KB * 6 = 3MB per connection. **More precisely**: yamux allocates the window per-session, not per-stream. So it's 512KB per connection. But both sides allocate. Total: 103 nodes * ~56 connections (25 peers * ~2.2 conns avg including churn) * 512KB = **~3 GB system-wide**.

### Hotspot 2: Connection Churn Transient Memory (HIGH)
- **Estimated memory at N=103**: Continuous allocation of ~1.1MB per churn cycle, with ~2.6 cycles/s/node = 103 * 2.6 * 1.1MB * 0.5s_half-life = **~148 MB transient**, but the **memory fragmentation effect** is far worse: Rust allocators (jemalloc or system) cannot efficiently reuse fragments from short-lived connections, leading to virtual memory bloat ~2-3x beyond the actual live set.
- **Location**: `crates/network/src/service/swarm_events.rs:77-148`
- **Why it matters**: Memory fragmentation from rapid alloc/dealloc causes RSS to grow monotonically even though live heap size is bounded. This is the most likely mechanism for the non-linear cliff.

### Hotspot 3: RocksDB Instances Per Node (HIGH)
- **Estimated memory**: 103 * 2 instances * (8MB block_cache + ~30MB mmap) = **103 * 76MB = ~7.8 GB**
- **Location**: `crates/storage/src/block_store.rs`, `crates/storage/src/state_db.rs`
- **Why it's high**: RocksDB is the single largest fixed cost per node. At 103 nodes, this is already nearly 8 GB just from storage layer.

### Hotspot 4: Tokio mpsc Channel Buffers (MEDIUM)
- **Estimated memory**: 103 nodes * (4096 event_buf + 2048 cmd_buf) * avg_msg_size(~2KB) = **103 * 6144 * 2KB = ~1.2 GB worst case**
- **Location**: `crates/network/src/service/mod.rs:76-79`
- **Note**: Channels are rarely full, so typical usage is ~10-20% of capacity. Typical: ~150-250 MB.

### Hotspot 5: GossipSub Internal State Per Topic Per Peer (MEDIUM)
- **Estimated memory**: 103 nodes * 7 topics * 25 peers * (mesh_entry + score_state + message_cache) ≈ 103 * 7 * 25 * ~2KB = **~36 MB system-wide**
- **Location**: `crates/network/src/gossip/config.rs:68-91` (configuration), internal to libp2p gossipsub
- **Why it matters**: Small individually, but combined with duplicate_cache_time=60s and 7 topics, the gossipsub state per node is non-trivial at ~350KB, totaling ~36MB across 103 nodes.

### Summary Memory Budget at N=103

| Component | Per Node | System Total (103 nodes) |
|-----------|----------|-------------------------|
| RocksDB (2 instances) | 76 MB | 7,828 MB |
| Yamux buffers (all connections) | 28 MB | 2,884 MB |
| Node baseline (runtime, structs) | 15 MB | 1,545 MB |
| Channel buffers (typical 15% fill) | 2 MB | 206 MB |
| Connection churn fragmentation | 5 MB | 515 MB |
| Gossipsub state | 0.35 MB | 36 MB |
| SyncManager + pipeline | 0.1 MB | 10 MB |
| Kademlia store | 0.01 MB | 1 MB |
| **Total estimated** | **~126 MB** | **~13 GB** |

**At N=150**: 150 * 126 = ~19 GB (if linear). The non-linear cliff adds ~30-50% from fragmentation.

---

## 4. The Cliff Mechanism: Why 103 is the Threshold

The cliff is not at a specific node count. It's at the point where **eviction churn rate exceeds the rate at which freed memory can be reclaimed**. Here's why:

1. **Below N=MP+1 (N<=26)**: No eviction. Every peer fits. Memory is stable.
2. **26 < N <= ~80**: Eviction churn exists but is low. Excess peers = N-25, churn = (N-25)/30s. At N=80: 1.8 evictions/s. Memory fragmentation is absorbed by the allocator.
3. **80 < N <= 103**: Churn rate = (N-25)/30s. At N=103: 2.6/s. The allocator starts fragmenting. RSS grows faster than live heap.
4. **N > 103**: The fragmentation exceeds the allocator's ability to coalesce. RSS growth becomes super-linear. Additionally:
   - macOS kernel TCP buffer allocation starts competing with user-space memory
   - File descriptor count (103 nodes * ~60 connections = 6,180 FDs) approaches `ulimit -n` (default 256 on macOS, typically raised to 10240)
   - macOS `net.inet.tcp.sendspace` and `recvspace` kernel buffers compound: 6,180 connections * 256KB = **~1.5 GB of kernel TCP buffers alone**

5. **At N=153+ (adding 50 more)**: System total = 153 * 126MB + fragmentation + kernel = ~20 GB + 3-4 GB fragmentation + 2.3 GB kernel = **~26 GB**.

---

## 5. Recommendations

### R1: Reduce Yamux Window to 128KB for Testnet Stress Tests (CRITICAL, Quick Win)
- **Current**: 512KB (`DOLI_YAMUX_WINDOW=524288`)
- **Proposed**: 128KB (`DOLI_YAMUX_WINDOW=131072`)
- **Impact**: Cuts per-connection memory from ~1.1MB to ~0.4MB. System-wide at 150 nodes: saves ~4.5 GB.
- **Risk**: Sync throughput may drop slightly for large header batches, but headers are fetched sequentially so 128KB is still sufficient for the 100KB header batches.
- **Action**: Set `DOLI_YAMUX_WINDOW=131072` in the `.env` file before stress test.

### R2: Reduce max_peers to 15 for 150-node Localhost Tests (HIGH, Quick Win)
- **Current**: 25 (testnet default)
- **Proposed**: 15 for stress test, configurable via `DOLI_MAX_PEERS=15`
- **Rationale**: mesh_n=12 requires 12 mesh peers per topic. With max_peers=15, gossipsub still has full mesh capacity. The 10-peer reduction saves:
  - Connection memory: 150 nodes * 10 fewer connections * 2 * 0.4MB = ~1.2 GB
  - Reduced churn: fewer excess peers means lower eviction rate
- **Risk**: Gossip propagation may slow slightly (fewer redundant paths). Acceptable for stress testing.
- **Action**: Set `DOLI_MAX_PEERS=15` in `.env`.

### R3: Increase Eviction Cooldown to 120 Seconds (HIGH, Quick Win)
- **Current**: 30 seconds (swarm_events.rs:83)
- **Proposed**: 120 seconds
- **Impact**: Reduces churn rate by 4x. At N=150 with MP=15: excess=135 peers, churn drops from 135/30=4.5/s to 135/120=1.1/s per node. System churn drops from 675/s to 165/s.
- **Risk**: New nodes take longer to integrate. Kademlia DHT exchange via bootstrap_slots (10s TTL) provides initial discovery, so the 120s cooldown only affects returning evicted peers.
- **Action**: Code change in `swarm_events.rs:83`. Recommend `/omega-improve --scope=networking`.

### R4: Share RocksDB Instances Across Co-Located Nodes (HIGH, Architecture Change)
- **Current**: Each node opens 2 RocksDB instances = 206 instances at 103 nodes.
- **Proposed**: For localhost stress tests, use a shared block_store (read-only) and per-node state_db. Nodes that are syncing can read from a single canonical block store.
- **Impact**: Saves ~103 * 38MB = ~3.9 GB from eliminated duplicate block_store instances.
- **Risk**: Requires architectural change. Block production still needs per-node write access.
- **Action**: This is a significant refactor. For immediate stress testing, the Yamux + max_peers fixes are more impactful. Recommend as Phase 2 optimization if needed.

### R5: Reduce Event Channel Buffer from 4096 to 1024 (MEDIUM)
- **Current**: `event_buf = (max_peers * 16).clamp(4096, 32768)` = 4096 for MP=25
- **Proposed**: `event_buf = (max_peers * 8).clamp(512, 4096)`
- **Impact**: At MP=15: event_buf = 512 (from 4096). Saves ~3KB * 3584 slots = ~10MB per node, or ~1.5 GB at 150 nodes.
- **Risk**: Under event flooding, backpressure hits sooner. The existing drain logic (event_loop.rs:79-94, cap=max_peers*3) mitigates this.
- **Action**: Code change in `service/mod.rs:76`. Recommend `/omega-improve`.

### R6: Add Per-Node Memory Budget and Refuse Connections Beyond It (MEDIUM)
- **Proposed**: Add a `DOLI_MAX_MEMORY_MB` environment variable. In `ConnectionEstablished`, before accepting the peer, check RSS against budget and refuse connection if over.
- **Impact**: Hard cap prevents the runaway condition entirely.
- **Action**: New feature. Recommend `/omega-new-feature`.

### R7: Reduce GossipSub Topics from 7 to 4 (MEDIUM, Architecture)
- **Current**: BLOCKS, TRANSACTIONS, PRODUCERS, VOTES, HEARTBEATS, HEADERS, ATTESTATION (7 topics)
- **Note**: HEARTBEATS is deprecated (no-op) but still subscribed (`gossip/config.rs:163`)
- **Proposed**: Unsubscribe from HEARTBEATS (dead code), HEADERS (only needed for tiered architecture not yet active), TIER1_BLOCKS (not active). This reduces topics from 7 to 4 (BLOCKS, TRANSACTIONS, PRODUCERS, ATTESTATION).
- **Impact**: 43% reduction in gossipsub per-peer state, heartbeat processing, and IHAVE/IWANT traffic.
- **Action**: Code change in `gossip/config.rs:158-189`. Recommend `/omega-improve`.

---

## 6. Quick Wins for Immediate Stress Test

These changes can be made via environment variables, requiring NO code changes:

| Variable | Current | Proposed | Memory Saved (150 nodes) |
|----------|---------|----------|--------------------------|
| `DOLI_YAMUX_WINDOW` | 524288 (512KB) | 131072 (128KB) | ~4.5 GB |
| `DOLI_MAX_PEERS` | 25 | 15 | ~1.2 GB |
| `DOLI_CONN_LIMIT` | 60 (auto) | 40 | ~0.8 GB |
| `DOLI_BOOTSTRAP_SLOTS` | 10 | 5 | Negligible, reduces conn_limit |
| **Total savings** | | | **~6.5 GB** |

**Expected total at 150 nodes with quick wins**:
- Per node: ~80 MB (down from ~126 MB)
- System total: 150 * 80 = ~12 GB + ~2 GB fragmentation = **~14 GB**
- Feasible on a 128 GB Mac with headroom for OS and application overhead.

**Without quick wins at 150 nodes**: ~19 GB linear + ~6-8 GB fragmentation/kernel = **~25-27 GB**.

---

## 7. OS-Level Factors (macOS Specific)

### File Descriptors
- Each connection uses 1 FD. At 150 nodes * 40 connections = 6,000 FDs.
- macOS default `ulimit -n` is 256, usually raised to 10240.
- **Must verify**: Run `ulimit -n` on the test machine. If <10240, set in launchd or shell profile.

### TCP Buffer Memory (Kernel)
- `sysctl net.inet.tcp.sendspace` and `recvspace` (default 131072 bytes each)
- 6,000 connections * 256KB = ~1.5 GB of kernel TCP buffers
- To reduce: `sysctl -w net.inet.tcp.sendspace=65536` and `recvspace=65536` (halves to ~750 MB)

### macOS Unified Buffer Cache
- RocksDB mmap'd files compete with all other memory. At 150 nodes * 60 MB = 9 GB of mmap'd regions.
- macOS aggressively evicts mmap'd pages under memory pressure, causing thrashing (I/O wait spikes).
- **Mitigation**: Reduce RocksDB block_cache_size from 8MB default to 2MB per instance via RocksDB options.

---

## Security Posture Summary

| Attack Vector | Protection | Status |
|---------------|-----------|--------|
| Eclipse attack | PeerDiversity (IP prefix tracking, disabled on localhost) | Partial — disabled on 127.0.0.1 |
| Sybil attack | No identity cost on localhost (no PoW, no stake check for connections) | Vulnerable (localhost only) |
| Routing table poisoning | Kademlia liveness checks, eviction cooldown | Protected |
| Gossip mesh manipulation | Peer scoring (3 scored topics), GRAFT rate limiting via scoring | Protected |
| Message suppression | mesh_n=12 (redundant paths), IHAVE/IWANT protocol | Protected |
| DoS / resource exhaustion | Rate limiting, connection limits, **but eviction churn is the DoS vector on localhost** | Partial |
| NAT traversal abuse | Relay server disabled by default, reservation limits | Protected |

---

## Performance Notes

### Block Propagation Latency at 150 Nodes
- Gossipsub with mesh_n=12 and gossip_factor=0.25: ~3 hops for full coverage
- At 10ms per hop (localhost TCP): ~30ms propagation latency
- With 10s slots: propagation is not the bottleneck

### Sync Throughput During Mass Sync
- 50 new nodes syncing simultaneously, each requesting 500 headers + 128 bodies
- Each seed node serves sync requests from max_peers=15 nodes
- Rate limit: 8 sync requests per tick per node (periodic.rs:394-403)
- Bottleneck: BlockStore disk I/O for serving 15 * 8 = 120 block reads per second

---

## Next Steps

1. **Immediate (before stress test)**: Apply env var quick wins — set `DOLI_YAMUX_WINDOW=131072` and `DOLI_MAX_PEERS=15` in `.env`. Verify `ulimit -n >= 10240`.

2. **Code changes (R3, R5, R7)**: Run `/omega-improve --scope=networking` with findings from this report:
   - Increase eviction cooldown from 30s to 120s
   - Reduce event channel buffer formula
   - Unsubscribe from deprecated/unused gossip topics

3. **Memory monitoring**: The existing `[HEALTH]` log (periodic.rs:531-537) reports RSS every 30s. Add per-node connection count to the log for correlation.

4. **If 150 nodes still OOMs**: Consider R4 (shared block_store) or R6 (per-node memory budget).
