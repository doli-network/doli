# Evaluator #2 — Restructurer Proposal

## TL;DR

- **Shared block cache across block_store + state_db + utxo_store** (single 64 MB LRU) replaces three separate 8 MB defaults; diagnostic_ledger gets its own 2 MB cache or none. Shared cache gives global LRU priority and eliminates wasted cache on cold instances.
- **Per-CF memtable differentiation** is the highest-impact change: dead/cold CFs (presence, cf_exit_history, meta) get 1-4 MB memtables; hot CFs (cf_utxo, headers, bodies) get 16-32 MB. Current uniform 64 MB x 2 is wrong for 14 of 19 CFs.
- **db_write_buffer_size must be set on ALL instances** — the current 0 (uncapped) on 3 of 4 instances is the root cause of INC-I-104.
- **WAL bounded on all instances** — block_store and utxo_store currently have max_total_wal_size=0. Set to 2x db_write_buffer_size per instance.
- **block_store put_block is non-atomic** (6 individual put_cf calls per block) — this is a structural coupling issue but NOT in redesign scope. Noted for cross-perspective.

## What I Don't Understand

1. Whether RocksDB's default BlockBasedOptions allocates a per-CF 8 MB cache or a per-DB 8 MB cache when no explicit cache is set. The `rust-rocksdb` crate documentation is ambiguous. If per-CF, the current implicit allocation is 9*8=72 MB for block_store alone, not 8 MB. I will assume per-DB based on RocksDB upstream documentation.
2. The exact current write amplification on mainnet — whether compaction is keeping up or creating backpressure that causes extra memtable allocation.
3. Whether `utxo_store` actually writes during normal block application or only the in-memory `UtxoSet` + `state_db`'s `BlockBatch` handles everything. The code path in `apply_block/tx_processing.rs` operates on `utxo_set` (in-memory enum) and `batch` (state_db) — I do not see `RocksDbUtxoStore::add_transaction` called during apply_block.
4. The steady-state UTXO set cardinality on mainnet (affects bloom filter sizing).

## Current State Analysis

### Dependency / Coupling Map (current)

```
                 apply_block()
                     |
        +------------+-------------+
        |            |             |
  [state_db]    [utxo_set]    [block_store]
  WriteBatch     in-memory     individual puts
  (atomic)       UtxoSet       (non-atomic)
        |            |
        |    RocksDb variant?
        |    (utxo_store)
        |            |
        +--- BOTH write UTXO data ---+
             but via DIFFERENT paths:
             state_db.batch.add_utxo()
             utxo_set.add_transaction()

  [diagnostic_ledger]
  Async writer task (decoupled)
  Only opened when --fork-diagnostics passed
```

### CF-level workload classification

Evidence: source code read of all 4 `open.rs` files + `writes.rs` + `batch.rs` + `apply_block/mod.rs`

| Instance | CF | Write class | Read class | Current memtable | Correct class |
|----------|-----|------------|-----------|-----------------|--------------|
| block_store | headers | HOT (every block) | HOT (validation, sync) | 64MB x 2 | Hot-write |
| block_store | bodies | HOT (every block) | WARM (sync, RPC) | 64MB x 2 | Hot-write, large values |
| block_store | height_index | HOT (set_canonical_chain) | HOT (height lookups) | 64MB x 2 | Warm-write (batched) |
| block_store | slot_index | HOT (every block) | WARM (RPC) | 64MB x 2 | Warm-write |
| block_store | hash_to_height | HOT (set_canonical_chain) | HOT (sync, fork detect) | 64MB x 2 | Warm-write (batched) |
| block_store | tx_index | HOT (every block, per-tx) | COLD (RPC only) | 64MB x 2 | Warm-write |
| block_store | addr_tx_index | HOT (every block, per-addr) | COLD (RPC only) | 64MB x 2 | Warm-write |
| block_store | presence | DEAD (never written) | DEAD (never read) | 64MB x 2 | Dead |
| block_store | meta | COLD (snap sync only) | COLD (startup) | 64MB x 2 | Cold |
| state_db | cf_utxo | HOT (per-input + per-output) | HOT (validation) | 64MB x 2 | Hot-write, hot-read |
| state_db | cf_utxo_by_pubkey | HOT (mirrors cf_utxo) | WARM (RPC prefix scan) | 64MB x 2 | Hot-write |
| state_db | cf_producers | WARM (epoch boundary only) | WARM (epoch, RPC) | 64MB x 2 | Cold-write |
| state_db | cf_exit_history | COLD (producer exit only) | COLD (re-registration check) | 64MB x 2 | Cold-write |
| state_db | cf_meta | HOT (chain_state every block) | HOT (startup, canary) | 64MB x 2 | Warm-write (small values) |
| state_db | cf_undo | HOT (one entry per block, large) | COLD (rollback only) | 64MB x 2 | Warm-write, large values |
| utxo_store | utxo | HOT (mirrors state_db) | WARM (RocksDb backend fallback) | 64MB x 2 | Hot-write |
| utxo_store | utxo_by_pubkey | HOT (mirrors state_db) | WARM (balance queries) | 64MB x 2 | Hot-write |
| utxo_store | unique_id | WARM (NFT/Asset/Pool mint only) | WARM (uniqueness check) | 64MB x 2 | Warm-write |
| diagnostic | cf_events | WARM (async batch, bursty) | COLD (RPC, pruner) | 4MB x 2 | Warm-write |

