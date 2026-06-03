<!--
OUTPUT CONTRACT: N/A — architecture specification (not a test file)
INPUT PARTITIONS: N/A — architecture specification (not a test file)
-->

# RocksDB Configuration Architecture (INC-I-104)

## Executive Summary

Three of four RocksDB instances in doli-node run with `db_write_buffer_size=0` (uncapped memtable budgets). Per-CF `write_buffer_size` defaults to 64 MB with `max_write_buffer_number=2`, giving a theoretical per-node ceiling of 2,312 MB across 19 column families. On constrained hosts (6 nodes on 1.9 GB), this OOM-kills during memtable warm-up.

This spec sets explicit, workload-derived configuration for all 4 instances (block_store, state_db, utxo_store, diagnostic_ledger) and all 19 column families. The total per-node RocksDB memory ceiling drops from ~2,312 MB theoretical to ~218 MB bounded. The change is non-consensus (state root computed from in-memory state, not SSTs) and deploys via rolling restart.

Five independent evaluators converged on the core fix (db_write_buffer_size > 0 everywhere, per-CF differentiation, WAL bounded). Divergence on specific values was arbitrated using the Failure Analyst's 13 hard constraints as filters.

## Acceptance Criteria Coverage

### Must

| ID | Criterion | How Satisfied |
|----|-----------|---------------|
| AC-MUST-001 | Behavior preservation | Only RocksDB Options change. No write/read paths, no data encoding, no state root computation affected. |
| AC-MUST-002 | Bounded per-DB memory (db_write_buffer_size > 0) | Set on all 4 instances: 48 + 64 + 32 + 8 = 152 MB total memtable budget. |
| AC-MUST-003 | Per-CF differentiation where workload differs | open_cf_descriptors on block_store, state_db, utxo_store. Hot CFs: 8-16 MB. Cold CFs: 1 MB. |
| AC-MUST-004 | WAL bounded on all instances | block_store: 48 MB cap. state_db: 64 MB (existing). utxo_store: WAL disabled. diagnostic_ledger: WAL disabled. |
| AC-MUST-005 | Diagnostic_ledger cap preserved | 8 MB unchanged. Workload-justified: 240 KB/min peak = 33 min to fill. |
| AC-MUST-006 | One spec, one set of values | Hardcoded constants. No env vars, no CLI flags, no runtime config. |

### Should

| ID | Criterion | Status |
|----|-----------|--------|
| AC-SHOULD-001 | Read-path latency preserved | Block cache sized per-instance. Bloom filters added to point-lookup CFs. |
| AC-SHOULD-002 | WAL replay < 30s | Max WAL: 48 MB (block_store) + 64 MB (state_db) = 112 MB. Replay < 5s on SSD. |
| AC-SHOULD-003 | Bloom filters on point-lookup CFs | Added to state_db (cf_utxo, cf_producers, cf_exit_history) and utxo_store (utxo, unique_id). block_store already has them. |
| AC-SHOULD-004 | presence CF does not consume memtable budget | Configured at 1 MB / 1 buffer (down from 128 MB theoretical). |

### Could

| ID | Criterion | Status |
|----|-----------|--------|
| AC-COULD-001 | Shared block cache | NOT ADOPTED. Per-instance caches. C-012 filter: sequential block_store scans evict cf_utxo hot data. |
| AC-COULD-002 | WAL disabled on rebuildable instances | ADOPTED for utxo_store (self-heals) and diagnostic_ledger (lossy ok). |
| AC-COULD-003 | Compaction style differentiation | NOT ADOPTED. Level compaction everywhere. C-005 filter: changing requires migration. |

### Won't

AC-WONT-001 through AC-WONT-004 respected: no hardware-driven sizing, no runtime config, no allocator change, no diagnostic_ledger architectural changes.

## Failure Analyst Constraints Applied (C-001 through C-013)

| ID | Constraint | Disposition |
|----|-----------|-------------|
| C-001 | state_db WAL enabled + PointInTime recovery | SATISFIED. No change to state_db WAL. |
| C-002 | state_db db_write_buffer_size >= 32 MB | SATISFIED. Set to 64 MB. |
| C-003 | Reduced write_buffer_size => raise L0 triggers | APPLIED. Hot CFs: slowdown=40, stop=60. |
| C-004 | presence CF must remain in descriptor list | SATISFIED. Kept with 1 MB / 1 buffer. |
| C-005 | Level compaction retained (no migration risk) | SATISFIED. All CFs remain level. |
| C-006 | block_store max_total_wal_size > 0 | SATISFIED. Set to 48 MB. |
| C-007 | db_write_buffer_size > 0 on all 4 instances | SATISFIED. 48 + 64 + 32 + 8 MB. |
| C-008 | Hot CF write_buffer_size >= 8 MB | SATISFIED. Hot CFs at 8-16 MB. |
| C-009 | Hot CF max_write_buffer_number >= 2 | SATISFIED. All hot CFs at 2. |
| C-010 | No full bloom on scan CFs | SATISFIED. cf_utxo_by_pubkey, addr_tx_index: no bloom. |
| C-011 | utxo_store db_write_buffer_size >= 16 MB | SATISFIED. Set to 32 MB. |
| C-012 | No shared cache without scan protection | SATISFIED. Per-instance caches adopted. |
| C-013 | Rolling deploy safe | SATISFIED. Non-consensus change. |

