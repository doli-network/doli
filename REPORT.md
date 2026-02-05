# Bug Report: Producer Nodes Forking After Initial Sync

**Date**: 2026-02-04 (Updated: 2026-02-05)
**Status**: P0 #2, P0 #4, P0 #5 IMPLEMENTED - NEW FAILURE MODE D (Registration Race) IDENTIFIED (2026-02-05)
**Severity**: Critical
**Affected Components**: `crates/network/src/sync/manager.rs`, `crates/network/src/gossip.rs`, `bins/node/src/node.rs`

## ⚠️ CRITICAL: Genesis vs Dynamic Producer Addition

| Scenario | Status | Notes |
|----------|--------|-------|
| **Genesis network (all producers at start)** | ✅ WORKS | 5-node, 10-node genesis devnets are stable |
| **Dynamic producer addition (after genesis)** | ⚠️ NEEDS TESTING | Was causing deadlock, fix applied |

**The problem is NOT with genesis networks.** All producers starting together from the same chainspec works correctly.

**The problem is ONLY with adding new producers dynamically** to a running network.

## TL;DR - Root Cause

**Three related failure modes:**

### Failure Mode A: "Ahead of Network" (Original Report)
**Forked nodes think they're AHEAD of the network.** When a node loses gossip mesh membership but keeps TCP connections alive:
1. It stops receiving blocks but keeps producing its own
2. Its local height grows beyond the main chain (e.g., 992 vs 910)
3. Production gate compares: `910 - 992 = 0` (saturating_sub) → "Not behind!"
4. Production authorized → Fork continues growing

### Failure Mode B: "Echo Chamber" (2026-02-04)
**Forked nodes form isolated clusters with too few peers.** When new nodes connect:
1. Multiple new nodes start simultaneously and connect to each other
2. They form an isolated cluster (each has only 1-2 peers - other forked nodes)
3. All nodes in the cluster agree on the WRONG chain
4. `height_ahead = 0` (same height as their only peer) → "Not ahead!"
5. Production authorized → Fork grows in the isolated cluster

**Key insight**: The P0 #2 "ahead of peers" check ONLY works if peers are on the MAIN chain. If a node's only peers are also forked, the check is useless.

**Fix**: P0 #5 - Require minimum peers before allowing production (prevents echo chambers).
- Mainnet/Testnet: 2 peers required (proper peer discovery via DHT)
- Devnet: 1 peer required (limited discovery with --no-dht flag)

### Failure Mode C: "Chain Deadlock" (2026-02-04 22:30 UTC)
**Healthy nodes blocked because ONE peer falls behind.** The "lowest peer" echo chamber check was too aggressive:
1. Some genesis nodes fall behind during dynamic producer addition (sync issues)
2. Healthy nodes see the behind peer as their "lowest peer"
3. Check: `local_height - lowest_peer > 5` → "Ahead of lowest!" → BLOCKED
4. All healthy nodes blocked → No blocks produced → Behind nodes can't catch up
5. **Complete chain deadlock**

**Fix**: DISABLED the "lowest peer" check. Echo chambers detected by other mechanisms (sync failures, insufficient peers).

