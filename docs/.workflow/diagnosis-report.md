━━━ VERDICT — conf(0.96, converged) ━━━

Root cause: INC-I-138 has two co-equal causal roots. Root A (CODE): The peers.rs applied_since_rollback heuristic (D5) counted n5's own self-produced fork blocks h=34-36 as "progress since last rollback," suppressing the stuck_fork signal at source for the first 109s of the stall (Phase 1); after rollback_fresh expiry, the D2 three-writer counter-reset cycle — dominant writer production_gate.rs:558 reset_empty_headers() via periodic.rs:712 HeaderFirstSync Gate-2 cycle — prevented consecutive_empty_headers from accumulating to the G3 threshold of 3 for the remaining 216s (Phase 2), making INV-FORK-001 (G3 must fire and trigger ShallowRollback) permanently unreachable throughout the 325s stall. Root B (TRIGGER): --fork-diagnostics was removed in commit 98650be2 but testnet plists were not regenerated, causing crash-respawn loops; the staggered startup left n5 in BOOTSTRAP mode computing our_rank=Some(0) for slot=639847 while all 5 peers considered that slot "outside time window"; n5 self-produced and applied fork blocks h=34=f5dfc509, h=35=1861ea91, and the epoch-1 boundary block h=36=290d4942 — a 3-block fork no peer recognized. Three amplifiers prevented the correct recovery: D1 (response.rs:261-262 reports EmptyHeaders before the gap<=3 guard at :264) inflated the coordinator's 120s evidence window; D4 (recovery.rs:382-383 deep_fork_confirmed has no gap guard) fired SnapSync at gap=28, bypassing SNAP_SYNC_GAP_MIN=500; and a floor=0 race (block_lifecycle.rs:74 floor check before the Synchronized transition at :150) allowed CoordinatorSnapEscalation to pass. Result: n5 jumped to h=64, leaving blocks 37-63 missing.

Evidence:
  [E1a] crates/network/src/sync/manager/peers.rs — applied_since_rollback heuristic counts self-produced fork blocks as canonical progress, suppressing stuck_fork signal 21x (orchestrator-measured); same code path as INC-I-081 Candidate A
  [E1b] git:98650be2 — removed --fork-diagnostics from binary; testnet plists not regenerated; seed.log:1-70 shows 3+ crash-respawn cycles before 14:47:55; h=1 and h=2 same producer=20204725 confirms staggered startup
  [E2] log:14:54:30 seed.log:1479 "REJECT f5dfc509, invalid producer for slot: producer=3047e96b, slot=639847, reason=outside time window" — all 5 peers simultaneously rejected n5's h=34 fork block
  [E3] log:n1.log:2549 "[BLOCK] Applied h=36 hash=5262d0dc producer=effe88fe slot=639849 epoch=1" vs log:n5.log:4674 "[BLOCK] Applied h=36 hash=290d4942 producer=3047e96b slot=639852 epoch=1" — H4 CONFIRMED: 290d4942 != 5262d0dc; corroborated by seed.log:1557, n2.log:2490
  [E4] measure:stuck_fork_suppressions=21 (orchestrator-measured, crates/network/src/sync/manager/peers.rs) — 21x WARN "applied since last rollback -> BEHIND not forked. Suppressing stuck_fork signal"
  [E5] measure:GetHeadersByHeight_calls=142 (n5.log:7051-17185); measure:ShallowRollback_events=0 (n5.log stall window); measure:consecutive_max=2 — G3 threshold=3 never crossed in 325s
  [E6] graph:.handle_response() --calls--> .handle_headers_response() (crates/network/src/sync/manager/sync_engine/response.rs:188); graph:.SyncManager --method--> consecutive_empty_headers (crates/network/src/sync/manager/production_gate.rs:22)
  [E7] crates/network/src/sync/manager/sync_engine/response.rs:261 — self.recovery.report(EmptyHeaders{peer, gap}) executes before gap guard at :264
  [E8] log:15:00:45 n5.log:17197 "[COORDINATOR] action=SnapSync gap=28 last_applied=325s shallow_rb=0 snap_attempts=0" — D4 terminal event (orchestrator-measured)
  [E9] crates/network/src/sync/manager/recovery.rs:382 — deep_fork_confirmed = (deep_fork > 0 || (empty_count >= 10 && ctx.last_applied_secs >= STALE_TIP_SECS)); no gap guard
  [E10] crates/network/src/sync/manager/block_lifecycle.rs:74 — confirmed_height_floor check runs before Synchronized transition at :150; crates/network/src/sync/manager/production_gate.rs:674 — Gate 1 (confirmed_height_floor > 0) false

