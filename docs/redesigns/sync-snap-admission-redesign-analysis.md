# Redesign Analysis: SnapSync Admission Consolidation (INC-I-139)

**Workflow**: `/omega-redesign --incident=INC-I-139 --scope=sync-snap-admission` (proposal-only, no `--fix`)
**Author**: Analyst · **Date**: 2026-07-09 · **RUN_ID**: 454
**Upstream input**: `docs/.workflow/domain-diagnosis-report.md` (INC-I-139 VERDICT, conf 0.95 converged)
**Role note**: This document is PROBLEM SCOPING, not solution design. It defines what a correct redesign must satisfy and hands acceptance criteria + a verified capability baseline to the evaluators. It proposes no code.

---

## Scope

Consensus-adjacent sync/recovery machinery that can admit a node into history-wiping SnapSync:

- `crates/network/src/sync/manager/sync_engine/decision.rs` — `start_sync()` / `should_snap` (the execution chokepoint)
- `crates/network/src/sync/manager/sync_engine/dispatch.rs` — empty-headers escalation + counter reset
- `crates/network/src/sync/manager/recovery.rs` — `RecoveryCoordinator::classify()` (Rule 1b / Rule 2 / Rule 4)
- `crates/network/src/sync/manager/production_gate.rs` — `request_genesis_resync()` (the existing 5-gate central funnel)
- `crates/network/src/sync/manager/cleanup.rs` — stuck-sync / blacklist / height-offset resync triggers
- `crates/network/src/sync/manager/block_lifecycle.rs` — apply-failure resync trigger
- `crates/network/src/sync/manager/types.rs` — `SnapSyncState` + `threshold: 50`
- `bins/node/src/node/block_handling.rs` — FORK_GUARD lower-slot tiebreak (lines 168–203) + finality interplay
- `bins/node/src/node/periodic.rs` — coordinator action consumer (`RecoveryAction::SnapSync/GenesisResync` → `request_genesis_resync`)
- `bins/node/src/node/fork_recovery.rs` — fork recovery integration (blast-radius dependent)

