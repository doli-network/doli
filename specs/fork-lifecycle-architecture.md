━━━ FINDINGS — 7 total (DECISION:7) ━━━

  [F1] DECISION conf(0.90, converged) — crates/network/src/metrics.rs:276-303, sync/manager/peers.rs:429, wedge_escape.rs:179-188 — observability + pre-AH canary ships first, alone (M0)
  [F2] DECISION conf(0.85, converged) — crates/network/src/sync/manager/{block_lifecycle.rs:410,528,594; types.rs:668; reorg/mod.rs:287}, crates/core/src/consensus/constants.rs:241, recovery.rs:89 — delete the five dead fork-authority symbols (M1)
  [F3] DECISION conf(0.85, converged) — crates/network/src/sync/manager/recovery.rs:336-490, periodic.rs:838-864 — ladder terminates by subtraction + one named non-lossy terminal that acquires evidence, never decides a branch (M3)
  [F4] DECISION conf(0.90, converged) — bins/node/src/node/production/mod.rs:630, rollback.rs:10 — escape-before-enforcement: audited operator escape ships and is drill-proven, THEN the poison arm retracts only a tip it created, behind a callee-side RollbackAuthority (M4)
  [F5] DECISION conf(0.85, converged) — crates/network/src/sync/reorg/mod.rs:155,306-420,423-560; crates/core/src/finality.rs — ONE fork-choice authority over real heights + ancestry finality + derived finality, behind ONE new activation height; the only CHOICE-class change, ships last (M5)
  [F6] DECISION conf(0.80, converged) — crates/network/src/sync/manager/sync_engine/decision.rs:81-108 — branch-aware best_peer via the existing checkpoint_health formulation, with mandatory unfiltered fallback (M2)
  [F7] DECISION conf(0.70, converged) — crates/network/src/sync/manager/{recovery.rs:447-454, sync_engine/dispatch.rs:96-98} — snap admission narrowed to bootstrap/behind-ness/operator; fork-reachability deleted only after the C-12 drill passes (M6)

  Speculative: 5 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Fork Lifecycle Architecture

**Incident:** INC-I-204 · **Run:** 541 · **Mode:** PROPOSAL-ONLY (no --fix) · **Date:** 2026-09-01
**Synthesized from:** 5 independent design evaluations (subtraction, restructure, patterns, failures, radical) over `docs/redesigns/fork-lifecycle-redesign-analysis.md` (canonical upstream).

## Problem Statement

Fork-lifecycle failure is the dominant failure class of DOLI: 107 of 192 incidents (55.7%) match the fork sweep; four incidents of one shape in 180 days (INC-I-081 → INC-I-147 → INC-I-190 → INC-I-204). The measured INC-I-204 chain: a user-submittable tx that fails only at apply reaches an unconditional rollback in the production error arm (`production/mod.rs:630`), retracting an already-gossiped valid tip on 5/5 producers; recovery then fails because ~13 distinct code sites answer "what is my canonical branch" from different data in different units; the only reachable terminal was lossy snap sync at gap=50, destroying archival history (blocks 77778-77826 permanently null).

