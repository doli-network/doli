# SnapSync Admission Architecture (INC-I-139)

**Status**: PROPOSAL-ONLY (pending User Gate) · **Date**: 2026-07-09 · **INC_ID**: INC-I-139
**Synthesis**: 5-evaluator convergence (Subtractionist, Restructurer, Pattern Matcher, Failure Analyst, Radical Simplifier)
**Input analysis**: `docs/redesigns/sync-snap-admission-redesign-analysis.md` · **Reasoning trace**: `docs/.workflow/architecture-reasoning.md`

## Problem Statement

Four incidents (INC-I-005, INC-I-033, INC-I-138, INC-I-139) are one recurrence class: SnapSync — a history-wiping recovery — is reachable from minor forks/stalls (gap < SNAP_SYNC_GAP_MIN=500) because admission guards are inconsistent across entry paths. Each incident patched one path; the next incident entered through another. The class-kill property: **no minor fork/stall may reach SnapSync without corroborated deep-fork evidence, on ANY path, provable by enumeration.** Chosen direction: consolidate admission into ONE guarded chokepoint. Posture: SUBTRACTION — the consolidating funnel (`request_genesis_resync`, production_gate.rs:660) already exists; the work is deleting the two paths that bypass it, plus one measured correctness fix to the funnel's floor gate that the bypass paths were silently compensating for. The rejected cheaper path (raise `snap.threshold` 50→500) is the WON'T-listed parameter band-aid (REQ-SNAP-012): it leaves the ungated door open at the new number.

━━━ RESOURCE COST — SUMMARY — COST-DECLARED ━━━
Dimensions:
  CPU:      -epsilon steady-state; Phase 2 adds one weight comparison per rare wedge event (inferred)
  Memory:   + tens of bytes for the Phase 2 evidence variant in the bounded 256-entry window; Phase 1 adds no state (measured)
  IO:       - fewer history wipes and full state_db rebuilds after spurious snaps (inferred)
  Network:  - each avoided minor-fork snap saves a multi-MB snapshot transfer; Phase 2 adds one bounded block re-fetch per wedge event (inferred)
  Disk:     - fewer history wipes; Phase 2 replaces snap-wipe with a single-block re-apply (observed)
  Latency:  + up to one 30s coordinator tick before a legitimate gap>=500 snap starts; -764s worst case on the wedge path once Phase 2 ships (measured)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: the smallest change set that makes snap admission provable-by-enumeration; the only cheaper-to-type path is the WON'T-listed threshold band-aid, and Phase 1 is net resource-NEGATIVE at runtime
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Evaluation Summary

| Evaluator | Lens | Top Proposal | Confidence | Key Finding |
|-----------|------|-------------|------------|-------------|
| Subtractionist | removal | Delete Route A + floor-gate companion | conf(0.7, measured) | Found the floor-gate contradiction: pure deletion strands floor>0 gap≥500 catch-up |
| Restructurer | boundaries | Delete Route A; coordinator owns wedge-escape | conf(0.7, observed) | Role collocation: dispatch writes fork evidence it should only read (dispatch.rs:84/113/126) |
| Pattern Matcher | patterns | Route bare-gap through funnel (INV-SYNC-009 precedent) | conf(0.70, observed) | Codebase already validated single-chokepoint consolidation (INC-I-120 RequestSync governor) |
| Failure Analyst | failures | Guaranteed-progress filter + floor-gate load-bearing proof | conf(0.7, measured) | Route A is simultaneously the bug AND the sole floor-blind forward-snap escape hatch |
| Radical Simplifier | minimal | Delete 2 bypass paths; classify() is already the single authority | conf(0.68, observed) | Minimum viable architecture already exists in code; 2 snap deciders → 1 by deletion |

## Convergence Matrix

```
                                          Subtr  Restr  Pattern  Failure  Radical   Verdict
Delete Route A (decision.rs:168)            Y      Y       Y      Y(cond)    Y      5/5 → DEFINITE
Delete/close A1 redirect (dispatch:96-117)  Y      Y*      Y*     Y(filt)    Y      4/5+filter → DEFINITE
Floor-gate companion (production_gate:674)  Y      -       -        Y        -      2/5 + CODE-VERIFIED → DEFINITE (measured)
Counter single-owner (dispatch.rs:84)       Y      Y       Y(con)   Y        Y(flag) 4/5 → DEFINITE (with co-test)
Wedge-escape needed (FORK_GUARD re-eval)    -      Y       Y        Y        Y      4/5 → RECOMMENDED (Phase 2, AH-gated)
Wedge-escape is consensus-adjacent → AH     -      Y       Y        Y        Y(inf)  4/5 checklist Q2=YES Q3=NO → AH REQUIRED
Demote snap.threshold to sentinel           Y      -       -        -        Y      2/5 → RECOMMENDED
SnapAdmission capability token              -      -       Y        -        -      1/5 → OPTION
Peer-relative snap-target corroboration     -      -       -        Y        -      1/5 → OPTION
* Restructurer/Patterns proposed rewiring A1 to the funnel; Subtractionist/Radical proposed deletion.
  Code shows the funnel fallthrough already exists at dispatch.rs:153-161 → deletion and rewiring are
  behaviorally identical; radical tiebreaker picks deletion.
```

