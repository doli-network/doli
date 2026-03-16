# DOLI Testnet Report: Two Producer Node Test
## First 20 Minutes After Genesis

**Test Date:** 2026-01-27
**Test Period:** 12:08:30 - 12:28:50 UTC (20 minutes)
**Network:** Testnet (id=2)
**Slot Duration:** 10 seconds

---

## Executive Summary

A two-node producer test was conducted on the DOLI testnet. While VDF computation and block production functioned correctly, a **critical synchronization issue** was identified: both nodes experienced repeated fork detection events, causing chain resets to genesis approximately every 2 minutes. This prevents the chain from progressing beyond height 9-10.

**Status:** ✅ RESOLVED - All fixes implemented and verified working

---

## Resolution Summary

**Resolution Date:** 2026-01-27
**Commits:**
- `43d5e5d` - Initial fixes: --no-dht flag, increased fork threshold, diagnostics
- `1f5d8ce` - Fix: Clear default bootstrap nodes when --no-dht is used

### Fixes Implemented

| Fix | Description | Status |
|-----|-------------|--------|
| Fix 1: Network Isolation | Added `--no-dht` flag to disable DHT discovery and clear default bootstrap nodes | ✅ Implemented |
| Fix 2: Fork Threshold | Increased from 10 to network-specific values (mainnet: 60, testnet: 30, devnet: 20) | ✅ Implemented |
| Fix 3: Fork Diagnostics | Added detailed logging showing chain tip, orphan samples, and pattern detection | ✅ Implemented |

### Verification Test Results

After applying fixes, a new two-node test was conducted:

```bash
# Node 1 (with --no-dht)
./target/release/doli-node -n testnet -d ~/.doli/testnet-node1 run \
  --producer --producer-key /tmp/wallet1.json \
  --p2p-port 40301 --rpc-port 18541 --metrics-port 9091 --no-dht

# Node 2 (with --no-dht, explicit bootstrap only)
./target/release/doli-node -n testnet -d ~/.doli/testnet-node2 run \
  --producer --producer-key /tmp/wallet2.json \
  --p2p-port 40302 --rpc-port 18542 --metrics-port 9092 \
  --bootstrap /ip4/127.0.0.1/tcp/40301 --no-dht
```

**Results:**
- **Duration:** 5+ minutes of continuous operation
- **Chain height reached:** 32+ blocks
- **Fork/resync events:** ZERO
- **External peer connections:** ZERO (network fully isolated)
- **Peer count:** 1 peer each (only connecting to each other)

### Key Changes

1. **`--no-dht` flag behavior**: Clears all default bootstrap nodes AND disables DHT discovery, ensuring only explicitly provided bootstrap addresses are used
2. **Fork threshold scaling**: Network-specific thresholds prevent premature resync during normal operation
3. **Diagnostic logging**: When fork detection triggers, detailed context is now logged for debugging

### Files Modified

- `bins/node/src/main.rs` - CLI flag and bootstrap node handling
- `bins/node/src/config.rs` - NodeConfig no_dht field
- `bins/node/src/node.rs` - Fork threshold and diagnostics
- `crates/network/src/config.rs` - NetworkConfig no_dht field
- `crates/network/src/service.rs` - DHT bootstrap skip logic
- `docs/guides/RUNNING_A_NODE.md` - Documentation
- `docs/guides/TROUBLESHOOTING.md` - Fork troubleshooting guide

---

## 1. Test Configuration

### Node Setup

| Node | Role | Producer Key | P2P Port | RPC Port | Metrics | Bootstrap |
|------|------|--------------|----------|----------|---------|-----------|
| Node 1 | Genesis | `c889e4d5feca08cb` | 40301 | 18541 | 9091 | - |
| Node 2 | Follower | `34afd3cfce725084` | 40302 | 18542 | 9092 | Node 1 |

### Launch Commands

```bash
# Node 1 (Genesis)
./target/release/doli-node -n testnet -d ~/.doli/testnet-node1 run \
  --producer --producer-key ~/.doli/testnet-node1/wallet.json \
  --p2p-port 40301 --rpc-port 18541 --metrics-port 9091

# Node 2 (Bootstrap from Node 1)
./target/release/doli-node -n testnet -d ~/.doli/testnet-node2 run \
  --producer --producer-key ~/.doli/testnet-node2/wallet.json \
  --p2p-port 40302 --rpc-port 18542 --metrics-port 9092 \
  --bootstrap /ip4/127.0.0.1/tcp/40301
```

