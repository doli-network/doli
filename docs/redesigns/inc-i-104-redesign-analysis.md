# INC-I-104 Redesign Analysis — RocksDB Configuration (4 instances)

## 1. Incident Scope (from intake + handoff)

Three of four RocksDB instances in doli-node run with uncapped memtable budgets: `db_write_buffer_size=0`, per-CF `write_buffer_size=64 MB`, `max_write_buffer_number=2`. Only the `diagnostic_ledger` was capped at 8 MB (commit f37febcf, INC-I-102). The theoretical per-node memory ceiling is: sum across all active CFs of (`write_buffer_size` × `max_write_buffer_number`). On family servers with 6 nodes × ~450 MB = 2.7 GB exceeding 1.9 GB total RAM, OOM kills fire during memtable warm-up before steady state. On ai5 (4 nodes) it plateaus at ~1.8 GB on 3.7 GB (fits). The layered network split (transport=45, sync=1) is a symptom of memory pressure, not root cause.

**Source of truth for RocksDB parameter values**: direct RocksDB LOG file dump on ai5/n9 (cited in intake).

## 2. Architecture Comprehension

### 2.1 The 4 instances at a glance

| Instance | Path on disk | Column families | Primary role | Opened at startup |
|----------|-------------|----------------|-------------|-------------------|
| `block_store` | `<data_dir>/blocks/` | 9 (8 active + 1 deprecated) | Block headers, bodies, canonical indexes, tx/address indexes | Always |
| `state_db` | `<data_dir>/state_db/` | 6 | Authoritative UTXO set, producer set, chain state, undo log, epoch state | Always |
| `utxo_store` | `<data_dir>/utxo_store/` | 3 | Secondary UTXO index with unique-ID lookups (NFT, Asset, Pool, Channel) | Always (node-heal excludes it; self-heals on startup) |
| `diagnostic_ledger` | `<data_dir>/diagnostics/` | 1 | Fork-diagnostic event log | Only when `--fork-diagnostics` is passed |

### 2.2 Dependency map: what depends on each DB

**block_store**:
- **Consumed by**: `apply_block()` (read headers/bodies during reorg), `set_canonical_chain()` (backward walk), sync manager (GetHeaders/GetBlocks responses), RPC (getBlock, getTransaction, getAddressHistory, getBlockByHeight, getBlockBySlot), block archiver (catch-up reads), `ensure_blocks_present()` (FORK_GUARD), rollback (find parent), snap sync (seed_canonical_index).
- **Written by**: `put_block()` on every received/produced block, `set_canonical_chain()` after every apply.

**state_db**:
- **Consumed by**: `apply_block()` (UTXO lookups, producer lookups), validation (UTXO existence, double-spend checks), mempool (UTXO existence for tx acceptance), state root computation, snap sync (serialize_canonical, atomic_replace), RPC (getBalance, getProducerInfo, getUtxo, getStorageInfo), epoch boundary logic (producer set, pending updates), rollback (undo data read).
- **Written by**: `BlockBatch::commit()` on every applied block (one atomic WriteBatch), `atomic_replace()` on snap sync, `delete_epoch_state()` on format version mismatch.

**utxo_store**:
- **Consumed by**: In-memory UtxoSet for fast reads (mempool validation, RPC balance queries, state root computation, unique-ID existence checks for NFT/Asset minting).
- **Written by**: In parallel with state_db batch writes during `apply_block()`, snap sync deserialization. Self-heals from state_db on startup if corrupted.

**diagnostic_ledger**:
- **Consumed by**: `getForkDiagnostic` RPC, pruner task, writer heartbeat.
- **Written by**: Async writer task (batches of 10 events or 100ms, whichever first).
- **Dependency direction**: Pure observability; consensus never reads from it. Graceful degradation to NoOpEmitter if DB fails to open.

### 2.3 Current uncapped behavior (verbatim parameter values from RocksDB LOG dump)