Independence verified: all five evaluators read decision.rs/dispatch.rs/production_gate.rs directly (not via each other); the Route-A deletion convergence rests on independent reads of the same ground truth, with three distinct argument styles (elimination, boundary-move, pattern-precedent). See reasoning trace for the per-cluster independence check.

## Why Prior Fixes Failed (Recurrence Analysis)

No `.redesign_recurrence` flag was set at intake (this is the first formal redesign of this domain), but the 4-incident history is the load-bearing evidence and is analyzed here for the same purpose.

- **INC-I-005 (March)**: root cause recorded as "multi-entry-point feedback loop: 5 independent entry points into snap sync cascade." The fix BUILT the consolidation (`request_genesis_resync`, production_gate.rs:649-658: "replaces 9 scattered assignments with a single decision point") — but wired it to only ONE of `should_snap`'s three OR-terms (Route B). Route A (bare gap, decision.rs:168) never entered the funnel.
- **INC-I-033**: minority fork after restart → snap after ~10min DEEP_FORK loop. Patched the loop, not the admission surface.
- **INC-I-138 (Jul 7-8)**: hardened the coordinator's Rule 2 (`deep_fork_confirmed` gap guard, recovery.rs:390-404) and the dispatch minor-fork regime guard (dispatch.rs:130-152). Both guard the FUNNEL-fed route only — the hardening was silently bypassable via Route A, which INC-I-139's N1 used 24 hours later at gap=51.
- **INC-I-139 (Jul 9)**: N1 snapped via the bare-gap term; N4 snapped after a 764s finality-guard/fork-guard deadlock; evidence-counter starvation via dispatch.rs:84.

**The architectural shape all four attempts left in place**: admission authority split across two deciders (`classify()` at recovery.rs:270-438 and `should_snap` at decision.rs:164-169), with one decider containing an evidence-free term. Every prior fix guarded the funnel or its feeders; none removed the term that bypasses the funnel. This proposal eliminates that seam by deletion — after Phase 1 there is exactly one gap-based admission authority, and the enumeration proof is a single greppable predicate. The prior approach (per-incident localized guards) is registered as a failed-approach class in memory.db; this spec does not propose a 5th localized guard.

## Contradiction Resolutions (code-verified)

**CR-1 — Floor gate (decides the companion fix). RESOLVED: Subtractionist + Failure Analyst are correct; Restructurer + Radical kill tests were incomplete.** Verified chain: (1) every healthy synced node has `confirmed_height_floor > 0` — set on block apply when `Synchronized && consecutive_resync_count == 0` (block_lifecycle.rs:74-80); (2) a synced-then-behind node at gap≥500 reaches `classify()` Rule 2 `large_gap` (recovery.rs:389) → `RecoveryAction::SnapSync` (recovery.rs:410) → `request_genesis_resync(CoordinatorSnapEscalation)` (periodic.rs:725-728); (3) `CoordinatorSnapEscalation` is NOT in the emergency match (production_gate.rs:666-671 lists only GenesisFallbackEmptyHeaders, AllPeersBlacklistedDeepFork, ApplyFailuresSnapThreshold); (4) Gate 1 refuses: `confirmed_height_floor > 0 && !is_emergency → return false` (production_gate.rs:674-681). Same refusal applies to B6 `StuckSyncLargeGap` (cleanup.rs:618) and B7. **Therefore Route A is today the ONLY floor-blind forward-snap path for a previously-synced node, and P1 is NOT a pure deletion — it requires the floor-gate companion (DC-2).** Degradation without the companion is bounded, not fatal: catch-up falls to header-first, and if peers lack our tip, ≥10 empty headers force B3 (emergency, floor-exempt) — slow but not stranded. The companion makes gap≥500 catch-up first-class again.

**CR-2 — Sequencing (guaranteed-progress vs. AH-gated wedge-escape). RESOLVED: Phase 1 alone satisfies guaranteed-progress; the Failure Analyst's atomicity constraint holds only in its weak form.** The N4-style wedge (`finality == local_tip == fork-tip`) retains a reachable rank-4 snap exit after Phase 1, with the evidence pipeline STRENGTHENED, not starved. Proof chain: (1) wedge forms — FORK_GUARD drops the better-slot competitor (block_handling.rs:188-202), Rule 1b refuses rollback (recovery.rs:367-377); (2) empty headers accumulate — post-Phase-1 the only counter resets are genuine block apply (block_lifecycle.rs:68) and the bounded gap≤3 gossip-wait (dispatch.rs:126, inert once wedge gap exceeds 3); the starvation writers dispatch.rs:84 (unconditional request-time reset) and dispatch.rs:113 (A1) are closed by DC-3/DC-4; (3) the network advances ~6 blocks/min, so wedge gap crosses MINOR_FORK_GAP_MAX=50 in ~8 min; (4) at `consecutive_empty_headers ≥ 10 && gap ≥ 50` the dispatch escalation falls through the minor-fork guard (dispatch.rs:144-152, which only diverts gap 4..49) to `request_genesis_resync(GenesisFallbackEmptyHeaders)` (dispatch.rs:157) — an EMERGENCY reason that bypasses Gate 1 and Gate 4 (production_gate.rs:666-671, 682-688, 722-730) while respecting Gates 2/3/5; (5) the flag admits Route B in `should_snap` (decision.rs:169) → X1 (decision.rs:267-276). In parallel, `deep_fork_confirmed` (recovery.rs:401-404: empties≥10 + stale≥300s + gap≥50) → CoordinatorSnapEscalation, which with DC-2 also passes Gate 1. **Conclusion: Phase-1 tightening does NOT convert the wedge into a permanent stall (INC-I-012/115 class avoided); the wedge resolves exactly as N4 did (evidence-gated snap at gap≥50), no worse and more reliably (no counter starvation). Phase 2 then upgrades the exit from rank-4 (history wipe) to rank-1 (single-block re-evaluation) behind its activation height.** Ordering "Phase 1 first, Phase 2 separate/AH-gated" is therefore safe and correct.

