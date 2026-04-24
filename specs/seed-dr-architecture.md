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

## Archive-to-Checkpoint Bridge

### Bridge Problem Statement

After checkpoint restore at height C, blocks C+1..A exist in the archive (flat files with BLAKE3 checksums) but the restore path never consults the archive. The checkpoint provides state at height C; the archive provides the sequential block log up to height A. No code composes them. This is the "Disconnected Pipeline" anti-pattern (Pattern Matcher) -- two complementary backup systems with no bridge.

### Bridge Evaluation Summary (Round 2)

| Evaluator | Lens | Top Proposal | Confidence | Key Finding |
|-----------|------|-------------|------------|-------------|
| Subtractionist | removal | Compose 2 existing functions (5-10 lines, 0 new abstractions) | conf(0.60, inferred) | Bridge is pure composition; nothing to add beyond function calls |
| Restructurer | boundaries | Compose inline (~25 lines) or dedicated function in restore.rs | conf(0.60, observed) | Guardian RPC path KILLED: blocks via RPC don't advance state_db |
| Pattern Matcher | patterns | Single ~30-line orchestrator callable from init.rs and guardian.rs | conf(0.65, observed) | Disconnected Pipeline anti-pattern; PostgreSQL Checkpoint+WAL analogue |
| Failure Analyst | failures | Bridge useless without D1; 11 failure modes; 9 constraints | conf(0.70, observed) | D1 is hard dependency for full DR capability; commitment is always stale |
| Radical Simplifier | minimal | 8-35 lines, 0 new types/traits/modules/interfaces | conf(0.65, inferred) | "Natural" 360-line design is 10-45x excess |

### Bridge Convergence Matrix

```
                                              Subtract  Restructure  Pattern  Failure  Radical
No new abstractions (0 modules/traits/types):    Y          Y           Y        Y        Y    -> 5/5 -> DEFINITE
Compose backfill_from_archive + delete_commit:   Y          Y           Y        Y        Y    -> 5/5 -> DEFINITE
Place bridge function in restore.rs:             Y          Y           Y(*)     -        -    -> 2-3/5 -> RECOMMEND
Delete commitment BEFORE backfill:               -          Y           -        Y        -    -> 2/5 -> RECOMMEND (contradiction resolved)
Parent-hash validation before import (FM-3):     -          -           -        Y        -    -> 1/5 -> RECOMMEND (prevents catastrophic FM-3)
Assert recovery_mode at bridge entry:            -          -           -        Y        -    -> 1/5 -> RECOMMEND (prevents FM-10 race)
```

(*) Pattern Matcher says "naturally belongs in operations/restore.rs" in cross-layer signals.

### Bridge Definite Changes

#### B1: Thin bridge function composing existing primitives

**Convergence**: 5/5 evaluators agree on approach. conf(0.70, converged)

**CONVERGENCE INDEPENDENCE CHECK:**
```
Addition: bridge_checkpoint_to_archive() function
Converging evaluators: All five (on approach), 4/5 (on function vs. inline)
Evidence independence:
  - Subtractionist: dead-code analysis -> both primitives exist and work, compose them
  - Restructurer: coupling analysis -> restore.rs has all dependencies, 0 new cross-crate deps
  - Pattern Matcher: industry pattern -> PostgreSQL Checkpoint+WAL recovery = compose checkpoint + replay
  - Failure Analyst: failure enumeration -> 11 FM handled by composition with validation guards
  - Radical Simplifier: complexity audit -> 8-35 lines vs 360-line "natural" design
  INDEPENDENT? YES -- five lenses, same conclusion via different evidence
```

**What**: A single function `bridge_checkpoint_to_archive()` in `operations/restore.rs` that:

```
Step 1: VALIDATE -- verify first archive block's parent hash matches block C in store
        (prevents FM-3: wrong-network archive import, CRITICAL blast radius)
Step 2: DELETE -- state_db.delete_chain_commitment()
        (unconditionally stale after checkpoint restore per Failure Analyst C3)
Step 3: BACKFILL -- backfill_from_archive(block_store, archive_dir)
        (existing function, idempotent, skips existing blocks)
Step 4: REPORT -- return (blocks_imported, gaps_remaining)
        (enables AC3: unfillable gaps produce warnings, not errors)
```

**Estimated size**: 15-35 lines (range across evaluators: 5-50 lines; median ~25)