### Current parameter evidence (from code + analyst's RocksDB LOG dump)

| Parameter | block_store | state_db | utxo_store | diagnostic_ledger |
|-----------|-------------|----------|------------|-------------------|
| db_write_buffer_size | 0 (uncapped) | 0 (uncapped) | 0 (uncapped) | 8 MB |
| Per-CF write_buffer_size | 64 MB (default) | 64 MB (default) | 64 MB (default) | 4 MB |
| max_write_buffer_number | 2 (default) | 2 (default) | 2 (default) | 2 |
| max_total_wal_size | 0 (uncapped) | 64 MB | 0 (uncapped) | not set |
| Block cache | ~8 MB (default) | ~8 MB (default) | ~8 MB (default) | 4 MB (explicit) |
| Bloom filter | 10 bits/key (all CFs) | none | none | none |
| Compression | Lz4 (all CFs) | Lz4 (all CFs) | Lz4 (all CFs) | Lz4 |
| Compaction style | Level (default) | Level (default) | Level (default) | Level (default) |
| max_open_files | 256 | 256 | not set (default) | 64 |
| Per-CF options | none | none | none | none |

Theoretical memtable ceiling (worst case):
- block_store: 9 CFs x 64 MB x 2 = 1,152 MB
- state_db: 6 CFs x 64 MB x 2 = 768 MB
- utxo_store: 3 CFs x 64 MB x 2 = 384 MB
- diagnostic_ledger: 8 MB (capped)
- **Total per node: 2,312 MB** memtables alone

## Boundary Analyses

### 1. Per-CF vs uniform memtable budget within an instance

**Current boundary**: Every CF in every instance gets 64 MB x 2 = 128 MB memtable ceiling.

**Evidence this is wrong**: The `presence` CF is dead — never written (cleaned on open, line 100-135 of block_store/open.rs). It still gets a 128 MB memtable allocation that is never used but pins WAL segments. `cf_exit_history` in state_db is written only on producer exit events (maybe once per month). `meta` in block_store is written once on snap sync. These CFs getting the same memtable budget as `cf_utxo` (written O(inputs+outputs) per block) is architecturally wrong.

**Proposed boundary**: Per-CF options via `ColumnFamilyDescriptor` (the Rust `rocksdb` crate supports this — content_store.rs at line 32-33 already uses this pattern in this codebase).

| Workload class | write_buffer_size | max_write_buffer_number | Rationale |
|---------------|-------------------|------------------------|-----------|
| Hot-write (cf_utxo, headers, bodies, utxo, utxo_by_pubkey) | 16 MB | 2 | High write volume; 16 MB gives ~2 blocks of buffering at steady state; 32 MB ceiling per CF |
| Warm-write (height_index, slot_index, hash_to_height, tx_index, addr_tx_index, cf_utxo_by_pubkey, cf_meta, cf_undo, unique_id) | 8 MB | 2 | Moderate writes; 16 MB ceiling per CF |
| Cold-write (cf_producers, cf_exit_history, meta, presence) | 2 MB | 2 | Rarely written; 4 MB ceiling per CF; prevents WAL pinning |

**Derivation math for hot CFs**:
- Block rate: 1 block per 10 seconds = 6 blocks/min
- cf_utxo writes per block: ~2-10 transactions x 2-4 outputs each = ~20-40 entries @ ~100 bytes = ~4 KB per block
- Memtable flush threshold: 16 MB / 4 KB = ~4,000 blocks = ~11 hours between flushes at steady state
- During sync catch-up: ~1000 blocks/min = ~4 MB/min → flushes every ~4 minutes. Acceptable.
- Bodies are larger (500B-2MB per block) but still compress well under Lz4 in the memtable

**conf(0.65, observed)** — I've verified every CF's write pattern from source code. The 16/8/2 MB split is my engineering judgment; actual optimal values depend on mainnet write amplification data I don't have.

**Kill test**: Would per-CF options via ColumnFamilyDescriptor break existing databases on upgrade? No — RocksDB applies CF options at open time, not at data format level. The content_store.rs already uses this pattern. Existing data files are unaffected.

### 2. Memory budget boundary across DBs

**Three options evaluated**:

(a) Per-DB `db_write_buffer_size` budgets, independent — each DB gets a fixed cap
(b) Global memory budget, divided per DB by workload weight — requires shared allocator
(c) Per-DB caps + shared block cache for the read side

**Verdict: (c) is the correct boundary.**

**Why not (b)**: RocksDB has no built-in cross-instance memory budgeting. The `WriteBufferManager` can be shared but requires all instances to open with the same manager — the Rust crate (`rust-rocksdb` v0.22) exposes `set_db_write_buffer_size` per instance but does NOT expose `WriteBufferManager` sharing across separate `DB` objects. Implementing global budgeting would require a custom wrapper — complexity not justified.

**Why (c)**: Per-DB `db_write_buffer_size` caps the write side (memtables) independently per instance. A shared block cache caps the read side globally. This matches the actual workload: writes are independent per DB (state_db and block_store write in different code paths), but reads compete for the same CPU cache lines.

**Proposed db_write_buffer_size per instance**:

