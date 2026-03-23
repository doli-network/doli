# Architecture Reasoning Trace: Sync Recovery Redesign

*Generated 2026-03-23, RUN_ID=54, scope: sync-recovery*
*Prior trace (syncmanager sub-struct extraction): preserved in git history*

---

## Context

INC-I-005 sync cascade: 9 targeted fixes over 10 sessions. Cascade persists because the architecture generates new cascade entry points faster than point fixes close them. The Architect evaluation (docs/redesigns/sync-architecture-evaluation.md) identified 5 structural flaws (F1-F5). The Analyst (docs/redesigns/sync-cascade-redesign-analysis.md) scoped the redesign with MoSCoW requirements (REQ-SYNC-100 through 111).

## Design Space Gate

### Forced Prediction

Predicted: The simplest intervention is NOT a new struct (RecoveryCoordinator). Instead, a gated method on SyncManager extending the existing `signal_stuck_fork()` pattern.

Result: CONFIRMED. Reading the codebase revealed that:
1. `signal_stuck_fork()` already implements the gated-method pattern (checks recovery_phase before mutating)
2. RecoveryCoordinator would need 6+ fields from SyncManager, creating ownership friction
3. The prior architecture spec's ProductionGate sub-struct was designed but NOT implemented, suggesting the extraction pattern has friction in practice

### Anti-Overengineering Gate

Q0 (Subtraction): YES. The 9 `needs_genesis_resync = true` write sites are 9 uncoordinated decision points. Routing them through 1 method effectively SUBTRACTS 8 decision points. The code at each write site stays, but the AUTHORITY to trigger a destructive reset is removed.

Q1 (Need): YES. 4 incidents, 9 fix attempts, recurring cascade.

Q2 (Scale): YES. 12-60 node network. Already causing production failures.

Q3 (Simplest): The gated method adds ~110 lines and 0 new files/structs. The RecoveryCoordinator struct alternative would add ~200 lines plus a new struct definition. The event queue alternative would require rewriting cleanup() (~543 lines).

## DD-1: Recovery Coordination Mechanism

### Explorer Phase 1

Generated 6 alternatives ordered by simplicity:
1. Do nothing -- conf(0.10, observed)
2. Delete needs_genesis_resync -- conf(0.25, inferred)
3. Single gated method -- conf(0.70, inferred)
4. RecoveryCoordinator struct -- conf(0.65, assumed)
5. Event queue -- conf(0.30, assumed)
6. Transition guards + gated method (refinement of 3) -- conf(0.75, inferred)

### Skeptic Phase 1

Eliminated: Alt 1 (9 iterations proved ineffective), Alt 2 (breaks onboarding), Alt 5 (scope explosion)
Weakened: Alt 4 (ownership friction in Rust)
Survived: Alt 3 and Alt 6

### Explorer Phase 2 (refinement)

Alt 3 and Alt 6 converged: "gated method" IS "transition guards + gated method" -- they're the same approach at different granularity. Merged into a single design with:
- `request_genesis_resync()` for the recovery gate
- `is_valid_transition()` for the transition guard
- Floor extension for monotonic progress

### Skeptic Phase 2

Applied complexity attack: "Is this more complex than it needs to be?"
- RecoveryReason enum: 20 lines. Needed for diagnostics. Keep.
- is_valid_transition: 30 lines. Needed for F1. Keep.
- request_genesis_resync: 50 lines with 5 gates. All 5 gates correspond to real cascade paths. Keep.
- Floor extensions: ~10 lines across 2 files. Closes Loops B and C. Keep.

Applied evidence attack:
- `signal_stuck_fork()` pattern proven in codebase (observed)
- `confirmed_height_floor` was the most effective fix (observed)
- Sub-struct extraction didn't prevent cascade (observed) -- structural not organizational problem

### Analogist Phase

Pattern match: This is the "Gatekeeper" pattern. A single function that receives all requests for a destructive action and decides whether to honor each one. Common in rate limiters, circuit breakers, and admission controllers. The existing `signal_stuck_fork()` is a partial implementation.

Anti-pattern check: NOT "God method" -- request_genesis_resync() has a single responsibility (decide whether to allow resync). NOT "Feature envy" -- it accesses fields it owns via `self`.

### Final Decision

Winner: Gated method approach. conf(0.78, observed). 110 lines, 0 new files, 0 new structs.

## DD-2: State Transition Validation

### Explorer (abbreviated -- simpler decision)

3 alternatives: match-based check in set_state(), const transition table, type-state pattern.

### Skeptic

Type-state eliminated (incompatible with struct field). Const table slightly more complex than match for same result.

### Decision

Winner: Match-based check. conf(0.78, observed). 30 lines in existing method.

## DD-3: Monotonic Progress Extension

### Explorer (abbreviated)

2 viable alternatives: extend floor to specific paths (3 sites), single guard method.

### Skeptic

Guard method is over-abstraction for 1-line check. Specific sites are clearer.

### Decision

Winner: Extend floor to specific paths. conf(0.80, observed). ~10 lines across 2 files.

## Key Insight

The evaluation recommended a RecoveryCoordinator struct. After reading the codebase, the struct approach creates problems that don't exist with a simple method:

1. Ownership: RecoveryCoordinator can't borrow SyncManager fields while being mutably borrowed itself
2. State duplication: Coordinator needs copies of 6+ fields that change every tick
3. Ceremony: Every call requires building a context parameter

The method approach avoids ALL of these because it IS SyncManager. `self.confirmed_height_floor`, `self.snap.attempts`, `self.consecutive_resync_count` are all available directly.

The lesson: in Rust, "extract coordinator" is often the wrong pattern for single-owner, single-threaded systems. A gated method achieves the same coordination with less friction.