**CR-3 — B7 `HeightOffsetDetected` (cleanup.rs:661-696). CLASSIFIED: narrow, floor-gated corruption detector — NOT a minor-fork snap hole, but a named residual in the enumeration.** It fires only when blocks ARE being applied (`last_block_applied < 30s`) while gap≥2 stays stable ±1 for >120s — the corrupted-height-counter signature. A minor-fork STALL cannot satisfy it (stalled ⇒ `blocks_recent == false` ⇒ tracker reset, cleanup.rs:693-696). It is non-emergency, so Gate 1 refuses it on every floor>0 node; reachable only on floor==0 nodes (fresh/post-resync). Residual: a floor==0 node self-producing on a minority branch at network pace could in principle match the signature. Verdict: KEEP floor-gated (it must NOT join the forward-exempt set of DC-2), name it in the extended invariant, and add the regression test "minor-fork stall never satisfies B7." Optional hardening is O-3.

**CR-4 — `--no-snap-sync` emergency re-enable (production_gate.rs:712-730). CLASSIFIED: evidence-guarded at every raiser; the `threshold = 10` magic value becomes dead semantics after Phase 1.** The re-enable is reachable only via the three emergency reasons, each of which carries divergence evidence at its raiser: B3 requires ≥10 empty headers AND (post-Phase-1) gap≥50 (dispatch.rs:96-161 regime guards); B4 requires ≥3 apply failures (state divergence); B5 requires all peers blacklisted for deep fork. No emergency reason is raisable on a bare minor gap. After DC-1/DC-3 delete both gap-comparator admission reads of `snap.threshold` (decision.rs:168, dispatch.rs:105), setting it to 10 is equivalent to "enabled" — the value no longer selects a gap floor anywhere in admission. The taxonomy conflation the Failure Analyst flagged ("critically stuck" vs "bypass floor" vs "bypass operator preference") is real; RC-2 untangles it explicitly. Answer to OQ-2: the override is acceptable AND documented ("--no-snap-sync is a preference for normal sync, not a ban on recovery", production_gate.rs:713) because every path to it is evidence-guarded; the class-kill does not require a further min-gap on emergencies (B3's gap≥50+empties≥10 IS the corroborated deep-fork signature per REQ-SNAP-002's own definition).

## Definite Changes (High Convergence)

