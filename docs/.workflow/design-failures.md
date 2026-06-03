# Evaluator #4 -- Failure Analyst Report

## TL;DR

- **Write stalls are the consensus-killing failure mode**: state_db uses RocksDB defaults for level0 triggers (slowdown=20, stop=36). During sync catch-up (burst writes), cf_utxo can accumulate L0 files faster than compaction drains them. A write stall during apply_block blocks the consensus hot path -- missed slot leads to exclusion.
- **Dual-write between state_db and utxo_store has a crash-consistency gap**: utxo_store commits per-transaction BEFORE the state_db atomic WriteBatch commits. A crash between them leaves divergent state. The self-heal mechanism (count mismatch detection + rebuild from state_db) covers this, but any configuration change that slows utxo_store writes or makes self-heal unreliable breaks the invariant.
- **WAL pinning by dead/cold CFs is active on block_store**: CF_PRESENCE (deprecated, cleaned to empty on open) and CF_META (written once during snap sync) pin WAL segments because their memtables never fill up. block_store WAL is uncapped, so this grows without bound.
- **Snap sync atomic_replace builds a WriteBatch containing the ENTIRE UTXO set**: With tens of thousands of UTXOs, this single batch can be 10+ MB. The `db_write_buffer_size` cap must be large enough to not force a memtable flush mid-batch, or the batch must be split (currently it is not).
- **Compaction style is immutable post-deploy**: Switching an existing RocksDB instance between level and universal compaction requires either a migration tool or a data wipe. Any proposal MUST use the same compaction style (level) as current production data, or include a migration plan.

## Analysis Lens

Error paths, blast radius, invariant violations, recovery behavior. Adversarial reasoning. Central question: What BREAKS in the current design, and what constraints must ANY redesign respect?

## What I Don't Understand

1. Whether `utxo.spend_transaction(tx)` on the RocksDb backend calls `self.db.write(batch)` synchronously (confirmed: YES, see utxo_rocks.rs:179) -- meaning the RocksDB write happens per-transaction within apply_block, not batched across all transactions in the block. This creates many small writes to utxo_store vs one large batch to state_db.
2. Exactly how large the state_db atomic_replace WriteBatch gets in production (with ~30k UTXOs, ~30 producers). I estimate 3-5 MB for UTXOs alone (30k entries x ~106 bytes key+value), plus indexes (~30k x 69 bytes) = ~5 MB total. This is well within any reasonable `write_buffer_size` but should be measured.
3. Whether RocksDB's `db_write_buffer_size` applies as a HARD cap (rejecting writes) or a SOFT cap (triggering flushes then proceeding). From RocksDB docs: it is a soft cap that triggers flushes. This means it cannot cause data loss -- only I/O pressure.
4. Whether there are any explicit `manual_flush` or `compact_range` calls anywhere in the codebase.

## Current State Analysis

### Configuration summary (measured from source code)

| Parameter | state_db | block_store | utxo_store | diagnostic_ledger |
|-----------|----------|-------------|------------|-------------------|
| CFs | 6 | 9 (8 active + 1 dead) | 3 | 1 |
| `db_write_buffer_size` | 0 (uncapped) | 0 (uncapped) | 0 (uncapped) | 8 MB |
| Per-CF `write_buffer_size` | 64 MB (default) | 64 MB (default) | 64 MB (default) | 4 MB |
| `max_write_buffer_number` | 2 (default) | 2 (default) | 2 (default) | 2 |
| `max_total_wal_size` | 64 MB | 0 (uncapped) | 0 (uncapped) | not set |
| WAL recovery mode | PointInTime | default (TolerateCorruptedTailRecords) | default | default |
| Bloom filter | no | yes (10 bits, all CFs) | no | no |
| Block cache | default (~8 MB) | default (~8 MB) | default (~8 MB) | 4 MB explicit |
| `max_open_files` | 256 | 256 | not set (default) | 64 |
| Compression | Lz4 | Lz4 | Lz4 | Lz4 |
| Compaction style | level (default) | level (default) | level (default) | level (default) |
| Per-CF options | NONE | NONE | NONE | NONE |
| `level0_slowdown_writes_trigger` | 20 (default) | 20 (default) | 20 (default) | 20 (default) |
| `level0_stop_writes_trigger` | 36 (default) | 36 (default) | 36 (default) | 36 (default) |
| `max_background_jobs` | 2 (default) | 2 (default) | 2 (default) | 2 (default) |

### Worst-case memtable budget (theoretical max)

All CFs use `open_cf()` (not `open_cf_descriptors()`), so ALL CFs in an instance share the same Options. No per-CF differentiation exists.

- state_db: 6 CFs x 64 MB x 2 = **768 MB** (but only cf_utxo and cf_utxo_by_pubkey are write-hot)
- block_store: 9 CFs x 64 MB x 2 = **1,152 MB** (presence is dead, meta is cold)
- utxo_store: 3 CFs x 64 MB x 2 = **384 MB**
- diagnostic_ledger: capped at **8 MB**
- **Per-node theoretical max: ~2,312 MB** (memtables only, excludes block cache, WAL, SSTs)

### Write patterns (measured from code)

