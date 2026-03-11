# REPORT: Simultaneous Dial Race Condition (libp2p#752)

**Date**: 2026-03-11
**Severity**: Critical (network-wide outage potential, confirmed once)
**Resolution**: v3.3.5 — 3-layer fix deployed to all 30 nodes
**Status**: Resolved — root cause confirmed, fix verified in production

## Incident Summary

N8 was stuck at height 0 with 0 peers after a binary upgrade. Investigation revealed a latent race condition in libp2p's connection handling that had previously caused a full network outage requiring genesis reset. The bug is a time bomb: any pair of co-located nodes (same server) can trigger it, and the probability increases with each new node added.

## Historical Context

Before this fix, the network experienced a catastrophic failure where all nodes lost connectivity simultaneously, requiring a full genesis reset. The root cause was never identified. Months of engineering were spent building resilience features (snap sync, header-first sync, peer blacklisting, quorum validation, state recovery, block archiver, `recover_from_peers()`, RPC backfill) under the assumption that sync was the problem. The actual root cause was this race condition, discovered only when N8 (a new node) reproduced the exact failure pattern in isolation.

## Root Cause Chain

The symptom was "node stuck at height 0" but the root cause was 5 layers deep:

```
Layer 1: Height index corrupt (dirty shutdown during upgrade)
  └── Layer 2: recover writes flat files, state_db ignores them (migration skip)
       └── Layer 3: No peers → reconnection storm (PeerDisconnected without backoff)
            └── Layer 4: biased select! starves production timer (event flood)
                 └── Layer 5: SIMULTANEOUS DIAL RACE (the actual root cause)
```

### The Race (libp2p#752)

When two co-located nodes (same server, ~0ms latency) both dial each other simultaneously:

1. Node A dials Node B → TCP handshake completes → Noise handshake completes
2. Node B dials Node A → TCP handshake completes → Noise handshake completes
3. Both connections are now fully established
4. `max_established_per_peer=1` → the second connection is denied by the ConnectionLimits behaviour
5. Denied connection emits `ConnectionClosed` → `PeerDisconnected` event
6. `PeerDisconnected` triggers immediate reconnection → goto step 1

This cycle runs at ~1000 iterations/second on localhost. The flood of `PeerConnected`/`PeerDisconnected` events overwhelms the `tokio::select! { biased; }` event loop, which gives network events absolute priority over the production timer. The node becomes a zombie — connected to peers in its sync manager but unable to execute `run_periodic_tasks()`.

### Three Triggers

1. **`max_established_per_peer=1`** — rejects the second half of a simultaneous dial
2. **`RoutingUpdated` auto-dial** — Kademlia DHT update triggers immediate `swarm.dial(peer)`, which on localhost (latency ≈ 0) guarantees the race fires
3. **`PeerDisconnected` without backoff** — every disconnect immediately redials all bootstrap nodes, feeding the storm

## Fix (3 Parts)

### Part 1: Connection Limits (`crates/network/src/service.rs`)

```rust
// Before: with_max_established_per_peer(Some(1))
// After:
.with_max_established_per_peer(Some(2))
```

Allow 2 connections per peer so both sides of a simultaneous dial survive. Also required for DCUtR hole-punching (relay + direct connections coexist briefly during upgrade).

### Part 2: Remove Auto-Dial Trigger (`crates/network/src/service.rs`)

```rust
// Before: if !swarm.is_connected(&peer) { swarm.dial(peer); }
// After: no-op (periodic DHT bootstrap every 60s handles discovery)
```

`RoutingUpdated` auto-dial was the main trigger for the race on co-located nodes. Periodic DHT bootstrap and explicit bootstrap dials already handle peer discovery adequately.

### Part 3: Reconnection Backoff (`bins/node/src/node.rs`)

```rust
// Rate-limit PeerDisconnected reconnection to 1 attempt per slot (10s)
let recently_dialed = self.last_peer_redial
    .map(|t| t.elapsed().as_secs() < self.params.slot_duration)
    .unwrap_or(false);
if !recently_dialed { /* reconnect */ }
```

Even if Parts 1-2 fail for an unforeseen reason, the backoff prevents the event storm from forming.

## Commits

| Hash | Description |
|------|-------------|
| `ff5d06b` | fix(network): rate-limit PeerDisconnected reconnection to prevent spin loop |
| `ef13610` | fix(network): prevent simultaneous-dial race condition (libp2p#752) |

## Verification

1. N8 deployed with fix, wiped data, synced from height 0 to chain tip with 15 peers — no connect/disconnect loops
2. Rolling restart of all 30 nodes (mainnet + testnet + seeds) across 3 servers
3. All nodes at v3.3.5, synced, producing

## Impact Assessment

- **Blast radius**: Any pair of co-located nodes. With N nodes on one server, there are N*(N-1)/2 potential race pairs
- **Probability**: Increased with each new node. At 6 nodes per server, 15 potential pairs. At 12 nodes, 66 pairs
- **Time to trigger**: Non-deterministic, but on localhost (latency ≈ 0) the race is essentially guaranteed whenever two nodes discover each other via Kademlia
- **Recovery without fix**: Only possible by wiping data and restarting — the spin loop is self-sustaining

## Lessons Learned

1. **`max_established_per_peer=1` is unsafe for co-located nodes** — this is a known libp2p issue but not well-documented in their guides
2. **`biased` select in event loops is dangerous** — network events starving timers is a class of bugs, not just this instance
3. **The previous "full network failure" was this bug** — months of sync resilience engineering were driven by a 3-line networking bug
4. **Silver lining**: The misdiagnosis forced construction of snap sync, header-first sync, peer blacklisting, quorum validation, state recovery, block archiver, `recover_from_peers()`, and RPC backfill — all necessary at scale regardless

## References

- [rust-libp2p#752](https://github.com/libp2p/rust-libp2p/issues/752) — simultaneous dial race condition
- `REPORT_HA_FAILURE.md` — the previous network outage (likely this same root cause, compounded by rolling deployment)