- **ARCHITECTURAL: Delete Route A — remove the bare-gap OR-term `|| gap > self.snap.threshold` from `should_snap` (decision.rs:168), making `request_genesis_resync` the sole gap-based admission authority.**
  Convergence: 5/5 (Subtractionist P1, Restructurer P1, Pattern Matcher P2, Radical P1; Failure Analyst P5 conditions it on DC-2)
  Evidence: decision.rs:164-169 (the 3-term OR); recovery.rs:389/410 (large-gap already decided by `classify()`); production_gate.rs:649-658 (funnel docstring); INC-I-139 E7/E8 (N1 snapped at gap=51 via this term). Bootstrap preserved via the retained `local_height == 0` term (Route C, decision.rs:167).
  Confidence: conf(0.9, converged)
  After: `should_snap = enough_peers && attempts<3 && snap_allowed && (local_height==0 || needs_genesis_resync)` — 2 doors, both gated; X1 becomes a pure executor. Satisfies REQ-SNAP-001/002/010. Rolling-deploy verdict: **rolling-safe** (node-local recovery; no block content, no consensus rule; INV-SYNC-007 preserved). MUST ship with DC-2 in the same commit.

  ━━━ RESOURCE COST — COST-DECLARED ━━━
  Dimensions:
    CPU:      -epsilon, one fewer u64 compare per start_sync (measured)
    Memory:   0 (observed)
    IO:       0 (observed)
    Network:  - kills spurious minor-fork snaps and their multi-MB snapshot transfers (inferred)
    Disk:     - fewer history wipes and state rebuilds (observed)
    Latency:  + up to one 30s coordinator tick before a legitimate gap>=500 snap starts (measured)
  Inevitability: INEVITABLE
  Cheaper alternative: NONE-EXISTS
  Why this proposal anyway: the single deletion that closes the exact door N1 used and makes REQ-SNAP-002 provable by enumeration; the threshold-raise band-aid is WON'T-listed
  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- **ARCHITECTURAL: Narrow Gate 1 — add a floor-exempt classification for forward-snap large-gap reasons (`CoordinatorSnapEscalation`, `StuckSyncLargeGap`) in `request_genesis_resync` (production_gate.rs:674-681), bypassing Gate 1 ONLY (floor), never Gates 2/3/4/5.**
  Convergence: 2/5 explicit (Subtractionist P1 companion, Failure Analyst P5) + CODE-VERIFIED by synthesizer (CR-1 chain: block_lifecycle.rs:74-80, periodic.rs:727, production_gate.rs:666-681)
  Evidence: CR-1 above; the floor protects against BACKWARD wipes, while a coordinator snap targets `best_height > local_height ≥ floor` and cannot violate it. Gate 4 still applies, so under `--no-snap-sync` these reasons are refused (no `reset_state_only` backward hazard, production_gate.rs:630-645). `CoordinatorGenesisEscalation` (Rule 4 full wipe) and `HeightOffsetDetected` (B7) stay floor-gated.
  Confidence: conf(0.85, measured)
  A correctness fix the deleted Route A was silently masking. Rolling-safe (node-local). Ships atomically with DC-1.

  ━━━ RESOURCE COST — NEGLIGIBLE ━━━
  Dimensions:
    CPU:      0 (observed)
    Memory:   0 (observed)
    IO:       0 (observed)
    Network:  0 (inferred)
    Disk:     0 (observed)
    Latency:  0 (observed)
  Inevitability: INEVITABLE
  Cheaper alternative: NONE-EXISTS
  Why this proposal anyway: without it, DC-1 strands floor>0 gap≥500 catch-up behind Gate 1 (REQ-SNAP-003 violation); the measured conflation must be resolved, not inherited
  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- **ARCHITECTURAL: Delete the A1 redirect (dispatch.rs:96-117) — deep-fork empty-headers falls through to the gated B3 emergency funnel (dispatch.rs:153-161); the `snap.attempts = 0` and counter-reset side effects (dispatch.rs:112-113) are removed with it.**
  Convergence: 4/5 (Subtractionist P2, Radical P2 delete; Restructurer P1, Pattern Matcher P2 rewire — behaviorally identical since the funnel fallthrough already exists; radical tiebreaker picks deletion)
  Evidence: dispatch.rs:96-117 vs dispatch.rs:153-161 (same evidence signature, gated path 40 lines below); the `snap.attempts=0` reset defeats the attempt limiter. Minor-fork exposure prevented by the retained regime guards (dispatch.rs:118-129 gap≤3, dispatch.rs:144-152 gap 4..49 — INC-I-138 D4, must NOT be removed).
  Confidence: conf(0.85, converged)
  Rolling-deploy verdict: **rolling-safe** (node-local). Ships in Phase 1.

  ━━━ RESOURCE COST — COST-DECLARED ━━━
  Dimensions:
    CPU:      -epsilon, removes one O(peers) fold in the escalation path (measured)
    Memory:   0 (observed)
    IO:       0 (observed)
    Network:  - removes an attempt-limiter bypass that could re-trigger redundant snap cycles (inferred)
    Disk:     - fewer history wipes (observed)
    Latency:  0 (observed)
  Inevitability: INEVITABLE
  Cheaper alternative: NONE-EXISTS
  Why this proposal anyway: DC-1's class-kill enumeration is only sound if no path re-enters the deleted term; leaving A1 as dead code preserves its live attempts/counter-zeroing side effects
  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- **ARCHITECTURAL: Single-owner evidence contract — remove the unconditional `consecutive_empty_headers = 0` reset at dispatch.rs:84 (keep the `use_height_based_headers` flag-clear at :83); genuine block application (block_lifecycle.rs:68) becomes the sole progress reset, with the gap≤3 gossip-wait (dispatch.rs:126) as the one documented bounded exception.**
  Convergence: 4/5 on the single-owner goal (Subtractionist P4, Restructurer P2, Failure Analyst filter, Pattern Matcher constraint 4); Restructurer's kill-test amendment adopted (reset semantics move from request-emission-time to apply-time, already provided by block_lifecycle.rs:68)
  Evidence: dispatch.rs:84 is the INC-I-139 E5 starvation writer; INC-I-138 D2 was the same defect class at periodic.rs:712. Post-snap false-positive window checked: post-snap gap is small, so B3 (needs gap≥50) is diverted by regime guards, and `deep_fork_confirmed` needs 300s staleness while the first post-snap apply lands in seconds.
  Confidence: conf(0.85, converged) — CONDITIONED on the mandatory co-test: INC-I-012 F1 + INC-I-138 D2 + INC-I-139 E5 in one regression suite (REQ-SNAP-007 AC). Rolling-safe. Ships in Phase 1.

  ━━━ RESOURCE COST — NEGLIGIBLE ━━━
  Dimensions:
    CPU:      0 (observed)
    Memory:   0 (observed)
    IO:       0 (observed)
    Network:  0 (observed)
    Disk:     0 (observed)
    Latency:  0 (inferred)
  Inevitability: INEVITABLE
  Cheaper alternative: NONE-EXISTS
  Why this proposal anyway: single-writer evidence makes counter-reset starvation unrepresentable; per-incident auditing of reset sites is exactly the failed-approach class (INC-I-138 lesson)
  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Recommended Changes (Medium Convergence)

