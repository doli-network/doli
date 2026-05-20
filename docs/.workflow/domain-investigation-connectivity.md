# Domain Investigation Report: Connectivity / Sync

## Domain Lens
Connectivity and synchronization -- peer counts, sync status, connection failures, network partitions, discovery failures, gossip propagation failures, RPC reachability. Examining intra-fleet libp2p peering, gossipsub mesh health, and the sync state machine on the frozen nodes in an 18-node local testnet.

## Chain Context
- **Chain:** DOLI testnet (PoS, 10s slots, 18-node fleet: seed + n1-n17, 14 producers)
- **Client:** `doli-node 6.21.20` (binary at HEAD `479711b5`, reports stale `3faeccc0`)
- **Time window:** 2026-05-19 20:16 (deploy) through 22:10 (investigation)
- **Deployed:** synchronized stop-all at 20:16, staggered restart, n2 wiped+snap-synced

## What I Don't Understand
1. Why the `CoordinatorSnapEscalation` reason specifically disables snap sync on n10/n8 -- is this triggered by a failed snap attempt or by the coordinator's classification logic?
2. Why the state root quorum check ("no group with >= 2 peers") fails during snap sync on n13/n16/seed when the fleet has many advancing nodes -- is it a timing issue or are the advancing nodes not responding to state root queries?
3. Whether the gossip mesh members on n7/n8 (which show `blocks=4` mesh members) are actually the other frozen nodes, creating a gossip island of non-forwarding peers.
4. How n3 successfully snap-synced but n10 (same fork, same block, same height) could not.

## Domain Relevance Assessment
**Relevance: LOW**

**Reasoning:** The frozen nodes are NOT connectivity-starved. Every single node in the fleet has 17 transport peers (full mesh for an 18-node fleet). Gossip mesh exists (`blocks=4` mesh members on all checked nodes). The frozen nodes n10 and seed DO receive gossip blocks -- they just cannot apply them because their sync state machine is stuck in `Syncing:Headers`. The problem is NOT that nodes cannot communicate. The problem is that the sync recovery state machine has exhausted all its recovery paths (snap sync disabled/exhausted, header-first deadlocked). This is a code/logic domain problem (sync state machine), not a connectivity problem.

The one genuine gossip-level finding is that n7 and n8 receive ZERO gossip blocks despite having 4 mesh peers -- their mesh members appear to be other frozen nodes that don't forward blocks. But even this is a CONSEQUENCE of the sync deadlock, not its cause.

## Hypotheses

### H1: Frozen nodes are peer-starved (eclipsed, partitioned) -- conf(0.0, measured) -- DEAD
- **Kill test:** Query `getNetworkInfo` on all frozen nodes. If peer count > 0, peers are present.
- **Kill test result:** EVERY frozen node has exactly 17 peers (the maximum possible in an 18-node fleet).
- **Evidence against:** `getNetworkInfo` on n7: peers=15, n10: peers=16, seed: peers=17, n8: peers=17, n13: peers=16, n16: peers=17. All have near-full peer sets.
- **Verdict:** DEAD. No peer starvation whatsoever.

### H2: n3+n10 form an eclipse pair (connected only to each other) -- conf(0.0, measured) -- DEAD
- **Kill test:** Check if n3/n10 have peers outside their pair.
- **Kill test result:** Both have 16-17 peers. n10's logs show it querying n15, n17, n14, n12, n9, n2, n1, seed, n7, n3, n4, n5, n6, n8, n16 -- essentially ALL peers.
- **Evidence:** n10's sync epoch logs cycle through all 17 peers. n3 recovered via snap sync at 22:01.
- **Verdict:** DEAD. n3 and n10 are not eclipsed -- they are fully connected to the entire fleet.