- **state_db**: One WriteBatch per block containing ALL state mutations (cf_utxo + cf_utxo_by_pubkey + cf_meta + cf_producers[epoch boundary] + cf_undo). Batch size = O(tx_count * utxo_churn). Committed atomically via `batch.commit()`.
- **block_store**: Individual `put_cf()` calls per header/body/slot/tx_index/addr_index (NOT batched across CFs for put_block). `set_canonical_chain` uses WriteBatch.
- **utxo_store**: Individual WriteBatch per transaction (utxo_rocks.rs:179, utxo_rocks.rs:247). During a block with N transactions, utxo_store does N separate `db.write(batch)` calls. Much higher write amplification than state_db.
- **diagnostic_ledger**: Async batched writes (10 events or 100ms). Low volume.

## Failure Mode Catalog

### FM-01: Write stall on state_db (cf_utxo L0 accumulation during sync catch-up)

**Trigger**: During sync catch-up, apply_block is called as fast as blocks are downloaded. At 6 blocks/min steady state, cf_utxo receives ~tens of deletes+inserts per block (modest). During initial sync catch-up, this can be hundreds of blocks per minute. Each block's WriteBatch generates one flush. If compaction (max_background_jobs=2) cannot merge L0 files fast enough, L0 count reaches 20 (slowdown trigger).

**Blast radius**: `batch.commit()` at apply_block/mod.rs:347 blocks for seconds to minutes. If the stall exceeds the slot time (10s), the node misses its production slot. Repeated stalls cause liveness exclusion (removed from active production list).

**Recovery**: Stall resolves once compaction catches up. No data loss. But missed slots mean lost rewards and possible exclusion.

**Configuration constraint**: `level0_slowdown_writes_trigger` for cf_utxo MUST be high enough that sync catch-up (peak: ~100 blocks/min with 2-5 txs each) does not trigger it. With default `target_file_size_base=64 MB` and `write_buffer_size=64 MB`, each L0 file is ~64 MB. Compaction at 2 background jobs processes ~1 file every few seconds. At peak sync rate, one flush per block, ~100 flushes/min, the L0 count WILL reach 20 within minutes. The redesign MUST either: (a) increase `level0_slowdown_writes_trigger` for write-hot CFs, OR (b) reduce `write_buffer_size` so flushes produce smaller L0 files that compact faster, OR (c) increase `max_background_jobs`. Most likely: combination of (b) and (c).

### FM-02: Write stop on state_db (complete write block)

**Trigger**: Same as FM-01 but L0 count reaches 36 (stop trigger). All writes block indefinitely.

**Blast radius**: Node completely freezes. Cannot apply blocks. Falls behind chain. Eventually excluded from production.

**Recovery**: Only resolves when compaction catches up. If disk I/O is saturated (e.g., by utxo_store and block_store writing simultaneously), this can take minutes.

**Configuration constraint**: `level0_stop_writes_trigger` MUST be set high enough that it is unreachable under any production load including sync catch-up. Sizing: must accommodate burst write rate x compaction lag. Given that the default of 36 has not been observed as a production issue (observed plateau at ~450 MB, not OOM from write stalls), the immediate risk is moderate. But reducing `write_buffer_size` (per the overall redesign goal of bounding memory) increases flush frequency and makes this more likely unless level0 triggers are raised proportionally.

**CRITICAL CONSTRAINT**: Reducing `write_buffer_size` without raising `level0_slowdown_writes_trigger` makes write stalls MORE likely, not less. The redesign MUST adjust both together. conf(0.65, inferred)

### FM-03: Memtable flush failure during apply_block

**Trigger**: Disk full, I/O error, or filesystem error during memtable flush. When `max_write_buffer_number` memtables are full and flush fails, new writes stall.

**Blast radius**: Same as FM-01 (block on consensus hot path). But unlike FM-01, recovery requires resolving the underlying disk issue.

**Recovery**: Free disk space, fix filesystem. RocksDB retries flushes. No data loss for state_db (WriteBatch is in WAL). utxo_store writes are not WAL-protected for correctness (self-heal from state_db).

**Configuration constraint**: `max_write_buffer_number` must be >= 2 (need one active + one flushing). For cf_utxo, 3 would provide buffer for transient flush delays. The `db_write_buffer_size` cap must accommodate at least: `(write_buffer_size * max_write_buffer_number)` for the hottest CF plus overhead for other CFs.

### FM-04: WAL corruption on crash

**Trigger**: Process killed (SIGKILL, OOM kill) during WAL write. Partial WAL record written.

**Blast radius**: On restart, RocksDB must decide how to handle the partial record. State_db uses `PointInTime` recovery mode (discard any incomplete tail record, recover everything before). Block_store and utxo_store use the default `TolerateCorruptedTailRecords` (similar but slightly more permissive -- tolerates truncated records).

For state_db, the `last_applied` canary (in the same WriteBatch as all state changes) ensures consistency. If the WAL tail is discarded, the incomplete WriteBatch is lost. On restart, `last_applied.height < chain_state.height` (or last_applied is missing) indicates partial commit. The node can detect this and trigger appropriate recovery.

For block_store, `put_block` uses individual `put_cf` calls (NOT a WriteBatch). A crash between writing the header and body leaves an orphaned header. The `has_block` + height check in apply_block handles this gracefully (treats it as a "poisoned block" and re-applies).

For utxo_store, WAL corruption is irrelevant because it self-heals from state_db.

**Configuration constraint**: 
- state_db MUST keep `PointInTime` WAL recovery mode (or equivalent). This is the strongest mode that still discards corrupted tail records. Switching to `AbsoluteConsistency` (reject ANY corruption) would prevent startup after a crash.
- block_store and utxo_store can keep default or switch to `PointInTime` for uniformity.
- Any proposal to disable WAL must NOT disable it on state_db (consensus-critical). CAN disable on utxo_store (self-heals) and diagnostic_ledger (lossy ok).

