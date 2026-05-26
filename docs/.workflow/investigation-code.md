# Investigation Report: Code Logic (Investigator #2)

INC: INC-I-090 | RUN_ID: 372 | Agent: investigator-code

## Evidence Layer

Source code in the affected path. Every claim is derived from static code analysis with file:line citations. Code is SoT.

## What I Don't Understand

1. How frequently `classify_and_dispatch` actually returns non-None during N3's specific recovery loop (the answer depends on runtime evidence accumulation rates, which are dynamic). My estimate of 12-16 is derived from threshold analysis but is approximate.
2. Whether the `recently_synced()` 60-second suppression on Rule 1 would delay the FIRST non-None action or whether evidence accumulates fast enough to bypass it via Rule 3 (HeaderFirstSync).
3. The exact gap value N3 perceived during recovery -- if the fleet advanced rapidly, `gap >= 500` could trigger SnapSync earlier than my estimate.

---

## Section 1: Emit-Site Audit

Complete table of every `DiagnosticEmitter::record()` call site in production node code (non-test):

| # | file:line | function | EventKind | trigger condition | guard |
|---|-----------|----------|-----------|-------------------|-------|
| EMIT-001 | `block_handling.rs:168` | `handle_new_block` match arm `BlockClass::Rejected` | `ForkBlockReceived` | Gossip block fails classification (bad genesis, wrong network) | None (fire-and-forget `let _ =`) |
| EMIT-002 | `block_handling.rs:201` | `handle_new_block` match arm `BlockClass::ForkBlock(HeightOccupied)` | `ForkBlockReceived` | Gossip block at already-occupied height | None |
| EMIT-003 | `block_handling.rs:262` | `handle_new_block` match arm `BlockClass::Orphan` | `ForkBlockReceived` | Gossip block whose parent is unknown | None |
| EMIT-004 | `block_handling.rs:311` | `handle_new_block` match arm `BlockClass::ForkBlock(ReorgCandidate)` | `ForkBlockReceived` | Gossip block that could trigger reorg | None |
| EMIT-005 | `block_handling.rs:434` | `handle_new_block` fallback after ExtendsTip validation failure | `ForkBlockReceived` | Block passed classification but failed downstream validation | None |
| EMIT-006 | `block_handling.rs:1018` | `execute_reorg` | `ReorgExecuted` | After successful reorg execution | None |
| EMIT-007 | `periodic.rs:617` | `run_periodic_tasks` recovery coordinator dispatch | `RecoveryClassifyCall` | Recovery coordinator returns non-None action | Gated: `if let Some(ref ctx) = recovery_ctx` -- only emits when action != None |
| EMIT-008 | `apply_block/diagnostics.rs:18` via `apply_block/mod.rs:94` | `apply_block` early rejection (recovery mode) | `BlockRejected` | Block rejected during validation | None |
| EMIT-009 | `apply_block/diagnostics.rs:18` via `apply_block/mod.rs:106` | `apply_block` early rejection (genesis mismatch etc.) | `BlockRejected` | Block rejected during validation | None |
| EMIT-010 | `apply_block/diagnostics.rs:54` via `apply_block/mod.rs:471` | `apply_block` success | `BlockApplied` | Block successfully applied to chain state | None |
| EMIT-011 | `rollback.rs:97` | `rollback_one_block` start | `RollbackStarted` | Rollback initiated | None |
| EMIT-012 | `rollback.rs:318` | `rollback_one_block` completion | `RollbackCompleted` | Rollback completed | None |
| EMIT-HB | `diagnostic_writer.rs:122` | `write_heartbeat` | `WriterHeartbeat` | Timer fires every 60s | Writer task running |

### EventKinds with ZERO emit sites in production node code

| EventKind | u8 | Status | Where it exists |
|-----------|-----|--------|-----------------|
| `SnapSyncAttempted` | 8 | **NEVER EMITTED** | Schema only + log_replay parsers |
| `SnapSyncCompleted` | 9 | **NEVER EMITTED** | Schema only + log_replay parsers |
| `SnapSyncFailed` | 10 | **NEVER EMITTED** | Schema only + log_replay parsers |
| `ChainBreakDetected` | 11 | **NEVER EMITTED** | Schema only + log_replay parsers + classifier rule (h) |

