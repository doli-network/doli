# INC-I-102 — Root Cause Report (0.95 confidence, measured)

**Incident:** Mainnet ai4/ai5 cascade — sustained disk-IOPS read pressure, OOM cascade on ai5, fork-observability deploy regression.
**Branch:** main (commit `c5a539f8`).
**Status:** Diagnosis closed. Resolution NOT applied. Confidence promoted 0.75 → 0.95.
**Report date:** 2026-05-30 (Saturday).

---

## 1. Verdict

**Root cause:** The INC-I-090 fork-observability deploy (binary built on commit `9cee7cb4`, deployed to mainnet 2026-05-27 00:00 UTC, replacing a pre-deploy binary from 2026-04-21) introduced a **fifth RocksDB instance per node** (`DiagnosticLedger` at `<data_dir>/diagnostics/`) opened with `rocksdb::Options::default()` and **no explicit memory bounds**. This DB instance allocates:

1. **32 MB block-cache (per-CF default)** — measured at 99.7% occupancy on n12 (`Block cache LRUCache@0x585fb220b1e0 capacity: 32.00 MB ... usage: 31.90 MB ... 8060 DataBlock entries`).
2. **Up to 128 MB of memtables** (`write_buffer_size=64 MB` × `max_write_buffer_number=2`, both defaults).
3. **Table reader memory** for up to 64 open SST files.

Combined with the **D1 emit rate of ~1 RecoveryClassifyCall per second** (commit `1147cc9a`) and the **D4 in-node consumer that runs a full CF scan every 30s** (commit `f01474e1`, calls `query_recent_impl` which uses `IteratorMode::Start` — full scan even with a 5-minute window), the diagnostic_ledger DB instance adds ~100–160 MB of resident RSS per node and continuously thrashes its own block-cache (oscillating from 0.08 KB → 31.90 MB on every monitor + pruner cycle).

The 4-node-on-3.7-GB-host ai5 layout was the breaking point. Each node's steady-state RSS climbed monotonically to ~900–1000 MB; total RSS reached 3.0+ GB on a 3.7 GB host, leaving ≤200 MB headroom. Any transient allocation (block apply with large UTXO churn, full-CF scan, classifier deserialization spike) tipped the kernel into `global_oom`, which selected the heaviest non-trigger process. n11 was killed 2026-05-29 11:33 UTC (anon-rss 958 MB), n9 was killed 2026-05-30 06:10 UTC (anon-rss 1.07 GB, peak 1.2 GB per systemd memory accounting).

The sustained read IOPS on the Hetzner host panels — ai5 to ~2,000 IOPS, ai4 to ~700 IOPS — is **not write amplification, not new block traffic, not network sync**. It is **block-cache thrashing on the main BlockStore + state_db RocksDB instances**, caused by:
- The cf_events SST files being continuously pinned hot by the 30s monitor scan
- OS page-cache pressure from 4 concurrent RocksDB processes each holding 600+ MB of per-CF block-cache reservations
- Memtable rotation forcing periodic SST flushes that compete for the same I/O bandwidth

ai4 (3 nodes, same binary, identical RocksDB config) has **zero OOM events** because the 3-node × 3.7-GB layout retains enough headroom; the lower (~700 IOPS) sustained reads on ai4 confirm the cache-thrashing mechanism is present but not yet tipping into OOM.

---

## 2. Evidence

### 2.1 Binary diff confirms diagnostic_ledger is NEW post-deploy

| Binary | Date (mtime) | `diagnostic_ledger` symbol count |
|--------|--------------|----------------------------------|
| `/mainnet/bin/doli-node` | 2026-04-21 22:36 +0100 | **0** |
| `/mainnet/bin/backup/doli-node-n12` | 2026-05-26 23:59 +0100 | 30 |
| `/mainnet/bin/doli-node-n12` (running) | 2026-05-27 00:00 +0100 | 40 |

Source: `strings /mainnet/bin/... | grep -c diagnostic_ledger\|DiagnosticLedger\|diagnostic_monitor`.

Pre-deploy binary (April 21) has **zero** diagnostic_ledger code. The May 27 deploy added the entire INC-I-090 cluster (commits `bccb1bdf` → `9cee7cb4`).

### 2.2 RocksDB cache configuration — code-confirmed defaults

Five RocksDB instances per node, each opened with `rocksdb::Options::default()`:

