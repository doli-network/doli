# Seed Node Disaster Recovery Architecture

## Problem Statement

Seed nodes are the last line of defense when the entire DOLI network goes down -- no producers, no peers to backfill from. The seed must be able to fully recover the chain from its own local backups (checkpoints and archive blocks). Currently, one of two recovery paths (archive restore via `recover`) is completely broken due to 5 compounding bugs, while the other (checkpoint restore) works but is entirely manual with no verification. The architecture must eliminate shadow implementations of the state transition function and provide reliable, self-sufficient disaster recovery.

## Evaluation Summary

| Evaluator | Lens | Top Proposal | Confidence | Key Finding |
|-----------|------|-------------|------------|-------------|
| Subtractionist | removal | Delete recover_chain_state(), replace with apply_block() replay | conf(0.65, observed) | recover is a broken reimplementation; the only writer of legacy files |
| Restructurer | boundaries | Extract replay_blocks() -- headless replay sharing canonical state transition | conf(0.6, observed) | State transition duplication is a boundary drawn at the wrong abstraction level |
| Pattern Matcher | patterns | Replace recover internals with Node::new_for_test + apply_block loop | conf(0.55, inferred) | This is the Shadow Implementation anti-pattern; Bitcoin solved it with single ConnectBlock |
| Failure Analyst | failures | Archive restore path broken by 3 compounding bugs (P2+P3+P4) | conf(0.95, measured) | Wrong genesis hash, wrong UTXO path, 3/10+ tx types -- each independently fatal |
| Radical Simplifier | minimal | Delete recover, replay via apply_block(), eliminate legacy formats | conf(0.65, observed) | Minimum viable is 1 state transition function, ~200 lines vs current ~935 |

## Convergence Matrix

### Deletion Convergence

```
                                           Subtract  Restructure  Pattern  Failure  Radical
Delete recover_chain_state() replay loop:     Y          Y           Y        Y        Y    -> 5/5 -> DEFINITE
Remove legacy file writes from recover:       Y          Y           -        Y        Y    -> 4/5 -> DEFINITE
Remove legacy migration code (init.rs):       Y          -           -        -        Y    -> 2/5 -> RECOMMEND
Remove archiver from DR path:                 Y          -           -        -        -    -> 1/5 -> OPTION
Remove utxo_store (dual-write):               Y          -           -        -        -    -> 1/5 -> OPTION
Delete recovery_mode flag:                    KILLED     -           -        -        -    -> KILLED
Remove rebuild_producer_set_from_blocks:       -          -           Y        -        -    -> 1/5 -> OPTION
```

### Restructuring Convergence

```
                                           Subtract  Restructure  Pattern  Failure  Radical
Reuse apply_block() for replay:               Y          Y           Y        Y        Y    -> 5/5 -> DEFINITE
Create headless Node for offline replay:      Y          Y           Y        -        Y    -> 4/5 -> DEFINITE
Add ValidationMode::Replay or equivalent:     -          -           -        Y        -    -> 1/5 -> RECOMMEND (via constraint C8)
Unify checkpoint ops (CheckpointManager):     -          Y           -        -        -    -> 1/5 -> OPTION
Split init.rs into focused modules:           -          Y           -        -        -    -> 1/5 -> OPTION
Compose checkpoint + archive pipeline:        -          -           Y        -        -    -> 1/5 -> OPTION
```

## Definite Changes (High Convergence)

### D1: Delete `recover_chain_state()` replay loop and replace with canonical `apply_block()` replay

**Convergence**: 5/5 evaluators independently proposed this. conf(0.92, converged)

