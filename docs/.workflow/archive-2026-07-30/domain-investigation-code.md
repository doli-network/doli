# Domain Investigation Report: Code / Logic

**INC**: proposed INC-I-146 · **RUN_ID**: 473 · **Agent**: blockchain-domain-investigator-code
**Date**: 2026-07-30 · **All commands read-only** (ps, journalctl read, Prometheus HTTP query, curl /metrics, git, source reads)

---

## Domain Lens

Source-code correctness and algorithmic complexity — data structures that retain more than they should,
O(N)/O(N²) growth in hot paths, missing eviction, per-block/per-epoch allocations never freed, and
consensus/validation work whose cost scales with producer count.

---

## Chain Context

| Item | Value | Source |
|---|---|---|
| Binary | `DOLI Node v6.24.0` | `/var/log/doli/mainnet/n6.log` line 1 |
| Network | mainnet (id=1), genesis=1784717100, slot=10s, embedded chainspec | n6.log startup banner |
| Chain height at investigation | 72,572–72,575 | n6.log `[APPLY_END] slot=72575`, `FINALITY ... height 72572` |
| **Producer set size** | **12** (`bonds_n=12 epl_n=12 apl_n=12`) — **not 7** | n6.log epoch component line |
| Attestation weight | 540/720 at finality | n6.log `FINALITY` line |
| Window examined | 2026-07-20 → 2026-07-30 20:45 UTC | Prometheus range queries |

**Host map established by `node_memory_MemTotal_bytes` + `doli_uptime_seconds` instance ports:**

| Host | Prometheus instance | RAM | Nodes |
|---|---|---|---|
| ai1 | `<server-ip>` | 16.77 GB | seed(:9000), n1, n2, n3 |
| ai2 | `<server-ip>` | 16.77 GB | seed(:9000), n4, n5 |
| ai3 | `<server-ip>` | 4.11 GB | seed(:9000) |
| ai4 | `<server-ip>` | 4.01 GB | n6, n7, n8 |
| ai5 | `<server-ip>` | 4.01 GB | n9, n10, n11, n12 + monitoring |

---

## What I Don't Understand

1. **Why the brief states the step was "fleet-wide, including nodes already running."** Measurement
   contradicts this: ai1, ai2 and ai3 host memory is flat across the entire window. Only ai4 and ai5
   moved. I cannot reconcile the user's scope claim with the metric.
2. **Whether the residual +20–25 MB/h/node drift I measured on ai4/ai5 continues or asymptotes.** A
   4-hour window with ±0.1 GB sample noise is too short to separate a slow leak from post-sync settling.
3. **Why n6–n12 plateau at ~370 MB while n1–n5 and the seeds sit at 137–201 MB** on the same binary.
   Bulk-sync heap/allocator retention is my leading explanation but I did not prove it.
4. **Why the DOLI application metrics exporter emits all zeros** while the node is demonstrably at
   height 72,572 with live peers.
5. **Operational context for the Jul 22 11:29 stop and Jul 30 restart of n6–n12** — not in the brief.

---

## Domain Relevance Assessment

**Relevance: LOW**

The reported symptom — a ~450 MB → ~1.9 GB step with a CPU shape change — is **explained by process
lifecycle, not by code**. Seven `doli-node` processes that had been stopped for eight days were started
today. On ai4 the "450 MB stable baseline" was an **idle host with zero DOLI nodes running**; it is not a
per-node baseline and is not comparable to the post-start figure.

I found **no code mechanism that produces a step at a producer-count boundary**, and I explicitly killed
the three code hypotheses the brief ranked highest (diagnostic_ledger, lazy state-root memo, wedge-escape
retention). Per the anti-hedging rule, I state this plainly: **this redirects synthesis toward
operations/observability, not code.**

One genuine code-adjacent concern survives at moderate confidence (H5, residual drift + June 29 OOM
history), and one clear defect was found that is not the reported symptom (H7, dead metrics exporter).

---

## Hypotheses

