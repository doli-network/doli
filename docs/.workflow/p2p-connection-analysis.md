# P2P Network Advisory: Why with_max_established(50) Does Not Cap TCP Connections in Devnet Stress Tests

## Task Type
Troubleshooting Advisory / Architecture Assessment

## Protocol Stack Map
| Layer | Implementation | Library/Custom | Status |
|-------|---------------|----------------|--------|
| Transport | TCP + DNS via tokio | libp2p 0.53.2 (library) | OK |
| Security | Noise XX (Ed25519) | libp2p-noise (library) | OK |
| Multiplexing | Yamux (256KB window) | libp2p-yamux 0.45.2 (library) | CONCERN |
| Discovery | Kademlia DHT + peer cache | libp2p-kad (library) | OK |
| Routing | GossipSub v1.1 + req/resp | libp2p-gossipsub (library) | OK |
| NAT Traversal | Relay client + DCUtR + AutoNAT | libp2p (library) | CONCERN |
| Connection Limits | libp2p-connection-limits 0.3.1 | library | CONCERN |
| Peer Scoring | GossipSub topic scoring | library | OK |
| Application | Status + Sync + TxFetch req/resp | custom codecs | OK |

---

## Executive Summary

The TCP count exceeding `with_max_established(50)` is **not a libp2p bug** -- ConnectionLimits works correctly. The overshoot has three compounding causes:

1. **Devnet max_peers=150 produces total_conn_limit=300**, not 50. The title premise is wrong for devnet.
2. **OS-level `lsof | grep TCP` counts sockets in all states** (ESTABLISHED + TIME_WAIT + CLOSE_WAIT + SYN_SENT + etc.), while libp2p `num_established()` only counts fully negotiated connections. The mismatch is an observation artifact.
3. **Bootstrap-only peers and eviction-cooldown peers hold established connections outside the peer table** but inside the ConnectionLimits budget. They are not "bypassing" the limit -- they consume slots that then cannot be used by the peer table.

At 156 nodes on devnet, the **real** connection limit is 300 (not 50), and the measured 53.5 avg TCP sockets per node is well within that 300-connection budget. The RAM explosion comes from the sheer volume: 300 connections x 256KB yamux = 75MB/node, x 156 nodes = 11.7GB in yamux buffers alone. Adding pending connections, Noise handshake buffers, gossipsub message caches, and Kademlia routing tables pushes total RAM to the observed 54GB.

---

## Findings

### Critical

#### F1: Devnet max_peers=150 Produces total_conn_limit=300, NOT 50

- **Issue**: The connection limit formula at `crates/network/src/service/mod.rs:139-142` computes `total_conn_limit = max_peers * 2`. For devnet, `max_peers=150` (set in `crates/core/src/network_params/defaults.rs:165`), yielding `total_conn_limit=300`. The "50" in the title of this investigation applies to **testnet** (max_peers=25 -> conn_limit=50), not devnet.
- **Impact**: Each devnet node can hold up to 300 established connections. At 256KB yamux window per connection, that is 75MB/node. For 156 nodes: 75MB x 156 = 11.7GB in yamux buffers alone. This is the primary RAM contributor -- not any connection limit bypass.
- **Location**: `crates/core/src/network_params/defaults.rs:165` (`max_peers: 150`) and `crates/network/src/service/mod.rs:139-142` (`max_peers * 2`)
- **Evidence**: The MEM-BOOT log line at `mod.rs:237-245` prints `conn_limit=300` for devnet. The 53.5 avg TCP sockets per node reported in the stress test is well below the 300 limit. The limit is working -- it is just too high.
- **Recommendation**: Reduce devnet max_peers for stress testing. See F6 for specific values.
- **Action**: `/omega-improve --scope=crates/core/src/network_params/defaults.rs`

#### F2: max_peers Is Not CLI-Configurable -- Requires Code Change or Env Var to Override

