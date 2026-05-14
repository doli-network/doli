# INC-I-071: cf_undo Storage Bloat Analysis

## Triage Verdict

```
TRIAGE VERDICT
Path: FAST
Confidence: conf(0.9, measured)
Reasoning: Root cause identified through code analysis + measurement. Single architectural decision
           (full snapshots per block) explains the bloat. Fix is localized to apply_block + rollback.
```

## Symptom

A freshly snap-synced mainnet node on a 4GB Hetzner VPS:
- `state_db/`: 624MB (99% of data dir)
- `cf_undo`: 605MB (97% of state_db)
- `raw_average_value_size`: ~1MB per undo entry (from RocksDB compaction stats)
- Node holds ~3,327 blocks with near-zero transaction volume

## Root Cause

**Each undo entry stores FULL snapshots of both ProducerSet and EpochState** for every applied block, regardless of whether those structures changed.

### UndoData structure (`crates/storage/src/state_db/types.rs:16`):
```rust
pub struct UndoData {
    pub spent_utxos: Vec<(Outpoint, UtxoEntry)>,   // Delta (correct)
    pub created_utxos: Vec<Outpoint>,               // Delta (correct)
    pub producer_snapshot: Vec<u8>,                  // FULL SNAPSHOT (bloat source)
    pub epoch_state_snapshot: Option<Vec<u8>>,       // FULL SNAPSHOT (bloat source)
    pub chain_commitment: Option<[u8; 32]>,          // Legacy, negligible
}
```

### Measured sizes (synthetic test, bincode serialization):

| Producers | ProducerSet | EpochState | UndoData/block | x 2000 blocks |
|-----------|-------------|------------|----------------|---------------|
| 30        | 12 KB       | 34 KB      | 46 KB          | 90 MB         |
| 100       | 40 KB       | 113 KB     | 153 KB         | 306 MB        |
| 500       | 198 KB      | 565 KB     | 763 KB         | 1.5 GB        |
| 1000      | 397 KB      | 1,129 KB   | 1,526 KB       | 3.0 GB        |

### The 6.7x gap

My test gives 46KB/entry for 30 producers, but mainnet shows ~300KB/entry (605MB / 2000 entries).
The gap is likely explained by:
1. **More registered producers than 30** — ProducerSet includes ALL producers (Active, Unbonding, Exited, Slashed), not just the ~30 active ones
2. **RocksDB space amplification** — tombstones from pruned entries, SST block index overhead
3. **Larger UTXO deltas at epoch boundaries** — EpochReward transactions create ~30 UTXOs per epoch

The RocksDB compaction stat (`raw_average_value_size: 999460`) suggests entries near epoch boundaries may be closer to 1MB when combined with UTXO deltas.

## Architecture Context

### Change frequency:
| Component | Change frequency | Current undo approach |
|-----------|-----------------|----------------------|
| UTXO set  | Every block | **Delta** (correct) |
| ProducerSet | Epoch boundary only (every 360 blocks) | **Full snapshot** (99.7% redundant) |
| EpochState | Every block (accumulate_block) | **Full snapshot** (large) |

### Why ProducerSet snapshot is redundant within epochs:
- All producer mutations (Register, AddBond, Exit, Slash, Withdrawal, Delegation) are **deferred to epoch boundaries** (CLAUDE.md invariant)
- Only `process_unbonding()` can change ProducerSet mid-epoch (rare — requires an active unbonding period to complete at that exact height)
- Maintainer changes are immediate but affect `MaintainerState`, NOT `ProducerSet`
- Result: for 359 out of every 360 blocks, the ProducerSet snapshot is byte-for-byte identical to the previous block's snapshot

### UNDO_KEEP_DEPTH vs operational need:
- `UNDO_KEEP_DEPTH = 2000` (apply_block/mod.rs:297)
- `MAX_CUMULATIVE_ROLLBACK = 50` (rollback.rs:57)
- `MAX_REORG_DEPTH = 1000` (sync/reorg/mod.rs:22)
- The 2000-block retention is 40x the operational rollback limit and 2x the reorg search depth