### H1: The reported step is process lifecycle, not a leak — 7 stopped nodes were started today — conf(0.7, measured)

- **Kill test**: find any host whose memory stepped **without** a node start, or find n6–n12 running
  continuously through the claimed stable week.
- **Kill test result**: **NOT FOUND — hypothesis survives.** systemd journal is unambiguous:

```
Jul 22 08:58:17 doli-ai4  Started doli-mainnet-n6/n7/n8.service
Jul 22 11:29:13 doli-ai4  Stopped doli-mainnet-n6/n7/n8.service   <-- OFF for 8 days
Jul 30 16:17:50 doli-ai4  Started  (local BST = 15:17:50 UTC)
Jul 30 16:31:13 doli-ai4  Started  (final; after two stop/start cycles)
```
```
Jul 22 11:29:17 doli-ai5  Stopped doli-mainnet-n9/n12.service     <-- OFF for 8 days
Jul 30 16:31:14 doli-ai5  Started
```

- **Evidence — 15-minute memory curve, `node_memory_MemTotal_bytes - node_memory_MemAvailable_bytes`:**

| UTC | ai4 (n6–n8) | ai5 (n9–n12) | ai1 (control, nodes up all week) |
|---|---|---|---|
| 15:00 | 0.49 | 0.86 | 1.23 |
| 15:30 | 1.15 | 1.66 | 1.36 |
| 16:15 | 1.30 | 1.89 | 1.40 |
| 18:00 | 1.47 | 2.14 | 1.29 |
| 20:15 | **1.54** | **2.29** | **1.36** |

  ai4 and ai5 step at exactly the node-start timestamp. **ai1/ai2/ai3 do not step at all.**
- **Arithmetic check (shown):** ai4 idle baseline 0.49 GB + 3 nodes × ~0.36 GB = 1.57 GB; observed 1.54 GB.
  ai5 baseline 0.86 GB (incl. Prometheus 238 MB + Grafana) + 4 nodes × ~0.37 GB = 2.34 GB; observed 2.29 GB.
  **Residual < 5 % — the step is fully accounted for by process count.**
- **Reconciliation with the user's numbers**: "450 MB" = ai4's idle-host baseline (0.49 GB).
  "1.9 GB" = ai5 at 16:15 (1.89 GB). Both figures are *host* memory from node_exporter, on two different
  hosts. There is no metric anywhere in this fleet that reads 1.9 GB for a single node.

!! **TRIGGER IDENTIFIED**: starting n6–n12 at 2026-07-30 15:17–15:31 UTC.
This answers WHAT happened. It does NOT by itself answer why per-node RSS is 370 MB rather than ~180 MB — see H5.

---

### H2: `diagnostic_ledger` retention (INC-I-101 / INC-I-102) — **DEAD** — conf(0.0, measured)

- **Kill test**: does the deployed binary contain the subsystem?
- **Kill test result**: **KILLED.** Commit `98650be2` *"refactor(observability): remove fork-diagnostics
  subsystem entirely"* (2026-07-07) is an **ancestor of the genesis release** `9647b809` (2026-07-22),
  verified with `git merge-base --is-ancestor 98650be2 9647b809` → true.
  `ls crates/storage/src/` contains no `diagnostic_ledger` module.
- **Consequence for synthesis**: the brief's strongest lead ("two open incidents already point at it")
  is **structurally impossible**. INC-I-101 and INC-I-102 should be closed as obsoleted-by-removal.

---

### H3: Lazy state-root commitment memo grows per height — **DEAD** — conf(0.0, measured)

- **Kill test**: is the memo a per-height map, or a single slot?
- **Kill test result**: **KILLED.** `bins/node/src/node/state_root_serve.rs:36`
  `let memo = *self.cached_state_root.read().await;` — the deref-copy proves the memo is a `Copy`
  `Option<(Hash, Hash, u64)>`, i.e. **one tuple**, keyed on the current tip and overwritten on staleness
  (`state_root_serve.rs:22–29`). O(1) forever. It cannot grow with chain height.
