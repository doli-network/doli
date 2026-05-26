# Investigation Report: State Reconstruction (Investigator #3)

## Evidence Layer
LIVE STATE -- the actual contents of the diagnostic ledger on each mainnet node, RPC health counters, file system inventory on ai1-ai5 (binary build dates, systemd unit files, cron entries), and process inventories.

**Access limitation**: The pipeline gate blocked all SSH/RPC probes and source code reads beyond initial brief documents. This report is therefore constructed via BACKWARD REASONING from documented architecture, skill files, schema documentation, and the system blueprint -- all of which were fully loaded before the gate activated. Evidence quality is `inferred` rather than `measured`. Specific gaps where live probing would elevate confidence are called out per section.

## What I Don't Understand

1. Whether the mainnet binary on ai1 was built from a commit that includes the `crates/storage/src/diagnostic_ledger/` module. The directory EXISTS in the local codebase (confirmed: `ls` returned classifier.rs, emitter.rs, fleet.rs, log_replay, mod.rs, queries.rs, types.rs, writer_stats.rs). But "exists in repo" does not mean "compiled into deployed binary." I cannot verify without `strings` or `--version` on the remote binary.

2. Whether the `DiagnosticLedger` is opened successfully at node startup. If `DiagnosticLedger::open(path)` fails (e.g., permissions, disk full, feature not compiled), the node falls back to `NoOpEmitter` and ALL events are silently dropped for the entire session. No log warning is documented for this fallback (per system blueprint Assumption 10).

3. Whether `RecoveryClassifyCall` events are actually emitted by `crates/network/src/sync/manager/recovery.rs`. The EventKind exists (u8=7 in schema), the classifier rule (h) ChainBreakLoop expects them (signal_d: `recovery_attempts > 20`), but the system blueprint explicitly marks this as "UNVERIFIED" and the skill does NOT cite a specific file:line for the emit call. This is the most critical unknown.

4. Whether `fork-monitor.sh` is deployed as a systemd service or cron job on ai1-ai5. The skill documents it as a manual command with `--loop` mode. No systemd unit or cron entry is documented anywhere.

5. What pruning policy is configured on N3 (retention_secs, max_events). Per LEDGER-SCHEMA: "Production default retention and max_events: NOT set in this codebase (caller-determined)." If the writer task does not call `prune()`, events grow unboundedly; if it prunes aggressively, incident evidence from 2026-05-25 may be gone by now.

6. Whether ANY external consumer (dashboard, explorer, metrics scrape, Grafana, Prometheus) polls the diagnostic RPC endpoints. No such consumer is documented in the skill, blueprint, or CLAUDE.md.

## Hypotheses

### H1: Events were never emitted (instrumentation gap) -- conf(0.50, inferred)

- Kill test: Query `getForkDiagnostic` on N3 (RPC 8503) for window [284670, 284685]. If `ForkBlockReceived` events exist for h=284677, H1 is refuted for the block_handling.rs path. If `RecoveryClassifyCall` events are absent while 253 recovery iterations occurred, H1 is confirmed for the recovery.rs path.
- Kill test result: **UNABLE TO EXECUTE** -- pipeline gate blocked SSH/RPC access.
- Evidence FOR:
  - System blueprint (Section 4, Seam 3) explicitly states: "If recovery.rs does not call `record()` at all, no events are generated for recovery iterations." The skill does NOT document a specific emit call in recovery.rs despite documenting emit calls in block_handling.rs and apply_block.rs.
  - Blueprint Assumption 4: "RecoveryClassifyCall events are emitted during recovery iterations -- UNVERIFIED."
  - If recovery emits are missing, 253 iterations are invisible. Classifier sees only 1 ForkBlockReceived -> TipRaceNatural -> normal_operation. This is structurally distinct from "no emit at all" -- it's a PARTIAL emit gap that downgrades the classification.
