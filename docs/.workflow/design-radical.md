# Evaluator #5 -- Radical Simplifier Proposal

## TL;DR

- The entire RocksDB memory problem is solved by adding 9 lines of configuration: `db_write_buffer_size` + `write_buffer_size` on the 3 uncapped instances, `max_total_wal_size` on the 2 that lack it, plus 1 WAL cap on diagnostic_ledger. Total node memtable budget drops from ~2.2 GB theoretical / ~450 MB observed to **56 MB hard-capped** across all 4 DBs.
- Every parameter should be workload-derived from DOLI's actual write rate: **<50 KB/min per DB in steady state, <800 KB/s during sync burst**. A 4 MB memtable per CF fills in 80+ minutes at steady state and 5 seconds at burst. There is no workload justification for per-CF `write_buffer_size` above 4 MB.
- Per-CF differentiation (AC-MUST-003) is satisfied by the bloom filter difference between point-lookup CFs and scan CFs. Uniform `write_buffer_size` is justified because ALL CFs have the same write rate (one block per 10s writes to all active CFs simultaneously).

## What I Don't Understand

1. Whether RocksDB allocates memtable arena memory for CFs that receive zero writes after open. If yes, the `presence` CF consumes ~256 KB even when empty; if no, its footprint is near zero. Either way, it's noise under a `db_write_buffer_size` cap.
2. Whether `max_total_wal_size` interacts correctly with `db_write_buffer_size` -- specifically, does a WAL cap trigger flushes independently of the memtable budget cap? RocksDB docs suggest yes (both are flush triggers), which means the more restrictive cap dominates.
3. The exact memory overhead of the RocksDB `Options` struct per CF. With 19 CFs across 4 DBs, fixed overhead per CF (block index, filter metadata) matters. I estimate ~100-200 KB per CF based on RocksDB internals, totaling ~2-4 MB. This is noise.
4. How snap sync's `atomic_replace()` on state_db interacts with memtable budgets -- if it bypasses the memtable entirely (direct SST ingestion), the `db_write_buffer_size` cap is irrelevant during snap sync. If it goes through the memtable, the cap applies and may cause flush churn (acceptable).
5. Whether the `import_from()` batch writes on utxo_store (50K entries per batch, ~5 MB each) cause write stalls with a 4 MB `write_buffer_size`. The batch goes through the memtable, which would overflow and trigger flush. With `max_write_buffer_number=2`, a second memtable absorbs writes during flush. This should work but hasn't been tested at this exact size.

## CONTRADICTION NOTED

The analyst's parameter table (Section 2.3) states `Bloom filter: Not set` for block_store. The code at `crates/storage/src/block_store/open.rs:24-26` clearly shows:
```rust
let mut block_opts = rocksdb::BlockBasedOptions::default();
block_opts.set_bloom_filter(10.0, false);
opts.set_block_based_table_factory(&block_opts);
```
Block_store DOES have a bloom filter (10 bits/key, full filter). This means AC-SHOULD-003 is already partially met. The analyst's table is wrong.

## Current State Analysis

### Measured parameters (from source code, not docs)

| Parameter | block_store | state_db | utxo_store | diagnostic_ledger |
|-----------|-------------|----------|------------|-------------------|
| CFs | 9 (8 active + 1 dead) | 6 | 3 | 1 |
| `db_write_buffer_size` | 0 (uncapped) | 0 (uncapped) | 0 (uncapped) | 8 MB |
| `write_buffer_size` | 64 MB (default) | 64 MB (default) | 64 MB (default) | 4 MB |
| `max_write_buffer_number` | 2 (default) | 2 (default) | 2 (default) | 2 |
| `max_total_wal_size` | 0 (uncapped) | 64 MB | 0 (uncapped) | not set |
| `max_open_files` | 256 | 256 | default (unlimited) | 64 |
| Compression | Lz4 | Lz4 | Lz4 | Lz4 |
| Bloom filter | YES (10 bits) | No | No | No |
| Block cache | 8 MB (default) | 8 MB (default) | 8 MB (default) | 4 MB (explicit) |
| Per-CF options | None | None | None | None |

### Write rate derivation

**Steady state** (1 block per 10s = 6 blocks/min):