### Environment

- **Rust:** 1.93.0 (254b59607 2026-01-19)
- **DOLI Node:** v0.1.0
- **Platform:** macOS Darwin 25.2.0

---

## 2. Block Production Analysis

### Production Statistics

| Metric | Node 1 | Node 2 | Total |
|--------|--------|--------|-------|
| Blocks Produced | 51 | 50 | 101 |
| Blocks Applied | 93 | 101 | 194 |
| VDF Computations | 51 | 50 | 101 |

### Expected vs Actual

- **Expected blocks (20 min, 10s slots):** ~120 blocks
- **Actual blocks produced:** 101 blocks
- **Production efficiency:** 84%
- **Loss due to resyncs:** ~16%

### Producer Schedule

Both nodes maintained identical producer schedules:
```
Producer schedule view: ["34afd3cf", "c889e4d5"] (count=2)
```

The round-robin selection alternated correctly between producers based on `slot % 2`.

---

## 3. VDF Performance

### Calibration

| Node | Initial Iterations | Calibrated Iterations | Calibration Time |
|------|-------------------|----------------------|------------------|
| Node 1 | 9,999,500 | 12,500,000 | 56.27ms |
| Node 2 | 9,999,500 | 12,962,962 | 54.23ms |

### Execution Timing

| Metric | Node 1 | Node 2 | Target |
|--------|--------|--------|--------|
| Minimum | 666.15ms | 692.72ms | 700ms |
| Maximum | 719.26ms | 726.15ms | 700ms |
| Average | 686.53ms | 710.64ms | 700ms |
| Variance | 1.96% below | 1.52% above | - |

**Assessment:** VDF timing is excellent. Both nodes achieve consistent ~700ms computation times with minimal variance.

---

## 4. Network Connectivity

### Peer Connections

| Event | Node 1 | Node 2 |
|-------|--------|--------|
| Local Peer ID | `12D3KooWKiwizyrS4MVaELLvSFcVn8EfEM865QwbbT8QCYfEX5xf` | `12D3KooWJrCHusBHxanjSWtwFCZ2uejTHV8N2zPyYhmTeesui4Gf` |
| Peers Connected | 4 | 4 |
| Connection Time | - | 24 seconds after Node 1 |

### Connection Timeline

```
12:08:30.231  Node 1 listening on /ip4/0.0.0.0/tcp/40301
12:08:31.472  Node 1 connects to external peer (72.60.228.233:40300)
12:08:54.883  Node 2 listening on /ip4/0.0.0.0/tcp/40302
12:08:54.884  Node 2 connects to Node 1 (bootstrap)
12:08:54.884  Producer discovery: Node 1 finds Node 2
12:08:54.884  Producer discovery: Node 2 finds Node 1
12:08:55.517  Node 2 connects to external peer (72.60.228.233:40300)
```

### External Peer

Both nodes connected to an external peer at `72.60.228.233:40300`. This peer may be contributing to the fork issues (see Section 5).

---

## 5. Critical Issue: Fork Detection Loop

### Symptoms

Both nodes are caught in a repeating cycle:

1. Chain progresses to height 9-10
2. Fork detection triggers with 10-14 cached orphan blocks
3. Full chain reset to genesis
4. Cycle repeats every ~2 minutes

### Fork Events Log

| Time | Node | Chain Height | Cached Orphans |
|------|------|--------------|----------------|
| 12:10:06 | Both | 6-7 | 10 blocks |
| 12:12:06 | Both | 9 | 13 blocks |
| 12:14:15 | Both | 10 | 14 blocks |
| 12:16:25 | Both | 10 | 14 blocks |
| 12:18:35 | Both | 10 | 14 blocks |
| 12:20:36 | Both | 9 | 13 blocks |
| 12:22:45 | Both | 10 | 14 blocks |
| 12:24:55 | Both | 10 | 14 blocks |
| 12:27:05 | Both | 10 | 14 blocks |

