<!--
OUTPUT CONTRACT: N/A — architecture specification file (not a test file)
INPUT PARTITIONS: N/A — architecture specification file (not a test file)
-->

# UTXO Storage Architecture — Approved Redesign

## Status
**Approved 2026-06-03.** Scope locked: Tier 1 + BlobDB + F1 monitor. All other tiers explicitly deferred.

**All 5 phases complete.** The UTXO storage redesign is finished.
- Phases 1-4: UTXO store elimination, read migration, write simplification, cleanup.
- Phase 5 (2026-06-04): BlobDB on cf_utxo + F1 snap-sync size monitor.

## Decision Record
- 5-evaluator parallel design analysis converged on **eliminate `utxo_store`** (4/5 evaluators independent).
- BlobDB promoted from Tier 2 to Tier 1 — confidence 0.70 (Pattern Matcher), cost ~6 config lines, addresses the dominant payload-bearing-UTXO concern (large-value compaction + cache amplification) at near-zero risk.
- F1 (snap-sync 16 MB wire limit) cannot be fixed in Tier 1/2 — but **monitoring is mandatory companion**. Detect approach to the wall before it bites.
- Pool TWAP byte-equivalence test is a **prerequisite gate on Step 4** (dual-write removal).
- All Tier 3 work (incremental hash, chunked snap sync, ContentStore wiring) explicitly **deferred** — design space not yet evidence-supported.

## Scope (approved)

### IN SCOPE
1. **Eliminate `utxo_store` RocksDB instance.** `state_db` becomes sole UTXO store. Removes dual-write, self-heal, INC-I-027 divergence class.
2. **Migrate `cf_unique_id`** from `utxo_store` to `state_db` as 7th CF; add `pending_unique_ids: HashSet<(u8, Hash)>` to `BlockBatch` for same-block uniqueness checks.
3. **Add 9 query methods** to `state_db/queries.rs` that currently exist only on `utxo_store` (`get_bonded_balance`, `count_bonds`, `get_bond_entries`, `get_all_pools`, `get_all_collateral`, `find_nft_by_token_id`, `total_confirmed`, `address_count`, etc.).
4. **Route all UTXO reads** through `state_db` (~40 RPC call sites, mechanical).
5. **Remove per-tx dual-writes** at `bins/node/src/node/apply_block/tx_processing.rs:139,156`.
6. **Delete `crates/storage/src/utxo_rocks.rs`** (1,035 lines) + self-heal in `bins/node/src/node/init.rs` + simplify `crates/storage/src/utxo/set.rs`. Net **-1,400 LOC**.
7. **Enable RocksDB BlobDB** on `cf_utxo` (and `cf_utxo_by_pubkey` if helpful) — 6 config lines:
   ```
   opts.set_enable_blob_files(true);
   opts.set_min_blob_size(4096);
   opts.set_blob_file_size(256 * 1024 * 1024);
   opts.set_blob_compression_type(Zstd);
   opts.set_enable_blob_gc(true);
   opts.set_blob_gc_age_cutoff(0.25);
   ```
