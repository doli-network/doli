# Domain Investigation Report: Fork / Divergence

**INC**: INC-I-146 (proposed) · **RUN_ID**: 473 · **Agent**: blockchain-domain-investigator-fork
**Date**: 2026-07-30 · **Mode**: read-only mainnet diagnostics

---

## Domain Lens

Chain splits, reorgs, competing/sibling blocks, state-root mismatches, consensus disagreement, block
rejection, nodes on different tips, orphan/sibling **retention** (memory), rollback/reorg churn.
Memory is the evidence; fork-adjacent retention is the target.

---

## Chain Context

| Item | Measured value | Source |
|---|---|---|
| Chain | DOLI mainnet, genesis `a91f6ba2aacc45c3…7cc1c8737338` | `getChainInfo` on 13 nodes |
| Genesis time | `1784717100` = **2026-07-22 10:45:00 UTC** | n6.log:21 `Genesis time from chainspec` |
| Binary | v6.24.0 on **all 13** reachable structural nodes — no version skew | `getChainInfo.version` |
| Tip at investigation | h=72531 `fc44d5b4…` (ai1–ai4) / h=72532 `78f5e965…` (ai5) | RPC sweep 21:1xZ |
| Producer set | **12 active** (5 genesis @ weight 100, **7 registered today** @ h=70991–70999) | `getProducers` seed1 |
| Epoch length | 360 blocks / 1 h | `[EPOCH]` cadence 17:45→18:45→19:45 |
| Typical block | **489–490 bytes**, 1 tx, 120→128 attestations | `getBlockByHeight` h=70000/72500 |

---

## What I Don't Understand

1. **Why ai4/ai5 host memory is still ramping** (+70 MB/h and +132 MB/h respectively, 4.5 h after the
   step) while the doli process RSS on those hosts is *flat* between two snapshots ~1 h apart
   (n7 387→383 MB, n6 371→370 MB, n8 351→349 MB). The growth is **outside** the node processes'
   RSS. I could not attribute it. This is the single largest gap in my picture and belongs to the
   memory/resource lens.
2. **Why per-node RSS on ai4/ai5 (349–387 MB) is ~2× that on ai1/ai2 (140–205 MB)** at comparable or
   *shorter* uptime (ai1 n1 restarted 15:30 → 158 MB; ai4 n7 restarted 16:31 → 383 MB). Same binary,
   same chain, same tip. Not explainable from fork state (both groups have zero fork events).
3. **Why all `doli_*` Prometheus gauges read 0** on every node I sampled (`doli_chain_height 0`,
   `doli_peers_connected 0`, `doli_storage_bytes 0`, `doli_utxo_set_size 0`) while the nodes are
   demonstrably at h=72531 with live peers. The exporter is not populating them.