- **Issue**: `max_peers` is set in `NodeConfig::for_network()` from `network.params().max_peers` (`bins/node/src/config.rs:92`). There is no `--max-peers` CLI flag. The only way to change it is to modify `defaults.rs` and recompile, or to add a `DOLI_MAX_PEERS` env var (which does not exist).
- **Impact**: Stress testing at different peer limits requires editing source code and rebuilding. This slows incident response (INC-I-014 required multiple rebuild cycles). The `DOLI_CONN_LIMIT` and `DOLI_PENDING_LIMIT` env vars exist for connection limits but not for the upstream `max_peers` that determines them.
- **Location**: `bins/node/src/config.rs:92`, `bins/node/src/cli.rs` (missing flag)
- **Recommendation**: Add `DOLI_MAX_PEERS` env var support in `config.rs` (consistent with existing `DOLI_CONN_LIMIT` pattern), OR add a `--max-peers` CLI flag. The env var approach is simpler and consistent with the existing pattern.
- **Action**: `/omega-new-feature` -- add DOLI_MAX_PEERS env var override in NodeConfig::for_network()

### High

#### F3: 86400-Second Idle Connection Timeout Prevents Natural Cleanup

- **Issue**: At `crates/network/src/service/mod.rs:208-209`, the idle connection timeout is 86400 seconds (24 hours). In a 156-node stress test lasting minutes, NO connections are ever cleaned up by timeout. Every connection established during the test lives for the entire test duration.
- **Impact**: In a high-churn scenario (nodes joining/leaving), the connection pool fills to the limit and stays there. The eviction logic in `swarm_events.rs:95-148` handles peer table overflow, but the underlying libp2p connections for evicted peers may persist until the eviction-disconnected peer is actually removed. The 24h timeout means connections from crashed nodes also persist as CLOSE_WAIT on the OS level, inflating the `lsof` count.
- **Location**: `crates/network/src/service/mod.rs:208-209`
- **Recommendation**: Use a shorter idle timeout for devnet/testnet stress tests. Suggested: 300s (5 min) for devnet, 3600s (1 hour) for testnet, 86400s for mainnet. This can be another env var (`DOLI_IDLE_TIMEOUT_SECS`).
- **Action**: `/omega-improve --scope=crates/network/src/service/mod.rs`

#### F4: Bootstrap-Only Peers Consume Connection Limit Slots While Being Invisible to Peer Table

- **Issue**: At `crates/network/src/service/swarm_events.rs:132-148`, when the peer table is full, up to `bootstrap_slots` (default 10) additional peers are accepted for temporary DHT exchange. These peers hold established libp2p connections (counted by ConnectionLimits) but are NOT in the peer table. They are disconnected after 10s via the cleanup timer at `swarm_loop.rs:165-184`.
- **Impact**: At any given moment, up to 10 connections are consumed by bootstrap peers. This is within the 300-connection budget and not a primary contributor. However, during rapid network growth (many nodes bootstrapping simultaneously), the 10 slots churn quickly -- each bootstrap peer consumes a connection for 10s, with 5s cleanup granularity, meaning up to 20 bootstrap connections could exist between cleanups. Still minor at 300 total budget.
- **Severity for devnet**: Low (10-20 out of 300 budget).
- **Severity for testnet**: Medium (10-20 out of 50 budget = 20-40% consumed by transient peers).
- **Location**: `crates/network/src/service/swarm_events.rs:132-148`, `crates/network/src/service/swarm_loop.rs:73-79`
- **Recommendation**: For testnet (conn_limit=50), reduce bootstrap_slots to 3-5. The current default of 10 is too high relative to the 50-connection budget. The env var `DOLI_BOOTSTRAP_SLOTS` already exists.

### Medium

#### F5: OS TCP Count vs libp2p Established Count -- Measurement Discrepancy

- **Issue**: The investigation used `lsof | grep TCP` to measure connections, reporting 4,869 to 8,349 TCP sockets across 156 nodes (53.5 avg). The libp2p `num_established()` counter reports only connections that have completed the TCP + Noise + Yamux handshake. The OS `lsof` count includes:
  - ESTABLISHED (= libp2p established)
  - TIME_WAIT (closed connections lingering 2x MSL, typically 60s on macOS)
  - CLOSE_WAIT (half-closed, waiting for application close)
  - SYN_SENT / SYN_RECV (pending connections)
  - FIN_WAIT_1/2 (closing)
