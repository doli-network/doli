# Network Recovery Architecture

> Design synthesis from 5 independent evaluators. Date: 2026-04-21.

## Problem Statement

DOLI has 6 individual recovery tools but no anti-poisoning mechanism. When seeds are restored from checkpoint after a fork cascade, non-recovered nodes immediately gossip fork blocks to the freshly restored seeds, silently undoing the restore. This has occurred in 10+ incidents (INC-I-016, INC-I-025, INC-I-026). The recovery procedure requires ~20 manual SSH commands across 5+ servers with a critical unprotected window at seed restart.

The core architectural gap: there is no way to quarantine a seed during recovery so it drops all inbound blocks while continuing to serve RPC and snap-sync.

## Evaluation Summary

| Evaluator | Lens | Top Proposal | Confidence | Key Finding |
|-----------|------|-------------|------------|-------------|
| Subtractionist | Removal | AtomicBool + 2 check sites (~15 lines) | conf(0.65, observed) | A1 is false; 7 paths via 2 terminal functions; SyncManager deadlock kills RwLock approach |
| Restructurer | Boundaries | Arc<AtomicBool> shared Node+RpcContext, check apply_block + apply_snap | conf(0.65, observed) | 3 application gates not 1; Arc sharing follows existing announcement_sequence pattern |
| Pattern Matcher | Analogies | recovery_mode as production_blocked sibling on SyncManager, 4 intake checks | conf(0.65, observed) | production_blocked is closest analogue; RecoveryPhase enum exists; Node-struct kill INVALID (see below) |
| Failure Analyst | Failures | Dual gate + CLI flag + cache clearing | conf(0.65, observed) | 12 failure modes; F5 startup race verified; F10 cache contamination on exit; F5a reverse-poisoning |
| Radical Simplifier | Minimum | Single AtomicBool at apply_block, ~38 lines | conf(0.65, observed) | P0 (script-only) dead via peers.cache; snap serving verified safe; UTXO self-heal is orthogonal |

## Critical Finding: Pattern Matcher Kill is Invalid

The Pattern Matcher killed Node-struct placement (P4) claiming "RpcContext has no reference to Node struct, only to SyncManager and Arc-wrapped state. RPC cannot set a Node struct field."

This kill is **WRONG**. Verified code evidence:

1. `RpcContext` already holds `backfill_state: Arc<BackfillState>` which contains `AtomicBool` (context.rs:82)
2. `RpcContext` already holds `sync_manager: Option<Arc<RwLock<SyncManager>>>` shared with Node (context.rs:84)
3. Node already uses `announcement_sequence: Arc<AtomicU64>` (mod.rs:124) -- established Arc-atomic pattern
4. `startup.rs:349` shows `.with_sync_manager(self.sync_manager.clone())` -- the builder pattern for wiring shared state
5. Adding `recovery_mode: Arc<AtomicBool>` to RpcContext with `.with_recovery_mode()` follows the exact existing pattern

The Pattern Matcher confused direct Node field access with Arc-shared indirect access. RPC communicates with Node through shared Arc objects, not direct struct references. An `Arc<AtomicBool>` created by Node and cloned to RpcContext is the established pattern.

## Convergence Matrix

### Deletions (What to Remove / Disprove)

```
                         Subtract  Restruct  Pattern  Failure  Radical
A1 (single gate myth):     KILL      KILL     KILL     KILL     KILL   -> 5/5 UNANIMOUS
P0 (script-only):           -         -        -       KILL     KILL   -> 2/5 DEAD
SyncManager RwLock:        KILL      KILL       -        -        -    -> 2/5 DEAD (deadlock)
Circuit breaker:             -         -       KILL      -        -    -> 1/5 DEAD (requirement W1)
Network disconnection:       -        KILL      -        -        -    -> 1/5 DEAD (breaks M4)
Node-struct kill:            -         -       KILL      -        -    -> 1/5 INVALID (see above)
```

### Structural Decisions