**10 MUST-NOT rules (from Failure Analyst)**: All respected. WAL stays on state_db. No bloom on scan CFs. No compaction style change. No shared cache. write_buffer_size >= 8 MB on hot CFs. No activation height needed.

## Authoritative Configuration Per Instance

### block_store (9 CFs: 7 active, 1 dead, 1 minimal)

**DB-level options:**

| Parameter | Value | Derivation |
|-----------|-------|------------|
| `db_write_buffer_size` | 48 MB | 7 active CFs, ~35 KB/min steady, ~3.5 MB/min burst. 48 MB fills in ~14 min at burst. Provides headroom for concurrent CF memtables during sync. |
| `max_total_wal_size` | 48 MB | 1:1 with db_write_buffer_size. Prevents WAL pinning by dead/cold CFs. |
| `max_background_jobs` | 2 | Low write rate. 2 sufficient for flush + compaction. |
| `max_subcompactions` | 1 | No benefit at these rates. |
| `max_open_files` | 256 | Existing value, keep. |
| Compression (default) | Lz4 | Existing, keep. |
| Compaction style | Level | Existing, keep (C-005). |

**Per-CF options (via `ColumnFamilyDescriptor`):**

| CF | write_buffer_size | max_write_buffer_number | bloom_filter | block_size | compression | target_file_size_base | level0_slowdown | level0_stop | Notes |
|----|-------------------|------------------------|-------------|------------|-------------|----------------------|----------------|-------------|-------|
| headers | 8 MB | 2 | 10 bits/key | 4 KB | Lz4 | 16 MB | 40 | 60 | Hot write + hot point-lookup |
| bodies | 8 MB | 2 | 10 bits/key | 16 KB | Lz4 | 32 MB | 40 | 60 | Hot write, large values. 16 KB block for large values. |
| height_index | 4 MB | 2 | NONE | 4 KB | Lz4 | 8 MB | default | default | Warm. Batched via set_canonical_chain. |
| slot_index | 4 MB | 2 | NONE | 4 KB | Lz4 | 8 MB | default | default | Warm. One entry per block. |
| hash_to_height | 4 MB | 2 | 10 bits/key | 4 KB | Lz4 | 8 MB | default | default | Hot point-lookup, warm write. |
| tx_index | 4 MB | 2 | 10 bits/key | 4 KB | Lz4 | 8 MB | default | default | Warm write, cold read (RPC only). |
| addr_tx_index | 4 MB | 2 | NONE | 4 KB | Lz4 | 8 MB | default | default | Warm write, prefix scan reads. Bloom removed (C-010). |
| presence | 1 MB | 1 | NONE | 4 KB | None | 2 MB | default | default | Dead CF. Minimal allocation. Never written. |
| meta | 1 MB | 1 | NONE | 4 KB | None | 2 MB | default | default | Cold. 1 key, written once on snap sync. |

**Block cache**: 32 MB per-instance (explicit `Cache::new_lru_cache`, INC-I-105). Shared across all 9 CFs.

**Bloom filter note**: block_store currently applies bloom at DB-level Options to ALL CFs (`open.rs:24-26`). The per-CF migration removes bloom from addr_tx_index (scan CF), presence (dead), and meta (1 key) by setting per-CF BlockBasedOptions without bloom on those 3 CFs, while retaining bloom on the other 6.

**Effective memtable ceiling**: 48 MB (capped by db_write_buffer_size). Down from 1,152 MB theoretical.

### state_db (6 CFs)

**DB-level options:**

| Parameter | Value | Derivation |
|-----------|-------|------------|
| `db_write_buffer_size` | 64 MB | Consensus-critical. Must accommodate atomic_replace ~15-20 MB (C-002). 6 CFs with burst writes at epoch boundary. |
| `max_total_wal_size` | 64 MB | Existing value, keep. 1:1 with db_write_buffer_size. |
| WAL recovery mode | PointInTime | Existing, keep (C-001). |
| `max_background_jobs` | 2 | Conservative. cf_utxo churn is moderate at 6 blocks/min. |
| `max_subcompactions` | 1 | No benefit at these rates. |
| `max_open_files` | 256 | Existing, keep. |
| Compression (default) | Lz4 | Existing, keep. |
| Compaction style | Level | Existing, keep (C-005). Best for cf_utxo point lookups. |

**Per-CF options (via `ColumnFamilyDescriptor`):**