**Step ordering rationale (CONTRADICTION RESOLVED)**:
- Pattern Matcher proposed delete AFTER backfill (transactional semantics: "if backfill fails, commitment still valid").
- Failure Analyst proposed delete BEFORE backfill (FM-4: "strictly safer").
- Restructurer proposed delete BEFORE backfill (blocks might be rejected if stale commitment references old tip H).
- **Resolution**: Delete BEFORE backfill wins. The Failure Analyst's analysis is more thorough (examines all 4 partial-failure combinations) and the Pattern Matcher's "still valid" reasoning incorrectly assumes the commitment references checkpoint height C when it actually references pre-restore tip H. The commitment is unconditionally stale. See reasoning trace for full analysis.

**Complexity cost**: +1 function, +15-35 lines, +0 modules, +0 interfaces, +0 cross-crate dependencies.

**Failure mode handling**:
| FM | Description | How handled |
|----|-------------|-------------|
| FM-1 | No archive dir | Early return with info log |
| FM-2 | Corrupt blocks (BLAKE3) | backfill_from_archive validates per-block |
| FM-3 | Wrong network | Parent-hash validation before any writes |
| FM-4 | Ordering | Delete commitment before backfill |
| FM-6 | Duplicate blocks | backfill skips existing (idempotent) |
| FM-7 | C > A | Early return when no gap |
| FM-9 | Zombie UTXOs | Out of scope; post-bridge state root logging is partial defense |
| FM-10 | Race condition | Recovery mode assertion at entry |
| FM-11 | Format evolution | deserialize_block_compat in archiver handles legacy |

### Bridge Recommended Changes

#### BR1: Assert recovery_mode at bridge entry (if called via RPC)

**Source**: Failure Analyst P5, conf(0.65, observed)

If the bridge runs via guardian RPC while the node is live, apply_block could concurrently process inbound blocks. Recovery mode prevents this race (same class as INC-I-041).

At startup (init.rs), this is not needed -- no concurrent processing occurs during Node::new().

#### BR2: Post-bridge state root logging

**Source**: Failure Analyst C7, conf(0.60, inferred)

After bridge + replay completes, log the state root at the final height. In the zero-peers seed DR scenario, no external reference exists for comparison. Internal consistency checks (UTXO count, epoch state) provide partial defense but cannot detect zombie UTXOs (FM-9).

### Bridge Integration Points (Options for User)

The bridge function lives in `operations/restore.rs`. The question is WHERE it is CALLED FROM:

#### OPTION F: Guardian RPC call site (AC4 compliant)

**Source**: Design brief (stated integration point), Pattern Matcher, Radical Simplifier
**What**: Call bridge function from guardian.rs after checkpoint restore confirmation (exitRecoveryMode or a post-restore hook).
**AC4**: YES -- satisfies the acceptance criterion as written.
**Risks**:
- Archive directory path may not be in RpcContext (highest-risk unknown per Radical Simplifier)
- backfill_from_archive is synchronous; RPC handler is async (needs spawn_blocking)
- Restructurer KILLED the "blocks advance state via RPC" assumption -- blocks sit unapplied
**Mitigation**: The bridge only fills BlockStore gaps and deletes commitment. State advancement is handled by step 4 (recover/replay, requires D1). The bridge does not need blocks to advance state.
**Confidence**: conf(0.50, weakened by RPC path concerns)

#### OPTION G: Startup auto-detection (operationally superior)

**Source**: Pattern Matcher P1, Radical Simplifier P3
**What**: Add a 4th self-heal check to init.rs startup sequence: detect archive directory, compare archive tip vs store tip, call bridge if gap exists.
**AC4**: NO -- does not integrate into guardian restore path.
**Advantage**: Handles manual `cp` checkpoint restores automatically. Zero operator intervention. Follows proven init.rs self-heal pattern (INC-I-027 UTXO self-heal, body gap recovery).
**Risk**: Startup delay for large gaps (thousands of blocks). init.rs already at 1,171 lines.
**Confidence**: conf(0.55, partial convergence)

#### OPTION F+G: Both (recommended by Pattern Matcher P3)

**Source**: Pattern Matcher P3
**What**: Function in restore.rs callable from both sites. Guardian satisfies AC4; startup provides automatic detection.
**Complexity cost**: +5-10 lines at each call site (function body is shared).
**Confidence**: conf(0.65, covers all scenarios)

### Bridge Constraints (from Failure Analyst)

These constraints apply to ANY bridge implementation:

