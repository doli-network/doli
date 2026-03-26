# Instrumentation Plan: RAM Scaling Visibility (>103 nodes)

## Problem
RAM crashes when deploying >103 nodes. Zero memory visibility exists today — no RSS tracking, no connection counting, no buffer utilization metrics.

## Current State
- **HEALTH log** (periodic.rs:520-535): Reports chain height, peers, sync state. No memory info.
- **No process memory tracking** anywhere in codebase.
- **No connection count tracking** (only peer count via HashMap).
- **No buffer utilization metrics** (Yamux, channels, sync pipeline).

## Dominant Memory Costs (by analysis)

| Component | Formula | Per-Node at 103 peers |
|-----------|---------|----------------------|
| Yamux receive window | peers × 2 conns × 512KB | ~105MB |
| Event channel | max(4096, max_peers×16) × ~1.5KB | ~6MB |
| RocksDB (state_db) | Fixed per node | ~50-200MB |
| Block store | grows with chain height | variable |
| UTXO set (in-memory) | grows with chain size | variable |
| ProducerSet (in-memory) | ~1KB per producer | ~100KB |
| Sync pipeline (during sync) | headers + bodies | ~10-50MB burst |
| **Total per node** | | **~170-360MB** |
| **Total 103 nodes** | | **~17-37GB** |

## Instrumentation Points (7 log statements)

### L1: Process RSS (every 30s) — `periodic.rs`
**Where**: Enhance existing HEALTH log at line 520
**What**: Add RSS in MB to the one-liner
**Why**: The #1 missing metric — shows actual memory consumption per process
**Level**: `warn!` (same as existing HEALTH)
**Implementation**: `libc::getrusage(RUSAGE_SELF)` → `ru_maxrss` (KB on Linux, bytes on macOS)

### L2: Connection count at startup — `service/mod.rs`
**Where**: After swarm creation, line ~216
**What**: Log max_peers, conn_limit, yamux_window, channel sizes
**Why**: Confirms the configured memory budget at node boot
**Level**: `info!`

### L3: Connection count change — `service/swarm_events.rs`
**Where**: After ConnectionEstablished and ConnectionClosed handlers
**What**: Log current established connection count (in + out)
**Why**: Tracks actual connection growth over time. The cliff is connection count × Yamux window.
**Level**: `debug!` (high frequency) with `info!` at milestones (every 10th connection)

### L4: Sync pipeline state (every 30s) — `periodic.rs`
**Where**: Same 30s tick as HEALTH, after HEALTH line
**What**: pending_headers count, pending_blocks count, downloaded bodies count, active sync requests
**Why**: Sync pipeline holds blocks in memory — can be significant during mass sync
**Level**: `debug!`

### L5: UTXO set size (every 60s) — `periodic.rs`
**Where**: New 60s tick in periodic tasks
**What**: utxo_set.len(), producer_set active count
**Why**: These grow with chain length and are fully in-memory
**Level**: `info!` (low frequency)

### L6: Peer table size at eviction — `service/swarm_events.rs`
**Where**: In the eviction handler (existing log)
**What**: Add current peer count + eviction_cooldown size + bootstrap_peers size
**Why**: Eviction storms are the #1 RAM amplifier — seeing the tracking table sizes reveals thrashing
**Level**: Already `info!`

### L7: Startup memory baseline — `startup.rs`
**Where**: After `Node::new()` completes, before entering event loop
**What**: RSS at startup, UTXO count, producer count, block store height
**Why**: Baseline to compare against runtime growth
**Level**: `info!`

## Implementation Details

### RSS Helper Function
Add to `bins/node/src/node/mod.rs`:
```rust
/// Get process RSS in MB (cross-platform via libc::getrusage)
fn process_rss_mb() -> f64 {
    unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &mut usage) == 0 {
            #[cfg(target_os = "macos")]
            { usage.ru_maxrss as f64 / 1_048_576.0 } // bytes on macOS
            #[cfg(not(target_os = "macos"))]
            { usage.ru_maxrss as f64 / 1024.0 } // KB on Linux
        } else {
            0.0
        }
    }
}
```

### Dependencies
- Add `libc = "0.2"` to `bins/node/Cargo.toml` (already a transitive dep, just needs direct)

### Files Modified
1. `bins/node/Cargo.toml` — add libc dependency
2. `bins/node/src/node/mod.rs` — add `process_rss_mb()` helper
3. `bins/node/src/node/periodic.rs` — L1 (enhanced HEALTH), L4 (sync pipeline), L5 (UTXO size)
4. `bins/node/src/node/startup.rs` — L7 (startup baseline)
5. `crates/network/src/service/mod.rs` — L2 (startup config)
6. `crates/network/src/service/swarm_events.rs` — L3 (connection lifecycle), L6 (eviction detail)

### Log Format Convention
All new logs use `[MEM]` prefix for easy grepping:
```
[MEM] rss=245MB  (in HEALTH line)
[MEM-BOOT] rss=42MB utxos=1523 producers=12 height=450 max_peers=150 conn_limit=310 yamux_window=512KB
[MEM-CONN] established=47/310 (in=23 out=24)
[MEM-SYNC] pending_headers=0 pending_blocks=0 downloaded=0 active_requests=0
[MEM-STATE] utxos=1523 producers=12
```

## Expected Output
After instrumentation, a 30s HEALTH log will look like:
```
[HEALTH] h=450 s=451 hash=a1b2c3d4 | peers=102 ... | rss=245MB
```

And a full memory dump at startup:
```
[MEM-BOOT] rss=42MB utxos=0 producers=12 height=0 max_peers=150 conn_limit=310 yamux_window=512KB
```
