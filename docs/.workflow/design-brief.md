# Design Brief — INC-I-104 RocksDB Configuration Redesign

## Refined Prompt (user's verbatim request, anchors already pre-empted)

First-principles design of the correct RocksDB configuration for the 4 RocksDB instances in doli-node: `block_store`, `state_db`, `utxo_store`, `diagnostic_ledger`. The design must reflect what these databases architecturally NEED based on workload and durability — not what fits a specific VPS. If the resulting per-node memory footprint doesn't fit a given server, that's an operational decision.

For each of the 4 instances, derive concrete values for:

- `db_write_buffer_size` (total memtable budget across all CFs)
- `write_buffer_size` per CF (or differentiated per-CF if some CFs deserve more)
- `max_write_buffer_number`
- `min_write_buffer_number_to_merge`
- `max_total_wal_size`
- Block cache size (and whether to share one Cache across all 4 instances or keep separate)
- `block_size`
- Compression style (Lz4 / Zstd / None) per level and per CF
- Compaction style (Universal / Level)
- `max_background_jobs`, `max_subcompactions`
- `target_file_size_base`, `max_bytes_for_level_base`
- `level0_file_num_compaction_trigger`, `level0_slowdown_writes_trigger`, `level0_stop_writes_trigger`
- `bloom_filter_bits_per_key` (if appropriate)
- Any per-CF overrides

## Hard Constraints (verbatim)

- Architecture is NOT reverse-engineered from hardware. Hardware fit is downstream operational.
- Do NOT anchor on INC-I-102's 8 MB value as the answer for the other 3 DBs. Derive each independently from workload.
- WebSearch / WebFetch permitted; cite RocksDB sources.
- Per-CF differentiation where workload differs (e.g., write-cold vs write-heavy CFs).
- One spec, one set of values, one architectural commitment.

## Incident context

INC-I-104 — "ai5 n9-n12 RAM growth ~8 MB/min + sync coordinator sees 1 peer while transport reports 45". Status: diagnosed. Severity: high. Domain: mainnet/network/sync.

**Root cause (recorded)**: 3 of 4 RocksDB instances uncapped. Direct RocksDB LOG dump on ai5/n9 shows: `state_db`, `block_store`, `utxo_store` all have `db_write_buffer_size=0` (no total memtable budget), per-CF `write_buffer_size=67108864` (64 MB), `max_write_buffer_number=2`. Only `diagnostic_ledger` has `db_write_buffer_size=8388608` (8 MB cap from f37febcf). `block_store` and `utxo_store` also have `max_total_wal_size=0`.

**Per-node natural ceiling** = sum across active CFs of `write_buffer_size × max_write_buffer_number`. ai5 plateaus at ~450 MB/node (4 × 450 = 1.8 GB on 3.7 GB box, fits). Family server fails because 6 × 450 MB = 2.7 GB > 1.9 GB total RAM; OOM during memtable warm-up.

## Analyst's Redesign Analysis

Full document: `docs/redesigns/inc-i-104-redesign-analysis.md` (must read).

**Workload inventory key findings** (19 column families total across 4 instances):

### block_store (9 CFs)
- Hot CFs: `headers`, `bodies`, `height_index`, `slot_index`, `hash_to_height` (every block, 6 blocks/min steady state, burst during sync)
- Cold CFs: `tx_index`, `addr_tx_index` (write per block, read RPC-only)
- Dead CF: `presence` (deprecated, cleaned on open, never written) — currently consumes 64 MB × 2 memtable budget for nothing
- `meta` (cold): written once on snap sync only
- WAL currently uncapped — WAL pinning by dead `presence` CF possible

### state_db (6 CFs)
- Hottest CF: `cf_utxo` (point lookups on every validation, deletes+inserts on every block; tens of millions of entries)
- Hot CFs: `cf_utxo_by_pubkey` (mirrors cf_utxo), `cf_meta` (chain_state + canary every block)
- Warm CFs: `cf_producers` (epoch boundary only, dirty-only writes), `cf_undo` (one entry per block, 1–100+ KB each)
- Cold CF: `cf_exit_history` (only on producer exit)
- WAL capped at 64 MB (only DB with WAL cap currently)