**CONVERGENCE INDEPENDENCE CHECK:**
```
Deletion: recover_chain_state() replay loop (chain.rs:332-412)
Converging evaluators: ALL FIVE
Evidence independence:
  - Subtractionist: based on dead-code analysis -- recover is the ONLY writer of legacy files
  - Restructurer: based on coupling analysis -- boundary drawn at wrong abstraction level (BV-1)
  - Pattern Matcher: based on industry pattern -- Bitcoin's -reindex uses single ConnectBlock; Shadow Implementation anti-pattern
  - Failure Analyst: based on failure enumeration -- 3 independently fatal bugs (P2, P3, P4)
  - Radical Simplifier: based on complexity audit -- 275 lines of parallel maintenance burden with zero savings
  INDEPENDENT? YES -- five different analytical lenses, five different evidence sources.
  True convergence. conf boost applies.
```

**What changes**:
- Delete the 275-line replay loop in `recover_chain_state()` (chain.rs:332-412)
- Replace with a thin wrapper (~50 lines) that:
  1. Opens BlockStore (existing blocks)
  2. Wipes or creates fresh state_db
  3. Constructs a headless Node (using `Node::new_for_test()` pattern, proven at ~170 lines)
  4. Calls `apply_block(ValidationMode::Light)` for each block height 1..tip
- Net change: approximately -224 lines (delete 274, add ~50)

**Evidence**:
- `apply_block()` handles ALL tx types including AddBond, DelegateBond, RevokeDelegation, EpochReward, ProtocolActivation, AddMaintainer, RemoveMaintainer
- `apply_block()` writes to state_db atomically via BlockBatch
- `apply_block()` rebuilds epoch state via post_commit_actions at epoch boundaries
- `apply_block()` handles genesis completion at genesis_blocks+1
- `Node::new_for_test()` already creates headless Node without networking (~170 lines, proven in test infrastructure)
- `ValidationMode::Light` already exists for skipping time-based checks during sync/reorg

### D2: Eliminate legacy file writes from the recovery path

**Convergence**: 4/5 evaluators. conf(0.88, converged)

**CONVERGENCE INDEPENDENCE CHECK:**
```
Deletion: Legacy file writes (chain_state.bin, utxo/, producers.bin)
Converging evaluators: Subtractionist, Restructurer, Failure Analyst, Radical Simplifier
Evidence independence:
  - Subtractionist: based on write-path analysis -- recover is the ONLY writer of legacy files
  - Restructurer: based on data flow analysis -- offline path writes to legacy files while runtime writes to state_db
  - Failure Analyst: based on path mismatch discovery -- recover writes to utxo/, init.rs reads utxo.bin or utxo_rocks/
  - Radical Simplifier: based on complexity audit -- 2 data formats where 1 suffices
  INDEPENDENT? YES -- four different evidence paths to the same conclusion.
  True convergence. conf boost applies.
```