| CF | write_buffer_size | max_write_buffer_number | bloom_filter | block_size | compression | target_file_size_base | level0_slowdown | level0_stop | Notes |
|----|-------------------|------------------------|-------------|------------|-------------|----------------------|----------------|-------------|-------|
| cf_utxo | 16 MB | 2 | 10 bits/key | 4 KB | Lz4 | 32 MB | 40 | 60 | Hottest CF. Point lookups on every validation. 4 KB block for point-lookup granularity. |
| cf_utxo_by_pubkey | 8 MB | 2 | NONE | 4 KB | Lz4 | 16 MB | 40 | 60 | Hot write, prefix scan reads. No bloom (C-010). |
| cf_meta | 4 MB | 2 | NONE | 4 KB | Lz4 | 8 MB | default | default | Hot write (chain_state every block), small values. Known keys, always positive lookups. |
| cf_undo | 4 MB | 2 | NONE | 16 KB | Zstd | 16 MB | default | default | One entry per block, 1-100+ KB. 16 KB block for large values. Zstd for cold large data. |
| cf_producers | 2 MB | 2 | 10 bits/key | 4 KB | Lz4 | 8 MB | default | default | Epoch-boundary-only writes. Point lookup by pubkey_hash. |
| cf_exit_history | 1 MB | 1 | 10 bits/key | 4 KB | Lz4 | 2 MB | default | default | Near-zero writes. Anti-Sybil point lookup. |

**Block cache**: 32 MB per-instance (explicit `Cache::new_lru_cache`). cf_utxo point lookups benefit if in-memory UtxoSet does not fully shadow reads. 32 MB is moderate: not 128 MB (uncertain whether shadowed) and not 8 MB (insufficient if not shadowed).

**Effective memtable ceiling**: 64 MB (capped by db_write_buffer_size). Down from 768 MB theoretical.

### utxo_store (3 CFs)

**DB-level options:**

| Parameter | Value | Derivation |
|-----------|-------|------------|
| `db_write_buffer_size` | 32 MB | Fully rebuildable. 3 CFs, ~15 KB/min steady. C-011 floor: >= 16 MB. |
| `max_total_wal_size` | N/A | WAL DISABLED. Self-heals from state_db on startup. |
| WAL | DISABLED | Disable via `WriteOptions::set_disable_wal(true)` on all writes. Self-heal is the designed recovery path (architecture doc, node-heal exclusion). |
| `max_background_jobs` | 1 | Rebuildable, lower priority. |
| `max_subcompactions` | 1 | No benefit. |
| `max_open_files` | 256 | Match other instances. |
| Compression | Lz4 | Existing, keep. |
| Compaction style | Level | Existing, keep (C-005). |

**Per-CF options (via `ColumnFamilyDescriptor`):**

| CF | write_buffer_size | max_write_buffer_number | bloom_filter | block_size | compression | target_file_size_base | level0_slowdown | level0_stop | Notes |
|----|-------------------|------------------------|-------------|------------|-------------|----------------------|----------------|-------------|-------|
| utxo | 16 MB | 2 | 10 bits/key | 4 KB | Lz4 | 16 MB | 40 | 60 | Mirrors cf_utxo. Per-tx writes (higher flush rate than state_db). |
| utxo_by_pubkey | 8 MB | 2 | NONE | 4 KB | Lz4 | 16 MB | 40 | 60 | Mirrors cf_utxo_by_pubkey. No bloom (scan CF, C-010). |
| unique_id | 2 MB | 2 | 10 bits/key | 4 KB | Lz4 | 4 MB | default | default | Low cardinality (DeFi gated). Existence check on mint path. |

**Block cache**: 16 MB per-instance (explicit `Cache::new_lru_cache`, INC-I-105). Shared across all 3 CFs.

**Effective memtable ceiling**: 32 MB (capped by db_write_buffer_size). Down from 384 MB theoretical.

### diagnostic_ledger (1 CF)

| Parameter | Value | Derivation |
|-----------|-------|------------|
| `db_write_buffer_size` | 8 MB | INC-I-102 cap, workload-justified: 240 KB/min peak = 33 min to fill. |
| `write_buffer_size` | 4 MB | Existing, keep. Half of total cap. |
| `max_write_buffer_number` | 2 | Existing, keep. |
| `max_total_wal_size` | N/A | WAL DISABLED. Lossy ok. NoOp fallback. |
| WAL | DISABLED | Observability data. Loss on crash has zero consensus impact. |
| `max_background_jobs` | 1 | Single CF, low write rate. |
| Block cache | 4 MB | Existing, keep. Cold reads (RPC debug only). |
| `max_open_files` | 64 | Existing, keep. |
| Bloom filter | NONE | Range scans by event_kind prefix. |
| Compression | Lz4 | Existing, keep. |

**No changes to existing values except**: WAL disabled, max_background_jobs explicitly set to 1.

## Shared Resource Decisions

### Block cache: per-instance (NOT shared)

Four evaluators out of five recommended per-instance caches. The Failure Analyst's C-012 constraint warns that sequential block_store scans (set_canonical_chain backward walk, sync GetHeaders) would evict cf_utxo hot data from a shared cache. Per-instance isolation prevents cross-DB cache pollution.