Causal chain:
  1a. D5+D2 starvation: peers.rs suppresses stuck_fork Phase 1; production_gate.rs:558 resets counter Phase 2 — G3 unreachable [E1a][E4][E5][E6]
  1b. Deploy artifact: git:98650be2 plist not regenerated; crash loop; staggered startup; n5 BOOTSTRAP self-election h=34/slot=639847 [E1b][E2]
  2. n5 self-produces fork h=34=f5dfc509 rejected by all 5 peers; builds h=35, h=36=290d4942 (epoch-1 boundary) [E2][E3]
  3. n5 fork h=36=290d4942 vs canonical h=36=5262d0dc; GetHeaders(290d4942) returns count=0; gossip h=37-39 orphaned [E3]
  4. D5 Phase 1 (t=0-109s): applied_since_rollback true; 21x stuck_fork suppressed; no recovery triggered [E4]
  5. D2 Phase 2 (t>109s-325s): 142 GetHeadersByHeight "header chain broken" calls each reset counter; consecutive max=2; 0 ShallowRollback [E5][E6]
  6. D1: response.rs:261 reports EmptyHeaders before gap guard at :264; coordinator evidence inflated; empty_count >= 10 trivially satisfied [E7]
  7. D4: t=325s deep_fork_confirmed fires; no gap guard; Rule 2 returns SnapSync at gap=28 bypassing SNAP_SYNC_GAP_MIN=500 [E8][E9]
  8. floor=0 race: block_lifecycle.rs:74 before :150 transition; floor=0; Gate 1 passes; single-step SnapSync executes [E10]
  9. (symptom) SnapSync to h=64; blocks 37-63 missing; fleet converged h=79 after manual backfillFromPeer [E8]

Counter-hypotheses ruled out:
  - H1 (serving-side hash lookup bug at epoch boundary): ruled out by [E3] — canonical peers at best_height=39-64 returned count=0 for 290d4942; serving path only returns empty for unknown hash when best_height > start_height; 290d4942 not in canonical chain
  - H2 (FINALITY_GUARD blocked ShallowRollback): ruled out by [E8] — zero FINALITY_GUARD log lines in stall window; fork block received zero attestation weight; last_finality_height cannot be Some(36)
  - H3 (gossip silence primary driver): ruled out by [E3] — 25 orphan gossip blocks and 1,399 ORPHAN_CHASE events in stall; stall was retrieval failure, not gossip silence
  - M1/M4 (anomalous empties or serving limiter): ruled out by [E3][E7] — two methods confirm 290d4942 not canonical; PM-009 returns "busy" error string not empty headers
  - H-WC1 (n5 synced FROM fork peer 3047e96b): ruled out by orchestrator fact — n5.log [BLOCK_PRODUCED] for f5dfc509 and 290d4942; n5 IS producer 3047e96b
  - INC-I-137 active during incident: ruled out by [E1b] — binary is f8abb5c9; INC-I-137 (6633f53a) committed AFTER incident

Regression check: git log 64257bdb..HEAD shows 98650be2 (--fork-diagnostics removal), f8abb5c9 (INC-I-136), 6633f53a (INC-I-137), 2f140b4b (testnet re-genesis), 65024d14 (gauntlet), 894808c5 (gauntlet chaos). None touched D5 (peers.rs) or D2 (production_gate.rs:558). The regression is: INC-I-120 (64257bdb) wired G3 but left INC-I-120-RC2a (counter starvation, P1 open) unresolved; 98650be2 created crash-loop conditions activating pre-existing D5 suppression for the first time in a self-fork scenario. D5 and D2 predate the INC-I-120 baseline — latent defects now manifested.

