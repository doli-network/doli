# P2P Network Engineering Report: Header-First Sync Performance Analysis

**Date**: 2026-03-25
**Author**: P2P Network Engineer (read-only advisory)
**Branch**: fix/sync-state-explosion-root-causes
**Scope**: `crates/network/src/sync/`, `bins/node/src/node/{event_loop,periodic,block_handling,fork_recovery}.rs`
**Context**: INC-I-010 (fork-stuck recovery), INC-I-012 (86-node stress test)

---

## 1. Current Architecture Map

### 1.1 High-Level Flow

```
                           +--------------------------+
                           |      SyncManager         |
                           |  state: Idle/Syncing/    |
                           |         Synchronized     |
                           +-----------+--------------+
                                       |
              +------------------------+------------------------+
              |                        |                        |
    +---------v--------+    +----------v---------+    +---------v--------+
    | HEADER-FIRST     |    | SNAP SYNC          |    | FORK RECOVERY    |
    | (gap <= 50)      |    | (gap > 50 + 3 peers)|   | (orphan blocks)  |
    +------------------+    +--------------------+    +------------------+
    | 1. GetHeaders    |    | 1. GetStateRoot    |    | 1. Walk parents  |
    |    from best_peer|    |    to ALL peers     |    |    backward      |
    | 2. Validate chain|    | 2. Quorum vote     |    | 2. One block at  |
    | 3. GetBodies     |    | 3. GetStateSnapshot|    |    a time         |
    |    in parallel   |    |    from quorum peer |    | 3. Check store   |
    | 4. Apply blocks  |    | 4. Verify & apply  |    |    connection     |
    +------------------+    +--------------------+    +------------------+
```

### 1.2 Sync State Machine

The SyncManager has 3 states (simplified from former 10+ states):

- **Idle**: Not syncing. Evaluates `should_sync()` on peer updates and periodic cleanup.
- **Syncing { phase }**: Active sync with diagnostic phase labels:
  - `DownloadingHeaders` -- requesting headers from a single peer
  - `DownloadingBodies` -- parallel body download from multiple peers
  - `ProcessingBlocks` -- bodies downloaded, applying to chain
  - `SnapCollecting` -- collecting state root votes from peers
  - `SnapDownloading` -- downloading snapshot from quorum peer
- **Synchronized**: Caught up with network. Transitions to Idle if gap >= 3 blocks.

### 1.3 Header-First Sync Detailed Flow

**Phase 1: Header Download** (serial, single peer)

1. `start_sync()` selects `best_peer()` -- highest height, not blacklisted.
2. Transitions to `Syncing { DownloadingHeaders }`.
3. `next_request()` creates `GetHeaders { start_hash: local_hash, max_count: 500 }`.
4. ONE request in flight at a time per peer (guarded by `pending_request`).
5. `handle_headers_response()` validates chain linkage via `HeaderDownloader::process_headers()`.
6. Valid headers appended to `pending_headers` (VecDeque) and `headers_needing_bodies` (VecDeque<Hash>).
7. Continues requesting until empty response received.
8. Empty response transitions to `DownloadingBodies`.

**Phase 2: Body Download** (parallel, multiple peers)

1. `BodyDownloader::next_request()` batches up to `max_bodies_per_request=128` hashes.
2. Up to `max_concurrent_body_requests=8` in flight simultaneously.
3. Round-robin peer selection among "free" peers (no active request).
4. Peers filtered by height threshold (within 10% of best peer height).
5. Failed hashes go to retry queue; permanent failure after 3 attempts per hash.
6. Bodies stored in `pending_blocks: HashMap<Hash, Block>`.
7. When `headers_needing_bodies` empty, transitions to `ProcessingBlocks`.

**Phase 3: Block Application** (serial, ordered)

1. `get_blocks_to_apply()` walks `pending_headers` in order, matching bodies from `pending_blocks`.
2. Only extracts contiguous chain from local tip (breaks at first missing body or chain gap).
3. Each block applied via `apply_block()` with `ValidationMode::Light`.
4. `block_applied_with_weight()` updates local tip, checks sync completion.
5. When `local_height >= network_tip_height` and slot aligned, transitions to `Synchronized`.