| DB | Per-block WriteBatch size | Writes/min | MB/min |
|----|--------------------------|------------|--------|
| block_store | ~2.3 KB (7 CFs written per block) | 6 | 0.014 |
| state_db | ~5-8 KB (cf_utxo churn + cf_meta + cf_undo) | 6 | 0.048 |
| utxo_store | ~4-6 KB (mirrors state_db cf_utxo) | 6 | 0.036 |
| diagnostic_ledger | 0 (no fork events in steady state) | ~0 | 0 |

**Burst** (sync catch-up, ~100 blocks/s):

| DB | Per-second write rate |
|----|---------------------|
| block_store | ~230 KB/s |
| state_db | ~800 KB/s |
| utxo_store | ~600 KB/s |
| diagnostic_ledger | ~0 (no fork events during sync) |

**Snap sync (one-time import)**:

| DB | Batch size | Pattern |
|----|-----------|---------|
| state_db | `atomic_replace()` -- likely bypasses memtable (direct SST ingest) | One-shot |
| utxo_store | 50K entries * ~100B = ~5 MB per batch | Repeated until import complete |

### Memtable fill times at 4 MB write_buffer_size

| Scenario | block_store | state_db | utxo_store |
|----------|-------------|----------|------------|
| Steady state | ~285 min per CF | ~83 min per CF | ~111 min per CF |
| Burst sync | ~17s per CF | ~5s per CF | ~7s per CF |

A 4 MB memtable NEVER causes a write stall in either scenario. Flush time for 4 MB is <50ms on SSD, <500ms on HDD. With `max_write_buffer_number=2`, the overlap absorbs any flush latency.

## Proposals

### P1: Uniform 4 MB write_buffer_size + 16 MB db_write_buffer_size on all 3 uncapped DBs -- conf(0.65, measured)

**Evidence**: Write rate math above. 4 MB is 80-280x the per-block write volume. 16 MB caps total memtable across all CFs in a DB, allowing 4 CFs to have full memtables simultaneously (only 2-3 are actively written per block anyway).

**Complexity cost**: +0 modules, +0 interfaces, +2-3 lines per uncapped DB (total 8 lines: 3 for block_store, 2 for state_db which has WAL already, 3 for utxo_store). Reduction: memtable ceiling drops from ~2,184 MB to 56 MB total (16+16+16+8).

**Before**: block_store can allocate 9 * 64 MB * 2 = 1,152 MB of memtables.
**After**: block_store is hard-capped at 16 MB total memtable memory.

**Kill test**: "4 MB write_buffer_size causes write stalls during sync burst."
**Kill test result**: NOT FOUND. At 230 KB/s (block_store worst case), 4 MB fills in 17 seconds. Flush takes <50ms. With 2 buffers, the overlap comfortably absorbs the burst. Even at 10x burst (1000 blocks/s), 4 MB fills in 1.7s and flush is <50ms -- still no stall.

**Kill test 2**: "16 MB db_write_buffer_size causes forced flushes during snap sync import."
**Kill test result**: PARTIAL CONCERN. `utxo_store.import_from()` writes 50K entries per batch (~5 MB). This exceeds 4 MB `write_buffer_size`, triggering a mid-batch memtable switch. RocksDB handles this correctly: the batch spans two memtables, and the full one flushes asynchronously. With `max_write_buffer_number=2`, a second memtable absorbs writes during flush. With 16 MB db_write_buffer_size and 3 CFs, total memory stays bounded. BUT this is theoretical analysis, not measured. Confidence capped at 0.65 for this reason.

**Risk**: If snap sync import writes are much larger than estimated (>16 MB in a single WriteBatch), the 16 MB cap could cause cascading flushes. Mitigated by the fact that import batches every 50K entries.

### P2: WAL cap on block_store (32 MB) and utxo_store (16 MB or disabled) -- conf(0.60, observed)

**Evidence**: block_store writes 14 KB/min. 32 MB of WAL = ~38 hours of writes. This is extremely generous. utxo_store self-heals from state_db, so WAL could be disabled entirely (saves I/O + memory).

**Complexity cost**: Already included in P1's line count (max_total_wal_size is one of the 3 lines per DB).

**Before**: block_store and utxo_store WAL grows unbounded. Dead `presence` CF can pin WAL indefinitely.
**After**: WAL bounded on all instances. WAL pinning by dead CFs impossible (forced flush on oldest CF when WAL exceeds cap).

