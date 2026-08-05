# Domain Investigation Report: Connectivity / Sync

- **INC_ID**: proposed INC-I-146 · **RUN_ID**: 473
- **Investigator**: blockchain-domain-investigator-connectivity
- **Date**: 2026-07-30, measurements taken 20:16–20:30 UTC
- **Status**: **VERDICT** (measured, not inferred)

---

## Domain Lens

Peer counts, connection buffers, gossip propagation and mesh, sync sessions, snap sync,
discovery, partitions, RPC. Key question I was given: *did the network layer's memory
footprint per node step up when N6/N12 joined, and through which structure?*

---

## BLUF — the reported symptom does not exist as described

**There is no ~1.9 GB per-node memory. The maximum RSS of any DOLI node on mainnet right
now is 383 MB.** The "step" is a **host-level** memory rise on ai4 and ai5, and it is
**exactly equal**, to within 0.4 % and 3 % respectively, to the resident memory of **seven
node processes that were started at 2026-07-30 15:31:13 UTC**. The step is **not
fleet-wide**: ai1/ai2/ai3 rose 30–60 MB per node, not 1450 MB.

Connectivity is **RULED OUT** as the cause of the reported 4.2× step by a factor of ~24×
in the top-down arithmetic and ~120× in the bottom-up code arithmetic. It is
**CONTRIBUTING** at a real, small, bounded ~7–9 MB per peer.

---

## Chain Context

| Item | Measured value | Source |
|---|---|---|
| Chain | DOLI mainnet, height 72,587, slot 72,590 | `[HEALTH]` n6.log 20:23:00Z |
| Genesis time | `1784717100` = **2026-07-22 ~10:45 UTC** | n6.log 15:31:13 `Genesis time from chainspec` |
| Binary | v6.24.0 on **all 15** scraped nodes, `commit="unknown"` | `doli_build_info` (Prometheus) |
| Nodes up now | 15 (seed1/2/3 + n1–n12) | `up{job="doli_node"}` |
| Peers per node | **14 on every node** (full mesh), `syncing:false` | `getNetworkInfo` RPC ×9 nodes |
| Producers / bonds | **5 → 12 producers, 5 → 720 bonds** at epoch 197→198 | seed3.log `[HEALTH] snap_bonds=` |
| Server class | ai1/ai2 = 4 cores/16 GB · ai3/ai4/ai5 = 2–? cores/3.8 GB | `nproc`, `free -m` |

---

## What I Don't Understand

1. **Why ai4/ai5 nodes hold ~2.4× the anonymous heap of ai1/ai2 nodes** (320 MB vs 121–166 MB)
   with identical binary, identical peer count (14), identical bond set, identical RocksDB caps
   and *fewer* threads (17 vs 21). This is the only genuinely unexplained residual in the whole
   investigation and it is **not** peer-count-driven. See H4.
2. **How n6 acquired blocks 1…70,828.** Its `blocks/` SST files have mtime **09:47**, which
   predates its first start today (15:17:50), and the current process only wrote heights
   70,829–72,618 (1,790 blocks). The data directory appears to have been **pre-populated out of
   band** (copy/rsync) before the node was started. Nobody's evidence covers this. It is an
   operational-provenance gap, not a code gap.
3. **Whether ai4/ai5 RSS has actually plateaued.** ai5 was flat at 2.28 GB for the last hour
   (19:16→20:16) but ai4 is still creeping (+21 MB across 3 nodes in 5 min, vs +0.8 MB on ai1).
   A 5-minute window cannot separate a decelerating asymptote from a slow leak.
4. **Why `--auto-checkpoint` differs** (4500 on n6, 9000 on n1). Not yet triggered in either
   process, so it cannot explain current RSS, but it is an unexplained config divergence.

---

## Domain Relevance Assessment

**Relevance: LOW** (for the reported symptom) / **MEDIUM** (as the trigger channel).

Reasoning: the connectivity layer did exactly what it should. Peer count doubled 7 → 14, the
mesh formed correctly across all 6 topics, zero connection churn in the last hour, zero gossip
load-shedding events, zero sync failures, all nodes `Synchronized`. The measured per-peer cost
is ~7–9 MB (upper bound, and that bound also absorbs the bond-set growth). Every bounded
collection I could enumerate in the network crate has a verified cap. **The network layer is
not holding the memory, because the memory being asked about does not exist.**

---

## Hypotheses

