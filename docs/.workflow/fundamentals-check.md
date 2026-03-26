# Fundamentals Check: INC-I-012 50-Node Stress Test

**Date**: 2026-03-25
**INC_ID**: INC-I-012
**RUN_ID**: 72
**Context**: Local Mac stress test with 50+ doli-node processes to find sync/gossip vulnerabilities

## Resource Verification

### Disk Space
**Command**: `df -h /Users/isudoajl/ownCloud/Projects/doli`
**Output**: `/dev/disk3s5   926Gi   563Gi   296Gi    66%`
**Status**: **PASS** — 296GB available (66% used), sufficient for 50-node stress test

### Memory
**Command**: `sysctl hw.memsize` + `vm_stat`
**Output**: 128GB total, ~84GB free, ~40GB used
**Status**: **PASS** — 84GB free should handle 50 nodes (previous session: 86GB explosion was at 136 nodes with max_peers=200)

### CPU/Load
**Command**: `uptime`
**Output**: `load averages: 19.82 18.31 18.51` (16-core system)
**Status**: **FAIL** — Load average >18 on 16-core system indicates current stress. System already under heavy load before test begins

### File Descriptors
**Command**: `ulimit -n`
**Output**: `unlimited`
**Status**: **PASS** — No file descriptor limits

### Network/Dependencies
**Status**: **N/A** — Local-only test, no external dependencies

## Configuration Verification

### Environment Variables
**Command**: `env | grep -i "doli\|mesh\|gossip"`
**Output**: No DOLI-specific env vars set
**Status**: **PASS** — Using default hardcoded parameters

### Configuration Files Present
**Command**: Checked `chainspec.testnet.json`, `crates/core/src/network_params/defaults.rs`
**Status**: **PASS** — Testnet chainspec exists, network params correctly configured

### Binary Version/Build
**Command**: `./target/release/doli-node --version`
**Output**: `doli-node 4.0.4 (2c94404e)` — includes latest INC-I-012 conn_headroom fix
**Status**: **PASS** — Binary is current and includes recent network fixes

### Network Parameters (Critical for 50-node test)
**Values Found**:
- `max_peers`: 50 (was 200, fixed in 16d6a5fb for INC-I-009)
- `mesh_n`: 12, `mesh_n_high`: 24, `gossip_lazy`: 12
- `conn_headroom`: max_peers*2+20 = 120 (restored in 2c94404e for INC-I-012)

**Status**: **PASS** — Parameters configured for 50-node capacity

## Recent Changes

**Command**: `git log --since="3 days ago" --oneline`
**Key Changes**:
- `2c94404e` — conn_headroom fix (max_peers*2+20) to unblock bootstrap
- `928440d3` — eviction cooldown to break reconnect thrashing
- `16d6a5fb` — max_peers reduced 200→50 for Yamux buffer explosion
- Multiple sync/consensus fixes from INC-I-006 through INC-I-008

**Status**: **PASS** — Recent fixes directly address network scaling issues found in previous stress tests

## Capacity Check (Critical Analysis)

### Physical Capacity for 50 Nodes
**CPU**: 16 cores, but load already >18 — **POTENTIAL ISSUE**
**Memory**: 84GB free vs previous 86GB explosion at 136 nodes — **MARGINAL**
**Connections**: 50 nodes × mesh_n(12) = ~600 active gossip connections total — **WITHIN LIMITS**

### Network Math Verification
**50 nodes with max_peers=50**:
- Each node can connect to up to 50 others ✓
- With mesh_n=12, each node maintains 12 eager gossip connections ✓
- Total system connections: ~300-600 (depends on mesh overlap) ✓
- conn_headroom=120 per node supports this ✓

**Status**: **CONDITIONAL PASS** — Within designed limits but system already under load

## Dimensional Audit

| Axis | Assessment | Implication |
|------|------------|-------------|
| **Scale/Volume** | 50 nodes approaching hardware limits. Previous explosion at 136 nodes with max_peers=200. Current max_peers=50 should be safe but system load already high (19.8). | Scale is at the upper boundary of single-machine capacity. Any additional load could tip into resource exhaustion. |
| **Time/Latency** | mesh_n=12 with 10s slots. Gossip propagation time should be <1s across 50 nodes at 3-4 hops max. Previous GSet flooding consumed seed event loop. | Time margins adequate IF gossip flooding is controlled. GSet fix status unknown. |
| **Complexity** | 50 interacting nodes with gossipsub mesh, sync state machines, fork recovery. Previous cascade failures due to interaction effects (sync→RAM→disconnection→reorg loops). | High complexity. Previous failures were emergent behaviors from component interactions, not simple bugs. |