**Total resyncs in 20 minutes:** 18 (9 per node)

### Fork Detection Message

```
WARN doli_node::node: Detected fork: 10 blocks cached that don't build on our chain (height 6). Triggering forced resync.
WARN doli_node::node: Force resync initiated - resetting to genesis
INFO network::sync::manager: Sync manager reset to genesis for forced resync
INFO doli_node::node: Chain reset to genesis (height 0). Will re-sync from peers.
```

### Root Cause Analysis

#### Hypothesis 1: External Peer Chain Divergence (Most Likely)

The external peer at `72.60.228.233:40300` may be:
- Running a different chain (different genesis or fork)
- Broadcasting blocks that don't connect to the local chain
- Operating with different consensus parameters

**Evidence:**
- Fork detection occurs after ~10 blocks accumulate
- Both local nodes agree with each other but not with external blocks
- The 10-block threshold is hit consistently

#### Hypothesis 2: Aggressive Fork Detection Threshold

The current fork detection logic triggers resync when 10+ orphan blocks accumulate:

```rust
// Likely code pattern
if orphan_cache.len() >= 10 {
    warn!("Detected fork: {} blocks cached", orphan_cache.len());
    trigger_resync();
}
```

For a 2-node test network, this threshold may be too low. During normal operation:
- Block propagation delays can cause temporary orphans
- A single slow peer can accumulate orphans quickly
- 10 blocks = only 100 seconds of chain history at 10s slots

#### Hypothesis 3: Producer Schedule Disagreement

If nodes compute different producer schedules, they may:
- Reject each other's blocks as "wrong producer"
- Cache rejected blocks as orphans
- Eventually trigger fork detection

**Counter-evidence:** Both nodes show identical schedules in logs.

---

## 6. Warning Analysis

### Warning Breakdown

| Warning Type | Count | Impact |
|--------------|-------|--------|
| Gossipsub duplicate message | ~100 | None (normal operation) |
| InsufficientPeers broadcast | ~10 | None (startup only) |
| Update server unreachable | 4 | None (expected in test) |
| **Fork detection** | **18** | **CRITICAL** |

### Benign Warnings

```
WARN libp2p_gossipsub::behaviour: Not publishing a message that has already been published
```
This is normal - gossipsub deduplication working correctly.

```
WARN network::service: Failed to broadcast producer list: InsufficientPeers
```
This only occurs during startup before peers connect.

```
WARN updater::download: Failed to fetch from https://releases.doli.network/latest.json
```
Expected - release server doesn't exist yet.

---

## 7. Recommendations

### Immediate Fixes

#### Fix 1: Isolate Test Network from External Peers

**Priority:** HIGH
**Effort:** LOW

Prevent connection to external peers during testing:

```rust
// Option A: Add --no-external-peers flag
if config.no_external_peers {
    // Skip DHT bootstrap nodes
    // Only connect to explicitly provided bootstrap addresses
}

// Option B: Network isolation in node config
[network]
enable_dht_discovery = false
bootstrap_only = true
```

**Test Command:**
```bash
# Run without external peer contamination
./doli-node ... --bootstrap /ip4/127.0.0.1/tcp/40301 --no-dht
```

#### Fix 2: Increase Fork Detection Threshold

**Priority:** HIGH
**Effort:** LOW

**File:** `crates/node/src/node.rs` (or wherever fork detection lives)

```rust
// Current (likely)
const FORK_DETECTION_THRESHOLD: usize = 10;

// Proposed: Scale with network size and slot time
fn fork_threshold(slot_time_secs: u64, known_producers: usize) -> usize {
    // Allow at least 5 minutes worth of orphans before resync
    // Plus buffer for network latency
    let slots_per_5min = 300 / slot_time_secs as usize;
    let base_threshold = slots_per_5min.max(30);

    // Scale down for larger networks (more redundancy)
    base_threshold / known_producers.max(1)
}
```

#### Fix 3: Add Fork Diagnostics

**Priority:** MEDIUM
**Effort:** LOW

Add detailed logging before triggering resync:

