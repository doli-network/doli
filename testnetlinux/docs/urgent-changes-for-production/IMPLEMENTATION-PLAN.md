# Implementation Plan: Unlock DOLI for 100K+ Producers

**Author:** Ivan D. Lozada
**Date:** 2026-03-14
**Context:** Stress testing with 105 active producers on a single Mac Pro exposed 6 critical issues that would prevent the network from scaling beyond ~50 producers. This document specifies the exact code changes needed. Each change is independent and can be implemented in any order.

---

## 1. Block Building Deadline

**Problem:** `try_produce_block()` validates all mempool TXs synchronously with no time limit. 50+ TXs causes block building to exceed the 10s slot → every producer misses the slot → chain halts.

**File:** `bins/node/src/node/production.rs:1024-1048`

**Change:** Add a deadline check inside the mempool TX validation loop.

```
Before entering the for loop at line 1037:
  let deadline = Instant::now() + Duration::from_millis(slot_duration_ms * 6 / 10);

Inside the loop, before each validate_transaction_with_utxos call:
  if Instant::now() > deadline {
      warn!("Block building deadline: including {}/{} TXs", count, total);
      break;
  }
```

`slot_duration_ms` is available from `self.params.slot_duration * 1000`. The 60% budget leaves 4s for VDF, signing, and gossip broadcast.

**Test:** Submit 100 registration TXs in 10 seconds. Chain must not stall. Blocks should include partial TX sets.

---

## 2. Dead Peer Exponential Backoff

**Problem:** After `Connection refused`, the node retries on the next tick (~1s). 50 dead peers = 200+ failed TCP handshakes/sec, starving the event loop. Block production takes >10s because the event loop is saturated.

**File:** `crates/network/src/service.rs` — `handle_swarm_event`, `ConnectionClosed` branch

**Change:** Track failed connection attempts per peer. On each failure, double the backoff (1s → 2s → 4s → ... → 5min max). Reset on successful connection.

Add to `NetworkService` or the peer tracking:
```rust
failed_attempts: HashMap<PeerId, (u32, Instant)>  // (count, last_attempt)
```

In the connection retry logic (periodic redial):
```
let (count, last) = failed_attempts.get(peer).unwrap_or((0, Instant::now()));
let backoff = Duration::from_secs(min(300, 1 << count));  // 1s, 2s, 4s... 5min max
if last.elapsed() < backoff { skip; }
```

**Test:** Start 50 nodes, stop them, verify producers continue producing at full throughput (no slot boundary crosses).

---

## 3. Liveness-Aware Epoch Scheduler

**Problem:** The epoch scheduler (`production.rs:746-801`) assigns slots proportional to bond weight with zero liveness awareness. When top-bonded producers die (91% of bonds), 97% of slots are assigned to dead nodes. The existing `bootstrap_schedule_with_liveness()` in `validation.rs:3039` already handles this correctly but is only used during genesis/bootstrap, not during normal epoch scheduling.

**File:** `bins/node/src/node/production.rs:746-801`

**Change:** Before building `DeterministicScheduler` at line 776, filter `active_with_weights` through the same liveness data used in bootstrap mode.

```
// At line 770, before building the scheduler:
let active_with_weights: Vec<(PublicKey, u64)> = active_with_weights
    .into_iter()
    .filter(|(pk, _)| {
        match self.producer_liveness.get(pk) {
            Some(&last_h) => last_h >= height.saturating_sub(liveness_window),
            None => true,  // New producers get benefit of doubt
        }
    })
    .collect();

// If filtering removed everyone, restore the full list (deadlock safety)
if active_with_weights.is_empty() {
    active_with_weights = original_active_with_weights;
}
```

`liveness_window` = `max(LIVENESS_WINDOW_MIN, num_producers * 3)` — same formula used at line 695.

The `producer_liveness` HashMap already exists on the Node struct, populated by `apply_block()`.

**Test:** Start 105 producers (5 with 200+ bonds, 100 with 1 bond). Kill the 5 heavy producers. Chain must continue producing at >50% throughput within 30 seconds using the remaining 100 producers.

---

## 3b. Inactivity Leak — Whale Bond Weight Decay