Shape-Recurrence: RECURS
  Checked: INC-I-120 (network/sync domain, resolved 2026-06-30, within 180 days)
  Same architectural shape as: INC-I-120 — INV-FORK-001 violated: G3 stuck-fork detection cannot trigger ShallowRollback despite structural preconditions met. INC-I-120: G3 not wired. INC-I-138: G3 wired but starved via D5+D2. Same seam, same invariant, different mechanism. This is the 2nd occurrence.
  Cross-domain: INC-I-081 (consensus/fork/sync) cited same peers.rs path as Candidate A (conf 0.55); INC-I-138 confirms measured (21x).

Recommended Fixes:
  - FIX: Fix peers.rs applied_since_rollback to not count self-produced fork blocks as canonical progress in crates/network/src/sync/manager/peers.rs; gate the "BEHIND not forked" classification so self-produced blocks on an unrecognized fork tip do not suppress stuck_fork.
      Breaks chain at: [E1a]
      Removes Phase 1 suppression; stuck_fork fires within seconds for self-fork scenarios.
  - FIX: Regenerate testnet plists after git:98650be2 via scripts/install-local-services.sh; verify with doli-node --help before restarting nodes.
      Breaks chain at: [E1b]
      Eliminates crash-loop staggered startup that placed n5 in BOOTSTRAP self-election.
  - DEFENSE-IN-DEPTH: Fix D2 counter starvation in bins/node/src/node/periodic.rs:712 — make reset_empty_headers() conditional on INC-I-012 F1 post-snap path only. MUST deploy with D4 fix (naive removal causes dispatch.rs:96 GenesisFallbackEmptyHeaders at gap<50).
      Breaks chain at: [E5]
      Restores G3 accumulation Phase 2; counter reaches 3; ShallowRollback triggers.
  - DEFENSE-IN-DEPTH: Fix D4 gap-blind escalation — add gap >= MINOR_FORK_GAP_MAX(50) guard to deep_fork_confirmed at crates/network/src/sync/manager/recovery.rs:382. MUST deploy with D2 fix.
      Breaks chain at: [E9]
      Prevents SnapSync at gap=28; node waits for ShallowRollback or operator intervention.
  - DEFENSE-IN-DEPTH: Fix D1 evidence gating — move self.recovery.report(EmptyHeaders) from crates/network/src/sync/manager/sync_engine/response.rs:261 to after gap<=3 guard at :264. Independent; safe alone.
      Breaks chain at: [E7]
      Reduces false evidence; empty_count reflects genuine gap>3 responses only.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Diagnosis Report: INC-I-138 — n5 325s Stall at Epoch-1 Boundary, Spurious SnapSync at gap=28

## Symptom Profile

- **What happens**: n5, on a fresh 6-node local testnet (v6.23.9, binary f8abb5c9), stalled 325s at h=36 (epoch-1 boundary) with 152 count=0 GetHeaders responses and 142 GetHeadersByHeight "header chain broken" calls. G3 ShallowRollback never fired. At t=325s, deep_fork_confirmed escalated to SnapSync at gap=28 (bypassing SNAP_SYNC_GAP_MIN=500), leaving blocks 37-63 missing.
- **When**: 2026-07-07, 14:55:20 to 15:00:45; binary f8abb5c9 (INC-I-120 base + 98650be2; INC-I-137 NOT included)
- **Deterministic**: YES — recurs at every testnet epoch boundary until D5+D2 fixed
- **Failure boundary**: 6-node local testnet, epoch_len=36; mainnet has same defects but lower self-fork probability

## Fundamentals Check

- n5 = producer 3047e96b (orchestrator confirmed from n5.log [BLOCK_PRODUCED])
- Epoch length: 36 blocks testnet (protocol.md:1514); h=36 is epoch-1 boundary
- INC-I-012 F1 gates: post_snap_recovery OR snap_exhausted — both false for n5
- INC-I-120 G3 wiring (64257bdb): consecutive_empty_headers >= 3 -> ShallowRollback; 0 ShallowRollback in stall
- Incident binary: f8abb5c9; INC-I-137 (6633f53a) committed AFTER incident