### FM-05: state_db / utxo_store divergence (dual-write crash gap)

**Trigger**: In `apply_block`, utxo_store writes happen per-transaction (utxo_rocks.rs:179, called from tx_processing.rs:156 via `utxo.spend_transaction(tx)` and `utxo.add_transaction(tx, ...)`). State_db writes are batched and committed once at apply_block/mod.rs:347. If the process crashes AFTER utxo_store has committed some/all transaction writes BUT BEFORE state_db `batch.commit()`:
- utxo_store has UTXOs from the new block
- state_db does NOT have the block's state (last_applied still at previous height)

**Blast radius**: On restart, state_db loads chain_state from the old height. utxo_store has extra UTXOs. The self-heal check in init.rs compares `store.len() != state_db.utxo_len()`. If counts differ, utxo_store is rebuilt from state_db. This WORKS -- the architecture handles this case.

However, there is a subtlety: if the crash happens between two transactions within the same block (e.g., after transaction 3's utxo_store write but before transaction 4's), utxo_store has a PARTIAL block applied. The count comparison might not detect this if the UTXOs added and removed by the partial apply happen to produce the same net count.

**Configuration constraint**: Any configuration change to utxo_store that affects write durability (e.g., disabling WAL) does NOT break this invariant -- it makes it BETTER because undurable writes simply disappear on crash, bringing utxo_store closer to state_db's state. Disabling WAL on utxo_store is SAFE and may be beneficial.

### FM-06: Snap sync atomic_replace WriteBatch size

**Trigger**: `atomic_replace` in state_db/writes.rs:135-230 builds a single WriteBatch containing: (a) delete keys for 4 CFs (iteration over existing data), (b) all new UTXOs + secondary index entries, (c) all producer records, (d) meta keys. For a chain at height ~300k with ~30k UTXOs and ~30 producers, the batch is approximately:
- UTXOs: 30k x (36B key + ~150B value) = ~5.5 MB for cf_utxo
- Secondary index: 30k x (68B key + 1B value) = ~2 MB for cf_utxo_by_pubkey
- Producers: 30 x (32B key + ~500B value) = ~16 KB
- Deletes: similar sizes
- Total: ~15 MB for a single WriteBatch

**Blast radius**: If `write_buffer_size` for the affected CFs is set below ~15 MB, the WriteBatch will trigger a memtable flush mid-write. This is NOT a correctness issue (RocksDB handles this atomically). But it means the `db_write_buffer_size` cap must accommodate the peak batch size.

If `db_write_buffer_size` is set too LOW (e.g., 8 MB as in diagnostic_ledger), the atomic_replace will trigger forced flushes of other CFs' memtables, which is fine for correctness but causes I/O pressure.

**Configuration constraint**: `db_write_buffer_size` for state_db MUST be >= maximum atomic_replace WriteBatch size (estimated ~15-20 MB for current chain). With growth (more UTXOs, more producers), this ceiling rises. Setting state_db `db_write_buffer_size = 32 MB` provides ~2x headroom.

Per-CF `write_buffer_size` for cf_utxo should be >= 16 MB to avoid unnecessary L0 files from mid-batch flushes during atomic_replace. conf(0.6, inferred)

### FM-07: OOM during memtable warm-up (the INC-I-104 root cause)

**Trigger**: On startup, RocksDB allocates memtables for each CF. With default settings: 6 CFs x 64 MB x 2 = 768 MB for state_db alone. If the node is on a constrained host (family server: 1.9 GB for 6 nodes), the total startup allocation exceeds available RAM.

**Blast radius**: OOM kill. Node cannot start. Repeated OOM kills if systemd restarts.

**Recovery**: Reduce memory configuration or reduce node count per host.

**Configuration constraint**: The total memtable budget across ALL instances on a single node MUST fit in memory alongside: block caches, WAL buffers, in-memory UtxoSet, in-memory ProducerSet, libp2p buffers (~50 MB), and OS overhead. For the target minimum deployment (1 node), total RocksDB memory should not exceed ~200-300 MB. For the actual pain point (6 nodes on 1.9 GB), it must be < ~250 MB per node.

With per-CF differentiation: hot CFs (cf_utxo, cf_utxo_by_pubkey, headers, bodies) get larger memtables; cold CFs (cf_exit_history, presence, meta, cf_undo, cf_producers) get minimal (1-2 MB).

### FM-08: Bloom filter false-positive cost on cold CFs

**Trigger**: block_store currently sets bloom filter (10 bits/key) at the DB level via `BlockBasedOptions`. This applies to ALL 9 CFs, including CF_PRESENCE (dead, 0 entries), CF_META (1 entry), CF_SLOT_INDEX (one entry per block, range scan pattern). Bloom filters on very low-cardinality CFs waste memory without benefit. Bloom filters on scan-heavy CFs (CF_ADDR_TX_INDEX uses prefix scan) may actually hurt if the bloom filter forces random reads.

**Blast radius**: Wasted memory. Each SST file carries a bloom filter block. For a CF with 300k entries at 10 bits/key, the bloom filter is ~375 KB per SST. Across multiple SSTs, this can add up to several MB of wasted cache space.

State_db and utxo_store have NO bloom filters despite having the highest-value point-lookup workload (cf_utxo). This is a missed optimization.