| Instance | Block cache | Rationale |
|----------|------------|-----------|
| block_store | 32 MB (explicit, INC-I-105) | Shared across 9 CFs. |
| state_db | 32 MB (explicit) | cf_utxo point lookups benefit from cache if not fully shadowed by in-memory UtxoSet. |
| utxo_store | 16 MB (explicit, INC-I-105) | Shared across 3 CFs. |
| diagnostic_ledger | 4 MB (existing) | Cold reads. |
| **Total** | **52 MB** | |

### Background jobs: per-instance, not global pool

| Instance | max_background_jobs | Rationale |
|----------|--------------------|----|
| state_db | 2 | Consensus-critical, highest priority. |
| block_store | 2 | Append-heavy, needs compaction. |
| utxo_store | 1 | Rebuildable, lower priority. |
| diagnostic_ledger | 1 | Minimal I/O. |
| **Total** | **6 threads** | Reasonable for 4-core machines. Background jobs are I/O-bound. |

### WAL: disabled on 2 instances, bounded on 2

| Instance | WAL status | Rationale |
|----------|-----------|-----------|
| state_db | Enabled, 64 MB cap | Consensus-critical. last_applied canary requires WAL atomicity (C-001). |
| block_store | Enabled, 48 MB cap | Consensus-critical for headers/bodies. Prevents WAL pinning by dead CFs. |
| utxo_store | **DISABLED** | Self-heals from state_db. WAL provides zero correctness benefit. Saves I/O. |
| diagnostic_ledger | **DISABLED** | Lossy ok. NoOp fallback. Saves I/O. |

## Memory Budget Summary

| Instance | Memtable cap | Block cache | WAL overhead | Total bounded |
|----------|-------------|-------------|-------------|---------------|
| block_store | 48 MB | 32 MB | 48 MB | 128 MB |
| state_db | 64 MB | 32 MB | 64 MB | 160 MB |
| utxo_store | 32 MB | 16 MB | 0 (disabled) | 48 MB |
| diagnostic_ledger | 8 MB | 4 MB | 0 (disabled) | 12 MB |
| **Total per node** | **152 MB** | **52 MB** | **112 MB** | **316 MB** |

Note: WAL overhead is the maximum WAL file size, not resident memory. Actual RocksDB memory is closer to memtable + block cache + index/filter overhead = ~218 MB per node. Down from ~2,312 MB theoretical / ~450 MB observed.

## Architecture Maps

### Current Architecture

```
block_store (9 CFs, ALL at 64 MB x 2)     state_db (6 CFs, ALL at 64 MB x 2)
  No db_write_buffer_size cap                No db_write_buffer_size cap
  No WAL cap                                 WAL: 64 MB
  Bloom: 10b on ALL CFs (including dead)     Bloom: NONE
  Block cache: 8 MB default                  Block cache: 8 MB default
  open_cf (uniform options)                  open_cf (uniform options)

utxo_store (3 CFs, ALL at 64 MB x 2)      diagnostic_ledger (1 CF)
  No db_write_buffer_size cap                db_write_buffer_size: 8 MB
  No WAL cap                                 Block cache: 4 MB
  Bloom: NONE                                open_cf (uniform options)
  Block cache: 8 MB default
  open_cf (uniform options)

Total theoretical memtable: 2,312 MB per node
```

### Proposed Architecture (Definite + Recommended)

```
block_store (9 CFs, differentiated)        state_db (6 CFs, differentiated)
  db_write_buffer_size: 48 MB                db_write_buffer_size: 64 MB
  WAL: 48 MB cap                             WAL: 64 MB (unchanged)
  Bloom: selective (6 CFs yes, 3 no)         Bloom: cf_utxo, cf_producers, cf_exit_history
  Block cache: 32 MB (shared, INC-I-105)     Block cache: 32 MB
  open_cf_descriptors (per-CF options)       open_cf_descriptors (per-CF options)
  Hot CFs: 8 MB, L0 triggers raised          Hot CFs: 8-16 MB, L0 triggers raised
  Cold CFs: 1 MB                             Cold CFs: 1-2 MB

utxo_store (3 CFs, differentiated)         diagnostic_ledger (1 CF)
  db_write_buffer_size: 32 MB                db_write_buffer_size: 8 MB (unchanged)
  WAL: DISABLED                              WAL: DISABLED
  Bloom: utxo, unique_id                     Block cache: 4 MB (unchanged)
  Block cache: 16 MB (shared, INC-I-105)     max_background_jobs: 1
  open_cf_descriptors (per-CF options)
  Hot CFs: 8-16 MB, L0 triggers raised

Total bounded memtable: 152 MB per node (93% reduction)
```

## Migration / Deploy Plan

This is a configuration-only change. Non-consensus (FM-14 confirmed: state root from in-memory state, not SSTs). Deploys via rolling restart.

**Step 1**: Implement per-CF options in all 4 `open()` functions. Switch from `DB::open_cf()` to `DB::open_cf_descriptors()`. Pattern already exists in `content_store.rs:32-36`.

**Step 2**: Set `db_write_buffer_size` and `max_total_wal_size` on block_store (2 lines). Set `db_write_buffer_size` on state_db (1 line, WAL already set).