```
                         Subtract  Restruct  Pattern  Failure  Radical
Arc<AtomicBool> (not RwLock):  Y       Y        -       Y        Y    -> 4/5 STRONG
Check at apply_block():        Y       Y        -       Y        Y    -> 4/5 STRONG
Check at apply_snap_snapshot(): Y      Y        -       Y        -    -> 3/5 STRONG
Field on Node struct:          Y       Y        -       Y        Y    -> 4/5 STRONG (Pattern Matcher kill invalid)
2 RPC methods (enter/exit):    Y       Y        Y       Y        Y    -> 5/5 UNANIMOUS
ADMIN_METHODS auth:            Y       Y        Y       Y        Y    -> 5/5 UNANIMOUS
Non-persistent (in-memory):    Y       Y        Y       Y        Y    -> 5/5 UNANIMOUS
--recovery-mode CLI flag:      -       -        -       Y        -    -> 1/5 RECOMMENDED
Cache clearing on exit:        -       -        -       Y        -    -> 1/5 RECOMMENDED
```

## Definite Changes (High Convergence)

These changes have conf(0.85+, converged) from 3+ independent evaluators and survive all failure mode filters.

### D1: Add `recovery_mode: Arc<AtomicBool>` to Node struct

- Field: `pub recovery_mode: Arc<AtomicBool>` in `bins/node/src/node/mod.rs`
- Initialized to `false` in `Node::new()` (`bins/node/src/node/init.rs`)
- Arc-wrapped so it can be shared with RpcContext
- Non-persistent: cleared on restart (M3 satisfied)
- Converged: Subtractionist, Restructurer, Failure Analyst, Radical Simplifier (4/5)
- conf(0.85, converged)

### D2: Add recovery_mode check at top of `apply_block()`

- File: `bins/node/src/node/apply_block/mod.rs`
- Check: `if self.recovery_mode.load(Ordering::Relaxed) { return Ok(()); }`
- MUST be the first check, before any state mutation
- Returns `Ok(())` (not `Err`) to avoid triggering unwanted recovery cascades in callers (matches existing snap_sync_height guard pattern at line 22-30)
- Performance: ~1ns AtomicBool load, negligible on hot path
- Converged: All 5 evaluators agree apply_block must be gated (4/5 agree on AtomicBool approach)
- conf(0.85, converged)

### D3: Add recovery_mode check at top of `apply_snap_snapshot()`

- File: `bins/node/src/node/fork_recovery.rs` (at function entry, line ~474)
- Check: same AtomicBool check as D2
- Blocks snap sync CONSUMPTION (receiving snapshots from potentially forked peers)
- Does NOT affect snap sync SERVING (which reads from in-memory state via separate code path)
- Converged: Subtractionist, Restructurer, Failure Analyst (3/5)
- Radical Simplifier argued seeds don't consume snap sync — true normally, but F6 (snap quorum vulnerability) makes this defense-in-depth mandatory
- conf(0.85, converged)

### D4: Add `enterRecoveryMode` and `exitRecoveryMode` RPC methods

- File: `crates/rpc/src/methods/guardian.rs` (after resolving merge conflicts)
- Pattern: identical to `pauseProduction`/`resumeProduction` (same file, same auth)
- enterRecoveryMode: sets Arc<AtomicBool> to true, logs warning
- exitRecoveryMode: sets Arc<AtomicBool> to false, logs info
- Both return success/failure JSON response
- conf(0.9, unanimous) — all 5 evaluators agree on 2 new RPC methods

### D5: Add both RPC methods to ADMIN_METHODS

- File: `crates/rpc/src/server.rs`, line 31-37
- Add `"enterRecoveryMode"` and `"exitRecoveryMode"` to the existing array
- Reuses existing auth: loopback/private IP = no token needed, public IP = bearer token required
- conf(0.9, unanimous)

### D6: Add `recovery_mode: Arc<AtomicBool>` to RpcContext

- File: `crates/rpc/src/methods/context.rs`
- New field: `pub recovery_mode: Arc<AtomicBool>`
- New builder: `pub fn with_recovery_mode(mut self, rm: Arc<AtomicBool>) -> Self`
- Wire up in `startup.rs` (same pattern as `.with_sync_manager()`)
- Expose in `getGuardianStatus` response: `"recovery_mode": true/false`
- conf(0.85, converged) — Restructurer, Subtractionist, Radical Simplifier all specify this

### D7: Resolve merge conflicts in guardian.rs (prerequisite)