- The brief's own caveat was right: a height-driven memo would produce a *ramp*, and no ramp exists.

---

### H4: Wedge-escape sibling retention is unbounded (`560b2af7`) — **DEAD** — conf(0.0, measured)

- **Kill test**: is there a cap and an eviction site?
- **Kill test result**: **KILLED.** `bins/node/src/node/wedge_escape.rs:44`
  `pub(crate) const WEDGE_RETAINED_CAP: usize = 64;` with eviction at lines 112–113
  (`if self.wedge_retained_tips.len() >= WEDGE_RETAINED_CAP { self.wedge_retained_tips.clear(); }`).
  The set stores `Hash` only (32 B) — bound ≈ **2 KB**. Not a memory mechanism.

---

### H5: Residual post-bulk-sync retention / slow drift on ai4/ai5 — **LIVE** — conf(0.45, measured)

- **Kill test**: is RSS genuinely flat after the initial ramp?
- **Kill test result**: **partially disconfirming — hypothesis survives weakened.**
  - Two `ps` snapshots 170 s apart: n11 375→373 MB, n7 378→378 MB — **flat at short timescale.**
  - But 16:15 → 20:15 (4 h) shows ai5 1.89 → 2.29 GB and ai4 1.30 → 1.54 GB. Linear slope ≈
    **+0.09 GB/h (ai5) / +0.06 GB/h (ai4)** = **~20–25 MB/h/node**, above the ±0.1 GB sample noise.
- **Corroborating precedent (same hosts, same binary family)** — `dmesg -T` on ai4:

```
Mon Jun 29 09:23:25  Killed process doli-node-n8  anon-rss:1411364kB  (1.41 GB)
Mon Jun 29 09:53:22  Killed process doli-node-n8  anon-rss:1705996kB  (1.71 GB)
Mon Jun 29 10:10:22  Killed process doli-node-n6  anon-rss:1644032kB  (1.64 GB)
Mon Jun 29 10:18:18  Killed process doli-node-n8  anon-rss:2124008kB  (2.12 GB)
Mon Jun 29 10:32:35  Killed process doli-node-n8  anon-rss:1366832kB  (1.37 GB)
```

  **Per-process anon-RSS of 1.4–2.1 GB is real and has happened** — but on **2026-06-29, pre-genesis**,
  on the previous chain. At 20–25 MB/h a node would reach ~1.9 GB in ~2.5–3 days, which is consistent
  with that OOM cadence. **No OOM has occurred in the last 3 days.**
- **Why n6–n12 (370 MB) > n1–n5 (137–201 MB)**: n6–n12 replayed ~69,000 blocks of backlog in ~45 min on
  restart; n1 restarted the same day (14:30 UTC) with no backlog and sits at 155 MB. Leading explanation
  is heap/allocator retention from the bulk-sync burst plus RocksDB compaction working set — **not proven.**
- **This needs a 24–48 h observation window to classify.** I will not call it a leak on 4 hours of data.

---

### H6: CPU shape change caused by O(N) attestation verification (INC-I-141) — **O(N) CONFIRMED, but NOT the cause here** — conf(0.6, measured)

- **The O(N) claim is TRUE.** `crates/core/src/attestation.rs:203–206`:

```rust
for (sig, pk) in self.signatures.iter().zip(self.attesters.iter()) {
    signature::verify_with_domain(ATTESTATION_DOMAIN, &msg, sig, pk)
    // ... sequential, one verify per attester, no batch/aggregate
}
```
  `RegionAggregate::verify()` is a **sequential per-signature loop** — genuinely O(N) in attester count,
  with no batching. INC-I-141 is correctly diagnosed and remains open.
- **Kill test**: does measured CPU scale as O(producer count)?
- **Kill test result**: **KILLED as the explanation for this incident.** Measured CPU
  (`100 - avg(rate(node_cpu_seconds_total{mode="idle"}[10m]))*100`):

