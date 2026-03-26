# P2P Network Advisory: Seed Bootstrap Bottleneck Under Burst Node Joins

## Task Type
Architecture Assessment + Troubleshooting Advisory

## Protocol Stack Map
| Layer | Implementation | Library/Custom | Status |
|-------|---------------|----------------|--------|
| Transport | TCP + Noise XX + Yamux (1MB window) | library (rust-libp2p) | OK |
| Security | Noise XX (ed25519 identity) | library | OK |
| Multiplexing | Yamux (per-peer limit: 2 connections) | library | CONCERN |
| Discovery | Kademlia DHT (replication=20, 60s refresh) | library | CONCERN |
| Routing | GossipSub v1.1 (mesh_n=12, peer scoring enabled) | library | OK |
| NAT Traversal | AutoNAT + Relay + DCUtR | library | OK |
| Peer Scoring | GossipSub topic scoring + custom PeerScorer | mixed | OK |
| Connection Mgmt | ConnectionLimits + custom eviction + cooldown | mixed | CONCERN |
| Application | Status/Sync/TxFetch request-response | custom on libp2p | OK |

## Root Cause Analysis

### The Exact Failure Sequence

The problem is well-diagnosed by the user. Here is the precise code-level mechanism:

**Step 1: Connection limits are enforced at the libp2p transport layer.**

In `service/mod.rs:139-143`:
```rust
let conn_limit = (config.max_peers * 2 + 10) as u32;
let limits = libp2p::connection_limits::ConnectionLimits::default()
    .with_max_established_per_peer(Some(2))
    .with_max_established_incoming(Some(conn_limit))
    .with_max_established_outgoing(Some(conn_limit));
```

With `max_peers=25`: `conn_limit = 60`. With `max_peers=150` (devnet): `conn_limit = 310`.

**Step 2: `connection_limits::Behaviour` rejects connections BEFORE the swarm event loop sees them.**

This is the critical insight the user correctly identified. The `ConnectionLimits` behaviour implements `handle_pending_inbound_connection()` and `handle_established_inbound_connection()` in libp2p. When the limit is reached, the TCP connection is RST'd at the transport layer. There is **no `SwarmEvent`** emitted for this rejection -- the connecting node sees a TCP RST or timeout, and the seed node's eviction logic in `swarm_events.rs:76-113` is **never triggered** because `ConnectionEstablished` is never fired.

**Step 3: Eviction is reactive, not preemptive.**

The eviction logic in `swarm_events.rs:76-106` runs inside the `ConnectionEstablished` handler:
```rust
if peers.len() >= config.max_peers {
    // find lowest-scored evictable peer
    // evict it
    // then add the new peer
}
```

This means eviction only fires when a new connection **successfully establishes**. When burst arrivals push past `conn_limit` at the transport layer, eviction never runs because the connections are rejected before `ConnectionEstablished`.

**Step 4: The eviction is serial and holds an async write lock.**

```rust
let mut peers = peers.write().await;  // <-- async RwLock, exclusive
```

Even when evictions do fire, they execute one-at-a-time because each `ConnectionEstablished` handler acquires the peers write lock. With 2 connections per peer and a busy swarm, the eviction throughput is limited by the event loop's ability to process `ConnectionEstablished` events sequentially.

**Step 5: Rejected nodes have no fallback discovery path.**

When the seed rejects a connection at the transport layer:
- The joining node has zero peers (fresh start, no peer cache)
- Its only known address is the seed (from `bootstrap_nodes` config)
- Kademlia `bootstrap()` requires at least one connected peer to query
- The node enters the retry loop in `periodic.rs:208-228` with exponential backoff (1s, 2s, 4s, ... 60s cap)
- But the seed is STILL full, so each retry gets the same RST
- The node is stuck until another peer independently frees a seed slot

### Key Insight: Two Separate Limits, One Bottleneck

There are two independent limits:
1. **Peer table limit** (`max_peers`): managed by custom eviction in `swarm_events.rs`
2. **Connection limit** (`conn_limit = max_peers * 2 + 10`): managed by libp2p `ConnectionLimits`