### 1.4 Request Scheduling

Sync requests are issued from two locations:
- **`run_periodic_tasks()`** (every 1s production interval): calls `sm.next_request()` -- ONE request per tick.
- **`run_periodic_tasks()`**: also calls `sm.next_snap_requests()` -- batch to ALL peers for snap sync.
- **Sync responses**: processed immediately in `handle_network_event()` via `sm.handle_response()`.

Incoming sync requests from OTHER nodes are rate-limited at `MAX_SYNC_REQUESTS_PER_INTERVAL = 8` per production interval. Excess requests are silently dropped.

### 1.5 Sync Decision Logic

```
should_sync():
  if peers < min_peers_for_sync (1): NO
  min_gap = 3 (or 1 at height 0)
  if best_peer_height - local_height >= min_gap: YES
  if network_tip_height - local_height >= min_gap: YES

start_sync():
  if already_syncing: NO-OP (idempotent guard)
  gap = best_height - local_height
  enough_peers = peers >= 3
  should_snap = enough_peers && snap_attempts < 3 && gap > snap_threshold(50)
  if should_snap: enter SnapCollecting
  else: enter DownloadingHeaders
```

---

## 2. Performance Bottleneck Analysis

### 2.1 CRITICAL: Single-Peer Serial Header Download

**File**: `crates/network/src/sync/manager/sync_engine.rs:263-337`
**Config**: `max_headers_per_request = 500`

**Problem**: Headers are downloaded from exactly ONE peer, ONE request at a time. The pipeline is:

```
Request 500 headers -> wait RTT -> validate -> queue bodies -> request next 500 -> ...
```

For a 10,000 block gap at 500 headers/request, this is 20 round trips. At 100ms RTT on local network (higher under load), that is 2+ seconds just for header discovery. Under CPU stress (INC-I-012: 50 nodes on 16 cores), RTT balloons to 500ms+, making header download take 10+ seconds.

**Why single peer**: The `SyncPipelineData::Headers { peer, ... }` locks headers to one peer. If that peer disconnects, the entire header chain is discarded and sync restarts.

**Impact**: The dominant bottleneck for small-to-medium gaps (50-5000 blocks). Body download is 8x parallel but must wait for ALL headers before starting.

### 2.2 HIGH: No Pipeline Overlap Between Headers and Bodies

**File**: `crates/network/src/sync/manager/sync_engine.rs:620-726`

**Problem**: Body download only starts when the header download is COMPLETE (signaled by empty headers response). This creates two sequential phases:

```
[====== HEADER DOWNLOAD ======][====== BODY DOWNLOAD ======][== APPLY ==]
```

An optimal pipeline would overlap:

```
[== HEADERS BATCH 1 ==][== HEADERS BATCH 2 ==][== HEADERS BATCH 3 ==]
       [== BODIES BATCH 1 ==][== BODIES BATCH 2 ==][== BODIES BATCH 3 ==]
              [== APPLY BATCH 1 ==][== APPLY BATCH 2 ==][== APPLY BATCH 3 ==]
```

**Impact**: Total sync time is sum of phases instead of max. For a 5,000-block sync: ~5s headers + ~10s bodies + ~5s apply = 20s. With pipelining: ~12s.

### 2.3 HIGH: One Sync Request Per Tick

**File**: `bins/node/src/node/periodic.rs:365-369`

```rust
// Normal sync: one request per tick
if let Some((peer_id, request)) = sm.next_request() {
    // ... send one request
}
```

**Problem**: The production interval is 1s (testnet/mainnet). That means at most ONE header request or ONE body batch request per second. With max_headers_per_request=500, header throughput is capped at 500 headers/sec.

For body download with 8 concurrent requests, this means only ONE new request is issued per second even when requests complete within 100ms. The body downloader has capacity for 8 concurrent but is refilled at 1/second.