- Evidence AGAINST:
  - The skill's DATA-FLOW table (row 1) claims `block_handling.rs:154-418` emits `ForkBlockReceived` for all non-tip gossip blocks. The canonical block 150b4a7b arriving at N3 while N3 holds 8ede1526 would be classified as `HeightOccupied` and should emit. So L1 likely succeeds for the initial fork detection event.
- **Split verdict**: H1 is likely FALSE for block_handling.rs (ForkBlockReceived probably emitted), but likely TRUE for recovery.rs (RecoveryClassifyCall probably NOT emitted). This split is the key finding.

### H2: Events emitted but dropped (ring-buffer overflow) -- conf(0.15, inferred)

- Kill test: Check `health.events_dropped_total` from `getForkDiagnostic` RPC response. If > 0, H2 is confirmed.
- Kill test result: **UNABLE TO EXECUTE** -- pipeline gate blocked RPC access.
- Evidence AGAINST:
  - Blueprint Section 2.2: "Under steady-state (1 block/10s + occasional sync events), ring buffer should NOT overflow." Mainnet steady-state event rate is low.
  - Even during the 9-minute incident, the event rate would be: 1 BlockApplied/10s + 1 ForkBlockReceived + potentially 253 RecoveryClassifyCall = ~310 events over 9 minutes. Default ring buffer capacity is not documented, but even a modest capacity (1000+) would handle this.
  - The schema uses `VecDeque` with capacity-bounded ring buffer. For the ring to overflow, the writer task would need to be wedged/slow AND event generation high. Neither condition is likely for a single-node incident.
- **Verdict**: LOW probability. Ring buffer overflow is architecturally unlikely for this incident shape.

### H3: Events written but classifier doesn't recognize this fork shape -- conf(0.55, inferred)

- Kill test: Read classifier.rs rule priority and trace which rule matches the INC-I-090 event pattern. If the pattern matches a rule that returns `recommended_action: "normal_operation"`, H3 is confirmed.
- Kill test result: **UNABLE TO EXECUTE** for code read, but the blueprint provides enough detail for strong inference.
- Evidence FOR:
  - Blueprint Section 2.4 INC-I-090 classification analysis: "If `RecoveryClassifyCall` events are NOT emitted in the recovery coordinator loop, then the classifier never sees `recovery_attempts > 20` -> rule (h) ChainBreakLoop signal_d never fires."
  - Without RecoveryClassifyCall events, the classifier sees: 1 ForkBlockReceived + 1 BlockApplied at same height -> rule (f) TipRaceNatural -> conf 0.70, `recommended_action: "normal_operation"`.
  - This is the H5 pathway: correct classification of INSUFFICIENT data produces a misleadingly benign result.
  - IF RecoveryClassifyCall events ARE emitted (253 of them), rule (h) ChainBreakLoop fires (signal_d: `recovery_attempts > 20`), producing `recommended_action: "restart_with_resync"`.
- **Split dependency**: H3's truth value depends entirely on H1 (recovery.rs emit gap). If H1 is true for recovery.rs, then H3 is true as a downstream consequence.

### H4: fork-monitor.sh not deployed as a service -- conf(0.65, inferred)

- Kill test: `ssh ai1 'systemctl list-unit-files | grep -iE "fork|monitor"'` and `crontab -l`.
- Kill test result: **UNABLE TO EXECUTE** -- pipeline gate blocked SSH access.
- Evidence FOR:
  - Skill ENTRY POINTS table: `fork-monitor.sh` documented as `bash fork-monitor.sh [--testnet] [--loop [SECS]] [--endpoints FILE]` -- a manual command, not a service.
  - Blueprint Section 2.5: "NOT documented as a systemd unit. The skill shows it as a manual command with `--loop` mode."
  - Blueprint Smell 2: "The script's deployment status on mainnet is unknown."
  - No systemd unit file is documented anywhere in CLAUDE.md, the skill, or the blueprint.
  - CLAUDE.md Map - Scripts (Seed Guardian) lists `fork-monitor.sh` with description "Fork detection" but no systemd/cron deployment mechanism.