**What changes**:
- Recovery path writes directly to state_db via atomic BlockBatch (inherent in D1's apply_block reuse)
- Legacy file writes in recover_chain_state() (chain.rs:419-448) are deleted along with the function

**This is automatically achieved by D1** -- since the new replay wrapper calls `apply_block()`, which writes to state_db, there are no legacy file writes.

## Recommended Changes (Medium Convergence)

### R1: Handle apply_block() side effects during headless replay

**Source**: Failure Analyst constraint C8, referenced by Pattern Matcher and Radical Simplifier. conf(0.75, observed)

`apply_block()` has 16 side effects. During headless replay, at least 5 must be suppressed:
1. **Recovery mode gate** (would drop blocks being replayed) -- suppress
2. **Snap sync guard** (not applicable) -- suppress
3. **Block deduplication** (blocks already in store during replay) -- suppress
4. **Mempool operations** (no mempool during recovery) -- safe when mempool is None
5. **Network broadcasts** (no network during recovery) -- safe when components are None

**Recommended approach**: Add a `ValidationMode::Replay` variant (or reuse the existing headless Node pattern where network/mempool components are None, which already suppresses 4 and 5). The deduplication check (mod.rs:45-73) needs explicit handling -- either skip it in replay mode or don't pre-load blocks into the store.

### R2: Remove legacy migration code after deprecation period

**Convergence**: 2/5 (Subtractionist, Radical Simplifier). conf(0.65, converged)

**What**: Delete the legacy migration code in init.rs:236-302 (~66 lines) that migrates chain_state.bin, producers.bin, utxo.bin, and utxo_rocks/ to state_db.

**Caveat**: Subtractionist explicitly lowered confidence because it cannot be verified whether any production node still has legacy files. Mainnet is at height 18000+, suggesting all nodes have migrated, but this is unverifiable without node access.

**Recommended path**: Mark the migration code as deprecated in documentation. Remove it in a future version after confirming all operators have migrated. Include a clear error message ("legacy files detected -- please use version X.Y.Z to migrate first") rather than silent failure.

### R3: Fix `producers.bin` readers outside of `recover`

**Source**: Subtractionist cross-layer signal, confirmed by Restructurer.

Two non-DR code paths still read `producers.bin`:
- `run.rs:350-421` -- loads producers for UpdateService Arc, but immediately overwritten by init.rs:587-590 from state_db
- `maintainer.rs:35` -- reads producers.bin directly instead of state_db (correctness risk)

**Recommended**: Update both to read from state_db. The run.rs path is effectively dead code (overwritten). The maintainer.rs path is a correctness bug -- it may show different producers than the node is actually using.

## Options for User Decision

### OPTION A: Unified CheckpointManager module

**Source**: Restructurer (P2), conf(0.55, observed)

**What**: Create a `CheckpointManager` module that unifies checkpoint creation, rotation, health tagging, and restoration into a single boundary. Currently:
- `periodic.rs` creates checkpoints with health.json and rotation
- `guardian.rs` creates checkpoints without health.json or rotation
- Checkpoint restore has NO code -- entirely manual `cp`

**Evidence**: Restructurer found that checkpoint creation and restoration are semantically paired operations split across three modules in three crates (BV-2). A CheckpointManager would add the missing `restore_from_checkpoint()` capability (REQ-DR-009).

**Complexity cost**: +1 new module (~150 lines), periodic.rs and guardian.rs both shrink by delegating, +1 missing capability (restore command)

**Failure mode filter**:
- C1 (atomicity): NEUTRAL -- checkpoints already use RocksDB atomic snapshots
- C9 (checkpoint health): RESOLVES -- centralizes health metadata handling
- FM P1 (checkpoint height mismatch): RESOLVES -- could add coordination/locking during creation

**vs. Radical floor**: +1 module above minimum viable. The Radical Simplifier did not propose this, so it adds complexity. However, it fills a genuine capability gap (no restore command exists today).

### OPTION B: Split init.rs into focused modules

**Source**: Restructurer (P3), conf(0.65, observed)

**What**: Split init.rs (1,171 lines, 2.3x the 500-line budget) into:
- `node/self_heal.rs` (~150 lines): init_utxo_set, recover_body_gaps, verify_state_consistency
- `node/migration.rs` (~80 lines): migrate_legacy_to_state_db
- `node/init.rs` (~940 lines): Node::new, Node::new_for_test

**Evidence**: init.rs embeds four unrelated responsibilities with different change frequencies. Extracting self-heal enables REQ-DR-008 (post-restore verification from CLI, currently impossible because self-heal is locked behind Node::new).

**Complexity cost**: +2 files, -0 modules removed, init.rs shrinks from 1,171 to ~940 lines. DR verification becomes independently testable and callable from CLI.

**Failure mode filter**:
- All failure modes: NEUTRAL -- this is a structural refactor, not a behavioral change.

**vs. Radical floor**: +2 files above minimum viable. But addresses a real project constraint violation (500-line module budget).

### OPTION C: Compose checkpoint + archive into unified DR pipeline

**Source**: Pattern Matcher (P3), conf(0.6, observed)

**What**: Create a `restore-checkpoint` CLI command that: (1) copies checkpoint state_db+blocks to data_dir, (2) optionally replays archive blocks from checkpoint height to archive tip using the canonical apply_block replay from D1.

**Evidence**: Pattern Matcher identified this as the Database Checkpoint + WAL pattern (PostgreSQL, MySQL, RocksDB). The infrastructure exists (checkpoint creation + archive blocks) but is not composed.

**Complexity cost**: +1 CLI command (~80 lines), reuses existing infrastructure. Requires D1 to land first (headless replay for the archive gap).

**Failure mode filter**:
- C9 (checkpoint health): VULNERABLE -- must select healthy checkpoint; needs health.json inspection
- C3 (epoch state): RESOLVES -- replay through apply_block handles epoch boundaries correctly

**vs. Radical floor**: This IS part of the radical minimum (the Radical Simplifier proposed 2 commands: `restore-checkpoint` and `replay`).

### OPTION D: Extend self-heal pattern to full state rebuild at startup

**Source**: Pattern Matcher (P4), conf(0.5, inferred)

**What**: Extend the INC-I-027 UTXO self-heal to cover state_db itself: if state_db is empty but blocks exist, automatically replay all blocks to rebuild state_db at startup.

**Evidence**: The self-heal pattern in init.rs:40-94 is proven and deployed. Extension to full state rebuild would make recovery automatic.

**Complexity cost**: +20 lines for detection, but requires D1 (headless replay capability) as a dependency.

**Failure mode filter**:
- All failure modes: NEUTRAL to RESOLVES -- eliminates the "blocks present, state missing" scenario automatically.

**vs. Radical floor**: Slightly above minimum viable (+20 lines). Low cost, high defensive value.

### OPTION E: Eliminate `rebuild_producer_set_from_blocks` (shadow #2)

**Source**: Pattern Matcher (P5), conf(0.45, inferred)

**What**: Delete the `rebuild_producer_set_from_blocks` function in rewards.rs:1013-1220 (~200 lines), which is a near-shadow of apply_block handling 7/9 tx types. Replace the rollback genesis-rebuild fallback with an error requiring the `replay` command.

**Caveat**: Kill test found this fallback fires for pre-upgrade blocks predating the undo system. Removing it breaks rollback for nodes that haven't replayed since undo introduction. Requires a version gate.

**Complexity cost**: -200 lines. Eliminates the second shadow implementation.

**Failure mode filter**:
- Pre-upgrade compatibility: VULNERABLE -- breaks rollback for old blocks without undo data.

**vs. Radical floor**: This IS part of the radical minimum. But the pre-upgrade risk makes it conditional.

## Constraints (from Failure Analyst)

Any chosen path MUST respect these invariants:

| ID | Constraint | Confidence | Impact on Proposals |
|----|-----------|------------|---------------------|
| C1 | All state changes for a single block committed in one RocksDB WriteBatch | conf(0.95) | D1 satisfies via apply_block's BlockBatch |
| C2 | Genesis hash must come from ChainSpec, not a literal | conf(0.95) | D1 satisfies by using canonical apply_block |
| C3 | Epoch state must be derived via EpochState::derive_at_boundary() | conf(0.85) | D1 satisfies via post_commit_actions |
| C4 | Producer mutations deferred to epoch boundaries (except epoch 0) | conf(0.95) | D1 satisfies by using canonical apply_block |
| C5 | Genesis completion at genesis_blocks+1 clears phantom producers | conf(0.95) | D1 satisfies via maybe_complete_genesis() |
| C6 | UTXO self-heal on startup reads from state_db | conf(0.95) | D1 writes to state_db, so self-heal works |
| C7 | Blocks already in store trigger deduplication in apply_block | conf(0.90) | R1 must handle: replay mode or pre-load skip |
| C8 | At least 5 of 16 apply_block side effects must be suppressed during replay | conf(0.80) | R1 addresses this explicitly |
| C9 | Checkpoint selection should prefer health.json-verified healthy checkpoints | conf(0.70) | Currently manual; Option A addresses |

## Architecture Maps

### Current Architecture

```
CLI (main.rs)
  |
  +-- operations/chain.rs (467 lines)
  |     +-- recover_chain_state() -> BROKEN: 3/10+ tx types, legacy files, wrong genesis hash
  |     +-- truncate_chain() -> WORKS: undo-based rollback
  |     +-- reindex_canonical_chain() -> WORKS: rebuild block index
  |
  +-- operations/restore.rs (336 lines)
  |     +-- restore_from_archive() -> calls recover_chain_state() -> BROKEN
  |     +-- restore_from_rpc() -> WORKS: downloads blocks via RPC
  |     +-- backfill_from_archive() -> WORKS: fills gaps from flat files
  |
  +-- node/init.rs (1,171 lines)
  |     +-- Node::new() -> includes legacy migration, UTXO self-heal, body gap recovery
  |     +-- init_utxo_set() -> self-heal from state_db (INC-I-027)
  |
  +-- node/periodic.rs
  |     +-- auto-checkpoint with health.json + rotation
  |
  +-- node/apply_block/ (~1,045 lines across 5 sub-modules)
  |     +-- CANONICAL state transition (ALL tx types, epoch state, undo data)
  |
  +-- rpc/guardian.rs (267 lines)
  |     +-- createCheckpoint (no health.json, no rotation)
  |     +-- recovery mode enter/exit
  |
  +-- storage/archiver.rs (367 lines)
        +-- BlockArchiver (flat file export)
        +-- import_archive_blocks (restore from flat files)

Data flow (BROKEN):
  Runtime:  block -> apply_block() -> state_db (atomic) -> CORRECT
  Recovery: blocks -> recover() -> legacy files (non-atomic) -> BROKEN
  Checkpoint: state_db + blocks -> RocksDB hardlinks -> manual cp -> WORKS
```

### Proposed Architecture (Definite + Recommended)

```
CLI (main.rs)
  |
  +-- operations/chain.rs (reduced by ~224 lines)
  |     +-- recover_chain_state() -> NEW: headless Node + apply_block() loop (~50 lines)
  |     +-- truncate_chain() -> UNCHANGED
  |     +-- reindex_canonical_chain() -> UNCHANGED
  |
  +-- operations/restore.rs (UNCHANGED)
  |     +-- restore_from_archive() -> calls new recover_chain_state() -> NOW WORKS
  |     +-- restore_from_rpc() -> UNCHANGED
  |     +-- backfill_from_archive() -> UNCHANGED
  |
  +-- node/init.rs (UNCHANGED structure, see Option B for split)
  |     +-- Node::new() -> unchanged
  |     +-- Node::new_for_replay() -> NEW: headless Node variant (~30 lines, based on new_for_test)
  |     +-- init_utxo_set() -> unchanged
  |
  +-- node/apply_block/ (UNCHANGED)
  |     +-- CANONICAL state transition -> now also used by recovery
  |     +-- Handles ValidationMode::Replay or headless None-components (R1)
  |
  +-- node/periodic.rs (UNCHANGED)
  +-- rpc/guardian.rs (UNCHANGED)
  +-- storage/archiver.rs (UNCHANGED)

Data flow (FIXED):
  Runtime:  block -> apply_block() -> state_db (atomic) -> CORRECT
  Recovery: blocks -> apply_block() -> state_db (atomic) -> CORRECT (same path!)
  Checkpoint: state_db + blocks -> RocksDB hardlinks -> manual cp (or Option A) -> WORKS
```

## Migration Path

### Milestone 1: Fix the replay path (D1 + R1) -- MUST

**Dependencies**: None (standalone change)

1. Create `Node::new_for_replay(data_dir, network)` -- variant of `new_for_test` that:
   - Accepts network parameter (not hardcoded Devnet)
   - Takes no producer keypairs (producers come from replayed blocks)
   - Opens real BlockStore and fresh StateDb
   - Does NOT start networking, mempool, sync, or archiver
2. Handle apply_block side effects for replay mode (R1):
   - Block deduplication: either add `ValidationMode::Replay` that skips it, or ensure blocks are consumed from BlockStore reads (not pre-stored)
   - Recovery mode gate: headless Node has recovery_mode=false
   - Network/mempool: None in headless Node (already no-ops)
3. Rewrite `recover_chain_state()` to:
   - Construct headless Node via `new_for_replay`
   - Loop `apply_block(block, ValidationMode::Light)` for blocks 1..tip
   - Delete the 275-line broken replay loop
   - Delete legacy file writes (chain_state.bin, utxo/, producers.bin)
4. Test: Create regression test that runs recover on a test chain and verifies state_root matches canonical path

**Verification**: After this milestone, `recover --yes` produces correct state_db with all tx types, correct genesis hash, complete epoch state, and undo data.

### Milestone 2: Fix producers.bin readers (R3) -- SHOULD

**Dependencies**: None (standalone)

1. Update `maintainer.rs:35` to read from state_db instead of producers.bin
2. Update `run.rs:350-421` to get producer data from state_db (or confirm it's already overwritten and mark as dead code)

### Milestone 3: Deprecate legacy migration (R2) -- COULD

**Dependencies**: Milestone 1 (so recover no longer writes legacy files)

1. Add deprecation warning to legacy migration code (init.rs:236-302)
2. In a future release: replace migration code with a clear error message directing to the previous version

### Milestones 4+: User-selected options -- COULD

**Dependencies**: Milestone 1 for Options C, D, E. Independent for Options A, B.

These are deferred to user choice:
- Option A (CheckpointManager): independent, can land any time
- Option B (Split init.rs): independent, can land any time
- Option C (Unified DR pipeline): requires Milestone 1
- Option D (Auto state rebuild): requires Milestone 1
- Option E (Kill rebuild_producer_set): requires version gate analysis

## Complexity Comparison

| Metric | Current | Radical Minimum | Proposed (D1+R1+R2+R3) |
|--------|---------|----------------|------------------------|
| State transition functions for replay | 2 (apply_block + recover) | 1 (apply_block) | 1 (apply_block) |
| DR-specific code lines | ~935 | ~200 | ~400 |
| Data formats written during recovery | 2 (legacy + state_db) | 1 (state_db) | 1 (state_db) |
| Recovery paths that work | 1 of 2 | 2 of 2 | 2 of 2 |
| TX types handled in replay | 3/10+ | All | All |
| Epoch state after replay | Absent | Correct | Correct |
| Legacy file readers remaining | 3 (recover, run.rs, maintainer.rs) | 0 | 0 (after R3) |
| CLI commands for DR | 4 | 2 | 4 (same, but all work) |
| Shadow implementations | 3 | 0-1 | 1 (rebuild_producer_set remains) |

The proposed architecture matches the Radical Simplifier's minimum for the core change (1 state transition function) while preserving existing CLI commands and not removing infrastructure that serves non-DR purposes (archiver, utxo_store). The ~200-line gap between proposed (~400) and radical minimum (~200) comes from keeping existing CLI structure and the rebuild_producer_set fallback.

## Design Synthesis Quality Gate

```
--- DESIGN SYNTHESIS QUALITY GATE ---
Evaluators completed:           5/5
Deletion convergence items:     2 (5/5 agreement on recover loop, 4/5 on legacy writes)
Restructuring convergence:      2 (5/5 on apply_block reuse, 4/5 on headless Node)
Addition options presented:     5 (A through E)
Failure modes identified:       9 (C1-C9 from Failure Analyst)
Failure modes applied as filters: 9/9
Radical floor gap:              935 lines -> 200 (radical) -> 400 (proposed)
Contradictions found:           1 (see reasoning trace)
Contradictions resolved:        1/1
Evidence independence verified: YES
-------------------------------------
```