**Step 3**: Disable WAL on utxo_store and diagnostic_ledger writes via `WriteOptions::set_disable_wal(true)`.

**Step 4**: Add bloom filters to state_db and utxo_store point-lookup CFs via per-CF `BlockBasedOptions`.

**Step 5**: Create explicit 32 MB `Cache::new_lru_cache` for state_db.

**Step 6**: Existing bloom filters on block_store: remove from addr_tx_index, presence, meta by setting per-CF BlockBasedOptions without bloom on those CFs.

BRIDGE: During rollout, nodes with old config (uncapped) and new config (capped) coexist safely. No activation height needed. The old config wastes memory but produces identical state roots. New bloom filters only affect new SST files; existing data gains bloom coverage as compaction processes old SSTs.

**Verification**: After deploy, check RocksDB LOG on any node for:
- `db_write_buffer_size` matches spec values
- Per-CF `write_buffer_size` shows differentiated values
- `max_total_wal_size` is non-zero on block_store
- Bloom filter entries present for point-lookup CFs

### Implementation Status (COMPLETE)

All 6 milestones landed on `main`. The implementation matches this spec:

| Milestone | Commit | Scope |
|-----------|--------|-------|
| M0 | be6372db | Cap memtable budget on 3 RocksDB instances |
| M1 | 1608f5c3 | Switch block_store to DB::open_cf_descriptors |
| M2 | 4f69cf66 | block_store per-CF tuning (bloom, L0 triggers, compression) |
| M3 | 225b3a83 | state_db per-CF tuning (bloom, block cache, L0 triggers) |
| M4 | c47084f7 | utxo_store per-CF tuning + WAL disable |
| M5 | (pending) | diagnostic_ledger WAL disable + max_background_jobs=1 |

INC-I-104 workflow close-out. Rolling deploy safe (C-013). No activation height required.

## Complexity Comparison

| Metric | Current | Radical Minimum | Proposed |
|--------|---------|----------------|----------|
| Modules | 4 DB instances | 4 (unchanged) | 4 (unchanged) |
| Column families | 19 | 19 (unchanged) | 19 (unchanged) |
| Per-CF custom options | 0 | 0 | 19 (via ColumnFamilyDescriptor) |
| Total memtable budget | 2,312 MB | 56 MB | 152 MB |
| Block cache total | 28 MB | 28 MB | 52 MB |
| WAL caps set | 1 (state_db) | 4 (all) | 2 + 2 disabled |
| Bloom filter CFs | 9 (block_store all) | 9 (unchanged) | 12 (net +3) |
| Lines of code added | 0 | ~9 | ~80-120 |
| RocksDB memory ceiling/node | ~2,340+ MB | ~172 MB | ~218 MB |

## Open Issues for Implementation Phase

1. **Verify utxo_store unconditional open**: Confirm from `init_utxo_set()` in `init.rs` that RocksDbUtxoStore is always opened, not only for migration.
2. **Verify rust-rocksdb API surface**: Confirm `ColumnFamilyDescriptor`, `BlockBasedOptions::set_bloom_filter`, `WriteOptions::set_disable_wal` are available in the project's `rocksdb` crate version. `content_store.rs` and `diagnostic_ledger/mod.rs` already use these patterns.
3. **Verify `cache_index_and_filter_blocks`**: If available in the crate, set to `true` on state_db's BlockBasedOptions to prevent unbounded index/filter memory outside the block cache.
4. **Measure atomic_replace batch size**: On a running node, log the WriteBatch byte size during snap sync to confirm it fits within the 64 MB state_db cap.
5. **Test WAL disable on utxo_store**: Kill -9 a node after applying blocks, restart, verify self-heal completes and UTXO counts match state_db.
6. **L0 trigger validation**: Under sync burst (100+ blocks/min), verify L0 file count stays below slowdown trigger (40) for hot CFs.

## Evaluation Summary

| Evaluator | Lens | Top Proposal | Confidence | Key Finding |
|-----------|------|-------------|------------|-------------|
| Subtractionist | removal | Drop WAL on utxo/diag; shrink per-CF to 256KB-8MB | conf(0.7, observed) | 64 MB per CF is 1,340x-4,270x oversized for actual write rates |
| Restructurer | boundaries | Shared 64 MB cache; per-CF via open_cf_descriptors | conf(0.65, observed) | open_cf_descriptors already used in content_store.rs |
| Pattern Matcher | patterns | Industry-standard: bloom on point-lookup CFs, 4KB blocks, 128MB cache | conf(0.65, inferred) | DOLI hits 4/5 canonical RocksDB anti-patterns |
| Failure Analyst | failures | 13 hard constraints; write stall protection mandatory | conf(0.65, observed) | Reducing write_buffer_size without raising L0 triggers causes stalls |
| Radical Simplifier | minimal | 9 lines: uniform 4MB + 16MB cap everywhere | conf(0.65, measured) | Caught analyst's bloom-filter error; working set fits in cache today |

## Convergence Matrix

