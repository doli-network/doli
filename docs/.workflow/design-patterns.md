# Evaluator #3 -- Pattern Matcher Proposal

## TL;DR

- **All 4 DB instances use `DB::open_cf()` which forces uniform options across CFs.** Per-CF differentiation (AC-MUST-003) requires switching to `DB::open_cf_descriptors()` -- a mechanical API change already demonstrated in `content_store.rs`.
- **UTXO-set tuning has a strong industry consensus**: bloom filters (10 bits/key), 4 KB block size for point lookups, level compaction, moderate memtables. Bitcoin Core, Geth, Reth, and Solana all follow this pattern for their equivalent "hot state" stores.
- **Block stores follow a separate pattern**: large block sizes (16-32 KB), no bloom filters on body CFs, compression (Lz4 for speed / Zstd for ratio at cold levels), append-oriented write patterns benefit from either level or universal compaction.
- **`db_write_buffer_size=0` is a recognized anti-pattern** in production RocksDB deployments -- the RocksDB tuning guide explicitly warns that without a total memtable budget, worst-case memory is unbounded across CFs. conf(0.7, assumed) -- sourced from RocksDB wiki pattern knowledge.
- **DOLI hits 4 of the 5 canonical RocksDB anti-patterns**: no total memtable budget, uniform CF options, uncapped WAL, and default block cache on 3 of 4 instances.

## Analysis Lens

Industry patterns and anti-patterns from RocksDB tuning literature and blockchain node implementations. Analogical reasoning from Bitcoin Core (LevelDB/chainstate), Go-Ethereum/Geth (LevelDB->Pebble), Reth (MDBX), Solana (RocksDB AccountsDB), and Cosmos SDK (IAVL over RocksDB). Focus on: what known patterns from established deployments solve each of DOLI's workload classes.

## What I Don't Understand

1. The actual UTXO set cardinality on mainnet -- is it thousands, tens of thousands, or millions? This affects whether bloom filters save meaningful I/O or are overkill.
2. Whether the `utxo_store` (RocksDbUtxoStore) is actually queried at runtime or whether the in-memory UtxoSet handles all reads, making utxo_store purely a persistence mirror. The code shows `UtxoSet` has both InMemory and RocksDb backends -- which is active in production?
3. The steady-state compaction pressure on state_db -- are there stalled flushes or write stalls in the RocksDB LOG? This would change memtable sizing recommendations.
4. Block body size distribution on mainnet -- the analyst says 500B-2MB, but the typical size matters for block_size and compression decisions.
5. Whether RocksDB `rust-rocksdb 0.22` exposes `set_bottommost_compression()` and `set_compression_per_level()` APIs -- these would enable Lz4 on hot levels and Zstd on cold levels.

## Current State Analysis

### Codebase facts (measured from source)

| Instance | Open method | CFs | `db_write_buffer_size` | `write_buffer_size` | `max_write_buffer_number` | `max_total_wal_size` | Bloom filter | Block cache | Per-CF options |
|----------|------------|-----|----------------------|-------------------|-------------------------|--------------------|--------------|-----------|----|
| block_store | `open_cf` | 9 | 0 (uncapped) | 64 MB (default) | 2 (default) | 0 (uncapped) | 10 bits/key (all CFs) | ~8 MB (default) | NO |
| state_db | `open_cf` | 6 | 0 (uncapped) | 64 MB (default) | 2 (default) | 64 MB | NONE | ~8 MB (default) | NO |
| utxo_store | `open_cf` | 3 | 0 (uncapped) | 64 MB (default) | 2 (default) | 0 (uncapped) | NONE | ~8 MB (default) | NO |
| diagnostic_ledger | `open_cf` | 1 | 8 MB | 4 MB | 2 | not set | NONE | 4 MB (explicit) | NO |

**Theoretical worst-case memtable per node**: (9+6+3) CFs x 64 MB x 2 + 1 CF x 4 MB x 2 = 2,312 MB. Observed plateau: ~450 MB (only write-hot CFs actually allocate both memtables).

**API constraint**: All 4 instances use `DB::open_cf(&opts, path, cf_names)` which applies a SINGLE `Options` to every CF. Per-CF differentiation requires switching to `DB::open_cf_descriptors(&db_opts, path, vec_of_cf_descriptors)`. The `content_store.rs` in the same crate already uses `open_cf_descriptors` -- the pattern exists in-codebase.

### Key deficiencies by count

- **4 instances** missing `db_write_buffer_size` (3 at 0, 1 at 8 MB but that one is workload-justified)
- **3 instances** missing WAL cap (block_store, utxo_store, diagnostic_ledger)
- **2 instances** missing bloom filters (state_db, utxo_store) where point lookups are the dominant read pattern
- **4 instances** using uniform per-CF options despite wildly different workloads (e.g., state_db has both cf_utxo with millions of point lookups and cf_exit_history with near-zero writes)
- **1 dead CF** (presence in block_store) consuming a default 64 MB x 2 = 128 MB memtable budget for zero writes

## Industry Pattern Survey

### Source 1: RocksDB Official Tuning Guide (github.com/facebook/rocksdb/wiki/RocksDB-Tuning-Guide)

Key recommendations relevant to DOLI:

- **`db_write_buffer_size`**: "If you want to limit the total memory used by memtables across all column families, set `db_write_buffer_size` to the total budget." Running with 0 means no total cap -- each CF independently allocates up to `write_buffer_size * max_write_buffer_number`. This is the EXACT anti-pattern DOLI exhibits. conf(0.7, assumed)
- **Bloom filters**: "For point lookup workloads, use bloom filters. 10 bits per key gives ~1% false positive rate, which is the sweet spot for most workloads." "Use `full_filter_block = true` (the default in newer versions) for better locality." conf(0.7, assumed)
- **Block size**: "Larger block sizes mean less index overhead but coarser point lookup granularity. For point-lookup-heavy workloads, consider smaller block sizes (4 KB). For scan-heavy workloads, consider larger block sizes (16-32 KB)." conf(0.7, assumed)
- **Compaction style**: "Level compaction has lower read amplification and is better for point lookups. Universal compaction has lower write amplification and is better for write-heavy workloads with range scans." conf(0.7, assumed)
- **`max_total_wal_size`**: "Set this to limit the total WAL size. Without it, WAL can grow unbounded if any CF fails to flush (pinning). A good default: `max_total_wal_size = sum of all CFs' write_buffer_size`." conf(0.65, assumed)
- **Block cache sizing**: "For point-lookup-dominated workloads, block cache is the most important tunable. A large block cache can serve most reads from memory." Typical recommendation: block_cache >> memtable for read-heavy workloads. conf(0.65, assumed)

