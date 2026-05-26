# Investigation Report: Wildcard (Lateral Thinking)

## Evidence Layer
UNCONSTRAINED -- environment, configuration, timing, data, architecture, external dependencies, emergent behavior. Looking at what investigators framed around H1-H7 cannot see.

## What I Don't Understand
1. Whether `NoOpEmitter` can be selected at startup without any log warning -- if RocksDB diagnostics/ open fails silently, the entire subsystem is dead from boot.
2. How the deployed binary on N3 was built -- whether it used a cargo feature gate for the diagnostic subsystem or whether the subsystem is always compiled in.
3. Whether `fork-monitor.sh` has EVER been deployed as a service on mainnet, or whether it exists solely as a manual tool.
4. Whether any operator runbook references `getForkDiagnostic` as a periodic check, or whether the entire L3+L4 pipeline was designed but never operationalized.
5. Whether the "dead code" `signal_stuck_fork()` / `take_stuck_fork_signal()` path (evidence entry 826) was INTENDED to be the bridge between the recovery coordinator and the observability layer, but was never wired in.
6. The exact wall-clock time it takes for `fork-monitor.sh` to cycle through all 13+ nodes in `--loop` mode -- if it takes >9 minutes per cycle, the fork window is always below detection resolution.
7. Whether the `events_dropped_total` counter was ever non-zero on production, or whether it was previously hardcoded to zero (INV-OBS-001 mentions this was a known issue fixed in workflow 353-354 for INC-I-086).

## Section 1: Wildcard Hypothesis List

### W1: Human/process failure -- runbook never matured to use the tools
**Mechanism**: The 4-layer pipeline (Emit -> Persist -> Classify -> Surface) was designed and built correctly at the code level. But the "Surface" layer (L4) has a human at the top of the stack. The operator runbook was never formalized to include periodic `getForkDiagnostic` calls, and `fork-monitor.sh` was treated as a development/debugging aid, not a production service. The tools exist; nobody uses them in steady-state.
**Kill test**: Check mainnet ai1 for `systemctl list-unit-files | grep fork-monitor`, `crontab -l | grep fork-monitor`, and `pgrep -f fork-monitor`. If ANY of these show an active deployment, this hypothesis is dead.
**Relevance**: HIGH. The analyst report (line 48) already flagged: "The skill does NOT document a systemd unit, cron job, or container that runs fork-monitor.sh on mainnet." This is the most prosaic explanation.

### W2: Cognitive ergonomics -- dashboard data not visually distinctive
**Mechanism**: The explorer or dashboard may show `sync_fails` or node status, but a single number incrementing on one row among 12 is not perceptually salient. The data WAS surfaced (via getChainInfo height lagging), but not LEGIBLY. The human eye doesn't parse "N3 at height 284677 while others at 284690" as FORK vs LAGGING without a visual alarm.
**Kill test**: Examine the explorer codebase (doli-explorer) for any component that compares node heights and highlights divergence with color/alarm. If such a component exists AND was active during the incident, this hypothesis weakens significantly.
**Relevance**: MEDIUM. Secondary contributing factor, not primary root cause. The user DID notice visually, suggesting the dashboard DOES show something, but without alarm salience.

### W3: Alert fatigue / threshold mis-tuning
**Mechanism**: Even if `fork-monitor.sh` runs periodically, it groups by `bestHash`. During the 9-minute window, N3 was STUCK at h=284677 while fleet was at h=284690+. N3's bestHash was its fork block (8ede1526), which is different from the fleet's canonical hash. But if `fork-monitor.sh` treats height divergence as a benign lag (not a fork), it would exit 0, not 1. The script's fork detection is hash-based, not height-based: it groups by `bestHash`. If N3 is behind, its bestHash is for a lower height -- the script might show it as a SEPARATE GROUP but also at a DIFFERENT HEIGHT. The question is: does the script treat "different hash at different height" as FORK or as LAG?
**Kill test**: Read `fork-monitor.sh` logic. If the script groups by `bestHash` regardless of height and reports >1 group as FORK, then any stuck node would trigger FORK. If it groups by `bestHash` only within the same height band, a lagging node might not trigger. The answer is in the script, which I cannot read due to gate restrictions. This kill test requires code-reading.
**Relevance**: MEDIUM-HIGH. If fork-monitor groups ALL hashes regardless of height, it would catch N3's divergence. If it requires same-height to compare, a lagging node evades detection.

