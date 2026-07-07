# INC-I-136 Analysis: Guardian Checkpoint System Defects

**Analyst**: INC-I-136, RUN_ID=446
**Date**: 2026-07-07
**Input**: `docs/bugfixes/inc-i-136-guardian-checkpoint-defects.md` (5 defects, 7 fixes F1-F7)
**Status**: All 5 defects CONFIRMED against current code. Triage verdict: FAST.

---

## 1. Defect Confirmation (current line citations)

### DEFECT 1 -- Checkpoint creation is NOT health-gated
**Verdict: CONFIRMED**
- `periodic.rs:868-870`: Height cadence fires unconditionally:
  ```
  if current_height > 0 && current_height >= self.last_checkpoint_height + interval
  ```
- `periodic.rs:906-909`: `healthy` is computed AFTER `state_ok && blocks_ok` (line 888), written to `health.json` as metadata only (line 922-925). Nothing gates creation on health.

### DEFECT 2 -- Health signal unusable when isolated
**Verdict: CONFIRMED**
- `periodic.rs:906-907`: `point_healthy = peer_count > 0 && peers_agreeing == peer_count && unique_hashes <= 1` -- requires `peer_count > 0`.
- `periodic.rs:908`: `window_healthy = self.health_window.iter().any(|&h| h)` -- after ~10 min of isolation, all 20 samples (30s x 20 = 600s) are false.
- `periodic.rs:909`: `healthy = point_healthy || window_healthy` -- both false when isolated.
- `checkpoint_health()` in `sync/manager/peers.rs:343-398`: Returns `(0, 0, 0)` when `peer_count == 0` (line 345-347). No self-consistency check exists.
- The health signal is structurally impossible to be true during isolation, which is the exact scenario needing recovery.

### DEFECT 3 -- Rotation evicts the last good checkpoint
**Verdict: CONFIRMED**
- `periodic.rs:950-958`: Keep-last-5-by-height, unconditional:
  ```
  if dirs.len() > 5 { for old in &dirs[..dirs.len() - 5] { remove_dir_all(old.path()); } }
  ```
- No `health.json` check before eviction. No protection for the last healthy checkpoint.

### DEFECT 4 -- No block-store completeness / undo-data validation at checkpoint time
**Verdict: CONFIRMED**
- `periodic.rs:879-886`: Checkpoint creates `state_db.create_checkpoint()` and `block_store.create_checkpoint()`. Both are RocksDB hard-link snapshots (`rocksdb::checkpoint::Checkpoint::new(&self.db).create_checkpoint(path)` -- see `state_db/queries.rs:28-33` and `block_store/queries.rs:18-23`).
- No contiguity check on block bodies, no undo-data presence check. The health.json only records peer agreement, not internal state completeness.

### DEFECT 5 -- UTXO rebuild-from-state_db doubles the in-memory count
**Verdict: CONFIRMED -- root cause corrected from spec hypothesis**

The spec inferred: "rebuild appends instead of replacing/clearing, doubling the live set." The actual mechanism is different and more subtle:

**True root cause -- `utxo_count` atomic counter inflation:**

1. `StateDb::open()` (`state_db/open.rs:192-204`): Counts all cf_utxo entries, initializes `utxo_count: AtomicU64::new(count)`. Correct count: 32888.

2. `UtxoSet::from_state_db(state_db.clone())` (`utxo/set.rs:46-48`): Creates `RocksDb(Arc<StateDb>)` variant. The UtxoSet IS the state_db -- no separate in-memory copy.

3. `recover_body_gaps()` called at `init.rs:399` when tip block has body gaps.

4. When no undo data found (`init.rs:107`), the rebuild path fires:
   ```rust
   utxo_set.clear();                                    // line 107
   for (outpoint, entry) in state_db.iter_utxos() {     // line 108
       let _ = utxo_set.insert(outpoint, entry);        // line 109
   }
   ```

5. **`utxo_set.clear()` is a NO-OP** for the RocksDb variant (`utxo/set.rs:71`):
   ```rust
   UtxoSet::RocksDb(_) => {
       // state_db clearing is handled by StateDb::clear_and_write_genesis.
   }
   ```