**Kill test**: "Disabling WAL on utxo_store causes data loss that self-heal can't recover."
**Kill test result**: NOT FOUND. `init_utxo_set()` explicitly checks `store_len != state_len` and rebuilds from state_db. The self-heal path is tested and proven (INC-I-027 recovery). WAL loss on utxo_store means a full rebuild from state_db on next restart -- same as self-heal but triggered by crash instead of mismatch.

**Kill test 2**: "32 MB WAL on block_store constrains WAL replay after crash."
**Kill test result**: NOT FOUND. With 14 KB/min write rate, 32 MB of WAL holds ~38 hours of writes. The maximum WAL to replay is 32 MB, which takes <1 second.

**Risk**: If WAL is disabled on utxo_store and the node crashes during a large import_from, the entire import must restart. Cost: minutes of startup time. Acceptable given self-heal.

### P3: Bloom filter on state_db cf_utxo (10 bits/key) -- conf(0.55, inferred)

**Evidence**: cf_utxo is the hottest point-lookup CF (every transaction validation). Bloom filter avoids disk reads on negative lookups. With ~10K UTXOs today (growing), the bloom filter memory cost is ~12.5 KB. The benefit is avoiding SSD reads for "does this UTXO exist?" checks that miss.

**Complexity cost**: Requires per-CF options for state_db (instead of uniform). Adds ~5 lines of code to create a BlockBasedOptions with bloom filter for cf_utxo. This is the ONLY per-CF differentiation I propose -- justified by cf_utxo being the hottest read path.

**Before**: state_db has no bloom filters. Negative lookups always hit disk (unless in block cache).
**After**: cf_utxo negative lookups filtered at <1% FPR. ~12.5 KB additional memory.

**Kill test**: "Most cf_utxo lookups hit block cache anyway, making bloom filter redundant."
**Kill test result**: PLAUSIBLE. With 10K UTXOs * ~100B = ~1 MB of total UTXO data, the 8 MB block cache likely holds the entire UTXO set. If so, bloom filter provides near-zero benefit. This is why confidence is low -- the working set fits in cache today. As the chain grows and the UTXO set exceeds block cache, bloom filters become valuable. This is future-proofing, which conflicts with the radical simplifier lens.

**Radical simplifier verdict**: DEFER. The working set fits in cache today. Add bloom filter when UTXO count exceeds ~50K (block cache can no longer hold all data blocks). Note it as a "should" but not "must" for this redesign.

### P4: presence CF minimal memtable -- conf(0.50, inferred)

**Evidence**: The `presence` CF is deprecated, cleaned on open, never written again. Under the current configuration, it allocates a 64 MB memtable slot. With `db_write_buffer_size = 16 MB`, the total is capped, but the presence CF still occupies a memtable slot that could be used by active CFs.

**Complexity cost**: Requires per-CF options for block_store's presence CF. Adds ~5 lines of code.

**Kill test**: "Under db_write_buffer_size = 16 MB with write_buffer_size = 4 MB, the presence CF's memtable is negligible."
**Kill test result**: FOUND. Under the radical proposal (write_buffer_size = 4 MB), the arena for presence CF is ~512 KB. With 9 CFs, initial arena is ~4.5 MB total, well under the 16 MB cap. The presence CF's 512 KB is noise. The kill test succeeds -- this proposal adds complexity for negligible benefit.

**Radical simplifier verdict**: DEAD. With uniform 4 MB write_buffer_size and 16 MB db_write_buffer_size, the presence CF's impact is ~512 KB. Not worth the per-CF options complexity. Just leave it with uniform settings.

### P5: Do NOT drop utxo_store or diagnostic_ledger -- conf(0.70, observed)

**Evidence**: utxo_store is the production UTXO backend (`UtxoSet::RocksDb`), opened unconditionally at startup via `init_utxo_set()`. Dropping it requires either: (a) in-memory store (RAM scales with UTXO count), or (b) routing all UTXO reads through state_db (major code change). Neither is in scope.

diagnostic_ledger opens ONLY with `--fork-diagnostics` flag. It's already capped at 8 MB. Its existence costs nothing when the flag is off. When on, it's an 8 MB overhead -- negligible.

**Kill test**: "utxo_store is just a redundant copy of state_db.cf_utxo and could be eliminated."
**Kill test result**: FOUND BUT OUT OF SCOPE. Yes, it IS a redundant copy. But it's the runtime UTXO backend. Eliminating it requires architectural changes (making state_db.cf_utxo the direct UTXO provider). This is a valid long-term simplification but NOT an RocksDB configuration change -- it's an architecture change.