### H3: Frozen nodes cannot receive gossip blocks (gossip mesh failure) -- conf(0.2, measured) -- PARTIALLY TRUE
- **Kill test:** Check for GOSSIP_BLOCK receipt on frozen nodes.
- **Kill test result:** MIXED.
  - n10: DOES receive gossip blocks (GOSSIP_BLOCK + GOSSIP_RECV at 22:05:29, 22:06:29, 22:06:39, 22:06:49). Mesh: blocks=4.
  - seed: DOES receive gossip blocks (GOSSIP_BLOCK at 22:07:49, 22:08:49). Mesh: blocks=2.
  - n7: ZERO gossip blocks received (GOSSIP_WATCHDOG gossip_silent=1988s). Mesh: blocks=4, but 13 peers have negative scores (-720).
  - n8: ZERO gossip blocks received (GOSSIP_WATCHDOG gossip_silent=1978s). Mesh: blocks=4, but 13 peers have negative scores (-720).
- **Verdict:** PARTIAL. n7 and n8 are genuinely gossip-isolated (INC-I-016 pattern: transport peers present, mesh exists on paper, but mesh members are non-forwarding frozen nodes). But n10 and seed receive gossip just fine -- their problem is that gossip blocks are orphans (50+ heights ahead, parent unknown).

### H4: Synchronized restart at 20:16 caused residual connectivity damage -- conf(0.1, measured) -- DEAD
- **Kill test:** Check current peer counts and whether any node has fewer peers than expected.
- **Kill test result:** All nodes have 17 peers (full mesh). The brief peerless period during restart (n2/n3/n4/n5 momentarily at 0 peers) was transient and fully recovered.
- **Evidence:** No connection failures, no peer scoring degradation from the restart, no NAT issues.
- **Verdict:** DEAD. The restart caused no lasting connectivity damage.

### H5: The deadlock is caused by sync recovery path exhaustion, not connectivity -- conf(0.7, measured)
- **Kill test:** If frozen nodes have working sync recovery paths (snap sync available, header-first capable), they should recover. If recovery paths are exhausted, that explains the deadlock.
- **Kill test result:** CONFIRMED. Every frozen node has exhausted its recovery paths:
  - **n10:** `--no-snap-sync` in launchd plist. Snap sync permanently disabled by configuration. Header-first deadlocked (fork tip not recognized by peers). `[RECOVERY] Genesis resync REFUSED: snap sync disabled (reason: CoordinatorSnapEscalation)` -- repeating every 30s.
  - **n7:** Snap attempts exhausted (3/3). `[RECOVERY] Genesis resync REFUSED: snap attempts exhausted (3/3) (reason: GenesisFallbackEmptyHeaders)` -- all 3 snap sync attempts failed because the fleet was too divergent for state root quorum.
  - **n8:** CoordinatorSnapEscalation + confirmed_height_floor. `[RECOVERY] Genesis resync REFUSED: confirmed_height_floor=110383 (reason: CoordinatorSnapEscalation). Manual intervention required.`
  - **n13:** All 3 snap sync attempts failed (`[SNAP_SYNC] State root collection timed out after 15s -- no group with >= 2 peers`). Fell back to header-first which is deadlocked.
  - **n16:** Same as n13 -- all 3 snap attempts timed out on state root quorum.
  - **seed:** Snap-synced successfully to h=110384 at 21:44:35, but landed on a fork. Then 3 more snap attempts all failed (state root quorum timeout). Now stuck.
- **Contrast with n3 (RECOVERED):** n3 was at the EXACT same fork tip as n10 (h=110367, hash `0b2750dcb31e`), but n3 did NOT have `--no-snap-sync` and successfully snap-synced at 22:01:04 from n11, recovering to h=110418.
- **Verdict:** CONFIRMED. The root cause of the permanent deadlock is recovery path exhaustion, not connectivity.

## Key Evidence Found

### 1. Full transport mesh -- NOT peer-starved
All 18 nodes have 17 peers each:
```
seed: peers=17  n1: peers=17  n2: peers=17  ...  n17: peers=17
```
Source: `getNetworkInfo` RPC queries at ~22:05.

### 2. Block store gaps prevent GetHeadersByHeight from serving headers
Peers that have snap-synced or recovered from a fork have block store gaps. When frozen n10 requests `GetHeadersByHeight(110367)`, many peers cannot serve:
```
n9 at 110368: NO BLOCK (gap)
n14 at 110368: NO BLOCK (gap -- n14 was previously frozen at 110358)
n1 at 110368: NO BLOCK
n2 at 110368: NO BLOCK
```
Peers that DO have block 110368 return headers with `prev_hash=c9ea87806bec` (canonical block at 110367), which causes a chain break because n10's local block at 110367 is `0b2750dcb31e` (forked block).