The problem is that limit #2 blocks connections BEFORE limit #1's eviction can run. The eviction logic is designed to handle limit #1, but limit #2 creates a hard wall below it.

With `max_peers=25` and `per_peer=2`: max established incoming = 60. If 25 peers each hold 2 connections = 50. That leaves 10 slots for simultaneous newcomers. Node 26 arrives: eviction frees 2 connections (one peer evicted = 2 connections freed), bringing it to 50 again. But if nodes 26-36 arrive simultaneously, only 10 can get in before `conn_limit` blocks the rest at the TCP level.

## Findings

### Critical

#### F1: Transport-Level Connection Rejection Bypasses Application-Level Eviction

- **Issue**: `libp2p::connection_limits::Behaviour` rejects inbound connections when `max_established_incoming` is reached. This rejection happens at the transport layer BEFORE the swarm emits `ConnectionEstablished`, so the custom eviction logic in `swarm_events.rs:76-106` never executes. Under burst joins (20+ simultaneous), this creates a deadlock: the seed cannot evict old peers to make room because the mechanism to trigger eviction (new connection established) is blocked by the mechanism that should protect it (connection limits).
- **Impact**: Fatal bootstrap failure. Nodes rejected at the transport layer have no peers, no DHT entries, and no path to discover the network. In the observed incident, 45 nodes were permanently stuck at h=0 with 0 peers.
- **Location**: `crates/network/src/service/mod.rs:139-143` (connection limits) interacting with `crates/network/src/service/swarm_events.rs:76-113` (eviction logic)
- **Recommendation**: See Solution Ranking below.
- **Action**: `/omega-new-feature` for the recommended solution.

### High

#### F2: Single Seed is a Single Point of Failure for Bootstrap

