# Security Audit Report: Cryptography & Data/State Protection (INC-I-139 M6)

## Attack Perspective
Snapshot-admission integrity for the M6 sync-refactor. SnapSync is history-wiping: it
discards local finalized chain state and rebuilds from a peer-served snapshot. I hunted
for whether M6's two changes — (a) re-homing the four `gap > self.snap.threshold`
comparators to `thresholds::SNAP_SYNC_GAP_MIN` (=500) + the `local_height==0` discv5 gate
(RC-1b/1c), and (b) replacing the emergency `snap.threshold = 10` with
`enable_snap_sync()` (=50) (RC-2) — weakens any integrity check protecting finalized state:
the `consensus_target_hash()` majority guard, the `confirmed_height_floor` monotonic guard,
or the finality guard (INC-I-081 lineage). Central question: can a peer-controllable input
now push a node into accepting a forged/minority snapshot it previously would have rejected?

## What I Don't Understand
1. `consensus_target_hash()` requires a plurality pair with `count >= 2` peers, NOT a strict
   >50% majority. Whether the higher-layer peer-admission/scoring guarantees honest peers
   dominate the peer table (making plurality ≈ majority) is outside the M6 diff and outside
   my traced scope — this is a pre-existing property, not an M6 change.
2. The exact conditions under which `needs_genesis_resync` is externally set for an h>0 node
   beyond the RecoveryReason funnel — I traced the funnel (`request_genesis_resync`) but did
   not exhaustively enumerate every writer of `fork.needs_genesis_resync`.

## Attack Surface Map
| Entry Point | Data Source | Trust Level | Flows To | Dangerous Operation |
|-------------|------------|-------------|----------|---------------------|
| peer `best_height` | lying peer | untrusted | `decision.rs:161` gap → wait gates | (gates a wait, not admission) |
| peer `best_height`/`best_hash` | lying peers | untrusted | `decision.rs:236 consensus_target_hash()` | SnapSync target selection (majority-guarded) |
| empty header responses | withholding peer | untrusted | `cleanup.rs:445 gap>12` → `request_genesis_resync` | floor-bypassing emergency snap |
| `RecoveryReason` (emergency) | internal classifier | semi | `production_gate.rs:692-751` Gates 1-5 | floor bypass + snap enable |
| peer `best_height`/`best_hash` | lying peers | untrusted | `production_gate.rs:822 is_deep_fork_detected` | **DEAD CODE — no caller** |

## Findings

### SEC-CRYPTO-M6-001: `is_deep_fork_detected()` comparator re-home is a semantically-inverted change inside dead code — P3 — conf(0.6, observed)
- **Location:** `crates/network/src/sync/manager/production_gate.rs:819-830` (function `is_deep_fork_detected`, lines 787-~850)
- **Vulnerability Class:** CWE-561 (dead code) / latent logic inconsistency
- **Data Flow:** peer `best_height` → `gap` → `if enough_peers && gap > SNAP_SYNC_GAP_MIN { return false }` → (would gate whether a mid-gap stall is classified deep-fork → emergency floor-bypassing snap). **No live caller: the funnel is never invoked.**
- **Evidence:** Repo-wide `grep -rn "is_deep_fork_detected()"` returns only the definition, doc-comments, and tests — zero call sites. Of the four re-homed comparators, this is the only one whose semantic direction *inverts*: `gap > threshold → return "not deep fork"`. Raising the constant 50→500 *widens* the gap band eligible for deep-fork emergency escalation from `(12,50]` to `(12,500]` — the exact "gap<500 reaching SnapSync" recurrence class INC-I-139 targets. Because the function is unreachable, this has **zero runtime effect today**. The live deep-fork emergency path is `cleanup.rs:445` (`gap > 12`, `AllPeersBlacklistedDeepFork`), which M6 did NOT touch.
- **False Positive Check:** Searched entire repo for callers (network + node crates, excluding tests/comments) — none. Confirmed the M6 diff does not add a caller. Confirmed the live emergency path (cleanup.rs:453) is unchanged by the diff. Therefore not attacker-reachable → not an active vulnerability.
- **Impact:** None at runtime. Latent: if a future change re-wires `is_deep_fork_detected()` into the funnel, the widened `(12,500]` window would let a Sybil-majority attacker force a floor-bypassing emergency snap across a 10x-wider gap range than pre-M6.
- **Remediation:** Either delete `is_deep_fork_detected()` (dead) or, if retained for future use, keep its "snap can handle" short-circuit at `MINOR_FORK_GAP_MAX(50)` rather than `SNAP_SYNC_GAP_MIN(500)` so the deep-fork window matches INV-SYNC-011's protected zone. Add a `#[cfg(test)]`-only or `#[allow(dead_code)]` note documenting it is currently unreferenced.

## Verification of the Core Claims (NOT findings — audit passed)