- **ARCHITECTURAL: Demote `snap.threshold` (types.rs:468) from gap-floor to on/off sentinel — `SNAP_SYNC_GAP_MIN=500` (recovery.rs:216) becomes the single source of truth for "snap-worthy gap"; gate the discv5-grace wait (decision.rs:204) on `local_height == 0`; do NOT introduce a duplicate 500 constant.**
  Convergence: 2/5 (Subtractionist P3, Radical P4) + Pattern Matcher coupling signal (threshold read in 3 modules, mutated as an emergency side-effect)
  Evidence: after DC-1/DC-3 the only admission-relevant read is the sentinel `snap_allowed = threshold < u64::MAX` (decision.rs:163); residual gap-comparator reads (decision.rs:179, :204) are bootstrap-timing only, and :204 is not gated on h==0 — after DC-1 an h>0 node could wait pointlessly for snap peers it will never use.
  Confidence: conf(0.65, converged)
  Effect: the zero-margin coupling (`threshold==MINOR_FORK_GAP_MAX==50`) dissolves structurally (REQ-SNAP-008) with no constant change and no new drift hazard. Rolling-safe.

  ━━━ RESOURCE COST — NEGLIGIBLE ━━━
  Dimensions:
    CPU:      0 (observed)
    Memory:   0 (observed)
    IO:       0 (observed)
    Network:  0 (observed)
    Disk:     0 (observed)
    Latency:  0 (observed)
  Inevitability: AVOIDABLE
  Cheaper alternative: leave threshold=50 in place, unused as a floor after DC-1
  Why this proposal anyway: an unused floor constant equal to MINOR_FORK_GAP_MAX is the exact trap that produced the zero-margin defect; a future reader will re-wire it
  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- **ARCHITECTURAL: Untangle the emergency-reason taxonomy — replace the `snap.threshold = 10` magic re-enable (production_gate.rs:729) with an explicit enable sentinel, and document three orthogonal capabilities per RecoveryReason: bypass-floor (emergency + forward-large-gap per DC-2), bypass-operator-disable (emergency only), rate/attempt limits (ALL reasons, no exceptions).**
  Convergence: 2/5 (Failure Analyst constraint, Pattern Matcher cross-signal); CR-4 code verification confirms the value 10 is dead semantics post-Phase-1
  Evidence: production_gate.rs:660-754 (the 5 gates + two bypass sites); CR-4 above.
  Confidence: conf(0.6, converged)
  Rolling-safe; pure clarification of an existing contract — the enable sentinel must preserve current emergency-recovery behavior bit-for-bit.

  ━━━ RESOURCE COST — NEGLIGIBLE ━━━
  Dimensions:
    CPU:      0 (observed)
    Memory:   0 (observed)
    IO:       0 (observed)
    Network:  0 (observed)
    Disk:     0 (observed)
    Latency:  0 (observed)
  Inevitability: AVOIDABLE
  Cheaper alternative: keep threshold=10 and add a comment explaining it is now only a sentinel
  Why this proposal anyway: a config knob that doubles as a mutable emergency flag is the coupling smell that hid OQ-2 for four incidents
  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- **ARCHITECTURAL: Wedge-escape (Phase 2, AH-gated) — FORK_GUARD stops terminally dropping the attestation-better same-height competitor (block_handling.rs:188-202); the competitor is retained/reported as coordinator evidence and re-evaluated through the existing weight machinery (`handle_new_block_weighted`, block_handling.rs:242-255), subordinating the slot tiebreak to attestation weight (INV-FORK-001). Gated by a NEW dedicated `fork_choice_weight_tiebreak_activation_height` in NetworkParams (devnet=0; testnet pinned with upgrade lead; mainnet=u64::MAX until a separate decision session).**
  Convergence: 4/5 on need (Restructurer P3+P4, Pattern Matcher P4, Radical P3, Failure Analyst P2/P4 as constraints); 3/5 independently ran the consensus-shape checklist with identical verdict
  Evidence: the wedge is a two-gate deadlock — forward apply refused (block_handling.rs:202 drop) + rollback refused (recovery.rs:367-377 finality guard) → N4's 764s park. 3-question checklist: **Q1=NO** (no user tx picks which block occupies a height), **Q2=YES** (two producers racing adjacent slots at one height is the trigger), **Q3=NO** (when slot tiebreak and attestation weight disagree, the canonical block changes) → **activation height REQUIRED**; never reuse an existing height (INC-I-054).
  Confidence: conf(0.7, converged)
  **Blocking precondition**: resolve the finality-marker grade (attestation-supermajority vs depth-based local marker; the `last_finality_height` writer is outside sync/ scope) BEFORE detailed design. If attestation-grade, the escape is a clean same-height forward swap; if depth-based, it must re-derive finality from observed attestation weight and must NOT blanket-weaken recovery.rs:364-377 (INC-I-081 class). Escape must be forward re-apply via canonical `apply_block()` (INV-SYNC-007), routed through the RequestSync governor (INV-SYNC-009), quorum-weighted with hysteresis (no flapping on locally-observed minority attestations), and evaluated earlier than Rule 2 in `classify()` ordering.

  ━━━ RESOURCE COST — COST-DECLARED ━━━
  Dimensions:
    CPU:      + one attestation-weight comparison per rare wedge event (inferred)
    Memory:   + tens of bytes, one evidence variant in the bounded 256-entry window at recovery.rs:244 (measured)
    IO:       0 (observed)
    Network:  + one bounded block re-fetch per wedge event if uncached, via the RequestSync governor (inferred)
    Disk:     - replaces snap wipe plus snapshot install with a single-block re-apply (observed)
    Latency:  -764s worst case on the wedge path, per the measured INC-I-139 N4 park (measured)
  Inevitability: INEVITABLE
  Cheaper alternative: NONE-EXISTS
  Why this proposal anyway: the only current wedge exit is snap, strictly more expensive on every dimension; this converts a 764s park plus history wipe into a bounded single-block re-evaluation
  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Options for User Decision