| Host | Jul 28–30 12h | Jul 30 16h+ |
|---|---|---|
| ai4 | **1 %** | **5 %** |
| ai5 | **1 %** | **4 %** |
| ai1 | 2 % | 3 % |
| ai2 | 2 % | 3 % |
| ai3 | 2 % | 3 % |

  ai4/ai5 were at 1 % because **they were running zero DOLI nodes**. The step to 4–5 % is 3–4 node
  processes starting. ai1/ai2/ai3 — which carry the same 12-producer attestation load — did not change.
  At N=12 attesters × 1 block / 10 s, the O(N) loop is ~1.2 verifications/s. **Negligible.**
  The O(N) defect is real and will bite at ~300 producers (as INC-I-141 records); it is not biting now.

---

### H7: DOLI application metrics exporter is emitting all zeros — **CONFIRMED DEFECT** (not the reported symptom) — conf(0.7, measured)

- **Evidence**, direct scrape of n6 on ai4 while the node is at height 72,572 with live peers:

```
$ curl -s http://127.0.0.1:9006/metrics | grep '^doli_'
doli_active_producers 0
doli_blocks_processed_total 0
doli_chain_height 0
doli_is_syncing 0
doli_mempool_size 0
doli_peers_connected 0
```

- Confirmed fleet-wide and across the whole week: a Prometheus range query for `doli_chain_height`
  from 2026-07-22 to 2026-07-30 returns **0 for all 15 targets at every sample**.
- **Two consequences:**
  1. The operator's Grafana dashboard **cannot** be showing per-node memory or per-node chain state.
     Every panel it can populate is node_exporter **host** data. This fully explains why "450 MB" and
     "1.9 GB" are host figures from two different machines.
  2. The fleet currently has **no application-level observability at all** — no height, no peer count,
     no sync state, no mempool. This also means INC-I-117's "memory watchdog dormant" question cannot
     be answered from metrics.

---

## Retained Structures Table

Every collection field reachable per-block / per-attestation / per-peer / per-producer that I enumerated,
with its insertion trigger, eviction site, and bound.

| Structure | file:line | Insertion trigger | Eviction site | Bound | Scales with |
|---|---|---|---|---|---|
| `fork_block_cache: HashMap<Hash, Block>` | `bins/node/src/node/mod.rs:128` | fork/orphan block received (`block_handling.rs:103`, `fork_recovery.rs:39`) | **TTL only** — `periodic.rs:322–336`, `cache.retain(\|_,b\| b.header.slot >= best_slot - 30)`; also `remove` at `block_handling.rs:424`, `clear` at `fork_recovery.rs:664` | **TTL-bounded (30 slots ≈ 5 min), NOT count-bounded** | fork/gossip block arrival rate × 300 s. ⚠️ **only residual unbounded-in-principle structure found** |
| `rejected_fork_tips: HashSet<Hash>` | `mod.rs:208` | rejected fork tip (`network_events.rs:149`) | `network_events.rs:151–152` `if len() > 1000 { clear() }` | 1000 × 32 B ≈ **32 KB** | fork rejection rate (capped) |
| `wedge_retained_tips: HashSet<Hash>` | `mod.rs:216` | wedge-escape sibling retained (`wedge_escape.rs:115`) | `wedge_escape.rs:112–113` at `WEDGE_RETAINED_CAP = 64` (`:44`) | 64 × 32 B ≈ **2 KB** | sibling arrival (capped) |
| `attest_fetch_tracker: HashMap<Hash,(Instant,u8,PeerId)>` | `mod.rs:292` | attestation for unknown block | `periodic.rs:313–315` `retain(< 30 s)`; `remove` at `:287`, `:310` | 30 s TTL, ≤3 peers/hash | attestation rate × 30 s |
| `peer_churn: HashMap<PeerId, VecDeque<Instant>>` | `mod.rs:240` | peer connect/disconnect | `network_events.rs:736–737` `if len() > 1024 { retain(...) }` | 1024 peers | peer count (capped) |
| `recent_canonical_hashes: VecDeque<(u64,Hash)>` | `sync/manager/mod.rs:164` | canonical block applied | `block_lifecycle.rs:51–52` and `mod.rs:317–318` `if len() > 200 { pop_front() }` | 200 entries ≈ **8 KB** | nothing (fixed) |
| `producer_liveness: HashMap<PublicKey,u64>` | `mod.rs:176` | `apply_block/mod.rs:137` per block | `rewards.rs:532` `clear()` each epoch | **one entry per producer** | producer count (12) — ~0.5 KB |
| `peers: HashMap<PeerId, PeerSyncStatus>` | `sync/manager/mod.rs:76` | peer handshake | peer disconnect | peer count | peer count |
| `cached_state_root: Option<(Hash,Hash,u64)>` | `state_root_serve.rs:36` | state-root serve on cold/stale | overwritten in place | **O(1), single tuple** | nothing |

