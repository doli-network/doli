# Incident Report: Chain Halt When Top-Bonded Producers Die

**Date:** 2026-03-14 21:44 PDT
**Severity:** Critical (complete chain halt, no self-recovery)
**Duration:** Until manual restart of N1-N5

## Summary

Killing 5 genesis producers (N1-N5) halted the chain permanently despite 100 active producers remaining online. The scheduler assigns slots proportional to bond weight. N1-N5 held 91.7% of bonds (1,109/1,209). No surviving producer could fill the dead slots.

## The Math

```
N1-N5:   1,109 bonds (91.7%) — DEAD
N13-N112:  100 bonds  (8.3%) — ALIVE

Slots per epoch: 360
Primary rank assigned to dead producers: 348/360 (97%)
Any fallback rank reaching a live producer: 112/360 (31%)
Actual blocks produced after kill: 0
```

## Why Zero Blocks

Three compounding failures:

1. **Scheduler is bond-weighted with no liveness awareness.** `DeterministicScheduler` in `scheduler.rs` assigns `slot % total_bonds → ticket → producer`. Dead producers keep their tickets. Live producers with 1 bond only get 1 ticket each out of 1,209.

2. **Fallback ranks also mostly hit dead producers.** `MAX_FALLBACK_RANKS=5` with offsets `(total_bonds * rank) / 5`. When 91% of tickets are dead, even spread-out fallback offsets land in the dead zone.

3. **Post-recovery grace blocks remaining producers.** 13 of 50 stress nodes were in `BlockedResync` with 228s grace remaining from earlier snap syncs. Even if scheduled, they wouldn't produce.

## Required Changes

### `production.rs:746-801` — Apply Existing Liveness Filter to Epoch Scheduler

**The fix already exists.** `bootstrap_schedule_with_liveness()` in `validation.rs:3039` splits producers into `live_producers` and `stale_producers` and schedules only from live ones with periodic re-entry for stale. This is used in bootstrap mode (line 363) but **not** in the epoch scheduler path (line 746).

The epoch scheduler at line 759 builds `DeterministicScheduler` from `active_with_weights` with no liveness check. Dead producers keep 100% of their bond tickets.

**Fix:** Before building `DeterministicScheduler` at line 776, filter `active_with_weights` through the same liveness window used in bootstrap mode (line 650-708). Remove producers that haven't produced a block in the last `liveness_window` slots. Redistribute their tickets to live producers.

This is ~20 lines of code. The liveness tracking (`producer_liveness` map) already exists from the bootstrap path.

### `consensus.rs` — Emergency Round-Robin When Chain Stalls

If no block is produced for `3 × slot_duration` (30 seconds), the epoch scheduler should deactivate and fall back to `bootstrap_schedule_with_liveness()` with the full producer set. This already handles live/stale separation and re-entry. No new code needed — just a conditional branch in `production.rs` at line 363 that checks chain staleness.

```
// Line 363: add chain stall detection
let chain_stalled = height == prev_height && slots_since_last_block > 3;
if in_genesis || active_with_weights.is_empty() || chain_stalled {
    // Use bootstrap liveness-aware scheduling
```

### `sync/manager.rs` — Cancel Resync Grace on Chain Stall

`BlockedResync` with 228s grace blocks producers from filling empty slots during emergencies. When no new blocks arrive for >30s, cancel the grace period.

Add to the resync grace check: `if blocks_since_grace_start == 0 && grace_elapsed > 30s → cancel grace`.

## Reproduction

```bash
# With 105 active producers (5 with 200+ bonds, 100 with 1 bond):
scripts/mainnet.sh stop n1 n2 n3 n4 n5
# Chain halts immediately. No recovery without restarting N1-N5.
```

## Impact on Production

This is the **"nothing at stake" inverse**: a **"everything at stake" single point of failure**. If the top 5 bonded producers on production mainnet go offline simultaneously (server outage, coordinated attack, bad deploy), the chain halts permanently until they return. 100 other producers can do nothing.