| Parameter | block_store | state_db | utxo_store | diagnostic_ledger |
|-----------|-------------|----------|------------|-------------------|
| `db_write_buffer_size` | 0 (uncapped) | 0 (uncapped) | 0 (uncapped) | 8,388,608 (8 MB) |
| Per-CF `write_buffer_size` | 67,108,864 (64 MB) | 67,108,864 (64 MB) | 67,108,864 (64 MB) | (governed by db_write_buffer_size) |
| `max_write_buffer_number` | 2 | 2 | 2 | 2 |
| `max_total_wal_size` | 0 (uncapped) | 67,108,864 (64 MB) | 0 (uncapped) | (not documented) |
| Block cache | Default (~8 MB shared) | Default (~8 MB shared) | Default (~8 MB shared) | (not documented) |
| Compaction style | Default (level) | Default (level) | Default (level) | Default (level) |
| Bloom filter | Not set | Not set | Not set | Not set |

**Theoretical memtable ceiling per instance** (worst case, all CFs active with 2 memtables each):
- `block_store`: 9 CFs × 64 MB × 2 = 1,152 MB (but `presence` is deprecated/empty, so effectively 8 × 64 × 2 = 1,024 MB)
- `state_db`: 6 CFs × 64 MB × 2 = 768 MB
- `utxo_store`: 3 CFs × 64 MB × 2 = 384 MB
- `diagnostic_ledger`: capped at 8 MB total
- **Total theoretical max per node**: ~2,184 MB memtables + block caches + WAL + overhead

**Observed plateau**: ~450 MB per node on mainnet (not all CFs are write-hot simultaneously, so RocksDB only allocates memtables when data is written to a CF).

## 3. Workload Inventory (per DB, per CF)

### 3.1 block_store (9 CFs)

| CF | Key shape | Value shape | Write hotness | Read hotness | Durability tier |
|----|-----------|-------------|---------------|-------------|----------------|
| `headers` | Hash (32B) | bincode BlockHeader (~200-400B) | **Hot** — every received/produced block | **Hot** — validation, reorg backward-walk, sync GetHeaders | Consensus-critical (cannot lose committed blocks) |
| `bodies` | Hash (32B) | bincode BlockBody (variable, 500B-2MB) | **Hot** — every received/produced block | **Warm** — sync GetBlocks, RPC getBlock, reorg re-apply | Consensus-critical |
| `height_index` | u64 LE (8B) | Hash (32B) | **Hot** — updated by `set_canonical_chain()` on every apply | **Hot** — getBlockByHeight, backward-walk, sync | Rebuildable (from headers via `rebuild_canonical_index()`) |
| `slot_index` | u32 LE (4B) | Hash (32B) | **Hot** — every block | **Warm** — getBlockBySlot, has_block_for_slot | Rebuildable |
| `hash_to_height` | Hash (32B) | u64 LE (8B) | **Hot** — updated by `set_canonical_chain()` | **Hot** — O(1) height lookup, sync, fork detection | Rebuildable |
| `tx_index` | tx_hash (32B) | u64 LE (8B) | **Hot** — one entry per tx per block | **Cold** — RPC getTxBlockHeight only | Rebuildable |
| `addr_tx_index` | pubkey_hash(32B) ++ height(8B BE) = 40B | empty (0B) | **Hot** — one entry per unique address per block | **Cold** — RPC getAddressHistory only | Rebuildable |
| `presence` | (deprecated) | (deprecated) | **Cold** — never written (cleaned on open) | **Cold** — never read | Dead (keep for backward compat only) |
| `meta` | string key (`"snap_horizon"`) | u64 LE (8B) | **Cold** — written once on snap sync | **Cold** — read at startup, canonical-walk boundary check | Rebuildable |