- File: `crates/rpc/src/methods/guardian.rs`, lines 83-126 and 201-214
- Pick HEAD versions (path traversal prevention, numeric sort by height)
- From commit `9699f511`
- Identified by: Subtractionist, Restructurer, Pattern Matcher, Failure Analyst, Radical Simplifier (5/5)
- This is a prerequisite, not a design choice — the file does not compile without it
- conf(0.95, observed)

## Recommended Changes (Medium Convergence)

These changes have conf(0.6-0.8, converged) and address verified failure modes.

### R1: Add `--recovery-mode` CLI flag

- Files: `bins/node/src/config.rs` (resolve merge conflicts first), `bins/node/src/node/init.rs`
- Sets `recovery_mode` to `true` during `Node::new()`, BEFORE `start_network()` is called
- Eliminates F5 race window (P2P starts before RPC — verified at startup.rs lines 21 vs 128)
- Does NOT require restart-to-exit: pair with RPC `exitRecoveryMode` for runtime toggle
- ~10 lines across 2 files
- Source: Failure Analyst (P5, conf 0.60). No other evaluator proposed it, but none contradicted it.
- Failure mode filter: Resolves F5 completely
- conf(0.65, inferred + F5 evidence)

### R2: Clear block caches on exitRecoveryMode

- In the exitRecoveryMode handler, also clear:
  - `fork_block_cache` (block_handling.rs:176-190) — may contain cached fork blocks
  - `rejected_fork_tips` (network_events.rs:136-150) — stale fork tip rejections
  - Sync pipeline buffers via `sync_manager.clear_pending_blocks()` (if available)
- Prevents F10: cached fork blocks applied after exiting recovery
- ~5 lines in the exit handler
- Source: Failure Analyst (F10). Subtractionist flagged `fork_block_cache` as cross-signal.
- conf(0.70, inferred)

### R3: Resolve merge conflicts in config.rs

- File: `bins/node/src/config.rs`, lines 73-87 and 146-151
- Identified by: Radical Simplifier
- Prerequisite for R1 (--recovery-mode CLI flag)
- conf(0.80, observed — merge conflicts block changes)

## Options for User Decision

These are divergent additions where evaluators proposed different approaches. The user decides which to include.

### Option A: Outbound sync suppression during recovery

**Source**: Restructurer (gap analysis)
**What**: Check recovery_mode in the sync engine's `next_request()` to suppress outbound GetHeaders/GetBlocks requests while in recovery mode.
**Evidence**: Without this, the sync engine requests blocks -> responses arrive -> rejected at apply_block -> engine requests more -> wasted CPU and bandwidth cycle.
**Complexity cost**: +1 check in sync engine, ~5 lines
**Failure modes**: No new failure modes. Reduces wasted I/O.
**vs. Radical floor**: +5 lines above minimum
**Confidence**: conf(0.55, inferred)

### Option B: Periodic log warning for long-running recovery mode

**Source**: Failure Analyst (F4)
**What**: In periodic.rs, log a WARN every 5 minutes if recovery_mode has been active.
**Evidence**: F4 — operator forgets to exit recovery mode; seed serves stale snapshots indefinitely.
**Complexity cost**: +5-10 lines in periodic.rs
**Failure modes**: Resolves F4 (forgotten recovery). No new failure modes.
**vs. Radical floor**: +10 lines above minimum
**Confidence**: conf(0.50, inferred)

### Option C: RecoveryPhase::ManualQuarantine enum variant (INSTEAD of AtomicBool)

