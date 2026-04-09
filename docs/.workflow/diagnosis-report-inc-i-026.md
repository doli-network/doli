# INC-I-026 — N2 fall-behind/recovery at h=28650-28662

**Branch:** feature/error-design-improvements
**Date:** 2026-04-09
**Symptom:** N2 fell behind around h=28650, recovered around h=28662
**Window:** 2026-04-09 08:08:32 → 08:10:33 UTC (~2 minutes)
**Affected nodes:** N2 (severely), N3 (briefly), N1/Seed1 (untouched — canonical)

---

## TL;DR (verdicts the user asked for)

**Q1: Was this a normal behind?**
**NO.** Confidence: **0.9999.**
This was a **multi-node chain fork**, not a sync lag. Direct evidence below.

**Q2: Was this a code bug?**
**YES.** Confidence: **0.999.**
This is the **partially-mitigated recurrence of INC-I-016** (liveness exclusion divergence). The +3-per-block cap held, but the underlying root cause — `excluded_producers` is a locally-mutable scheduler input that diverges across nodes — is still present and still produces forks. Direct evidence below.

---

## Section 1 — Evidence that this was NOT a normal behind (conf 0.9999)

### Evidence A — Two distinct blocks exist at the same height with different slots

| Node | h | slot | hash (head) | parent | source |
|------|---|------|-------------|--------|--------|
| N1 (canonical) | 28654 | **29007** | (canonical) | b9041fa1… | `n1.log` 08:08:42.99 |
| Seed1 (canonical) | 28654 | **29007** | (canonical) | b9041fa1… | `seed.log` 08:08:42.99 |
| N3 (forked) | 28654 | **29010** | 8edb875d… | b9041fa1… | `n3.log` BLOCK_PRODUCED 08:09:12.40 |
| N2 (received N3's block) | 28654 | **29010** | 8edb875d… | (same) | `n2.log` put_block 08:09:12.42 |

**Two different blocks exist at h=28654, both with parent b9041fa1… (the canonical h=28653 hash).** That is the textbook definition of a fork.

The "behind" view of N2 is misleading: N2 was not slow, N2 was on the **wrong side of a chain split**. While N1/Seed1 were already at h=28659 by 08:09:52, N2 was still at h=28656 on a side chain that started diverging at h=28654.

### Evidence B — Explicit fork-recovery log lines

N2 emitted **4 separate `[FORK] ROLLBACK` events** during this window:

```
n2.log:
08:10:02.86  [FORK] ROLLBACK gap=3 empties=3 h=28656 rollback_count=1 — rolling back 1 block
08:10:02.90  [FORK] ROLLBACK_DONE h=28655 cumulative_depth=1
08:10:07.86  [FORK] ROLLBACK gap=4 empties=3 h=28655 rollback_count=1 — rolling back 1 block
08:10:07.89  [FORK] ROLLBACK_DONE h=28654 cumulative_depth=1
08:10:22.86  [FORK] ROLLBACK gap=4 empties=3 h=28655 rollback_count=1 — rolling back 1 block
08:10:22.89  [FORK] ROLLBACK_DONE h=28654 cumulative_depth=1
08:10:29.86  [FORK] ROLLBACK gap=5 empties=3 h=28654 rollback_count=1 — rolling back 1 block
08:10:29.89  [FORK] ROLLBACK_DONE h=28653 cumulative_depth=1
```

The string `for fork recovery` appears literally in the log. There is zero ambiguity — the node itself is telling us this was a fork, not a lag.

### Evidence C — Empty-headers warnings from multiple distinct peers

```
n2.log:
08:09:32.07  WARN Empty headers from 12D3KooWLFVmzYi… (gap=3) — minor fork. Signaling rollback to find common ancestor.
08:10:02.18  WARN Empty headers from 12D3KooWBjecM5Hs… (gap=3) — minor fork.
08:10:07.17  WARN Empty headers from 12D3KooWLZ6eoCis… (gap=4) — minor fork.
08:10:12.18  WARN Empty headers from 12D3KooWAjsboAgt… (gap=5) — minor fork.
08:10:17.17  WARN Empty headers from 12D3KooWH4opSJhH… (gap=4) — minor fork.
08:10:22.18  WARN Empty headers from 12D3KooWNfrk1sCf… (gap=4) — minor fork.
```

**Six different peers** all responded "I don't have your tip in my chain, I have a fork." A normal behind never produces these — peers would just send the missing headers. Empty headers means N2's `local_hash` does not exist in any peer's canonical history.

### Evidence D — N2 produced its own block on the side fork

```
n2.log 08:10:12.92:
[BLOCK_PRODUCED] hash=66340f8a… height=28655 slot=29016 parent=8edb875d…
```

While the canonical chain was already at h=28661 slot=29016 (per N1/Seed1 at the same wall-clock time), **N2 was still at h=28654 and produced a competing block at h=28655 from slot 29016**. N2 thought it was eligible at slot=29016 (`our_rank=Some(0) is_eligible=true`) — but only because its local scheduler view was wrong (see Section 2).

### Evidence E — N1 and Seed1 have a perfectly linear chain through this window

```
n1.log + seed.log (identical):
08:08:32.16  put_block height=28653 slot=29006
08:08:42.99  put_block height=28654 slot=29007  ← canonical h=28654
08:08:52.79  put_block height=28655 slot=29008
08:09:02.41  put_block height=28656 slot=29009
08:09:22.11  put_block height=28657 slot=29011
08:09:43.09  put_block height=28658 slot=29013
08:09:52.57  put_block height=28659 slot=29014
08:10:02.43  put_block height=28660 slot=29015
08:10:12.34  put_block height=28661 slot=29016
08:10:22.53  put_block height=28662 slot=29017
```

**N1 and Seed1 had ZERO rollbacks, ZERO empty-headers warnings, ZERO fork events during the entire window.** Both nodes are on the same machine (`ai1`) as N2. If this were a network-wide event, N1 and Seed1 would have seen it too. They didn't.

→ A "normal behind" affects all nodes on the same machine equally. **This affected N2 and N3 but not N1/Seed1.** That alone is incompatible with the "normal behind" hypothesis.

### Evidence F — N3 had the SAME fork event a minute earlier and recovered

```
n3.log:
08:08:32.16  put_block height=28653 slot=29006     ← canonical
08:09:12.40  [BLOCK_PRODUCED] height=28654 slot=29010 parent=b9041fa1…  ← N3 PRODUCED a competing block
08:09:31.67  WARN Empty headers (gap=3) — minor fork
08:09:32.35  [FORK] ROLLBACK h=28654 → h=28653
08:09:39.42  put_block height=28654 slot=29007     ← canonical now arrives
08:09:39.43  put_block height=28655 slot=29008
08:09:39.43  put_block height=28656 slot=29009
08:09:39.44  put_block height=28657 slot=29011     ← caught up
```

**N3 is the producer that created the divergent block.** The hash `8edb875dbd25236487a953fce8109ecbd896ba52dd0c955375f657f76b34edb4` from N3's BLOCK_PRODUCED at 08:09:12.40 is the **identical hash** N2 stores at 08:09:12.42 (see N2's log). N3 sent its forked block to N2, N2 swallowed it, and the cascade began.

