# Two Pillars of Network Stability: Orphan Chase & Full Bitfield Decode

## Summary

Two hidden bugs destabilized the DOLI network for months. Both were silent — no errors, no crashes, no log entries. The network appeared healthy: all nodes synced, same chain, same height. Then without warning, a node would fall 1 block behind. Still synced. Still on the same chain. Nobody noticed.

At the next epoch boundary, that 1-block gap became a fork. The node computed a different `epoch_state` because it missed one attestation bitfield. Different `attested_sets` → different `producer_list` → different scheduler → different producer for the same slot → fork. Now the node was 10 blocks behind. Then 50. Then the fork detector triggered a rollback cascade. Then auto-snap kicked in. The node wiped its state and re-synced — but the same silent bug was still there, waiting for the next gossip hiccup.

Nobody knew what caused it because the trigger (a dropped orphan block, a misaligned index) left no trace. The consequences (fork, rollback, snap) appeared minutes or hours later, far from the cause. We built defenses around every symptom — silence pull, fork detection, rollback guards, mesh scoring fixes, direct attestation delivery — but the underlying bugs persisted. The symptoms kept returning in different forms.

When the two root causes were finally identified and fixed, all the symptoms disappeared simultaneously. The defenses became redundant — but remain as safety nets.

## Pillar 1: Orphan Chase (v6.16.1, 2026-04-16)

### The hidden problem

When a gossip block arrived whose parent was unknown (orphan), the node dropped it silently. No error. No log. The node just... didn't have that block. It waited for gossip to deliver the parent — but gossip has no obligation to deliver blocks in order. If the parent arrived late or not at all, the node fell 1 block behind.

1 block behind looks fine. The node is synced. Same chain. The explorer shows green. But that 1 missing block means 1 missing attestation bitfield. At the epoch boundary, the node's `attested_sets` differs from the rest of the network. Different `attested_sets` → different `producer_list` → the node thinks producer A should build the next block, the network thinks producer B should. Fork.

The fork triggers `stuck_fork_signal`. The node rolls back. But the rollback doesn't fix the missing block — it just makes the gap bigger. Now it's 10 behind. The sync manager tries to catch up, but new orphans keep arriving (because the node is behind, every block looks like an orphan). Cascade. Eventually auto-snap fires, wipes the state, and re-syncs from a peer. The node looks healthy again. Until the next gossip hiccup.

This cycle repeated for months. We attributed it to mesh quality, peer count, server co-location, gossip scoring. Each explanation was partially true. None was the root cause.

### What we built around it (symptoms)

- Silence pull: proactive block request when gossip is silent for 30s
- ACTIVE_FORK_DETECT: rollback when peers are ahead
- stuck_fork_signal with 60s guard
- Direct attestation delivery to next-slot producer
- Mesh scoring fixes (mesh_message_deliveries penalty disabled)
- Staggered restarts on shared servers
- Multiple rsync/wipe recovery procedures

These mitigated the impact but never fixed the cause. Nodes still fell behind intermittently, triggered rollbacks, and sometimes cascaded into forks.

### The fix (14 lines)

When an orphan block arrives, immediately request `GetBlockByHeight(local_height+1)` from the sender peer. Cache the orphan in `fork_block_cache`. After the parent is applied, drain the cache for blocks whose `prev_hash` matches the new tip and apply them immediately.

Causal, deterministic, point-to-point. No heuristics, no timers, no thresholds.

**Files**: `bins/node/src/node/block_handling.rs`

---

## Pillar 2: Full Bitfield Decode (v6.17.1, activation h=14000, 2026-04-18)

### The hidden problem

The attestation bitfield encoder (`assembly.rs`) used the order `[epoch_state.producer_list | extra sorted]` where extra = producers activated mid-epoch but filtered from the epoch list by the 3-epoch attestation lookback. The decoder in three places used different lists:

1. **`accumulate_block` (post_commit.rs)**: decoded with `epoch_state.producer_list.len()` only — indices for extra producers (N+) silently returned `None` and were ignored. Extra producers' attestations were encoded but never counted.

2. **`calculate_epoch_rewards` (rewards.rs)**: decoded with `active_producers_at_height` (all active, sorted globally) — a completely different order from the encoder. Index N in one list mapped to a different producer in the other. This caused rewards to be misattributed.

3. **`getAttestationStats` (schedule.rs)**: same wrong list as rewards — the explorer showed incorrect attestation percentages.

### The death spiral

