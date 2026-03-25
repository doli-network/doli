# P2P Network Advisory: INC-I-012 -- Post-Snap Header Deadlock + 50-Node Sync Hardening

## Task Type
Security Audit + Architecture Assessment + Code Review (combined)

## Protocol Stack Map

| Layer | Implementation | Library/Custom | Status |
|-------|---------------|----------------|--------|
| Transport | TCP (tokio) | library (libp2p) | OK |
| Security | Noise XX (libp2p default) | library | OK |
| Multiplexing | Yamux (libp2p default) | library | CONCERN -- INC-I-009 buffer explosion at scale |
| Discovery | Kademlia DHT + bootstrap nodes | library (libp2p) | OK |
| Gossip | GossipSub v1.1 with peer scoring | library (libp2p) | OK -- scoring configured |
| NAT Traversal | AutoNAT + DCUtR + Relay | library (libp2p) | OK -- all layers present |
| Peer Scoring | GossipSub peer scoring (blocks topic) | library | CONCERN -- scoring only on blocks topic |
| Sync Protocol | Custom request-response (header-first + snap) | custom | CONCERN -- multiple issues |
| Fork Choice | Weight-based with deterministic tie-break | custom | OK -- recently fixed |
| Connection Mgmt | Connection limits + eviction cooldown | library + custom | CONCERN -- see F3 |
| Rate Limiting | Token bucket (per-peer) + global sync cap | custom | CONCERN -- see F6 |

---

## Findings

### Critical

#### F1: Missing `GetHeadersByHeight` -- Post-Snap Header Deadlock Remains Unresolved

- **Issue**: After snap sync, the node's `local_hash` may reference a block hash that no peer has on their canonical chain (because the snapshot came from a forked peer that has since reorganized). The `GetHeaders` protocol message (line 24-29 of `protocols/sync.rs`) takes `start_hash` as input. The server-side handler (line 496-535 of `validation_checks.rs`) looks up `start_hash` via `get_height_by_hash()`. If the hash is unknown, it returns an empty `Headers` response. The client interprets this as fork evidence, increments `consecutive_empty_headers`, and eventually triggers genesis resync -- which may snap sync to the same forked peer again, creating an infinite loop.

- **Impact**: Permanent node stuckness after snap sync. The node cycles: snap sync -> get unknown hash -> empty headers -> genesis resync -> snap sync. This is the root cause described in INC-I-012 Problem 1.

- **Location**: `crates/network/src/protocols/sync.rs:22-29` (protocol definition), `bins/node/src/node/validation_checks.rs:511-534` (server handler), `crates/network/src/sync/manager/sync_engine.rs:628-734` (client handler)