6. `state_db.iter_utxos()` (`state_db/writes.rs:332-347`) returns all 32888 UTXOs from cf_utxo.

7. `utxo_set.insert()` dispatches to `sdb.insert_utxo()` (`state_db/writes.rs:22-38`):
   - `put_cf(cf_utxo, &key, &value)` -- RocksDB upsert (idempotent, data unchanged)
   - **`self.utxo_count.fetch_add(1, Ordering::Relaxed)`** (line 38) -- UNCONDITIONAL increment

8. Result: `utxo_count` = 32888 (original init) + 32888 (re-inserted) = **65776** (2x).
   The RocksDB data itself is correct (same keys overwritten). Only the `AtomicU64` counter is inflated.

9. `utxo_len()` (`state_db/queries.rs:90-92`) returns `self.utxo_count.load(Ordering::Relaxed)` -- the inflated value. This feeds `getStateRootDebug.utxoCount`, making the recovery convergence check meaningless.

**The spec's directional diagnosis was correct** (rebuild doubles the count), but the mechanism is not "appending duplicate data" -- it is the `insert_utxo()` function unconditionally incrementing an atomic counter on upsert. The fix must target the counter, not the data.

---

## 2. Architecture Context + Blast Radius

### Module Boundaries

| Module | Responsibility | Depends On | Depended By |
|--------|---------------|------------|-------------|
| `bins/node/src/node/periodic.rs` | Auto-checkpoint creation, health tagging, rotation, periodic health diagnostics | `StateDb.create_checkpoint()`, `BlockStore.create_checkpoint()`, `SyncManager.checkpoint_health()`, `Node.health_window` | `getGuardianStatus` RPC (reads checkpoint dirs + health.json) |
| `bins/node/src/node/init.rs` | Startup body-gap recovery, UtxoSet construction | `StateDb`, `BlockStore`, `UtxoSet`, `ChainState` | Node::new() callers |
| `crates/storage/src/state_db/writes.rs` | `insert_utxo()` with counter management | `rocksdb::DB`, `AtomicU64` | `UtxoSet::insert()`, `recover_body_gaps()` |
| `crates/storage/src/utxo/set.rs` | `UtxoSet` enum dispatch (InMemory vs RocksDb) | `StateDb`, `InMemoryUtxoStore` | `apply_block`, `rollback`, `init.rs`, RPC methods |
| `crates/network/src/sync/manager/peers.rs` | `checkpoint_health()` -- peer agreement signal | SyncManager peer table, `recent_canonical_hashes` ring buffer | `periodic.rs` health tagging |
| `crates/rpc/src/methods/guardian.rs` | `getGuardianStatus` -- reads checkpoint dirs, finds last healthy | filesystem (checkpoints/), `chain_state`, `sync_manager` | CLI `guardian status`, operator procedures |

### Data Flows Through Affected Area

```
Every N blocks (periodic.rs:868):
  StateDb.create_checkpoint() + BlockStore.create_checkpoint()
    -> checkpoint dir on disk
  SyncManager.checkpoint_health() -> (peer_count, agreeing, unique_hashes)
  Node.health_window (VecDeque<bool>) -> point_healthy || window_healthy
    -> health.json written to checkpoint dir
  Rotation: keep last 5 by height, remove_dir_all oldest

getGuardianStatus RPC (guardian.rs:162-234):
  Reads checkpoint dirs -> sorts by height -> last = most recent
  Scans in reverse for first health.json with healthy=true -> last_healthy_checkpoint

Startup body-gap recovery (init.rs:56-138):
  Detects body gaps -> attempts rollback with undo data
  If undo missing -> utxo_set.clear() + state_db.iter_utxos() + utxo_set.insert()
    -> utxo_count inflated (Defect 5)
```

### Architectural Constraints & Invariants

- **Checkpoint = RocksDB hard-link snapshot**: Near-instant, near-zero extra disk. Both `create_checkpoint()` methods use `rocksdb::checkpoint::Checkpoint`. This is opaque to the guardian -- it snapshots whatever the DB holds.
- **`utxo_count` is an in-memory shadow counter**: Initialized by scanning cf_utxo on open (`open.rs:192-200`), maintained by `insert_utxo (+1)` and `remove_utxo (-1)`. The counter and the actual key count can diverge if `insert_utxo` is called with an existing key (upsert).
- **`health_window` is non-persistent**: Lost on restart. 20 samples at 30s = 10 min memory.

