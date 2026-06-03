# Evaluator #1 -- Subtractionist Proposal

## TL;DR

- **DROP** the `presence` CF's memtable budget entirely (64 MB x 2 = 128 MB wasted per node, pinning WAL)
- **DISABLE WAL** on `utxo_store` (self-heals from state_db; WAL is pure waste) and `diagnostic_ledger` (lossy ok)
- **SHRINK** per-CF `write_buffer_size` from the universal 64 MB to workload-derived values ranging 256 KB -- 8 MB depending on CF hotness
- **SET `db_write_buffer_size`** on all 4 instances (currently 0/uncapped on 3 of 4) as the global safety net
- **MINIMIZE** bloom filters, block cache, and compaction overhead -- only where the workload justifies

## Analysis Lens
Dead code, unused abstractions, unnecessary complexity. Key question: What can be REMOVED from the current architecture to solve this problem?

## What I Don't Understand

1. Whether `RocksDbUtxoStore` is opened unconditionally on every startup or only when old `utxo_rocks/` data exists for migration. The docs say "Always" in the analyst's table, the architecture doc says "migrated into StateDb" for old files, and node-heal excludes it. The RocksDB LOG on ai5 confirms it is live. I proceed on the assumption it is always opened. conf(0.6, inferred).

2. The exact sync burst multiplier during catch-up. Steady state is 6 blocks/min. During catch-up, it could be 100-600 blocks/min (network-limited). I use 100x as a conservative upper bound for memtable sizing math.

3. Whether any per-CF `Options` are set in the current open.rs files beyond the defaults. The analyst confirmed "RocksDB defaults" from the LOG dump, and no evidence of custom per-CF options was documented. I proceed on the assumption all CFs share identical default options. conf(0.7, observed) -- the LOG dump is direct evidence.

4. The actual UTXO set cardinality (number of live UTXOs). This affects bloom filter sizing. Architecture says "tens of millions" for cf_utxo. I use 10M as a working estimate.

5. Whether `utxo_store` can be dropped entirely and replaced with an in-memory-only UtxoSet plus state_db persistence. The `unique_id` CF provides NFT/Asset/Pool/Channel deduplication not present in state_db. Removing utxo_store requires moving unique_id tracking to state_db (a code change beyond pure configuration).

## Current State Analysis

### Memtable budget (per node, all 4 instances)

| Instance | CFs | Per-CF write_buffer_size | max_write_buffer_number | db_write_buffer_size | Theoretical max memtable |
|----------|-----|-------------------------|------------------------|---------------------|------------------------|
| block_store | 9 (8 active + 1 dead) | 64 MB | 2 | 0 (uncapped) | 9 x 64 x 2 = 1,152 MB |
| state_db | 6 | 64 MB | 2 | 0 (uncapped) | 6 x 64 x 2 = 768 MB |
| utxo_store | 3 | 64 MB | 2 | 0 (uncapped) | 3 x 64 x 2 = 384 MB |
| diagnostic_ledger | 1 | (governed) | 2 | 8 MB | 8 MB |
| **Total** | 19 | — | — | — | **2,312 MB** |

Observed plateau: ~450 MB/node (most CFs never fill even one memtable at steady-state write rates). But the _theoretical_ ceiling is what the OOM killer sees during memtable warm-up on constrained servers.

### Actual write rates (steady state, 6 blocks/min)

| Instance | Write rate (steady) | Time to fill ONE 64 MB memtable | Verdict |
|----------|--------------------|---------------------------------|---------|
| block_store (total) | ~35 KB/min | 31 hours | 64 MB is 1,850x oversized |
| state_db (total) | ~48 KB/min | 22 hours | 64 MB is 1,340x oversized |
| utxo_store (total) | ~15 KB/min | 71 hours | 64 MB is 4,270x oversized |
| diagnostic_ledger | ~240 KB/min (active) | 4.5 hours | 8 MB is ~34x oversized (acceptable) |

Even at 100x sync burst, the hottest individual CF (block_store `bodies` at ~30 KB/min steady = ~3 MB/min burst) would take 21 minutes to fill a 64 MB memtable. A 4 MB memtable fills in 1.3 min at burst -- plenty of time for background flush. A 2 MB memtable fills in 40 seconds at burst -- still fine.

### WAL status