**Verdict on the sweep: no unbounded collection with an insertion site and an absent eviction site was
found.** The one structure worth flagging is `fork_block_cache`, which stores **full `Block` values** and
is bounded only by a 30-slot TTL — under a gossip flood its size is (flood rate × 300 s), which is
precisely the INC-I-114 shape (unbounded `VecDeque` → 7.44 GB). It is a **latent capacity risk, not the
current cause**: no fork/flood activity appears in the current logs, and the measured RSS is accounted for
arithmetically by process count.

---

## Causal Chain

| # | Item | Derived? | Derivation |
|---|---|---|---|
| 1 | ai4 host memory 0.49 GB for a week | YES | n6/n7/n8 `Stopped` Jul 22 11:29 → host ran OS + node_exporter only |
| 2 | "~450 MB stable baseline" in the report | YES | = item 1. It is an **idle-host** figure, never a per-node figure |
| 3 | Step at 2026-07-30 15:17–15:31 UTC | YES | systemd `Started doli-mainnet-n6/n7/n8/n9/n12` at that timestamp |
| 4 | ai4 → 1.54 GB, ai5 → 2.29 GB | YES | baseline + (3 or 4) × ~0.37 GB; residual < 5 % |
| 5 | "~1.9 GB" in the report | YES | ai5 host memory at 16:15 UTC = 1.89 GB |
| 6 | CPU 1 % → 4–5 % on ai4/ai5 only | YES | 3–4 node processes started on hosts that had none |
| 7 | ai1/ai2/ai3 flat in both memory and CPU | YES | their nodes never stopped; nothing changed for them |
| 8 | Claim "affected all nodes incl. already-running" | **NO — CONTRADICTED** | Prometheus shows ai1/ai2/ai3 flat. The claim is not supported by any metric I could find |
| 9 | n6–n12 at ~370 MB vs n1–n5 at 137–201 MB | **NO — UNEXPLAINED** | Leading hypothesis: ~69k-block bulk-sync heap/allocator retention. Needs heap profiling or a 24 h decay observation |
| 10 | +20–25 MB/h/node residual drift | **NO — UNEXPLAINED** | 4 h window insufficient. Needs 24–48 h re-measure before classifying leak vs settling |
| 11 | June 29 OOMs at 1.4–2.1 GB anon-rss | **NO — UNEXPLAINED** | Pre-genesis, different chain. Real precedent; root cause never established |

---

## Cross-Domain Signals

**→ Parameters/Tuning investigator**
- ai4 and ai5 have **4.01 GB RAM** and host **3 and 4** producer nodes respectively, plus (on ai5) the
  entire Prometheus + Grafana + Alertmanager stack. ai5 is at **2.29 / 4.01 GB used with zero headroom
  budgeted**. ai1/ai2 have 16.77 GB for the same or fewer nodes. This is a **capacity/placement
  asymmetry**, and it is where every OOM in this fleet's history has landed (INC-I-104, INC-I-102, the
  June 29 kills). **No `MemoryMax=` / `MemoryLimit=` is set on `doli-mainnet-n6.service`** — I grepped the
  unit and found none, so there is no per-service cgroup protection.
- `--auto-checkpoint` is enabled at **every 4500 blocks, keeping last 5** (n6.log startup line).