### Blast Radius

**Direct impact (files touched by fixes):**
- `bins/node/src/node/periodic.rs` -- F1, F2, F3, F4, F5 (checkpoint creation, health, rotation)
- `bins/node/src/node/init.rs` -- F6 (body-gap rebuild path)
- `crates/storage/src/state_db/writes.rs` -- F6 (insert_utxo counter behavior)

**Indirect impact (consumers of output):**
- `crates/rpc/src/methods/guardian.rs` -- reads health.json; benefits from fixes but needs no code changes
- `crates/network/src/sync/manager/peers.rs` -- `checkpoint_health()` may need extension for F2 (return self-consistency signal alongside peer signal)
- Recovery procedures (`bridge_from_archive`, manual restore scripts) -- benefit from reliable `last_healthy_checkpoint`

**NOT impacted:**
- `apply_block()` -- not touched
- `validation.rs` -- not touched
- Block production, consensus rules, epoch boundaries -- not touched
- Wire protocol, peer communication -- not touched

### Consensus Impact Verification

**CONFIRMED: These fixes are confined to local operational state.** None of the 5 defects or 7 fixes touch:
- Block validation rules
- Transaction validation
- Epoch boundary logic
- Producer scheduling
- State root computation
- Wire protocol / peer messages

All changes are in the checkpoint subsystem (periodic snapshots + startup recovery), which is local node-level infrastructure. **No activation height is needed.** The fixes can be deployed via normal binary upgrade -- no synchronized deploy required because they do not change block content or consensus rules.

---

## 3. Brittleness Check

```
--- BRITTLENESS CHECK ---
Signals detected: 1/5
Details:
  1. Cross-module blast radius: NO (2 files primary, 1 storage helper)
  2. Invariant gaps: YES -- utxo_count atomic has no invariant enforcing count == actual keys
  3. Data flow reversal: NO
  4. Shared mutable state: NO (utxo_count is single-writer at the call sites in question)
  5. Contract absence: NO (insert_utxo's counter semantics are implicit but single-use)
Verdict: LOCALIZED
---
```

---

## 4. Triage Verdict

```
--- TRIAGE VERDICT ---
Path: FAST
Confidence: conf(0.97, code-verified)
Reasoning: All 5 defects confirmed at exact line numbers; blast radius confined to 2-3 files; no consensus rules touched; root cause of UTXO doubling ground-truthed to atomic counter inflation in insert_utxo.
---
```

---

## 5. Milestone Plan

### M1 -- F6: UTXO rebuild counter fix (init.rs + state_db/writes.rs)

**The simplest fix that addresses the root cause**: In `recover_body_gaps()` (`init.rs:107-109`), when the UtxoSet is RocksDb-backed, skip the clear+re-insert loop entirely -- the UtxoSet IS the state_db, so the "rebuild" is a no-op on the data and only corrupts the counter. Add a method `UtxoSet::is_rocksdb_backed() -> bool` and guard the loop.

Alternatively (defense-in-depth): Fix `insert_utxo()` to not increment `utxo_count` when the key already exists (check-before-increment). This is the structural root cause, but has a minor perf cost (extra RocksDB get per insert).

**Recommended approach**: Both. The primary fix is the guard in `recover_body_gaps()` (skip the loop when RocksDb-backed). The secondary fix is making `insert_utxo()` idempotent on the counter (defense-in-depth for any future caller).

**Files touched:**
- `bins/node/src/node/init.rs` (~5 lines)
- `crates/storage/src/utxo/set.rs` (~3 lines, add `is_rocksdb_backed()`)
- `crates/storage/src/state_db/writes.rs` (~3 lines, check key existence before counter increment)

**Acceptance criteria (maps to F6):**
- [ ] After the rebuild path fires, `getStateRootDebug.utxoCount` equals the init-time persisted count
- [ ] State root computation is valid after rebuild
- [ ] `insert_utxo` on an existing key does not change `utxo_count`

