# QA Report: INC-I-139 Sync Snap-Admission M2+M3 (DC-1 + DC-2, CR-1)

- **Run**: 455 | **Incident**: INC-I-139 | **Milestone**: M2+M3
- **Spec**: `specs/sync-snap-admission-architecture.md` (DC-1, DC-2, CR-1)
- **Analysis**: `docs/redesigns/sync-snap-admission-redesign-analysis.md` (REQ-SNAP-001/002/003/010)
- **Date**: 2026-07-10
- **Constraint honored**: code is FINAL/FROZEN — validation only, no source edits.

## Scope Validated
- DC-2: `crates/network/src/sync/manager/production_gate.rs` `request_genesis_resync` forward-large-gap floor exemption.
- DC-1: `crates/network/src/sync/manager/sync_engine/decision.rs` `should_snap` bare-gap OR-term deletion.
- Test migration: `tests.rs` (T-RG-001, T-RG-009, T-M2-009) + `tests_inc_i139.rs` (class2/4/6/8).

## Summary
**PASS.** Both atomic changes are correctly scoped and behave exactly as DC-1/DC-2 specify. The full network lib suite is green (442 passed, 0 failed, 2 ignored). Gate 4 (`--no-snap-sync`) is confirmed unchanged (emergency-only bypass). Exploratory analysis confirms no bare-gap path can admit snap post-DC-1: `needs_genesis_resync` is set in exactly one gated location, and the only floor-exempt admission reasons require gap ≥ 1000 (stuck-sync) or gap ≥ 500 / explicit deep-fork evidence (coordinator). The test migration is honest — it asserts the new correct behavior and adds positive coverage rather than weakening.

## System Entrypoint
Unit-level consensus/sync validation. Command:
`source ~/.cargo/env && cargo test -p network --lib` → `442 passed; 0 failed; 2 ignored`.
Multi-node runtime (gauntlet) is out of scope for this frozen unit-diff validation; the change is confined to the sync decision function and the recovery admission gate, both fully exercised by deterministic unit tests.

## Traceability Matrix Status
| Requirement | Priority | Has Tests | Tests Pass | Acceptance Met | Notes |
|---|---|---|---|---|---|
| REQ-SNAP-001/002 (no bare-gap snap) | Must | Yes (class2) | Yes | Yes | bare gap=51 → not snapped |
| REQ-SNAP-003 (legit gap≥500 catch-up survives) | Must | Yes (class4, class6) | Yes | Yes | CoordinatorSnapEscalation honored; Route C intact |
| REQ-SNAP-010 (over-exemption regression) | Must | Yes (class8, T-M2-009) | Yes | Yes | HeightOffset/GenesisEscalation still floor-gated |
| CR-1 (floor guards BACKWARD only) | Must | Yes (T-M2-009) | Yes | Yes | forward snap exempt, backward wipe still refused |

### Gaps Found
- None. class3 (M5/DC-4) remains `#[ignore] FAILS-BY-DESIGN` as designed — out of M2+M3 scope. One pre-existing ignore (`test_adaptive_gossip_large_network_floor`) is unrelated.

## Acceptance Criteria Results

### Must Requirements
- [x] **REQ-SNAP-001/002** — PASS. `should_snap` post-DC-1 = `enough_peers && attempts<3 && snap_allowed && (local_height==0 || needs_genesis_resync)`. Bare-gap OR-term deleted (decision.rs:167). class2: local=100, 3 peers @151 (gap=51), no fork signal → `pipeline_data != SnapCollecting`. PASS.
- [x] **REQ-SNAP-003 (coordinator catch-up)** — PASS. class4: floor=100, gap=600, `CoordinatorSnapEscalation` → honored=true, `needs_genesis_resync`=true (Gate 1 exemption). PASS.
- [x] **REQ-SNAP-003 (Route C bootstrap)** — PASS. class6: h==0, gap=600 → snapped via Route C (local_height==0 term untouched by DC-1). PASS.
- [x] **REQ-SNAP-010 (no over-exemption)** — PASS. class8: floor=100, `HeightOffsetDetected{gap:10}` → refused, flag unset. T-M2-009 additionally proves `CoordinatorGenesisEscalation` remains floor-gated. PASS.