**Impact**: Body download throughput is approximately `max_bodies_per_request * 1/s = 128 blocks/sec` instead of theoretical `128 * 8 = 1024 blocks/sec`.

### 2.4 MEDIUM: Aggregate Sync Serving Rate Limit

**File**: `bins/node/src/node/event_loop.rs:526-536`

```rust
const MAX_SYNC_REQUESTS_PER_INTERVAL: u32 = 8;
```

**Problem**: Each node serves at most 8 sync requests per production interval (1s). When a syncing node sends a body batch request for 128 blocks, it counts as ONE of those 8. But when 20 nodes are syncing simultaneously from the same seed, 12 of them are silently dropped per interval.

**Impact**: Seed nodes become the bottleneck in 50+ node networks. Dropped requests are never retried by the requester (the response simply never arrives, timing out after 30s).

### 2.5 MEDIUM: Full State Reset on Sync Restart

**File**: `crates/network/src/sync/manager/sync_engine.rs:120-129`

```rust
// Clean slate for the new cycle. Clear ALL sync state from previous cycles.
self.pipeline.header_downloader.clear();
self.pipeline.pending_headers.clear();
self.pipeline.pending_blocks.clear();
self.pipeline.headers_needing_bodies.clear();
self.pipeline.pending_requests.clear();
self.pipeline.body_downloader.clear();
```

**Problem**: When sync restarts (peer disconnect, stuck timeout, fork detection), ALL downloaded headers and bodies are discarded. If 3,000 headers and 2,800 bodies were already downloaded when a timeout fires at 30s, all that work is lost.

**Impact**: Under stress (CPU oversubscription, network congestion), the 30s stuck-sync timeout in `cleanup()` fires frequently, destroying hours of accumulated sync progress. The INC-I-012 diagnosis notes: "This nuked 20-30K downloaded headers every 30s."

### 2.6 MEDIUM: Body Download Stall Recovery is Destructive

**File**: `crates/network/src/sync/manager/cleanup.rs:157-314`

Body stall recovery has 3 retries (soft), then a hard reset that clears ALL state. The soft retry rebuilds `headers_needing_bodies` but the hard reset path destroys everything:

```rust
// Full reset -- let start_sync() decide header-first vs snap sync
self.pipeline.pending_headers.clear();
self.pipeline.pending_blocks.clear();
```

**Impact**: After 3 soft retries (3x 120s = 6 minutes), a single missing body causes the loss of all downloaded data. The node starts over from scratch.

### 2.7 LOW: Block Application is Synchronous and Single-Threaded

**File**: `bins/node/src/node/periodic.rs:100-112`

```rust
let blocks = self.sync_manager.write().await.get_blocks_to_apply();
for block in blocks {
    if let Err(e) = self.apply_block(block, ValidationMode::Light).await {
        self.sync_manager.write().await.block_apply_failed();
        break;  // One failure stops all application
    }
}
```

**Problem**: Block application holds the sync_manager write lock, blocking all other sync operations. A slow `apply_block()` (disk I/O, UTXO updates, producer set mutations) blocks header/body downloads.

**Impact**: Under disk I/O contention (50 RocksDB instances per INC-I-012), block application takes 50-100ms per block. For 1,000 blocks, that is 50-100 seconds of blocking.

---

## 3. Failure Mode Catalog

### 3.1 FM-001: Sync Peer Disconnect (Single Point of Failure)

**Trigger**: Header download peer disconnects or times out.
**Behavior**: `remove_peer()` detects the header peer is gone, resets to Idle, discards all headers.
**File**: `crates/network/src/sync/manager/mod.rs:924-934`
**Severity**: HIGH -- all download progress lost. Common under CPU stress.
**Mitigation in code**: `start_sync()` picks a new best peer. But all headers are re-downloaded.

### 3.2 FM-002: Empty Headers Fork Detection Cascade

**Trigger**: Node on a fork sends GetHeaders with unrecognized tip hash.
**Behavior**:
1. First 1-2 empty responses: blacklist peer (30s cooldown)
2. After 3: clear blacklist, signal stuck fork
3. After 10: attempt snap sync redirect or genesis resync
4. Gap <= 50: redirect to rollback
5. Gap > 50: escalate to snap sync

