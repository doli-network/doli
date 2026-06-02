# Architecture Context — INC-I-105: Block Cache Multiplication

## Structural Question
Why does the architecture allow a per-CF construction pattern to silently 9x/3x the block cache footprint across sibling storage instances?

## Explorer — Candidate Structural Explanations

1. **Copy-paste-modify without shared scaffold** — The 3 per-CF helper functions (`cf_opts_block_store`, `cf_opts_utxo_store`, `cf_opts_state_db`) have near-identical bodies but were written as private functions in separate files with no shared trait, builder, or utility. M3 (state_db, commit 225b3a83) added the `cache: &rocksdb::Cache` parameter to `cf_opts_state_db` (state_db/open.rs:22). M2 and M4 were copy-paste-modified from the same template but WITHOUT the cache parameter. The architecture has no type-level mechanism to enforce that all siblings pass a shared cache. — conf(0.92, observed). Evidence: state_db/open.rs:22 has `cache: &rocksdb::Cache`; block_store/open.rs:19 and utxo_rocks.rs:38 have identical signatures WITHOUT it. Three functions, three files, no shared contract.

2. **Spec prescribed "8 MB default" for block_store/utxo_store but never prescribed HOW** — The spec table (rocksdb-configuration-architecture.md:100,159,217) says "Block cache: 8 MB per-instance (default)" for block_store and utxo_store, meaning "rely on RocksDB default." But the spec never mandated an explicit `Cache::new_lru_cache(8 MB)` call — only state_db got "explicit `Cache::new_lru_cache`" (line 132). The word "default" in the spec was ambiguous: it meant "we intend 8 MB" but the implementation path was "don't set it, trust RocksDB." When `DB::open_cf` (shared opts) became `DB::open_cf_descriptors` (per-CF opts), each CF instantiated its OWN default — the spec's "8 MB default" silently became "32 MB x N CFs." — conf(0.90, observed). Evidence: spec line 100 says "8 MB per-instance (default)"; spec line 132 says "32 MB per-instance (explicit `Cache::new_lru_cache`)". The parenthetical "(default)" vs "(explicit)" is the gap.

3. **Metric scraper was designed for shared-cache topology, never updated for per-CF topology** — The `first_cf_prop` pattern in metrics.rs:129-134 was correct when all CFs shared one `BlockBasedOptions` (pre-M1). After M2/M4 gave each CF its own `BlockBasedOptions::default()`, each CF has its own cache, but the scraper still reads only the first CF. The metric comment (metrics.rs:28-29) acknowledges this as a "documented limitation" but cites "default 8 MB" — a stale value from RocksDB < 8.2. The observability channel was designed for a topology that no longer exists. — conf(0.88, observed). Evidence: metrics.rs:28-29 documents the under-reporting and cites wrong default value.

4. **RocksDB version-dependent default trusted without pinning** — The code relies on `BlockBasedOptions::default()` for block cache sizing in 2/4 instances. RocksDB 8.2 changed the default from 8 MB to 32 MB. The Cargo.lock pins librocksdb-sys at 0.16.0+8.10.0 but no code or spec documents which RocksDB defaults are version-sensitive. When the default quadrupled, the memory accounting (spec line 217: "Block cache: 8 MB" for block_store) silently became wrong. — conf(0.85, observed). Evidence: metrics.rs:29 says "default 8 MB"; Cargo.lock says 8.10.0; RocksDB 8.2 changelog documents the 8->32 MB change.

5. **INC-I-104 review was memtable-scoped, not memory-scoped** — The redesign intake (`docs/.workflow/redesign-intake.md`) and the spec's problem statement focus on memtable growth. The word "block_cache" appears in the spec only in the per-instance summary tables and the shared-resource-decision section — never as a risk or constraint to audit. The INC-I-104 review framing asked "are memtables bounded?" not "is total RocksDB memory bounded?" — conf(0.85, inferred). Evidence: redesign-intake.md has 0 matches for "block_cache" or "block.cache". Spec summary table (line 217) lists block_store block cache as "8 MB" — never updated after M2 made it 9x32 MB.

## Skeptic — Eliminations Against Diagnosis Evidence

- **No eliminations.** All 5 candidates are mutually reinforcing, not competing. They form a causal chain: (1) no shared scaffold allowed divergence -> (2) spec said "default" without mandating explicit construction -> (4) the default changed across RocksDB versions -> (3) metric was designed for old topology -> (5) review framing never questioned block cache. Each candidate explains a different link in the chain. Diagnosis evidence E1a (per-CF `BlockBasedOptions::default()`) confirms #1/#2. E3 (RocksDB 8.10.0 = 32 MB default) confirms #4. E8 (first-CF-only metric + stale comment) confirms #3. E4 (pre-M1 shared opts → post-M2 per-CF) confirms the topology shift in #3.