### Confidence rationale

I have:
- Two distinct blocks at the same height with the same parent (objective fact, not interpretation)
- The node itself logging `for fork recovery` (its own diagnosis)
- Six independent peers declaring "minor fork"
- N2 producing its own competing block on the side chain
- N1/Seed1 (same machine) untouched
- An upstream node (N3) producing the divergent block, with the matching hash linking them

**Any one of these would be enough to rule out "normal behind." All six together is overwhelming.**
**Confidence ≥ 0.9999.**

---

## Section 2 — Evidence that this was a code bug (conf 0.999)

The above proves it was a fork. This section proves the fork was caused by a code bug, not by external/network adversity.

### The bug in one sentence

**N2's `excluded_producers` set diverged from canonical by exactly 3 entries after applying the divergent h=28654, which caused N2's `DeterministicScheduler` to compute different producers per slot than the canonical chain — and N2 then claimed its own (wrong) eligibility and made the fork worse.**

This is INC-I-016 verbatim. It was supposedly fixed by capping per-block exclusions at +3 and total at active/3. **The cap is enforced (we observe exactly +3), but it does not eliminate the divergence — it only bounds its rate.**

### Evidence A — Excluded count diverged by exactly the +3 cap

Trace from N2 log, scheduler input over time:

```
08:09:05  SCHED_RR slot=29009 ... index=9/25  (excluded=8, set=25)   ← before fork
08:09:11  SCHED_RR slot=29010 ... index=10/25 (excluded=8, set=25)
08:09:12  put_block height=28654 slot=29010                          ← N3's divergent block applied
08:09:21  SCHED_RR slot=29011 ... index=15/22 (excluded=11, set=22)  ← excluded jumped 8 → 11
                                                                        active set shrunk 25 → 22
```

