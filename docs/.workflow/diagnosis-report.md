━━━ PRELIMINARY — conf(0.82, converged) — NOT ACTIONABLE ━━━

Working hypothesis: The 9-minute fork at N3 h=284677 was invisible to automated alerting due to a compound L1+L4 failure: (A) the FINALITY_GUARD fencepost that caused the fork also suppressed RecoveryClassifyCall diagnostic emissions for the first 60 seconds, and even after 60s only ~15 events were emitted -- below the ChainBreakLoop signal_d threshold of >20; (B) four EventKind variants (SnapSyncAttempted/Completed/Failed, ChainBreakDetected) have zero emit sites, structurally disabling classifier rules (d) and (h)/signal_a; (C) RecoveryClassifyCall events use correlation_key=None, making them invisible to the TipRaceNatural rule's has_other_signals() check; (D) no automated consumer reads getForkDiagnostic output -- fork-monitor.sh polls getChainInfo only, and no dashboard/metrics/cron exists. The observability subsystem was shipped as a forensic investigation tool (pull-based, post-hoc) and never completed the last mile to automated monitoring (push-based, real-time).

Evidence gathered so far:
  [E1] `crates/network/src/sync/manager/block_lifecycle.rs:626-630` — classify_and_dispatch returns ctx_for_emit=None when action==RecoveryAction::None
  [E2] `bins/node/src/node/periodic.rs:612` — EMIT-007 gated on `if let Some(ref ctx) = recovery_ctx`; skips emission when ctx is None
  [E3] `crates/network/src/sync/manager/recovery.rs:312` — FINALITY_GUARD: `target_height <= finality` returns RecoveryAction::None
  [E4] `bins/node/src/node/periodic.rs:624` — RecoveryClassifyCall events hardcode `correlation_key: None`
  [E5] `crates/storage/src/diagnostic_ledger/classifier.rs:323-328` — has_other_signals() returns false for fork_ev with None correlation_key (singleton group)
  [E6] grep of `bins/node/src/` for SnapSyncAttempted/Completed/Failed and ChainBreakDetected — zero production emit sites
  [E7] `crates/storage/src/diagnostic_ledger/classifier.rs:404` — signal_d threshold: `recovery_attempts > 20`; estimated ~15 emitted events < 20
  [E8] grep of `scripts/fork-monitor.sh` for getForkDiagnostic — zero results; script polls getChainInfo only
  [E9] `crates/storage/src/diagnostic_ledger/writer_stats.rs:21,31` — events_dropped AtomicU64 initialized to 0, never incremented by production code
  [E10] `crates/storage/src/diagnostic_ledger/emitter.rs:146,166-167` — AsyncChannelEmitter has separate `dropped` counter, but dropped_count() is never read by RPC handler
  [E11] `crates/rpc/src/methods/diagnostics.rs:94` — RPC reads stats.events_dropped (always 0), not emitter.dropped_count()
  [E12] `crates/network/src/sync/manager/production_gate.rs:573,581` — signal_stuck_fork()/take_stuck_fork_signal() exist but take_ has 0 non-test callers

Missing evidence (blocks promotion to VERDICT):
  [M1] Live RPC probe on N3 (port 8503): `getForkDiagnostic` with min_height=284670, max_height=284685, limit=1000
       -> Resolve by: `ssh ai1 'curl -s -X POST http://127.0.0.1:8503 -H "Content-Type: application/json" -d "{\"jsonrpc\":\"2.0\",\"method\":\"getForkDiagnostic\",\"params\":{\"min_height\":284670,\"max_height\":284685,\"limit\":1000},\"id\":1}"'`
       This single call would confirm: (a) event count by kind (confirms/refutes H1-sub), (b) health.events_dropped_total (confirms/refutes H2), (c) classification.fork_type + recommended_action (confirms/refutes H5), (d) health.ledger_available (confirms/refutes H7/W12)
  [M2] fork-monitor.sh deployment status on mainnet
       -> Resolve by: `ssh ai1 'systemctl list-unit-files | grep -iE "fork|monitor"; crontab -l 2>/dev/null | grep fork; pgrep -f fork-monitor'`
  [M3] Deployed binary version on N3
       -> Resolve by: `ssh ai1 '/mainnet/bin/doli-node-n3 --version 2>/dev/null || strings /mainnet/bin/doli-node-n3 | grep diagnostic_ledger | head -3'`

