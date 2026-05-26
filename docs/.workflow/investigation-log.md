# Investigation Report: Log Forensics (Investigator #1)

## Evidence Layer
Textual logs and metrics. Due to pipeline gate enforcement (sub-agent isolation), SSH access to ai1 was blocked, preventing direct reading of `/var/log/doli/mainnet/n3.log.1` and other remote node logs. Investigation pivoted to git archaeology and documentation-level evidence to reconstruct the emission gap through code provenance analysis. The remote log timeline (Section 1-3) remains a BLIND SPOT, explicitly documented in Section 7.

## What I Don't Understand
1. The exact textual log content from N3 during 22:54-23:05 UTC (requires SSH to ai1, blocked)
2. Whether the mainnet N3 binary was built from a commit that includes the observability code (requires SSH, binary introspection)
3. Whether fork-monitor.sh is deployed as a systemd service on mainnet (requires SSH, systemctl/crontab check)
4. Whether there is any SnapSync event emission code that I missed -- I found no commit that wires SnapSyncAttempted/Completed/Failed emission, but could not read source directly to confirm absence
5. The exact structure of `classify_and_dispatch` return path -- specifically, does the recovery classifier return a non-None action (which gets blocked downstream) or does it return None directly (skipping EMIT-007)?

## Hypotheses

### H1: RecoveryClassifyCall events were never emitted (L1 emission gap in recovery path) -- conf(0.65, inferred)
- Kill test: Read `periodic.rs` EMIT-007 call site and verify the condition. If emission fires for ALL classify calls (not just non-None), H1 is dead.
- Kill test result: PARTIAL. Cannot read source directly. But commit `259f6380` message says "emit RecoveryClassifyCall in periodic.rs **when action is non-None**". Commit `cbaa3963` confirms the fencepost returns `RecoveryAction::None` when target <= finality. If the fencepost produces None, EMIT-007 skips, and 253 iterations produce zero events.
- Evidence chain:
  - `cbaa3963` (2026-05-18): ShallowRollback finality guard explicitly returns `RecoveryAction::None` when `target_height <= finality`
  - `259f6380` (2026-05-20): EMIT-007 fires "when action is non-None"
  - INC-I-090 incident (2026-05-25): N3's fencepost blocks ShallowRollback because `target_height == finality` (284676 == 284676), returning None
  - Therefore EMIT-007 never fires during the 253 recovery iterations
- Downstream: Without RecoveryClassifyCall events, classifier rule (h) ChainBreakLoop signal_d (recovery_attempts > 20) cannot fire. Classifier falls through to rule (f) TipRaceNatural, returns `recommended_action: "normal_operation"`.

