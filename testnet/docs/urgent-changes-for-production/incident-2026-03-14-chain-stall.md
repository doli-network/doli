# Incident Report: Chain Stall — Registration TX Flood + Dead Peer Poisoning

**Date:** 2026-03-14
**Severity:** Critical (complete chain halt for ~10 minutes, all 5 producers affected)
**Duration:** ~10 minutes (manual restart required to recover)

## Summary

50 producer registration transactions submitted in rapid succession caused all 5 genesis producers to stall simultaneously. Every block production attempt failed with "Slot boundary crossed" — the block building phase exceeded the 10-second slot window. The chain halted at height 832 for 60+ slots.

## Root Cause Analysis

**Two compounding failures:**

### 1. Block Building Timeout (PRIMARY)

`try_produce_block()` in `production.rs:1024-1048` validates every mempool TX against the UTXO set before inclusion. With 57 TXs in mempool (50 registrations + 7 funding TXs), the synchronous validation loop in `validate_transaction_with_utxos()` takes **>5 seconds** per block attempt.

Combined with late slot entry (production offset up to 8959ms into the 10s slot), the remaining time budget is often <2 seconds. Block building consistently overruns the slot boundary:

```
01:39:15.959 Producing block for slot 840 at height 833 (offset 8959ms)  ← 1.04s left
01:39:21.030 Slot boundary crossed (slot 840 → 841) — aborting           ← took 5.07s
```

This repeated for **every slot from 839 to 893** — 54 consecutive failed production attempts across all 5 producers. Height never advanced past 832.

**Why all 5 producers fail simultaneously:** All producers share the same mempool via gossip. All attempt to validate the same 57 TXs. On a single host, they compete for the same CPU cores, making the problem worse.

### 2. Dead Peer Reconnection Storm (SECONDARY)

After stopping the 50 stress-test nodes, the DHT/peer store retained their addresses. Producers spent CPU cycles attempting reconnections to 50 dead peers (ports 30313-30362), ~200+ `Connection refused` errors per minute. This consumed event loop time that should have gone to block production.

```
01:47:52 Failed to connect to peer /ip4/10.0.0.88/tcp/30328 — Connection refused
01:47:52 Failed to connect to peer /ip4/10.0.0.88/tcp/30330 — Connection refused
... (20+ per second)
```

## Timeline

| Time | Event |
|------|-------|
| 01:37:xx | 50 registration TXs submitted via CLI (1/sec from 4 wallets in parallel) |
| 01:38:07 | Last successful block produced (height 832, slot 834) |
| 01:38:58 | First failed block attempt (height 833, slot 839, offset 1240ms) |
| 01:39:15 | Pattern established: every production attempt crosses slot boundary |
| 01:39:xx–01:47:xx | **Chain halted.** 54 consecutive slot boundary crosses. Height stuck at 832. |
| 01:47:52 | Dead peer reconnection storm begins compounding the stall |
| 01:48:24 | Manual restart of N1-N5 (clears peer cache + restarts event loop) |
| 01:48:40 | Chain resumes producing at height 833 |

## Impact

- **Chain halted for ~10 minutes** (60+ empty slots)
- **57 TXs stuck in mempool** — not lost, but delayed until restart
- **All producers affected simultaneously** — no fallback producer could take over
- **Would affect production mainnet identically** — this is not a local-only issue

## Urgent Production Changes Required

### P0: Block Building Must Be Time-Budgeted

**File:** `bins/node/src/node/production.rs:1024-1048`

The mempool TX validation loop has no time budget. It processes ALL selected TXs regardless of how long it takes.

**Fix:** Add a deadline check inside the TX validation loop. If elapsed time exceeds `slot_duration * 0.6` (6 seconds for 10s slots), stop adding TXs and finalize the block with whatever is already included.

```rust
let deadline = Instant::now() + Duration::from_millis(slot_duration_ms * 6 / 10);
for tx in &mempool_txs {
    if Instant::now() > deadline {
        warn!("Block building deadline reached, finalizing with {} of {} TXs",
              builder.tx_count(), mempool_txs.len());
        break;
    }
    // validate and add...
}
```

**Severity:** CRITICAL — without this, any mempool flood (legitimate or attack) halts the chain.

### P0: Peer Reconnection Backoff for Dead Peers

**File:** `crates/network/src/service.rs`

After `Connection refused`, the peer should be backed off exponentially (1s → 2s → 4s → ... → 5min max). Currently the node retries immediately on the next tick, flooding the event loop with failed TCP handshakes.

**Severity:** HIGH — dead peers waste CPU and event loop time that block production needs.

### P1: Registration TX Validation Caching

**File:** `crates/core/src/validation.rs`

Registration TX validation includes VDF proof verification. When the same TX is validated multiple times (once per block attempt), the VDF result should be cached.

**Severity:** MEDIUM — reduces block building time for mempool with registration TXs.

### P1: Production Offset Optimization

**File:** `bins/node/src/node/production.rs`

Production starts at arbitrary offsets (0-9s into the slot). When offset is 8959ms, only 1.04s remains for block building. Production should be scheduled to start at the beginning of the slot, not at an arbitrary point.

**Severity:** MEDIUM — ensures maximum time budget for block building.

### P2: Mempool TX Priority Queue

**File:** `crates/mempool/src/`

`select_for_block()` should prioritize simple Transfer TXs over Registration TXs (which are more expensive to validate). This ensures the block always includes some transactions even when registrations are slow.

**Severity:** LOW — optimization, not a correctness fix.

## Reproduction

```bash
# Start 5 producers (local mainnet)
scripts/mainnet.sh start seed n1 n2 n3 n4 n5

# Wait for chain to reach height 800+

# Submit 50 registration TXs in rapid succession
for i in $(seq 13 62); do
  doli -r http://127.0.0.1:8502 -w ~/mainnet/keys/producer_${i}.json producer register -b 1 &
done

# Observe: chain halts, all blocks fail with "Slot boundary crossed"
# Height stops advancing, mempool grows
```

## Lessons

1. **Block building is a hard real-time operation** — it must complete within the slot window or the chain stalls. Any synchronous work in the block building path is a DoS vector.
2. **Mempool floods are a natural event** — epoch boundaries, exchange withdrawals, and registration waves all create TX bursts. The block builder must gracefully degrade.
3. **Dead peers are toxic** — a node that remembers 50 dead peers and retries them all every second wastes more CPU than producing blocks. Peer stores need TTL and exponential backoff.
4. **Single-host testing exposes real issues** — CPU contention between 5 producers + 50 stress nodes on 16 cores revealed timing bugs that would also occur on production hardware under load.