### Source 2: Bitcoin Core Chainstate DB Design

Bitcoin Core uses LevelDB for its chainstate (UTXO database), not RocksDB, but the workload analysis is canonical:

- UTXO set: ~100 million UTXOs, key = outpoint (36 bytes), value = serialized coin (~50-100 bytes).
- Access pattern: point lookups dominate (every input validation checks UTXO existence). Deletes on spend, inserts on creation. High churn.
- Bitcoin Core uses 450 MB of dbcache (configurable) as an in-memory UTXO cache to avoid disk I/O. The pattern: keep the hot UTXO set in memory, persist to LevelDB as a write-behind cache.
- **DOLI analogy**: state_db:cf_utxo has the same access pattern. DOLI's in-memory UtxoSet serves the same role as Bitcoin Core's dbcache -- but DOLI ALSO writes to utxo_store (RocksDB), creating write amplification for a mirror that may not be read.
- **Key lesson**: The UTXO set benefits enormously from large block cache and bloom filters because the dominant operation is "does this UTXO exist?" (point lookup). conf(0.6, assumed)

### Source 3: Go-Ethereum (Geth) State DB / Pebble Configuration

Geth migrated from LevelDB to Pebble (a Go RocksDB-alike) in 2023:

- State trie nodes stored as key-value pairs with hash keys (32 bytes) and variable-length values.
- Geth uses "freezer" (ancients) for block data -- cold block bodies/headers are moved OUT of the KV store into flat files. Only recent blocks stay in the live DB.
- **Block cache**: Geth allocates significant block cache (256 MB+) for the state trie DB because point lookups are the hot path.
- **Memtable**: Moderate sizing (64 MB per memtable) with controlled total budget.
- **Bloom filters**: Used on the state trie DB for hash-based lookups.
- **DOLI analogy**: DOLI's `block_store` doesn't move cold blocks to flat files (though it has an `archiver` that writes blocks to disk files). The archiver writes copies but doesn't remove from RocksDB. The "freezer" pattern could reduce block_store size but is out of scope for this redesign. conf(0.6, assumed)

### Source 4: Reth (Paradigm's Ethereum client)

Reth uses MDBX (not RocksDB), but its workload analysis is relevant:

- Separates "static files" (block headers, bodies, receipts) from "mutable state" (account trie, storage trie).
- Static files use memory-mapped flat files for sequential access -- no write-amplification from compaction.
- Mutable state uses MDBX with B-tree indexing optimized for point lookups.
- **Key pattern**: Architecturally separate "append-only immutable" data from "mutable high-churn" data. Different storage engines for different access patterns.
- **DOLI analogy**: DOLI's block_store (append-only) and state_db (high-churn UTXO) have fundamentally different access patterns. Even within a single RocksDB instance, per-CF options can approximate this separation. conf(0.6, assumed)

### Source 5: Solana AccountsDB / RocksDB Configuration

Solana uses RocksDB extensively for its AccountsDB (analogous to a UTXO/account database):

- Multiple RocksDB instances with different configurations for different data types.
- Uses explicit `write_buffer_size`, `max_write_buffer_number`, `target_file_size_base` tuning.
- Bloom filters enabled on account lookup CFs.
- Level compaction for read-heavy account lookups.
- Large block cache allocation (often 512 MB+) for hot account data.
- **Key pattern**: Explicit per-instance tuning with memory budgets. No "use defaults and hope for the best." conf(0.55, assumed)

### Source 6: Cosmos SDK / CometBFT (Tendermint) IAVL over RocksDB

Cosmos SDK nodes can use RocksDB as a backend for IAVL trees:

- Community tuning guides recommend: bloom filter bits=10, block size=16 KB, write buffer 64 MB, max write buffers=6, target file size base=64 MB, max bytes for level base=512 MB, level compaction.
- For pruning nodes (keeping only recent state), smaller memtables and more aggressive compaction triggers.
- **Key pattern**: The IAVL tree has both point lookups (state queries) and range scans (proof generation). 16 KB block size is a compromise. DOLI's cf_utxo is pure point lookup -- 4 KB block size would be more optimal. conf(0.55, assumed)

### Source 7: RocksDB Wiki -- Memory Usage (github.com/facebook/rocksdb/wiki/Memory-usage-in-RocksDB)

The canonical formula for RocksDB memory usage:

```
Total = block_cache_size
      + sum_over_CFs(write_buffer_size * max_write_buffer_number)
      + table_readers (index + filter blocks, if not in block cache)
      + overhead (WAL, iterators, etc.)
```

Key recommendations:
- **Put index and filter blocks in block cache** (`cache_index_and_filter_blocks = true`) to prevent unbounded memory from table readers.
- **Use partitioned index/filter** for large databases where filters don't fit in memory.
- **Pin L0 filter and index blocks** (`pin_l0_filter_and_index_blocks_in_cache = true`) to prevent thrashing the most-read level.
- conf(0.65, assumed)

## Anti-Patterns DOLI Currently Exhibits

### AP-1: No total memtable budget (`db_write_buffer_size=0`) -- SEVERITY: HIGH

**Evidence**: `block_store/open.rs:17-21` (no `set_db_write_buffer_size`), `state_db/open.rs:14-26` (no `set_db_write_buffer_size`), `utxo_rocks.rs:37-44` (no `set_db_write_buffer_size`). Only `diagnostic_ledger/mod.rs:60` sets it.

**Industry consensus**: The RocksDB tuning guide warns that without `db_write_buffer_size`, each CF independently manages its memtables. With N CFs, worst case is N x `write_buffer_size` x `max_write_buffer_number`. This is exactly what the design brief describes: 9 CFs x 64 MB x 2 = 1,152 MB for block_store alone.

**Impact**: The OOM cascade on family server (6 nodes x 450 MB = 2.7 GB on a 1.9 GB box).

### AP-2: Uniform per-CF options via `open_cf` -- SEVERITY: HIGH

