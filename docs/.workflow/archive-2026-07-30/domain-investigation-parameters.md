# Domain Investigation Report: Parameters / Tuning

- **INC_ID**: proposed INC-I-146 · **RUN_ID**: 473 · **Date**: 2026-07-30
- **Agent**: `blockchain-domain-investigator-parameters`
- **Constraint compliance**: all operations read-only (`ps`, `free`, `grep` on unit files, `sudo grep` on
  logs, Prometheus HTTP `/api/v1/query*`, Grafana `/api/dashboards` GET, one `getChainInfo` read RPC).
  No restart, no config change, no write RPC, no wipe.

---

## Domain Lens

Configuration and capacity parameters: RocksDB tuning (block cache, write buffers, memtables, CF count),
mempool limits, `max_peers` / mesh degree, buffer sizes, watchdog thresholds, archiver/checkpoint
retention, systemd unit resource settings, and allocator behaviour.

**Key question**: is ~1.9 GB the arithmetically correct consequence of configured limits at 7 producers
(capacity reality) — or is a limit absent/wrong (defect)?

---

## Chain Context

| Item | Value | Source |
|---|---|---|
| Chain / tip | mainnet, height **72,583**, hash `612fa574…`, all sampled nodes identical | `getChainInfo` on ai1:8501, ai4:8506, ai5:8509 |
| Binary | `doli-node 6.24.0 (9647b809)` on **every** active service | `--version` on ai1/ai4/ai5 |
| Binary integrity | md5 `0782278a61e12c37ec3b6481c5f343df` **identical across all 11 active per-service copies** | `md5sum /mainnet/bin/doli-node-*` |
| Allocator | **system glibc malloc** — zero jemalloc/mimalloc/snmalloc references in the workspace | grep over `Cargo.toml` + `*.rs` |
| Cores | ai1=4, ai4=2, ai5=2 | `nproc` |
| Threads/process | 17–21 | `/proc/<pid>/task` |

---

## ⚠️ TOP-LINE CORRECTION TO THE BRIEF (measured, not inferred)

The brief's premises **"per-node RSS ~450 MB → ~1.9 GB"** and **"fleet-wide, incl. unchanged nodes"** are
both contradicted by direct measurement.

### 1. No DOLI process is anywhere near 1.9 GB

`ps -eo rss` across the whole structural fleet, 2026-07-30 ~20:20 UTC:

| Host | Processes (RSS MB) | uptime |
|---|---|---|
| ai1 | seed 160, n1 155, n2 200, n3 137 | n1 4.8 h; rest 61.5 h |
| ai2 | seed 181, n4 189, n5 201 | 38 h |
| ai3 | seed 138 | 38 h |
| ai4 | n6 363, n7 378, n8 343 | 4.8 h |
| ai5 | n9 373, n10 368, n11 375, n12 358 | 4.8 h |

**Fleet maximum per-process RSS = 378 MB.** `process_resident_memory_bytes` is *not exported by
doli-node* (only by node_exporter/Prometheus themselves) — so no Grafana panel can show per-node RSS.

### 2. The 4.2× step is **ai5 HOST-level** memory, and it reproduces exactly

Node Exporter Full "used/Apps" expression
(`MemTotal − MemFree − Buffers − Cached − Slab − PageTables − SwapCached`), 2026-07-30, 30-min step:

| Host | 13:00 | 15:00 | 15:30 | 17:00 | 20:00 |
|---|---|---|---|---|---|
| **ai5** | 0.48 | **0.48** | 1.27 | 1.65 | **2.04 GB** |
| **ai4** | 0.13 | 0.13 | 0.78 | 1.05 | 1.21 GB |
| ai1 | 0.72 | 0.71 | 0.70 | 0.74 | 0.83 GB |
| ai2 | 0.59 | 0.59 | 0.59 | 0.68 | 0.76 GB |
| ai3 | 0.28 | 0.26 | 0.27 | 0.28 | 0.29 GB |

- ai5: **0.48 GB → 2.04 GB = 4.25×**. The user's "≈450 MB → ≈1.9 GB, ≈4.2×", to two significant figures.
- Step, not ramp: one 30-minute sample.
- **Arithmetic**: 0.48 GB base + 4 × 0.37 GB (n9–n12) = **1.96 GB** ✅ (measured 2.04, +4 % for page-table
  and slab growth). ai4: 0.13 + 3 × 0.36 = **1.21 GB** ✅ (measured 1.21).