| Instance | db_write_buffer_size | Derivation |
|----------|---------------------|-----------|
| state_db | 64 MB | 6 CFs: 2 hot (16 MB x 2) + 2 warm (8 MB x 2) + 2 cold (2 MB x 2) = 64+32+8 = 104 MB theoretical, but db_write_buffer_size caps the TOTAL across all CFs. 64 MB forces RocksDB to flush the least-recently-written CF when total memtable reaches 64 MB. This is the consensus-critical instance — needs headroom for burst writes at epoch boundaries. |
| block_store | 48 MB | 9 CFs: 2 hot (16 MB x 2) + 5 warm (8 MB x 2) + 2 cold (2 MB x 2) = 64+80+8 = 152 MB theoretical. 48 MB cap forces frequent flushes on cold CFs, preventing WAL pinning. Writes are append-only — flush is cheap. |
| utxo_store | 32 MB | 3 CFs: 2 hot (16 MB x 2) + 1 warm (8 MB x 2) = 64+16 = 80 MB theoretical. 32 MB cap is sufficient because this store self-heals from state_db — durability can be relaxed. |
| diagnostic_ledger | 8 MB | 1 CF: warm (4 MB x 2) = 8 MB theoretical. Cap matches. INC-I-102 value is workload-justified. |

**Total memtable budget per node: 64 + 48 + 32 + 8 = 152 MB** (down from 2,312 MB theoretical, ~450 MB observed)

**conf(0.6, inferred)** — The exact values depend on mainnet write patterns I can't measure. The structure (per-DB caps, db_write_buffer_size > 0 everywhere) is high-confidence; the specific numbers are medium.

**Kill test**: Would 64 MB for state_db be too low during sync catch-up when blocks arrive at max rate? At 1000 blocks/min, cf_utxo writes ~4 KB/block = 4 MB/min. Total across all CFs ~10 MB/min. 64 MB cap → flush every ~6 minutes during sync. This is fine — RocksDB flushes are fast (memtable → L0 SST, sequential write).

### 3. Block cache boundary

**Current state**: Each instance gets RocksDB's default 8 MB LRU cache. Total: 4 x 8 = 32 MB across node.

**Analysis of access patterns**:
- state_db cf_utxo: Point lookups (get by outpoint key). High locality for recently-created UTXOs (same-block-spend). Benefits strongly from cache.
- block_store headers: Point lookups by hash (hot) + sequential backward walk (set_canonical_chain, sync). The sequential walk would thrash a small cache, but steady-state is dominated by point lookups.
- block_store bodies: Large values (500B-2MB). Caching entire blocks is wasteful — a single block body can evict hundreds of UTXO cache entries.
- utxo_store: Mirrors state_db reads. If both are cached, there's redundancy.
- diagnostic_ledger: Cold reads (RPC only). Cache is wasted.

**Proposed boundary**: Shared 64 MB LRU cache across state_db + block_store + utxo_store. Diagnostic_ledger gets its own 2 MB cache (or zero).

**Justification for shared over per-instance**:
1. Global LRU means the hottest data wins regardless of which DB it's in. If cf_utxo is hot and block bodies are cold, the cache fills with UTXO entries — correct behavior.
2. Per-instance caches waste memory on cold instances. utxo_store's 8 MB is largely wasted if reads go through the in-memory UtxoSet.
3. The mutual eviction concern (sequential block body reads evicting UTXO point lookups) is mitigated by block_size tuning — with 16 KB blocks and large values, bodies are cached at the block level; a single body read loads only 1-2 cache blocks, not the entire value.

**Why separate diagnostic_ledger**: Its reads are RPC-only and would pollute the shared cache with cold data during a debug session.

**conf(0.55, inferred)** — Shared cache is well-documented in RocksDB upstream (the `Cache` object is explicitly designed to be shared). The 64 MB size is inferred from workload estimates. The mutual eviction risk is the main uncertainty.

**Kill test**: Does the Rust `rocksdb` crate support sharing a `Cache` across multiple `DB` instances? Yes — `rocksdb::Cache::new_lru_cache()` returns a `Cache` object that can be passed to multiple `BlockBasedOptions::set_block_cache()` calls. The diagnostic_ledger code already creates a cache this way (line 65-66). This is confirmed by reading the existing code.

### 4. Compaction style per CF

**Current**: All CFs use Level compaction (RocksDB default).

**Analysis**:
- **cf_utxo**: High write churn (deletes + inserts) AND hottest read path. Level compaction is correct — better read amplification for point lookups (sorted runs at each level). The write amplification cost is acceptable because UTXO values are small (~100 bytes).
- **bodies**: Append-only, never deleted (blocks are immutable). Universal compaction would reduce write amplification (fewer merge operations). But the write rate is low (1 block per 10s) — compaction overhead is negligible. Level is fine.
- **cf_undo**: High write rate (one entry per block, 1-100+ KB values), cold reads (rollback only), pruned after 360 blocks. FIFO compaction would be ideal (entries are consumed and deleted in order), but FIFO requires `compaction_options_fifo` which may not be well-supported in the Rust crate.

**Proposed**: Keep Level compaction for all CFs. The write rate (6 blocks/min) is too low for compaction style to make a measurable difference. Universal compaction's disadvantage (higher space amplification) outweighs its advantage (lower write amplification) at this write rate.

**conf(0.5, inferred)** — Without measured write amplification, I cannot confidently propose compaction style changes. The "keep Level" recommendation is the safe default.

**Kill test**: Would keeping Level compaction cause problems for cf_undo with large values? cf_undo entries are 1-100+ KB but pruned after 360 blocks. With Level compaction, these large values participate in compaction even though they'll be deleted soon. But at 1 entry per block * 360 retention * 100 KB max = 36 MB total cf_undo data — compaction overhead is negligible.

