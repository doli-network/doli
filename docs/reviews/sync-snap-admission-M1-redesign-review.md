# Code Review: INC-I-139 M1 — Snap-Admission Redesign Regression Suite (run_id 455)

## Scope Reviewed
- `crates/network/src/sync/manager/tests_inc_i139.rs` (498 lines, 8 classes / 10 test fns)
- `crates/network/src/sync/manager/mod.rs` (registration only — 2-line `#[cfg(test)] mod tests_inc_i139;`)
- Ground-truth cross-check against: `sync_engine/decision.rs`, `sync_engine/dispatch.rs`, `production_gate.rs`, `recovery.rs`, `types.rs`, `peers.rs`, and the `tests_inc_i090_d1.rs` harness.

## Summary
**Approved.** Every FAIL-by-design class fails against current code on a genuine root-cause assertion; every PASS class locks behavior the current code actually produces; the mod.rs change is exactly the 2-line registration; no source logic was touched.

## Verification Detail (all confirmed against source)

### FAIL-by-design classes — genuine root-cause, correct landing milestone
- **Class 2 (DC-1 → M3):** `decision.rs:167-169` OR-clause `|| gap > self.snap.threshold` (line 168). local=100 / 3 peers @151 → gap=51>50, `needs_genesis_resync=false` → `should_snap=true` → admits snap today. Asserts NOT-SnapCollecting → fails today, correct. `#[ignore] = "…DC-1 lands (INC-I-139 M3)"`.
- **Class 3 (DC-4 → M5):** `dispatch.rs:83-84` unconditionally zeroes `consecutive_empty_headers` on every height-based request. Loop increments→records(max=1)→resets each cycle → `max_counter` stays 1. Asserts ≥10 → fails today, correct. Post-DC-4 the manual increments accumulate to 15.
- **Class 4 (DC-2 → M2):** `production_gate.rs:674` Gate 1 (`confirmed_height_floor>0 && !is_emergency`). Emergency set (666-671) is `{GenesisFallbackEmptyHeaders, AllPeersBlacklistedDeepFork, ApplyFailuresSnapThreshold}` — `CoordinatorSnapEscalation` (types.rs:368) is NOT in it → refused with floor=100. Asserts `honored==true` → fails today, correct.

**Prior harness-bug class is gone:** none of Class 2/3/4 assert `state==Idle` (or any precondition artifact). Class 2 sets `state=Idle` as setup and asserts on `pipeline_data`; Class 3 asserts on the counter; Class 4 asserts on the `bool` + `needs_genesis_resync()`.

### PASS classes lock currently-correct behavior (traced through `recovery.rs::classify` and `dispatch.rs`)
- Class 1a: 2 empties gap=10, finality=100 → Rule 1 finality guard (target 99<100) → `None`.
- Class 1b: 10 empties gap=55 + stale=325 → `deep_fork_confirmed` (empty≥10 ∧ stale≥300 ∧ gap≥50) → `SnapSync`.
- Class 5 / 7b: gap=28, counter=10, floor=0 → `dispatch.rs:144` minor-fork regime guard → parks, no genesis-resync.
- Class 6: h==0 Route C (`decision.rs:167 local_height==0`) → `SnapCollecting`.
- Class 7a: gap=12 epoch-boundary, finality=36 → target 35<36 → `None`.
- Class 8: `HeightOffsetDetected` not in emergency set → floor-gated refuse (`honored==false`).

**Harness conventions:** matches `tests_inc_i090_d1.rs` (direct field mutation + `add_peer`; child-module private-field access is valid). `add_peer` auto-invoke of `start_sync` (`peers.rs:67-68`) and `min_peers_for_sync=1` (`types.rs:46`) confirm the "first-peer commits to header-first" note in Class 2/6. `SyncPipelineData::SnapCollecting` (types.rs:120) is a real observable.

## Minor Findings (non-blocking)
1. **Class 3 uses an indirect proxy** — it asserts the counter *can reach* ≥10, not that escalation *fires* (the height-based branch still returns early at `dispatch.rs:89`). The test comment is transparent about this; acceptable as a unit-level proxy for "evidence pipeline not starved." conf(0.85, observed). No fix required.
2. **Class 1a locks the "wedge parks" behavior** (`None`) which the redesign ultimately targets. Honestly labeled current behavior (not a silent latent bug); the defect assertions live in the FAIL classes — correct for an M1 baseline. conf(0.8, inferred). No action (out of M1 scope).

## Source Drift
`mod.rs` change is exactly the 2-line `#[cfg(test)] mod tests_inc_i139;` registration; `decision.rs`, `dispatch.rs`, `production_gate.rs`, `recovery.rs` contain only pre-existing logic (historical INC references, no M1 edits). Consistent with a tests-only change.

## Specs/Docs
M1 requires no spec edits (M7 handles INV-SYNC-011 memory.db registration). The tests are internally consistent with the redesign's REQ-SNAP / DC framing. Not a blocker.

## Final Verdict
**Approved for merge.**

**Security Audit Verdict: AUDIT-SKIP** (tests-only change to internal sync state-machine test code; no external-data handling, no new production logic, no crypto/auth/network-input surface).