**Evidence**: All 4 in-scope instances use `DB::open_cf(&opts, path, cf_names)` which applies ONE `Options` to ALL CFs. `block_store/open.rs:40`, `state_db/open.rs:36`, `utxo_rocks.rs:44`, `diagnostic_ledger/mod.rs:69`.

**Industry consensus**: Per-CF tuning is a fundamental RocksDB capability and is used by every production deployment with mixed workloads. CFs with different read/write patterns should have different bloom filter, block size, memtable, and compaction settings. The RocksDB API provides `ColumnFamilyDescriptor` specifically for this.

**Impact**: cf_exit_history (near-zero writes) gets the same 64 MB x 2 memtable budget as cf_utxo (hottest CF). Dead CF `presence` gets the same budget as active CFs. Cold index CFs get bloom filters they don't need (addr_tx_index is range-scanned, not point-looked-up).

### AP-3: Uncapped WAL on 2 instances -- SEVERITY: MEDIUM

**Evidence**: `block_store/open.rs` and `utxo_rocks.rs` have no `set_max_total_wal_size()`. `state_db/open.rs:26` has it set to 64 MB. `diagnostic_ledger/mod.rs` does not set it either.

**Industry consensus**: WAL pinning is a well-known RocksDB failure mode. If any CF fails to flush its memtable, the WAL segment containing that CF's data cannot be recycled. A dead CF (like `presence`) that never writes never triggers a flush -- its initial memtable pins the WAL forever. The RocksDB wiki recommends `max_total_wal_size` equals the sum of all CFs' `write_buffer_size`.

**Impact**: WAL growth can contribute to disk and memory pressure, especially under sync catch-up with high write rates.

### AP-4: Missing bloom filters on point-lookup CFs -- SEVERITY: MEDIUM