4. Who the other 6 new producers are (only n6's key `d13ae338…` is identifiable from its log). The
   brief says "N6 and N12"; the chain says **seven** registrations in ~90 s.
5. Why ai4+ai5 (7 nodes across 2 hosts) were restarted simultaneously at **16:31:12–16:31:13**.

---

## Domain Relevance Assessment

**Relevance: MINIMAL — the fork domain is RULED OUT as the cause.**

Reasoning, in order of decisiveness:

1. **The fleet is converged.** Not "approximately" — byte-identical hashes at three independently
   chosen heights across independently-synced nodes (§Key Evidence E1).
2. **Zero fork events have ever occurred on this chain.** Not "few" — literally zero across the
   entire 8-day post-genesis life, on both a producer that has run since genesis and a seed
   (§E2). The INC-I-143 F2 wedge-escape retention path — the specific mechanism this lens was
   dispatched to test — **has never executed once on mainnet**.
3. **Every fork-retention structure is bounded and small.** Worst-case total across all four is
   **< 11 MB**, i.e. **≤ 1.0 %** of the 1,091 MB step measured on ai4 (§E3, derivations shown).
4. **The producer-count hypothesis fails its own timing test.** The memory step is at 15:30Z; the
   producer set does not actually change until the epoch-199 freeze at 17:45:31Z; nothing moves at
   17:45 (§E5). Two hours fifteen minutes of separation.

A fork-domain cause would require *some* fork event, *somewhere*, in *some* log. There are none.

---

## Hypotheses

### H1: INC-I-143 F2 sibling retention (`wedge_escape.rs`) accumulates blocks in memory — conf(0.02, measured) — **DEAD**
- **Kill test**: find any `[WEDGE_ESCAPE]` or `[FORK_GUARD]` line in any node log post-genesis; or
  show `wedge_retained_tips` / `fork_block_cache` can hold >100 MB.
- **Kill test result**: **DISPROVING EVIDENCE FOUND — hypothesis is dead.**
  `WEDGE_ESCAPE` = **0** and `FORK_GUARD` = **0** in `/var/log/doli/mainnet/n7.log` on ai4
  (2026-07-22 → 2026-07-30, entire chain life) and in `/var/log/doli/mainnet/seed.log` on ai1
  (2026-07-29 → now). The retention path is cold. Independently, both caps are trivially small:
  `WEDGE_RETAINED_CAP = 64` hashes = **2 KB** (`bins/node/src/node/wedge_escape.rs:44`,
  insert+clear at `:112-115`), `fork_block_cache` ≤ 100 blocks × 490 B = **49 KB**
  (`bins/node/src/node/block_handling.rs:102-113`).
- **Evidence**: also `bins/node/src/node/periodic.rs:322-336` TTL-evicts cache entries older than
  30 slots (~5 min), a second independent bound.

### H2: More producers ⇒ more same-height siblings ⇒ more retained blocks — conf(0.03, measured) — **DEAD**
- **Kill test**: show competing blocks at the same height after the producer set grew; or show the
  scheduler admits >1 leader per slot.
- **Kill test result**: **DISPROVING EVIDENCE FOUND — hypothesis is dead.** Zero `FORK_GUARD`
  (the *only* code path that fires on a same-height sibling,
  `bins/node/src/node/block_handling.rs:168-207`) after the set grew 5→12. Zero `Fork detected`,
  zero `Reorg`, zero `fork recovery` (`grep -iE 'reorg|sibling|competing|diverg|equivocat|fork
  recovery|Fork detected'` on seed.log → **no matches**).
- **Note**: the producer set is a *frozen epoch list* with a deterministic slot leader
  (`[EPOCH] Frozen producer list for epoch 199: 12 producers`); more producers changes *who*
  produces slot N, not *how many* produce slot N.

### H3: The epoch-boundary producer-set mutation (5→12) forced an EpochState rebuild / reorg — conf(0.05, measured) — **DEAD**
- **Kill test**: find a rebuild, rollback, reorg, or divergence at the epoch-199 boundary; or a
  memory inflection at that boundary.
- **Kill test result**: **DISPROVING EVIDENCE FOUND — hypothesis is dead.** The transition executed
  cleanly and *silently*: `2026-07-30T17:45:31.483245Z [EPOCH] Frozen producer list for epoch 199:
  12 producers, active_list=12 (was: 5 producers)`, followed by clean epochs 200 and 201 with a
  stable bond fingerprint. No rollback/reorg lines. And the memory curve is flat across it:
  ai4 17:30=1474 → 18:00=1475 MB; ai5 2128 → 2141 MB; ai1 1401 → **1261** (decreased).

### H4: The freshly snap-synced joiner installed a bad anchor → height-index offset (INC-I-143 D1/D2, INC-I-144 shape) — conf(0.03, measured) — **DEAD**
- **Kill test**: compare hashes at identical heights between a node that snap-synced today and a node
  that has had the chain since genesis; check block 1 (the INC-I-143 damage marker).
- **Kill test result**: **DISPROVING EVIDENCE FOUND — hypothesis is dead.** n12 (RPC 8512, snap-synced
  from scratch today) vs seed1: h=1 `3f28c1a1…` **=**, h=70991 `d2ee28ce…` **=**, h=72531
  `fc44d5b4…` **=**. Block 1 present on the new joiner — the `BLOCK 1 ✗ / INTEGRITY -1` signature of
  INC-I-143/139 is **absent** from this chain.

### H5: `EquivocationDetector` retention scales with producer count — conf(0.15, measured) — **ALIVE but negligible**
- **Kill test**: compute the bound and compare to the 1,091 MB step.
- **Kill test result**: survives as *real but ~1000× too small*. This is the only fork-domain
  structure that genuinely scales with producer count — see §E3 row 4. Δ(5→12 producers) ≈ **+1.2 MB**.
- **Evidence**: `crates/network/src/sync/equivocation.rs:85` keyed `(PublicKey, slot)`, bounded by
  `SLIDING_WINDOW_SLOTS = 360` (`:23`) and `MAX_TRACKED_ENTRIES = 10_000` (`:19`).

---

## Key Evidence Found

### E1 — Fleet is converged (measured, RPC, 2026-07-30 ~21:10Z)

13 structural nodes answered `getChainInfo`; **all** report genesis `a91f6ba2aacc45c3…`, version
`6.24.0`, and one of two tips one block apart (normal propagation):

| Host | Nodes (RPC ports) | Height | Best hash |
|---|---|---|---|
| ai1 | seed,n1,n2,n3 (8500-8503) | 72531 | `fc44d5b4fbf2d22d…6081ef38` |
| ai2 | seed,n4,n5 (8500,8504,8505) | 72531 | `fc44d5b4…` |
| ai3 | seed (8500) | 72531 | `fc44d5b4…` |
| ai4 | n6,n7,n8 (8506-8508) | 72531 | `fc44d5b4…` |
| ai5 | n9,n10,n11,n12 (8509-8512) | 72532 | `78f5e965…` |

Cross-height hash agreement between the **newest** node (n12, snap-synced from zero today) and the
**oldest** (seed1): identical at h=1, h=70991, h=72531. **No divergence, no height offset.**

### E2 — Zero fork events in the entire chain life (measured, log scan)

Per-day counts, ANSI-stripped, `/var/log/doli/mainnet/`:

| Pattern | ai4 `n7.log` (07-22 → 07-30, whole life) | ai1 `seed.log` (07-29 → now) |
|---|---|---|
| `FORK_GUARD` | **0** | **0** |
| `WEDGE_ESCAPE` | **0** | **0** |
| `REORG` / `Reorg` | **0** | **0** |
| `ROLLBACK` | **0** | **0** |
| `STUCK_FORK` | **0** | **0** |
| `EQUIVOCATION` | — | **0** |
| `ORPHAN_CHASE` | 333 (07-22, initial sync) · 527 (07-30) · **0 on 07-23…07-29** | 0 |
| `SNAP_SYNC` | 38 (07-22) · 191 (07-30) · **0 on 07-23…07-29** | 51 (07-30) |

Also `grep -iE 'reorg|sibling|competing|diverg|equivocat|fork recovery|Fork detected'` on seed.log →
**no matches**. The chain has been fork-free since genesis. All 07-30 activity is orphan-chase and
snap-sync — i.e. **new-node onboarding**, not divergence.

### E3 — Fork-retention bounds, quantified (the deliverable)

Per-entry sizes are bounded by the **measured** whole-block size of 489 B (h=70991, incl. header +
1 tx + 120 attestations), so a `BlockHeader` is < 489 B.

| # | Structure | Location | Cap | Scales with producer count? | Worst-case bytes | Derivation |
|---|---|---|---|---|---|---|
| 1 | `fork_block_cache: HashMap<Hash, Block>` | decl `bins/node/src/node/mod.rs:128`; insert/evict `block_handling.rs:102-113`; TTL sweep `periodic.rs:322-336` | **100 blocks** (evict 50 oldest-by-slot when >100) **AND** slot-TTL 30 slots (~5 min) | **NO** — O(1) | **49 KB** typical (100 × 490 B); **4.6 MB** if every slot were the largest block observed (46,354 B @ h=72000) | fixed integer cap, two independent bounds |
| 2 | `wedge_retained_tips: HashSet<Hash>` | decl `mod.rs:216`; cap `wedge_escape.rs:44`; enforce `wedge_escape.rs:112-115` | **`WEDGE_RETAINED_CAP = 64`** (clears wholesale on overflow) | **NO** — O(1) | **2 KB** | 64 × 32 B hash |
| 3 | ReorgHandler `block_weights` + `recent_blocks` + `block_parents` + `lru_order` | `crates/network/src/sync/reorg/mod.rs:82` (`max_tracked: 10000`); LRU evict `:152-158` | **10,000 entries** | **NO** — O(1) | **< 4 MB** | 10,000 × (BlockWeight 56 B + Hash 32 + (Hash,Hash) 64 + Hash 32 = 184 B) = 1.84 MB, ×2 for HashMap overhead |
| 4 | `EquivocationDetector.seen_blocks: HashMap<(PublicKey,u32), BlockHeader>` | `crates/network/src/sync/equivocation.rs:85`; window `:23` (360); cap `:19` (10,000); sweep `:134-135` | **min(10_000, producers × 360)** | **YES — O(producers)** | **< 2.1 MB** at 12 producers; < 0.9 MB at 5 | 12 × 360 = 4,320 entries × < 489 B = < 2.1 MB. Δ(5→12) = 2,520 entries ≈ **+1.2 MB** |
| 5 | `EquivocationDetector.pending_proofs: Vec<EquivocationProof>` | `equivocation.rs:95`, push `:157` | **UNBOUNDED** in production | grows per *equivocation event*, not per producer | **0 B today** | see E6 |

**Total fork-domain memory, worst case: < 11 MB.** Measured step on ai4: **+1,091 MB**.
Fork domain accounts for **≤ 1.0 %**. `MAX_REORG_DEPTH = 1000` bounds `plan_reorg`'s ancestor walk —
CPU only, and gated behind a reorg, of which there have been zero.

### E4 — TIMELINE CORRECTION: the step is TODAY 15:00→15:30Z, and it is NOT fleet-wide (measured, Prometheus)

`(node_memory_MemTotal_bytes - node_memory_MemAvailable_bytes)/1e6`, 15-min step, 2026-07-30 UTC:

| Host | 08:00–15:00 (flat) | 15:30 | 20:00 | Δ | Restarted today? |
|---|---|---|---|---|---|
| **ai4** (n6,n7,n8) | ~490 | **1149** | **1581** | **+1091 (3.2×)** | yes, 16:31:12 |
| **ai5** (n9–n12 + monitoring) | ~863 | **1656** | **2422** | **+1559 (2.8×)** | yes, 16:31:13 |
| ai1 (seed,n1–n3) | ~1231 | 1355 | 1350 | +119 | **n1 only**, 15:30:18 |
| ai2 (seed,n4,n5) | ~1090 | 1092 → 1174 @16:00 | 1265 | +175 | **no** (up since 07-29 06:17) |
| ai3 (seed) | ~671 | 677 | 702 | +31 | no |

Daily maxima confirm the pre-step week was genuinely flat: ai4 = 539/521/526/499/505/508/502 MB on
07-23…07-29, then 1581 on 07-30. **ai4 and ai5 did not step at any point before today.**

⚠️ **This corrects the brief's stated "most discriminating fact."** The step is **not fleet-wide** and
**not a week old**. There are two superimposed components:
- a **large host-local** component on ai4/ai5 only (+1091 / +1559 MB), and
- a **small fleet-wide** component of roughly **+30 MB per node** — visible on ai2 (+90 MB across
  3 nodes at 16:00 with **no restart**, so it is not a restart artifact) and ai3 (+31 MB).

The user's "450 MB → 1.9 GB, 4.2×" is best matched by the **ai4/ai5 host-memory series**, not by any
per-process RSS: no `process_resident_memory_bytes` series exists for doli nodes (only for
node_exporter/prometheus themselves), so any Grafana memory panel must be host-level.

### E5 — KILL TEST: producer-set activation is temporally decoupled from the step (measured)

The two events the brief conflates are 2 h 15 m apart:

| Time (UTC, 07-30) | Event | Evidence |
|---|---|---|
| 15:17:50 | n6 starts **fresh from genesis** — `DOLI Node v6.24.0`, `Genesis VDF proof computed`, `[RECOVERY] Genesis resync ACCEPTED` | n6.log:1,21,72,122 |
| 15:26:03, 15:31:13 | n6 restarts twice more | n6.log:591,18806 |
| **15:00 → 15:30** | **MEMORY STEP** on ai4 (+659) and ai5 (+793) | Prometheus |
| ~16:04–16:10 | 7 producer registrations land at h=70991–70999 | `getProducers` regHeight |
| 16:31:12/13 | ai4 + ai5 all 7 nodes restart together | `ps -o lstart` |
| **17:45:31** | **PRODUCER SET ACTIVATES 5 → 12** at epoch 199 | `[EPOCH] Frozen producer list for epoch 199: 12 producers, active_list=12 (was: 5 producers)` |
| 17:30 → 18:00 | **no memory inflection anywhere**: ai4 1474→1475, ai5 2128→2141, ai1 1401→**1261** | Prometheus |

**Conclusion: producer-count 5→12 is NOT the causal input for the memory step.** The step precedes
the activation by 2 h 15 m and the activation itself is memory-neutral. Whatever changed at 15:17–15:30
is tied to **node processes starting and syncing**, not to the consensus producer set.

Corollary: the producer count is **12, not 7**. Any hypothesis sized on "5→7" is sized on the wrong
number (the real ratio is 2.4×), and any hypothesis timed on "the day N6/N12 started" must use
**2026-07-30 15:17Z**, not the week-old date in the brief.

### E6 — Latent unbounded structure (NOT causal here)

`EquivocationDetector.pending_proofs: Vec<EquivocationProof>`
(`crates/network/src/sync/equivocation.rs:95`) is pushed at `:157` on every detected equivocation and
**never drained in production**: `take_pending_proofs()` (`:189`) and `has_pending_proofs()` (`:194`)
have **zero non-test callers repo-wide** (only `testing/integration/equivocation_slashing.rs`).
Each `EquivocationProof` retains **two full `BlockHeader`s**. It grows monotonically for the process
lifetime.

**Current contribution: 0 bytes** — zero `EQUIVOCATION DETECTED` lines on mainnet. Report it as a
latent defect, **not** as this incident's cause. (The proof returned by `check_block` *is* consumed by
`handle_equivocation` at `block_handling.rs:138-141`; it is the retained duplicate that leaks.)