### H1: Per-peer connection state (Yamux windows, buffers, queues) accounts for the ~1.45 GB step — conf(0.02, measured) — **DEAD**

- **Kill test**: compute the per-peer resident cost required (1.45 GB ÷ 7 new peers) and compare
  against (a) the measured delta on nodes that did *not* restart, and (b) bottom-up code sizing.
- **Kill test result**: **DISPROVED both ways.** See "The Arithmetic" below. Required =
  207 MB/peer; measured upper bound = 8.6 MB/peer (24× short); bottom-up code ceiling =
  ~1.7 MB/peer (120× short).
- **Evidence**: `crates/network/src/transport.rs:18-19` yamux receive window = 262,144 B/stream;
  `getNetworkInfo` peerCount=14 on all 9 nodes probed; ai3 host Δ = +50 MB for a single
  continuously-running process.

### H2: Gossip caches (DOLI `SeenCache`, gossipsub mcache/dedup/mesh) grew with producer or peer count — conf(0.03, measured) — **DEAD**

- **Kill test**: read the cache implementations and prove the bound; check whether instances are
  per-peer or global; check whether the bound is actually enforced (classic bug: pop from the
  order queue but not from the map).
- **Kill test result**: **DISPROVED.** The DOLI `SeenCache` is a **single global instance**
  (`crates/network/src/service/swarm_loop.rs:86`), not per-peer. Capacity
  `SEEN_CACHE_CAPACITY = 16_384` (`crates/network/src/gossip/staleness.rs:110`), TTL 180 s
  (`:99`). The bound is **genuinely enforced** at `staleness.rs:438-448`:

  ```rust
  pub fn record(&mut self, key: [u8; 32], now_unix: u64) {
      while self.order.len() >= self.capacity {
          if let Some(old) = self.order.pop_front() {
              self.entries.remove(&old);        // <-- map AND deque both shrink
          } else { break; }
      }
      if self.entries.insert(key, now_unix).is_none() {
          self.order.push_back(key);            // <-- only push on genuinely-new key
      }
  }
  ```
  `entries.remove` is inside the same loop iteration as `order.pop_front`, and `order.push_back`
  is gated on `insert(...).is_none()`, so `order.len() == entries.len() <= capacity` holds.
  16,384 × ~120 B = **~1.97 MB, peer-count-independent and producer-count-independent.**
- **This satisfies the evidence-assembly instruction to verify commit `1c510919`'s ~2 MB claim by
  reading the code rather than restating it. The claim is CORRECT.**
- Gossipsub's own caches are also negligible at this scale: `history_length(5)` +
  `history_gossip(3)` at 1 s heartbeat (`gossip/config.rs:193,215-216`) = 5 s of messages;
  measured gossip block size is **488 bytes** (n6.log 15:30:01.631489 `[GOSSIP_BLOCK] … size=488`)
  and blocks arrive 1 per 10 s, so the mcache holds ≤ 1 block. `duplicate_cache_time(60s)`
  (`gossip/config.rs:220`) over a measured ~25 msg/min → ~25 IDs resident.

### H3: Sync-session memory from the newcomers' catch-up was never released — conf(0.15, measured) — **MOSTLY DEAD, but note a real latent shape**

- **Kill test**: (a) did the *current* process run a bulk sync at all? (b) do sync buffers get
  cleared? (c) is memory still elevated after sync finished?
- **Kill test result**: **(a) NO.** The current n6 process (started 15:31:13) logged **0**
  `SNAP_SYNC` lines and wrote only heights 70,829–72,618 (1,790 blocks, i.e. normal live
  cadence). The 178 `SNAP_SYNC` lines belong to the **previous, now-dead process**
  (15:17:50–15:30:05), whose memory is gone. So retained sync buffers cannot be holding the
  current 363 MB. Hypothesis largely killed.
- **(b) Buffers ARE cleared** — `pending_headers`, `pending_blocks`, `headers_needing_bodies`,
  `pending_requests`, `body_downloader`, `header_downloader` are all `.clear()`ed at
  `crates/network/src/sync/manager/cleanup.rs:110-119, 329-330, 368-377, 395-406` and at 6 sites
  in `block_lifecycle.rs`.