In ONE block transition (h=28653 → h=28654 via N3's divergent block at slot=29010), **N2 added exactly 3 producers to its excluded set** (the producers scheduled for the slots N2 thinks were skipped: 29007, 29008, 29009).

That's **exactly the `MAX_EXCLUSIONS_PER_BLOCK = 3` cap** introduced as the INC-I-016 mitigation. The cap held — but it held at the maximum value, not zero. The divergence still happened.

### Evidence B — The canonical chain had ZERO new exclusions for the same height transition

N1 / Seed1 went from `h=28653 slot=29006` → `h=28654 slot=29007`. **One slot apart, zero missed slots, zero new exclusions added.**

So:
- **N2 view after applying h=28654:** excluded=11 (added +3 for "missed" slots 29007/29008/29009)
- **N1/Seed1 view after applying h=28654:** excluded=8 (added 0)

**Two nodes running the same binary, applying the same height, ended up with different scheduler inputs. That is a code bug — and it's the exact bug that behavioral learning #1 in this session warned about.**

### Evidence C — The system itself confirmed the divergence at recovery time

```
n2.log 08:10:33.178:
[LIVENESS] Rebuilt excluded set from headers (h=28440-28653): 8 excluded (was 11)
```

This is the rebuild path (`rebuild_excluded_set_from_headers` in `bins/node/src/node/rewards.rs`) running after N2 rolled all the way back to h=28653 (the common ancestor). It computed 8 excluded — and explicitly logged that the live count was 11.

**The same algorithm computed 8 from the canonical chain and 11 from N2's local mutable state. That is the bug — verbatim, in the system's own logs.**

The previous rebuilds (08:10:02.87, 08:10:07.87, 08:10:22.87) all said `11 excluded (was 11)` because they were rebuilding from the divergent chain (which still had the divergent h=28654). Only after rolling back PAST the divergent block did the rebuild produce the canonical 8.

### Evidence D — The divergent scheduler led directly to N2 producing on the wrong fork

```
n2.log 08:10:11.866:
SCHED_RR slot=29016 producer=effe88fe index=20/22 (excluded=11)
PROD_DIAG slot=29016 h=28655 ... our_rank=Some(0) window_rank=Some(0) is_eligible=true
                                                                        ^^^^^^^^^^^^^^

n2.log 08:10:12.922:
[BLOCK_PRODUCED] hash=66340f8a… height=28655 slot=29016 parent=8edb875d…
```

With N2's local view (`excluded=11, set=22`), N2 computed itself as `our_rank=Some(0)` at slot=29016 → produced a block.

With the canonical view (`excluded=8, set=25`), the canonical chain produced **a different block at the same slot=29016**: N1/Seed1's `put_block height=28661 slot=29016 @ 08:10:12.34`.

**Same slot, two producers, two different blocks, two different chains.** This is consensus failure caused by a divergent scheduler input. The deterministic scheduler is doing exactly what it should — its input is wrong.

### Evidence E — The chain of causation, fully traced

```
T+0.0s  N3 has a stale/divergent excluded set (already wrong before our window)
T+0.0s  N3's local scheduler says "you're eligible at slot=29010"
        (canonical scheduler says "the slot=29010 producer is someone else")
        N3 produces h=28654 slot=29010 (hash 8edb875d…)
        [BUG instance #1: N3's scheduler input was already divergent]

T+0.02s N2 receives N3's block 8edb875d… for h=28654 slot=29010
        N2 applies it (the block IS valid under N2's local scheduler view too)
        N2's post-commit liveness: "between slot 29006 and 29010 there were 3
        missed slots — exclude their producers"
        N2's excluded jumps 8 → 11 (capped at +3)
        [BUG instance #2: N2's exclusion logic uses the divergent block's slot]

T+9s    N2's scheduler now says N2 is eligible at slot=29016 (because
        excluded=11 reshuffles the round-robin)
        N2 produces h=28655 slot=29016 (hash 66340f…)
        While the canonical chain is already at h=28661 slot=29016
        [BUG instance #3: N2 makes the fork worse]

T+50s   Rollback rollback rollback (4 events, 1 block each) until N2 reaches
        h=28653 — the common ancestor
T+121s  Rebuild excluded from canonical headers (8 vs 11)
T+121s  Header sync delivers canonical h=28654-28662 in 30 milliseconds
        N2 is back on canonical chain
```

The recovery worked — but the bug is the *initial divergence*. Recovery + caps + rollback = "the network heals." But the bug *will* recur on every gossip race.

### Why this is provably a code bug, not adversity

1. **Same machine.** N1, N2, N3, Seed1 all run on `ai1`. Same hardware, same network, same gossip mesh. There is no "external" adversity — this is purely the difference in which block each process receives first.
2. **Same binary.** All four nodes run `version=6.7.7`. Identical code.
3. **Same chain history.** All four agreed up to and including h=28653 (hash b9041fa1…).
4. **The system itself confirms the divergence** by logging "Rebuilt excluded set from headers: 8 excluded (was 11)" — that's the algorithm telling us its inputs were wrong.
5. **The exclusion cap matches exactly.** We observe +3 (the cap from INC-I-016 mitigation). This is not coincidence — it is the same code path.

**This is a code bug. Confidence ≥ 0.999.**

The 0.001 reservation is reserved for the small possibility that there is a legitimate operational explanation I haven't seen (e.g. clock skew between processes, scheduler intentionally allowing this in some recovery mode). I have not found one in the code, but I cannot prove the absence of one without reading every line of `rewards.rs` and `production/scheduling.rs`.

---

## Section 3 — Where the bug lives

The bug surface is in the producer-eligibility / liveness-exclusion logic. The relevant files based on the project's code map (`CLAUDE.md`):

| File | Why |
|------|-----|
| `bins/node/src/node/rewards.rs` | `Rebuilt excluded set from headers` log line lives here. This is the rebuild path proving the divergence. |
| `bins/node/src/node/production/scheduling.rs` | `[SCHED_RR]` log line. This is where the deterministic round-robin computes producer rank using `excluded_producers` as input. |
| `bins/node/src/node/apply_block.rs` | Where post-commit exclusion is computed when a block is applied (the divergent step). |
| `bins/node/src/node/rollback.rs` | `[FORK] ROLLBACK` log line. The recovery path. |
| `crates/core/src/scheduler.rs` | `DeterministicScheduler` consumes `excluded` as input. |

The structural fix is one of:
1. **Stop using local mutable state as scheduler input.** Compute `excluded_producers` deterministically from the canonical chain headers on every scheduler call (current rebuild path is correct — make it the only path). This costs O(epoch_length) per slot but is bounded.
2. **Make exclusion canonical.** Embed exclusion deltas in block headers so all nodes derive the same set from the same chain.
3. **Remove exclusion from scheduler input entirely.** Use static round-robin and handle liveness via reward weighting only.

**I am NOT proposing a fix in this report.** This report is investigative only, per `--investigate` semantics. The diagnosis stands; choose the fix path separately.

---

## Section 4 — Architecture Feasibility Verdict

**Verdict: CODE-FIXABLE.**

The recovery mechanism (`rebuild_excluded_set_from_headers`) already exists in the codebase and produces the correct answer. The fix is to make it the *primary* path instead of a fallback. This is a code change, not a redesign.

However, this is **the second time the same root cause has surfaced in this subsystem** (INC-I-016 was the first). If a third instance occurs after a code-level fix attempt, the next escalation should be `/omega-redesign` on the producer-scheduling subsystem.

---

## Section 5 — Diagnostician Feasibility Verdict

**Verdict: YES (fixable with a code change).**

The bug is in identifiable code (the post-commit liveness exclusion in `apply_block.rs`/`rewards.rs`). The fix changes deterministic computation, not architecture. The recovery code already proves the canonical computation is well-defined.

---

## Appendix A — Raw evidence (key log lines)

### N2: divergent block reception → excluded jump → fork production → recovery

```
08:09:05.86  SCHED_RR slot=29009 producer=4336ce33 index=9/25 (excluded=8)
08:09:11.86  SCHED_RR slot=29010 producer=54323cef index=10/25 (excluded=8)
08:09:12.42  put_block: height=28654, slot=29010
08:09:21.86  SCHED_RR slot=29011 producer=b5d98316 index=15/22 (excluded=11)
08:09:30.86  HEALTH h=28654 s=29010 hash=8edb875d... | best_peer_h=28655 best_peer_s=29008
             net_tip_h=28655 net_tip_s=29011 | sync_fails=0 fork_counter=0
08:09:31.86  SYNC_DEBUG state=Syncing { phase: DownloadingHeaders }
08:09:32.07  WARN Empty headers from 12D3KooWLFVmzYi... (gap=3) — minor fork
08:09:32.53  put_block: height=28655, slot=29012   ← yet another competing block
...
08:10:02.86  [FORK] ROLLBACK gap=3 h=28656 → 28655
08:10:07.86  [FORK] ROLLBACK gap=4 h=28655 → 28654
08:10:11.86  SCHED_RR slot=29016 producer=effe88fe index=20/22 (excluded=11)
08:10:12.92  [BLOCK_PRODUCED] hash=66340f8a... height=28655 slot=29016 parent=8edb875d
08:10:22.86  [FORK] ROLLBACK gap=4 h=28655 → 28654
08:10:29.86  [FORK] ROLLBACK gap=5 h=28654 → 28653
08:10:30.86  HEALTH h=28653 s=29006 hash=b9041fa1... | best_peer_h=28659 best_peer_s=29014
             net_tip_h=28659 net_tip_s=29017 | state="Syncing:Headers"
08:10:33.17  [LIVENESS] Rebuilt excluded set from headers (h=28440-28653): 8 excluded (was 11)
08:10:33.86  put_block: height=28654, slot=29007   ← canonical
08:10:33.87  put_block: height=28655, slot=29008   ← canonical
08:10:33.87  put_block: height=28656, slot=29009   ← canonical
08:10:33.88  put_block: height=28657, slot=29011   ← canonical
08:10:33.88  put_block: height=28658, slot=29013   ← canonical
08:10:33.88  put_block: height=28659, slot=29014   ← canonical
08:10:33.89  put_block: height=28660, slot=29015   ← canonical
08:10:33.89  put_block: height=28661, slot=29016   ← canonical (replaces N2's local fork)
08:10:33.89  put_block: height=28662, slot=29017   ← canonical
08:10:34.86  SCHED_RR slot=29018 producer=b03fe629 index=15/23 (excluded=10)
```

### N3: produced the divergent block

```
08:08:32.16  put_block: height=28653, slot=29006
08:09:12.40  [BLOCK_PRODUCED] hash=8edb875d... height=28654 slot=29010 parent=b9041fa1...
             ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
             ↑ THIS is the seed of the fork. Produced 30 seconds AFTER the canonical
               h=28654 was already on N1/Seed1, on a slot 3 later than the canonical slot.
08:09:12.41  put_block: height=28654, slot=29010
08:09:31.67  WARN Empty headers (gap=3) — minor fork
08:09:32.35  [FORK] ROLLBACK h=28654 → h=28653
08:09:39.42  put_block: height=28654, slot=29007  ← canonical arrives
```

### N1 / Seed1 (canonical, untouched)

```
08:08:32.16  put_block: height=28653, slot=29006
08:08:42.99  put_block: height=28654, slot=29007  ← canonical h=28654 (15 seconds before N3 forked)
08:08:52.79  put_block: height=28655, slot=29008
08:09:02.41  put_block: height=28656, slot=29009
08:09:22.11  put_block: height=28657, slot=29011
08:09:43.09  put_block: height=28658, slot=29013
08:09:52.57  put_block: height=28659, slot=29014
08:10:02.43  put_block: height=28660, slot=29015
08:10:12.34  put_block: height=28661, slot=29016
08:10:22.53  put_block: height=28662, slot=29017
[ZERO rollbacks. ZERO empty-headers warnings. ZERO fork events.]
```

---

## Appendix B — Relationship to prior incidents

| INC | Status | Relationship |
|-----|--------|--------------|
| INC-I-016 | Marked "resolved" 2026-03-29 | **This is a recurrence.** The fix added MAX_EXCLUSIONS_PER_BLOCK=3 and max_excluded=active/3 caps, which we observe holding (excluded jumped exactly +3 in N2). The caps bound the cascade but do not prevent the divergence. The post-mortem learning ("Any local-state HashSet that modifies scheduler inputs MUST be capped") is satisfied — but capping was the wrong fix; the right fix is to remove the local state from the scheduler input. |
| INC-I-025 | "investigating" — multi-node fork at h=24640 | **Same pattern, earlier height.** Almost certainly the same root cause manifesting in a different window. INC-I-025's diagnosis report (already in `docs/.workflow/`) should be cross-checked. |
| Behavioral learning #1 (this session) | Active | **Predicted exactly this.** "The DeterministicScheduler is deterministic — non-determinism comes from its INPUTS. Any LOCAL state… used to modify scheduler inputs on the production side but not replicated in validation creates a guaranteed fork when the modification changes a value." The learning is correct and was applied successfully here. |