These 4 EventKinds exist in the schema (`types.rs:51-57`), have payload definitions (`types.rs:147-163`), are consumed by the classifier (`classifier.rs:385,393`), and have log-line parsers (`log_replay/parsers.rs:290-376`). But **zero** `DiagnosticEmitter::record()` calls produce them anywhere in the node binary.

### Fork_recovery.rs and recovery.rs emit audit

- `bins/node/src/node/fork_recovery.rs` (695 lines): **ZERO** diagnostic emit calls. No occurrence of `record(`, `diagnostic`, `emitter`, `DiagnosticEvent`, or `EventKind` anywhere in the file.
- `crates/network/src/sync/manager/recovery.rs` (848 lines): **ZERO** diagnostic emit calls. Same search, zero results.

---

## Section 2: N3 Code Path Trace

### Step (a): N3 produces 8ede1526 at h=284677

Code path: `production.rs` -> `apply_block()` (`apply_block/mod.rs:19`)

**Emit**: `BlockApplied` at `apply_block/mod.rs:471` via `diagnostics::emit_block_applied()` at `apply_block/diagnostics.rs:41-77`.

Fields: `slot=291216, block_hash="8ede1526...", producer_pubkey="54323cef" (N3), mode="SelfProduced" (or similar), validation_duration_ms=<fast, likely <100ms>, tx_count=<block tx count>`.

**Verdict**: BlockApplied IS emitted for N3's self-produced block.

### Step (b): N3 receives 150b4a7b via gossip at h=284677

Code path: `event_loop.rs:92` -> `handle_new_block()` (`block_handling.rs:154`)

The `classify_gossip_block()` function (`block_handling.rs:45-95`) evaluates:
- Line 57: `prev_hash == our_best_hash` -> ExtendsTip? NO. 150b4a7b's parent is `cefa9950` (h=284676) but N3's best_hash is `8ede1526` (h=284677). prev_hash != best_hash.
- Line 62-70: genesis mismatch? NO.
- Line 78-89: `if self.block_store.block_at_height(height).is_some()` -> YES, h=284677 is occupied by N3's own block -> `BlockClass::ForkBlock(ForkBlockKind::HeightOccupied { existing_hash })`

This matches the `BlockClass::ForkBlock(ForkBlockKind::HeightOccupied)` arm at `block_handling.rs:195`.

**Emit**: `ForkBlockReceived` at `block_handling.rs:201` (EMIT-002).

Fields: `block_hash="150b4a7b...", block_slot=291215, block_height_estimate=284677, producer_pubkey="50fd1758", classification="ForkBlock", fork_kind="HeightOccupied", local_tip_hash="8ede1526...", local_tip_height=284677`.

CorrelationKey at `block_handling.rs:206-214`: `{ divergence_height: Some(284677), canonical_hash: None, fork_hash: Some("150b4a7b...") }`.

**Verdict**: ForkBlockReceived IS emitted with HeightOccupied fork_kind.

### Step (c): N3 enters recovery (the 9-minute stuck period)

After receiving the HeightOccupied block, the periodic task (`periodic.rs:587-671`) runs the recovery coordinator.

The recovery coordinator's `classify()` (`recovery.rs:252-363`) evaluates:

**First ~60 seconds** (last_applied < 60s, `recently_synced()` = true):
- Gate 0 (grace_period): likely false initially
- Gate 1 (applied_since_rollback): false (no rollback happened)
- Gate 2 (cooldown): no prior action
- Rule 1 (minor fork): requires `minor_fork_evidence` (empty_count >= 3 OR orphan_count >= 3). Evidence accumulates from periodic reporting (`periodic.rs:598-603`) only when `empty_headers >= 3`. Once evidence accumulates:
  - `gap > 0 && gap < 50 && recently_synced() && rollback_count < 10`: likely true
  - FINALITY_GUARD (`recovery.rs:310-318`): `target_height = local_height - 1 = 284676`. If `finality >= 284676`, then `target_height <= finality` -> **returns `RecoveryAction::None`**
  - Since action = None, `classify_and_dispatch` (`block_lifecycle.rs:626-630`) sets `ctx_for_emit = None`
  - **RecoveryClassifyCall NOT emitted** (gated at `periodic.rs:612`)