- **LATENT SHAPE WORTH RECORDING (not the current cause):** `.clear()` in Rust drops elements but
  **retains allocated capacity**. `shrink_to_fit` appears **nowhere** in
  `crates/network/src/sync/manager/`. After a bulk catch-up that fills `pending_blocks:
  HashMap<Hash, Block>` and `pending_headers: VecDeque<BlockHeader>`
  (`sync/manager/types.rs:271-277`), the *table* stays resident for the process lifetime. At an
  observed peak of a few thousand entries this is single-digit MB — **bounded and benign today**,
  but it is the same shape as INC-I-114 and would matter if the pipeline high-water mark ever
  grew by orders of magnitude. `SnapshotData` at `sync/manager/types.rs:159-174` holds
  `chain_state`/`utxo_set`/`producer_set`/`epoch_state_bytes` as full `Vec<u8>` blobs — that is
  the structure to watch as the UTXO set grows, but it is not resident now (no snap sync active).
- **(c)** Memory IS still elevated on ai4/ai5 — but on nodes that never bulk-synced in this
  process, so (c) does not rescue the hypothesis. It points at H4 instead.

### H4: The ai4/ai5 residual is glibc malloc arena high-water marks, not a network structure — conf(0.5, measured for the mechanism, inferred for the trigger)

- **Kill test**: if it were per-peer/network state, it would scale with peer count — but peer
  count is **identical (14)** on every node, so any node-to-node RSS difference is by definition
  *not* per-peer. Check whether the anon memory is in libp2p structures or in allocator arenas.