### E7 — Git archeology on the pre-genesis batch (per the brief's mandate)

The deployed binary is v6.24.0 on all 13 nodes (no skew — checked). The brief establishes
`9647b809..HEAD` is OMEGA-tooling only, so the archeology target is the pre-genesis batch. Ruling out
the fork-domain members of that batch **by runtime evidence, not by reading alone**:

| Commit(s) | Change | Ruled out because |
|---|---|---|
| `560b2af7` (INC-I-143 F2) | FORK_GUARD retains + re-evaluates same-height siblings instead of dropping | Path never executed: `WEDGE_ESCAPE` = 0 across the whole chain life (E2). Retention capped at 64 hashes / 100 blocks ≈ 51 KB (E3 rows 1-2). |
| `d19de7ee` (INC-I-143 F3) | `StuckFork` → bounded `SiblingFetch` | Never triggered: `STUCK_FORK` = 0 (E2). |
| `0de4c536` (INC-I-143 F4) | snap-anchor admission gates (D1/D2) | New joiner n12 landed on the identical chain incl. block 1 (E1/H4) — the gates worked, no offset installed. |
| `b57b1869`, `fa9835dc` (INC-I-144) | height-index purge on rollback/reorg | Path requires a rollback/reorg: **0** of each (E2). Cross-node hash agreement at 3 heights confirms no fossil index (E1). |
| `6738e660` (INC-I-143 F5) | `ECON_EPOCH_INPUTS_MISMATCH` message | Zero occurrences; epochs 199/200/201 clean. |
| `1c510919`, `b7f13756` (INC-I-142) | unified gossip staleness gate | Out of my lens (gossip/parameters) — **flagged, not cleared**. See Cross-Domain Signals. |