**File**: `crates/network/src/sync/manager/sync_engine.rs:620-726`
**Severity**: MEDIUM -- correct behavior but slow. Each empty header cycle takes one periodic tick (1s). 10 cycles = 10s minimum before escalation.
**Root cause**: Header-first sync has no way to discover WHERE the fork diverged. It can only try GetHeaders from local tip and hope the peer recognizes it.

### 3.3 FM-003: Processing Stall (Downloaded Chain Mismatch)

**Trigger**: All headers and bodies downloaded, but the first header does not chain to local tip.
**Behavior**: `get_blocks_to_apply()` detects mismatch, clears ALL pending data, increments fork counter.
**File**: `crates/network/src/sync/manager/sync_engine.rs:855-878`
**Severity**: HIGH -- entire sync cycle wasted. This happens when the node receives headers from a peer on a different fork than expected.

### 3.4 FM-004: Body Stall Death Spiral

**Trigger**: Some bodies are permanently unavailable (peer disconnected, block pruned, etc.)
**Behavior**:
1. Soft retry x3 (120s each = 6 minutes)
2. Hard reset: clear all, start over
3. If gap > snap_threshold: snap sync activates
4. If snap fails 3 times: node is stuck

**File**: `crates/network/src/sync/manager/cleanup.rs:184-282`
**Severity**: CRITICAL for INC-I-010 scenario. The body stall + snap exhaustion + confirmed_height_floor creates an unrecoverable state.

### 3.5 FM-005: Stuck-Sync Timeout Destroying Progress

**Trigger**: No sync activity for 30s (headers phase) or 120s (bodies phase).
**Behavior**: Hard reset of ALL sync state.
**File**: `crates/network/src/sync/manager/cleanup.rs:167-313`
**Severity**: HIGH under CPU stress. The INC-I-012 diagnosis explicitly calls this out as destroying 20-30K headers.

### 3.6 FM-006: Sync-Request Deference Causing Silent Stalls

**Trigger**: Node serving 8+ sync requests per interval from syncing peers.
**Behavior**: Requests 9+ are silently dropped (no error sent to requester).
**File**: `bins/node/src/node/event_loop.rs:526-536`
**Severity**: MEDIUM. The requesting peer's request times out after 30s, wastes a sync cycle.

### 3.7 FM-007: Network Tip Inflation from Stale Peers

**Trigger**: A peer reports inflated height (during fork), then disconnects.
**Behavior**: `network_tip_height` was permanently inflated. Fixed in `remove_peer()` by recomputing from remaining peers.
**File**: `crates/network/src/sync/manager/mod.rs:890-902`
**Severity**: LOW (fixed). But residual risk: `update_peer()` still ratchets tip height upward without validation.

### 3.8 FM-008: Fork Recovery Serialization Under Load

**Trigger**: Multiple orphan blocks arrive from different forks simultaneously.
**Behavior**: Only ONE fork recovery can be active at a time. Others are queued behind a 30s cooldown.
**File**: `crates/network/src/sync/fork_recovery.rs:70-93`
**Severity**: HIGH at scale. With 50 nodes producing weight=1 blocks, fork storms generate 5-10 orphan blocks per slot. At 30s cooldown per recovery, resolving 10 forks takes 300s (5 minutes).

---

## 4. Prioritized Recommendations

### 4.1 CRITICAL: Pipeline Header and Body Download (P1)

**Problem solved**: Eliminates the sequential header-then-body bottleneck (Section 2.2) and increases throughput by 3-5x.

**Design**: Start body download as soon as the first batch of headers arrives, not after ALL headers are complete.

```
Current:  [headers 1..N complete] -> [start bodies] -> [apply]
Proposed: [headers 1..500] -> [bodies 1..500 + headers 501..1000] -> [bodies 501..1000 + apply 1..500] -> ...
```

