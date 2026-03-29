# Fork Recovery & Risk Analysis: fix/sync-state-explosion-root-causes vs main

**Date**: 2026-03-28
**Merge-base**: `103fc31a`
**Source branch**: `fix/sync-state-explosion-root-causes` (96 commits ahead)
**Target branch**: `main` (162 commits ahead)

---

## Question 1: Are the fork recovery changes truly incompatible or can they be composed?

**Verdict: Structurally incompatible. Cannot be composed by mechanical merge.**

The two branches made fundamentally different design decisions in `fork_recovery.rs`:

### Main branch changes (89 lines changed):
1. **Deterministic hash tiebreak**: Replaces the solo-fork heuristic with `fork_tip.hash() < current_tip` for delta=0 forks. All nodes converge regardless of gossip order.
2. **Visibility changes**: `pub(super)` → `pub` on 6 functions (for integration test access).
3. **Log format changes**: Structured `[FORK]` prefixed logging.
4. **No new recovery paths**: `apply_checkpoint_state` remains unchanged in signature and behavior.

### Source branch changes (288+ lines changed):
1. **`rejected_fork_tips` tracking**: When reorg fails (no common ancestor), marks all block hashes as rejected. Future gossip blocks extending these tips are skipped (INC-I-014 fix).
2. **`force_recover_from_peers()` fallback**: When rollback fails in `maybe_auto_resync()`, escalates to peer recovery instead of waiting. Exponential backoff (60→120→300→600→960s) replaces flat 3600s cooldown.
3. **`apply_checkpoint_state` → `apply_snap_snapshot`**: Complete replacement. New function takes a `network::VerifiedSnapshot` struct, verifies state root, uses `storage::StateSnapshot::deserialize()`, preserves genesis_hash, caches state root atomically, persists via `atomic_replace`, seeds canonical index, sets store floor, records snap sync in sync manager. This is ~150 lines of new code replacing ~60 lines.
4. **`reset_state_only()`**: New layered state reset (9.5 layers) with INC-I-005 Fix C confirmed-height-floor guard. Replaces the old `force_resync_from_genesis()` as the automatic recovery path. The nuclear reset still exists but only for manual `recover --yes`.
5. **`recover_from_peers_inner()`**: New method with 90% tip guard, block-1-gap detection, and force bypass.
6. **`last_producer_list_change` type change**: `Option<Instant>` → `Arc<RwLock<Option<Instant>>>` (for spawned gossip tasks).

**Composition analysis**: The deterministic hash tiebreak from main is a pure improvement that source branch lacks. Source branch's `rejected_fork_tips`, `force_recover_from_peers`, and `apply_snap_snapshot` are structural additions main lacks. However, the `apply_checkpoint_state` → `apply_snap_snapshot` replacement is NOT additive — it removes the function main still references. The `reset_state_only()` factoring likewise rewrites `force_resync_from_genesis()` internals.

**Recommendation**: Source branch's fork recovery is the correct base. Main's deterministic hash tiebreak (the `fork_tip.hash() < current_tip` logic) should be manually ported INTO the source branch's version, replacing whatever tiebreak logic source currently uses (source kept the original solo-fork heuristic which main correctly replaced).

---

## Question 2: Does the prior analysis miss any CRITICAL conflicts in apply_block or validation_checks?

**Yes. Three critical conflicts the prior analysis missed or understated:**

### apply_block/mod.rs — CRITICAL CONFLICT

**Main changes** (2 lines): Only changes `pub(super)` → `pub` on `apply_block()` signature.

**Source changes** (~150 lines): Adds the entire **Reactive Round-Robin Scheduling** system inline in `apply_block()`:
- Captures `prev_slot` before `update_chain_state_for_block`
- Detects missed slots → unschedules rank 0 producers
- Block producer → schedules (proved alive)
- Attestation bitfield → re-enters attesters
- **State root recomputation** after scheduling mutations (FIX C1)
- Removes `cumulative_rollback_depth = 0` reset