- **Protocol Design Review for `GetHeadersByHeight` Fix**:

  The recommended fix is to add a `GetHeadersByHeight { height: u64, max_count: u32 }` message to the sync protocol. This is **sound** with the following caveats:

  **Correctness**: Height-based header requests solve the deadlock because after snap sync the node knows its `local_height` (from the snapshot's `block_height`) even if its `local_hash` is unrecognizable. The server can serve canonical headers starting from `height+1` regardless of what hash the client thinks is at that height. The response provides the client with a canonical hash to anchor its chain.

  **Edge Cases**:
  1. **Height ambiguity during reorgs**: Two peers may have different blocks at the same height. This is not a problem because the requesting node will validate the header chain linkage -- if the returned headers do not chain to the node's state, it can detect and retry. The header chain must be validated by the client anyway.
  2. **Race: peer reorgs between request and response**: The server could reorg while serving. This is the same race that exists with `GetHeaders` today and is handled by the client's chain linkage validation.
  3. **Amplification concern**: Without `start_hash` validation, an attacker could request headers at arbitrary heights to probe chain structure. This is minimal risk since `GetBlockByHeight` already exists in the protocol (line 38-41 of `protocols/sync.rs`) and serves the same purpose. The existing rate limiter (`MAX_SYNC_REQUESTS_PER_INTERVAL=8`) bounds amplification.

  **Race Condition**: There is a subtle race in the snap sync flow. Between `take_snap_snapshot()` (which sets `local_hash` to the snapshot's `block_hash`) and the first `GetHeaders` request, the node's `local_hash` is set but may be unrecognizable. With `GetHeadersByHeight`, the node would instead use `local_height` for the first request, which is always valid. However, the node must update `local_hash` to the hash returned by the first header's `prev_hash` to maintain chain linkage consistency. This requires a small state machine change in the header download logic.

  **Protocol Versioning**: Adding `GetHeadersByHeight` to the `SyncRequest` enum is a **breaking protocol change** if peers running the old version receive the new message type. Since `SyncCodec` uses bincode deserialization (`protocols/sync.rs:202`), an unknown variant will fail deserialization, causing the peer to close the stream. **Recommendation**: The `SyncResponse::Error` variant already exists. New nodes should send `GetHeadersByHeight` and fall back to `GetHeaders` if they receive an `Error` response or deserialization failure. Old nodes will reject the unknown variant at the codec level (IO error, not `Error` response), so the fallback must handle both cases.

- **Recommendation**: Implement `GetHeadersByHeight` with: (1) protocol fallback for backward compatibility, (2) client-side chain linkage validation on the response, (3) `local_hash` update from the response's first header. The sync engine should attempt `GetHeadersByHeight(local_height, max_count)` first after snap sync, and fall back to `GetHeaders(local_hash, max_count)` on error.

- **Action**: `/omega-new-feature` to add `GetHeadersByHeight` to the sync protocol.

---

### High

#### F2: `SyncResponse::Error` Handler Unconditionally Calls `handle_snap_download_error` -- Wrong for Non-Snap Contexts

- **Issue**: In `sync_engine.rs:583-586`, the `SyncResponse::Error` handler calls `self.handle_snap_download_error(peer)` regardless of the current sync phase. This means that a "busy" error from the P5 rate limiter fix, which is a normal header-first sync response, is routed to snap sync error handling. If the node is in `SnapDownloading` state, this blacklists the peer and tries alternates. If NOT in `SnapDownloading`, the method is a no-op (guarded by the `if !matches!` check at line 207 of `snap_sync.rs`), so the error is silently swallowed.

- **Impact**: When a node in header-first sync receives "busy: sync serving limit reached", the error is silently dropped. The peer's `pending_request` was cleared by `handle_response()`, but no retry logic is triggered. The node must wait for the next `cleanup()` tick (up to 30s) to retry. This partially negates the benefit of the P5 "busy" fix -- the requester knows the peer is busy but does nothing about it.

- **Location**: `crates/network/src/sync/manager/sync_engine.rs:583-586`

- **Recommendation**: The `SyncResponse::Error` handler should check the error message and current sync phase. For "busy" errors during header-first sync, the node should immediately try a different peer rather than waiting for the next cleanup tick. For snap sync errors, the current behavior is correct. The handler should be:
  ```
  SyncResponse::Error(err) => {
      if pipeline_data is SnapDownloading -> handle_snap_download_error(peer)
      else if err contains "busy" -> mark peer as temporarily busy, try next peer
      else -> log and reset to Idle for retry
  }
  ```

- **Action**: `/omega-improve --scope=networking`

#### F3: Bootstrap Connection Headroom Formula Creates Oscillation Under Churn

- **Issue**: The connection headroom formula (`service/mod.rs:137`) is `max_peers * 1.5 + 20`, which at `max_peers=50` gives 95. This is the max for BOTH incoming AND outgoing established connections independently (`with_max_established_incoming` and `with_max_established_outgoing`). Combined with the eviction logic (`swarm_events.rs:76-112`) which evicts at `peers.len() >= max_peers` (50), there is a 45-connection gap between the peer table eviction threshold (50) and the libp2p connection limit (95). This means libp2p will accept up to 95 inbound connections, but the peer table will immediately evict the 51st peer, triggering disconnect -> reconnect -> evict cycles. The 30s eviction cooldown (`swarm_events.rs:64`) mitigates this but only for the specific evicted peer. With 100+ nodes all trying to connect, different peers cycle through eviction.

- **Impact**: Under 50+ node stress, the node enters a steady state of accept-evict-disconnect that wastes CPU on connection setup/teardown and gossipsub mesh churn. The 30s cooldown prevents individual peer thrashing but not aggregate churn. At 106 nodes (the reported scale), 56 nodes are perpetually cycling through the eviction queue.

- **Location**: `crates/network/src/service/mod.rs:137-141` (headroom), `crates/network/src/service/swarm_events.rs:76-112` (eviction)

- **Recommendation**: Two complementary changes: (1) Reduce `max_established_incoming` to `max_peers + 10` (60) instead of 95. The extra 10 is enough for simultaneous-dial races and relay connections. This reduces the eviction pressure zone from 45 connections to 10. (2) Add a `pending_connections` check before accepting new inbound -- if the peer table is full AND the inbound peer is not a known producer, reject at the connection level rather than accepting and then evicting. This is a libp2p `ConnectionDenied` approach via the `connection_limits` behaviour, which is cheaper than accepting the full handshake and then disconnecting.

- **Action**: `/omega-improve --scope=networking`

#### F4: Snap Sync Quorum Cap of 15 Is Too High for Fast-Moving Chains

- **Issue**: The quorum calculation (`snap_sync.rs:73`) uses `min(total_peers / 2 + 1, 15)`. With 50 peers, quorum is `min(26, 15) = 15`. The `root_timeout` is 15 seconds. On an active chain producing blocks every 10 seconds, peers advance 1-2 blocks during the voting window. Votes are grouped by `state_root` only (not height), so peers at different heights but with the same root form a group. However, if more than 1 block is produced during the window (which is likely with 15s timeout), the state root changes and groups fragment.

  The batch-send optimization (`next_snap_requests()` at line 427) helps but does not eliminate the problem: even with simultaneous requests, responses arrive over 1-3 seconds. With 10s block time, 15% of responses will be at a different height than the majority.

  Reaching quorum of 15 requires at least 15 peers to respond at the same state root within 15 seconds. At 50 peers, this means 30% response rate at the same state root. With 10s block intervals and 1-3s response jitter, the probability of exactly matching is low. The fallback (line 92 of `cleanup.rs`) picks the best group with >= 2 peers, which is effectively what always happens.

- **Impact**: Snap sync almost never reaches true quorum. It always falls through to the timeout handler's best-effort group selection with >= 2 peers. The quorum mechanism provides false security -- the real protection is `compute_state_root_from_bytes()` verification after download, as the comment correctly notes.

- **Location**: `crates/network/src/sync/manager/snap_sync.rs:73`

- **Recommendation**: Reduce the quorum cap to `min(total_peers / 2 + 1, 5)`. 5 is sufficient to prevent single-peer poisoning while being achievable within the 15s window. Alternatively, accept that the quorum is a best-effort selection mechanism (not a security mechanism) and lower the timeout to 5s with quorum of 3. The state root recomputation after download is the actual security layer.

- **Action**: `/omega-improve --scope=networking`

#### F5: Equal-Weight Tie-Breaking in `check_reorg_weighted` Has Correct Logic but Inconsistent Callers

- **Issue**: The tie-breaking logic in `check_reorg_weighted()` (`reorg.rs:276-288`) is correct: when `new_chain_weight == current_chain_weight`, the fork block wins if `block_hash < current_tip`. This is deterministic and ensures all nodes converge. The separate method `should_reorg_by_weight_with_tiebreak()` (line 201-211) implements the same logic identically. Both are correct.

  However, there is a consistency concern: `should_reorg_by_weight()` (line 190-193) does NOT include tie-breaking. If any caller uses `should_reorg_by_weight()` instead of the tiebreak variant, ties will always favor the incumbent chain, recreating the original fork storm. Currently `should_reorg_by_weight()` is not used in the hot path (it is standalone and not called from `check_reorg_weighted`), but its existence is a maintenance hazard.

- **Impact**: Low immediate risk (the dead `should_reorg_by_weight` is not called in the reorg path). High maintenance risk -- a future caller using the wrong method would reintroduce fork storms.

- **Location**: `crates/network/src/sync/reorg.rs:190-193` (no tiebreak), `reorg.rs:201-211` (with tiebreak), `reorg.rs:276-288` (inline in check_reorg_weighted)

- **Recommendation**: Mark `should_reorg_by_weight()` as `#[deprecated(note = "Use should_reorg_by_weight_with_tiebreak or check_reorg_weighted")]` to prevent accidental use. Better: remove the method entirely and keep only the tiebreak variant.

- **Action**: `/omega-improve --scope=networking`

---

### Medium

#### F6: Global Sync Serving Rate Limit (`MAX_SYNC_REQUESTS_PER_INTERVAL=8`) Is Too Restrictive and Interval Is Unclear

- **Issue**: The rate limiter at `event_loop.rs:526` caps sync responses at 8 per "interval". The counter `sync_requests_this_interval` is reset somewhere per production interval (slot). With a 10-second slot, this means 8 sync requests per 10 seconds = 0.8 req/sec aggregate. When serving 50 syncing peers, each peer gets on average 0.16 requests served per slot. At 500 headers per response, a peer syncing 60K blocks needs 120 requests = 1500 seconds (25 minutes) of continuous request-response cycles. But with 50 peers competing for 8 slots per interval, each peer's effective throughput is 120 / 0.16 = 750 slots = 7500 seconds (2 hours+).

  The P5 fix (sending "busy" instead of dropping) helps the requester retry with a different peer, but the AGGREGATE serving capacity of the network is still capped at 8 req/slot per serving node.

- **Impact**: Sync throughput is severely constrained in large networks. The rate limit was necessary to prevent event loop starvation from disk I/O, but the value (8) was calibrated for the pre-P3 single-request-per-cycle model. With the P3 `while let` loop improvement that processes headers 4-8x faster, the node can handle more sync serving.

- **Location**: `bins/node/src/node/event_loop.rs:526`

- **Recommendation**: (1) Increase `MAX_SYNC_REQUESTS_PER_INTERVAL` to 16-24 to reflect the P3 throughput improvements. (2) Make the limit configurable via `NetworkParams` so seed nodes (which primarily serve sync requests) can have a higher limit. (3) Distinguish between lightweight requests (GetHeaders returns ~500 headers, small) and heavyweight requests (GetBodies returns full blocks, GetStateSnapshot returns entire state). Headers should have a higher limit than bodies.

- **Action**: `/omega-improve --scope=networking`

#### F7: Peer Blacklisting for Empty Headers Has Correct Escalation But Wrong Initial Response

- **Issue**: When a peer returns empty headers, the sync engine (`sync_engine.rs:668-727`) follows this escalation path:
  1. First 1-2 empties: blacklist the responding peer for 30s (line 715-726)
  2. 3+ empties: clear ALL blacklists and signal fork (line 698-712)

  The initial blacklisting (step 1) is wrong in the post-snap context. After snap sync, the node's `local_hash` is unrecognizable to ALL canonical peers. Blacklisting the first responder doesn't help -- ALL peers will return empty. The node must blacklist 2 peers before step 2 activates, wasting 60+ seconds (2 blacklist cycles with 30s cooldown).

  The code at line 650-665 handles the post-snap case separately (via `AwaitingCanonicalBlock` check), which is correct. But this only activates if `recovery_phase` is `AwaitingCanonicalBlock`. If the recovery phase has already transitioned to `Normal` (via the 60s timeout in cleanup), the post-snap interception misses, and the node falls through to the generic blacklisting path.

- **Impact**: After snap sync to a forked hash, the node wastes 60-120 seconds blacklisting canonical peers before recognizing it is on a fork. With `GetHeadersByHeight` (F1), this becomes irrelevant because the first request would succeed. Without F1, this slows recovery.

- **Location**: `crates/network/src/sync/manager/sync_engine.rs:650-727`

- **Recommendation**: The first empty headers response after snap sync should NOT blacklist the peer. Instead, check if the node recently completed snap sync (within last 5 minutes, regardless of current `recovery_phase`) and skip blacklisting. This is a defense-in-depth measure alongside F1.

- **Action**: `/omega-improve --scope=networking`

#### F8: GossipSub Peer Scoring Only Covers Blocks Topic -- Missing Transaction and Producer Topics

- **Issue**: Peer scoring (`gossip/config.rs:98-108`) only defines `TopicScoreParams` for `BLOCKS_TOPIC`. The `TRANSACTIONS_TOPIC`, `PRODUCERS_TOPIC`, `HEARTBEATS_TOPIC`, and `VOTES_TOPIC` have no topic-specific scoring. This means:
  1. Peers that flood transactions cannot be scored down per-topic
  2. Peers that send invalid producer announcements face no scoring penalty
  3. The gossipsub mesh for non-block topics is purely random with no quality signal

  The `PRODUCERS_TOPIC` is particularly concerning because producer announcements are the GSet protocol that caused O(N*mesh_n) gossip flooding at 106 nodes.

- **Impact**: Medium. The block topic scoring works correctly and is the highest-bandwidth topic. Transaction and producer topic flooding is bounded by the per-peer rate limiter in `rate_limit.rs`. However, the absence of topic scoring on `PRODUCERS_TOPIC` means mesh manipulation attacks (GRAFT flooding to force mesh membership) on the producer topic are not penalized.

- **Location**: `crates/network/src/gossip/config.rs:98-108`

- **Recommendation**: Add `TopicScoreParams` for at least `TRANSACTIONS_TOPIC` and `PRODUCERS_TOPIC` with `invalid_message_deliveries_weight` configured to penalize peers that deliver invalid messages. For `PRODUCERS_TOPIC`, set `mesh_message_deliveries_weight` to prefer peers that deliver unique producer announcements (similar to the first-message-deliveries scoring on blocks).

- **Action**: `/omega-improve --scope=networking`

#### F9: Message ID Function Uses `DefaultHasher` -- Non-Deterministic Across Platforms

- **Issue**: The gossipsub message ID function (`gossip/config.rs:60-64`) uses `std::collections::hash_map::DefaultHasher` to hash message data. `DefaultHasher` is documented as NOT guaranteed to produce the same output across Rust versions or platforms. If two nodes running different Rust versions or platforms (x86 vs ARM) compute different message IDs for the same message, gossipsub will treat them as different messages, causing duplicate delivery and breaking deduplication.

- **Impact**: Currently low risk because all nodes compile from the same source with the same Rust toolchain. High risk if the network expands to heterogeneous infrastructure (different OS, architecture, or Rust versions).

- **Location**: `crates/network/src/gossip/config.rs:60-64`

- **Recommendation**: Replace `DefaultHasher` with a deterministic hash function. The standard approach in gossipsub implementations is to use `sha256(message.data)` truncated to 20 bytes, or `blake3(message.data)` since the project already uses BLAKE3. This ensures message IDs are platform-independent.

- **Action**: `/omega-improve --scope=networking`

#### F10: `consensus_target_hash` Assumes Same Hash at Same Height -- Violation During Forks

- **Issue**: The `consensus_target_hash()` method (`sync_engine.rs:46-75`) groups peers by height and assumes "all honest peers at the same height MUST have the same hash (deterministic consensus)" (comment at line 52). This assumption is violated during active forks: two honest peers at the same height may have different blocks if the network has not yet converged. The method takes the hash from the first peer at the most-populated height, which could be either fork.

- **Impact**: During active forks (which are normal in a young network with equal-weight producers), `consensus_target_hash` may select a hash from the minority fork. Snap sync would then download state from the minority chain. The state root recomputation verification protects against corrupted state, but the node may need to re-snap after the fork resolves.

- **Location**: `crates/network/src/sync/manager/sync_engine.rs:46-75`

- **Recommendation**: Instead of using a single peer's hash as representative for a height, group peers by `(height, hash)` pairs. If no single `(height, hash)` pair has majority, fall back to header-first sync. This correctly handles the fork case where the same height has multiple valid hashes.

- **Action**: `/omega-improve --scope=networking`

---

### Low

#### F11: Fork Recovery Single-Peer Dependency

- **Issue**: `ForkRecoveryTracker` (`fork_recovery.rs:27-38`) requests all parent blocks from a single peer (`recovery.peer`). If that peer disconnects or is slow, the entire recovery stalls until the 120s timeout. There is no fallback to a different peer during an active recovery.

- **Impact**: Fork recovery for 10-50 block forks takes up to 120s when the initial peer is unreliable. In a 50-node network, the probability of peer disconnection during a 120s window is non-trivial.

- **Location**: `crates/network/src/sync/fork_recovery.rs:27-38`

- **Recommendation**: Add peer failover: if a block response does not arrive within 10s, cancel the pending request to the current peer, select a new peer, and continue from where the walk left off. The `blocks` vector already holds all collected blocks, so only the `peer` field and `pending` flag need updating.

- **Action**: `/omega-improve --scope=networking`

#### F12: Body Downloader Permanently Fails After 3 Attempts Per Hash

- **Issue**: `BodyDownloader` (`bodies.rs:254-259`) marks a hash as `permanently_failed` after 3 failed attempts, with no recovery path. If all 3 attempts happen to hit peers that genuinely don't have the block (e.g., snap-synced peers with no block history), the body is lost. The header chain becomes uncompletable, and the sync stalls.

- **Impact**: In mixed networks (some full-sync, some snap-synced nodes), body download can permanently fail for blocks that only the original full-sync peers have. The node must wait for the stuck-sync timeout (30-120s) to restart the entire sync pipeline.

- **Location**: `crates/network/src/sync/bodies.rs:254-259`

- **Recommendation**: Add `permanently_failed` expiry: after 60s, move permanently failed hashes back to the retry queue. This allows the body downloader to try peers that may have come online or synced the missing blocks in the interim.

- **Action**: `/omega-improve --scope=networking`

#### F13: `MAX_SYNC_SIZE` at 64MB Allows Large Allocation Attacks

- **Issue**: The sync codec (`protocols/sync.rs:18`) accepts messages up to 64MB. The `read_request` and `read_response` methods allocate `vec![0u8; len]` based on the length prefix, without additional validation. A malicious peer can send a 4-byte length prefix claiming 64MB, forcing the node to allocate 64MB. With 50 concurrent connections, this is 3.2GB of memory from length prefixes alone, before any actual data is sent.

- **Impact**: Memory exhaustion DoS. A single malicious peer can force 64MB allocation per stream. With Yamux multiplexing and default max_concurrent_streams, this could be amplified.

- **Location**: `crates/network/src/protocols/sync.rs:18, 198-203, 214-229`

- **Recommendation**: (1) Reduce `MAX_SYNC_SIZE` to 16MB for state snapshots, which is generous for any reasonable state size. For non-snapshot messages (GetHeaders, GetBodies), enforce a tighter limit of 2MB. (2) Add per-message-type size limits in the codec by peeking at the message type before allocating the full buffer. (3) Alternatively, use streaming deserialization instead of buffering the entire message.

- **Action**: `/omega-improve --scope=networking`

---

### Informational

#### F14: Idle Connection Timeout Set to 86400s (24 Hours)

- **Issue**: `service/mod.rs:191` sets `with_idle_connection_timeout(Duration::from_secs(86400))`. Idle connections that carry no gossipsub or sync traffic will remain open for 24 hours. In a 50-node network this is acceptable, but at scale (1000+ nodes), idle connections consume file descriptors and Yamux session state.

- **Location**: `crates/network/src/service/mod.rs:191`

- **Recommendation**: Reduce to 600s (10 minutes) for non-seed nodes. Seed nodes can keep the 24h timeout since they serve as rendezvous points.

#### F15: Dynamic Mesh Config Scaling May Overshoot at 50 Nodes

- **Issue**: `compute_dynamic_mesh` (`gossip/config.rs:23-51`) uses `sqrt(N) * 1.5` for networks > 20 peers, capped at 50. At 50 peers, this gives `mesh_n = ceil(sqrt(50) * 1.5) = ceil(10.6) = 11`, `mesh_n_low = 8`, `mesh_n_high = 16`. At 106 nodes (the stress test scale), `mesh_n = ceil(sqrt(106) * 1.5) = ceil(15.4) = 16`. With 16 mesh peers per topic and 7 topics, each node maintains 112 mesh relationships. The gossip overhead per message is O(mesh_n * topics), which at 16 * 7 = 112 messages per gossip round is significant.

- **Location**: `crates/network/src/gossip/config.rs:23-51`

- **Recommendation**: Consider per-topic mesh configuration. High-bandwidth topics (blocks, transactions) should have lower mesh_n (6-8) to reduce bandwidth. Low-bandwidth topics (producers, votes, heartbeats) can maintain higher mesh_n for reliability.

#### F16: Protocol Version `/doli/sync/1.0.0` Should Be Bumped When Adding `GetHeadersByHeight`

- **Issue**: When implementing F1, the sync protocol version should be bumped to `/doli/sync/1.1.0` to signal support for the new message type. Nodes running 1.0.0 that receive the new message will fail at the codec level. The new node should negotiate version support before sending the new message type.

- **Location**: `crates/network/src/protocols/sync.rs:15`

- **Recommendation**: Implement protocol version negotiation: (1) bump to `/doli/sync/1.1.0`, (2) track each peer's supported sync protocol version (from identify handshake), (3) only send `GetHeadersByHeight` to peers that advertise 1.1.0+.

---

## Architecture Recommendations

### 1. Sync Protocol Evolution Strategy

The current sync protocol mixes several concerns into a single request-response channel: header download, body download, block fetch, and state snapshot. This creates contention (a state snapshot transfer blocks the channel for seconds) and makes rate limiting imprecise (one limit for both lightweight header requests and heavyweight snapshot transfers).

**Recommendation for next major version**: Split into separate protocols:
- `/doli/headers/1` -- header chain download (lightweight, high-rate)
- `/doli/blocks/1` -- block body download (medium weight, parallel)
- `/doli/snap/1` -- state snapshot (heavyweight, rare, long-timeout)

This allows independent rate limiting, different timeouts, and independent protocol evolution. Not urgent for current scale but becomes necessary at 500+ nodes.

### 2. Fork Choice and Sync State Machine Coupling

The fork choice logic (`reorg.rs`) and the sync state machine (`manager/`) are tightly coupled through shared state (`consecutive_empty_headers`, `needs_genesis_resync`, `stuck_fork_signal`). The sync manager interprets empty headers as both "peer doesn't have our block" AND "we are on a fork", using the same counter for both. This conflation is the root cause of several incidents (INC-I-005, INC-I-012).

**Recommendation**: Separate "sync progress tracking" from "fork detection". Sync should track "can we make progress with peers?" (yes/no). Fork detection should track "do peers agree with our chain?" (via explicit chain comparison, not empty-header inference). The `GetHeadersByHeight` fix (F1) helps because height-based requests succeed even on forks, so empty responses become genuine "peer doesn't have data" rather than ambiguous "maybe fork, maybe not".

### 3. Snap Sync State Root Voting Is a Selection Mechanism, Not a Security Mechanism

The quorum-based state root voting (`snap_sync.rs`) is designed to select WHICH chain to download from, not to verify chain correctness. The actual security comes from `compute_state_root_from_bytes()` after download. The comment at line 72 correctly identifies this, but the code still uses a complex quorum formula with caps and minimums that suggest it's a security mechanism.

**Recommendation**: Simplify snap sync selection: instead of voting, use the peer with the highest height among the majority-height group (already computed by `consensus_target_hash`). Download from that peer, verify with `compute_state_root_from_bytes()`. If verification fails, try the next peer. This eliminates the voting round entirely, saving 15s per snap sync attempt.

---

## Security Posture Summary

| Attack Vector | Protection | Status |
|---------------|-----------|--------|
| Eclipse attack | Connection limits (max_peers=50), eviction cooldown (30s), Kademlia DHT diversity | Partial -- no ASN/subnet diversity enforcement on inbound connections |
| Sybil attack | Peer scoring on blocks topic, producer-aware eviction | Partial -- scoring only on 1 of 7 topics |
| Routing table poisoning | Kademlia (libp2p default protections) | Protected -- using standard libp2p Kademlia |
| Message manipulation | GossipSub message signing (Strict validation), header chain linkage | Protected |
| DoS / resource exhaustion | Per-peer rate limiter, global sync cap (8/interval), connection limits | Partial -- 64MB allocation attack vector (F13) |
| NAT traversal abuse | Relay rate limiting (max_reservations, max_circuits_per_peer) | Protected |
| Gossip mesh manipulation | GossipSub v1.1 peer scoring on blocks topic | Partial -- no scoring on 6 other topics (F8) |
| Sync protocol abuse | Rate limiter, request timeout (30s), busy response | Partial -- global cap too low (F6), allocation attack (F13) |
| Fork-based isolation | Weight-based fork choice with deterministic tiebreak, finality check | Protected -- recently hardened |
| Post-snap deadlock | Currently unprotected -- snap sync to forked hash = permanent stuck | Vulnerable -- F1 is the fix |

---

## Performance Notes

### Sync Throughput Analysis

- **Header download**: 500 headers per request, 1 request per peer per tick. With P3 `while let` loop, multiple requests chain within a single tick. Effective rate: ~500-2000 headers/second with 1 peer.
- **Body download**: 128 bodies per request, up to 8 concurrent requests (default `max_concurrent_body_requests`). With round-robin peer selection and 4 available peers, effective rate: ~32 bodies/second (128 * 4 / 16s response time).
- **Bottleneck**: The global sync serving cap (8 req/interval at 10s/slot) limits aggregate network sync throughput to 0.8 req/sec per serving node. In a 50-node network with 5 serving nodes, aggregate capacity is 4 req/sec. With 45 syncing nodes, each gets 0.089 req/sec = 44 headers/sec. Syncing 60K blocks takes 60000/44 = 1363 seconds = 22 minutes for headers alone.
- **Snap sync**: Eliminates header+body download for catch-up > 50 blocks. State snapshot size depends on UTXO set and producer set. Download is single-stream, bounded by the 60s timeout.

### GSet (Producer Announcement) Gossip Analysis

The producer announcements use gossipsub on `PRODUCERS_TOPIC`. At 106 nodes with `mesh_n=16`, each announcement is forwarded to 16 mesh peers. With N producers each announcing, the total messages per round is `N * mesh_n = N * 16`. At 106 nodes with 12 producers, this is 192 messages per round. The bloom filter delta optimization reduces per-announcement size, but the message count remains O(N_producers * mesh_n).

The reported "1200 messages/round" suggests either multiple topics are being counted or the announcements include full GSet state rather than deltas. The bloom filter approach (encoding producer set as a bloom and sending deltas) is correct but requires verification that the delta encoding is actually smaller than the full set for small producer counts.

---

## Next Steps

**Immediate (blocks further sync incidents)**:
1. `/omega-new-feature` -- Add `GetHeadersByHeight` to sync protocol (F1)
2. `/omega-improve --scope=networking` -- Fix `SyncResponse::Error` handler to be phase-aware (F2)

**Short-term (next deployment)**:
3. `/omega-improve --scope=networking` -- Reduce snap quorum cap to 5 (F4)
4. `/omega-improve --scope=networking` -- Increase `MAX_SYNC_REQUESTS_PER_INTERVAL` to 16-24 (F6)
5. `/omega-improve --scope=networking` -- Replace `DefaultHasher` in message ID (F9)
6. `/omega-improve --scope=networking` -- Reduce `MAX_SYNC_SIZE` and add per-type limits (F13)

**Medium-term (next stress test)**:
7. `/omega-improve --scope=networking` -- Fix connection headroom oscillation (F3)
8. `/omega-improve --scope=networking` -- Add topic scoring for PRODUCERS and TRANSACTIONS (F8)
9. `/omega-improve --scope=networking` -- Fix `consensus_target_hash` fork assumption (F10)
10. `/omega-improve --scope=networking` -- Deprecate/remove `should_reorg_by_weight` (F5)