**Code change**:
- `crates/network/src/sync/manager/sync_engine.rs`: In `handle_headers_response()`, after queuing headers to `headers_needing_bodies`, check if we have enough (e.g., 128+) and ALSO enter DownloadingBodies state concurrently.
- The state machine needs a composite `DownloadingHeadersAndBodies` phase, or decouple header download from the phase entirely (headers always download from one peer; bodies always download in parallel).
- `SyncPipelineData` needs to support both `Headers` and `Bodies` data simultaneously.

**Expected impact**: 2-3x faster sync for gaps of 500-10,000 blocks.
**Risk**: MEDIUM. Requires careful management of the header/body boundary. Headers arriving after body download starts need to be appended correctly. Chain mismatch detection (FM-003) must still work.

### 4.2 CRITICAL: Multi-Peer Header Download (P2)

**Problem solved**: Eliminates single-peer bottleneck (Section 2.1) and single point of failure (FM-001).

**Design**: Download headers from multiple peers in parallel ranges. Each peer gets a non-overlapping height range based on our local tip.

**Approach A -- Range Splitting**:
```
Peer A: GetHeaders(local_tip, count=500)       -> validates chain 1..500
Peer B: GetHeaders(hash_at_500, count=500)      -> validates chain 501..1000
```
Problem: We don't know hash_at_500 until Peer A responds.

**Approach B -- Parallel Verification**:
Request the same range from 2 peers. First valid response wins. Second provides redundancy.

**Approach C -- Checkpoint Headers**:
1. Request headers from 3 peers simultaneously (same range)
2. Compare responses: if 2/3 agree, accept that chain
3. If disagreement, fall back to single-peer

**Recommended**: Approach B (simplest). Request from 2 peers. First complete valid response wins. Second is discarded.

**Code change**:
- `crates/network/src/sync/manager/sync_engine.rs`: Modify `next_request()` for Headers phase to select 2 peers.
- `crates/network/src/sync/headers.rs`: Track which peer's chain was accepted.
- `SyncPipelineData::Headers` needs to store multiple peer IDs.

**Expected impact**: 1.5-2x faster header download. Eliminates FM-001 (peer disconnect kills progress).
**Risk**: LOW-MEDIUM. Duplicate requests increase network load but are bounded by 2x.

### 4.3 HIGH: Increase Sync Request Rate (P3)

**Problem solved**: Body download throughput capped at 1 request/sec (Section 2.3).

**Design**: Issue multiple sync requests per periodic tick, or issue requests from body response handlers.

**Option A -- Multiple requests per tick**:
```rust
// In periodic.rs:365
while let Some((peer_id, request)) = sm.next_request() {
    network.request_sync(peer_id, request).await;
    if request_count >= 8 { break; }  // Cap per tick
}
```

**Option B -- Response-driven pipelining**:
In `handle_response()` for bodies, immediately issue next body request to the same peer that just responded (pipeline the response-request cycle).

**Recommended**: Option A (simpler). Change the single `if let` to a `while let` loop with a cap.

**Code change**: `bins/node/src/node/periodic.rs:365` -- change `if let` to `while let` with cap.

**Expected impact**: 4-8x faster body download. Current: 128 blocks/sec. After: 512-1024 blocks/sec.
**Risk**: LOW. The body downloader already has `max_concurrent_body_requests=8` which self-limits.

### 4.4 HIGH: Incremental Sync Restart (Preserve Downloaded Data) (P4)

**Problem solved**: Full state reset on sync restart wastes accumulated work (Section 2.5, FM-005).

**Design**: When sync restarts (timeout, peer disconnect), preserve valid downloaded headers and bodies that chain from local tip. Only discard data that conflicts.

**Code change**:
- `crates/network/src/sync/manager/sync_engine.rs:120-129`: In `start_sync()`, instead of clearing everything, validate existing `pending_headers` chain from `local_hash`. Keep valid prefix, discard suffix after first gap/mismatch.
- `crates/network/src/sync/manager/cleanup.rs:282-314`: In stuck-sync hard reset, same approach -- validate and keep rather than clear all.