| Instance | Source | CFs | max_open_files | Explicit block_cache | Explicit write_buffer |
|----------|--------|-----|----------------|----------------------|------------------------|
| BlockStore | `crates/storage/src/block_store/open.rs:17-40` | 9 | 256 | NO (default 32 MB/CF) | NO (default 64 MB × 2) |
| StateDb | `crates/storage/src/state_db/open.rs:15-36` | n | 256 | NO | NO |
| UtxoSet (rocks) | `crates/storage/src/utxo_rocks.rs:38-44` | m | default | NO | NO |
| ContentStore | `crates/storage/src/content_store.rs:27-33` | 2 | default | NO | NO |
| **DiagnosticLedger** | `crates/storage/src/diagnostic_ledger/mod.rs:44-50` | 1 | 64 | **NO** | **NO** |

Direct RocksDB LOG measurement on n12 (`/mainnet/n12/data/<db>/LOG`):

- `blocks` (BlockStore): per-CF cache `capacity: 32.00 MB`, peak `usage: 31.87 MB`
- `state_db`: per-CF cache `capacity: 32.00 MB`, peak `usage: 25.25 MB`
- `utxo_store`: per-CF cache `capacity: 32.00 MB`, peak `usage: 3.82 MB`
- **`diagnostics`** (cf_events): per-CF cache `capacity: 32.00 MB`, **oscillates between 0.08 KB and 31.90 MB** every monitor + pruner cycle, current `8060 DataBlock entries` = **97.59% cache portion**

### 2.3 Per-node memory layout (measured via /proc/PID/smaps, 2026-05-30 14:42 UTC)

| Node | Host | Uptime | RSS | VmSize | Anon RSS | ~64 MB anon mappings |
|------|------|--------|-----|--------|----------|----------------------|
| n6  | ai4 | 3d 14h | 1,483 MB | 2,224 MB | n/a (smaps restricted) | **19** |
| n7  | ai4 | 3d 14h | 872 MB | 1,466 MB | — | — |
| n8  | ai4 | 3d 14h | 990 MB | 1,530 MB | — | — |
| n9  | ai5 | 8h 31m | 421 MB | 1,368 MB | 396 MB | 3 |
| n10 | ai5 | 3d 14h | 925 MB | 1,530 MB | 887 MB | 6 |
| n11 | ai5 | 1d 3h | 781 MB | 1,461 MB | 748 MB | 5 |
| n12 | ai5 | 3d 14h | 985 MB | 1,563 MB | 947 MB | 7 |

**Memtable count grows monotonically with uptime.** The 64 MB anon mappings are RocksDB write buffers (`write_buffer_size = 64 MB` default). At 3.5d uptime: 6–7 (typical) or 19 (n6, outlier — likely the producer with heaviest write history pre-INC-I-083). Each 64 MB mapping is fully touched (RSS ≈ virt = 65,520 KB per mapping).

### 2.4 OOM kill events (journalctl, ai5)

```
May 29 11:33:39 kernel: tokio-rt-worker invoked oom-killer: gfp_mask=0x140cca, order=0
May 29 11:33:39 kernel: oom-kill:constraint=CONSTRAINT_NONE, cpuset=doli-mainnet-n12.service, global_oom, task=doli-node-n11, pid=854926
May 29 11:33:39 kernel: Killed process 854926 (doli-node-n11) total-vm:1563432kB anon-rss:958092kB

May 30 06:10:41 kernel: systemd invoked oom-killer: gfp_mask=0x140cca, order=0
May 30 06:10:41 kernel: oom-kill:constraint=CONSTRAINT_NONE, global_oom, task=doli-node-n9, pid=854924
May 30 06:10:41 kernel: Killed process 854924 (doli-node-n9) total-vm:1640492kB anon-rss:1076580kB
May 30 06:10:41 systemd[1]: doli-mainnet-n9.service: Consumed 4h 12min 47.859s CPU time, 1.2G memory peak
```

**ai4: zero OOM events since 2026-05-20** (journalctl confirms).

### 2.5 Read amplification — main DB, not diagnostic_ledger

Per-process `/proc/PID/io` lifetime totals on ai5 (2026-05-30 14:42 UTC):

| Node | Read bytes (GB) | Write bytes (GB) | Hours uptime | Read rate |
|------|-----------------|-------------------|--------------|-----------|
| n9 | 14.5 | 0.89 | 8.5 | 470 KB/s |
| n10 | 181.9 | 9.02 | 86 | 600 KB/s |
| n11 | 115.5 | 2.98 | 27 | 1.2 MB/s |
| n12 | 190.6 | 9.17 | 86 | 600 KB/s |