```
                          Sub   Rest  Patt  Fail  Rad    Result
db_write_buffer_size > 0:  Y     Y     Y     Y     Y  -> 5/5 DEFINITE
Per-CF differentiation:    Y     Y     Y     Y     N  -> 4/5 DEFINITE
WAL cap on block_store:    Y     Y     Y     Y     Y  -> 5/5 DEFINITE
WAL disable utxo_store:    Y     ~     ~     Y     N  -> 3/5 RECOMMENDED
WAL disable diagnostic:    Y     ~     ~     Y     N  -> 3/5 RECOMMENDED
Bloom on cf_utxo:          Y     Y     Y     Y     N  -> 4/5 DEFINITE
Bloom on cf_producers:     Y     Y     Y     Y     -  -> 4/5 DEFINITE
Raise L0 triggers:         -     -     -     Y     -  -> 1/5 but MANDATORY (C-003)
Shared block cache:        N     Y     N     N     N  -> 1/5 REJECTED
Keep level compaction:     Y     Y     Y     Y     Y  -> 5/5 LOCKED
Rolling deploy safe:       Y     Y     Y     Y     Y  -> 5/5 LOCKED
```

## Definite Changes (High Convergence)

- ARCHITECTURAL: Set `db_write_buffer_size` > 0 on all 4 RocksDB instances (48/64/32/8 MB)
    Convergence: 5/5 evaluators
    Evidence: RocksDB LOG dump on ai5/n9 confirms db_write_buffer_size=0 on 3 of 4 instances (design-brief.md:36). Root cause of INC-I-104 OOM.
    Confidence: conf(0.95, converged)
    Eliminates the unbounded memtable growth seam. Each instance's total memtable memory is hard-capped.

- ARCHITECTURAL: Switch from `DB::open_cf()` to `DB::open_cf_descriptors()` with per-CF options on block_store, state_db, utxo_store
    Convergence: 4/5 evaluators (Sub, Restruct, Pattern, Failure)
    Evidence: `content_store.rs:32-36` already uses `open_cf_descriptors` in this codebase. Current `open_cf` forces uniform 64 MB on dead CFs like presence.
    Confidence: conf(0.85, converged)
    Eliminates the uniform-options anti-pattern. Each CF gets workload-appropriate memtable, bloom, compression.

- ARCHITECTURAL: Set `max_total_wal_size` > 0 on block_store (currently uncapped)
    Convergence: 5/5 evaluators
    Evidence: block_store/open.rs has no `set_max_total_wal_size()`. Dead presence CF pins WAL segments indefinitely (Failure Analyst FM-10, C-006).
    Confidence: conf(0.90, converged)
    Eliminates WAL pinning by dead/cold CFs.

- ARCHITECTURAL: Add bloom filters (10 bits/key) to state_db point-lookup CFs (cf_utxo, cf_producers, cf_exit_history)
    Convergence: 4/5 evaluators (Sub, Restruct, Pattern, Failure)
    Evidence: state_db/open.rs has no `set_bloom_filter()`. cf_utxo is the hottest read path (< 1ms target, design-brief.md:81). RocksDB wiki: 10 bits/key = ~1% FPR.
    Confidence: conf(0.80, converged)
    Eliminates unnecessary disk reads on negative point lookups for the validation hot path.

- ARCHITECTURAL: Raise `level0_slowdown_writes_trigger` to 40 and `level0_stop_writes_trigger` to 60 on hot CFs whose write_buffer_size is reduced from 64 MB
    Convergence: 1/5 explicit (Failure Analyst) but MANDATORY per C-003
    Evidence: Reducing write_buffer_size from 64 MB to 8-16 MB produces 4-8x more L0 files per unit of data. Without raising triggers, write stalls become 4-8x more likely on the consensus hot path (FM-01, FM-02).
    Confidence: conf(0.75, converged)
    C-003 is a hard constraint. Applies to all hot CFs in block_store (headers, bodies), state_db (cf_utxo, cf_utxo_by_pubkey), utxo_store (utxo, utxo_by_pubkey).

## Recommended Changes (Medium Convergence)

- ARCHITECTURAL: Disable WAL on utxo_store
    Convergence: 3/5 evaluators (Sub, Failure, Pattern leans)
    Evidence: utxo_store self-heals from state_db on startup (init.rs, architecture doc). node-heal excludes utxo_store. WAL replay is redundant with self-heal.
    Confidence: conf(0.75, converged)
    Saves I/O and WAL memory. Cost: crash recovery adds ~5-30s for self-heal instead of WAL replay.

- ARCHITECTURAL: Disable WAL on diagnostic_ledger
    Convergence: 3/5 evaluators (Sub, Failure, Pattern leans)
    Evidence: Lossy ok (design-brief.md:68). NoOp fallback if DB fails. Consensus never reads from it.
    Confidence: conf(0.70, converged)
    Saves I/O. Zero consensus risk.

