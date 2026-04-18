# Two Pillars of Network Stability: Orphan Chase & Full Bitfield Decode

## Summary

Two hidden bugs destabilized the DOLI network for months. Both had the same pattern: the root cause was invisible, so defenses were built around the symptoms instead of fixing the source. When the root causes were finally identified and fixed, the defenses became redundant — but remain as safety nets.

## Pillar 1: Orphan Chase (v6.16.1, 2026-04-16)

### The hidden problem

When a gossip block arrived whose parent was unknown (orphan), the node dropped it silently. The node waited for gossip to deliver the parent — but gossip has no obligation to deliver blocks in order. If the parent arrived late or not at all, the node fell behind permanently.

### What we built around it (symptoms)

- Silence pull: proactive block request when gossip is silent for 30s
- ACTIVE_FORK_DETECT: rollback when peers are ahead
- stuck_fork_signal with 60s guard
- Direct attestation delivery to next-slot producer

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

The producer attests, the encoder includes them, but the decoder throws away the data. They can never re-enter.

### What we built around it (symptoms)

- mesh_message_deliveries penalty disabled (gossip scoring fix)
- Direct attestation with minute_tracker registration
- Multiple restart procedures to "fix" attestation
- Node-by-node rsync to recover state

These made attestation delivery more reliable but never addressed the fact that delivered attestations for extra producers were silently discarded.

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

Both bugs shared a trait: the system appeared to work. Blocks were produced, attestations were sent, rewards were distributed. The failures were silent — a `None` return from `producer_list.get(idx)`, a misattributed index in a reward calculation. No errors, no crashes, no logs.

The symptoms (stuck nodes, low attestation percentages, missing rewards) were attributed to gossip mesh quality, peer count, restart timing, and network topology. Each explanation was partially true — mesh quality did affect delivery timing — but the root cause was simpler: the decoder didn't match the encoder.

## Future considerations

These two pillars may need strengthening as the network scales:

- **Orphan Chase**: currently requests one block at a time. At 100K+ nodes with higher orphan rates, batch requests or pipeline chasing may be needed.
- **Full Bitfield Decode**: the extra list changes block-by-block as producers activate mid-epoch. The decoder reconstructs it from `active_producers_at_height(height)` on every block. At scale, this lookup should be O(1) (cached) not O(n) (ProducerSet scan).