**FAIL->PASS test:**
- Unit test: open a StateDb with N UTXOs, call `insert_utxo()` with an existing key, assert `utxo_len() == N` (not N+1).
- Integration test: construct a UtxoSet::RocksDb, call `recover_body_gaps()` with a body gap and missing undo, assert `utxo_set.len() == state_db.utxo_len()` after.

---

### M2 -- F1+F2+F4: Checkpoint validity (periodic.rs, possibly sync/manager/peers.rs)

**The simplest fix that addresses the root cause**: Add a `self_consistent()` predicate that checks (a) block bodies contiguous to tip, (b) undo data present for the last N blocks (configurable rollback window, e.g. 100), (c) epoch state loadable. This predicate is peer-independent -- it validates internal state. Redefine `healthy` as `self_consistent AND (point_healthy OR window_healthy OR isolated_but_consistent)`. Refuse to tag a checkpoint `healthy` if `self_consistent` is false. Introduce `isolated` as a distinct status from `forked/corrupt`.

**Files touched:**
- `bins/node/src/node/periodic.rs` (~40 lines: add self-consistency check before/during health tagging)
- `crates/storage/src/block_store/queries.rs` or `maintenance.rs` (~15 lines: add `has_contiguous_bodies(from, to)` + `has_undo_data(from, to)` helpers, or expose them if they exist)
- `crates/storage/src/state_db/queries.rs` (~5 lines: expose `get_undo()` existence check range)
- `crates/network/src/sync/manager/peers.rs` -- possibly extend `checkpoint_health()` return type to include self-consistency, or keep it separate

**Acceptance criteria (maps to F1, F2, F4):**
- [ ] F1: An isolated-but-internally-consistent node produces a `healthy` checkpoint
- [ ] F2: `healthy=true` achievable with `peers=0` when state is self-consistent
- [ ] F4: No `healthy` checkpoint can exist that fails to serve its own tip or roll back
- [ ] A forked/gappy node cannot produce a `healthy` checkpoint regardless of peer agreement

**FAIL->PASS tests:**
- Test: Node with 0 peers, contiguous block store, valid undo data -> checkpoint tagged `healthy=true` with `isolated=true`.
- Test: Node with body gap at tip -> checkpoint tagged `healthy=false` regardless of peer count.
- Test: Node with missing undo data in rollback window -> checkpoint tagged `healthy=false`.

---

### M3 -- F3+F5: Rotation immunity + deeper retention (periodic.rs)

**The simplest fix that addresses the root cause**: Before rotation, scan the eviction candidates for the last `healthy` checkpoint. If evicting it would leave zero healthy checkpoints, skip its eviction. For deeper retention (F5), keep the last-known-healthy checkpoint outside the 5-slot recent window.

**Files touched:**
- `bins/node/src/node/periodic.rs` (~30 lines: read health.json of eviction candidates, protect last healthy)

**Acceptance criteria (maps to F3, F5):**
- [ ] F3: After an arbitrarily long incident (>5 checkpoints created), the last pre-incident healthy checkpoint still exists on disk
- [ ] F5: A multi-day incident cannot rotate away all pre-incident anchors
- [ ] The number of checkpoint directories on disk is bounded (prevent unbounded growth)

**FAIL->PASS tests:**
- Test: Create 10 checkpoints (first 2 healthy, rest unhealthy). After rotation, the most recent healthy checkpoint still exists.
- Test: With 5 recent unhealthy + 1 old healthy, the old healthy is retained even though it's outside the last-5 window.

---

### F7 -- Archive-based state reconstruction (SEPARATE TICKET)

**Recommendation: Split F7 to a separate ticket.** F7 is a feature (archive replay -> state reconstruction), not a bugfix. It has different scope (touches the archive system, potentially adds a new CLI command or RPC method), different testing requirements, and different risk profile. F1-F6 fix the guardian so it produces usable checkpoints; F7 is a backstop for when even good checkpoints don't survive.

F7 depends on F1-F6 (the guardian must first produce valid checkpoints before we design the fallback). Implementing them together increases risk and review burden.

---