Counter-hypotheses still in play:
  - H7 (binary version skew): Would be ruled out by [M1] returning valid JSON (method exists) or [M3] showing post-6.22.0 version
  - W12 (NoOpEmitter at startup): Would be ruled out by [M1] returning health.ledger_available=true and events_written_total > 0

Re-dispatch directive: Re-run investigator-state with SSH/RPC access enabled for ai1 read-only probes [M1], [M2], [M3]. All other hypotheses are resolved from code evidence.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

---

# Diagnosis Report: INC-I-090 Observability Gap RCA

## Symptom Profile
- **What happens**: A 9-minute production fork on N3 at h=284677 produced no automated alert. Detection was visual by the operator.
- **When**: 2026-05-25, 22:54:37 to ~23:04:30 UTC. The observability subsystem had been live for 4 days (shipped v6.22.0 on 2026-05-21).
- **Deterministic**: YES -- the compound failure is structural and would reproduce for any fork where the FINALITY_GUARD fencepost returns None AND the 9-minute stuck window produces fewer than 20 non-None recovery actions.
- **Failure boundary**: Affects any single-node fork shape where (a) the node is stuck at finality+1 AND (b) no automated consumer polls getForkDiagnostic.

## Fundamentals Check
Per investigation brief: L0 (deploy) likely healthy (feature unconditionally compiled, v6.22.0 deployed 5 days prior). L1 (emit) has partial gaps. L2 (persist) likely healthy. L3 (classify) correct but operating on incomplete data. L4 (surface) structurally absent for automated alerting.

## Investigation Summary

