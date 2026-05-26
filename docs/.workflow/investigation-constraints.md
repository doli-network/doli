# Investigation Report: Constraint Elimination (Investigator #4)

## Evidence Layer
Failed approaches, working cases, and constraint table from evidence assembly. Deductive elimination (modus tollens) across H1-H7. Also: the ABSENCE of certain evidence as an informative signal.

## What I Don't Understand
1. Whether `RecoveryClassifyCall` events are actually emitted in recovery.rs -- the schema defines kind=7 and rule (h) depends on `recovery_attempts > 20`, but skill does NOT cite a specific emit call. System blueprint marks this "UNVERIFIED."
2. Whether fork-monitor.sh is deployed as a systemd service/cron on mainnet ai1-ai5.
3. Whether the mainnet N3 binary `/mainnet/bin/doli-node-n3` contains the diagnostic subsystem (INC-I-087 fix committed 2026-05-21 as 954afc45; deployment lag unknown).
4. The exact event count in N3's diagnostic ledger for heights [284670, 284685].
5. Whether `health.events_dropped_total` on N3 is zero or nonzero.
6. Whether any external process (outside the codebase) consumes `recommended_action`.

---

## Section 1: Working Cases

The observability subsystem has been proven to work for **post-hoc** investigation:

### Working case 1: INC-I-090 itself was diagnosed post-hoc using these RPCs
The underlying fork at h=284677 was diagnosed by 4 domain investigators (entries 818-828 in memory.db) who used:
- **FORK_GUARD textual log lines** in `/var/log/doli/mainnet/n3.log.1` -- confirmed present per the refined prompt and the log-sufficiency assessment.
- **RPC data from getChainInfo** -- height, bestHash used to establish fork timeline.
- **Code-level trace** through block_handling.rs, fork_recovery.rs, recovery.rs -- the investigators traced the exact code path N3 executed.
- **Conclusion**: L1 textual logging works. L4 getChainInfo RPC works. The post-hoc investigation was successful using evidence from the SAME subsystem that supposedly failed. This is NOT a contradiction because post-hoc human curl is a different consumer than automated alerting.

### Working case 2: INC-I-087 diagnostic health fix (workflow #353-354)
- Entry 807 in memory.db: "Committed as 954afc45: lifted writer counters into Arc<DiagnosticWriterStats>, wired into RpcContext, replaced hardcoded literals in diagnostics.rs:91-96 with live atomic reads."
- This proves: (a) the diagnostic ledger + RPC pipeline was functional enough to identify a bug (hardcoded zeros), (b) the fix was tested with FAIL-to-PASS test evidence, (c) the subsystem exists in the codebase as of 2026-05-21.

### Working case 3: INC-I-084 instant-fork regression (workflow #351)
- Entry 781: "New Node struct fields diagnostic_emitter and diagnostic_ledger (Arc<dyn Trait>) added in 1ffc5df8."
- This proves the emitter and ledger are structurally wired into the Node struct.

### Working case 4: fork-monitor.sh script exists and is functional
- Per skill OPERATIONS table: `fork-monitor.sh [--testnet] [--loop [SECS]] [--endpoints FILE]` -- the script is documented, has testnet and mainnet modes, and produces exit codes 0=OK/1=FORK/2=error.
- Per skill DATA-FLOW table: "getChainInfo on each port -> group by bestHash -> OK/FORK output."
- The script WORKS when run manually. The question is whether it runs automatically.

### Contrast conclusion
The subsystem's **components** work individually. The failure is at the **integration** level: components exist but are not wired into an automated detection loop. Post-hoc manual curl succeeds; proactive automated alert does not exist.

---

## Section 2: Failed Approaches Summary

**Result: EMPTY for this domain.**

Query:
```sql
SELECT domain, approach, failure_reason FROM failed_approaches
WHERE domain LIKE '%observ%' OR domain LIKE '%diagnos%'
  OR domain LIKE '%fork-monitor%' OR domain LIKE '%alert%'
  OR domain LIKE '%dashboard%' OR domain LIKE '%monitor%';
```
Returns: 0 rows.