- **Impact**: On a 156-node localhost network with 24h connection timeout, there is minimal connection churn, so TIME_WAIT/CLOSE_WAIT should be small. However, eviction cooldowns (every node cycling at the peer table boundary) generate ~1 disconnection per second per node, each leaving a TIME_WAIT socket for 60s. At steady state: 156 nodes x ~1 eviction/s x 60s TIME_WAIT / 156 nodes = ~60 extra sockets per node from TIME_WAIT alone. This could explain why `lsof` shows 53.5 avg when libp2p `num_established()` might show ~40-45.
- **Location**: Observation methodology, not code
- **Recommendation**: When measuring connection counts, use the `[MEM-CONN]` milestone log (which reports `num_established()` from `conn_counters`) rather than `lsof`. Alternatively, filter lsof to `ESTABLISHED` state only: `lsof -i TCP -sTCP:ESTABLISHED`.

#### F6: Relay Client Transport Composed But Not Used -- Holds Resources

- **Issue**: At `crates/network/src/service/mod.rs:100-104`, every node creates a relay client transport (`relay::client::new()`) and composes it with the base TCP transport via `.or_transport()`. The relay server is also instantiated for every node (`mod.rs:162-178`), even when `enable_relay_server=false` (set to 0 reservations/0 circuits).
- **Impact**: The relay client behaviour (`relay::client::Behaviour`) is included in the `DoliBehaviour` struct and participates in the swarm event loop. On localhost devnet, relay is never used (all nodes are directly reachable), but the behaviour still processes events. The relay server with max_reservations=0 correctly rejects all reservations, so it does not hold connection resources. The relay client transport is composed with `.or_transport()` which has near-zero overhead when not matching any addresses.
- **Assessment**: **Not a connection overflow contributor**. The relay transport does not create additional connections -- it only provides an alternative dialing path for `/p2p-circuit` addresses. Since no nodes on devnet use circuit addresses, the relay transport handles zero connections.
- **Location**: `crates/network/src/service/mod.rs:100-104`, `crates/network/src/transport.rs:44-61`
- **Recommendation**: No action needed for connection analysis. For cleanliness, relay client/server could be made optional for devnet, but this is cosmetic.

#### F7: Pending Connection Limits Are Set But Generous

- **Issue**: `with_max_pending_incoming(Some(pending_limit))` and `with_max_pending_outgoing(Some(pending_limit))` are correctly set at `mod.rs:158-159`, with `pending_limit = max_peers` (150 for devnet). Each pending connection allocates TCP socket + partial Noise/Yamux buffers (~256KB-1MB) before either completing or timing out.
- **Impact**: Up to 150 inbound + 150 outbound = 300 pending connections could exist simultaneously, consuming up to 300MB in handshake buffers. In practice, on localhost with sub-ms latency, handshakes complete in <100ms, so the steady-state pending count is low. But during initial network formation (all 156 nodes starting within seconds), a burst of ~150 concurrent pending connections per node is plausible.
- **Location**: `crates/network/src/service/mod.rs:149-152`
- **Recommendation**: For devnet stress tests, reduce pending_limit to 20-30 via `DOLI_PENDING_LIMIT=30`. The formula `pending_limit = max_peers` was designed for mainnet where max_peers=50, not devnet where max_peers=150.

### Low / Informational

#### F8: Eviction Cooldown Creates Steady-State Connection Churn

- **Issue**: At `swarm_events.rs:82-92`, peers evicted from the peer table are put on a 120-second cooldown. During this window, if the evicted peer reconnects (which it will, because Kademlia re-discovers us within 60s), the connection is immediately disconnected. This creates a steady-state churn pattern: connect -> evict -> cooldown -> reconnect -> immediate disconnect -> TIME_WAIT.
- **Impact**: Each churn cycle creates a TIME_WAIT socket (60s on macOS). With ~100+ evicted peers cycling, this adds ~60-100 phantom TCP sockets per node visible in `lsof` but not in libp2p counters. This is a secondary contributor to the observed lsof vs expected mismatch.
- **Location**: `crates/network/src/service/swarm_events.rs:82-92`
- **Recommendation**: The eviction cooldown mechanism is correct and necessary. The churn is a natural consequence of having more nodes than max_peers. The fix is not to change eviction logic but to reduce max_peers so the network stabilizes without constant eviction.