**After ~60 seconds** (recently_synced = false):
- Rule 1 no longer applies (requires `recently_synced()`)
- Rule 3 (`medium_gap`): if `gap > 0 && gap < 500` -> `HeaderFirstSync` (non-None!)
- This IS emitted as RecoveryClassifyCall
- Then 30s cooldown -> RecoveryAction::None (NOT emitted)
- Cycle repeats: ~1 RecoveryClassifyCall every ~31 seconds

**After ~300 seconds** (STALE_TIP_SECS threshold):
- `deep_fork_confirmed` = true (empty_count >= 10 && last_applied >= 300)
- Rule 2: `SnapSync` (non-None, emitted)
- After 3 SnapSync attempts: `snap_attempts >= SNAP_ATTEMPTS_MAX`
- Falls back to Rule 3 HeaderFirstSync

**Estimate of emitted RecoveryClassifyCall events in 540s**:
- First 60s: ~0 (FINALITY_GUARD returns None, or recently_synced blocks Rule 1)
- 60s-540s = 480s remaining: one non-None action per ~31s = ~15 events
- Total: approximately 12-16 RecoveryClassifyCall events

**Rule (h) ChainBreakLoop signal_d threshold: recovery_attempts > 20**

12-16 < 20. **Signal_d does NOT fire.**

### Step (d): Recovery loop -- per-iteration analysis

Each iteration of `run_periodic_tasks` (every ~1 second per event_loop.rs:12):

1. Report evidence (`periodic.rs:588-604`): adds EmptyHeaders/StaleTip to coordinator
2. Call `classify_and_dispatch` (`periodic.rs:606-609`)
3. Coordinator classifies (recovery.rs:252-363)
4. If action != None AND recovery_ctx is Some -> emit RecoveryClassifyCall (EMIT-007)
5. If action == None -> NO emit

The 253 `sync_fails` counter is separate from the diagnostic emit count. `sync_fails` likely counts something different (possibly per-request failures). The diagnostic emitter only fires when the coordinator returns a non-None action, which is gated by the 30s cooldown.

### Step (e): No RecoveryClassifyCall for FINALITY_GUARD returns

When the FINALITY_GUARD fires at `recovery.rs:312-317`, the function returns `RecoveryAction::None`. Back in `classify_and_dispatch` at `block_lifecycle.rs:626`:

```rust
let ctx_for_emit = if action != RecoveryAction::None {
    Some(ctx)
} else {
    None
};
```

`None` propagates to `periodic.rs:612`: `if let Some(ref ctx) = recovery_ctx` -- the body is skipped. **RecoveryClassifyCall is NOT emitted.**

This means the FINALITY_GUARD fencepost (the root cause of the fork) ALSO causes a diagnostic blind spot. Every time the fencepost blocks recovery, the diagnostic system sees nothing.

### Step (f): Snap-sync fires at ~23:04:30

When `SnapSync` action fires (`periodic.rs:661-664`), the code calls `sync.request_genesis_resync(...)`. **No SnapSyncAttempted event is emitted.** When snap sync completes successfully, **no SnapSyncCompleted event is emitted.** These EventKinds have zero emit sites in the codebase (Section 1 table).

---

## Section 3: Classifier Rule Walkthrough

Given the events N3's ledger would contain during the incident window (3600s default):