**Source**: Pattern Matcher (P3)
**What**: Extend existing RecoveryPhase enum with ManualQuarantine variant instead of adding a separate AtomicBool.
**Evidence**: Team already chose enum over scattered booleans for recovery state (types.rs:374-394). Compiler enforces exhaustive match.
**Complexity cost**: +1 enum variant, forces 4+ match site updates, adds semantic dependency
**Failure modes**: Semantic mismatch (enum represents automatic recovery phases; ManualQuarantine is externally triggered). Adds lock contention (RecoveryPhase is behind RwLock on SyncManager — same deadlock risk as Contradiction #2).
**vs. Radical floor**: Significantly more complex than AtomicBool
**Confidence**: conf(0.50, inferred) — pattern purity vs practical simplicity tradeoff
**Synthesis note**: The definite changes (D1-D6) use AtomicBool. This option would REPLACE D1-D6 with a fundamentally different approach. It is not additive.

### Option D: Seed recovery script (M5)

**Source**: Subtractionist (P3), Requirements analysis (M5)
**What**: Single bash script orchestrating seed recovery: query checkpoints -> stop seeds -> restore best checkpoint -> delete peers.cache -> start with --recovery-mode -> verify convergence -> exit recovery mode.
**Evidence**: Current procedure is ~20 manual SSH commands with critical unprotected window. Script automates and adds anti-poisoning step. All building blocks exist except the recovery RPC methods (D4).
**Complexity cost**: +1 script (~150 lines bash), +0 Rust code beyond definite changes
**Failure modes**: F8 (SSH timeout — must verify each step), F9 (concurrent runs — should detect). Must delete peers.cache after restore (Radical Simplifier evidence: peers.cache at service/mod.rs:281-296 redials forked peers).
**vs. Radical floor**: Separate deliverable, does not affect core code complexity
**Confidence**: conf(0.60, observed)

### Option E: UTXO self-heal on startup (M8)

**Source**: Requirements analysis (M8)
**What**: Port INC-I-027 fix — detect utxo_store vs state_db length mismatch on startup, auto-rebuild.
**Evidence**: After checkpoint restore, utxo_store may contain stale data from pre-restore state. Current init code at init.rs:177 checks `!store.is_empty()` and trusts existing data.
**Complexity cost**: ~50 lines in init.rs (mismatch detection + rebuild trigger)
**Failure modes**: F7 (rebuild from corrupt state_db could amplify error)
**Subtraction alternative**: Recovery script deletes `utxo_store/` during restore; existing init migration handles rebuild. This is 1 line in script vs ~50 lines in Rust. Script approach only covers scripted recovery; code approach covers manual restores too.
**vs. Radical floor**: Orthogonal to anti-poisoning core (all 5 evaluators agree these are independent concerns)
**Confidence**: conf(0.50, inferred)

## Constraints (from Failure Analyst)

These failure modes MUST be addressed by any chosen path.

| ID | Constraint | Source | Addressed By |
|----|-----------|--------|-------------|
| F1 | Snap sync SERVING must continue during recovery mode | All evaluators | D2+D3 (gate at application functions, not serving functions) |
| F2 | ALL state-modification paths must be gated | 5/5 evaluators | D2 (apply_block) + D3 (apply_snap_snapshot) |
| F3 | Recovery mode must be non-persistent | All evaluators | D1 (AtomicBool, cleared on restart) |
| F5 | P2P-before-RPC startup race | Failure Analyst | R1 (--recovery-mode CLI flag) |
| F5a | Production broadcast of non-applied block | Failure Analyst | Seeds don't produce (moot for primary use case). Producers: check recovery_mode in try_produce_block() |
| F10 | Cached fork blocks applied after exit | Failure Analyst | R2 (clear caches on exit) |
| C_prereq | Merge conflicts in guardian.rs must be resolved | 5/5 evaluators | D7 (prerequisite) |
| C_peer | peers.cache must be deleted during checkpoint restore | Radical Simplifier | Option D (recovery script) |

### Unmitigated Failure Modes (Operational, Not Code)

| ID | Description | Mitigation |
|----|------------|-----------|
| F4 | Operator forgets to exit recovery mode | Option B (periodic warning) + non-persistence (restart clears) |
| F6 | Checkpoint taken after fork started | Recovery script queries health.json for last healthy checkpoint |
| F6a | health.json says "healthy" but all peers on wrong chain | UNFIXABLE by automation. Operator must verify checkpoint hash against external reference. Document in runbook. |
| F7 | UTXO self-heal from corrupt state_db | Verify rebuilt state against stored state root (if implementing Option E) |
| F8 | SSH timeout during multi-server recovery | Script must verify each step, abort on failure (Option D) |
| F11 | Recovery mode entered WITHOUT checkpoint restore | Operational procedure enforcement. Document that recovery_mode is quarantine, not repair. |

## Architecture Maps

### Current Architecture (Block Intake)

```
ENTRY POINTS                    ROUTING                      APPLICATION
============                    =======                      ===========

1. Gossip block                 on_new_block_event()         handle_new_block()
   event_loop.rs:282            [7 pre-filters]                apply_block()    <-- NO GATE

2. Sync response                on_sync_response()           handle_new_block()
   event_loop.rs:308            sync_mgr.handle_response()     apply_block()    <-- NO GATE

3. Periodic sync blocks         get_blocks_to_apply()        apply_block()      <-- NO GATE
   periodic.rs:160              [BYPASSES handle_new_block]    (DIRECT)

4. Snap sync (via sync resp)    take_snap_snapshot()         apply_snap_snapshot() <-- NO GATE
   network_events.rs:370                                       (replaces state)

5. Snap sync (via periodic)     take_snap_snapshot()         apply_snap_snapshot() <-- NO GATE
   periodic.rs:175                                             (replaces state)

6. Reorg                        execute_reorg()              apply_block() in loop <-- NO GATE
   block_handling.rs:224                                       (via handle_new_block)

7. Fork cache chain             try_apply_cached_chain()     apply_block()      <-- NO GATE
   block_handling.rs:279                                       (via handle_new_block)

SNAP SYNC SERVING (SEPARATE PATH — READS STATE, NEVER WRITES):
   handle_sync_request_bg()     reads chain_state/utxo/producer  --> responds to peer
   event_loop.rs:394            [NO apply_block involvement]
```

### Proposed Architecture (Definite + Recommended)

```
ENTRY POINTS                    ROUTING                      APPLICATION
============                    =======                      ===========

1-7. (all paths unchanged)      (unchanged)                  apply_block()
                                                               |
                                                               +-> [RECOVERY GATE] if recovery_mode.load() -> return Ok(())
                                                               |   (D2 — AtomicBool check, ~1ns)
                                                               +-> normal block application

4-5. Snap sync                  (unchanged)                  apply_snap_snapshot()
                                                               |
                                                               +-> [RECOVERY GATE] if recovery_mode.load() -> return Ok(())
                                                               |   (D3 — same AtomicBool)
                                                               +-> normal snap application

SNAP SYNC SERVING (UNCHANGED — continues during recovery):
   handle_sync_request_bg()     reads state -> responds       [NOT GATED — correct]

RPC INTERFACE:
   enterRecoveryMode  -----> recovery_mode.store(true)   [D4, restricted to ADMIN_METHODS]
   exitRecoveryMode   -----> recovery_mode.store(false)  [D4, clears caches per R2]
                              + clear fork_block_cache
                              + clear rejected_fork_tips
   getGuardianStatus  -----> includes recovery_mode field [D6]

STARTUP (with R1):
   Node::new(config)  -----> if --recovery-mode: recovery_mode.store(true)
   start_network()           P2P active, but apply_block already gated
   start_rpc()               RPC available, operator can exitRecoveryMode
   run_event_loop()          normal operation (blocks dropped while gated)
```

### Ownership Model

```
Node                                    RpcContext
====                                    ==========
recovery_mode: Arc<AtomicBool> -------> recovery_mode: Arc<AtomicBool>
         |                                       |
         v                                       v
   apply_block() checks                  enterRecoveryMode sets
   apply_snap_snapshot() checks          exitRecoveryMode clears
                                         getGuardianStatus reads
```

## Migration Path

### Phase 0: Prerequisites (no functional change)

1. Resolve merge conflicts in `guardian.rs` (D7) — lines 83-126 and 201-214, pick HEAD versions
2. Resolve merge conflicts in `config.rs` (R3) — lines 73-87 and 146-151
3. Verify compilation: `cargo build --release`

### Phase 1: Core Anti-Poisoning Gate (D1-D6)

1. Add `recovery_mode: Arc<AtomicBool>` to Node struct (D1) and initialize in Node::new()
2. Add recovery_mode check at top of `apply_block()` (D2) — before any state mutation
3. Add recovery_mode check at top of `apply_snap_snapshot()` (D3)
4. Add `recovery_mode: Arc<AtomicBool>` to RpcContext (D6) with builder method
5. Wire up in startup.rs: `.with_recovery_mode(self.recovery_mode.clone())`
6. Add `enterRecoveryMode` and `exitRecoveryMode` RPC methods in guardian.rs (D4)
7. Add dispatch entries in dispatch.rs
8. Add both to ADMIN_METHODS array (D5)
9. Expose in getGuardianStatus response
10. Test: set recovery mode, submit block via gossip -> dropped. Set recovery mode, trigger snap sync -> dropped. Read RPCs still work. Snap sync serving still works.

### Phase 2: Hardening (R1-R2)

1. Add `--recovery-mode` CLI flag (R1) — sets AtomicBool before start_network()
2. Add cache clearing to exitRecoveryMode (R2) — clear fork_block_cache, rejected_fork_tips
3. Test: start with --recovery-mode, verify blocks dropped from first network event. Exit via RPC, verify caches cleared.

### Phase 3: Operational Tooling (Options D, E — user decides)

1. Seed recovery script (Option D) — bash orchestration using D4 RPC methods
2. UTXO self-heal port (Option E) — if chosen, OR handle in script by deleting utxo_store/

Each phase is independently deployable. Phase 1 provides the core safety property. Phase 2 hardens edge cases. Phase 3 is operational convenience.

## Complexity Comparison

| Metric | Current | Radical Minimum | Proposed (D+R) | Full Brief (M1-M8, S1-S4) |
|--------|---------|----------------|-----------------|---------------------------|
| Anti-poisoning gates | 0 | 1 (apply_block only) | 2 (apply_block + apply_snap) | 2 |
| New fields (Node) | 0 | 1 | 1 | 1 |
| New fields (RpcContext) | 0 | 1 | 1 | 1 |
| New RPC methods | 0 | 2 | 2 | 2+ |
| New CLI flags | 0 | 0 | 1 | 1 |
| Check sites | 0 | 1 | 2 | 2 |
| Lines of Rust changed | 0 | ~38 | ~60 | ~200+ |
| New scripts | 0 | 0 | 0 | 3+ |
| New files | 0 | 0 | 0 | 3+ |
| Failure modes open | F1-F12 | F2,F5,F10 | F4,F6a (operational only) | F6a (operational only) |

The proposed architecture (Definite + Recommended) adds ~60 lines of Rust to close the anti-poisoning gap. The radical minimum (~38 lines) leaves 3 failure modes open. The full brief scope (~200+ Rust + 3+ scripts) provides operational convenience at 3x the code cost with identical safety properties to the proposed architecture.

## Milestones

This redesign touches 4+ files, so milestones are defined per architect.md rules.

| Milestone | Scope | Deliverable | Verification |
|-----------|-------|-------------|-------------|
| M0 | Prerequisite cleanup | Resolve merge conflicts in guardian.rs and config.rs | `cargo build --release` compiles |
| M1 | Core gate | D1-D6: AtomicBool, 2 check sites, 2 RPC methods, RpcContext wiring | Test: enterRecoveryMode -> blocks dropped, snap serving works |
| M2 | Hardening | R1-R2: CLI flag, cache clearing | Test: --recovery-mode startup, cache state after exit |
| M3 (optional) | Tooling | Option D: seed recovery script | Test: script runs end-to-end on testnet |

## Design Synthesis Quality Gate

```
--- DESIGN SYNTHESIS QUALITY GATE ---
Evaluators completed:           5/5
Deletion convergence items:     6 (A1 disproved 5/5, P0 killed 2/5, SyncManager-RwLock killed 2/5,
                                   circuit breaker killed 1/5, network disconnect killed 1/5,
                                   Pattern Matcher Node-kill invalidated by code evidence)
Restructuring convergence:      2 (Arc<AtomicBool> 4/5, terminal function check 3.5/5)
Addition options presented:     5 (CLI flag, cache clearing, sync suppression,
                                   periodic warning, recovery script)
Failure modes identified:       12 (from Failure Analyst: F1-F12)
Failure modes applied as filters: 12/12
Radical floor gap:              0 gates -> 1 gate (radical) -> 2 gates (proposed)
                                0 lines -> 38 lines (radical) -> 60 lines (proposed)
Contradictions found:           4 (flag placement, SyncManager scope,
                                   1 vs 2 check sites, CLI flag necessity)
Contradictions resolved:        4/4
Evidence independence verified: YES (all convergence clusters use independent evidence sources)
-------------------------------------
```