**Significance**: Nobody has attempted and failed to make automated alerting work. The gap is "not attempted" rather than "attempted and broken." This is a clean-slate absence, which is itself strong evidence:

- If the alerting system had been deployed and failed (e.g., fork-monitor.sh running but cadence too slow), there WOULD be a failed_approaches entry from prior incident investigations.
- No such entry exists across 20+ fork-related workflow runs (evidence-assembly: "20 prior runs in broader fork/sync domain").
- This implies the surface layer (L4) was never deployed as an automated service -- it was only ever used manually.

---

## Section 3: Elimination Matrix

### Methodology
For each hypothesis H_X: "If H_X were true, we would expect to see Y. Do we see Y?"

Evidence sources:
- **[BRIEF]**: investigation-brief.md
- **[SKILL]**: .claude/skills/observability-fork/SKILL.md
- **[SCHEMA]**: LEDGER-SCHEMA.md
- **[BLUEPRINT]**: system-blueprint.md
- **[EVIDENCE]**: evidence-assembly.md
- **[LOG-SUFF]**: log-sufficiency.md
- **[INC-ENTRIES]**: memory.db incident_entries for INC-I-090
- **[INV-OBS-001]**: memory.db invariants table
- **[INC-I-087]**: memory.db entry 807 (diagnostic health fix)
- **[ANALYST]**: observability-gap-inc-i-090-analysis.md

