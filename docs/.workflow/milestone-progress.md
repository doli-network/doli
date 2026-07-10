# Milestone Progress — INC-I-139 Phase 1 (RUN 455, redesign --fix)

Spec: specs/sync-snap-admission-architecture.md · Analysis: docs/redesigns/sync-snap-admission-redesign-analysis.md
Workflow type: redesign · Scope: crates/network/src/sync/ (production_gate.rs, decision.rs, dispatch.rs, block_lifecycle.rs — all in the `network` crate; the payload's `bins/node/...production_gate.rs` path was a drift, corrected during M2+M3)
Phase 2 (M8-M10, AH-gated wedge-escape) explicitly OUT of this run — separate decision session.

| ID | Name | Scope | Depends | Status |
|----|------|-------|---------|--------|
| M1 | Regression test suite (classes 1-8; 2, 3 & 4 FAIL-by-design against current code) | tests only | — | COMPLETE (2026-07-10) |
| M2+M3 | DC-2 floor-gate forward exemption + DC-1 delete Route A (ATOMIC, one commit) | production_gate.rs:666-700, sync_engine/decision.rs:164-167 | M1 | COMPLETE (2026-07-10, 622c373c) |
| M4 | DC-3 delete A1 redirect (keep regime guards; funnel fallthrough retained) | sync_engine/dispatch.rs A1 block | M2+M3 | COMPLETE (2026-07-10) |
| M5 | DC-4 counter single-owner (remove dispatch.rs:84 reset, keep :83) + co-test suite | dispatch.rs:84 | M4 | COMPLETE (2026-07-10) |
| M6 | RC-1 threshold demotion + discv5-grace h==0 gate + RC-2 emergency taxonomy sentinel | types.rs:468, decision.rs:163/177/204, production_gate.rs:741 | M5 | COMPLETE (2026-07-10) — QA PASS (6/6 ACs), review APPROVED, 5-auditor sweep PROCEED (0 M6-introduced P0/P1); tests split to tests_inc_i139_m6.rs (800-line budget); network 451/0/1 |
| M7 | Close-out: extend INV-SYNC-011, register regression_tests + protection_mechanisms, docs, gauntlet run | memory.db, docs/ | M6 | COMPLETE (2026-07-10) — sha 015342ed. INV-SYNC-011 extended (L3), 19 regression_tests + PM-016..019 + monitoring signal + GS-002 mapping. docs troubleshooting §7.3 + architecture §4.3. Gauntlet 7/7 non-waived PASS (row 12); GS-001 refined (absent block-1 ≠ divergence) + waived-with-evidence (pre-existing genesis-reset artifact, genesis uniform). Runs 455/456 closed; INC-I-139 resolved. |

## M4 outcome (RUN 455, 2026-07-10)

- DC-3: deleted the A1 `deep_fork_snap_redirect` block in `sync_engine/dispatch.rs` (the
  `enough_peers` binding + the `gap > snap.threshold && floor > 0` redirect that zeroed
  `snap.attempts` and reset `consecutive_empty_headers`). Deep-fork empty-headers now falls through
  to the gated B3 emergency funnel `request_genesis_resync(GenesisFallbackEmptyHeaders)` (CR-2).
- Retained: gap<=3 gossip-wait guard, gap 4..49 minor-fork regime guard, funnel fallthrough,
  and dispatch.rs:83-84 (M5 scope, untouched).
- TDD: new CLASS 9 `class9_a1_does_not_reset_snap_attempts` (tests_inc_i139.rs) FAILED-by-design on
  HEAD (observed snap.attempts=0, expected 2), PASSES post-deletion. Obsolete
  `test_inc_i017_deep_fork_snap_redirect_allowed_for_synced_nodes` (F4, asserted the deleted A1
  side effect) removed; F3 fresh-node sibling retained and green.
- Suite: 442 passed / 0 failed / 2 ignored (class3 stays #[ignore] for M5/DC-4).
- QA: PASS (`docs/qa/inc-i-139-M4-qa-report.md`). Review: APPROVED, AUDIT-SKIP
  (`docs/reviews/inc-i-139-M4-deletion-review.md`).

## M1 outcome (RUN 455, 2026-07-10)
- Deliverable: `crates/network/src/sync/manager/tests_inc_i139.rs` (10 tests / 8 classes) + `mod.rs` registration. TESTS-ONLY, zero source-logic change.
- FAIL/PASS profile (verified against current code): 7 non-ignored PASS (classes 1×2, 5, 6, 7×2, 8 — lock current good behavior); 3 `#[ignore]` FAIL-by-design reproductions:
  - class2 (DC-1 → de-ignored in M3): Route A bare-gap `decision.rs:168` admits snap at gap 51.
  - class3 (DC-4 → de-ignored in M5): `dispatch.rs:84` unconditional counter reset starves the evidence counter.
  - class4 (DC-2 → de-ignored in M2): Gate 1 `production_gate.rs:674` refuses CoordinatorSnapEscalation — proves DC-2 is load-bearing.
- Full `network` crate suite green: 440 passed, 0 failed, 4 ignored. fmt + clippy clean.
- Class 7 GAP: SyncManager unit layer cannot simulate true epoch-boundary GetHeaders-returns-empty (node/block_store layer); feasible PASS-variants covered, gap documented in the test file.
- QA: PASS (`docs/qa/sync-snap-admission-M1-qa-report.md`). Review: APPROVED, AUDIT-SKIP (`docs/reviews/sync-snap-admission-M1-redesign-review.md`).

## M2+M3 outcome (RUN 455, 2026-07-10, commit 622c373c, pushed)
- Atomic two-change commit (DC-1 never ships without DC-2, CR-1). Both target files are in the `network` crate (payload's bins/node path was drift — corrected).
- DC-2 (`crates/network/src/sync/manager/production_gate.rs`, request_genesis_resync): added `is_forward_large_gap` match {CoordinatorSnapEscalation, StuckSyncLargeGap}; Gate 1 (floor) now exempts emergency ∪ forward-large-gap. Gates 2/3/5 unchanged; Gate 4 (--no-snap-sync) stays emergency-only. CoordinatorGenesisEscalation + HeightOffsetDetected stay floor-gated.
- DC-1 (`crates/network/src/sync/manager/sync_engine/decision.rs`, should_snap): deleted bare-gap OR-term `|| gap > self.snap.threshold`. Route B/C preserved; fresh-node h==0 block untouched.
- TDD: de-ignored class2 + class4 → both FAILED pre-change, PASS post-change (FAIL→PASS captured). class3 stays ignored (DC-4/M5).
- Behavior-change casualties (investigated, not papered over): 4 pre-existing floor-gate tests (T-RG-001, T-RG-009, T-M2-002, T-M2-009) locked the OLD contract that DC-2 supersedes for StuckSyncLargeGap/CoordinatorSnapEscalation. Migrated honestly — assert the new HONORED behavior; floor-refusal coverage retained via CoordinatorGenesisEscalation (still gated). T-M2-002 was passing vacuously (130s < 300s stuck threshold → unreachable path); reviewer F-1 fix drives the real path (310s) + flips to `needs_genesis_resync == true`.
- Full `network` crate suite green: 442 passed, 0 failed, 2 ignored (class3=DC-4/M5 + 1 pre-existing). Workspace release build clean; network clippy + fmt clean. (Pre-existing workspace clippy failures in `crates/storage/src/state_db/tests.rs` — clippy::bool_assert_comparison — are unrelated and out of scope.)
- QA: PASS (`docs/qa/sync-snap-admission-M2M3-qa-report.md`). Review: APPROVED, AUDIT-SKIP (`docs/reviews/sync-snap-admission-M2M3-redesign-review.md`).
- Deploy: rolling-safe, node-local, NO activation height (Q1 consensus RULES=NO, Q2 block CONTENT=NO). INV-SYNC-007 preserved.

## M5 outcome (RUN 455, 2026-07-10)
- DC-4: removed the single line `self.fork.consecutive_empty_headers = 0;` at request-dispatch time
  in `crates/network/src/sync/manager/sync_engine/dispatch.rs` (the `use_height_based_headers` branch
  of `next_request`). Kept the adjacent `use_height_based_headers = false;` flag-clear and the bounded
  gap<=3 gossip-wait reset. Comment reworded to cite INV-SYNC-011 + INC-I-139 E5. This was the last
  request-dispatch/admission-path reset of the evidence counter — the E5 starvation writer.
- TDD FAIL->PASS: de-ignored `class3_counter_not_starved_by_dispatch84_reset` (tests_inc_i139.rs).
  RED against HEAD: "consecutive_empty_headers maxed at 1 across 15 cycles". GREEN post-DC-4:
  the counter accumulates to >=10 across request-shape changes.
- Pin reconciliation (DC-4 supersedes the INC-I-138 D2 carve-out per spec L136 "same defect class
  D2 fixed at periodic.rs:712"): `test_inc_i017_height_based_request_fires_before_genesis_fallback`
  now asserts counter PRESERVED (==15); `test_inc_i138_d2_height_fallback_dispatch_resets_counter_pin`
  renamed `test_inc_i139_dc4_height_fallback_dispatch_preserves_counter_pin`, asserts ==4. Sibling
  `test_inc_i138_d2_block_applied_resets_counter_pin` (block_lifecycle.rs:68 reset) untouched.
- Single-owner writer enumeration (REQ-SNAP-007 AC — post-change, non-test src): NO request-dispatch
  or snap-admission reset remains. Allowed: block_lifecycle.rs:68 (block apply) + dispatch.rs:118
  (gap<=3 gossip-wait). Legitimate progress/tip-change/recovery resets (out of E5 class):
  response.rs:400 (valid headers), response.rs:316 (anti-cascade recently_synced<60s),
  production_gate.rs:558 (rollback tip change), production_gate.rs:620 (post-recovery grace),
  block_lifecycle.rs:303/360 (full genesis reset).
- Post-snap false-positive check (spec DC-4): safe without the reset — small post-snap gap is diverted
  by regime guards (gap<=3 gossip-wait, 4..49 minor-fork park), B3 needs gap>=50, deep_fork_confirmed
  needs >=300s staleness while the first post-snap apply lands in seconds. Co-tested by class1/class3/class5.
- Suite: 443 passed / 0 failed / 1 ignored (the 1 ignore is pre-existing, unrelated to DC-4).
  Release build clean; network clippy -D warnings clean; fmt --check clean. (Pre-existing storage-crate
  clippy noise out of scope.)
- QA: PASS (`docs/qa/inc-i-139-M5-qa-report.md`). Review: APPROVED, AUDIT-SKIP
  (`docs/reviews/inc-i-139-M5-refactor-review.md`).
- Deploy: rolling-safe, node-local, NO activation height (Q1 consensus RULES=NO, Q2 block CONTENT=NO).
  INV-SYNC-007 preserved.

## M6 test coverage (RUN 456, test-writer, 2026-07-10)
- RC-1/RC-2 source changes landed concurrently in the working tree (decision.rs re-homes the
  fresh-node + discv5-grace gap comparators onto `thresholds::SNAP_SYNC_GAP_MIN` and gates the
  discv5-grace wait on `local_height==0`; production_gate.rs:750 replaces `threshold = 10` with
  `enable_snap_sync()` — canonical enabled sentinel = 50, `< u64::MAX` is the only observable effect).
- M6 test set in `tests_inc_i139.rs` (8 tests, all green; REQ-SNAP-008 + RC-2 taxonomy):
  - `m6_h_gt_0_skips_discv5_grace_proceeds_header_first` (RC-1c; genuine red vs pre-RC-1 ungated :202)
  - `m6_rc1b_no_gap_comparator_read_of_threshold_in_decision` (RC-1b structural; genuine red vs
    pre-RC-1 `> self.snap.threshold` at :177/:202)
  - `m6_rc2_emergency_reenable_admits_snap_under_no_snap_sync` (RC-2 bit-for-bit backstop)
  - `m6_rc2_forward_large_gap_not_operator_disable_exempt` (RC-2 capability ii: Gate 4 emergency-ONLY)
  - `m6_rc2_rate_and_attempt_limits_apply_to_emergencies` (RC-2 capability iii: Gates 3/5, no exception)
  - `m6_rc2_emergency_reenable_restores_enabled_sentinel_not_magic_10` (RC-2 exact sentinel ==50, !=10;
    genuine red vs pre-RC-2 `threshold = 10`)
  - `m6_rc1_fresh_node_h0_still_waits` (RC-1 bootstrap preservation, REQ-SNAP-003)
  - `m6_rc1_exact_ceiling_gap_does_not_float_snap` (REQ-SNAP-008 gap==MINOR_FORK_GAP_MAX non-promotion)
- Suite: `cargo test -p network --lib` = 446 passed / 0 failed / 1 ignored (pre-existing). fmt + clippy clean.
- Traceability matrix updated (REQ-SNAP-001, REQ-SNAP-008 Test IDs filled).
- No `#[ignore]` FAIL-by-design tests remain for M6: RC-1/RC-2 co-landed with the tests, so the
  final tree is green. Three tests would fail against pre-M6 code (documented above) — genuine, not vacuous.