I did **not** clear the non-fork members of the batch (INC-I-139 snap admission internals, lazy
state-root, BLS-aggregate removal, disk-guardian). Those belong to other lenses.

---

## Causal Chain

No fork-domain root cause identified — the domain is ruled out. What I *can* establish causally about
the boundary itself:

| # | Item | Derived? | Derivation |
|---|---|---|---|
| 1 | Memory step occurred 2026-07-30 15:00→15:30Z | **YES** | Prometheus 15-min series, all 5 hosts (E4) |
| 2 | Step is concentrated on ai4 + ai5 (+1091 / +1559 MB), with a small ~+30 MB/node fleet-wide component | **YES** | Same series; ai2 (+90 MB, no restart) isolates the fleet-wide part from restart artifacts (E4) |
| 3 | The 15:17Z trigger event is **node process starts + full genesis→72k sync**, not a consensus change | **YES** | n6.log first line 15:17:50 `DOLI Node v6.24.0` + `Genesis resync ACCEPTED`; SNAP_SYNC 191 / ORPHAN_CHASE 527 on 07-30 vs 0 on 07-23…07-29 (E2, E5) |
| 4 | Producer-set growth 5→12 is NOT the trigger | **YES** | Activation at 17:45:31Z is 2 h 15 m after the step and produces zero memory inflection (E5) |
| 5 | No fork, reorg, sibling retention, or height divergence occurred at any point | **YES** | E1 + E2 |
| 6 | **What consumes the ~1.45 GB** | **NO — UNEXPLAINED (out of my lens)** | Node-process RSS on ai4 sums to 1,102 MB of a 1,581 MB host total; the remaining ~479 MB is outside the doli processes, and host memory is still climbing while process RSS is flat. Route to the memory/resource lens. |
| 7 | Why ai4/ai5 per-node RSS is ~2× ai1/ai2 at equal uptime | **NO — UNEXPLAINED** | Same binary, same tip, zero fork events on both groups. Route to memory/parameters lens. |