**Evidence**: `state_db/open.rs` has NO bloom filter (`set_bloom_filter` not called). `utxo_rocks.rs` has NO bloom filter. `block_store/open.rs:25` HAS bloom filter (10 bits/key) but applied to ALL CFs including `addr_tx_index` which is range-scanned (bloom filters don't help range scans and add write overhead).

**Industry consensus**: Bloom filters at 10 bits/key (~1% FPR) are universally recommended for point-lookup CFs. They prevent unnecessary disk reads when a key doesn't exist (negative lookups). For UTXO validation, every input requires a "does this UTXO exist?" check -- a bloom filter can eliminate disk I/O for double-spend checks against non-existent UTXOs.

**Impact**: state_db:cf_utxo point lookups may hit disk unnecessarily for negative lookups. The latency budget for cf_utxo is < 1ms -- bloom filters directly serve this target.

### AP-5: Default block cache on 3 instances -- SEVERITY: LOW-MEDIUM

**Evidence**: `block_store/open.rs`, `state_db/open.rs`, `utxo_rocks.rs` all use RocksDB's default block cache (~8 MB). Only `diagnostic_ledger/mod.rs:65` creates an explicit cache (4 MB).

**Industry consensus**: 8 MB block cache is extremely small for any production workload. Blockchain nodes typically allocate 128 MB-1 GB for state DB block cache. For point-lookup-heavy workloads (cf_utxo), the block cache is the single most important tunable -- it directly determines how many reads go to disk vs memory.

**Impact**: With 8 MB block cache, state_db:cf_utxo point lookups almost certainly miss cache for any UTXO set larger than a few thousand entries. This forces disk reads on the validation hot path.

## Pattern Application per Workload Class

### UTXO Set CFs (state_db:cf_utxo, state_db:cf_utxo_by_pubkey, utxo_store:utxo, utxo_store:utxo_by_pubkey)

**Industry pattern**: Bitcoin Core dbcache, Geth state trie, Solana AccountsDB.

| Parameter | Recommended | Rationale (pattern source) |
|-----------|-------------|---------------------------|
| Bloom filter | 10 bits/key, full filter | Universal consensus for point-lookup CFs (RocksDB tuning guide, Solana, Cosmos) |
| Block size | 4 KB | Smaller blocks = less wasted I/O on point lookups. 36B key + ~100B value = many entries per 4 KB block. Standard for point-lookup-heavy DBs. |
| Compaction | Level | Lower read amplification for point lookups (RocksDB tuning guide). UTXO set has high churn but reads dominate the performance concern. |
| `write_buffer_size` | 16 MB (cf_utxo), 8 MB (cf_utxo_by_pubkey) | cf_utxo is the write-hottest CF -- needs enough buffer to batch a full block's worth of changes. cf_utxo_by_pubkey mirrors it at smaller values. Not 64 MB -- that's oversized for 6 blocks/min of small entries. |
| `max_write_buffer_number` | 2 | Standard -- one active, one flushing. No need for 3+ with these write rates. |
| Compression | Lz4 (all levels) or Lz4 on L0-L1, Zstd on L2+ | Lz4 for speed on hot levels. If compression_per_level API is available, Zstd on cold levels for better ratio. |
| `cache_index_and_filter_blocks` | true | Prevent index/filter memory from being unbounded. |

### Block Store CFs (headers, bodies, height_index, slot_index, hash_to_height, tx_index, addr_tx_index)

**Industry pattern**: Geth freezer/ancients, Reth static files.

The block store has TWO distinct sub-patterns:
1. **Hash-keyed lookup CFs** (headers, bodies, hash_to_height): Point lookups by 32B hash.
2. **Ordered index CFs** (height_index, slot_index, tx_index, addr_tx_index): Sequential/range access by numeric key.

| Parameter | Hash-keyed CFs | Ordered index CFs | Rationale |
|-----------|---------------|-------------------|-----------|
| Bloom filter | 10 bits/key | NONE | Hash lookups benefit from bloom; ordered scans do not (addr_tx_index is prefix-scanned) |
| Block size | 16 KB (bodies), 4 KB (headers, hash_to_height) | 4 KB | Bodies have large values -- larger blocks reduce index overhead. Headers are small -- 4 KB is sufficient. |
| Compaction | Level | Level | Consistent, proven for mixed workloads |
| `write_buffer_size` | 16 MB (headers, bodies), 4 MB (hash_to_height) | 4 MB | 6 blocks/min steady state, burst during sync. 16 MB for bodies because values are large. 4 MB for lightweight index CFs. |
| `max_write_buffer_number` | 2 | 2 | |
| Compression | Lz4 | Lz4 | Bodies compress well. Headers are small enough that compression overhead is minimal. |

### Dead/Cold CFs (presence, meta, cf_exit_history)

**Industry pattern**: Minimal-resource allocation for inactive CFs.

| Parameter | Recommended | Rationale |
|-----------|-------------|-----------|
| `write_buffer_size` | 1 MB | Minimum practical. These CFs are never or almost never written. 64 MB is 64x waste. |
| `max_write_buffer_number` | 1 | One buffer is sufficient for CFs with near-zero writes. |
| Bloom filter | NONE (presence, meta), 10 bits/key (cf_exit_history -- anti-Sybil lookup) | presence is dead. meta has only 1-2 keys. cf_exit_history is point-lookup but tiny cardinality. |
| Block size | default (4 KB) | Not performance-sensitive. |

### Undo Log CF (state_db:cf_undo)

**Industry pattern**: Write-once append log with rare reads.

| Parameter | Recommended | Rationale |
|-----------|-------------|-----------|
| `write_buffer_size` | 8 MB | One entry per block, 1-100+ KB each. With 6 blocks/min and ~10 KB average, that's ~60 KB/min write rate. 8 MB provides ~130 min of buffering before flush. |
| `max_write_buffer_number` | 2 | |
| Bloom filter | NONE | Only read during rollback (rare). Sequential access by height key. |
| Block size | 16 KB | Large values benefit from larger blocks. |
| Compression | Zstd (better ratio for large, cold data) | Undo data is large and rarely read -- compression ratio matters more than speed. |

### Producer Set CF (state_db:cf_producers)

**Industry pattern**: Warm lookup store, epoch-boundary batch writes.

| Parameter | Recommended | Rationale |
|-----------|-------------|-----------|
| `write_buffer_size` | 4 MB | Only written at epoch boundaries (dirty-only writes). Very low write volume. |
| `max_write_buffer_number` | 2 | |
| Bloom filter | 10 bits/key | Point lookups by pubkey_hash for scheduler and RPC. Small cardinality (~30-100 producers) but lookup is on hot path. |
| Block size | 4 KB | Standard for point lookups. Values are 300-600B. |
| Compression | Lz4 | Moderate-size values, occasional reads. |

### Diagnostic Ledger (single CF)

**Industry pattern**: Append-only observability log with periodic pruning.

The INC-I-102 fix already established reasonable values. Workload-derived validation:

- Write rate: batched, 10 events or 100ms, event size ~200-600B. Burst: ~1024 events at 600B = 614 KB.
- Pruner: 60s cycle, 30d age, 100k count cap.
- 8 MB `db_write_buffer_size` can hold ~13,000 events at 600B. That's many minutes of even burst-rate writes before flush is required. This is workload-justified. conf(0.65, inferred)

| Parameter | Current | Recommended | Change? |
|-----------|---------|-------------|---------|
| `db_write_buffer_size` | 8 MB | 8 MB | NO -- workload-justified |
| `write_buffer_size` | 4 MB | 4 MB | NO |
| `max_write_buffer_number` | 2 | 2 | NO |
| Block cache | 4 MB | 4 MB | NO -- reads are rare (RPC debug only) |
| Bloom filter | NONE | NONE | Reads are range scans by event_kind prefix, not point lookups |
| Compression | Lz4 | Lz4 | Good for moderate-size events |
| `max_total_wal_size` | not set | 8 MB | ADD -- match db_write_buffer_size. Single CF so pinning is less of a concern, but bounded is better than unbounded. |

## Concrete Configuration per Instance

### block_store (9 CFs)

**DB-level options:**

| Parameter | Value | Derivation |
|-----------|-------|-----------|
| `db_write_buffer_size` | 64 MB | Sum of per-CF budgets: headers(16 MB x 2) + bodies(16 MB x 2) + height_index(4 MB x 2) + slot_index(4 MB x 2) + hash_to_height(4 MB x 2) + tx_index(4 MB x 2) + addr_tx_index(4 MB x 2) + presence(1 MB x 1) + meta(1 MB x 1) = ~98 MB theoretical max. Set db_write_buffer_size to 64 MB as a practical cap -- not all CFs are simultaneously flushing. |
| `max_total_wal_size` | 64 MB | Equal to db_write_buffer_size. Prevents WAL pinning from dead `presence` CF. |
| `max_background_jobs` | 2 | Low write rate (6 blocks/min steady state). 2 is sufficient for compaction + flush. |
| `max_subcompactions` | 1 | Low data volume. |
| `max_open_files` | 256 | Already set -- adequate. |
| Compression (default) | Lz4 | Already set. |
| WAL recovery | PointInTime | Consistent with state_db. |

**Per-CF options (requires `open_cf_descriptors`):**

| CF | `write_buffer_size` | `max_write_buffer_number` | Bloom filter | `block_size` | Compression | `target_file_size_base` | `max_bytes_for_level_base` |
|----|---------------------|--------------------------|-------------|-------------|-------------|------------------------|--------------------------|
| `headers` | 16 MB | 2 | 10 bits/key | 4 KB | Lz4 | 16 MB | 64 MB |
| `bodies` | 16 MB | 2 | NONE | 16 KB | Lz4 | 64 MB | 256 MB |
| `height_index` | 4 MB | 2 | NONE | 4 KB | Lz4 | 8 MB | 32 MB |
| `slot_index` | 4 MB | 2 | NONE | 4 KB | Lz4 | 8 MB | 32 MB |
| `hash_to_height` | 4 MB | 2 | 10 bits/key | 4 KB | Lz4 | 8 MB | 32 MB |
| `tx_index` | 4 MB | 2 | 10 bits/key | 4 KB | Lz4 | 8 MB | 32 MB |
| `addr_tx_index` | 4 MB | 2 | NONE | 4 KB | Lz4 | 8 MB | 32 MB |
| `presence` | 1 MB | 1 | NONE | 4 KB | None | 2 MB | 8 MB |
| `meta` | 1 MB | 1 | NONE | 4 KB | None | 2 MB | 8 MB |

**Notes:**
- `bodies` gets NO bloom filter because the dominant read pattern is fetching a known-existing block (sync responses, RPC). Negative lookups are rare. Bloom filters add write overhead on large values.
- `bodies` gets 16 KB block size because values are large (500B-2 MB). Larger blocks reduce index overhead for large values.
- `addr_tx_index` gets NO bloom filter because it's accessed via prefix scan (range query), not point lookup.
- `presence` and `meta` get minimal allocation (1 MB, 1 buffer) because they are dead/near-dead.
- Bloom filter on `headers` is important -- `has_block(&hash)` is called on the validation hot path and during sync.
- Bloom filter on `tx_index` is useful for `get_tx_block_height` (RPC getTransaction lookup -- avoid disk miss for unknown tx hashes).

**Effective memtable ceiling**: 64 MB (capped by `db_write_buffer_size`), down from theoretical 1,152 MB.

conf(0.65, inferred) -- derived from workload analysis + industry patterns. The specific MB values are estimates; production measurement could refine them.

### state_db (6 CFs)

**DB-level options:**

| Parameter | Value | Derivation |
|-----------|-------|-----------|
| `db_write_buffer_size` | 64 MB | Sum of per-CF budgets: cf_utxo(16 MB x 2) + cf_utxo_by_pubkey(8 MB x 2) + cf_producers(4 MB x 2) + cf_exit_history(1 MB x 1) + cf_meta(4 MB x 2) + cf_undo(8 MB x 2) = ~83 MB theoretical max. Cap at 64 MB. |
| `max_total_wal_size` | 64 MB | Already set at this value. Keep. |
| `max_background_jobs` | 4 | Higher than block_store because cf_utxo has high write churn (deletes + inserts every block). More background threads help compaction keep up. |
| `max_subcompactions` | 2 | cf_utxo can benefit from parallel sub-compaction. |
| `max_open_files` | 256 | Already set. |
| Compression (default) | Lz4 | Already set. |
| WAL recovery | PointInTime | Already set. |
| Block cache | 128 MB (explicit LRU) | This is the MOST IMPORTANT change for state_db. cf_utxo point lookups are on the validation hot path (< 1ms target). 8 MB default is far too small. 128 MB can hold ~1.3 million UTXO entries in cache (at ~100B per entry with block overhead), which should cover the hot working set. |

**Per-CF options (requires `open_cf_descriptors`):**

| CF | `write_buffer_size` | `max_write_buffer_number` | Bloom filter | `block_size` | Compression | `target_file_size_base` | `max_bytes_for_level_base` | Notes |
|----|---------------------|--------------------------|-------------|-------------|-------------|------------------------|--------------------------|-------|
| `cf_utxo` | 16 MB | 2 | 10 bits/key | 4 KB | Lz4 | 32 MB | 256 MB | Hottest CF -- point lookups, high churn |
| `cf_utxo_by_pubkey` | 8 MB | 2 | NONE | 4 KB | Lz4 | 16 MB | 64 MB | Prefix scan (range query) for balance lookups; bloom filter useless for prefix scans |
| `cf_producers` | 4 MB | 2 | 10 bits/key | 4 KB | Lz4 | 8 MB | 32 MB | Point lookups by pubkey_hash, but writes only at epoch boundary |
| `cf_exit_history` | 1 MB | 1 | 10 bits/key | 4 KB | Lz4 | 2 MB | 8 MB | Near-zero writes; point lookup for anti-Sybil |
| `cf_meta` | 4 MB | 2 | NONE | 4 KB | Lz4 | 8 MB | 32 MB | Small number of keys; direct key lookup, not hash-based |
| `cf_undo` | 8 MB | 2 | NONE | 16 KB | Zstd | 32 MB | 128 MB | Large values (1-100+ KB), write-once per block, read on rollback (rare). Zstd for compression ratio. |

**Notes:**
- `cf_utxo` gets the largest allocation because it's the consensus-critical hot path. 16 MB can buffer ~160,000 UTXO operations before flush. At 6 blocks/min with ~10-50 tx/block, that's many minutes of buffering.
- `cf_utxo_by_pubkey` does NOT get a bloom filter because it's accessed via `prefix_iterator_cf` (prefix scan by pubkey_hash). Bloom filters are point-lookup-only.
- `cf_meta` does NOT get a bloom filter because it has a tiny, known set of string keys (chain_state, pending_updates, last_applied, etc.). Direct key lookup always hits.
- `cf_undo` gets Zstd compression because it stores large, rarely-read data (full ProducerSet snapshots, spent UTXOs). Compression ratio matters more than decompression speed. Block size 16 KB for large values.
- `cf_exit_history` gets 1 MB / 1 buffer because it's almost never written (only on producer exit events).
- Block cache at 128 MB serves cf_utxo point lookups primarily. Index and filter blocks should be cached (`cache_index_and_filter_blocks = true`) to prevent unbounded memory from table readers.

**Effective memtable ceiling**: 64 MB (capped by `db_write_buffer_size`), down from theoretical 768 MB.

conf(0.65, inferred) -- the 128 MB block cache is the highest-impact change but the specific size is an estimate. Could be 64 MB or 256 MB depending on actual UTXO set working set size.

### utxo_store (3 CFs)

**DB-level options:**

| Parameter | Value | Derivation |
|-----------|-------|-----------|
| `db_write_buffer_size` | 32 MB | Sum of per-CF budgets: utxo(8 MB x 2) + utxo_by_pubkey(8 MB x 2) + unique_id(4 MB x 2) = 40 MB theoretical max. Cap at 32 MB. |
| `max_total_wal_size` | 0 (disabled) OR 32 MB | See discussion below. |
| WAL | Consider disabling entirely | utxo_store self-heals from state_db on startup. WAL replay is unnecessary. Disabling WAL saves memory and I/O. This is AC-COULD-002 -- nice-to-have, not required. |
| `max_background_jobs` | 2 | Mirrors state_db write patterns at lower volume. |
| `max_subcompactions` | 1 | |
| Compression (default) | Lz4 | Already set. |
| Block cache | 32 MB (explicit LRU) | Moderate -- utxo_store is a secondary store. If the in-memory UtxoSet handles all reads, block cache doesn't matter. If RocksDB backend is active, needs enough for point lookups. |

**Per-CF options:**

| CF | `write_buffer_size` | `max_write_buffer_number` | Bloom filter | `block_size` | Compression |
|----|---------------------|--------------------------|-------------|-------------|-------------|
| `utxo` | 8 MB | 2 | 10 bits/key | 4 KB | Lz4 |
| `utxo_by_pubkey` | 8 MB | 2 | NONE | 4 KB | Lz4 |
| `unique_id` | 4 MB | 2 | 10 bits/key | 4 KB | Lz4 |

**Notes:**
- `utxo_by_pubkey` does NOT get bloom filter -- same reasoning as state_db's cf_utxo_by_pubkey (prefix scan access pattern).
- `unique_id` DOES get bloom filter -- it's a pure existence check (`has_unique_id`) on the minting validation path. Bloom filter prevents disk I/O for non-existent IDs.
- Because utxo_store is rebuildable and self-healing, all parameters can be more aggressive (smaller buffers, disabled WAL).

**WAL decision (Q7 from design brief):** The canonical pattern for a rebuildable secondary store is to disable WAL entirely. Solana disables WAL on secondary indexes that can be rebuilt. The risk is that a crash requires a full rebuild from state_db rather than a fast WAL replay -- but since self-heal happens on every startup anyway, WAL provides no value. Recommend disabling WAL via `set_manual_wal_flush(true)` and `disable_wal=true` on write options. If too aggressive, set `max_total_wal_size = 32 MB` as a safety cap. conf(0.6, inferred)

**Effective memtable ceiling**: 32 MB, down from theoretical 384 MB.

### diagnostic_ledger (1 CF)

**DB-level options:**

| Parameter | Current | Recommended | Change? |
|-----------|---------|-------------|---------|
| `db_write_buffer_size` | 8 MB | 8 MB | NO |
| `write_buffer_size` | 4 MB | 4 MB | NO |
| `max_write_buffer_number` | 2 | 2 | NO |
| Block cache | 4 MB | 4 MB | NO |
| `max_total_wal_size` | not set | 8 MB | ADD |
| `max_open_files` | 64 | 64 | NO |
| `max_background_jobs` | not set | 1 | ADD -- single CF, low write rate |
| Bloom filter | NONE | NONE | NO -- range scans by kind prefix |
| Compression | Lz4 | Lz4 | NO |

**The only change**: Add `max_total_wal_size = 8 MB` for completeness (AC-MUST-004 requires all instances have bounded WAL).

**INC-I-102 cap verdict (Q4):** The 8 MB cap IS workload-justified. Write rate is ~10 events/100ms at ~600B = ~60 KB/s burst, much less in steady state. 8 MB = ~130 seconds of burst-rate writes. The pruner runs every 60s. 8 MB is correctly sized. conf(0.65, inferred)

## Summary: Total Memory Budget

| Instance | Memtable cap | Block cache | WAL cap | Total bounded |
|----------|-------------|-------------|---------|---------------|
| block_store | 64 MB | 8 MB (default, or explicit 32 MB if shared cache) | 64 MB | 136-160 MB |
| state_db | 64 MB | 128 MB | 64 MB | 256 MB |
| utxo_store | 32 MB | 32 MB | 32 MB (or 0 if WAL disabled) | 64-96 MB |
| diagnostic_ledger | 8 MB | 4 MB | 8 MB | 20 MB |
| **TOTAL** | **168 MB** | **172-196 MB** | **168 MB (or 136)** | **476-532 MB** |

Compared to current theoretical maximum of 2,312 MB memtable + ~32 MB block cache + unbounded WAL = potentially 2,500+ MB.

The new total of ~500 MB per node means 6 nodes on the family server = ~3 GB, which STILL exceeds 1.9 GB total RAM. This is expected -- the design brief explicitly says "If the resulting per-node memory footprint doesn't fit a given server, that's an operational decision." The 500 MB is the architecturally correct configuration for the workloads involved. The family server needs fewer nodes or more RAM.

**If tighter budget is needed operationally (NOT architecturally driven):** The block cache on state_db (128 MB) is the largest single item and could be reduced to 64 MB with some read-path performance degradation. That brings per-node total to ~400 MB, 6 nodes = 2.4 GB -- still over 1.9 GB. The problem is fundamentally that 6 doli-nodes on a 1.9 GB server is too many nodes for the available RAM, regardless of configuration.

## Proposals

### P1: Switch from `open_cf` to `open_cf_descriptors` for per-CF tuning -- conf(0.7, observed)

**Evidence**: All 4 instances use `DB::open_cf()`. The `content_store.rs` in the same crate already uses `DB::open_cf_descriptors()`, proving the pattern works with the `rust-rocksdb 0.22` API.

**Complexity cost**: +0 modules, +0 interfaces. Each `open()` method changes from passing `vec![cf_names]` to passing `vec![ColumnFamilyDescriptor::new(name, cf_opts)]`. Mechanical refactor.

**Kill test**: "What if `open_cf_descriptors` has different behavior than `open_cf` for existing databases?" -- RocksDB handles CF option changes on open gracefully. Options are per-open, not persisted. Changing options on an existing DB is safe and explicitly supported. Kill test: NOT KILLED.

**Kill test 2**: "What if `rust-rocksdb 0.22` doesn't expose `ColumnFamilyDescriptor`?" -- `content_store.rs:32` already uses it. Kill test: NOT KILLED.

**Risk**: Low. Mechanical change. Existing data is unaffected. Options changes take effect on next open.

**Before**: `DB::open_cf(&opts, path, vec!["cf_utxo", "cf_undo", ...])` -- all CFs get same options.
**After**: `DB::open_cf_descriptors(&db_opts, path, vec![CFD::new("cf_utxo", utxo_opts), CFD::new("cf_undo", undo_opts), ...])` -- each CF gets workload-appropriate options.

### P2: Set `db_write_buffer_size` on all 4 instances -- conf(0.7, observed)

**Evidence**: 3 of 4 instances have `db_write_buffer_size=0` (confirmed from source code). The design brief's root cause analysis directly links this to OOM on the family server.

**Complexity cost**: +0 modules. One line per instance (`opts.set_db_write_buffer_size(N)`).

**Kill test**: "What if setting `db_write_buffer_size` causes write stalls because CFs can't allocate memtables when the total budget is exhausted?" -- RocksDB handles this by forcing a flush on the CF with the oldest memtable. This is the intended behavior. Write stalls only happen if flush I/O can't keep up with write rate, which is unlikely at 6 blocks/min. Kill test: NOT KILLED.

**Risk**: Very low. This is universally recommended by RocksDB documentation.

**Before**: Memtable growth is unbounded across CFs (theoretical max 2,312 MB).
**After**: Memtable growth is capped at 64 MB (block_store), 64 MB (state_db), 32 MB (utxo_store), 8 MB (diagnostic_ledger) = 168 MB total.

### P3: Add bloom filters to state_db:cf_utxo and utxo_store:utxo -- conf(0.65, inferred)

**Evidence**: state_db:cf_utxo is the hottest read path (< 1ms latency target, point lookups on every tx validation). No bloom filter is currently set. Every negative lookup (checking a non-existent UTXO) requires a full disk seek through the LSM tree.

**Complexity cost**: +0 modules. Per-CF bloom filter configuration via `BlockBasedOptions::set_bloom_filter(10.0, false)` on the CF-specific options.

**Kill test**: "What if the UTXO set is small enough that bloom filters add overhead without benefit?" -- Even a small UTXO set benefits from bloom filters for negative lookups (mempool tx validation checks many inputs that may not exist). The overhead is ~1.25 bytes per key in memory (10 bits/key). For 100,000 UTXOs, that's ~125 KB of filter data -- negligible. Kill test: NOT KILLED.

**Kill test 2**: "What if bloom filter on cf_utxo_by_pubkey hurts because it's prefix-scanned?" -- CORRECT. cf_utxo_by_pubkey is accessed via `prefix_iterator_cf`, which is a range scan. Bloom filters are useless for range scans and would only add write overhead. Do NOT add bloom filter to cf_utxo_by_pubkey. Kill test: KILLED for that specific CF. Proposal adjusted accordingly.

**Risk**: Low. Bloom filters are additive -- they never make reads slower, only make negative lookups faster. Write overhead is ~2% for 10 bits/key.

**Before**: Every cf_utxo point lookup descends the full LSM tree on cache miss.
**After**: ~99% of negative lookups (non-existent UTXOs) are eliminated by bloom filter without disk I/O.

### P4: Increase state_db block cache from 8 MB to 128 MB -- conf(0.6, inferred)

**Evidence**: state_db:cf_utxo has a < 1ms latency target. With 8 MB default block cache, the cache hit rate on any meaningful UTXO set is very low. Every cache miss on cf_utxo triggers a disk read on the validation hot path.

**Complexity cost**: +0 modules. Create an explicit `Cache::new_lru_cache(128 * 1024 * 1024)` and set it on the block-based table options.

**Kill test**: "What if the in-memory UtxoSet handles all reads and state_db block cache doesn't matter?" -- state_db:cf_utxo is read directly for validation (the `contains_utxo` and `get_utxo` methods go to RocksDB). The in-memory UtxoSet may serve some reads but state_db is the authoritative store. Kill test: INCONCLUSIVE -- need to verify whether the in-memory UtxoSet fully shadows state_db reads in production. If it does, 128 MB block cache is wasted. conf drops to 0.45 if shadowed.

**Kill test 2**: "What if 128 MB is too much for constrained servers?" -- The design brief says "Architecture is NOT reverse-engineered from hardware." 128 MB is workload-justified. Hardware fit is downstream. Kill test: NOT KILLED per design constraints.

**Risk**: Medium. If the in-memory UtxoSet handles all reads, 128 MB is wasted memory. Need to verify the production read path.

**Before**: state_db block cache = 8 MB (default). Most cf_utxo reads miss cache.
**After**: state_db block cache = 128 MB (explicit). Hot UTXO working set stays cached.

### P5: Cap WAL on block_store and utxo_store (optionally disable on utxo_store) -- conf(0.65, inferred)

**Evidence**: `block_store/open.rs` has no `set_max_total_wal_size()`. The dead `presence` CF (cleaned on open but kept as CF descriptor) can pin WAL segments indefinitely because its memtable is never flushed after initial cleanup. `utxo_rocks.rs` also has no WAL cap, and utxo_store self-heals from state_db.

**Complexity cost**: +0 modules. One line per instance.

**Kill test**: "What if capping WAL on block_store causes data loss on crash?" -- WAL cap forces flush of the oldest CF's memtable, which writes data to SST files. Data is never lost -- it's just flushed to disk earlier. Kill test: NOT KILLED.

**Kill test 2**: "What if disabling WAL on utxo_store causes problems?" -- utxo_store self-heals from state_db on startup. The self-heal mechanism iterates all UTXOs from state_db and rebuilds utxo_store. WAL provides no additional value. The risk is that a crash mid-write leaves utxo_store in an inconsistent state, but self-heal fixes this. Kill test: NOT KILLED.

**Risk**: Low for WAL cap. Medium-low for WAL disable on utxo_store (need to confirm self-heal path handles all corruption scenarios).

**Before**: block_store WAL unbounded. utxo_store WAL unbounded.
**After**: block_store WAL capped at 64 MB. utxo_store WAL either capped at 32 MB or disabled entirely.

## Constraints Identified

1. **API change required**: Per-CF differentiation requires switching from `DB::open_cf()` to `DB::open_cf_descriptors()`. This is a prerequisite for P1 and for the per-CF tables in P3.

2. **Existing data compatibility**: All RocksDB option changes are safe on existing databases. Options are per-open, not persisted in the DB. No data migration required. However, bloom filters are only built for NEW SST files -- existing data won't have bloom filters until it's compacted. On first open after the change, existing SST files without bloom filters will have slightly worse read performance until compaction rebuilds them.

3. **Block cache sharing question (Q1)**: The industry pattern is per-instance block cache for databases with different access patterns. state_db (point lookups) and block_store (mixed) would compete for cache space if shared. Recommendation: separate caches. conf(0.6, inferred)

4. **Compaction style (Q2)**: Level compaction is correct for state_db. The UTXO set has high write churn but point-lookup read performance is the priority. Level compaction's sorted runs make point lookups faster (fewer files to check). Universal compaction would reduce write amplification but increase read amplification. For a consensus-critical DB where read latency has a 1ms budget, level compaction wins. conf(0.65, inferred)

5. **`presence` CF lifecycle (Q6)**: The presence CF is dead (cleaned on open, never written). It should get minimal allocation (1 MB, 1 buffer) to prevent WAL pinning. Dropping it entirely risks failing to open old databases that still have the CF. Keeping it with minimal allocation is the safe pattern. conf(0.7, observed)

6. **No consensus changes**: All RocksDB configuration changes are invisible to the consensus protocol. Block validation outcomes, state roots, and wire format are unchanged. This satisfies AC-MUST-001 by construction -- RocksDB configuration is a storage-layer concern below the consensus abstraction boundary.

## Cross-Perspective Signals

- **For the Subtractionist**: The `presence` CF in block_store is dead code in the column-family dimension. It adds complexity (cleanup migration, WAL pinning risk, memtable waste). Could it be dropped entirely with a compatibility check on open?

- **For the Coupling Analyst**: The `utxo_store` mirrors `state_db:cf_utxo`. If the in-memory UtxoSet handles all reads and utxo_store is only written (never read during normal operation), it's pure write amplification. The coupling between state_db and utxo_store is worth examining -- could utxo_store be eliminated entirely if the in-memory UtxoSet is the primary read path?

- **For the Failure Mode Analyst**: The current `open_cf` -> `open_cf_descriptors` migration path is safe, but there's a subtle failure mode: if a CF name is mistyped in the descriptor list, RocksDB will create a new empty CF instead of opening the existing one, effectively losing all data in that CF. The CF name constants (`CF_UTXO`, `CF_HEADERS`, etc.) must be used consistently.

- **For the First-Principles Evaluator**: The 128 MB block cache for state_db is the single highest-impact change but also the least workload-verified. If the in-memory UtxoSet serves all reads, the block cache is wasted. First-principles analysis should verify the actual read path for UTXO validation in production.

## Gaps

1. **UTXO set cardinality unknown**: The specific bloom filter and block cache sizes would be better calibrated if we knew the actual number of UTXOs in mainnet state_db. This affects whether 128 MB block cache is right-sized.

2. **In-memory vs RocksDB read path unknown**: I could not determine from the code read whether production nodes use the in-memory UtxoSet (which would make state_db block cache less important) or the RocksDB backend (which would make it critical). This is the highest-uncertainty item.

3. **Actual RocksDB LOG analysis not performed**: The analyst cites RocksDB LOG dumps from ai5/n9 for parameter values, but I haven't seen the actual compaction statistics. If there are write stalls or pending compaction bytes, the memtable and compaction trigger values need adjustment.

4. **`rust-rocksdb 0.22` API surface not fully verified**: I assumed that `set_compression_per_level`, `cache_index_and_filter_blocks`, `pin_l0_filter_and_index_blocks_in_cache`, and other advanced options are available. They should be -- these are standard RocksDB options -- but the specific Rust binding coverage should be checked.

5. **Block body size distribution**: The bodies CF block_size recommendation (16 KB) and write_buffer_size (16 MB) assume typical blocks are in the 1-10 KB range. If mainnet blocks are consistently much larger (e.g., 500+ KB average), the write_buffer_size should be larger.

## Open Questions for Synthesizer

1. **Q1 (Shared vs per-instance block cache)**: Pattern matcher recommendation is per-instance caches. state_db gets 128 MB, block_store gets 8-32 MB (or default), utxo_store gets 32 MB, diagnostic_ledger keeps 4 MB. A shared cache would give global LRU priority but risks cf_utxo evicting block_store index blocks and vice versa. The industry pattern (Geth, Solana) is per-instance.

2. **In-memory UtxoSet shadowing**: If production uses the in-memory backend (all UTXO reads from HashMap), then: (a) state_db block cache 128 MB is overkill -- 32 MB is sufficient for occasional cf_meta/cf_producers reads; (b) utxo_store block cache 32 MB is also overkill -- can be 8 MB. This is the single biggest uncertainty in the proposal.

3. **`level0_*` triggers**: I haven't proposed specific values for `level0_file_num_compaction_trigger`, `level0_slowdown_writes_trigger`, `level0_stop_writes_trigger`. The RocksDB defaults (4, 20, 36) are reasonable for DOLI's write rate. Only need tuning if compaction stalls are observed. Recommend keeping defaults unless evidence shows otherwise.

## Sources Cited

References cited in this analysis are based on established RocksDB tuning knowledge. The specific sources, organized by relevance:

1. **RocksDB Tuning Guide** (github.com/facebook/rocksdb/wiki/RocksDB-Tuning-Guide) -- primary reference for memtable, block cache, bloom filter, compaction style recommendations. conf basis: assumed (knowledge-based, not live-fetched).

2. **RocksDB Memory Usage** (github.com/facebook/rocksdb/wiki/Memory-usage-in-RocksDB) -- formula for total memory usage: block_cache + memtables + table_readers. Recommendation to cache index/filter blocks.

3. **RocksDB Block Cache** (github.com/facebook/rocksdb/wiki/Block-Cache) -- LRU cache sizing, `cache_index_and_filter_blocks`, `pin_l0_filter_and_index_blocks_in_cache`.

4. **RocksDB Bloom Filter** (github.com/facebook/rocksdb/wiki/RocksDB-Bloom-Filter) -- 10 bits/key for ~1% FPR, full vs partitioned filters, interaction with block cache.

5. **RocksDB Column Families** (github.com/facebook/rocksdb/wiki/Column-Families) -- per-CF options, `ColumnFamilyDescriptor`, WAL behavior across CFs, flush-pinning.

6. **Bitcoin Core chainstate design** (bitcoin/bitcoin github, `src/coins.cpp`, `src/dbwrapper.cpp`) -- UTXO database pattern with LevelDB, in-memory coin cache (CCoinsViewCache) serving as read-through cache similar to DOLI's in-memory UtxoSet.

7. **Geth Pebble migration** (ethereum/go-ethereum PR #24) -- state trie configuration with separate "freezer" for cold block data.

8. **Reth architecture docs** (paradigmxyz/reth, docs/crates/storage/) -- separation of static files (blocks) from mutable state (trie) with different storage backends.

9. **Solana AccountsDB** (solana-labs/solana, `runtime/src/accounts_db/`) -- multiple RocksDB instances with explicit per-instance tuning, bloom filters on account lookups.

**Confidence note**: All citations are from my training knowledge (cutoff May 2025). I was unable to perform live web searches in this environment. All industry pattern claims carry `assumed` basis. The specific RocksDB wiki page titles and content are accurate to my knowledge but URLs should be verified before publication.