## 6. Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|-------------------|
| REQ-GUARD-001 | Fix UTXO rebuild counter doubling (F6) | Must | utxoCount == persisted count after rebuild; insert_utxo idempotent on counter |
| REQ-GUARD-002 | Health-gate checkpoint creation with self-consistency predicate (F1+F2) | Must | isolated-but-consistent node produces healthy checkpoint; forked/gappy node cannot |
| REQ-GUARD-003 | Validate block-store completeness + undo-data at checkpoint time (F4) | Must | No healthy checkpoint with body gaps or missing undo data |
| REQ-GUARD-004 | Protect last healthy checkpoint from rotation (F3) | Must | Last pre-incident healthy survives arbitrarily long incident |
| REQ-GUARD-005 | Deeper retention horizon (F5) | Should | Multi-day incident cannot rotate away all pre-incident anchors |
| REQ-GUARD-006 | Archive-based state reconstruction (F7) | Won't (this iteration) | Deferred to separate ticket |

## 7. Traceability Matrix

| Requirement ID | Priority | Fix IDs | Test IDs | Milestone |
|---------------|----------|---------|----------|-----------|
| REQ-GUARD-001 | Must | F6 | TEST-M1-COUNTER (P1a,P1b,P2a,P2b), TEST-M1-REBUILD (P2c direct + UtxoSet), TEST-M1-EDGE (remove+reinsert) | M1 |
| REQ-GUARD-002 | Must | F1, F2 | TEST-M2-HEALTH (P1a,P1b,P1c,P2a,P2b,P3a,P3b,P4a,P4b,P5a + 4 edge cases) | M2 |
| REQ-GUARD-003 | Must | F4 | TEST-M2-CONTIGUOUS (P1a,P2a,P2b,P2c,P3a,P4a,P4b), TEST-M2-UNDO (P1a,P2a,P2b,P2c,P3a,P4a,P4b) | M2 |
| REQ-GUARD-004 | Must | F3 | (test-writer) | M3 |
| REQ-GUARD-005 | Should | F5 | (test-writer) | M3 |
| REQ-GUARD-006 | Won't | F7 | N/A | Separate ticket |

## 8. Impact Analysis

### Existing Code Affected
- `init.rs` (recover_body_gaps): Logic change to skip rebuild loop for RocksDb-backed UtxoSet -- Risk: low (additive guard, no behavior change for InMemory variant)
- `state_db/writes.rs` (insert_utxo): Add existence check before counter increment -- Risk: low (minor perf cost, correct semantics)
- `periodic.rs` (auto-checkpoint block): Add self-consistency check + rotation protection -- Risk: low (local operational state, no consensus impact)
- `utxo/set.rs`: Add `is_rocksdb_backed()` method -- Risk: negligible

### Regression Risk Areas
- `insert_utxo` is also called by `add_transaction` for the RocksDb variant during rollback. The existence-check fix must not break rollback paths where the key genuinely does not exist.
- Rotation logic change must not cause unbounded checkpoint directory growth.

## 9. Assumptions

| # | Assumption (technical) | Explanation (plain language) | Confirmed |
|---|----------------------|---------------------------|-----------|
| 1 | `insert_utxo` is never intentionally called to count duplicates | The unconditional fetch_add is a bug, not a feature | Yes (code review) |
| 2 | All production nodes use the RocksDb-backed UtxoSet | InMemory variant is only for testing and snap sync | Yes (init.rs:305) |
| 3 | Checkpoint rotation at 5 is sufficient for normal operation | Deeper retention (F5) is for incident survival, not normal ops | Yes (current behavior) |

## 10. What I Don't Understand

- How often `recover_body_gaps()` fires in production. The body-gap check runs on every startup when `chain_state.best_height > 0` and the tip block exists in the block store, but it only triggers the rebuild path when undo data is missing. This should be rare (post-checkpoint-restore scenario), but I have no frequency data.
- Whether `BlockBatch.commit()` (the normal apply_block path) also has the counter-inflation bug. It uses `utxo_delta: i64` (a signed delta applied atomically on commit), which is a different and presumably correct mechanism. Not investigated because it's outside scope, but worth verifying.

## 11. Specs Drift Detected

None. The checkpoint system has no dedicated spec file. The guardian procedures are in `.claude/skills/guardian/` which is a skill file, not a spec.