### 5. WAL boundary

**Current**: state_db has max_total_wal_size=64 MB. block_store and utxo_store have 0 (uncapped). diagnostic_ledger has no explicit WAL setting.

**WAL pinning problem**: When a CF with an active memtable has data in a WAL file, that entire WAL file cannot be deleted until the CF's memtable is flushed. Dead CFs (presence) or cold CFs (cf_exit_history) that never receive writes never flush their memtable, pinning ALL WAL files that were created since the last full flush. Setting `max_total_wal_size` forces RocksDB to flush the oldest CF's memtable when total WAL exceeds the limit, allowing old WAL files to be deleted.

**Proposed**:

| Instance | max_total_wal_size | Ratio to db_write_buffer_size | Rationale |
|----------|-------------------|-------------------------------|-----------|
| state_db | 64 MB | 1:1 (64 MB / 64 MB) | Current value, already working. 1:1 ratio ensures WAL files don't exceed memtable budget. |
| block_store | 48 MB | 1:1 (48 MB / 48 MB) | Matches db_write_buffer_size. Forces cold CF flush and WAL rotation. |
| utxo_store | 32 MB | 1:1 (32 MB / 32 MB) | Matches db_write_buffer_size. OR: disable WAL entirely (see proposal P3). |
| diagnostic_ledger | 8 MB | 1:1 (8 MB / 8 MB) | Matches db_write_buffer_size. OR: disable WAL entirely (see proposal P3). |

**Rationale for 1:1 ratio**: The WAL contains uncommitted memtable data. When max_total_wal_size equals db_write_buffer_size, RocksDB is forced to flush at least one CF's memtable when WAL reaches the memtable budget — this is the tightest bound that doesn't cause unnecessary flushes.

**conf(0.65, observed)** — state_db's 64 MB WAL cap is already proven in production. The 1:1 ratio is a reasonable heuristic from RocksDB tuning guides.

**Kill test**: Would 1:1 WAL-to-memtable ratio cause excessive flushing? Only if all CFs have active memtables that collectively equal db_write_buffer_size AND the WAL is also full. This shouldn't happen because db_write_buffer_size already forces flushes when memtable budget is exhausted — the WAL cap is a belt-and-suspenders measure, not the primary flush trigger.

### 6. Background jobs and subcompactions allocation

**Current**: All instances use RocksDB defaults (max_background_jobs=2, max_subcompactions=1 on most platforms).

**Analysis**: With 4 RocksDB instances per node, the default gives 4 x 2 = 8 background threads for flush/compaction. On a 4-core VPS, this is already near saturation.

**Proposed**:

| Instance | max_background_jobs | max_subcompactions | Rationale |
|----------|--------------------|--------------------|-----------|
| state_db | 2 | 1 | Consensus-critical, highest priority. Gets 2 jobs for flush + compaction. |
| block_store | 2 | 1 | Append-heavy, needs compaction bandwidth for bodies. |
| utxo_store | 1 | 1 | Mirrors state_db; lower priority. |
| diagnostic_ledger | 1 | 1 | Minimal I/O, lowest priority. |

**Total: 6 background threads** across all instances. On a 4-core machine, this is reasonable — background jobs are I/O-bound, not CPU-bound.

There is NO global rate limiter or SST file manager shared across instances in the current code. Each DB has its own thread pool. Sharing a `RateLimiter` across instances would require passing the same `RateLimiter` object to all `Options::set_ratelimiter()` calls — not currently done. This is a "could" optimization, not a "must".

**conf(0.45, inferred)** — Background job counts are system-dependent. The proposed values are conservative defaults. Without I/O profiling, I cannot be confident these are optimal.

**Kill test**: Would reducing utxo_store to 1 background job cause write stalls during sync? During sync catch-up, utxo_store receives the same write volume as state_db. With 1 background job and 32 MB db_write_buffer_size, flush capacity is limited to 1 concurrent flush. If memtable fills faster than flushes complete, RocksDB will stall writes (level0_stop_writes_trigger). However, utxo_store is rebuildable — a write stall here doesn't block consensus. The in-memory UtxoSet handles validation reads regardless.

### 7. Cross-DB write coupling

**Finding**: state_db and utxo_store/utxo_set write UTXO data independently per block, via completely different code paths:

1. **state_db**: `BlockBatch::add_utxo()` / `spend_utxo()` in `batch.rs` — accumulates in a `WriteBatch`, committed atomically at end of `apply_block`.
2. **utxo_set**: `UtxoSet::add_transaction()` / `spend_transaction()` in `utxo/set.rs` — writes to the `RocksDbUtxoStore` backend immediately (individual `db.write(batch)` per transaction).

**Key structural observation**: The utxo_set (RocksDbUtxoStore) writes are NOT in the same WriteBatch as state_db. They happen during the transaction processing loop (lines 192-208 of apply_block/mod.rs), while the state_db batch is committed at line 347. If the node crashes between utxo_set writes and state_db commit, the two stores diverge — but this is by design, because utxo_store self-heals from state_db on startup.

**Should they share a WriteBatch?**: No. The two stores serve different purposes:
- state_db is the authoritative UTXO set (consensus-critical, atomic commits)
- utxo_store/utxo_set is the in-process read cache (rebuildable, immediate writes for fast reads)