**Expected impact**: Under stress, eliminates the "20-30K headers nuked every 30s" problem identified in INC-I-012.
**Risk**: MEDIUM. Must carefully handle stale headers from a different fork. The chain linkage check (`prev_hash` validation) provides natural filtering.

### 4.5 HIGH: Increase Sync Serving Capacity (P5)

**Problem solved**: Seed node bottleneck at 8 requests/interval (Section 2.4, FM-006).

**Design**: Replace fixed rate limit with a semaphore or token bucket.

**Option A -- Dynamic rate limit**: `MAX_SYNC_REQUESTS = max(8, peers.len() / 2)`
**Option B -- Async semaphore**: Spawn sync request handling to a separate task pool with bounded concurrency.
**Option C -- Send error response**: Instead of silently dropping, send `SyncResponse::Error("busy")` so the requester can retry immediately with a different peer.

**Recommended**: Option C (simplest, most impactful). The requester currently waits 30s for a response that will never arrive.

**Code change**: `bins/node/src/node/event_loop.rs:527-531` -- send error response instead of silently dropping.

**Expected impact**: Eliminates 30s timeout waste. Requesting nodes fail over to alternative peers in <1s.
**Risk**: LOW. The error response is already handled in `handle_response()`.

### 4.6 MEDIUM: Smarter Peer Selection for Headers (P6)

**Problem solved**: Current peer selection picks highest-height peer, ignoring latency and reliability.

**Design**: Track per-peer metrics (response time, success rate, bytes delivered) and select based on composite score.

```rust
struct PeerSyncStatus {
    // ... existing fields ...
    avg_response_ms: u64,        // EMA of response times
    success_rate: f32,           // successful_responses / total_requests
    bytes_delivered: u64,        // Total sync data received
}
```

**Code change**:
- `crates/network/src/sync/manager/mod.rs`: Add tracking fields to `PeerSyncStatus`.
- `crates/network/src/sync/manager/sync_engine.rs:78-90`: Modify `best_peer()` to use composite score.

**Expected impact**: Avoids slow/unreliable peers. Under stress, fast peers get more requests.
**Risk**: LOW. Additive change, falls back to current behavior if metrics unavailable.

### 4.7 MEDIUM: Adaptive Timeout Based on Network Conditions (P7)

**Problem solved**: Fixed 30s/120s timeouts (Section 2.5) fire too early under CPU stress, too late in healthy conditions.

**Design**: Track actual response times and set timeout to `3 * p95_response_time` (minimum 10s, maximum 120s).

**Code change**:
- `crates/network/src/sync/manager/mod.rs`: Add `recent_response_times: VecDeque<Duration>` to NetworkState.
- `crates/network/src/sync/manager/cleanup.rs:167-171`: Use dynamic threshold instead of fixed 30s/120s.

**Expected impact**: Prevents false stuck detection under load. Faster detection in healthy conditions.
**Risk**: LOW. Additive, with fallback to current fixed values.

### 4.8 MEDIUM: Parallel Fork Recovery (P8)

**Problem solved**: Serialized fork recovery with 30s cooldown (FM-008) cannot keep up with fork storms.

**Design**: Allow up to N concurrent fork recoveries (N=3). Each tracks a different orphan block independently.

**Code change**:
- `crates/network/src/sync/fork_recovery.rs`: Change `active: Option<ActiveRecovery>` to `active: Vec<ActiveRecovery>` (capped at 3).
- `start()`, `next_fetch()`, `handle_block()`: Support multiplexed recoveries keyed by orphan hash.

**Expected impact**: 3x faster fork resolution during storms. Critical for 50+ node networks.
**Risk**: MEDIUM. Multiple concurrent recoveries can conflict if they find overlapping chains. Need deduplication.

### 4.9 LOW: Memory-Bounded Pending State (P9)

**Problem solved**: `pending_headers` (VecDeque<BlockHeader>) and `pending_blocks` (HashMap<Hash, Block>) grow unbounded during long syncs.