## Diff Scope Verification (Task 2)
Changed source files (only): `production_gate.rs`, `decision.rs` (+ test files `tests.rs`, `tests_inc_i139.rs`). No other network source modified (`git status` confirmed).
- **DC-2** adds `is_forward_large_gap = matches!(reason, CoordinatorSnapEscalation | StuckSyncLargeGap{..})`, joined to the Gate-1 bypass (`!is_emergency && !is_forward_large_gap`). Gates 2/3/4/5 untouched. `CoordinatorGenesisEscalation` and `HeightOffsetDetected` deliberately excluded → remain floor-gated. Correct.
- **DC-1** deletes only `|| gap > self.snap.threshold`. Route C (`local_height==0`) and Route B (`needs_genesis_resync`) preserved verbatim.
- **Test migration honesty**: T-RG-001 and T-RG-009 swapped `StuckSyncLargeGap` → `CoordinatorGenesisEscalation` (a reason still floor-gated) to preserve floor-refusal coverage now that StuckSyncLargeGap is exempt — legitimate, not weakened. T-M2-009 removes the two now-exempt reasons from the "blocked" list AND adds a new positive block asserting they are HONORED — strengthened coverage. tests_inc_i139 change is purely removing `#[ignore]` from class2/class4 (DC-1/DC-2 now landed). Honest.

## Gate 4 Verification (Task 3)
`production_gate.rs:726` reads `if self.snap.threshold == u64::MAX && !is_emergency` — unchanged. `is_forward_large_gap` is NOT in the Gate-4 condition. Confirmed: forward-large-gap reasons are floor-exempt (Gate 1) but still refused by `--no-snap-sync` (Gate 4). Matches DC-2 intent exactly.

## Exploratory Testing Findings (Task 4)
| # | What Was Tried | Expected | Actual | Severity |
|---|---|---|---|---|
| 1 | Trace all writers of `needs_genesis_resync = true` | single gated writer | Exactly one: `production_gate.rs:764`, after all 5 gates pass | none |
| 2 | Can a bare small gap reach `request_genesis_resync` with an honored result on a floor>0 node? | no | No. Floor-exempt reasons: `StuckSyncLargeGap` requires gap>1000 (cleanup.rs:610); `CoordinatorSnapEscalation` from coordinator Rule 2 requires large_gap≥500 OR rollback-exhausted OR deep-fork-confirmed (gap≥50 + 10 empty headers + stale tip). `HeightOffsetDetected`/`AllPeersBlacklistedDeepFork`/`ApplyFailuresSnapThreshold` are floor-gated or emergency and require additional evidence (stable-gap 120s / 20 empty headers / 3 apply failures). | none |

**Conclusion**: post-DC-1 a bare gap contributes nothing to `should_snap`; the only way to set `needs_genesis_resync` is the gated method, and no bare-gap trigger produces a floor-exempt honored request. REQ-SNAP-001/002 hold structurally, not just in the unit fixture.

## Failure Mode Validation
| Scenario | Triggered | Detected | Recovered | Degraded OK | Notes |
|---|---|---|---|---|---|
| Minor fork/stall (gap<500) attempts snap | Yes (class2) | Yes | n/a | Yes | falls through to header-first; no unnecessary snap |
| Legit gap≥500 coordinator catch-up | Yes (class4) | Yes | Yes | Yes | reaches snap, not stranded |
| Backward genesis wipe on synced node | Yes (T-M2-009/class8) | Yes | n/a | Yes | still refused (CR-1: floor guards backward only) |

## Security Validation
Not applicable — change is internal sync-decision logic with no external-data trust boundary or new input surface. No injection/attack surface introduced.

## Specs/Docs Drift
| File | Documented | Actual | Severity |
|---|---|---|---|
| `specs/sync-snap-admission-architecture.md` (DC-1/DC-2/CR-1) | matches | code matches spec exactly | none |

## Blocking Issues
None.

## Non-Blocking Observations
- **OBS-001**: `decision.rs:177,202` still reference `gap > self.snap.threshold` inside the fresh-node-wait and discv5-grace guards. These are gated by `self.local_height == 0` / `!enough_peers` (peer-wait timing only) and do not admit snap by themselves, so they are correct and out of DC-1 scope — noted only for future readers who may mistake them for a surviving bare-gap admission.

## Final Verdict
**PASS** — All Must acceptance criteria (REQ-SNAP-001/002/003/010, CR-1) met. Full network lib suite green (442/0/2). DC-1 and DC-2 correctly scoped; Gate 4 unchanged; no bare-gap snap-admission path survives; test migration honest and strengthened. Approved for review.