- **Issue**: The testnet/devnet configuration has a single seed node address in `bootstrap_nodes`. When that seed is at capacity, there is zero redundancy. Even with the eviction fix, a single seed can only process one eviction per event loop tick, creating a throughput ceiling for node joins.
- **Impact**: At 100+ nodes, even with perfect eviction, the seed processes ~1 eviction per event loop tick (~1ms). 100 simultaneous joins would take ~100ms best-case, but with Noise handshake overhead (~50ms per connection), real throughput is much lower. This limits burst join capacity to roughly 10-15 nodes/second.
- **Location**: `crates/core/src/network_params/defaults.rs:91-95` (testnet bootstrap_nodes), node configuration
- **Recommendation**: Add 2-3 seed nodes. In the local stress test environment, designate the first 2-3 nodes as seeds (configure them with each other's addresses, no `bootstrap_nodes` of their own). All other nodes list all seeds in `bootstrap_nodes`.

#### F3: Eviction Cooldown Prevents Rapid Slot Recycling

- **Issue**: The 30-second eviction cooldown (`swarm_events.rs:63-73`) prevents recently-evicted peers from reconnecting. This is correct for preventing thrashing (INC-I-011), but during burst bootstrap it means an evicted peer's slot is effectively locked for 30 seconds. If the seed evicts 5 peers to make room for 5 newcomers, those 5 evicted peers cannot get back in for 30 seconds -- even if they still need the seed for DHT bootstrap.
- **Impact**: During burst joins, the eviction cooldown amplifies the bottleneck. Each eviction frees a slot but also creates a 30-second lockout for the evicted peer. In a 50-node burst, after the first round of evictions (say 5 peers evicted), those 5 are locked out. If those 5 were not yet synced, they need the seed again and cannot reach it.
- **Location**: `crates/network/src/service/swarm_events.rs:63-73` (eviction cooldown check)
- **Recommendation**: The cooldown is necessary for steady-state operation. The real fix is ensuring evicted peers do not need the seed (i.e., they have other peers via DHT). This is solved by the DHT bootstrap-then-disconnect approach (Solution B below).

#### F4: Bootstrap Release is Too Conservative

- **Issue**: The bootstrap release mechanism (`periodic.rs:236-258`) only releases seed connections when the node is `Synchronized` AND has > 3 peers. The `Synchronized` state may take minutes (full header+body sync). During this time, the node holds its seed connection, consuming a slot that could serve a newcomer.
- **Impact**: In a 50-node burst join: all nodes that successfully connect to the seed hold their seed connection for the full sync duration (~30-120 seconds each). Only after sync completes do they release. This means the seed has effectively zero free slots during the entire burst sync window.
- **Location**: `bins/node/src/node/periodic.rs:236-258`
- **Recommendation**: Two-phase release: release the seed connection as soon as the node has >= 3 non-seed peers (regardless of sync state), since at that point it can bootstrap Kademlia from those peers. Keep the current Synchronized check as a secondary trigger.

### Medium

#### F5: DHT Bootstrap Requires Connected Peer

- **Issue**: Kademlia `bootstrap()` fails with `NoKnownPeers` if the routing table is empty (`swarm_loop.rs:143-150`). For a fresh node, the routing table only gets populated when an Identify exchange happens with a connected peer (`behaviour_events.rs:314-323`). This means a node that cannot connect to ANY peer cannot use Kademlia at all, creating a circular dependency: need a connection to populate DHT, need DHT to find peers beyond the seed.
- **Impact**: This is the fundamental chicken-and-egg problem. It is inherent to Kademlia's design (all DHT implementations require at least one initial contact). The fix is ensuring that initial contact is always available (multiple seeds, or a non-connection-limited bootstrap endpoint).
- **Location**: Architectural -- Kademlia specification constraint
- **Recommendation**: Addressed by Solutions A and B below.

#### F6: Peer Cache Excludes Loopback Addresses

- **Issue**: `peer_cache.rs:75` rejects all `127.0.0.*` addresses. In a local stress test where all nodes are on `127.0.0.1`, the peer cache is effectively empty. This means nodes that restart or reconnect cannot use cached peers as an alternative to the seed.
- **Impact**: On the local stress test specifically, the peer cache provides zero value. Every reconnection must go through the seed. On a real deployment with distinct IPs, this is not an issue.
- **Location**: `crates/network/src/peer_cache.rs:75-77`
- **Recommendation**: For local testing, either disable the loopback filter (behind a `--local-test` flag) or use distinct `127.0.0.X` addresses per node (the OS supports the entire `127.0.0.0/8` range). This is a testing-only concern, not a production issue.

### Low / Informational

#### F7: Connection Limit Formula Has Minimal Headroom

- **Issue**: `conn_limit = max_peers * 2 + 10` provides only 10 connections of headroom beyond the theoretical maximum (max_peers * per_peer). The "+10" accommodates simultaneous-dial races and briefly-overlapping connections during eviction, but under burst conditions the headroom fills in <1 second.
- **Impact**: Minor contributor. Even increasing headroom to +50 only delays the wall by a few hundred milliseconds under a 50-node burst. The fundamental issue is that the wall exists at all (F1).
- **Location**: `crates/network/src/service/mod.rs:139`

#### F8: Kademlia Replication Factor of 20 is Aggressive

- **Issue**: `discovery.rs:19` sets replication factor to 20, which means each DHT record is stored on 20 peers. In a 25-peer network, this means every node stores every record -- effectively defeating the point of DHT partitioning. In a 100-node network it is still high (20% of nodes per record).
- **Impact**: Higher bandwidth and storage for DHT maintenance. Not a bottleneck in the current problem, but worth reducing to the standard 8-12 range as the network grows.
- **Location**: `crates/network/src/discovery.rs:19`

## Solution Analysis

### Solution A: Multiple Seed Nodes

**User's Proposal #1**

**Mechanism**: Configure 2-3 seed nodes instead of 1. Each new node tries all seeds in sequence (or parallel). If seed A is full, seed B may have capacity.

**Implementation Complexity**: Low. This is a configuration change, not a code change.
- Devnet/testnet: add 2-3 entries to `bootstrap_nodes` in network_params defaults
- Local stress test: start first 2-3 nodes as seeds (no `bootstrap_nodes`), all others list all seeds

**Effectiveness**:
- At 100 nodes with 3 seeds: each seed handles ~33 concurrent bootstrap connections. With `max_peers=25` per seed, this is manageable -- each seed has enough capacity.
- At 500 nodes with 3 seeds: ~167 per seed. At `max_peers=25`, each seed will still hit the conn_limit wall. Need either more seeds or higher max_peers on seeds.
- At 1000 nodes with 3 seeds: ~333 per seed. Insufficient without Solution B.

**Risk**: Low. Additive change. Does not modify wire protocol. Seeds are just regular nodes that know each other.

**Limitation**: Linear scaling -- need to add more seeds as network grows. Does not solve the fundamental conn_limit wall; just distributes it across more nodes.

**Verdict**: **Necessary but insufficient alone.** Should be the FIRST thing deployed because it requires zero code changes and immediately multiplies bootstrap capacity by the number of seeds.

### Solution B: Brief Accept for DHT Bootstrap (Connect-Exchange-Disconnect)

**User's Proposal #2**

**Mechanism**: When the seed is at capacity and a new node connects, instead of rejecting at the transport layer, briefly accept the connection long enough for a Kademlia exchange (peer routing table), then disconnect. The new node now has N peer addresses from the DHT and can connect to those peers directly.

**Implementation Complexity**: Medium. Requires modifying the interaction between `ConnectionLimits` and the eviction logic.

**Two implementation approaches**:

**B1: Raise connection limit, keep peer table limit** (Recommended)
- Set `max_established_incoming` to `conn_limit + bootstrap_headroom` (e.g., `max_peers * 2 + 50`)
- In `ConnectionEstablished`: if `peers.len() >= max_peers` AND no evictable peer, accept the connection into a "bootstrap-only" state: add the peer to Kademlia but NOT to the peer table, set a 10-second disconnect timer
- After 10 seconds (enough for Identify + Kademlia exchange), disconnect the peer
- The peer now has DHT entries and can connect to non-seed peers

**B2: Remove `ConnectionLimits` entirely, rely only on peer table eviction**
- Delete the `ConnectionLimits` behaviour
- Let all TCP connections establish
- Eviction in `ConnectionEstablished` handles the peer count limit
- Risk: without connection limits, a DoS attacker can open thousands of connections, exhausting file descriptors and Yamux buffer memory (this was the original INC-I-011 trigger: 86GB RAM from Yamux buffers)
- **Not recommended** because INC-I-011 proved this is dangerous

**B1 is the correct approach.** The key insight: the purpose of the connection limit is to bound resource usage (Yamux buffers, file descriptors). The purpose of the peer table limit is to bound the gossip mesh. These are different concerns. We can allow MORE connections (for brief DHT exchanges) while keeping FEWER peers (for gossip efficiency).

**Effectiveness**:
- At 100 nodes: seed accepts all 100 connections briefly, each gets DHT entries in ~1 second, then disconnects. Total wall-clock time: ~5 seconds (limited by Noise handshake throughput, ~20 connections/second on a single core).
- At 500 nodes: same pattern, ~25 seconds for all 500 to bootstrap. Manageable.
- At 1000 nodes: ~50 seconds. Acceptable for bootstrap.

**Risk**: Medium.
- The "bootstrap-only" connection state needs careful implementation to avoid leaking connections (timer must be guaranteed to fire even if the event loop is under load).
- The 10-second window must be long enough for Identify -> Kademlia -> GetClosestPeers to complete. In practice: Identify fires within 1 second, Kademlia addresses are added immediately, GetClosestPeers takes 1-5 seconds. 10 seconds is sufficient.
- Attacker concern: an attacker could repeatedly connect, get DHT data, disconnect, repeat. This is rate-limited by the Noise handshake cost (~50ms per connection) and the eviction cooldown. Not a significant attack vector beyond what Kademlia already exposes.

**Wire protocol changes**: None. Uses existing Identify + Kademlia protocols.

**Verdict**: **The highest-impact fix.** Solves the fundamental problem: new nodes always get DHT entries, even when the seed is at capacity. Combined with Solution A, this makes the network robust to burst joins at any scale.

### Solution C: Peer Exchange via Gossip

**User's Proposal #3**

**Mechanism**: Nodes share peer addresses alongside producer announcements in gossip messages. A new node that connects to ANY peer (not just the seed) receives a batch of peer addresses it can dial.

**Implementation Complexity**: Medium-High.
- New gossip topic or extension to existing producer announcements
- Needs anti-spam protection: without limits, each node would broadcast its entire peer list to all peers on every producer announcement cycle
- Needs freshness tracking: stale addresses cause connection failures
- The PEX (Peer Exchange) protocol is a well-studied design -- see BitTorrent PEX (BEP-11) and libp2p Rendezvous protocol

**Effectiveness**:
- Only useful if the node can connect to at least ONE peer. If the seed rejects the connection, the node has nobody to receive PEX messages from. This means PEX alone does NOT solve the bootstrap problem -- it requires at least Solution A or B to be effective.
- Once connected, PEX provides redundant peer discovery alongside Kademlia. This is valuable for resilience but redundant with the existing 60-second DHT refresh.

**Risk**: Medium.
- If poorly implemented, PEX can leak network topology (which peers are connected to which). This is an information leakage vector for traffic analysis.
- PEX messages can be used for routing table poisoning if not validated (attacker broadcasts fake peer addresses). Need to verify that advertised addresses are reachable (dial-back verification).
- Adds a new gossip message type that increases baseline bandwidth usage.

**Wire protocol changes**: Yes. New gossip topic or message format. All nodes must understand it.

**Verdict**: **Nice-to-have, not essential.** The existing Kademlia DHT already provides peer discovery once a node has any connection. PEX would reduce discovery latency from ~60 seconds (DHT refresh interval) to ~1 second (next gossip message), but Solution B already provides DHT entries within seconds of connecting to the seed. Defer to a later iteration.

## Recommended Implementation Order

### Phase 1: Config-Only (Immediate, Zero Code Changes)

**Multiple seeds for the local stress test:**

For the local testnet, designate nodes 1-3 as seed nodes. All other nodes list all three in `bootstrap_nodes`. This immediately provides 3x the bootstrap capacity with no code changes.

For the testnet defaults, add additional bootstrap addresses to the `defaults.rs` configuration. This requires a binary rebuild but no logic changes.

**Devnet max_peers**: Currently 150, which gives `conn_limit = 310`. This is already sized for 100+ node stress tests. If you are running 50+ node tests on the testnet profile (`max_peers=25`, `conn_limit=60`), consider running with the devnet profile or overriding `DOLI_MAX_PEERS=150` via environment variable.

### Phase 2: Bootstrap Headroom (Recommended Fix)

**The minimum code change that unblocks 100-node stress tests.**

Introduce a "bootstrap headroom" concept: allow more connections than peers. Specifically:

1. **Split `conn_limit` from `max_peers`**: Instead of `conn_limit = max_peers * 2 + 10`, use `conn_limit = max_peers * 2 + bootstrap_headroom` where `bootstrap_headroom` defaults to 50 (or `max_peers`).

2. **Add a bootstrap-only connection state**: When `peers.len() >= max_peers` and no peer is evictable, accept the connection but start a 10-second disconnect timer. During those 10 seconds, Identify and Kademlia exchanges proceed normally. After 10 seconds, disconnect. The joining node now has DHT entries.

3. **Early bootstrap release**: Release the seed connection once the node has >= 3 non-seed peers, regardless of sync state. This frees the seed slot faster.

**Estimated implementation**: ~100 lines changed in `swarm_events.rs` and `service/mod.rs`. No wire protocol changes. No breaking changes.

### Phase 3: Peer Exchange (Future)

Only if Kademlia proves insufficient for large-scale discovery. Not needed for 100-node tests. Consider for 1000+ nodes.

## Security Posture Summary

| Attack Vector | Protection | Status |
|---------------|-----------|--------|
| Eclipse attack | IP diversity tracking (`peer.rs`), gossipsub peer scoring | Partial -- diversity tracking exists but is not wired into the eviction path. Eviction uses gossipsub score only. |
| Sybil attack | No explicit protection. Ed25519 identity is free to generate. | Vulnerable -- mitigated by producer-aware eviction (producers are never evicted) but non-producer peers have no Sybil resistance. |
| Routing table poisoning | Kademlia server mode, dead peer backoff (`swarm_events.rs:226-253`) | Partial -- no S/Kademlia or signed routing records. Acceptable for a permissioned PoS network where producers are stake-weighted. |
| Gossip mesh manipulation | GossipSub v1.1 peer scoring with topic-specific weights | Protected -- BLOCKS_TOPIC has 10x first_message_deliveries_weight, producers marked `is_producer` are immune to eviction. |
| Message suppression | Gossipsub mesh degree D=12, IHAVE/IWANT protocol | Protected -- standard gossipsub redundancy. |
| DoS / resource exhaustion | ConnectionLimits (60 incoming), per-peer rate limiting, eviction cooldown | Partial -- the proposed fix (increasing conn_limit for bootstrap headroom) slightly increases the DoS surface. Mitigated by the Noise handshake cost (~50ms per connection) and the 10-second bootstrap-only timeout. |
| NAT traversal abuse | Relay server disabled by default, configurable per-circuit limits | Protected -- relay is opt-in for public nodes. |

Note on the proposed fix: increasing `bootstrap_headroom` by 50 connections means the seed can have up to 110 TCP connections (with `max_peers=25`). Each Yamux connection costs ~1MB (the configured receive window). 110 connections = ~110MB -- well within the INC-I-009 safety margin (86GB was at 200+ peers). The 10-second disconnect timer ensures bootstrap-only connections are short-lived.

## Performance Notes

**Connection establishment throughput**: The Noise XX handshake requires 2 round-trips. On localhost (RTT ~0.1ms), this completes in ~1ms. On a real network (RTT ~50ms), it takes ~100ms. The CPU cost of the handshake (Curve25519 DH) is ~0.05ms. So on localhost, the bottleneck is the event loop's ability to process `ConnectionEstablished` events, not crypto. Practical throughput: ~100-200 connections/second on localhost.

**Kademlia exchange latency**: After `ConnectionEstablished`, the Identify exchange fires within 1 heartbeat (1 second). Kademlia addresses are added immediately after Identify. A `GetClosestPeers` query takes 1-5 seconds (iterative, with 20-peer replication). Total time from connection to having DHT entries: 2-6 seconds.

**Bootstrap headroom memory cost**: Each bootstrap-only connection holds a Yamux session (1MB window) for up to 10 seconds. With 50 headroom connections: peak memory = 50MB. At 10 connections/second turnover: steady-state memory = ~100MB during a burst. Acceptable.

**DHT refresh interval**: Currently 60 seconds (`swarm_loop.rs:43`). For burst bootstrap scenarios, consider reducing to 30 seconds. This doubles DHT query traffic but halves the time for newly-connected nodes to discover additional peers. The additional bandwidth is negligible (Kademlia queries are ~200 bytes).

## Next Steps

1. **Immediate (config only)**: For the local stress test, configure 3 seed nodes. Override `DOLI_MAX_PEERS=150` if running on testnet profile. This unblocks immediate testing.

2. **Phase 2 implementation**: Run `/omega-new-feature` with the following scope:
   - Add `bootstrap_headroom` parameter to `NetworkConfig` (default: 50)
   - Modify `conn_limit` formula: `max_peers * 2 + bootstrap_headroom`
   - Add bootstrap-only connection state with 10-second disconnect timer in `swarm_events.rs`
   - Change bootstrap release condition: release when >= 3 non-seed peers (drop Synchronized requirement)

3. **Validation**: After Phase 2, run the 50-node burst test. All nodes should reach synced state within 2 minutes. If not, collect logs and run `/omega-doctor`.