Coupling them into one WriteBatch would require either: (a) opening both in the same RocksDB instance (cross-CF batch across instances is not supported), or (b) two-phase commit. Both add complexity for no functional benefit — self-heal already handles divergence.

**Should they share an Env / thread pool?**: Not worth it. The Env (file system interface) is lightweight, and sharing it across instances provides no benefit for I/O-bound operations on the same disk.

**The right structure**: Two parallel write paths with self-heal. Current design is correct. No change needed.

**conf(0.65, observed)** — I've verified the exact write paths in source code. The self-heal mechanism at startup (init.rs lines 36-66) handles divergence.

### 8. The deprecated `presence` CF in block_store

**Current**: `presence` is listed in the CF descriptor array (open.rs line 33). Its data is cleaned on startup (cleanup_presence_cf, lines 100-135). It is never written to or read from after cleanup.

**Problem**: RocksDB allocates a memtable for every CF that is opened, even if never written. With default options, presence gets a 64 MB x 2 = 128 MB memtable allocation that is entirely wasted. More importantly, it can pin WAL files: if presence's memtable never flushes (because it has no data), the WAL file containing the cleanup deletes stays pinned.

**Proposed boundary**: Keep the CF descriptor (for backward compatibility with existing DBs) but configure it with minimal memtable:

```
presence CF: write_buffer_size = 1 MB, max_write_buffer_number = 1
```

This limits waste to 1 MB (WAL pinning is resolved by setting max_total_wal_size on block_store). Dropping the CF entirely would cause existing databases to fail to open (RocksDB errors on missing CFs that have data). The cleanup-on-open already deletes the data, but the CF descriptor must remain.

**Alternative (subtraction)**: Drop the CF from the descriptor list and handle the error on open by re-opening without it. More complex, higher risk, marginal benefit (saves 1 MB). Not recommended.

**conf(0.7, observed)** — The cleanup logic is verified in code. The 1 MB minimal memtable is a standard RocksDB pattern for deprecated CFs.

## Proposals

### P1: Per-CF differentiated memtable budgets — conf(0.65, observed)

**Evidence**: All 4 open.rs files use uniform options via `DB::open_cf()`. No per-CF overrides exist. content_store.rs (line 32-33) proves the codebase already uses `ColumnFamilyDescriptor` for per-CF options — the pattern is established.

**Complexity cost**: +0 modules, +0 interfaces. Each `open()` function changes from `DB::open_cf(&opts, path, cfs)` to `DB::open_cf_descriptors(&opts, path, cf_descriptors)`. The `cf_descriptors` vec contains per-CF `ColumnFamilyDescriptor` with differentiated `Options`.

**Kill test**: Would per-CF options break existing databases? No — CF options are applied at open time. Existing SST files and WAL files are unaffected. The RocksDB documentation explicitly states that CF options can be changed between opens.

**Kill test result**: Not found. Pattern already used in content_store.rs. Safe.

**Risk**: If the per-CF write_buffer_size is set too low for a hot CF, it will flush more frequently, increasing write amplification. The 16 MB floor for hot CFs provides ~4 hours of steady-state buffering (vs ~24 hours with 64 MB) — acceptable.

**Before**: 9 CFs in block_store all at 64 MB x 2. 6 CFs in state_db all at 64 MB x 2.
**After**: block_store: 2 hot CFs at 16 MB x 2, 5 warm at 8 MB x 2, 2 cold at 2 MB x 2. state_db: 2 hot CFs at 16 MB x 2, 2 warm at 8 MB x 2, 2 cold at 2 MB x 2.

### P2: db_write_buffer_size on all 4 instances — conf(0.7, observed)

**Evidence**: RocksDB LOG dump on ai5/n9 (cited in intake) confirms db_write_buffer_size=0 on 3 of 4 instances. This is the root cause of INC-I-104.

**Complexity cost**: +0 modules, +0 interfaces. One line of code per instance: `opts.set_db_write_buffer_size(N)`.

**Kill test**: Would setting db_write_buffer_size cause write stalls on a busy node? RocksDB documentation: when total memtable usage across all CFs exceeds db_write_buffer_size, the DB triggers a flush on the CF with the largest memtable. This can cause a brief write pause if the flush queue is full. At the proposed values (64 MB state_db, 48 MB block_store, 32 MB utxo_store), the headroom is sufficient for the observed steady-state write rate.

**Kill test result**: Not found. Flush-on-budget-exceeded is the desired behavior — it prevents unbounded memory growth.

**Risk**: During sync catch-up, writes burst at 100x steady state. The db_write_buffer_size cap will cause more frequent flushes. This is acceptable — flush is fast (sequential write to L0 SST file), and the alternative (unbounded memory growth) is what caused INC-I-104.

**Before**: 3 of 4 instances have no memtable budget. Total theoretical ceiling: 2,312 MB.
**After**: All 4 instances capped. Total memtable budget: 152 MB.

### P3: WAL bounded on all instances — conf(0.65, observed)

**Evidence**: block_store and utxo_store have max_total_wal_size=0 (confirmed from RocksDB LOG dump). The `presence` CF's dead memtable can pin WAL files indefinitely.

**Complexity cost**: +0 modules. One line per instance.

**Kill test**: Would capping WAL cause data loss on crash? No — WAL cap triggers a flush (memtable → SST file), not data deletion. Flushed data is durable. The WAL cap ensures crash recovery replays only recent WAL entries.