### utxo_store (3 CFs)
- Self-heals from state_db on startup (architecture doc confirms; node-heal excludes utxo_store)
- All 3 CFs are rebuildable (no consensus criticality of its own)
- WAL currently uncapped

### diagnostic_ledger (1 CF)
- Pure observability (NoOpEmitter fallback)
- Async batched writes (10 events or 100ms)
- Channel bounded at 1024 with drop-oldest
- Currently capped at 8 MB total (INC-I-102 fix)

## Durability tiering

| Tier | Definition | CFs |
|------|-----------|-----|
| Consensus-critical | Loss → state root divergence | state_db {cf_utxo, cf_producers, cf_exit_history, cf_meta}, block_store {headers, bodies} |
| Rebuildable | Recomputable from network or other DB | block_store secondary indexes, state_db {cf_utxo_by_pubkey, cf_undo}, utxo_store (all 3) |
| Observability | Lossy ok, NoOp fallback | diagnostic_ledger |

## Latency budgets (read paths)

- `cf_utxo` point lookup: < 1ms (validation hot path)
- WriteBatch commit: < 10ms (must fit in 10s slot)
- `headers` lookup: < 10ms (sync responses)
- `cf_utxo_by_pubkey` prefix scan: < 50ms (RPC)
- Diagnostic RPC: < 500ms (debug)

## Crash-recovery profile

- block_store: WAL replay; alternatively network re-sync (slow)
- state_db: WAL replay + last_applied canary; alternatively snap sync (medium)
- utxo_store: self-heals from state_db (fast, WAL replay unnecessary)
- diagnostic_ledger: WAL replay; lossy ok; NoOp fallback

## Acceptance Criteria

Full table in `docs/redesigns/inc-i-104-redesign-analysis.md` §7. Key items:

- **Must**: behavior preserved (no state root change), bounded per-DB memory (db_write_buffer_size > 0 everywhere), per-CF differentiation, WAL bounded on all instances, one spec one set of values
- **Should**: read-path latency preserved, WAL replay < 30s, bloom filters on point-lookup CFs, deprecated `presence` CF doesn't consume memtable budget
- **Could**: shared block cache, WAL disabled on rebuildable-only instances, compaction-style differentiation per workload
- **Won't**: hardware-driven sizing, runtime-configurable memory, jemalloc swap

## Open Questions

Q1 — Shared vs per-instance block cache?
Q2 — state_db compaction style (level vs universal)?
Q3 — block_store block cache justification?
Q4 — diagnostic_ledger 8 MB cap workload-justified or band-aid?
Q5 — Optimal bloom_filter_bits_per_key per CF?
Q6 — presence CF lifecycle (keep minimal / drop / migrate)?
Q7 — utxo_store WAL necessity (given self-heal)?
Q8 — cf_undo memtable sizing (large values, low cardinality)?

## Scope

- RUN_ID: not registered (proposal-only, no `--fix`)
- INC_ID: INC-I-104
- Workflow: omega-redesign, parallel design evaluation (5 evaluators), no implementation

## Source files (evaluators MUST read)

- `crates/storage/src/block_store/open.rs` + adjacent CF defs and write paths
- `crates/storage/src/state_db/open.rs` + adjacent CF defs and write paths
- `crates/storage/src/utxo_store/open.rs` + adjacent CF defs and write paths
- `crates/storage/src/diagnostic_ledger/mod.rs` (current 8 MB cap)
- `.claude/skills/storage/SKILL.md`
- `CLAUDE.md` "If You Touch" → storage
- `docs/bugfixes/inc-i-104-analysis.md` and `docs/bugfixes/inc-i-104-handoff.md`