| Instance | max_total_wal_size | WAL necessity | Waste |
|----------|-------------------|---------------|-------|
| block_store | 0 (uncapped) | Needed (consensus-critical headers/bodies) | WAL growth unbounded; presence CF pins WAL |
| state_db | 64 MB | Needed (consensus-critical, atomicity) | Acceptable |
| utxo_store | 0 (uncapped) | NOT needed (self-heals from state_db) | Entire WAL is waste |
| diagnostic_ledger | not documented | NOT needed (lossy ok) | Entire WAL is waste |

## Subtraction Analysis (10 items)

### 1. `presence` CF in block_store -- SIMPLIFY (cannot fully DROP)

**Evidence**: The analyst's workload inventory says "deprecated, cleaned on open, never written." The architecture doc (line 267) calls it "deprecated, cleaned on startup." The storage skill (line 231) confirms "cleaned up on open."

**Why not DROP entirely**: RocksDB requires all existing CFs to be listed in `ColumnFamilyDescriptor` when opening a database. If the CF exists on disk (in an old node's data directory), omitting it from the descriptor list causes the open to fail. The current code cleans keys but keeps the CF descriptor. Dropping the CF entirely requires a migration step (RocksDB `DropColumnFamily()` call on open, then removing from descriptor list on subsequent opens). This is doable but adds migration complexity.

**SIMPLIFY to**: Keep the CF descriptor for backward compatibility but set `write_buffer_size = 256 KB` (minimum practical) and `max_write_buffer_number = 1`. This reduces the presence CF's memtable cost from 128 MB theoretical to 256 KB actual. The cleaned-empty CF will never fill even this minimal memtable, so no WAL pinning.

**Math**: 64 MB x 2 = 128 MB saved. The 256 KB replacement is 0.2% of original.

**Kill test**: Could setting a tiny memtable cause problems? Only if the CF receives writes. It doesn't -- it's cleaned on open and never written again. PASSED.

conf(0.7, observed) -- behavior documented in analyst workload inventory and verified in architecture doc.

### 2. `utxo_store` entire instance -- KEEP (cannot drop without code change)

**Evidence**: utxo_store provides `unique_id` CF for NFT/Asset/Pool/Channel deduplication. This functionality does NOT exist in state_db. The architecture doc confirms self-heal from state_db for UTXO data, but the unique_id index is rebuilt from a UTXO scan (not directly from state_db CFs). DeFi is currently gated (`defi_activation_height = u64::MAX`), so `unique_id` writes are minimal, but the functionality must exist for when DeFi activates.

**What I considered**: Moving `unique_id` tracking to a 7th CF in state_db would eliminate utxo_store entirely. However, this is a CODE change (not a configuration change) and is outside the scope of a pure RocksDB configuration redesign. It would also add a CF to the consensus-critical state_db, increasing its memtable footprint.

**KEEP but AGGRESSIVELY MINIMIZE**: Treat utxo_store as a fully rebuildable, non-durable store. Disable WAL. Minimize memtables. No bloom filters (data mirrors state_db which has bloom filters). Minimal block cache.

**Kill test for dropping**: Would the node function without utxo_store? Only if unique_id checks were moved elsewhere. Since they haven't been, dropping breaks NFT minting validation. KILL TEST FAILED -- cannot drop.

**Subtraction note**: The entire utxo_store DB is architecturally redundant for UTXO data (state_db is authoritative). Only the unique_id CF justifies its existence. A future code change could add unique_id to state_db and eliminate this entire instance, saving ~3 CFs of memtable overhead. Flag for synthesizer.

conf(0.6, inferred) -- cannot verify from docs whether utxo_store is truly needed vs. migration artifact.

### 3. WAL on utxo_store -- DROP (disable WAL entirely)

**Evidence**: 
- Architecture doc (line 913-914): "excludes utxo_store/ (self-healed on startup to avoid INC-I-027 silent corruption)"
- Analyst doc (line 157): "Self-heals from state_db on startup. WAL replay as first attempt... WAL replay is unnecessary given self-heal capability."
- Crash recovery profile: "self-heals from state_db (fast, WAL replay unnecessary)"

**Mechanism**: On crash, the node restarts, detects utxo_store inconsistency (or missing data), and rebuilds from state_db's cf_utxo + cf_utxo_by_pubkey + UTXO scan for unique_id. WAL replay would only recover the last few writes that will be immediately overwritten by the self-heal.

**Cost of disabling**: Zero -- the self-heal path is the DESIGNED recovery mechanism. WAL replay is wasted I/O.

**Kill test**: Is there any scenario where utxo_store data that survived a crash (via WAL) is needed BEFORE self-heal runs? No -- self-heal replaces everything unconditionally. PASSED.

**Implementation**: `WriteOptions::set_disable_wal(true)` on every write to utxo_store, or set at the DB options level. In RocksDB's Rust bindings, this is `WriteOptions::disable_wal(true)`.

conf(0.7, observed) -- self-heal is documented in architecture and confirmed by node-heal exclusion pattern.

### 4. WAL on diagnostic_ledger -- DROP (disable WAL entirely)

**Evidence**:
- Analyst doc (line 158): "Loss of recent events acceptable. Graceful degradation to NoOpEmitter if DB fails."
- Crash recovery profile: "lossy ok; NoOp fallback"
- Design brief (line 68): "Pure observability; consensus never reads from it."
- The entire DB only opens when `--fork-diagnostics` is passed; defaults to NoOpEmitter.

**Mechanism**: On crash, losing the last batch of diagnostic events (~10 events or 100ms) has zero impact. The pruner already discards events older than 30 days. The NoOp fallback means the node can run without ANY diagnostic data.

**Kill test**: Is there any scenario where diagnostic event persistence across crash matters? No -- the data is purely observational and the pruner keeps only recent data anyway. PASSED.

conf(0.7, observed) -- lossy nature explicitly documented.

### 5. Block cache on block_store -- SIMPLIFY (reduce to 2 MB)

**Evidence**: block_store access patterns:
- **Writes**: Append-only (new blocks). No random writes. No updates.
- **Sequential reads**: `set_canonical_chain()` backward walk, sync GetHeaders batches. These THRASH a block cache because they scan consecutive-but-different keys.
- **Random reads**: RPC getBlock by hash (cold, infrequent), validation checks.

**Analysis**: Sequential reads dominate the hot path. A block cache optimizes RANDOM re-reads of the same data, which are rare for block_store. The dominant access pattern (backward canonical walk during set_canonical_chain) reads each block header once and moves on -- classic sequential scan that evicts itself from cache before re-use.

The default 8 MB block cache serves no purpose for sequential access. A 2 MB cache is sufficient to hold a small working set of recently-accessed headers for RPC queries.

**Kill test**: Would reducing the block cache to 2 MB cause latency regression on the sync GetHeaders hot path? No -- GetHeaders is a sequential scan; cache hits are unlikely regardless of cache size because each header is read once per scan. The OS page cache handles sequential reads efficiently. PASSED.

conf(0.5, inferred) -- I cannot measure the actual cache hit rate without profiling. The inference is from access pattern analysis.

### 6. `tx_index` and `addr_tx_index` CFs in block_store -- SIMPLIFY (reduce memtable to 512 KB)

**Evidence from analyst workload inventory**:
- `tx_index`: write-hot (one entry per tx per block), read-cold (RPC getTxBlockHeight only)
- `addr_tx_index`: write-hot (one entry per unique address per block), read-cold (RPC getAddressHistory only)
- Both are REBUILDABLE secondary indexes

**Write rate**: At ~6 txns/block * 6 blocks/min:
- `tx_index`: ~36 writes/min * 40B (key+value) = 1.4 KB/min. A 512 KB buffer fills in 6 hours.
- `addr_tx_index`: ~36 writes/min * 40B (key) = 1.4 KB/min. Same math.

Even at 100x sync burst: 140 KB/min, 512 KB fills in 3.6 minutes. Plenty of time for background flush.

**Why 512 KB and not less**: RocksDB's minimum practical memtable is ~64 KB, but very small memtables produce tiny L0 files, increasing read amplification on compaction. 512 KB balances memory savings with reasonable L0 file size.

**Kill test**: Could reducing from 64 MB to 512 KB cause write stalls on these CFs? Only if the write rate exceeds the flush rate, which at 1.4 KB/min (or 140 KB/min burst) is impossible. Flush to L0 takes milliseconds for a 512 KB file. PASSED.

conf(0.7, observed) -- write rates derived from block structure documented in analyst inventory.

### 7. Bloom filters -- SELECTIVE (only on point-lookup-heavy CFs)

**Bloom filters cost memory** (bits_per_key * number_of_keys / 8 bytes per SST file's filter block) but save disk reads on negative point lookups by answering "definitely not here" without reading the data block.

**CFs that benefit from bloom filters** (point-lookup dominant):
- `cf_utxo` (state_db): millions of entries, point lookup on EVERY validation. FPR at 10 bits/key with 10M entries = ~1%. This saves one disk read per ~100 negative lookups. On the hot validation path, this matters. **ADD bloom, 10 bits/key.**
- `headers` (block_store): point lookup by hash during validation and sync. Tens of thousands of entries. **ADD bloom, 10 bits/key.**
- `hash_to_height` (block_store): point lookup by hash for O(1) height resolution. **ADD bloom, 10 bits/key.**
- `cf_producers` (state_db): point lookup by pubkey_hash. Small cardinality (~30-100 producers). Bloom filter costs almost nothing. **ADD bloom, 10 bits/key.**

**CFs where bloom filters are NOISE** (range scan, sequential, or too small):
- `height_index` (block_store): range scan / sequential access. Bloom doesn't help range queries. **SKIP.**
- `slot_index` (block_store): same as height_index. **SKIP.**
- `cf_utxo_by_pubkey` (state_db): prefix scan (all UTXOs for a pubkey). Bloom doesn't accelerate prefix iteration. **SKIP.** (Could use prefix bloom, but the cardinality is moderate and prefix scans are RPC-only, not hot-path.)
- `tx_index` (block_store): cold RPC reads. Not worth the memory. **SKIP.**
- `addr_tx_index` (block_store): prefix scan for address history. **SKIP.**
- `cf_undo` (state_db): keyed by height (sequential u64), never negative lookups. **SKIP.**
- `cf_exit_history` (state_db): very few entries, cold reads. Bloom is cheap but unnecessary. **SKIP** (borderline).
- `cf_meta` (state_db): tiny number of keys, always positive lookups (known key names). **SKIP.**
- `presence` (block_store): dead CF. **SKIP.**
- `meta` (block_store): 1 key. **SKIP.**
- All utxo_store CFs: mirrors state_db; if state_db has blooms, utxo_store doesn't need them for correctness. In-memory UtxoSet handles the hot reads. **SKIP.**
- diagnostic_ledger: range scan for pruner, cold RPC reads. **SKIP.**

**Kill test**: Could adding bloom filters to cf_utxo cause problems? Bloom filters are pure read-optimization; they can never cause incorrect behavior. They increase memory proportional to key count (10M keys * 10 bits = 12.5 MB in filter data across all SST files, loaded into block cache on demand). This is a reasonable cost. PASSED.

conf(0.6, inferred) -- bloom filter benefit depends on actual cache miss rate which I cannot measure.

### 8. `min_write_buffer_number_to_merge` above 1 -- KEEP DEFAULT (1)

**Evidence**: This parameter controls how many immutable memtables are merged before flushing to L0. The default is 1 (flush each memtable individually). Setting it higher reduces write amplification by merging multiple memtables before creating an L0 file, at the cost of holding more memtables in memory longer.

**For DOLI's workload**: Write rates are low (35-48 KB/min per instance at steady state). Merging multiple memtables before flush would mean holding memtables for hours waiting for a second one to fill. This delays data reaching the SST layer, increasing read latency for data that's only in memtables. No benefit.

**Verdict**: KEEP at 1. No complexity to add.

conf(0.7, observed) -- low write rates make multi-merge pointless.

### 9. `max_subcompactions` > 1 -- KEEP DEFAULT (1)

**Evidence**: Subcompactions parallelize a single compaction job across multiple threads by partitioning the key range. This helps when:
- Key ranges are large and partitionable
- Compaction is a bottleneck
- CPU cores are abundant

**For DOLI**: Write rates are low enough that compaction keeps up easily with a single thread. Block_store keys are hashes (random distribution, good for partitioning) but compaction is never a bottleneck at 35 KB/min. State_db cf_utxo has high churn but the volume is still modest (7 KB/min steady).

**Verdict**: KEEP at 1. Adding parallelism would consume CPU for no measurable benefit.

conf(0.6, inferred) -- cannot measure compaction pressure without profiling.

### 10. Collapse four instances to fewer? -- NO

**Evidence**: The four instances have genuinely different lifecycles:
- `block_store`: append-only, consensus-critical, WAL needed, never self-heals
- `state_db`: read-modify-write, consensus-critical, WAL needed, authoritative
- `utxo_store`: mirrors state_db, fully rebuildable, WAL unnecessary
- `diagnostic_ledger`: opt-in, observability, WAL unnecessary, NoOp fallback

**Why not merge**:
- `block_store` + `state_db`: Different write patterns (append vs read-modify-write). Different compaction needs. Different backup/checkpoint semantics (block_store can be pruned; state_db cannot). Merging would force shared WAL and shared `db_write_buffer_size`, preventing per-instance tuning.
- `utxo_store` into `state_db`: Would add 3 CFs to the consensus-critical DB, increasing its memtable footprint and complicating the atomic WriteBatch. The `unique_id` CF has different lifecycle from UTXO data.
- `diagnostic_ledger` into anything: Violates the observability isolation principle. A failing diagnostic DB should not affect consensus.

**Kill test**: Would merging block_store and state_db simplify anything? No -- it would create a single DB with 15 CFs, shared WAL, shared `db_write_buffer_size`, and mixed durability requirements. Complexity increases, not decreases. KILL TEST PASSED (cannot merge).

conf(0.7, observed) -- lifecycle differences are documented and architecturally sound.

## Concrete Configuration per Instance

### block_store (9 CFs, 8 active + 1 dead)

**Write rate**: ~35 KB/min steady, ~3.5 MB/min at 100x burst

| Parameter | Value | Derivation |
|-----------|-------|------------|
| `db_write_buffer_size` | 16 MB | Total memtable cap. At 100x burst (3.5 MB/min), fills in 4.5 min. At steady state, 7.6 hours. Generous headroom for concurrent CF memtables during sync. |
| `max_total_wal_size` | 32 MB | 2x db_write_buffer_size. Ensures WAL rotation happens before WAL grows unbounded. Prevents dead CFs from pinning WAL segments. |
| `max_background_jobs` | 2 | Low write rate; 2 threads sufficient for flush + compaction. |
| `max_subcompactions` | 1 | No benefit at these write rates. |
| Shared block cache | 2 MB | Sequential reads dominate; cache has low hit rate. Just enough for RPC random lookups. |
| Compaction style | Level (default) | No reason to change. |
| `target_file_size_base` | 16 MB | Default 64 MB is excessive for CFs with tiny write rates. Smaller files = faster compaction. |
| `max_bytes_for_level_base` | 64 MB | 4x target_file_size_base. Default is fine. |
| `level0_file_num_compaction_trigger` | 4 | Default. Adequate. |
| `level0_slowdown_writes_trigger` | 20 | Default. Will never be reached at these write rates. |
| `level0_stop_writes_trigger` | 36 | Default. Unreachable. |

**Per-CF overrides:**

| CF | write_buffer_size | max_write_buffer_number | bloom (bits/key) | compression | Notes |
|----|-------------------|------------------------|-------------------|-------------|-------|
| `headers` | 4 MB | 2 | 10 | Lz4 (L0-L1), Zstd (L2+) | Hot writes + point lookups. ~1.8 KB/min = 37 hours to fill 4 MB. |
| `bodies` | 4 MB | 2 | None | Lz4 (all levels) | Largest values. ~30 KB/min = 2.2 hours to fill. No point lookups (accessed by hash via headers). |
| `height_index` | 1 MB | 2 | None | None | Tiny KV (8+32B). ~240 B/min. Range scan access, no bloom benefit. Small enough to skip compression. |
| `slot_index` | 1 MB | 2 | None | None | Same shape as height_index. |
| `hash_to_height` | 1 MB | 2 | 10 | None | Point lookups for O(1) height resolution. Tiny values (32+8B). |
| `tx_index` | 512 KB | 2 | None | None | Write-hot, read-cold (RPC only). 1.4 KB/min. |
| `addr_tx_index` | 512 KB | 2 | None | None | Write-hot, read-cold (RPC only). 1.4 KB/min. |
| `presence` | 256 KB | 1 | None | None | Dead CF. Minimum viable allocation. Never written. |
| `meta` | 256 KB | 1 | None | None | 1 key, written once on snap sync. |

**Total theoretical max**: (4+4+1+1+1+0.5+0.5+0.25+0.25) x 2 - (presence 1 memtable + meta 1 memtable) = 23 MB + 0.5 MB = 23.5 MB. Capped at 16 MB by `db_write_buffer_size`.

### state_db (6 CFs)

**Write rate**: ~48 KB/min steady, ~4.8 MB/min at 100x burst

| Parameter | Value | Derivation |
|-----------|-------|------------|
| `db_write_buffer_size` | 32 MB | Hottest instance. cf_utxo churn is highest. At 100x burst, fills in 6.7 min. At steady state, 11 hours. |
| `max_total_wal_size` | 64 MB | Already set at 64 MB; keep as-is. Matches 2x db_write_buffer_size. |
| `max_background_jobs` | 2 | cf_utxo churn justifies 2 threads. |
| `max_subcompactions` | 1 | No benefit. |
| Shared block cache | 8 MB | cf_utxo point lookups benefit from cache. Keep RocksDB default. |
| Compaction style | Level | Best for cf_utxo point-lookup-heavy reads. |
| `target_file_size_base` | 32 MB | cf_utxo will have the most data; 32 MB files are reasonable. |
| `max_bytes_for_level_base` | 128 MB | 4x target_file_size_base. |
| `level0_file_num_compaction_trigger` | 4 | Default. |
| `level0_slowdown_writes_trigger` | 20 | Default. |
| `level0_stop_writes_trigger` | 36 | Default. |

**Per-CF overrides:**

| CF | write_buffer_size | max_write_buffer_number | bloom (bits/key) | compression | Notes |
|----|-------------------|------------------------|-------------------|-------------|-------|
| `cf_utxo` | 8 MB | 2 | 10 | Lz4 | Hottest CF. Millions of entries. Point lookups on every validation. 7.2 KB/min = 18.5 hours to fill 8 MB. Bloom critical for negative lookup avoidance. |
| `cf_utxo_by_pubkey` | 4 MB | 2 | None | None | Mirrors cf_utxo writes. Prefix scan access (no point-lookup bloom benefit). Key-only (value = 0x00), no compression benefit. |
| `cf_producers` | 2 MB | 2 | 10 | Lz4 | Epoch-boundary-only writes. Small cardinality (~30-100 entries). Bloom is cheap and helps RPC lookups. |
| `cf_exit_history` | 512 KB | 1 | None | None | Near-zero writes. ~0 in steady state. |
| `cf_meta` | 4 MB | 2 | None | None | Hot writes (chain_state + accumulators every block). Many small KVs. ~3 KB/min. No bloom (always positive lookups on known keys). |
| `cf_undo` | 4 MB | 2 | None | Zstd | Large values (1-100+ KB each). One write per block. Zstd compression justified for large bincode blobs. Only read on rollback (rare). |

**Total theoretical max**: (8+4+2+0.5+4+4) x 2 - (cf_exit_history 1 memtable) = 44.5 MB. Capped at 32 MB by `db_write_buffer_size`.

### utxo_store (3 CFs -- keep but minimize)

**Write rate**: ~15 KB/min steady

| Parameter | Value | Derivation |
|-----------|-------|------------|
| `db_write_buffer_size` | 8 MB | Fully rebuildable. Minimum viable. At 100x burst (1.5 MB/min), fills in 5.3 min. |
| `max_total_wal_size` | N/A | **WAL disabled.** Self-heals from state_db. |
| WAL | **DISABLED** | Set `WriteOptions::disable_wal(true)` on all writes. Or `Options::set_manual_wal_flush(true)` + never flush. |
| `max_background_jobs` | 1 | Minimal. Rebuildable store. |
| `max_subcompactions` | 1 | No benefit. |
| Block cache | 1 MB | In-memory UtxoSet handles hot reads. RocksDB reads are fallback only. |
| Compaction style | Level | Default. |
| Bloom filter | None (all CFs) | In-memory UtxoSet provides the hot-path lookups. RocksDB backend is for persistence/fallback. State_db bloom filters handle negative lookups on the authoritative store. |
| `target_file_size_base` | 16 MB | Low write rate, moderate data volume. |
| `max_bytes_for_level_base` | 64 MB | 4x target. |

**Per-CF overrides:**

| CF | write_buffer_size | max_write_buffer_number | Notes |
|----|-------------------|------------------------|-------|
| `utxo` | 4 MB | 2 | Mirrors state_db cf_utxo. |
| `utxo_by_pubkey` | 2 MB | 2 | Mirrors state_db cf_utxo_by_pubkey. |
| `unique_id` | 512 KB | 1 | Near-zero writes (DeFi gated at u64::MAX). |

**Total theoretical max**: (4+2+0.5) x 2 - (unique_id 1 memtable) = 12.5 MB. Capped at 8 MB by `db_write_buffer_size`.

### diagnostic_ledger (1 CF)

**Write rate**: ~240 KB/min when active (batched); 0 when NoOp

| Parameter | Value | Derivation |
|-----------|-------|------------|
| `db_write_buffer_size` | 8 MB | **Keep existing INC-I-102 cap.** Workload-justified: at 240 KB/min peak, this gives 33 min of buffering. Even at 10x spike (fork storm), 3.3 min. More than adequate. |
| `max_total_wal_size` | N/A | **WAL disabled.** Lossy ok. |
| WAL | **DISABLED** | Observability data. Loss of recent events is acceptable. |
| `max_background_jobs` | 1 | Minimal. Single CF, low write rate. |
| Block cache | 512 KB | Cold reads (RPC only, debug). Pruner uses iterator, not cache. |
| Compaction style | Level | Default. Single CF, simple. |
| `write_buffer_size` | 4 MB | Half of total cap. At 240 KB/min, fills in 16.7 min. |
| `max_write_buffer_number` | 2 | Default. One active + one flushing. |
| `target_file_size_base` | 8 MB | Low write volume. |
| Bloom filter | None | Range scan access for pruner. Point lookups are RPC-only (cold). |
| Compression | Lz4 | Diagnostic events contain hex strings (compressible). |

## Summary of Memory Impact

### Before (current)

| Instance | Theoretical memtable max | Block cache | WAL overhead | Total theoretical |
|----------|------------------------|-------------|-------------|-------------------|
| block_store | 1,152 MB | 8 MB | unbounded | ~1,160 MB |
| state_db | 768 MB | 8 MB | 64 MB | ~840 MB |
| utxo_store | 384 MB | 8 MB | unbounded | ~392 MB |
| diagnostic_ledger | 8 MB | 8 MB | ? | ~16 MB |
| **Total** | **2,312 MB** | **32 MB** | **unbounded** | **~2,408 MB** |

### After (proposed)

| Instance | Theoretical memtable max | Block cache | WAL overhead | Total theoretical |
|----------|------------------------|-------------|-------------|-------------------|
| block_store | 16 MB (capped) | 2 MB | 32 MB | ~50 MB |
| state_db | 32 MB (capped) | 8 MB | 64 MB | ~104 MB |
| utxo_store | 8 MB (capped) | 1 MB | 0 (disabled) | ~9 MB |
| diagnostic_ledger | 8 MB (capped) | 0.5 MB | 0 (disabled) | ~8.5 MB |
| **Total** | **64 MB** | **11.5 MB** | **96 MB** | **~171.5 MB** |

**Reduction**: ~2,408 MB theoretical -> ~171.5 MB theoretical = **93% reduction** in theoretical maximum RocksDB memory footprint per node.

**Observed impact**: The actual plateau of ~450 MB/node (which includes non-RocksDB memory) would drop to approximately ~200-250 MB/node. Family server with 6 nodes: 6 x 250 = 1.5 GB, fitting within 1.9 GB total RAM.

## What I Cannot Remove and Why

1. **state_db WAL**: Consensus-critical. The atomic WriteBatch guarantees crash consistency. Without WAL, a crash during flush could leave state_db partially written, causing state root divergence. The `last_applied` canary DETECTS inconsistency but does not PREVENT it -- it requires snap sync recovery, which is expensive.

2. **block_store WAL**: Consensus-critical for headers and bodies. A crash that loses a committed block header could cause the canonical chain walk to fail. While blocks can be re-fetched from the network, WAL replay is faster and more reliable.

3. **state_db block cache**: cf_utxo point lookups on the validation hot path benefit from caching recently-accessed UTXOs (e.g., during mempool validation of multiple transactions spending from the same UTXO).

4. **Level compaction on state_db**: cf_utxo's access pattern is dominated by point lookups with high read amplification sensitivity. Level compaction's sorted runs optimize point lookup reads at the cost of higher write amplification -- the correct tradeoff for a read-heavy workload.

5. **utxo_store instance**: Cannot remove without moving unique_id tracking to state_db (a code change beyond configuration scope).

## Constraints Identified

**C1**: `db_write_buffer_size` MUST be > 0 on all 4 instances. Current 0 (uncapped) on 3 of 4 is the root cause of OOM.

**C2**: Per-CF `write_buffer_size` must be large enough that at 100x sync burst, the memtable flush rate keeps up with the write rate. The floor is approximately 512 KB for any actively-written CF (below this, L0 files become too small and compaction overhead increases).

**C3**: `max_total_wal_size` must be > 0 on instances with WAL enabled (block_store, state_db). Otherwise dead/cold CFs pin WAL segments indefinitely.

**C4**: Disabling WAL on utxo_store and diagnostic_ledger is safe ONLY because both have non-WAL recovery paths (self-heal and NoOp fallback respectively). Any future change that makes these instances non-rebuildable would require re-enabling WAL.

**C5**: `presence` CF in block_store must remain in the CF descriptor list for backward compatibility with existing data directories. Its memtable budget can be minimized but the CF cannot be removed from the descriptor without migration code.

**C6**: Bloom filters on cf_utxo should use full key bloom (not prefix bloom) because lookups are by exact Outpoint (36 bytes). Prefix bloom would be inappropriate for cf_utxo but appropriate for cf_utxo_by_pubkey if prefix lookups were on the hot path (they aren't -- RPC only).

**C7**: The diagnostic_ledger 8 MB cap (INC-I-102) is workload-justified at ~240 KB/min peak write rate and should be preserved.

## Cross-Perspective Signals

1. **utxo_store architectural redundancy**: The entire utxo_store DB exists because the in-memory UtxoSet needs a persistence layer separate from state_db. However, state_db already stores the same UTXO data in cf_utxo + cf_utxo_by_pubkey. The only unique value of utxo_store is the `unique_id` CF. A restructuring evaluator should consider whether adding `unique_id` as a 7th CF in state_db would eliminate the entire utxo_store instance.

2. **Shared block cache opportunity**: Currently each RocksDB instance gets its own default block cache (~8 MB each = 32 MB total). A shared LRU cache across instances would let state_db's hot cf_utxo data compete fairly with block_store's cold block data, resulting in better effective caching. The patterns evaluator should assess whether a single shared cache (e.g., 16 MB total) would outperform four separate 8 MB caches.

3. **Compression opportunity on cf_undo**: UndoData entries contain bincode-serialized ProducerSet snapshots (300-600B per producer x number of producers) plus spent UTXOs. These are highly compressible. Zstd compression could reduce disk I/O and storage, which matters for the undo log's 2000-block retention window.

4. **block_store bodies CF could benefit from dictionary compression**: Block bodies contain repetitive structure (transaction format, output types). Zstd with a trained dictionary could achieve better compression ratios than generic Zstd. However, this is an optimization beyond the scope of subtraction.

## Open Questions for Synthesizer

1. Should `utxo_store` WAL be disabled at the DB options level (globally for all CFs) or per-write via `WriteOptions`? DB-level is simpler but prevents any future per-write WAL control.

2. The analyst flagged uncertainty about whether `utxo_store` is opened unconditionally. If it's only opened for migration, the configuration matters less (it would be closed after migration). The synthesizer should ensure this is verified from `Node::new()` source.

3. Should the block cache be shared across all 4 instances (one `Cache::new_lru_cache()`) or per-instance? Sharing is simpler (one allocation, one LRU) but couples the instances' cache behavior. Given the vastly different access patterns, per-instance may be better for isolation. My recommendation: per-instance with the reduced sizes specified above.

4. The `min_write_buffer_number_to_merge = 1` default and `max_subcompactions = 1` default are both KEEP decisions. No evaluator needs to change these. But if another evaluator proposes changing them, the kill test is: demonstrate a measured compaction or flush bottleneck first.

## Sources Cited

- RocksDB documentation: `write_buffer_size` default is 64 MB, `max_write_buffer_number` default is 2, `db_write_buffer_size` default is 0 (uncapped). Source: RocksDB wiki "Basic Operations" and "Memory usage in RocksDB" (https://github.com/facebook/rocksdb/wiki/Memory-usage-in-RocksDB).
- RocksDB WAL behavior: WAL pinning occurs when a CF's memtable has not been flushed, preventing rotation of the WAL segment containing that CF's writes. Source: RocksDB wiki "Write Ahead Log" (https://github.com/facebook/rocksdb/wiki/Write-Ahead-Log).
- `disable_wal` in WriteOptions: documented in RocksDB API. Disables WAL for individual writes or can be set globally. Source: RocksDB C++ API WriteOptions::disableWAL.
- Bloom filter false positive rate: 10 bits/key = ~1% FPR. Source: RocksDB wiki "RocksDB Bloom Filter" (https://github.com/facebook/rocksdb/wiki/RocksDB-Bloom-Filter).
- `docs/redesigns/inc-i-104-redesign-analysis.md` -- analyst workload inventory, durability tiering, crash recovery profiles
- `docs/.workflow/design-brief.md` -- compiled context with root cause and acceptance criteria
- `docs/architecture.md` lines 240-270, 630-634, 910-915 -- storage architecture, utxo_store self-heal
- `.claude/skills/storage/SKILL.md` -- column family definitions, function signatures, data flow
- `docs/bugfixes/inc-i-104-handoff.md` -- fleet OOM evidence, regression timeline, RocksDB LOG dump findings