| # | Hypothesis | If true, would expect to see | Do we see it? | Evidence citation | Elimination verdict |
|---|-----------|------------------------------|---------------|-------------------|---------------------|
| H1 | No emit -- DiagnosticEmitter::record() never called for h=284677 | (a) No ForkBlockReceived for h=284677 in N3 ledger; (b) FORK_GUARD textual logs absent | (a) PENDING -- requires getForkDiagnostic RPC query on N3:8503 (state investigator); (b) FORK_GUARD textual logs ARE confirmed present in n3.log.1 [BRIEF line 6, LOG-SUFF line 27] | [LOG-SUFF]: "FORK_GUARD signals are confirmed present in /var/log/doli/mainnet/n3.log.1" | **PARTIALLY REFUTED** for block_handling.rs path. FORK_GUARD lines prove the code path that emits ForkBlockReceived was entered. However, CANNOT eliminate a SUB-CASE: RecoveryClassifyCall may NOT be emitted in recovery.rs even though the code path was entered 253 times. See H1-sub below. |
| H1-sub | RecoveryClassifyCall never emitted in recovery.rs (emit gap in recovery coordinator) | (a) Zero RecoveryClassifyCall events (kind=7) in N3 ledger for the incident window; (b) Skill does NOT cite emit call in recovery.rs | (a) PENDING -- requires getForkDiagnostic RPC query; (b) CONFIRMED -- [BLUEPRINT Seam 3, BLUEPRINT section 2.1 recovery.rs paragraph]: "the skill does NOT mention recovery.rs emitting RecoveryClassifyCall" and "the DEPENDENCIES section says block_handling.rs emits ForkBlockReceived and apply_block.rs emits BlockApplied -- it does NOT mention recovery.rs emitting RecoveryClassifyCall" | [BLUEPRINT section 2.1, SKILL DEPENDENCIES section line 85-86] | **ALIVE (HIGH PRIORITY)** -- Structural evidence supports this sub-hypothesis. If confirmed, it explains why the classifier cannot distinguish "stuck 9 minutes" from "benign tip race." Pending code-level verification from investigator-code. |
| H2 | Ring buffer overflow -- events emitted but dropped | (a) health.events_dropped_total > 0 in getForkDiagnostic response; (b) High event rate during incident | (a) PENDING -- requires RPC query on N3:8503 (state investigator); (b) Event rate analysis: mainnet steady-state is ~1 event/10s. Even during recovery, 253 iterations over 9 min = ~0.47 events/sec. Ring buffer capacity (not documented, but typical=1024) would NOT overflow at this rate. [BLUEPRINT section 5, assumption 1]: "Under steady-state (1 block/10s), this holds." | [BLUEPRINT section 2.2] | **LIKELY REFUTED** (pending RPC confirmation). Event rate during INC-I-090 is orders of magnitude below plausible ring buffer overflow. BUT: this analysis assumes RecoveryClassifyCall events are actually emitted. If they are NOT emitted (H1-sub), then the question is moot for those events. Overflow of block_handling events (1-2 events total) is architecturally impossible. |
| H3 | Classifier does not recognize this fork shape | (a) classification.fork_type = "Unknown" (rule g fallback); (b) No rule matches 1-block self-produced minority + snap-sync recovery | (a) PENDING -- requires RPC query; (b) DEDUCTIVE ANALYSIS: IF all events were emitted (H1 false, H1-sub false), then the event set would include: 1 BlockApplied + 1 ForkBlockReceived + 253 RecoveryClassifyCall + 1 SnapSyncCompleted. Rule (h) ChainBreakLoop signal_d fires when recovery_attempts > 20 in 1h window. 253 > 20 = true. Rule (h) produces `recommended_action: "restart_with_resync"` [SCHEMA line 111]. IF H1-sub is TRUE (no RecoveryClassifyCall events), then event set = 1 BlockApplied + 1 ForkBlockReceived + (possibly) 1 SnapSyncCompleted. Rule (d) PostSnapDeadTip fires if SnapSyncCompleted + ForkBlockReceived within 300s [SCHEMA line 81]. This SHOULD match, producing `recommended_action: "auto_recover"`. BUT: rule (d) checks "SnapSyncCompleted THEN ForkBlockReceived" -- temporal order matters. In INC-I-090, ForkBlockReceived came FIRST (22:54:37), SnapSyncCompleted came LAST (~23:04:30). Rule (d) checks SnapSyncCompleted THEN ForkBlockReceived, not the reverse. So rule (d) would NOT fire. Classifier would fall through to (e)/(f): TipRaceHighLatency or TipRaceNatural depending on validation_duration_ms. Neither produces an actionable recommended_action. | [SCHEMA lines 105-114, BLUEPRINT section 2.4] | **CONDITIONALLY ALIVE** -- If H1-sub is true (no RecoveryClassifyCall), classifier falls through to TipRaceNatural with `recommended_action: "normal_operation"`. This is H5, not H3 per se. H3 as "classifier broken" is REFUTED -- the classifier works correctly for its inputs. The issue is that its inputs are INCOMPLETE due to H1-sub. |
| H4 | fork-monitor.sh not deployed / wrong cadence on mainnet | (a) No systemd unit with "fork" or "monitor" in name on ai1; (b) No cron entry for fork-monitor.sh | (a) PENDING -- requires ssh ai1 + systemctl check (state investigator); (b) STRUCTURAL EVIDENCE: Skill documents fork-monitor.sh as a MANUAL command, not a service [SKILL line 32]: "bash fork-monitor.sh [--testnet] [--loop [SECS]] [--endpoints FILE]". No systemd unit or cron deployment documented. [ANALYST line 48]: "The skill does NOT document a systemd unit, cron job, or container that runs fork-monitor.sh on mainnet." [EVIDENCE]: 0 failed approaches in monitor/alert domains = nobody has tried deploying it as a service. | [SKILL, ANALYST, EVIDENCE] | **HIGHLY LIKELY TRUE** -- All structural evidence points to fork-monitor.sh not being deployed as an automated service. The absence of failed approaches in this domain is consistent with "never attempted." Pending live verification from state investigator. |
| H5 | Classifier returns low-priority recommended_action | (a) classification.recommended_action = "normal_operation" or equivalent low-priority; (b) This happens because event set lacks RecoveryClassifyCall | (a) DEDUCTIVE: If H1-sub is true, the event set for the incident window contains only ForkBlockReceived + BlockApplied at h=284677 (and possibly SnapSyncCompleted at ~23:04:30). Per temporal analysis above, rule (d) PostSnapDeadTip does NOT match (wrong temporal order). Rules (e)/(f) check for tip race. The ForkBlockReceived at h=284677 with a BlockApplied at same height: if validation_duration_ms < 500ms, rule (f) TipRaceNatural fires with `recommended_action: "normal_operation"` [SCHEMA line 113]. (b) This is a DIRECT CONSEQUENCE of H1-sub -- missing recovery events cause the classifier to under-classify. | [SCHEMA lines 111-114, BLUEPRINT section 2.4 INC-I-090 classification analysis] | **ALIVE -- but DOWNSTREAM of H1-sub.** H5 is not an independent failure; it is a predictable consequence of H1-sub. The classifier correctly implements its rules, but the rules are starved of the signal (RecoveryClassifyCall) that would trigger the correct classification. |
| H6 | No surface consumer -- RPC methods work but nothing reads them automatically | (a) No code path in codebase that reads recommended_action and triggers action; (b) fork-monitor.sh polls getChainInfo, not getForkDiagnostic; (c) No dashboard/explorer/metrics consumer | (a) [BLUEPRINT Smell 1]: "no code path in the codebase automatically reads the recommended_action field and triggers any action." [ANALYST line 44]: "The recommended_action field is computed, serialized into JSON, and... returned to whoever called the RPC. If nobody calls, the field is never read." (b) [SKILL line 32, DATA-FLOW line 66]: fork-monitor.sh polls getChainInfo, groups by bestHash, reports divergence. It NEVER calls getForkDiagnostic. (c) [BLUEPRINT Smell 7]: health-check.sh does NOT consume diagnostic ledger data. [EVIDENCE]: 0 failed approaches in dashboard/alert domains. | [BLUEPRINT, ANALYST, SKILL, EVIDENCE] | **CONFIRMED TRUE** from architectural evidence. Even if L1+L2+L3 work perfectly, there is no automated consumer. The recommended_action field is dead data unless a human manually curls the RPC. |
| H7 | Binary version skew -- mainnet binary missing observability subsystem | (a) getForkDiagnostic returns "method not found" on N3:8503; (b) Binary predates feature merge | (a) PENDING -- requires RPC probe on N3:8503 (state investigator); (b) INDIRECT EVIDENCE AGAINST: INC-I-087 (hardcoded zeros fix) was committed 2026-05-21 as 954afc45. The underlying fork happened 2026-05-25. If the INC-I-087 fix was deployed (which it must have been, as it was an INC fix), the binary includes the diagnostic subsystem. Entry 781 confirms diagnostic_emitter and diagnostic_ledger were added to Node struct in 1ffc5df8. The observability skill was documented in workflow #349 (prior to #353-354). | [INC-I-087 entry 807, INC-I-084 entry 781] | **LIKELY REFUTED** -- The diagnostic subsystem has been in the codebase since at least the INC-I-087 fix. Deployment lag is possible but unlikely for a 5-day gap. Pending live binary check from state investigator. |