**Design**: Cap `pending_headers` at 10,000 and `pending_blocks` at 5,000. When cap reached, stop requesting more until applied blocks drain the queues (backpressure).

**Code change**:
- `crates/network/src/sync/manager/sync_engine.rs:263-337`: In `next_request()` for Headers, check `pending_headers.len() < 10_000` before issuing request.
- For Bodies, already implicitly bounded by `headers_needing_bodies`.

**Expected impact**: Prevents OOM during initial sync of very long chains. Memory stays bounded.
**Risk**: LOW. Natural backpressure, no data loss.

### 4.10 LOW: GetHeaders by Height Range (Protocol Enhancement) (P10)

**Problem solved**: Current `GetHeaders { start_hash, max_count }` requires the peer to know the starting hash. If the peer doesn't recognize it (fork), empty response. This is the root cause of FM-002.

**Design**: Add `GetHeadersByHeight { start_height: u64, max_count: u32 }` to the sync protocol.

```rust
SyncRequest::GetHeadersByHeight {
    start_height: u64,
    max_count: u32,
}
```

**Code change**:
- `crates/network/src/protocols/sync.rs`: Add new request variant.
- Server side: Look up block by height and serve headers from there.
- Client side: In `next_request()`, use height-based request when fork detected (after first empty headers response).

**Expected impact**: Eliminates FM-002 entirely. Fork detection becomes immediate instead of requiring 10 cycles.
**Risk**: MEDIUM. Protocol change requires version negotiation. Both old and new protocol must be supported during rollout.

---

## 5. Implementation Roadmap

### Phase 1: Quick Wins (1-2 days, no protocol changes)

| Priority | Rec | Description | Dependency |
|----------|-----|-------------|------------|
| P3 | 4.3 | Multiple sync requests per tick | None |
| P5 | 4.5 | Error response on rate limit (not silent drop) | None |
| P4 | 4.4 | Preserve valid data on sync restart | None |

**Rationale**: These three changes are independent, low-risk, and address the most impactful bottlenecks. P3 alone should give 4-8x body download improvement. P5 eliminates 30s timeout waste. P4 eliminates the "nuked headers" problem.

### Phase 2: Pipeline Improvements (3-5 days)

| Priority | Rec | Description | Dependency |
|----------|-----|-------------|------------|
| P1 | 4.1 | Pipelined header + body download | P3 |
| P7 | 4.7 | Adaptive timeouts | P4 |
| P6 | 4.6 | Peer quality scoring | None |

**Rationale**: P1 is the biggest architectural improvement but requires P3 to be effective (pipelining with 1 req/s is still slow). P7 depends on P4 (no point tuning timeouts if restarts still destroy data).

### Phase 3: Scalability Improvements (5-7 days)

| Priority | Rec | Description | Dependency |
|----------|-----|-------------|------------|
| P2 | 4.2 | Multi-peer header download | P1 |
| P8 | 4.8 | Parallel fork recovery | None |
| P9 | 4.9 | Memory-bounded pending state | P1 |

**Rationale**: Multi-peer headers requires the pipeline (P1) to be in place. Parallel fork recovery is independent but less impactful once equal-weight tie-breaking (already committed as INC-I-012 fix) reduces fork frequency.

### Phase 4: Protocol Evolution (7-14 days, requires coordinated deploy)

| Priority | Rec | Description | Dependency |
|----------|-----|-------------|------------|
| P10 | 4.10 | GetHeadersByHeight protocol | Phase 1-3 |

**Rationale**: Protocol changes require all nodes to upgrade simultaneously. Delay until Phase 1-3 improvements are proven stable.

---

## 6. Architecture Assessment

### 6.1 Is Header-First the Right Approach?

**Yes, with caveats.** Header-first sync is the right foundation for DOLI because:

1. **Lightweight validation**: Headers are ~300 bytes vs full blocks at ~2-10KB. Chain linkage and basic timestamp checks can be done without downloading transaction data.
2. **Fork detection**: Headers-first reveals the canonical chain structure before committing to body download. A competing fork is detected at the header level.
3. **Bandwidth efficiency**: If 3 of 5 peers are on forks, header validation rejects their chains before downloading bodies.