**→ Connectivity/Sync investigator**
- n6–n12 replayed ~69,000 blocks of backlog on restart after 8 days offline. Post-catch-up they report
  `state=Synchronized`, `FINALITY ... 540/720`. The sync burst is the leading explanation for their
  elevated steady-state RSS.
- Log shows a repeating pattern of `DirectAttestation` sync requests answered with
  `Block(None)` / `has_block=false` (`n6.log` tail, 20:20:51). Possibly benign, possibly wasted round trips.

**→ Fork/Divergence investigator**
- `bonds_n=12 epl_n=12 apl_n=12` and `epl == apl` (identical hashes) on n6 — producer lists agree. No
  divergence signal seen from this lens.

**→ All investigators (operational)**
- **The DOLI metrics exporter is dead fleet-wide (H7).** Any conclusion drawn from a `doli_*` panel in
  Grafana is invalid. Only `node_exporter` host metrics are trustworthy right now.
- Two of the brief's open incidents, **INC-I-101 and INC-I-102**, are obsoleted: the `diagnostic_ledger`
  they concern was deleted in `98650be2`, before genesis.

---

## Gaps

1. **Leak vs settling for the +20–25 MB/h/node drift is unresolved.** Requires re-measuring
   `node_memory_MemTotal_bytes - node_memory_MemAvailable_bytes` on ai4/ai5 in 24 h and 48 h. This is the
   single follow-up I would prioritise, given the June 29 OOM history on these exact hosts.
2. **No heap profiling was performed** (would require restarting a node under a profiler — forbidden by
   the read-only constraint). The n6–n12 vs n1–n5 RSS gap therefore remains an inference.
3. **I did not read the INC-I-139 snap-admission commits line by line.** I ruled them out indirectly: the
   memory arithmetic closes to within 5 % on process count alone, leaving no unexplained mass for a snap
   path to account for. If the synthesizer needs those commits ruled out directly, that work remains.
4. **`fork_block_cache` under gossip flood was not load-tested.** Its TTL-only bound is a real latent
   risk (INC-I-114 shape) but I have no evidence it is active now.
5. **I could not reconcile the "fleet-wide" claim** with the metrics. If the user has a specific Grafana
   panel showing ai1/ai2/ai3 stepping, that would falsify H1 and should be surfaced.
6. **`crates/core/src/validation.rs` (5,698 lines) was not read in full** — only searched. `apply_block`
   epoch-boundary allocation was assessed via `producer_liveness` (bounded, cleared per epoch) rather than
   a full read of the epoch transition.

---

## Verdict

**CODE / LOGIC DOMAIN: RULED OUT as the cause of the reported symptom.**

- **Memory question**: the ~450 MB → ~1.9 GB step is **not a code defect**. It is seven `doli-node`
  processes starting on two hosts that had been running none for eight days. The arithmetic closes to
  within 5 %. The "450 MB stable week" was an idle host. The step is **not fleet-wide** — ai1/ai2/ai3 are
  flat. Classification: **bounded, expected-for-topology**, not an unbounded leak.
  *Caveat*: a residual ~20–25 MB/h/node drift (H5) is unclassified and, given the June 29 OOM history at
  1.4–2.1 GB anon-rss on these same 4 GB hosts, warrants a 24–48 h re-measure. That is the one live risk.
- **CPU question — separate answer**: the 1 % → 4–5 % change on ai4/ai5 is the same process-start event.
  The O(N) attestation-verification defect (INC-I-141) is **real and confirmed at
  `crates/core/src/attestation.rs:203–206`** — a sequential, unbatched per-signature loop — but at 12
  producers it costs ~1.2 verifications/s and is **not** the cause. It remains a genuine scaling defect
  for ~300 producers.
- **Genuine defect found, unrelated to the report**: the DOLI application metrics exporter emits zeros
  fleet-wide (H7). This is the reason the reported figures are host metrics from two different machines.

**Confidence ceiling respected: no claim above conf(0.7).**