## Analogist — Antipattern Matches

- **Inconsistent invariant enforcement across siblings**: state_db enforces shared cache via type signature (`cache: &rocksdb::Cache` parameter). block_store and utxo_store have no type-level mechanism to prevent independent caches. The invariant exists in one sibling and is absent in the others. Classic: when a safety property is enforced by convention rather than by type, it erodes on the first copy-paste.

- **Trust-by-default of library defaults that drift across versions**: The spec says "8 MB (default)" for block_store/utxo_store block cache. This pins the design to a library version's behavior without documenting which version or which default. When the library default changes (8 MB -> 32 MB in RocksDB 8.2), the actual allocation diverges from the spec silently. This is the "invisible dependency on upstream default" antipattern.

- **Observability channel co-evolved with the wrong abstraction**: The metric scraper reads `block-cache-usage` from the first CF under the assumption that all CFs share one cache. This was true before INC-I-104 but false after. The observability code was not updated when the storage topology changed. The operator's view of reality diverged from actual reality — the metric became a lie.

- **Bound was thought to be at one layer but is actually at another**: INC-I-104 bounded memtables (db_write_buffer_size) and reported success. But the memory ceiling has TWO components: memtable + block cache. Only one was bounded. The spec's summary table (line 221-223) adds them: "memtable + block cache + index/filter = ~218 MB." But the block cache column says "8 MB" for block_store/utxo_store — wrong after the per-CF change.

- **Copy-paste-modify across sibling modules without shared scaffold**: Three nearly-identical functions in three files with no shared abstraction. M3 added the `&cache` parameter to one; M2 and M4 copied the template without it. No compile-time mechanism to catch the omission. This is the "divergent sibling" antipattern — siblings that look alike but have subtly different safety properties.

## Confirmed Structural Findings

1. **Three `cf_opts_*` helpers are near-clones with a critical divergence** — `cf_opts_state_db` (state_db/open.rs:21-54) takes `cache: &rocksdb::Cache` and calls `bbo.set_block_cache(cache)` at line 46. `cf_opts_block_store` (block_store/open.rs:19-50) and `cf_opts_utxo_store` (utxo_rocks.rs:38-69) have identical structure but no cache parameter, creating independent `BlockBasedOptions::default()` at lines 42 and 61 respectively. — conf(0.95, observed).

2. **Spec prescribes "8 MB (default)" for block_store/utxo_store block cache but "32 MB (explicit)" for state_db** — The word "default" vs "explicit" in the spec (lines 100, 132, 159) is the gap. "Default" was interpreted as "don't set it" in code, which means "whatever RocksDB allocates per `BlockBasedOptions::default()`." After the `open_cf` -> `open_cf_descriptors` migration, "default" changed from "one shared" to "one per CF." — conf(0.93, observed).

3. **Metric scraper under-reports by design and documents stale default** — metrics.rs:25-29 explicitly documents "this under-reports the aggregate" for per-CF caches and cites "default 8 MB" which is wrong for RocksDB 8.10.0 (actual: 32 MB). The metric was the ONLY feedback channel for operators; its known limitation became a blind spot when the actual per-CF default quadrupled. — conf(0.95, observed).

4. **Spec summary table (line 215-221) lists stale block cache values** — block_store shows "8 MB", utxo_store shows "8 MB". Actual: block_store = 9 x 32 MB capacity = 288 MB; utxo_store = 3 x 32 MB = 96 MB. The "Total per node" row says "52 MB" block cache; actual is ~448 MB. — conf(0.95, observed).

5. **diagnostic_ledger creates explicit 4 MB cache (line 73) but uses `DB::open_cf` with string CF names (line 77)** — The explicit cache is set on DB-level `Options` via `set_block_based_table_factory`. But `DB::open_cf` with string CF names may not propagate the DB-level table factory to named CFs (the named CF gets its own default `BlockBasedOptions`). This is why E5 shows 18.4 MB usage exceeding the 4 MB cap. — conf(0.80, inferred; exact rust-rocksdb propagation semantics need unit test validation).

## Blast Radius for Any Fix

**Direct dependents of the 3 broken patterns:**
- `cf_opts_block_store` — called 9 times in `block_store/open.rs:99-139`, nowhere else.
- `cf_opts_utxo_store` — called 3 times in `utxo_rocks.rs:125-138`, nowhere else.
- `diagnostic_ledger::DiagnosticLedger::open()` — called from `node/init.rs`, nowhere else.
- `collect_db_metrics()` — called from `bins/node/src/metrics.rs` (the Prometheus exporter), once per instance per scrape tick.