---

## Section 4: Cross-Hypothesis Consistency

### Independence matrix

| H pair | Relationship | Notes |
|--------|-------------|-------|
| H1 ^ H2 | REDUNDANT if H1 full | If block_handling.rs never emits, events_dropped_total cannot increase for those events. But H1-sub (recovery path only) is INDEPENDENT of H2 -- block_handling events could still be emitted and overflow (unlikely but logically possible). |
| H1-sub ^ H3 | CAUSAL | H1-sub causes H3/H5. Missing RecoveryClassifyCall events cause the classifier to under-classify. They are not independent failures -- H3/H5 are downstream effects of H1-sub. |
| H1-sub ^ H4 | INDEPENDENT | Recovery emit gap is an L1 issue; fork-monitor deployment is an L4 issue. Both can be true simultaneously without causal connection. |
| H1-sub ^ H5 | CAUSAL (H1-sub -> H5) | H5 is a predictable consequence of H1-sub. If RecoveryClassifyCall events are absent, classifier falls to TipRaceNatural with normal_operation. |
| H1-sub ^ H6 | INDEPENDENT | Even if recovery events WERE emitted, no automated consumer exists (H6). Both can be true simultaneously. |
| H4 ^ H5 | COMPOUNDING | fork-monitor.sh not deployed (H4) means no polling. Low-priority recommended_action (H5) means even if polled, no alarm would fire. Both failures reinforce the observability gap. |
| H4 ^ H6 | OVERLAPPING | H4 is a specific instance of H6. fork-monitor.sh not deployed is a sub-case of "no surface consumer." H6 is the broader statement; H4 is a concrete manifestation. |
| H5 ^ H6 | COMPOUNDING | Even if the classifier produced "restart_with_resync" (which it would IF recovery events existed), no automated consumer would act on it. H5 is moot if H6 is true, because NOBODY reads the output regardless. |
| H7 ^ ALL | SUPERSEDING | If the binary lacks the subsystem, L1-L4 are ALL dark. H7 alone would explain everything. But evidence weighs against H7 (INC-I-087 fix deployed 5 days prior). |