**Radical simplifier verdict**: Keep both. Cap them properly. The configuration fix is the scope.

## What an SSF (Stupid Simple First) Configuration Looks Like

For each of the 3 uncapped DBs, add exactly these lines:

**block_store** (+3 lines):
```rust
opts.set_db_write_buffer_size(16 * 1024 * 1024); // 16 MB total memtable budget
opts.set_write_buffer_size(4 * 1024 * 1024);     // 4 MB per-CF memtable
opts.set_max_total_wal_size(32 * 1024 * 1024);   // 32 MB WAL cap
```

**state_db** (+2 lines, WAL already set):
```rust
opts.set_db_write_buffer_size(16 * 1024 * 1024); // 16 MB total memtable budget
opts.set_write_buffer_size(4 * 1024 * 1024);     // 4 MB per-CF memtable
```

**utxo_store** (+3 lines):
```rust
opts.set_db_write_buffer_size(16 * 1024 * 1024); // 16 MB total memtable budget
opts.set_write_buffer_size(4 * 1024 * 1024);     // 4 MB per-CF memtable
opts.set_max_total_wal_size(16 * 1024 * 1024);   // 16 MB WAL cap (self-heals anyway)
```

**diagnostic_ledger** (+1 line, other caps already set):
```rust
opts.set_max_total_wal_size(8 * 1024 * 1024);    // 8 MB WAL cap
```

That's 9 new lines of code total. No per-CF options. No bloom filter changes. No compaction tuning. No block cache changes. No compression changes. No new abstractions. No shared cache. No CF removal.

## Per-Instance Concrete Values

### block_store

| Parameter | Value | Justification |
|-----------|-------|---------------|
| `db_write_buffer_size` | 16 MB | Caps total memtable across 9 CFs. 14 KB/min write rate; 16 MB = ~19 hours of writes. |
| `write_buffer_size` | 4 MB | Per-CF memtable. Fills in ~285 min at steady state, ~17s at burst. Flush < 50ms. |
| `max_write_buffer_number` | 2 (default, keep) | Overlap absorbs flush latency during burst sync. |
| `max_total_wal_size` | 32 MB | 38 hours of WAL at steady state. Prevents WAL pinning by dead `presence` CF. |
| Compression | Lz4 (keep) | Already set. No change needed. |
| Bloom filter | 10 bits (keep) | Already set. No change needed. |
| Block cache | 8 MB default (keep) | Already default. Sufficient for cold lookups. |
| `max_open_files` | 256 (keep) | Already set. No change needed. |
| Per-CF options | None | Uniform is justified: all CFs written at same rate (1 block per 10s). |

### state_db

| Parameter | Value | Justification |
|-----------|-------|---------------|
| `db_write_buffer_size` | 16 MB | Caps total memtable across 6 CFs. 48 KB/min write rate. |
| `write_buffer_size` | 4 MB | Per-CF memtable. cf_utxo fills in ~83 min steady, ~5s burst. |
| `max_write_buffer_number` | 2 (default, keep) | Overlap for burst sync. |
| `max_total_wal_size` | 64 MB (keep) | Already set. Adequate. |
| Compression | Lz4 (keep) | No change. |
| Bloom filter | None (keep for now) | Working set fits in 8 MB block cache today. Add when UTXO count > 50K. |
| Block cache | 8 MB default (keep) | Holds entire UTXO dataset at current scale. |
| `max_open_files` | 256 (keep) | No change. |
| Per-CF options | None | cf_undo has larger values but same write frequency. Uniform is simpler. |

### utxo_store

| Parameter | Value | Justification |
|-----------|-------|---------------|
| `db_write_buffer_size` | 16 MB | Caps total memtable across 3 CFs. 36 KB/min write rate. |
| `write_buffer_size` | 4 MB | Per-CF memtable. Mirrors state_db write pattern. |
| `max_write_buffer_number` | 2 (default, keep) | Overlap for burst and import_from batches. |
| `max_total_wal_size` | 16 MB | Self-heals from state_db. WAL is nice-to-have, not critical. Small cap bounds it. Could also be disabled entirely (see P2). |
| Compression | Lz4 (keep) | No change. |
| Bloom filter | None | RocksDB is a durable backend; hot reads go through UtxoSet enum dispatch. |
| Block cache | 8 MB default (keep) | No change. |
| Per-CF options | None | All 3 CFs mirror the same write pattern. |