**Write pattern**: Append-heavy. Every block produces writes to 7 CFs (all except `presence` and `meta`). Write rate = 1 block per 10-second slot = 6 blocks/min max. Body size dominates value size. Under sync catch-up, writes can burst (blocks applied as fast as they're downloaded).

**Read pattern**: `headers`, `height_index`, `hash_to_height` are read on the hot validation/sync path. `bodies` on sync and RPC. `tx_index` and `addr_tx_index` are RPC-only (cold).

**Crash recovery**: RocksDB WAL replay. All data is also available from the network (re-sync) or archive files (backfill). Block data is append-only — once written, blocks are immutable.

### 3.2 state_db (6 CFs)

| CF | Key shape | Value shape | Write hotness | Read hotness | Durability tier |
|----|-----------|-------------|---------------|-------------|----------------|
| `cf_utxo` | Outpoint (36B) = txhash(32) + index(4 LE) | bincode UtxoEntry (~70-200B) | **Hot** — every tx input (delete) + every tx output (insert) per block | **Hot** — validation double-spend check, mempool acceptance, balance queries | **Consensus-critical** (authoritative UTXO set) |
| `cf_utxo_by_pubkey` | pubkey_hash(32B) ++ outpoint(36B) = 68B | 0x00 (1B) | **Hot** — mirrors cf_utxo writes (secondary index) | **Warm** — RPC getBalance, getUtxosByPubkey (prefix scan) | Rebuildable from cf_utxo |
| `cf_producers` | pubkey_hash (32B) | bincode ProducerInfo (~300-600B) | **Warm** — epoch boundary only (deferred mutations); dirty-only writes O(changed) | **Warm** — epoch boundary (load all), scheduler, RPC getProducerInfo | Consensus-critical |
| `cf_exit_history` | pubkey_hash (32B) | u64 LE (8B) | **Cold** — only on producer exit events | **Cold** — anti-Sybil check on re-registration | Consensus-critical (anti-Sybil) |
| `cf_meta` | string key (variable) | varies (see meta keys) | **Hot** — chain_state + last_applied + epoch accumulators updated every block; epoch_state on epoch boundary | **Hot** — startup (chain_state, epoch_state), every block (last_applied canary check) | Consensus-critical (chain_state, epoch_state); Rebuildable (accumulators, bond snapshot) |
| `cf_undo` | u64 LE (8B) | bincode UndoData (variable, 1-100+ KB) | **Hot** — one entry per applied block (stores full rollback data) | **Cold** — only on rollback (rare path) | Rebuildable (can regenerate by re-applying blocks from block_store, but expensive) |

**Write pattern**: Dominated by `cf_utxo` + `cf_utxo_by_pubkey` churn. A block with N transactions creates O(inputs+outputs) deletes and inserts across both CFs. `cf_undo` writes one entry per block containing all spent UTXOs (can be large for blocks with many transactions). All writes are in a single atomic WriteBatch per block.

**Read pattern**: `cf_utxo` is the hottest CF — read on every transaction validation (mempool + block). `cf_meta` is read at startup and for the consistency canary on every block. `cf_undo` is only read during rollback (rare). `cf_producers` is read at epoch boundaries and by RPC.

**Crash recovery**: Single WriteBatch atomicity guarantees consistency. WAL replay recovers the last uncommitted batch. The `last_applied` canary lets the node detect partial writes on startup. State can also be rebuilt via snap sync from a peer.

### 3.3 utxo_store (3 CFs)

| CF | Key shape | Value shape | Write hotness | Read hotness | Durability tier |
|----|-----------|-------------|---------------|-------------|----------------|
| `utxo` | Outpoint (36B) | bincode UtxoEntry (~70-200B) | **Hot** — mirrors state_db cf_utxo writes | **Warm** — fast in-memory UtxoSet reads; RocksDB backend as fallback | Rebuildable (from state_db) |
| `utxo_by_pubkey` | pubkey_hash(32B) ++ outpoint(36B) = 68B | empty | **Hot** — mirrors state_db cf_utxo_by_pubkey | **Warm** — balance queries | Rebuildable |
| `unique_id` | prefix(1B) + id(32B) = 33B | empty | **Warm** — only on NFT/Asset/Pool/Channel creation | **Warm** — uniqueness check on minting (prevents duplicate NFT IDs) | Rebuildable (from UTXO scan) |

**Write pattern**: Mirrors state_db UTXO writes. Exists as a separate, self-healing store — if corrupted, rebuilt from state_db at startup. The `unique_id` CF adds DeFi/NFT-specific uniqueness tracking not present in state_db.

**Read pattern**: When UtxoSet uses the RocksDb backend, all UTXO reads (validation, mempool, RPC) go through this store. The `unique_id` index provides O(1) duplicate detection for NFT/Asset/Pool minting.

**Crash recovery**: Self-heals from state_db on startup. No WAL replay needed for correctness — data can always be regenerated.

### 3.4 diagnostic_ledger (1 CF)

| CF | Key shape | Value shape | Write hotness | Read hotness | Durability tier |
|----|-----------|-------------|---------------|-------------|----------------|
| (default CF) | ULID (26B sortable) | bincode DiagnosticEvent (~200-600B, includes hex strings) | **Warm** — event rate depends on fork activity; batched writes (10 events or 100ms) | **Cold** — RPC getForkDiagnostic only; pruner scan every 60s | Observability (lossy ok) |

**Write pattern**: Async batched writes from the writer task. Rate is variable — quiet during normal operation, can spike during fork events (12+ emit sites, some firing per-gossip-block). Channel bounded at 1024 events with drop-oldest policy.

**Read pattern**: RPC query (rare, diagnostic). Pruner scans every 60s for age (30d) and count (100k) limits.

**Crash recovery**: Tolerates loss of last ~100ms of events. No consensus impact whatsoever. Graceful degradation to NoOpEmitter if DB fails.

**Current cap (INC-I-102)**: `db_write_buffer_size=8 MB`. This was applied as a targeted fix for this specific instance; the redesign should evaluate whether this value is workload-justified or was a band-aid.

## 4. Durability Tiering

| Tier | Definition | Instances / CFs |
|------|-----------|----------------|
| **Consensus-critical** | Data loss or corruption causes state root divergence, fork, or invalid block acceptance. Cannot lose any committed write. | `state_db`: cf_utxo, cf_producers, cf_exit_history, cf_meta (chain_state, epoch_state). `block_store`: headers, bodies. |
| **Rebuildable** | Can be recomputed from consensus-critical data or re-fetched from the network. Loss is recoverable (possibly expensive). | `block_store`: height_index, slot_index, hash_to_height, tx_index, addr_tx_index, meta. `state_db`: cf_utxo_by_pubkey, cf_undo (regenerable by re-applying blocks). `utxo_store`: all 3 CFs (self-heals from state_db). |
| **Observability** | Lossy is acceptable. No consensus impact. | `diagnostic_ledger`: all data. |

**Implication for the redesign**: Consensus-critical CFs need WAL + fsync guarantees; rebuildable CFs can tolerate relaxed durability (e.g., `disableWAL` for secondary indexes); observability CFs can run with minimal durability.

## 5. Latency Budget per Read Path

| Path | CFs on hot path | Latency constraint | Justification |
|------|----------------|-------------------|---------------|
| **Block validation** (`validate_block_*`) | state_db:cf_utxo (point lookup per input) | < 1ms per UTXO lookup | On the critical path of block acceptance; 10s slot budget shared with VDF, networking, apply |
| **apply_block** | state_db (all CFs via WriteBatch), block_store (put_block + set_canonical_chain) | WriteBatch commit < 10ms | Must complete within slot time; batch write is single I/O operation |
| **Mempool tx acceptance** | state_db:cf_utxo (existence check per input) | < 5ms per tx | Gossip throughput; not slot-critical |
| **Sync GetHeaders response** | block_store:headers (sequential scan up to 500 headers) | < 100ms per batch | Network request; bounded by max_headers_per_response |
| **Sync GetBlocks response** | block_store:headers + bodies (up to 100 blocks) | < 500ms per batch | Network request; large I/O but sequential |
| **RPC getBalance** | state_db:cf_utxo_by_pubkey (prefix scan) | < 50ms | User-facing but not consensus |
| **RPC getBlock** | block_store:headers + bodies (single lookup) | < 10ms | User-facing |
| **State root computation** | In-memory UtxoSet (not RocksDB), state_db:cf_producers (via ProducerSet in-memory) | < 50ms | Computed after every apply_block; uses in-memory representations |
| **Diagnostic RPC** | diagnostic_ledger (range scan) | < 500ms | Debug-only, no latency SLA |

## 6. Crash-Recovery Profile per DB

| DB | WAL status | Recovery mechanism | Recovery time | Notes |
|----|-----------|-------------------|---------------|-------|
| `block_store` | WAL enabled, `max_total_wal_size=0` (uncapped) | WAL replay on restart. Alternatively: re-sync from network or backfill from archive. | Fast (WAL replay) to slow (re-sync) | Uncapped WAL can grow large if compaction stalls; `presence` CF (never written) can pin WAL segments |
| `state_db` | WAL enabled, `max_total_wal_size=64 MB` | WAL replay on restart. `last_applied` canary detects inconsistency. Alternatively: snap sync from peer. | Fast (WAL replay, canary check) | Only instance with a WAL cap. The 64 MB cap means WAL files are rotated, bounding recovery time. |
| `utxo_store` | WAL enabled, `max_total_wal_size=0` (uncapped) | Self-heals from state_db on startup. WAL replay as first attempt. | Fast (self-heal from state_db UTXO iteration) | WAL replay is unnecessary given self-heal capability. WAL could be disabled entirely. |
| `diagnostic_ledger` | WAL enabled (per spec) | WAL replay. Loss of recent events acceptable. Graceful degradation to NoOpEmitter if DB fails. | Instant (NoOp fallback) | Durability is not required. Could disable WAL. |

**Key finding**: `block_store` and `utxo_store` have uncapped WAL (`max_total_wal_size=0`). Combined with write-cold CFs (e.g., `presence` in block_store which is emptied at startup but never written again), this creates WAL pinning: a single inactive CF's memtable prevents the WAL from rotating, causing unbounded WAL growth.

## 7. Redesign Acceptance Criteria (Must/Should/Could/Won't)

### Must (non-negotiable)

| ID | Criterion | Acceptance test |
|----|-----------|----------------|
| AC-MUST-001 | **Behavior preservation**: No consensus break, no state root change, no change to block validation outcome for any valid or invalid block. | All existing integration tests pass; state root computation produces identical results before and after. |
| AC-MUST-002 | **Bounded per-DB memory**: Every RocksDB instance has an explicit `db_write_buffer_size` > 0, derived from its workload profile (not hardware). | The parameter is set in code; RocksDB LOG dump on any node confirms non-zero value. |
| AC-MUST-003 | **Per-CF differentiation where workload differs**: CFs with different write hotness or value sizes have differentiated `write_buffer_size` and/or `max_write_buffer_number`. At minimum: hot CFs (cf_utxo, headers, bodies) vs cold CFs (presence, cf_exit_history, meta) must not share identical memtable budgets. | Code review shows per-CF options set; RocksDB LOG confirms differentiated values. |
| AC-MUST-004 | **WAL bounded on all instances**: Every instance has `max_total_wal_size` > 0. No WAL pinning from dead/cold CFs. | RocksDB LOG dump confirms non-zero `max_total_wal_size` on block_store, utxo_store (state_db already has 64 MB). |
| AC-MUST-005 | **Diagnostic_ledger cap preserved or revised with justification**: The INC-I-102 cap (8 MB) is either preserved as-is or revised with explicit workload derivation. Not silently removed. | Cap exists in code; documented justification if changed. |
| AC-MUST-006 | **One spec, one set of values**: Exactly one configuration per instance/CF. No runtime-selectable profiles, no environment variable overrides for memory budgets, no hardware-detection logic. | Code review shows hardcoded or const values. No `std::env` or CLI flags for RocksDB memory params. |

### Should (important, but feature works degraded without)

| ID | Criterion | Acceptance test |
|----|-----------|----------------|
| AC-SHOULD-001 | **Read-path latency preserved or improved on hot CFs**: Point-lookup latency on `cf_utxo` (state_db) and `headers`/`hash_to_height` (block_store) does not regress. | Benchmark or RPC response time comparison before/after on equivalent workload. |
| AC-SHOULD-002 | **WAL replay time bounded**: Crash recovery (WAL replay) completes within 30 seconds for each instance. | Manual test: kill -9 node, measure restart time to "chain_state loaded" log. |
| AC-SHOULD-003 | **Bloom filters on point-lookup CFs**: CFs that are primarily accessed via point lookups (`cf_utxo`, `headers`, `hash_to_height`, `utxo`, `cf_producers`) have `bloom_filter_bits_per_key` set to avoid unnecessary disk reads on negative lookups. | RocksDB LOG confirms bloom filter configured on specified CFs. |
| AC-SHOULD-004 | **Deprecated CF (`presence`) does not consume memtable budget**: Either dropped entirely or configured with minimal memtable (e.g., 1 MB / 1 buffer) to prevent WAL pinning. | Code review shows `presence` CF has reduced or zero memtable allocation. |

### Could (nice-to-have)

| ID | Criterion | Acceptance test |
|----|-----------|----------------|
| AC-COULD-001 | **Shared block cache across instances**: If architecturally justified, a single LRU block cache shared by all 4 instances replaces per-instance defaults, giving better memory utilization. | Code uses `Cache::new_lru_cache()` shared across DB opens. |
| AC-COULD-002 | **WAL disabled on rebuildable-only instances**: `utxo_store` (self-heals from state_db) and `diagnostic_ledger` (lossy ok) could disable WAL entirely, saving memory and I/O. | `Options::set_enable_write_ahead_log(false)` or manual WAL disable per instance. |
| AC-COULD-003 | **Compaction style differentiation**: Append-heavy CFs (block_store headers/bodies) could benefit from universal compaction (lower write amplification); point-lookup-heavy CFs (cf_utxo) stay with level compaction (better read amplification). | Per-CF compaction style set in code; documented rationale. |

### Won't (explicitly excluded)

| ID | Criterion | Rationale |
|----|-----------|-----------|
| AC-WONT-001 | **Hardware-driven sizing**: No auto-detection of available RAM, no percentage-of-memory allocation. | Architecture is not reverse-engineered from hardware. Workload-derived spec; fleet adapts. |
| AC-WONT-002 | **Runtime-configurable memory budgets**: No CLI flags or env vars for `write_buffer_size` etc. | One spec, one set of values. Operational tuning creates divergent fleet configurations. |
| AC-WONT-003 | **Global allocator change (jemalloc/mimalloc)**: Orthogonal to RocksDB configuration. | Separate investigation if needed; VmHWM > VmRSS gap on mainnet confirms glibc IS returning pages. |
| AC-WONT-004 | **Diagnostic_ledger architectural changes**: No changes to the async channel, writer task, pruner, or emit site guards. | Those are INC-I-090 / fork-observability scope, not RocksDB configuration scope. |

## 8. Open Questions for Design Evaluators

**Q1 — Shared vs. per-instance block cache**: Should there be a single shared LRU block cache across all 4 RocksDB instances, or per-instance caches? Tradeoffs: shared cache gives global LRU priority (hot data from any DB competes fairly); per-instance prevents one DB's workload from evicting another's hot blocks. Given that `state_db` and `block_store` have very different access patterns (point lookup vs sequential scan), a shared cache may cause mutual eviction.

**Q2 — state_db compaction style**: Should `state_db` keep level compaction (lower read amplification for point lookups on cf_utxo) or move to universal compaction (lower write amplification)? Given that cf_utxo has high write churn (deletes + inserts on every block) AND is the hottest read path (validation), this is a genuine tradeoff. Level compaction's sorted runs are better for point lookups but more expensive on writes.

**Q3 — block_store block cache justification**: Does `block_store` benefit from a block cache at all? Its writes are append-only (new blocks), and its reads fall into two patterns: (a) sequential backward-walk during `set_canonical_chain()` and sync responses, (b) random lookup by hash for validation/RPC. Pattern (a) would thrash a block cache; pattern (b) benefits from it. What's the dominant access pattern in steady state vs. sync catch-up?

**Q4 — diagnostic_ledger 8 MB cap adequacy**: Under the new workload-derived model, is 8 MB the right total memtable budget for a single-CF observability store with batched writes (10 events or 100ms)? The INC-I-102 fix applied 8 MB as a targeted cap. The evaluators should derive this from write rate and batch size, not inherit it as a given.

**Q5 — Bloom filter bits_per_key**: What's the optimal `bloom_filter_bits_per_key` for: (a) `cf_utxo` (millions of entries, high point-lookup rate, false positives cause unnecessary disk reads), (b) `headers`/`hash_to_height` (tens of thousands of entries, moderate lookup rate), (c) `unique_id` (small cardinality, but existence checks are on the minting validation path)? Standard RocksDB guidance suggests 10 bits/key for ~1% FPR — is that sufficient for DOLI's UTXO set size?

**Q6 — presence CF lifecycle**: The `presence` CF is deprecated and cleaned on open. It still exists as a column family descriptor in the block_store open call. Should the redesign: (a) keep it with minimal memtable to avoid WAL pinning, (b) drop it entirely (risk: old DBs that haven't been cleaned yet fail to open), (c) migrate it out on open and remove the CF? This affects the effective CF count for memtable budget calculation.

**Q7 — utxo_store WAL necessity**: Given that `utxo_store` self-heals from `state_db` on startup (architecture doc confirms this, node-heal excludes it), is WAL needed at all? Disabling WAL on `utxo_store` would save memory (no WAL buffer) and I/O (no fsync), with the only cost being a full rebuild from state_db on crash (which happens anyway during self-heal).

**Q8 — cf_undo sizing**: The `cf_undo` CF in state_db stores full rollback data (spent UTXOs, full ProducerSet snapshot, optional EpochState snapshot) per block. Individual entries can be 1-100+ KB. Pruning keeps the last 100 blocks (UNDO_KEEP_DEPTH). This CF has very different write/value characteristics from `cf_utxo` — should it get its own memtable size, and if so, what drives the derivation?

## 9. Out-of-Scope Explicit List

1. **Hardware sizing / fleet operational tuning** — The spec is workload-derived. How operators fit nodes to specific hardware (swap, cgroups, memory overcommit) is downstream.
2. **Sync pipeline buffer caps** (`pending_headers`, `pending_blocks`) — These are sync-manager redesign scope (INC-I-103), not RocksDB configuration.
3. **Orphan pool caps** — Block handling scope, not storage.
4. **Diagnostic emit site guards** (the 12 unguarded sites) — Fork-observability / INC-I-090 scope.
5. **Global allocator change** (jemalloc/mimalloc) — Orthogonal; the VmHWM > VmRSS evidence shows glibc page return is working.
6. **RocksDB version upgrade** — Not in scope; configuration changes only.
7. **Compression algorithm changes** — Could be future work but not part of this redesign.
8. **Auto-update interaction** — The family server's `--no-auto-update` flag bypassing the May 30 fix is an operational issue, not an architectural one.

---

**Analyst-flagged uncertainties (intellectual honesty):**

1. Whether `utxo_store` (RocksDbUtxoStore) is opened unconditionally at startup or only as a migration artifact. The architecture doc says old `utxo_rocks/` files are migrated into StateDb, but node-heal excludes `utxo_store/` and the RocksDB LOG dump shows it as a live instance. Verify from `Node::new()` initialization code.

2. The exact block cache size for each instance. The handoff says "typically 8 MB shared" but neither the analyst nor the user has cited code lines confirming whether any instance overrides this default. The 8 MB is RocksDB's default `LRUCache` size.

3. Whether any CF has custom `Options` (compaction style, bloom filter, compression) set in the `open.rs` files. The analysis assumes all CFs use RocksDB defaults because no documentation or evidence contradicts this, but evaluators should verify from source.

4. The exact write amplification characteristics of the current configuration — whether compaction is keeping up with write rate or creating I/O pressure that contributes to memory growth (stalled flushes can cause additional memtable allocation beyond `max_write_buffer_number`).