**Configuration constraint**: Bloom filters SHOULD be set per-CF, not per-DB. This requires switching from `open_cf()` to `open_cf_descriptors()`. Point-lookup CFs (cf_utxo, headers, hash_to_height, cf_producers, utxo, unique_id) benefit most. Scan CFs (cf_utxo_by_pubkey, cf_addr_tx_index) should NOT have full bloom filters (prefix bloom may help).

### FM-09: Compaction style change risk

**Trigger**: Switching from level to universal compaction (or vice versa) on an existing RocksDB instance. RocksDB supports this at runtime since version 5.18 via `SetOptions`, but the SST file layout differs fundamentally. A switch causes RocksDB to treat all existing files under the new compaction strategy, which can lead to unexpected compaction storms or read amplification spikes.

**Blast radius**: Compaction storm during the transition period. I/O pressure. Potential write stalls.

**Recovery**: The transition is permanent -- no easy rollback without data wipe.

**Configuration constraint**: The redesign SHOULD keep level compaction (current default) unless there is strong evidence that a workload benefits from universal. Level compaction is well-suited for state_db (point-lookup-heavy on cf_utxo) and block_store (append-heavy but read-frequent). Universal compaction reduces write amplification but increases space amplification and read amplification -- wrong tradeoff for cf_utxo.

### FM-10: WAL replay time on restart

**Trigger**: Large uncapped WAL. block_store and utxo_store have `max_total_wal_size=0` (uncapped). If compaction stalls and WAL grows (e.g., to 500 MB), restart requires replaying the entire WAL before the node is operational.

**Blast radius**: Slow restart. Node misses slots during replay. For state_db (WAL capped at 64 MB), worst-case replay is bounded. For block_store and utxo_store, replay time is unbounded.

**Recovery**: Wait for replay. No data loss.

**Configuration constraint**: `max_total_wal_size` MUST be set > 0 on ALL instances. For block_store, cap at 64 MB (matching state_db). For utxo_store, WAL can be disabled entirely (self-heals). For diagnostic_ledger, WAL can be disabled entirely (lossy ok). Target: WAL replay < 30s on any instance.

### FM-11: Snap source UTXO consistency

**Trigger**: During snap sync, the source node serializes its UTXO set via `serialize_canonical()`. This reads from the in-memory UtxoSet (or RocksDbUtxoStore). If the source node's utxo_store diverges from state_db (FM-05), the serialized snapshot reflects the wrong state.

**Blast radius**: Receiving node gets an inconsistent snapshot. State root verification should catch this (the snapshot includes a state_root that is verified against the received data). If state_root is computed from in-memory state (which matches state_db, not utxo_store), and the snapshot data comes from utxo_store, there's a mismatch.

However, looking at the code: `compute_state_root` in snapshot.rs uses `utxo_set.serialize_canonical()`. The UtxoSet is the in-memory representation that should match state_db (authoritative). So this is safe IF the in-memory UtxoSet is consistent with state_db (which it should be, since apply_block updates both).

**Configuration constraint**: No additional constraint beyond FM-05. The state_root verification is the safety net.

### FM-12: last_applied canary invariant

**Trigger**: The `last_applied` key is written inside the same WriteBatch as all state changes (apply_block/mod.rs:316). This is the consistency canary -- if the node crashes mid-batch, either ALL writes (including last_applied) commit, or NONE do.

**Blast radius**: If any configuration change breaks WriteBatch atomicity (e.g., disabling WAL on state_db), the canary becomes unreliable. A crash could commit some CF writes but not the last_applied key, leaving the DB in an inconsistent state with no way to detect it.

**Configuration constraint**: State_db MUST have WAL enabled. `db_write_buffer_size` must not force a flush between CF writes within a single WriteBatch (RocksDB guarantees this -- a WriteBatch is atomic regardless of memtable pressure). WAL recovery mode must be `PointInTime` or `TolerateCorruptedTailRecords` (not `SkipAnyCorruptedRecords`, which could silently skip the last_applied write while committing other keys).

### FM-13: CF_PRESENCE lifecycle hazard

**Trigger**: If the `CF_PRESENCE` descriptor is removed from the CF list passed to `open_cf()`, RocksDB will fail to open existing databases that have the CF on disk (error: "Column family not found").

**Blast radius**: Node fails to start. Cannot recover without manual RocksDB repair or data wipe.

**Recovery**: Re-add CF_PRESENCE to the descriptor list, or use `DBOptions::create_missing_column_families` (already set) plus manual drop. But dropping a CF from the descriptor list is different from not creating it -- if it already exists on disk, it MUST be in the descriptor list.

**Configuration constraint**: The redesign MUST keep CF_PRESENCE in the descriptor list even with minimal options. Setting it to minimal memtable (write_buffer_size=1MB, max_write_buffer_number=1) is safe. Dropping it is NOT safe without a migration that calls `db.drop_cf("presence")` first.

### FM-14: Cross-node config divergence (consensus safety)

**Trigger**: Different nodes running different RocksDB configurations (e.g., after a partial fleet update).

**Blast radius**: NONE for consensus. State root is computed from in-memory UtxoSet and ProducerSet, not from RocksDB SST files. RocksDB is a storage engine -- its internal format (SSTs, bloom filters, block cache) does not affect the logical data visible to the application. Two nodes with identical logical state but different RocksDB configurations will compute identical state roots.