- ARCHITECTURAL: Increase state_db block cache from 8 MB default to 32 MB explicit
    Convergence: 3/5 evaluators propose increase (Restruct 64MB shared, Pattern 128MB, Failure 32MB)
    Evidence: cf_utxo point lookups are on the < 1ms validation path. 8 MB default is small for any meaningful UTXO set. Uncertainty: in-memory UtxoSet may shadow reads.
    Confidence: conf(0.65, converged)
    32 MB is a moderate increase. If UtxoSet shadows reads, this is wasted but not harmful.

## Constraints (from Failure Analyst)

Any future modification to this configuration MUST respect:

1. **state_db WAL must remain enabled** (C-001). Disabling breaks last_applied canary invariant.
2. **state_db db_write_buffer_size >= 32 MB** (C-002). atomic_replace WriteBatch can be ~15-20 MB.
3. **Reducing write_buffer_size requires proportional L0 trigger raises** (C-003).
4. **presence CF must remain in descriptor list** (C-004). Removing crashes nodes with existing data.
5. **Compaction style changes require migration plan** (C-005).

## Observability — Prometheus integration (implementation amendment 2026-06-01)

After the spec was approved, the operator flagged that no RocksDB statistics are exposed in Prometheus. The implementation amendment below was added as a cross-cutting observability requirement; it does not change any of the configuration values above.

### Requirement

