# REPORT: HA Architecture Failure During Consensus-Critical Deployment

**Date**: 2026-03-10
**Severity**: Critical (full chain loss)
**Resolution**: Genesis reset v21 (timestamp 1773112630)
**Status**: Resolved — root cause documented, procedures updated

## Incident Summary

A consensus-critical fix (`count_bonds()`) was deployed using a rolling strategy (ai1 first, then ai2). The fix changed scheduling weights, causing ai1 and ai2 to compute different producer schedules. Both sides produced incompatible blocks simultaneously, creating an irreconcilable fork. The entire chain history was lost and a genesis reset was required.

## Timeline

1. `count_bonds()` fix deployed — changes from counting Bond UTXOs to counting bond units
2. ai1 restarted with new binary → `total_bonds` = 198 (6 producers × 33 units each)
3. ai2 still on old binary → `total_bonds` = 12 (6 producers × 2 UTXOs each)
4. Different `slot % total_bonds` → completely different producer schedule per slot
5. Both sides reject each other's blocks as `InvalidProducer`
6. Fork grows past `MAX_REORG_DEPTH` (100 blocks) — reorg impossible
7. Full genesis reset required (new timestamp, wipe all data, restart)

## Root Cause

The `count_bonds()` function determines how many "tickets" each producer gets in the scheduler (`slot % total_tickets`). Changing this calculation is **consensus-critical** — it directly affects which producer is authorized for each slot.

**Old code**: counted Bond UTXOs (1 UTXO = 1 ticket regardless of amount)
**New code**: sums Bond UTXO amounts and divides by `bond_unit` (330 DOLI / 10 DOLI = 33 tickets)

A rolling deployment meant ai1 and ai2 ran different versions simultaneously. Since the scheduler is deterministic but with different inputs, every slot had a different "correct" producer on each side.

## Why HA Did Not Protect Us

### 1. HA mirrors run the same binary — they can't arbitrate consensus changes

The HA architecture (ai1 + ai2 with seeds on both) is designed for **infrastructure failure**: server crash, network partition, hardware fault. Seeds are just regular nodes with `--relay-server --rpc-bind 0.0.0.0` — they use the same binary, same consensus logic. When ai1's seed got the new binary, it joined ai1's fork. ai2's seed stayed on ai2's fork.

```
BEFORE: ai1 (schedule A) ←── consensus ──→ ai2 (schedule A)  ✅
AFTER:  ai1 (schedule B) ←── FORK ──→ ai2 (schedule A)       ❌
```

### 2. genesis_hash was identical — no clean partition

`genesis_hash = BLAKE3(timestamp || network_id || slot_duration || message)` only covers genesis parameters, not binary logic changes. Both sides had the same `genesis_hash`, so nodes stayed connected and gossiped incompatible blocks to each other. A genesis mismatch would have cleanly separated the networks.

### 3. Blocks silently rejected — no alarm, no recovery trigger

When a node receives a block from a producer who isn't scheduled for that slot, it logs a warning and drops the block (`return Ok(())`). Both sides silently discarded each other's blocks while continuing to produce their own. The fork grew quietly.

### 4. 90% guard prevented recovery during symmetric split

`recover_from_peers_inner()` skips state wipe if `local_height > best_peer * 90/100`. During a symmetric split, both sides are at ~100% of their observed peer tip. The guard says "wait for reorg" but reorg never comes because the other chain is incompatible.

### 5. Snap sync quorum picks a random side

When a node tries re-snap, peers from both forks report different state roots. The quorum picks whichever side has more connected peers — not the "canonical" chain (which doesn't exist anymore).

### 6. No "canonical" chain to recover to

Both forks diverged from the same parent. Neither is "wrong" given its own rules. The heaviest chain rule can't resolve it because both forks accumulate weight simultaneously. Once both exceed `MAX_REORG_DEPTH`, even stopping one side can't recover the other.

## HA Threat Model (What It Actually Protects Against)

| Threat | HA Protects? |
|--------|:---:|
| Server hardware failure | ✅ |
| Network partition (ISP) | ✅ |
| DDoS on one server | ✅ |
| **Consensus-critical rolling deploy** | ❌ |
| **Binary logic bug in scheduling** | ❌ |

## Architectural Gaps Identified

1. **No protocol version in blocks** — nodes can't detect that a peer's block was produced with incompatible scheduling logic. They just see "wrong producer" and silently discard.

2. **No hard fork height mechanism** — new consensus rules can't be pre-scheduled to activate at a specific block height. All nodes must switch simultaneously via operational coordination.

3. **genesis_hash doesn't cover logic changes** — only genesis parameters (timestamp, network_id, slot_duration, message) are hashed. A logic change with identical genesis params produces the same hash.

4. **ProtocolActivation (TxType 15) exists but is unused** — could be extended to handle scheduling rule changes via 3/5 maintainer vote to activate at a specific height.

5. **No binary coordination mechanism** — no way for the network to verify all nodes are on the same binary version before producing.

## Prevention: Consensus-Critical Deployment Procedure

See `docs/infrastructure.md` Section "Consensus-Critical Deployment" for the mandatory procedure. Key rule: **simultaneous deployment only** — stop ALL nodes, deploy ALL, start ALL.

## Key Files

| File | Role |
|------|------|
| `crates/core/src/scheduler.rs` | Ticket-based scheduler (`slot % total_tickets`) |
| `crates/storage/src/utxo.rs:334` | `count_bonds()` — the function whose change caused divergence |
| `crates/core/src/validation.rs:564` | genesis_hash check (rule #0) |
| `crates/core/src/validation.rs:2607` | `validate_producer_eligibility` — rejects "wrong producer" blocks |
| `bins/node/src/node.rs:3024` | 90% guard in `recover_from_peers_inner()` |
| `crates/network/src/service.rs:1078` | P2P genesis hash mismatch = instant disconnect |

## Lessons

1. **Any value used in `slot % total_tickets` is consensus-critical.** This includes `count_bonds()`, `bond_unit`, producer list, sort order. Changes to ANY of these require simultaneous deployment.

2. **HA does not protect against protocol-level splits.** Mirrors run the same binary — they mirror the split, not prevent it.

3. **Silent block rejection is dangerous.** Blocks from the "wrong" producer are dropped without triggering recovery. The fork grows undetected until it's too late.

4. **The chain has no "memory" of which fork is canonical.** Once a consensus-critical split occurs, both sides are equally valid. The only resolution is human coordination (stop all, pick one version, wipe, restart).