### diagnostic_ledger

| Parameter | Value | Justification |
|-----------|-------|---------------|
| `db_write_buffer_size` | 8 MB (keep) | Already set by INC-I-102. Workload-justified: single CF, batched writes. |
| `write_buffer_size` | 4 MB (keep) | Already set. |
| `max_write_buffer_number` | 2 (keep) | Already set. |
| `max_total_wal_size` | 8 MB (add) | Currently not set. Should be bounded. 8 MB is generous for observability writes. |
| Compression | Lz4 (keep) | No change. |
| Block cache | 4 MB (keep) | Already set. |
| `max_open_files` | 64 (keep) | Already set. |

## Complexity Comparison

| Dimension | Current | Radical Minimum | Reduction |
|-----------|---------|-----------------|-----------|
| Total memtable budget per node (theoretical worst case) | ~2,184 MB | 56 MB (16+16+16+8) | **97.4% reduction** |
| Total memtable budget per node (observed) | ~450 MB | 56 MB | **87.6% reduction** |
| Total CFs across 4 DBs | 19 | 19 (no CF changes) | 0% |
| Number of DB instances | 4 | 4 (no instance changes) | 0% |
| Per-CF custom options | 0 | 0 (uniform is sufficient) | 0% |
| Block caches | 4 separate (~28 MB total) | 4 separate (~28 MB total, unchanged) | 0% |
| Bloom filters | 1 (block_store only) | 1 (block_store only, unchanged) | 0% |
| Lines of code added | 0 | 9 | +9 lines |
| WAL caps set | 1 (state_db only) | 4 (all instances) | +3 caps |
| Total RocksDB memory ceiling (memtable + cache + WAL) | ~2,240 MB+ | ~172 MB (56 memtable + 28 cache + 88 WAL) | **92.3% reduction** |

## Why This Works (Acceptance Criteria Check)

### Must

| ID | Criterion | How Radical Proposal Satisfies |
|----|-----------|-------------------------------|
| AC-MUST-001 | Behavior preservation | Only `Options` parameters change. No code logic, no CF structure, no write paths, no read paths modified. RocksDB Options are transparent to the application -- same data in, same data out. State root computation is unaffected. |
| AC-MUST-002 | Bounded per-DB memory | `db_write_buffer_size` set on all 4 instances: 16+16+16+8 = 56 MB total. Every instance has an explicit nonzero cap. |
| AC-MUST-003 | Per-CF differentiation where workload differs | **Radical position**: per-CF memtable differentiation is NOT workload-justified. ALL CFs in each DB are written at the same frequency (once per block). The existing bloom filter on block_store IS per-CF differentiation (point-lookup vs scan). If the synthesizer requires additional differentiation, adding bloom filter to state_db.cf_utxo (P3) is the highest-value option. |
| AC-MUST-004 | WAL bounded on all instances | `max_total_wal_size` set on all 4 instances: 32+64+16+8 = 120 MB total. No WAL pinning possible. |
| AC-MUST-005 | Diagnostic_ledger cap preserved | 8 MB cap unchanged. WAL cap added (8 MB). |
| AC-MUST-006 | One spec, one set of values | Hardcoded constants. No env vars, no CLI flags, no runtime configuration. |

### Should

| ID | Criterion | Status |
|----|-----------|--------|
| AC-SHOULD-001 | Read-path latency preserved | Unchanged. Block cache, bloom filter, compression all unchanged. Smaller memtables mean slightly more frequent compaction but with 4 MB SST files, compaction is faster. Net: neutral or slight improvement. |
| AC-SHOULD-002 | WAL replay < 30s | With max WAL sizes of 32/64/16/8 MB, replay of ~120 MB total takes <5 seconds on SSD, <30 seconds on HDD. |
| AC-SHOULD-003 | Bloom filters on point-lookup CFs | Partially met (block_store already has bloom filter). state_db.cf_utxo deferred -- working set fits in cache today. |
| AC-SHOULD-004 | Deprecated presence CF minimal | Under 4 MB uniform write_buffer_size, presence CF's arena overhead is ~512 KB. Negligible. No special handling needed. |

### Could