Verified: `compute_state_root` in snapshot.rs reads from `utxo_set.serialize_canonical()` (in-memory), `chain_state.serialize_canonical()` (in-memory), `producer_set.serialize_canonical()` (in-memory). No RocksDB-internal data participates in state root computation.

**Configuration constraint**: NONE. RocksDB configuration changes do NOT require activation heights, synchronized deploys, or consensus gates. They can be deployed via rolling restart. This is a key safety property that the redesign inherits for free. conf(0.7, observed)

## Proposals

### P1: Per-CF options via `open_cf_descriptors` with write-stall protection -- conf(0.65, observed)

- Evidence: All 3 uncapped instances use `open_cf()` which applies identical options to every CF. This wastes 64 MB x 2 memtable budget on dead CFs (presence) and cold CFs (cf_exit_history, meta, cf_undo). Switching to `open_cf_descriptors()` enables per-CF tuning.
- Complexity cost: +0 modules, +0 interfaces. Change is localized to 3 open functions.
- Kill test: "Would per-CF options break existing data on disk?" -- NO. RocksDB `open_cf_descriptors` is backward-compatible with `open_cf`. Existing SST files are unaffected. Per-CF options only affect new memtables and new SST files.
- Kill test result: Not found. Safe.
- Risk: If per-CF `write_buffer_size` is set too low for hot CFs, write stalls become MORE likely (FM-02). Must raise `level0_slowdown_writes_trigger` proportionally.
- Before: All CFs share 64 MB write_buffer_size, 2 max_write_buffer_number, no bloom filter (except block_store DB-wide).
- After: Hot CFs (cf_utxo, headers, bodies) get 16 MB + 3 buffers. Cold CFs (presence, cf_exit_history, meta) get 1 MB + 1 buffer. Point-lookup CFs get bloom filters. level0_slowdown_writes_trigger raised to 40 for hot CFs.

### P2: Disable WAL on utxo_store -- conf(0.6, observed)

