# Code Review: INC-I-139 DC-4 (M5) — single-owner evidence counter

Run: 455 · Incident: INC-I-139 · Milestone: M5 · Workflow: redesign
Base: 2930c80b · Scope: `crates/network/` only

## Scope Reviewed
- `crates/network/src/sync/manager/sync_engine/dispatch.rs` (deletion + comment reword)
- `crates/network/src/sync/manager/tests_inc_i139.rs` (class 3 de-ignored)
- `crates/network/src/sync/manager/tests.rs` (2 pin tests reconciled)
- Cross-checked writers in `block_lifecycle.rs`, `sync_engine/response.rs`, `production_gate.rs`
- Spec `specs/sync-snap-admission-architecture.md` (DC-4 L134-137, INV-SYNC-011 L264, Migration Path L285)

## Summary
Approved. Root-cause-correct single-owner fix, matches spec M5 verbatim, both reconciled pin
tests correctly assert preservation, INV-SYNC-011's operative prohibition is satisfied. No
blocking findings; three Minor/observational notes.

## Verification of the 5 required points

**1. Root-cause correctness — CONFIRMED.** The `dispatch.rs` height-based branch now clears
only `use_height_based_headers = false` and issues `GetHeadersByHeight` without touching
`consecutive_empty_headers`. The E5 starvation writer is gone. The height-based branch is
checked *before* the `>=10` escalation, so the counter now accumulates as evidence across
height-based retries while the request still fires. Matches spec L134. conf(0.9, observed)

**2. The two reconciled pin tests — LEGITIMATE, no hidden regression.**
- `test_inc_i017_height_based_request_fires_before_genesis_fallback`: pre-set counter=15,
  asserts request fires (`GetHeadersByHeight{start_height:1}`), flag cleared, and counter
  preserved (== 15). The old "reset" assertion *was* the E5 defect.
- `test_inc_i139_dc4_height_fallback_dispatch_preserves_counter_pin` (renamed): counter=4 →
  next_request → asserts == 4 (preserved).
- Sibling `test_inc_i138_d2_block_applied_resets_counter_pin` untouched — still asserts == 0
  after `block_applied_with_weight` (block_lifecycle.rs:68). Reset-on-apply retained.
- INC-I-012 F1 post-snap window still safe: at gap=28 the counter=10 path parks at
  `minor_fork_await_g3_coordinator`, never genesis-resync; small-gap diverted; deep_fork needs
  300s staleness (recovery.rs guard, pinned by `test_inc_i138_d4_*`). conf(0.88, observed)

**3. No unintended behavior — CONFIRMED.** The only behavioral delta is the removed counter
zeroing. Height-based request still fires before genesis fallback; flag still cleared. No other
lines changed. conf(0.9, observed)

**4. Remaining `consecutive_empty_headers = 0` writers — INV-SYNC-011 operative prohibition
SATISFIED.** None is a request-dispatch-time or snap-admission reset:

| Writer | Class | Verdict |
|---|---|---|
| block_lifecycle.rs:68 | genuine block apply | allowed (a) |
| block_lifecycle.rs:303,360 | `reset_local_state` full genesis reset | tip-fundamentally-reset, not dispatch/admission |
| dispatch.rs:118 | gap≤3 gossip-wait | allowed (b), bounded exception |
| response.rs:400 | valid-headers-received progress reset | progress, not dispatch/admission |
| response.rs:316 | anti-cascade, gated on `recently_synced<60s` + gap<50 | recency-bounded; hands off to `signal_stuck_fork` after 60s |
| production_gate.rs:558 | `reset_empty_headers` after rollback tip change | tip-change, not dispatch/admission |
| production_gate.rs:620 | `set_post_recovery_grace` post-recovery | recovery reset, not the E5 class |

The sole request-dispatch reset (dispatch.rs:84) is removed; no admission path resets the
counter. conf(0.85, observed)

**5. Spec/docs alignment — MATCHES; no drift introduced by M5.** Spec Migration Path M5 (L285)
= "remove the unconditional reset dispatch.rs:84 (keep :83 flag-clear); run the M1 co-test
suite" — done exactly. Complexity table L301 ("1 + 1 documented bounded exception") now holds.
Doc updates + memory.db INV-SYNC-011 extension are correctly scoped to M7 (L287), not M5.

## Minor Findings (non-blocking, no fix required for M5)
- INV-SYNC-011 draft text (`specs/sync-snap-admission-architecture.md:264`) is narrower than the
  codebase — it does not enumerate the legitimate progress/tip-change/recovery resetters. The
  *operative* clause ("no request-shape change or admission path may reset it") is fully
  satisfied. Suggest M7 clarify the enumeration scopes to the empty-headers-stall regime.
- M7 close-out remains pending (memory.db INV extension, docs/troubleshooting.md,
  docs/architecture.md, protection-mechanism registration, gauntlet INC-I-139 seed). Not an M5
  defect; flagged so it isn't lost.

## Resource Cost
The change *removes* a field-zeroing on a per-tick path — net negative cost, no hot-path
allocation added. No resource-cost concern.

## Final Verdict
Approved for merge. Zero blocking issues; 2 Minor tracked observations.

Security Audit Verdict: AUDIT-SKIP
Signals: none — node-local internal sync-recovery counter refactor; no trust boundary, no
external/attacker-controlled input surface change (empty-headers responses were already counted;
only the reset-writer was removed).