| ID | Criterion | Status |
|----|-----------|--------|
| AC-COULD-001 | Shared block cache | NOT IMPLEMENTED. Adds code complexity (shared Cache object) for ~12 MB savings. Not worth it at this scale. |
| AC-COULD-002 | WAL disabled on rebuildable instances | PARTIALLY ADDRESSED. utxo_store WAL capped at 16 MB (could be disabled). Diagnostic_ledger WAL capped at 8 MB (could be disabled). Conservative choice: cap rather than disable, since disabling changes crash-recovery behavior. |
| AC-COULD-003 | Compaction style differentiation | NOT IMPLEMENTED. Default level compaction is fine for all workloads. Write amplification is not a concern at <50 KB/min write rate. |

## Why This Might Fail

1. **Snap sync import_from writes**: The utxo_store import_from() batches of 50K entries (~5 MB) exceed the 4 MB write_buffer_size. This causes mid-batch memtable switch + flush. RocksDB handles this correctly (WriteBatch can span memtable boundaries), but there may be a transient memory spike of ~8 MB (old memtable flushing + new memtable receiving). With db_write_buffer_size = 16 MB, this is within bounds. **Mitigation**: import_from already batches at 50K entries; this caps per-batch write size.

2. **Large cf_undo entries**: cf_undo entries can be 100+ KB. A single large undo entry in a 4 MB memtable is fine (4 MB >> 100 KB). But if blocks become much larger (thousands of transactions), undo entries could grow to MB scale. At 4 MB write_buffer_size, a single 2 MB undo entry would fill half the memtable. **Mitigation**: DOLI's max block size is 2 MB; undo data is at most ~2x block size (inputs + outputs). A 4 MB memtable handles this.

3. **Chain scale growth**: At 200K blocks today, the UTXO set is small (~10K entries). At 10M blocks, the UTXO set could be millions of entries. The 8 MB block cache on state_db would no longer hold the working set. Point lookups would hit disk. Bloom filters would become essential. The configuration proposed here is correct for today's scale; it would need bloom filter additions at ~50K+ UTXOs. **This is acceptable**: tune when measured, not speculatively.

4. **Compaction pressure**: With 4 MB write_buffer_size, SST files are ~4 MB. With default level compaction and 64 MB max_bytes_for_level_base (RocksDB default), L0 holds ~16 files (4 * 4 MB), L1 holds ~64 MB, L2 holds ~640 MB. This is fine for DOLI's total dataset size at current chain height.

5. **Uniform write_buffer_size may be suboptimal for cf_undo**: cf_undo has large values (1-100 KB) and is write-once-read-rarely. A larger memtable (e.g., 8 MB) would reduce flush frequency. But at 1 entry per block (~10 KB average), cf_undo fills 4 MB in ~400 blocks = ~67 minutes. Flush frequency of once per hour for a cold-read CF is perfectly acceptable.

## Open Questions for Synthesizer

1. **AC-MUST-003 interpretation**: The acceptance criterion says "per-CF differentiation where workload differs." The radical position is that ALL CFs are written at the same frequency (once per block) and memtable sizing should be uniform. The difference is in VALUE SIZE (cf_undo: large, cf_exit_history: tiny) and READ PATTERN (cf_utxo: point lookup, cf_undo: rarely read). Value size affects SST file size and compaction, not memtable sizing. Read pattern affects bloom filter and block cache, not memtable sizing. If the synthesizer disagrees and requires memtable differentiation, the highest-value split is: hot CFs (cf_utxo, headers, bodies) at 4 MB vs cold CFs (cf_exit_history, presence, meta) at 1 MB. But the total savings is ~6 MB -- not worth the complexity.

2. **utxo_store WAL: cap vs disable**: The radical proposal caps at 16 MB. Disabling entirely saves ~16 MB WAL + flush I/O. The self-heal mechanism is proven (INC-I-027). The downside of disabling is: every node restart after crash requires full utxo_store rebuild from state_db, adding ~10-30 seconds to startup. Is this acceptable?

3. **Bloom filter on state_db.cf_utxo**: Deferred in the radical proposal because the working set fits in 8 MB block cache. Other evaluators may disagree. The cost is ~5 lines of per-CF options code + ~12.5 KB of bloom filter memory. The benefit is future-proofing. If the synthesizer wants it, it's low-risk.