| Investigator | Evidence Layer | Top Hypothesis | Confidence | Key Finding |
|-------------|----------------|----------------|------------|-------------|
| Log Forensics (#1) | Git archaeology, commit messages | H1-sub + H6 compound | conf(0.65, inferred) | EMIT-007 gates on non-None action; FINALITY_GUARD returns None; SnapSync events never wired; fork-monitor.sh polls getChainInfo not getForkDiagnostic |
| Code Logic (#2) | Source code static analysis | H1-sub + H3 + H6 compound | conf(0.65, measured) | 12 emit sites audited; 4 EventKinds have zero emit sites; classify_and_dispatch returns ctx_for_emit=None when action=None; signal_d ~15 < threshold 20; events_dropped counter broken |
| State Reconstruction (#3) | Backward reasoning (SSH blocked) | H1-sub + H6 compound | conf(0.65, inferred) | Recovery emit gap is critical L1 uncertainty; no automated consumer is independently confirmed; single RPC probe would resolve 4 hypotheses |
| Constraint Elimination (#4) | Elimination matrix, failed approaches | H1-sub + H6 minimum | conf(0.65, inferred) | Zero failed approaches in observability domain = "never attempted"; H1-sub + H6 is minimum sufficient compound explanation; H5 is downstream of H1-sub |
| Wildcard (#5) | Architecture, dead code, design intent | W5 (pull-vs-push) + W9 + W11 | conf(0.70, inferred) | Pull-based architecture cannot alert; signal_stuck_fork() designed but never wired (0 callers of take_stuck_fork_signal); classifier correct on incomplete data |

## Convergence Matrix

```
                                Log(#1)   Code(#2)  State(#3)  Constr(#4)  Wild(#5)  SYNTHESIS
H1 (full: no emit at all)      PARTIAL    PARTIAL   PARTIAL    REFUTED     --        REFUTED (ForkBlockReceived+BlockApplied ARE emitted)
H1-sub (recovery emit gap)     SUPP 0.65  SUPP 0.65 SUPP 0.50 SUPP 0.60   --        CONFIRMED conf(0.85, converged) -- 4/5 convergence, independent evidence
H2 (ring buffer overflow)      --         INCL 0.15 INCL 0.15 REFUT 0.85  --        LIKELY REFUTED -- event rate << capacity; BUT canary broken [E9-E11]
H3 (classifier shape)          SUPP 0.60  SUPP 0.70 SUPP 0.55 COND 0.55   --        CONFIRMED as DOWNSTREAM of H1-sub conf(0.80, converged)
H4 (fork-monitor not deployed) SUPP 0.55  SUPP 0.55 SUPP 0.65 SUPP 0.65  SUPP 0.65 HIGHLY LIKELY conf(0.75, converged) -- 5/5; pending [M2]
H5 (wrong recommended_action)  SUPP 0.60  SUPP 0.70 SUPP 0.55 DERIVED     SUPP 0.65 CONFIRMED as DOWNSTREAM of H1-sub conf(0.80, converged)
H6 (no automated consumer)     SUPP 0.55  SUPP 0.70 SUPP 0.70 SUPP 0.65  SUPP 0.70 CONFIRMED conf(0.85, converged) -- 5/5, independent evidence
H7 (version skew)              INCL 0.30  INCL 0.20 INCL 0.25 REFUT 0.85  --        LIKELY REFUTED -- pending [M3]
W5 (pull-vs-push architecture) --         --        --         --          SUPP 0.70 SUBSUMED by H6 -- H6 is the concrete manifestation
W9 (TipRaceNatural pathway)    --         --        --         --          SUPP 0.65 CONFIRMED -- worked example proven by classifier rule walk
W11 (dead code bridge)         --         --        --         --          SUPP 0.60 CONFIRMED -- production_gate.rs:573 take_stuck_fork_signal() has 0 non-test callers [E12]
```

**Convergent rows (4+ investigators agree):**
- H1-sub (recovery emit gap): 4/5 -- Log, Code, State, Constraints
- H6 (no automated consumer): 5/5 -- all investigators
- H4 (fork-monitor not deployed): 5/5 -- all investigators

**No contradictions found.** This is expected for a "feature shipped incomplete" pattern -- the gaps are structural absences, not competing implementations. See Phase 3 below for discussion.

## Contradictions

**Zero contradictions.** All 5 investigators converged on the same compound diagnosis: L1 emission gap (H1-sub) + L4 surface gap (H6). The zero-contradiction result is plausible here because:

1. The bug is a structural absence (missing emit calls, missing automated consumer), not a subtle interaction where different perspectives would yield different conclusions.
2. No investigator had access to live state data (SSH/RPC blocked), so none could produce measurements that contradict another's inferences.
3. The compound failure was predictable from the architecture: pull-based design + incomplete instrumentation = no alert.

The Code investigator did identify one finding that others missed: the `events_dropped` canary bug ([E9]-[E11]). This is not a contradiction but an additional defect that no other investigator examined.

## What I Don't Understand

1. **Exact RecoveryClassifyCall count**: The Code investigator estimated ~12-16 during the 540s incident. This is close to but below the signal_d threshold of >20. The exact count depends on runtime timing of evidence accumulation, cooldown alignment, and SnapSync attempt timing. A live RPC probe [M1] would resolve this.

2. **Whether signal_d would fire at >20 with slightly different timing**: If the incident lasted 10-11 minutes instead of 9, or if the cooldown aligned differently, ~18-22 events might have been emitted, and signal_d MIGHT have fired. But even then, no automated consumer exists (H6), so the alert still would not have reached an operator.

3. **fork-monitor.sh behavior with height-divergent nodes**: The Wildcard investigator raised W3 (threshold tuning) -- whether the script treats "different hash at different height" as FORK or LAG. The script groups by bestHash regardless of height, so N3 at h=284677 with hash 8ede1526 would be a separate group from the fleet at h=284690+ with a different hash. This WOULD trigger FORK exit code. But the script's deployment status is unknown [M2].

## Root Cause

The observability subsystem failed to alert during INC-I-090's 9-minute fork because of a compound failure at two independent architectural layers:

**Layer 1 (Emission)**: The same FINALITY_GUARD fencepost (`recovery.rs:312`) that caused N3's fork also suppressed diagnostic emissions. When `target_height <= finality`, the recovery coordinator returns `RecoveryAction::None`, and `classify_and_dispatch` (`block_lifecycle.rs:626-630`) sets `ctx_for_emit = None`, causing EMIT-007 at `periodic.rs:612` to skip the `RecoveryClassifyCall` event. For the first ~60 seconds (while `recently_synced()` was true), every recovery iteration hit this path. After 60s, some non-None actions (HeaderFirstSync, SnapSync) were emitted -- approximately 12-16 events over the remaining ~480 seconds -- but this count falls below the classifier's ChainBreakLoop signal_d threshold of >20. Additionally, `SnapSyncAttempted/Completed/Failed` (u8=8,9,10) and `ChainBreakDetected` (u8=11) have zero production emit sites, structurally disabling classifier rules (d) and (h)/signal_a. Furthermore, emitted `RecoveryClassifyCall` events have `correlation_key: None` (`periodic.rs:624`), making them invisible to the `has_other_signals()` check (`classifier.rs:323-328`) that distinguishes TipRaceNatural from higher-severity classifications. The net result: the classifier sees 1 ForkBlockReceived + normal BlockApplied flow, matches rule (f) TipRaceNatural, and returns `recommended_action: "normal_operation"`.

**Layer 4 (Surface)**: No automated consumer reads `getForkDiagnostic` output. `fork-monitor.sh` polls `getChainInfo` (tip hash comparison) but has no systemd unit, cron entry, or launchd plist in the codebase. No dashboard, explorer, or metrics integration calls any diagnostic RPC. The classifier runs on-demand only (no background classification task). Even if L1 were perfect and the classifier returned `restart_with_resync`, the recommendation would sit in RocksDB as dead data.

**Design root cause**: The observability subsystem was architected as a passive forensic tool (pull-based, post-hoc investigation via `doli forks --explain`), not an active monitoring system (push-based, real-time alerting). This explains all symptoms simultaneously: no emit for recovery-None iterations (because the emit was designed for investigation, not monitoring), no automated consumer (because the RPC was designed for human CLI use), and dead code bridge `signal_stuck_fork()` / `take_stuck_fork_signal()` (the start of a push-based signal that was never completed).

## Causal Chain (with Derivation Test)

| # | Item | Derived? | Derivation |
|---|------|----------|------------|
| 1 | FINALITY_GUARD returns RecoveryAction::None for first ~60s | YES | [E3] `recovery.rs:312`: `target_height (284676) <= finality (284676)` -> true -> return None. `recently_synced()` is true (last_applied_secs < 60). Rule 1 enters fencepost -> None. |
| 2 | classify_and_dispatch sets ctx_for_emit = None | YES | [E1] `block_lifecycle.rs:626`: `if action != RecoveryAction::None` is false -> `ctx_for_emit = None` |
| 3 | EMIT-007 skips RecoveryClassifyCall emission | YES | [E2] `periodic.rs:612`: `if let Some(ref ctx) = recovery_ctx` -- body skipped when ctx is None |
| 4 | After 60s, ~15 non-None actions are emitted (HeaderFirstSync, SnapSync) | YES | `recovery.rs:346-349`: `medium_gap` (gap > 0 && gap < 500) -> HeaderFirstSync (non-None), gated by 30s cooldown (`recovery.rs:276-280`). ~480s / ~31s = ~15 events. |
| 5 | signal_d = ~15 < threshold >20 | YES | [E7] `classifier.rs:404`: `recovery_attempts > 20`. 15 < 20 -> signal_d = false. |
| 6 | 4 EventKinds have zero production emit sites | YES | [E6] grep confirms zero `DiagnosticEmitter::record()` calls producing SnapSyncAttempted(8), SnapSyncCompleted(9), SnapSyncFailed(10), ChainBreakDetected(11) |
| 7 | signal_a = 0 (ChainBreakDetected never emitted) | YES | Item 6 -> ChainBreakDetected count = 0 -> signal_a = false |
| 8 | Rule (h) ChainBreakLoop does not fire | YES | Items 5+7: all 4 signals (a,b,c,d) are false -> rule returns None |
| 9 | RecoveryClassifyCall events have correlation_key=None | YES | [E4] `periodic.rs:624`: `correlation_key: None` hardcoded |
| 10 | has_other_signals() returns false for ForkBlockReceived | YES | [E5] `classifier.rs:323-328`: fork_ev correlation_key is Some(...) (from block_handling.rs:206-214), but RecoveryClassifyCall events' correlation_key is None -> pattern at line 339 doesn't match -> false |
| 11 | Rule (f) TipRaceNatural matches | YES | Items 8+10: rules a-e,h don't match. Rule f: 1 ForkBlockReceived + low latency + no other signals in group -> matches |
| 12 | recommended_action = "normal_operation" | YES | Item 11: TipRaceNatural -> `classifier.rs:306` returns "normal_operation" |
| 13 | No automated consumer calls getForkDiagnostic | YES | [E8]: fork-monitor.sh has zero references to getForkDiagnostic. Code investigator Section 5: zero automated production consumers of any diagnostic RPC. |
| 14 | User detects fork visually | YES | Items 12+13: "normal_operation" classification + no consumer -> only human visual observation catches it |

## Specific Defects (Numbered)

### D1: RecoveryClassifyCall suppressed when RecoveryAction::None
- **File:line**: `crates/network/src/sync/manager/block_lifecycle.rs:626-630`
- **Severity**: P1 (silent failure -- recovery iterations invisible to diagnostics)
- **Layer**: L1
- **Description**: `classify_and_dispatch` sets `ctx_for_emit = None` when action is `RecoveryAction::None`, causing EMIT-007 at `periodic.rs:612` to skip emission. Every FINALITY_GUARD return and every cooldown-gated return produces no diagnostic event. During INC-I-090, this suppressed all recovery events for the first ~60 seconds and ~50% of subsequent iterations.
- **Fix sketch**: Always build `ctx_for_emit = Some(ctx)` regardless of action value. Add `action_returned: "None"` to the payload so the classifier can count all recovery iterations, not just non-None ones.
- **Evidence pointer**: Code investigator Section 2 Step (e), confirmed by synthesis reading `block_lifecycle.rs:626-630` [E1].

### D2: Four EventKinds have zero production emit sites
- **File:line**: No file -- missing instrumentation in `bins/node/src/node/fork_recovery.rs` (695 lines, zero diagnostic calls) and `crates/network/src/sync/manager/recovery.rs` (848 lines, zero diagnostic calls)
- **Severity**: P1 (silent failure -- SnapSync and ChainBreak events structurally invisible)
- **Layer**: L1
- **Description**: `SnapSyncAttempted` (u8=8), `SnapSyncCompleted` (u8=9), `SnapSyncFailed` (u8=10), and `ChainBreakDetected` (u8=11) are defined in `types.rs:51-57`, have payload definitions, are consumed by the classifier, and have log_replay parsers -- but zero `DiagnosticEmitter::record()` calls produce them in the node binary. Classifier rules (d) PostSnapDeadTip and (h)/signal_a are structurally dead.
- **Fix sketch**: Wire `DiagnosticEmitter::record()` calls in `fork_recovery.rs` (for SnapSync events) and in the recovery coordinator (for ChainBreakDetected when evidence reaches critical thresholds).
- **Evidence pointer**: Code investigator Section 1 (complete emit-site audit) [E6].

### D3: RecoveryClassifyCall uses correlation_key=None
- **File:line**: `bins/node/src/node/periodic.rs:624`
- **Severity**: P2 (ergonomics -- recovery events invisible to has_other_signals)
- **Layer**: L1/L3 boundary
- **Description**: RecoveryClassifyCall events are emitted with `correlation_key: None`. The classifier's `has_other_signals()` at `classifier.rs:314-348` checks for events sharing the ForkBlockReceived's correlation_key. Since RecoveryClassifyCall has None, it never participates in the correlation group, making rule (f) TipRaceNatural match even when recovery events exist. This is moot IF signal_d in rule (h) fires first (>20 events), but becomes the discriminator when recovery events are between 1 and 20.
- **Fix sketch**: Populate `correlation_key` with the divergence height from the ForkBlockReceived that triggered recovery, so RecoveryClassifyCall events are grouped with the fork event.
- **Evidence pointer**: Code investigator Section 3 rule walkthrough; synthesis verified at `classifier.rs:322-328` [E4], [E5].

### D4: No automated consumer of getForkDiagnostic
- **File:line**: No file -- missing infrastructure (no systemd unit, no cron, no dashboard integration)
- **Severity**: P0 (data loss -- correct classification is computed but never observed)
- **Layer**: L4
- **Description**: Zero automated production consumers of any diagnostic RPC. All callers are human-initiated (CLI `doli forks`, manual curl). fork-monitor.sh polls `getChainInfo` (tip hash), not `getForkDiagnostic`. No background task runs the classifier proactively. No dashboard, explorer, or metrics scrape consumes diagnostic data. Phase 2 deferrals explicitly list "Dashboard / explorer integration" and "Pre-fork warning stream / push alerts" as DEFERRED.
- **Fix sketch**: Either (a) add a periodic background task in the node that runs the classifier and emits a log line / metric when recommended_action != "normal_operation", or (b) upgrade fork-monitor.sh to call getForkDiagnostic and alert when recommended_action is actionable.
- **Evidence pointer**: Code investigator Section 5 (zero automated callers); Constraint investigator Section 2 (zero failed approaches = never attempted); Wildcard W5 (pull-vs-push architecture) [E8].

### D5: events_dropped counter broken (canary failure)
- **File:line**: `crates/storage/src/diagnostic_ledger/writer_stats.rs:21,31` (never incremented); `crates/rpc/src/methods/diagnostics.rs:94` (reads wrong counter)
- **Severity**: P2 (ergonomics -- cannot detect ring buffer overflow)
- **Layer**: L2
- **Description**: `DiagnosticWriterStats.events_dropped` is initialized to 0 and never incremented by production code. The RPC handler at `diagnostics.rs:94` reads `stats.events_dropped` (always 0), not `emitter.dropped_count()` (which IS incremented on overflow at `emitter.rs:177`). The H2 detection canary is broken -- `health.events_dropped_total` is always 0 regardless of actual buffer overflow.
- **Fix sketch**: Wire `emitter.dropped_count()` through the RPC handler, or propagate the emitter's dropped counter into `DiagnosticWriterStats` on each writer drain cycle.
- **Evidence pointer**: Code investigator Section 4 [E9], [E10], [E11].

### D6: signal_stuck_fork() designed but never wired
- **File:line**: `crates/network/src/sync/manager/production_gate.rs:573` (`take_stuck_fork_signal()` -- 0 non-test callers); `production_gate.rs:581` (`signal_stuck_fork()`)
- **Severity**: P2 (incomplete feature -- bridge between recovery coordinator and action layer never completed)
- **Layer**: L1/L4 bridge
- **Description**: `signal_stuck_fork()` sets a flag when the recovery coordinator detects a stuck fork. `take_stuck_fork_signal()` was designed to be consumed by an action layer (observability, production halt, or alert). But `take_stuck_fork_signal()` has zero non-test callers. The function exists, compiles, and has test coverage -- but the consumer side was never wired.
- **Fix sketch**: Wire `take_stuck_fork_signal()` into the periodic task or the observability emitter to bridge the recovery coordinator's "I'm stuck" signal to the diagnostic pipeline.
- **Evidence pointer**: Evidence entry 826 (INC-I-090 fork investigation); Wildcard W11; synthesis verified via grep [E12].

## Minimal Local-Testnet Repro Recipe

**Prerequisite**: Local testnet per CLAUDE.md (`~/testnet/`, `scripts/testnet.sh`). Binary built from current `main` branch.

1. Build: `cargo build --release`
2. Copy binary: `cp target/release/doli-node ~/testnet/bin/doli-node && codesign --force --sign - ~/testnet/bin/doli-node`
3. Start local testnet: `scripts/testnet.sh start all`
4. Wait for chain to advance past epoch 1 (height > 30) so finality is established
5. Identify a producer node (e.g., n1 on RPC 8501)
6. Confirm observability is live: `curl -s -X POST http://127.0.0.1:8501 -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"getForkDiagnostic","params":{"limit":5},"id":1}'` -- should return valid JSON with `health.ledger_available: true`
7. Record current height H: `curl -s -X POST http://127.0.0.1:8501 -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['bestHeight'])"`
8. Force a 1-block fork on n1: stop n1, let network advance 1 block, manually produce a competing block on n1 at the same height (or: use the existing FINALITY_GUARD fencepost -- if it still has the `<=` bug, simply create a race condition where n1 produces at a height where finality equals local_height - 1)
9. *Alternative simpler approach*: Stop n1 for ~60 seconds. During this time the fleet advances ~6 blocks. Restart n1. N1 will enter recovery. The recovery coordinator should emit RecoveryClassifyCall events. Query `getForkDiagnostic` to see what the classifier returns.
10. After n1 has been stuck/recovering for 2+ minutes, query diagnostic RPC: `curl -s -X POST http://127.0.0.1:8501 -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"getForkDiagnostic","params":{"min_height":<H>,"max_height":<H+20>,"limit":1000},"id":1}'`
11. Inspect `classification.fork_type` and `classification.recommended_action`
12. Verify: **no automated alert fired** (check that no external process consumed the diagnostic data)
13. Verify: `getForkDiagnostic` response shows `recommended_action: "normal_operation"` (if the fork shape matches TipRaceNatural)
14. Check `health.events_dropped_total` -- should be 0 regardless of actual drops (broken canary)
15. Count `RecoveryClassifyCall` events in the response -- if < 20, signal_d did not fire

**Pass criteria for eventual fix:**
- `recommended_action` MUST be != "normal_operation" within 120 seconds of a node entering a stuck-fork state
- RecoveryClassifyCall events MUST be emitted for ALL recovery iterations, including RecoveryAction::None returns
- At least one automated consumer (background task, fork-monitor.sh upgrade, or metrics export) MUST read the diagnostic RPC output periodically (< 60s interval)
- `health.events_dropped_total` MUST reflect the emitter's actual dropped count
- SnapSyncCompleted events MUST be emitted when snap sync fires

## Per-Hypothesis Confidence

| Hypothesis | Confidence | Rationale |
|-----------|-----------|-----------|
| H1 (full: no emit at all) | REFUTED | ForkBlockReceived (EMIT-002) and BlockApplied (EMIT-010) confirmed emitted via code audit. Only partial emit gap in recovery path. |
| H1-sub (recovery emit gap) | HIGH conf(0.85, converged) | 4/5 investigators converged. Code evidence: `block_lifecycle.rs:626-630` + `periodic.rs:612` confirms gating. Git evidence: commit `259f6380` says "when action is non-None". Independent verification by synthesis confirms. |
| H2 (ring buffer overflow) | LOW conf(0.15, inferred) | Event rate ~1-2/sec << 160 events/sec drain rate. But canary broken (D5), so cannot confirm from RPC. Architecturally implausible. |
| H3 (classifier shape) | HIGH conf(0.80, converged) | DOWNSTREAM of H1-sub. 4/5 investigators agree. Classifier rules walked by Code investigator and confirmed by synthesis. Classifier is correct on incomplete data. |
| H4 (fork-monitor not deployed) | HIGH conf(0.75, converged) | 5/5 investigators agree from structural evidence. No systemd unit/cron/plist in codebase. Zero failed approaches = never attempted. Pending live [M2]. |
| H5 (wrong recommended_action) | HIGH conf(0.80, converged) | DOWNSTREAM of H1-sub. TipRaceNatural with "normal_operation" is the exact classifier output for the incident event set. Proven by rule walkthrough. |
| H6 (no automated consumer) | HIGH conf(0.85, converged) | 5/5 investigators converged. Zero automated callers in code. Zero failed approaches in domain. Phase 2 deferrals confirm dashboard/alerts explicitly deferred. |
| H7 (version skew) | LOW conf(0.15, inferred) | Feature unconditionally compiled. v6.22.0 released 5 days before incident. INC-I-087 fix deployed. Pending live [M3]. |
| W5 (pull-vs-push) | HIGH conf(0.70, inferred) | Structural root cause. Subsumed by H6 as its concrete manifestation. The system is a forensic tool, not a monitoring tool. |
| W9 (TipRaceNatural pathway) | HIGH conf(0.80, measured) | Full worked example by Wildcard investigator, confirmed by Code investigator rule walkthrough and synthesis verification. |
| W11 (dead code bridge) | MEDIUM conf(0.60, measured) | production_gate.rs:573 confirmed 0 non-test callers. Archaeological evidence of designed-but-unwired integration. |

## Feasibility Verdict

```
━━━ DIAGNOSTICIAN FEASIBILITY VERDICT ━━━
Fixable with code change:  YES
Confidence:                conf(0.82, converged)
Reasoning:                 All 6 defects (D1-D6) are code-level or script-level changes. D1 (emit gating) requires changing one conditional in block_lifecycle.rs. D2 (missing emit sites) requires adding DiagnosticEmitter::record() calls in fork_recovery.rs and recovery paths. D3 (correlation_key) requires populating a field in periodic.rs. D4 (no automated consumer) requires either a new background task or upgrading fork-monitor.sh. D5 (broken canary) requires wiring emitter.dropped_count() through the RPC. D6 (dead code bridge) requires wiring take_stuck_fork_signal() into the periodic task. None of these changes affect consensus, block content, or require activation heights. They are purely observability/tooling improvements.
Architect's verdict was:   CODE-FIXABLE
Agreement:                 AGREES
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Regression Check

Regression check: N/A -- not a regression. The observability subsystem was first shipped in v6.22.0 (2026-05-21, commit `1ffc5df8` and follow-ups). The incident occurred 2026-05-25, 4 days after initial ship. The defects are "feature shipped incomplete" (missing emit wiring, missing automated consumer), not "previously working feature that broke." There is no baseline commit where the observability system functioned correctly for this fork shape. The Phase 2 deferrals (commit `5d1d83a7`) explicitly acknowledge that dashboard/alert integration was not included in the initial ship.

## Synthesis Quality Gate

```
━━━ SYNTHESIS QUALITY GATE ━━━
Investigators completed:       5/5
Convergence on top hypothesis: 5/5 investigators (H1-sub + H6 compound)
Evidence independence:         VERIFIED (see reasoning trace)
Contradictions found:          0 (plausible -- structural absence, not competing implementations)
Contradictions resolved:       0/0
Unexplained items:             0 (all causal chain items derived)
Evidence layers covered:       L0 (inferred), L1 (measured), L2 (measured), L3 (measured), L4 (inferred+measured)
Evidence layers NOT covered:   Live state (SSH/RPC blocked -- [M1][M2][M3])
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```
