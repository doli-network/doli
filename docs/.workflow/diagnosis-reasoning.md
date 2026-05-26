# Diagnostic Reasoning Trace: INC-I-090 Observability Gap

## Conclusion-First Protocol Status

No `report-conclusion-*.md` files existed. Fallback mode: read all 5 full investigation reports directly. To counter sequential anchoring, I read all 5 reports before forming any conclusions, then verified key claims against actual source code.

## Investigator Reports Summary

### Investigator #1 (Log Forensics)
- **Role**: Textual logs and metrics
- **Constraint**: SSH to ai1 blocked; pivoted to git archaeology
- **Hypotheses**: H1 (recovery emit gap, conf 0.65), H4/H6 (no automated consumer, conf 0.55), H5 (wrong recommended_action, conf 0.60), H_SNAP (SnapSync emission never wired, conf 0.55), H7 (version skew, conf 0.30)
- **Key evidence**: Commit `259f6380` message ("emit when action is non-None"); commit `cbaa3963` (FINALITY_GUARD returns None); commit `1ffc5df8` (7 event types wired, SnapSync not included)
- **Gaps**: Cannot read N3 logs, cannot verify EMIT-007 condition in source, cannot verify SnapSync absence
- **Causal chain**: 11-step chain from fencepost through classification to user visual detection

### Investigator #2 (Code Logic)
- **Role**: Source code static analysis
- **Hypotheses**: H1 partial (conf 0.65, measured), H2 (conf 0.15), H3 (conf 0.70, measured), H4 (conf 0.55), H5 (conf 0.70, measured), H6 (conf 0.70, measured), H7 (conf 0.20)
- **Key evidence**: Complete emit-site audit (12 emit sites, 4 EventKinds with zero sites); `block_lifecycle.rs:626-630` ctx_for_emit gating; `classifier.rs:314-348` has_other_signals() correlation_key logic; `writer_stats.rs:21,31` events_dropped never incremented; Section 5 zero automated consumers
- **Unique finding**: events_dropped counter broken (DiagnosticWriterStats.events_dropped never written; emitter.dropped_count() never read by RPC). This was NOT found by any other investigator.
- **Gaps**: Cannot verify exact RecoveryClassifyCall count at runtime

### Investigator #3 (State Reconstruction)
- **Role**: Live state (SSH/RPC)
- **Constraint**: Pipeline gate blocked ALL probes; backward reasoning only
- **Hypotheses**: H1-sub (conf 0.50), H2 (conf 0.15), H3 (conf 0.55), H4 (conf 0.65), H5 (conf 0.55), H6 (conf 0.70), H7 (conf 0.25)
- **Key evidence**: Identified single most valuable probe (getForkDiagnostic on N3:8503); backward reasoning from "no alert fired" to compound L1+L4 failure
- **Gaps**: ALL live probes blocked -- this investigator produced the most explicit gap catalog

### Investigator #4 (Constraint Elimination)
- **Role**: Failed approaches, working cases, elimination matrix
- **Hypotheses**: H1-sub + H6 minimum sufficient (conf 0.65)
- **Key evidence**: Zero failed approaches in observability domain = "never attempted"; working cases prove components function individually (post-hoc investigation succeeded); cross-hypothesis consistency matrix showing H5 is derived from H1-sub
- **Unique insight**: "Feature shipped incomplete, not feature broken" -- the absence of failed approaches distinguishes design gap from implementation bug
- **Gaps**: Same SSH/RPC gaps as others

### Investigator #5 (Wildcard)
- **Role**: Lateral thinking, architecture, dead code, emergent behavior
- **Hypotheses**: 14 wildcards (W1-W14). Top 3: W5 pull-vs-push (conf 0.70), W9 TipRaceNatural pathway (conf 0.65), W11 dead code bridge (conf 0.60)
- **Key evidence**: Full worked example of classifier rule matching for INC-I-090 fork shape (Section 2); signal_stuck_fork()/take_stuck_fork_signal() dead code bridge (production_gate.rs:573,581); architecture mismatch diagnosis (pull-based forensic tool vs push-based monitoring)
- **Unique findings**: W11 (dead code bridge) provides archaeological evidence of design intent that was never completed; W5 reframes H6 as an architectural category error