- **Kill test result**: **Not disproved.** `smaps` for n6 (PID 1840184) shows RSS 369,916 kB of
  which **Anonymous = 345,340 kB (93 %)**, `Pss_File` only 20,041 kB. The anonymous memory sits in
  ~10 mappings of 20–36 MB each, **all at 64 MiB-aligned addresses** (`78efe4000000`,
  `78efe8000000`, `78eff4000000`, `78effc000000`, `78f004000000`, `78f00c000000`, `78f010000000`,
  `78f014000000`, `78f018000000`, `78f020000000`) — the signature of glibc non-main malloc arenas
  (`HEAP_MAX_SIZE` = 64 MiB, mmap'd 64 MiB-aligned). Comparison at identical peer count:

  | node | host | uptime | threads | anon | 64MiB-aligned arenas >20 MB |
  |---|---|---|---|---|---|
  | n1 | ai1 (4c/16G) | 4.8 h | 21 | **121.1 MB** | **3** (35.0, 27.6, 27.5) |
  | n2 | ai1 (4c/16G) | 61.5 h | 21 | **166.2 MB** | **4** (36.0, 31.1, 26.6, 26.6) |
  | n6 | ai4 (2c/3.8G) | 4.8 h | 17 | **319.6 MB** | **~10** (20–36 MB each) |

  n6 has **fewer** threads yet **2.5×** the populated arenas. `MALLOC_ARENA_MAX` is not set in any
  systemd unit. glibc's default `arena_max = 8 × nproc` = 16 on ai4, 32 on ai1 — so the *limit* is
  not the discriminator; the arenas' **high-water marks** are.
- **Honest limitation**: I have the mechanism measured but not the trigger proven. I cannot show
  *why* the ai4/ai5 processes drove more arenas to a higher watermark. This is where the
  synthesizer should look, and it is **not a connectivity structure** — I am handing it over.

### H5: Connection/gossip queues recurred the INC-I-114 unbounded-`VecDeque` shape — conf(0.02, measured) — **DEAD**

- **Kill test**: count load-shed events and check the queue cap.
- **Kill test result**: **DISPROVED.** `grep -cE "GOSSIP_SHED|memory_shed|MEMORY_|watchdog|OOM"` over
  n6.log since restart = **0**. The queue never even filled. Cap is enforced in
  `crates/network/src/service/backpressure.rs` (`enqueue_or_shed`, non-blocking try-send,
  shed-on-full); its own test asserts `tx.max_capacity() == 8` and that 1000 enqueues yield
  exactly 992 drops with no growth (`backpressure.rs:133-153`). `[MEM-CONN-BUDGET] peers=14
  established=14 (in=8 out=6) pending=(in=0 out=0) bootstrap=0 eviction_cooldown=0
  evictions_1m=0` — no pending connections, no evictions.

---

## The Arithmetic (the core deliverable)

### A. Top-down — the host step is fully accounted for by process count

Baseline sample 2026-07-30 **15:16 UTC**; post sample **20:16 UTC**
(`node_memory_MemTotal_bytes - node_memory_MemAvailable_bytes`, step 300 s):

| host | pre (15:16) | post (20:16) | Δ | new doli processes | Σ their RSS (20:17) | residual |
|---|---|---|---|---|---|---|
| **ai4** | 0.48 GB | 1.56 GB | **+1.08 GB** | n6, n7, n8 | 378.1+363.1+343.3 = **1.084 GB** | **−0.004 GB (0.4 %)** |
| **ai5** | 0.85 GB | 2.28 GB | **+1.43 GB** | n9, n10, n11, n12 | 375.3+372.7+368.1+358.1 = **1.474 GB** | **−0.044 GB (3 %)** |

**There is no unexplained remainder.** The entire ai4/ai5 "step" is seven processes existing.

### B. Top-down — what +7 peers actually cost a node that did NOT restart

ai3 is the clean control: **exactly one** DOLI process (seed3, 38.1 h uptime, never restarted,
137.7 MB now).

```
ai3 host used:  0.67 GB (pre) -> 0.72 GB (post)   =>  Δ = +50 MB
peers:          7 -> 14                            =>  Δ = +7 peers
per-peer upper bound = 50 MB / 7 = 7.1 MB/peer
```

ai2 corroborates (3 processes, all 38 h uptime, none restarted):

```
ai2 host used:  1.09 GB -> 1.27 GB  =>  Δ = +180 MB / 3 nodes = +60 MB/node
per-peer upper bound = 60 MB / 7 = 8.6 MB/peer
```

This 7.1–8.6 MB/peer is an **upper bound**, because the same Δ also absorbs bonds 5→720 and
producers 5→12.

```
Required to explain the reported step:  1.45 GB / 7 peers = 207 MB per peer
Measured upper bound:                                        8.6 MB per peer
                                              ------------------------------
                                              SHORT BY A FACTOR OF 24
```

### C. Bottom-up — per-peer cost from the code

| structure | file:line | per-peer bytes | ×7 new peers |
|---|---|---|---|
| Yamux receive window, ~6 concurrent substreams (gossipsub, kad, identify, ping, req-resp sync, status) | `crates/network/src/transport.rs:18-19` — `262_144` B/stream | 6 × 256 KB = **1.5 MB** | **10.5 MB** |
| gossipsub mesh entry × 6 topics (`mesh_n=8, low=6, high=12` for ≤20 peers) | `crates/network/src/gossip/config.rs:147-164`, observed `total_mesh=14 gossip_peers=14` | ~6 × few hundred B | **< 1 MB** |
| 12 per-peer `HashMap<PeerId, …>` (scoring, banned, rate-limit, peer_ips, peer_asns, sync peers, bodies, header blacklist, genesis-mismatch, dial-backoff, eviction, bootstrap, stale) | `scoring.rs:148,150`; `rate_limit.rs:167`; `peer.rs:170,172`; `sync/bodies.rs:38`; `sync/manager/mod.rs:76`; `sync/manager/types.rs:514`; `service/swarm_events.rs:38,40,41,42,43` | 12 × ~200 B ≈ **2.4 KB** | **~17 KB** |
| DOLI `SeenCache` | `gossip/staleness.rs:110` + single instance `service/swarm_loop.rs:86` | **0** (global, not per-peer) | **0** |
| gossip backpressure channel | `service/backpressure.rs` (cap 8, shed-on-full) | **0** (global) | **0** |
| **TOTAL** | | **~1.7 MB/peer** | **~11.5 MB** |

```
Required:   207 MB per peer
Bottom-up:  1.7 MB per peer
            --------------------------------
            SHORT BY A FACTOR OF ~120
```

The design budget in the source agrees: `transport.rs:10` — *"At 50 established connections:
50 × 256KB = 12.5MB/node (INC-I-014)."* At the actual 14 peers that is **3.5 MB**.

**Conclusion of the arithmetic: connectivity/sync per-peer state cannot produce a 1.45 GB step.
It produces roughly 11 MB, and the measured whole-node delta on unrestarted nodes was 50–60 MB.**

---

## Key Evidence Found

1. **Exact step timestamp — established independently of the user's date, and it does NOT match
   their framing.** `systemctl show doli-mainnet-n{6,7,8} -p ActiveEnterTimestamp` on ai4 =
   **Thu 2026-07-30 16:31:13 WEST = 15:31:13 UTC**, `NRestarts=0` (clean manual start, no crash
   loop). Prometheus host memory transitions in the very same 30-min bucket (ai4 15:16 = 0.48 GB →
   15:46 = 0.66 → 16:16 = 1.33). **The boundary is ~4.8 hours ago, not "roughly a week after
   genesis".**

2. **Seven nodes started, not two.** `up{job="doli_node"}` over 9 days: n6, n7, n8, n9, n10, n11,
   n12 all transition 0→1 at the same scrape (`07-30_15:46`), having been 0 since `07-22 10:46`.
   Since genesis time is `1784717100` = 2026-07-22 ~10:45 UTC, **these seven nodes were down for
   the entire life of this chain** and were all started together today.

3. **The step is NOT fleet-wide.** Host memory Δ across the boundary:
   ai4 +1.08 GB · ai5 +1.43 GB · ai2 +0.18 GB · ai1 +0.13 GB · ai3 +0.05 GB.
   Normalised per node: ai4/ai5 = new processes; **ai1 = +32 MB/node, ai2 = +60 MB/node,
   ai3 = +50 MB**. That is a 10–40 % rise on already-running nodes, not 320 %.

4. **No node is near 1.9 GB.** Direct `ps -eo rss` on all five servers at 20:17 UTC:
   ai1 seed=160.2 n1=154.6 n2=199.6 n3=136.9 · ai2 seed=181.0 n4=188.8 n5=200.9 ·
   ai3 seed=137.7 · ai4 n6=363.1 n7=378.1 n8=343.3 · ai5 n9=372.7 n10=368.1 n11=375.3 n12=358.1.
   **Max = 378.1 MB.**

5. **Connectivity health is clean.** `getNetworkInfo` on 9 nodes: `peerCount:14, syncing:false`
   uniformly. n6.log 20:23:13 `[MEM-CONN-BUDGET] peers=14 established=14 (in=8 out=6)
   pending=(in=0 out=0) bootstrap=0 eviction_cooldown=0 evictions_1m=0`. `[GOSSIP_MESH] blocks=10
   txs=11 producers=11 attestations=11 heartbeats=12 headers=10 | total_mesh=14 gossip_peers=14`.
   **Zero** connection-churn events in the last hour. **Zero** `GOSSIP_SHED`. `sync_fails=0`,
   `state="Synchronized"`, `rollback_depth=0`.

6. **RocksDB is arithmetically ruled out for the residual.** `doli_rocksdb_block_cache_bytes`
   3.4–5.6 MB and `doli_rocksdb_memtable_bytes` 3.9–6.6 MB on **all 15 nodes**, against caps
   50.3 MB / 67.1 MB. `doli_rocksdb_table_readers_bytes` = 0. Total RocksDB ≈ 8–12 MB/node,
   near-identical everywhere. INC-I-104's static ~152 MB cap is intact and the column-family
   count has not grown.

7. **The real count change is bonds, not peers.** seed3 (continuously running, never restarted)
   `[HEALTH]` line: `2026-07-30T16:00 snap_epoch=197 snap_bonds=5 snap_producers=5` →
   `2026-07-30T16:46 snap_epoch=198 snap_bonds=720 snap_producers=12`. **Bonds ×144, producers
   ×2.4, peers ×2.** If anything scales badly with a count here, the 144× input dominates the 2×
   input by two orders of magnitude. **This is not my domain — flagged to the synthesizer.**

8. **Metrics instrumentation is largely dead — this is why nobody could see the truth.**
   `doli_peers_connected`, `doli_chain_height`, `doli_blocks_behind`, `doli_is_syncing`,
   `doli_utxo_set_size`, `doli_mempool_bytes`, `doli_active_producers` all report **0** on every
   node while the chain is demonstrably at height 72,587 with 14 peers. The metric is declared at
   `bins/node/src/metrics.rs:97` but never updated. **No DOLI process exports
   `process_resident_memory_bytes` at all** — only node_exporter and Prometheus do. Therefore
   **the Grafana panel the user read cannot have been per-node RSS**; it must have been
   host-level `node_memory_*`. That fully explains the misreading: ai5 host went 0.85 → 2.28 GB
   and ai4's pre-step baseline was 0.48 GB.

---

## Causal Chain

| # | Item | Derived? | Derivation |
|---|---|---|---|
| 1 | Seven nodes (n6–n12), down since chain genesis, started at 2026-07-30 15:31:13 UTC | YES | `systemctl show … ActiveEnterTimestamp` on ai4; `up{job="doli_node"}` 0→1 at 07-30 15:46 for all 7 |
| 2 | ai4 host used memory +1.08 GB, ai5 +1.43 GB | YES | Prometheus `MemTotal−MemAvailable`, 15:16 vs 20:16 |
| 3 | That rise = the RSS of exactly those 7 processes | YES | Σ RSS = 1.084 GB (ai4) / 1.474 GB (ai5); residual 0.4 % / 3 % |
| 4 | User read a **host-level** Grafana panel and interpreted it as per-node RSS | YES | No DOLI process exports `process_resident_memory_bytes`; only `node_*` and `prometheus`/`node_exporter` `process_*` series exist |
| 5 | Peer count on existing nodes 7 → 14 | YES | 8 nodes up pre-boundary, 15 post; `getNetworkInfo` peerCount=14 on all probed nodes |
| 6 | Existing nodes gained 50–60 MB, i.e. ≤8.6 MB/peer | YES | ai3 (1 process, unrestarted): +50 MB ÷ 7; ai2 (3 processes, unrestarted): +180 MB ÷ 3 ÷ 7 |
| 7 | Per-peer state cannot produce 1.45 GB | YES | 207 MB/peer required vs 8.6 MB measured vs 1.7 MB from code — 24×/120× short |
| 8 | ai4/ai5 nodes hold 320 MB anon vs 121–166 MB on ai1, at identical peer count | **NO — UNEXPLAINED** | Measured (smaps, 64 MiB-aligned glibc arenas), mechanism identified, **trigger not proven**. Not peer-driven (peers identical). Route to code/parameters domain. |
| 9 | Bonds 5 → 720 at epoch 197→198 | YES (observation) | seed3 `[HEALTH]` 16:00 vs 16:46 |
| 10 | Whether item 9 drives item 8 | **NO — UNEXPLAINED** | Out of my domain. But it is a 144× count change vs the 2× peer change I was asked about. |

---

## Cross-Domain Signals

1. **→ Parameters / Consensus (HIGHEST PRIORITY).** `snap_bonds` went **5 → 720** and
   `snap_producers` **5 → 12** at epoch 197→198 (seed3.log 16:00 → 16:46). The brief framed the
   causal input as "producer-set size 5→7". **It is actually 5→12 producers and 5→720 bonds.**
   Any O(N) or O(N²) structure keyed on bonds saw a 144× input change today. That dwarfs the 2×
   peer change I was asked to test.

2. **→ Code / Memory management.** The unexplained residual (item 8) is glibc malloc arena
   high-water marks: n6 anon 319.6 MB across ~10 64 MiB-aligned arenas vs n1 anon 121.1 MB across
   3, at identical peer count, identical bonds, identical binary, and *fewer* threads. Note
   `MALLOC_ARENA_MAX` is unset in every systemd unit. Whoever owns the code lens should look at
   allocation churn in the attestation/bond path, not at libp2p.

3. **→ Consensus (CPU-shape question).** Attestations are being sent as **unicast
   request/response**, not only gossip: `[SYNC_DEBUG] Sending sync request to peer=…,
   request=DirectAttestation { data: [...] }` (`crates/network/src/service/command_handling.rs`),
   each answered with `Block(None)`. Log-kind census of n6's last 20k lines: `[DIRECT_ATTEST] 949`,
   `[DIRECT_ATTEST_RECV] 885`, `[ATTEST_DECODE] 730`. This fan-out scales with producer count,
   which went 5→12 today. **That is a far more plausible driver of a CPU shape change than
   anything peer-related**, and it matches open INC-I-141 (O(N) attestation verification).

4. **→ Observability (operational).** Seven `doli_*` gauges are permanently 0 and no node exports
   process RSS. The fleet has **no per-process memory metric at all**, which is why an
   8-investigator swarm was needed to establish a fact that `ps` answers in one second. Also
   relevant to open **INC-I-117** (memory watchdog dormant): there is no
   `memory_watchdog_trips_total` series in Prometheus, and n6 logged 0 watchdog/OOM events.

5. **→ Operations (provenance gap).** n6's `blocks/` SST files carry mtime **09:47**, predating
   its 15:17:50 first start, while the current process wrote only heights 70,829–72,618. The data
   directory was pre-populated out of band. Somebody should confirm what was copied where before
   any conclusion is drawn about n6–n12's local state.

6. **→ Capacity (live risk, read-only observation).** ai5 is at **2.28 GB used of 3.8 GB (60 %)**
   while also hosting Prometheus (230.7 MB), Grafana and Alertmanager. ai4 is at 1.56 GB of
   3.8 GB and still creeping (+21 MB across 3 nodes in the 5 min 20:21→20:26, vs +0.8 MB across
   4 nodes on ai1). ai5's rise flattened over 19:16→20:16, so this looks asymptotic rather than
   linear — but the headroom is thin and the `HighMemory` alert only fires at >90 %.

---

## Answer to My Assigned Key Question

> *Did the network layer's memory footprint per node step up when N6/N12 joined, and through
> which structure? Why would unchanged nodes each gain ~1.45 GB because two peers joined?*

**They didn't.** Unchanged nodes gained **50–60 MB**, not 1.45 GB — a 10–40 % rise, entirely
consistent with doubling peer count from 7 to 14 at a measured ≤8.6 MB/peer. The 1.45 GB figure
is host-level memory on ai4/ai5 and is 100 % accounted for (to within 0.4 % and 3 %) by seven new
node processes, not by peer-driven growth on existing nodes.

The premise of the question — "two peers joined" — is also wrong on the facts: **seven** nodes
started, and they were down since genesis, not "already running and unchanged".

---

## Verdict

**CONNECTIVITY / SYNC DOMAIN: RULED OUT as causal for the reported 4.2× step.
CONTRIBUTING for a real, bounded, ~50–60 MB per-node rise on existing nodes.**

Settling arithmetic:
- Required per-peer cost to explain 1.45 GB = **207 MB/peer**.
- Measured upper bound from unrestarted control nodes (ai3, ai2) = **7.1–8.6 MB/peer**.
- Bottom-up from source (yamux 256 KB × ~6 streams + mesh + 12 per-peer maps) = **~1.7 MB/peer**.
- Shortfall: **24×** (measured) to **120×** (code).
- And the per-node step being asked about does not exist: **max node RSS on mainnet = 378 MB.**

Every bounded structure in the network crate that I could enumerate has a verified cap:
`SeenCache` 16,384 entries ≈ 2 MB global (`gossip/staleness.rs:110`, enforced at `:438-448`);
gossip queue cap 8 with shed-on-full (`service/backpressure.rs`, 0 sheds observed); yamux window
262,144 B (`transport.rs:18`); gossipsub `history_length(5)`/`duplicate_cache_time(60s)`
(`gossip/config.rs:215,220`); sync pipeline cleared at 12 sites in
`sync/manager/{cleanup,block_lifecycle}.rs`. **INC-I-114's unbounded-`VecDeque` shape has not
recurred.**

---

## Gaps

1. I could not prove the *trigger* for the ai4/ai5 anon-heap residual (item 8) — only the
   mechanism (glibc arena watermarks) and the negative (it is not peer-count-driven).
2. I could not obtain per-process RSS **history**; no such metric is exported. All per-process
   numbers are spot measurements from 20:17–20:26 UTC plus a 5-minute delta. Longer-horizon
   growth is inferred from host-level series, which on ai4/ai5 is confounded by the processes
   having started only 4.8 h ago.
3. Bounded-vs-leak for ai4/ai5 is **not settled, but leans bounded**. ai5 host was flat at
   2.28 GB across 19:16 / 19:46 / 20:16. I sampled ai4 process RSS three times over 10 minutes:
   Σ(n6,n7,n8) = 1082.7 MB (20:21) → 1103.9 MB (20:26) → 1110.8 MB (20:31), i.e. **+21.2 MB then
   +6.9 MB — decelerating**, against ai1's Σ = 651.3 → 652.1 → 652.9 MB (flat). Deceleration is
   consistent with an asymptote rather than a linear leak, but 10 minutes cannot prove it.
   Needs a 12–24 h observation window. **I explicitly do not claim it is a leak, and I explicitly
   do not claim it is bounded.**
4. I did not read `docs/postmortems/2026-07-21-inc-i-143-snapsync-cascade.md`; the snap-sync path
   ran only in a now-dead process, so it fell out of scope once that was measured.
5. I did not audit the external producer fleet (family, folsi, nano, caraquita, martino, copeton,
   joser, mguilarte, leandro). Their `doli_node` scrape targets have been **down since
   2026-07-22 07:16–07:46** — i.e. they are not on this chain. Their host memory is flat at
   0.39–0.53 GB throughout, so they contribute nothing to the signal, but their absence from the
   chain may itself warrant attention.
