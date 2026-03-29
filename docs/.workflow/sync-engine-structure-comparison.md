# Sync Engine Structure Comparison: fix/sync-state-explosion-root-causes vs main

**Date**: 2026-03-28
**Divergence point**: 103fc31a (confirmed as actual merge-base)

---

## 1. Is the sync_engine.rs delete-vs-modify conflict real?

**YES. This is a genuine delete-vs-modify conflict.**

- **Main** kept `crates/network/src/sync/manager/sync_engine.rs` (735 lines, modified +15/-19 from merge-base's 739 lines)
- **Source** deleted `sync_engine.rs` entirely and replaced it with a directory `sync_engine/` containing 4 files:
  - `mod.rs` (5 lines) — re-exports
  - `decision.rs` (272 lines) — sync decision logic
  - `dispatch.rs` (299 lines) — request dispatch
  - `response.rs` (569 lines) — response handling
  - **Total**: 1,145 lines (vs original 739)

Git cannot auto-merge this. The file `sync_engine.rs` is deleted on source and modified on main — any merge tool will flag this as a conflict requiring manual resolution.

## 2. New sync modules on source that don't exist on main

| Source-only file | Lines | Purpose |
|-----------------|-------|---------|
| `sync_engine/decision.rs` | 272 | Sync decision logic (extracted from sync_engine.rs) |
| `sync_engine/dispatch.rs` | 299 | Request dispatch (extracted from sync_engine.rs) |
| `sync_engine/mod.rs` | 5 | Module re-exports |
| `sync_engine/response.rs` | 569 | Response handling (extracted from sync_engine.rs) |
| `manager/peers.rs` | 270 | Peer management (extracted from manager/mod.rs) |
| `manager/snap_sync.rs` | 330 | Snap sync logic (extracted from manager/mod.rs) |
| `manager/types.rs` | 612 | Type definitions incl. ForkState, SyncConfig, etc. |
| `adversarial_tests.rs` | 905 | New adversarial test suite |
| `reorg/mod.rs` | 505 | Reorg logic (restructured from reorg.rs) |
| `reorg/tests.rs` | 513 | Reorg tests (new) |

**Files on main that source deleted:**

| Main-only file | Lines | Fate on source |
|----------------|-------|----------------|
| `sync_engine.rs` | 735 | Split into sync_engine/ directory (4 files) |
| `chain_follower.rs` | 876 | Deleted entirely (functionality presumably absorbed elsewhere) |
| `fork_sync.rs` | 723 | Deleted entirely (functionality presumably absorbed elsewhere) |
| `reorg.rs` | 900 | Split into reorg/ directory (mod.rs + tests.rs = 1,018 lines) |

## 3. Are main's incremental fixes captured in source's restructured version?

**NO. Main has 4 specific fixes that are NOT present in source's code.**

### Fix A: Gap threshold 12 -> 50 in empty-header escalation
- **Main**: Changed `gap <= 12` to `gap <= 50` in the consecutive-empty-headers handler (sync_engine.rs:252)
- **Source**: `response.rs` has `gap <= 50` (line 229) but `dispatch.rs` still has `gap <= 12` (line 95)
- **Status**: PARTIALLY captured. The dispatch path still uses the old threshold.

### Fix B: Don't inflate consecutive_sync_failures on empty headers
- **Main**: Removed `self.consecutive_sync_failures += 1` in the empty-headers handler, added comment: "Empty headers are fork evidence, NOT sync failures"
- **Source**: `response.rs` line 227 still has `self.fork.consecutive_sync_failures += 1` with the old comment "Activate Layer 8 (sync failure fork detection)"
- **Status**: NOT captured. Source still inflates sync failure counter on empty headers.

### Fix C: Genesis height guard in production gate (Gate 2)
- **Main**: Added `&& self.local_height > 0` to the height-lag production gate to prevent genesis deadlock (production_gate.rs:280)
- **Source**: Has a completely different production gate structure (no "Gate 2: Height lag" section). Source's `can_produce()` goes Check 1 -> Check 1b -> Check 2 (min peers) -> Check 3 (removed) -> Check 4 (finality) -> Authorized. No height-lag gate exists at all.
- **Status**: NOT captured (different architecture — source lacks this entire gate).

### Fix D: set_post_rollback() method on SyncManager
- **Main**: Added `pub fn set_post_rollback(&mut self, value: bool)` to block_lifecycle.rs (N5 incident fix)
- **Source**: No such method exists in block_lifecycle.rs.
- **Status**: NOT captured.

### Fix E: Structured [SYNC] log format
- **Main**: Converted 6 log lines to structured `[SYNC] TAG key=value` format across sync_engine.rs, cleanup.rs, production_gate.rs
- **Source**: Has some `[SYNC]` format in response.rs (2 lines), but most logs use the old verbose narrative format. cleanup.rs still has old format.
- **Status**: PARTIALLY captured (2 of ~8 log lines converted).

## 4. Scale of the sync manager redesign

**This is a significant structural redesign, not just a file split.**

### Quantitative comparison

| Metric | Merge-base | Main | Source |
|--------|-----------|------|--------|
| Total sync lines | 8,565 | 8,572 (+7) | 12,351 (+3,786) |
| Number of files | 14 | 14 | 20 |
| sync_engine lines | 739 (1 file) | 735 (1 file) | 1,145 (4 files) |
| manager/mod.rs lines | 977 | 977 | 452 |
| Test lines | 1,035 | 1,035 | 5,523 (tests.rs + adversarial + reorg/tests) |

### Key architectural differences in source

1. **ForkState sub-struct** (types.rs): Source extracts all fork-related fields into a dedicated `ForkState` struct with a `recommend_action()` method. Main keeps these as flat fields on `SyncManager`. All sync_engine sub-modules use `self.fork.*` instead of `self.*`.

2. **State transition validator** (mod.rs): Source adds `set_state(new_state, trigger)` with transition validation and logging. Main uses direct `self.state = SyncState::X` assignment. Currently `is_valid_transition()` returns `true` for all transitions (the 3-state model has no invalid transitions), but the infrastructure is there.

3. **Manager decomposition**: Source extracted `peers.rs` (270 lines), `snap_sync.rs` (330 lines), and `types.rs` (612 lines) from `manager/mod.rs`, shrinking it from 977 to 452 lines.

4. **Deleted modules**: Source removed `chain_follower.rs` (876 lines) and `fork_sync.rs` (723 lines) entirely — 1,599 lines deleted. Their functionality is presumably redistributed across the new modules.

5. **Test expansion**: Source grew tests from 1,035 to 5,523 lines (5.3x), adding adversarial tests and reorg-specific tests.

### Verdict on scale
This is an **incremental restructuring with new functionality**, not a complete rewrite. The core algorithms are recognizable but the field access patterns (`self.fork.*` vs `self.*`), state mutation API (`set_state()` vs direct assignment), and module boundaries are all different. The 44% increase in total lines (8,565 -> 12,351) comes from new tests (+4,488 lines), new types (+612 lines), and the expanded sync_engine (+406 lines).

## 5. Can source's sync modules be dropped into main as new files?

**NO. They cannot be used as drop-in additions.**

### Blocking incompatibilities

1. **Field access pattern mismatch**: Source's sync_engine sub-modules reference `self.fork.consecutive_empty_headers`, `self.fork.consecutive_sync_failures`, etc. Main's SyncManager has these as flat fields (`self.consecutive_empty_headers`). Every reference would fail to compile.

2. **Missing ForkState struct**: Source's modules depend on `ForkState` (defined in `types.rs`), which doesn't exist on main. Adding `types.rs` alone doesn't help — `SyncManager`'s struct definition in `mod.rs` would also need to change from flat fields to `fork: ForkState`.

3. **set_state() vs direct assignment**: Source calls `self.set_state(SyncState::Idle, "reason")` (29+ call sites across the sub-modules). Main uses `self.state = SyncState::Idle`. The `set_state` method doesn't exist on main's SyncManager.

4. **Missing methods**: Source's sub-modules call `self.state_label()`, `self.request_genesis_resync()`, `self.is_valid_transition()` — none exist on main.

5. **Deleted dependencies**: Source deleted `chain_follower.rs` and `fork_sync.rs`. If you add source's modules to main, any code in main that imports from these (still-existing) files would conflict with source's replacement logic.

6. **SyncPipelineData enum**: Source has this in `types.rs` with variants not present on main. The sync_engine sub-modules reference these variants.

### What a transplant would require

To port source's sync_engine restructuring to main:
1. Add `types.rs` with ForkState, SyncConfig, etc.
2. Modify `manager/mod.rs` to replace flat fields with `fork: ForkState` and add `set_state()`/`state_label()`
3. Delete `sync_engine.rs`, add `sync_engine/` directory with all 4 files
4. Apply main's 4 missing fixes to the new sub-modules
5. Verify all callers in `block_lifecycle.rs`, `cleanup.rs`, `production_gate.rs`, `snap_sync.rs` use the new field paths
6. Decide fate of `chain_follower.rs` and `fork_sync.rs` (keep main's or delete per source)

This is a non-trivial integration that requires modifying main's existing files, not just adding new ones.