### Mutual exclusivity analysis
- H1 (full) is mutually exclusive with H2 (can't overflow what was never emitted). But H1-sub + H2 can coexist.
- H7 supersedes all others but is unlikely.
- H4, H5, H6 can ALL be true simultaneously -- they are independent or compounding failures at different layers.
- H1-sub is causally upstream of H5 -- if H1-sub is true, H5 is a predictable consequence, not an independent failure.

### Compound failure assessment
The most parsimonious compound explanation is:
**H1-sub + H6 (with H4 and H5 as downstream consequences)**

This means:
1. Recovery path does not emit RecoveryClassifyCall (L1 gap) -> classifier cannot distinguish stuck-for-9-minutes from benign tip race (H5 consequence)
2. No automated consumer reads getForkDiagnostic output (L4 gap) -> even correct classification would not reach an operator (H6)

---

## Section 5: Occam's Razor Application

### Single-cause candidates
- **H7 alone**: Would explain everything -- binary missing subsystem means no emit, no persist, no classify, no surface. Occam score: BEST if true. But evidence weighs AGAINST (INC-I-087 deployed 5 days prior). conf(0.15, inferred).
- **H6 alone**: "No automated surface consumer" explains the symptom "user noticed visually" regardless of whether L1-L3 worked. Even perfect emission + persistence + classification is invisible without a consumer. Occam score: GOOD. But this does NOT explain why the classifier would produce a low-priority result -- it's a necessary but possibly not sufficient explanation. conf(0.55, inferred).

### Dual-cause candidates
- **H1-sub + H6**: Recovery emit gap (L1) + no automated consumer (L4). Two independent failures at two different layers. H1-sub degrades the classifier's ability to produce an actionable signal. H6 means even if the signal were actionable, nobody reads it. BOTH are needed to explain the full observability gap. Occam score: VERY GOOD -- two failures, but they share a COMMON CAUSE: "the observability subsystem was designed and implemented as a passive data store, not an active monitoring system." The architect never wired recovery.rs to emit, AND never wired a consumer to poll. Both are symptoms of the same design philosophy: build the data layer, defer the alerting layer. conf(0.65, inferred).

### Triple-cause candidates  
- **H1-sub + H4 + H6**: Adds fork-monitor.sh not deployed. But H4 is a sub-case of H6 (fork-monitor is one possible consumer that doesn't exist). The triple explanation adds no explanatory power beyond H1-sub + H6. Occam penalizes unnecessary terms.

### Verdict
**H1-sub + H6** is the minimal explanation consistent with all evidence. H5 is a derived consequence of H1-sub, not an independent failure. H4 is a specific instance of H6, not a separate root cause.

Common root cause: **The observability subsystem was shipped as a passive data store without completing the last mile -- emitting recovery events and wiring an automated consumer.** This is a "feature shipped incomplete" pattern, not a "feature broken" pattern.

---

## Section 6: Final Consistent Hypothesis Set

### Primary: Two-layer gap with shared design-origin

**ROOT CAUSE A: Recovery coordinator emission gap (H1-sub)** -- conf(0.60, inferred)
- The `RecoveryClassifyCall` EventKind (u8=7) exists in the schema. The classifier's rule (h) `ChainBreakLoop` has signal_d (`recovery_attempts > 20`). But the skill does NOT document an emit call in recovery.rs, and the blueprint explicitly marks this as "UNVERIFIED." If recovery.rs does not call `DiagnosticEmitter::record(RecoveryClassifyCall)`, then the 253 recovery iterations during INC-I-090 produced ZERO diagnostic events, and the classifier falls through to TipRaceNatural (`recommended_action: "normal_operation"`).
- **Kill test**: Read recovery.rs for `DiagnosticEmitter::record` calls. If present, H1-sub is dead. If absent, H1-sub is confirmed.
- **Derivation**: H1-sub -> classifier receives only ForkBlockReceived + BlockApplied -> rule (h) signal_d cannot fire (0 RecoveryClassifyCall < 20 threshold) -> rules (a)-(d) don't match -> rule (f) TipRaceNatural matches -> `recommended_action: "normal_operation"` -> even if consumed, no alert fires.

**ROOT CAUSE B: No automated surface consumer (H6)** -- conf(0.65, inferred)
- The blueprint, analyst, and skill all confirm: no code path consumes `recommended_action` automatically. fork-monitor.sh polls getChainInfo (tip hash only), not getForkDiagnostic. health-check.sh does not query the diagnostic ledger. No dashboard, explorer, or metrics integration is documented. 0 failed approaches in alert/monitor/dashboard domains = never attempted.
- **Kill test**: grep entire codebase + infrastructure for any automated consumer of getForkDiagnostic or recommended_action. If found, H6 is dead.
- **Derivation**: H6 -> even if classifier produces "restart_with_resync" (which it would if H1-sub were false), the recommendation sits in the Classification struct, gets serialized into JSON, and is returned to nobody.

### Compound explanation
Both ROOT CAUSE A and ROOT CAUSE B are needed. Either alone is necessary but not sufficient:
- A alone: Even if the classifier sees only TipRaceNatural, a sufficiently smart automated consumer could notice the HEIGHT divergence growing over 9 minutes via getChainInfo polling -- but only if deployed (B).
- B alone: Even if an automated consumer exists, it would receive `recommended_action: "normal_operation"` (due to A) and take no action.

The combination A+B is the **minimum sufficient** explanation.

### Eliminated hypotheses
| H | Status | Reason |
|---|--------|--------|
| H1 (full) | REFUTED | FORK_GUARD textual logs confirm block_handling.rs code path was entered |
| H2 | LIKELY REFUTED | Event rate during incident (~0.47/sec max) is far below plausible overflow threshold |
| H3 | REFUTED (as independent) | Classifier rules are correct for their inputs; under-classification is H5, caused by H1-sub |
| H5 | ALIVE but DERIVED | Downstream consequence of H1-sub, not an independent root cause |
| H7 | LIKELY REFUTED | INC-I-087 fix deployed 5 days prior; binary almost certainly includes subsystem |

### Surviving hypothesis requiring live verification
| H | What's needed | Who provides it |
|---|---------------|-----------------|
| H1-sub | grep/read recovery.rs for DiagnosticEmitter::record calls | investigator-code |
| H4 (instance of H6) | ssh ai1 + systemctl/crontab check | investigator-state |
| H6 | grep codebase for automated consumers | investigator-code |
| H7 | getForkDiagnostic RPC probe on N3:8503 | investigator-state |

---

## Section 7: Confidence with Reasoning

**Overall diagnosis confidence: conf(0.65, inferred)**

Basis: The elimination matrix is built from 7 evidence sources (brief, skill, schema, blueprint, evidence assembly, log sufficiency, memory.db). All sources are consistent. No contradictions found. The two surviving root causes (H1-sub + H6) are:

1. **Structurally supported**: Blueprint explicitly flags both as architectural smells (Smells 1 and 4). The analyst independently assigns HIGH prior weight to both H5 (which is downstream of H1-sub) and H6. The log-sufficiency assessment confirms the evidence channels exist to verify both.

2. **Not yet measured**: Neither root cause has been confirmed by live RPC probe or code-level verification. The confidence ceiling for inference-only analysis is 0.70 per investigation protocol. I rate at 0.65 because the structural evidence is strong but one critical verification (RecoveryClassifyCall emit in recovery.rs) remains pending.

3. **Consistent with the "never attempted" signal**: Zero failed approaches in observability domains means the surface layer was never deployed. This is the strongest piece of evidence from my constraint-elimination layer: the absence of failure records implies absence of attempts.

**What would change my confidence:**
- conf -> 0.70: Code investigator confirms RecoveryClassifyCall is NOT emitted in recovery.rs AND state investigator confirms no systemd/cron service for fork-monitor.sh.
- conf -> 0.40: Code investigator finds RecoveryClassifyCall IS emitted in recovery.rs (H1-sub dies; H6 alone is necessary but the under-classification mystery reopens).
- conf -> 0.10: State investigator finds getForkDiagnostic returns "method not found" on N3:8503 (H7 supersedes everything).

---

## Cross-Layer Signals

1. **For investigator-code**: HIGHEST PRIORITY -- verify whether `crates/network/src/sync/manager/recovery.rs` contains any call to `DiagnosticEmitter::record(RecoveryClassifyCall)`. This is the single most load-bearing verification in the entire investigation.

2. **For investigator-state**: (a) Query `getForkDiagnostic` on N3:8503 for heights [284670, 284685] -- capture event count, event kinds, health.events_dropped_total, classification.fork_type, classification.recommended_action. (b) Check `systemctl list-unit-files | grep -iE 'fork|monitor'` and `crontab -l` on ai1. (c) Confirm binary contains diagnostic subsystem via the RPC probe.

3. **For investigator-logs**: The FORK_GUARD textual log lines in n3.log.1 are the anchor for H1 refutation. Confirm they exist in the incident window and are colocated with the emit call site in block_handling.rs.

4. **For the synthesizer**: The "never attempted" signal from zero failed approaches is as informative as any positive finding. It shifts the diagnosis from "feature broken" to "feature incomplete" -- specifically, the surface/alerting layer was never wired up.

## Gaps

1. **Live RPC data from N3**: The elimination matrix has 4 cells marked "PENDING" that require getForkDiagnostic RPC results from N3:8503. These would convert several "LIKELY REFUTED" verdicts to "CONFIRMED REFUTED."

2. **recovery.rs source code**: The single most important unresolved question. Without reading the actual code, H1-sub remains at conf(0.60, inferred) rather than conf(0.80+, measured).

3. **External consumers outside codebase**: An operator could have a private monitoring script that calls getForkDiagnostic. The investigation only covers the documented codebase and skill. If such a script exists and was not running, H6 narrows to "external consumer not running" rather than "no consumer exists."

4. **Pruning configuration**: If aggressive pruning removed incident evidence before the post-hoc investigation, earlier events could be lost. The pruning policy is "caller-determined" with no documented production defaults. However, the incident is only ~24 hours old, making aggressive pruning unlikely.
