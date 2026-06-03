# Architecture Reasoning Trace: RocksDB Configuration (INC-I-104)

## Evaluator Reports Summary

| Evaluator | Lens | Top Proposal | Confidence | Key Finding |
|-----------|------|-------------|------------|-------------|
| Subtractionist | removal | Drop WAL on utxo_store+diag; shrink per-CF memtables 256KB-8MB; set db_write_buffer_size everywhere | conf(0.7, observed) | 64 MB per CF is 1,340x-4,270x oversized; presence CF wastes 128 MB |
| Restructurer | boundaries | Shared 64 MB block cache; per-CF via open_cf_descriptors; db_write_buffer_size on all | conf(0.65, observed) | Per-CF differentiation requires open_cf_descriptors, pattern exists in content_store.rs |
| Pattern Matcher | patterns | Industry-standard per-CF tuning: bloom on point-lookup CFs, 4KB block_size for UTXO, 128MB state_db cache | conf(0.65, inferred) | DOLI hits 4 of 5 canonical RocksDB anti-patterns |
| Failure Analyst | failures | 13 hard constraints (C-001 to C-013); write stall risk if write_buffer_size reduced without L0 trigger raise | conf(0.65, observed) | state_db db_write_buffer_size >= 32 MB (atomic_replace); hot CF write_buffer_size >= 8 MB |
| Radical Simplifier | minimal | Uniform 4 MB write_buffer_size, 16 MB db_write_buffer_size on all 3 uncapped DBs; 9 lines total | conf(0.65, measured) | Bloom-filter contradiction in analyst's table; working set fits in cache today |

## Convergence Matrix (built from all 5 reports)

### db_write_buffer_size

| Instance | Subtractionist | Restructurer | Pattern Matcher | Failure Analyst | Radical |
|----------|---------------|-------------|----------------|----------------|---------|
| block_store | 16 MB | 48 MB | 64 MB | 48 MB | 16 MB |
| state_db | 32 MB | 64 MB | 64 MB | 64 MB | 16 MB |
| utxo_store | 8 MB | 32 MB | 32 MB | 32 MB | 16 MB |
| diagnostic_ledger | 8 MB | 8 MB | 8 MB | 8 MB | 8 MB |

**diagnostic_ledger**: 5/5 converge at 8 MB. LOCKED.
**state_db**: 3/5 at 64 MB (Restructurer, Pattern, Failure). Radical at 16, Subtractionist at 32. C-002 (>= 32 MB) disqualifies Radical's 16 MB. MAJORITY at 64 MB with Subtractionist's 32 MB as minimum floor.
**block_store**: SPLIT. 2 at 16 MB, 2 at 48 MB, 1 at 64 MB. No majority. Arbitrate below.
**utxo_store**: 3/5 at 32 MB (Restructurer, Pattern, Failure). MAJORITY at 32 MB.

### Per-CF write_buffer_size approach

| Question | Sub | Restruct | Pattern | Failure | Radical |
|----------|-----|----------|---------|---------|---------|
| Use open_cf_descriptors? | YES | YES | YES | YES | NO |
| Per-CF differentiation? | YES (6 tiers) | YES (3 tiers) | YES (workload classes) | YES (hot/cold) | NO (uniform) |

**4/5 require open_cf_descriptors**. Radical argues uniform is sufficient. MAJORITY wins: per-CF differentiation.

### Hot-CF write_buffer_size (cf_utxo, headers, bodies)

| CF | Sub | Restruct | Pattern | Failure | Radical |
|----|-----|----------|---------|---------|---------|
| cf_utxo | 8 MB | 16 MB | 16 MB | 16 MB | 4 MB |
| headers | 4 MB | 16 MB | 16 MB | 8 MB | 4 MB |
| bodies | 4 MB | 16 MB | 16 MB | 8 MB | 4 MB |

**C-008 (hot CF >= 8 MB) disqualifies Subtractionist's 4 MB for headers/bodies and Radical's 4 MB for all.**
**cf_utxo**: 3/5 at 16 MB. MAJORITY.
**headers**: After C-008 filter: 2 at 16 MB, 1 at 8 MB, 2 disqualified. Pattern is 8-16 MB range. Adopt 8 MB (failure analyst floor, simpler).
**bodies**: Same reasoning. Adopt 8 MB.

### Cold-CF write_buffer_size (presence, meta, cf_exit_history)

| CF | Sub | Restruct | Pattern | Failure | Radical |
|----|-----|----------|---------|---------|---------|
| presence | 256 KB | 1 MB | 1 MB | 1 MB | 4 MB (uniform) |
| meta (block_store) | 256 KB | 2 MB | 1 MB | 1 MB | 4 MB |
| cf_exit_history | 512 KB | 2 MB | 1 MB | 1 MB | 4 MB |