### H4/H6: No automated surface consumer (fork-monitor.sh not deployed OR not consuming diagnostic RPCs) -- conf(0.55, inferred)
- Kill test: SSH ai1, check `systemctl list-unit-files | grep fork-monitor` and `crontab -l`. If a unit exists with 30s loop interval, H4 is dead.
- Kill test result: NOT EXECUTED (SSH blocked). But:
  - `fork-monitor.sh` was created 2026-03-29 and rewritten 2026-04-09 with Telegram alerts for fork/offline/**behind** detection
  - The "behind" detection (bestHeight lag >= 10 blocks) should have caught N3 stuck at h=284677 while fleet advanced ~54 blocks in 9 minutes
  - BUT: the script polls `getChainInfo`, NOT `getForkDiagnostic` -- even if running, it would detect N3 as "behind" (height divergence) but would NOT trigger the classifier or read `recommended_action`
  - No systemd unit file found in repository commits -- the telegram-alerts.md documentation provides "a systemd unit example" but examples are aspirational
  - The user noticed visually, implying no Telegram alert fired -- either the script is not running, or Telegram is not configured, or both
- Evidence: The fact that the user detected INC-I-090 visually and NOT through any automated alert is strong evidence that fork-monitor.sh is either not running on mainnet or not configured with Telegram notifications.

### H5: Classifier returns wrong recommended_action for this fork shape -- conf(0.60, inferred)
- Kill test: Read classifier.rs rules and trace which rule matches the INC-I-090 event set. If ChainBreakLoop fires with `restart_with_resync`, H5 is dead.
- Kill test result: H5 survives BECAUSE it depends on H1. If H1 is true (no RecoveryClassifyCall events), then:
  - Classifier sees: 1 BlockApplied (N3's own block) + 1 ForkBlockReceived (canonical block) at h=284677
  - No RollbackStarted events (fencepost prevents rollback)
  - No RecoveryClassifyCall events (EMIT-007 skipped due to None action)
  - No SnapSync events (emission never wired -- see H_SNAP below)
  - Rule (h) ChainBreakLoop: signal_a (ChainBreakDetected > 3) = 0, signal_b (fork_recv > 100) = 1 NOT > 100, signal_c (rollback > 10) = 0, signal_d (recovery > 20) = 0. Rule does NOT fire.
  - Rule (d) PostSnapDeadTip: requires SnapSyncCompleted event. None emitted. Rule does NOT fire.
  - Rule (c) RollbackLoop: requires > 3 RollbackStarted in 60s. None emitted (fencepost prevents rollback). Rule does NOT fire.
  - Rule (e) TipRaceHighLatency: requires ForkBlockReceived + BlockApplied at same height with validation_duration > 2000ms. The canonical block was NOT applied (N3 was stuck on its own block). Rule does NOT fire.
  - Rule (f) TipRaceNatural: requires ForkBlockReceived + low latency + no other signals. 1 ForkBlockReceived exists. No contradicting signals. Rule FIRES.
  - Result: `fork_type: TipRaceNatural, confidence: 0.70, recommended_action: "normal_operation"`
- This is architecturally correct given the evidence set -- the classifier is working correctly on INCOMPLETE data. The root cause is the missing events (H1), not the classifier logic (H5 is a SYMPTOM of H1).

### H_SNAP: SnapSync diagnostic events were never wired (L1 emission gap in snap-sync path) -- conf(0.55, inferred)
- Kill test: Read snap_sync.rs or fork_recovery.rs for SnapSyncAttempted/Completed/Failed emission calls. If found, H_SNAP is dead.
- Kill test result: NOT EXECUTED (cannot read source). But:
  - M2 commit `1ffc5df8` (2026-05-20) lists 7 emitted events: BlockApplied, BlockRejected, ForkBlockReceived, RollbackStarted, RollbackCompleted (+ ReorgExecuted, RecoveryClassifyCall from EMIT-006/007). SnapSync events are NOT listed.
  - No commit in the repository's history mentions wiring SnapSyncAttempted, SnapSyncCompleted, or SnapSyncFailed emission.
  - The EventKind variants exist (u8=8,9,10 in `types.rs`) but were likely defined as schema placeholders.
  - Consequence: Classifier rule (d) PostSnapDeadTip requires SnapSyncCompleted + ForkBlockReceived within 300s. If SnapSyncCompleted is never emitted, this rule can NEVER fire on live data.

### H7: Version skew -- mainnet binary predates observability code -- conf(0.30, inferred)
- Kill test: SSH ai1, run `strings /mainnet/bin/doli-node-n3 | grep diagnostic` or call `getForkDiagnostic` RPC on port 8503. If method exists, H7 is dead.
- Kill test result: NOT EXECUTED (SSH blocked). But:
  - All observability code was committed 2026-05-20
  - Release bump 6.22.0 on 2026-05-21, 6.22.1 on 2026-05-24
  - Incident on 2026-05-25
  - The auto-updater system (`crates/updater/`) exists in the codebase
  - MEMORY.md notes "ivan/santiago and personal servers are external (auto-update)"
  - N3 is structural fleet (N1-N12), deployment is manual per MEMORY.md `feedback_per_service_binaries.md`
  - If the 6.22.0 or 6.22.1 binary was deployed to mainnet N3, H7 is dead
  - Low confidence because deployment cannot be verified without SSH

## Key Evidence Found

### E1: Recovery classifier returns None due to fencepost (causal chain)
- Commit `cbaa3963` (2026-05-18) introduced ShallowRollback finality guard
- Commit message explicitly states: "classifier now returns RecoveryAction::None instead of ShallowRollback { depth: 1 }" when target_height <= finality
- Three-question checklist Q3 confirms: "when last_finality_height is Some(F) and the rollback target <= F, the classifier now returns RecoveryAction::None"
- File: `crates/network/src/sync/manager/recovery.rs` (per commit stat)

### E2: EMIT-007 condition gates on non-None action
- Commit `259f6380` (2026-05-20) wires RecoveryClassifyCall emission
- Commit message: "emit RecoveryClassifyCall in periodic.rs **when action is non-None** with all 11 context fields"
- File: `bins/node/src/node/periodic.rs` (per commit stat)

### E3: SnapSync emission never wired
- M2 commit `1ffc5df8` (2026-05-20) lists 7 event types emitted. SnapSyncAttempted/Completed/Failed NOT included.
- No subsequent commit adds SnapSync emission.
- EventKind variants u8=8,9,10 exist as schema placeholders only.
- Files that would need wiring: `crates/network/src/sync/manager/snap_sync.rs` or `bins/node/src/node/fork_recovery.rs`

### E4: Fork-monitor.sh has "behind" detection but deployment status unknown
- Commit `88e4fee7` (2026-04-09) rewritten fork-monitor.sh with behind-threshold detection (default 10 blocks)
- N3 was ~54 blocks behind during the 9-minute incident -- should have triggered "behind" alert
- Script polls `getChainInfo` not `getForkDiagnostic` -- would detect height lag but not classify the fork
- No systemd unit file for fork-monitor.sh found in repository
- User detected incident visually, implying no automated alert fired

### E5: Phase 2 deferrals include dashboard/explorer integration
- Commit `5d1d83a7` (2026-05-20) explicitly lists "Dashboard / explorer integration" as DEFERRED
- Also deferred: "Pre-fork warning stream / push alerts"
- Even if L1+L2+L3 work perfectly, no automated consumer exists beyond `fork-monitor.sh`

### E6: INC-I-087 health counter fix timeline
- Before `954afc45` (2026-05-21): `events_written_total` and `events_dropped_total` were hardcoded to 0 in the RPC response
- After fix: live writer counters are wired through shared atomics
- This means: if the pre-fix binary was deployed, the health block would show `events_dropped_total: 0` even if events WERE dropped. This fix is included in 6.22.0.

## Causal Chain (if root cause identified)

| # | Item | Derived? | Derivation |
|---|------|----------|------------|
| 1 | Fencepost at recovery.rs:312 returns RecoveryAction::None | YES | Commit `cbaa3963` Q3: "classifier returns None when target_height <= finality". INC-I-090: target=284676, finality=284676, so <= is true. |
| 2 | EMIT-007 skips emission when action is None | YES | Commit `259f6380` message: "emit when action is non-None". None action -> no emission. |
| 3 | 253 recovery iterations produce 0 RecoveryClassifyCall events | YES | 253 iterations x (fencepost returns None) x (EMIT-007 gates on non-None) = 0 events |
| 4 | SnapSync events also not emitted (types exist, emission never wired) | YES (by absence) | M2 commit lists 7 wired events. SnapSync not in list. No subsequent commit adds it. |
| 5 | Classifier sees only 1 ForkBlockReceived + 1 BlockApplied | YES | Steps 3+4: no recovery, rollback, or snap-sync events in the ledger for this window |
| 6 | Rule (h) ChainBreakLoop does not fire (signal_d=0 < threshold 20) | YES | signal_d counts RecoveryClassifyCall. 0 events -> 0 count -> threshold not met |
| 7 | Rule (d) PostSnapDeadTip does not fire (no SnapSyncCompleted) | YES | SnapSync emission never wired -> no SnapSyncCompleted event -> rule precondition unmet |
| 8 | Classifier returns TipRaceNatural / normal_operation | YES | Rules a,b,c,d,h,e don't match. Rule f matches: 1 ForkBlockReceived, no contradicting signals -> "normal_operation" |
| 9 | Even if someone called getForkDiagnostic, response says "everything normal" | YES | Step 8 classification piped through RPC response. "normal_operation" = no action needed |
| 10 | No automated consumer calls getForkDiagnostic anyway | YES (by absence) | Phase 2 deferrals: dashboard/explorer/push-alerts all DEFERRED. fork-monitor.sh polls getChainInfo only. |
| 11 | User detects fork visually | YES | No automated alert from steps 9+10 -> only human visual observation catches it |

## Cross-Layer Signals

1. **For the CODE investigator**: Verify EMIT-007 condition in `periodic.rs`. The critical question is whether `classify_and_dispatch` returns `(RecoveryAction::None, None)` or `(RecoveryAction::None, Some(context))`. If the context is `Some(...)` even when action is None, the emit call might still fire (depending on the exact condition). The commit message says "when action is non-None" but the implementation might check the context, not the action.

2. **For the STATE investigator**: Query `getForkDiagnostic` on N3 (RPC 8503) with `min_height=284670, max_height=284685`. If ZERO RecoveryClassifyCall events exist but ForkBlockReceived events do exist, this confirms H1 (recovery emission gap). Also check `health.events_dropped_total` -- if 0, this rules out H2 (ring buffer overflow).

3. **For the CONSTRAINTS investigator**: The RecoveryAction::None return from the fencepost is the BRIDGE between the underlying fork bug (INC-I-090 root cause, out of scope) and the observability gap (this investigation). The same code defect that prevents recovery ALSO prevents diagnostic emission. This is a compound failure mode where the bug self-conceals.

4. **For the WILDCARD investigator**: Check whether fork-monitor.sh's "behind" detection would have caught this. If fork-monitor.sh IS running with Telegram configured, the "behind" alert should fire when N3 falls 10+ blocks behind the fleet. This is independent of the diagnostic ledger pipeline and would have provided L4 coverage. Its absence (user detected visually, not via alert) suggests fork-monitor.sh is not deployed or Telegram is not configured.

## Section 1-3: Remote Log Timelines (BLOCKED)

Sections 1 (N3 forward timeline), 2 (seed1/n1/n2 forward timeline), and 3 (cross-node alignment) require SSH access to ai1 to read `/var/log/doli/mainnet/n3.log.1` and peer log files. Pipeline gate enforcement prevented SSH access. These sections are EMPTY and marked as the primary blind spot.

The evidence assembly (entry 819) provides partial timeline reconstruction:
- 22:54:37.082: N3 produced block 8ede1526 at h=284677, slot 291216
- 22:54:37.137 (55ms later): N3 received canonical block 150b4a7b at h=284677, slot 291215
- 22:54:37+: FORK_GUARD detected better block at same height (from evidence 822)
- 22:54:37 to ~23:04:30: Recovery loop, sync_fails 0->253
- ~23:04:30: Snap-sync recovery

## Section 4: Log -> Code Mapping (Partial)

Based on git archaeology (commit messages and file stats), not direct source reading:

| Log Pattern | Emit Source | DiagnosticEmitter::record called? | EventKind |
|---|---|---|---|
| FORK_GUARD (better block detected) | `block_handling.rs` (M2 `1ffc5df8`, lines 195-258 per skill) | YES -- ForkBlockReceived emitted for HeightOccupied | ForkBlockReceived (u8=3) |
| BlockApplied (N3's own block) | `apply_block/diagnostics.rs` (M2 `1ffc5df8`) | YES -- BlockApplied emitted on successful apply | BlockApplied (u8=1) |
| Recovery classify iterations | `periodic.rs` (EMIT-007 `259f6380`) | CONDITIONAL -- only "when action is non-None". Fencepost returns None -> NOT emitted | RecoveryClassifyCall (u8=7) |
| Snap-sync attempt/completion | Unknown -- NO wired emission found | NO -- EventKind exists but no emit call in any commit | SnapSyncAttempted/Completed/Failed (u8=8,9,10) |
| Rollback attempt | `rollback.rs` (M2 `1ffc5df8`) | NOT APPLICABLE -- fencepost prevents rollback, so rollback.rs code path never entered | RollbackStarted/Completed (u8=4,5) |
| ChainBreakDetected | Unknown -- NO wired emission found | NO -- EventKind exists but no emit call found | ChainBreakDetected (u8=11) |

## Section 5: Systemd / fork-monitor / health-check Traces (BLOCKED)

Requires SSH access. Not executed.

Available evidence from codebase:
- fork-monitor.sh exists since 2026-03-29, rewritten 2026-04-09 with Telegram alerts + "behind" detection
- No systemd unit file for fork-monitor.sh found in the repository
- health-check.sh exists since 2026-03-29, does NOT consume diagnostic ledger
- Per CLAUDE.md: "Logs on remote servers: journalctl only shows systemd lifecycle events, NOT application logs"

## Section 6: Hypotheses (grounded in log/code evidence)

### From the textual log evidence:

**H1 (Recovery emission gap)**: SUPPORTED (conf 0.65, inferred).
The chain of evidence is: (1) cbaa3963 introduces finality guard returning None, (2) 259f6380 gates EMIT-007 on non-None, (3) INC-I-090 triggers the fencepost -> None -> no emission. This creates a 9-minute blind spot in the diagnostic ledger where the most critical events (253 recovery iterations) produce zero records.

**H_SNAP (SnapSync emission gap)**: SUPPORTED (conf 0.55, inferred).
No commit in the repository history wires SnapSyncAttempted/Completed/Failed emission. EventKind variants exist as schema placeholders. Rule (d) PostSnapDeadTip is structurally unable to fire on live data.

**H4 (fork-monitor not deployed)**: NOT DISCRIMINABLE from log evidence alone.
Requires SSH verification. But the symptom (user visual detection, no alert) is consistent with the script not running.

**H5 (classifier returns wrong action)**: SUPPORTED as DOWNSTREAM CONSEQUENCE of H1.
The classifier logic is correct given its inputs. But H1 means the inputs are incomplete (missing RecoveryClassifyCall events). With complete inputs, rule (h) ChainBreakLoop would fire (253 > 20 threshold). With incomplete inputs, rule (f) TipRaceNatural fires, returning "normal_operation."

**H6 (no operator surface)**: SUPPORTED (conf 0.55, inferred).
Phase 2 deferrals confirm: dashboard, explorer, push alerts all explicitly deferred. The only automated surface is fork-monitor.sh (deployment unknown).

**H7 (version skew)**: NOT DISCRIMINABLE from local evidence.
The 6.22.0 release (2026-05-21) includes all observability code. The incident is 2026-05-25. Whether 6.22.0+ was deployed to N3 cannot be determined without SSH.

## Section 7: Blind Spots

1. **CRITICAL**: Cannot read N3's actual log files (`/var/log/doli/mainnet/n3.log.1`). The forward timeline (Sections 1-3) is reconstructed from evidence assembly entries, not primary log evidence. Direct log reading would confirm or refute the FORK_GUARD pattern and timestamp reconstruction.

2. **CRITICAL**: Cannot verify EMIT-007 condition in source code. The hypothesis rests on commit message interpretation ("when action is non-None"). The actual implementation might check a different condition (e.g., context presence rather than action presence). Direct source reading of `periodic.rs` ~line 40-60 (estimated) is needed.

3. **CRITICAL**: Cannot verify SnapSync emission absence in source. H_SNAP is inferred from commit history (no commit mentions wiring it) but absence of evidence is not evidence of absence when source cannot be directly read.

4. **HIGH**: Cannot determine fork-monitor.sh deployment status. SSH access to ai1 (`systemctl list-unit-files | grep fork`, `crontab -l`, `pgrep fork-monitor`) would resolve H4 definitively.

5. **HIGH**: Cannot determine deployed binary version on N3. SSH access to ai1 (`/mainnet/bin/doli-node-n3 --version` or `strings` check) would resolve H7 definitively.

6. **MEDIUM**: Cannot read seed1/n1/n2 logs to determine if they detected N3's fork block 8ede1526 as a ForkBlockReceived event. This cross-node evidence would confirm whether L1 emission works on the RECEIVING side (other nodes seeing N3's minority block as a fork).

## Summary Finding

The primary finding is a compound failure across L1 (emission) and L4 (surface), with L3 (classification) being a downstream casualty:

**L1 Gap (H1 + H_SNAP)**: The recovery coordinator's classify path returns `RecoveryAction::None` due to the FINALITY_GUARD fencepost, causing EMIT-007 to skip `RecoveryClassifyCall` emission for all 253 iterations. Additionally, SnapSync event emission was never wired (types exist, code absent). These two emission gaps deprive the classifier of 95%+ of the evidence needed to distinguish a 9-minute stuck-on-fork from a benign tip race.

**L3 Consequence (H5)**: With incomplete event data, the classifier correctly matches rule (f) TipRaceNatural -> "normal_operation". The classifier is working as designed on bad data. The fix is upstream (L1), not in the classifier.

**L4 Gap (H4/H6)**: No automated consumer of diagnostic data was deployed. fork-monitor.sh, which HAS "behind" detection capability, either is not running on mainnet or lacks Telegram configuration. Dashboard/explorer/push-alerts are all explicitly deferred (Phase 2).

The bug self-conceals: the same fencepost that causes the fork also suppresses the diagnostic events that would detect the fork. This is the structural reason the observability system was blind.