Source: `getBlockByHeight` RPC on all 18 nodes at height 110368.

### 3. Fork topology: same blocks at different heights
The forked blocks exist on both chains but at different height offsets:
```
Block 0b2750dcb31e: n9 height=110365, n10 height=110367 (offset +2)
Block 5c8683f10fc3: n9 height=110364, n10 height=110366 (offset +2)
Block 6f6c1f2fcaaf: n9 height=110363, n10 height=110365 (offset +2)
```
This means n10's chain had 2 extra blocks below this range that n9 didn't have, creating a height-mapping fork. Source: `getBlockByHeight` RPC cross-node comparison.

### 4. Snap sync disabled by configuration on 4 nodes
```
n9: --no-snap-sync ENABLED (in launchd plist)
n10: --no-snap-sync ENABLED (in launchd plist) <-- FROZEN
n11: --no-snap-sync ENABLED
n12: --no-snap-sync ENABLED
```
Source: `grep "no-snap-sync" ~/Library/LaunchAgents/network.doli.testnet-*.plist`

n10 has `--no-snap-sync` and is frozen. n3 (same fork, snap sync allowed) recovered. This is the structural difference.

### 5. Snap sync quorum failure during fleet divergence
Multiple nodes' snap sync failed because "no group with >= 2 peers" agreed on a state root within 15 seconds:
```
n13: [SNAP_SYNC] Attempt 1/3 failed
n13: [SNAP_SYNC] State root collection timed out after 15s -- no group with >= 2 peers
n16: Same pattern, all 3 attempts
seed: Same pattern after landing on fork at h=110384
```
Source: `~/testnet/logs/n13.log`, `n16.log`, `seed.log` grep for SNAP_SYNC.

### 6. Gossip block receipt vs gossip isolation
- n10 RECEIVES gossip blocks (GOSSIP_RECV at 22:04:29, 22:05:29, 22:06:29, 22:06:39, 22:06:49 -- approximately one per 60s). Mesh: blocks=4.
- n7 receives ZERO gossip blocks. GOSSIP_WATCHDOG: gossip silent for 1988s with 14 peers ahead. Mesh: blocks=4, but 13/17 peers have scores -720.
- n8 receives ZERO gossip blocks. Same pattern as n7.

Source: `~/testnet/logs/n10.log`, `n7.log`, `n8.log` grep for GOSSIP_BLOCK/GOSSIP_RECV.

### 7. n3's successful recovery path
At 22:01:04 (sync_fails=514, gap=51), n3 triggered snap sync. At 22:01:19, received snapshot from n11 at h=110418. Snapshot applied successfully. n3 is now advancing at h=110426+.

Source: `~/testnet/logs/n3.log` grep for SNAP_SYNC.

### 8. n10's permanent deadlock loop
```
22:01:30 [HEALTH] h=110367 sync_fails=537 state="Syncing:Headers" last_applied_ago=1343s
22:01:31 [SYNC] Using GetHeadersByHeight(height=110367) -- post-snap hash fallback
22:01:32 [HEADER_DEBUG] Chain break: prev_hash=c9ea87806bec expected=0b2750dcb31e valid_so_far=0
22:01:32 Empty headers from peer (peer doesn't have blocks at this height)
... repeats with every peer, ~1 iteration/second
22:03:38 [RECOVERY] Genesis resync REFUSED: snap sync disabled (reason: CoordinatorSnapEscalation)
... repeats every 30s
```
Source: `~/testnet/logs/n10.log` tail analysis.

## Causal Chain (connectivity perspective)