**No downstream consensus impact.** Block cache is a read-side optimization; changing its size does not affect block validation, state roots, or serialization. All 4 RocksDB instances are local to each node; no cross-node protocol dependency.

**Rolling-deploy safety: SAFE.** Block cache size is a per-process, per-instance resource allocation. Nodes with different block cache configurations will read the same data from the same SST files; only read performance differs during the mixed-version window. No activation height needed. No synchronized deploy needed. conf(0.95, inferred — block cache is explicitly not consensus-visible; different nodes already run with different OS page caches without diverging).

## Architectural Invariants (proposed for memory.db `invariants` table)

- **INV-STORAGE-001**: Every RocksDB instance MUST explicitly construct ONE `rocksdb::Cache` and pass it to every CF's `BlockBasedOptions` via `set_block_cache()`. No `BlockBasedOptions::default()` may escape into a `ColumnFamilyDescriptor` without going through a shared cache. Enforcement: the `cf_opts_*` helper signature MUST include a `&rocksdb::Cache` parameter (compile-time).

- **INV-STORAGE-002**: The reported `doli_rocksdb_block_cache_bytes` metric MUST reflect total block cache usage across all CFs. When a shared cache is used (INV-STORAGE-001), reading from any one CF is sufficient. If per-CF caches ever exist, the metric MUST sum across all CFs.

- **INV-STORAGE-003**: The spec summary table (`specs/rocksdb-configuration-architecture.md`, Memory Budget section) MUST list explicit cache sizes for every instance, never "default." When a RocksDB version bump changes a default value, the spec table MUST be updated in the same commit.

- **INV-STORAGE-004**: All named-CF RocksDB instances MUST use `DB::open_cf_descriptors` (not `DB::open_cf` with string names). `open_cf_descriptors` is the only path that guarantees per-CF `BlockBasedOptions` (including explicit cache) are applied. diagnostic_ledger currently violates this.

- **INV-STORAGE-005**: The `cf_opts_*` per-CF helper functions across storage instances SHOULD be consolidated into a single shared function (or at minimum, share an identical type signature including `&rocksdb::Cache`) to prevent future divergence. Copy-paste-modify across sibling modules is the root architectural force that created this bug.

## Recommendations (NOT a fix — structural only)

- **Short-term tactical (within current arch)**: Add `cache: &rocksdb::Cache` parameter to `cf_opts_block_store` and `cf_opts_utxo_store`. Create shared caches in their respective `open()` functions (block_store: 32 MB, utxo_store: 16 MB — matching spec intent). Switch diagnostic_ledger from `DB::open_cf` to `DB::open_cf_descriptors`. Update metric comment to "32 MB (RocksDB 8.2+)".

- **Mid-term**: Consolidate the 3 `cf_opts_*` functions into a single `fn cf_opts(base, cache, ...)` in a shared module (e.g. `crates/storage/src/rocks_util.rs`). This makes the `&cache` parameter structurally mandatory — you can't call the function without one.

- **Long-term**: Add a startup log line reporting total configured block cache capacity across all DB instances (sum of all explicit `Cache::new_lru_cache` sizes). Alert if > configurable threshold. The per-CF-options model is correct — per-instance isolation (C-012) is well-justified. The problem is not the model; it's the lack of a shared scaffold enforcing the invariant.

## Why This Bug Was Not Caught

1. **INC-I-104 review framing was memtable-scoped** — The redesign intake focused on memtable growth (the original INC-I-104 symptom). Block cache was mentioned in the spec but not as a risk dimension. The review question was "are memtables bounded?" not "is total RocksDB memory bounded?"

2. **Metric blind spot** — The scraper read block cache from the first CF only (metrics.rs:129-134). For block_store with 9 independent 32 MB caches, the metric reported ~32 MB when actual capacity was ~288 MB. This was the only operator-visible feedback channel.

3. **Spec ambiguity** — The spec said "8 MB per-instance (default)" for block_store/utxo_store, meaning "we intend 8 MB total." But the code path for "default" changed from "one shared default" to "one default per CF" when `open_cf` became `open_cf_descriptors`. The spec's "default" was version- and topology-dependent.

4. **Sibling divergence tolerated** — M3 (state_db) got it right: explicit `Cache::new_lru_cache(32 MB)` threaded through all CFs. M2 and M4 (block_store, utxo_store) were copy-paste-modified from the same template without the cache parameter. No compile-time or review-time mechanism caught the omission because the helpers are private functions in separate files.

5. **Stale upstream default assumption** — The code comment and spec cited "8 MB" as the RocksDB default. RocksDB 8.2 changed it to 32 MB. The Cargo.lock pins 8.10.0. Nobody updated the assumption when the dependency was bumped.