**Kill test result**: Not found. state_db already has this cap at 64 MB with no issues.

**Risk**: None identified. This is strictly safer than the current uncapped state.

**Before**: 2 instances have uncapped WAL.
**After**: All instances have WAL capped at 1:1 ratio to db_write_buffer_size.

### P4: Shared block cache for state_db + block_store + utxo_store — conf(0.55, inferred)

**Evidence**: Each instance currently uses the RocksDB default 8 MB LRU cache (confirmed by absence of explicit cache creation in open.rs for these 3 instances). Total: 24 MB across 3 instances.

**Complexity cost**: +0 modules, +1 shared object. A `Cache::new_lru_cache(64 * 1024 * 1024)` created once and passed to all 3 instances' `BlockBasedOptions`.

**Kill test**: Would a shared cache cause mutual eviction that degrades hot-path latency? If a sync burst loads many block bodies, their cache blocks could evict cf_utxo entries. But: (a) bodies are large values — RocksDB caches at the block level, not the value level, so a single body read loads at most 1-2 16KB cache blocks; (b) cf_utxo reads are point lookups on 36-byte keys, highly cache-friendly; (c) global LRU means the most-accessed data wins regardless of origin.

**Kill test result**: The mutual eviction risk is real but bounded by block_size. With 16 KB blocks, a 2 MB body spans ~125 cache blocks = ~2 MB of cache space. The 64 MB cache can absorb this without significant eviction of cf_utxo data.

**Risk**: If a single RPC query (getAddressHistory) triggers a large sequential scan on block_store, it could temporarily evict hot data. But RPC reads are infrequent relative to consensus reads.

**Before**: 3 separate 8 MB caches (24 MB total, no cross-instance LRU).
**After**: 1 shared 64 MB cache (better memory efficiency, global LRU priority).

### P5: Bloom filters on point-lookup CFs — conf(0.6, observed)

**Evidence**: block_store already has a 10 bits/key bloom filter (open.rs line 25). state_db and utxo_store have NO bloom filters (confirmed by absence of `set_bloom_filter` calls in their open paths).

**Complexity cost**: +0 modules. Per-CF BlockBasedOptions with bloom filter.

**CFs that benefit from bloom filters** (point-lookup dominant):
- state_db: cf_utxo (point lookup per validation, high cardinality), cf_producers (point lookup by pubkey_hash)
- utxo_store: utxo (point lookup), unique_id (existence check)