---

## Cross-Domain Signals

1. **[memory/resource — highest value]** On ai4/ai5 the host memory is **still ramping** at
   +70 MB/h (ai4: 1335→1581 over 16:30→20:00) and **+132 MB/h** (ai5: 1960→2422), 4.5 h after the
   event — while the three doli process RSS values on ai4 were **flat** across two snapshots ~1 h
   apart (n7 387→383, n6 371→370, n8 351→349 MB). **The growth is not in the node processes' RSS.**
   That distinguishes "bounded scaling" from "leak" differently than the brief assumes, and it points
   the search *outside* the Rust heap (page-cache-backed RocksDB, slab, other processes, journald).
   ai5 has only **3,819 MB** total RAM and is at 2,422 MB used — headroom is the operational risk.

2. **[monitoring — blocking for every other lens]** **All `doli_*` Prometheus gauges read 0** on every
   node sampled: `doli_chain_height 0`, `doli_peers_connected 0`, `doli_storage_bytes 0`,
   `doli_utxo_set_size 0`, `doli_mempool_size 0` — on ai1 seed (:9000) and on all of ai4/ai5
   (:9006–:9012), while the nodes are provably at h=72531 with live peers. **There is no
   `process_resident_memory_bytes` series for doli nodes at all.** Consequences: (a) the Grafana
   "memory" panel can only be host-level, which reframes the user's "per-node 450 MB → 1.9 GB";
   (b) no RocksDB/mempool/peer decomposition is available from Prometheus — other lenses must read
   `/proc` and logs directly, not Prometheus. This also blocks Evidence-Assembly open question #1
   (RocksDB cap reconciliation) and #4.

