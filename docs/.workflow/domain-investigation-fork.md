# Domain Investigation Report: Fork/Divergence

## Domain Lens
Fork analysis -- chain splits, reorgs, competing blocks, state root mismatches, consensus disagreements, block rejection, nodes on different chain tips. Central question: Did the chains diverge? If yes, WHEN, WHERE, and WHY?

## Chain Context
- **Chain:** DOLI PoS, 10s slots, 18-node local testnet (seed + n1-n17, 14 producers)
- **Client version:** `doli-node 6.21.20 (3faeccc0)` cosmetically; actual code HEAD `479711b5`
- **Binary md5:** `15e0d6c7e847f0ac37ea10a2e76c291e`
- **Time window:** 2026-05-19 20:16 (deploy) through 22:11 UTC (investigation end)
- **Activation heights:** INC-I-078/080 AH=109,559 crossed and active

## What I Don't Understand
1. **Why the block store height offset exists at all.** The offset (-1 or -2) affects blocks from genesis through approximately h=110357-110396. This is not created by the current code -- it is a pre-existing condition, likely from a prior snap-sync or block-store rebuild event. The exact origin event is outside this session's log window.
2. **Why `set_canonical_chain()` did not correct the offset.** When a node snap-syncs to a correct height, `set_canonical_chain()` walks backward via `prev_hash` assigning heights. But it stops at the snap horizon floor (line 131-135 of writes.rs), so old blocks below the floor retain their wrong heights.
3. **Why `GetHeadersByHeight` returns empty when responding peers have canonical blocks above the requested height.** The responding peer's `get_hash_by_height()` returns None for heights in the gap between their snap horizon and their current tip -- they have blocks above but not below the snap height in their height index.

## Domain Relevance Assessment
**Relevance: HIGH**
This is fundamentally a fork/divergence problem. The fleet is split into multiple chain tips with distinct block stores, and the frozen nodes are stuck because the sync protocol cannot bridge the gap between their forked tip and the canonical chain. The height-offset corruption amplifies the problem by making block comparison unreliable.

## Hypotheses

### H1: The "multi-way fork" is actually a SINGLE natural tip race at h=110360, amplified by block store height-index corruption and sparse canonical indexes -- conf(0.65, measured)
- Kill test: If the fork is natural, both fork blocks at h=110360 should reference the same parent and have different producers/slots.
- Kill test result: CONFIRMED. Both blocks at h=110360 reference parent h=110359 (hash=`1cbc1c406022`, slot=218661). Canonical: producer=`2d27fdcc6a24`, slot=218668. Fork: producer=`b5d98316008d`, slot=218663. Two different producers produced valid blocks on the same parent.
- Evidence: The fork branch (slot=218663) was produced SOONER than the canonical branch (slot=218668), but the canonical branch accumulated more weight (more blocks built on it). n3/n10 chose the fork branch and got stuck when the majority chain outpaced them.
- The frozen nodes (seed/n8/n16 at h=110388) are on a THIRD chain variant -- a fork that diverged at a different height. They are stuck because most peers they contact have sparse height indexes and cannot serve headers in the 110389+ range.

### H2: Block store height-index corruption is propagating via snap sync -- conf(0.55, measured)
- Kill test: If snap sync propagates offset, nodes that snap-synced from offset peers should inherit the offset.
- Kill test result: PARTIAL. n9 has -2 offset in OLD blocks but correct heights in NEW blocks (post-snap-sync). The snap sync correctly sets `chain_state.best_height` but does NOT rewrite old block store entries. So the offset is pre-existing, not actively propagating via snap sync. However, if a node's `chain_state.best_height` is set from an offset peer, the state root response height would be wrong.
- Evidence: n1(offset -2), n9(offset -2), n6(offset -1), n7(offset -1) have offsets in old blocks. Their NEW blocks (written after recent snap syncs) are at correct heights.

### H3: The deadlock is caused by sparse canonical height indexes creating a "header desert" -- conf(0.60, measured)
- Kill test: If this is the cause, the majority of peers should lack blocks at the height the frozen nodes need (e.g., h=110389 for the seed).
- Kill test result: CONFIRMED. Only 4/18 nodes (n4/n11/n15/n17) have a block at h=110389. The other 14 nodes return NONE because they snap-synced to heights above 110389 and have no height-index entries below their snap horizon.
- Evidence: Seed log shows `Headers(empty)` responses consistently. The `GetHeadersByHeight` handler (validation_checks.rs:1012) iterates `get_hash_by_height(height)` which returns None for heights below the responding peer's snap horizon, causing the loop to break immediately (line 1021) and return 0 headers.