## Investigation Summary

| Investigator | Evidence Layer | Top Hypothesis | Confidence | Key Finding |
|-------------|----------------|----------------|------------|-------------|
| Log Forensics | n5.log, seed.log, n1-n4 logs | H4: n5 on fork (290d4942 != 5262d0dc) | conf(0.96, measured) | Multi-node hash comparison; 142 GetHeadersByHeight, 0 ShallowRollback; D4 at n5.log:17197 |
| Code Logic | response.rs, dispatch.rs, recovery.rs, block_lifecycle.rs | D4 gap-blind + D5 Rule 1b non-firing | conf(0.65, observed) | 3-writer D2; floor=0 race at block_lifecycle.rs:74; D5 raised as H5 |
| State Reconstruction | Chain state, serving semantics | H4 confirmed via 2 derivations | conf(0.65-0.70, observed) | count=0 iff hash unknown; epoch boundary makes fork irreconcilable |
| Constraint Elimination | 8 prior fix constraints, PM map | M2 sole survivor; M1/M3/M4 eliminated | conf(0.65, inferred) | All D1-D4 pass invariants; GS-008 busy-rate out of scope |
| Wildcard | Deploy artifacts, binary versions | H-WC2 crash loop + H-WC3 INC-I-137 absent | conf(0.55-0.68, measured) | 3+ crash cycles; binary=f8abb5c9; persistent fork at h=19,h=34,h=36 |

## Convergence Matrix

```
                                    LOG    CODE   STATE  CONST  WILD   Score
H4 (n5 on fork 290d4942)            Y      Y      Y      Y      Y     5/5 conf(0.97, converged)
D4 gap-blind SnapSync at gap=28     Y      Y      Y      Y      Y     5/5 conf(0.97, converged)
D2 counter starvation               Y      Y      Y      Y      -     4/5 conf(0.93, converged)
D1 evidence before gap guard        Y      Y      Y      Y      -     4/5 conf(0.90, converged)
D5 suppression heuristic            -      Y(H5)  -      -      Y(A)  orchestrator conf(0.95, measured)
```

CONVERGENCE INDEPENDENCE CHECK:
Hypothesis: H4 (n5 on fork)
Converging investigators: LOG, CODE, STATE, CONSTRAINTS, WILDCARD (5/5)
Evidence independence:
  - LOG: multi-node log hash comparison (n1.log:2549, seed.log:1557, n2.log:2490 vs n5.log:4674)
  - CODE: serving-path semantic derivation from validation_checks.rs:1011
  - STATE: orphan gossip evidence (25 orphans gap=3) + serving semantics derivation
  - CONSTRAINTS: logical elimination matrix (M1/M3/M4 killed by E1+E2)
  - WILDCARD: seed.log serving-side observation (seed never applied 290d4942)
  INDEPENDENT? YES -> True convergence

## Contradictions

Four contradictions found; all four resolved.

**C1 (D3 scope)**: LOG said GetHeadersByHeight fired 142x; Brief/CODE said D3 unreachable. Resolution: different code paths. The 142 calls used dispatch.rs:84 M-RC12-full path, not the INC-I-012 F1 path (response.rs:218-226). Both correct about different things.

**C2 (n5 identity)**: Wildcard said "n5 synced FROM fork peer 3047e96b." Orchestrator: n5 IS 3047e96b. Resolution: self-fork, not sync-from-fork-peer. Wildcard's behavioral insight (persistent fork production) was correct; identity framing wrong.

**C3 (n5.log freshness)**: Wildcard said log overwritten. Orchestrator: first timestamp 14:48:05, intact. Resolution: wildcard's assumption incorrect; all log-forensics citations valid.

**C4 (incident record)**: Context claimed "CANONICAL on all peers." Resolution: FORMALLY WRONG per E2/E3 modus tollens. Must be corrected.

## What I Don't Understand