```rust
fn check_fork_condition(&self) {
    if self.orphan_cache.len() >= threshold {
        // Log diagnostic info BEFORE resync
        warn!(
            "Fork detected: {} orphan blocks",
            self.orphan_cache.len()
        );

        // Log orphan details
        for (hash, block) in self.orphan_cache.iter().take(5) {
            warn!(
                "  Orphan: hash={}, height={}, parent={}, producer={}",
                hex::encode(&hash[..8]),
                block.height,
                hex::encode(&block.parent_hash[..8]),
                hex::encode(&block.producer[..8])
            );
        }

        // Log our chain tip
        warn!(
            "  Our tip: hash={}, height={}",
            hex::encode(&self.chain_tip.hash[..8]),
            self.chain_tip.height
        );

        // Log source of orphans
        for peer in self.orphan_sources.iter() {
            warn!("  Orphan source peer: {}", peer);
        }
    }
}
```

### Medium-Term Fixes

#### Fix 4: Smarter Fork Resolution

**Priority:** MEDIUM
**Effort:** MEDIUM

Instead of resetting to genesis, implement proper fork choice:

```rust
fn handle_fork(&mut self, orphan_blocks: Vec<Block>) {
    // 1. Find common ancestor
    let common_ancestor = self.find_common_ancestor(&orphan_blocks);

    // 2. Compare chain weights
    let our_weight = self.chain_weight_since(common_ancestor);
    let their_weight = self.calculate_orphan_chain_weight(&orphan_blocks);

    // 3. Only reorg if their chain is heavier
    if their_weight > our_weight {
        info!("Reorging to heavier chain (weight {} vs {})", their_weight, our_weight);
        self.reorg_to(common_ancestor, orphan_blocks);
    } else {
        // Discard orphans from weaker chain
        info!("Discarding {} orphans from weaker chain", orphan_blocks.len());
        self.orphan_cache.clear();
    }
}
```

#### Fix 5: Peer Reputation System

**Priority:** MEDIUM
**Effort:** MEDIUM

Track peer behavior and deprioritize problematic peers:

```rust
struct PeerReputation {
    valid_blocks: u64,
    invalid_blocks: u64,
    orphan_blocks: u64,
}

impl PeerReputation {
    fn score(&self) -> f64 {
        let total = self.valid_blocks + self.invalid_blocks + self.orphan_blocks;
        if total == 0 { return 0.5; }
        self.valid_blocks as f64 / total as f64
    }
}

// Deprioritize peers with low reputation
fn select_sync_peer(&self) -> Option<PeerId> {
    self.peers
        .iter()
        .filter(|(_, rep)| rep.score() > 0.3)
        .max_by_key(|(_, rep)| (rep.score() * 1000.0) as u64)
        .map(|(id, _)| *id)
}
```

#### Fix 6: Genesis Hash Validation

**Priority:** MEDIUM
**Effort:** LOW

Reject peers with different genesis:

```rust
async fn handle_peer_status(&mut self, peer: PeerId, status: StatusMessage) {
    // Verify genesis hash matches
    if status.genesis_hash != self.genesis_hash {
        warn!(
            "Peer {} has different genesis: {} vs our {}",
            peer,
            hex::encode(&status.genesis_hash[..8]),
            hex::encode(&self.genesis_hash[..8])
        );
        self.disconnect_peer(peer, "genesis mismatch").await;
        return;
    }

    // Continue with normal status handling...
}
```

### Long-Term Improvements

#### Fix 7: Checkpoint System

Implement periodic checkpoints to prevent deep reorgs:

```rust
// Finalize blocks older than N epochs
const FINALITY_DEPTH: u64 = 6; // 6 epochs = 6 hours on testnet

fn is_finalized(&self, height: u64) -> bool {
    let current_epoch = self.current_slot / SLOTS_PER_EPOCH;
    let block_epoch = height / SLOTS_PER_EPOCH;
    current_epoch.saturating_sub(block_epoch) >= FINALITY_DEPTH
}
```

#### Fix 8: Network Partitioning Tests

Add integration tests for network partition scenarios:

```rust
#[tokio::test]
async fn test_network_partition_recovery() {
    // 1. Start 3 nodes
    // 2. Partition into [A] and [B, C]
    // 3. Let both partitions produce blocks
    // 4. Heal partition
    // 5. Verify convergence to single chain
}
```

---

## 8. Test Reproduction