#### F9: Gossipsub mesh_n=12 with max_peers=150 Creates Overhead

- **Issue**: Devnet gossipsub parameters are mesh_n=12, mesh_n_low=8, mesh_n_high=24 (`defaults.rs:215-218`). With 150 peers, only 12 are in the eager-push mesh. The remaining 138 receive blocks via IHAVE/IWANT (lazy gossip), adding 1-2 heartbeat intervals (1-2s) latency. This is acceptable for devnet.
- **Impact**: Not a connection issue -- gossipsub does not create additional connections. All gossipsub messages flow over existing yamux streams within established connections.
- **Location**: `crates/core/src/network_params/defaults.rs:215-218`
- **Recommendation**: No action needed for connection analysis.

---

## Root Cause Summary

The connection limit is **not being bypassed**. The observed behavior is explained by:

| Factor | Contribution to Observed Behavior |
|--------|-----------------------------------|
| Devnet max_peers=150 -> conn_limit=300 | **Primary**: The limit is 300, not 50. The code works correctly -- the limit is just too high for 156-node stress testing on a single machine |
| OS lsof vs libp2p num_established | **Secondary**: lsof counts TIME_WAIT, CLOSE_WAIT, SYN_SENT etc. Inflates observed count by ~10-20% |
| Eviction churn -> TIME_WAIT sockets | **Tertiary**: Steady-state eviction at the peer boundary creates ~60-100 phantom sockets per node |
| Bootstrap-only peers | **Minor**: 10-20 connection slots consumed transiently |
| Pending connections during startup | **Minor**: Burst of ~150 pending connections during network formation, resolving within seconds |

---

## Connection Budget Analysis

### Per-Connection RAM Cost

| Component | Size | Notes |
|-----------|------|-------|
| Yamux receive window | 256KB | Configured in transport.rs |
| Yamux send buffer | ~64KB | Internal yamux buffering |
| Noise session state | ~4KB | Post-handshake symmetric keys + counters |
| GossipSub per-peer state | ~2KB | Mesh membership, score, message cache refs |
| Kademlia routing entry | ~0.5KB | If peer is in routing table |
| Request-response state | ~1KB | Per-protocol connection handler |
| TCP socket buffers (OS) | ~128KB | macOS default: 64KB send + 64KB recv |
| **Total per connection** | **~456KB** | Conservative estimate |

### Budget for 100-Node Devnet (64GB Mac)

| max_peers | conn_limit | Conns/node (steady) | RAM/node (yamux+overhead) | Total RAM (100 nodes) | Verdict |
|-----------|------------|---------------------|---------------------------|----------------------|---------|
| 150 | 300 | ~99 (all peers) | 99 x 456KB = 44MB | 4.4GB | OK but tight with other overhead |
| 50 | 100 | ~50 | 50 x 456KB = 22MB | 2.2GB | Safe |
| 25 | 50 | ~25 | 25 x 456KB = 11MB | 1.1GB | Comfortable |

### Budget for 150-Node Devnet (64GB Mac)

| max_peers | conn_limit | Conns/node (steady) | RAM/node (yamux+overhead) | Total RAM (150 nodes) | Verdict |
|-----------|------------|---------------------|---------------------------|----------------------|---------|
| 150 | 300 | ~149 | 149 x 456KB = 66MB | 9.9GB yamux + ~20GB other = ~30GB | EXCEEDS budget |
| 50 | 100 | ~50 | 50 x 456KB = 22MB | 3.3GB yamux + ~12GB other = ~15GB | Tight but feasible |
| 25 | 50 | ~25 | 25 x 456KB = 11MB | 1.7GB yamux + ~8GB other = ~10GB | Safe |
| 15 | 30 | ~15 | 15 x 456KB = 7MB | 1.0GB yamux + ~6GB other = ~7GB | Comfortable |

### Budget for 200-Node Devnet (64GB Mac)