**presence**: 3/5 at 1 MB. MAJORITY. (Subtractionist's 256 KB is below concern but valid; 1 MB is simpler.)
**meta**: 3/5 at 1-2 MB range. Adopt 1 MB.
**cf_exit_history**: 3/5 at 1 MB. MAJORITY.

### WAL decisions

| Instance | Sub | Restruct | Pattern | Failure | Radical |
|----------|-----|----------|---------|---------|---------|
| block_store WAL cap | 32 MB | 48 MB | 64 MB | 64 MB | 32 MB |
| utxo_store WAL | DISABLE | 32 MB or disable | disable or 32 MB | DISABLE | 16 MB |
| diagnostic WAL | DISABLE | 8 MB or disable | 8 MB | disable or 4 MB | 8 MB |
| state_db WAL | Keep 64 MB | Keep 64 MB | Keep 64 MB | MUST keep (C-001) | Keep 64 MB |

**state_db WAL**: 5/5 keep. LOCKED.
**utxo_store WAL disable**: 3/5 favor disable (Sub, Failure, Pattern leans disable). MAJORITY: disable.
**diagnostic WAL**: 3/5 favor disable or cap. Since diagnostic_ledger only opens with --fork-diagnostics and is lossy, disable WAL.
**block_store WAL cap**: SPLIT. Range 32-64 MB. Adopt 48 MB (2 at 48 or higher, 2 at 32, 1 at 64; midpoint is conservative).

### Bloom filters

| CF | Sub | Restruct | Pattern | Failure | Radical |
|----|-----|----------|---------|---------|---------|
| block_store (existing) | Keep | Keep | Keep selectively | Keep | Keep |
| cf_utxo (state_db) | ADD 10b | ADD 10b | ADD 10b | ADD 10b | DEFER |
| cf_producers | ADD 10b | ADD 10b | ADD 10b | ADD 10b | - |
| cf_utxo_by_pubkey | SKIP | SKIP | SKIP | Prefix only (C-010) | - |
| utxo (utxo_store) | SKIP | ADD 10b | ADD 10b | ADD 10b | - |
| unique_id | - | ADD 10b | ADD 10b | ADD 10b | - |

**cf_utxo bloom**: 4/5 ADD. Radical defers but concedes low cost. CONVERGED: ADD.
**cf_producers bloom**: 4/5 ADD. CONVERGED: ADD.
**cf_utxo_by_pubkey**: 4/5 SKIP full bloom (C-010 prohibits). CONVERGED: SKIP.
**utxo/unique_id**: 3/5 ADD. MAJORITY: ADD.

### Block cache

| Question | Sub | Restruct | Pattern | Failure | Radical |
|----------|-----|----------|---------|---------|---------|
| Shared cache? | No (per-inst) | YES (64 MB shared) | No (per-inst) | No (C-012 warns) | No (keep defaults) |
| state_db cache | 8 MB | 64 MB (shared) | 128 MB | 32 MB | 8 MB (default) |

**Shared vs per-instance**: 4/5 say per-instance (or warn against shared). C-012 is a hard filter. CONVERGED: per-instance.
**state_db cache size**: SPLIT. Range 8-128 MB. Pattern Matcher wants 128 MB but acknowledges conf(0.6) and "if UtxoSet shadows reads, wasted." Multiple evaluators note in-memory UtxoSet may serve all reads. Adopt 32 MB (Failure Analyst's value) as moderate position.

### Compaction style

5/5 agree: keep level compaction for all CFs. C-005 prohibits changing without migration. LOCKED.

### max_background_jobs

| Instance | Sub | Restruct | Pattern | Failure | Radical |
|----------|-----|----------|---------|---------|---------|
| state_db | 2 | 2 | 4 | 4 | 2 (default) |
| block_store | 2 | 2 | 2 | 4 | 2 (default) |

**state_db**: SPLIT (2 vs 4). Failure Analyst raises this for compaction; Pattern Matcher raises for cf_utxo churn. But Failure Analyst also notes (Gap 5) that 4 may cause CPU contention on 2-core machines. Adopt 2 (conservative; "architecture NOT reverse-engineered from hardware" but 4 background threads for one DB is aggressive given 4 DBs total).
**block_store**: 3/5 at 2. MAJORITY: 2.

### L0 triggers (write stall protection)

| Parameter | Sub | Restruct | Pattern | Failure | Radical |
|-----------|-----|----------|---------|---------|---------|
| Raise L0 triggers for hot CFs? | No | No | No mention | YES (C-003: mandatory) | No |

**C-003 is a hard constraint**: reducing write_buffer_size from 64 MB to 8-16 MB means ~4-8x more L0 files. Without raising triggers, write stalls become more likely. Only Failure Analyst explicitly addresses this, but it's an engineering consequence of the majority's memtable shrink. APPLY C-003: raise level0_slowdown_writes_trigger to 40 and level0_stop_writes_trigger to 60 for hot CFs (cf_utxo, cf_utxo_by_pubkey, headers, bodies).

### max_write_buffer_number

| CF class | Sub | Restruct | Pattern | Failure | Radical |
|----------|-----|----------|---------|---------|---------|
| Hot CFs | 2 | 2 | 2 | 3 | 2 |
| Cold CFs | 1-2 | 2 | 1-2 | 1-2 | 2 |

**Hot CFs**: 4/5 at 2. Failure Analyst recommends 3 for burst absorption (C-009 says >= 2). Adopt 2 (majority; 3 would increase peak memory per CF by 50%).
**Cold CFs**: Converge at 1 for dead CFs (presence), 2 for others.

## Failure Analyst Hard Rejection Filters Applied

### C-001 (state_db WAL must stay enabled): No evaluator proposed disabling. SATISFIED.
### C-002 (state_db db_write_buffer_size >= 32 MB): Radical's 16 MB REJECTED. Subtractionist's 32 MB is floor. Majority at 64 MB adopted.
### C-003 (reduce write_buffer_size => raise L0 triggers): Applied to all hot-CF proposals. level0_slowdown=40, level0_stop=60.
### C-004 (presence CF must remain in descriptor): All evaluators agree. SATISFIED.
### C-005 (keep level compaction): All evaluators agree. SATISFIED.
### C-006 (block_store max_total_wal_size > 0): All evaluators set WAL cap. SATISFIED.
### C-007 (db_write_buffer_size > 0 everywhere): All evaluators set it. SATISFIED.
### C-008 (hot CF write_buffer_size >= 8 MB): Subtractionist 4 MB for headers/bodies REJECTED. Radical 4 MB for all REJECTED. Minimum 8 MB for hot CFs applied.
### C-009 (max_write_buffer_number >= 2 for hot CFs): All proposals satisfy. SATISFIED.
### C-010 (no full bloom on scan CFs): All evaluators skip cf_utxo_by_pubkey bloom. SATISFIED.
### C-011 (utxo_store db_write_buffer_size >= 16 MB): Subtractionist at 8 MB is below floor. Corrected to majority 32 MB.
### C-012 (shared block cache risk): Restructurer's 64 MB shared cache REJECTED. Per-instance adopted.
### C-013 (rolling deploy safe): All evaluators confirm. SATISFIED.

## SSF Gate Decision (Radical vs. Synthesized)

**Radical proposal**: 9 lines, uniform 4 MB write_buffer_size + 16 MB db_write_buffer_size, no per-CF options, no bloom changes, no cache changes. conf(0.65, measured).

**Synthesized proposal**: Per-CF options, differentiated memtables (1-16 MB), bloom filters on point-lookup CFs, WAL disabled on rebuildable instances, per-instance block cache tuning. conf(0.65-0.70, converged).

**Constraint filter**: Radical's 16 MB state_db cap FAILS C-002 (>= 32 MB). Radical's 4 MB hot-CF write_buffer_size FAILS C-008 (>= 8 MB). The radical proposal as-stated does not pass the Failure Analyst's hard constraints.

**SSF Gate Result**: Radical does NOT satisfy all constraints. Present the synthesized full proposal. However, the radical's simplicity pressure shapes the synthesis: do not add complexity beyond what convergence demands.

## Per-Parameter Arbitration (Split Decisions)

### block_store db_write_buffer_size

SPLIT: 16 MB (Sub, Radical), 48 MB (Restruct, Failure), 64 MB (Pattern).

**Arbitration**: The Failure Analyst's constraint C-003 is key -- smaller memtables produce more L0 files. With 8 hot CFs at 8 MB each (16 MB per CF slot if max_write_buffer_number=2), the theoretical sum is 134 MB. The db_write_buffer_size forces flush when the TOTAL exceeds the cap. At 48 MB, approximately 3 hot CFs can have full memtables simultaneously before forcing a flush. block_store writes to 7 CFs per block simultaneously, but memtable fill rates differ -- bodies fills fastest. 48 MB provides headroom for sync burst (3.5 MB/min total, 48 MB fills in ~14 min at burst) without being wasteful.

Adopt: **48 MB**. Evidence: 2 evaluators proposed it directly; consistent with C-003 protection.

### state_db block cache

SPLIT: 8 MB (Sub, Radical), 32 MB (Failure), 64 MB (Restruct shared), 128 MB (Pattern).

**Arbitration**: The critical question is whether the in-memory UtxoSet shadows all cf_utxo reads. Multiple evaluators flag this uncertainty. If shadowed, cache is wasted. If not, cache is the highest-impact read optimization. Given the uncertainty (Pattern Matcher conf drops to 0.45 if shadowed), adopt the moderate position: **32 MB**. This is 4x the default, provides meaningful cache for non-shadowed reads, and doesn't waste 128 MB on a potentially-shadowed workload.

### block_store block cache

Sub: 2 MB. Restruct: part of 64 MB shared. Pattern: 8-32 MB. Failure: 16 MB. Radical: 8 MB (default).

**Arbitration**: block_store reads are dominated by sequential scans (set_canonical_chain, GetHeaders). Subtractionist's kill test shows sequential access thrashes caches. Point lookups (RPC, validation) are infrequent. Adopt **8 MB** (default). Changing it adds no demonstrated value.

## Contradiction Resolved: Analyst's Bloom-Filter Claim vs. Actual Code

**Analyst (Section 2.3)**: "Bloom filter: Not set" for block_store.
**Code (`block_store/open.rs:24-26`)**: `block_opts.set_bloom_filter(10.0, false)` -- bloom IS set.
**Radical Simplifier flagged this**.
**Resolution**: Code is SOT. block_store has a 10 bits/key full bloom filter on ALL CFs (applied at DB-level Options, not per-CF). The analyst's table was wrong. The spec reflects the code.

**Consequence**: AC-SHOULD-003 (bloom on point-lookup CFs) is already partially satisfied for block_store. The new work is adding bloom to state_db and utxo_store point-lookup CFs.

## Cross-Evaluator Signals Worth Noting

1. **utxo_store architectural redundancy** (flagged by Sub, Restruct, Pattern, Failure, Radical): All 5 evaluators note that utxo_store mirrors state_db. The only unique value is the `unique_id` CF. A future code change adding unique_id to state_db would eliminate the entire instance. This is OUT OF SCOPE (configuration-only redesign) but should be noted for future architecture work.

2. **block_store put_block non-atomicity** (flagged by Restruct, Failure): 6 individual `put_cf` calls, not a WriteBatch. A crash between any two leaves block_store inconsistent. Out of scope but noted.

3. **utxo_store per-transaction writes vs state_db per-block batch** (flagged by Restruct, Failure): utxo_store does O(txns_per_block) `db.write(batch)` calls while state_db does O(1). Higher write amplification on utxo_store despite being rebuildable. Relevant to WAL-disable decision (reduces I/O) and L0 trigger sizing.

## Confidence Per Major Design Decision

| Decision | Confidence | Basis |
|----------|-----------|-------|
| db_write_buffer_size > 0 on all 4 instances | conf(0.95, converged) | 5/5 evaluators, root cause of INC-I-104 |
| Per-CF options via open_cf_descriptors | conf(0.85, converged) | 4/5 evaluators, pattern exists in codebase |
| state_db db_write_buffer_size = 64 MB | conf(0.80, converged) | 3/5 + C-002 floor |
| block_store db_write_buffer_size = 48 MB | conf(0.70, converged) | 2/5 direct + arbitrated |
| utxo_store db_write_buffer_size = 32 MB | conf(0.80, converged) | 3/5 majority |
| diagnostic_ledger stays 8 MB | conf(0.95, converged) | 5/5 |
| WAL disabled on utxo_store | conf(0.75, converged) | 3/5 + self-heal proven |
| WAL disabled on diagnostic_ledger | conf(0.70, converged) | 3/5 + lossy-ok documented |
| Bloom on cf_utxo (10 bits/key) | conf(0.80, converged) | 4/5 |
| L0 triggers raised on hot CFs | conf(0.75, converged) | C-003 mandatory + 1 evaluator explicit |
| Per-instance block cache (not shared) | conf(0.80, converged) | 4/5 + C-012 |
| Level compaction everywhere | conf(0.90, converged) | 5/5 + C-005 |
| Rolling deploy safe | conf(0.95, converged) | 5/5 + C-013 + FM-14 |

## Open Questions Deferred to Implementation

1. Whether `utxo_store` (RocksDbUtxoStore) is opened unconditionally or conditionally. All evaluators assume always-open based on RocksDB LOG evidence. Implementation phase should verify from `init_utxo_set()`.
2. Actual UTXO set cardinality on mainnet (affects bloom filter sizing and block cache effectiveness).
3. Whether the in-memory UtxoSet fully shadows state_db cf_utxo reads in production (affects block cache value).
4. Whether `rust-rocksdb 0.22` exposes `cache_index_and_filter_blocks` and `pin_l0_filter_and_index_blocks_in_cache`. These are recommended but not required.
5. snap sync `atomic_replace()` behavior under the new caps: whether it bypasses memtable or goes through it.
6. utxo_store WAL disable mechanism: DB-level `set_manual_wal_flush(true)` vs per-write `WriteOptions::set_disable_wal(true)`. Implementation choice.