3. **[connectivity/sync]** The only elevated activity at the boundary is onboarding: `ORPHAN_CHASE`
   527 and `SNAP_SYNC` 191 on n7 on 07-30 vs **0 on 07-23…07-29**; seed1 `SNAP_SYNC` 51 on 07-30.
   n6 restarted 3× in 14 min (15:17:50, 15:26:03, 15:31:13) and re-ran genesis resync each time.
   Sync-session buffers and block-serving are the natural place to look for the ai4/ai5 host-local
   component — n6/n12 pulled ~72k blocks, most plausibly from their same-host peers.

4. **[CPU / INC-I-141]** Attestation count per block rose **120 → 128** (+6.7 %) across the boundary
   (h=70000/70990/71000 = 120; h=71300/72000/72500 = 128). If attestation/vote verification is O(N)
   per block (INC-I-141, OPEN), the CPU-shape change should track **+6.7 % attestations** and the
   **2.4× producer set**, not the memory step. Note the attestation increase lands between h=71000 and
   h=71300 — i.e. at the **epoch-199 boundary**, exactly where memory did *not* move. That is a clean
   separation: the producer-set change moved CPU-side inputs but not memory.

5. **[parameters]** 7 producer registrations landed within **9 blocks (~90 s)** — a 2.4× producer-set
   jump in one epoch. INC-I-038 records gossip-mesh scoring becoming unreachable at a specific
   producer count; a 2.4× step is exactly the shape of this fleet's documented threshold knees.
   INC-I-142's staleness gate (`1c510919`) is in the binary and is **not** cleared by my lens.