## Scalability Projection

| Scenario | Per entry | x 2000 | With 6.7x factor |
|----------|----------|--------|-------------------|
| 30 producers (current) | 46 KB | 90 MB | **600 MB** |
| 100 producers | 153 KB | 306 MB | **2 GB** |
| 500 producers | 763 KB | 1.5 GB | **10 GB** |
| 1000 producers | 1,526 KB | 3.0 GB | **20 GB** |

At 500+ producers, cf_undo alone exceeds the total storage capacity of a 4GB VPS.

## Proposed Fix: Epoch-Aligned Sentinel (P1 + P3)

### Overview
Store full ProducerSet and EpochState snapshots **only at epoch boundary blocks** (every 360 blocks). For non-boundary blocks, store a sentinel value indicating "no change" for ProducerSet, and only the delta for EpochState.

### Implementation (2 milestones):

**M1: ProducerSet sentinel** (highest impact, lowest risk)

1. Change `producer_snapshot` to use a 2-variant enum:
   - `[0x00]` = Unchanged (sentinel)
   - `[0x01][...data...]` = Full snapshot

2. In `apply_block`: at epoch boundary OR if `process_unbonding` returned any completions, store `Full`. Otherwise, store `Unchanged` (single zero byte).

3. In `rollback`: if `Unchanged`, skip ProducerSet restore (the in-memory ProducerSet was already correct before this block). If `Full`, restore as today.

4. Backward compatibility: old undo entries (no prefix byte) decode as `Full` (the deserializer tries new format first, falls back to raw bincode).

Reduction: ProducerSet contribution drops from ~24MB to ~400KB (98.3% reduction for that component).

**M2: EpochState epoch-only snapshot**

1. At epoch boundary: store full `epoch_state_snapshot` (as today)
2. At non-boundary blocks: store `None` for `epoch_state_snapshot`
3. In rollback: if `None`, rebuild EpochState by loading the nearest epoch boundary's undo snapshot and replaying `accumulate_block` for each block from boundary to target height
4. This requires the blocks to exist in the block_store (guaranteed within UNDO_KEEP_DEPTH)

Reduction: EpochState contribution drops from ~68MB to ~200KB (99.7% reduction).

**M3 (optional): Reduce UNDO_KEEP_DEPTH from 2000 to 1000**

Halves total entries with zero format change. Safe because `MAX_REORG_DEPTH = 1000` is the actual ceiling.

### Expected result

| Component | Before | After (P1+P2+P3) |
|-----------|--------|-------------------|
| ProducerSet snapshots | ~24 MB | ~400 KB |
| EpochState snapshots | ~68 MB | ~200 KB |
| UTXO deltas | ~2 MB | ~2 MB (unchanged) |
| Total cf_undo (raw) | ~92 MB | ~3 MB |
| With RocksDB factor | ~600 MB | ~20 MB |

### Deployment: Rolling deploy safe
- UndoData is internal storage only, not consensus
- No activation height needed
- Old format entries handled by fallback deserialization
- No state root change

### Risks
1. `process_unbonding` completing mid-epoch with sentinel → mitigated by checking return value
2. Rollback across epoch boundary needing full EpochState → handled by M2 storing full snapshot at boundaries
3. CLI `truncate` command uses undo data → same format, backward compatible

## Requirements

| ID | Priority | Requirement | Acceptance Criteria |
|----|----------|-------------|---------------------|
| REQ-071-001 | Must | ProducerSet sentinel for non-boundary blocks | cf_undo ProducerSet contribution < 1MB for 2000-block window |
| REQ-071-002 | Must | Backward compat with existing undo entries | Old entries deserialize correctly |
| REQ-071-003 | Must | Rollback correctness with sentinel | rollback_one_block produces identical state for sentinel and full-snapshot blocks |
| REQ-071-004 | Should | EpochState epoch-only snapshots | cf_undo EpochState contribution < 1MB |
| REQ-071-005 | Could | Reduce UNDO_KEEP_DEPTH to 1000 | Total cf_undo < 5MB |