**Problem:** Even with liveness filtering (#3), the deterministic scheduler is rebuilt at epoch boundaries using the full bond-weighted producer set. A dead whale with 300 bonds holds 300 slot tickets for the entire epoch. The liveness filter skips them at runtime, but the fallback rank offsets are still calculated from the full ticket space — meaning live producers with 1 bond only get fallback access to ~8% of slots.

The fundamental issue: **bond weight must decay when a producer is offline.** Without this, a whale that goes permanently offline captures slot capacity forever.

**File:** `crates/core/src/consensus.rs` — new constants

```rust
/// Inactivity leak: after INACTIVITY_LEAK_START missed consecutive slots,
/// a producer's effective bond weight decays by INACTIVITY_LEAK_RATE per epoch.
/// This matches Ethereum's inactivity leak: offline validators lose stake
/// quadratically until online validators control >2/3.
pub const INACTIVITY_LEAK_START: u64 = 360;       // 1 epoch of missed slots
pub const INACTIVITY_LEAK_RATE: u64 = 10;         // 10% per epoch
pub const INACTIVITY_LEAK_FLOOR: u64 = 1;         // Minimum 1 bond (never fully zeroed)
```

**File:** `bins/node/src/node/production.rs:306-318` — where `active_with_weights` is built

**Change:** After computing bond counts from UTXO set, apply inactivity decay:

```rust
let active_with_weights: Vec<(PublicKey, u64)> = active_producers
    .into_iter()
    .map(|(pk, raw_bonds)| {
        let effective = match self.producer_liveness.get(&pk) {
            Some(&last_h) => {
                let missed = height.saturating_sub(last_h);
                if missed > INACTIVITY_LEAK_START {
                    let epochs_inactive = (missed - INACTIVITY_LEAK_START) / slots_per_epoch;
                    let decay = (INACTIVITY_LEAK_RATE * epochs_inactive).min(99);
                    let effective = raw_bonds * (100 - decay) / 100;
                    effective.max(INACTIVITY_LEAK_FLOOR)
                } else {
                    raw_bonds
                }
            }
            None => raw_bonds,  // New producer, no liveness data yet
        };
        (pk, effective)
    })
    .collect();
```

This is consensus-critical — all nodes compute the same decay because they all see the same chain height and the same `last_produced_height` from block headers. No additional state needed.

**Effect on whale death scenario:**
- Epoch 0: Whale has 300 bonds → 300 tickets
- Epoch 1 (whale offline): 300 × 90% = 270 tickets
- Epoch 2: 300 × 80% = 240 tickets
- Epoch 5: 300 × 50% = 150 tickets
- Epoch 9: 300 × 10% = 30 tickets (same as a 30-bond producer)
- Epoch 10+: Floor at 1 ticket

Live producers with 1 bond go from 0.08% of slots to majority within 5 epochs automatically. No manual intervention, no governance vote, no fork.

**File:** `bins/node/src/node/production.rs:363-375` — emergency round-robin (already implemented)

**Change:** The `chain_stalled` detection (>3 empty slots) already falls back to bootstrap liveness scheduling. Add the inactivity leak to this path too — when chain_stalled, apply aggressive instant decay (treat any producer that missed the last 10 slots as weight=1):

```rust
if chain_stalled {
    // Emergency: instant weight equalization
    active_with_weights = active_with_weights.iter()
        .map(|(pk, _)| {
            let live = self.producer_liveness.get(pk)
                .map(|&h| height.saturating_sub(h) < 10)
                .unwrap_or(false);
            (*pk, if live { 1 } else { 0 })
        })
        .filter(|(_, w)| *w > 0)
        .collect();
}
```

This ensures that during a chain stall, ONLY producers that produced in the last 10 blocks get tickets. Dead whales get 0. If nobody produced recently, the deadlock safety (already implemented) treats everyone as weight=1.

**Test:** Start 5 producers with 300 bonds + 100 with 1 bond. Kill the 5 whales. Verify:
- Epoch 1: chain continues at reduced throughput (100 producers sharing 100/1309 = 7.6% of slots via fallback)
- Epoch 3: throughput increases as whale weight decays
- Epoch 10: 100 live producers have majority of slots, full throughput restored

---

## 4. Producer Peer Reservation in GossipSub

**Problem:** GossipSub selects mesh peers randomly from all connected peers. With 50 peer slots and 40 non-producers, there's a 14.3% chance per heartbeat that a producer has zero other producers in its mesh. This causes solo blocks → forks → cascading reorgs.

**File:** `crates/network/src/gossip/config.rs` — gossipsub configuration

**Change:** Use GossipSub's peer scoring to prioritize producers in the mesh.

In the gossipsub `ConfigBuilder`:
```rust
// Peers that deliver first-seen blocks on BLOCKS_TOPIC score higher
let topic_params = TopicScoreParams {
    topic_weight: 1.0,
    first_message_deliveries_weight: 10.0,
    first_message_deliveries_cap: 100.0,
    ..Default::default()
};
params.topics.insert(BLOCKS_TOPIC.into(), topic_params);
```

Producers naturally deliver first-seen blocks (they create them). Non-producers only relay. This makes GossipSub preferentially keep producers in the mesh without any explicit "is_producer" check.

**Also in** `crates/network/src/service.rs:764-773`:

When registering a new peer at max_peers, the existing `find_evictable_peer()` (already implemented) evicts zombies. Extend it: peers with low gossipsub score (non-producers with no first-message deliveries) should also be evictable.

**Test:** Start 5 producers + 50 non-producing nodes. Zero forks after 1 hour. Verify via explorer that all 5 producers maintain 100% attestation.

---

## 5. Auto-Backfill After Snap Sync

**Problem:** Snap sync restores state (UTXO set, producer set, chain state) but not the block store. Nodes show integrity gaps of -1200 to -1980 blocks. Currently requires manual `backfillFromPeer` RPC call.

**File:** `bins/node/src/node/fork_recovery.rs` — after snap sync completion

**Change:** After successful snap sync, automatically trigger backfill from the snap sync peer.

At the end of the snap sync apply function (where it logs `[SNAP_SYNC] Snapshot applied successfully`):
```rust
// Auto-backfill block store gaps from the snap sync peer
if let Some(peer_rpc) = snap_sync_peer_rpc {
    info!("[SNAP_SYNC] Starting auto-backfill from {}", peer_rpc);
    self.start_backfill(peer_rpc).await;
}
```

The `backfillFromPeer` RPC handler already contains the backfill logic — extract it into a reusable method on Node.

**Test:** Wipe a node's data, restart it. After snap sync completes, verify block store integrity is 100% without manual intervention.

---

## 6. Startup Warning for --no-dht with Many Producers

**Problem:** `--no-dht` disables Kademlia peer discovery. Nodes can only connect to their bootstrap peer. With >5 nodes this causes: snap sync deadlock (needs 3 peers, has 1), gossip mesh starvation (needs 6 peers, has 1), and permanent isolation if bootstrap dies. This was the root cause of 20% sync failures in stress testing.

**File:** `bins/node/src/node/startup.rs` — after loading network config

**Change:** Warn (and optionally refuse) when `--no-dht` is used with a network that has many producers.

```rust
if config.disable_dht {
    let active = producer_set.read().await.active_count();
    if active > 5 {
        warn!(
            "--no-dht is set but {} active producers detected. \
             Peer discovery will be limited to bootstrap nodes only. \
             Snap sync requires 3+ peers. Gossip mesh requires 6+ peers. \
             Consider enabling DHT for networks with >5 nodes.",
            active
        );
    }
}
```

**No test needed** — this is a warning, not a behavioral change.

---

## Priority Order

Implement in this order for maximum impact:

1. **Block Building Deadline** (#1) — prevents chain halt from mempool floods. Simplest fix, highest impact.
2. **Liveness-Aware Epoch Scheduler** (#3) — prevents chain halt when top producers die. Uses existing code.
3. **Inactivity Leak** (#3b) — decays dead whale bond weight so live producers recover majority. The difference between "chain limps at 3%" and "chain fully recovers in 5 epochs."
4. **Producer Peer Reservation** (#4) — prevents gossip mesh dilution → forks. Uses existing GossipSub scoring.
5. **Dead Peer Backoff** (#2) — prevents event loop starvation from dead peers.
6. **Auto-Backfill** (#5) — quality of life, prevents manual intervention after snap sync.
7. **DHT Warning** (#6) — guardrail for operators.

## Verification

After all 7 changes, this test must pass:

```bash
# Start 105 producers (5 with 300 bonds, 100 with 1 bond) + 50 non-producing relay nodes
# Wait for all to sync and produce for 2 epochs (stable state)
# Kill the 5 whale producers (91% of bond weight)
# Submit 50 registration TXs simultaneously

# Chain must:
#   - NOT halt (zero slots with all-dead-producer assignment)
#   - Resume producing within 30s via emergency round-robin
#   - Epoch 1: >10% throughput (100 light producers filling fallback slots)
#   - Epoch 3: >30% throughput (inactivity leak reducing whale tickets)
#   - Epoch 10: >90% throughput (whale weight decayed to floor, live producers dominant)
#   - Include registration TXs within 2 epochs (block building deadline prevents stall)
#   - Have zero forks lasting >3 blocks (gossip mesh not diluted)
#   - All 100 light producers attesting at >90%
#   - Block store integrity 100% on all nodes (auto-backfill after snap sync)
#   - No manual intervention required at any point
```