## Convergence Analysis

### Primary convergence: H1-sub (recovery emit gap)
- **Converging investigators**: Log (#1), Code (#2), State (#3), Constraints (#4)
- **Independence verification**:
  - Log #1 based on: git commit messages (`259f6380`, `cbaa3963`)
  - Code #2 based on: direct source reading (`block_lifecycle.rs:626`, `periodic.rs:612`)
  - State #3 based on: backward reasoning from architecture blueprint
  - Constraints #4 based on: skill doc absence (no emit call documented for recovery.rs) + elimination matrix
  - **INDEPENDENT? YES** -- 4 distinct evidence sources (git history, source code, architecture docs, documentation gaps). True convergence.

### Primary convergence: H6 (no automated consumer)
- **Converging investigators**: All 5
- **Independence verification**:
  - Log #1 based on: Phase 2 deferral commit `5d1d83a7`
  - Code #2 based on: Section 5 RPC consumer audit (zero automated callers)
  - State #3 based on: architecture blueprint Smell 1
  - Constraints #4 based on: zero failed approaches in observability domain
  - Wildcard #5 based on: pull-vs-push architecture analysis (W5)
  - **INDEPENDENT? YES** -- 5 distinct evidence angles. Strongest convergence in the investigation.

### Secondary convergence: H4 (fork-monitor not deployed)
- **All 5 investigators** flagged this based on: no systemd unit in codebase, script documented as manual command, zero failed approaches.
- **Independence**: Partially independent. All 5 drew from the same absence (no systemd unit file), but the Constraints investigator added the "zero failed approaches" signal independently. The Wildcard added the W3 threshold analysis independently. Mixed independence -- boost applies but capped.

### Subsumption relationships identified:
- H5 (wrong recommended_action) is DOWNSTREAM of H1-sub. Not an independent root cause.
- H3 (classifier shape) is DOWNSTREAM of H1-sub. The classifier is correct given its inputs.
- W5 (pull-vs-push) is the STRUCTURAL explanation for H6. H6 is the concrete manifestation.
- H4 (fork-monitor not deployed) is a SPECIFIC INSTANCE of H6.

## Contradiction Analysis

**Zero contradictions found across all 5 investigators.**

This is unusual but explicable:
1. The bug is a structural absence, not a subtle interaction. There is no "competing implementation" to disagree about.
2. All investigators were working from the same constraint: SSH/RPC blocked, so none could produce measurements that might contradict others' inferences.
3. The compound failure (L1 + L4) was predictable from the architecture. The pull-based design + incomplete instrumentation = no alert is a straightforward deduction that all 5 investigators reached through different paths.

I considered whether the Code investigator's estimate of "~12-16 RecoveryClassifyCall events" could contradict the Log investigator's "253 iterations produce zero events." These are NOT contradictory -- they describe different time windows:
- Log #1: "253 iterations" refers to ALL iterations, most of which produce RecoveryAction::None (no emit)
- Code #2: "~12-16" refers to the subset that produce non-None actions (after 60s grace period expires)

The Code investigator explicitly reconciled this in Section 2 Step (d): "The 253 sync_fails counter is separate from the diagnostic emit count."

## Gap Analysis

### Evidence layer coverage:
- **L0 (Deploy)**: Inferred only. No live binary inspection. H7 likely refuted but not confirmed.
- **L1 (Emit)**: WELL COVERED by Code investigator (complete emit-site audit with file:line citations). Synthesis verified key claims against source code.
- **L2 (Persist)**: Partially covered. Ring buffer capacity and drain rate analyzed (Code #2). Broken canary identified (Code #2). No live health counter check.
- **L3 (Classify)**: WELL COVERED. Full 8-rule walkthrough by Code #2 and Wildcard #5 independently. Synthesis verified against `classifier.rs` source.
- **L4 (Surface)**: Covered by all 5 investigators from different angles. No live deployment verification (SSH blocked).

### Critical gap:
The single most valuable missing piece is **[M1]: live getForkDiagnostic RPC probe on N3**. This one call would resolve 4 hypotheses (H1-sub, H2, H5, H7) with measured evidence, lifting confidence from 0.82 to potentially 0.95+.

### Thin reports flagged:
State investigator (#3) explicitly stated "UNABLE TO PROBE LIVE STATE" and produced the thinnest report. This is expected and appropriate -- the pipeline gate blocked their primary evidence channel. Their backward reasoning from architecture docs was methodologically sound given the constraint.

## Confidence Evolution

1. **Individual assessments**: All 5 investigators individually rated H1-sub at conf(0.50-0.65) and H6 at conf(0.55-0.70).

2. **After convergence detection**: 4/5 convergence on H1-sub (independent evidence) -> boost to conf(0.85). 5/5 convergence on H6 (independent evidence) -> boost to conf(0.85).

3. **After synthesis verification**: I verified key code claims against source:
   - `block_lifecycle.rs:626-630` confirmed ctx_for_emit gating (MEASURED)
   - `periodic.rs:612` confirmed EMIT-007 conditional (MEASURED)
   - `recovery.rs:312` confirmed FINALITY_GUARD fencepost (MEASURED)
   - `classifier.rs:323-328` confirmed has_other_signals() correlation_key logic (MEASURED)
   - `writer_stats.rs:21,31` confirmed events_dropped never incremented (MEASURED)
   - `production_gate.rs:573` confirmed take_stuck_fork_signal() has 0 non-test callers (MEASURED)
   
   These verifications upgraded evidence basis from "inferred" to "measured" for the code-level claims.

4. **After compound assessment**: The compound H1-sub + H6 diagnosis has:
   - H1-sub: conf(0.85, converged) -- 4/5 independent convergence + synthesis code verification
   - H6: conf(0.85, converged) -- 5/5 independent convergence
   - Compound: conf(0.82, converged) -- slightly below individual because the compound claim requires BOTH to be true, and the product of independent probabilities is lower than either alone. Also, [M1] live probe is missing.

5. **PRELIMINARY, not VERDICT**: conf(0.82) < 0.95 threshold. The missing live RPC probe [M1] is the primary blocker. With [M1] data confirming the expected RecoveryClassifyCall absence and TipRaceNatural classification, confidence would likely reach 0.95+.

## Drift Between Skill/Docs/Code

1. **events_dropped counter**: The INC-I-087 fix (workflow 353-354, commit `954afc45`) was supposed to wire live counters into the RPC. The `events_written_total` was wired (via `DiagnosticWriterStats.events_written`). But `events_dropped_total` reads from `DiagnosticWriterStats.events_dropped` (always 0), not from `emitter.dropped_count()` (actually incremented). This is a partial fix -- the INC-I-087 resolution was incomplete. INV-OBS-001 ("must reflect live writer counters, never hardcoded literals") is violated for the dropped counter.

2. **SnapSync event types**: The schema defines `SnapSyncAttempted/Completed/Failed` and the classifier consumes them, but zero production code emits them. This is not a drift -- it is a documented Phase 2 deferral. But the classifier rules that depend on these events (rule (d) PostSnapDeadTip) are structurally dead without being marked as such.

3. **signal_stuck_fork bridge**: The `types.rs:520` documentation says "Consumed by take_stuck_fork_signal() in resolve_shallow_fork()." But `take_stuck_fork_signal()` has zero non-test callers, and `resolve_shallow_fork` does not appear to call it either. The docstring describes intended behavior, not actual behavior.

## Why PRELIMINARY, Not VERDICT

Three specific evidence items are missing:

1. **[M1] Live RPC probe**: Would confirm (a) actual RecoveryClassifyCall event count (measured vs estimated 12-16), (b) classification result (measured vs inferred TipRaceNatural), (c) health counters (measured vs inferred), (d) ledger availability (measured vs inferred).

2. **[M2] fork-monitor.sh deployment**: Would confirm or refute H4 definitively. Currently inferred from structural evidence.

3. **[M3] Binary version**: Would rule out H7 definitively. Currently inferred from timeline (5-day gap between ship and incident).

None of these gaps would likely CHANGE the diagnosis. The compound H1-sub + H6 failure is strongly supported by measured code evidence. But per evidence-floor protocol, citable measured evidence from the live system is required for VERDICT confidence. The synthesis is based on code-level measurements + architectural inference, not live system measurements.