### H4: The "two advancing clusters" from the snapshot (cluster A vs seed-cluster) were actually the same chain with different lag -- conf(0.35, inferred)
- Kill test: If they were the same chain, blocks at a common height should match.
- Kill test result: KILLED. At h=110388, seed has `63ea535511a3` while n4 has `a7e2dc08adf6` -- DIFFERENT blocks, different producers, different slots. These are genuine forks, not lag.
- This hypothesis is DEAD.

### H5: The fork at h=110360 was caused by the activation height crossing (AH=109559) or INC-I-082 rebuild divergence -- conf(0.10, inferred)
- Kill test: If AH-related, blocks around h=109559 should show divergence.
- Kill test result: n3 and n4 AGREE through h=110359 (well past AH=109559). The divergence at h=110360 is 801 blocks after activation. The AH crossing did not cause the fork.
- The psHash is identical (`6eb003ff40`) across ALL nodes, meaning ProducerSet is consistent. This rules out INC-I-082 rebuild divergence as a cause.
- This hypothesis is effectively DEAD.

## Key Evidence Found

### 1. Block Store Height-Index Offset (MEASURED)
Verified by comparing blocks at h=1002 across all nodes against canonical reference (n4):
- **Offset 0 (correct):** seed, n3, n4, n5, n8, n10, n11, n12, n13, n14, n15, n16, n17
- **Offset -1:** n6, n7 (block at their local h=1001 matches canonical h=1002)
- **Offset -2:** n1, n9 (block at their local h=1000 matches canonical h=1002)
- **No old blocks:** n2 (wiped and snap-synced, only has recent blocks)

The offset means `n1[h] == canonical[h+2]` and `n6[h] == canonical[h+1]` for ALL heights tested from genesis through ~h=110357. This is exact and consistent, not random corruption.

Critically, n1's block at h=0 has hash `c6952246f768` which equals n4's canonical block at h=2. n1 does NOT have the true genesis block (h=1, hash=`7dd7cffffaf0`).

### 2. Fork at h=110360 (MEASURED)
Last common block: h=110359, hash=`1cbc1c406022`, slot=218661, producer=`b03fe629a0ab`
- **Canonical branch** (n4/n11/n15/n17): h=110360 hash=`0730528ca46e`, slot=218668, producer=`2d27fdcc6a24`
- **Fork branch** (n3/n10): h=110360 hash=`327b903bf4b2`, slot=218663, producer=`b5d98316008d`

The fork branch was abandoned by the majority. n3/n10 remained on it and are stuck at h=110367.

### 3. Seed-cluster Fork (MEASURED)
The seed/n8/n16 are at h=110388 with hash=`63ea535511a3`. This is a DIFFERENT block from n4's h=110388 (hash=`a7e2dc08adf6`). The seed-cluster is on its own fork, distinct from both the canonical chain and the n3/n10 fork.

Seed blocks h=110384-110388: all different producers/hashes from n4's blocks at the same heights.

### 4. GetHeadersByHeight "Header Desert" (MEASURED)
Block availability at h=110389 across all 18 nodes:
- **Have block:** n1 (offset version), n4, n11, n15, n17 (5 nodes)
- **No block:** seed, n2, n3, n5, n6, n7, n8, n9, n10, n12, n13, n14, n16 (13 nodes)

The seed's sync loop sends `GetHeadersByHeight(start_height=110388, max_count=500)` to peers. The handler iterates from `start_height+1 = 110389` calling `get_hash_by_height(110389)` which returns None on 13/18 nodes. The loop breaks immediately, returning 0 headers.

### 5. Dynamic Recovery (MEASURED)
Between the snapshot (22:51) and current state (22:11 next day), several nodes recovered:
- n3: was stuck at 110367, now at 110421+ with new blocks matching canonical (offset +1 in new blocks)
- n7: was stuck at 110383, now at 110436+ with blocks matching canonical (offset 0 in new blocks)
- n9: was at 110396, now at 110438+ with blocks matching canonical (offset 0 in new blocks)
- n12: was at 110385, now at 110436+ advancing
- n14: was stuck at 110358, now at 110414+ advancing

Still frozen: seed (110388), n8 (110388), n16 (110388), n10 (110367), n13 (110385)