- Evidence: utxo_store self-heals from state_db on startup (init.rs:3-36). WAL on utxo_store provides NO correctness benefit. It only saves the self-heal time on crash restart (which is fast -- it's a state_db iteration). Disabling WAL saves memory (no WAL buffer) and I/O (no WAL fsync per write).
- Complexity cost: +0 modules. One line: `opts.set_enable_write_ahead_log(false)` -- actually this sets it on WriteOptions, not Options. Would need to use `WriteOptions` with `disable_wal=true` on each write call, or configure at the DB level.
- Kill test: "Does any code path depend on utxo_store surviving a crash without self-heal?" -- The init.rs self-heal path is the ONLY recovery mechanism. No code reads utxo_store's WAL state directly. Self-heal is triggered by count mismatch.
- Kill test result: No dependency found. Safe.
- Risk: If self-heal from state_db is slow (e.g., millions of UTXOs), crash recovery time increases. With ~30k UTXOs, this is seconds.
- Before: utxo_store has WAL enabled, `max_total_wal_size=0` (uncapped).
- After: utxo_store has WAL disabled. Zero WAL memory/I/O. Crash recovery adds ~5s for self-heal.

### P3: Bounded db_write_buffer_size on all instances with atomic_replace headroom -- conf(0.65, inferred)

- Evidence: Only diagnostic_ledger has `db_write_buffer_size` set (8 MB). The other 3 instances have 0 (uncapped). This is the root cause of INC-I-104 (OOM on family server). Setting `db_write_buffer_size` bounds total memtable memory per instance.
- Complexity cost: +0 modules. 3 lines added to open functions.
- Kill test: "Could a db_write_buffer_size cap cause atomic_replace to fail?" -- NO. RocksDB's `db_write_buffer_size` is a soft cap that triggers memtable flushes, not a hard cap that rejects writes. A WriteBatch that exceeds the remaining memtable space causes a flush then proceeds. BUT: the flush + new memtable allocation can temporarily exceed the cap. With `max_write_buffer_number=3`, peak memory = `db_write_buffer_size + write_buffer_size` (old memtable being flushed + new one filling).
- Kill test result: Not found. Safe.
- Risk: If cap is too low, excessive flush frequency increases L0 file count, triggering write stalls (FM-01/FM-02). Must be sized with FM-01 constraints.
- Before: Uncapped. Per-node: up to ~2.3 GB theoretical memtable budget.
- After: state_db=64 MB, block_store=48 MB, utxo_store=32 MB, diagnostic_ledger=8 MB. Per-node total: ~152 MB memtables + block caches.

### P4: WAL cap on block_store and diagnostic_ledger -- conf(0.7, observed)

- Evidence: block_store has `max_total_wal_size=0` (uncapped). CF_PRESENCE (dead) and CF_META (cold) pin WAL segments because their memtables never fill to trigger flush+rotation. Measured: WAL pinning is architecturally guaranteed because `cleanup_presence_cf()` writes deletes (which go into WAL) but the CF is never written again, so its memtable never fills, so RocksDB never rotates the WAL file containing those deletes.
- Complexity cost: +0 modules. 1 line per instance.
- Kill test: "Could capping WAL cause data loss on block_store?" -- WAL cap forces flush of the oldest memtable. If the oldest memtable belongs to a cold CF (presence, meta), flushing it is harmless (creates a tiny or empty SST). The WAL segment is then rotatable.
- Kill test result: No risk found. The cap SOLVES WAL pinning.
- Risk: Minimal. WAL cap is a standard RocksDB best practice.
- Before: block_store WAL unbounded; can grow to hundreds of MB if compaction stalls.
- After: block_store WAL capped at 64 MB (matching state_db). diagnostic_ledger WAL capped at 8 MB or disabled entirely.

## Configuration Constraints (Filters for Synthesizer)

Every proposal from ANY evaluator MUST satisfy these constraints, or be rejected/modified.

**C-001**: `state_db` MUST have WAL enabled and `wal_recovery_mode = PointInTime`. Disabling WAL on state_db breaks the `last_applied` canary invariant (FM-12). The canary is the ONLY mechanism for detecting partial commits after crash. Any redesign proposal that disables state_db WAL MUST be rejected.

**C-002**: `state_db db_write_buffer_size` MUST be >= 32 MB. The `atomic_replace` WriteBatch can be ~15-20 MB for the current chain (FM-06). With growth headroom, 32 MB is the floor. Setting it lower risks forced mid-batch flushes during snap sync (not a correctness issue but an I/O pressure issue that could cascade with FM-01).

**C-003**: If `write_buffer_size` is reduced from default (64 MB), `level0_slowdown_writes_trigger` and `level0_stop_writes_trigger` MUST be raised proportionally for write-hot CFs (cf_utxo, cf_utxo_by_pubkey, headers, bodies, utxo, utxo_by_pubkey). Reducing write_buffer_size from 64 MB to 16 MB means 4x more L0 files per unit of data. Without raising the triggers, write stalls become 4x more likely (FM-01, FM-02). Formula: `slowdown_trigger >= (old_write_buffer / new_write_buffer) * old_trigger`.

**C-004**: `CF_PRESENCE` MUST remain in block_store's CF descriptor list. Removing it crashes nodes with existing data directories (FM-13). It SHOULD have minimal memtable (1 MB write_buffer_size, 1 max_write_buffer_number) to avoid WAL pinning.

**C-005**: Compaction style MUST remain `level` for all existing instances unless a migration path is included. Changing compaction style on existing data risks compaction storms (FM-09). Level compaction is the correct choice for cf_utxo (point-lookup-dominant) and block_store (read-heavy after write).

**C-006**: `max_total_wal_size` MUST be > 0 on block_store. Currently uncapped, enabling unbounded WAL growth via CF pinning (FM-10). Cap at <= 64 MB to bound WAL replay time to < 30s.

**C-007**: `db_write_buffer_size` MUST be > 0 on ALL 4 instances. This is the root cause of INC-I-104. No exceptions.

**C-008**: Per-CF `write_buffer_size` for hot CFs MUST be >= 8 MB. Below 8 MB, flush frequency becomes so high that compaction cannot keep up on low-core machines (2 vCPU), triggering write stalls (FM-01). For cf_utxo specifically, 16 MB is the recommended floor.

**C-009**: `max_write_buffer_number` for hot CFs MUST be >= 2 (allows one flushing + one active). 3 is recommended for burst absorption. Setting to 1 makes any flush a blocking operation.

**C-010**: Bloom filters MUST NOT be set on scan-heavy CFs (cf_utxo_by_pubkey, cf_addr_tx_index). Full bloom filters on these CFs waste memory and may increase read latency for prefix scans. Prefix bloom filters are acceptable but require careful `prefix_extractor` configuration.

**C-011**: `db_write_buffer_size` for utxo_store MAY be lower than state_db because utxo_store is rebuildable. But it MUST be >= 16 MB because utxo_store writes are per-transaction (not batched per block like state_db), creating higher flush frequency per unit of data.

**C-012**: Any proposal to share a single block cache across all 4 instances MUST ensure that cf_utxo point lookups are not evicted by block_store sequential scans. Sequential scans (set_canonical_chain backward walk, sync GetBlocks responses) can thrash a shared LRU cache. Either: (a) separate caches per instance, or (b) use RocksDB's `CacheIndexAndFilterBlocks` + `pin_l0_filter_and_index_blocks_in_cache` to protect hot data.

**C-013**: Rolling deploy is SAFE for RocksDB configuration changes (FM-14). No activation height, no synchronized deploy required. State root is computed from in-memory state, not RocksDB internals.

## Concrete Configuration Per Instance (Respecting All Constraints)

### block_store (9 CFs, 5 hot, 2 cold, 1 dead, 1 minimal)

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| `db_write_buffer_size` | 48 MB | 5 hot CFs with 8 MB each + headroom |
| `max_total_wal_size` | 64 MB | Prevent WAL pinning by dead/cold CFs (C-006) |
| `max_open_files` | 256 | Keep existing |
| Compression | Lz4 | Keep existing |
| `max_background_jobs` | 4 | Increase from 2 to handle burst sync |

| CF | `write_buffer_size` | `max_write_buffer_number` | Bloom | Notes |
|----|---------------------|--------------------------|-------|-------|
| headers | 8 MB | 3 | 10 bits | Hot write + hot point-lookup |
| bodies | 8 MB | 3 | No | Hot write + sequential read (bloom wastes space on large values) |
| height_index | 4 MB | 2 | No | Hot write, value is 32B hash |
| slot_index | 4 MB | 2 | No | Hot write, small values |
| hash_to_height | 4 MB | 2 | 10 bits | Hot point-lookup |
| tx_index | 4 MB | 2 | No | Hot write, cold read |
| addr_tx_index | 4 MB | 2 | No | Hot write, prefix scan (no full bloom) |
| presence | 1 MB | 1 | No | Dead CF, minimal allocation (C-004) |
| meta | 1 MB | 1 | No | Written once on snap sync |

L0 triggers (per hot CF): `level0_file_num_compaction_trigger=4`, `level0_slowdown_writes_trigger=40`, `level0_stop_writes_trigger=60` (C-003: raised proportionally for reduced write_buffer_size).

Block cache: 16 MB (separate from state_db). Sequential scan patterns would thrash a shared cache (C-012).

### state_db (6 CFs, 2 hot, 2 warm, 2 cold)

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| `db_write_buffer_size` | 64 MB | Must accommodate atomic_replace ~20 MB (C-002) + normal operation |
| `max_total_wal_size` | 64 MB | Keep existing |
| WAL recovery mode | PointInTime | Keep existing (C-001) |
| `max_open_files` | 256 | Keep existing |
| Compression | Lz4 | Keep existing |
| `max_background_jobs` | 4 | Increase from 2 for cf_utxo compaction |

| CF | `write_buffer_size` | `max_write_buffer_number` | Bloom | Notes |
|----|---------------------|--------------------------|-------|-------|
| cf_utxo | 16 MB | 3 | 10 bits | Hottest CF. Point-lookup-heavy (C-008, C-003) |
| cf_utxo_by_pubkey | 16 MB | 3 | Prefix 32B | Hot write, prefix scan. Prefix bloom for 32B pubkey_hash (C-010) |
| cf_meta | 2 MB | 2 | No | Hot write (per block) but tiny values |
| cf_producers | 2 MB | 2 | 10 bits | Warm. Point-lookup for getProducerInfo |
| cf_undo | 4 MB | 2 | No | Hot write (one entry/block, 1-100 KB), cold read |
| cf_exit_history | 1 MB | 1 | 10 bits | Cold. Anti-Sybil point-lookup |

L0 triggers (cf_utxo, cf_utxo_by_pubkey): `level0_file_num_compaction_trigger=4`, `level0_slowdown_writes_trigger=40`, `level0_stop_writes_trigger=60`.
L0 triggers (other CFs): default (4/20/36) is fine due to low write volume.

Block cache: 32 MB. cf_utxo point lookups dominate -- larger cache reduces disk I/O.

### utxo_store (3 CFs, 2 hot, 1 warm)

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| `db_write_buffer_size` | 32 MB | Rebuildable, smaller budget acceptable (C-011) |
| `max_total_wal_size` | N/A | WAL disabled (self-heals from state_db) |
| `max_open_files` | 128 | Lower than state_db, fewer lookups |
| Compression | Lz4 | Keep existing |
| `max_background_jobs` | 2 | Lower priority than state_db |

| CF | `write_buffer_size` | `max_write_buffer_number` | Bloom | Notes |
|----|---------------------|--------------------------|-------|-------|
| utxo | 12 MB | 2 | 10 bits | Mirrors cf_utxo. Per-tx writes (not batched) |
| utxo_by_pubkey | 12 MB | 2 | Prefix 32B | Mirrors cf_utxo_by_pubkey |
| unique_id | 2 MB | 2 | 10 bits | Low cardinality but point-lookup on mint path |

L0 triggers: `level0_slowdown_writes_trigger=40`, `level0_stop_writes_trigger=60` (per-tx writes create more L0 files than state_db's per-block batches).

Block cache: 8 MB (rebuildable, lower priority).

### diagnostic_ledger (1 CF, observability)

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| `db_write_buffer_size` | 8 MB | Keep INC-I-102 cap (C-007). Workload-justified: batched writes of 10 events x ~400B = 4KB per batch, plenty of headroom |
| `write_buffer_size` | 4 MB | Keep existing |
| `max_write_buffer_number` | 2 | Keep existing |
| Block cache | 4 MB | Keep existing |
| WAL | Disabled (or capped at 4 MB) | Lossy is acceptable |
| `max_open_files` | 64 | Keep existing |

## What ANY Proposal MUST NOT Do

1. **MUST NOT disable WAL on state_db** -- breaks last_applied canary invariant (FM-12, C-001).
2. **MUST NOT set `db_write_buffer_size < 32 MB` on state_db** -- breaks atomic_replace headroom (FM-06, C-002).
3. **MUST NOT reduce `write_buffer_size` without raising L0 triggers** -- creates write stalls on consensus hot path (FM-01, FM-02, C-003).
4. **MUST NOT remove CF_PRESENCE from block_store descriptor list** -- crashes nodes with existing data (FM-13, C-004).
5. **MUST NOT change compaction style without migration plan** -- existing SST files become problematic (FM-09, C-005).
6. **MUST NOT use `SkipAnyCorruptedRecords` WAL recovery mode** -- could silently discard last_applied write while committing other state (FM-12).
7. **MUST NOT set bloom filters on scan-heavy CFs** (cf_utxo_by_pubkey, cf_addr_tx_index) -- wastes memory, may hurt prefix scan performance (C-010).
8. **MUST NOT share a single block cache between block_store and state_db** without scan protection -- sequential block_store scans would evict cf_utxo hot data (C-012).
9. **MUST NOT set `write_buffer_size < 8 MB` on hot CFs** -- flush frequency too high for 2-core machines (C-008).
10. **MUST NOT require activation height or synchronized deploy** -- RocksDB config changes are non-consensus and can be deployed via rolling restart (C-013).

## Verification Tests Required

1. **Write stall detection test**: Write N blocks at burst rate (simulating sync catch-up). Assert that no `STALL` log entries appear in RocksDB LOG. Verify L0 file count stays below `level0_slowdown_writes_trigger` for all CFs.

2. **atomic_replace under cap test**: Call `atomic_replace` with 50k UTXOs (2x current chain). Assert success. Verify `db_write_buffer_size` accommodates the batch without error.

3. **Crash recovery canary test**: Write a block via `batch.commit()`, kill the process, restart. Assert `last_applied` matches chain_state. Run 100 times with random kill timing.

4. **utxo_store self-heal after crash test**: With WAL disabled on utxo_store, write 10 blocks, kill process mid-block, restart. Assert utxo_store rebuilds from state_db with correct count.

5. **WAL pinning test**: Open block_store with `max_total_wal_size=64 MB`. Write blocks for 1 hour. Assert total WAL size never exceeds 64 MB.

6. **CF_PRESENCE minimal allocation test**: Open block_store with existing data that contains CF_PRESENCE. Assert successful open with minimal memtable (1 MB).

7. **Memory ceiling test**: Start node with proposed configuration. Assert per-node RocksDB memory (via `GetApproximateMemoryUsageByType`) stays below 200 MB under steady-state load.

8. **Bloom filter effectiveness test**: With bloom filters on cf_utxo, measure point-lookup latency for existing and non-existing keys. Compare against no-bloom baseline. Assert at least 2x improvement on negative lookups.

## Cross-Perspective Signals

- **For the Subtractionist**: CF_PRESENCE should be dropped (after migration) rather than kept with minimal allocation. The migration is: `db.drop_cf("presence")` at open time if the CF exists, then remove it from the descriptor list in the NEXT release. Two-phase deprecation.

- **For the Pattern Analyst**: The dual-write pattern (in-memory UtxoSet + state_db BlockBatch + utxo_store per-tx writes) is architecturally redundant. utxo_store is a third copy of data already in state_db. The in-memory UtxoSet exists for performance. Consider whether utxo_store should be replaced by making state_db cf_utxo the authoritative persistent store and loading into memory at startup (which already happens via `init_utxo_set`).

- **For the Coupling Analyst**: `block_store.put_block()` uses individual `put_cf` calls (not WriteBatch). This is safe because block_store data is rebuildable from network, but it's inconsistent with the batched pattern used everywhere else in state_db. If anyone proposes per-CF options, `put_block` should be batched too.

- **For the Workload Analyst**: The `utxo_store` per-transaction write pattern (N separate `db.write(batch)` calls per block vs state_db's single batched commit) means utxo_store generates N times more WAL entries and N times more L0 files per block. This is the most write-amplified instance despite being rebuildable.

## Gaps

1. **Actual production L0 file counts not measured** -- the analysis infers write stall risk from code patterns. Measuring L0 counts from a running node's RocksDB LOG would confirm or refute FM-01.

2. **Actual atomic_replace batch size not measured** -- estimated from key/value sizes. Should be measured on a production node.

3. **No measurement of WAL file sizes on block_store** -- WAL pinning by CF_PRESENCE is architecturally certain but the actual WAL size in production is not measured.

4. **Prefix bloom filter effectiveness on cf_utxo_by_pubkey not validated** -- the recommendation for prefix bloom (C-010) is based on RocksDB documentation, not empirical measurement on DOLI's key distribution.

5. **max_background_jobs recommendation (4) not validated against target hardware** -- on 2-core machines, 4 background jobs may cause CPU contention. Should be tested.

6. **Impact of per-CF options on RocksDB memory fragmentation** -- multiple CFs with different memtable sizes may cause more memory fragmentation than uniform sizes. Not measured.

## Sources Cited

- RocksDB Wiki: "Write Stalls" (https://github.com/facebook/rocksdb/wiki/Write-Stalls)
- RocksDB Wiki: "Write Buffer Manager" (https://github.com/facebook/rocksdb/wiki/Write-Buffer-Manager)
- RocksDB source: `db/db_impl/db_impl_write.cc` -- `db_write_buffer_size` is a soft limit that triggers flush
- RocksDB Wiki: "WAL Recovery Modes" (https://github.com/facebook/rocksdb/wiki/WAL-Recovery-Modes)
- RocksDB Wiki: "Bloom Filter" (https://github.com/facebook/rocksdb/wiki/RocksDB-Bloom-Filter)
- RocksDB source: `DBImpl::Open` -- `open_cf()` applies same Options to all CFs; `open_cf_descriptors()` allows per-CF
- DOLI source: `crates/storage/src/state_db/open.rs` -- state_db opens with 6 CFs, PointInTime WAL recovery, 64 MB WAL cap
- DOLI source: `crates/storage/src/block_store/open.rs` -- block_store opens with 9 CFs, 10-bit bloom filter, no WAL cap
- DOLI source: `crates/storage/src/utxo_rocks.rs` -- utxo_store opens with 3 CFs, no WAL cap, per-tx WriteBatch
- DOLI source: `crates/storage/src/diagnostic_ledger/mod.rs` -- diagnostic_ledger opens with 8 MB db_write_buffer_size, 4 MB block cache
- DOLI source: `bins/node/src/node/apply_block/mod.rs:347` -- batch.commit() is on consensus hot path
- DOLI source: `bins/node/src/node/apply_block/tx_processing.rs:156,166` -- dual-write pattern
- DOLI source: `crates/storage/src/state_db/writes.rs:135-230` -- atomic_replace builds single large WriteBatch
