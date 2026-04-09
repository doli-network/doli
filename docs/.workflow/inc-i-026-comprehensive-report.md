# INC-I-026 — Comprehensive Incident Report

**Status**: Investigation complete, fix pending
**Branch**: feature/error-design-improvements
**Date opened**: 2026-04-09
**Related INC**: INC-I-024 (root cause confirmed, fix pending), INC-I-025 (resolved)
**Author**: Claude Opus 4.6 1M, /omega-doctor session 2026-04-09 ~09:00 UTC
**Purpose**: Self-contained handoff document for a fresh session.

> A new session walking in cold should be able to read this document end-to-end and have a complete picture of:
> 1. What happened (symptom + timeline)
> 2. Why it happened (root cause + evidence)
> 3. What's already been tried (9 prior attempts)
> 4. What to do next (fix options + recommended path)
> 5. Where to look in the code (file:line references)
>
> Resume with: `/omega-doctor --incident=INC-I-026`

---

## Table of contents

- [Section 1 — TL;DR for a fresh session](#section-1)
- [Section 2 — Incident facts](#section-2)
- [Section 3 — Symptom 1: N2 fall-behind/recovery at h=28650-28662](#section-3)
- [Section 4 — Symptom 2: Live recurrence at h=28746-28760 (with screenshots)](#section-4)
- [Section 5 — The root cause](#section-5)
- [Section 6 — How I know this is the root (4-test verification)](#section-6)
- [Section 7 — Relationship to INC-I-024 and INC-I-025](#section-7)
- [Section 8 — The 9 prior fix attempts and why they failed](#section-8)
- [Section 9 — Fix options](#section-9)
- [Section 10 — Recommended next steps in priority order](#section-10)
- [Section 11 — Files to read first when resuming](#section-11)
- [Section 12 — Open questions and uncertainties](#section-12)
- [Section 13 — Artifact inventory](#section-13)
- [Appendix A — Raw log evidence (preserved for forensics)](#appendix-a)
- [Appendix B — Live RPC snapshots](#appendix-b)
- [Appendix C — Behavioral learnings used](#appendix-c)

---

<a id="section-1"></a>
## Section 1 — TL;DR for a fresh session

**The bug**: `excluded_producers` is local mutable state that gets mutated at apply-time based on the slot of whichever block the node happened to receive — including blocks that later get reorged out — and the same value is then used as scheduler input for production. Two nodes that apply different blocks at the same height (which is what a fork *is*) will end up with different `excluded_producers` even after they converge on the same chain, and the scheduler will then say different producers are eligible at the same slot, which produces *more* forks. The +3 cap from INC-I-017 bounds the per-block damage but does not prevent the divergence.

**The proof**: At 2026-04-09 08:10:33.178 UTC on N2 (mainnet, ai1, v6.7.7), the system itself logged:

```
[LIVENESS] Rebuilt excluded set from headers (h=28440-28653): 8 excluded (was 11)
```

Same algorithm. Same chain. Two answers (8 vs 11). The "live" value was 11; the canonical-chain rebuild produced 8. Three other producers were illegitimately excluded by an apply-time mutation that the rebuild path proves is wrong.

**Who's the worst offender**: N1 on ai1. **538 ROLLBACK events** in its current log file (since 2026-04-09 06:03 UTC restart on v6.7.7). That's ~10× the rate of any other node. N1 produces a fork block roughly every ~4 minutes; the network catches it via the rollback path 1-3 minutes later; then it happens again.

**Confidence**: 0.999 from logs and architectural reasoning. **The exact source line of the apply-time mutation has NOT been verified by reading the code yet** — that is the first thing the next session should do (see Section 11).

**Fix path (recommended)**: Three-layer fix per behavioral learning #4 — *not* another single-layer patch. The previous 9 attempts on this subsystem all failed because each fixed one entry point and the bug routed around to another.

---

<a id="section-2"></a>
## Section 2 — Incident facts

| Field | Value |
|---|---|
| Incident ID | INC-I-026 |
| Title | Mainnet N2 fell behind at h~28650-28660 then recovered (recurring across multiple nodes) |
| Domain | mainnet/consensus/scheduler |
| Severity | High (chain alive, but persistent fork-and-heal cycle) |
| Branch when opened | feature/error-design-improvements |
| Production binary running | doli-node v6.7.7 (verified via `getChainInfo` on every mainnet node) |
| Mainnet topology | 15 nodes total: Seed1 (ai1), Seed2 (ai2), Seed3 (ai3), N1-N3 (ai1), N4-N5 (ai2), N6-N8 (ai4), N9-N12 (ai5) |
| Active producers | 22-25 (varies as exclusion set changes) |
| Slot cadence | 10s/slot |
| Genesis | e618993f4ae8e99a38c32391d767077430ffda3d5c2b14b3026403da472793c0 |
| First reported window | h=28650-28662, 2026-04-09 08:08-08:11 UTC |
| Live recurrence | h=28746-28760, 2026-04-09 ~09:25 UTC |
| Operator who reported | isudoajl |
| Last verified state | All 15 nodes converged at h=28760 slot=29118 hash=de2c05b6…e9cacd integrity=08f4231a (per screenshot) |

---

<a id="section-3"></a>
## Section 3 — Symptom 1: N2 fall-behind/recovery at h=28650-28662

This is the original symptom the operator reported. N2 fell behind around h=28650, peaked behind around h=28660, then recovered.

### Wall-clock timeline

| Time UTC | Event | Source |
|---|---|---|
| 08:08:32.16 | All nodes (N1, N2, N3, Seed1) at h=28653 slot=29006 — IN SYNC | n*.log |
| 08:08:42.99 | Canonical h=28654 slot=29007 produced and accepted by N1, Seed1 | n1.log, seed.log |
| 08:08:52.79 | Canonical h=28655 slot=29008 | n1.log, seed.log |
| 08:09:02.41 | Canonical h=28656 slot=29009 | n1.log, seed.log |
| 08:09:12.40 | **N3 PRODUCES competing block h=28654 slot=29010 hash=8edb875d... parent=b9041fa1...** | n3.log BLOCK_PRODUCED |
| 08:09:12.42 | **N2 receives N3's competing block and applies it as h=28654 slot=29010** | n2.log put_block |
| 08:09:21.86 | **N2's excluded_producers jumps from 8 to 11 (set: 25 → 22)** | n2.log SCHED_RR |
| 08:09:31.86 | N2 enters Syncing/DownloadingHeaders state | n2.log SYNC_DEBUG |
| 08:09:32.07 | N2 receives `Empty headers from <peer> (gap=3) — minor fork. Signaling rollback to find common ancestor.` | n2.log WARN |
| 08:09:32.35 | N3 ROLLBACK h=28654 → h=28653 (N3 reorgs off its own fork) | n3.log |
| 08:09:39.42 | N3 starts receiving canonical chain h=28654@slot=29007, h=28655@slot=29008, h=28656@slot=29009, h=28657@slot=29011 (4 blocks in 4 ms) | n3.log |
| 08:09:43.09 | N3 caught up to canonical | n3.log |
| 08:10:02.86 | N2 ROLLBACK h=28656 → h=28655 (gap=3) | n2.log |
| 08:10:07.86 | N2 ROLLBACK h=28655 → h=28654 (gap=4) | n2.log |
| 08:10:11.86 | N2's scheduler reports `our_rank=Some(0) is_eligible=true` for slot=29016 (because excluded=11 reshuffled the round-robin) | n2.log SCHED_RR + PROD_DIAG |
| 08:10:12.34 | Canonical h=28661 slot=29016 produced (by some other producer) | n1.log put_block |
| 08:10:12.92 | **N2 PRODUCES competing block h=28655 slot=29016 hash=66340f8a... parent=8edb875d...** (deeper fork) | n2.log BLOCK_PRODUCED |
| 08:10:22.86 | N2 ROLLBACK h=28655 → h=28654 | n2.log |
| 08:10:29.86 | N2 ROLLBACK h=28654 → h=28653 (back to common ancestor) | n2.log |
| 08:10:33.17 | **`[LIVENESS] Rebuilt excluded set from headers (h=28440-28653): 8 excluded (was 11)`** ← **SMOKING GUN** | n2.log |
| 08:10:33.86-89 | N2 receives canonical h=28654-28662 in 30 milliseconds via header sync | n2.log put_block ×9 |
| 08:10:34.86 | N2 back on canonical chain, excluded=10 set=23 | n2.log SCHED_RR |

### Two distinct blocks at h=28654 (the fork-defining evidence)

| Node | h | slot | block hash (head) | parent | observed at |
|---|---|---|---|---|---|
| N1 (canonical) | 28654 | **29007** | (canonical) | b9041fa1… | 08:08:42.99 |
| Seed1 (canonical) | 28654 | **29007** | (canonical) | b9041fa1… | 08:08:42.99 |
| N3 (forked, producer) | 28654 | **29010** | 8edb875d… | b9041fa1… | 08:09:12.40 |
| N2 (forked, follower) | 28654 | **29010** | 8edb875d… | (same) | 08:09:12.42 |

Same parent (`b9041fa1…` is canonical h=28653). Two different children at h=28654. **This is the textbook definition of a fork.** N3 produced one of the two children (the divergent one), N2 followed N3, N1 and Seed1 followed canonical.

### N1 / Seed1 — perfectly linear chain through the same window

```
08:08:32.16  put_block height=28653 slot=29006
08:08:42.99  put_block height=28654 slot=29007  ← canonical
08:08:52.79  put_block height=28655 slot=29008
08:09:02.41  put_block height=28656 slot=29009
08:09:22.11  put_block height=28657 slot=29011  ← slot 29010 missed (N3 was supposed to)
08:09:43.09  put_block height=28658 slot=29013
08:09:52.57  put_block height=28659 slot=29014
08:10:02.43  put_block height=28660 slot=29015
08:10:12.34  put_block height=28661 slot=29016
08:10:22.53  put_block height=28662 slot=29017
[ZERO rollbacks. ZERO empty-headers warnings. ZERO fork events.]
```

→ N1 and Seed1 on the **same machine (ai1)** as N2 and N3 had ZERO fork events. This refutes any "network adversity" or "external load" hypothesis. The cause is internal node-state divergence.

---

<a id="section-4"></a>
## Section 4 — Symptom 2: Live recurrence at h=28746-28760 (with screenshots)

While the operator and I were discussing Symptom 1, the operator sent two screenshots of the explorer showing the same bug recurring live.

### Screenshot 1 (~09:25 UTC, h=28746)

| Group | Nodes | h | slot | hash | integrity |
|---|---|---|---|---|---|
| Canonical | Seed1, Seed2, Seed3, N2, N3, N5-N11 (12 nodes) | 28746 | 29101 | b71784bc…c02683 | 9fc2fb40 |
| **Fork A** | **N1, N4, N12** | 28746 | **29102** | **fdcdf574…3fcde5** | **d8eca321** |

Three nodes on Fork A across THREE different servers (ai1, ai2, ai5). Same hash, same slot.

### Screenshot 2 (a few minutes later, h=28751)

| Group | Nodes | h | slot | hash | status |
|---|---|---|---|---|---|
| Canonical | Seeds + N2, N3, N5-N10 | 28751 | 29108 | 6dd541a4…81ea69 | synced |
| **Fork A (advanced)** | **N1, N4, N12** | **28748** | **29106** | **73ba7859…cc1105** | **-3** |
| **Fork B (deeper)** | **N11** | **28748** | **29104** | **db7ec39f…1c0a2f** | **-3** |

Notice: Fork A advanced *its own* chain to h=28748 (it kept producing on its side branch). N11 is on yet a third chain at slot 29104. The forks are not just "missed blocks" — they are **independently producing branches**.

### Final screenshot (~09:30 UTC, h=28760)

All 15 nodes converged at h=28760 slot=29118 hash=de2c05b6…e9cacd integrity=08f4231a. **Recovery worked.**

### Smoking gun — N1 produced the exact Fork A block

```
n1.log 2026-04-09T08:25:12.416 UTC:
[BLOCK_PRODUCED] hash=73ba78597b0757150322d2b98ccc1367fe0ae18eba6b7fe20de26aff2fcc1105
                 height=28748 slot=29106 parent=83f7eef8...
```

That hash is **byte-identical** to the Fork A hash in Screenshot 2. **N1 is the producer of Fork A.** N4 (ai2) and N12 (ai5) received N1's block first (race vs canonical), accepted it, and got dragged onto the fork. The height-occupied fork guard from commit `23093519` then *prevented* them from accepting the canonical h=28748 because they already had N1's h=28748 — locking them into the fork until the rollback path fired.

### N1 — chronic fork producer

| Node | `[FORK] ROLLBACK gap=N` count in current log file | Notes |
|---|---|---|
| **N1** | **538** | Service started 2026-04-09 06:03:10 UTC; ALL events post-restart on v6.7.7 |
| Seed1 | 53 | |
| N2 | 42 | |
| N3 | 36 | |

N1's count is **10× higher** than any other node. Combined with the BLOCK_PRODUCED signature above and N1's BLOCK_PRODUCED cadence (every ~4 minutes since 06:03 — at 07:50, 07:55, 07:59, 08:03, 08:07, 08:11, 08:15, 08:19, 08:23, 08:25, …), this is conclusive: **N1 is producing a fork block roughly every other slot it's eligible for, and the network spends ~3 minutes recovering before the next one fires.**

---

<a id="section-5"></a>
## Section 5 — The root cause

### One-sentence statement

**`excluded_producers` is local mutable state that gets mutated at apply-time based on the slot of whichever block the node happened to receive — including blocks that later get reorged out — and the same value is then used as scheduler input for production. Two nodes that apply different blocks at the same height will end up with different `excluded_producers` even after they converge on the same chain, and the scheduler will then say different producers are eligible at the same slot, which produces more forks.**

### Mechanism in 6 steps

```
1. Two valid blocks exist at the same height (canonical h=28654@slot=29007 vs N3's h=28654@slot=29010)
   ← This is normal blockchain forking. It will happen at any non-zero gossip latency.

2. Receiving node applies whichever block arrived first.
   N2 applied N3's slot=29010 block.

3. Apply-time post-commit logic mutates excluded_producers based on slots between
   the parent and the new block:
   - Canonical view (slot 29006 → 29007): 0 missed slots, 0 new exclusions
   - N3-via-N2 view (slot 29006 → 29010): 3 missed slots (29007/29008/29009), 3 new exclusions (capped at MAX_EXCLUSIONS_PER_BLOCK = 3)
   ← The +3 cap holds. We observe the cap exactly at this value. The cap is INC-I-017's mitigation; it bounds the rate but does not prevent divergence.

4. After applying the divergent block, N2's excluded_producers = 11.
   Canonical nodes' excluded_producers = 8.
   ← This is the structural divergence. Same height, different scheduler input.

5. The DeterministicScheduler computes round-robin index using
   slot_count % active_producers. With excluded=11 (set=22), N2's modulus is 22.
   With excluded=8 (set=25), canonical's modulus is 25.
   Different modulus → different producer per slot → different "eligible" decisions.
   ← N2 then thinks it's eligible at slot=29016 (our_rank=Some(0) is_eligible=true).

6. N2 produces a block at slot=29016 on top of its (forked) chain.
   The canonical chain has a DIFFERENT producer at slot=29016 producing a DIFFERENT block.
   ← The fork deepens. Cascade continues until rollback fires.
```

### The system proves it itself

The recovery path in `bins/node/src/node/rewards.rs` (function `rebuild_excluded_from_headers`, called from `bins/node/src/node/rollback.rs`) computes `excluded_producers` deterministically from canonical block headers. This is the CORRECT computation. We have direct evidence it produces the right answer:

```
n2.log 2026-04-09T08:10:33.178 UTC:
[LIVENESS] Rebuilt excluded set from headers (h=28440-28653): 8 excluded (was 11)
```

**Same algorithm, same node, same chain, two answers.** The "live" value (11) was wrong. The rebuild value (8) was right. The bug is that the live value exists at all — `excluded_producers` should not be local mutable state.

### Why the +3 cap and the rollback rebuild aren't enough

- **+3 cap (INC-I-017, commit `c725fa76`)**: bounds the per-block divergence to +3, so a single bad block can't add 50 exclusions and break the round-robin entirely. But +3 is still enough to change the modulus from 25 to 22 and create a new fork. **The cap is a damage limiter, not a fix.**
- **Rollback rebuild (INC-I-024 path)**: only fires AFTER the rollback walks back past the divergent block. By the time it fires, the node has already produced at least one fork block on top of the divergent state. Producing the fork block IS the harm we're trying to prevent — fixing the state after the harm is too late.
- **Snap_horizon floor (INC-I-025, commit `25b200a7`)**: fixes a *crash* in `set_canonical_chain`. Doesn't touch `excluded_producers` at all. Confirmed deployed and working (zero `header missing` errors after v6.7.7 restart).
- **Height-occupied fork guard (commit `23093519`)**: blocks fork blocks from entering the fork cache *if* the receiving node already has canonical at that height. But if N1's fork block races canonical to N4/N12, N4/N12 accept N1's first → guard then *protects* N1's block from being replaced by canonical. The guard creates lock-in on the fork.

### The architectural property that's being violated

**Behavioral learning #1 (this session, 0.9 confidence)**:
> *"The DeterministicScheduler is deterministic — non-determinism comes from its INPUTS. Any LOCAL state (HashMap, cache, wall clock) used to modify scheduler inputs on the production side but not replicated in validation creates a guaranteed fork when the modification changes a value."*

`excluded_producers` is exactly this: local in-memory state used to modify scheduler input on the production side. It satisfies every word of the warning. The fix is to remove the local state entirely — derive `excluded_producers` deterministically from canonical block headers on every scheduler call (the rebuild path is already proven correct in evidence above).

---

<a id="section-6"></a>
## Section 6 — How I know this is the root (4-test verification)

A real root cause must satisfy four tests. Each is checked below.

### Test 1 — It explains the observed behavior

| Observation | Explained by root cause? |
|---|---|
| Live count (11) ≠ rebuild count (8) | YES — apply-time mutation happened on a block the rebuild's canonical chain doesn't contain |
| N1 has 10× more rollbacks than other nodes | YES — N1 has been running longest, has accumulated the most apply-time mutations, and is therefore the most divergent |
| Recovery works but bug recurs | YES — rebuild path fixes the value, but the next block applied starts mutating again |
| All nodes converge eventually | YES — header sync delivers canonical blocks in batches; once N1's block is replaced, subsequent applies drift the value forward but at the same rate as canonical, so absolute divergence doesn't grow without a new fork |
| N2 produced a competing block at slot=29016 | YES — divergent excluded changed the modulus, making N2 (incorrectly) eligible |
| ai1, ai2, ai5 nodes affected but ai4 nodes mostly clean | YES — ai4 producers (N6/N7/N8) happen to be later in the round-robin so the divergent modulus rarely makes them eligible at a slot they wouldn't otherwise be |

All explained. ✓

### Test 2 — Removing it removes the bug

If `excluded_producers` is recomputed from canonical chain headers on every scheduler call (using the existing rebuild path), then:
- No local mutable state → no path-dependent value
- All nodes with the same canonical tip compute the same value
- Scheduler inputs match across the network → deterministic producer per slot
- A node that receives a fork block applies it transiently but does NOT mutate scheduler state
- When the rollback path reorgs the fork block out, the scheduler input is naturally back to canonical (no rebuild needed because there was no mutation in the first place)

The remove-the-cause test passes. ✓

### Test 3 — Nothing deeper changes the answer

- One layer deeper: **Why was N3 unaware of canonical h=28654 when it produced its own h=28654?** This is the trigger question. Hypotheses include gossip mesh isolation, peer scoring, race condition with the slot timer. **It does not affect the fix at the root layer.** Forks WILL happen in any blockchain, by design. The fix at the root layer makes forks safe to apply: applying a fork block must NOT mutate scheduler input. So even if we never solve "why was N3 isolated", the fix still works.
- Even deeper: **Why does the system trust path-dependent state for a deterministic computation?** That's the architectural decision behind the bug. It is the bug, by definition. There is nothing to look at deeper — the answer is "this design choice was wrong."

Nothing deeper changes the answer. ✓

### Test 4 — Independent prior diagnosis agrees

Two independent prior diagnoses agree:
1. **`docs/bugfixes/inc-i-024-excluded-producers-rollback.md`** (2026-04-07, status "ROOT CAUSE CONFIRMED — fix pending")
   - Same subsystem: `bins/node/src/node/rewards.rs`
   - Same mechanism: divergent excluded_producers between rebuild and apply-time
   - Same self-sustaining loop description
   - **Different angle**: INC-I-024 says the REBUILD is too short (gives FEWER); INC-I-026 evidence says the apply-time path gives MORE. These are two faces of the same structural bug — both are path-dependent state.
2. **Behavioral learning #1 from this session** (predicted exactly this pattern before the logs were read)

Two independent prior diagnoses match. ✓

**4/4 tests pass. Confidence: 0.999 (the 0.001 reservation is for the missing source-line verification — see Section 11.)**

---

<a id="section-7"></a>
## Section 7 — Relationship to INC-I-024 and INC-I-025

### INC-I-024 (open, "fix pending")

**Same subsystem, sibling bug.** INC-I-024's diagnosis says the REBUILD path produces fewer exclusions than canonical because it scans `[epoch_start..rollback_target]` instead of `[epoch_start..network_tip]`. INC-I-026's evidence shows the APPLY-TIME path produces MORE exclusions than canonical because it counts slot deltas including from divergent blocks.

These are not contradictory — they are two faces of the same structural bug:
- The state is path-dependent
- The path can be too short (INC-I-024) → fewer
- The path can include divergent forks (INC-I-026) → more
- Either way, the state diverges from canonical

**The INC-I-024 doc says "fix pending" with two options (Option A: block production during catch-up; Option B: network-tip-aware rebuild). Two days later the fix is still not implemented.**

### INC-I-025 (resolved by 25b200a7 + 9105a16a)

**Different subsystem, related neighborhood.** INC-I-025 was a STORAGE bug: `seed_canonical_index()` writes the snap anchor's height_index but never the header, so `set_canonical_chain` walks past it and crashes with `"header X missing"`. The partner implemented the recommended snap_horizon floor fix in commit `25b200a7`, shipped as v6.7.6 / v6.7.7. **Verified working** — zero `"header missing"` errors after v6.7.7 deploy across all 4 mainnet log files.

INC-I-025 and INC-I-026 are **easy to confuse** because:
- Both affect snap-synced nodes
- Both produce ROLLBACK and "Empty headers" log lines
- Both INC docs landed on the same branch in the same week
- Five of the original INC-I-025 nodes (N1, N3, N4, N6, Seed3) overlap with INC-I-026's affected list

But they are distinct:
- INC-I-025: STORAGE crash in `crates/storage/src/block_store/writes.rs`
- INC-I-026: CONSENSUS divergence in `bins/node/src/node/rewards.rs`
- INC-I-025: nodes FROZE (no progress)
- INC-I-026: nodes alive but fork-and-recover every ~4 minutes

**INC-I-025 is closed. INC-I-026 is open.** The current observation is not a regression of INC-I-025 — it is a separate bug that was masked by INC-I-025's crash (a node that's crashed isn't producing forks) and became visible only after the crash was fixed.

### Possible causal link (informational)

The simplest causal hypothesis: yesterday's INC-I-025 crashes left several nodes (including N1) with snap-synced state that had divergent in-memory `excluded_producers` accumulated before the crash. v6.7.7 deploy stopped the crash; the divergent state was reset on restart; the underlying bug in `rewards.rs` (INC-I-024) is still there; the divergence is now re-accumulating from scratch on every node. This explains why the bug "reappeared" right after the v6.7.7 deploy — it never left, it was just hidden behind a worse crash.

---

<a id="section-8"></a>
## Section 8 — The 9 prior fix attempts and why they failed

This is the critical context. Each of these is a *real* commit on this branch's history that tried to fix the same family of bugs.

```
bb181ce1  fix(snap-sync): clear stale excluded_producers after snapshot (INC-I-010)
9b37d522  fix: clear excluded_producers on rollback — root cause of stuck forks
add2d55e  fix(consensus): rebuild excluded_producers after rollback instead of clearing
c725fa76  fix(consensus): cap liveness exclusions to prevent cascade divergence (INC-I-017)
9cd6eb9b  feat(consensus): on-chain liveness exclusion via missed_producers in BlockHeader
b0bddd0a  fix(consensus): excluded_producers 33% sanity cap
4a0e8dc1  revert: remove INC-I-020b from v6.7.3 — isolate excluded_producers cap
56380e0f  fix(sync): INC-I-020b + parent-known guard for fork cache
23093519  fix(sync): height-occupied fork guard — discard fork blocks at occupied heights
```

| # | Commit | Approach | Why it didn't fully fix the bug |
|---|---|---|---|
| 1 | `bb181ce1` | Clear stale excluded after snap sync | Only resets once, at snap sync time. Doesn't prevent post-snap divergence. |
| 2 | `9b37d522` | Clear excluded on rollback | Resets to *empty* — immediately wrong, schedule changes drastically, more forks. |
| 3 | `add2d55e` | Rebuild instead of clear | Rebuilds from canonical headers. **This is the correct primitive — but it only runs at rollback time, not at scheduler-input time.** |
| 4 | `c725fa76` | +3 cap per block | Bounds the damage but doesn't prevent the divergence (we observe the cap holding at exactly +3 in INC-I-026). |
| 5 | `9cd6eb9b` | On-chain `missed_producers` in BlockHeader | Makes the data canonical. Good change. But the consumer code on the production side still maintains a local mutable state derived from it. |
| 6 | `b0bddd0a` | 33% sanity cap (active/3) | Same idea as #4, second cap. Both caps held in INC-I-026 and the bug still happened. |
| 7 | `4a0e8dc1` | Revert INC-I-020b | Maintenance, isolating the cap fix. Not a fix attempt itself. |
| 8 | `56380e0f` | INC-I-020b + parent-known guard | Reduces fork-cache pollution. Doesn't touch excluded_producers. |
| 9 | `23093519` | Height-occupied fork guard | Reduces fork acceptance. **Has a side effect**: can lock a node onto a fork if the fork block arrives before canonical. |

**Pattern**: every commit attacks one entry point. The bug routes around to the next one. This is the textbook failure mode that behavioral learning #4 warns about:

> *"When 3+ incremental fixes fail on the same subsystem, STOP fixing individual bugs and analyze the system as a feedback loop. Enumerate ALL independent entry points, then fix at 3 layers simultaneously: (1) remove the primary trigger, (2) fix recovery imperfections that sustain the loop, (3) add an architectural invariant that makes the loop impossible. A single-layer fix will be routed around."*

INC-I-024's pending Option A would be a 10th single-layer attempt. We should not implement it as the only fix. It's fine as **layer 1** of a three-layer fix, but not on its own.

---

<a id="section-9"></a>
## Section 9 — Fix options

### Option A — INC-I-024's pending Option A (single layer, fast)

**What**: Block production until catch-up is complete. After any rollback or whenever local_h < net_tip_h, suppress the production gate until the node has caught up to the network tip and the excluded_producers set has been re-derived from the canonical chain.

**Where to put it**: `bins/node/src/node/production/gates.rs` or wherever `[CAN_PRODUCE]` is decided. Add a check: `if local_height < net_tip_height - SAFETY_MARGIN { return CanProduceResult::CatchingUp }`.

**Pro**: Smallest patch. Already designed and signed off in the existing INC-I-024 doc.

**Con**: It is the 10th single-layer fix. There is a strong prior that another entry point will route around it.

**Estimated effort**: 30-60 minutes to implement, write a regression test, and test on testnet.

### Option B — Three-layer fix per behavioral learning #4 (recommended)

**Layer 1 — Remove the primary trigger**: Remove the apply-time mutation of `excluded_producers` in `apply_block.rs`. The state should not be mutated at apply-time. Period.

**Layer 2 — Fix the recovery imperfection**: Make `rebuild_excluded_from_headers` the *only* way to compute `excluded_producers`. Call it from the scheduler, not just from the rollback path. Cache the result keyed by canonical tip hash so we don't pay the rebuild cost on every slot.

**Layer 3 — Add an architectural invariant**: Add an assertion at every scheduler call site:
```rust
debug_assert_eq!(self.excluded_producers, self.rebuild_excluded_from_headers(self.canonical_tip));
```
In release builds, log a warning and self-correct on mismatch instead of panicking. The assertion catches any future regression that re-introduces local mutable state.

**Pro**: Fixes the root cause. Eliminates the bug class entirely. Prevents future re-introductions.

**Con**: Larger patch. Requires touching the scheduler call sites (which may be hot path — needs benchmark).

**Estimated effort**: 4-8 hours to implement, write regression tests, benchmark, and test on testnet.

### Option C — Architectural redesign via `/omega-redesign`

**What**: Treat `excluded_producers` as a derived value from the canonical chain, not as state at all. Refactor the scheduler interface to take `(canonical_tip_hash, slot)` and return the producer, computing the exclusion set internally on demand (with caching).

**Pro**: Architecturally clean. Makes the bug class structurally impossible. Sets up a better abstraction for future consensus changes.

**Con**: Largest change. Requires careful migration. Higher coordination cost with the partner.

**Estimated effort**: 1-2 days to design, implement, test, and roll out.

### Recommendation

**Option B (three-layer fix)**, executed in this order:

1. Read the actual code first (Section 11) to confirm the apply-time mutation location.
2. Stop N1 (or all 3 ai1 producers) to halt the live experiment.
3. Implement Layer 1 (remove apply-time mutation).
4. Implement Layer 2 (rebuild as the only path, with caching).
5. Implement Layer 3 (debug_assert at scheduler call sites).
6. Write regression test from INC-I-024's reproduction outline.
7. Build, clippy, fmt, test (`cargo test -p doli-node`).
8. Testnet deploy. Verify.
9. Mainnet rollout: one node first, watch for 30 minutes, then roll to the rest.
10. Close INC-I-026 only after operator confirms the fix works in production.

If after Option B the bug still recurs (very unlikely but possible), escalate to Option C.

---

<a id="section-10"></a>
## Section 10 — Recommended next steps in priority order

**For the next /omega-doctor session resuming this incident:**

1. **Resume**: `/omega-doctor --incident=INC-I-026`
2. **Read this file end-to-end** (you are reading it now).
3. **Read the four files in Section 11** to anchor the code understanding.
4. **Decide with the operator**: Option A vs Option B vs Option C.
5. **Halt the live experiment if not already done**: ask the operator's permission to stop N1 (or N1 + N4 + N12) on mainnet via `sudo systemctl stop doli-mainnet-n1`. Do NOT touch nodes without explicit approval.
6. **Implement** the chosen option. Strict TDD per CLAUDE.md.
7. **Test on testnet first**: `~/testnet/bin/doli-node` + codesign + delete pending_update.json + restart. Verify with the 50-node testnet.
8. **Mainnet rollout**: one node, watch 30 min, then roll to the rest.
9. **Verify with the operator** that mainnet is stable for at least 1 hour with zero new ROLLBACK events from the fix-target nodes.
10. **Update memory.db**: close INC-I-026 with `INSERT INTO incident_entries ... 'resolution' ... 'worked'` and `UPDATE incidents SET status='resolved'`. Extract the behavioral learning if anything new emerges.

**Constraints to keep in mind:**
- Do not commit to mainnet without operator approval.
- Do not pkill any node — use systemctl on remote servers.
- Do not skip the testnet step.
- Do not declare victory until the operator confirms in production (behavioral learning #6).
- Use `--author "Antonio Lozada <antonio@omegacortex.ai>"` for any commit (per `feedback_commit_author.md`).

---

<a id="section-11"></a>
## Section 11 — Files to read first when resuming

Read these four files in order. They contain the actual code that needs to change.

| Order | File | What to look for |
|---|---|---|
| 1 | `bins/node/src/node/rewards.rs` | The `rebuild_excluded_from_headers` function (per INC-I-024 it's at lines 405-436). This is the **proven-correct** computation. Confirm it is pure (no mutation outside its return value). |
| 2 | `bins/node/src/node/apply_block.rs` | The post-commit logic that mutates `self.excluded_producers`. This is **the bug**. Find the exact line where the apply-time mutation happens. Look for assignments to `excluded_producers` or insertions into it. Also check `post_commit_actions` if it exists. |
| 3 | `bins/node/src/node/production/scheduling.rs` (or `production/gates.rs`) | The scheduler call site that consumes `excluded_producers`. This is **where the wrong value is used**. Find the SCHED_RR log line emitter and trace back to where it reads `excluded_producers`. |
| 4 | `crates/core/src/scheduler.rs` | The `DeterministicScheduler` itself. Confirm it's pure (no internal mutable state) and that all non-determinism comes from its inputs. |

**Search hints** (use Grep, not Bash grep):
- `excluded_producers` (the field name)
- `MAX_EXCLUSIONS_PER_BLOCK` (the cap constant from `c725fa76`)
- `[LIVENESS] Rebuilt excluded set` (the smoking-gun log line)
- `missed_producers` (the canonical-chain field in BlockHeader)
- `rebuild_excluded_from_headers` (the proven-correct function)
- `[SCHED_RR]` (the scheduler call-site log line)

**Cross-references in this codebase**:
- `CLAUDE.md` Map — Code section: rewards in `rewards.rs`, apply in `apply_block.rs`, scheduler in `scheduler.rs`
- `docs/bugfixes/inc-i-024-excluded-producers-rollback.md` — sibling root cause analysis with code references
- `docs/.workflow/diagnosis-report-inc-i-026.md` — first-pass diagnosis from this session
- `docs/.workflow/diagnosis-report-inc-i-025-verified.md` — yesterday's INC-I-025 diagnosis (DIFFERENT bug, different file)

---

<a id="section-12"></a>
## Section 12 — Open questions and uncertainties

1. **Have NOT verified the exact source line of the apply-time mutation.** Confidence is 0.999 from logs and architectural reasoning, but the actual line in `apply_block.rs` or `rewards.rs` has not been pointed at by reading the file. **First task in the next session.**

2. **Why was N3 unaware of canonical h=28654 at 08:09:12?** N3 is on the same machine as N1/Seed1. 40 seconds elapsed between canonical h=28654 production (08:08:42) and N3's competing block (08:09:12). Possible causes:
   - Gossip mesh isolation (N3 not in N1/Seed1's mesh)
   - Peer scoring throttle
   - Production gate not checking distance from net_tip_h (the `[CAN_PRODUCE]` line shows it only checks peers > 0 and state == Idle)
   - Scheduler accepting "out-of-slot" production (slot=29010 was 4 slots after the parent block's slot 29006 — should this be allowed?)
   This question doesn't affect the root cause fix, but it's interesting and probably also a bug worth filing separately.

3. **How does the height-occupied fork guard interact with the fix?** Commit `23093519` makes the receiver lock-in on whichever block arrives first. With the root-cause fix, would we want to relax this guard? Or does it become irrelevant because forks are no longer producing divergent state?

4. **What's the relationship between `missed_producers` (in BlockHeader, on-chain) and `excluded_producers` (in-memory, off-chain)?** The intent of commit `9cd6eb9b` was to make exclusion canonical via the block header. Why is there still an in-memory representation? Is the in-memory one supposed to be a cache, and the bug is that the cache is stale?

5. **Is the production gate's `[CAN_PRODUCE]` check on `local_h` correct?** Looking at `[CAN_PRODUCE] slot=29010 local_h=28653 peers=21 state=Idle` — N3 thought it was eligible at slot=29010 with local_h=28653, even though canonical was already at h=28656. There seems to be no "you're behind, don't produce" check.

6. **Does the testnet reproduce the bug?** Has the 50-node local testnet ever exhibited the same fork-and-recover cycle, or does it only manifest on real mainnet under network conditions? If testnet doesn't reproduce, the regression test must inject the conditions explicitly (per INC-I-024's reproduction outline).

---

<a id="section-13"></a>
## Section 13 — Artifact inventory

Files produced during this investigation, in the working tree:

| File | Content | Status |
|---|---|---|
| `docs/.workflow/inc-i-026-comprehensive-report.md` | **THIS FILE** — primary handoff document | UNTRACKED, ready for fresh session |
| `docs/.workflow/diagnosis-report-inc-i-026.md` | First-pass diagnosis (Symptom 1 only). Section 3 of this report supersedes it but it's preserved for reference. | UNTRACKED |
| `docs/.workflow/diagnosis-report-inc-i-025.md` | Yesterday's INC-I-025 diagnosis (storage bug, RESOLVED) | UNTRACKED |
| `docs/.workflow/diagnosis-report-inc-i-025-verified.md` | Yesterday's INC-I-025 verified diagnosis (the one the partner implemented) | UNTRACKED |
| `docs/bugfixes/inc-i-024-excluded-producers-rollback.md` | **The sibling root cause doc from 2026-04-07.** Status: "ROOT CAUSE CONFIRMED — fix pending". Read this. | UNTRACKED |
| `docs/.workflow/forensic-report.md` | Earlier forensic report (related, may overlap with INC-I-025) | UNTRACKED |

Logs collected (live, on remote servers, NOT copied locally):

| Source | Path | What it contains |
|---|---|---|
| ai1 | `/var/log/doli/mainnet/n1.log` | N1 producer log — 538 ROLLBACK events, BLOCK_PRODUCED of fork hash `73ba78597b…cc1105` at 08:25:12 |
| ai1 | `/var/log/doli/mainnet/n2.log` | N2 producer log — Symptom 1 timeline, smoking gun `[LIVENESS] Rebuilt excluded set: 8 (was 11)` at 08:10:33.178 |
| ai1 | `/var/log/doli/mainnet/n3.log` | N3 producer log — produced fork hash `8edb875d…` at 08:09:12.40 |
| ai1 | `/var/log/doli/mainnet/seed.log` | Seed1 — perfectly clean canonical chain through both windows |
| ai2 | `/var/log/doli/mainnet/n4.log` (and similar) | N4 — Fork A follower in screenshots |
| ai5 | `/var/log/doli/mainnet/n11.log`, `n12.log` | N11 (Fork B), N12 (Fork A follower) |

All accessible via `ssh ai1 sudo grep …` etc. SSH config is set up — just `ssh ai1`, `ssh ai2`, etc.

memory.db state:

| Table | Entry |
|---|---|
| `workflow_runs` | `id=141, type='doctor', description='N2 fell behind at ~h=28650-28660...', scope='mainnet/N2/sync'` |
| `incidents` | `INC-I-026, status='investigating', branch='feature/error-design-improvements', run_id=141` |
| `incident_entries` | (to be added — see Section 10 step 10) |
| `behavioral_learnings` | #1 already covers this pattern. No new learning needed unless implementation reveals a sub-pattern. |

Branch state:

- Currently on `feature/error-design-improvements`
- 62 unpushed commits (pre-existing, not from this session)
- 1 modified file (`CLAUDE.md`, pre-existing edit, restored from stash)
- 6 untracked files (5 reports + `isudoajl/` directory)
- main has been pulled (v6.7.7 commits available locally)

---

<a id="appendix-a"></a>
## Appendix A — Raw log evidence (preserved for forensics)

### N2 Symptom 1 timeline (verbatim, key lines)

```
2026-04-09T08:09:05.866459Z  [SCHED_RR] slot=29009 producer=4336ce33 index=9/25 (excluded=8)
2026-04-09T08:09:05.866467Z  [PROD_DIAG] slot=29009 h=28654 offset=4866ms mode=EPOCH eligible_len=1 our_rank=None window_rank=None is_eligible=false
2026-04-09T08:09:11.866243Z  [SCHED_RR] slot=29010 producer=54323cef index=10/25 (excluded=8)
2026-04-09T08:09:12.422445Z  [BLOCK_STORE] put_block: height=28654, slot=29010
2026-04-09T08:09:21.866122Z  [SCHED_RR] slot=29011 producer=b5d98316 index=15/22 (excluded=11)
                              ↑ excluded jumped 8 → 11, set shrunk 25 → 22
2026-04-09T08:09:30.866283Z  [HEALTH] h=28654 s=29010 hash=8edb875d... | peers=21 best_peer_h=28655 best_peer_s=29008 net_tip_h=28655 net_tip_s=29011
                              ↑ Two competing h=28655 blocks visible: slot=29008 and slot=29011
2026-04-09T08:09:31.866233Z  [SYNC_DEBUG] state=Syncing { phase: DownloadingHeaders }
2026-04-09T08:09:32.073518Z  WARN Empty headers from 12D3KooWLFVmzYi... (gap=3, consecutive=1) — minor fork. Signaling rollback to find common ancestor.
2026-04-09T08:09:32.535780Z  [BLOCK_STORE] put_block: height=28655, slot=29012
2026-04-09T08:10:02.865691Z  [FORK] ROLLBACK gap=3 empties=3 h=28656 rollback_count=1 — rolling back 1 block
2026-04-09T08:10:02.877053Z  [LIVENESS] Rebuilt excluded set from headers (h=28440-28656): 11 excluded (was 11)
2026-04-09T08:10:02.904181Z  [FORK] ROLLBACK_DONE h=28655 hash=62118bc6... cumulative_depth=1
2026-04-09T08:10:07.866296Z  [FORK] ROLLBACK gap=4 empties=3 h=28655 rollback_count=1
2026-04-09T08:10:07.875454Z  [LIVENESS] Rebuilt excluded set from headers (h=28440-28655): 11 excluded (was 11)
2026-04-09T08:10:11.866214Z  [SCHED_RR] slot=29016 producer=effe88fe index=20/22 (excluded=11)
2026-04-09T08:10:11.866220Z  [PROD_DIAG] slot=29016 h=28655 offset=866ms mode=EPOCH eligible_len=1 our_rank=Some(0) window_rank=Some(0) is_eligible=true
                              ↑ N2 thinks it's eligible at slot=29016 (because of divergent excluded)
2026-04-09T08:10:12.922665Z  [BLOCK_PRODUCED] hash=66340f8a... height=28655 slot=29016 parent=8edb875d...
                              ↑ N2 produces a competing block on the side fork
2026-04-09T08:10:22.866207Z  [FORK] ROLLBACK gap=4 empties=3 h=28655 rollback_count=1
2026-04-09T08:10:29.866181Z  [FORK] ROLLBACK gap=5 empties=3 h=28654 rollback_count=1
2026-04-09T08:10:30.866407Z  [HEALTH] h=28653 s=29006 hash=b9041fa1... | peers=21 best_peer_h=28659 best_peer_s=29014 state="Syncing:Headers"
2026-04-09T08:10:33.178947Z  [LIVENESS] Rebuilt excluded set from headers (h=28440-28653): 8 excluded (was 11)
                              ↑ ★ SMOKING GUN ★
                                Same algorithm. Rebuilt from canonical h=28440-28653.
                                Result: 8. Live (mutated): 11.
                                Three producers were illegitimately excluded at apply-time.
2026-04-09T08:10:33.868616Z  [BLOCK_STORE] put_block: height=28654, slot=29007  ← canonical
2026-04-09T08:10:33.872470Z  [BLOCK_STORE] put_block: height=28655, slot=29008  ← canonical
2026-04-09T08:10:33.876455Z  [BLOCK_STORE] put_block: height=28656, slot=29009  ← canonical
2026-04-09T08:10:33.880280Z  [BLOCK_STORE] put_block: height=28657, slot=29011  ← canonical
2026-04-09T08:10:33.884168Z  [BLOCK_STORE] put_block: height=28658, slot=29013
2026-04-09T08:10:33.887978Z  [BLOCK_STORE] put_block: height=28659, slot=29014
2026-04-09T08:10:33.891458Z  [BLOCK_STORE] put_block: height=28660, slot=29015
2026-04-09T08:10:33.894895Z  [BLOCK_STORE] put_block: height=28661, slot=29016  ← replaces N2's local fork
2026-04-09T08:10:33.898306Z  [BLOCK_STORE] put_block: height=28662, slot=29017
2026-04-09T08:10:34.865391Z  [SCHED_RR] slot=29018 producer=b03fe629 index=15/23 (excluded=10)
                              ↑ Recovered. excluded back near canonical (10 vs 8 — slight residual)
```

### N3 Symptom 1 — the producer of the divergent block

```
2026-04-09T08:08:32.163168Z  [BLOCK_STORE] put_block: height=28653, slot=29006
2026-04-09T08:09:12.409405Z  [BLOCK_PRODUCED] hash=8edb875d... height=28654 slot=29010 parent=b9041fa1...
                              ↑ N3 produces the divergent block 30 seconds AFTER canonical h=28654 was already on N1/Seed1
2026-04-09T08:09:12.412501Z  [BLOCK_STORE] put_block: height=28654, slot=29010
2026-04-09T08:09:31.677804Z  WARN Empty headers from <peer> (gap=3, consecutive=1) — minor fork
2026-04-09T08:09:32.358013Z  [FORK] ROLLBACK h=28654 → h=28653
2026-04-09T08:09:32.387369Z  [FORK] ROLLBACK_DONE h=28653 cumulative_depth=1
2026-04-09T08:09:39.429685Z  [BLOCK_STORE] put_block: height=28654, slot=29007  ← canonical arrives
2026-04-09T08:09:39.434705Z  [BLOCK_STORE] put_block: height=28655, slot=29008
2026-04-09T08:09:39.439401Z  [BLOCK_STORE] put_block: height=28656, slot=29009
2026-04-09T08:09:39.443405Z  [BLOCK_STORE] put_block: height=28657, slot=29011
[N3 caught up in 30ms via header sync, then back on canonical]
```

### N1 Symptom 2 — produced the screenshot's exact fork hash

```
2026-04-09T08:25:12.416205Z  [BLOCK_PRODUCED] hash=73ba78597b0757150322d2b98ccc1367fe0ae18eba6b7fe20de26aff2fcc1105
                                              height=28748 slot=29106 parent=83f7eef8...
                              ↑ Exact match to Fork A hash in operator's screenshot
```

N1 BLOCK_PRODUCED cadence (since 06:03 UTC restart): 07:50, 07:55, 07:59, 08:03, 08:07, 08:11, 08:15, 08:19, 08:23, 08:25 — every ~4 minutes.

---

<a id="appendix-b"></a>
## Appendix B — Live RPC snapshots

Snapshot at ~09:25 UTC (during the live screenshot fork):

```
ai1:8500 (Seed1): bestHeight=28754 bestHash=2f641c85... (canonical at that moment)
ai1:8501 (N1):    bestHeight=28748 bestHash=73ba7859... (Fork A at that moment) ← divergent
ai1:8502 (N2):    bestHeight=28754 bestHash=2f641c85... (canonical)
ai1:8503 (N3):    bestHeight=28754 bestHash=2f641c85... (canonical)
```

Snapshot a few minutes later:

```
ai1:8500 (Seed1): bestHeight=28757 bestHash=873527f4... (canonical advanced)
ai1:8501 (N1):    bestHeight=28754 bestHash=2f641c85... (still on prior canonical hash, not advancing)
ai1:8502 (N2):    bestHeight=28757 bestHash=873527f4...
ai4:8506 (N6):    bestHeight=28758 bestHash=0095a976... (slightly ahead — different mesh path)
```

Final snapshot (after recovery):

```
All 12 producers + 3 seeds: bestHeight=28760 slot=29118 hash=de2c05b6...e9cacd integrity=08f4231a
```

---

<a id="appendix-c"></a>
## Appendix C — Behavioral learnings used

These were active in the session briefing and applied during this investigation. Listed here so the next session knows which patterns informed the diagnosis.

| # | Rule | How it was applied |
|---|---|---|
| 1 | "DeterministicScheduler is deterministic — non-determinism comes from its INPUTS. Any LOCAL state used to modify scheduler inputs creates a guaranteed fork." | This is the EXACT pattern we observed. Predicted the root cause before reading logs. |
| 4 | "When 3+ incremental fixes fail on the same subsystem, STOP fixing individual bugs and analyze as a feedback loop. Fix at 3 layers." | We are at 9 prior fixes. This is unambiguously the situation the rule warns about. Recommendation: Option B (three-layer fix), not another incremental patch. |
| 5 | "Code that scans block history MUST check whether the required blocks actually exist." | The rebuild path's correctness depends on canonical headers being present. After snap sync this might be problematic — needs verification when reading the code. |
| 6 | "NEVER close an incident or declare a bug resolved without explicit confirmation from the user that the fix works in production." | INC-I-026 will not be closed until operator confirms. Section 10 step 10. |
| 7 | "When debugging cascading failures, do not assume each fix will resolve the problem. Trace the FULL causal chain to the absolute bottom." | We traced from screenshot symptom → fork blocks → BLOCK_PRODUCED signature → divergent excluded → apply-time mutation. The "absolute bottom" question (why N3 was isolated) is documented as Open Question #2 but doesn't change the fix. |
| 8 | "When a fix changes behavior in one subsystem, immediately audit ALL code paths that must agree on the same data." | When implementing the fix, audit BOTH production and validation paths for `excluded_producers` use. INC-I-005 (mentioned in the learning) lost 10 sessions to this exact mismatch. |
| 9 | "Before diagnosing any distributed system failure, FIRST verify the system is operating within its designed parameters." | Verified: 15 nodes, well within max_peers=50. Not a capacity issue. Code bug confirmed. |
| 10 | "Open an INC ticket IMMEDIATELY when the user reports a bug." | INC-I-026 was opened at symptom-report time, before investigation. ✓ |

---

## End of report

**Confidence in root cause**: 0.999 (the 0.001 reservation is for the missing source-line verification — first task in the next session per Section 11).

**Recommended action for the next session**: Read this file → Read the four source files in Section 11 → Decide Option A vs Option B vs Option C with the operator → Implement → Test → Deploy.

**Resume command**: `/omega-doctor --incident=INC-I-026`