### W4: Time-of-day / staffing gap
**Mechanism**: 22:54-23:04 UTC. If operator is in Latin America (UTC-5 or UTC-6), this is 5-6 PM local time -- end of business day. Attention may have been diverted. This is not a root cause of the observability GAP (which is about automated systems), but explains the 9-minute human detection latency.
**Kill test**: If automated monitoring WAS running and DID fire an alert during the window, then human response time is the issue, not the monitoring. If no automated monitoring exists at all, then time-of-day is irrelevant to the root cause.
**Relevance**: LOW as root cause. Contributing factor for response latency at most.

### W5: Architecture mismatch -- pull vs push
**Mechanism**: The entire diagnostic ledger is a PULL system (RPC request/response). For alerting, you need PUSH (event-driven, or an external polling loop). The skill explicitly describes `getForkDiagnostic` as an on-demand RPC. The classifier runs only when the RPC is called. No background classification, no event subscription, no webhook. This is an architectural category error: the team built an investigation tool (pull-based, post-hoc) and assumed it would also serve as a monitoring tool (push-based, real-time). These are fundamentally different systems.
**Kill test**: Search the codebase for any code that calls `classify()` outside of an RPC handler (e.g., in a periodic task, background thread, or event listener). If such a call exists, the architecture is not purely pull-based, and this hypothesis is weakened.
**Relevance**: HIGH. This is the structural root cause that explains ALL of H1-H7 simultaneously. Even if every layer works perfectly (events emitted, persisted, classifiable), the pull-based architecture means nobody sees the classification until they manually request it. The system is a forensic tool, not a monitoring tool.

### W6: Ledger pruning destroying evidence
**Mechanism**: LEDGER-SCHEMA states: "Production default retention and max_events: NOT set in this codebase (caller-determined)." If the writer task prunes events aggressively (or if default parameters result in very short retention), the fork events from the 9-minute window might have been evicted before anyone investigated.
**Kill test**: Query `getForkDiagnostic` on N3 for the incident window (h=284670-284685). If events ARE present, pruning did not destroy them. If events are absent AND `events_dropped_total` is 0 AND `events_written_total` is nonzero, pruning is the suspect.
**Relevance**: LOW. This would explain post-hoc evidence loss, not the real-time detection failure. The issue is that no automated system checked during the 9 minutes, not that evidence was destroyed afterward.

### W7: Single-node fork below fleet divergence threshold
**Mechanism**: `getFleetForkDiagnostic` builds a `divergence_table` from `BlockApplied` events across peers. If only N3 had the fork hash and 11 other nodes agreed on canonical, the divergence_table would have one entry with low weight. But more critically: `getFleetForkDiagnostic` is ALSO an on-demand RPC that nobody calls automatically (same as W5). Even if the divergence_table would perfectly detect the fork, it requires a human to call the RPC.
**Kill test**: If any automated consumer calls `getFleetForkDiagnostic` periodically, this hypothesis needs a more nuanced test about thresholds. If no automated consumer exists, the fleet aggregation level doesn't matter.
**Relevance**: LOW independently, subsumed by W5 (pull vs push problem).

### W8: Classification confidence floor filtering
**Mechanism**: Some downstream consumer might filter by `confidence > 0.9`, dropping `TipRaceNatural` classifications (conf 0.70) silently. If the fork is classified as TipRaceNatural (which is likely given the fork shape without RecoveryClassifyCall events), and a consumer filters by high confidence, the classification is effectively invisible.
**Kill test**: Grep the codebase for any code that reads `classification.confidence` and applies a threshold filter. If no such code exists, this hypothesis is dead -- there IS no downstream consumer at all, making the filtering irrelevant (subsumed by W5/H6).
**Relevance**: LOW. The problem is not that the classification is filtered -- the problem is that no code reads the classification at all.