## Hard-Stop Assessment

**Status**: **NO HARD-STOP** — System is at capacity limits but within designed parameters
**WARNING**: Current load average 19.8 on 16-core system indicates pre-existing stress

## Invariant Mapping

**Expected**: Network should handle 50 nodes smoothly with max_peers=50, mesh_n=12
**Previous Violations**:
1. **Resource bounds**: Yamux buffer explosion (86GB RAM) violated memory limits
2. **Backpressure**: GSet gossip flooding overwhelmed event loops
3. **Consistency**: ProducerSet scheduled flag divergence after snap sync

**Current Risk**: Resource bounds invariant (load average should remain <16 on 16-core system)

## Silent Absence Check

### Rate Limiters
**GSet gossip flooding**: Previous session identified 106 nodes × mesh_n × gossip_lazy = ~1200 messages/round
**Status**: **MISSING** — No rate limiting on producer announcements identified

### Circuit Breakers
**Event loop saturation**: Seed event loops reached 100% CPU on GSet processing
**Status**: **MISSING** — No circuit breaker when event processing falls behind

### Backpressure
**Channel buffers**: Fixed with proportional sizing (max_peers * 16 for events)
**Status**: **PRESENT** — Channel buffer sizing scales with max_peers

### Retry Limits
**Reconnect thrashing**: Fixed with eviction cooldown (928440d3)
**Status**: **PRESENT** — Eviction cooldown prevents reconnect loops

### Bounds Checking
**Connection limits**: conn_headroom formula restored (max_peers*2+20)
**Status**: **PRESENT** — Connection limits properly enforced

## Occam's Razor Ordering

**Level 1 (Configuration)**: CHECKED — max_peers=50, mesh_n=12, conn_headroom=120 correct for 50-node test
**Level 2 (Resources)**: CHECKED — 84GB free RAM, but load average 19.8 shows stress
**Level 3 (Dependencies)**: N/A — Local test, no external dependencies
**Level 4 (Data)**: N/A — Fresh testnet, no corrupt data expected
**Level 5 (Build/Deploy)**: CHECKED — Binary current (2c94404e) with latest fixes
**Level 6 (Logic errors)**: CHECKED — Recent fixes address known conn_headroom/eviction bugs
**Level 7 (Interaction effects)**: **LIKELY** — Previous failures were gossip×sync×memory interactions
**Level 8 (Race conditions)**: **POSSIBLE** — ProducerSet scheduled flag divergence suggests timing issues
**Level 9 (Emergent behavior)**: **LIKELY** — GSet flooding was emergent property of mesh×node-count

## Initial Prediction

**Root Cause Prediction**: **Interaction effects (Level 7)** or **Emergent behavior (Level 9)**
**Reasoning**: Previous session identified GSet gossip flooding as primary bottleneck. This is an emergent property of mesh_n × node_count that creates O(n²) message amplification. The fixes (conn_headroom, eviction cooldown, max_peers reduction) address symptoms but may not eliminate the GSet root cause.

**Confidence**: 7/10
**If Wrong, Check**: Resource exhaustion (pre-existing load 19.8), missing rate limiters on producer announcements, or ProducerSet sync bugs causing fork recovery deadlocks

## Critical Flags

1. **System already under load**: 19.8 load average before test begins
2. **GSet flooding fix status unknown**: Primary bottleneck from previous session
3. **ProducerSet divergence unresolved**: Fork recovery deadlock mentioned but no fix committed
4. **Marginal memory capacity**: 84GB free vs 86GB previous explosion (though at 136 vs 50 nodes)

## Recommendation

**PROCEED** with enhanced monitoring:
1. Monitor system load and memory usage continuously during test
2. Watch for GSet message amplification patterns in logs
3. Check ProducerSet consistency if fork recovery issues appear
4. Be prepared to halt test if load exceeds 25 or memory usage approaches 100GB

**HARD-STOP trigger**: If load average exceeds 25 or memory usage exceeds 100GB during test