| ID | Constraint | Confidence | Source |
|----|-----------|------------|--------|
| BC1 | Archive directory must be discoverable (via --archive-to config) | conf(0.80) | Pattern Matcher, Radical |
| BC2 | Backfill is blocks-only, not state (does NOT advance state_db) | conf(0.95) | Restructurer |
| BC3 | Chain commitment is unconditionally stale after checkpoint restore | conf(0.65) | Failure Analyst |
| BC4 | Recovery mode must be active during RPC-triggered bridge execution | conf(0.65) | Failure Analyst |
| BC5 | Block store deduplication handled by backfill idempotency | conf(0.90) | Pattern Matcher |
| BC6 | Bridge requires D1 to deliver full DR capability (partial value without) | conf(0.70) | Failure Analyst |
| BC7 | Post-bridge verification is essential but incomplete in zero-peers scenario | conf(0.60) | Failure Analyst |
| BC8 | Archive block deserialization must use deserialize_block_compat | conf(0.55) | Failure Analyst |
| BC9 | Bridge must handle C > A (checkpoint newer than archive) gracefully | conf(0.70) | Failure Analyst |

### Bridge Dependency on D1

The Failure Analyst identified (FM-8a) that the bridge delivers blocks to step 4 (recover/replay), which is currently broken (5 fatal bugs in `recover_chain_state()`). Without D1, the bridge fills the block store and cleans the commitment but the downstream replay always fails.

**Resolution**: The bridge is independently valuable for 3 of 4 acceptance criteria:
- AC1: Block store gaps filled -- YES, independently
- AC2: Stale chain_commitment deleted -- YES, independently
- AC3: Unfillable gaps produce warnings -- YES, independently
- AC4: Full DR flow via guardian -- REQUIRES D1 for the replay step

The bridge should be built as infrastructure. D1 unlocks the full capability. Neither is useful for complete DR without the other, but both can be implemented and tested independently.

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
  Bridge: DOES NOT EXIST (checkpoint and archive are disconnected)
```

### Proposed Architecture (Definite + Recommended + Bridge)

```
CLI (main.rs)
  |
  +-- operations/chain.rs (reduced by ~224 lines)
  |     +-- recover_chain_state() -> NEW: headless Node + apply_block() loop (~50 lines)
  |     +-- truncate_chain() -> UNCHANGED
  |     +-- reindex_canonical_chain() -> UNCHANGED
  |
  +-- operations/restore.rs (+15-35 lines for bridge)
  |     +-- restore_from_archive() -> calls new recover_chain_state() -> NOW WORKS
  |     +-- restore_from_rpc() -> UNCHANGED
  |     +-- backfill_from_archive() -> UNCHANGED
  |     +-- bridge_checkpoint_to_archive() -> NEW: compose validate+delete+backfill+report
  |
  +-- node/init.rs (UNCHANGED structure, see Option B for split)
  |     +-- Node::new() -> unchanged
  |     +-- Node::new_for_replay() -> NEW: headless Node variant (~30 lines, based on new_for_test)
  |     +-- init_utxo_set() -> unchanged
  |     +-- (OPTION G: startup archive gap detection -> calls bridge_checkpoint_to_archive)
  |
  +-- node/apply_block/ (UNCHANGED)
  |     +-- CANONICAL state transition -> now also used by recovery
  |     +-- Handles ValidationMode::Replay or headless None-components (R1)
  |
  +-- node/periodic.rs (UNCHANGED)
  +-- rpc/guardian.rs (+5-10 lines for bridge call site)
  |     +-- createCheckpoint (no health.json, no rotation)
  |     +-- recovery mode enter/exit
  |     +-- (OPTION F: post-restore hook -> calls bridge_checkpoint_to_archive)
  |
  +-- storage/archiver.rs (UNCHANGED)

Data flow (FIXED):
  Runtime:  block -> apply_block() -> state_db (atomic) -> CORRECT
  Recovery: blocks -> apply_block() -> state_db (atomic) -> CORRECT (same path!)
  Checkpoint: state_db + blocks -> RocksDB hardlinks -> manual cp (or Option A) -> WORKS
  Bridge: checkpoint(C) + archive(A) -> validate -> delete_commitment -> backfill(C+1..A) -> WORKS
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

### Milestone 2: Archive-to-Checkpoint Bridge (B1) -- SHOULD

**Dependencies**: Can be implemented independently of Milestone 1 (provides partial value). Full DR requires Milestone 1.