### W9: Consecutive-slot fork vs ProducerEquivocation -- classifier mis-categorization
**Mechanism**: INC-I-090 had DIFFERENT producers at the SAME height but CONSECUTIVE slots (N3 at slot 291216 vs canonical at slot 291215). Rule (a) ProducerEquivocation requires "2x BlockApplied same height+producer, different hash" -- but these are DIFFERENT producers. Rule (a) does NOT match. Without RecoveryClassifyCall events, the classifier sees:
- 1 BlockApplied (N3's self-produced block at h=284677)
- 1 ForkBlockReceived (canonical block at h=284677)
This maps to rule (f) TipRaceNatural (conf 0.70, recommended_action: normal_operation) or POSSIBLY rule (e) TipRaceHighLatency if validation_duration > 2000ms.
The classifier is technically CORRECT -- a single ForkBlockReceived at a natural slot boundary IS a tip race. The problem is that the classifier cannot distinguish "tip race that resolves in 1 slot" from "tip race that results in 9 minutes of being stuck" without the RecoveryClassifyCall events that signal the node is stuck.
**Kill test**: If RecoveryClassifyCall events WERE emitted (253 of them), rule (h) ChainBreakLoop should fire (signal_d: `recovery_attempts > 20`). Query the ledger for RecoveryClassifyCall events in the window. If they exist, W9 is dead -- the classifier had enough information. If they do NOT exist, W9 is confirmed: the classifier's input was too narrow to distinguish benign from pathological.
**Relevance**: HIGH. This is the key coupling point. See Section 2 for full worked example.

### W10: Self-produced fork bypasses emit path
**Mechanism**: 95% of fork detection logic assumes the fork block comes FROM A PEER via gossip. When N3 produced its own block at h=284677, that block was not received via gossip -- it was produced locally and applied directly. The ForkBlockReceived event is triggered when a GOSSIP BLOCK arrives at a height already occupied. So the sequence is:
1. N3 produces and applies its own block (BlockApplied -- self-produced)
2. 55ms later, canonical block arrives via gossip at same height (ForkBlockReceived -- this should emit)
Step 2 should emit because the gossip block arrives at a height that's already occupied by N3's own block. The question is whether `handle_new_block()` in `block_handling.rs` treats "received gossip block at occupied height" identically regardless of whether the occupying block was self-produced or peer-received.
There's a subtle variant: what if `classify_gossip_block()` checks whether the existing block at that height was produced by the local node, and handles it differently (e.g., by immediately attempting a reorg instead of emitting ForkBlockReceived)? If the reorg path does NOT emit, the fork event is lost.
**Kill test**: Read `block_handling.rs` for `classify_gossip_block()` and check whether it branches on `is_self_produced` for the existing block. If it does NOT branch (treats all occupied heights identically), W10 is dead. If it DOES branch and the self-produced path skips the emit, W10 is confirmed.
**Relevance**: MEDIUM. See Section 3 for analysis.

### W11: Dead code bridge -- signal_stuck_fork() was the missing link
**Mechanism**: Evidence entry 826 found: "Dead code: signal_stuck_fork() at production_gate.rs:579 sets stuck_fork_signal flag, but take_stuck_fork_signal() at production_gate.rs:573 is never called. CONFIRMED via grep: 0 call sites." This function was DESIGNED to signal when a node is stuck on a fork. It exists. It compiles. Nobody calls `take_stuck_fork_signal()`. This was likely the intended bridge between the recovery coordinator (which knows the node is stuck) and the observability layer (which needs that signal to classify the event as pathological rather than benign). Someone implemented the signal mechanism but never wired the consumer side.
**Kill test**: Read `production_gate.rs` to verify `signal_stuck_fork()` and `take_stuck_fork_signal()`. Then search for ANY caller of `take_stuck_fork_signal`. If a caller exists (perhaps behind a feature flag or in dead test code), this hypothesis weakens. If truly zero callers, this confirms the bridge was designed but never completed.
**Relevance**: HIGH. This directly explains WHY the recovery coordinator's 253 iterations are invisible. The mechanism to propagate "I'm stuck on a fork" already exists in code form but is unwired.

### W12: NoOpEmitter selected at startup due to silent RocksDB failure
**Mechanism**: Per the architecture blueprint (Smell 5): "If a code path constructs the node with NoOpEmitter (e.g., due to a RocksDB open failure at startup), ALL diagnostic events for the entire session are silently dropped." The node runs normally for consensus, but diagnostic events go into a black hole. There's no log warning documented for this fallback.
**Kill test**: Query `getForkDiagnostic` on N3 RPC 8503. If `health.ledger_available = true` and `events_written_total > 0`, NoOpEmitter was NOT used. If `ledger_available = false` or the method returns "method not found," this hypothesis gains weight.
**Relevance**: MEDIUM-LOW. The subsystem was recently used for INC-I-086 (workflow 353-354, fixed hardcoded zeros), suggesting the emitter was working on mainnet at that time.

### W13: Temporal gap between BlockApplied (self) and ForkBlockReceived (gossip) causes classifier rule skip
**Mechanism**: N3 produced its block, then 55ms later received the canonical block. The BlockApplied event has `applied_at_ms = T`. The ForkBlockReceived event has `timestamp_ms = T + 55`. Rule (f) TipRaceNatural requires "ForkBlockReceived with latency < 500ms" -- but latency here means validation_duration_ms of the BlockApplied at the same height, not the time between events. If the self-produced block has a very LOW validation_duration (because the node validated its own block fast), rule (f) matches with conf 0.70. But what if the validation_duration_ms is not populated for self-produced blocks (since the node didn't validate them -- it built them)? If validation_duration_ms is None or 0 for self-produced blocks, the "< 500ms" check might be vacuously true or might not match at all.
**Kill test**: Check the BlockApplied payload structure for self-produced blocks. Is `validation_duration_ms` populated? If so, with what value? This determines whether rule (e) or (f) is the matching rule.
**Relevance**: LOW. Even if the rule assignment differs between (e) and (f), both produce `normal_operation` or `investigate_latency` -- neither triggers an alert.

### W14: INV-OBS-001 hardcoded-zeros regression
**Mechanism**: INV-OBS-001 states: "The DiagnosticHealth block returned by getForkDiagnostic RPC must reflect the live writer counters -- never hardcoded literals on the live production path." This was fixed in workflow 353-354 (INC-I-086, completed 2026-05-21). But what if the fix was deployed to the binary in the repo but NOT deployed to N3's running binary on mainnet? The deployed binary might still have the hardcoded zeros, meaning `events_written_total = 0` and `events_dropped_total = 0` even though the subsystem is working. This would make the health counters useless for investigation.
**Kill test**: Call `getForkDiagnostic` on N3 RPC 8503. If `health.events_written_total > 0`, the fix IS deployed. If `events_written_total = 0` and the ledger query returns events, the counter is still hardcoded. If `events_written_total = 0` and no events exist, either the subsystem is dead or there are genuinely no events.
**Relevance**: MEDIUM. Workflow 353-354 completed only 5 days before the incident. If the binary wasn't redeployed since then, the counter fix might not be live. This is a version skew variant of H7 but more specific.

## Section 2: W9 -- Consecutive-Slot Fork Classifier Outcome (Full Worked Example)

### Setup

From the incident ground truth:
- N3 block: hash=8ede1526, slot=291216, producer=54323cef (N3), height=284677
- Canonical block: hash=150b4a7b, slot=291215, producer=50fd1758, height=284677
- Both extend parent cefa9950 at h=284676
- N3 produced first, canonical arrived 55ms later

### Step 1: Events that SHOULD exist in the ledger

**If L1 emit works correctly:**
1. `BlockApplied { slot: 291216, block_hash: "8ede1526", producer_pubkey: "54323cef", mode: "SelfProduced", validation_duration_ms: ?, height: 284677, timestamp_ms: T1 }`
2. `ForkBlockReceived { block_hash: "150b4a7b", block_slot: 291215, block_height_estimate: 284677, producer_pubkey: "50fd1758", classification: "ForkBlock", fork_kind: "HeightOccupied", local_tip_hash: "8ede1526", local_tip_height: 284677, timestamp_ms: T1+55 }`

**If RecoveryClassifyCall events exist (the critical unknown):**
3-255. `RecoveryClassifyCall { local_height: 284677, network_tip_height: 284678+, recovery_attempts: 1..253, action_returned: "...", rule_matched: "..." }` x253

**If snap sync events exist:**
256. `SnapSyncAttempted { height: 284677, timestamp_ms: T_snap_start }`
257. `SnapSyncCompleted { height: ?, timestamp_ms: T_snap_end ~23:04:30 }`

### Step 2: Classifier rule matching

**Scenario A: Only events 1 + 2 exist (no RecoveryClassifyCall)**

Walk through rules in priority order:

**(a) ProducerEquivocation**: Requires 2x BlockApplied, same height, same producer, different hash.
- We have 1 BlockApplied (N3's block) and 1 ForkBlockReceived (canonical).
- These are DIFFERENT event kinds (BlockApplied vs ForkBlockReceived).
- Even if both were BlockApplied, producers are DIFFERENT (54323cef vs 50fd1758).
- **RULE DOES NOT MATCH.**

**(b) EpochBoundaryInvalid**: Requires BlockRejected at epoch boundary with "EpochReward".
- No BlockRejected event.
- **RULE DOES NOT MATCH.**

**(c) RollbackLoop**: Requires >3 RollbackStarted in 60s.
- No RollbackStarted events.
- **RULE DOES NOT MATCH.**

**(d) PostSnapDeadTip**: Requires SnapSyncCompleted then ForkBlockReceived within 300s.
- If SnapSyncCompleted exists (event 257) but arrives AFTER the ForkBlockReceived (event 2), the temporal ordering is reversed: ForkBlockReceived came first, then SnapSyncCompleted. The rule checks "SnapSyncCompleted THEN ForkBlockReceived" -- but the fork happened BEFORE the snap sync resolved it. **RULE LIKELY DOES NOT MATCH** because the ordering is wrong (snap sync came after the fork event, not before it).
- If however SnapSyncCompleted comes first and THEN a new ForkBlockReceived arrives (post-snap dead tip), the rule would match. But in this incident, the fork preceded the snap sync.

**(h) ChainBreakLoop**: Requires any of 4 signals in 1h window:
- signal_a: chain_break_count > 3 -- requires ChainBreakDetected events. None exist.
- signal_b: fork_recv > 100 AND fork/applied ratio > 10 -- only 1 ForkBlockReceived. NOT MET.
- signal_c: rollback_count > 10 -- no RollbackStarted events. NOT MET.
- signal_d: recovery_attempts > 20 -- NO RecoveryClassifyCall events in this scenario. NOT MET.
- **RULE DOES NOT MATCH** without RecoveryClassifyCall events.

**(e) TipRaceHighLatency**: Requires ForkBlockReceived + BlockApplied at same height with validation_duration_ms > 2000.
- ForkBlockReceived at h=284677 exists. BlockApplied at h=284677 exists.
- Question: what is validation_duration_ms for N3's self-produced block? If N3 produced the block, it didn't "validate" it in the traditional sense -- it built it. The validation_duration_ms might be 0 or very low.
- If validation_duration_ms < 2000: **RULE DOES NOT MATCH.**
- If validation_duration_ms > 2000: Rule matches, fork_type = TipRaceHighLatency, confidence = 0.75, recommended_action = "investigate_latency". Still not an alert-worthy action.

**(f) TipRaceNatural**: Requires ForkBlockReceived + low latency + no other signals in correlation group.
- ForkBlockReceived exists at h=284677.
- BlockApplied exists at h=284677 with likely low validation_duration_ms (self-produced).
- No other signals (no RollbackStarted, no ChainBreakDetected, no RecoveryClassifyCall).
- **RULE MATCHES.** fork_type = TipRaceNatural, confidence = 0.70, recommended_action = "normal_operation".

### Step 3: Verdict for Scenario A

**The classifier returns `TipRaceNatural` with `recommended_action: normal_operation` and confidence 0.70.**

This is technically CORRECT for the input it receives. A single ForkBlockReceived at a natural slot boundary IS a tip race. The problem is that the classifier has no way to know that the tip race resulted in a 9-minute stuck state, because the RecoveryClassifyCall events that would signal "stuck" were (probably) never emitted.

### Step 4: Scenario B -- with RecoveryClassifyCall events

If all 253 RecoveryClassifyCall events exist:

**(h) ChainBreakLoop**: signal_d requires recovery_attempts > 20 in 1h window.
- 253 RecoveryClassifyCall events in 9 minutes >> 20.
- **RULE MATCHES.** fork_type = ChainBreakLoop, confidence = 0.85, recommended_action = "restart_with_resync".

This is the CORRECT classification. The system would correctly identify a stuck node needing intervention.

### Step 5: Delta between Scenario A and B

The SOLE difference is the presence or absence of `RecoveryClassifyCall` events. Without them, the classifier sees a benign tip race. With them, it sees a chain break loop requiring operator action.

**Conclusion for W9**: The classifier is not mis-categorizing. It is correctly classifying based on incomplete input. The root cause is upstream: the recovery coordinator's iterations are not emitting diagnostic events. This makes W9 a CONFIRMED pathway to the symptom, but the root cause is in L1 (emit gap in recovery.rs), not L3 (classifier logic).

## Section 3: W10 -- Self-Produced Fork Emit-Path Bypass

### Theoretical Analysis (code reading blocked by pipeline gate)

Per the skill documentation and architecture blueprint:

**Expected behavior**: When N3 receives a gossip block (canonical 150b4a7b) at h=284677, `handle_new_block()` calls `classify_gossip_block()`. Since h=284677 is already occupied by N3's own block (8ede1526), the classification should be `BlockClass::ForkBlock(HeightOccupied)`. This should trigger the emit site at block_handling.rs:195-258 (per skill).

**Potential bypass**: The question is whether `classify_gossip_block()` has special handling when the existing block at the height was self-produced. Possible scenarios:

1. **No special handling** (most likely): `classify_gossip_block()` checks `chain_state.best_height` and `chain_state.best_hash`, sees height occupied, returns `ForkBlock(HeightOccupied)` regardless of who produced the existing block. The emit proceeds normally.

2. **Self-produced reorg shortcut** (possible): `classify_gossip_block()` detects that the incoming block has a better slot (291215 < 291216) and the existing block is self-produced, so it fast-paths to a reorg attempt. If the reorg code path does NOT emit ForkBlockReceived before attempting the rollback, the event is missed.

3. **FORK_GUARD special case** (possible): The FORK_GUARD detection in block_handling.rs might handle "received better block at my occupied height" as a special case that logs textually (FORK_GUARD) but takes a different code path than the standard ForkBlockReceived emit.

**Evidence for scenario 1**: The analyst report states "FORK_GUARD signals visible in /var/log/doli/mainnet/n3.log.1" -- this proves the code path for fork detection was entered. The FORK_GUARD textual log and the DiagnosticEmitter::record() call are described as being at the "same code site" in block_handling.rs. If the FORK_GUARD log was produced, the emit call should also have been reached.

**Evidence against scenario 2/3**: The skill explicitly states emit happens for ALL ForkBlock classifications: "ForkBlockReceived on every non-tip gossip block" (DEPENDENCIES line 85). No mention of self-produced exemption.

**Verdict for W10**: Scenario 1 is most likely. The ForkBlockReceived event was PROBABLY emitted for the canonical block arriving at N3's occupied height. W10 is **weakly implausible** but cannot be definitively killed without reading the actual code.

### Key caveat

Even if ForkBlockReceived WAS emitted, this only places ONE event in the ledger. Without RecoveryClassifyCall events from the subsequent 9-minute recovery loop, the classifier still sees TipRaceNatural (per W9 analysis). So W10 is likely not the PRIMARY gap even if it exists.

## Section 4: Other Non-Obvious Factors Found

### F1: dead code bridge (signal_stuck_fork) suggests designed-but-unwired integration

Evidence entry 826 reveals `signal_stuck_fork()` at `production_gate.rs:579` and `take_stuck_fork_signal()` at `production_gate.rs:573`. The signal is SET by the recovery coordinator (or production gate) when it detects a stuck fork. The signal is supposed to be TAKEN by... something. But `take_stuck_fork_signal()` has zero callers.

This is strong evidence that the integration between the recovery coordinator and the observability/action layer was DESIGNED but never COMPLETED. The function names suggest a producer-consumer pattern:
- `signal_stuck_fork()` = producer (recovery side says "I'm stuck")
- `take_stuck_fork_signal()` = consumer (observability/action side reads the flag)

The consumer was never wired. This is a partially implemented feature, not a missing design. Someone wrote the plumbing for both sides but never connected them.

### F2: INV-OBS-001 fix timeline creates version skew risk

INV-OBS-001 (hardcoded zeros in DiagnosticHealth) was fixed in workflow 353-354 on 2026-05-21 -- just 5 days before the incident on 2026-05-25. The question is whether the fix was deployed to N3's running binary in that window. If not, the health counters on N3 might still be hardcoded zeros, making the L2 persist investigation unreliable.

### F3: fork-monitor.sh's getChainInfo polling creates a structural blind spot

`fork-monitor.sh` polls `getChainInfo`, which returns `{bestHeight, bestHash}`. It groups by `bestHash` and reports >1 group as FORK. But consider:
- During the 9-minute window, N3 was STUCK at h=284677 with bestHash=8ede1526
- The rest of the fleet was advancing: h=284678, 284679, ...
- Each fleet node has a DIFFERENT bestHash at each height (because each slot has a different block)
- The script polls at discrete intervals

At poll time T (say 2 minutes after fork):
- N3: bestHeight=284677, bestHash=8ede1526
- Seed: bestHeight=284689, bestHash=<some hash at 284689>
- N1: bestHeight=284689, bestHash=<same hash as seed, probably>

The script groups by bestHash. It sees 3 groups if each node is at a different height... wait, no. The fleet nodes should all be at the SAME height (within a few seconds) and have the SAME bestHash. N3 is the outlier.

So the script sees TWO groups: {fleet at h=284689, hash=X} and {N3 at h=284677, hash=8ede1526}. This IS a divergence and should trigger FORK exit code.

**BUT**: The script treats this as a hash divergence, which it is. The ACTION text says "Run scripts/emergency-halt.sh to stop all producers." That's a high-severity response. Would the operator treat a single-node lag as a reason to halt the entire fleet? More likely, the operator would see N3 lagging and assume it's a sync issue, not a fork. The script's output doesn't distinguish "N3 is on a different chain" from "N3 is behind." Both show as different bestHash groups.

This is a W2 (cognitive ergonomics) variant: the script correctly detects the divergence but its output frames it as "FORK" which might be dismissed as "just N3 being slow."

### F4: No escalation path from classification to action

Even if the classifier returns `ChainBreakLoop` with `recommended_action: restart_with_resync`, there is no code that:
1. Reads the classification result outside an RPC handler
2. Sends an alert (email, Slack, webhook, systemd notification)
3. Takes automated action (stop/restart the node)
4. Logs to a centralized monitoring system (Prometheus, Grafana, etc.)

The entire action surface is: "return JSON in RPC response." The recommended_action is advisory text in a response body that requires a human to actively request and read.

## Section 5: Hypothesis Verdicts

| ID | Hypothesis | Verdict | Confidence | Basis |
|----|-----------|---------|-----------|-------|
| W1 | Human/process failure -- runbook never matured | **plausible** | conf(0.65, inferred) | Skill does not document deployment; analyst flagged; consistent with all symptoms |
| W2 | Cognitive ergonomics -- data not visually distinctive | **needs evidence** | conf(0.35, assumed) | Cannot verify explorer UI; user DID notice eventually, suggesting partial visibility |
| W3 | Alert fatigue / threshold tuning | **needs evidence** | conf(0.40, inferred) | Cannot read fork-monitor.sh code; depends on whether script is deployed at all |
| W4 | Time-of-day / staffing | **plausible but not root cause** | conf(0.30, assumed) | Contributing factor at most; irrelevant if no automated monitoring exists |
| W5 | Architecture mismatch -- pull vs push | **plausible (structural)** | conf(0.70, inferred) | Strongest architectural explanation; pull-based system cannot alert; fully consistent with symptom |
| W6 | Ledger pruning destroying evidence | **implausible** | conf(0.15, inferred) | Would explain post-hoc evidence loss, not real-time detection failure |
| W7 | Single-node fork below fleet threshold | **implausible** | conf(0.15, inferred) | Subsumed by W5; fleet aggregator also pull-based |
| W8 | Classification confidence floor filtering | **implausible** | conf(0.10, inferred) | No downstream consumer exists to filter; subsumed by W5 |
| W9 | Consecutive-slot fork classified as benign TipRaceNatural | **plausible (confirmed pathway)** | conf(0.65, inferred) | Full worked example shows TipRaceNatural classification without RecoveryClassifyCall events |
| W10 | Self-produced fork bypasses emit path | **weakly implausible** | conf(0.25, inferred) | FORK_GUARD logs suggest emit path was reached; no documented self-produced exemption |
| W11 | Dead code bridge -- signal_stuck_fork never wired | **plausible (supporting)** | conf(0.60, inferred) | Evidence entry 826 confirms zero callers of take_stuck_fork_signal(); designed but incomplete |
| W12 | NoOpEmitter selected at startup | **needs evidence** | conf(0.20, inferred) | Requires RPC probe on N3; recent INC-I-086 work suggests emitter was working |
| W13 | Validation duration_ms not populated for self-produced blocks | **needs evidence** | conf(0.20, assumed) | Even if true, both rule (e) and (f) produce non-alerting actions |
| W14 | INV-OBS-001 hardcoded-zeros regression (version skew) | **needs evidence** | conf(0.30, inferred) | 5-day window between fix and incident; requires binary artifact check |

## Section 6: Top 3 Wildcards Ranked by Probability x Explanatory Power

### Rank 1: W5 -- Architecture Mismatch (Pull vs Push) -- conf(0.70, inferred)

**Why top**: This is the minimal explanation that accounts for ALL symptoms simultaneously. Even if L1-L3 work perfectly (events emitted, persisted, classified correctly as ChainBreakLoop), the pull-based architecture means the classification sits in RocksDB until someone manually calls `getForkDiagnostic`. No background task, no polling loop, no webhook, no alert channel. The system is an investigation tool, not a monitoring tool. This explains:
- Why the user noticed visually (the only "push" channel is human eyes on a dashboard)
- Why no automated alert fired (there IS no automated alert mechanism)
- Why fork-monitor.sh (which IS a polling tool) doesn't consume getForkDiagnostic (it was built to poll getChainInfo, a simpler endpoint)

**Kill test**: Find ANY code path that calls `classify()` or reads `recommended_action` outside an RPC request handler. If such code exists, the architecture is not purely pull-based. The synthesizer should verify this.

### Rank 2: W9 -- TipRaceNatural Mis-classification Due to Missing RecoveryClassifyCall Events -- conf(0.65, inferred)

**Why second**: This identifies the exact mechanism by which a pathological 9-minute stuck state is indistinguishable from a benign 1-second tip race in the classifier's view. The worked example in Section 2 shows that without RecoveryClassifyCall events, the classifier MUST return TipRaceNatural. This is not a classifier bug -- it is correct given its input. The bug is that the input is incomplete because recovery.rs does not emit diagnostic events.

**Kill test**: Query N3's diagnostic ledger for RecoveryClassifyCall events in the incident window. If they exist, this hypothesis is dead and the classifier had sufficient input. If they do not exist, this confirms the L1 gap in the recovery path.

### Rank 3: W11 -- Dead Code Bridge (signal_stuck_fork) -- conf(0.60, inferred)

**Why third**: This provides archaeological evidence of INTENT. Someone designed the stuck-fork signaling mechanism (signal_stuck_fork / take_stuck_fork_signal). It was implemented at the signal-producer side but never wired at the consumer side. This suggests the team KNEW the recovery coordinator needed to communicate "I'm stuck" to an action layer, started building the bridge, and never finished it. This is not a design gap -- it is an implementation gap.

**Kill test**: Verify zero callers of `take_stuck_fork_signal()`. If there IS a caller (perhaps behind a feature flag), the bridge may be partially wired and the gap is narrower than assumed.

## Cross-Layer Signals

1. **Evidence entry 826** (dead code: signal_stuck_fork) was found during the FORK root-cause investigation, not the observability investigation. Other investigators may not have seen it. The synthesizer should cross-reference this with the L1 code investigator's findings about recovery.rs emit sites.

2. **INV-OBS-001 timeline**: The hardcoded-zeros fix (workflow 353-354, completed 2026-05-21) is only 5 days before the incident. The state investigator should verify whether N3's deployed binary includes this fix by checking `health.events_written_total` in the getForkDiagnostic RPC response.

3. **INC-I-086 workflow 353-354**: This was a fix for the SAME observability subsystem just 5 days prior. Any binary redeployment for that fix would include the latest code. If the fix was NOT deployed, N3 may be running a pre-observability binary (supporting H7). If it WAS deployed, the subsystem should be functional.

4. **The "pull vs push" gap (W5)** is a DESIGN issue, not a CODE bug. It cannot be killed by code inspection alone -- it requires examining the operational environment (dashboards, scripts, cron jobs, systemd timers) to determine if any external consumer fills the push role.

## Gaps

1. **Cannot read source code** due to pipeline gate (investigation mode requires parallel sub-agents). All code-path analysis is based on skill documents and architecture blueprints, not direct code reading. Key files that need verification:
   - `classifier.rs` -- verify 8-rule priority order matches schema
   - `block_handling.rs` -- verify ForkBlockReceived emit for HeightOccupied when existing block is self-produced
   - `recovery.rs` -- verify presence/absence of RecoveryClassifyCall emit
   - `production_gate.rs` -- verify signal_stuck_fork/take_stuck_fork_signal dead code
   - `fork-monitor.sh` -- verify hash-grouping logic for different-height nodes

2. **Cannot probe mainnet RPCs** -- all live system probes are investigator-state's lane.

3. **Cannot verify binary deployment timeline** -- version skew (W14) requires ssh ai1 artifact inspection.

4. **Cannot verify explorer/dashboard UI** -- cognitive ergonomics (W2) requires examining the explorer codebase.
