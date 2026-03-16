# INCIDENT REPORT: SNAP SYNC CASCADE & HASH=0x00 DOS VECTOR

**Date:** 2026-03-14
**Severity:** Critical
**Duration:** ~6 hours active, ~21 hours observable degradation
**Networks:** Testnet (primary), Mainnet (secondary)
**Fix commit:** dd77c7e
**Author:** E. Weil

---

## Summary

A single peer reporting best_hash = 0x0000...0000 (all zeros) was sufficient
to trigger a forced recovery and snap sync on any node it connected to.
Snap-synced nodes then report Hash::ZERO for blocks they no longer have,
so the condition propagated — one infected node infected its neighbors in a
chain reaction.

The attack surface was open since the first snap sync occurred on the network.
It operated as an accidental recurring failure for 21 hours before being
identified as a deliberate DoS vector.

- **Attack cost:** $5/month VPS + reading manager.rs
- **Fix:** 3 code changes, ~20 lines total
- **Recovery:** Wipe and rebuild 22 nodes from known-good data source

---

## Timeline

### 2026-03-13 07:01 — NT6 initial snap sync (legitimate)

NT6 registered at height 0 with a gap of 19,607 blocks. Snap sync
completed correctly. However it left NT6 with an incomplete block store
(blocks 1-19,607 missing). INTEGRITY_RISK warning appeared immediately:

```
[INTEGRITY_RISK] Genesis producer mismatch: StateDb has 13 producers
but block store has 5. Block store may be incomplete (snap sync gap).
```

This warning appeared on every restart of NT6 for the next 21 hours
and was never resolved by any automated mechanism.

### 2026-03-13 11:42 — First cascade trigger on NT6

NT6 fell 1 block behind peers. The peer
12D3KooWKLsxgV3CkXTG8oQVEjQPHwAkXegZhoxn6T5pBuCcv59f
reported its hash for height 21,297 as:

```
peer=9b0ea1c9...  (first report — real hash, genuine mismatch)
peer=0000000000000000000000000000000000000000  (all subsequent reports)
```

The sync manager interpreted Hash::ZERO as a different chain hash,
incremented disagree counter, detected minority fork, set
fork_mismatch_detected = true, and blocked production.

Simultaneously, NT6 requested headers from other peers. Those peers
had already applied the block NT6 needed and returned 0 headers.
The sync manager interpreted empty headers as fork evidence,
blacklisted each peer, and incremented consecutive_empty_headers.

After 10 consecutive empty header responses the sync manager triggered
genesis resync, which called force_recover_from_peers(), which called
reset_state_only(), which triggered snap sync.

Block store wiped. NT6 became a carrier node.

### 2026-03-13 12:18 — Snap sync #2 on NT6 (gap=21,507)

NT6 recovered via snap sync but immediately had incomplete block store
again. INTEGRITY_RISK warning reappeared. Same conditions as before.

### 2026-03-13 14:24 to 14:42 — Snap syncs #3 through #9 on NT6

Five snap syncs in 18 minutes as NT6 cycled through the same failure
pattern at increasing heights. Each snap sync left a larger block store
gap, making the next trigger faster.

```
14:24 — gap=22,261
14:25 — gap=22,268
14:26 — gap=22,275 and 22,276
14:27 — gap=22,277
14:40 — gap=22,357
14:42 — gap=22,359 (forced recovery)
```

### 2026-03-14 00:36 — Snap sync #10 triggered by consecutive apply failures

3+ consecutive apply failures with gap=38 triggered genesis resync.
This is a separate code path from the Hash::ZERO vector but the same
wrong escalation: a small gap that gossip would resolve in seconds
caused a full block store wipe.

### 2026-03-14 02:54 — Snap sync #11 (gap=26,743, forced recovery)

### 2026-03-14 04:00 — Snap sync #12 — gap=1 block

This is the definitive proof of the bug severity:

```
Recovery (forced): resetting state, snap sync will restore
local h=27133, peer h=27134
gap = 1
```

A single block gap triggered a complete snap sync. This is the worst
case: the node is 10 seconds behind and the system responds by wiping
its entire block store.

### 2026-03-14 04:36 — N10 mainnet falls into same pattern

N10 showed fork_counter=19 and h=0 with hash=dee2ef98 while peers
were at h=27,536. The same peer pattern was present. N10 was in a
snap sync loop it could not exit.

### 2026-03-14 ~05:00 — Cascading fragmentation across testnet and mainnet

The dashboard showed:
```
Testnet: NT1 h=0, NT4 h=39, NT5 h=16, NT7 h=23, NT9 h=180,
         NT10 h=78, NT11 h=144, NT12 h=94
Mainnet: N4 h=1, N6 h=107 (stuck)
Two different hashes on synced nodes — active fork
```