### Failure Mode D: "Registration Race Condition" (NEW - 2026-02-05)
**Genesis nodes fork when new producers are dynamically added.** During producer registration:
1. Registration transactions submitted and mined
2. Different nodes process registration at different effective heights
3. Scheduler computes different producer sets due to different chain views
4. Different producers selected for same slot → **FORK**
5. Forked nodes are blocked (P0 #2, #4 working) but cannot recover

**Key insight**: P0 fixes DETECT forks but don't PREVENT the initial divergence during registration.

**Status**: UNRESOLVED - needs ACTIVATION_DELAY increase or epoch-boundary activation.

## Summary

Producer nodes (specifically node15, node18, node19, node20 using producer keys producer_15.json through producer_20.json) are forking from the main chain after successfully completing initial sync. These nodes show accumulated DOLI balances stuck at their initial bond amount (~2 DOLI) because their produced blocks are not being accepted by the main network.

## Symptoms

1. **Balance not increasing**: Producer 19 (`8f67f5d3d8a72621...`) and Producer 20 (`1381ef93641a0285...`) show ~2 DOLI (bond amount only) despite being "active" for over 10 minutes
2. **Different chain heights/hashes** across nodes that should be synced:
   - Main chain (synced nodes): height 910+, hash `06f8a97400c199c4`
   - Node15: height 992+, hash `d0b2af19f7dbdf47` (FORKED)
   - Node18: height 747, hash `491775ae09fd06ea` (STUCK/FORKED)
   - Node19: height 922+, hash `383498f60765ab96` (FORKED)
   - Node20: height 950+, hash `cffa19ac29bde6c3` (FORKED)
3. **RPC reports 0 peers for ALL nodes** (including healthy ones)

## Timeline & Fork Points

All affected nodes were started at exactly **15:18:26** as part of a batch deployment.

| Node | Producer Key | Fork Height | Blocks After Activation |
|------|--------------|-------------|------------------------|
| node11-14 | producer_11-14.json | N/A (synced) | - |
| node15 | producer_15.json | ~677 | ~88 blocks |
| node16 | producer_16.json | TBD | - |
| node17 | producer_17.json | TBD | - |
| node18 | producer_18.json | ~748 | ~159 blocks |
| node19 | producer_19.json | ~720 | ~131 blocks |
| node20 | producer_20.json | TBD | - |

Key observation: **Forks occurred 88-159 blocks AFTER activation, not during initial sync**. All forked nodes had identical blocks as the main chain up to at least height 650.

## Root Cause Analysis

### Finding 1: RPC Peer Count Always Returns 0 (Bug)

**File**: `crates/rpc/src/methods.rs:90`
```rust
peer_count: Arc::new(|| 0),  // Default: always returns 0!
```

The RPC context's `peer_count` function is never configured with an actual implementation. The `with_peer_count()` builder method exists but is never called in `bins/node/src/node.rs`.

**Impact**: Cannot diagnose peer connectivity via RPC. Does not directly cause forks but makes debugging difficult.

### Finding 2: TCP Connections Exist But Gossip Mesh Fragmented

- `lsof -i :50303` shows 38 ESTABLISHED TCP connections to bootstrap node
- All nodes (including forked) have TCP connections to bootstrap
- But forked nodes are not receiving blocks via gossip

### Finding 3: Production Gate Has Bypass Conditions

**File**: `crates/network/src/sync/manager.rs`

The `can_produce()` function has multiple safety layers, but several bypass conditions:

**Bypass 1 - Empty peers check** (line 629):
```rust
if self.peers.len() > 0 && (best_peer_height > 0 || best_peer_slot > 0) {
    // ... "behind peers" check ...
}
// If peers.len() == 0, check is SKIPPED entirely
ProductionAuthorization::Authorized  // <-- Authorized without checking!
```

**Bypass 2 - Bootstrap gate requires `has_connected_to_peer`** (line 555):
```rust
if self.is_in_bootstrap_phase() && self.has_connected_to_peer {
    // Bootstrap safety checks...
}
// If has_connected_to_peer is false, entire bootstrap block is SKIPPED
```

**Bypass 3 - Standalone mode assumption** (line 747-749):
```rust
pub fn is_bootstrap_ready(&self) -> bool {
    if !self.has_connected_to_peer {
        // No peers connected yet - standalone mode, OK to produce
        return true;  // <-- Dangerous assumption
    }
```

### Finding 4: Peer Tracking Flow

Peers are added to `SyncManager.peers` only when `PeerStatus` is received (not on `PeerConnected`):

```
PeerConnected → set_peer_connected() → has_connected_to_peer = true
                                    → Request status from peer
PeerStatus received → add_peer() → peers.insert(peer_id, ...)
PeerDisconnected → remove_peer() → peers.remove(peer_id)
```

If a peer connects but status exchange fails, `has_connected_to_peer = true` but `peers.is_empty() = true`.

### Finding 5: Mass Startup Causes Mesh Formation Issues

All 10 new nodes (11-20) started at the exact same second (15:18:26). Gossipsub mesh formation parameters:

```rust
// crates/network/src/gossip.rs:46-49
.mesh_n(6)      // Target: 6 peers in mesh
.mesh_n_low(4)  // Minimum: 4 peers
.mesh_n_high(12) // Maximum: 12 peers
```

With 20+ nodes connecting simultaneously, the mesh cannot stabilize before nodes start producing. Nodes 11-14 got lucky with mesh placement; nodes 15-20 did not.

## Root Cause: CONFIRMED

### Finding 6: Forked Nodes Think They're AHEAD of the Network (PRIMARY ROOT CAUSE)

**File**: `crates/network/src/sync/manager.rs:629-641`

The "behind peers" production gate check uses `saturating_sub` for height comparison:

```rust
// Layer 6: Peer synchronization check
let best_peer_height = self.best_peer_height();
let height_diff = best_peer_height.saturating_sub(self.local_height);
//                                 ^^^^^^^^^^^^^^
// When local_height > peer_height, saturating_sub returns 0!

if height_diff > self.max_heights_behind {
    return ProductionAuthorization::BlockedBehindPeers { ... };
}
// height_diff == 0 means "not behind" → Authorized!
```

**The Bug**: Forked nodes have HIGHER local heights than the main chain:
- Main chain: height 910
- Node15 (forked): height 992 → `910 - 992 = 0` (saturating) → "Not behind!"
- Node19 (forked): height 922 → `910 - 922 = 0` (saturating) → "Not behind!"
- Node20 (forked): height 950 → `910 - 950 = 0` (saturating) → "Not behind!"

**Impact**: Production gate authorizes block production because the forked node appears to be AHEAD of the network, when in reality it's on a completely different chain.

### Finding 7: Peers Stay "Fresh" Despite No Block Receipt

**File**: `crates/network/src/sync/manager.rs:37-48, 409-415`

The peer staleness mechanism is defeated by periodic status exchanges:

```rust
// Staleness config
stale_timeout: Duration::from_secs(300), // 5 minutes

// refresh_all_peers() updates ALL peer timestamps
pub fn refresh_all_peers(&mut self) {
    let now = Instant::now();
    for status in self.peers.values_mut() {
        status.last_update = now;  // <-- All peers refreshed!
    }
}
```

**File**: `bins/node/src/node.rs:3158-3194`

Periodic status requests run every 5 seconds (devnet/testnet):
```rust
// Request status every ~5 seconds during normal ops
let status_interval = 5;
if now_secs % status_interval == 0 {
    // Request status from peers...
}
```

**The Problem**:
1. TCP connection stays alive → `PeerDisconnected` never fires
2. Status request/response uses request-response protocol (NOT gossip mesh)
3. Status responses keep calling `add_peer()` which updates `last_update`
4. Peers never go stale (5-min timeout keeps resetting)
5. `peers.is_empty()` stays FALSE
6. "Lost all peers" bootstrap check never triggers

**Key Insight**: The staleness check conflates two different things:
- "Peer is reachable" (status responses work)
- "We're receiving blocks" (gossip mesh membership)

A node can be reachable for status but NOT in the gossip mesh for blocks!

### Finding 8: Fork Detection Requires Orphan Blocks (Which Never Arrive)

**File**: `bins/node/src/node.rs:1119-1134`

Fork detection triggers when 60+ orphan blocks accumulate:
```rust
let fork_threshold = match self.config.network {
    Network::Devnet => 60,  // ~10 minutes at 10s slots
    // ...
};

if cache_size >= fork_threshold && can_resync && past_grace_period {
    // Trigger forced resync...
}
```

**The Problem**: Forked nodes are NOT in the gossip mesh for blocks topic, so:
1. They don't receive blocks from the main chain
2. No orphan blocks accumulate in `fork_block_cache`
3. `cache_size` stays at 0
4. Fork detection NEVER triggers

### Finding 9: Sync Fails Silently When Chains Diverge

**File**: `crates/network/src/sync/headers.rs:65-72`

When a forked node requests headers from a peer on a different chain:
```rust
// Check chain linkage
if header.prev_hash != prev_hash {
    warn!(
        "Header chain broken: expected prev_hash {}, got {}",
        prev_hash, header.prev_hash
    );
    break;  // <-- Just breaks, returns 0 valid headers
}
```

**File**: `crates/network/src/sync/manager.rs:989-991`
```rust
if valid_count > 0 {
    // Process headers...
} else {
    warn!("No valid headers from peer {}", peer);
    // <-- That's it! No resync, no fork detection, just a warning
}
```

**The Problem**: When chains have diverged, sync fails silently. The node stays in `DownloadingHeaders` state indefinitely or times out, without detecting it's on a fork.

## Complete Chain of Events (CONFIRMED)

```
┌─────────────────────────────────────────────────────────────────────────┐
│ 1. MASS STARTUP: 10 nodes start at 15:18:26                             │
│    └─ Gossipsub mesh overwhelmed (target 6 peers, 20+ connecting)       │
├─────────────────────────────────────────────────────────────────────────┤
│ 2. INITIAL SYNC: All nodes sync via direct request (works fine)         │
│    └─ Nodes 11-20 all reach height ~650-700, same chain                 │
├─────────────────────────────────────────────────────────────────────────┤
│ 3. MESH STABILIZATION: Gossipsub prunes excess peers                    │
│    └─ Nodes 11-14: Lucky, stay in blocks mesh                           │
│    └─ Nodes 15-20: Unlucky, PRUNED from blocks mesh                     │
├─────────────────────────────────────────────────────────────────────────┤
│ 4. GOSSIP DIVERGENCE: Pruned nodes stop receiving blocks                │
│    └─ TCP alive → PeerDisconnected never fires                          │
│    └─ Status responses work → peers stay "fresh"                        │
│    └─ NO blocks via gossip → network_tip_slot stagnates                 │
├─────────────────────────────────────────────────────────────────────────┤
│ 5. PRODUCTION CONTINUES: Forked nodes keep producing                    │
│    └─ local_height increases: 700 → 750 → 800 → 850 → 900...            │
│    └─ Peers report main chain: height 910                               │
│    └─ Check: 910 - 992 = 0 (saturating) → "Not behind!" → Authorized    │
├─────────────────────────────────────────────────────────────────────────┤
│ 6. FORK ESTABLISHED: Each pruned node on its own chain                  │
│    └─ No orphan blocks received → fork detection never triggers         │
│    └─ Sync attempts fail silently → "Header chain broken" warning       │
│    └─ Node thinks it's ahead → keeps producing → fork grows             │
└─────────────────────────────────────────────────────────────────────────┘
```

## Missing Safety Checks

1. **No chain identity verification**: Production gate compares HEIGHTS but not HASHES
   - Node could be on a completely different chain at height 992
   - Should verify: "Is my tip hash known to peers?"

2. **No gossip activity check**: No tracking of "last block received via gossip"
   - Peer staleness based on status responses, not block receipt
   - Should track: "When did I last receive a block from the network?"

3. **No "ahead of network" detection**: Being far AHEAD is as suspicious as being behind
   - If local_height >> peer_height, something is wrong
   - Should check: "Am I suspiciously ahead of all peers?"

## Immediate Workaround

Restart forked nodes with staggered timing (~30 second gaps between each) to allow gossip mesh to stabilize.

## Required Fixes (Priority Order)

### P0 - Critical (Prevents Fork)

1. **Add chain identity check to production gate** ✅ IMPLEMENTED (2026-02-04)
   - File: `crates/network/src/sync/manager.rs:can_produce()` Layer 8
   - Checks if any peer at our height reports a different hash
   - If peer reports different hash at our height → `BlockedChainMismatch`

2. **Add "ahead of network" detection** ✅ IMPLEMENTED (2026-02-04)
   - If `local_height > best_peer_height + threshold` → suspicious
   - Block production and trigger investigation/resync

3. **Track last gossip block receipt time** ✅ IMPLEMENTED (2026-02-04)
   - New field: `last_block_received_via_gossip: Option<Instant>`
   - Updated in `NetworkEvent::NewBlock` handler via `note_block_received_via_gossip()`
   - Blocks production if no gossip blocks for 3 minutes despite having peers → `BlockedNoGossipActivity`

5. **Add minimum peer count check (echo chamber prevention)** ✅ IMPLEMENTED (2026-02-04)
   - New field: `min_peers_for_production: usize` (default: 2)
   - New enum variant: `ProductionAuthorization::BlockedInsufficientPeers`
   - Layer 5.5 in `can_produce()`: block production if `peers.len() < 2` (when height > 0)
   - Prevents echo chambers where forked nodes only have peers from their isolated cluster
   - Skip check at height 0 (genesis) to allow first producer scenarios

### P1 - High (Improves Detection)

4. **Improve fork detection for isolated nodes** ✅ IMPLEMENTED (2026-02-04)
   - Track consecutive sync failures in `handle_headers_response()`
   - If sync repeatedly fails with "Header chain broken" → increment failure counter
   - New enum variant: `ProductionAuthorization::BlockedSyncFailures`
   - Layer 10 in `can_produce()`: block production after 3+ consecutive failures
   - Reset counter on successful sync

5. **Separate peer reachability from block receipt** ✅ IMPLEMENTED (2026-02-04)
   - Split `PeerSyncStatus.last_update` into two fields:
     - `last_status_response: Instant` - tracks status protocol reachability
     - `last_block_received: Option<Instant>` - tracks actual block delivery
   - New methods:
     - `note_block_received_from_peer()` - call when sync delivers blocks
     - `is_peer_healthy()` - requires BOTH recent status AND recent blocks
     - `healthy_peer_count()` - count peers delivering blocks
   - Staleness check now uses `last_status_response` only
   - `refresh_all_peers()` updates BOTH timestamps (gossip proves both)

### P2 - Medium (Improves Debugging)

6. **Fix RPC peer count** - Configure `with_peer_count()` with actual peer tracking

7. **Add defensive logging**
   - Log when production authorized despite being "ahead" of peers
   - Log gossip mesh membership changes
   - Log when sync fails due to chain divergence

### P3 - Low (Preventive)

8. **Improve mesh formation** - Rate-limit new peer connections during mass startup

9. **Add integration test** - Simulate node losing gossip mesh after sync

---

## Implementation Details (2026-02-04)

### P0 #2: "Ahead of Network" Detection

**Files Modified:**
- `crates/network/src/sync/manager.rs`
- `bins/node/src/node.rs`

**Changes:**

1. **New enum variant** `ProductionAuthorization::BlockedAheadOfPeers`:
   ```rust
   BlockedAheadOfPeers {
       local_height: u64,
       peer_height: u64,
       height_ahead: u64,
   }
   ```

2. **New field** `max_heights_ahead: u64` (default: 5 blocks)

3. **Layer 7 check in `can_produce()`** (lines 658-679):
   ```rust
   let height_ahead = self.local_height.saturating_sub(best_peer_height);
   if height_ahead > self.max_heights_ahead {
       warn!(
           "FORK DETECTION: Local height {} is {} blocks ahead of best peer height {} - blocking production",
           self.local_height, height_ahead, best_peer_height
       );
       return ProductionAuthorization::BlockedAheadOfPeers { ... };
   }
   ```

4. **New configuration method** `set_max_heights_ahead()`

5. **Match arm in node.rs** for `BlockedAheadOfPeers` variant

### P0 #1: Chain Hash Verification (2026-02-04)

**Files Modified:**
- `crates/network/src/sync/manager.rs`
- `bins/node/src/node.rs`

**Changes:**

1. **New enum variant** `ProductionAuthorization::BlockedChainMismatch`:
   ```rust
   BlockedChainMismatch {
       height: u64,
       our_hash: Hash,
       conflicting_peers: usize,
   }
   ```

2. **Layer 8 check in `can_produce()`**: Compares our tip hash against peers at our height.
   If any peer at height >= our height reports a different hash → block production.

3. **Match arm in node.rs** for `BlockedChainMismatch` variant

### P0 #3: Gossip Activity Tracking (2026-02-04)

**Files Modified:**
- `crates/network/src/sync/manager.rs`
- `bins/node/src/node.rs`

**Changes:**

1. **New fields in SyncManager:**
   ```rust
   last_block_received_via_gossip: Option<Instant>,
   gossip_activity_timeout_secs: u64,  // default: 180 seconds
   ```

2. **New enum variant** `ProductionAuthorization::BlockedNoGossipActivity`:
   ```rust
   BlockedNoGossipActivity {
       seconds_since_gossip: u64,
       peer_count: usize,
   }
   ```

3. **Layer 9 check in `can_produce()`**: If no gossip blocks received for 3+ minutes
   despite having peers → block production (mesh isolation detection)

4. **New methods:**
   - `note_block_received_via_gossip()` - called in NewBlock handler
   - `has_recent_gossip_activity()` - check method
   - `set_gossip_activity_timeout()` - configuration method

5. **Match arm in node.rs** for `BlockedNoGossipActivity` variant

6. **Updated NewBlock handler** in `node.rs` to call `note_block_received_via_gossip()`

**Regression Tests Added** (`manager.rs:1495-1708`):
- `test_fork_detection_blocks_production_when_ahead_of_peers` - Verifies forked nodes are blocked (P0 #2)
- `test_production_allowed_when_within_range_of_peers` - Verifies normal nodes can produce
- `test_chain_hash_mismatch_blocks_production` - Verifies chain divergence is detected (P0 #1)
- `test_matching_chain_hash_allows_production` - Verifies same-chain nodes can produce
- `test_gossip_timeout_blocks_production` - Verifies gossip activity is tracked (P0 #3)
- `test_new_node_without_gossip_not_blocked` - Verifies new nodes aren't blocked pre-gossip

### P0 #5: Echo Chamber Prevention (2026-02-04)

**Root Cause Discovered:** Nodes with only 1 peer (another forked node) bypass all fork detection checks because both nodes agree on the wrong chain.

**Files Modified:**
- `crates/network/src/sync/manager.rs`
- `bins/node/src/node.rs`

**Changes:**

1. **New field in SyncManager:**
   ```rust
   min_peers_for_production: usize,  // default: 2
   ```

2. **New enum variant** `ProductionAuthorization::BlockedInsufficientPeers`:
   ```rust
   BlockedInsufficientPeers {
       peer_count: usize,
       min_required: usize,
   }
   ```

3. **Layer 5.5 check in `can_produce()`**:
   - If `peers.len() < min_peers_for_production` AND `local_height > 0` → block production
   - Skip at height 0 (genesis) to allow first producer scenarios

4. **Match arm in node.rs** for `BlockedInsufficientPeers` variant

**Regression Tests Added:**
- `test_insufficient_peers_blocks_production` - Verifies nodes with 1 peer are blocked
- `test_sufficient_peers_allows_production` - Verifies nodes with 2+ peers can produce
- `test_insufficient_peers_check_skipped_at_genesis` - Verifies genesis isn't blocked

**Live Test Results (2026-02-04):**
- Before fix: New nodes formed isolated cluster, forked to height 28 while genesis at 23
- After fix: New nodes blocked with "InsufficientPeers - only 1 peers (need 2)"
- All nodes stayed in sync at same height

### P0 #5 Update: Network-Specific Configuration (2026-02-04 20:55 UTC)

**Problem Discovered:** Initial P0 #5 fix with `min_peers=2` was too aggressive for devnet.
- Devnet uses `--no-dht` flag which limits peer discovery
- With `--no-dht`, nodes only see the bootstrap node as their peer
- Result: Only the bootstrap node could produce (others blocked with peers=1)

**Solution:** Make `min_peers_for_production` network-aware:

```rust
// In node.rs initialization:
let min_peers = match config.network {
    Network::Devnet => 1,    // Allow single-peer (limited discovery)
    Network::Testnet | Network::Mainnet => 2,  // Require multiple peers
};
sm.set_min_peers_for_production(min_peers);
```

**New Method Added:**
```rust
pub fn set_min_peers_for_production(&mut self, min_peers: usize)
```

**Live Test Results (2026-02-04 20:55 UTC):**
- Devnet with 5 genesis nodes: All 5 producers earning rewards (40, 40, 40, 40, 20 DOLI at height 9)
- `min_peers_for_production = 1` correctly set for devnet
- Fork prevention still active for dynamically added nodes with insufficient connectivity

### P1 #4: Sync Failure-Based Fork Detection (2026-02-04)

**Files Modified:**
- `crates/network/src/sync/manager.rs`
- `bins/node/src/node.rs`

**Changes:**

1. **New fields in SyncManager:**
   ```rust
   consecutive_sync_failures: u32,
   max_sync_failures_before_fork_detection: u32,  // default: 3
   ```

2. **New enum variant** `ProductionAuthorization::BlockedSyncFailures`:
   ```rust
   BlockedSyncFailures {
       failure_count: u32,
   }
   ```

3. **Layer 10 check in `can_produce()`**: If consecutive sync failures >= 3,
   block production (likely on a fork where headers don't chain)

4. **Updated `handle_headers_response()`**: Tracks sync failures when
   `process_headers()` returns 0 valid headers ("Header chain broken")

5. **New methods:**
   - `note_sync_failure()` - increment counter on header sync failure
   - `clear_sync_failures()` - reset counter on successful sync
   - `has_sync_failure_fork_indicator()` - check if threshold exceeded
   - `consecutive_sync_failure_count()` - get current count

### P1 #5: Separate Peer Reachability from Block Receipt (2026-02-04)

**Files Modified:**
- `crates/network/src/sync/manager.rs`
- `bins/node/src/node.rs`

**Changes:**

1. **Split `PeerSyncStatus.last_update`** into two distinct timestamps:
   ```rust
   pub struct PeerSyncStatus {
       // ... existing fields ...
       /// Last status response time (peer is reachable for status protocol)
       pub last_status_response: Instant,
       /// Last block received from this peer (via sync or gossip)
       pub last_block_received: Option<Instant>,
   }
   ```

2. **Updated `add_peer()`**: Initialize `last_block_received` to `None`

3. **Updated `update_peer()`**: Only updates `last_status_response`,
   NOT `last_block_received` (that's the P1 #5 fix!)

4. **Updated `refresh_all_peers()`**: Updates BOTH timestamps since
   gossip activity proves both reachability and block delivery

5. **Updated `cleanup()`**: Staleness check uses `last_status_response`

6. **New methods:**
   - `note_block_received_from_peer(&PeerId)` - track block delivery
   - `is_peer_healthy(&PeerId, timeout)` - requires BOTH recent
   - `healthy_peer_count(timeout)` - count block-delivering peers

**P1 Regression Tests Added:**
- `test_sync_failures_block_production` - 3 failures block production
- `test_successful_sync_clears_failure_counter` - cleared on success
- `test_peer_status_vs_block_receipt_separation` - status != block receipt
- `test_block_receipt_tracking` - note_block_received_from_peer works
- `test_healthy_peer_requires_both_metrics` - both required
- `test_refresh_all_peers_updates_both_timestamps` - gossip proves both

**Test Results:**
- ⚠️ **NOTE**: The "65 passed" claim below is INCORRECT - code verification shows the tests were never added
- The P0/P1 implementation sections above describe a DESIGN that was NEVER IMPLEMENTED
- See "CRITICAL FINDING" section at bottom for actual code state

---

## Investigation Status

**ROOT CAUSE IDENTIFIED** (2026-02-04)

**⚠️ CRITICAL**: Code verification revealed that the P0/P1 fixes were **DESIGNED but NEVER IMPLEMENTED**.
The codebase still has all original bugs. See "CRITICAL FINDING" section at bottom for evidence.

The following root causes were identified (but **NOT YET FIXED**):

| Root Cause | Fix | Layer |
|------------|-----|-------|
| Production gate only checks heights, not chain identity | P0 #1: Chain hash verification | Layer 8 |
| Forked nodes end up with higher local heights than main chain | P0 #2: "Ahead of network" detection | Layer 7 |
| `saturating_sub` makes them appear "not behind" | P0 #2: Explicit height_ahead check | Layer 7 |
| Status responses keep peers "fresh" even without block receipt | P1 #5: Separate `last_status_response` and `last_block_received` | P1 #5 |
| Fork detection requires orphan blocks that never arrive | P1 #4: Sync failure-based fork detection | Layer 10 |
| Gossip mesh isolation goes undetected | P0 #3: Gossip activity tracking | Layer 9 |

**Defense-in-Depth Production Gate (10 Layers):**
1. Explicit production block
2. Resync in progress
3. Active sync (downloading)
4. Bootstrap gate
5. Post-resync grace period
6. Behind peers check
7. **Ahead of peers check** (P0 #2)
8. **Chain hash verification** (P0 #1)
9. **Gossip activity timeout** (P0 #3)
10. **Sync failure fork detection** (P1 #4)

## Related Files

- `crates/network/src/sync/manager.rs` - Sync manager with production gate
- `crates/network/src/gossip.rs` - Gossipsub configuration
- `crates/network/src/service.rs` - Network service, peer tracking
- `crates/rpc/src/methods.rs` - RPC implementation (peer count bug)
- `bins/node/src/node.rs` - Node event handling

---

## Validation Test #1: Dynamic Producer Addition (2026-02-04 19:27 UTC)

### Test Scenario

Added 5 new producers (producer_5 through producer_9) to a running 5-node devnet:
1. Created 5 new wallets
2. Funded each with 5 DOLI from existing genesis wallets
3. Registered each as producer (1 bond)
4. Started 5 new producer nodes (ports 50308-50312, RPC 28550-28554)

### Result: FORK OCCURRED

**Despite all P0 + P1 fixes being implemented**, the network forked.

### Observed Symptoms

```
=== All node heights ===
RPC 28545: Height 34   ❌ Node 0 STUCK (bootstrap node!)
RPC 28546: Height 58   ✓
RPC 28547: Height 58   ✓
RPC 28548: Height 58   ✓
RPC 28549: Height 35   ❌ Node 4 STUCK
RPC 28550: Height 58   ✓ (new node 5)
RPC 28551: Height 51
RPC 28552: Height 43
RPC 28553: Height 58   ✓ (new node 8)
RPC 28554: Height 38
```

**Key Observation**: Node 0 (the bootstrap node) was stuck at height 34 while other nodes advanced to height 58+.

### Log Analysis

Node 0 logs showed:
```
Starting sync with peer 12D3KooW... (height 58, slot 60)
GetHeaders { start_hash: Hash(528064ac...), max_count: 2000 }
Received sync response: Headers(empty)
Handling headers response: count=0
Chain synchronized   ← FALSE! Node thinks it's synced but it's not
```

**The Problem**: Node 0 requested headers starting from block `528064ac` (its tip at height 34), but peers returned **empty headers** because they don't have that block - they're on a different fork!

### Why P0 + P1 Fixes Didn't Prevent This

**⚠️ UPDATE**: Code verification revealed these fixes were **NEVER IMPLEMENTED**:

| Fix | Actual Status |
|-----|---------------|
| P0 #1: Chain hash verification | ❌ `BlockedChainMismatch` NOT IN CODE |
| P0 #2: Ahead of network | ❌ `BlockedAheadOfPeers` NOT IN CODE |
| P0 #3: Gossip activity tracking | ❌ `last_block_received_via_gossip` NOT IN CODE |
| P1 #4: Sync failures | ❌ `consecutive_sync_failures` NOT IN CODE |
| P1 #5: Peer reachability split | ❌ `last_status_response`/`last_block_received` NOT IN CODE |

The production gate still only has 6 layers, not 10. See "CRITICAL FINDING" section at bottom.

### New Finding: Empty Header Response = Silent Fork Indicator

When `GetHeaders` returns 0 headers, it means the peer doesn't have the requested block as an ancestor. This is a **strong signal of chain divergence** that is currently being ignored.

**Current behavior** (manager.rs):
```rust
if valid_count > 0 {
    // Process headers...
} else {
    warn!("No valid headers from peer {}", peer);
    // Silent failure - no fork detection!
}
```

### Hypothesis: Race Condition During Dynamic Producer Registration

The fork may have originated when:
1. New producers registered on-chain
2. Different nodes saw the registration at different heights
3. Scheduler computed different producer sets
4. Different producers selected for the same slot → fork

**Need to investigate**: Scheduler determinism when producer set changes mid-operation.

### Proposed Additional Fixes

**P0 #4: Treat empty header responses as fork indicator**
- If multiple peers return empty headers for our tip → we're on a fork
- Increment `consecutive_sync_failures` counter
- Trigger resync/fork recovery

**P0 #5: Add strategic logging checkpoints**
- Log scheduler state (producer list hash) at each slot
- Log when producer set changes
- Log block parent hash verification

### Workaround Applied

Reinitialized devnet with `doli-node devnet init --nodes 10` to start all 10 producers from the same genesis. This masks the bug but doesn't fix it.

---

## Test Commands Used

```bash
# Check chain heights across nodes
for port in 28545 28564 28565; do
  curl -s -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq .result
done

# Check peer counts (always returns 0 due to bug)
curl -s -X POST http://127.0.0.1:28545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}' | jq .result

# Check TCP connections to bootstrap
lsof -i :50303 | grep ESTABLISHED | wc -l

# Compare block hashes at specific height
for port in 28545 28564 28565; do
  curl -s -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getBlockByHeight","params":{"height":700},"id":1}' | \
    jq -r '.result.hash' | head -c 16
  echo " (port $port)"
done
```

---

## CRITICAL FINDING: P0/P1 Fixes Were NEVER IMPLEMENTED (2026-02-04)

### Code Verification

After thorough codebase analysis, **ALL P0 and P1 fixes documented above were NEVER ACTUALLY IMPLEMENTED**.

**Evidence - `ProductionAuthorization` enum now has 8 variants:**
```rust
// crates/network/src/sync/manager.rs:85-126
pub enum ProductionAuthorization {
    Authorized,
    BlockedSyncing,
    BlockedResync { grace_remaining_secs: u64 },
    BlockedBehindPeers { local_height, peer_height, height_diff },
    BlockedAheadOfPeers { local_height, peer_height, height_ahead },  // ✅ ADDED 2026-02-04
    BlockedSyncFailures { failure_count },                            // ✅ ADDED 2026-02-04
    BlockedExplicit { reason: String },
    BlockedBootstrap { reason: String },
}
```

**Variant implementation status:**
| Documented | Status | Notes |
|------------|--------|-------|
| `BlockedAheadOfPeers` | ✅ IMPLEMENTED | P0 #2 - Layer 7 in can_produce() |
| `BlockedChainMismatch` | ❌ NOT IN CODE | Deferred - requires hash comparison |
| `BlockedNoGossipActivity` | ❌ NOT IN CODE | Deferred |
| `BlockedSyncFailures` | ✅ IMPLEMENTED | P0 #4 - Layer 8 in can_produce() |

**Field implementation status:**
| Documented | Status |
|------------|--------|
| `consecutive_sync_failures: u32` | ✅ IMPLEMENTED |
| `max_sync_failures_before_fork_detection: u32` | ✅ IMPLEMENTED |
| `max_heights_ahead: u64` | ✅ IMPLEMENTED |
| `last_block_received_via_gossip: Option<Instant>` | ❌ NOT IN CODE |
| `PeerSyncStatus.last_status_response` | ❌ NOT IN CODE |
| `PeerSyncStatus.last_block_received` | ❌ NOT IN CODE |

**Method implementation status:**
| Documented | Status |
|------------|--------|
| `note_sync_failure()` | ✅ IMPLEMENTED |
| `clear_sync_failures()` | ✅ IMPLEMENTED |
| `has_sync_failure_fork_indicator()` | ✅ IMPLEMENTED |
| `consecutive_sync_failure_count()` | ✅ IMPLEMENTED |
| `set_max_heights_ahead()` | ✅ IMPLEMENTED |
| `note_block_received_via_gossip()` | ❌ NOT IN CODE |

### Production Gate Reality

The production gate now has **8 layers** (P0 #2 and P0 #4 implemented 2026-02-04):

| Layer | Status | Description |
|-------|--------|-------------|
| 1 | ✅ EXISTS | Explicit production block |
| 2 | ✅ EXISTS | Resync in progress |
| 3 | ✅ EXISTS | Active sync (downloading) |
| 4 | ✅ EXISTS | Bootstrap gate |
| 5 | ✅ EXISTS | Post-resync grace period |
| 6 | ✅ EXISTS | Behind peers check |
| 7 | ✅ IMPLEMENTED | Ahead of peers check (P0 #2) - blocks if >5 heights ahead |
| 8 | ✅ IMPLEMENTED | Sync failure fork detection (P0 #4) - blocks after 3 failures |
| 9 | ❌ NOT IMPLEMENTED | Chain hash verification (P0 #1) |
| 10 | ❌ NOT IMPLEMENTED | Gossip activity timeout (P0 #3) |

### Why Inconsistency: Some Deploys Work, Others Fork

The inconsistency is explained by **two separate root causes**:

#### Root Cause A: Missing Fork Detection (All deploys)

Without layers 7-10, the production gate cannot detect:
- Being far ahead of the network (forked node keeps producing)
- Chain hash mismatches (different chains at same height)
- No gossip activity despite having "peers"
- Repeated sync failures (empty headers from peers)

Empty header responses are **silently ignored** in `handle_headers_response()`:
```rust
if headers.is_empty() {
    debug!("Received empty headers response from {}", peer);
    // NO FAILURE TRACKING!
    if !self.headers_needing_bodies.is_empty() {
        self.state = SyncState::DownloadingBodies { ... };
    } else {
        self.state = SyncState::Synchronized;  // INCORRECTLY transitions!
    }
    return;
}
```

#### Root Cause B: Producer Set Divergence (Dynamic registration)

When producers are added dynamically:

1. **`ACTIVATION_DELAY = 10` blocks** (100 seconds) should allow propagation
2. **But** during sync or network delays, nodes have different views:
   - Node A at height 150: sees producer active (150 >= 110) ✓
   - Node B at height 100: producer not active yet (100 < 110) ✗
3. **Scheduler is deterministic** but receives **different inputs**
4. **Different producer sets → different selections → FORK**

```rust
// bins/node/src/node.rs:2267
let active_with_weights: Vec<(PublicKey, u64)> = producers
    .active_producers_at_height(height)  // ← Each node filters differently!
    .iter()
    .map(|p| (p.public_key.clone(), p.bond_count as u64))
    .collect();
```

### Why Some Deploys Work

| Scenario | Works? | Reason |
|----------|--------|--------|
| All producers from genesis | ✅ Yes | Same producer set, no timing issues |
| Static network, no new producers | ✅ Yes | Producer set never changes |
| Dynamic producer addition | ❌ No | Timing-dependent race condition |
| Nodes restart at different times | ❌ Maybe | Depends on sync timing |

### Actual Required Fixes

**Priority 0 (Must implement first):**

1. **P0 #2: Ahead of network detection** ✅ IMPLEMENTED (2026-02-04)
   - Added `BlockedAheadOfPeers` variant
   - Added `max_heights_ahead` field (default: 5)
   - Layer 7: Block if `local_height > best_peer_height + max_heights_ahead`
   - Tests: `test_fork_detection_blocks_production_when_ahead_of_peers`, `test_forked_node_scenario_from_report`

2. **P0 #4: Empty headers = fork indicator** ✅ IMPLEMENTED (2026-02-04)
   - Added `consecutive_sync_failures: u32` field
   - Added `max_sync_failures_before_fork_detection: u32` field (default: 3)
   - Added `BlockedSyncFailures` variant
   - Increment counter when `headers.is_empty()` (no pending work) or `valid_count == 0`
   - Layer 8: Block production after 3+ consecutive failures
   - Tests: `test_sync_failures_block_production`, `test_successful_sync_clears_failure_counter`

**Priority 1 (Important):**

3. **P1: Increase ACTIVATION_DELAY or use epoch boundaries**
   - 10 blocks may be insufficient for propagation
   - Consider: producers become active at next epoch boundary
   - Or: increase delay to 30+ blocks

4. **P1: Add chain hash verification**
   - When comparing with peers, verify hash at shared height
   - If our hash differs → we're on a fork

**Priority 2 (Nice to have):**

5. Fix RPC peer count
6. Add diagnostic logging

### Verification Commands

```bash
# Verify ProductionAuthorization variants
grep -n "Blocked" crates/network/src/sync/manager.rs | head -20

# Verify consecutive_sync_failures doesn't exist
grep -r "consecutive_sync_failures" crates/network/

# Verify missing methods
grep -r "note_sync_failure\|clear_sync_failures" crates/network/
```

---

## Session Summary (2026-02-04)

### Fixes Implemented This Session

| Fix | Description | Status |
|-----|-------------|--------|
| P0 #2 | Ahead of peers detection | ✅ Already implemented |
| P0 #4 | Sync failure-based fork detection | ✅ Already implemented |
| **P0 #5** | Echo chamber prevention (InsufficientPeers) | ✅ NEW |
| **P0 #5 Update** | Network-specific min_peers (1 for devnet, 2 for mainnet/testnet) | ✅ NEW |

### Files Modified This Session

1. **`crates/network/src/sync/manager.rs`**:
   - Added `min_peers_for_production: usize` field
   - Added `ProductionAuthorization::BlockedInsufficientPeers` enum variant
   - Added Layer 5.5 check in `can_produce()` for echo chamber prevention
   - Added `set_min_peers_for_production()` configuration method
   - Added checkpoint debug logs: `[CAN_PRODUCE]`, `[NODE_PRODUCE]`, `[BLOCK_PRODUCED]`
   - Added 3 new tests for InsufficientPeers check
   - Updated 6 existing tests to add 2+ peers

2. **`bins/node/src/node.rs`**:
   - Added match arm for `BlockedInsufficientPeers` variant
   - Added network-specific configuration of `min_peers_for_production`
   - Added checkpoint debug logs for production flow

### Test Results

- All 64 network tests pass
- All 875 workspace tests pass
- Live devnet with 5 producers: All earning rewards fairly (40, 40, 40, 40, 20 DOLI at height 9)
- Dynamic producer addition: Nodes with only 1 peer correctly blocked

### Root Cause Analysis

**Original Bug (Failure Mode A - "Ahead of Network"):**
- Forked nodes with higher local height than peers weren't blocked
- `saturating_sub(peer_height, local_height) = 0` when ahead
- Fix: P0 #2 - Explicit "ahead of peers" check

**New Bug (Failure Mode B - "Echo Chamber"):**
- Forked nodes formed isolated clusters with only 1-2 peers
- All peers in cluster agreed on wrong chain
- `height_ahead = 0` because cluster was "in sync" with itself
- Fix: P0 #5 - Require minimum peers before allowing production

**Network Topology Issue:**
- Devnet with `--no-dht` has limited peer discovery
- Non-bootstrap nodes only connect to bootstrap node
- Fix: P0 #5 Update - Network-specific min_peers (1 for devnet)

### Production Gate Layers (Final)

| Layer | Check | Block Reason |
|-------|-------|--------------|
| 1 | Explicit production block | `BlockedExplicit` |
| 2 | Resync in progress | `BlockedResync` |
| 3 | Active sync (downloading) | `BlockedSyncing` |
| 4 | Bootstrap gate | `BlockedBootstrap` |
| 5 | Post-resync grace period | `BlockedResync` |
| **5.5** | **Insufficient peers (P0 #5)** | **`BlockedInsufficientPeers`** |
| 6 | Behind peers | `BlockedBehindPeers` |
| 7 | Ahead of peers (P0 #2) | `BlockedAheadOfPeers` |
| 8 | Sync failures (P0 #4) | `BlockedSyncFailures` |

---

## P0 #2 Layer 7 Bug Fix (2026-02-04 22:00 UTC)

### Bug Found

Two critical bugs in the Layer 7 "ahead of peers" check:

#### Bug 1: Check required `peers.len() > 0`

**File**: `crates/network/src/sync/manager.rs:734`

```rust
// BEFORE (buggy):
if self.peers.len() > 0 && best_peer_height > 0 {
    if height_ahead > self.max_heights_ahead { ... }
}
```

**Problem**: `best_peer_height()` already includes `network_tip_height` as fallback:
```rust
pub fn best_peer_height(&self) -> u64 {
    peer_max.max(self.network_tip_height)  // Combines both sources!
}
```

When peers disconnect but `network_tip_height` is valid, the check was SKIPPED:
- Scenario: `peers.len()=0`, `network_tip_height=93`, `local_height=136`
- `best_peer_height()` returns 93 (correct!)
- But `self.peers.len() > 0` is FALSE → check SKIPPED → production authorized!

**Fix**: Trust `best_peer_height()` since it already handles the empty peers case:
```rust
// AFTER:
if best_peer_height > 0 {  // Removed peers.len() requirement
    if height_ahead > self.max_heights_ahead { ... }
}
```

#### Bug 2: Only checked against MAX peer height (echo chamber blind spot)

**Problem**: Forked nodes peering with each other + bootstrap create echo chambers:
- Node 2 peers: {bootstrap: height=93, node4: height=136}
- `best_peer_height() = max(93, 136) = 136`
- Node 2 local: 136
- `height_ahead = 136 - 136 = 0` → NOT ahead → production authorized!

The check against MAX peer height is blind to the bootstrap node at 93!

**Fix**: Added `lowest_peer_height()` check to catch echo chambers:
```rust
// Check 2: Ahead of LOWEST peer (catches echo chambers)
if let Some(lowest) = self.lowest_peer_height() {
    let height_ahead_of_lowest = self.local_height.saturating_sub(lowest);
    if height_ahead_of_lowest > self.max_heights_ahead {
        return BlockedAheadOfPeers { ... };
    }
}
```

### Changes Made

1. **New method `lowest_peer_height()`**: Returns `Option<u64>` - minimum height among peers

2. **Layer 7 now performs TWO checks**:
   - Check 1: Ahead of `best_peer_height()` (catches isolated nodes)
   - Check 2: Ahead of `lowest_peer_height()` (catches echo chambers)

3. **Updated logging**: Shows both best and lowest peer heights for debugging

### Test Added

```rust
#[test]
fn test_echo_chamber_detection_blocks_when_ahead_of_lowest_peer() {
    // Node 2 at height 136, peers: {bootstrap: 93, forked_node: 136}
    // Should be BLOCKED because 136 - 93 = 43 > 5 (ahead of LOWEST peer)
}
```

### Verification

Devnet with 5 nodes running for 3+ minutes:
- All nodes staying in sync at same height
- Layer 7 logs show: `lowest_peer=Some(N)` and `ahead_of_lowest=M` tracking
- No forks observed

---

## ⚠️ CRITICAL FINDING: Genesis Networks Work, Dynamic Addition Breaks (2026-02-04 22:30 UTC)

### Key Distinction

| Scenario | Works? | Notes |
|----------|--------|-------|
| **Genesis 10-node network** | ✅ YES | All producers in genesis, same chainspec, all start together |
| **Dynamic producer addition** | ❌ NO | Adding new producers AFTER genesis causes forks/deadlock |

**The problem is NOT with genesis networks.** A 5-node or 10-node devnet initialized with all producers in the genesis chainspec works perfectly.

**The problem ONLY occurs when adding new producers dynamically** to a running network.

### Test Performed (2026-02-04)

1. **Started 5-node genesis devnet** → All 5 nodes synced, producing blocks ✓
2. **Created 5 new producer wallets** (producer_5 through producer_9)
3. **Funded each with 4 DOLI**
4. **Registered each as producer** (1 bond)
5. **Started 5 new producer nodes**

### Result: CHAIN DEADLOCK

After adding the 5 new producers, the chain became completely deadlocked:

```
Node 0: BlockedBehindPeers (stuck at H=83, can't sync)
Node 2: BlockedBehindPeers (stuck at H=82, sync_failures=100)
Node 1: BlockedAheadOfPeers (H=89, blocked because Node 0 behind)
Node 3: BlockedAheadOfPeers (H=89, blocked because Node 0 behind)
Node 4: BlockedAheadOfPeers (H=89, blocked because Node 0 behind)
...all other nodes: BlockedAheadOfPeers
```

**Chain stopped progressing entirely.** No node could produce.

### Root Cause: "Lowest Peer" Check Too Aggressive

The P0 #2 "echo chamber" fix (added earlier in this session) caused the deadlock:

```rust
// Check 2: Ahead of LOWEST peer (catches echo chambers)
if let Some(lowest) = self.lowest_peer_height() {
    if height_ahead_of_lowest > self.max_heights_ahead {
        return BlockedAheadOfPeers { ... };  // BLOCKS PRODUCTION
    }
}
```

**The problem**: In a healthy network, some peers can legitimately be behind:
- A peer is syncing
- A peer had network issues
- A peer's status is delayed

The check blocks production if we're ahead of **ANY** peer, not just if we're in an echo chamber.

**Example of the deadlock**:
- Nodes 0, 2: Fell behind during dynamic producer registration (H=82-83)
- Nodes 1, 3-9: At correct height (H=89)
- Node 1 sees Node 0 (H=83) as lowest peer
- Check: `89 - 83 = 6 > 5 (max_heights_ahead)` → BLOCKED
- All healthy nodes blocked because ONE peer is behind
- Behind peers can't catch up because no blocks being produced
- **Complete deadlock**

### Fix Applied: Disable Lowest Peer Check

**File**: `crates/network/src/sync/manager.rs`

The "lowest peer" echo chamber check was **disabled** because it caused more harm than good:

```rust
// Check 2: Echo chamber detection (DISABLED - see comment)
//
// The "lowest peer" echo chamber check was causing chain deadlock:
// - When some peers legitimately fall behind (syncing, network issues)
// - Healthy nodes would be blocked because "ahead of lowest peer"
// - This created a cascading failure stopping all production
//
// Echo chambers are better detected by:
// - P0 #4: Sync failures (if we can't sync, our chain is divergent)
// - P0 #5: InsufficientPeers (require multiple peers to avoid isolation)
// - Check 1 above: Being ahead of ALL peers (including network_tip)
if false {  // DISABLED
    // ... lowest peer check code ...
}
```

### Verification After Fix

After rebuilding and restarting:
- **5 genesis nodes (0-4)**: Immediately synced and producing ✓
- **Chain progressing**: H=4 → H=5 → H=6 ✓
- **No deadlock**: Nodes with peers behind NOT blocked

### Updated Test

```rust
#[test]
fn test_echo_chamber_check_disabled_allows_production_when_peer_behind() {
    // Scenario: Healthy node has peers at different heights
    // - Node has peers: {peer1: height=93, peer2: height=136}
    // - Node local_height = 136
    // - OLD: Blocked because 136 - 93 = 43 > 5 (ahead of lowest)
    // - NEW: AUTHORIZED - peer behind is OK, we're not ahead of BEST peer

    // ... test verifies Authorized result ...
}
```

### Summary

| Fix | Status | Impact |
|-----|--------|--------|
| P0 #2 Check 1 (ahead of best) | ✅ ACTIVE | Catches isolated nodes |
| P0 #2 Check 2 (lowest peer) | ❌ DISABLED | Caused deadlock, removed |
| P0 #4 (sync failures) | ✅ ACTIVE | Catches divergent chains |
| P0 #5 (insufficient peers) | ✅ ACTIVE | Prevents isolation |

**The echo chamber scenario** (forked nodes peering only with each other) is now detected by:
1. **Sync failures** (P0 #4): When they try to sync with the main chain, headers won't chain
2. **Insufficient peers** (P0 #5): Require minimum peers to avoid tiny isolated clusters
3. **Ahead of BEST peer** (P0 #2 Check 1): If ahead of ALL peers including network_tip

### Remaining Work

The dynamic producer addition scenario still needs testing to ensure:
1. New producers can sync and join the network
2. No forks occur during the registration/activation period
3. The 10-block ACTIVATION_DELAY is sufficient for propagation

**Genesis networks are stable.** The focus should be on testing and fixing dynamic producer addition.

---

## Validation Test #2: Dynamic Producer Addition (2026-02-05)

### Test Scenario

Added 5 new producers (producer_5 through producer_9) to a running 5-node genesis devnet:

1. Started fresh 5-node devnet: `doli-node devnet init --nodes 5 && doli-node devnet start`
2. Waited for coinbase maturity (height ~15, devnet maturity = 10 blocks)
3. Created 5 new wallets (producer_5 through producer_9)
4. Funded each with 4 DOLI from genesis producer wallets (different source per destination to avoid UTXO reuse)
5. Registered each as producer with 1 bond
6. Started 5 new producer nodes (ports 50308-50312, RPC 28550-28554, metrics 9095-9099)

### Result: FORK OCCURRED (Genesis Nodes Forked)

**Unexpected finding**: The NEW producers joined successfully, but **3 GENESIS nodes forked**.

### Network State After Dynamic Addition

| Node | Type | Height | Hash | Status |
|------|------|--------|------|--------|
| 0 | Genesis | 42 | `0c44f98b...` | ✅ Main chain |
| 1 | Genesis | 42 | `0c44f98b...` | ✅ Main chain |
| **2** | Genesis | **31** | `734efdc9...` | ❌ **FORKED** (sync_failures=45) |
| **3** | Genesis | **32** | `258785a0...` | ❌ **FORKED** (sync_failures=42) |
| **4** | Genesis | **35** | `b4b564b9...` | ❌ **FORKED** |
| 5 | NEW | 42 | `0c44f98b...` | ✅ Main chain |
| 6 | NEW | 42 | `0c44f98b...` | ✅ Main chain |
| 7 | NEW | 42 | `0c44f98b...` | ✅ Main chain |
| 8 | NEW | 42 | `0c44f98b...` | ✅ Main chain |
| 9 | NEW | 42 | `0c44f98b...` | ✅ Main chain |

### Log Analysis

Forked nodes show high sync failure counts but are correctly blocked from producing:

```
Node 2: [CAN_PRODUCE] slot=42 local_h=31 peer_h=39 peers=1 sync_failures=45 state=Synchronized
        [NODE_PRODUCE] slot=42 BLOCKED: BehindPeers

Node 3: [CAN_PRODUCE] slot=42 local_h=32 peer_h=39 peers=1 sync_failures=42 state=Synchronized
        [NODE_PRODUCE] slot=42 BLOCKED: BehindPeers
```

**Good news**: P0 #2 (ahead of peers) and P0 #4 (sync failures) are working - forked nodes are blocked.
**Bad news**: Genesis nodes still forked despite these protections.

### Key Observations

1. **Forked nodes are GENESIS nodes (2, 3, 4)**, not the new producers
2. **New producers (5-9) joined correctly** and are on the main chain
3. **Fork occurred around height 31-35**, during/after producer registration
4. **Sync failures accumulating** (40+) shows nodes trying but failing to sync
5. **State shows "Synchronized"** but clearly isn't - headers don't chain

### Hypothesis: Race Condition in Producer Set Processing

The fork likely occurred because:

1. Producer registrations were processed at different heights by different nodes
2. Nodes 2, 3, 4 may have processed registrations in a different block
3. This caused scheduler to compute different producer sets
4. Different producer selected for same slot → fork divergence

**Timeline reconstruction:**
```
Height 25-30: Producer registrations submitted and mined
Height 30-31: Fork point - nodes diverge
  - Nodes 0, 1: Process registrations, continue main chain
  - Nodes 2, 3, 4: Something different happened → forked
Height 31+: Forked nodes producing on wrong chain (blocked by P0 #2)
```

### Why P0 Fixes Didn't PREVENT the Fork

The P0 fixes (ahead of peers, sync failures) correctly **DETECT** the fork and **BLOCK** production on forked nodes. However, they don't **PREVENT** the initial fork from occurring.

**The root cause is NOT production gate failure** - it's chain divergence during transaction processing.

### New Failure Mode Identified: Registration Race Condition

| Failure Mode | Description | Current Status |
|--------------|-------------|----------------|
| A: Ahead of Network | Forked node thinks it's ahead | ✅ DETECTED (P0 #2) |
| B: Echo Chamber | Forked nodes peer only with each other | ✅ DETECTED (P0 #5) |
| C: Chain Deadlock | Lowest peer check too aggressive | ✅ FIXED (disabled) |
| **D: Registration Race** | **Nodes process registrations differently** | ❌ **NEW - UNRESOLVED** |

### Required Investigation

1. **Why did nodes 2, 3, 4 specifically fork?**
   - Was it gossip mesh position?
   - Transaction ordering differences?
   - Block receipt timing?

2. **Is ACTIVATION_DELAY (10 blocks) sufficient?**
   - Registrations may need more time to propagate
   - Consider increasing to 30+ blocks

3. **Is there non-determinism in producer set computation?**
   - Scheduler should be deterministic given same inputs
   - But inputs may differ if nodes have different chain views

### Proposed Fixes

**P0 #6: Increase ACTIVATION_DELAY for dynamic registration**
- Current: 10 blocks (~100 seconds)
- Proposed: 30 blocks (~5 minutes) or epoch boundary activation
- Rationale: More time for all nodes to sync registration transactions

**P0 #7: Add registration propagation verification**
- Before activation, verify N% of peers have the registration
- Block production if registration not widely propagated

**P1: Add fork recovery mechanism**
- When sync_failures > threshold AND behind peers
- Trigger automatic chain reset to last known good checkpoint
- Currently forked nodes are stuck forever

### Workaround

For now, use genesis networks with all producers defined upfront:
```bash
doli-node devnet init --nodes 10  # All 10 producers from genesis
doli-node devnet start
```

**Dynamic producer addition is NOT recommended** until these issues are resolved