**Why prior attempts did not end the class** (all five evaluators accept the analyst's calibration): two structural simplifications already shipped (`sync/manager/mod.rs` 67→25 fields 14→3 states; `production_gate.rs` 590→130 lines) and forks continued — they reduced *representation* while leaving the *number of independent branch-answerers* unchanged. Every prior fix added a guard; none removed an authority. The lever is authority reduction (13 → 3), executed mostly by deletion, with the single consensus-visible CHOICE change gated behind one new activation height, last.

━━━ RESOURCE COST — SUMMARY — COST-DECLARED ━━━
Dimensions:
  CPU:      +O(peers) ring scan per sync-source pick, +1 RocksDB point read per fork-choice eval post-M5; −classifier composites per 30s tick (observed/inferred, per-decision blocks below)
  Memory:   net negative — −finality mirror field, −up to ~480KB shadow-map height bookkeeping post-M5; +32B/peer finalized-hash only if Option O1 chosen (measured struct shapes)
  IO:       −large in the tail — every avoided snap escape avoids a full state rewrite; +1 point read per fork-choice call post-M5 (inferred, unbenchmarked — named risk)
  Network:  −large — removes the 1Hz sibling re-evaluation storm, the 414:3 empty-header thrash, and the fleet re-sync storm per poison event (measured in INC-I-204)
  Disk:     −large — no snap-induced permanent body holes; +metric/alert lines (M0) (measured: 49-block hole per event today)
  Latency:  −minutes-to-days on the wedge path (measured 479.97s to lossy escape today, 7-day undetected wedges); +sub-ms on reorg evaluation post-M5
Inevitability: AVOIDABLE
Cheaper alternative: ship only the diagnosis synthesizer's one-line poison-arm conditional and stop — kills the INC-I-204 manufacturing trigger, leaves the recovery ladder, the plural authorities, and the lossy-only escape intact.
Why this proposal anyway: the shape has recurred 4 times in 180 days from 3 different triggers; the one-liner breaks one trigger's chain while the absorbing ladder and 13 disagreeing authorities remain armed for the next — d.35 records that only consolidation-by-subtraction has ever ended a recurrence class here.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Evaluation Summary

| Evaluator | Lens | Top Proposal | Confidence | Key Finding |
|-----------|------|-------------|------------|-------------|
| Subtractionist | removal | Path B executed as subtraction (delete non-acting rungs, dead classifiers) | conf(0.7, measured) | 4 dead symbols + 2 redundant encodings inflate the "20+ authorities" count; the finality mirror is an order-gated compensator |
| Restructurer | boundaries | Path A; first two rungs AH-free; branch answer already computed, wired to wrong consumers (CS-5) | conf(0.7, measured) | `record_fork_block` has no height param — the INC-I-147 AH fix never reaches the fork path on ANY network |
| Pattern Matcher | patterns | Path A destination, Path B milestone 1; no mature chain has a recovery ladder | conf(0.65, observed) | The canonical persistent index already exists in `block_store`; the network layer shadows it with an ephemeral LRU |
| Failure Analyst | failures | B-then-A, ONLY if the terminal acquires evidence rather than decides a branch | conf(0.7, measured) | Absorbing cell is `0<gap<50 ∧ tip==finality ∧ sibling-exhausted`; t_escape=(50−gap₀)×10s verified to 0.006%; the incident turned on a WEIGHT TIE |
| Radical Simplifier | minimal | 3 authorities (fork_choice, converge, FinalityTracker); gap is not a fork quantity | conf(0.6-0.7, measured) | 13 authorities, 2 silent absorbing states, 3 lossy escapes measured; `forceReorgTo` operator escape must not be dropped |

## Convergence Matrix

Y = independently proposed/confirmed; (y) = confirmed as cross-signal outside own lens.

| Item | Subtr. | Restr. | Pattern | Failure | Radical | Verdict |
|---|---|---|---|---|---|---|
| A-vs-B is a false binary; B (termination, largely by deletion) is A's mandatory milestone 1 | Y | Y | Y | Y | Y | 5/5 — DEFINITE framing |
| Only the final CHOICE change is AH-gated; everything before is rolling-safe | Y | Y | Y | Y | Y | 5/5 — DEFINITE |
| Observability first (FORK_GUARD site labels, unique_chain_tips, refusal-reason split, canary) | (y) | (y) | (y) | Y | (y) | 5/5 — DEFINITE (M0) |
| Delete legacy reorg pair `SyncManager::handle_new_block`+`check_reorg` (0 callers) | Y | Y | Y | — | (y) | 3/5 + synthesizer grep — DEFINITE (M1) |
| Delete finality mirror F2/F3; derive effective finality; STRICTLY after REQ-FORK-002 | Y | Y | Y | (y) | Y | 4/5 — DEFINITE (M5), order-gated |
| Poison arm must not retract a tip it did not create (REQ-FORK-002) | (y) | Y | Y | Y | Y | 4/5 + diagnosis verdict — DEFINITE (M4b) |
| Escape ships BEFORE enforcement (REQ-FORK-012), proven by drill | Y | Y | Y | Y | Y | 5/5 — DEFINITE ordering (M4a) |
| Absorbing cell needs a named, non-lossy, evidence-acquiring terminal | Y (by deletion) | (y) | Y | Y | Y | 4/5 — DEFINITE (M3) |
| Branch-aware `best_peer` from existing computation (checkpoint_health form, F7-safe) | — | Y | Y | (filter T4) | (y) | 2/5 + verdict fix #4 — RECOMMENDED (M2) |
| Snap stops being reachable as a fork remedy (bootstrap/behind-ness/operator only) | — | — | Y (S4/C1) | (C-16 framing) | Y | 2/5 — RECOMMENDED (M6), drill-gated |
| Corroborated automatic finality retraction (≥2/3 local ProducerSet weight) | — | — | — | (constrains) | Y | 1/5, conf 0.55 — OPTION O1 |

**Evidence-independence check (key convergences):** the legacy-pair deletion was reached from graph+grep (subtraction), from a boundary map (restructure), and from an industry shadow-index classification (patterns) — three different evidence chains; independently re-verified by synthesizer grep over `crates/` and over `bins/` this session (definition-only hits in each root). The poison-arm convergence rests on four different chains: compensator analysis, caller-guard-regime asymmetry, Bitcoin template-discard analogy, and the measured [E3] retraction — TRUE convergence, confidence boost applied. The escape-before-enforcement ordering converges from five chains but shares one root (INV-PROD-002/REQ-FORK-012) — same-evidence convergence, no boost above 0.85; it is additionally mechanical (see Contradiction 4).

## Adjudicated Contradictions

**1. `check_reorg_weighted` — is mainnet exposed?** (radical's "ungated and mainnet-live" vs INV-SYNC-012's 0-reject measurement vs "mainnet is protected"). Adjudicated by reading the function this session (`reorg/mod.rs:306-420`). Both claims are correct at different layers. The finality comparison (:373-386, `block_weights.get(&ancestor).map(|w| w.height).unwrap_or(0)` vs real `last_finality_height`, **no AH gate — verified**) executes only after four earlier gates: fork-not-extension (:316), parent known (:321), weight ≥ current (:332-359), common ancestor found (:365 ff). On the gossip path nearly every block is a direct extension and exits at :316 — the comparison sits on a rare sub-path, which is why 0 rejects were measured (INC-I-204's reorg attempts routed through wedge_escape→`plan_reorg`, whose distinct veto message was measured 23×). Rarity is not soundness. When the sub-path is reached on mainnet, the ancestor height is REAL only if it was canonically applied post-AH since the last restart and is still in the 10,000-entry LRU; the residual cases — fork-path-recorded ancestor (synthetic ALWAYS, see Contradiction 2), LRU eviction/restart (`unwrap_or(0)`) — all UNDER-state the height, so the failure direction is always **false VETO, never false PERMIT**. **Mainnet exposure verdict: mainnet is protected from finality VIOLATION via this site and from [E4]'s `plan_reorg` pre-AH veto (AH 129,500 crossed), but is NOT protected from ungated false-veto wedges (the INC-I-204 non-recovery shape) via this site.** No AH-free fix exists (radical kill test: all 3 formulations consensus-visible) — the site is deleted inside M5's gated unification. M0 adds a site-labelled counter on entries/rejects of this comparison so the live frequency is measured before M5.

**2. `record_fork_block` passes no height.** VERIFIED this session at `reorg/mod.rs:155`: body is `record_block_internal(hash, prev, weight, false, None)`; `record_block_internal` derives `parent_height + 1` with parent defaulting to 0. Every block recorded on a competing branch during wedge escape (`wedge_escape.rs:97`) or fork recovery (`fork_recovery.rs:32`) carries a synthetic height **on every network, in every epoch** — the INC-I-147 AH fix is writer-side and never reaches this writer. Restructure conf(0.65) CONFIRMED → raised to conf(0.8, measured+verified). Placement: the writer fix changes fork-choice inputs for reachable inputs (radical kill test (i): consensus-visible), so it rides **M5's single new AH** — restructure's "rolling, step 3" hedge is overruled by radical's and patterns' agreeing classification (2-vs-1, and the kill-test evidence is stronger).

**3. n6 logged `gap=0` throughout the wedge — does gap-banded reasoning survive?** Reconciled: the patterns lens read 22:44:08–22:45:10 — precisely the 50-second window in which ALL five producers were poison-stalled at h=77779, so the whole fleet's tip was static and `gap` (a max over peers) was genuinely 0. The failure analyst's window extends to 22:53:10 and measures gap growing 2→50 at exactly one per 10s slot once the winning branch advanced: `t_escape=(50−gap₀)×10s`, predicted 480s, measured 479.97s (0.006% error). **Both measurements are correct in their windows; gap DOES grow in this wedge class.** The relocated cliff stands: the absorbing cell is `0 < gap < 50 ∧ tip==finality ∧ sibling-exhausted ∧ snap_attempts=0` (failure §3.2), entered immediately and exited only by degradation to gap=50. Gap-banded reasoning survives as a *description of the current system*; as a *design input* it is rejected — radical's structural claim holds (fork cost is fork depth, not distance-behind; constraint: gap must not appear in any fork decision of the successor design).

**4. Finality-mirror deletion is order-gated — hard precondition.** VERIFIED: `FinalityTracker`'s public API (`crates/core/src/finality.rs`) has no reset/lower — `last_finalized` is monotone by construction (subtraction P3 kill test FOUND; restructure P2 second kill test TRUE). The erasable mirror (`ReorgHandler.last_finality_height`) is a compensator whose only distinguishing capability is erasure, and its trigger is created by the unguarded rollback. **Hard precondition, mechanical not political: the mirror may not be deleted until REQ-FORK-002 (M4b) is live and proven — otherwise any node that lands below finality is permanently wedged with no API to release it.** The successor is restructure's derived form `effective_finality = min(tracker.last_finalized, local_tip)` — a pure function of two owned values that reproduces the clearer's release semantics with zero erasable state — plus wiring the already-written, zero-caller `is_at_or_below_finalized` (`finality.rs:191`) into the single rewind door. Encoded in the migration path: M5 depends on M4b + C-12 drill.

**5. The tie warning — a new authority in a mixed-version fleet can itself split the fleet.** The incident turned on `fork_w=10390 <= our_w=10390` — a weight tie, the exact input where two versions of a fork-choice rule diverge. The migration avoids a version-partitioned fork by construction: (a) NO tie-break or fork-choice semantics change ships outside M5's gate (trap T9 honored — M2/M3/M4 are SOURCING/RETENTION class, verified per-milestone below); (b) below the activation height, old and new binaries are byte-identical on every fork decision (constant gate, dormant); (c) at the activation height every activated node flips on the same deterministic height threshold — version-independent; (d) the residual risk (a laggard binary at activation) is handled by a `getForkChoiceVersion` RPC + guardian fleet-readiness check before the AH is reached, an AH margin sized for auto-update convergence across ~30 external producers, and the M0 pre-activation canary. This is the same discipline that made Pillar 2 (constant gate, never HardForkSchedule) succeed on the second attempt.

**6. `forceReorgTo` operator escape — include or refute?** INCLUDED (M4a). Radical flags it as the single item that must not be dropped: with snap's fork-doors deleted (M6) and the poison bypass closed (M4b), an all-wedged fleet (INC-I-190 shape — nobody advances, so no corroborating evidence exists) has **zero exits** without it. Patterns independently proposed the same shape (Bitcoin `invalidateblock`: explicit, named, operator-invoked) — 2/5 convergence on mechanism, 5/5 on the ordering it serves. Patterns C9 is honored as a hard design term: the escape's authorization **expires or is restart-scoped** (a sticky operator mark on an auto-updating fleet is the INC-I-196 self-brick shape), it requires the target to be corroborated by ≥2/3 of **local ProducerSet weight** (F7-safe: peer-count corroboration is Sybil-advanceable and is DEAD), and every invocation is logged and metered.

**Minor contradiction resolved:** radical deferred `SEED_CONFIRMATION_DEPTH` as possibly CHOICE-class; subtraction measured it at **zero reads repo-wide** — re-verified by synthesizer grep this session (definition only, `constants.rs:241`). A constant with no readers cannot influence any computed value: INV-12 Q3 is YES by construction. Subtraction wins on evidence quality; the constant is deletable in M1 and analyst row U4 / REQ-FORK-018 collapse to a spec correction (`specs/engine-parts.md:772` documents behavior no code implements — fix in the same change).

## Definite Changes (High Convergence)

- ARCHITECTURAL: Observability and canaries become the first shipped layer of the redesign — the class must be visible before any behavior changes (M0).
    Convergence: 5/5 (failure A0/B0 primary; subtraction, restructure, patterns, radical cross-signals name the identical gaps)
    Evidence: `metrics.rs:276-303` instruments FORK_GUARD_REFUSALS for `site="producer_rebuild"` only while the measured refusals (n6=43, n17=53, seed=32 over 7 days) come from `recovery.rs:358/:405`; `checkpoint_health()` already computes `unique_chain_tips` every tick and discards it (`peers.rs:429`, `periodic.rs:1036`); `wedge_escape.rs:179-188` collapses finality-veto and not-heavier into one 1Hz message; a `None`-classified node logs nothing (`block_lifecycle.rs:664-676`); INV-SYNC-012 has 0 canaries.
    Confidence: conf(0.90, converged)
    What changes: FORK_GUARD refusal counter gains `site` labels; `unique_chain_tips > 1` sustained becomes a fleet fork alarm; wedge_escape refusal reasons split; bounded absorbing-state logging; pre-AH canary counter for every pre-activation branch (armed BEFORE M5, per REQ-FORK-014); a site-labelled probe on `check_reorg_weighted`'s finality comparison (Contradiction 1); export path verified against INC-I-187 (28/57 metrics never written).

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      +negligible (label attachment on existing counters; the tip computation already runs every checkpoint tick) (observed)
      Memory:   +few counters per node (observed)
      IO:       0 (observed)
      Network:  +metric export bytes per scrape interval (inferred)
      Disk:     +bounded log/alert lines; −large from removing nothing yet (M3 removes the 1Hz storm) (observed — n6.log is 688MB today)
      Latency:  0 on all block paths (observed)
    Inevitability: AVOIDABLE
    Cheaper alternative: rely on operator log greps, as today.
    Why this proposal anyway: the INC-I-204 signature accumulated for 7 days undetected; every later milestone (especially M5's dormant window) is unverifiable without this layer, and the alarm input is already computed and thrown away.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: Delete the five production-dead fork-authority symbols — the claimed "20+ disagreeing authorities" shrinks to ~12 live ones by pure subtraction before anything is built (M1).
    Convergence: legacy pair 3/5 (subtraction P4, restructure P5, patterns P3 step 1); D1/D2/D4 + `shadow_classify_recovery` measured by one evaluator each and INDEPENDENTLY RE-VERIFIED by synthesizer grep over `crates/` and over `bins/` this session (definition-only hits in each root)
    Evidence: `SyncManager::handle_new_block` (`block_lifecycle.rs:410`) + `ReorgHandler::check_reorg` (`reorg/mod.rs:287`, hardcodes weight=1 — a fourth weight unit) — 0 production callers; third ladder classifier `ForkAction`/`recommend_action`/`recommend_fork_action` (`types.rs:668`, `block_lifecycle.rs:528`) — callers are the dead wrapper + 6 tests; `SEED_CONFIRMATION_DEPTH` (`constants.rs:241`) — zero reads; `RecoveryEvidence::DeepForkSuspected` (`recovery.rs:89`) — no production reporter; `shadow_classify_recovery` (`block_lifecycle.rs:594`) — 0 callers, contains an already-drifted duplicate `RecoveryContext` construction.
    Confidence: conf(0.85, converged)
    What changes: −5 symbols, ~−120 LOC, −1 reorg entry point (5→4), ladder classifiers 3→2, finality depths 2→1; `specs/engine-parts.md:772` corrected in the same change (C5 — deleted symbols must not survive as spec claims); restores `graphify explain` on `handle_new_block` (name ambiguity removed). Precondition: `v_regression_map` query for the ~9 tests to be deleted; any invariant-linked test gets a written successor (REQ-FORK-005). Caveat recorded: deleting `DeepForkSuspected` removes the only non-heuristic route into `deep_fork_confirmed`; re-adding is ~5 lines if a future design wants an explicit deep-fork signal.

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (dead code never executes; −1 VecDeque scan per classify tick is below measurement) (observed)
      Memory:   0 (no runtime allocation existed) (observed)
      IO:       N-A (no syscall path touched)
      Network:  N-A (no wire shape touched)
      Disk:     0 (binary shrinks marginally) (observed)
      Latency:  0 (off every executed path) (observed)
    Inevitability: INEVITABLE
    Cheaper alternative: NONE-EXISTS
    Why this proposal anyway: dead code cannot be cheaper than deleted; its stale doc-comments (12 references to a function that does not exist) actively misdirect readers of the live ladder, and one dead constant already cost a requirement (REQ-FORK-018) written against a mismatch that does not exist.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: Every rung of the recovery ladder terminates or returns None — achieved by deleting non-acting rungs and adding exactly one named, non-lossy, alarmed terminal that ACQUIRES EVIDENCE and never decides a branch (M3).
    Convergence: 4/5 (subtraction P2/R1, patterns P4, failure P1 filter + §3.2 terminal census, radical absorbing-state derivation; restructure cross-signal confirms the comment-only rung)
    Evidence: `HeaderFirstSync`'s dispatch arm is comment-only (`periodic.rs:838-854`) yet arms a 30s cooldown via `record_action` (`recovery.rs:488`, Gate 2 at :318-323) — the absorbing state is actively maintained; `GenesisResync` is refused by its own Gate 1 whenever `confirmed_height_floor > 0` (`production_gate.rs:687-714`); Rule 1 returns `None` on guard refusal (no cooldown → INC-I-143's 454-hot-refusal shape) while Rule 1b returns bounded `SiblingFetch` (`recovery.rs:344-366` vs `:384-424`) — the INC-I-143 fix was applied to one of two copies of the same decision; measured: 380 recovery signals, 6 correct detectors, 3 state changes, 0 non-lossy terminals (failure §3.2).
    Confidence: conf(0.85, converged)
    What changes: the classifier/dispatch CONTRACT changes — an action that cannot act returns `None` and never consumes the cooldown (subtraction C3); Rule 1's refusal path unified with Rule 1b (one decision, one copy); `GenesisResync`'s reason re-pointed (transitional, see Migration); the producer/seed `shallow_rollback_count` asymmetry fixed or documented (budgets that do not count make rungs look live in review while dead in production); `rollback.rs:194`'s success-that-mutated-nothing returns a distinct outcome; new terminal `RecoveryAction::Wedged{reason}` — named, metric-labelled, non-lossy, whose action is to fetch the competing branch's headers/bodies by hash and let existing validated fork choice decide (failure B-F1: a terminal that DECIDES a branch is a consensus change in a recovery costume). Acceptance: C-6 exhaustive cell enumeration (gap band × tip-vs-finality × each budget) with a terminating rung named for every cell; INV-SYNC-006 gains its first regression tests.

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      ~0 (same 30s-paced classifier; fewer arms evaluated) (observed)
      Memory:   +1 enum variant, +reason field (observed)
      IO:       0 (observed)
      Network:  −the 1Hz sibling re-evaluation storm (measured 11 identical evaluations/10s on n6); +1 bounded header/body fetch per wedge event — replaces a full state snapshot (measured, failure §8 cost block)
      Disk:     −large log volume (the 1Hz repeat line dies); no snap-induced holes on this path (measured)
      Latency:  −wedge duration from 479.97s-then-lossy to fetch-and-validate seconds (measured baseline)
    Inevitability: AVOIDABLE
    Cheaper alternative: leave the ladder and let nodes degrade to the gap=50 snap exit, as today.
    Why this proposal anyway: the cheaper alternative IS the measured defect — recovery-requires-getting-worse with a lossy-only exit; and the fix here is net subtraction (non-acting rungs removed) plus one variant, not a seventh primitive.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: Retraction of applied state gets exactly one door with a callee-side authority contract; the production error arm may retract only a tip it itself created; a replacement escape ships FIRST and is drill-proven (M4a → M4b).
    Convergence: ordering 5/5; poison-arm containment 4/5 (restructure P1, patterns P5, failure §4.1/P2, radical P3) + the diagnosis synthesizer's SSF verdict; operator-escape mechanism 2/5 (patterns P5b, radical gap item 3)
    Evidence: `rollback_one_block` doc: "No preconditions — it just rolls back" (`rollback.rs:10`); exactly 2 production callers with OPPOSITE guard regimes (`periodic.rs:831` guarded+counted vs `production/mod.rs:630` unguarded+uncounted — grep-verified by two evaluators); measured [E3]: n1 published `b39d350f` at 22:44:09 and retracted that very block at 22:44:59; the same door rescued 13/27 mainnet nodes in INC-I-190 (INV-PROD-002's caveat); restructure P1 kill test: no observed case where the poisoned block became the tip.
    Confidence: conf(0.90, converged)
    What changes: M4a — audited operator RPC `forceReorgTo(hash)`: permissioned, logged, target corroborated by ≥2/3 local-ProducerSet weight (F7-safe), authorization EXPIRING/restart-scoped (patterns C9 — INC-I-196 self-brick filter); then the C-12 drill (recorded testnet recovery from `tip==finality ∧ 0<gap<50 ∧ sibling-exhausted`, ABOVE h=80,700 per T10, no snap, no poison bypass). M4b — `rollback_one_block(authority: RollbackAuthority)` with `CoordinatorApproved{depth}`, `ReorgPlan{ancestor}`, `WedgeEscape{fork_tip}` (today's bypass verbatim — C-3: renamed, never removed in the first change), `ProductionSelfApply{failed_height}` permitted iff `local_tip == failed_height`; otherwise the error arm purges toxic txs and keeps the published tip; the finality marker is never cleared as a production-failure side effect.
    INV-12 adjudicated: RETENTION class — Q1 YES, Q2 YES, Q3: no block is judged differently by any binary; NO activation height (radical + patterns + SSF verdict vs restructure's hedged strict reading; mixed fleet converges TOWARD the branch the 13 non-producers already hold). Restructure's unstated Q4 recorded: post-change nodes never select a different canonical tip from identical inputs — they merely stop retracting one.

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      −one full rollback (undo replay + state-root recompute) per poison event; +one height compare per rollback decision (measured event rate: 2 per 30-min window during incident, ~0 steady state)
      Memory:   0 (enum is 2 words by value) (observed)
      IO:       −one RocksDB undo batch write + re-apply cycle per poison event (observed)
      Network:  −the fleet-wide re-gossip/re-sync storm per poison event (measured: 9-node wedge, 414:3 thrash); +nothing steady state
      Disk:     −undo churn per event; +escape-RPC audit line (observed)
      Latency:  −50s of chain stall per event (measured 22:44:19→22:45:09); +2 release cycles to full enforcement (the deliberate price of REQ-FORK-012)
    Inevitability: AVOIDABLE
    Cheaper alternative: the SSF one-liner alone (wrap `production/mod.rs:630` in `if local_h == failed_height`), no enum, no escape RPC.
    Why this proposal anyway: the one-liner fixes the caller that defected today and leaves the "no preconditions" contract for the next caller to defect against (4 incidents argue one will); and without the shipped-first escape, closing the bypass leaves an all-wedged fleet with zero exits — the INC-I-190 outcome inverted.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: One fork-choice authority over real heights with an ancestry finality guard and derived finality — the single CHOICE-class change, behind ONE new activation height, shipped LAST (M5).
    Convergence: 5/5 on "the final CHOICE change is AH-gated and is the class-ending move"; mechanism composed from restructure P2+P4, patterns P1+P3 steps 2-3, radical P4; the finality-mirror deletion inside it is 4/5
    Evidence: `plan_reorg` pre-AH synthetic branch (`reorg/mod.rs:515-548`, own comment documents H_syn = H_real − I); `check_reorg_weighted`'s ungated comparison (:373-386, verified this session — Contradiction 1); `record_fork_block` no-height writer (:155, verified — Contradiction 2); `FinalityTracker` monotone with no lowering API + zero-caller `is_at_or_below_finalized` (`finality.rs:191`); radical kill test: all three AH-free formulations are consensus-visible — U1 is irreducibly AH-gated; the persistent canonical index already exists (`block_store/queries.rs:66/84`, `set_canonical_chain`) — the authority is a REWIRING plus deletion of the ephemeral shadow, not new construction.
    Confidence: conf(0.85, converged) on shape and gating; conf(0.7, converged) on the composed mechanism
    What changes (one change, one new height in `NetworkParams`, constant gate, NEVER HardForkSchedule, NEVER reusing/moving `inc_i_147_activation_height`): (a) `record_fork_block` gains `real_height`; (b) `check_reorg_weighted` + `plan_reorg` collapse to one `fork_choice` whose heights come from the block-store height index only; (c) finality admissibility becomes ancestry — the candidate branch must CONTAIN the finalized hash (height kept as cheap pre-filter; unit mixing becomes unrepresentable; the hash is already in `FinalityCheckpoint` and is being discarded); (d) the finality mirror F2/F3 is deleted, replaced by derived `effective_finality = min(tracker.last_finalized, local_tip)` and by `is_at_or_below_finalized` wired into the single rewind door — HARD PRECONDITION: M4b live + drill-proven (Contradiction 4); (e) a unit distinction (newtype or property test) covering the wedge_escape tie-break too (failure P3: the 19-line comment narrated the bug it sits above). Deploy: rolling binary (gate off = byte-identical, INV-8 not triggered — stated per C-11); activation is height-synchronized; fleet-readiness check + canary per Contradiction 5.

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      +O(fork_depth) hash compares per reorg evaluation (walk already exists, bounded MAX_REORG_DEPTH=1000); +1 RocksDB point read per fork-choice height resolution (inferred, UNBENCHMARKED — named risk, resolve before cut-over)
      Memory:   −the shadow height bookkeeping (~10,000 × 48B ≈ 480KB ceiling) net of a retained bounded fork-parent map (DoS guard, patterns C7); +32B finality hash beside the marker (measured struct shapes)
      IO:       +1 point read per fork-choice call (~1 per 10s slot + siblings); 0 on steady-state apply (observed shape)
      Network:  0 (every input already received) (observed)
      Disk:     −large in the tail: each avoided lossy escape avoids a full state replacement and a permanent body hole (measured: 49 blocks × 6 nodes in INC-I-204) 
      Latency:  +sub-ms per reorg decision against a 10s slot; −minutes-to-days on the wedge tail (inferred from measured baselines)
    Inevitability: AVOIDABLE
    Cheaper alternative: stop after M4 (Path-B-only) — class reduced (0 lossy escapes, 1 named terminal) but not ended: A1/A3 still disagree and A1 stays ungated.
    Why this proposal anyway: the disagreement between plural authorities IS the class (the analyst's genealogy: every fix added a guard, none removed an authority; Pillar 2 proved authority-consolidation-behind-a-constant-gate is the cure for exactly this sentence shape); stopping after M4 must be recorded as STOPPING, not as an alternative design.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Recommended Changes (Medium Convergence)

- ARCHITECTURAL: Branch-aware peer selection — route the EXISTING branch classification into `best_peer`, with a mandatory unfiltered fallback (M2).
    Convergence: 2/5 (restructure P3 conf 0.7 via wiring analysis; patterns P2 conf 0.7 via Bitcoin `pindexBestKnownBlock` + measured [E5]) + the diagnosis verdict's defense-in-depth fix #4 — independent evidence chains
    Evidence: `best_peer`'s whole predicate is `status.best_height > local_height && !blacklisted` (`decision.rs:88-89`) while `status.best_hash` sits one field away ignored; `checkpoint_health()` already classifies every peer against OUR canonical ring buffer (F7-safe — a local quantity no adversary can move) with 4 tests; `best_hash` is already on the wire (`protocols/status.rs:121`) and already stored (`types.rs:244`) — zero wire cost; measured consequence of blindness: 58.8% losing-branch re-draw odds, 414:3 sync-epoch thrash.
    Confidence: conf(0.80, converged)
    What changes: partition eligible peers agreeing/divergent via the `recent_canonical_hashes` comparison (NEVER `majority_best_hash` — peer-count majority is Sybil-advanceable, F7); prefer agreeing; FALL BACK to the unfiltered set when the agreeing set is empty (this clause is mandatory — without it the proposal is trap T4: a wedged node's tip matches no useful peer and selection deadlocks); INC-I-014 shuffle + INC-I-017 seeding applied within the preferred partition. Honestly bounded: a wedged node's "agreeing" set IS the losing branch — M2 is necessary, not sufficient; it pairs with M3's evidence-acquiring rung. INV-12: Q1 NO, Q2 NO (sourcing only; arrival order is already nondeterministic) — no AH, rolling-safe (precedent: INC-I-014/017 changed `best_peer` with no AH). INV-SYNC-005 (deferred since 2026-05-18) re-opened.

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      +O(peers) ring-buffer lookups per sync-source pick (~17-30 peers; checkpoint_health already does this every tick) (observed)
      Memory:   0 (ring and best_hash already allocated) (observed)
      IO:       0 (observed)
      Network:  −moderate: removes repeated empty-header round trips to wrong-branch peers (measured 414:3 thrash) 
      Disk:     0 (observed)
      Latency:  +microseconds per pick on a path that already awaits network IO; −large on wedge recovery (converging header loop) (inferred)
    Inevitability: AVOIDABLE
    Cheaper alternative: lengthen the `header_blacklisted_peers` cooldown so wrong-branch peers stay excluded longer.
    Why this proposal anyway: the blacklist is CLEARED on every sync-complete transition (`block_lifecycle.rs:154`) — learned branch knowledge is discarded exactly when the node re-enters the re-infecting population; tuning a deliberately-flushed cache cannot substitute for a missing predicate.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: Snap sync stops being reachable as a FORK remedy — admission narrows to bootstrap (h=0), genuine behind-ness (gap ≥ 500), and operator flag; the fork-reachable triggers are deleted (M6, drill-gated).
    Convergence: 2/5 (radical P1 conf 0.65; patterns S4/A8/C1 — snap-as-bootstrap is the uniform industry shape, and the 50..500 capability hole is the geometry of the cliff); failure analyst constrains rather than opposes (C-16: the defect is snap being the ONLY terminal; LB-5 bootstrap/gap≥500 admission preserved)
    Evidence: 3 lossy escapes reachable by a full-history node (Rule 2's three triggers `recovery.rs:447-454`, Rule 4, `dispatch.rs:96-98` empty-headers funnel at gap≥50); every one sets `store_floor` (`fork_recovery.rs:740`) with no automatic repair; all six INC-I-204 wedged nodes took the lossy exit SIMULTANEOUSLY at gap=50 (t_escape deterministic → correlated fleet event, failure signal 5); the operator opt-out already exists and is currently defeated by the Gate-4 "emergency" bypass (`production_gate.rs:679`).
    Confidence: conf(0.70, converged)
    What changes: delete `deep_fork_confirmed` as a snap trigger and the `dispatch.rs` gap≥50 funnel; keep bootstrap + genesis-window + `gap ≥ 500` behind-ness admission unchanged (REQ-FORK-004, C-16, 23 INV-SYNC-011 tests stay green — LB-6 attempts-never-reset untouched, exonerated by measurement); delete the Gate-4 emergency bypass of the operator's explicit `--no-snap-sync`. HARD PRECONDITION: ships only after M3's terminal + M4a's escape have passed the C-12 drill — deleting the measured cell's only exit before its replacement is proven is strictly worse than today (radical's own kill test). INV-12: SOURCING — refusing to install a snapshot changes no block's validity; no AH, rolling.

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      −classifier composite evaluation per 30s tick (observed shape)
      Memory:   −SnapSyncState field pressure (estimate)
      IO:       −large per avoided escape: a snap install rewrites the entire state; a header walk writes nothing (inferred)
      Network:  −O(UTXO set) bytes per avoided snapshot transfer; +O(fork_depth × header) for the walk (inferred)
      Disk:     −permanent body holes cease being manufactured (measured baseline: 49-block hole × 6 nodes)
      Latency:  +minutes worst-case for deep-fork header-walk-and-apply vs a snapshot jump — the deliberate trade (inferred)
    Inevitability: AVOIDABLE
    Cheaper alternative: keep snap as a fork remedy and add the automatic post-snap backfill task (Option O3, ~120 lines) that refills the hole afterwards.
    Why this proposal anyway: the backfill repairs the SYMPTOM (the hole) while snap-as-fork-remedy remains a branch ADOPTION decided by whichever peer answered — the exact unowned CHOICE this redesign exists to end; O3 is still independently worthwhile as a transition aid.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Options for User Decision

**O1 — Corroborated automatic finality retraction** (radical P2) — LOW-EVIDENCE, conf(0.55, inferred). If peers holding ≥2/3 of local-ProducerSet weight advertise a tip whose chain does not contain my finalized hash, retract the finality MARKER (never blocks) and let fork_choice re-decide; otherwise refuse, alarm, hold full history. Requires +32B STATUS field (additive/optional; `MIN_PEER_PROTOCOL_VERSION` consideration; NEVER `CURRENT_PROTOCOL_VERSION` — d.27/INV-4). The Pareto argument (fires only where the old binary emits `None`) is UNPROVEN — if the enumeration fails, this becomes AH-class. Alternative already in the plan: M4a's operator RPC covers the all-wedged case manually. Decide after M4's drill data exists.

**O2 — AddBond cap at mempool/builder admission** (failure P4) — LOW-EVIDENCE, conf(0.6, measured but kill test NOT RUN: whether the cap is computable pre-apply was not read). Owned by INC-I-203, not this redesign. Net-negative resource ledger per event (−5 block builds, −3+ rollbacks, −50s stall). If the cap consults post-apply state, admission-side rejection may be impossible or consensus-visible (own AH).

**O3 — Automatic post-snap backfill task** (radical signal 5) — LOW-EVIDENCE, conf(0.65, measured refs). ~120 lines reusing `backfill.rs:76` + `archiver.rs:292`; converts [E7] permanent history loss into temporary loss regardless of M6's fate; removes "operator wipe+rsync" from the runbook. Independently valuable; can land in any milestone.

**O4 — Time-windowed NON-DESTRUCTIVE budgets** (patterns P4 part 1) — LOW-EVIDENCE, conf(0.65, measured). Decay `sibling_fetch_attempts`/`shallow_rollback_count` on elapsed time. HARD FILTER: applies to non-destructive budgets ONLY — time-based snap re-arm in the 50..500 band is DEAD (kill test found: recreates INC-I-138 D4, snap at gap=28, blocks 37-63 lost). If chosen, INV-SYNC-011's intent is preserved as a rate bound with a written successor (REQ-FORK-005).

**O5 — GenesisResync variant deletion (step 2)** — conf(0.65, measured, one corner unproven). `SnapSync ≥ GenesisResync` at every evaluated input, EXCEPT Rule 4 has no `peer_count` precondition where Rule 2 requires ≥3 — the `peers < 3` corner is unresolved (does `request_genesis_resync` re-check peers downstream?). M3 ships the safe transitional re-point; delete the variant only after this corner is answered.

## Constraints (from the Failure Analyst — filters every choice above already passed)

**Regression traps T1-T12** (each is a plausible fix that breaks a named invariant — the migration tests against them): T1 relax finality guard to `<=` (breaks INV-SYNC-001/004/008; the 33 refusals were CORRECT — the guard is the hero); T2 poison-arm fix before `plan_reorg`/escape ordering (recreates INC-I-190); T3 attester-weight authority (F7/INC-I-191); T4 hash-match-only `best_peer` (0 eligible sources measured — M2's fallback clause exists because of this); T5 lower MINOR_FORK_GAP_MAX (makes the LOSSY exit faster: t_escape closed form); T6 raise/reset SNAP_ATTEMPTS_MAX (fixes a non-cause — attempts measured 0); T7 bundle rollback consolidation with door removal (collapses the two-release ordering); T8 let the rollback budget exhaust into snap (makes the lossy terminal reachable); T9 change the `fork_w <= our_w` tie-break ungated (forks the fleet at the first tie — measured tie 10390); T10 validate on testnet below h=80,700 (exercises the pre-AH branch mainnet no longer runs); T11 delete wedge_escape/SiblingFetch before C-6 passes (recreates INC-I-143's 454-refusal livelock); T12 clear finality "to re-derive cleanly" (INV-FINALITY-001's exact violation).

**Hard filters:** F7 — no bound advanceable by a ≥threshold holder (kills peer-count corroboration everywhere); d.29 — nothing may depend on `rebuild_epoch_state_from_blocks`; C9 — any operator escape expires or is restart-scoped (INC-I-196 shape); Pillar-2 — constant/NetworkParams gates only, NEVER HardForkSchedule; d.28/C-23 — no crossed AH moved, `inc_i_147_activation_height` never reused; d.27/C-25 — no `CURRENT_PROTOCOL_VERSION` bump; C-22 — every change works in a mixed old/new fleet or is AH-gated past full rollout (~30 external producers, no stop-all); C-24 — anything touching `plan_reorg` must be validated on testnet ABOVE 80,700; gap must not appear in any fork decision of the successor design (radical constraint 7); exactly one absorbing state survives, and it is named `Wedged`/`FinalityConflict`, alarmed, non-lossy, exitable by corroboration or audited operator action (radical constraint 6).

**All 10 load-bearing behaviors preserved:** LB-1/LB-2 guard + strict `<` copied verbatim (strengthened: moves inside the single door); LB-3 vindicated (no AH-free fork-choice fix exists); LB-4 replaced-not-removed, third in order; LB-5 snap preserved for bootstrap/behind-ness; LB-6 untouched (exonerated by measurement); LB-7 preserved (promoted to the walk bound `MAX_REWIND_DEPTH`, same value); LB-8 `AwaitingCanonicalBlock` deliberately NOT merged into fork_choice; LB-9 superseded-with-successor only after C-6 (T11); LB-10 unchanged. No failed approach re-proposed (INC-I-176 M2.5 wire-break shape avoided: nothing here touches block/header/tx shape; O1's STATUS field is peer-handshake, additive).

## Architecture Maps

### Current (measured)
```
13 branch authorities: A1 check_reorg_weighted (ungated, per-process heights) · A2 check_reorg (dead)
· A3 plan_reorg (AH-gated) · A4 recovery ladder (gap bands, 8 thresholds) · A5 best_peer (height-only)
· A6 snap admission (5 gates) · A7 BLOCK_POISON arm (unguarded retract) · A8 AwaitingCanonicalBlock
· A9 dispatch.rs empty-headers funnel (3rd gap formula) · A10 wedge_escape · F1 FinalityTracker (truth,
monotone) · F2 ReorgHandler.last_finality_height (erasable mirror) · F3 clear_finality forwarder.
bins/node has WRITE-ONLY access to finality (the erase); the read predicate has 0 callers.
Branch identity is computed twice (checkpoint_health, majority_best_hash) and routed only to paths
that destroy history (snap target) or write a label (checkpoint tag). 2 absorbing states, unnamed,
silent. 3 lossy escapes reachable by a full-history node.
```

### Proposed (after M6)
```
fork_choice(local: TipRef, cand: TipRef) -> Keep | Switch{ancestor} | NeedData{from}
    TipRef = (real_height from block-store index, cumulative_weight, hash); admissible iff the
    candidate branch CONTAINS the finalized hash. One implementation, gossip + recovery + escape.
converge(): candidates from branch-aware best_peer (M2) -> backward header walk to a held hash
    -> rewind_to(ancestor) + apply forward | ancestor < finality -> Wedged/FinalityConflict
    (named, alarmed, non-lossy, exits: corroboration [O1] or audited forceReorgTo [M4a]).
rewind_to(): the ONE door; finality check INSIDE (is_at_or_below_finalized + derived
    effective_finality = min(tracker, local_tip)); callers declare RollbackAuthority.
FinalityTracker: the ONE store. No mirror, no forwarder, no erasure function.
snap_sync: bootstrap (h=0) | gap >= 500 behind-ness | operator flag. Never a fork remedy.
Kept separate by design: AwaitingCanonicalBlock production gate (LB-8); SiblingFetch until
REQ-FORK-019's enumeration passes.
```

## Migration Path

Order is forced by REQ-FORK-012 + Contradiction 4, not by preference. Each milestone states INV-12, deploy mode, invariants, and the traps its tests must include.

| M | Content | INV-12 / AH | Deploy | Invariants touched | Traps tested |
|---|---|---|---|---|---|
| **M0** | Observability + canaries: FORK_GUARD site labels; `unique_chain_tips` alarm; wedge_escape refusal-reason split; bounded `None`-state logging; pre-AH canary (armed BEFORE M5); `check_reorg_weighted` comparison probe; INC-I-187 export-path verification | Q1 N / Q2 N / Q3 identical → **no AH** | rolling, ships alone | none touched; INV-SYNC-012 gains its first canary (REQ-FORK-014) | replay INC-I-204 signature → alert in minutes (REQ-FORK-016); T10 noted for all later validation |
| **M1** | Delete 5 dead symbols (D1-D4 + `shadow_classify_recovery`); correct `specs/engine-parts.md:772` in the same change; `v_regression_map` check before deleting ~9 tests | unreachable code, Q3 trivially YES → **no AH** | rolling, any order | none linked (verify); REQ-FORK-005 successor if any test is invariant-linked | none applicable |
| **M2** | Branch-aware `best_peer` (checkpoint_health formulation + mandatory unfiltered fallback); INV-SYNC-005 re-opened | Q1 N / Q2 N (SOURCING) → **no AH** | rolling | INV-SYNC-005 (re-open), INV-SYNC-009; INC-I-014/017 properties re-proven; INV-SYNC-011 untouched | **T4** (the naive form is the trap), T10 |
| **M3** | Ladder termination by subtraction + `Wedged{reason}` evidence-acquiring terminal; Rule 1/1b unification; C3 contract (no cooldown for non-acting rungs); counter asymmetry fixed; `rollback.rs:194` distinct outcome. BRIDGE: `periodic.rs:862` reason re-pointed to `CoordinatorSnapEscalation` — transitional until O5 deletes the variant; must be paired with proof the floor bypass admits no backward snap for Rule-4's input set | recovery-action timing only, Q1 N / Q2 N → **no AH** | rolling | INV-SYNC-006 gains first tests (REQ-FORK-010); INV-FORK-001; LB-9 preserved; INV-SYNC-011 untouched | **T1, T5, T6, T8, T11**; C-6 cell enumeration is the acceptance gate |
| **M4a** | `forceReorgTo(hash)` operator escape: permissioned, audited, ≥2/3 local-ProducerSet-weight corroborated target, EXPIRING authorization (C9). Then the **C-12 drill**: recorded testnet recovery from `tip==finality ∧ 0<gap<50 ∧ sibling-exhausted`, above h=80,700, no snap, no poison bypass. BRIDGE: the poison door stays open through this release — deliberately; release 1 is node-locally self-sufficient (failure P2 kill test) | operator RPC, node-local → **no AH** | rolling (release 1 of 2) | INV-PROD-002 sequencing satisfied by construction | T2 (this milestone IS its answer), T10 |
| **M4b** | `RollbackAuthority` callee-side contract; poison arm retracts only if `local_tip == failed_height`, else purge + keep tip; finality never cleared as a side effect. GATED ON the M4a drill passing | RETENTION: Q1 Y / Q2 Y / Q3 no block judged differently → **no AH** (adjudicated, 3 evaluators + verdict) | rolling (release 2 of 2) | INV-PROD-002 (enforcement), INV-FINALITY-001 (strengthened), INV-SYNC-001/004/008 verbatim, INV-CONSENSUS-089 untouched | **T2, T7, T12**; FAIL→PASS reproduction test first (Output Contract) |
| **M5** | ONE new AH in `NetworkParams` (constant gate; above both tips with auto-update convergence margin): `record_fork_block` real_height; `fork_choice` unification (deletes the ungated `check_reorg_weighted` site); ancestry finality guard; finality mirror deleted → derived `effective_finality` + `is_at_or_below_finalized` wired into the single door; unit newtype/property test incl. wedge_escape tie-break. PRECONDITION: M4b live + drill-proven. Fleet-readiness: `getForkChoiceVersion` + guardian check before AH; canary already armed (M0) | **CHOICE: Q1 N / Q2 Y / Q3 NO → AH REQUIRED** — the only gated milestone; not a block-content change → INV-8 synchronized deploy NOT triggered (stated per C-11) | rolling binary; height-synchronized activation | INV-SYNC-012 (enforcement + canary), INV-SYNC-002, INV-SYNC-004 superseded-with-successor (written FIRST), INV-FINALITY-001 strengthened, INV-SYNC-011 verbatim | **T3, T9, T10**, C-24 (validate post-AH on testnet); benchmark the point-read before cut-over |
| **M6** | Snap fork-reachability deleted (F7 entry above); `execute_reorg` undo loop consolidated onto the single door (REQ-FORK-017; C-2: the executor currently has no finality check of its own); SiblingFetch retirement ONLY after C-6 green (REQ-FORK-019); O5 if its corner is answered; O3 backfill optional | SOURCING/dead-code → **no AH** | rolling | INV-SYNC-015 (undo/legacy preserved), INV-SYNC-011 verbatim | **T7, T11** |

Every milestone that touches consensus-adjacent code re-answers the three questions in its commit message (REQ-FORK-006), and every gated behavior states its dormant window + canary (REQ-FORK-014).

## Complexity Comparison

| Metric | Current (measured) | Proposed (after M6) | Radical minimum |
|--------|---------|----------------|----------|
| Distinct branch authorities | 13 (A1-A10, F1-F3) | 4 (fork_choice, converge, FinalityTracker, + SiblingFetch until REQ-FORK-019) | 3 |
| Fork-choice planners | 2 + 1 dead wrapper | 1 | 1 |
| Finality stores | 3 + 2 depths | 1 + 1 depth (D1 deleted in M1) | 1 |
| Rollback doors / guard regimes | 4 primitives, 2 implicit regimes | 1 door, 4 named authorities | 2 primitives |
| RecoveryAction variants | 6 (2 non-acting) | 5 (−GenesisResync, −HeaderFirstSync-as-cooldown-sink, +Wedged) | 3 |
| Lossy escapes reachable by a full-history node | 3 | 0 | 0 |
| Absorbing states | 2, unnamed, silent | 1, named, alarmed, non-lossy | 1, named |
| Named ladder thresholds | 8 + local consts | ~6 | 3 + MAX_REWIND_DEPTH |
| Non-test decision-surface lines | ≈6,285-6,500 | ≈2,600 (estimate) | ≈1,900 (estimate) |
| New activation heights consumed | — | 1 | 1 |
| INV-SYNC-012 canaries / INV-SYNC-006 tests | 0 / 0 | ≥1 / ≥1 | ≥1 / ≥1 |

**SSF GATE (radical tiebreaker, applied):** the radical minimum's end-state is bounded by its weakest required member — the automatic escape (radical P2, conf 0.55, Pareto argument unproven) and the AH-gated unification (conf 0.6). The unified staged proposal carries conf(0.85-0.9, converged) on every definite milestone and reaches the SAME destination (the proposed column converges on the radical column; the only structural deltas are deliberate: LB-8 kept separate, SiblingFetch retired only after enumeration, operator escape instead of the conf-0.55 automatic one). Confidence gap ≈ 0.25-0.30 > 0.1 → the staged proposal is primary ON MERIT; the radical minimum is adopted as the destination architecture, not as the shipping plan. Simplicity is not sacrificed — the plan is ~70% deletion by line count.

## Milestones

M0 → M1 → M2 → M3 → M4a → (C-12 drill) → M4b → M5 (AH) → M6. Gates: M4b blocks on the drill; M5 blocks on M4b + fleet-readiness + benchmark; M6 blocks on M5 proven + C-6 enumeration. Stopping after M4 is a legitimate recorded decision ("class reduced, not ended"), per the failure analyst's B-F5 warning it must be recorded as stopping.

## Design Synthesis Quality Gate

```
━━━ DESIGN SYNTHESIS QUALITY GATE ━━━
Evaluators completed:           5/5
Deletion convergence items:     4 (3+/5: legacy pair, finality mirror, poison retract call, non-acting rungs)
Restructuring convergence:      3 (single door w/ authority, fork_choice unification, best_peer rewiring)
Addition options presented:     5 (O1-O5)
Failure modes identified:       22 (A-F1..5, B-F1..5, T1-T12) from Failure Analyst
Failure modes applied as filters: 22/22 (T4 reshaped M2; T9/T3 shaped M5; T2/T7/T12 shaped M4; T1/T5/T6/T8/T11 shaped M3; B-F1 shaped the terminal; C9/F7 shaped M4a)
Radical floor gap:              13 authorities → 3 (radical) → 4 (proposed); ~6,500 → ~1,900 → ~2,600 lines
Contradictions found:           9 (6 mandated + 3 minor)
Contradictions resolved:        9/9 (1 residual live measurement recommended: check_reorg_weighted comparison frequency, probe ships in M0)
Evidence independence verified: YES (per-cluster checks in Convergence Matrix; synthesizer re-grep on all zero-caller claims)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

*Synthesis of evaluator reports only; no evaluation performed by the synthesizer beyond verification reads. Reasoning trace: `docs/.workflow/architecture-reasoning.md`.*