- **O-1: `SnapAdmission` capability token** (Pattern Matcher P1, conf(0.68, observed)). X1's signature requires a typed token constructible only from {bootstrap, catchup≥500, deep-fork evidence, emergency} — makes the class-kill compile-time-provable and non-forecloseable (future tiers = new constructors). Cost: +1 module (~80 lines). Failure filter: neutral-positive. vs. radical floor: +1 module above minimum; runtime behavior identical to DC-1..4, so this buys 5th-incident-proofing, not behavior. Radical tiebreaker: deletion core (0.9) beats token (0.68) by >0.1 → optional hardening, not the spine.
- **O-2: Peer-count-relative snap-target corroboration** (Failure Analyst P3, conf(0.7, observed)). `consensus_target_hash` admits at absolute `count >= 2` (decision.rs:65) — fakeable by 2 lagging/partitioned peers on the 8-node fleet. Change to a peer-set fraction, corroboration applied ONLY to the 50..500 band (gap≥500 stays self-sufficient — gossip silence must never starve legitimate catch-up, INC-I-016), plus a liveness fallback. Small, aligned with INC-I-012 F10 intent. Low-evidence tag: single evaluator; recommend bundling with Phase 1 tests if accepted.
- **O-3: B7 hardening** (from CR-3, conf(0.6, inferred), low-evidence). Add corroboration (peer-majority tip mismatch at our height) or a second observation window to `HeightOffsetDetected` before it can request resync on floor==0 nodes. Cheapest alternative: regression test only (covered by M1).
- **O-4: Operator-forced resync (OQ-3)** — not proposed by any evaluator; DEFER. If ever added, it becomes a new guarded feeder of `request_genesis_resync` (a constructor under O-1), never a new transition into SnapCollecting — consistent with REQ-SNAP-009 non-foreclosure.
- **O-5: `ForkEvidence` single-owner wrapper** (Restructurer P2 mechanism, conf(0.6, observed)) — a newtype with `record_empty_header()`/`record_progress()` as the only mutators, as the enforcement form of DC-4. Radical tiebreaker: DC-4's deletion form wins (a wrapper ADDS a method to enforce what deleting one errant assignment achieves); adopt only if the co-test suite shows repeated new-writer regressions.

## Constraints (from Failure Analyst — pass/fail filters applied to every change)