**Risk**: These are independent changes (visibility vs. scheduling logic). They compose cleanly IF the merge tool handles the `pub(super)` → `pub` alongside the scheduling insertion. The scheduling code is an addition after line 166 in source; main only touches line 11. **Low conflict risk, but the state root recomputation (FIX C1) is consensus-critical** — if lost during merge, snap sync nodes diverge.

### validation_checks.rs — CRITICAL CONFLICT (624 lines vs 205 lines)

**Main changes** (205 lines):
- `pub(super)` → `pub` on 5 functions
- Replaces bond-weighted scheduling with `bond_weights_for_scheduling()` helper (epoch-locked snapshot)
- Adds `excluded_producers` to ValidationContext
- Adds scheduler fingerprint logging
- Adds per-byte extra_fees to coinbase validation (v4.9.0 feature)
- Backward-compatible coinbase: accepts both `base_reward + extra_fees` and `base_reward` only

**Source changes** (624 lines):
- Replaces bond-weighted scheduling with **Reactive Round-Robin** (`scheduled_producers_at_height()`, equal weight 1 ticket each)
- Adds hardcoded genesis producers for INC-001 RC-10 (testnet/mainnet match)
- Replaces `SyncResponse::Block(Box<Option<Block>>)` → `SyncResponse::Block(Option<Block>)`
- Replaces `GetStateAtCheckpoint` → `GetStateSnapshot` + `GetStateRoot` + `GetHeadersByHeight`
- Replaces `StateAtCheckpoint` → `StateSnapshot` + `StateRoot`
- Removes `GetBlocksByHeightRange`
- Adds `SEED_CONFIRMATION_DEPTH` filtering for seed mode
- Removes Light mode skip for EpochReward validation (validates in both modes)
- Removes re-broadcast from `handle_new_transaction` (gossipsub handles forwarding)

**CRITICAL MISSED RISK**: The scheduling model is fundamentally different:
- **Main** uses `bond_weights_for_scheduling()` (epoch-locked bond snapshot, weighted by bond count)
- **Source** uses `scheduled_producers_at_height()` (equal weight 1 ticket each, filtered by `scheduled` flag)

These are **mutually exclusive consensus models**. A node running main's scheduling cannot validate blocks produced by source's scheduling, and vice versa. The merge must choose ONE scheduling model. This is not a textual conflict — it's a **semantic conflict** that git merge-tree cannot detect.

### The SyncRequest/SyncResponse enum is a WIRE PROTOCOL BREAK

Source branch replaces the entire sync protocol vocabulary:
- `GetStateAtCheckpoint` → removed
- `GetBlocksByHeightRange` → removed
- `GetStateSnapshot` → new
- `GetStateRoot` → new
- `GetHeadersByHeight` → new
- `StateAtCheckpoint` → removed
- `StateSnapshot` → new
- `StateRoot` → new
- `Block(Box<Option<Block>>)` → `Block(Option<Block>)` (breaks serde compatibility)

Main's `handle_sync_request` in validation_checks.rs still references `GetStateAtCheckpoint`, `GetBlocksByHeightRange`, and `StateAtCheckpoint`. Source's handler references the new variants. **These cannot coexist — all nodes must use the same protocol.**

---

## Question 3: Are there storage-layer changes that could cause state divergence during the port?

**Yes. Source has 552 insertions across 9 storage files. Key consensus-critical changes:**

### crates/storage/src/snapshot.rs (+221 lines, -30 lines)
- New `compute_state_root_from_bytes()`: Deserializes from wire format (bincode), re-encodes canonically, then hashes. This is the snap sync verification path.
- New `StateSnapshot::verify()` and `StateSnapshot::deserialize()`: Used by the new `apply_snap_snapshot()`.
- New `deserialize_canonical_utxo()`: Manual parser for canonical UTXO format (8-byte count prefix, 36-byte keys, 61+ byte entries). Replaces `UtxoSet::deserialize_canonical()`.
- **Test coverage**: 4 new tests verify roundtrip + cross-format root agreement.

