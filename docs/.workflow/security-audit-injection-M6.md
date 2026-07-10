# Security Audit Report: Injection & Input Validation (INC-I-139 M6)

## Attack Perspective
Peer-controllable input reaching the history-wiping SnapSync admission surface via the M6 refactor of `crates/network/src/sync/manager/`. Specifically: can a lying/withholding/Sybil peer inject `best_height` (→ gap), peer count, empty-header counts, or a target hash that — through the re-homed gap comparators (`gap > SNAP_SYNC_GAP_MIN`), the new `local_height == 0` discv5-grace gate, or the `enable_snap_sync()` (10→50) emergency re-enable — pushes a node into SnapSync (or suppresses a needed guard) that pre-M6 code would have rejected?

## Method
Compared committed HEAD (pre-M6) to the uncommitted working tree (M6) via `git diff`; traced every changed comparator and the demoted `snap.threshold` to its live consumers; verified the single-authority snap-admission setter; killed false positives by confirming reachability.

## What I Don't Understand
1. Whether `SnapSyncState::discv5_peer_grace_deadline` is ever armed for an `h > 0` node (I confirmed it is only *read/cleared* in `decision.rs`; I did not exhaustively locate every arm site). This bounds how much the RC-1c `h==0` gate actually changes runtime behavior, but does not change the injection verdict (the gate only *removes* a wait, never admits snap).
2. The exact recovery-coordinator wiring downstream of `signal_stuck_fork()` / `needs_genesis_resync()` in `periodic.rs`/`node.rs` — out of the 5-file scope; I verified the *setter* is single-authority and gated, which is sufficient for this perspective.

## Attack Surface Map
| Entry Point | Data Source | Trust Level | Flows To | Dangerous Operation |
|-------------|------------|-------------|----------|---------------------|
| peer `best_height` / `network_tip_height` | lying peer | untrusted | `gap` in decision.rs:161/177/208, cleanup.rs:488/492, prod_gate.rs:814/822 | SnapSync admission (state wipe) |
| `consecutive_empty_headers` | header-withholding peer | untrusted | prod_gate.rs:788 (`is_deep_fork_detected`) | deep-fork → emergency snap |
| peer count (`self.peers.len()`) | Sybil | untrusted | `enough_peers` guards | snap quorum gates |
| `--no-snap-sync` operator flag → `snap.threshold==u64::MAX` | operator | trusted | prod_gate.rs:732/740 sentinel + `enable_snap_sync()` | emergency re-enable |

## Findings

### SEC-INJECTION-001: 50→500 comparator change lands in dead function `is_deep_fork_detected` (latent, would EXPAND emergency-snap window if re-wired) — P3 — conf(0.7, observed)
- **Location:** `crates/network/src/sync/manager/production_gate.rs:787-854` (changed line 822)
- **Vulnerability Class:** CWE-561 (Dead Code) / latent CWE-670 (Always-Incorrect Control Flow if reactivated)
- **Data Flow:** `peer best_height → gap` and `peer withholds headers → consecutive_empty_headers` → `is_deep_fork_detected()` gap gate at line 822.
- **Evidence:** `is_deep_fork_detected()` has **no production caller** — repo-wide search for `is_deep_fork_detected()` returns only the definition (prod_gate.rs:787) and two doc-comment mentions (`block_lifecycle.rs:729`, `prod_gate.rs:40`); no `.is_deep_fork_detected()` invocation exists in `crates/` or `bins/`. The M6 line changed the early-exit gate from `gap > self.snap.threshold` (default 50, `types.rs:468`) to `gap > SNAP_SYNC_GAP_MIN` (500, `recovery.rs:216`). Because the early-exit *prevents* deep-fork detection, raising 50→500 **expands** the detectable-deep-fork window from gaps `(12, 50]` to `(12, 500]`. If this function is ever re-wired to `needs_genesis_resync`/emergency snap, a header-withholding peer (`consecutive_empty_headers ≥ 10`) plus a `best_height` yielding gap 51–500 plus one close peer would trigger emergency snap where pre-M6 it returned `false`.
- **False Positive Check:** Searched for any live call (`.is_deep_fork_detected()`) across the whole tree — none. Therefore the change has **zero runtime effect today**; this is not a live vulnerability. It is a latent semantic drift buried in dead code, contradicting the brief's "pure re-home / bit-for-bit" framing for this one site (50 ≠ 500).
- **Impact:** None at runtime (dead code). Future risk: a re-wiring inherits an expanded emergency-snap window for medium gaps without an accompanying activation-height/review gate.
- **Remediation:** Either delete `is_deep_fork_detected()` (it is unreferenced), or add a `#[cfg(test)]`/`#[allow(dead_code)]` marker with a comment that its 500 gate is intentional and must stay consistent with the live large-gap route before any re-wiring. Add a compile-time `#[deny(dead_code)]` note or a test asserting non-reachability.