**"10→50 emergency-enable is bit-for-bit inert" — VERIFIED TRUE, conf(0.7, observed).**
Every remaining *functional* read of `snap.threshold` is a sentinel comparison:
`decision.rs:163` (`< u64::MAX`), `production_gate.rs:630/732/740` (`== u64::MAX`). Both 10 and
50 satisfy `< u64::MAX`, so "enabled" is the only observable effect. `dispatch.rs` previously
read `snap.threshold.min(10)` — `50.min(10)==10` and `10.min(10)==10`, so it was already
insensitive to 10-vs-50; M6 replaces it with literal `+10` (identical). No live code reads the
magnitude as a gap floor. Claim holds across all reachable inputs.

**Integrity guards on the LIVE snap-admission path — UNCHANGED by M6, conf(0.7, observed):**
- `consensus_target_hash()` (`decision.rs:44-70`, invoked at `:236`): groups peers by
  `(height, hash)`, picks the plurality pair, requires `count >= 2` to prevent single-peer
  poisoning; returns `None` (→ header-first fallback) if no agreement. `git diff` shows M6
  touches 0 lines of this function. It sits on the execution path for **every** snap
  (`should_snap` at `decision.rs:230` → `consensus_target_hash`). A minority/forged snapshot
  is rejected exactly as before.
- `confirmed_height_floor` monotonic guard (Gate 1, `production_gate.rs:692`): bypass set
  (`is_emergency ∪ is_forward_large_gap`) is unchanged by M6; the floor logic in
  `block_lifecycle.rs:74-79/165-166/287-292/718-724` is untouched (M6 only *added* the
  `enable_snap_sync()` fn).
- Finality guard (`recovery.rs:325-336/364-373` FINALITY_GUARD on ShallowRollback,
  INV-SYNC-001/INC-I-081): entirely outside the M6 diff.

**`should_snap` does not read the gap (conf 0.7, observed):** the actual trigger
(`decision.rs:164-167`) is `enough_peers && attempts<3 && snap_allowed &&
(local_height==0 || needs_genesis_resync)`. The four re-homed comparators only gate *wait
loops* (177/208) or a *retry-counter reset* (cleanup 492) or dead code (822) — none is the
admission decision. RC-1c's `local_height==0` gate makes an h>0 node do header-first instead
of parking; since `should_snap` for an h>0 node still requires `needs_genesis_resync`, this
is strictly *less* snapping (integrity-safer).

## Static Analysis Patterns
| Pattern | Files Matched | Risk | Notes |
|---------|--------------|------|-------|
| `snap\.threshold` functional reads | decision.rs:163, production_gate.rs:630/732/740 | clean | all sentinel `==`/`< u64::MAX`; magnitude never read |
| `gap > .*SNAP_SYNC_GAP_MIN` | decision.rs:177/208, production_gate.rs:822, cleanup.rs:492 | P3 | 3 are wait/retry/dead; only :822 inverts and is dead |
| `is_deep_fork_detected()` callers | none (repo-wide) | P3 | dead code |
| `consensus_target_hash` M6 diff | 0 lines | clean | majority guard untouched |
| `confirmed_height_floor` M6 diff | 0 lines (floor logic) | clean | Gate 1 + block_lifecycle untouched |

## Cross-Perspective Signals
- **For the logic/auth auditor (INC-I-081 Sybil resistance, pre-existing, NOT M6):**
  `consensus_target_hash()` accepts a plurality pair with only `count >= 2` peers, not a
  strict >50% majority. If honest peers fragment across many `(height,hash)` pairs during an
  active fork while an attacker presents 2+ coordinated Sybil peers on one forged pair, the
  forged pair can win the plurality. This is the ultimate guard against forged-snapshot
  admission and it predates M6 — worth an independent look at whether "plurality-of-2" is
  strong enough given the peer-table admission model.
- **For the config auditor:** `SnapSyncState::new()` default `threshold: 50` and the named
  `SNAP_SYNC_GAP_MIN=500` now diverge in meaning; a future contributor could reasonably
  assume the default *is* the gap floor. The dead-semantics comment mitigates this but the
  divergence is a config-clarity smell.

## Gaps
- Did not runtime-reproduce (static trace only; conf capped at observed).
- Did not exhaustively enumerate every writer of `fork.needs_genesis_resync` outside
  `request_genesis_resync` — a second writer bypassing Gates 1-5 would change the analysis,
  but none was found in the M6 diff.
- Peer-table admission/scoring (which determines whether plurality ≈ majority) is upstream of
  this diff and outside crypto scope.

## Summary
- P0: 0 findings
- P1: 0 findings
- P2: 0 findings
- P3: 1 finding (SEC-CRYPTO-M6-001, dead-code latent inconsistency)

**Bottom line:** M6 does NOT weaken snapshot-admission integrity. The two guards protecting
finalized state on the live snap path — `consensus_target_hash()` plurality
(`decision.rs:236`) and the `confirmed_height_floor` monotonic guard
(`production_gate.rs:692`) — are byte-for-byte untouched, and the finality guard is outside
the diff. The 10→50 emergency-enable is genuinely inert. The only concerning re-home
(`is_deep_fork_detected`, which inverts direction and widens the emergency window) sits in
dead code with no runtime effect.