**Risk**: Main does NOT have these functions. If main's `apply_checkpoint_state` is kept instead of source's `apply_snap_snapshot`, the new snapshot functions are unused but harmless. If source's snap sync is ported, these functions are REQUIRED and must come along.

### crates/storage/src/producer/set_core.rs (+98 lines)
- `scheduled_producers_at_height()`: Filters by `scheduled` flag
- `unschedule_producer()` / `schedule_producer()`: Reactive scheduling mutations
- `reset_scheduled_flags()`: Clean slate for rebuild

**Risk**: These are the storage-side implementation of the reactive round-robin. If the reactive scheduling model is chosen, these are required. If main's bond-weighted model is chosen, these are dead code (but harmless). The `scheduled` field on `ProducerInfo` is the real concern — if it's added to the struct, it affects serialization and thus **state root computation on all nodes**.

### crates/storage/src/producer/types.rs (+26 lines)
The `scheduled` field is added to `ProducerInfo`. This changes the bincode serialization of `ProducerSet`, which means:
- All nodes must use the same version
- State roots computed before/after this change will differ
- Snap sync snapshots from old nodes won't deserialize on new nodes (or vice versa)

**This is the single biggest state divergence risk in the entire port.** If `ProducerInfo` struct layout changes, every node must upgrade simultaneously or the network forks.

### crates/storage/src/block_store/open.rs (+5 lines)
- Adds `clear_indexes()` method (used by `reset_state_only()`).

### crates/storage/src/state_db/open.rs (+7 lines)
- Adds `atomic_replace()` method (used by `apply_snap_snapshot()`).

---

## Question 4: Are there dependency conflicts that could block compilation?

**Minor conflicts only. No blocking issues.**

### bins/node/Cargo.toml

**Main adds**:
- `[lib]` section (name = "doli_node", path = "src/lib.rs")
- `futures` in dev-dependencies
- `maintainer-scripts` and `post_install_script` for packaging

**Source adds**:
- `libc = "0.2"` dependency (for `process_rss_mb()` via `getrusage`)

These are independent additions to different sections. They compose without conflict.

**However**, main's `[lib]` section implies main created a library target (`src/lib.rs`). Source branch does NOT have this file. If source's code depends on internal module visibility (`pub(super)` vs `pub`), the library target may expose or fail to expose needed symbols.

Main changed 30+ functions from `pub(super)` to `pub` for integration test access. Source kept `pub(super)` on most functions. After merge, the `lib.rs` on main expects `pub` functions. If source's `pub(super)` versions overwrite main's `pub` versions, the lib target won't compile (the test crate can't access the functions).

---

## Question 5: What's the biggest risk the prior analysis didn't identify?

**The biggest missed risk is the SEMANTIC CONSENSUS CONFLICT in the scheduling model.**

The prior analysis correctly identified file-level conflicts but missed that the two branches implement **incompatible consensus rules**:

### Main: Bond-weighted scheduling with epoch-locked snapshot
```
active_producers_at_height() → bond_weights_for_scheduling() → weighted tickets
```
- Bond count determines tickets in the scheduler
- Epoch-locked snapshot prevents mid-epoch divergence
- `excluded_producers` HashSet on Node struct for fork recovery