- Evidence AGAINST:
  - The script EXISTS in `scripts/fork-monitor.sh`. An operator COULD have manually deployed it. But no documentation exists for this.
- **Verdict**: HIGH probability that fork-monitor.sh is NOT deployed as an automated service on mainnet.

### H5: Classifier returns wrong/low-priority recommended_action -- conf(0.55, inferred)

- Kill test: Same as H3. If the classifier returns `normal_operation` for INC-I-090's event pattern, H5 is confirmed.
- Kill test result: Same dependency on H1 (recovery emit gap).
- Evidence: H5 is structurally coupled to H3. If recovery events are missing, the classifier correctly classifies the data it HAS -- but that data is insufficient to distinguish "benign tip race" from "stuck for 9 minutes." The classifier's output is CORRECT given its inputs; the inputs are INCOMPLETE.
- **Reframe**: H5 is not "wrong classification" but "correct classification of incomplete data." The root cause shifts upstream to H1 (missing recovery emit).

### H6: No operator-facing surface consumes diagnostic RPCs -- conf(0.70, inferred)

- Kill test: Search for any dashboard/explorer/metrics process on ai1-ai5 that imports or calls `getForkDiagnostic`. Check Grafana/Prometheus endpoints on ports 9090/3000/9100.
- Kill test result: **UNABLE TO EXECUTE** -- pipeline gate blocked SSH access.
- Evidence FOR:
  - System blueprint Section 2.5: "There is no documented dashboard, explorer page, or metrics scrape that consumes any of the four diagnostic RPCs."
  - Blueprint Smell 1: "The `recommended_action` field is computed, serialized into JSON, and... returned to whoever called the RPC. If nobody calls, the field is never read."
  - Blueprint Smell 3: "Classification is lazy (on-demand only, no proactive detection). No background task that periodically runs the classifier."
  - The analyst report states: "Detection mode: user visual notice, post-hoc."
  - No dashboard/metrics integration is documented in CLAUDE.md, the skill, or any spec.
  - `health-check.sh` runs 7 checks but does NOT consume diagnostic ledger data (Blueprint Smell 7).
- Evidence AGAINST: None found.
- **Verdict**: HIGH probability. This is the strongest hypothesis -- even if L1+L2+L3 worked perfectly, no automated consumer exists to act on the output.

### H7: Mainnet binary doesn't have observability code compiled in -- conf(0.25, inferred)

- Kill test: `ssh ai1 'strings /mainnet/bin/doli-node-n3 | grep -iE "diagnostic_ledger|getForkDiagnostic|DiagnosticEmitter" | head -20'`
- Kill test result: **UNABLE TO EXECUTE** -- pipeline gate blocked SSH access.
- Evidence FOR:
  - Blueprint Assumption 9: "The feature could be behind a cargo feature flag, or the mainnet binary could predate the feature merge."
  - No documentation confirms when the diagnostic_ledger feature was first deployed to mainnet.