- **ai1, ai2, ai3 did not step.** The change is confined to the two hosts where processes were started.
  **The "fleet-wide" premise is false.**

### 3. The boundary is 2026-07-30 **15:17:50 UTC** — today, not "roughly a week after genesis"

`/var/log/doli/mainnet/n6.log`: `2026-07-30T15:17:50.869857Z Starting node with data directory
"/mainnet/n6/data"`. Prometheus shows ai4:9006-9008 and ai5:9009-9012 with **no series at all from
2026-07-22 until 2026-07-30 ~15:20** — n6–n12 were **down for 8 days**, i.e. the "3 seeds + 5 producers"
topology during the stable window was literally seed1/2/3 + n1–n5, and **seven** processes (not two)
came up at the boundary.

`!! TRIGGER IDENTIFIED: seven doli-node processes started on two 4 GB hosts at 2026-07-30 15:17 UTC,
each snap-syncing 70,753 blocks from scratch.`
This answers WHAT. The WHY of the *per-process* 2× (below) is the real domain finding.

---

## What I Don't Understand

1. **Why n6–n12 sit at 343–378 MB while n1–n5/seeds sit at 137–201 MB** — same binary (identical md5),
   same configured limits, same chain tip, and n6 has *shorter* uptime. My best-supported explanation is
   an allocator high-water mark from snap sync, but I could not confirm it directly: there is no
   live-heap metric to compare against RSS. I state this as `inferred`, not `measured`.
2. **Whether the user's Grafana panel was ai5's host panel or something else.** The arithmetic match is
   exact (4.25× vs 4.2×) but I did not watch the user's screen. A second candidate exists: the DOLI
   RocksDB dashboard's stat panel *"Per-node RocksDB resident estimate"* uses
   `sum(memtable + block_cache + table_readers)` **without `by (instance)`** — a fleet SUM mislabelled
   "per-node", which also stepped today (398 MB → 774 MB at 15:30). Either way the mechanism is the same
   class: an aggregate that grows when scrape targets/processes appear.
3. **Why n6–n12 were down for 8 days**, and whether that was intentional. Outside my lens.
4. **The CPU-shape change.** Out of my lens except to note that 7 concurrent snap syncs of 70,753 blocks
   against a 5-node serving fleet is a large, transient, and expected CPU event.

---

## Domain Relevance Assessment

**Relevance: HIGH** — but *not* as a defect in a tuned parameter.

The observed numbers are the arithmetically correct consequence of process placement and the configured
limits. The domain-relevant defects are **absent limits**, not wrong ones: no cgroup `MemoryMax`, a
disabled memory watchdog, an unbounded snap-sync heap high-water, and a dashboard aggregate that made a
host-capacity event look like a fleet-wide leak.

---

## THE MEMORY BUDGET (core deliverable — nobody had sized this before)

Per **process**. "Observed" = live Prometheus gauges for `<server-ip>:9002` (n2, 61.5 h uptime) and
`<server-ip>:9006` (n6, 4.8 h uptime), 2026-07-30 ~21:00 UTC.