### Source: Reactive Round-Robin with scheduled flag
```
scheduled_producers_at_height() → 1 ticket each (equal weight)
```
- ALL producers get 1 ticket (bonds don't affect scheduling)
- `scheduled` flag on ProducerInfo filters producers in/out
- Missed slot → unscheduled; attestation → re-scheduled
- Changes are persisted in ProducerSet (affects state root)

**Impact**: If nodes disagree on which scheduling model to use, they compute different eligible producers for each slot. Block validation fails. The network forks.

### Additional missed risks:

1. **`cumulative_rollback_depth` removal**: Source removes the cumulative rollback depth tracking and the MAX_CUMULATIVE_ROLLBACK=50 guard. Source also removes the genesis rollback protection (`target_height == 0` check). These were safety invariants on main. Source replaces them with `peak_height` tracking and the 10-block MAX_SAFE_ROLLBACK from peak — a different safety model. The merge must choose one.

2. **`seen_blocks_for_slot` removal**: Source removes this field from Node. Main still uses it. If source's version overwrites main, any code referencing `seen_blocks_for_slot` won't compile.

3. **Network crate public API divergence**: Source exports `ForkAction`, `RecoveryPhase`, `RecoveryReason`, `SyncPhase`, `SyncPipelineData`, `VerifiedSnapshot`. Main exports none of these. Source removes `ForkSync`, `ForkSyncResult`, `ProbeResult` exports. Main still uses `ForkSync`. These are not just visibility changes — they represent different architectures in the sync manager.

4. **`last_producer_list_change` type change**: Source wraps this in `Arc<RwLock<>>`. All access sites must use `.write().await` / `.read().await` instead of direct assignment. This touches `init.rs`, `periodic.rs`, `fork_recovery.rs`, and `event_loop.rs`.

5. **event_loop.rs is 80% rewritten**: Source removes 580 lines and adds 114 (net -466). Main adds 60 and removes 38 (net +22). Source extracted network event handling into `network_events.rs` and tx announcement handling into `tx_announcements.rs`. Main's changes are inline in the original monolithic event loop. These are structurally incompatible — the merge will produce a broken event_loop.rs.

---

## Summary Table

| # | Risk | Severity | Prior Analysis Caught? |
|---|------|----------|----------------------|
| 1 | Scheduling model conflict (bond-weighted vs reactive round-robin) | **CRITICAL** (consensus) | No — listed as file conflict, missed semantic incompatibility |
| 2 | SyncRequest/SyncResponse wire protocol break | **CRITICAL** (network) | Partially — noted protocol changes but not that they're mutually exclusive |
| 3 | `ProducerInfo.scheduled` field changes bincode serialization | **CRITICAL** (state root) | No |
| 4 | `apply_checkpoint_state` → `apply_snap_snapshot` replacement | HIGH | Yes |
| 5 | `reset_state_only()` replaces rollback safety model | HIGH | Partially |
| 6 | event_loop.rs 80% rewrite vs inline patches | HIGH | Partially |
| 7 | Node struct field additions/removals (rejected_fork_tips, seen_blocks_for_slot, etc.) | HIGH | Partially |
| 8 | Network crate public API divergence (ForkAction, VerifiedSnapshot, etc.) | HIGH | No |
| 9 | `last_producer_list_change` type change (Option → Arc<RwLock<Option>>) | MEDIUM | No |
| 10 | `pub(super)` vs `pub` visibility across 30+ functions | MEDIUM | No |
| 11 | Cargo.toml dependency additions (libc, futures, lib target) | LOW | No |
| 12 | Main's deterministic hash tiebreak not in source | LOW (easy to port) | Yes |

---

## Recommendation

The fork recovery changes from both branches serve different purposes and cannot be mechanically merged. The port should:

1. **Use source branch as the fork_recovery.rs base** (it has the deeper structural fixes)
2. **Manually port main's deterministic hash tiebreak** into source's `handle_completed_fork_recovery()`
3. **Decide on ONE scheduling model** before attempting any merge — this is a prerequisite, not a merge artifact
4. **Port the `ProducerInfo.scheduled` field** only if reactive round-robin is chosen; if bond-weighted is chosen, the field must NOT be added
5. **Port main's `excluded_producers` clearing on rollback** — source has `rebuild_scheduled_from_blocks` instead, which serves the same purpose differently

The prior analysis's transplant plan should be revised to include the scheduling model decision as a gating prerequisite. Without that decision, the transplant will produce a node that cannot reach consensus.
