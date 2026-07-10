# Code Review: INC-I-139 M2+M3 (SnapSync Admission Redesign) — run_id 455

**Workflow**: redesign · **Incident**: INC-I-139 · **Verdict**: APPROVED (one Minor finding, addressed in-milestone)
**Security Audit Verdict**: **AUDIT-SKIP**

## Scope Reviewed
- `crates/network/src/sync/manager/sync_engine/decision.rs` (DC-1)
- `crates/network/src/sync/manager/production_gate.rs` (DC-2)
- `crates/network/src/sync/manager/tests.rs` (migrated floor-gate tests)
- `crates/network/src/sync/manager/tests_inc_i139.rs` (de-ignored class2 + class4)
- Cross-checked: `recovery.rs` (classify Rule 2 gating), `cleanup.rs` (raise sites), `types.rs` (`RecoveryReason` enum), `mod.rs`, spec `specs/sync-snap-admission-architecture.md`

## Summary
The production change is correct and closes N1's gap=51 door without stranding gap≥500 catch-up. Root-cause completeness, exemption scoping, and INV-SYNC-011 preservation all verified.

## Goal-by-goal verdict

**1. Root-cause completeness — PASS.** DC-1 reduces `should_snap` to `enough_peers && attempts<3 && snap_allowed && (local_height==0 || needs_genesis_resync)` — the bare-gap OR-term is gone. `needs_genesis_resync` is set *only* by `request_genesis_resync` (single funnel). DC-2 restores gap≥500 forward catch-up by making the two forward reasons floor-exempt. Genuine single-admission-authority fix, not a symptom patch.

**2. Over-reach / under-reach — PASS.** `is_forward_large_gap` matches **exactly** `CoordinatorSnapEscalation | StuckSyncLargeGap`. `CoordinatorGenesisEscalation` and `HeightOffsetDetected` remain floor-gated. Gate 4 (`--no-snap-sync`) bypasses on `is_emergency` **only** — forward-large-gap is NOT operator-disable-exempt. DC-1 left the fresh-node block (gated on `local_height==0`) untouched.

**3. Unintended behavior — PASS.** No bare minor-fork gap can reach snap: every floor-exempt reason carries corroborated evidence at its raise site — `StuckSyncLargeGap` raised only inside `if gap > 1000` (cleanup.rs); `CoordinatorSnapEscalation` originates from `classify()` Rule 2 (large_gap≥500 || deep_fork_confirmed(gap≥50) || rollback_exhausted). INV-SYNC-011/007 preserved.

**4. Test migration honesty — PASS.** T-RG-001, T-RG-009, T-M2-009 migrated correctly: each swaps the now-exempt reason for `CoordinatorGenesisEscalation` to keep asserting real Gate-1 refusal; T-M2-009 adds a positive block asserting the forward-large-gap reasons are HONORED via DC-2. class2/class4 de-ignored and pass; class3 correctly stays ignored (DC-4 is M5).

**5. Specs/docs drift — flagged (out of scope, M7).** Spec header still "PROPOSAL-ONLY"; `docs/troubleshooting.md`, `docs/architecture.md`, and INV-SYNC-011 registration pending M7 per Migration Path.

## Minor Findings

### F-1: Stale floor-gate test `T-M2-002` passed vacuously (ADDRESSED in this milestone)
- **Location:** `tests.rs` `test_stuck_sync_large_gap_uses_recovery_gate`
- **Severity:** Minor · conf(0.9, observed)
- **Evidence:** Test used `last_block_applied = now - 130s`, but the `StuckSyncLargeGap` raise (cleanup.rs) is guarded by `stuck_secs > 300`, so the path was unreachable and `!needs_genesis_resync` passed vacuously. Post-DC-2 the stated contract inverts: `StuckSyncLargeGap` with floor>0 is floor-EXEMPT → `needs_genesis_resync` becomes **true**. The stale test would mask a DC-2 partial-revert regression.
- **Resolution:** Fixed per option (a): drive the real path (`now - 310s`) and assert `needs_genesis_resync == true` (DC-2-consistent cleanup()-integration assertion).

## Observations (non-blocking)
- **OBS-1 (RC-2/M6):** discv5-grace block at decision.rs still uses `gap > snap.threshold` without an `h==0` gate; bounded impact, explicitly deferred to RC-2/M6.
- **OBS-2 (intended):** `classify()` `rollback_exhausted` → `CoordinatorSnapEscalation` is now floor-exempt via DC-2, so a floor>0 node that exhausted 10 shallow rollbacks at a small gap can snap. Evidence-gated (10 failed rollbacks = corroborated wedge), aligns with guaranteed-progress; not a bare-gap hole.

## Security Audit Verdict
**AUDIT-SKIP** — node-local sync-recovery admission logic; no external/untrusted input parsing, no crypto/auth/serialization change, no new network message or trust boundary added.

## Final Verdict
**Approved for merge.** DC-1 + DC-2 are correct, minimal, and rolling-safe; test migrations are honest and preserve floor-gate coverage via `CoordinatorGenesisEscalation`. F-1 addressed in-milestone.