### 6. ProducerSet Consistency (MEASURED)
psHash = `6eb003ff40` on ALL 18 nodes. ProducerSet is fleet-identical. The divergence is exclusively in ChainState (csHash) and UtxoSet (utxoHash), meaning the blocks themselves disagree but the producer registry does not.

## Causal Chain

| # | Item | Derived? | Derivation |
|---|------|----------|------------|
| 1 | Pre-existing block store height-index offset (-1 or -2) on n1, n6, n7, n9 | NO -- UNEXPLAINED | Origin event predates this session's logs. Likely from a prior snap-sync that received state from a node with a corrupted height, or from a `rebuild_canonical_index` that assigned wrong heights. |
| 2 | Natural tip race at h=110360 produces two competing blocks | YES | Two producers (`2d27fdcc6a24` and `b5d98316008d`) both built on h=110359. Both blocks are valid. Standard PoS behavior with 14 producers. |
| 3 | n3/n10 choose the minority fork branch at h=110360 | YES | They received/accepted the fork block (slot=218663) first and built on it. The majority built on the other block (slot=218668). |
| 4 | n3/n10 cannot roll back to the canonical chain | YES -> PARTIAL | The chain-break / empty-headers loop cannot find common ancestor because: (a) peers return empty headers due to sparse height indexes, (b) the FINALITY_GUARD requires `recently_synced()` which returns false after minutes of no progress. |
| 5 | Seed-cluster (seed/n8/n16) stuck at h=110388 on own fork | YES | The seed accepted a different producer's block at h=110384+ than the majority. Now stuck because GetHeadersByHeight returns empty from 13/18 peers. |
| 6 | Peers return empty headers because their height indexes have gaps | YES | Most nodes snap-synced to heights above 110389, leaving no height-index entries for the range the seed needs. `get_hash_by_height()` returns None -> loop breaks -> 0 headers. |
| 7 | Some nodes spontaneously recover | YES | When they happen to connect to one of the 4-5 nodes (n4/n11/n15/n17) that have complete height indexes, they receive valid headers and can sync. The probability depends on peer selection. |

## Cross-Domain Signals

### For Parameters/Tuning Investigator
- The `GetHeadersByHeight` handler breaks on the first missing height (writes.rs:1021). A more lenient approach -- skipping gaps or continuing iteration -- would allow peers with sparse indexes to still serve useful headers. This is a parameter/design choice, not a bug per se.
- The snap_horizon floor in `set_canonical_chain()` prevents rewriting old height indexes. This is intentional to avoid walking into missing headers, but it preserves offset corruption indefinitely.

### For Connectivity/Sync Investigator
- The deadlock is fundamentally a sync protocol issue: the `GetHeadersByHeight` fallback fails when responding peers have sparse canonical indexes. The sync manager does not distinguish between "peer has no blocks" and "peer has blocks but not at this exact height."
- Peer selection appears random -- frozen nodes keep hitting peers without the needed heights. There's no preference for peers with complete block histories.

### For Code/Logic Investigator
- The `GetHeadersByHeight` handler (validation_checks.rs:1012-1021) has a fragile iteration pattern: it breaks on the first missing height entry. If a peer has blocks at h=110396+ but not at h=110389, it cannot serve any headers when asked starting from h=110388.
- The block store height-index offset may originate from `rebuild_canonical_index()` (writes.rs:196) which scans all headers to find the tip with the highest slot, then walks backward assigning heights decrementally. If two competing blocks have different slots but are at different heights, this algorithm could assign wrong heights.
- The snap sync path does not verify that `chain_state.best_height` matches the block header's expected canonical height -- it trusts the height provided by the peer.

## Gaps
1. **Origin of the height-index offset.** I cannot determine WHEN or HOW n1, n6, n7, n9 got their offset (-1 or -2). The logs for this event predate the current session. This is a critical gap because fixing the propagation mechanism requires understanding the root cause of the initial corruption.
2. **Why some nodes spontaneously recover and others don't.** The recovery appears to depend on which peers the node happens to connect to. I was unable to trace the exact peer selection that enables recovery because it happens asynchronously.
3. **Seed-cluster fork origin.** I identified that seed/n8/n16 are on a different fork at h=110384+, but I did not trace back to find the exact divergence height where the seed-cluster split from the canonical chain. The seed's block store only has entries from h=110384, suggesting it snap-synced and the fork occurred in the narrow window above that.
4. **Whether the slot gaps (e.g., 192 slots between h=110361 and h=110362 on the fork chain) indicate production failures or network partitions.** These gaps suggest significant liveness issues during the fork period, but I did not cross-reference with connectivity logs.