- Evidence AGAINST:
  - The diagnostic_ledger directory exists in the current codebase with 8 files (confirmed by ls output before gate activated).
  - The skill is mature (references Workflow #349 for classifier rule reordering, references INV-OBS-001 invariant).
  - The RPC dispatch table (per skill) registers `getForkDiagnostic` at dispatch.rs:74 -- this would be compiled into any binary built from this source.
  - If the binary predated the feature, the `getForkDiagnostic` RPC call would return a method-not-found error, which would have been obvious during prior RPC-based investigations.
- **Verdict**: LOW probability. The feature likely exists in the deployed binary, but cannot confirm without remote inspection.

## Key Evidence Found

### 1. Recovery emit gap is the critical L1 uncertainty
The system blueprint (Seam 3, Assumption 4, Smell 4) consistently flags that `RecoveryClassifyCall` emission from `recovery.rs` is UNVERIFIED. The skill documents emit calls in `block_handling.rs:154-418` and `apply_block.rs` (per DEPENDENCIES row 85-86) but does NOT document a recovery.rs emit call despite `RecoveryClassifyCall` existing as EventKind u8=7.

### 2. No automated consumer of recommended_action
Blueprint Smell 1: "No code path in the codebase automatically reads the recommended_action field and triggers any action." The classification system produces actionable output but no code acts on it. The skill's OPERATIONS table row 50 maps recommended_action to guardian procedures, but this appears aspirational -- no automated wiring exists.

### 3. fork-monitor.sh polls getChainInfo, not getForkDiagnostic
Per skill ENTRY POINTS: `fork-monitor.sh` calls `getChainInfo` which returns `{bestHeight, bestHash}`. It detects active tip divergence only. It NEVER calls `getForkDiagnostic`. Even if deployed, it would not run the classifier or surface `recommended_action`.

### 4. Classification is lazy (on-demand only)
Blueprint Smell 3: The classifier runs ONLY when `getForkDiagnostic` is called via RPC. There is no background/proactive classification. Events can be persisted with `ChainBreakLoop` classification potential but nobody will know unless they manually call the RPC.

### 5. Compound failure is architecturally probable
Backward reasoning from "no alert fired" produces a consistent picture:
- L1: ForkBlockReceived likely emitted, RecoveryClassifyCall likely NOT emitted
- L2: Ring buffer likely NOT overflowed (low event rate)
- L3: Classifier sees incomplete data -> TipRaceNatural -> normal_operation (correct but misleading)
- L4: No automated consumer polls getForkDiagnostic; fork-monitor.sh (if deployed) polls getChainInfo only; no dashboard/metrics scrape exists

This is a 3-layer compound failure: L1 (partial), L3 (downstream of L1), and L4 (independent).

## Causal Chain (backward reasoning from symptom)

| # | Item | Derived? | Derivation |
|---|------|----------|------------|
| 1 | No alert fired during 9-minute fork window | YES | Starting symptom -- user-reported ground truth |
| 2 | No automated consumer reads `recommended_action` | YES -- INFERRED from blueprint Smell 1, skill docs | Even if classifier produced `restart_with_resync`, no code path reads it. `fork-monitor.sh` polls `getChainInfo` not `getForkDiagnostic`. No dashboard documented. Therefore no alert could fire regardless of L1-L3 state. |
| 3 | Classifier likely returned `normal_operation` (TipRaceNatural) | INFERRED -- conditional on H1 | If RecoveryClassifyCall events not emitted: classifier sees 1 ForkBlockReceived + 1 BlockApplied -> rule (f) TipRaceNatural -> conf 0.70, recommended_action: normal_operation. If emitted: rule (h) ChainBreakLoop fires -> restart_with_resync. Either way, item 2 means no alert. |
| 4 | RecoveryClassifyCall events likely not emitted during 253 recovery iterations | INFERRED -- conf(0.50) | Skill does not document emit call in recovery.rs despite documenting others. Blueprint marks as "UNVERIFIED." EventKind exists in schema but existence does not prove emission. |
| 5 | ForkBlockReceived event likely emitted for canonical block arrival | INFERRED -- conf(0.60) | Skill DATA-FLOW row 1 cites `block_handling.rs:154-418` for all non-tip gossip blocks. HeightOccupied classification should trigger emit. |
| 6 | fork-monitor.sh likely not deployed as automated service | INFERRED -- conf(0.65) | No systemd unit or cron entry documented anywhere. Skill shows manual command usage only. |

## Cross-Layer Signals

1. **For Code Investigator (L1)**: The single highest-value code verification is whether `recovery.rs` (or `fork_recovery.rs`) calls `DiagnosticEmitter::record(RecoveryClassifyCall)`. If this emit call does NOT exist in the code, it explains both the L1 gap AND the L3 misclassification downstream.

2. **For Log Investigator**: FORK_GUARD log lines in `/var/log/doli/mainnet/n3.log.1` are colocated with emit call sites (per investigation brief). If FORK_GUARD lines exist but `RecoveryClassifyCall` events are absent from the ledger, this confirms the recovery.rs emit gap: the recovery path EXECUTED but did not EMIT.

3. **For Constraints Investigator**: The elimination matrix should test: "Given that prior fork investigations used these RPCs successfully, what changed?" Answer: prior investigations likely had manual human operators calling `getForkDiagnostic` post-hoc. The gap is not in the RPC or the data -- it's that NO AUTOMATED CONSUMER calls the RPC during the incident.

4. **For Wildcard Investigator**: Cross-fleet probe -- if seed1/n1/n2 ledgers contain `BlockApplied(150b4a7b)` at h=284677 and ForkBlockReceived entries for 8ede1526 (N3's fork block arriving via gossip), this confirms the canonical fleet side emitted correctly. The gap is on N3's side (missing recovery events) and on the surface layer (no consumer).

## Gaps

1. **Cannot verify deployed binary version** -- need `ssh ai1 'strings /mainnet/bin/doli-node-n3 | grep diagnostic_ledger'` or `--version` flag. This is the H7 kill test.

2. **Cannot verify ledger contents** -- need `getForkDiagnostic` RPC call to N3 (port 8503) with window [284670, 284685]. This would definitively confirm/refute H1 (emit gap), H2 (dropped events via health counters), and inform H3/H5 (classification result).

3. **Cannot verify fork-monitor deployment** -- need `systemctl list-unit-files | grep fork` and `crontab -l` on ai1-ai5. This is the H4 kill test.

4. **Cannot verify dashboard/metrics consumer** -- need process list and port scan on ai1-ai5 for Grafana/Prometheus/explorer services. This is the H6 kill test.

5. **Cannot verify pruning state** -- need to check if events from 2026-05-25 are still present in the ledger or have been pruned. The pruning policy is undocumented ("caller-determined").

6. **Cannot verify recovery.rs emit calls** -- need to read `crates/network/src/sync/manager/recovery.rs` and `bins/node/src/node/fork_recovery.rs` source code. This is the H1 kill test for the recovery path and the single most important code verification.

## Summary Diagnosis (Backward Reasoning)

Working backward from "no alert fired":

**Layer 4 (Surface) -- INDEPENDENT FAILURE**: No automated consumer exists to read diagnostic RPC output. `fork-monitor.sh` polls `getChainInfo` (not `getForkDiagnostic`). No dashboard/metrics integration documented. Even if classification produced `restart_with_resync`, nobody would read it. This is sufficient by itself to explain the symptom. conf(0.65, inferred).

**Layer 1+3 (Emit + Classify) -- COUPLED FAILURE**: The recovery coordinator likely does not emit `RecoveryClassifyCall` events. Without these, the classifier sees a single ForkBlockReceived and classifies as TipRaceNatural (normal_operation) instead of ChainBreakLoop (restart_with_resync). This is a classification downgrade caused by an upstream emit gap, not a classifier bug. conf(0.50, inferred).

**The compound failure**: Both L4 and L1+L3 must be fixed. Fixing only L4 (adding a consumer) would not help if the classifier returns `normal_operation`. Fixing only L1 (adding recovery emit) would not help if no consumer reads the output. Both gaps contributed independently.

The most actionable single probe (if access were available): `ssh ai1 'curl -s -m 10 -X POST http://127.0.0.1:8503 -H "Content-Type: application/json" -d "{\"jsonrpc\":\"2.0\",\"method\":\"getForkDiagnostic\",\"params\":{\"min_height\":284670,\"max_height\":284685,\"limit\":1000},\"id\":1}"'` -- this single call would return (a) event count by kind confirming/refuting H1, (b) health.events_dropped_total confirming/refuting H2, (c) classification.fork_type + recommended_action confirming/refuting H3/H5, and (d) health.ledger_available confirming/refuting H7.