4. **target_file_size_base**: RocksDB default is 64 MB. With 4 MB write_buffer_size, each flush produces a ~4 MB SST file, which is well below the 64 MB target for L1. This means L0->L1 compaction combines multiple SST files. For DOLI's tiny write rate, this doesn't matter -- compaction runs infrequently regardless.

5. **max_background_jobs**: RocksDB default is 2. With 4 DB instances, that's 8 background threads for flush/compaction. On a 2-core VPS, this could cause CPU contention. Reducing to 1 per DB (4 total) would halve the thread count. But with DOLI's tiny write rate, background jobs rarely run -- contention is theoretical. Leave defaults unless measured.

## Constraints Identified

1. **INVARIANT: db_write_buffer_size must be > 0 on ALL instances.** This is the root cause fix. Without it, memtable memory is unbounded.
2. **INVARIANT: max_total_wal_size must be > 0 on ALL instances.** Dead CFs can pin WAL files indefinitely without a cap.
3. **OBSERVATION: RocksDB handles WriteBatch larger than write_buffer_size correctly.** The batch spans memtable boundaries, triggering async flush of the full memtable while a new memtable receives the remainder. This means write_buffer_size does NOT need to be >= max WriteBatch size. However, smaller write_buffer_size means more frequent memtable switches during large batches (snap sync import), which increases flush I/O.
4. **INVARIANT: diagnostic_ledger options are ONLY relevant when --fork-diagnostics flag is passed.** The DB doesn't open otherwise. No need to optimize for the common case (flag off).
5. **CONSTRAINT: presence CF must remain in the CF list.** Removing it breaks DB open for existing nodes with the CF on disk. The CF is already cleaned on open and never written -- its cost is negligible under a db_write_buffer_size cap.
6. **CONSTRAINT: state_db WAL must remain enabled.** It provides crash recovery for consensus-critical data. Disabling would require snap sync on every crash -- much slower recovery.
7. **CONSTRAINT: No code logic changes.** This is a configuration-only fix. All proposals change only RocksDB Options parameters, never the write/read paths themselves.

## Cross-Perspective Signals

1. **For the Coupling evaluator**: `utxo_store` is a full redundant copy of `state_db.cf_utxo` + `state_db.cf_utxo_by_pubkey`. Every write is duplicated. This is the highest-leverage architectural simplification opportunity -- eliminating utxo_store entirely and routing all UTXO operations through state_db would remove 1 DB instance, 3 CFs, and all dual-write synchronization concerns. But it's an architecture change, not a configuration change.

2. **For the Pattern evaluator**: The 4 DB open functions (block_store, state_db, utxo_rocks, diagnostic_ledger) share no common configuration code. Each constructs its own `Options` independently. A shared `fn default_doli_opts() -> Options` function could ensure all instances get the mandatory caps (db_write_buffer_size, write_buffer_size, max_total_wal_size) without repeating the values. With only 9 lines of new code, this is marginal -- but it would prevent future regressions where a new DB instance is opened without caps.

3. **For the Dead Code evaluator**: The `presence` CF cleanup migration in block_store runs on every startup, iterating to check if it's empty. After the first successful cleanup, subsequent starts iterate an empty CF (immediate return). This is harmless but could be gated by a marker in `meta` CF ("presence_cleaned=true").

4. **For the Failure Mode evaluator**: `state_db.open()` counts all UTXO entries on startup via full iterator scan (line 41-47 in open.rs). Similarly, `utxo_rocks.open()` does the same (line 47-54). For large UTXO sets (millions), these startup scans could take minutes. This is NOT a RocksDB configuration issue but is worth noting as a potential startup latency concern at scale.

## Gaps

1. **No measurement of actual SST file sizes** on mainnet nodes. The write rate math is derived from code analysis, not observed RocksDB statistics. Actual compaction behavior may differ.
2. **No measurement of block cache hit rates** on any instance. The assertion that "working set fits in 8 MB block cache" is inferred from UTXO count * entry size, not measured.
3. **No testing of 4 MB write_buffer_size under actual sync burst**. The math says it works, but it hasn't been benchmarked.
4. **snap sync `atomic_replace()`** interaction with the proposed memtable caps is unknown. If it bypasses the memtable (direct SST ingestion), the caps are irrelevant during snap sync. If it goes through the memtable, the 16 MB cap may cause churn.
5. **RocksDB arena allocation behavior for empty CFs** is assumed based on documentation, not measured. The actual initial memory per CF on open may differ from the ~1/8 of write_buffer_size estimate.