**CFs that do NOT benefit** (prefix scan or sequential):
- state_db: cf_utxo_by_pubkey (prefix scan by pubkey_hash — bloom filters don't help prefix scans), cf_undo (sequential by height)
- block_store: already has bloom on all CFs (some wasted on addr_tx_index prefix scans, but not harmful)

**bits_per_key**: 10 bits/key gives ~1% FPR. At 100K UTXOs, the bloom filter is ~125 KB per SST file — negligible. At 1M UTXOs, ~1.25 MB — still negligible. 10 bits/key is the standard recommendation and already used by block_store.

**Kill test**: Would bloom filters increase memory usage significantly? Bloom filters are stored in SST file metadata, loaded into block cache on access. With 10 bits/key and <1M entries, the total bloom filter data is <1.5 MB per CF — fits easily in the 64 MB shared cache.

**Kill test result**: Not found. Memory impact is negligible.

**Risk**: Bloom filters have a false positive rate (~1% at 10 bits/key). A false positive on cf_utxo means an unnecessary disk read — adds ~100us to one validation lookup out of 100. Acceptable.

**Before**: Only block_store has bloom filters.
**After**: state_db (cf_utxo, cf_producers, cf_exit_history) and utxo_store (utxo, unique_id) get bloom filters at 10 bits/key.

## Concrete Configuration per Instance

### block_store

| Parameter | Value | Derivation |
|-----------|-------|-----------|
| db_write_buffer_size | 48 MB | 9 CFs, sum of per-CF budgets: 2x16MB + 5x8MB + 2x2MB = 76MB theoretical; cap at 48MB forces cold CF flush |
| max_total_wal_size | 48 MB | 1:1 with db_write_buffer_size |
| max_background_jobs | 2 | Standard, append-heavy |
| max_subcompactions | 1 | Default |
| max_open_files | 256 | Current, keep |
| compression | Lz4 | Current, keep |
| compaction_style | Level | Default, keep |

**Per-CF overrides (via ColumnFamilyDescriptor)**:

| CF | write_buffer_size | max_write_buffer_number | bloom_filter | Notes |
|----|-------------------|------------------------|-------------|-------|
| headers | 16 MB | 2 | 10 bits/key (existing) | Hot read+write |
| bodies | 16 MB | 2 | 10 bits/key (existing) | Hot write, large values |
| height_index | 8 MB | 2 | 10 bits/key (existing) | Warm, batched via set_canonical_chain |
| slot_index | 8 MB | 2 | 10 bits/key (existing) | Warm |
| hash_to_height | 8 MB | 2 | 10 bits/key (existing) | Hot read, warm write |
| tx_index | 8 MB | 2 | 10 bits/key (existing) | Warm write, cold read |
| addr_tx_index | 8 MB | 2 | 10 bits/key (existing) | Warm write, cold read |
| presence | 1 MB | 1 | none | Dead CF, minimal footprint |
| meta | 2 MB | 2 | none | Cold, tiny values |

**Block cache**: Shared 64 MB LRU (see P4)

### state_db

| Parameter | Value | Derivation |
|-----------|-------|-----------|
| db_write_buffer_size | 64 MB | 6 CFs, consensus-critical, needs headroom for epoch boundary bursts |
| max_total_wal_size | 64 MB | 1:1 with db_write_buffer_size (current value, keep) |
| wal_recovery_mode | PointInTime | Current, keep |
| max_background_jobs | 2 | Current default, keep |
| max_subcompactions | 1 | Default |
| max_open_files | 256 | Current, keep |
| compression | Lz4 | Current, keep |
| compaction_style | Level | Best for cf_utxo point lookups |

**Per-CF overrides (via ColumnFamilyDescriptor)**:

| CF | write_buffer_size | max_write_buffer_number | bloom_filter | Notes |
|----|-------------------|------------------------|-------------|-------|
| cf_utxo | 16 MB | 2 | 10 bits/key | Hottest CF — validation reads + write churn |
| cf_utxo_by_pubkey | 16 MB | 2 | none | Hot write, prefix scan reads (bloom doesn't help) |
| cf_meta | 8 MB | 2 | none | Warm, small values, updated every block |
| cf_undo | 8 MB | 2 | none | Warm write (large values, 1 per block), cold read |
| cf_producers | 2 MB | 2 | 10 bits/key | Cold write (epoch boundary only), warm read |
| cf_exit_history | 2 MB | 2 | 10 bits/key | Cold write (producer exit), cold read |

**Block cache**: Shared 64 MB LRU (see P4)

### utxo_store

| Parameter | Value | Derivation |
|-----------|-------|-----------|
| db_write_buffer_size | 32 MB | 3 CFs, rebuildable, lower priority |
| max_total_wal_size | 32 MB | 1:1 with db_write_buffer_size (or disable WAL entirely — see note) |
| max_background_jobs | 1 | Rebuildable, lower priority |
| max_subcompactions | 1 | Default |
| max_open_files | 256 | Match other instances (currently not set, uses default) |
| compression | Lz4 | Current, keep |
| compaction_style | Level | Default, keep |

**Per-CF overrides (via ColumnFamilyDescriptor)**:

| CF | write_buffer_size | max_write_buffer_number | bloom_filter | Notes |
|----|-------------------|------------------------|-------------|-------|
| utxo | 16 MB | 2 | 10 bits/key | Hot write, warm read |
| utxo_by_pubkey | 8 MB | 2 | none | Hot write, prefix scan reads |
| unique_id | 4 MB | 2 | 10 bits/key | Warm write, existence checks |

**Block cache**: Shared 64 MB LRU (see P4)

**Note on WAL**: utxo_store self-heals from state_db on startup (init.rs lines 36-66). WAL could be disabled entirely (`Options::set_enable_wal(false)` or `WriteOptions::set_disable_wal(true)` per write). This saves I/O but means crash recovery always triggers a full rebuild from state_db (currently ~2 seconds for 100K UTXOs). Recommendation: disable WAL on utxo_store (AC-COULD-002).

### diagnostic_ledger

| Parameter | Value | Derivation |
|-----------|-------|-----------|
| db_write_buffer_size | 8 MB | Current INC-I-102 value, workload-justified (see below) |
| write_buffer_size (per-memtable) | 4 MB | Current, keep |
| max_write_buffer_number | 2 | Current, keep |
| max_total_wal_size | 8 MB | 1:1 with db_write_buffer_size |
| max_background_jobs | 1 | Minimal, observability only |
| max_subcompactions | 1 | Default |
| max_open_files | 64 | Current, keep |
| compression | Lz4 | Current, keep |
| bloom_filter | none | Range scans dominant (not point lookups) |
| block_cache | 2 MB (own cache, not shared) | Cold reads, would pollute shared cache |

**Workload derivation for 8 MB cap**:
- Write rate: async batches of 10 events or 100ms, whichever first. Event size: 200-600 bytes. Max throughput: 10 events x 600 bytes / 100ms = 60 KB/s sustained during fork events.
- Memtable fill time at max rate: 4 MB / 60 KB/s = ~67 seconds → flush every ~1 minute during a fork event. During quiet operation: essentially never flushes (maybe once per hour).
- 8 MB db_write_buffer_size with 4 MB per-memtable and 2 max means: the cap allows exactly 2 memtables (4 MB active + 4 MB flushing = 8 MB). This is tight but correct for a single-CF observability store.

**conf(0.7, observed)** — INC-I-102 value is workload-justified by the derivation above. The 8 MB cap is correct.

## Shared Resource Decisions

### Block cache: shared (64 MB LRU) across state_db, block_store, utxo_store

**Justification**: Point lookups on cf_utxo and headers dominate the read path. A shared 64 MB cache gives 2.7x the total cache of the current 3 x 8 MB = 24 MB, with global LRU priority ensuring the hottest data wins. diagnostic_ledger excluded to prevent cold RPC reads from polluting consensus-hot data.

**Implementation**: Create `rocksdb::Cache::new_lru_cache(64 * 1024 * 1024)` once in Node::new(). Pass to block_store, state_db, and utxo_store open functions. Each creates `BlockBasedOptions` and calls `set_block_cache(&shared_cache)`. The Cache is Clone (Arc internally), so this is zero-cost sharing.

### WAL ratio per instance: 1:1 with db_write_buffer_size

**Justification**: WAL size should never exceed the memtable budget it protects. 1:1 ensures WAL rotation happens at most when the memtable budget is exhausted. state_db already uses 1:1 (64 MB / 64 MB) in production.

### Background jobs: per-instance (not global pool)

**Justification**: RocksDB's `Env` (thread pool manager) is per-instance by default. Sharing requires creating a custom `Env` and passing it to all instances — this adds complexity for marginal benefit. The proposed 2+2+1+1=6 total background threads is reasonable for a 4-core machine.

## Constraints Identified

1. **No per-CF write_buffer_size in `DB::open_cf()`**: Must switch to `DB::open_cf_descriptors()` for per-CF options. This changes the function signature but not the data format.

2. **presence CF must remain in descriptor list**: Removing it would cause existing DBs with presence data to fail to open. Must keep with minimal memtable.

3. **db_write_buffer_size is a TOTAL cap across all CFs**: If set lower than the largest single CF's write_buffer_size, it takes precedence. The per-CF values must sum to more than db_write_buffer_size for the cap to be meaningful (it forces flushes on the least-recently-written CF).

4. **Block cache sharing requires passing Cache to open() functions**: The current `open(path: &Path)` signatures don't accept a cache parameter. Must add an optional `cache: Option<&Cache>` parameter or create a builder pattern.

5. **utxo_store WAL disable requires per-write `disable_wal` flag**: `set_enable_wal(false)` at DB level does not exist in rust-rocksdb. Must use `WriteOptions::set_disable_wal(true)` per write. This is a code-level change in `RocksDbUtxoStore::add_transaction()`, `spend_transaction()`, etc.

6. **Behavioral preservation (AC-MUST-001)**: None of these changes affect data encoding, consensus logic, or state root computation. They only change RocksDB performance parameters. No activation height needed.

## Cross-Perspective Signals

1. **block_store put_block is non-atomic**: 6 individual `db.put_cf()` calls per block (writes.rs lines 31-68). A crash between any two puts leaves the block_store in an inconsistent state (e.g., header written but not body). This is not a RocksDB configuration issue — it's a write-path design issue. The block_store should use a WriteBatch for put_block, similar to how state_db uses BlockBatch. (Relevant to Patterns evaluator.)

2. **utxo_store write independence from state_db batch**: The UtxoSet (RocksDb backend) writes are not in the state_db atomic WriteBatch. The self-heal mechanism handles divergence, but during normal operation, utxo_store does per-transaction commits (add_transaction calls db.write(batch) per transaction), while state_db commits the entire block's changes in one batch. This means utxo_store does O(transactions_per_block) write syscalls while state_db does O(1). (Relevant to Patterns or Failures evaluator.)

3. **Startup UTXO count scan is O(n)**: Both state_db (open.rs line 41-46) and utxo_store (utxo_rocks.rs line 47-54) iterate the entire cf_utxo CF on startup to count entries. At scale (millions of UTXOs), this could take significant time. A persistent count in cf_meta would eliminate this. (Relevant to Subtractionist.)

## Gaps

1. **No measured write amplification data**: I derived memtable sizes from write rate estimates, not measured compaction statistics. Actual optimal values require RocksDB statistics (`rocksdb.stats` or `db.get_property("rocksdb.stats")`).

2. **No measured UTXO set cardinality**: Bloom filter sizing depends on entry count. I assumed <1M based on mainnet age (~3 months), but this needs verification.

3. **utxo_store write path during apply_block**: I identified that `process_transaction_utxos` writes to `utxo` (in-memory UtxoSet) and `batch` (state_db), but I could not find where the RocksDbUtxoStore backend of UtxoSet actually writes to disk during apply_block. It may happen inside the UtxoSet enum dispatch, but I didn't trace through the full dispatch chain. This affects whether utxo_store's write volume matches state_db's.

4. **No L0-to-L1 compaction timing data**: The `level0_file_num_compaction_trigger` (default 4) and `level0_slowdown_writes_trigger` (default 20) values were not analyzed because compaction timing depends on hardware I/O characteristics. The defaults are reasonable for the observed write rate.

5. **target_file_size_base and max_bytes_for_level_base**: These LSM-tree shape parameters were not tuned because they depend on the total data size per CF, which varies by chain age. RocksDB defaults (64 MB target file, 256 MB level base) are reasonable for current chain sizes but may need revision as the chain grows to millions of blocks.

## Sources cited

1. RocksDB documentation: "Column Family" — CF options can be changed between opens without affecting existing data.
2. RocksDB documentation: "Write Buffer Manager" — db_write_buffer_size acts as total memtable budget across all CFs within one DB instance.
3. RocksDB documentation: "Block Cache" — LRU cache can be shared across multiple DB instances by passing the same Cache object.
4. RocksDB Tuning Guide: "WAL" — max_total_wal_size forces flush of oldest CF's memtable when WAL exceeds the limit.
5. Codebase: `crates/storage/src/content_store.rs:32-36` — ColumnFamilyDescriptor pattern already used in this codebase.
6. Codebase: `crates/storage/src/diagnostic_ledger/mod.rs:64-66` — Explicit Cache creation pattern already used.
7. Codebase: `crates/storage/src/block_store/open.rs:24-26` — Bloom filter already set on block_store.
8. Codebase: `bins/node/src/node/apply_block/mod.rs:147-348` — Write path coupling analysis.
9. Codebase: `bins/node/src/node/init.rs:36-66` — utxo_store self-heal mechanism.