**The problem is not the approach but the implementation**:
- Serial single-peer header download is unnecessary. Bitcoin downloads headers from multiple peers.
- Sequential phases (all headers then all bodies) is an Ethereum 1.0 design that Ethereum abandoned.
- One request per tick is an artificial bottleneck from the event loop design.

### 6.2 Comparison with Other Chains

| Aspect | DOLI (current) | Bitcoin | Ethereum (snap sync) |
|--------|---------------|---------|---------------------|
| Header download | 1 peer, serial | Multiple peers, parallel | Multiple peers, parallel |
| Body download | 8 concurrent | 16+ concurrent | State trie parallel download |
| Pipeline overlap | None | Full overlap | Full overlap (state chunks) |
| Fork handling | Single recovery, 30s cooldown | Parallel, no cooldown | Checkpoint-based, no recovery needed |
| Peer selection | Highest height | Latency + throughput scoring | Random with banning |
| Request rate | 1/sec | Unlimited (backpressure) | Unlimited (backpressure) |
| Progress preservation | Full reset on restart | Checkpoint-based resume | Checkpoint-based resume |

### 6.3 What Would an Ideal Sync Protocol Look Like?

```
1. DISCOVERY (parallel, all peers):
   - Request chain summaries (height, hash at every 1000th block)
   - Find common ancestor via binary search in 10 round trips

2. HEADER DOWNLOAD (parallel, 4 peers):
   - Split range into 4 chunks, one per peer
   - Verify chain linkage within and across chunks
   - Start body download as soon as first chunk verified

3. BODY DOWNLOAD (parallel, 8 peers):
   - Request bodies by hash from any peer with that height range
   - Track per-peer throughput, prefer fast peers
   - Backpressure: pause body requests when apply queue > 1000

4. APPLICATION (batched):
   - Apply blocks in batches of 100 for amortized DB writes
   - Checkpoint every 1000 blocks for resume
   - Deferred validation (VDF, signatures) in parallel threads

5. FORK RESOLUTION:
   - Detect forks at header level via chain summaries
   - Weight comparison before any body download
   - Parallel fork recovery with deduplication
```

---

## 7. Key Metrics to Track

For measuring improvement, instrument these counters:

| Metric | Current Baseline | Target |
|--------|-----------------|--------|
| Header download throughput | ~500/sec (1 batch/tick) | ~2000/sec |
| Body download throughput | ~128/sec (1 batch/tick) | ~1000/sec |
| Sync restart data loss | 100% (full reset) | <10% (preserve valid prefix) |
| Fork recovery latency | 30s cooldown + 10s/fork | <5s/fork |
| Sync serving capacity | 8 req/interval | 32 req/interval or async |
| Time to sync 1000 blocks | ~30-60s | ~5-10s |
| Sync stuck timeout waste | ~20-30K headers/reset | 0 (preserved) |

---

## 8. Summary of Findings

The header-first sync architecture is fundamentally sound but the implementation has 7 significant performance bottlenecks that compound under scale:

1. **Serial single-peer header download** -- the primary bottleneck, fixable by parallelizing across 2+ peers.
2. **No pipeline overlap** -- headers must ALL complete before bodies start, doubling total sync time.
3. **One sync request per tick** -- artificial 1/sec cap limits body download to 128 blocks/sec instead of 1024.
4. **Full state reset on restart** -- any timeout destroys all accumulated sync progress.
5. **Silent request dropping** -- requesting peers wait 30s for responses that will never arrive.
6. **Serialized fork recovery** -- 30s cooldown between recoveries cannot keep up with fork storms.
7. **No progress checkpointing** -- interrupted syncs restart from scratch.

The three highest-impact fixes (P3: request rate, P5: error response, P4: preserve state) require minimal code changes and no protocol modifications. They should provide a 4-8x improvement in sync throughput and eliminate the most destructive failure mode (progress destruction on restart).