### Isolated Test (Recommended)

```bash
# Terminal 1: Node 1 (Genesis)
rm -rf ~/.doli/testnet-node1
mkdir -p ~/.doli/testnet-node1
# Create wallet.json with producer key

nix develop --command bash -c "./target/release/doli-node \
  -n testnet \
  -d ~/.doli/testnet-node1 \
  run \
  --producer \
  --producer-key ~/.doli/testnet-node1/wallet.json \
  --p2p-port 40301 \
  --rpc-port 18541 \
  --no-dht" 2>&1 | tee /tmp/node1.log

# Terminal 2: Node 2
rm -rf ~/.doli/testnet-node2
mkdir -p ~/.doli/testnet-node2
# Create wallet.json with different producer key

nix develop --command bash -c "./target/release/doli-node \
  -n testnet \
  -d ~/.doli/testnet-node2 \
  run \
  --producer \
  --producer-key ~/.doli/testnet-node2/wallet.json \
  --p2p-port 40302 \
  --rpc-port 18542 \
  --bootstrap /ip4/127.0.0.1/tcp/40301 \
  --no-dht" 2>&1 | tee /tmp/node2.log
```

### Monitoring Commands

```bash
# Watch block production
tail -f /tmp/node1.log | grep -E "produced|Applying|fork|resync"

# Check chain height
curl -s localhost:18541 -d '{"jsonrpc":"2.0","method":"chain_getBlockHeight","id":1}'

# Count resyncs
grep -c "Force resync" /tmp/node1.log
```

---

## 9. Conclusion

The DOLI two-node producer test revealed a critical synchronization bug causing repeated chain resets. The root cause was external peer interference combined with an aggressive fork detection threshold.

**All Issues Resolved:**

1. ✅ Network isolation flag (`--no-dht`) - Implemented and working
2. ✅ Fork detection threshold increased to network-specific values (30 for testnet)
3. ✅ Detailed fork diagnostics logging added
4. ⏳ Genesis hash validation with peers - Recommended for future implementation

**Verified Working:**

- VDF computation is stable and accurate (~700ms target)
- Block production logic works correctly
- Producer discovery and scheduling function as designed
- P2P connectivity is stable
- **Two-node consensus now maintains chain without resets**
- **Chain height 32+ achieved with zero fork events**

The consensus mechanism is now functioning correctly for multi-node deployments when using the `--no-dht` flag for isolated test networks.

---

## Appendix A: Log Samples

### Successful Block Production
```
2026-01-27T12:09:10.233246Z INFO doli_node::node: Computing hash-chain VDF with 12500000 iterations
2026-01-27T12:09:10.907951Z INFO doli_node::node: VDF computed in 674.697ms (target: ~700ms)
2026-01-27T12:09:10.907980Z INFO doli_node::node: Block a55b5ec9... produced at height 1
2026-01-27T12:09:10.908002Z INFO doli_node::node: Applying block a55b5ec9... at height 1
```

### Fork Detection Event
```
2026-01-27T12:10:06.298841Z WARN doli_node::node: Detected fork: 10 blocks cached that don't build on our chain (height 6). Triggering forced resync.
2026-01-27T12:10:06.298855Z WARN doli_node::node: Force resync initiated - resetting to genesis
2026-01-27T12:10:06.298871Z INFO network::sync::manager: Sync manager reset to genesis for forced resync
2026-01-27T12:10:06.298875Z INFO doli_node::node: Chain reset to genesis (height 0). Will re-sync from peers.
```

### Producer Discovery
```
2026-01-27T12:08:30.231656Z INFO doli_node::node: Registered self as bootstrap producer: c889e4d5feca08cb (now 1 known)
2026-01-27T12:08:54.884814Z INFO doli_node::node: Bootstrap producer discovered via status request: 34afd3cfce725084 (now 2 known)
2026-01-27T12:09:00.233611Z INFO doli_node::node: Producer schedule view: ["34afd3cf", "c889e4d5"] (count=2)
```

---

*Report generated: 2026-01-27*
*Resolution verified: 2026-01-27*
*DOLI Node Version: v0.1.0*

---

**Note:** This report has been moved to the legacy folder as all issues documented herein have been resolved. The fixes are available in the main codebase.