| # | Consumer | Configured limit | file:line | Scales with | Observed n2 | Observed n6 |
|---|---|---|---|---|---|---|
| 1 | `block_store` memtables (all 9 CFs) | **48 MB** (`db_write_buffer_size`) | `crates/storage/src/block_store/types.rs:87`, applied `block_store/open.rs:71` | constant — **DB-wide cap, not ×CF** | 6.8 MB | 4.7 MB |
| 2 | `block_store` block cache (LRU, shared by 9 CFs) | **32 MB** | `crates/storage/src/block_store/open.rs:81` | constant | 33.1 MB | 33.1 MB |
| 3 | `state_db` memtables (all 7 CFs) | **64 MB** (`db_write_buffer_size`) | `crates/storage/src/state_db/types.rs:39`, applied `state_db/open.rs:85` | constant — DB-wide cap | 7.6 MB | 3.9 MB |
| 4 | `state_db` block cache (LRU, shared by 7 CFs) | **48 MB** | `crates/storage/src/state_db/open.rs:95` | constant | 5.5 MB | 3.6 MB |
| 5 | Table readers (index+filter blocks, both DBs) | bounded by `max_open_files = 256`/DB | `state_db/open.rs:65`, `block_store/open.rs:64` | ×SST count → ×chain height | 0.54 MB | 0.58 MB |
| 6 | RocksDB background jobs | `max_background_jobs=2`, `max_subcompactions=1` per DB | `state_db/open.rs:88-89`, `block_store/open.rs:75-76` | constant | — | — |
| 7 | WAL | `max_total_wal_size` = 64 MB (state_db) / 48 MB (block_store) | `state_db/open.rs:77`, `block_store/open.rs:72` | constant (mostly disk) | — | — |
| 8 | Mempool | **10 MB / 5,000 tx**; `max_tx_size` 600 KB | `crates/mempool/src/policy.rs:26,27,29` | constant | gauge **broken (0)** | gauge **broken (0)** |
| 9 | Gossip `SeenCache` | **16,384 entries**, TTL 180 s ⇒ ≈0.9 MB/cache | `crates/network/src/gossip/staleness.rs:99,110` | constant | not exported | not exported |
| 10 | Peers | `max_peers = 50` (mainnet) | `crates/core/src/network_params/defaults.rs:29` | ×peers, capped | gauge **broken (0)** | gauge **broken (0)** |
| 11 | Gossip mesh | `mesh_n=12`, `n_low=8`, `n_high=24`, `lazy=12` | `crates/network/src/config.rs:125-128` | constant | — | — |
| 12 | In-memory `UtxoSet` | **NO configured bound** | — | ×UTXO count | gauge **broken (0)** | gauge **broken (0)** |
| 13 | `ProducerSet` / `EpochState` | **NO configured bound** | — | ×producers | not exported | not exported |
| 14 | libp2p per-peer buffers (Yamux/QUIC windows) | **NO explicit cap** | — | ×peers | not exported | not exported |
| 15 | Snap-sync / backfill working set | **NO configured bound** | — | ×blocks-to-sync | n/a | see below |
| 16 | **Memory watchdog** | **`0` = DISABLED by default** | `crates/network/src/config.rs:137`; only enabled via `DOLI_MEMORY_WATCHDOG_BYTES` (`bins/node/src/node/startup.rs:267`) | — | **UNARMED** | **UNARMED** |
| 17 | **cgroup `MemoryMax` / `MemoryHigh`** | **ABSENT on all 11 mainnet units** | `/etc/systemd/system/doli-mainnet-{seed,n1..n12}.service` (ai1/ai4/ai5) | — | **NONE** | **NONE** |
| 18 | `MALLOC_ARENA_MAX` | **not set** ⇒ glibc default `8 × ncores` arenas (16 on ai4/ai5, 32 on ai1), 64 MB heap each | no `Environment=` line in any unit | ×threads × burst | — | — |

### Theoretical configured maximum, per process

```
 48  block_store memtable cap        (types.rs:87)
+ 32  block_store block cache        (open.rs:81)
+ 64  state_db  memtable cap         (types.rs:39)
+ 48  state_db  block cache          (open.rs:95)
+ 10  mempool                        (policy.rs:27)
+  1  gossip SeenCache               (staleness.rs:110)
+  1  table readers (measured 0.55)  (max_open_files=256)
──────
= 204 MB  hard-configured ceiling  (+ ~20 MB file-backed: binary text + mmap)
```

Live RocksDB total, both nodes: n2 = **53.5 MB**, n6 = **45.9 MB** — i.e. RocksDB is at **~25 % of its
own 192 MB cap** and is **identical between the high and low nodes**. RocksDB is not the variable.

### Comparison to observation

| | Configured ceiling | Observed |
|---|---|---|
| Settled nodes (n2, n3, n4, n5, seeds) | 204 MB + ~20 MB file | **137–201 MB** ✅ inside budget |
| Freshly snap-synced nodes (n6–n12) | 204 MB + ~20 MB file | **343–378 MB** ❌ **1.7× over the configured ceiling** |

`/proc/<pid>/smaps_rollup` (sudo), same instant:

| | n6 (ai4) | n2 (ai1) | delta |
|---|---|---|---|
| Rss | 369,896 kB | 204,420 kB | +165,476 kB |
| **Anonymous / Private_Dirty** | **345,320 kB** | **180,312 kB** | **+165,008 kB** |
| Pss_File | 20,047 kB | 19,174 kB | +873 kB |