At this point patch-based recovery was abandoned. The cascading
snap syncs were faster than surgical fixes could be deployed.

### 2026-03-14 ~06:00 — Decision: wipe and rebuild from known-good source

NT2 was identified as the node with a complete, clean block store.
All other nodes were rebuilt from NT2's data in this order:

1. Stop all affected nodes
2. Wipe data directories
3. Copy NT2 data to all testnet nodes
4. Start seeds first (Seed1, Seed2, Seed3)
5. Start producers after seeds are established
6. Run backfillFromPeer to verify block store integrity
7. Repeat procedure for mainnet using Seed2 as source

### 2026-03-14 ~07:00 — 30/30 nodes synced

- Mainnet: 15/15 synced, h=27,599, all integrity checks passing
- Testnet: 15/15 synced, h=27,582, all integrity checks passing
- Fix dd77c7e deployed to all 30 nodes

---

## Root Cause

The sync manager had one escalation path for two completely different
problems:

- **Problem A:** node is 1-2 blocks behind (normal, resolves in <10 seconds)
- **Problem B:** node is on a different chain (genuine fork)

Both problems triggered the same response: consecutive failure counters
leading to snap sync.

### Bug 1 — Layer 9 of can_produce() (manager.rs line 1166)

When a peer is 1-2 blocks ahead, the sync manager counts it as
disagreeing with the local chain. This is correct for genuine forks.
But snap-synced peers report Hash::ZERO for blocks they don't have.
Hash::ZERO != local_hash, so it counted as disagree, triggering
fork_mismatch_detected.

```
A peer with a snap sync gap reports Hash::ZERO.
The node interprets this as a fork.
The node triggers snap sync.
Now the node also has a snap sync gap.
The node reports Hash::ZERO to its peers.
Those peers trigger snap sync.
The cascade propagates.
```

### Bug 2 — handle_headers_response() (manager.rs line 2554)

When a peer returns 0 headers with a gap of 1-2 blocks, the peer
has already applied that block and has nothing to give. This is
normal behavior. The sync manager interpreted it as fork evidence,
blacklisted the peer, and incremented consecutive_empty_headers.
After 10 such responses, it triggered genesis resync.

### Bug 3 — block_apply_failed() (manager.rs line 2853)

3+ consecutive apply failures triggered genesis resync regardless
of the gap size. A gap of 1-50 blocks that gossip would resolve
in minutes caused a full block store wipe.

---

## The Fixes (commit dd77c7e)

### Fix 1 — Layer 9: Hash::ZERO is not a fork (manager.rs line 1166)

```
Before:
  if peer is 1-2 blocks ahead:
      disagree += 1

After:
  if peer is 1-2 blocks ahead:
      if peer.best_hash == Hash::ZERO:
          continue  // snap sync gap, not a fork
      disagree += 1
```

This breaks the contagion chain. A carrier node can no longer
infect healthy nodes.

### Fix 2 — Empty headers with small gap is not fork evidence (line 2554)

```
Before:
  empty headers = fork evidence, blacklist peer

After:
  if gap <= 2:
      return to Idle, wait for gossip  // peer is at tip, normal
  if gap > 2:
      fork evidence, blacklist peer
```

### Fix 3 — Apply failures with small gap are not corruption (line 2853)

```
Before:
  3+ apply failures = genesis resync

After:
  3+ apply failures with gap <= 50 = reset counter, wait for gossip
  3+ apply failures with gap > 50  = genesis resync
```

---

## Recovery Procedure

When nodes have fragmented from snap sync cascades and surgical repair
is slower than the cascade propagates, use this procedure:

**Step 1** — Identify a node with a clean, complete block store.
A node that has never had a snap sync gap. Verify with:
```bash
curl -X POST http://NODE:PORT -d '{"method":"getBlockByHeight",
"params":{"height":1},"id":1}'
```
If block 1 exists, the store is clean from genesis.

**Step 2** — Stop all affected nodes.
Do not attempt to repair while the cascade is active.
Stopping cleanly prevents further block store corruption.

**Step 3** — Wipe data directories of all affected nodes.
```bash
find /path/to/node/data -mindepth 1 -delete
```
Do not wipe the source node.

**Step 4** — Copy source node data to all targets.
```bash
# Same server:
cp -r /source/data/* /target/data/
# Different server:
scp -r /source/data/* server:/target/data/
```

**Step 5** — Start seeds first.
Seeds must be established before producers attempt to connect.
Wait 10 seconds after seeds start before starting producers.