Out of scope (WON'T, see below): snapshot format, snap transfer protocol, consensus rules, snapshot install/backfill.

## Summary (plain language)

Four separate incidents (INC-I-005, INC-I-033, INC-I-138, INC-I-139) are the same bug wearing different clothes: a node hits a tiny, recoverable disagreement (a one-slot block race, a restart), gets stuck for a few minutes, and then — instead of nudging itself back onto the main chain — throws away its entire block history and re-downloads a state snapshot. Each incident patched the one door the node walked through that time. INC-I-139 walked through a different door (a "you're 51 blocks behind, just snap" shortcut) that nobody had guarded. The redesign's job is to stop building doors and start building **one hallway**: every path that wants to snap must pass through a single guarded checkpoint, and no minor fork may pass it. The default posture is **subtraction** — the codebase already contains a half-finished version of exactly this (`request_genesis_resync`), so the work is largely about routing the last ungated path through the checkpoint that already exists, not adding a new subsystem.

---

## Capability Inventory (verified baseline — PRIOR-KNOWLEDGE-GATE)

No "missing guard" claim below is made without a file:line. This is the enumerated ground truth the class-kill property must be proven against.

### A. The single execution chokepoint (1)

There is exactly **one** state transition that actually starts a snap:

| # | Transition | Site | Guard |
|---|-----------|------|-------|
| X1 | `set_syncing(SnapPhase::SnapCollecting, SyncPipelineData::SnapCollecting{...})` | `decision.rs:267–276` (trigger `"start_snap_sync"`) | `should_snap` (decision.rs:164–169) |

Every other `SnapCollecting` reference is a read/continuation of an already-admitted snap: `snap_sync.rs:114/183/234` (SnapCollecting→SnapDownloading→SnapReady phase advance), `dispatch.rs:259/287` (`next_snap_requests` reads votes/asked), `cleanup.rs:175/206` (timeout continuation to SnapDownloading), `types.rs`/`mod.rs` (enum/label). **Confirmed: X1 is the sole admission execution point.**

### B. The guard predicate `should_snap` has 3 independent OR-terms (decision.rs:164–169)

```
should_snap = enough_peers(>=3) && snap.attempts<3 && snap_allowed(threshold<u64::MAX)
              && ( local_height == 0            // Route C — bootstrap
                 || gap > snap.threshold        // Route A — BARE GAP, UNGATED
                 || fork.needs_genesis_resync ) // Route B — flag, gated by request_genesis_resync
```

- **Route A (`gap > snap.threshold`, threshold=50)** — UNGATED. No fork evidence, no rate limit, no floor check beyond `snap.attempts<3`. **This is the N1 INC-I-139 path (E7/E8).** Not named by INV-SYNC-011.
- **Route B (`needs_genesis_resync`)** — set ONLY by `request_genesis_resync()` (production_gate.rs:660, the 5-gate funnel). This is the consolidated path.
- **Route C (`local_height == 0`)** — legitimate fresh-node bootstrap (plus the fresh-node peer-wait loop decision.rs:175–224).

### C. The existing central funnel `request_genesis_resync()` (production_gate.rs:660–754)

Sets `fork.needs_genesis_resync = true` (→ Route B) only after 5 gates:
1. Monotonic progress floor (`confirmed_height_floor`) — **bypassed for emergency reasons**
2. No concurrent recovery (`ResyncInProgress`)
3. Rate limit (`consecutive_resync_count >= MAX_CONSECUTIVE_RESYNCS`)
4. Snap availability (`threshold==u64::MAX` = `--no-snap-sync`) — **bypassed for emergency, and re-enables snap at `threshold=10`** (production_gate.rs:722–730)
5. Snap attempt limit (`snap.attempts >= 3`)

Emergency reasons that bypass gates 1 & 4: `GenesisFallbackEmptyHeaders`, `AllPeersBlacklistedDeepFork`, `ApplyFailuresSnapThreshold`.

The docstring at production_gate.rs:649–658 and block_lifecycle.rs:253–257 explicitly state this method was built (INC-I-005) to **"replace 9 scattered `needs_genesis_resync = true` assignments with a single decision point."** This is the smoking gun: the single-chokepoint design already exists — but only for Route B.

### D. The 7 sites that feed Route B (all already funnel through the 1 gate in C)

| # | Site | RecoveryReason | Emergency bypass? |
|---|------|----------------|-------------------|
| B1 | `periodic.rs:727` | `CoordinatorSnapEscalation` (from `classify()` Rule 2, recovery.rs:410) | no |
| B2 | `periodic.rs:731` | `CoordinatorGenesisEscalation` (from `classify()` Rule 4, recovery.rs:434) | no |
| B3 | `dispatch.rs:157` | `GenesisFallbackEmptyHeaders` (empty≥10, gap≥50 fallthrough) | YES |
| B4 | `block_lifecycle.rs:264` | `ApplyFailuresSnapThreshold` (3+ apply fails) | YES |
| B5 | `cleanup.rs:452` | `AllPeersBlacklistedDeepFork` | YES |
| B6 | `cleanup.rs:618` | `StuckSyncLargeGap` (gap>1000) | no |
| B7 | `cleanup.rs:680` | `HeightOffsetDetected` | no |

### E. The 1 redirect into the ungated Route A

| # | Site | Mechanism |
|---|------|-----------|
| A1 | `dispatch.rs:96–116` | Deep-fork empty-headers (≥10, gap>threshold, `confirmed_height_floor>0`): sets `snap.attempts=0`, resets `consecutive_empty_headers=0`, `set_state(Idle)`, calls `start_sync()` → re-enters Route A. Now partially fenced for gap<50 by the INC-I-138 minor-fork guard at dispatch.rs:144, but it re-enters **Route A**, not the gate. |

### F. Guards inside the coordinator's own `classify()` Rule 2 (recovery.rs:387–411)

`RecoveryAction::SnapSync` returned only if `(rollback_exhausted || large_gap || deep_fork_confirmed) && snap_attempts<3 && peer_count>=3`, where:
- `large_gap = gap >= SNAP_SYNC_GAP_MIN(500)`
- `deep_fork_confirmed = deep_fork>0 || (empty_count>=10 && last_applied>=STALE_TIP_SECS(300) && gap>=MINOR_FORK_GAP_MAX(50))` ← INC-I-138 D4 hardening
- `rollback_exhausted = minor_fork_evidence && shallow_rollback_count>=SHALLOW_ROLLBACK_MAX(10)`

These are the guards INC-I-138 added — but they gate B1 only, NOT Route A.

### G. Legitimate SnapSync consumers (must be preserved by any redesign)

| Consumer | Site | Legitimate trigger |
|----------|------|--------------------|
| Fresh-node bootstrap | `decision.rs:167` (`local_height==0`) + fresh-peer wait `decision.rs:175–224` | Route C |
| Genuine far-behind catch-up (≥500) | coordinator `large_gap` recovery.rs:389 → B1; also cleanup.rs:618 gap>1000 → B6 | Route B |
| Operator control | `--no-snap-sync` sets `snap.threshold=u64::MAX` (block_lifecycle.rs:496–497 `disable_snap_sync`; init.rs:695) | disables all routes; emergency re-enables at threshold=10 |

**Note (open question OQ-3):** No explicit "operator-forced resync" RPC/CLI was found in scope — snap is only toggled via `--no-snap-sync`/threshold. If a forced-resync capability is intended as a legitimate consumer, it does not currently exist as a distinct path.

### Entry-point count (honest answer)

**1 execution chokepoint (X1)**, admitted by a **3-condition guard** of which **1 condition (Route A bare-gap) is ungated**; the gated condition (Route B) is fed by a **5-gate funnel** already consolidating **7 call sites**; plus **1 redirect (A1)** that re-enters the ungated condition. The recurrence class exists because the funnel (C) does not sit AT the chokepoint (X1) — it guards only one of X1's three admission conditions.

### Zero-margin constant coupling

`MINOR_FORK_GAP_MAX = 50` (recovery.rs:212) **==** `SnapSyncState.threshold = 50` (types.rs:468). The "minor fork, roll back" ceiling and the "snap" floor are the same number. A parked node auto-promotes to snap the instant its gap ticks past 50 (N1 fired at gap=51 via Route A; N4's deep_fork_confirmed requires gap≥50). Zero margin.

---

## Architecture Context (MANDATORY)

### Module Boundaries

- **`decision.rs` (SyncEngine::start_sync)** — decides header-first vs snap; owns the execution chokepoint X1. Depends on: `SnapSyncState`, peer table, `fork.needs_genesis_resync`. Depended by: `periodic.rs` (drives it each tick), `dispatch.rs:115` (redirect A1).
- **`recovery.rs` (RecoveryCoordinator::classify)** — pure decision function returning `RecoveryAction`; no side effects. Depends on: `RecoveryContext` (gap, finality, evidence, counters). Depended by: `periodic.rs:694` (`classify_and_dispatch`).
- **`production_gate.rs` (request_genesis_resync)** — the central funnel setting Route B's flag. Depends on: floor, recovery_phase, resync count, snap state. Depended by: B1–B7.
- **`dispatch.rs` (next_request)** — per-tick request emitter + empty-headers escalation; owns counter `consecutive_empty_headers` and one of its resets (dispatch.rs:84). Depended by: `periodic.rs:755`.
- **`block_handling.rs` (FORK_GUARD)** — gossip block classification + lower-slot tiebreak; owns `signal_stuck_fork()` emission. Depends on: block store height lookups. Depended by: sync coordinator (consumes StuckFork evidence).
- **`types.rs`** — `SnapSyncState`, thresholds, `ForkState::fork_action`.

### Data Flows Through Affected Area

1. **Gossip block → FORK_GUARD → wedge**: block_handling.rs:168–203 classifies a same-height competitor; if `is_better` (lower slot) → `signal_stuck_fork()` but **drops the better block with no re-fetch/re-apply** (N5 escaped only via a separate ATTEST_FETCH path). → node held on minority branch.
2. **Periodic tick → classify → dispatch**: periodic.rs:692 `classify_and_dispatch` → `RecoveryAction` → {ShallowRollback | HeaderFirstSync | SnapSync→`request_genesis_resync` | GenesisResync→`request_genesis_resync`}.
3. **start_sync → chokepoint**: periodic.rs:738 → `next_request`/`start_sync` → `should_snap` → X1.
4. **Counter lifecycle**: `consecutive_empty_headers` incremented on empty header responses; reset by block apply (block_lifecycle.rs:68), by F1 post-snap height-fallback (**dispatch.rs:84** — the INC-I-139 starvation writer), and formerly by periodic HeaderFirstSync (removed by INC-I-138 D2).

### Architectural Constraints & Invariants (must be preserved)

- **INV-SYNC-011 (id 56)** — "SnapSync must be unreachable for gaps below MINOR_FORK_GAP_MAX(50) via the empty-headers evidence branch AND via the dispatch genesis-fallback path; `consecutive_empty_headers` may only be reset by genuine progress." **INCOMPLETE — must be EXTENDED** to cover Route A (bare-gap) and the dispatch.rs:84 reset writer.
- **INV-SYNC-010 (id 19, revised)** — snap target must respect prior finality; a node with finalized h=N must reject a snap target whose chain does not include canonical h=N+1. (Constrains any change that alters snap target selection.)
- **INV-SYNC-014 (id 48)** — post-snap `self.utxo_set` must use the state_db-backed backend. (Constrains snapshot install; out of redesign scope but must not regress.)
- **INV-SYNC-001/004/008 (G2)** — never roll back below finality (recovery.rs:364–376). The wedge-escape requirement (REQ-SNAP-004) must NOT weaken this.
- **INV-FORK-001** — heaviest/attestation-weight chain is canonical; the FORK_GUARD lower-slot tiebreak must be subordinate to attestation weight.
- **Snap-attempt limiter** (`snap.attempts<3`) and **rate limiter** (`MAX_CONSECUTIVE_RESYNCS`) must remain enforceable on every admission route.

### Blast Radius (graph + manual)

Graphify could not be provisioned in this pass (`blast.py`/`graphify` not run — see OQ-4); blast radius assembled manually from the code graph citations already in the diagnosis report ([E11] `SyncManager::start_sync`, [E12] `RecoveryCoordinator::classify`) plus direct grep of call sites.

- **Direct impact**: `decision.rs::start_sync` (X1 guard), `production_gate.rs::request_genesis_resync`, `recovery.rs::classify` (Rule 1b/2), `dispatch.rs::next_request` (A1 + counter reset), `types.rs` thresholds.
- **Indirect (consumes output of the above)**:
  - `periodic.rs:692–760` — drives classify + start_sync every tick; any signature/semantic change to `RecoveryAction` or `request_genesis_resync` return contract ripples here.
  - `snap_sync.rs` — phase machine downstream of X1 (SnapCollecting→Downloading→Ready); unaffected if X1's *admission* changes but its *transition target* does not.
  - `block_lifecycle.rs`, `cleanup.rs` — B4–B7 callers; behavior changes if the funnel contract changes.
  - `fork_recovery.rs` (bins/node) — integration tests + reorg paths; consume rollback/recovery outcomes.
  - `block_handling.rs` FORK_GUARD — coupled via `signal_stuck_fork()` and the wedge-escape requirement.
  - Production gate (`is_production_paused` / snap-in-progress) — a node in SnapCollecting is production-gated; changing admission frequency changes production availability.

### Brittleness Check (bugfix workflows only)

```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 4/5
Details:
  [1] Cross-module blast radius — YES: a correct class-kill touches decision.rs + recovery.rs + dispatch.rs + production_gate.rs + block_handling.rs (5 modules, no single shared dependency owns "snap admission").
  [2] Invariant gaps — YES: INV-SYNC-011 does not cover Route A; no module enforces "no minor fork may snap on ANY path" as a single checkable predicate.
  [3] Data flow reversal — NO: the fix does not require reversing flow direction.
  [4] Shared mutable state — YES: consecutive_empty_headers / snap.attempts / needs_genesis_resync are read/written across dispatch, periodic, cleanup, block_lifecycle, decision with no single owner (INC-I-138 D2 and INC-I-139 both are counter-reset defects).
  [5] Contract absence — YES: the "decision to snap" has no explicit interface; it is an emergent OR-predicate (should_snap) plus a partial funnel (request_genesis_resync), relying on convention that all paths set the flag — a convention Route A violates.
Verdict: BRITTLE (4/5)
━━━━━━━━━━━━━━━━━━━━━━━━━
```

This BRITTLE verdict corroborates the diagnosis report's DEEP routing and the 4th-occurrence Shape-Recurrence: the problem is architectural (fragmented admission with no single contract), not a single code line. It supports the redesign direction (single guarded chokepoint) over a 5th round of per-path guards.

---

## Impact Analysis

### Existing Code Affected

- `decision.rs:164–169` (`should_snap`) — **Risk: HIGH.** The guard predicate is the chokepoint's gate; any change to its OR-terms changes admission for ALL nodes including legitimate bootstrap/catch-up.
- `production_gate.rs:660–754` (`request_genesis_resync`) — **Risk: MEDIUM.** Already the intended funnel; routing Route A through it must preserve emergency-bypass and rate-limit semantics.
- `recovery.rs:387–411` (Rule 2) + `343–378` (Rule 1b finality guard) — **Risk: HIGH (consensus-adjacent).** The wedge-escape requirement touches finality-guard interplay; changing it risks INV-SYNC-001/004/008.
- `block_handling.rs:168–203` (FORK_GUARD tiebreak) — **Risk: HIGH (fork-choice).** Re-evaluating the dropped better block is fork-choice behavior → run the 3-question consensus-shape checklist (see REQ-SNAP-006).
- `dispatch.rs:83–84` (counter reset) — **Risk: MEDIUM.** Second writer of the INC-I-138 starvation class.
- `types.rs:468` (threshold=50) — **Risk: MEDIUM.** Margin change; parameter-only band-aids are explicitly WON'T, but a margin constant may be part of a structural fix.

### What Breaks If This Changes (and mitigation)

- **Fresh-node bootstrap** (Route C) if the chokepoint guard is tightened without carving out `local_height==0` → new nodes can't snap → stuck at h=0. **Mitigation:** REQ-SNAP-005 behavior-preservation criterion + explicit bootstrap test.
- **Genuine ≥500 catch-up** if Route A is removed without a replacement large-gap admission → a node truly 10k blocks behind falls back to header-first (hours). **Mitigation:** the large-gap path already exists in the coordinator (recovery.rs:389, gap≥500) and cleanup.rs:618 (gap>1000) → Route B; verify it still reaches X1.
- **Emergency recovery** (apply-failure divergence, all-peers-blacklisted deep fork) if the emergency bypass is lost → a genuinely diverged node can't recover. **Mitigation:** REQ-SNAP-003 preserves emergency reasons.
- **Rolling deploy** — mainnet has ~external producers, NO synchronized stop-all. A mixed fleet where some nodes route Route A through the gate and others don't could snap-diverge during the window. **Mitigation:** REQ-SNAP-007 rolling-deploy verdict + consensus-shape checklist on the FORK_GUARD change.

### Regression Risk Areas

- **Finality guard** (recovery.rs:364–376) — the untested branch `finality == local_tip == fork-tip` (N4's 764s trap) must gain a test and must not be weakened.
- **Counter starvation** — any counter-reset change must be validated against INC-I-138 (D2) AND INC-I-139 (dispatch.rs:84) simultaneously.
- **Snap phase machine** (snap_sync.rs) — must be untouched; admission change must not alter SnapCollecting→Downloading→Ready.

---

## Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|---------------------|
| REQ-SNAP-001 | Single guarded admission chokepoint: every path that transitions a node into SnapCollecting (X1) passes through ONE guard predicate; no admission condition bypasses it | Must | - [ ] Enumeration proof: exactly one code path sets `SyncPipelineData::SnapCollecting`, and its guard is a single named predicate<br>- [ ] Route A (bare `gap>threshold`) no longer admits snap without passing the same guard as Route B<br>- [ ] All 7 Route-B feeders + the A1 redirect resolve to the same guard |
| REQ-SNAP-002 | Class-kill: no minor fork/stall (gap < SNAP_SYNC_GAP_MIN=500) may reach SnapSync without corroborated deep-fork evidence, on ANY path, provable by enumeration | Must | - [ ] For every admission route, `gap < 500` ⇒ requires deep-fork evidence (deep_fork signal OR empty_count≥10+stale) OR rollback-exhausted; bare gap alone never admits<br>- [ ] Reproduce INC-I-139 (self-fork at finalized height, 5-producer set): node does NOT snap at gap 50/51<br>- [ ] Reproduce INC-I-138 (gap=28): no snap<br>- [ ] Test asserts `SNAP_TRIGGER count == 0` for the minor-fork scenario |
| REQ-SNAP-003 | Behavior preservation for legitimate snap consumers | Must | - [ ] Fresh-node bootstrap (`local_height==0`) still snaps (Route C intact)<br>- [ ] Genuine gap≥500 catch-up still snaps<br>- [ ] Emergency reasons (ApplyFailuresSnapThreshold, GenesisFallbackEmptyHeaders, AllPeersBlacklistedDeepFork) still recover<br>- [ ] `--no-snap-sync` still disables snap on every route |
| REQ-SNAP-004 | Wedge-escape: the finality-guard/fork-guard deadlock resolves WITHOUT snap | Must | - [ ] Test models `finality == local_tip == fork-tip` (N4's 764s trap) and asserts convergence to canonical without SnapSync<br>- [ ] Dropped fork-choice competitor (FORK_GUARD lower-slot tiebreak) is re-evaluated against attestation weight rather than permanently dropped<br>- [ ] Finality invariant INV-SYNC-001/004/008 (never roll back below finality) is NOT weakened |
| REQ-SNAP-005 | No consensus-visible change without activation-height assessment; rolling-deploy safety verdict | Must | - [ ] 3-question consensus-shape checklist answered for any FORK_GUARD / fork-choice tiebreak change (see REQ-SNAP-006)<br>- [ ] Explicit rolling-deploy verdict (safe / needs-AH / needs-synchronized-deploy) given that mainnet has external producers and NO stop-all<br>- [ ] If block content or acceptance changes: activation height in NetworkParams + deploy-before-height plan |
| REQ-SNAP-006 | Fork-choice consistency: FORK_GUARD lower-slot tiebreak subordinate to attestation-weight canonical | Should | - [ ] A same-height competitor dropped by the lower-slot tiebreak is retained/re-fetched and re-applied once attestation weight favors it (the ATTEST_FETCH path that saved N5)<br>- [ ] Consensus-shape Q1/Q2/Q3 documented: (Q1) can a user tx trigger it? (Q2) producer/attestation pattern? (Q3) bit-identical for all reachable inputs? — with activation-height decision if any answer forces it |
| REQ-SNAP-007 | Evidence pipeline immune to counter-reset starvation | Should | - [ ] `consecutive_empty_headers` reset ONLY on genuine progress; the dispatch.rs:84 reset (INC-I-139 writer) and the periodic reset class (INC-I-138 D2) are both closed<br>- [ ] Stuck-fork evidence reaches the coordinator before gap grows to the snap floor<br>- [ ] Single owner/writer contract for the evidence counters documented |
| REQ-SNAP-008 | Real margin between minor-fork ceiling and snap floor | Should | - [ ] `MINOR_FORK_GAP_MAX(50)` and the snap admission floor are no longer the same number (non-zero margin), OR the margin is made structurally irrelevant by REQ-SNAP-002's evidence requirement<br>- [ ] A node parked at exactly the minor-fork ceiling does not auto-promote to snap |
| REQ-SNAP-009 | Non-foreclosure: future escalation tiers addable without new snap-admission points | Should | - [ ] Design admits new recovery tiers (e.g., header-range backfill, targeted block fetch) as inputs to the single guard, not as new transitions into SnapCollecting<br>- [ ] Does not foreclose INC-I-115's open fresh-genesis recovery needs (fresh nodes with no finality still bootstrap) |
| REQ-SNAP-010 | Extend INV-SYNC-011 to name every admission route | Must | - [ ] INV-SYNC-011 statement updated to cover Route A (decision.rs bare-gap), its `local_height==0` and `needs_genesis_resync` sub-branches, and the dispatch.rs:84 reset writer<br>- [ ] Linked regression tests registered in `v_regression_map` |
| REQ-SNAP-011 | Observability of admission decisions | Could | - [ ] Every snap admission (and every refusal by the guard) emits a structured log/metric with the deciding route + gap + evidence, sufficient to grep-verify "SNAP_TRIGGER count" on the live fleet |
| REQ-SNAP-012 | No changes to snapshot format, snap transfer protocol, or consensus rules; no parameter-only band-aid | Won't | N/A (explicitly excluded — see Out of Scope) |

### Decompose-to-Atoms note (REQ-SNAP-002, the class-kill)

The class-kill is not a single predicate; it is the conjunction of atomic guarantees, one per enumerated admission route:
- Route A (bare gap) ⇒ must require evidence OR gap≥500.
- Route B feeders B1/B2 (coordinator) ⇒ already gated by Rule 2 evidence — verify unchanged.
- Route B feeders B3/B4/B5 (emergency) ⇒ must retain bypass but only for genuine divergence (apply-fail / deep-fork), never bare gap.
- Route B feeders B6/B7 (cleanup large-gap / height-offset) ⇒ B6 is gap>1000 (legitimate); B7 height-offset must carry evidence.
- Redirect A1 (dispatch deep-fork) ⇒ must route to the guard, not re-enter Route A.
The class is killed only if ALL atoms hold; a proof that one route is guarded is not a proof of the class.

---

## Acceptance Criteria (detailed — key requirements)

### REQ-SNAP-002: Class-kill by enumeration
- [ ] Given a 5-producer network and a single-slot block race at a finalized height, when a node is held on the minority branch and its gap crawls to 50/51, then it converges to canonical WITHOUT any SnapCollecting transition (asserted by log absence + state-root convergence).
- [ ] Given `gap = 28` with empty-header evidence (INC-I-138 replay), when classify/dispatch run, then no admission route reaches X1.
- [ ] Given `gap = 600` (genuine catch-up), when start_sync runs, then X1 IS reached (behavior preserved).
- [ ] Edge: `gap = 499` with no evidence ⇒ no snap; `gap = 500` with no evidence ⇒ decision documented (this boundary is a design choice for the evaluators, flagged OQ-1).

### REQ-SNAP-004: Wedge-escape
- [ ] Given `finality == local_tip == fork-tip` and FORK_GUARD simultaneously dropping a same-height better-slot competitor, when the coordinator classifies, then it produces a convergent action (re-evaluate competitor / bounded reorg to canonical) — NOT `RecoveryAction::None` (the 764s park) and NOT SnapSync.
- [ ] Given the same state, the finality guard still refuses any rollback strictly below finality (INV preserved).

### REQ-SNAP-005 / REQ-SNAP-006: Consensus-shape + rolling deploy
- [ ] The 3-question checklist is answered in writing for the FORK_GUARD re-evaluation change.
- [ ] A rolling-deploy verdict is produced; if the FORK_GUARD change alters which block a node treats as canonical for any reachable input, an activation height is specified.

---

## Traceability Matrix

| Requirement ID | Priority | Incident Evidence | Test IDs | Architecture Section | Implementation Module |
|---------------|----------|-------------------|----------|---------------------|----------------------|
| REQ-SNAP-001 | Must | E8, E11; INC-I-005 "5 entry points" | class2, class6, class9; **M6 RC-2 taxonomy**: m6_rc2_emergency_reenable_admits_snap_under_no_snap_sync, m6_rc2_forward_large_gap_not_operator_disable_exempt, m6_rc2_rate_and_attempt_limits_apply_to_emergencies (tests_inc_i139.rs) | B, C, X1 | decision.rs / production_gate.rs |
| REQ-SNAP-002 | Must | E6, E7, E8 (N1 gap=51); INC-I-138 gap=28 | (test-writer) | B, F | decision.rs / recovery.rs |
| REQ-SNAP-003 | Must | G (legit consumers) | (test-writer) | G | decision.rs / production_gate.rs |
| REQ-SNAP-004 | Must | E4 (N4 764s finality trap) | (test-writer) | recovery.rs Rule 1b/2 | recovery.rs / block_handling.rs |
| REQ-SNAP-005 | Must | rolling-deploy constraint | (test-writer) | Constraints | NetworkParams / block_handling.rs |
| REQ-SNAP-006 | Should | E1, E2, E3 (FORK_GUARD; N5 escape) | (test-writer) | Data Flow 1 | block_handling.rs |
| REQ-SNAP-007 | Should | E5 (dispatch.rs:84 starvation) | (test-writer) | Data Flow 4 | dispatch.rs |
| REQ-SNAP-008 | Should | E8 (threshold==MINOR_FORK_GAP_MAX) | **M6 RC-1**: m6_h_gt_0_skips_discv5_grace_proceeds_header_first, m6_rc1b_no_gap_comparator_read_of_threshold_in_decision, m6_rc1_fresh_node_h0_still_waits, m6_rc1_exact_ceiling_gap_does_not_float_snap; **M6 RC-2 sentinel**: m6_rc2_emergency_reenable_restores_enabled_sentinel_not_magic_10 (tests_inc_i139.rs) | Zero-margin | decision.rs / production_gate.rs / block_lifecycle.rs (enable_snap_sync) |
| REQ-SNAP-009 | Should | Shape-Recurrence (4th) | (test-writer) | Non-foreclosure | decision.rs |
| REQ-SNAP-010 | Must | INV-SYNC-011 incomplete | (test-writer) | Constraints | invariants + regression tests |
| REQ-SNAP-011 | Could | fleet grep-verify | (test-writer) | Observability | logging/metrics |
| REQ-SNAP-012 | Won't | — | N/A | Out of scope | — |

---

## Specs Drift Detected

- **INV-SYNC-011 (memory.db invariant id 56)** — INCOMPLETE: names only the empty-headers `deep_fork_confirmed` branch and the dispatch genesis-fallback path; does NOT cover the `decision.rs:164–169` bare-gap Route A that fired on N1. Must be extended (REQ-SNAP-010). Flagged, not yet edited (edit belongs to the fix workflow, not this proposal-only pass).
- No `specs/*.md` file documents the snap-admission decision surface as a single contract — the behavior is emergent across 6 files. A redesign should produce/anchor such a spec section.

## Assumptions

| # | Assumption (technical) | Explanation (plain language) | Confirmed |
|---|------------------------|------------------------------|-----------|
| 1 | X1 (`decision.rs:267`) is the sole `SnapCollecting` transition | There is exactly one place the code actually starts a snap | Yes — grep-verified (Capability Inventory A) |
| 2 | `request_genesis_resync` was built (INC-I-005) as the intended single funnel but guards only Route B | The single-checkpoint idea already exists half-built in the code | Yes — docstring production_gate.rs:649–658, block_lifecycle.rs:253–257 |
| 3 | `MINOR_FORK_GAP_MAX == snap.threshold == 50` | The "roll back" ceiling and "snap" floor are the same number | Yes — recovery.rs:212, types.rs:468 |
| 4 | Coordinator Rule-2 evidence guards (INC-I-138) gate B1 only, not Route A | The last incident's fix protected one door, not the one N1 used | Yes — recovery.rs:401–411 vs decision.rs:168 |
| 5 | No explicit operator-forced-resync path exists (only `--no-snap-sync`) | There is no "force a snap now" button today | Yes — grep (Capability Inventory G, OQ-3) |
| 6 | Snapshot install/backfill (BLOCK 1 MISSING symptom) is out of redesign scope | Fixing the missing-history dashboard symptom is separate operational work | Yes — diagnosis report Routing "operational, not urgent" |

## Identified Risks

- **R1 — Fork-choice change is consensus-adjacent (REQ-SNAP-006).** Re-evaluating the FORK_GUARD-dropped block changes which block a node treats as canonical → must pass the consensus-shape checklist and may need an activation height. Mitigation: REQ-SNAP-005/006 gate it explicitly.
- **R2 — Rolling deploy on a mixed fleet.** External producers can't be stop-all'd; a half-deployed admission change could transiently diverge. Mitigation: rolling-deploy verdict is a Must (REQ-SNAP-005).
- **R3 — Over-tightening starves legitimate recovery.** If the single guard is too strict, a genuinely diverged node (apply failures) can't recover. Mitigation: REQ-SNAP-003 emergency-preservation + the emergency-bypass reasons must survive.
- **R4 — Regression on the two counter-reset writers.** Fixing dispatch.rs:84 without regressing INC-I-138 D2. Mitigation: REQ-SNAP-007 tests both simultaneously.
- **R5 — Solving admission but not the wedge.** If only admission is gated (REQ-SNAP-002) but the finality/fork-guard deadlock (REQ-SNAP-004) is left, nodes park forever instead of snapping — a different but still-bad outcome. Both are Must.

## What I Don't Understand (intellectual honesty)

1. **The height/slot bookkeeping (diagnosis "Unresolved a")** that let N4's FORK_GUARD label `de211caf` as `h=7221` post-rollback — I have not traced the post-rollback height reassignment logic; it may matter for REQ-SNAP-004's re-evaluation design.
2. **Propagation asymmetry (diagnosis "Unresolved b")** — why the slot-7222 block reached N1/N4/N5 but not the seed/majority. Out of admission scope but relevant to whether FORK_GUARD re-evaluation is sufficient.
3. Whether the **emergency-bypass re-enable at `threshold=10`** (production_gate.rs:729) is itself a latent minor-fork snap vector under `--no-snap-sync` — I did not exhaustively trace whether an emergency reason can be raised at a minor gap. Flagged OQ-2.

## Open Questions for User

- **OQ-1 (boundary policy):** For a genuine catch-up with NO fork evidence, what is the correct gap floor to admit snap — keep the current `>threshold` semantics (but raise the constant), or require `gap ≥ SNAP_SYNC_GAP_MIN(500)`? This sets the minor-fork/catch-up boundary and is a design choice the evaluators need pinned.
- **OQ-2 (emergency bypass under --no-snap-sync):** Is it acceptable that an emergency reason re-enables snap at `threshold=10` even when the operator set `--no-snap-sync`? Should the class-kill also constrain the emergency path's minimum gap?
- **OQ-3 (operator-forced resync):** No explicit force-snap RPC/CLI exists today. Is an operator-forced resync a required legitimate consumer the redesign must add a dedicated (guarded) path for, or is it out of scope?
- **OQ-4 (blast-radius verification):** Do you want the evaluators to run `blast.py`/`graphify` to machine-verify the blast radius before design, or is the manually-assembled radius (from the diagnosis report's graph citations) sufficient for the proposal stage?

## Out of Scope (Won't)

- Snapshot serialization format, snap-sync transfer protocol, GetStateSnapshot/GetStateRoot wire changes — untouched (would require chain reset).
- Consensus RULES (validation, scheduler, coinbase, bitfield) — untouched; only fork-choice tiebreak *subordination* is in scope, and only with the consensus-shape gate.
- Post-snap backfill / BLOCK 1 MISSING dashboard symptom — operational healing (backfill from seed archive), tracked separately.
- Parameter-only band-aid (merely raising `snap.threshold`) — explicitly rejected; margin (REQ-SNAP-008) may be *part* of a structural fix but is not a standalone solution.