| max_peers | conn_limit | Conns/node (steady) | RAM/node (yamux+overhead) | Total RAM (200 nodes) | Verdict |
|-----------|------------|---------------------|---------------------------|----------------------|---------|
| 150 | 300 | ~150+ (eviction churn) | 150 x 456KB = 67MB | 13.4GB yamux + ~30GB other = ~43GB | EXCEEDS |
| 50 | 100 | ~50 | 50 x 456KB = 22MB | 4.4GB yamux + ~16GB other = ~20GB | Feasible |
| 25 | 50 | ~25 | 25 x 456KB = 11MB | 2.2GB yamux + ~10GB other = ~12GB | Safe |
| 15 | 30 | ~15 | 15 x 456KB = 7MB | 1.4GB yamux + ~8GB other = ~9.4GB | Comfortable |

**"Other" includes**: Per-node base overhead (~40MB for RocksDB + mempool + UTXO set + block store + runtime), gossipsub message caches (duplicate_cache_time=60s, ~5MB at block rate), Kademlia routing table (K=20 buckets x 256 entries), channel buffers (event_buf + command_buf x ~1KB per message).

---

## Recommended Devnet max_peers Values

| Node Count | Recommended max_peers | conn_limit | pending_limit | Est. Total RAM | Rationale |
|------------|----------------------|------------|---------------|----------------|-----------|
| 10-50 | 50 (current default would be fine if overridable) | 100 | 50 | <4GB | Full mesh, all peers connected |
| 50-100 | 30 | 60 | 30 | ~8GB | Each node connects to ~30 peers. Gossip mesh_n=12 fits. 3-hop propagation |
| 100-150 | 20 | 40 | 20 | ~10GB | Tighter budget. mesh_n=12 still works. 4-hop propagation. Eviction churn moderate |
| 150-200 | 15 | 30 | 15 | ~9GB | Minimum viable. mesh_n=12 > max_peers, so gossipsub auto-caps. 5-hop propagation |

**Key constraint**: gossipsub mesh_n must be <= max_peers. With devnet mesh_n=12, max_peers >= 12 is required. Going below 12 would require reducing mesh_n, which degrades block propagation latency.

**Recommendation for INC-I-014 stress testing**: Set devnet `max_peers = 25` (matching testnet). This gives conn_limit=50, which at 156 nodes means ~9GB total RAM -- well within the 64GB budget. Apply via new `DOLI_MAX_PEERS` env var to avoid code changes between test iterations.

---

## Security Posture Summary

| Attack Vector | Protection | Status |
|---------------|-----------|--------|
| Eclipse attack | Eviction prefers low-gossipsub-score peers; producer peers protected | Partial -- no ASN/subnet diversity (all localhost) |
| Sybil attack | Bond-based identity (on-chain); GSet CRDT requires signed announcements | Protected (on-chain) |
| Routing table poisoning | Kademlia liveness checks; dead-peer backoff; 5-failure eviction | Protected |
| Gossip mesh manipulation | Topic scoring on blocks/txs/producers; ip_colocation_factor_threshold=500 | Protected |
| Message suppression | mesh_n=12 redundant paths; IHAVE/IWANT fallback; gossip_factor=0.5 | Protected |
| DoS / resource exhaustion | Connection limits (pending + established); rate limiter per peer | **Vulnerable** -- max_peers=150 for devnet allows resource exhaustion on single-machine deployments |
| Connection exhaustion | with_max_established + with_max_pending_* | Protected -- limits enforced correctly |
| NAT traversal abuse | Relay server max_reservations=0 for non-relay nodes | Protected |

---

## Architecture Recommendations

### Immediate (INC-I-014 resolution)

1. **Add `DOLI_MAX_PEERS` env var** -- mirrors the pattern of `DOLI_CONN_LIMIT`, `DOLI_PENDING_LIMIT`, `DOLI_BOOTSTRAP_SLOTS`. Apply in `bins/node/src/config.rs` or `startup.rs`. This unblocks stress test iteration without rebuilds.

2. **Reduce devnet max_peers default from 150 to 30** -- 150 was set for "100+ nodes stress tests" but the math shows it causes RAM explosion at exactly that scale. A node with max_peers=30 on a 150-node network connects to 30 peers (20% of the network), which is healthy topology. Gossipsub mesh_n=12 fits within 30.