| # | Item | Derived? | Derivation |
|---|------|----------|------------|
| 1 | Natural fork at ~h=110365 creates two competing chains | NO -- UNEXPLAINED | Fork cause is in the fork/divergence domain |
| 2 | Some nodes follow the minority fork tip | YES | Direct consequence of fork -- some producers built on the minority chain |
| 3 | Frozen nodes' local tip hash is unrecognized by majority peers | YES | GetHeaders(forked_tip_hash) returns empty from canonical peers |
| 4 | GetHeadersByHeight returns canonical headers that don't chain to forked tip | YES | Canonical block at same height has different hash/prev_hash |
| 5 | Many peers have block store gaps and return EMPTY for GetHeadersByHeight | YES | Peers that snap-synced or recovered from forks lose blocks in the gap range |
| 6 | Sync state machine exhausts header-first attempts | YES | All peers either return chain-break or empty |
| 7 | Snap sync is the only remaining recovery path | YES | Header-first cannot find a common ancestor when the fork diverges before the block store gap |
| 8a | n10: Snap sync disabled by `--no-snap-sync` config | YES (measured) | Launchd plist contains `--no-snap-sync` |
| 8b | n7/n13/n16/seed: Snap sync attempts exhausted (3/3) | YES (measured) | State root quorum failed during fleet divergence |
| 8c | n8: Snap sync refused by CoordinatorSnapEscalation | YES (measured) | confirmed_height_floor prevents snap |
| 9 | No recovery path remains -- permanent deadlock | YES | Header-first fails + snap sync exhausted/disabled = stuck |

## Cross-Domain Signals

### For Fork/Divergence Investigator
- The fork at ~h=110365 involves blocks at different height offsets: the same block `0b2750dcb31e` appears at height 110365 on canonical nodes and height 110367 on forked nodes (2-block height shift). This suggests the fork originated from a slot/height disagreement, not just a competing tip.
- seed snap-synced to h=110384 (hash `da352488`) at 21:44:35, which turned out to be a fork block. The snap sync source was on a minority chain. This means the fleet was already diverging BEFORE the deadlock became permanent.

### For Parameters/Tuning Investigator
- `--no-snap-sync` on n9/n10/n11/n12 is a deployment configuration choice that makes those nodes structurally unable to recover from deep forks. With 4/18 nodes having snap sync disabled, any fork that requires snap recovery will permanently freeze those nodes.
- The snap sync state root quorum requires `>= 2 peers` to agree on a state root within 15 seconds. During fleet divergence, this threshold is unachievable -- a catch-22 where the fleet is too divergent for snap sync to work, but snap sync is needed to end the divergence.
- Snap attempt limit of 3 is too low for a fleet experiencing cascading divergence. By the time the fleet stabilizes enough for snap quorum to work, the attempts may be exhausted.

### For Code/Logic Investigator
- The header-first sync loop has no mechanism to roll back the local tip when the fork is detected. The sync engine sees "chain break at valid_so_far=0" and "empty headers" but cannot initiate a rollback because `recently_synced()` returns false (no block applied in 60s).
- The APPLY_START events on n10 (gossip blocks) produce no APPLY_END -- the blocks are silently classified as orphans (parent not in store, 50+ heights ahead) and cached. No ORPHAN_CHASE is triggered (possibly because the orphan is too far ahead?).
- The `confirmed_height_floor` in n8's snap sync refusal suggests a safety guard is preventing snap sync below a previously confirmed height. This guard may be too conservative when the node is on a fork.

## Gaps
1. **Gossip mesh member identity:** Could not determine WHICH 4 peers are in n7/n8's gossip mesh. If they are all frozen nodes, that explains the gossip silence. The GOSSIP_MESH log does not include peer IDs.
2. **Why n9/n11 (with --no-snap-sync) are NOT frozen:** They are on the canonical chain and never needed snap sync. The `--no-snap-sync` flag is only fatal when combined with a fork that exceeds header-first recovery capability.
3. **Seed's fork entry point:** Seed snap-synced to h=110384 at 21:44:35 onto what became a fork block. What was the fleet state at that moment? Why did the snap sync quorum point to a minority chain?
4. **Whether the 2-block height offset fork is caused by the INC-I-078/080 activation at h=109559:** The activation could cause block content differences (cap enforcement) that some nodes accept and others reject, creating competing chains.