**99.7 % of the excess is anonymous heap.** Not page cache, not mmap'd SSTs, not the binary, not RocksDB
(which is equal on both). **RSS ≠ live heap** — this is retained-but-free heap held by glibc arenas, or
genuinely live unbounded structures; the exported metric set cannot distinguish the two (see Gaps).

---

## Explicit answer: is 1.9 GB arithmetically expected at 7 producers?

**HOST level — YES.** ai5 = 0.48 GB (Prometheus + Grafana + Alertmanager + OS) + 4 × 0.37 GB
(n9–n12) = **1.96 GB**, measured 2.04 GB. Fully explained by process placement. It has **nothing to do
with producer count** — it is a co-residency arithmetic result.

**PROCESS level — NO, and it never happened.** Peak per-process RSS on the fleet is 378 MB.

**Producer count 5→7 explains none of it.** Walking rows 1–18: **not one configured limit scales with
producer count.** Every RocksDB cap, the mempool cap, `max_peers`, the mesh degree and the SeenCache are
constants. Rows 12/13 (UtxoSet, ProducerSet/EpochState) are the only producer-adjacent structures and
they are unbounded — but a 5→7 producer change adds 2 entries to a producer map, i.e. kilobytes.
**Strong negative result: the configuration cannot produce a producer-count knee at 5→7.** Redirect the
synthesis away from producer-set sizing.

**Do the configured maxima reach 1.9 GB?** No — they sum to 204 MB. Because n6–n12 measure 378 MB,
**an unbounded consumer demonstrably exists** (rows 12–15/18). It is ~165 MB per process today.

---

## Hypotheses

### H1 — The reported "450 MB → 1.9 GB, 4.2×, fleet-wide" is ai5's **host** memory panel, and is the exact arithmetic sum of four newly-started node processes. — conf(0.70, measured)
- **Kill test**: find any host or process whose *own* series went 450 MB → 1.9 GB independent of process
  count; or find ai1/ai2/ai3 stepping at the same time.
- **Result: NOT FOUND → H1 survives.** ai1 0.71→0.83, ai2 0.59→0.76, ai3 0.26→0.29 across the boundary —
  flat. Only ai4 and ai5 stepped, both by exactly (n_processes × per-process RSS).
- **Evidence**: ai5 0.48→2.04 GB (4.25× vs reported 4.2×) at 15:30 UTC; boundary 15:17:50 UTC in
  `n6.log`; 0.48 + 4 × 0.37 = 1.96 GB.

### H2 — n6–n12's 2× per-process RSS is a retained heap high-water mark from snap-syncing 70,753 blocks, not a steady-state requirement (glibc arenas, no jemalloc, no `MALLOC_ARENA_MAX`). — conf(0.60, inferred)
- **Kill test A**: if RocksDB explained it, n6's gauges would exceed n2's. **KILLED THE ALTERNATIVE** —
  n6 RocksDB 45.9 MB *< * n2's 53.5 MB while n6's RSS is 165 MB *higher*.