3. **Add `DOLI_IDLE_TIMEOUT_SECS` env var** -- defaulting to 86400 for mainnet, 3600 for testnet, 300 for devnet. Shorter idle timeouts for devnet allow natural connection cleanup during stress tests.

### Medium-term

4. **Log ConnectionLimits denials** -- when a connection is denied by ConnectionLimits (pending or established), libp2p emits `IncomingConnectionError` or `OutgoingConnectionError` with a specific `ConnectionDenied` error. Currently these are handled in the catch-all `_ => {}` arm at `swarm_events.rs:308`. Add explicit logging for ConnectionDenied events to make limit enforcement visible in logs.

5. **Periodic connection budget log** -- alongside the existing `[MEM-CONN]` milestone logs, add a 60s periodic log that reports: established_in, established_out, pending_in, pending_out, bootstrap_peers, eviction_cooldown.len(). This makes steady-state connection distribution visible without requiring `lsof`.

### Not Recommended

- **Do NOT add custom Transport wrappers** -- per the existing analysis in `docs/redesigns/transport-connection-limits-analysis.md`, the library provides all needed controls.
- **Do NOT add RSS-based admission controllers** -- reactive, platform-specific, and unnecessary given proactive limits.
- **Do NOT reduce yamux window below 256KB** -- already at minimum viable for sync protocol (~100KB header batches need 2.5x headroom for flow control). Further reduction risks sync stalls.

---

## Next Steps

1. **To add DOLI_MAX_PEERS env var**: `/omega-improve --scope=bins/node/src/config.rs` -- add env var override in `NodeConfig::for_network()`, pattern: `std::env::var("DOLI_MAX_PEERS").ok().and_then(|v| v.parse().ok()).unwrap_or(network.params().max_peers)`
2. **To reduce devnet max_peers default**: `/omega-improve --scope=crates/core/src/network_params/defaults.rs` -- change devnet `max_peers: 150` to `max_peers: 30`
3. **To add idle timeout env var**: `/omega-improve --scope=crates/network/src/service/mod.rs` -- replace hardcoded `Duration::from_secs(86400)` with env-configurable value
4. **To add connection budget logging**: `/omega-improve --scope=crates/network/src/service/swarm_loop.rs` -- add 60s periodic log for connection counters
5. **To re-run stress test with corrected params**: `DOLI_MAX_PEERS=25 DOLI_PENDING_LIMIT=25 DOLI_IDLE_TIMEOUT_SECS=300` (after implementing env vars)

---

## Appendix: libp2p Version Inventory

| Crate | Version | Notes |
|-------|---------|-------|
| libp2p | 0.53.2 | Umbrella crate |
| libp2p-connection-limits | 0.3.1 | ConnectionLimits behaviour -- confirmed no bypass in established limit checks |
| libp2p-swarm | 0.44.2 | Swarm runtime |
| libp2p-yamux | 0.45.2 | Multiplexer -- 256KB configured window |

## Appendix: ConnectionLimits Verification

Verified from `libp2p-connection-limits 0.3.1` source:

- `handle_established_inbound_connection()` checks THREE limits sequentially: max_established_incoming, max_established_per_peer, max_established_total (sum of in + out). ALL must pass.
- `handle_established_outbound_connection()` checks THREE limits: max_established_outgoing, max_established_per_peer, max_established_total. ALL must pass.
- `handle_pending_inbound_connection()` checks max_pending_incoming BEFORE Noise/Yamux allocation.
- `handle_pending_outbound_connection()` checks max_pending_outgoing BEFORE dial proceeds.
- `ConnectionDenied` immediately closes the connection. The `SwarmEvent::ConnectionEstablished` event is NOT emitted for denied connections.
- The `check_limit()` function compares `current >= limit` and returns `ConnectionDenied` when exceeded. No bypass path exists.

**Conclusion**: ConnectionLimits works as documented. The observed TCP overshoot is explained by the limit being set to 300 (devnet), not 50 (testnet), combined with OS-level socket state accounting differences.