Live `iostat -x` on ai5 sda: sustained 729 KB/s read average over 10s, 924 KB/s over the next 5s.

INC-I-101 measurement: `cumulative read 0.01 GB` on cf_events over 3.5 days → diagnostic_ledger DB itself is **I/O-cold**. The 600+ KB/s sustained read is on `blocks/` and `state_db/` SST files in the **main BlockStore process**, driven by block-cache eviction (cache capacity is reserved but main DB hot-set exceeds it under memory pressure).

### 2.6 Code paths confirmed for memory amplification

1. **`crates/storage/src/diagnostic_ledger/mod.rs:44-50`** — `DiagnosticLedger::open()` uses `rocksdb::Options::default()`. No `set_db_write_buffer_size`, no `set_block_based_table_factory` with explicit cache. The only override is `set_max_open_files(64)`.
2. **`crates/storage/src/diagnostic_ledger/queries.rs:118`** — `query_recent_impl` uses `rocksdb::IteratorMode::Start` (full CF scan), then filters by `timestamp_ms` in memory. Window filtering is post-scan, NOT seek-based.
3. **`bins/node/src/node/diagnostic_monitor.rs:25`** — `DIAGNOSTIC_MONITOR_INTERVAL_SECS: u64 = 30`. Triggers `check_for_actionable_alerts(ledger, 300, ...)` every 30s.
4. **`bins/node/src/node/periodic.rs:1035-1039`** — `check_for_actionable_alerts(ledger, 300, &mut self.last_diagnostic_alerted)`. 5-minute filter window is in-memory only.
5. **`crates/storage/src/diagnostic_ledger/mod.rs:148-253`** — `prune()` uses `IteratorMode::Start`, collects ALL events into `Vec<(Vec<u8>, DiagnosticEvent)>`, partitions into stale/fresh, groups by correlation_key, sorts by timestamp. Runs every 60s per pruner config.
6. **Emit rate (D1, commit `1147cc9a`)** — RecoveryClassifyCall emitted ~1/sec on every classifier iteration, regardless of action. INC-I-091 (`c5a539f8`, 2026-05-30 10:55 UTC) reduced false-positive classification but NOT the emit rate.

### 2.7 Timing correlation

| Event | Timestamp (UTC) | Hours from deploy |
|-------|------------------|--------------------|
| Binary deployed on ai5 (n12) | 2026-05-27 00:00:37 (mtime, local +0100 = 2026-05-26 23:00 UTC) | 0 |
| IOPS spike onset (Hetzner host panel) | 2026-05-29 01:35 UTC | ~50.6 h |
| n11 OOM-killed | 2026-05-29 11:33 UTC | ~60.5 h |
| n9 OOM-killed | 2026-05-30 06:10 UTC | ~79 h |

The 50-hour delay between deploy and visible IOPS spike matches the time for per-node RSS to climb to ~800–900 MB and for the diagnostic_ledger event store to accumulate enough SST files that the 30s monitor scan begins evicting main-DB cache pages from the OS page cache.

---

## 3. Counter-evidence considered and refuted