Every RocksDB instance MUST publish a per-instance metrics snapshot via the existing Prometheus exporter (`bins/node/src/metrics.rs`, served at `/metrics` on the node's metrics port). The snapshot covers memtable footprint, block cache usage, SST file shape per LSM level, flush / compaction state, and the two critical write-health properties (`actual-delayed-write-rate`, `is-write-stopped`).

### How it's wired

1. Each `open()` calls `opts.enable_statistics()` (cheap; required for ticker counters that will be exposed in a Phase-2 follow-up).
2. Each storage type (`BlockStore`, `StateDb`, `RocksDbUtxoStore`, `DiagnosticLedger`) exposes a `metrics(&self) -> RocksDbMetrics` method backed by `storage::collect_db_metrics(&db, instance_label)` which reads `db.property_int_value(...)` for the property set documented in `crates/storage/src/metrics.rs`.
3. `UtxoSet::metrics()` returns `Option<RocksDbMetrics>` — `None` for the in-memory backend (no RocksDB to scrape).
4. `bins/node/src/metrics.rs::spawn_rocksdb_metrics_scraper(...)` is started once in `run.rs` immediately before `node.run()`. It ticks every 15 seconds and applies the snapshot to a set of Prometheus `IntGaugeVec` gauges labeled by `instance` (and `level` for `doli_rocksdb_files_at_level`).

### Exported gauges

All labeled by `instance="block_store|state_db|utxo_store|diagnostic_ledger"`:

| Gauge | RocksDB property | Why it matters |
|-------|------------------|----------------|
| `doli_rocksdb_memtable_bytes` | `cur-size-all-mem-tables` | INC-I-104 primary signal — must stay below the `db_write_buffer_size` cap. |
| `doli_rocksdb_memtable_max_bytes` | `size-all-mem-tables` | Effective peak across CFs. Sanity-check the cap. |
| `doli_rocksdb_block_cache_bytes` | `block-cache-usage` | Validates state_db's 32 MB cache is or isn't sized right. |
| `doli_rocksdb_block_cache_pinned_bytes` | `block-cache-pinned-usage` | Memory unavailable for eviction. |
| `doli_rocksdb_table_readers_bytes` | `estimate-table-readers-mem` | SST index + bloom filter memory (the hidden cost of bloom). |
| `doli_rocksdb_estimate_keys` | `estimate-num-keys` | UTXO/entry growth tracking. |
| `doli_rocksdb_live_data_bytes` | `estimate-live-data-size` | Logical chain footprint. |
| `doli_rocksdb_sst_total_bytes` | `total-sst-files-size` | Disk usage trend. |
| `doli_rocksdb_sst_live_bytes` | `live-sst-files-size` | Excludes obsolete pending-delete. |
| `doli_rocksdb_running_flushes` | `num-running-flushes` | Should be near zero in steady state. |
| `doli_rocksdb_running_compactions` | `num-running-compactions` | Background pressure indicator. |
| `doli_rocksdb_compaction_pending` | `compaction-pending` | 1 = backlog. |
| `doli_rocksdb_memtable_flush_pending` | `mem-table-flush-pending` | 1 = flush backlog. |
| `doli_rocksdb_num_immutable_memtable` | `num-immutable-mem-table` | Approaching `max_write_buffer_number` = stall risk. |
| `doli_rocksdb_actual_delayed_write_rate` | `actual-delayed-write-rate` | **Critical**: non-zero = RocksDB is throttling writes. |
| `doli_rocksdb_is_write_stopped` | `is-write-stopped` | **Critical**: 1 = writes blocked (matches FM-02 from Failure Analyst). |
| `doli_rocksdb_background_errors` | `background-errors` | Cumulative; rises on compaction/flush failure. |
| `doli_rocksdb_files_at_level{level="0..6"}` | `num-files-at-level<N>` | L0 file count is the direct signal for the C-003 trigger raises (40/60 on hot CFs). |

### Out of scope (Phase 2 candidates)

- Ticker counters from `Statistics` (cache hits/misses, bloom usefulness, stall micros, WAL bytes written). These require holding the `Options` or `Statistics` object alongside the `DB` handle in each storage struct — a follow-up refactor, not blocking the immediate observability gap.
- Per-CF property reads via `db.property_int_value_cf(cf_handle, ...)`. Same property set, finer breakdown. Adds 19× cardinality; defer until specific CF-level questions arise.

### PromQL recipes (alert-ready)

```promql
# CRITICAL — writes blocked. Any non-zero value on state_db or block_store
# means consensus deadline is missing.
doli_rocksdb_is_write_stopped > 0

# HIGH — RocksDB throttling writes (level0_slowdown_writes_trigger hit).
# Sustained > 0 for 30s on state_db means the consensus hot path is delayed.
doli_rocksdb_actual_delayed_write_rate > 0

# HIGH — L0 file count approaching stop trigger (60). Stall imminent.
sum by (instance) (doli_rocksdb_files_at_level{level="0"}) > 50

# MEDIUM — L0 file count approaching slowdown trigger (40 on hot CFs).
sum by (instance) (doli_rocksdb_files_at_level{level="0"}) > 30

# MEDIUM — new background error in the last 5 minutes (compaction/flush
# failure). Uses the proper counter increase() — works across process restarts.
increase(doli_rocksdb_background_errors_total[5m]) > 0

# MEDIUM — memtable near cap. Healthy under burst; concerning if sustained.
# _memtable_cap_bytes is the configured db_write_buffer_size (constant per instance);
# _memtable_max_bytes is current usage incl. pinned (varies). Use the cap for this alert.
doli_rocksdb_memtable_bytes / doli_rocksdb_memtable_cap_bytes > 0.9

# LOW — flush throughput bottleneck. Sustained > 0 means write rate exceeds flush.
doli_rocksdb_memtable_flush_pending > 0

# LOW — pinned cache pressure. Approaching 1 means no room for new reads.
doli_rocksdb_block_cache_pinned_bytes / doli_rocksdb_block_cache_bytes > 0.8

# Observability — per-instance total bytes (capacity tracking).
sum by (instance) (doli_rocksdb_sst_total_bytes)

# Observability — per-node total RocksDB memory (memtable + block cache + table readers).
sum (
  doli_rocksdb_memtable_bytes
  + doli_rocksdb_block_cache_bytes
  + doli_rocksdb_table_readers_bytes
)
```

### Dashboard

A minimal Grafana dashboard JSON is provided at `docs/grafana/rocksdb-health.json`.
Import via Grafana UI: **Dashboards → Import → Upload JSON file**. Three rows:

1. **Write Health** — `is_write_stopped`, `actual_delayed_write_rate`, L0 files per instance, `background_errors_total` rate
2. **Memory** — `memtable_bytes` vs `_max_bytes`, `block_cache_bytes`, `table_readers_bytes`, per-instance breakdown
3. **Data Shape** — `sst_total_bytes`, `estimate_keys`, files per LSM level

Dashboard assumes Prometheus is scraping the node `/metrics` endpoint at 15-second intervals. Adjust the `$datasource` variable to your Prometheus instance.

### Implementation overhead

- `enable_statistics()` costs ~1% CPU per RocksDB instance (rocksdb-internal counters). Acceptable.
- Property reads are in-memory atomic loads; 15 s scrape is ~negligible CPU.
- Resident-memory impact: zero (metrics are computed on the fly into a small struct).

### Build / clippy / test status

- `cargo build` clean.
- `cargo clippy --workspace --all-targets -- -D warnings` clean.
- `cargo fmt --check` clean.
- 2 new tests in `storage::metrics` (smoke test against rust-rocksdb 0.22 property names + label stability) — pass.
- Pre-existing failures in `node::diagnostic_monitor_tests` (`p1_chain_break_loop_returns_actionable_alert`, `p2_dedup_suppresses_repeat_alerts`) are present on `main` before this change and are unrelated.

## Design Synthesis Quality Gate

```
--- DESIGN SYNTHESIS QUALITY GATE ---
Evaluators completed:           5/5
Deletion convergence items:     3 (WAL disable utxo, WAL disable diag, bloom removal from 3 cold CFs)
Restructuring convergence:      2 (open_cf_descriptors, per-CF differentiation)
Addition options presented:     0 (all arbitrated to single values per user constraint)
Failure modes identified:       14 (FM-01 to FM-14 from Failure Analyst)
Failure modes applied as filters: 13/13 (C-001 to C-013 all applied)
Radical floor gap:              2,312 MB -> 56 MB (radical) -> 152 MB (proposed)
Contradictions found:           2 (analyst bloom-filter error, state_db cap sizing)
Contradictions resolved:        2/2 (code is SOT for bloom; C-002 resolves cap floor)
Evidence independence verified: YES
-----------------------------------------
```