1. Add `bridge_checkpoint_to_archive(block_store, state_db, archive_dir, genesis_hash) -> Result<BridgeReport>` to `operations/restore.rs`:
   - Validate: first archive block's parent hash matches block C in store (FM-3 defense)
   - Delete: `state_db.delete_chain_commitment()` (unconditionally safe after restore)
   - Backfill: `backfill_from_archive(block_store, archive_dir)` (existing, idempotent)
   - Report: return blocks imported + gaps remaining
2. Wire into guardian.rs call site (AC4 compliance -- Option F):
   - After checkpoint restore confirmation, call bridge function
   - Requires: archive_dir available in RpcContext or passed as parameter
   - Requires: spawn_blocking for synchronous backfill in async handler
3. Test: Regression test with checkpoint at height C, archive at height A, verify blocks C+1..A imported and commitment deleted

**Verification**: After this milestone, `backfill_from_archive` fills block store gaps after checkpoint restore. Stale chain_commitment is cleaned. Full DR requires Milestone 1 for the replay step.

### Milestone 3: Fix producers.bin readers (R3) -- SHOULD

**Dependencies**: None (standalone)

1. Update `maintainer.rs:35` to read from state_db instead of producers.bin
2. Update `run.rs:350-421` to get producer data from state_db (or confirm it's already overwritten and mark as dead code)

### Milestone 4: Deprecate legacy migration (R2) -- COULD

**Dependencies**: Milestone 1 (so recover no longer writes legacy files)

1. Add deprecation warning to legacy migration code (init.rs:236-302)
2. In a future release: replace migration code with a clear error message directing to the previous version

### Milestones 5+: User-selected options -- COULD

**Dependencies**: Milestone 1 for Options C, D, E. Independent for Options A, B.

These are deferred to user choice:
- Option A (CheckpointManager): independent, can land any time
- Option B (Split init.rs): independent, can land any time
- Option C (Unified DR pipeline): requires Milestone 1
- Option D (Auto state rebuild): requires Milestone 1
- Option E (Kill rebuild_producer_set): requires version gate analysis
- Option F+G (Both integration points): Milestone 2 for F, Milestone 1 for G

## Complexity Comparison

| Metric | Current | Radical Minimum | Proposed (D1+R1+R2+R3+B1) |
|--------|---------|----------------|---------------------------|
| State transition functions for replay | 2 (apply_block + recover) | 1 (apply_block) | 1 (apply_block) |
| DR-specific code lines | ~935 | ~200 | ~435 (+35 for bridge) |
| Data formats written during recovery | 2 (legacy + state_db) | 1 (state_db) | 1 (state_db) |
| Recovery paths that work | 1 of 2 | 2 of 2 | 2 of 2 |
| TX types handled in replay | 3/10+ | All | All |
| Epoch state after replay | Absent | Correct | Correct |
| Legacy file readers remaining | 3 (recover, run.rs, maintainer.rs) | 0 | 0 (after R3) |
| CLI commands for DR | 4 | 2 | 4 (same, but all work) |
| Shadow implementations | 3 | 0-1 | 1 (rebuild_producer_set remains) |
| Checkpoint-archive bridge | None | Composed | Composed (15-35 lines) |
| Bridge integration points | 0 | 1 | 1-2 (guardian + optional startup) |

The proposed architecture matches the Radical Simplifier's minimum for the core change (1 state transition function) while preserving existing CLI commands and not removing infrastructure that serves non-DR purposes (archiver, utxo_store). The ~235-line gap between proposed (~435) and radical minimum (~200) comes from keeping existing CLI structure, the rebuild_producer_set fallback, and the bridge function.

## Design Synthesis Quality Gate

```
--- DESIGN SYNTHESIS QUALITY GATE (Round 1 + Bridge) ---
Evaluators completed:           5/5 (both rounds)
Deletion convergence items:     2 (5/5 on recover loop, 4/5 on legacy writes)
                                + 1 bridge-specific (5/5 on no new abstractions)
Restructuring convergence:      2 (5/5 on apply_block reuse, 4/5 on headless Node)
                                + 1 bridge (5/5 on compose two primitives)
Addition options presented:     5 original (A-E) + 4 bridge (F-I)
Failure modes identified:       9 original (C1-C9) + 11 bridge (FM-1 to FM-11)
Failure modes applied as filters: 9/9 original + 11/11 bridge
Radical floor gap:              935 -> 200 (radical) -> 435 (proposed with bridge)
Contradictions found:           2 (ordering + D1 dependency)
Contradictions resolved:        2/2
Evidence independence verified: YES
---------------------------------------------------------
```