---

## Follow-up: stranded entries cleanup (INC-I-074, commit on `main`)

After commit `3610cbb2` shipped the per-block sentinel fix and reduced
`UNDO_KEEP_DEPTH` from 2000 to 360, mainnet measurements on N4/N5 (ai2) showed
`state_db` was still ~562 MB ~10 hours post-deploy — far above the expected
~10 MB. The per-block sentinel writes were verified correct, so the bloat had
to be **historical** entries retained by the pre-fix code.

### Root cause of the leftover bloat

`prune_undo_before(keep_height)` (`crates/storage/src/state_db/undo.rs:39-61`)
walks **forward only**: it deletes a single entry at `keep_height - 1` per
call, on the assumption that `keep_height` advances monotonically by 1 each
block. When `UNDO_KEEP_DEPTH` shrank from 2000 → 360, every entry in the range
`[H_deploy - 1999, H_deploy - 361]` (~1640 entries × ~280 KB ≈ 459 MB) was
already on disk and never got revisited — `prune_undo_before` walks forward
from `H_deploy - 360` only.

### Fix: one-shot startup bulk cleanup

Added `StateDb::prune_undo_below(keep_height) -> u64` in
`crates/storage/src/state_db/undo.rs`, mirroring the existing
`prune_undo_above` bulk-delete pattern (iterator → `WriteBatch` →
`compact_range_cf` hint). The new method is wired in
`bins/node/src/node/init.rs` to run **once at startup**, after `chain_state`
is loaded and **before** network/event-loop/production:

```rust
const UNDO_KEEP_DEPTH: u64 = 360;
let tip_height = chain_state.best_height;
if tip_height > UNDO_KEEP_DEPTH {
    let horizon = tip_height - UNDO_KEEP_DEPTH;
    let deleted = state_db.prune_undo_below(horizon);
    if deleted > 0 {
        info!("[STARTUP] Pruned {} stranded cf_undo entries below h={} \
              (INC-I-071 followup, post-UNDO_KEEP_DEPTH-reduction cleanup)",
              deleted, horizon);
    }
}
```

After one restart per node, `cf_undo` collapses to ~5–10 MB and the existing
per-block `prune_undo_before` continues to handle aging correctly.

### Test coverage (Output Contract — `crates/storage/src/state_db/tests.rs`)

- **P1 (stranded scenario)** — `prune_undo_below_bulk_deletes_stranded_entries`:
  insert heights 0..=10, call `prune_undo_below(5)`, assert return == 5 and
  entries 0..=4 gone while 5..=10 retained intact.
- **P2 (idempotent re-run)** — `prune_undo_below_idempotent_when_already_clean`:
  with no entries below the horizon, return == 0 and re-running yields 0 again.
- **P3 (`keep_height == 0`)** — `prune_undo_below_zero_keep_height_is_noop`:
  matches `prune_undo_before` semantics — return == 0, all entries retained.

### Safety properties (no consensus / no wire / no protocol bump)

- `cf_undo` is **local-only RocksDB state**, never on the wire and never in a
  state root, so this is a pure storage-layer cleanup.
- **No** `CURRENT_PROTOCOL_VERSION` bump — `EpochState` is unchanged.
- **No** activation height — not a consensus-rule change.
- **No** block content change — production path untouched.
- **Idempotent and backward-compatible** — running on an already-pruned DB
  is a no-op; running on a pre-INC-I-071 binary's data is the intended path.
- **Safe for rolling deploy** — each node self-heals on its next restart.

### Tracking
- Incident: **INC-I-074**.
- Tests: 3/3 PASS in `cargo test -p storage --lib prune_undo_below`.
- Regression: `cargo test -p doli-node --test inc_i_071_undo_snapshot_sentinel` — 3/3 PASS.
- Lint/format: `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo fmt --check` clean.