- **Kill test B**: if it were file-backed (page cache / mmap'd SST), `Pss_File` would differ.
  **NOT FOUND** — 20.0 MB vs 19.2 MB. It is anonymous heap.
- **Kill test C**: if it were simply "recently restarted", n1 (restarted the same 15:17 window) would also
  be high. **FOUND THE OPPOSITE → refines the hypothesis**: n1 = 155 MB, the *lowest* on ai1. n1 already
  had its chain on disk and caught up a handful of blocks; n6 snap-synced 70,753. The discriminator is
  **sync volume, not uptime**. This strengthens H2.
- **Evidence**: `n6.log 15:17:51 [SNAP_SYNC] Fresh node … gap=70753`; `[STARTUP] Block store floor set to
  0 (snap sync gap — block 1 missing)`; zero jemalloc/mimalloc/`global_allocator` in the workspace;
  no `MALLOC_ARENA_MAX` in any unit; 17–21 threads on 2-core hosts ⇒ glibc default cap of 16 arenas.
- **Residual doubt (why 0.60, not higher)**: I cannot separate "retained free heap" from "live
  allocations" — no live-heap metric exists. A genuinely live unbounded structure retained after sync
  would look identical from outside. **This is the single most important open question and it is a
  code/logic question, not a parameters one — flagged cross-domain.**

### H3 — The RocksDB caps are correct and the CF count has NOT grown since INC-I-104. — conf(0.70, measured)
- **Kill test**: count CFs in the deployed code and confirm the cap is DB-scoped rather than per-CF.
- **Result: hypothesis CONFIRMED, INC-I-104 regression ruled out.** `state_db` = 7 CFs
  (`state_db/types.rs:42-51`), `block_store` = 9 CFs (`block_store/types.rs:90-98`) — exactly the counts
  the INC-I-104 comments assume (`open.rs:76` "With 7 CFs", `open.rs:78` "all 9 CFs"). Crucially the cap
  is `set_db_write_buffer_size` (**DB-wide**, `state_db/open.rs:85`, `block_store/open.rs:71`), so adding
  CFs could *not* silently multiply it. Live gauges confirm: `memtable_cap_bytes` = 67,108,864 and
  50,331,648 exactly as coded. **The brief's concern #2 is answered: no silent multiplication.**
- Note the brief's ~152 MB figure is stale: today's caps total **112 MB** memtable + **80 MB** block cache
  = **192 MB** RocksDB ceiling per process.

### H4 — The memory watchdog is still unarmed fleet-wide (INC-I-117 open, unfixed). — conf(0.70, measured)
- **Kill test**: find any node with `DOLI_MEMORY_WATCHDOG_BYTES` set, or a `memory_watchdog_trips_total`
  series.
- **Result: NOT FOUND → confirmed unarmed.** `memory_watchdog_threshold_bytes: 0, // Disabled by default`
  (`crates/network/src/config.rs:137`); the only enable path is the env var
  (`bins/node/src/node/startup.rs:267`); the only `Environment=` line in every mainnet unit is
  `DOLI_RPC_ADMIN_TOKEN`. `memory_watchdog_trips_total` **does not exist** in Prometheus' metric index.
  Separately, `doli-watchdog.timer` (an unrelated RPC health check) is `enabled` but `inactive` and
  `systemctl list-timers` shows **0 timers** on ai1/ai4/ai5.
- **Consequence**: with H4 + row 17 (no `MemoryMax`), the fleet currently has **zero automatic memory
  protection at either the application or the cgroup layer.**

### H5 — Config/binary drift on the new nodes (n6–n12) propagated fleet-wide. — conf(0.05, measured) — **DEAD**
- **Kill test**: compare binaries and unit files across ai1/ai4/ai5.
- **Result: FOUND DISPROVING EVIDENCE → H5 IS DEAD.** All 11 active per-service binaries share md5
  `0782278a61e12c37ec3b6481c5f343df`, all report `6.24.0 (9647b809)`. Unit files are structurally
  identical (`Restart=always`, `RestartSec=10`, `LimitNOFILE=65535`, one `Environment=`, no memory
  directives). Only cosmetic drift: `--auto-checkpoint 4500` (n6) vs `9000` (n1) — RocksDB checkpoints
  are hard-link based, negligible RAM. And "fleet-wide" is false anyway (H1).

### H6 — `ContentStore` (`content_store.rs`, `Options::default()` ⇒ uncapped 64 MB/CF memtables) is a live uncapped RocksDB instance. — conf(0.05, measured) — **DEAD**
- **Kill test**: find a production call site.
- **Result: NONE.** `grep -rn "ContentStore::open\|open_db("` over `crates/` + `bins/` returns only the
  definitions (`crates/storage/src/content_store.rs:36`, `crates/storage/src/lib.rs:202`) and a doc
  comment. Both are dead code in the node path. **Worth noting for the future**: if `ContentStore` is
  ever wired up it ships with *no* `db_write_buffer_size`, *no* block-cache limit and *no*
  `max_open_files` — an INC-I-104 repeat waiting to happen.

---

## Key Evidence Found

1. **Fleet per-process RSS** (`ps -eo rss`, 2026-07-30 20:20 UTC): 137–378 MB, max 378 MB (n7). No 1.9 GB.
2. **`process_resident_memory_bytes` is not exported by doli-node** — only by node_exporter (18–26 MB) and
   Prometheus (235 MB). The brief's suggested metric does not exist for DOLI processes.
3. **ai5 host Apps memory 0.48 → 2.04 GB at 15:30 UTC** (4.25×); ai4 0.13 → 1.21 GB; **ai1/ai2/ai3 flat**.
4. **Boundary = 2026-07-30 15:17:50 UTC**, from `/var/log/doli/mainnet/n6.log` — *today*, not a week after
   genesis. n6–n12 had **no Prometheus series at all from 2026-07-22 to 2026-07-30 15:20** (8 days down).
5. **Seven** processes started, not two; each snap-syncing `gap=70753` blocks.
6. **RocksDB is not the variable**: n6 45.9 MB vs n2 53.5 MB tracked, while n6's RSS is 165 MB higher.
   Caps verified live: `memtable_cap_bytes` 67,108,864 + 50,331,648; `block_cache_capacity_bytes`
   50,331,648 + 33,554,432.
7. **The delta is 99.7 % anonymous heap** (`smaps_rollup`: +165,008 kB Anonymous of +165,476 kB Rss).
8. **CF count unchanged** — 7 (`state_db/types.rs:42-51`) and 9 (`block_store/types.rs:90-98`); caps are
   DB-scoped, so CF growth could not multiply them.
9. **No `MemoryMax`/`MemoryHigh` on any of the 11 mainnet units**; no `MALLOC_ARENA_MAX`.
10. **Memory watchdog disabled by default** (`config.rs:137`) and never enabled in any unit → INC-I-117
    still open, verified at both source and deployment level.
11. **System allocator** — no jemalloc/mimalloc/`global_allocator` anywhere in the workspace.
12. **Grafana panel defect**: `doli-rocksdb-health` → *"Per-node RocksDB resident estimate"* uses
    `sum(memtable + block_cache + table_readers)` with **no `by (instance)`** — a fleet SUM labelled
    "per-node". It stepped 398 → 774 MB at 15:30 today purely because 7 scrape targets reappeared.
13. **Broken gauges**: `doli_chain_height`, `doli_peers_connected`, `doli_mempool_bytes`,
    `doli_mempool_size`, `doli_utxo_set_size`, `doli_storage_bytes` all read **0** on every instance while
    the real chain height is 72,583. Budget rows 8, 10, 12 are unobservable in production.
14. **All nodes on the same tip** (72,583 / `612fa574…`) — no fork, no divergence.
15. **Capacity headroom**: ai5 = 2,184 / 3,819 MB used (1,635 available) hosting 4 producers + Prometheus
    + Grafana + Alertmanager. ai4 = 1,473 / 3,819. ai1 = 1,297 / 15,993.

---

## Causal Chain

| # | Item | Derived? | Derivation |
|---|---|---|---|
| 1 | ai5 host memory 0.48 → 2.04 GB at 15:30 UTC | **YES** | 4 processes started 15:17:50 × 358–375 MB each + 0.48 GB pre-existing base = 1.96 GB predicted; 2.04 GB measured (+4 % page tables/slab) |
| 2 | ai4 host memory 0.13 → 1.21 GB | **YES** | 0.13 + 3 × 0.36 = 1.21 GB predicted = 1.21 GB measured |
| 3 | "4.2×" | **YES** | 2.04 / 0.48 = 4.25 |
| 4 | "Step, not ramp" | **YES** | seven processes started within one 30-min scrape window |
| 5 | "Fleet-wide" appearance | **YES** | (a) the only DOLI dashboard's memory stat is a fleet `sum()` without `by (instance)` — it steps when targets reappear; (b) 7 concurrent snap syncs load every serving node's CPU/gossip simultaneously |
| 6 | Settled-node RSS 137–201 MB | **YES** | inside the 204 MB configured ceiling derived above |
| 7 | n6–n12 RSS 343–378 MB (1.7× the configured ceiling), 99.7 % anonymous | **NO — UNEXPLAINED BY CONFIGURATION** | No configured limit covers it. Correlates with sync volume (70,753 blocks) not uptime (n1 restarted in the same window, synced ~nothing, is at 155 MB). Whether the retained 165 MB is *live* or *free-but-untrimmed glibc arena* cannot be settled from outside the process — needs a heap profile or a jemalloc build. **This is the one genuine open item.** |
| 8 | CPU shape change | **NO — out of lens** | 7 concurrent 70,753-block snap syncs is a large transient; needs the code/logic lens (see INC-I-141 O(N) attestation verification) |

---

## Cross-Domain Signals

- **To Code/Logic**: the only unexplained number is item 7 — **+165 MB of anonymous heap retained after a
  70,753-block snap sync, above every configured cap**. Recommended discriminator: build one node with
  `tikv-jemallocator` + `jemalloc_ctl` stats (or run one node under `MALLOC_ARENA_MAX=2`) and compare RSS
  vs `stats.allocated`. If RSS stays high while `allocated` is low ⇒ allocator retention (benign,
  fixable by allocator swap). If `allocated` is also high ⇒ a genuine unbounded structure retained by the
  snap-sync path (INC-I-139/143 territory).
- **To Connectivity/Sync**: n6–n12 were **down for 8 days** (no metrics 07-22 → 07-30 15:20). The stable
  "3 seeds + 5 producers" week was a *degraded* topology, not a designed one. Also `[STARTUP] Block store
  floor set to 0 (snap sync gap — block 1 missing)` on n6.
- **To Fork/Divergence**: no divergence — all sampled nodes report height 72,583 / `612fa574…`.
- **To everyone**: `doli_chain_height`, `doli_peers_connected`, `doli_mempool_*`, `doli_utxo_set_size`,
  `doli_storage_bytes` are all stuck at **0**. Any hypothesis resting on these Prometheus series is
  resting on nothing.
- **Prior-incident links**: INC-I-104 caps **verified intact** (H3) — do not re-open. INC-I-117
  **confirmed still open and unfixed** (H4). INC-I-102 (ai4/ai5 OOM) — same two hosts, both 4 GB, both
  now unprotected; the shape can recur without a new bug.

---

## Verdict

**Parameters/config domain: CONTRIBUTING — not CAUSAL.**

No parameter is mis-tuned. The RocksDB caps are correct, DB-scoped, verified live, and the CF count has
not grown. Nothing in the configuration scales with producer count, so **producer count 5→7 cannot be the
causal input** through this domain.

What the domain *does* own:

1. **A measurement/aggregation defect, not a memory defect.** The reported symptom is host-level
   arithmetic (4 × 370 MB on a 3.8 GB host) plus a Grafana panel that sums across instances while
   labelled "per-node". Remediation = fix the panel (`sum by (instance)`) and export
   `process_resident_memory_bytes` from doli-node. **Do not "fix" a leak that measurement does not show.**
2. **A real capacity risk.** ai5 runs 4 producers + the entire monitoring stack in 3.8 GB with **1.6 GB
   free, no `MemoryMax`, no memory watchdog, and a system allocator that does not return memory.** That is
   exactly the INC-I-104/INC-I-102 configuration.
3. **One genuinely unbounded consumer**: +165 MB anonymous heap per node retained after snap sync, above
   a 204 MB configured ceiling. Bounded-vs-leak cannot be settled from configuration alone.

**Classification against the brief's bounded-vs-unbounded fork**: the *observed 1.9 GB* is **bounded and
expected** (capacity arithmetic). The *165 MB post-snap-sync residue* is **currently unbounded** and needs
the code lens. These are two different findings and must not be merged.

Under the evidence-floor protocol this is a **VERDICT** for items 1–2 (measured, reproduced arithmetically)
and **PRELIMINARY** for item 3 (mechanism inferred, discriminating test named but not run — mainnet
read-only forbids it).

---

## Gaps

- Could not distinguish **live heap** from **retained-free glibc arena**: no allocator stats exported and
  no heap profiler may be attached under the read-only mainnet constraint.
- Could not measure rows 9, 12, 13, 14 (SeenCache, UtxoSet, ProducerSet/EpochState, per-peer buffers) —
  no metrics exist. The SeenCache bound is asserted from the constant (16,384 @
  `staleness.rs:110`), **not measured** — the brief explicitly warned against restating that claim, and I
  have not verified it. It stays unverified.
- Did not inspect ai2/ai3 unit files (sampled ai1/ai4/ai5); ai1 vs ai4 vs ai5 were byte-identical in
  structure and binary md5, so drift is unlikely but unproven for ai2/ai3.
- The **CPU shape change** is untouched by this lens beyond noting the 7-concurrent-snap-sync trigger.
- Grafana panel identification is inference from an exact arithmetic match, not direct observation of the
  user's screen. If the user confirms the panel, item 1 of the verdict hardens; if not, H1 needs re-testing
  against whatever panel they used.