8. **Increase `cf_utxo` block_size 4 KB -> 16 KB.** Matches existing precedent (`block_store` `CF_BODIES`, `state_db` `cf_undo`).
9. **Unify block cache 32 + 16 -> 48 MB single pool** (entailed by #1).
10. **F1 monitor**: Prometheus gauge for UTXO canonical serialization size; alert at 12 MB (75% of `MAX_SYNC_SIZE`). Cached snapshot, not per-request.

### OUT OF SCOPE (explicitly deferred)
- Tier 2-B payload/metadata CF split — BlobDB subsumes ~80% of benefit.
- Tier 2-C streaming state root — defer until `serialize_canonical` > 500 ms measured.
- Tier 3-A chunked snap sync — design separately as standalone workstream when monitor approaches 6-month warning.
- Tier 3-B incremental UTXO hash — defer until light-client / ZK / stateless story is concrete.
- Tier 3-C ContentStore wiring — defer until NFT/payload workload patterns are measured.

## Prerequisite Gate (Step 4 blocker)
**Pool TWAP byte-equivalence test.** `crates/storage/src/utxo_rocks.rs:237-268` and `crates/storage/src/state_db/batch.rs:129-146` both perform Bond/Pool extra_data stamping. Before removing the `utxo_rocks` write path, a property test must prove byte-for-byte output equivalence across all OutputType variants with non-empty extra_data. **If the test fails, scope expands** to fix the inequivalence before Step 4 proceeds.

## Constraints Preserved (no changes to)
- Canonical UTXO serialization format (`utxo/types.rs:57-78`)
- State root formula `H(H(cs) || H(utxo) || H(ps))` (`snapshot.rs:24-58`)
- Snap sync wire format and `MAX_SYNC_SIZE = 16 MB`
- Transaction structure, `OutputType` layout, `extra_data` field
- Undo data format (full `UtxoEntry` copies in `cf_undo`)
- `UNDO_KEEP_DEPTH = 100`
- `CURRENT_PROTOCOL_VERSION` (no bump)
- `EPOCH_STATE_FORMAT_VERSION` (no bump)
- All existing activation heights, `HardForkSchedule`
- Block content/header format
- Era growth schedule for `max_extra_data_size`

## Implementation Phases (each independently deployable via rolling restart)

**Phase 1 — Additive foundation (low risk): COMPLETE**
- Step 1: Add 9 query methods to `state_db/queries.rs`.
- Step 2: Add `cf_unique_id` to `state_db` + `pending_unique_ids` to `BlockBatch`.

**Phase 2 — Read migration (medium risk): COMPLETE**
- Step 3: Route all UTXO reads to `state_db`. Both stores still receive writes (bridge).

**Phase 3 — Write simplification (gated): COMPLETE**
- **Step 4 (GATE: Pool TWAP equivalence test must pass)**: Remove per-tx dual-writes.

**Phase 4 — Cleanup (high LOC reduction): COMPLETE**
- Step 5: Deleted `utxo_rocks.rs`, self-heal in `init.rs`, simplified `utxo/set.rs`. Startup disk cleanup removes orphaned `utxo_store/` dirs.
- Step 6: Tuned survivor — `cf_utxo` block_size 4 KB -> 16 KB, unified cache 32 -> 48 MB.
- Migration tools removed: `pool_byte_diff.rs`, `pool_backfill.rs` (no longer compilable without `utxo_store`).
- Obsolete tests removed: `state_db_query_equivalence_test.rs`, `phase2_read_migration_test.rs`, `inc_i_027_utxo_restore_selfheal.rs`.
- Metrics scraper simplified: 2 instances (block_store, state_db).

**Phase 5 — BlobDB + F1 monitor (low risk, high impact): COMPLETE**
- Step 7: BlobDB enabled on `cf_utxo` — 6 config lines in `state_db/open.rs`. Applied ONLY to cf_utxo (not cf_utxo_by_pubkey — 1-byte values don't benefit). BlobDB is transparent to application code; on-disk layout changes only. State root invariant preserved.
- Step 8: F1 snap-sync size monitor — `doli_utxo_canonical_size_bytes` gauge (60s cached recomputation via `UtxoSizeMonitor`). Threshold gauge `doli_utxo_canonical_size_threshold_bytes` set to `MAX_SYNC_SIZE` (16 MB). Alert rule recommended at 12 MB (75%).
- Startup log: "RocksDB BlobDB enabled on cf_utxo (min_blob_size=4096, blob_file_size=256MB, compression=Zstd)"
- Tests: 5 new tests — BlobDB takes-effect (.blob files appear), roundtrip (50 large UTXOs), state root invariance, F1 monitor accuracy, F1 monitor caching.

## Escalation Triggers (monitor these; act if hit)
| Threshold | Signal | Next Action |
|-----------|--------|-------------|
| UTXO canonical size > 12 MB | F1 monitor alert (`doli_utxo_canonical_size_bytes > 12582912`) | Start Tier 3-A chunked snap sync work (separate workstream) |
| `serialize_canonical` > 500 ms | `[STATE_ROOT]` log timing | Implement Tier 2-C streaming hash |
| cf_utxo SST > 1 GB individual | RocksDB compaction logs | Re-tune BlobDB thresholds or split CF |
| Block cache hit rate < 70% | RocksDB statistics | Re-evaluate cache size / CF split |
| Total UTXO > 1 M | `getChainStats` RPC | Re-evaluate full-scan operations |

## Confidence
**Overall: 0.80 (converged across 4 of 5 evaluators).** BlobDB promotion + F1 monitor close the two highest-leverage open gaps.

## Reference
Synthesizer's original 455-line proposal and 141-line reasoning trace were overwritten by parallel INC-I-104 workflow artifacts. This file is now the authoritative approved scope. Original analysis preserved in conversation history.