**Step 6** — Start producers.

**Step 7** — Verify block 1 on all nodes.
```bash
curl -X POST http://NODE:PORT -d '{"method":"getBlockByHeight",
"params":{"height":1},"id":1}'
```

**Step 8** — Run backfillFromPeer on all nodes.
```bash
curl -X POST http://NODE:PORT -d '{"method":"backfillFromPeer",
"params":{"rpc_url":"http://SEED:PORT"},"id":1}'
```
If result shows no gaps, the node is fully intact.

**Step 9** — Monitor for 1 epoch (1 hour) to confirm stability.

---

## The DOS Vector

Before fix dd77c7e, any external party could execute this attack:

1. Spin up a standard DOLI node ($5/month VPS)
2. Modify the client to always report best_hash = Hash::ZERO
   in status responses
3. Connect to the network
4. Wait

Every node that connected to this peer would eventually trigger a snap
sync. Those nodes would then report Hash::ZERO to their peers. The
cascade would propagate through the entire network.

No cryptographic capability was required. No stake was required.
The attack exploited a logic error in the sync manager's interpretation
of missing data as fork disagreement.

After fix dd77c7e, Hash::ZERO is silently ignored in fork detection.
A peer reporting all zeros is treated as having a snap sync gap, not
as being on a different chain. The cascade cannot start.

---

## What Worked

- Identifying NT2 as the clean source node
- Copying data rather than re-syncing (minutes vs hours)
- Ordered restart: seeds before producers
- backfillFromPeer for integrity verification
- The operator's decision to abandon patch-by-patch repair
  and rebuild from known-good state

## What Did Not Work

- Backfilling individual nodes while the cascade was active
  (new snap syncs were faster than backfills)
- Adjusting thresholds and cooldowns as bandaid fixes
  (addressed symptoms, not the root cause)
- Agent-driven patch deployment under time pressure
  (each fix required compile + deploy cycle, too slow)

---

## Lessons Learned

1. **The operator's intuition was correct before the code analysis confirmed it.**

   The operator had identified Hash::ZERO as a potential attack vector
   and had previously created the genesis hash mechanism for related
   defensive reasons. The agents minimized the concern. 21 hours of
   log evidence and a cascading network incident confirmed the operator
   was right.

   When an operator with deep system knowledge flags a potential vector,
   treat it as a confirmed vector until proven otherwise.

2. **Patch-by-patch repair during an active cascade is slower than the
   cascade itself.**

   Every fix required: identify problem, write code, compile (~15 min),
   deploy, restart, observe. The cascade propagated faster than this
   cycle. The correct response once a cascade is confirmed is to stop
   all nodes, identify the clean source, and rebuild.

3. **INTEGRITY_RISK warnings that persist across restarts are critical alerts.**

   The warning "Genesis producer mismatch: StateDb has N producers but
   block store has 5" appeared on NT6's first restart at 08:06 on March 13
   and on every restart thereafter until March 14. This warning means the
   node is fragile — any small gap will trigger snap sync.

   A node showing this warning should be immediately rebuilt from a
   clean source. It should not be left running.

4. **Missing data (Hash::ZERO) and disagreeing data (different hash) are
   different conditions requiring different responses.**

   Hash::ZERO means: I don't have this block.
   Different non-zero hash means: I have this block and it's different.

   Only the second condition is fork evidence. The sync manager treated
   both identically for years. This was the root cause of the entire
   incident.

5. **There is no manual for this.**

   The recovery procedure above was derived from operational judgment
   during the incident. It did not exist before this incident. It should
   be consulted for any future snap sync cascade.

---

## Open Items

1. Monitor the network for the peer 12D3KooWSMWQuANxMRXDm55onhJu9rgGtGB8ptS54k9s2kXnRU8U
   which appeared in N10 logs attempting to connect with a different
   genesis hash 26 times between March 12-13. This may be a
   misconfigured external node or a deliberate probe. If it reappears
   after fix dd77c7e is deployed, investigate further.

2. Implement an automated alert when INTEGRITY_RISK warning appears
   on startup. Current behavior: warning is logged and ignored.
   Required behavior: alert operator, trigger automatic rebuild from
   nearest clean peer.

3. The deployment procedure should be changed to rolling restart
   (one node at a time with health verification) rather than
   simultaneous restart of all nodes. Simultaneous restarts create
   the conditions for snap sync cascades by taking multiple nodes
   offline and online at the same moment.

4. Add a metric for consecutive snap syncs per node. Three snap syncs
   in one hour should trigger an alert. Twelve snap syncs in 21 hours
   went undetected until the cascade became critical.

---

E. Weil
weil@doli.network