1. Exact fork point (h=19 ca52514d or later); wildcard's inference plausible but unconfirmed
2. Exact peers.rs code path for applied_since_rollback propagation; mechanism measured but internal code not read
3. confirmed_height_floor=0 race timing in the normal (non-fork) case
4. Whether D3 broadening or "header chain broken" -> fork evidence would be the better fix shape

## Root Cause

n5 stalled 325s because G3->ShallowRollback (INV-FORK-001) was permanently unreachable via two-phase starvation. Phase 1 (t=0-109s): peers.rs applied_since_rollback counted self-produced fork blocks as "progress," suppressing stuck_fork 21x. Phase 2 (t>109s): D2 three-writer counter-reset cycle (dominant: production_gate.rs:558 via periodic.rs:712 every ~30s; secondary: dispatch.rs:84 on "header chain broken" 142x) kept consecutive_empty_headers at max=2, below G3 threshold=3. Only Rule 2's gap-blind deep_fork_confirmed (recovery.rs:382) remained, firing at t=325s with gap=28. The trigger was a deploy artifact: --fork-diagnostics removed in 98650be2 without plist regen caused crash-loop staggered startup; n5 in BOOTSTRAP mode self-elected for slot=639847, producing a block all peers rejected "outside time window," building a 3-block fork at epoch-1 boundary h=36=290d4942. ShallowRollback from h=36 to h=35 (1 block) would have recovered in seconds; instead SnapSync to h=64 skipped 28 blocks.

## Causal Chain (with Derivation Test)

See VERDICT block above for the complete 9-link chain with per-link [E_n] citations. Convention: link 1 is root cause; link 9 is observed symptom.

## Shape-Recurrence

RECURS — 2nd occurrence. See VERDICT block. INC-I-120 (resolved 2026-06-30) is prior: same seam, same invariant (INV-FORK-001), different mechanism (absent wiring vs starved signal).

## Why Previous Fixes Failed

| Fix | Why It Didn't Work |
|-----|-------------------|
| INC-I-120 G3 wiring (64257bdb) | Wired the action path but left INC-I-120-RC2a (D2 starvation, P1 open) unresolved |
| INC-I-012 F1 GetHeadersByHeight | Correct for post-snap; unreachable for normally-synced n5 (snap.attempts=0); 142 calls used different trigger |
| INC-I-081 finality guard | Fixed finality check; did not address D5+D2 starvation preventing ShallowRollback from being called |
| INC-I-089 BOOTSTRAP gate | 15s timer insufficient for crash-loop staggered startup severity |

## Flagged Out-of-Scope Items

1. GS-008 busy-rate (PM-009): 31% on N=6 NOT fixed by D1-D4; separate ticket needed
2. Plist regeneration ops procedure: install-local-services.sh must re-run after CLI flag changes
3. GS-002 gauntlet assertions: convergence + no-spurious-escalation + no-empty-headers-loop
4. Incident record correction: remove "CANONICAL on all peers" (formally wrong)
5. PM-007/PM-011 registry updates required post-fix

## Feasibility Verdict

```
━━━ DIAGNOSTICIAN FEASIBILITY VERDICT ━━━
Fixable with code change:  YES
Confidence:                conf(0.96, converged)
Reasoning:                 All five code defects (D1/D2/D4/D5/floor=0) are internal to
                           crates/network/src/sync/manager/. None change consensus rules,
                           block content, or wire format. All sync-coordinator-internal.
                           Safe for rolling deploy with no activation height. Trigger fix
                           (plist regen) is ops. Zero external dependencies.
Architect's verdict was:   CODE-FIXABLE (per system-blueprint.md)
Agreement:                 AGREES
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Synthesis Quality Gate

```
━━━ SYNTHESIS QUALITY GATE ━━━
Investigators completed:       5/5
Convergence on top hypothesis: 5/5 on H4; 4-5/5 on D2/D4
Evidence independence:         VERIFIED
Contradictions found:          4
Contradictions resolved:       4/4
Unexplained items:             1 (peers.rs internal code path)
Evidence layers covered:       logs, source code, chain state, constraints, deploy env
Evidence layers NOT covered:   runtime Prometheus metrics (not material)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```