| Filter | Applied result |
|--------|----------------|
| Guaranteed-progress (every wedge state has a reachable exit) | PASS — CR-2 proof: rank-4 evidence-gated snap stays reachable after Phase 1 (B3 emergency + DC-2-exempted coordinator path); Phase 2 upgrades to rank-1 |
| INV-SYNC-001/004/008 (never roll back below finality) | PASS Phase 1 (untouched); Phase 2 constrained to forward re-apply / same-height swap, finality guard not blanket-weakened |
| INV-SYNC-007 (bit-identical 3-state) | PASS — no new apply path; Phase 2 escape must use canonical `apply_block()` |
| INV-SYNC-009 (RequestSync governor) | PASS — no new outbound request paths in Phase 1; Phase 2 re-fetch routes through the governor |
| INV-SYNC-010 (self-produced applies must not suppress stuck-fork signal) | PASS with test obligation — N4 wedge regression test MUST include self-production on the minority branch |
| INV-SYNC-011 (extended — see below) | PASS — DC-1/DC-3/DC-4 are its implementation |
| INV-SYNC-014 (post-snap state_db-backed utxo) | PASS — snapshot install untouched |
| INV-FORK-001 (attestation weight canonical, slot subordinate) | Phase 2 target — quorum-weight + hysteresis required to prevent flapping |
| Corroboration two-sided (fakeable ≥2 / gossip-silence starvation) | Phase 1 uses existing evidence signatures unchanged; O-2 addresses the target-hash side |
| Epoch-boundary composition (Light mode, Decision #29, INC-I-118/138) | Test obligation — replay all admission scenarios at an epoch boundary; epoch-boundary GetHeaders emptiness must not count as deep-fork evidence in tests |
| Rolling-deploy divergence | Phase 1 rolling-safe (node-local); Phase 2 AH-gated |

## Architecture Maps

### Current
```
should_snap (decision.rs:164-169) ← THREE doors:
  Route C  local_height==0                      [gated by nature]
  Route A  gap > snap.threshold(50)             [UNGATED — N1's door] ←── A1 redirect (dispatch.rs:96-117,
  Route B  needs_genesis_resync                 [5-gate funnel]            zeroes attempts + counter)
             ↑ request_genesis_resync (production_gate.rs:660)
                 Gate 1 floor — REFUSES CoordinatorSnapEscalation/StuckSyncLargeGap on ANY synced node
                 ← B1/B2 (coordinator via periodic.rs:727/731) ← classify() Rule 2 (recovery.rs:406-411)
                 ← B3/B4/B5 (emergency, floor-exempt)  ← B6/B7 (cleanup)
Evidence counter: 4 reset writers (block_lifecycle.rs:68 apply; dispatch.rs:84 request-shape; :113 A1; :126 gossip-wait)
Wedge: FORK_GUARD drops competitor (block_handling.rs:202) + finality guard refuses rollback (recovery.rs:367-377) → snap is sole exit
```

### Proposed (Definite + Recommended)
```
should_snap ← TWO doors: {local_height==0} ∪ {needs_genesis_resync}
  needs_genesis_resync ← request_genesis_resync (single admission authority)
    Gate 1 floor: refuses backward wipes; EXEMPT: emergency ∪ forward-large-gap (CoordinatorSnapEscalation, StuckSyncLargeGap)
    Gates 2/3/5 (concurrency, rate, attempts): ALL reasons, no exceptions; Gate 4 (--no-snap-sync): emergency-only bypass
    ← classify() Rule 2 (large_gap≥500 ∨ deep_fork_confirmed ∨ rollback_exhausted)  ← B3/B4/B5 emergency  ← B6  ← B7 (floor-gated)
Evidence counter: 1 progress reset (apply) + 1 documented bounded exception (gap≤3 gossip-wait)
Phase 2: FORK_GUARD reports dropped competitor → coordinator ReevaluateForkChoice (rank-1 exit) behind fork_choice_weight_tiebreak_activation_height
```

## Extended Invariant (draft text for INV-SYNC-011, REQ-SNAP-010)

> **INV-SYNC-011 (extended, all-paths)**: The SnapCollecting transition (X1, decision.rs `start_sync`) is reachable ONLY via (a) `local_height == 0` bootstrap, or (b) `needs_genesis_resync`, which is set ONLY by `request_genesis_resync()`. Every feeder of that gate requires corroborated evidence (≥10 consecutive empty headers with gap ≥ MINOR_FORK_GAP_MAX, explicit deep-fork signal, ≥3 apply failures, all-peers-blacklisted, height-offset signature) or gap ≥ SNAP_SYNC_GAP_MIN(500). No bare-gap term admits snap on any path. `consecutive_empty_headers` is reset ONLY by genuine block application (block_lifecycle.rs) or the bounded gap≤3 gossip-wait; no request-shape change or admission path may reset it. `snap.attempts` is never reset by any admission or redirect path.

**Regression-test classes** (register in `regression_tests` + `v_regression_map`):
1. **N4 wedge**: `finality == local_tip == fork-tip`, self-producing on the minority branch (INV-SYNC-010) → no snap below gap 50; evidence-gated snap only at gap≥50+empties≥10 (Phase 1); convergence without snap (Phase 2).
2. **N1 bare-gap**: gap=51, no fork evidence → NO snap (INC-I-139 replay; `SNAP_TRIGGER count == 0`).
3. **Counter-starvation writers**: INC-I-012 F1 post-snap height-fallback window + INC-I-138 D2 + INC-I-139 E5 co-test — counter reaches escalation threshold under sustained empty headers; no false resync post-snap.
4. **floor>0, gap≥500 catch-up**: coordinator SnapSync passes Gate 1 via DC-2 and reaches X1 (REQ-SNAP-003).
5. **INC-I-138 replay**: gap=28 + empties → no genesis-resync (regime guard intact).
6. **Fresh bootstrap**: h==0 snaps via Route C; INC-I-115 fresh-genesis path unaffected.
7. **Epoch-boundary replay** of classes 1-5 (Failure Scenario 5).
8. **B7 negative**: a minor-fork stall never satisfies `HeightOffsetDetected` (blocks_recent=false path).

## Migration Path

No `BRIDGE:` entries are required: Phase 1 items are the final architecture, not transitional scaffolding, and the interim rank-4 wedge exit retained between Phase 1 and Phase 2 is pre-existing behavior (B3/coordinator), not new bridge code.

**Phase 1 — rolling-safe, one deploy (local testnet first, then gauntlet, then mainnet one node at a time):**
- **M1 — Tests first (TDD)**: write regression classes 1-8 above; classes 2 and 3 must FAIL against current code; class 4 must FAIL against a Route-A deletion WITHOUT DC-2 (proves the companion is load-bearing).
- **M2 — DC-2**: forward-large-gap floor exemption in `request_genesis_resync` (production_gate.rs:674-681; reason classification alongside `is_emergency` at :666-671).
- **M3 — DC-1**: delete `|| gap > self.snap.threshold` (decision.rs:168). Atomic with M2 (same commit).
- **M4 — DC-3**: delete dispatch.rs:96-117 (A1); verify regime guards :118-129 and :144-152 retained.
- **M5 — DC-4**: remove the unconditional reset dispatch.rs:84 (keep :83 flag-clear); run the M1 co-test suite.
- **M6 — RC-1 + RC-2** (recommended, separable): threshold demotion + discv5-grace h==0 gate (decision.rs:204) + emergency taxonomy sentinel (production_gate.rs:729).
- **M7 — Institutional close-out**: extend INV-SYNC-011 in memory.db, register regression tests + protection mechanisms, update `docs/troubleshooting.md` + `docs/architecture.md`, gauntlet run (map scenarios: minor-fork stall, node-down + snap-rebuild; add an INC-I-139 replay as a gauntlet scenario seed).

**Phase 2 — separate deploy, AH-gated (`fork_choice_weight_tiebreak_activation_height`):**
- **M8 — Finality-grade investigation** (BLOCKING): locate the `last_finality_height` writer; determine attestation-grade vs depth-based. The answer decides the escape shape (same-height swap vs weight-re-derivation).
- **M9 — Design + tests**: coordinator `BetterCompetitorDropped` evidence + `ReevaluateForkChoice` action (earlier than Rule 2 in classify ordering); wedge test class 1 upgraded to assert convergence WITHOUT snap; flapping test (asymmetric attestation propagation).
- **M10 — Implementation + AH field** in NetworkParams (devnet=0, testnet pinned with upgrade lead per the external-producers rule, mainnet=u64::MAX pending a separate decision session); deploy-before-height plan.

## Complexity Comparison

| Metric | Current | Radical Minimum | Proposed (Def+Rec) |
|--------|---------|-----------------|--------------------|
| Snap admission authorities | 2 (classify + should_snap re-derivation) | 1 | 1 |
| `should_snap` OR-terms | 3 (one ungated) | 2 | 2 |
| Ungated admission paths + redirects | 1 + 1 (Route A, A1) | 0 | 0 |
| Unconditional evidence-counter reset writers | 3 (dispatch.rs:84/113/126) | 2 | 1 + 1 documented bounded exception |
| Gate-1 reason classes | 2 (emergency/normal) | 2 | 3 (emergency/forward-large-gap/normal) |
| Snap-floor constants | 2 coupled (50==50, zero margin) | 1 (500) | 1 (500; threshold demoted to sentinel) |
| New modules / abstractions | — | 0 | 0 Phase 1; Phase 2: +1 evidence variant, +1 action variant, +1 AH field |
| Net LOC (admission) | — | ~ -23 | ~ -23 +10 (DC-2/RC-2); Phase 2 +15-30 |

The radical minimum IS the Phase-1 spine (its 0.9 converged confidence holds only WITH the DC-2 companion — the companion is what the Radical's kill test missed); everything beyond it is explicitly separable (M6 recommended, Phase 2 gated, O-1..O-5 optional).

## Non-Foreclosure

Future escalation tiers (header-range backfill, targeted block fetch, operator-forced resync) enter as new evidence/feeders of the single funnel — never as new transitions into SnapCollecting (REQ-SNAP-009). INC-I-115 (open, fresh-genesis recovery): Route C (`local_height==0`) is untouched, and Rule 4 GenesisResync semantics are unchanged; any INC-I-115 fix slots in as a funnel feeder or a Route-C refinement, not a new admission point. O-1 (token) would make this structural if adopted.

## Design Synthesis Quality Gate

```
━━━ DESIGN SYNTHESIS QUALITY GATE ━━━
Evaluators completed:           5/5
Deletion convergence items:     3 (Route A 5/5; A1 4/5; counter-writer 4/5)
Restructuring convergence:      2 (floor-gate companion — code-verified; wedge-escape 4/5, AH-gated)
Addition options presented:     5 (O-1..O-5)
Failure modes identified:       11 (6 adversarial scenarios + 5 constraint filters)
Failure modes applied as filters: 11/11 (see Constraints table)
Radical floor gap:              current (2 deciders, 1 ungated door) → radical minimum (-23 LOC, 1 decider) → proposed (radical + companion +10 LOC; Phase 2 separable)
Contradictions found:           4 (floor gate, sequencing, B7, --no-snap-sync re-enable)
Contradictions resolved:        4/4 (all by direct code reads — see CR-1..CR-4)
Evidence independence verified: YES (5 independent direct reads of decision/dispatch/production_gate; distinct argument styles)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```