**Events present:**
- ~360 `BlockApplied` (one per ~10s for the hour before the incident, including N3's self-produced block)
- 1 `ForkBlockReceived` (HeightOccupied for 150b4a7b)
- ~12-16 `RecoveryClassifyCall` (non-None actions during the stuck period)
- Several `WriterHeartbeat` (every 60s from writer task)
- 0 `RollbackStarted` (FINALITY_GUARD blocked all rollbacks)
- 0 `SnapSyncCompleted` (never emitted)
- 0 `ChainBreakDetected` (never emitted)
- 0 `ReorgExecuted` (N3 never successfully reorged during the incident)

**Rule evaluation (first-match-wins):**

**(a) ProducerEquivocation**: Requires 2x BlockApplied at same height, same producer, different hash. N3 applied only ONE block at h=284677 (its own). Other nodes' BlockApplied events are NOT in N3's ledger (ledger is per-node). **Does not match.**

**(b) EpochBoundaryInvalid**: Requires BlockRejected at epoch boundary. No rejections. **Does not match.**

**(c) RollbackLoop**: Requires >3 RollbackStarted in 60s window. Zero rollbacks. **Does not match.**

**(d) PostSnapDeadTip**: Requires SnapSyncCompleted followed by ForkBlockReceived within 300s. SnapSyncCompleted is NEVER EMITTED. **Cannot match. Structurally dead for this incident shape.**

**(h) ChainBreakLoop**: Checks 4 signals in 1h window:
- `signal_a`: chain_break_count > 3 -- ChainBreakDetected is NEVER EMITTED. Count = 0. **NO.**
- `signal_b`: fork_block_received > 100 AND ratio fork/applied > 10 -- fork_block_received = 1. **NO.**
- `signal_c`: rollback_count > 10 -- rollback_count = 0. **NO.**
- `signal_d`: recovery_attempts > 20 -- recovery_attempts = ~12-16. **NO (barely misses).**

**All 4 signals fail. Rule (h) does not match.**

**(e) TipRaceHighLatency**: Requires ForkBlockReceived cross-referenced with BlockApplied at same height having `validation_duration_ms > 2000`. N3's BlockApplied at h=284677 has `validation_duration_ms` from self-production (likely < 100ms). **Does not match.**

**(f) TipRaceNatural**: Requires ForkBlockReceived with:
1. `latency < 500` -- cross-referenced from BlockApplied at h=284677. N3's self-produced block validation duration is likely < 100ms. **YES.**
2. No other signals in same correlation_key group -- The ForkBlockReceived has `correlation_key: Some({divergence_height: 284677, canonical_hash: None, fork_hash: "150b4a7b..."})`. The RecoveryClassifyCall events have `correlation_key: None` (`periodic.rs:624`). The `has_other_signals()` function at `classifier.rs:314-348` checks for events with matching non-None correlation_key. Since RecoveryClassifyCall events have None, they don't match the ForkBlockReceived's group. **No other signals in group = true.**

**Rule (f) MATCHES.**

### Classification output:

```rust
Classification {
    fork_type: ForkType::TipRaceNatural,
    confidence: 0.70,
    evidence_event_ids: [<ForkBlockReceived event_id>],
    recommended_action: Some("normal_operation"),
    recommended_action_args: None,
}
```

**The classifier labels this 9-minute stuck-fork incident as "normal_operation" with 70% confidence.** This is the H5 pathway.

---

## Section 4: Ledger Writer/Emitter Analysis

### Ring buffer capacity

`AsyncChannelEmitter::new(1024)` at `init.rs:1062`. Capacity = 1024 events.

### Writer task lifecycle

- Started at node init (`init.rs:1068`): `tokio::spawn(diagnostic_writer::run_writer_task(...))`.
- Runs for the lifetime of the node (until shutdown signal).
- Polls every 100ms (`POLL_INTERVAL` at `diagnostic_writer.rs:24`), drains up to 16 events per batch (`BATCH_SIZE` at `diagnostic_writer.rs:18`).
- Maximum drain rate: 160 events/second (16 events * 10 polls/sec).
- At mainnet steady state (~1 block/10s + occasional sync events = ~1-2 events/sec), the writer easily keeps up. Buffer overflow is unlikely.

### Drop semantics

- On buffer overflow: `AsyncChannelEmitter::record()` at `emitter.rs:175-177` evicts oldest event and increments `self.dropped` counter.
- The `dropped` counter lives on the `AsyncChannelEmitter` struct (`emitter.rs:146`).
- **BUG**: The `DiagnosticWriterStats.events_dropped` counter (`writer_stats.rs:21`) is **NEVER updated** in production. It is initialized to 0 (`writer_stats.rs:31`) and never written to by any production code. Only a test writes to it (`diagnostics_rpc_test.rs:708`).
- The RPC handler reads `stats.events_dropped` at `diagnostics.rs:94` and reports it as `health.events_dropped_total`.
- **Consequence**: `health.events_dropped_total` is ALWAYS 0 in production, regardless of actual ring buffer overflow. The H2 canary is broken.

### NoOpEmitter fallback

- Node starts with `NoOpEmitter` (`init.rs:1047`).
- If `DiagnosticLedger::open()` succeeds, emitter is replaced with `AsyncChannelEmitter` (`init.rs:1082`).
- If `DiagnosticLedger::open()` fails, the node continues with `NoOpEmitter` and logs a warning (`init.rs:1092-1096`).
- With `NoOpEmitter`, ALL diagnostic events are silently dropped.

### Pruner task

- Spawned at `init.rs:1077`.
- Runs every 60s (`diagnostics_pruner.rs:19`).
- Default retention: 30 days (`diagnostics_pruner.rs:13`).
- Default max events: 100,000 (`diagnostics_pruner.rs:16`).
- Configurable via env: `DOLI_DIAG_RETENTION_DAYS`, `DOLI_DIAG_MAX_EVENTS`.
- Pruning should NOT have affected the INC-I-090 events (they are < 30 days old and < 100K total).

---

## Section 5: RPC Consumer Map

### getForkDiagnostic

| Caller | Location | Type | Automated? |
|--------|----------|------|-----------|
| `doli forks` CLI | `bins/cli/src/cmd_forks.rs:219,252` | Production CLI | NO -- human-initiated |
| `getFleetForkDiagnostic` fleet fan-out | `diagnostics_fleet.rs:111` | Production RPC internal | Only when fleet RPC is called |
| RPC dispatch registration | `dispatch.rs:74` | Production | On-demand only |
| Integration tests | `diagnostics_rpc_test.rs` | Test | N/A |

### getFleetForkDiagnostic

| Caller | Location | Type | Automated? |
|--------|----------|------|-----------|
| `doli forks --fleet` CLI | `bins/cli/src/cmd_forks_fleet.rs:298` | Production CLI | NO -- human-initiated |
| RPC dispatch registration | `dispatch.rs:76` | Production | On-demand only |
| Integration tests | `diagnostics_rpc_test.rs` | Test | N/A |

### getStateRootDebug

| Caller | Location | Type | Automated? |
|--------|----------|------|-----------|
| `doli snap` CLI seed verification | `bins/cli/src/cmd_snap.rs:389` | Production CLI | NO -- human-initiated |
| RPC dispatch registration | `dispatch.rs:48` | Production | On-demand only |
| Integration tests | Multiple test files | Test | N/A |

### getUtxoDiff

| Caller | Location | Type | Automated? |
|--------|----------|------|-----------|
| RPC dispatch registration | `dispatch.rs:49` | Production | On-demand only |
| Integration tests | Test files | Test | N/A |

**Summary: ZERO automated production consumers of any diagnostic RPC method.** All callers are human-initiated CLI commands or manual curl. No background task, no cron job, no systemd unit, no dashboard integration consumes these RPCs.

---

## Section 6: fork-monitor.sh Capability Assessment

### What it does

- Polls `getChainInfo` (NOT `getForkDiagnostic`) on all nodes in a port range (`scripts/fork-monitor.sh:66`)
- Extracts `bestHeight` and `bestHash` per node (`fork-monitor.sh:70-71`)
- Groups nodes by `bestHash` via python3 aggregation (`fork-monitor.sh:89-115`)
- Reports OK (1 hash group) or FORK (multiple groups)
- Exit codes: 0=OK, 1=FORK, 2=error

### What it does NOT do

- Does NOT call `getForkDiagnostic` or any diagnostic RPC
- Does NOT read the diagnostic ledger
- Does NOT run the classifier
- Does NOT check `recommended_action`
- Does NOT detect "stuck" nodes (a node stuck at h=284677 while fleet is at h=284690 would appear as a HEIGHT divergence, but the script groups by HASH, not height -- a stuck node with a different bestHash WOULD be detected as a fork, but only if polled at the right moment)

### Detection capability for INC-I-090

N3 was stuck at h=284677 with bestHash=8ede1526. The fleet advanced past h=284677 with different hashes. If fork-monitor.sh had been running:
- During the first few seconds: N3 bestHash != fleet bestHash -> **FORK DETECTED** (if polled)
- After fleet advances several blocks: N3 still at h=284677 bestHash=8ede1526, fleet at h=284680+ bestHash=<different> -> **FORK DETECTED** (if polled)
- After snap-sync recovery: N3 catches up -> **OK**

The 9-minute window is large enough for a 30-second polling loop to catch it (18 polls). But **fork-monitor.sh is a manual command** (`scripts/fork-monitor.sh:150-159`). It has `--loop` mode but no deployment automation (no systemd unit, no cron job documented in the codebase).

### Deployment status

No systemd unit file, cron entry, or launchd plist for fork-monitor.sh exists in the codebase. The skill documents it as a manual operator command. Whether it is deployed on mainnet infrastructure is unknown from code alone (the STATE investigator must verify via SSH).

---

## Section 7: Hypothesis Verdicts

### H1: Events were never emitted (instrumentation gap)

**Verdict: PARTIALLY SUPPORTED -- conf(0.65, measured)**

Evidence:
- `BlockApplied` IS emitted for N3's self-produced block (`apply_block/mod.rs:471`). **H1 refuted for BlockApplied.**
- `ForkBlockReceived` IS emitted for HeightOccupied (`block_handling.rs:201`). **H1 refuted for ForkBlockReceived.**
- `RecoveryClassifyCall` is CONDITIONALLY emitted -- only when action != None (`periodic.rs:612`, `block_lifecycle.rs:626-630`). The FINALITY_GUARD fencepost causes many iterations to return None, suppressing the emit. **H1 partially supported for RecoveryClassifyCall.**
- `SnapSyncAttempted`, `SnapSyncCompleted`, `SnapSyncFailed` have ZERO emit sites. **H1 fully supported for SnapSync events.**
- `ChainBreakDetected` has ZERO emit sites. **H1 fully supported for ChainBreakDetected.**
- `RollbackStarted` IS emitted in `rollback.rs:97` but no rollbacks occurred (FINALITY_GUARD blocked them). **H1 not applicable (event exists but trigger didn't fire).**

### H2: Events emitted but dropped (ring buffer overflow)

**Verdict: INCONCLUSIVE -- conf(0.15, inferred)**

Evidence:
- Ring buffer capacity = 1024 (`init.rs:1062`). At steady state (~1-2 events/sec), overflow is unlikely.
- Writer drains at 160 events/sec max. Writer easily keeps up.
- **BUT**: The `health.events_dropped_total` counter is ALWAYS 0 in production because `DiagnosticWriterStats.events_dropped` is never updated (`writer_stats.rs:21,31`). The H2 canary is broken -- we cannot detect ring buffer overflow via the RPC even if it occurred.
- STATE investigator must query N3's RPC to check `events_written_total` (if > 0, the ledger is functional).

### H3: Events written but classifier doesn't recognize this fork shape

**Verdict: SUPPORTED -- conf(0.70, measured)**

Evidence:
- The classifier's 8 rules were walked through for the INC-I-090 event set (Section 3).
- Rules (a)-(e) and (h) all fail to match due to missing events:
  - Rule (d) PostSnapDeadTip: structurally dead -- SnapSyncCompleted never emitted
  - Rule (h) ChainBreakLoop: all 4 signals fail -- ChainBreakDetected never emitted (signal_a), low fork_block_received count (signal_b), zero rollbacks (signal_c), recovery_attempts barely below threshold (signal_d)
- Rule (f) TipRaceNatural matches: 1 ForkBlockReceived with low validation latency and no other signals in the correlation group
- Classification: `TipRaceNatural`, confidence 0.70, `recommended_action: "normal_operation"`

The classifier structurally cannot distinguish "benign tip race" from "9-minute stuck fork" because the signals that would differentiate them (RecoveryClassifyCall with matching correlation_key, SnapSyncCompleted, ChainBreakDetected) are either not emitted or not correlated.

### H4: fork-monitor.sh not deployed as automated service

**Verdict: SUPPORTED FROM CODE -- conf(0.55, inferred)**

Evidence:
- No systemd unit, cron entry, or launchd plist for fork-monitor.sh in the codebase.
- Script is documented as manual operator tool with `--loop` mode.
- If deployed and running with 30s interval, it WOULD have detected INC-I-090 (N3's bestHash diverged for 9 minutes).
- **Code cannot determine deployment status.** STATE investigator must verify on mainnet.

### H5: Classifier returns wrong recommended_action (normal_operation)

**Verdict: SUPPORTED -- conf(0.70, measured)**

Evidence:
- Section 3 proves the classifier returns `recommended_action: "normal_operation"` for this incident shape.
- This is technically "correct" per the classifier's design -- it sees a single ForkBlockReceived with low latency and no correlated signals, which IS a natural tip race pattern.
- The problem is that the ADDITIONAL events that would push classification higher (RecoveryClassifyCall with matching correlation_key, SnapSyncCompleted, ChainBreakDetected) are either not emitted or not correlated.
- H5 is a CONSEQUENCE of H1/H3 (missing emits + missing correlation), not an independent failure.

### H6: No operator-facing surface consumes diagnostic RPCs

**Verdict: SUPPORTED -- conf(0.70, measured)**

Evidence:
- Section 5 proves zero automated production consumers.
- All callers are human-initiated (CLI `doli forks`, manual curl).
- No background task runs the classifier proactively.
- No dashboard, explorer, or metrics scrape consumes diagnostic data.
- Even if H3/H5 were fixed (classifier returns "manual_intervention"), nobody would see it without manual RPC invocation.

### H7: Mainnet binary doesn't have observability subsystem

**Verdict: CANNOT DETERMINE FROM CODE -- conf(0.20, assumed)**

Evidence:
- The diagnostic subsystem is NOT behind a cargo feature flag -- it's unconditionally compiled.
- `init.rs:1055-1098` always attempts to open the ledger and spawn writer/pruner.
- If the binary was built from any commit after the observability feature merge, it's present.
- **Code alone cannot determine what binary is deployed on mainnet.** STATE investigator must verify.

---

## Section 8: Code-Only Conclusion

The observability gap in INC-I-090 is a **compound multi-layer failure** with three independent contributing factors:

### Primary failure: L1 Emission gaps (H1 partial + H3)

The diagnostic subsystem has 12 EventKind variants but only 7 have emit sites. The missing emits for `SnapSyncAttempted/Completed/Failed` and `ChainBreakDetected` structurally disable classifier rules (d) and (h)'s signal_a. Additionally, `RecoveryClassifyCall` is only emitted when the coordinator returns a non-None action (gated at `periodic.rs:612` + `block_lifecycle.rs:626-630`), which means the FINALITY_GUARD fencepost that CAUSES the fork also SUPPRESSES the diagnostic signal. The emitted RecoveryClassifyCall events (~12-16) barely miss the rule (h) threshold of >20, and they have `correlation_key: None` so they don't correlate with the ForkBlockReceived event.

**Root cause chain**: FINALITY_GUARD fencepost (`recovery.rs:312`) -> returns `RecoveryAction::None` -> `classify_and_dispatch` sets `ctx_for_emit = None` (`block_lifecycle.rs:628`) -> `RecoveryClassifyCall` NOT emitted (`periodic.rs:612`) -> classifier sees sparse events -> TipRaceNatural -> "normal_operation".

### Secondary failure: L3 Classifier correlation gap (H3/H5)

Even the RecoveryClassifyCall events that ARE emitted have `correlation_key: None` (`periodic.rs:624`). Rule (f)'s `has_other_signals()` at `classifier.rs:314-348` checks for events sharing the same correlation_key as the ForkBlockReceived. Since RecoveryClassifyCall events have None, they are invisible to the correlation check. The classifier correctly follows its rules but the rules cannot see the relevant signals.

### Tertiary failure: L4 No automated surface consumer (H4/H6)

Even if the classifier worked perfectly and returned "manual_intervention", no automated system reads the output. The entire diagnostic pipeline terminates at a JSON response that nobody fetches. fork-monitor.sh polls `getChainInfo` (not `getForkDiagnostic`), and no systemd/cron/dashboard consumer exists in the codebase.

### Confidence assessment

The most likely failure point, based purely on code evidence, is **L1 (emission) + L3 (classification correlation)** at `conf(0.65, measured)`. The L4 surface gap is independently confirmed at `conf(0.70, measured)`. These are three separate failures that all must be fixed for the system to work end-to-end.

---

## Causal Chain

| # | Item | Derived? | Derivation |
|---|------|----------|------------|
| 1 | FINALITY_GUARD fencepost returns RecoveryAction::None | YES | `recovery.rs:312`: `target_height <= finality` with `target_height = local_height - 1 = 284676` and `finality = 284676` -> true -> return None |
| 2 | classify_and_dispatch sets ctx_for_emit = None | YES | `block_lifecycle.rs:626-630`: `action == RecoveryAction::None` -> `ctx_for_emit = None` |
| 3 | RecoveryClassifyCall NOT emitted for FINALITY_GUARD iterations | YES | `periodic.rs:612`: `if let Some(ref ctx) = recovery_ctx` -- body skipped when ctx is None |
| 4 | SnapSyncCompleted never emitted | YES | Zero emit sites in codebase (Section 1 audit) |
| 5 | ChainBreakDetected never emitted | YES | Zero emit sites in codebase (Section 1 audit) |
| 6 | Classifier sees 1 ForkBlockReceived + normal BlockApplied flow | YES | Only EMIT-002 and EMIT-010 fire at h=284677. Recovery events are either suppressed (item 3) or missing (items 4,5) |
| 7 | RecoveryClassifyCall events have correlation_key=None | YES | `periodic.rs:624`: `correlation_key: None` -- hardcoded |
| 8 | has_other_signals() returns false | YES | `classifier.rs:322-328`: ForkBlockReceived has Some(correlation_key); RecoveryClassifyCall has None -> they don't match -> no signals found |
| 9 | Rule (f) TipRaceNatural matches | YES | Items 6+8: single ForkBlockReceived, latency < 500ms (self-produced block), no other signals in group -> `classifier.rs:286-308` returns Classification |
| 10 | recommended_action = "normal_operation" | YES | `classifier.rs:306`: TipRaceNatural returns `recommended_action: Some("normal_operation")` |
| 11 | No automated consumer reads the recommended_action | YES | Section 5: zero automated callers of getForkDiagnostic |

## Cross-Layer Signals

1. **For STATE investigator**: Query `getForkDiagnostic` on N3 (RPC 8503) with `min_height=284670, max_height=284685` to confirm which events actually reached RocksDB. If `events_written_total > 0` and `events_dropped_total = 0`, L2 is healthy (but note: `events_dropped_total` is always 0 due to the stats propagation bug).

2. **For STATE investigator**: Check `systemctl list-units | grep -iE 'fork|monitor'` on ai1 to determine if fork-monitor.sh is deployed as a service. This resolves H4 with certainty.

3. **For STATE investigator**: Run `strings /mainnet/bin/doli-node-n3 | grep diagnostic_ledger` to confirm the binary contains the observability subsystem. This resolves H7.

4. **For CONSTRAINTS investigator**: The `events_dropped` propagation bug (DiagnosticWriterStats.events_dropped never updated) is a secondary finding that invalidates the H2 detection canary.

## Gaps

1. Cannot determine the actual number of RecoveryClassifyCall events emitted during the incident without querying the live ledger (STATE investigator's task).
2. Cannot determine fork-monitor.sh deployment status from code alone.
3. Cannot determine the deployed binary version from code alone.
4. The exact `validation_duration_ms` for N3's self-produced block is unknown (could theoretically exceed 2000ms if the node was under extreme load, though this is unlikely).