### SEC-INJECTION-002: RC-2 `enable_snap_sync()` 10→50 change is genuinely inert — NO FINDING (verification record) — P3 — conf(0.7, observed)
- **Location:** `block_lifecycle.rs:503-508` (new method), consumed at `production_gate.rs:750`.
- **Verification:** Enumerated every read of `snap.threshold` in the 5-file scope + `sync_engine/`: `decision.rs:163` (`< u64::MAX` sentinel), `production_gate.rs:630/732/740` (`== u64::MAX` sentinel), `block_lifecycle.rs:261` (log-format arg only), `block_lifecycle.rs:497/508` (writes). **No read uses the numeric value as a gap floor.** Both 10 and 50 satisfy `< u64::MAX`, so admission behavior is identical. `enable_snap_sync()` has exactly one production caller (`production_gate.rs:750`), reachable only when `snap.threshold==u64::MAX && is_emergency` (the three evidence-guarded emergency `RecoveryReason`s at lines 672-677). The RC-2 "bit-for-bit inert" claim **holds**.
- **Note:** Residual behavior (identical pre- and post-M6): after an emergency recovery on a `--no-snap-sync` node, `snap.threshold` is left `< u64::MAX` (enabled) — it is not restored to the disabled sentinel. This is pre-existing, not introduced by M6, and not attacker-triggerable (requires an emergency evidence-gate to already fire).

## Static Analysis Patterns
| Pattern | Files Matched | Risk | Notes |
|---------|--------------|------|-------|
| `gap > self.snap.threshold` (pre-M6) → `gap > SNAP_SYNC_GAP_MIN` | decision.rs:177/208, cleanup.rs:492, prod_gate.rs:822 | P3 | 50→500: all sites inert or **more restrictive**; only prod_gate.rs:822 is dead code (SEC-INJECTION-001) |
| `self.snap.threshold` numeric read (gap floor) | none live (only sentinel `==/<`  + 1 log arg) | clean | confirms RC-1 demotion complete |
| `needs_genesis_resync = true` (snap-admission setter) | prod_gate.rs:773 only | clean | single-authority; M6 adds no new setter; all 5 gates intact |
| `.is_deep_fork_detected()` invocation | **0** | P3 | dead function (SEC-INJECTION-001) |
| `enable_snap_sync()` invocation | prod_gate.rs:750 only | clean | emergency-only, value inert |

## Verdict on the Key Question
**No new attacker-reachable SnapSync path is opened by M6.** Directional analysis of every changed comparator:
- `decision.rs:177` (fresh-node wait) — `h==0`-only WAIT gate; 50→500 is *more* restrictive; no `h>0` reach.
- `decision.rs:208` (discv5 grace) — RC-1c **adds** an `h==0` guard; for `h>0` it now *skips the wait* and proceeds to header-first (does **not** admit snap; `should_snap` at line 164 still requires `local_height==0 || needs_genesis_resync`). Net: closes an `h>0` stall.
- `cleanup.rs:492` (snap-attempt reset) — 50→500 more restrictive; only resets `snap.attempts`, does not admit snap.
- `production_gate.rs:822` — dead code, zero runtime effect (SEC-INJECTION-001).
- `enable_snap_sync()` 10→50 — verifiably inert (SEC-INJECTION-002).

For an `h>0` node, snap still requires `needs_genesis_resync=true`, set *only* at `production_gate.rs:773` behind Gates 1–5 (floor, no-concurrent, rate-limit, availability, attempt-limit), none of which M6 loosens. INV-SYNC-011 (no snap for gaps <50 without corroborated deep-fork evidence) and INV-SYNC-009 (rate governor) are not weakened by M6.

## Cross-Perspective Signals
- **(DoS / logic auditor)** RC-1c removes the discv5 peer-discovery grace *wait* for `h>0` nodes (`decision.rs:204`). Such nodes now proceed to header-first sync immediately instead of parking. This slightly increases outbound sync-request pressure timing — worth confirming the INC-I-120 outbound rate governor (INV-SYNC-009, in `next_request`/`sync_engine`) still gates it, since that governor is a *separate* mechanism from this grace. I saw no governor change in the M6 diff, so this is likely mitigated, but it is outside my injection lane.
- **(Availability auditor)** `cleanup.rs:492` and the fresh-node waits raising 50→500 mean gaps in `[51,500]` no longer trigger snap-retry / fresh-node snap-wait. Consistent with the new "snap only for gap≥500" design, but a legitimately-stuck node at gap 51–499 now relies entirely on header-first + the evidence-gated emergency funnel. Liveness, not injection.

## Gaps
- Did not exhaustively trace every arm site of `discv5_peer_grace_deadline` (bounds the *magnitude* of RC-1c's effect, not its safety direction).
- Downstream recovery-coordinator consumption of `needs_genesis_resync` (`periodic.rs`, `node.rs`) is out of the 5-file scope; verified only that the *setter* is single-authority and fully gated.
- Did not runtime-reproduce; all findings are `observed` (static data-flow traced), none `measured`. Per auditor ceiling, confidence capped at 0.7.

## Summary
- P0: 0
- P1: 0
- P2: 0
- P3: 2 (one latent dead-code semantic drift; one inert-change verification record)