1. **"The cf_events scan is too cheap to cause IOPS pressure"** — REFUTED but PRESERVED as latent. INC-I-101's measurement (0.01 GB cumulative read on cf_events / 3.5d) confirms the DB itself is I/O-cold. The mechanism is NOT direct disk reads from cf_events; it is **OS-page-cache eviction on the main BlockStore SST files** caused by the diagnostic_ledger DB instance's resident-memory footprint (block_cache reservation + memtables + open-file table readers) competing for the same physical RAM.
2. **"It might be a memory leak in non-RocksDB code"** — REFUTED. Anonymous mappings on n12 are dominated by ~64 MB chunks (7 mappings = 448 MB) and a single 110 MB segment + 93 MB heap. The 64 MB chunks are RocksDB write buffers; this is configuration, not a leak.
3. **"D1 emit rate alone is the problem"** — INSUFFICIENT. Even with D1 disabled, the DB instance still allocates ~100–160 MB resident per node from the default `Options::default()` config. D1 + D4 + pruner amplify the cost but don't create the baseline pressure.
4. **"ai4 doesn't OOM, so the bug is host-specific"** — REFUTED. ai4 has the same binary and the same per-node memory signature; ai4 has 3 nodes instead of 4, which leaves ~474 MB headroom (vs. ai5's 168–424 MB). Sustained read IOPS on ai4 is ~700 (the same mechanism, milder). ai4's n6 already shows 19 × 64 MB mappings = 1.2 GB of memtable accumulation; ai4 is on track to OOM, just slower.
5. **"INC-I-091 (`c5a539f8`) caused the regression"** — REFUTED for IOPS pressure (deployed 2026-05-30 10:55 UTC, after IOPS spike was already sustained for 33+ hours). INC-I-091 fixed cry-wolf false positives, did NOT change emit rate or scan cost. The regression originated in the May 26-27 deploy (commits `bccb1bdf` → `9cee7cb4`), not the May 30 hotfix.

---

## 4. Confidence assessment

**0.95** — promoted from 0.75.

Supporting:
- Direct measurement of 99.7%-full cf_events block_cache on running n12 (LOG file extract).
- Direct measurement of memtable mapping count per node correlating with uptime (smaps).
- Binary symbol-table diff confirms diagnostic_ledger was absent from pre-deploy binary.
- OOM events match predicted timing window (50–80h post-deploy).
- Code path traced end-to-end from `DiagnosticLedger::open()` → `Options::default()` → `LRUCache@... capacity: 32.00 MB`.

What would push 0.95 → 0.99 (not done — would require operational action):
- A/B measurement: revert one ai5 node to the April 21 binary, capture RSS-vs-time delta over 24h. Operationally risky on mainnet.
- Direct attribution of OS-page-cache eviction to specific SST files. Would require `pcstat` or `vmtouch` instrumentation under live traffic.

Confidence is bounded at 0.95 because the **mechanism step** "diagnostic_ledger memory footprint causes OS page cache eviction" is inferred from the cache-thrashing measurement (cf_events block_cache oscillating 0.08 KB ↔ 31.90 MB) and the sustained read IOPS on host-level panels, not from a direct measurement of which SST pages got evicted from page cache.

---

## 5. Why ai5 OOMs and ai4 does not (open question Q4 closed)

ai5 layout: 4 nodes × ~900 MB steady RSS = 3.6 GB / 3.7 GB total → 100–400 MB headroom. ANY transient spike → OOM.
ai4 layout: 3 nodes × ~1.1 GB steady RSS = 3.3 GB / 3.7 GB total → 474 MB headroom. Same memory pressure, larger margin.
ai4 will OOM eventually unless the underlying config is fixed. n6 (1.48 GB RSS, 19 × 64 MB mappings) is on a trajectory that will cross threshold within days.

---

## 6. Why n11 is the cache-starved outlier (Q3 closed)

n11 had the highest read-amplification ratio (115.5 GB read / 27h = 1.2 MB/s, vs n10/n12 at 600 KB/s) because:
1. n11 was OOM-killed at 2026-05-29 11:33 and systemd respawned it 11 seconds later.
2. Post-respawn, n11's RocksDB cold-cache filling forces SST re-reads from disk for every block validation, UTXO lookup, and producer-set query.
3. Other nodes (n10/n12) have had 3.5d to warm their caches; n11 has only 27h and is still in the warm-up phase competing for memory with already-warm neighbors.
This is not n11-specific; any freshly-respawned node on ai5 would exhibit the same pattern.

---

## 7. Why N12 "dawned with snap-sync" (user's reported observation)

Not corroborated in the log file. n12's `/var/log/doli/mainnet/n12.log` between 07:00–07:03 UTC today shows `state="Idle"` and `net_tip_h == best_height = 322098`, with normal `[STATE_ROOT]` and `[HEALTH]` cadence. No `snap-sync` lifecycle events fired today. The `snap_epoch=894 snap_bonds=24429 snap_producers=42` fields in the HEALTH log are **snapshot-state metadata**, not snap-sync triggers — they show the last persisted snapshot, not an active sync.

If n12 was observed lagging from RPC `getChainInfo` earlier, that could be the result of memory pressure causing transient block-apply slowdowns (block_cache misses → SST reads → 1–2 second delays per block), not an actual snap-sync recovery. Resolution: re-check `getChainInfo` now (currently h=324514, version 6.22.1, matching peers) → caught up.

---

## 8. What is NOT the cause (alternative hypotheses ruled out)

- ❌ INC-I-091 `c5a539f8` (deployed AFTER the IOPS spike started).
- ❌ INC-I-090 D1 alone (necessary but not sufficient; the DB instance baseline is the larger contributor).
- ❌ INC-I-101's scan amplifier (latent risk, not yet active — store is 38–50 MB, not hundreds).
- ❌ A code memory leak (the 64 MB anon mappings are RocksDB-shaped, not heap-grown).
- ❌ Host-level resource exhaustion outside the doli processes (free -m shows nothing else consuming memory).
- ❌ Snap-sync recovery on N12 (log file does not show any snap-sync lifecycle event today).

---

## 9. Resolution preview (NOT applied — diagnosis-only per user request)

Three changes, smallest blast radius first. None require a consensus activation height (no block content change).

1. **Bound DiagnosticLedger memory explicitly** in `crates/storage/src/diagnostic_ledger/mod.rs:44-50`:
   ```rust
   let mut opts = rocksdb::Options::default();
   opts.create_if_missing(true);
   opts.create_missing_column_families(true);
   opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
   opts.set_max_open_files(64);
   opts.set_db_write_buffer_size(8 * 1024 * 1024);  // 8 MB total memtable budget (was 128 MB default)
   opts.set_write_buffer_size(4 * 1024 * 1024);     // 4 MB per memtable
   opts.set_max_write_buffer_number(2);
   let mut block_opts = rocksdb::BlockBasedOptions::default();
   let cache = rocksdb::Cache::new_lru_cache(4 * 1024 * 1024);  // 4 MB block cache (was 32 MB default)
   block_opts.set_block_cache(&cache);
   opts.set_block_based_table_factory(&block_opts);
   ```
   Estimated savings: ~120 MB resident per node → ~480 MB across 4 ai5 nodes. Brings ai5 from ~200 MB headroom to ~680 MB headroom.

2. **Switch `query_recent_impl` to time-prefixed reverse iteration** (`crates/storage/src/diagnostic_ledger/queries.rs:118`) so the 30s monitor scan is O(window) instead of O(retention). Reduces transient allocations during scan.

3. **Tighten pruner defaults** — retention 30d→24h, cap 100,000→10,000. Diagnostic events are live signals, not historical evidence. Aligns with INC-I-101's recommendation.

**Operational immediate (separate from code fix):** Resize ai5 CX23 → CX31 (3.7 GB → 8 GB) OR move one node off ai5. This stops the OOMs today regardless of the code fix landing.

---

## 10. Refuted from prior hypothesis space

- INC-I-101 hypothesized "scan amplifier on growing event store with 30-day retention" — REFUTED as active cause (store is 38–50 MB; cumulative cf_events disk read is 0.01 GB / 3.5d). PRESERVED as latent risk.
- "ai5 OOM is due to a single bad commit (9cee7cb4)" — REFUTED narrowly. 9cee7cb4 is the LATEST piece; the architectural pressure (5 RocksDB instances × `Options::default()` × 32 MB block cache per CF) was introduced cumulatively across the May 20 → May 27 deploys, the LARGEST single contributor being the introduction of the diagnostic_ledger DB instance itself (commit `bccb1bdf`).

---

## 11. Invariants to extract on resolution

- **INV-OBSERVABILITY-001**: Every RocksDB instance opened in the storage layer MUST set `set_db_write_buffer_size` and `set_block_cache` explicitly. `Options::default()` is BANNED for production DBs. Regression test: structural check that opens each DB and asserts non-default options.
- **INV-OBSERVABILITY-002**: Any background scan over a RocksDB CF with retention > 1h MUST use time-prefixed seek + break-on-cutoff, NEVER `IteratorMode::Start` + filter. Regression test: query_recent wall time at synthetic 1M-event store size < 100ms.
- **INV-OBSERVABILITY-003**: Per-node steady-state RSS on the structural mainnet fleet MUST stay below `physical_memory / (node_count + 1)`. Monitoring signal: alert when RSS_percentile_99 > host_total_memory / (node_count + 1) for >10 minutes.

---

*Evidence sources:* SSH measurements on ai4 (CX23) and ai5 (CX23) at 2026-05-30 14:42 UTC; RocksDB LOG file extracts; binary symbol-table dumps; journalctl OOM records; main worktree at `/private/tmp/doli-inc-i091` (commit `c5a539f8`); INC-I-102 description and INC-I-101 latent-risk record from `.omega/memory.db`.
