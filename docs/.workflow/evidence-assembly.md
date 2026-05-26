# Evidence Assembly: INC-I-090

## Incident Timeline
**INC-I-090**: "N3 stuck 9min after 1-block fork: FINALITY_GUARD fencepost blocks ShallowRollback"  
**Status**: investigating  
**Root cause**: FINALITY_GUARD fencepost at crates/network/src/sync/manager/recovery.rs:312 (OUT OF SCOPE - this investigation is about OBSERVABILITY gaps)

| ID  | Type           | Content (200 char)                                                                                                                                       | Result                          | Agent                                       | Date                |
|-----|----------------|----------------------------------------------------------------------------------------------------------------------------------------------------------|--------------------------------|---------------------------------------------|---------------------|
| 818 | discovery      | FINALITY_GUARD fence-post error in recovery.rs:312: target_height <= finality should be target_height < finality. When local_tip=F+1 and finality=F... | Root cause identified: crates/... | blockchain-domain-investigator-connectivity | 2026-05-26 12:24:40 |
| 819 | trigger        | N3 produced block at slot 291216 (h=284677) at 22:54:37. 55ms later, received canonical block for slot 291215 (same height, better slot). FORK_GUARD... | Trigger: race between production... | blockchain-domain-investigator-connectivity | 2026-05-26 12:24:44 |
| 820 | discovery      | User hypothesized -50 peer score. Actual thresholds: app disconnect=-200, app ban=-500, gossipsub gossip=-10, publish=-50, graylist=-80. Measured wor... | REFUTED: peer scoring hypothesis... | blockchain-domain-investigator-parameters   | 2026-05-26 12:25:47 |
| 821 | discovery      | Recovery coordinator recently_synced() threshold (60s) suppressed shallow rollback after N3 produced fork block. N3 applied its OWN fork block so las... | CONFIRMED: recovery.rs:181-183... | blockchain-domain-investigator-parameters   | 2026-05-26 12:25:51 |
| 822 | trigger        | TRIGGER: N3 missed canonical block at h=284677/slot=291215 (19s gap), then produced competing block at slot=291216. FORK_GUARD detected better block... | Timeline: 22:54:11 last good block... | blockchain-domain-investigator-parameters   | 2026-05-26 12:25:53 |
| 823 | discovery      | N3 forked at h=284677: produced block 8ede1526 at slot 291216 while canonical block 150b4a7b existed at slot 291215. Root cause: 16-second gossip del... | FORK CONFIRMED - not a sync lag | blockchain-domain-investigator-fork         | 2026-05-26 12:27:57 |
| 824 | discovery      | FINALITY_GUARD off-by-one bug: recovery.rs:312 uses target_height <= finality (should be <). When N3 needed to roll back from h=284677 to h=284676, a... | BUG FOUND in crates/network/src... | blockchain-domain-investigator-fork         | 2026-05-26 12:28:04 |
| 825 | discovery      | Root cause: fencepost error in finality guard at recovery.rs:311. Uses target_height <= finality but should be target_height < finality. Rolling bac... | CONFIRMED via logs: FINALITY_GUARD... | blockchain-domain-investigator-code         | 2026-05-26 12:30:40 |
| 826 | discovery      | Dead code: signal_stuck_fork() at production_gate.rs:579 sets stuck_fork_signal flag, but take_stuck_fork_signal() at production_gate.rs:573 is never... | CONFIRMED via grep: 0 call sites... | blockchain-domain-investigator-code         | 2026-05-26 12:30:46 |
| 827 | trigger        | Trigger: 26-second gossip delay for slot 291215 block. N3 was eligible for slot 291216, produced at h=284677 at 22:54:37.082. Canonical block for sl... | The trigger is gossip latency... | blockchain-domain-investigator-code         | 2026-05-26 12:30:52 |
| 828 | domain-synthesis | Primary domain: CODE (recovery.rs:312 fencepost <= should be <). Presenting domain: FORK. Cross-domain: YES (Connectivity trigger -> Fork consequenc... | VERDICT conf(0.95, converged)  | blockchain-domain-synthesizer               | 2026-05-26 12:36:10 |
| 829 | resume         | Resumed for observability gap RCA — separate from underlying fencepost root cause. Investigating WHY observability missed the fork.                    | started                        | orchestrator                                | 2026-05-26 12:50:17 |
| 830 | triage         | Triage: DEEP. 7 hypotheses across 4 architectural layers + version skew. Multi-layer compound failure plausible. None of FAST disqualifiers apply.    | triaged                        | analyst                                     | 2026-05-26 12:54:13 |

**Observability vs Root Cause Split**: Entries 818-828 are about the underlying FINALITY_GUARD fencepost bug (OUT OF SCOPE). Entries 829-830 are about the observability gap investigation (IN SCOPE).

## Prior Artifacts
- `docs/bugfixes/observability-gap-inc-i-090-analysis.md` (2026-05-26 12:54:13) - analyst report on observability gap

## Prior System Model
No prior model — investigators will build from scratch

## Constraint Table
No failed approaches logged in observability/diagnostic/fork-monitor domains. Clean slate investigation.

## Scope-Specific Context

### Hotspots
- `bins/node/src/node/block_handling.rs` - CRITICAL risk level, 2 touches, "Fork sync weight comparison at line 568 causes infinite loop rejection"

### Open Bugs
#### P0 Findings
- **STRESS-P0-001**: --no-snap-sync permanent deadlock — no recovery path for forked nodes with snap sync disabled (`crates/network/src/sync/manager/production_gate.rs`)
- **DIAG-SYNC-001**: Fork sync weight check uses <= instead of <, rejecting equal-weight canonical chains during remedial fork recovery (`bins/node/src/node/block_handling.rs`)
- **DIAG-SYNC-003**: --no-snap-sync unconditionally blocks needs_genesis_resync() preventing recovery escalation (`crates/network/src/sync/manager/production_gate.rs`)

#### P1 Findings
- **DIAG-SYNC-002**: Empty headers handler always blacklists the responding peer causing isolation from canonical chain (`crates/network/src/sync/manager/sync_engine.rs`)
- **DIAG-SYNC-004**: best_peer_for_recovery() selects max height peer without quality threshold (`crates/network/src/sync/manager/mod.rs`)
- **INC-I-064-F002**: Reorg blocks bypass pool UTXO input verification in ValidationMode::Light (`bins/node/src/node/block_handling.rs`)

### Findings
Multiple active P0/P1 findings in fork/sync systems indicating architectural stress in observability gaps.

### Relevant Invariants
- **INV-OBS-001** (rpc/observability): "The DiagnosticHealth block returned by the getForkDiagnostic RPC must reflect the live writer counters (events_written_total, events_dropped_total, last_heartbeat_ms). It must never be hardcoded literals on the live production path."
- **INV-SYNC-006** (sync/recovery): Chain continuity verification requirement for any code path advancing local tip
- **INV-SYNC-007** (sync/recovery): Every node must converge to canonical chain's bit-identical 3-state

### Patterns
From behavioral learnings (confidence ≥0.5):
- **Observability tool bypassing**: "When a regression or fork is suspected, IMMEDIATELY use the fork-observability tooling we shipped (doli forks --explain, --fleet --by-producer, --replay) BEFORE proposing any operator workaround"
- **Read-only investigation defaults**: Multiple rules about using proper investigation tools vs manual log reading
- **Fork classification workflow**: Post-hoc discovery patterns vs real-time detection

## Shared Incidents (Cortex)
No shared incidents found (directory exists but contains only .gitkeep)

## Prior Workflow Runs (20 runs in fork/observability domains)
Notable fork-related workflow history:
- **353-354**: INC-I-086 getDiagnosticHealth RPC hardcoded zeros fix (completed 2026-05-21)
- **351**: INC-I-084 instant-fork regression (completed 2026-05-20)
- **244**: Fork recovery + sync + catch-up redesign (completed 2026-04-24)
- **212-225**: Multiple INC-I-040 persistent fork problem investigations (2026-04-21)
- **157-160**: INC-I-029 post-snap fork deadlock (2026-04-12)
- **192**: Systemic sync/fork divergence since scheduler architecture change (2026-04-15)

**Pattern**: 20+ doctor/diagnose/bugfix runs in fork-related domains since March 2026, indicating high incident frequency in this space.

## Evidence Summary
- **Escalation signals**: 0 failed attempts + 0 failed approaches + 20 prior runs = 20 total
- **Prior attempts**: 0 failed approaches logged  
- **Constraint table**: Empty — no prior attempts in observability domain specifically
- **System model**: not available
- **Shared matches**: 0 strong — no external incidents
- **Key constraints**: None from prior attempts, but INV-OBS-001 establishes that diagnostic RPC hardcoding is a known anti-pattern
- **Prior artifacts**: 1 file from prior sessions — `docs/bugfixes/observability-gap-inc-i-090-analysis.md`
- **Hotspots**: `bins/node/src/node/block_handling.rs` marked CRITICAL for fork sync issues
- **Related bugs**: 6 open P0/P1 findings in fork/sync subsystems, indicating architectural stress that may compound observability gaps