6. **[deploy/ops]** ai4 + ai5 (7 nodes) restarted simultaneously at 16:31:12–16:31:13, and ai1's n1
   at 15:30:18. Per INC-I-062 / INC-I-143 §12, unserialized fleet restarts are this project's
   recorded fork trigger. **It did not cause a fork this time** (E1/E2) — but the operational pattern
   recurred and should be noted.

7. **[postmortem status correction]** The brief's Evidence Assembly says INC-I-143's root cause was
   "NOT established". The postmortem's **§12 (run 466)** supersedes that: root cause **was**
   established at conf 0.95 as the unserialized rolling restart shipping block-content commit
   `427d5050`. Also, the INC-I-143 damage signature (`BLOCK 1 ✗`, `INTEGRITY -1`) is **gone** — the
   2026-07-22 genesis reset cleared it; n12 has block 1 (`3f28c1a1…`) matching seed1.

---

## Gaps

- **Per-process memory history does not exist.** No `process_resident_memory_bytes` for doli nodes in
  Prometheus, so I could only measure host-level memory over time plus two live `ps` snapshots. The
  per-node attribution of the ai4/ai5 step is therefore *inferred from host arithmetic*, not measured.
- **ai4/ai5 pre-step per-node RSS is unmeasured.** My statement that n7/n8 were "~200–245 MB before"
  is derived from host totals minus estimated OS overhead — labeled inferred, not measured.
- **seed.log on ai1 only covers 2026-07-29 → now** (rotated). My "whole chain life" fork-free claim
  rests on ai4 `n7.log` (07-22 → now). I did not scan the rotated/gzipped seed archives, nor ai2/ai3
  seeds, nor the ~30 external producers (no access).
- **I did not identify 5 of the 7 new producers.** Only n6 (`d13ae338…`) is confirmed by log. Whether
  all 7 are structural nodes or include externals is unresolved.
- **Not cleared by this lens**: INC-I-139 snap-admission internals, INC-I-142 staleness gate, lazy
  state-root, BLS-aggregate removal, disk-guardian, `diagnostic_ledger` (INC-I-101/102, both OPEN and
  both implicating ai4/ai5 specifically), and the memory-watchdog arming question (INC-I-117).
- **The `pending_proofs` leak (E6) is unverifiable at runtime** — there is no metric and no
  equivocation has occurred, so it cannot be observed, only read.
- Confidence ceiling honored: no hypothesis above 0.7.