Once a producer was filtered by the attestation lookback (didn't attest in the last 3 epochs), they entered a death spiral:

1. Filtered from `epoch_state.producer_list` → moved to "extra" (indices N+)
2. `accumulate_block` ignores indices N+ → attestations never counted
3. `attested_sets` never updated → lookback filter keeps them out
4. Next epoch: still filtered → repeat forever

The producer attests. The block producer includes them in the bitfield. The encoder writes the bit. But the decoder reads `producer_list.get(idx)` which returns `None` for extra indices — silently. No error. No warning. The attestation data exists on-chain in every block, but the consensus ignores it.

From the explorer, it looked like these producers had "no data" — as if they weren't attesting. But they were. Every single block. The data was there, encoded correctly. The decoder just didn't read it.

Meanwhile, the rewards code used a completely different list (`active_producers_at_height`, sorted globally) to decode the same bitfield. This produced a third interpretation of the same data — index 5 in the encoder meant producer A, index 5 in the rewards decoder meant producer B, and index 5 in `accumulate_block` meant producer C (or `None`). Three decoders, three answers, one bitfield. Rewards were misattributed for months. Producers received rewards meant for other producers. Nobody noticed because the total pool was always distributed — the amounts looked plausible.

### What we built around it (symptoms)

- mesh_message_deliveries penalty disabled (gossip scoring fix)
- Direct attestation with minute_tracker registration
- Multiple restart procedures to "fix" attestation
- Node-by-node rsync to recover state
- Staggered restarts to improve mesh positioning
- Theories about peer count, IP co-location, gossip factor

These made attestation delivery more reliable but never addressed the fact that delivered attestations for extra producers were silently discarded, and rewards were computed with the wrong index mapping.

### The fix (two activation heights)

**REWARDS_EPOCH_LIST_FIX_HEIGHT = 13320** (v6.17.0): `calculate_epoch_rewards` uses `epoch_state.producer_list` instead of `active_producers_at_height`. Indices now match the encoder for the base list. Extra producers can't qualify for rewards this epoch anyway (can't reach 54/60 minutes mid-epoch), so ignoring them is semantically correct.

**FULL_BITFIELD_DECODE_HEIGHT = 14000** (v6.17.1): `accumulate_block` in post_commit.rs decodes ALL indices. For indices 0..base_len: passed to `accumulate_block` as before. For indices base_len+: resolved against the extra list and inserted directly into `epoch_state.attested_sets[0]` and `attestation_accum[0]`. The RPC `getAttestationStats` also shows all producers.

At the next epoch boundary, `derive_at_boundary` sees the extra producers in the 3-epoch lookback and includes them in `producer_list`. The death spiral is broken.

**Files**:
- `crates/core/src/consensus/constants.rs` — activation heights
- `bins/node/src/node/apply_block/post_commit.rs` — full decode with [base | extra sorted]
- `bins/node/src/node/rewards.rs` — epoch_list for rewards decode
- `crates/rpc/src/methods/schedule.rs` — three-era RPC display

### Critical lesson: HardForkSchedule vs constant gate

The first attempt to deploy the rewards fix used a `HardForkSchedule` entry. This immediately broke the network because `current_fork_id()` passes `u64::MAX` to include ALL scheduled forks — not just active ones. Adding an entry changes `fork_id` from the first block, not from the activation height. Nodes with the new binary rejected blocks from nodes without it.

The correct approach: use a constant (`REWARDS_EPOCH_LIST_FIX_HEIGHT`, `FULL_BITFIELD_DECODE_HEIGHT`) as a gate in the code. No `HardForkSchedule` entry. The `fork_id` doesn't change. Rolling deploy is safe. The fix activates atomically when all nodes cross the activation height.

`HardForkSchedule` entries are for changes that require simultaneous binary upgrades (like the state root formula change at h=2750). Constant gates are for changes that are backward-compatible during the rolling deploy window.

---

## Why these were invisible

Both bugs shared a trait: the network looked healthy. All nodes synced. Same chain. Same height. Same hash on the explorer. Green status across the board. Then a node would fall 1 block behind — still green, still synced, still on the same chain. Nobody noticed because 1 block behind at 10-second slots is invisible.

The consequences appeared minutes or hours later, far from the cause. A fork at the epoch boundary. A rollback cascade. An auto-snap that wiped state. A producer showing 25% attestation on the explorer. Rewards going to the wrong producer. Each symptom triggered its own investigation, its own theory, its own fix. Mesh quality. Peer count. IP co-location. Restart timing. Gossip scoring. Each theory was partially true — which made it convincing — but none was the root cause.

The root causes were both the same class of bug: a silent discard. An orphan block dropped with no log. An index returning `None` with no warning. The data was there — encoded correctly, transmitted correctly, received correctly — but the node threw it away and kept going as if nothing happened. No error code. No panic. No metric. Just a gap that grew until something broke loudly enough to notice.

We spent months building defenses around the symptoms. Silence pull. Fork detection. Rollback guards. Direct attestation. Mesh scoring fixes. Staggered restarts. Each one helped — fewer cascades, faster recovery, better mesh. But the symptoms kept returning in different forms because the silent discards were still happening on every block.

When the two root causes were fixed — 14 lines for Orphan Chase, a decoder alignment for Full Bitfield Decode — all the symptoms disappeared simultaneously. The defenses we built became redundant. They remain as safety nets, but the network no longer needs them.

## Future considerations

These two pillars may need strengthening as the network scales:

- **Orphan Chase**: currently requests one block at a time. At 100K+ nodes with higher orphan rates, batch requests or pipeline chasing may be needed.
- **Full Bitfield Decode**: the extra list changes block-by-block as producers activate mid-epoch. The decoder reconstructs it from `active_producers_at_height(height)` on every block. At scale, this lookup should be O(1) (cached) not O(n) (ProducerSet scan).
